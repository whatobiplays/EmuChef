"""Shared planner-side step validation and normalization."""

from __future__ import annotations

from emuchef.domain import (
    ErrorCode,
    ErrorMessage,
    LiteralParamValue,
    ParamMode,
    Recipe,
    RefKind,
    RefParamValue,
    Step,
    parse_reference,
)
from emuchef.steps import builtin_step_registry

from .ids import make_execution_artifact_id, make_execution_input_id, make_execution_step_id

RUNTIME_ARTIFACT_FIELDS = {"status", "local_path", "resolved_url", "filename", "cache_hit", "error"}


def validate_step_contract(recipe_ref: str, step: Step, recipe: Recipe | None = None) -> tuple[ErrorMessage, ...]:
    plugin = builtin_step_registry().get(step.type)
    if plugin is None:
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unsupported step type {step.type!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "step_type": step.type},
            ),
        )

    spec = plugin.spec
    errors: list[ErrorMessage] = []
    provided = set(step.params)
    expected = set(spec.params)

    for param_name in sorted(provided - expected):
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unexpected param {param_name!r} for step type {step.type!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
        )

    normalized = _with_defaults(step)
    for param_name, param_spec in spec.params.items():
        if param_spec.required and param_name not in normalized:
            errors.append(
                ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Missing required param {param_name!r} for step type {step.type!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
            )
            continue
        if param_name not in normalized:
            continue
        value = normalized[param_name]
        if param_spec.mode is ParamMode.REF and not isinstance(value, RefParamValue):
            errors.append(
                ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Param {param_name!r} must use {{ref: ...}} for step type {step.type!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
            )
        if param_spec.mode is ParamMode.LITERAL and isinstance(value, RefParamValue):
            errors.append(
                ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Param {param_name!r} must remain a literal for step type {step.type!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
            )

    if plugin.validate is not None:
        errors.extend(plugin.validate(recipe_ref, step, normalized, recipe))
    return tuple(errors)


def validate_step_references(recipe: Recipe, step: Step) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    step_ids = {item.id: item for item in recipe.steps}

    for param_name, ref in referenced_bindings(step):
        try:
            parsed = parse_reference(ref)
        except ValueError:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.INVALID_REF_FORMAT,
                    message=f"Param {param_name!r} on step {step.id!r} has an invalid ref {ref!r}.",
                    details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                )
            )
            continue

        if parsed.kind is RefKind.INPUT:
            if parsed.target_id not in recipe.inputs:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.UNKNOWN_INPUT_REF,
                        message=f"Step {step.id!r} references unknown input {parsed.target_id!r}.",
                        details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                    )
                )
            continue

        if parsed.kind is RefKind.ARTIFACT_FIELD:
            if parsed.target_id not in recipe.artifacts:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.UNKNOWN_ARTIFACT_REF,
                        message=f"Step {step.id!r} references unknown artifact {parsed.target_id!r}.",
                        details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                    )
                )
                continue
            if parsed.field not in RUNTIME_ARTIFACT_FIELDS:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.UNKNOWN_ARTIFACT_FIELD,
                        message=f"Artifact ref {ref!r} uses unknown field {parsed.field!r}.",
                        details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                    )
                )
            continue

        target_step = step_ids.get(parsed.target_id)
        if target_step is None:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.UNKNOWN_STEP_REF,
                    message=f"Step {step.id!r} references unknown step {parsed.target_id!r}.",
                    details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                )
            )
            continue

        primary_output = builtin_step_registry().primary_output_name(target_step.type)
        if parsed.kind is RefKind.STEP_SHORTHAND:
            if primary_output is None:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.UNKNOWN_STEP_OUTPUT,
                        message=f"Step shorthand ref {ref!r} targets step type {target_step.type!r}, which has no primary output.",
                        details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                    )
                )
            continue

        if primary_output is None or parsed.field != primary_output:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.UNKNOWN_STEP_OUTPUT,
                    message=f"Step output ref {ref!r} targets unsupported output {parsed.field!r}.",
                    details={"recipe_ref": recipe.id, "step_id": step.id, "param": param_name, "ref": ref},
                )
            )

    return tuple(errors)


def normalize_step_params_for_execution(recipe: Recipe, step: Step) -> dict[str, object]:
    normalized = _with_defaults(step)
    plugin = builtin_step_registry().require(step.type)
    spec = plugin.spec
    result: dict[str, object] = {}

    if plugin.normalize is not None:
        return dict(plugin.normalize(recipe, step, normalized))

    for param_name, param_spec in spec.params.items():
        if param_name not in normalized:
            continue
        value = normalized[param_name]
        if isinstance(value, RefParamValue):
            result[param_name] = RefParamValue(ref=normalize_ref_for_execution(recipe, value.ref))
            continue
        result[param_name] = LiteralParamValue(value=value)

    return result


def normalize_ref_for_execution(recipe: Recipe, ref: str) -> str:
    parsed = parse_reference(ref)
    if parsed.kind is RefKind.INPUT:
        return f"inputs.{make_execution_input_id(recipe.id, parsed.target_id)}"
    if parsed.kind is RefKind.ARTIFACT_FIELD:
        return f"artifacts.{make_execution_artifact_id(recipe.id, parsed.target_id)}.{parsed.field}"
    if parsed.kind is RefKind.STEP_SHORTHAND:
        step = next(item for item in recipe.steps if item.id == parsed.target_id)
        primary_output = builtin_step_registry().primary_output_name(step.type)
        if primary_output is None:
            raise ValueError(f"Step {parsed.target_id!r} has no primary output.")
        return f"steps.{make_execution_step_id(recipe.id, parsed.target_id)}.outputs.{primary_output}"
    return f"steps.{make_execution_step_id(recipe.id, parsed.target_id)}.outputs.{parsed.field}"


def referenced_bindings(step: Step) -> tuple[tuple[str, str], ...]:
    refs: list[tuple[str, str]] = []
    for param_name, value in step.params.items():
        if isinstance(value, RefParamValue):
            refs.append((param_name, value.ref))
    return tuple(refs)


def _with_defaults(step: Step) -> dict[str, object]:
    spec = builtin_step_registry().require(step.type).spec
    normalized = dict(step.params)
    for param_name, param_spec in spec.params.items():
        if param_name not in normalized and param_spec.default is not None:
            normalized[param_name] = param_spec.default
    return normalized

"""Shared planner-side step validation and normalization."""

from __future__ import annotations

from collections import Counter
from collections.abc import Mapping, Sequence

from emuchef.domain import (
    BoundParamValue,
    CopyPolicy,
    ErrorCode,
    ErrorMessage,
    LiteralParamValue,
    ParamMode,
    PERMISSION_POLICY_ON_FAILURE_VALUES,
    Recipe,
    RefKind,
    RefParamValue,
    STEP_SPECS,
    Step,
    StepType,
    parse_reference,
)

from .ids import make_execution_artifact_id, make_execution_input_id, make_execution_step_id

RUNTIME_ARTIFACT_FIELDS = {"status", "local_path", "resolved_url", "filename", "cache_hit", "error"}


def validate_step_contract(recipe_ref: str, step: Step, recipe: Recipe | None = None) -> tuple[ErrorMessage, ...]:
    spec = STEP_SPECS.get(step.type)
    if spec is None:
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unsupported step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "step_type": step.type.value},
            ),
        )

    errors: list[ErrorMessage] = []
    provided = set(step.params)
    expected = set(spec.params)

    for param_name in sorted(provided - expected):
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Unexpected param {param_name!r} for step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
            )
        )

    normalized = _with_defaults(step)
    for param_name, param_spec in spec.params.items():
        if param_spec.required and param_name not in normalized:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Missing required param {param_name!r} for step type {step.type.value!r}.",
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
                    message=f"Param {param_name!r} must use {{ref: ...}} for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )
        if param_spec.mode is ParamMode.LITERAL and isinstance(value, RefParamValue):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Param {param_name!r} must remain a literal for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step.id, "param": param_name},
                )
            )

    errors.extend(_validate_step_specifics(recipe_ref, step, normalized, recipe))
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

        primary_output = STEP_SPECS.get(target_step.type).primary_output_name if STEP_SPECS.get(target_step.type) else None
        if parsed.kind is RefKind.STEP_SHORTHAND:
            if primary_output is None:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.UNKNOWN_STEP_OUTPUT,
                        message=f"Step shorthand ref {ref!r} targets step type {target_step.type.value!r}, which has no primary output.",
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
    spec = STEP_SPECS[step.type]
    result: dict[str, object] = {}

    if step.type in {StepType.RESOLVE_ARTIFACTS, StepType.EXTRACT_ARTIFACTS}:
        artifacts = expand_artifact_selection(recipe, normalized.get("artifacts"), normalized.get("artifact_groups"))
        result["artifacts"] = LiteralParamValue(
            value=[make_execution_artifact_id(recipe.id, artifact_id) for artifact_id in artifacts]
        )
        if step.type is StepType.EXTRACT_ARTIFACTS:
            result["extract_on"] = LiteralParamValue(value=normalized.get("extract_on", "host"))
        return result

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
        primary_output = STEP_SPECS[step.type].primary_output_name
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


def expand_artifact_selection(
    recipe: Recipe,
    artifacts: object | None,
    artifact_groups: object | None,
) -> tuple[str, ...]:
    selected: list[str] = []
    for artifact_id in _coerce_string_tuple(artifacts):
        selected.append(artifact_id)
    for group_id in _coerce_string_tuple(artifact_groups):
        selected.extend(recipe.artifact_groups.get(group_id, ()))
    return tuple(selected)


def _with_defaults(step: Step) -> dict[str, object]:
    spec = STEP_SPECS[step.type]
    normalized = dict(step.params)
    for param_name, param_spec in spec.params.items():
        if param_name not in normalized and param_spec.default is not None:
            normalized[param_name] = param_spec.default
    return normalized


def _validate_step_specifics(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    recipe: Recipe | None,
) -> list[ErrorMessage]:
    errors: list[ErrorMessage] = []
    step_id = step.id

    if step.type in {StepType.RESOLVE_ARTIFACTS, StepType.EXTRACT_ARTIFACTS}:
        artifacts = _coerce_string_tuple(normalized.get("artifacts"))
        groups = _coerce_string_tuple(normalized.get("artifact_groups"))
        if not artifacts and not groups:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Step {step_id!r} must declare at least one of 'artifacts' or 'artifact_groups'.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id},
                )
            )
        if recipe is not None:
            for artifact_id in artifacts:
                if artifact_id not in recipe.artifacts:
                    errors.append(
                        ErrorMessage(
                            code=ErrorCode.UNKNOWN_ARTIFACT_REF,
                            message=f"Step {step_id!r} references unknown artifact {artifact_id!r}.",
                            details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "artifacts"},
                        )
                    )
            for group_id in groups:
                if group_id not in recipe.artifact_groups:
                    errors.append(
                        ErrorMessage(
                            code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                            message=f"Step {step_id!r} references unknown artifact group {group_id!r}.",
                            details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "artifact_groups"},
                        )
                    )
            expanded = list(artifacts)
            for group_id in groups:
                expanded.extend(recipe.artifact_groups.get(group_id, ()))
            duplicates = tuple(sorted(item for item, count in Counter(expanded).items() if count > 1))
            if duplicates:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                        message=f"Step {step_id!r} resolves duplicate artifact ids after group expansion.",
                        details={"recipe_ref": recipe_ref, "step_id": step_id, "duplicates": list(duplicates)},
                    )
                )

    if step.type in {StepType.RESOLVE_ARTIFACTS, StepType.EXTRACT_ARTIFACTS} and normalized.get("artifact_groups") is not None:
        if not isinstance(normalized.get("artifact_groups"), (list, tuple)):
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Param 'artifact_groups' must be a list of strings for step type {step.type.value!r}.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "artifact_groups"},
                )
            )

    if step.type is StepType.EXTRACT_ARCHIVE:
        extract_on = normalized.get("extract_on", "host")
        has_dest = "dest" in normalized
        if extract_on == "device" and not has_dest:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message="Param 'dest' is required when extract_archive.extract_on == 'device'.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "dest"},
                )
            )
        if extract_on == "host":
            for irrelevant in ("dest", "device_temp_path"):
                if irrelevant in step.params:
                    errors.append(
                        ErrorMessage(
                            code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                            message=f"Param {irrelevant!r} is only valid when extract_archive.extract_on == 'device'.",
                            details={"recipe_ref": recipe_ref, "step_id": step_id, "param": irrelevant},
                        )
                    )

    if step.type is StepType.COPY_FILES:
        try:
            CopyPolicy(str(normalized.get("copy_policy", CopyPolicy.MERGE.value)))
        except ValueError:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=(
                        "Param 'copy_policy' must be one of: "
                        + ", ".join(policy.value for policy in CopyPolicy)
                        + "."
                    ),
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "copy_policy"},
                )
            )

    if step.type is StepType.WAIT:
        duration = normalized.get("duration_ms")
        if isinstance(duration, bool) or not isinstance(duration, int) or duration <= 0:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message="Param 'duration_ms' must be a positive integer for step type 'wait'.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "duration_ms"},
                )
            )

    if step.type is StepType.FORCE_STOP_APP:
        package_name = normalized.get("package_name")
        if not isinstance(package_name, str) or not package_name.strip():
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message="Param 'package_name' must be a non-empty string for step type 'force_stop_app'.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "package_name"},
                )
            )

    if step.type is StepType.LAUNCH_APP:
        package_name = normalized.get("package_name")
        if not isinstance(package_name, str) or not package_name.strip():
            errors.append(
                ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message="Param 'package_name' must be a non-empty string for step type 'launch_app'.",
                    details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "package_name"},
                )
            )

    if step.type is StepType.GRANT_PERMISSIONS:
        errors.extend(_validate_grant_permissions_params(recipe_ref, step_id, normalized))

    return errors


def _validate_grant_permissions_params(
    recipe_ref: str,
    step_id: str,
    normalized: Mapping[str, object],
) -> list[ErrorMessage]:
    errors: list[ErrorMessage] = []
    runtime = _unwrap_literal(normalized.get("runtime"))
    appops = _unwrap_literal(normalized.get("appops"))
    policy = _unwrap_literal(normalized.get("policy"))

    if runtime is not None:
        errors.extend(
            _validate_permission_items(
                recipe_ref,
                step_id,
                "runtime",
                runtime,
                required_fields=("package_name", "name"),
                allowed_fields={"package_name", "name", "required", "when"},
            )
        )
    if appops is not None:
        errors.extend(
            _validate_permission_items(
                recipe_ref,
                step_id,
                "appops",
                appops,
                required_fields=("package_name", "op", "mode"),
                allowed_fields={"package_name", "op", "mode", "required", "when"},
            )
        )
    if policy is not None:
        errors.extend(_validate_permission_policy(recipe_ref, step_id, policy))
    return errors


def _validate_permission_items(
    recipe_ref: str,
    step_id: str,
    param_name: str,
    value: object,
    *,
    required_fields: tuple[str, ...],
    allowed_fields: set[str],
) -> list[ErrorMessage]:
    errors: list[ErrorMessage] = []
    if not isinstance(value, (list, tuple)):
        return [
            _param_error(
                recipe_ref,
                step_id,
                param_name,
                f"Param {param_name!r} must be a list for step type 'grant_permissions'.",
            )
        ]
    for index, item in enumerate(value):
        field_prefix = f"{param_name}[{index}]"
        if not isinstance(item, Mapping):
            errors.append(
                _param_error(
                    recipe_ref,
                    step_id,
                    field_prefix,
                    f"Param {field_prefix!r} must be a mapping for step type 'grant_permissions'.",
                )
            )
            continue
        for field_name in sorted(str(key) for key in item.keys() if str(key) not in allowed_fields):
            errors.append(
                _param_error(
                    recipe_ref,
                    step_id,
                    f"{field_prefix}.{field_name}",
                    f"Param {field_prefix!r} contains unsupported field {field_name!r}.",
                )
            )
        for field_name in required_fields:
            _append_required_text_error(errors, recipe_ref, step_id, item, f"{field_prefix}.{field_name}", field_name)
        if "required" in item and not isinstance(item["required"], bool):
            errors.append(
                _param_error(
                    recipe_ref,
                    step_id,
                    f"{field_prefix}.required",
                    f"Param {field_prefix}.required must be a boolean.",
                )
            )
        if "when" in item:
            errors.extend(_validate_permission_when(recipe_ref, step_id, f"{field_prefix}.when", item["when"]))
    return errors


def _validate_permission_policy(recipe_ref: str, step_id: str, value: object) -> list[ErrorMessage]:
    errors: list[ErrorMessage] = []
    if not isinstance(value, Mapping):
        return [
            _param_error(
                recipe_ref,
                step_id,
                "policy",
                "Param 'policy' must be a mapping for step type 'grant_permissions'.",
            )
        ]
    allowed_fields = {"on_failure", "require_all"}
    for field_name in sorted(str(key) for key in value.keys() if str(key) not in allowed_fields):
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                f"policy.{field_name}",
                f"Param 'policy' contains unsupported field {field_name!r}.",
            )
        )
    on_failure = value.get("on_failure", "warn")
    if on_failure not in PERMISSION_POLICY_ON_FAILURE_VALUES:
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                "policy.on_failure",
                "Param 'policy.on_failure' must be one of: " + ", ".join(PERMISSION_POLICY_ON_FAILURE_VALUES) + ".",
            )
        )
    if "require_all" in value and not isinstance(value["require_all"], bool):
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                "policy.require_all",
                "Param 'policy.require_all' must be a boolean.",
            )
        )
    return errors


def _validate_permission_when(
    recipe_ref: str,
    step_id: str,
    field_prefix: str,
    value: object,
) -> list[ErrorMessage]:
    errors: list[ErrorMessage] = []
    if not isinstance(value, Mapping):
        return [_param_error(recipe_ref, step_id, field_prefix, f"Param {field_prefix!r} must be a mapping.")]
    allowed_fields = {"rooted", "android_api_min", "android_api_max"}
    for field_name in sorted(str(key) for key in value.keys() if str(key) not in allowed_fields):
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                f"{field_prefix}.{field_name}",
                f"Param {field_prefix!r} contains unsupported field {field_name!r}.",
            )
        )
    rooted = value.get("rooted")
    if rooted is not None and not isinstance(rooted, bool):
        errors.append(_param_error(recipe_ref, step_id, f"{field_prefix}.rooted", f"Param {field_prefix}.rooted must be a boolean."))
    api_min = value.get("android_api_min")
    api_max = value.get("android_api_max")
    if api_min is not None and (isinstance(api_min, bool) or not isinstance(api_min, int)):
        errors.append(
            _param_error(recipe_ref, step_id, f"{field_prefix}.android_api_min", f"Param {field_prefix}.android_api_min must be an integer.")
        )
    if api_max is not None and (isinstance(api_max, bool) or not isinstance(api_max, int)):
        errors.append(
            _param_error(recipe_ref, step_id, f"{field_prefix}.android_api_max", f"Param {field_prefix}.android_api_max must be an integer.")
        )
    if isinstance(api_min, int) and not isinstance(api_min, bool) and isinstance(api_max, int) and not isinstance(api_max, bool) and api_min > api_max:
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                field_prefix,
                f"Param {field_prefix!r} must not set android_api_min above android_api_max.",
            )
        )
    return errors


def _append_required_text_error(
    errors: list[ErrorMessage],
    recipe_ref: str,
    step_id: str,
    item: Mapping[str, object],
    param_name: str,
    field_name: str,
) -> None:
    value = item.get(field_name)
    if not isinstance(value, str) or not value.strip():
        errors.append(
            _param_error(
                recipe_ref,
                step_id,
                param_name,
                f"Param {param_name!r} must be a non-empty string.",
            )
        )


def _param_error(recipe_ref: str, step_id: str, param_name: str, message: str) -> ErrorMessage:
    return ErrorMessage(
        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
        message=message,
        details={"recipe_ref": recipe_ref, "step_id": step_id, "param": param_name},
    )


def _unwrap_literal(value: object) -> object:
    if isinstance(value, LiteralParamValue):
        return value.value
    return value


def _coerce_string_tuple(value: object | None) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, (list, tuple)):
        return tuple(str(item) for item in value)
    return ()


def _duplicates(values: Sequence[str]) -> tuple[str, ...]:
    seen: set[str] = set()
    duplicates: set[str] = set()
    for value in values:
        if value in seen:
            duplicates.add(value)
        else:
            seen.add(value)
    return tuple(sorted(duplicates))

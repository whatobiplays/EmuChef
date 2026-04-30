"""Planner hooks owned by built-in step plugins."""

from __future__ import annotations

from collections import Counter
from collections.abc import Mapping

from emuchef.domain.codes import ErrorCode
from emuchef.domain.copy_policy import CopyPolicy
from emuchef.domain.issues import ErrorMessage
from emuchef.domain.param_values import LiteralParamValue
from emuchef.domain.recipe import PERMISSION_POLICY_ON_FAILURE_VALUES, Recipe
from emuchef.domain.step import Step


def normalize_artifact_selection(recipe: Recipe, step: Step, normalized: Mapping[str, object]) -> dict[str, object]:
    from emuchef.planner.ids import make_execution_artifact_id

    result: dict[str, object] = {}
    artifacts = _expand_artifact_selection(recipe, normalized.get("artifacts"), normalized.get("artifact_groups"))
    result["artifacts"] = LiteralParamValue(
        value=[make_execution_artifact_id(recipe.id, artifact_id) for artifact_id in artifacts]
    )
    if "extract_on" in normalized:
        result["extract_on"] = LiteralParamValue(value=normalized.get("extract_on", "host"))
    return result


def validate_artifact_selection(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    step_id = step.id
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

    if normalized.get("artifact_groups") is not None and not isinstance(normalized.get("artifact_groups"), (list, tuple)):
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Param 'artifact_groups' must be a list of strings for step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step_id, "param": "artifact_groups"},
            )
        )
    return tuple(errors)


def validate_extract_archive(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    _recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    extract_on = normalized.get("extract_on", "host")
    has_dest = "dest" in normalized
    if extract_on == "device" and not has_dest:
        errors.append(
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message="Param 'dest' is required when extract_archive.extract_on == 'device'.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": "dest"},
            )
        )
    if extract_on == "host":
        for irrelevant in ("dest", "device_temp_path"):
            if irrelevant in step.params:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                        message=f"Param {irrelevant!r} is only valid when extract_archive.extract_on == 'device'.",
                        details={"recipe_ref": recipe_ref, "step_id": step.id, "param": irrelevant},
                    )
                )
    return tuple(errors)


def validate_copy_files(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    _recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    try:
        CopyPolicy(str(normalized.get("copy_policy", CopyPolicy.MERGE.value)))
    except ValueError:
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message="Param 'copy_policy' must be one of: " + ", ".join(policy.value for policy in CopyPolicy) + ".",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": "copy_policy"},
            ),
        )
    return ()


def validate_wait(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    _recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    duration = normalized.get("duration_ms")
    if isinstance(duration, bool) or not isinstance(duration, int) or duration <= 0:
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message="Param 'duration_ms' must be a positive integer for step type 'wait'.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": "duration_ms"},
            ),
        )
    return ()


def validate_package_name(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    _recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    package_name = normalized.get("package_name")
    if not isinstance(package_name, str) or not package_name.strip():
        return (
            ErrorMessage(
                code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                message=f"Param 'package_name' must be a non-empty string for step type {step.type.value!r}.",
                details={"recipe_ref": recipe_ref, "step_id": step.id, "param": "package_name"},
            ),
        )
    return ()


def validate_grant_permissions(
    recipe_ref: str,
    step: Step,
    normalized: Mapping[str, object],
    _recipe: Recipe | None,
) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    runtime = _unwrap_literal(normalized.get("runtime"))
    appops = _unwrap_literal(normalized.get("appops"))
    policy = _unwrap_literal(normalized.get("policy"))

    if runtime is not None:
        errors.extend(
            _validate_permission_items(
                recipe_ref,
                step.id,
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
                step.id,
                "appops",
                appops,
                required_fields=("package_name", "op", "mode"),
                allowed_fields={"package_name", "op", "mode", "required", "when"},
            )
        )
    if policy is not None:
        errors.extend(_validate_permission_policy(recipe_ref, step.id, policy))
    return tuple(errors)


def _expand_artifact_selection(
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

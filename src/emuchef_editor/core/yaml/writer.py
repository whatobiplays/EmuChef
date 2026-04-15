"""Canonical YAML emission for authored recipe documents."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

import yaml

from emuchef.domain import (
    AppOpGrant,
    InputDeclaration,
    InputValidation,
    LiteralParamValue,
    PermissionPolicy,
    PermissionSet,
    PermissionWhen,
    Recipe,
    RefParamValue,
    RemoteFileArtifact,
    RuntimePermissionGrant,
    Step,
    StepCondition,
    StepConstraints,
    STEP_SPECS,
)

_TOP_LEVEL_FIELDS = (
    "schema_version",
    "kind",
    "id",
    "name",
    "description",
    "recipe_dependencies",
    "provides",
    "inputs",
    "artifacts",
    "artifact_groups",
    "permissions",
    "steps",
)


def emit_recipe_yaml(recipe: Recipe) -> str:
    return yaml.safe_dump(build_recipe_payload(recipe), sort_keys=False, allow_unicode=True)


def write_recipe_yaml(recipe: Recipe, path: str | Path) -> str:
    payload = emit_recipe_yaml(recipe)
    Path(path).write_text(payload, encoding="utf-8")
    return payload


def build_recipe_payload(recipe: Recipe) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    recipe_values = {
        "schema_version": recipe.schema_version,
        "kind": recipe.kind,
        "id": recipe.id,
        "name": recipe.name,
        "description": recipe.description,
        "provides": {"features": list(recipe.provides.features)},
        "inputs": {input_id: _serialize_input(declaration) for input_id, declaration in sorted(recipe.inputs.items())},
        "artifacts": {
            artifact_id: _serialize_artifact(artifact)
            for artifact_id, artifact in sorted(recipe.artifacts.items())
        },
        "artifact_groups": {
            group_id: list(artifact_ids)
            for group_id, artifact_ids in sorted(recipe.artifact_groups.items())
        },
        "permissions": _serialize_permissions(recipe.permissions),
        "steps": [_serialize_step(step) for step in recipe.steps],
    }
    if hasattr(recipe, "recipe_dependencies"):
        recipe_values["recipe_dependencies"] = list(recipe.recipe_dependencies)
    for field_name in _TOP_LEVEL_FIELDS:
        if field_name not in recipe_values:
            continue
        value = recipe_values[field_name]
        if value is None and field_name == "description":
            continue
        payload[field_name] = value
    return payload


def _serialize_input(declaration: InputDeclaration) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "type": declaration.type.value,
        "role": declaration.role.value,
        "label": declaration.label,
        "required": declaration.required,
        "multiple": declaration.multiple,
        "validation": _serialize_input_validation(declaration.validation),
        "default": _serialize_json_value(declaration.default),
    }
    if declaration.description is not None:
        payload["description"] = declaration.description
    if declaration.metadata:
        payload["metadata"] = _serialize_json_mapping(declaration.metadata)
    return _ordered_mapping(
        payload,
        (
            "type",
            "role",
            "label",
            "description",
            "required",
            "multiple",
            "validation",
            "default",
            "metadata",
        ),
    )


def _serialize_input_validation(validation: InputValidation) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "must_exist": validation.must_exist,
        "allowed_extensions": list(validation.allowed_extensions),
    }
    if validation.path_kind is not None:
        payload["path_kind"] = validation.path_kind.value
    return _ordered_mapping(payload, ("must_exist", "allowed_extensions", "path_kind"))


def _serialize_artifact(artifact: RemoteFileArtifact) -> dict[str, Any]:
    return _ordered_mapping(
        {
            "type": artifact.type.value,
            "url": artifact.url,
            "cache": artifact.cache.value,
        },
        ("type", "url", "cache"),
    )


def _serialize_permissions(permissions: PermissionSet) -> dict[str, Any]:
    return {
        "runtime": [_serialize_runtime_permission(item) for item in permissions.runtime],
        "appops": [_serialize_appop_permission(item) for item in permissions.appops],
        "policy": _serialize_permission_policy(permissions.policy),
    }


def _serialize_runtime_permission(permission: RuntimePermissionGrant) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "package_name": permission.package_name,
        "name": permission.name,
        "required": permission.required,
    }
    if permission.when is not None:
        payload["when"] = _serialize_permission_when(permission.when)
    return _ordered_mapping(payload, ("package_name", "name", "required", "when"))


def _serialize_appop_permission(permission: AppOpGrant) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "package_name": permission.package_name,
        "op": permission.op,
        "mode": permission.mode,
        "required": permission.required,
    }
    if permission.when is not None:
        payload["when"] = _serialize_permission_when(permission.when)
    return _ordered_mapping(payload, ("package_name", "op", "mode", "required", "when"))


def _serialize_permission_policy(policy: PermissionPolicy) -> dict[str, Any]:
    return _ordered_mapping(
        {
            "on_failure": policy.on_failure,
            "require_all": policy.require_all,
        },
        ("on_failure", "require_all"),
    )


def _serialize_permission_when(when: PermissionWhen) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    if when.rooted is not None:
        payload["rooted"] = when.rooted
    if when.android_api_min is not None:
        payload["android_api_min"] = when.android_api_min
    if when.android_api_max is not None:
        payload["android_api_max"] = when.android_api_max
    return _ordered_mapping(payload, ("rooted", "android_api_min", "android_api_max"))


def _serialize_step(step: Step) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "id": step.id,
        "type": step.type.value,
        "name": step.name,
        "user_toggleable": step.user_toggleable,
        "dependencies": list(step.dependencies),
        "constraints": _serialize_constraints(step.constraints),
        "skip_if": [_serialize_condition(condition) for condition in step.skip_if],
        "params": _serialize_step_params(step),
        "verify": [_serialize_condition(condition) for condition in step.verify],
    }
    if step.description is not None:
        payload["description"] = step.description
    return _ordered_mapping(
        payload,
        (
            "id",
            "type",
            "name",
            "description",
            "user_toggleable",
            "dependencies",
            "constraints",
            "skip_if",
            "params",
            "verify",
        ),
    )


def _serialize_constraints(constraints: StepConstraints) -> dict[str, Any]:
    return {
        "capabilities": list(constraints.capabilities),
        "conflicts_with": list(constraints.conflicts_with),
    }


def _serialize_condition(condition: StepCondition) -> dict[str, Any]:
    return _ordered_mapping(
        {
            "type": condition.type,
            "params": _serialize_json_mapping(condition.params),
        },
        ("type", "params"),
    )


def _serialize_step_params(step: Step) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    spec = STEP_SPECS.get(step.type)
    ordered_names: list[str] = []
    if spec is not None:
        ordered_names.extend(spec.params.keys())
    ordered_names.extend(sorted(param_name for param_name in step.params if param_name not in ordered_names))
    for param_name in ordered_names:
        if param_name not in step.params:
            continue
        payload[param_name] = _serialize_param_value(step.params[param_name])
    return payload


def _serialize_param_value(value: Any) -> Any:
    if isinstance(value, RefParamValue):
        return {"ref": value.ref}
    if isinstance(value, LiteralParamValue):
        return _serialize_json_value(value.value)
    return _serialize_json_value(value)


def _serialize_json_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        return _serialize_json_mapping(value)
    if isinstance(value, tuple):
        return [_serialize_json_value(item) for item in value]
    if isinstance(value, list):
        return [_serialize_json_value(item) for item in value]
    return value


def _serialize_json_mapping(value: Mapping[str, Any]) -> dict[str, Any]:
    return {str(key): _serialize_json_value(value[key]) for key in sorted(value)}


def _ordered_mapping(payload: Mapping[str, Any], order: tuple[str, ...]) -> dict[str, Any]:
    ordered: dict[str, Any] = {}
    for key in order:
        if key in payload:
            ordered[key] = payload[key]
    for key in sorted(payload):
        if key not in ordered:
            ordered[key] = payload[key]
    return ordered

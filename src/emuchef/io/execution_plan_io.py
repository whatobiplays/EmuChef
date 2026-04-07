"""Execution-plan loading from YAML files."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

from emuchef.domain import (
    ArtifactCacheMode,
    ArtifactType,
    ExecutionArtifact,
    ExecutionInputValue,
    DeviceContext,
    ExecutionPermissionPlan,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    LiteralParamValue,
    PermissionPlanAction,
    PermissionPlanReason,
    PermissionPlanSource,
    RefParamValue,
    RuntimeCapabilities,
    RuntimeValue,
    RuntimeValueType,
    StepCondition,
    StepConstraints,
    StepType,
    PermissionPolicy,
)

from .serde import load_yaml

PLANNER_ONLY_STEP_KEYS = {"selected", "user_toggleable", "availability", "reason"}


def load_execution_plan_file(path: str | Path) -> ExecutionPlan:
    raw = load_yaml(path)
    if not isinstance(raw, dict):
        raise ValueError("Execution plan file must contain a top-level mapping.")
    if raw.get("kind") == "planning_result":
        raw = raw.get("execution_plan")
        if raw is None:
            raise ValueError("Planning result does not contain an execution_plan.")
        if not isinstance(raw, dict):
            raise ValueError("planning_result.execution_plan must be a mapping.")
    if raw.get("kind") != "execution_plan":
        raise ValueError(f"Unsupported plan kind: {raw.get('kind')!r}")
    return parse_execution_plan(raw)


def parse_execution_plan(data: Mapping[str, Any]) -> ExecutionPlan:
    allowed_top_level = {
        "schema_version",
        "kind",
        "id",
        "source",
        "device_context",
        "runtime_capabilities",
        "inputs",
        "artifacts",
        "steps",
        "permission_plan",
    }
    unknown_top_level = set(data) - allowed_top_level
    if unknown_top_level:
        raise ValueError(f"Execution plan contains planner-only or unknown top-level fields: {sorted(unknown_top_level)}")

    return ExecutionPlan(
        id=str(data["id"]),
        source=ExecutionPlanSource(
            device_profile_ref=str(data["source"]["device_profile_ref"]),
            device_plan_ref=str(data["source"]["device_plan_ref"]),
            selected_recipe_refs=tuple(str(item) for item in data["source"].get("selected_recipe_refs", [])),
            expanded_recipe_refs=tuple(str(item) for item in data["source"].get("expanded_recipe_refs", [])),
        ),
        device_context=DeviceContext(
            manufacturer=str(data["device_context"]["manufacturer"]),
            model=str(data["device_context"]["model"]),
            android_version=int(data["device_context"]["android_version"]),
            android_api_level=int(data["device_context"]["android_api_level"])
            if data["device_context"].get("android_api_level") is not None
            else None,
            device_tags=tuple(str(item) for item in data["device_context"].get("device_tags", [])),
        ),
        runtime_capabilities=RuntimeCapabilities(
            adb_available=bool(data["runtime_capabilities"]["adb_available"]),
            apk_install=bool(data["runtime_capabilities"]["apk_install"]),
            shared_storage_write=bool(data["runtime_capabilities"]["shared_storage_write"]),
            app_launch=bool(data["runtime_capabilities"]["app_launch"]),
            shell_command=bool(data["runtime_capabilities"]["shell_command"]),
            package_remove_for_user=bool(data["runtime_capabilities"]["package_remove_for_user"]),
            root_shell=bool(data["runtime_capabilities"]["root_shell"]),
            app_data_write=bool(data["runtime_capabilities"]["app_data_write"]),
        ),
        inputs=tuple(
            ExecutionInputValue(id=str(item["id"]), value=_parse_runtime_value(item["value"])) for item in data.get("inputs", [])
        ),
        artifacts=tuple(_parse_execution_artifact(item) for item in data.get("artifacts", [])),
        steps=tuple(_parse_execution_step(item) for item in data.get("steps", [])),
        permission_plan=_parse_permission_plan(data.get("permission_plan")),
        schema_version=int(data["schema_version"]),
        kind=str(data["kind"]),
    )


def _parse_execution_step(data: Mapping[str, Any]) -> ExecutionStep:
    planner_only_fields = set(data) & PLANNER_ONLY_STEP_KEYS
    if planner_only_fields:
        raise ValueError(f"Execution step contains planner-only fields: {sorted(planner_only_fields)}")
    return ExecutionStep(
        id=str(data["id"]),
        recipe_ref=str(data["recipe_ref"]),
        type=StepType(str(data["type"])),
        name=str(data["name"]),
        dependencies=tuple(str(item) for item in data.get("dependencies", [])),
        constraints=StepConstraints(
            capabilities=tuple(str(item) for item in data.get("constraints", {}).get("capabilities", [])),
            conflicts_with=tuple(str(item) for item in data.get("constraints", {}).get("conflicts_with", [])),
        ),
        params={str(key): _parse_param_value(value) for key, value in data.get("params", {}).items()},
        skip_if=tuple(_parse_condition(item) for item in data.get("skip_if", [])),
        verify=tuple(_parse_condition(item) for item in data.get("verify", [])),
    )


def _parse_condition(data: Mapping[str, Any]) -> StepCondition:
    return StepCondition(type=str(data["type"]), params=dict(data.get("params", {})))


def _parse_permission_plan(data: Mapping[str, Any] | None) -> ExecutionPermissionPlan | None:
    if data is None:
        return None
    return ExecutionPermissionPlan(
        actions=tuple(_parse_permission_plan_action(item) for item in data.get("actions", [])),
        policies={
            str(recipe_id): PermissionPolicy(
                on_failure=str(policy.get("on_failure", "warn")),
                require_all=bool(policy.get("require_all", False)),
            )
            for recipe_id, policy in data.get("policies", {}).items()
        },
    )


def _parse_permission_plan_action(data: Mapping[str, Any]) -> PermissionPlanAction:
    allowed_fields = {"status", "kind", "package_name", "source", "permission", "op", "desired_mode", "required", "reason"}
    unexpected_fields = sorted(str(field) for field in data.keys() if str(field) not in allowed_fields)
    if unexpected_fields:
        raise ValueError(f"Permission plan action contains unknown fields: {unexpected_fields}")
    return PermissionPlanAction(
        status=str(data["status"]),
        kind=str(data["kind"]),
        package_name=str(data["package_name"]),
        source=PermissionPlanSource(
            recipe_id=str(data["source"]["recipe_id"]),
            section=str(data["source"]["section"]),
        ),
        permission=_optional_str(data.get("permission")),
        op=_optional_str(data.get("op")),
        desired_mode=_optional_str(data.get("desired_mode")),
        required=bool(data.get("required", True)),
        reason=_parse_permission_plan_reason(data.get("reason")),
    )


def _parse_execution_artifact(data: Mapping[str, Any]) -> ExecutionArtifact:
    return ExecutionArtifact(
        id=str(data["id"]),
        type=ArtifactType(str(data["type"])),
        url=str(data["url"]),
        cache=ArtifactCacheMode(str(data.get("cache", ArtifactCacheMode.DEFAULT.value))),
    )


def _parse_runtime_value(data: Mapping[str, Any]) -> RuntimeValue:
    return RuntimeValue(
        type=RuntimeValueType(str(data["type"])),
        value=data.get("value"),
        location=str(data["location"]) if data.get("location") is not None else None,
    )


def _parse_param_value(value: Any):
    if isinstance(value, Mapping) and set(value.keys()) == {"ref"}:
        return RefParamValue(ref=str(value["ref"]))
    if isinstance(value, Mapping) and set(value.keys()) == {"value"}:
        return LiteralParamValue(value=value["value"])
    return LiteralParamValue(value=value)


def _parse_permission_plan_reason(data: Mapping[str, Any] | None) -> PermissionPlanReason | None:
    if data is None:
        return None
    return PermissionPlanReason(code=str(data["code"]), message=str(data["message"]))


def _optional_str(value: Any) -> str | None:
    if value is None:
        return None
    return str(value)

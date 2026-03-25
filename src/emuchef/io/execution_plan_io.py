"""Execution-plan loading from YAML files."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

from emuchef.domain import (
    DeviceContext,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    ResolvedInputValue,
    RuntimeCapabilities,
    StepCondition,
    StepType,
)

from .serde import load_yaml

PLANNER_ONLY_STEP_KEYS = {"selected", "user_toggleable", "availability", "reason", "dependencies", "constraints"}


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
        "inputs_resolved",
        "steps",
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
        inputs_resolved=tuple(
            ResolvedInputValue(id=str(item["id"]), value=item["value"]) for item in data.get("inputs_resolved", [])
        ),
        steps=tuple(_parse_execution_step(item) for item in data.get("steps", [])),
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
        params=dict(data.get("params", {})),
        skip_if=tuple(_parse_condition(item) for item in data.get("skip_if", [])),
        verify=tuple(_parse_condition(item) for item in data.get("verify", [])),
    )


def _parse_condition(data: Mapping[str, Any]) -> StepCondition:
    return StepCondition(type=str(data["type"]), params=dict(data.get("params", {})))

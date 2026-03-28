"""Execution plan models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_non_empty, ensure_unique
from .constants import SCHEMA_VERSION
from .device_context import DeviceContext
from .device_profiles import RuntimeCapabilities
from .param_values import JSONValue
from .step import StepCondition
from .step_types import StepType


@dataclass(frozen=True, slots=True)
class ExecutionPlanSource:
    device_profile_ref: str
    device_plan_ref: str
    selected_recipe_refs: tuple[str, ...] = ()
    expanded_recipe_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class PermissionPlanSource:
    recipe_id: str
    section: str

    def __post_init__(self) -> None:
        ensure_non_empty(self.recipe_id, "permission plan source recipe_id")
        ensure_non_empty(self.section, "permission plan source section")


@dataclass(frozen=True, slots=True)
class PermissionPlanReason:
    code: str
    message: str

    def __post_init__(self) -> None:
        ensure_non_empty(self.code, "permission plan reason code")
        ensure_non_empty(self.message, "permission plan reason message")


@dataclass(frozen=True, slots=True)
class PermissionPlanAction:
    status: Literal["applicable", "skipped", "manual_required"]
    kind: Literal["runtime_permission", "appop", "manual_requirement"]
    package_name: str
    source: PermissionPlanSource
    permission: str | None = None
    op: str | None = None
    desired_mode: str | None = None
    manual_type: str | None = None
    required: bool = True
    command: tuple[str, ...] = ()
    reason: PermissionPlanReason | None = None

    def __post_init__(self) -> None:
        if self.status not in {"applicable", "skipped", "manual_required"}:
            raise ValueError(f"Unsupported permission plan action status: {self.status!r}")
        if self.kind not in {"runtime_permission", "appop", "manual_requirement"}:
            raise ValueError(f"Unsupported permission plan action kind: {self.kind!r}")
        ensure_non_empty(self.package_name, "permission plan action package_name")
        if self.kind == "runtime_permission":
            if self.permission is None:
                raise ValueError("Runtime permission plan actions require permission")
            ensure_non_empty(self.permission, "permission plan action permission")
        if self.kind == "appop":
            if self.op is None or self.desired_mode is None:
                raise ValueError("App-op permission plan actions require op and desired_mode")
            ensure_non_empty(self.op, "permission plan action op")
            ensure_non_empty(self.desired_mode, "permission plan action desired_mode")
        if self.kind == "manual_requirement":
            if self.manual_type is None:
                raise ValueError("Manual permission plan actions require manual_type")
            ensure_non_empty(self.manual_type, "permission plan action manual_type")


@dataclass(frozen=True, slots=True)
class ExecutionPermissionPlan:
    actions: tuple[PermissionPlanAction, ...] = ()


@dataclass(frozen=True, slots=True)
class ResolvedInputValue:
    id: str
    value: JSONValue


@dataclass(frozen=True, slots=True)
class ExecutionStep:
    id: str
    recipe_ref: str
    type: StepType
    name: str
    params: Mapping[str, JSONValue] = field(default_factory=dict)
    skip_if: tuple[StepCondition, ...] = ()
    verify: tuple[StepCondition, ...] = ()


@dataclass(frozen=True, slots=True)
class ExecutionPlan:
    id: str
    source: ExecutionPlanSource
    device_context: DeviceContext
    runtime_capabilities: RuntimeCapabilities
    inputs_resolved: tuple[ResolvedInputValue, ...]
    steps: tuple[ExecutionStep, ...]
    permission_plan: ExecutionPermissionPlan | None = None
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["execution_plan"] = "execution_plan"

    def __post_init__(self) -> None:
        ensure_unique((item.id for item in self.inputs_resolved), "resolved input ids")
        ensure_unique((step.id for step in self.steps), "execution step ids")

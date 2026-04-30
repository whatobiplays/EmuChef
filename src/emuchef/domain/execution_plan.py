"""Execution plan models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_unique
from .artifacts import ArtifactCacheMode, ArtifactType
from .constants import SCHEMA_VERSION
from .device_context import DeviceContext
from .device_profiles import RuntimeCapabilities
from .param_values import ParamValue
from .runtime_state import RuntimeValue
from .step import StepCondition, StepConstraints
from .step_types import StepType


@dataclass(frozen=True, slots=True)
class ExecutionPlanSource:
    device_profile_ref: str
    device_plan_ref: str
    selected_recipe_refs: tuple[str, ...] = ()
    expanded_recipe_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class ExecutionInputValue:
    id: str
    value: RuntimeValue


@dataclass(frozen=True, slots=True)
class ExecutionArtifact:
    id: str
    type: ArtifactType
    url: str
    cache: ArtifactCacheMode = ArtifactCacheMode.DEFAULT


@dataclass(frozen=True, slots=True)
class ExecutionStep:
    id: str
    recipe_ref: str
    type: StepType
    name: str
    dependencies: tuple[str, ...] = ()
    constraints: StepConstraints = field(default_factory=StepConstraints)
    params: Mapping[str, ParamValue] = field(default_factory=dict)
    skip_if: tuple[StepCondition, ...] = ()
    verify: tuple[StepCondition, ...] = ()


@dataclass(frozen=True, slots=True)
class ExecutionPlan:
    id: str
    source: ExecutionPlanSource
    device_context: DeviceContext
    runtime_capabilities: RuntimeCapabilities
    inputs: tuple[ExecutionInputValue, ...]
    artifacts: tuple[ExecutionArtifact, ...]
    steps: tuple[ExecutionStep, ...]
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["execution_plan"] = "execution_plan"

    def __post_init__(self) -> None:
        ensure_unique((item.id for item in self.inputs), "execution input ids")
        ensure_unique((artifact.id for artifact in self.artifacts), "execution artifact ids")
        ensure_unique((step.id for step in self.steps), "execution step ids")

"""Draft plan models."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Literal

from ._validation import ensure_unique
from .constants import SCHEMA_VERSION
from .device_context import DeviceContext
from .device_profiles import RuntimeCapabilities
from .issues import AvailabilityReason, WarningMessage
from .param_values import JSONValue
from .step_types import StepTypeId


class Availability(str, Enum):
    AVAILABLE = "available"
    UNAVAILABLE = "unavailable"


@dataclass(frozen=True, slots=True)
class DraftPlanSource:
    device_profile_ref: str
    device_plan_ref: str
    selected_recipe_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class DraftRecipeState:
    id: str
    selected: bool
    auto_included: bool
    user_toggleable: bool
    availability: Availability
    reason: AvailabilityReason | None = None


@dataclass(frozen=True, slots=True)
class DraftStepState:
    id: str
    recipe_ref: str
    type: StepTypeId
    name: str
    selected: bool
    user_toggleable: bool
    availability: Availability
    reason: AvailabilityReason | None = None

    def __post_init__(self) -> None:
        if self.selected and self.availability is Availability.UNAVAILABLE:
            raise ValueError("Draft steps may not be selected when unavailable")


@dataclass(frozen=True, slots=True)
class DraftInputState:
    id: str
    label: str
    required: bool
    multiple: bool
    resolved: bool
    value: JSONValue = None
    required_by: tuple[str, ...] = ()
    description: str | None = None


@dataclass(frozen=True, slots=True)
class DraftPlan:
    id: str
    source: DraftPlanSource
    device_context: DeviceContext
    runtime_capabilities: RuntimeCapabilities
    recipes: tuple[DraftRecipeState, ...]
    steps: tuple[DraftStepState, ...]
    inputs: tuple[DraftInputState, ...]
    warnings: tuple[WarningMessage, ...] = ()
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["draft_plan"] = "draft_plan"

    def __post_init__(self) -> None:
        ensure_unique((recipe.id for recipe in self.recipes), "draft recipe ids")
        ensure_unique((step.id for step in self.steps), "draft step ids")
        ensure_unique((item.id for item in self.inputs), "draft input ids")

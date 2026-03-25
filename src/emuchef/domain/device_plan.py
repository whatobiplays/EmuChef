"""Device plan models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_unique
from .constants import SCHEMA_VERSION
from .param_values import JSONValue


@dataclass(frozen=True, slots=True)
class DevicePlanRecipeSelection:
    recipe_ref: str
    selected_by_default: bool


@dataclass(frozen=True, slots=True)
class DevicePlan:
    id: str
    name: str
    device_profile_ref: str
    recipes: tuple[DevicePlanRecipeSelection, ...]
    description: str | None = None
    defaults: Mapping[str, JSONValue] = field(default_factory=dict)
    overrides: Mapping[str, JSONValue] = field(default_factory=dict)
    metadata: Mapping[str, JSONValue] = field(default_factory=dict)
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["device_plan"] = "device_plan"

    def __post_init__(self) -> None:
        ensure_unique((item.recipe_ref for item in self.recipes), "device plan recipe refs")

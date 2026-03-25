"""Recipe step models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field

from ._validation import ensure_unique
from .param_values import AuthoredParamValue, JSONValue
from .step_types import StepType


@dataclass(frozen=True, slots=True)
class StepConstraints:
    capabilities: tuple[str, ...] = ()
    conflicts_with: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class StepCondition:
    type: str
    params: Mapping[str, JSONValue] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class Step:
    id: str
    type: StepType
    name: str
    user_toggleable: bool
    dependencies: tuple[str, ...]
    constraints: StepConstraints
    skip_if: tuple[StepCondition, ...]
    params: Mapping[str, AuthoredParamValue]
    verify: tuple[StepCondition, ...]
    description: str | None = None

    def __post_init__(self) -> None:
        ensure_unique(self.dependencies, "step dependency ids")

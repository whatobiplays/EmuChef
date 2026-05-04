"""Shared step spec models and registry-derived compatibility projections."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum

from .step_types import StepTypeId


class ParamMode(str, Enum):
    LITERAL = "literal"
    REF = "ref"


@dataclass(frozen=True, slots=True)
class ParamSpec:
    mode: ParamMode
    required: bool = True
    default: object | None = None
    enum_values: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class StepSpec:
    type_name: StepTypeId
    params: Mapping[str, ParamSpec]
    primary_output_name: str | None = None
    # Transitional metadata only. Runtime dispatch is owned by StepPlugin.handler.
    executor_handler: str | None = None


def _registry_step_specs() -> dict[StepTypeId, StepSpec]:
    from emuchef.steps import builtin_step_registry

    return dict(builtin_step_registry().step_specs)


STEP_SPECS: dict[StepTypeId, StepSpec] = _registry_step_specs()
"""Compatibility projection of specs from the built-in step plugin registry."""


PRIMARY_OUTPUT_STEP_TYPES = {
    step_type: spec.primary_output_name for step_type, spec in STEP_SPECS.items() if spec.primary_output_name is not None
}
"""Compatibility projection of primary output names from the built-in registry."""

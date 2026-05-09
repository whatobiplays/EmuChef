"""Shared step spec models and registry-derived compatibility projections."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum
from types import MappingProxyType

from .step_types import StepTypeId


class ParamMode(str, Enum):
    LITERAL = "literal"
    REF = "ref"


@dataclass(frozen=True, slots=True)
class ParamFieldSpec:
    """Schema metadata for one known field inside a structured param value."""

    kind: str
    required: bool = False
    enum_values: tuple[str, ...] = ()
    default: object | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", str(self.kind))
        object.__setattr__(self, "enum_values", tuple(str(value) for value in self.enum_values))


@dataclass(frozen=True, slots=True)
class ParamShapeSpec:
    """Generic schema metadata for structured step params.

    This metadata describes authored value shape only. Editor clients may use it
    to choose safer controls, but command application and validation remain owned
    by the Python domain model and sidecar session.
    """

    kind: str
    item_kind: str | None = None
    target: str | None = None
    ordered: bool = False
    unique: bool = False
    fields: Mapping[str, ParamFieldSpec] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", str(self.kind))
        if self.item_kind is not None:
            object.__setattr__(self, "item_kind", str(self.item_kind))
        if self.target is not None:
            object.__setattr__(self, "target", str(self.target))
        object.__setattr__(
            self,
            "fields",
            MappingProxyType({str(name): field_spec for name, field_spec in self.fields.items()}),
        )


@dataclass(frozen=True, slots=True)
class ParamSpec:
    mode: ParamMode
    required: bool = True
    default: object | None = None
    enum_values: tuple[str, ...] = ()
    shape: ParamShapeSpec | None = None


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

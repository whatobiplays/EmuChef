"""Runtime reference parsing utilities."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class RefKind(str, Enum):
    INPUT = "input"
    ARTIFACT_FIELD = "artifact_field"
    STEP_OUTPUT = "step_output"
    STEP_SHORTHAND = "step_shorthand"


@dataclass(frozen=True, slots=True)
class RuntimeRef:
    raw: str
    kind: RefKind
    target_id: str
    field: str | None = None


def parse_reference(value: str) -> RuntimeRef:
    if value.startswith("inputs."):
        target_id = value[len("inputs.") :]
        if not target_id:
            raise ValueError(f"Invalid reference: {value!r}")
        return RuntimeRef(raw=value, kind=RefKind.INPUT, target_id=target_id)

    if value.startswith("steps."):
        step_body = value[len("steps.") :]
        step_id, separator, output_name = step_body.partition(".outputs.")
        if separator:
            if not step_id or not output_name:
                raise ValueError(f"Invalid reference: {value!r}")
            return RuntimeRef(raw=value, kind=RefKind.STEP_OUTPUT, target_id=step_id, field=output_name)
        if not step_body:
            raise ValueError(f"Invalid reference: {value!r}")
        return RuntimeRef(raw=value, kind=RefKind.STEP_SHORTHAND, target_id=step_body)

    if value.startswith("artifacts."):
        body = value[len("artifacts.") :]
        artifact_id, separator, field = body.rpartition(".")
        if not separator or not artifact_id or not field:
            raise ValueError(f"Invalid reference: {value!r}")
        return RuntimeRef(raw=value, kind=RefKind.ARTIFACT_FIELD, target_id=artifact_id, field=field)

    raise ValueError(f"Invalid reference: {value!r}")

"""Helpers for step handlers that consume resolved runtime values."""

from __future__ import annotations

from emuchef.domain import RuntimeValue


def literal_string_list(value: object | None) -> list[str]:
    if value is None:
        return []
    if isinstance(value, RuntimeValue):
        value = value.value
    return [str(item) for item in list(value)]


def require_runtime_value(value: object) -> RuntimeValue:
    if not isinstance(value, RuntimeValue):
        raise ValueError(f"Expected a resolved runtime value, got: {value!r}")
    return value

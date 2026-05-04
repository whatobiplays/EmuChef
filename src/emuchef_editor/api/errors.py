"""Structured API errors for editor JSON callers."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import fields, is_dataclass
from enum import Enum
from pathlib import Path
from typing import Any


ERROR_CODES: tuple[str, ...] = (
    "unknown_document",
    "invalid_request",
    "invalid_command",
    "command_failed",
    "load_failed",
    "save_failed",
    "validation_failed",
    "internal_error",
)


class ApiError(Exception):
    """A stable, JSON-serializable API failure."""

    def __init__(self, code: str, message: str, details: Mapping[str, Any] | None = None) -> None:
        if code not in ERROR_CODES:
            raise ValueError(f"Unknown API error code: {code!r}.")
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = _json_safe(details or {})

    def to_dict(self) -> dict[str, Any]:
        return {
            "code": self.code,
            "message": self.message,
            "details": self.details,
        }


def _json_safe(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, Mapping):
        return {str(key): _json_safe(item) for key, item in value.items()}
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return [_json_safe(item) for item in value]
    if is_dataclass(value):
        return {field.name: _json_safe(getattr(value, field.name)) for field in fields(value)}
    return str(value)

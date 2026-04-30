"""Typed warning, error, and availability message payloads."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field

from .codes import AvailabilityCode, ErrorCode, WarningCode
from .param_values import JSONValue


@dataclass(frozen=True, slots=True)
class WarningMessage:
    code: WarningCode
    message: str
    details: Mapping[str, JSONValue] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ErrorMessage:
    code: ErrorCode
    message: str
    details: Mapping[str, JSONValue] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class AvailabilityReason:
    code: AvailabilityCode
    message: str
    details: Mapping[str, JSONValue] = field(default_factory=dict)

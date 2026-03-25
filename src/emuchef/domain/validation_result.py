"""Validation result models."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Literal

from .constants import SCHEMA_VERSION
from .issues import ErrorMessage, WarningMessage


class ValidationStatus(str, Enum):
    SUCCESS = "success"
    WARNING = "warning"
    ERROR = "error"


@dataclass(frozen=True, slots=True)
class ValidationResult:
    status: ValidationStatus
    warnings: tuple[WarningMessage, ...]
    errors: tuple[ErrorMessage, ...]
    validated_paths: tuple[str, ...]
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["validation_result"] = "validation_result"

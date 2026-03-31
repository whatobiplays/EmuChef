"""Input declaration models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum

from .param_values import JSONValue


class InputType(str, Enum):
    FILE = "file"
    DIRECTORY = "directory"


class InputRole(str, Enum):
    APK = "apk"
    BIOS = "bios"
    ROMS = "roms"
    CONFIG_BUNDLE = "config_bundle"
    GENERIC = "generic"


@dataclass(frozen=True, slots=True)
class InputValidation:
    must_exist: bool = False
    allowed_extensions: tuple[str, ...] = ()
    path_kind: InputType | None = None


@dataclass(frozen=True, slots=True)
class InputDeclaration:
    id: str
    type: InputType
    required: bool
    multiple: bool
    role: InputRole = InputRole.GENERIC
    label: str = ""
    validation: InputValidation = field(default_factory=InputValidation)
    description: str | None = None
    default: JSONValue = None
    metadata: Mapping[str, JSONValue] = field(default_factory=dict)

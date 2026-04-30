"""Device profile models and runtime capabilities."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_unique
from .constants import SCHEMA_VERSION
from .param_values import JSONValue


@dataclass(frozen=True, slots=True)
class AndroidVersionRange:
    min: int | None = None
    max: int | None = None


@dataclass(frozen=True, slots=True)
class DeviceMatchCriteria:
    manufacturer_contains: tuple[str, ...] = ()
    brand_contains: tuple[str, ...] = ()
    model_patterns: tuple[str, ...] = ()
    android_version: AndroidVersionRange | None = None


@dataclass(frozen=True, slots=True)
class RuntimeCapabilities:
    adb_available: bool
    apk_install: bool
    shared_storage_write: bool
    app_launch: bool
    shell_command: bool
    package_remove_for_user: bool
    root_shell: bool
    app_data_write: bool


@dataclass(frozen=True, slots=True)
class DeviceProfile:
    id: str
    name: str
    match: DeviceMatchCriteria
    capability_defaults: RuntimeCapabilities
    device_tags: tuple[str, ...]
    description: str | None = None
    metadata: Mapping[str, JSONValue] = field(default_factory=dict)
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["device_profile"] = "device_profile"

    def __post_init__(self) -> None:
        ensure_unique(self.device_tags, "device profile tags")

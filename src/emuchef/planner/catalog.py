"""Authored catalog models for planner and IO layers."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from emuchef.domain import (
    AppDefinition,
    ArtifactDefinition,
    DevicePlan,
    DeviceProfile,
    ErrorMessage,
    InputDeclaration,
    Recipe,
)


class CatalogLoadError(Exception):
    """Raised when authored data cannot be loaded into a valid catalog."""

    def __init__(self, errors: tuple[ErrorMessage, ...]) -> None:
        super().__init__("Authored data validation failed")
        self.errors = errors


@dataclass(frozen=True, slots=True)
class AuthoredCatalog:
    root_path: Path
    apps: Mapping[str, AppDefinition]
    recipes: Mapping[str, Recipe]
    device_profiles: Mapping[str, DeviceProfile]
    device_plans: Mapping[str, DevicePlan]
    binding_inputs: Mapping[str, InputDeclaration]
    recipe_artifacts: Mapping[str, ArtifactDefinition]

    @property
    def asset_root(self) -> Path:
        return self.root_path.parent

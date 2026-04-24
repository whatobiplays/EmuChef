"""Recipe definition models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_non_empty, ensure_ordered_range, ensure_unique
from .artifacts import ArtifactDefinition
from .constants import SCHEMA_VERSION
from .input_declaration import InputDeclaration
from .step import Step

PERMISSION_POLICY_ON_FAILURE_VALUES: tuple[str, ...] = ("warn", "fail")


@dataclass(frozen=True, slots=True)
class RecipeProvides:
    features: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class PermissionWhen:
    rooted: bool | None = None
    android_api_min: int | None = None
    android_api_max: int | None = None

    def __post_init__(self) -> None:
        ensure_ordered_range(self.android_api_min, self.android_api_max, "permission android api range")


@dataclass(frozen=True, slots=True)
class RuntimePermissionGrant:
    package_name: str
    name: str
    required: bool = True
    when: PermissionWhen | None = None

    def __post_init__(self) -> None:
        ensure_non_empty(self.package_name, "runtime permission package_name")
        ensure_non_empty(self.name, "runtime permission name")


@dataclass(frozen=True, slots=True)
class AppOpGrant:
    package_name: str
    op: str
    mode: str
    required: bool = True
    when: PermissionWhen | None = None

    def __post_init__(self) -> None:
        ensure_non_empty(self.package_name, "appop package_name")
        ensure_non_empty(self.op, "appop op")
        ensure_non_empty(self.mode, "appop mode")


@dataclass(frozen=True, slots=True)
class PermissionPolicy:
    on_failure: str = "warn"
    require_all: bool = False

    def __post_init__(self) -> None:
        ensure_non_empty(self.on_failure, "permission policy on_failure")


@dataclass(frozen=True, slots=True)
class PermissionSet:
    runtime: tuple[RuntimePermissionGrant, ...] = ()
    appops: tuple[AppOpGrant, ...] = ()
    policy: PermissionPolicy = field(default_factory=PermissionPolicy)


@dataclass(frozen=True, slots=True)
class Recipe:
    id: str
    name: str
    recipe_dependencies: tuple[str, ...]
    provides: RecipeProvides
    inputs: Mapping[str, InputDeclaration]
    steps: tuple[Step, ...]
    artifacts: Mapping[str, ArtifactDefinition] = field(default_factory=dict)
    artifact_groups: Mapping[str, tuple[str, ...]] = field(default_factory=dict)
    permissions: PermissionSet = field(default_factory=PermissionSet)
    description: str | None = None
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["recipe"] = "recipe"

    def __post_init__(self) -> None:
        ensure_unique(self.recipe_dependencies, "recipe dependency refs")
        ensure_unique(self.inputs.keys(), "recipe input ids")
        ensure_unique(self.artifacts.keys(), "recipe artifact ids")
        ensure_unique(self.artifact_groups.keys(), "recipe artifact group ids")
        ensure_unique((step.id for step in self.steps), "recipe step ids")

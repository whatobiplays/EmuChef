"""Recipe definition models."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_known, ensure_non_empty, ensure_ordered_range, ensure_unique
from .constants import SCHEMA_VERSION
from .input_declaration import InputDeclaration
from .step import Step


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
class ManualPermissionRequirement:
    package_name: str
    manual_type: str
    reason: str
    required: bool = True
    when: PermissionWhen | None = None

    def __post_init__(self) -> None:
        ensure_non_empty(self.package_name, "manual permission package_name")
        ensure_non_empty(self.manual_type, "manual permission manual_type")
        ensure_non_empty(self.reason, "manual permission reason")


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
    manual: tuple[ManualPermissionRequirement, ...] = ()
    policy: PermissionPolicy = field(default_factory=PermissionPolicy)


@dataclass(frozen=True, slots=True)
class Recipe:
    id: str
    name: str
    recipe_dependencies: tuple[str, ...]
    provides: RecipeProvides
    inputs: tuple[InputDeclaration, ...]
    steps: tuple[Step, ...]
    permissions: PermissionSet = field(default_factory=PermissionSet)
    description: str | None = None
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["recipe"] = "recipe"

    def __post_init__(self) -> None:
        ensure_unique(self.recipe_dependencies, "recipe dependency refs")
        ensure_unique((item.id for item in self.inputs), "recipe input ids")
        ensure_unique((step.id for step in self.steps), "recipe step ids")
        step_ids = {step.id for step in self.steps}
        for step in self.steps:
            ensure_known(step.dependencies, step_ids, "step dependencies")

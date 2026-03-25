"""App definition models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Literal

from ._validation import ensure_unique
from .constants import SCHEMA_VERSION
from .input_declaration import InputDeclaration
from .param_values import JSONValue


@dataclass(frozen=True, slots=True)
class AppPackage:
    primary: str
    aliases: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class AppInstallSource:
    type: str
    resolver: str
    options: Mapping[str, JSONValue] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class AppTrackingSource:
    type: str
    config_snapshot: str
    app_id: str


@dataclass(frozen=True, slots=True)
class AppArtifactSupport:
    required: bool | None = None
    supported: bool | None = None


@dataclass(frozen=True, slots=True)
class AppArtifacts:
    apk: AppArtifactSupport
    shared_storage_config: AppArtifactSupport | None = None
    app_data_config: AppArtifactSupport | None = None
    byo_apk: AppArtifactSupport | None = None


@dataclass(frozen=True, slots=True)
class AppConfigTarget:
    id: str
    type: str
    path: str


@dataclass(frozen=True, slots=True)
class AppProvisioning:
    launch_once_recommended: bool = False
    shared_storage_paths: tuple[str, ...] = ()
    app_data_paths: tuple[str, ...] = ()
    config_targets: tuple[AppConfigTarget, ...] = ()

    def __post_init__(self) -> None:
        ensure_unique((target.id for target in self.config_targets), "app config target ids")


@dataclass(frozen=True, slots=True)
class AppDefinition:
    id: str
    name: str
    package: AppPackage
    install_source: AppInstallSource
    tracking_source: AppTrackingSource
    artifacts: AppArtifacts
    provisioning: AppProvisioning
    inputs: tuple[InputDeclaration, ...]
    description: str | None = None
    category: str | None = None
    metadata: Mapping[str, JSONValue] = field(default_factory=dict)
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["app_definition"] = "app_definition"

    def __post_init__(self) -> None:
        ensure_unique((item.id for item in self.inputs), "app input ids")

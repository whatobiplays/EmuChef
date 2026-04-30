"""Executor runtime state models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum
from typing import Literal, TypeAlias

from .param_values import JSONValue


class RuntimeValueType(str, Enum):
    FILE_PATH = "file_path"
    DIRECTORY_PATH = "directory_path"
    PATH_LIST = "path_list"
    STRING = "string"
    INTEGER = "integer"
    BOOLEAN = "boolean"
    OBJECT = "object"
    NULL = "null"


@dataclass(frozen=True, slots=True)
class RuntimeValue:
    type: RuntimeValueType
    value: JSONValue
    location: Literal["host", "device"] | None = None


class ArtifactRuntimeStatus(str, Enum):
    PENDING = "pending"
    RESOLVED = "resolved"
    FAILED = "failed"


@dataclass(slots=True)
class ArtifactRuntimeState:
    artifact_id: str
    status: ArtifactRuntimeStatus = ArtifactRuntimeStatus.PENDING
    local_path: str | None = None
    resolved_url: str | None = None
    filename: str | None = None
    cache_hit: bool = False
    error: str | None = None


class StepRuntimeStatus(str, Enum):
    PENDING = "pending"
    SKIPPED = "skipped"
    BLOCKED = "blocked"
    SUCCEEDED = "succeeded"
    FAILED = "failed"


@dataclass(slots=True)
class StepRuntimeState:
    step_id: str
    status: StepRuntimeStatus = StepRuntimeStatus.PENDING
    outputs: dict[str, RuntimeValue] = field(default_factory=dict)
    error: str | None = None


RuntimeValuePrimitive: TypeAlias = RuntimeValue | JSONValue


@dataclass(slots=True)
class ExecutionState:
    inputs: dict[str, RuntimeValue]
    artifacts: dict[str, ArtifactRuntimeState] = field(default_factory=dict)
    steps: dict[str, StepRuntimeState] = field(default_factory=dict)

"""Artifact definition models."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Literal, TypeAlias

from ._validation import ensure_non_empty


class ArtifactType(str, Enum):
    REMOTE_FILE = "remote_file"


class ArtifactCacheMode(str, Enum):
    DEFAULT = "default"
    NONE = "none"


@dataclass(frozen=True, slots=True)
class RemoteFileArtifact:
    id: str
    url: str
    cache: ArtifactCacheMode = ArtifactCacheMode.DEFAULT
    type: Literal[ArtifactType.REMOTE_FILE] = ArtifactType.REMOTE_FILE

    def __post_init__(self) -> None:
        ensure_non_empty(self.id, "artifact id")
        ensure_non_empty(self.url, "artifact url")


ArtifactDefinition: TypeAlias = RemoteFileArtifact

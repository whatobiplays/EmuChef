"""Serialization helpers for dataclass-based YAML IO."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import fields, is_dataclass
from enum import Enum
from pathlib import Path
from typing import Any

import yaml


def to_primitive(value: Any) -> Any:
    if is_dataclass(value):
        return {field.name: to_primitive(getattr(value, field.name)) for field in fields(value)}
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, Mapping):
        return {str(key): to_primitive(item) for key, item in value.items()}
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return [to_primitive(item) for item in value]
    return value


def dump_yaml(value: Any, path: str | Path | None = None) -> str:
    payload = yaml.safe_dump(to_primitive(value), sort_keys=False)
    if path is not None:
        Path(path).write_text(payload, encoding="utf-8")
    return payload


def load_yaml(path: str | Path) -> Any:
    return yaml.safe_load(Path(path).read_text(encoding="utf-8"))

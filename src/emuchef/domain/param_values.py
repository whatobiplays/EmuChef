"""Param value models for authored and resolved params."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TypeAlias

from .refs import Reference

ScalarValue: TypeAlias = str | int | float | bool | None
JSONValue: TypeAlias = ScalarValue | list["JSONValue"] | dict[str, "JSONValue"]


@dataclass(frozen=True, slots=True)
class LiteralParamValue:
    value: JSONValue


@dataclass(frozen=True, slots=True)
class BoundParamValue:
    ref: Reference


ParamValue: TypeAlias = LiteralParamValue | BoundParamValue
AuthoredParamValue: TypeAlias = JSONValue | ParamValue

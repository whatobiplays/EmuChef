"""Param value models for authored and execution-plan params."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TypeAlias

ScalarValue: TypeAlias = str | int | float | bool | None
JSONValue: TypeAlias = ScalarValue | list["JSONValue"] | dict[str, "JSONValue"]


@dataclass(frozen=True, slots=True)
class LiteralParamValue:
    value: JSONValue


@dataclass(frozen=True, slots=True)
class RefParamValue:
    ref: str


BoundParamValue = RefParamValue
ParamValue: TypeAlias = LiteralParamValue | RefParamValue
AuthoredParamValue: TypeAlias = JSONValue | ParamValue

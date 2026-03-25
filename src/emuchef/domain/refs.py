"""Reference parsing utilities."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class Reference:
    scope: str
    name: str

    @property
    def full(self) -> str:
        return f"{self.scope}.${self.name}"

    def __str__(self) -> str:
        return self.full


def parse_reference(value: str) -> Reference:
    scope, separator, name = value.partition(".$")
    if not separator or not scope or not name:
        raise ValueError(f"Invalid reference: {value!r}")
    return Reference(scope=scope, name=name)

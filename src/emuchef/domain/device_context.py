"""Detected device context models."""

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DeviceContext:
    manufacturer: str
    model: str
    android_version: int
    device_tags: tuple[str, ...] = ()

"""Detected device context models."""

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DeviceContext:
    manufacturer: str
    model: str
    android_version: int
    android_api_level: int | None = None
    device_tags: tuple[str, ...] = ()

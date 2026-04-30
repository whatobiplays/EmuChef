"""Reusable device-profile matching helpers."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Iterable

from emuchef.domain import DeviceProfile


@dataclass(frozen=True, slots=True)
class ProfileMatchFacts:
    manufacturer: str
    brand: str
    model: str
    android_version: int


@dataclass(frozen=True, slots=True)
class ProfileMatchResult:
    profile_id: str
    profile_name: str
    matched: bool
    reasons: tuple[str, ...]


def match_device_profile(profile: DeviceProfile, facts: ProfileMatchFacts) -> ProfileMatchResult:
    reasons: list[str] = []
    matched = True

    manufacturer = facts.manufacturer.casefold()
    if profile.match.manufacturer_contains:
        expected = tuple(profile.match.manufacturer_contains)
        if any(token.casefold() in manufacturer for token in expected):
            reasons.append(f"manufacturer matched one of: {', '.join(expected)}")
        else:
            matched = False
            reasons.append(
                f"manufacturer {facts.manufacturer!r} did not contain any of: {', '.join(expected)}"
            )

    brand = facts.brand.casefold()
    if profile.match.brand_contains:
        expected = tuple(profile.match.brand_contains)
        if any(token.casefold() in brand for token in expected):
            reasons.append(f"brand matched one of: {', '.join(expected)}")
        else:
            matched = False
            reasons.append(f"brand {facts.brand!r} did not contain any of: {', '.join(expected)}")

    if profile.match.model_patterns:
        patterns = tuple(profile.match.model_patterns)
        if any(re.search(pattern, facts.model) for pattern in patterns):
            reasons.append(f"model matched one of: {', '.join(patterns)}")
        else:
            matched = False
            reasons.append(f"model {facts.model!r} did not match any of: {', '.join(patterns)}")

    minimum_android = profile.match.android_version.min if profile.match.android_version is not None else None
    if minimum_android is not None:
        if facts.android_version >= minimum_android:
            reasons.append(f"android version {facts.android_version} met minimum {minimum_android}")
        else:
            matched = False
            reasons.append(f"android version {facts.android_version} was below minimum {minimum_android}")

    return ProfileMatchResult(
        profile_id=profile.id,
        profile_name=profile.name,
        matched=matched,
        reasons=tuple(reasons),
    )


def match_device_profiles(
    profiles: Iterable[DeviceProfile],
    facts: ProfileMatchFacts,
) -> tuple[ProfileMatchResult, ...]:
    return tuple(sorted((match_device_profile(profile, facts) for profile in profiles), key=lambda item: item.profile_id))

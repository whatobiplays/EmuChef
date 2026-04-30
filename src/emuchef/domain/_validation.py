"""Internal helpers for structural validation."""

from collections.abc import Iterable


def ensure_non_empty(value: str, label: str) -> None:
    if not value.strip():
        raise ValueError(f"{label} must not be empty")


def ensure_ordered_range(min_value: int | None, max_value: int | None, label: str) -> None:
    if min_value is not None and max_value is not None and min_value > max_value:
        raise ValueError(f"Invalid {label}: min {min_value} exceeds max {max_value}")


def ensure_unique(values: Iterable[str], label: str) -> None:
    seen: set[str] = set()
    duplicates: set[str] = set()

    for value in values:
        if value in seen:
            duplicates.add(value)
        else:
            seen.add(value)

    if duplicates:
        joined = ", ".join(sorted(duplicates))
        raise ValueError(f"Duplicate {label}: {joined}")


def ensure_known(values: Iterable[str], known: set[str], label: str) -> None:
    unknown = sorted({value for value in values if value not in known})
    if unknown:
        joined = ", ".join(unknown)
        raise ValueError(f"Unknown {label}: {joined}")

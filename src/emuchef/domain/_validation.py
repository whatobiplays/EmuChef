"""Internal helpers for structural validation."""

from collections.abc import Iterable


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

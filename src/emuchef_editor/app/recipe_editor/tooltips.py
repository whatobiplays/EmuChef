"""Shared tooltip copy for recipe editor field surfaces."""

from __future__ import annotations

from emuchef.domain import ArtifactCacheMode, InputRole, InputType
from emuchef.domain.constants import SCHEMA_VERSION


def _enum_values(*values: str) -> str:
    """Render an accepted-values list for tooltip copy."""

    return ", ".join(values)


FIELD_TOOLTIPS: dict[str, str] = {
    "overview.id": (
        "Use a stable authored recipe id such as 'app.retroarch.provision'. "
        "Changing it updates YAML content only."
    ),
    "overview.name": "Human-readable recipe name shown in the editor and authored YAML.",
    "overview.description": "Optional free-form description for authors and maintainers.",
    "overview.kind": (
        "This identifies the authored document type. "
        "It stays read-only in Milestone 2 because the editor only supports recipe authoring."
    ),
    "overview.schema_version": (
        f"This is the authored recipe schema version. "
        f"It stays read-only in Milestone 2 because the editor supports only the latest schema, {SCHEMA_VERSION}."
    ),
    "overview.recipe_dependencies": (
        "Ordered list of recipe ids this recipe depends on. Each entry must match another authored recipe id."
    ),
    "overview.provides_features": (
        "Ordered list of feature names this recipe provides. Use stable strings other recipes can reference."
    ),
    "inputs.id": (
        "This stable authored id is used by refs such as 'inputs.<id>'. "
        "It stays read-only in Milestone 2 because rename with ref rewrite is not implemented yet."
    ),
    "inputs.type": (
        "Path shape accepted for this input. "
        f"Accepted values: {_enum_values(*(value.value for value in InputType))}."
    ),
    "inputs.role": (
        "Optional usage hint for the input. "
        f"Accepted values: {_enum_values(*(value.value for value in InputRole))}."
    ),
    "inputs.label": "Human-readable label shown for this input in editor and validation surfaces.",
    "inputs.description": "Optional author-facing description of what this input should contain.",
    "inputs.required": "When enabled, the input must have at least one bound value before planning can succeed.",
    "inputs.multiple": "When enabled, the input accepts multiple path values instead of a single path.",
    "inputs.must_exist": "When enabled, each bound path must already exist on disk.",
    "inputs.allowed_extensions": (
        "Optional comma-separated file extensions such as 'zip, cfg'. "
        "Extensions are checked when a bound path has a suffix."
    ),
    "inputs.path_kind": (
        "Optional bound-path override. "
        f"Accepted values: {_enum_values(*(value.value for value in InputType))}, or no override. "
        "If unset, the input type is used."
    ),
    "inputs.default": (
        "This is the authored default value for the input. "
        "It stays read-only in Milestone 2 because default editing is out of scope."
    ),
    "inputs.metadata": (
        "This is the authored metadata map attached to the input. "
        "It stays read-only in Milestone 2 because metadata editing is out of scope."
    ),
    "artifacts.id": (
        "This stable authored id is used by refs such as 'artifacts.<id>.<field>'. "
        "It stays read-only in Milestone 2 because rename with ref rewrite is not implemented yet."
    ),
    "artifacts.kind": (
        "This identifies how the artifact is authored and resolved. "
        "It stays read-only in Milestone 2 because artifact editing currently supports only 'remote_file'."
    ),
    "artifacts.url": "Remote file URL to resolve and download for this artifact.",
    "artifacts.cache": (
        "Artifact download caching mode. "
        f"Accepted values: {_enum_values(*(value.value for value in ArtifactCacheMode))}."
    ),
    "artifact_groups.id": (
        "This stable authored id names the artifact group. "
        "It stays read-only in Milestone 2 because rename with membership and ref updates is not implemented yet."
    ),
}


PROMPT_TOOLTIPS: dict[str, str] = {
    "inputs.id": (
        "Choose a stable authored id for this input. "
        "It becomes read-only after creation because rename with ref rewrite is not implemented in Milestone 2."
    ),
    "artifacts.id": (
        "Choose a stable authored id for this artifact. "
        "It becomes read-only after creation because rename with ref rewrite is not implemented in Milestone 2."
    ),
    "artifacts.url": "Enter the remote file URL to download for this artifact.",
    "artifact_groups.id": (
        "Choose a stable authored id for this artifact group. "
        "It becomes read-only after creation because rename support is not implemented in Milestone 2."
    ),
    "overview.recipe_dependencies": (
        "Enter another authored recipe id this recipe depends on. "
        "Use a value such as 'base.android.permissions'."
    ),
    "overview.provides_features": (
        "Enter a stable feature string this recipe provides. "
        "Other recipes or tooling may reference it."
    ),
}


def _normalize_tooltip(value: str | None) -> str | None:
    """Return a stripped tooltip string, or none when the entry is missing or blank."""

    if value is None:
        return None
    normalized = value.strip()
    return normalized or None


def field_tooltip(key: str) -> str | None:
    """Return tooltip text for a persistent editor field."""

    return _normalize_tooltip(FIELD_TOOLTIPS.get(key))


def prompt_tooltip(key: str) -> str | None:
    """Return tooltip text for a creation-time prompt field."""

    return _normalize_tooltip(PROMPT_TOOLTIPS.get(key))

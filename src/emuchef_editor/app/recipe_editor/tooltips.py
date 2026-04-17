"""Shared tooltip copy for recipe editor field surfaces."""

from __future__ import annotations

from emuchef.domain import ArtifactCacheMode, InputRole, InputType
from emuchef.domain.constants import SCHEMA_VERSION


def _enum_values(*values: str) -> str:
    """Render an accepted-values list for tooltip copy."""

    return ", ".join(values)


FIELD_TOOLTIPS: dict[str, str] = {
    "overview.id": (
        "Unique authored recipe id. Use a stable dotted id such as "
        "'app.retroarch.provision'. Changing it updates YAML content only."
    ),
    "overview.name": "Human-readable recipe name shown in the editor and authored YAML.",
    "overview.description": "Optional free-form description for recipe authors and maintainers.",
    "overview.kind": "Read-only. Authored recipes always use the kind 'recipe'.",
    "overview.schema_version": (
        f"Read-only. The editor supports the latest authored recipe schema only. "
        f"Current value: {SCHEMA_VERSION}."
    ),
    "overview.recipe_dependencies": (
        "Ordered list of recipe ids this recipe depends on. Each entry must match another authored recipe id."
    ),
    "overview.provides_features": (
        "Ordered list of feature names this recipe provides. Use stable strings other recipes can reference."
    ),
    "inputs.id": (
        "Read-only after creation. Unique input id used by authored refs such as 'inputs.<id>'."
    ),
    "inputs.type": (
        "Input path type. Accepted values: "
        f"{_enum_values(*(value.value for value in InputType))}."
    ),
    "inputs.role": (
        "Optional usage hint for the input. Accepted values: "
        f"{_enum_values(*(value.value for value in InputRole))}."
    ),
    "inputs.label": "Human-readable label shown for this input in editor and validation surfaces.",
    "inputs.description": "Optional author-facing description of what the input should contain.",
    "inputs.required": "When enabled, the input must have at least one bound value before planning can succeed.",
    "inputs.multiple": "When enabled, the input accepts multiple path values instead of a single path.",
    "inputs.must_exist": "When enabled, each bound path must already exist on disk.",
    "inputs.allowed_extensions": (
        "Optional comma-separated file extensions such as 'zip, cfg'. "
        "Extensions are checked when a bound path has a suffix."
    ),
    "inputs.path_kind": (
        "Optional bound-path override. Accepted values: "
        f"{_enum_values(*(value.value for value in InputType))}, or no override. "
        "If unset, the input type is used."
    ),
    "inputs.default": "Read-only. Optional default value loaded from authored YAML and preserved on save.",
    "inputs.metadata": "Read-only. Optional authored metadata map preserved on save.",
    "artifacts.id": (
        "Read-only after creation. Unique artifact id used by authored refs such as 'artifacts.<id>.<field>'."
    ),
    "artifacts.kind": "Read-only, currently supports only 'remote_file'.",
    "artifacts.url": "Remote file URL to resolve and download for this artifact.",
    "artifacts.cache": (
        "Artifact download caching mode. Accepted values: "
        f"{_enum_values(*(value.value for value in ArtifactCacheMode))}."
    ),
    "artifact_groups.id": (
        "Read-only after creation. Unique artifact group id. Group order and member order are preserved in YAML."
    ),
}


PROMPT_TOOLTIPS: dict[str, str] = {
    "input_id": (
        "Unique input id. It becomes read-only after creation and is referenced as 'inputs.<id>' in authored YAML."
    ),
    "artifact_id": (
        "Unique artifact id. It becomes read-only after creation and is referenced as 'artifacts.<id>.<field>'."
    ),
    "artifact_url": "Remote file URL to download for this artifact.",
    "artifact_group_id": (
        "Unique artifact group id. Group order and membership order are preserved in authored YAML."
    ),
    "recipe_dependency": (
        "Recipe id this recipe depends on. Use another authored recipe id such as 'base.android.permissions'."
    ),
    "provided_feature": (
        "Feature name this recipe provides. Use a stable string that other recipes or tooling can reference."
    ),
}


def field_tooltip(key: str) -> str:
    """Return tooltip text for a persistent editor field."""

    return FIELD_TOOLTIPS[key]


def prompt_tooltip(key: str) -> str:
    """Return tooltip text for a creation-time prompt field."""

    return PROMPT_TOOLTIPS[key]

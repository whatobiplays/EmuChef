"""Shared UI metadata for the Milestone 4 step editor."""

from __future__ import annotations

from dataclasses import fields

from emuchef.domain import RuntimeCapabilities, RuntimeValueType
from emuchef.steps import builtin_step_registry

SUPPORTED_EDITOR_STEP_TYPES = tuple(
    plugin.type for plugin in builtin_step_registry().plugins if plugin.editor.supported
)

SUPPORTED_CONDITION_TYPES: tuple[str, ...] = (
    "path_exists",
    "file_exists",
    "package_installed",
)

CONDITION_PARAM_FIELD: dict[str, tuple[str, str]] = {
    "path_exists": ("path", "Path"),
    "file_exists": ("path", "Path"),
    "package_installed": ("package_name", "Package"),
}

KNOWN_CAPABILITIES: tuple[str, ...] = tuple(field.name for field in fields(RuntimeCapabilities))

REF_VALUE_FILTERS: dict[tuple[object, str], tuple[RuntimeValueType, ...]] = {
    (plugin.type, param_name): allowed_types
    for plugin in builtin_step_registry().plugins
    for param_name, allowed_types in plugin.editor.ref_filters.items()
}

COPY_FILES_HELP = (
    "Directory and path-list sources treat dest as a destination directory. "
    "A file source copied into an existing directory lands at dest/<basename>. "
    "Otherwise dest is the exact target path. App-private destinations under "
    "/data/user/ and /data/data/ require privileged app-data writes."
)

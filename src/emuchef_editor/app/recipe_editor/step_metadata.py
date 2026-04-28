"""Shared UI metadata for the Milestone 4 step editor."""

from __future__ import annotations

from dataclasses import fields

from emuchef.domain import RuntimeCapabilities, RuntimeValueType, StepType

SUPPORTED_EDITOR_STEP_TYPES: tuple[StepType, ...] = (
    StepType.RESOLVE_ARTIFACTS,
    StepType.EXTRACT_ARTIFACTS,
    StepType.EXTRACT_ARCHIVE,
    StepType.COPY_FILES,
    StepType.INSTALL_APK,
    StepType.GRANT_PERMISSIONS,
    StepType.LAUNCH_APP,
    StepType.WAIT,
    StepType.FORCE_STOP_APP,
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

REF_VALUE_FILTERS: dict[tuple[StepType, str], tuple[RuntimeValueType, ...]] = {
    (StepType.EXTRACT_ARCHIVE, "archive"): (RuntimeValueType.FILE_PATH,),
    (
        StepType.COPY_FILES,
        "source",
    ): (
        RuntimeValueType.FILE_PATH,
        RuntimeValueType.DIRECTORY_PATH,
        RuntimeValueType.PATH_LIST,
    ),
    (StepType.INSTALL_APK, "app"): (RuntimeValueType.FILE_PATH,),
}

COPY_FILES_HELP = (
    "Directory and path-list sources treat dest as a destination directory. "
    "A file source copied into an existing directory lands at dest/<basename>. "
    "Otherwise dest is the exact target path. App-private destinations under "
    "/data/user/ and /data/data/ require privileged app-data writes."
)

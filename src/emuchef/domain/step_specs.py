"""Shared step registry and param specs."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum

from .copy_policy import CopyPolicy
from .step_types import StepType


class ParamMode(str, Enum):
    LITERAL = "literal"
    REF = "ref"


@dataclass(frozen=True, slots=True)
class ParamSpec:
    mode: ParamMode
    required: bool = True
    default: object | None = None
    enum_values: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class StepSpec:
    type_name: StepType
    params: Mapping[str, ParamSpec]
    primary_output_name: str | None = None
    executor_handler: str | None = None


STEP_SPECS: dict[StepType, StepSpec] = {
    StepType.RESOLVE_ARTIFACTS: StepSpec(
        type_name=StepType.RESOLVE_ARTIFACTS,
        params={
            "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
            "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
        },
        executor_handler="resolve_artifacts",
    ),
    StepType.EXTRACT_ARTIFACTS: StepSpec(
        type_name=StepType.EXTRACT_ARTIFACTS,
        params={
            "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
            "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
            "extract_on": ParamSpec(ParamMode.LITERAL, required=False, default="host", enum_values=("host", "device")),
        },
        primary_output_name="extracted_paths",
        executor_handler="extract_artifacts",
    ),
    StepType.EXTRACT_ARCHIVE: StepSpec(
        type_name=StepType.EXTRACT_ARCHIVE,
        params={
            "archive": ParamSpec(ParamMode.REF),
            "extract_on": ParamSpec(ParamMode.LITERAL, required=False, default="host", enum_values=("host", "device")),
            "dest": ParamSpec(ParamMode.LITERAL, required=False),
            "device_temp_path": ParamSpec(ParamMode.LITERAL, required=False),
            "cleanup": ParamSpec(ParamMode.LITERAL, required=False, default=True),
        },
        primary_output_name="extracted_path",
        executor_handler="extract_archive",
    ),
    StepType.COPY_FILES: StepSpec(
        type_name=StepType.COPY_FILES,
        params={
            "source": ParamSpec(ParamMode.REF),
            "dest": ParamSpec(ParamMode.LITERAL),
            "copy_policy": ParamSpec(
                ParamMode.LITERAL,
                required=False,
                default=CopyPolicy.MERGE.value,
                enum_values=tuple(policy.value for policy in CopyPolicy),
            ),
        },
        primary_output_name="copied_paths",
        executor_handler="copy_files",
    ),
    StepType.INSTALL_APK: StepSpec(
        type_name=StepType.INSTALL_APK,
        params={
            "app": ParamSpec(ParamMode.REF),
            "replace_existing": ParamSpec(ParamMode.LITERAL, required=False, default=False),
        },
        executor_handler="install_apk",
    ),
    StepType.GRANT_PERMISSIONS: StepSpec(
        type_name=StepType.GRANT_PERMISSIONS,
        params={
            "runtime": ParamSpec(ParamMode.LITERAL, required=False),
            "appops": ParamSpec(ParamMode.LITERAL, required=False),
            "policy": ParamSpec(ParamMode.LITERAL, required=False),
        },
        executor_handler="grant_permissions",
    ),
    StepType.LAUNCH_APP: StepSpec(
        type_name=StepType.LAUNCH_APP,
        params={
            "package_name": ParamSpec(ParamMode.LITERAL),
            "activity": ParamSpec(ParamMode.LITERAL, required=False),
        },
        executor_handler="launch_app",
    ),
    StepType.WAIT: StepSpec(
        type_name=StepType.WAIT,
        params={
            "duration_ms": ParamSpec(ParamMode.LITERAL),
        },
        executor_handler="wait",
    ),
    StepType.FORCE_STOP_APP: StepSpec(
        type_name=StepType.FORCE_STOP_APP,
        params={
            "package_name": ParamSpec(ParamMode.LITERAL),
        },
        executor_handler="force_stop_app",
    ),
}


PRIMARY_OUTPUT_STEP_TYPES = {
    step_type: spec.primary_output_name for step_type, spec in STEP_SPECS.items() if spec.primary_output_name is not None
}

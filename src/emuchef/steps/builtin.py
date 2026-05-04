"""Built-in step plugin registrations."""

from __future__ import annotations

from functools import lru_cache

from emuchef.domain.copy_policy import CopyPolicy
from emuchef.domain.runtime_state import RuntimeValueType
from emuchef.domain.step_specs import ParamMode, ParamSpec, StepSpec

from .contracts import StepEditorMetadata, StepOutputMetadata, StepPlugin, StepRegistry
from .handlers import (
    copy_files,
    extract_archive,
    extract_artifacts,
    force_stop_app,
    grant_permissions,
    install_apk,
    launch_app,
    resolve_artifacts,
    wait,
)
from .planner_hooks import (
    normalize_artifact_selection,
    validate_artifact_selection,
    validate_copy_files,
    validate_extract_archive,
    validate_grant_permissions,
    validate_package_name,
    validate_wait,
)


BUILTIN_STEP_PLUGINS: tuple[StepPlugin, ...] = (
    StepPlugin(
        type="resolve_artifacts",
        spec=StepSpec(
            type_name="resolve_artifacts",
            params={
                "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
                "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="resolve_artifacts",
        ),
        handler=resolve_artifacts.handle,
        normalize=normalize_artifact_selection,
        validate=validate_artifact_selection,
        editor=StepEditorMetadata(
            label="Resolve Artifacts",
            param_order=("artifacts", "artifact_groups"),
            tooltip_key_prefix="steps.resolve_artifacts",
        ),
    ),
    StepPlugin(
        type="extract_artifacts",
        spec=StepSpec(
            type_name="extract_artifacts",
            params={
                "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
                "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
                "extract_on": ParamSpec(ParamMode.LITERAL, required=False, default="host", enum_values=("host", "device")),
            },
            primary_output_name="extracted_paths",
            executor_handler="extract_artifacts",
        ),
        handler=extract_artifacts.handle,
        normalize=normalize_artifact_selection,
        validate=validate_artifact_selection,
        outputs=(StepOutputMetadata("extracted_paths", RuntimeValueType.PATH_LIST, primary=True),),
        editor=StepEditorMetadata(
            label="Extract Artifacts",
            param_order=("artifacts", "artifact_groups", "extract_on"),
            tooltip_key_prefix="steps.extract_artifacts",
        ),
    ),
    StepPlugin(
        type="extract_archive",
        spec=StepSpec(
            type_name="extract_archive",
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
        handler=extract_archive.handle,
        validate=validate_extract_archive,
        outputs=(StepOutputMetadata("extracted_path", RuntimeValueType.DIRECTORY_PATH, primary=True),),
        editor=StepEditorMetadata(
            label="Extract Archive",
            param_order=("archive", "extract_on", "dest", "device_temp_path", "cleanup"),
            ref_filters={"archive": (RuntimeValueType.FILE_PATH,)},
            tooltip_key_prefix="steps.extract_archive",
        ),
    ),
    StepPlugin(
        type="copy_files",
        spec=StepSpec(
            type_name="copy_files",
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
        handler=copy_files.handle,
        validate=validate_copy_files,
        outputs=(StepOutputMetadata("copied_paths", RuntimeValueType.PATH_LIST, primary=True),),
        editor=StepEditorMetadata(
            label="Copy Files",
            param_order=("source", "dest", "copy_policy"),
            ref_filters={
                "source": (
                    RuntimeValueType.FILE_PATH,
                    RuntimeValueType.DIRECTORY_PATH,
                    RuntimeValueType.PATH_LIST,
                )
            },
            tooltip_key_prefix="steps.copy_files",
        ),
    ),
    StepPlugin(
        type="install_apk",
        spec=StepSpec(
            type_name="install_apk",
            params={
                "app": ParamSpec(ParamMode.REF),
                "replace_existing": ParamSpec(ParamMode.LITERAL, required=False, default=False),
            },
            executor_handler="install_apk",
        ),
        handler=install_apk.handle,
        editor=StepEditorMetadata(
            label="Install APK",
            param_order=("app", "replace_existing"),
            ref_filters={"app": (RuntimeValueType.FILE_PATH,)},
            tooltip_key_prefix="steps.install_apk",
        ),
    ),
    StepPlugin(
        type="grant_permissions",
        spec=StepSpec(
            type_name="grant_permissions",
            params={
                "runtime": ParamSpec(ParamMode.LITERAL, required=False),
                "appops": ParamSpec(ParamMode.LITERAL, required=False),
                "policy": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="grant_permissions",
        ),
        handler=grant_permissions.handle,
        validate=validate_grant_permissions,
        editor=StepEditorMetadata(
            label="Grant Permissions",
            param_order=("runtime", "appops", "policy"),
            tooltip_key_prefix="steps.grant_permissions",
        ),
    ),
    StepPlugin(
        type="launch_app",
        spec=StepSpec(
            type_name="launch_app",
            params={
                "package_name": ParamSpec(ParamMode.LITERAL),
                "activity": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="launch_app",
        ),
        handler=launch_app.handle,
        validate=validate_package_name,
        editor=StepEditorMetadata(
            label="Launch App",
            param_order=("package_name", "activity"),
            tooltip_key_prefix="steps.launch_app",
        ),
    ),
    StepPlugin(
        type="wait",
        spec=StepSpec(
            type_name="wait",
            params={"duration_ms": ParamSpec(ParamMode.LITERAL)},
            executor_handler="wait",
        ),
        handler=wait.handle,
        validate=validate_wait,
        editor=StepEditorMetadata(
            label="Wait",
            param_order=("duration_ms",),
            tooltip_key_prefix="steps.wait",
        ),
    ),
    StepPlugin(
        type="force_stop_app",
        spec=StepSpec(
            type_name="force_stop_app",
            params={"package_name": ParamSpec(ParamMode.LITERAL)},
            executor_handler="force_stop_app",
        ),
        handler=force_stop_app.handle,
        validate=validate_package_name,
        editor=StepEditorMetadata(
            label="Force Stop",
            param_order=("package_name",),
            tooltip_key_prefix="steps.force_stop_app",
        ),
    ),
)


@lru_cache(maxsize=1)
def builtin_step_registry() -> StepRegistry:
    return StepRegistry(BUILTIN_STEP_PLUGINS)

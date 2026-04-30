"""Built-in step plugin registrations."""

from __future__ import annotations

from functools import lru_cache

from emuchef.domain.copy_policy import CopyPolicy
from emuchef.domain.runtime_state import RuntimeValueType
from emuchef.domain.step_specs import ParamMode, ParamSpec, StepSpec
from emuchef.domain.step_types import StepType

from .contracts import StepEditorMetadata, StepOutputMetadata, StepPlugin, StepRegistry
from .planner_hooks import (
    normalize_artifact_selection,
    validate_artifact_selection,
    validate_copy_files,
    validate_extract_archive,
    validate_grant_permissions,
    validate_package_name,
    validate_wait,
)


def _handler(name: str, *, takes_step: bool = True):
    """Resolve executor implementation lazily to keep plugin metadata Qt/core-safe."""

    def run(context, step, resolved_params):
        from emuchef.executor import step_handlers

        function = getattr(step_handlers, name)
        result = function(context, step, resolved_params) if takes_step else function(context, resolved_params)
        return {} if result is None else result

    run.__name__ = name.removeprefix("_")
    return run


BUILTIN_STEP_PLUGINS: tuple[StepPlugin, ...] = (
    StepPlugin(
        type=StepType.RESOLVE_ARTIFACTS,
        spec=StepSpec(
            type_name=StepType.RESOLVE_ARTIFACTS,
            params={
                "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
                "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="resolve_artifacts",
        ),
        handler=_handler("_resolve_artifacts"),
        normalize=normalize_artifact_selection,
        validate=validate_artifact_selection,
        editor=StepEditorMetadata(
            label="Resolve Artifacts",
            param_order=("artifacts", "artifact_groups"),
            tooltip_key_prefix="steps.resolve_artifacts",
        ),
    ),
    StepPlugin(
        type=StepType.EXTRACT_ARTIFACTS,
        spec=StepSpec(
            type_name=StepType.EXTRACT_ARTIFACTS,
            params={
                "artifacts": ParamSpec(ParamMode.LITERAL, required=False),
                "artifact_groups": ParamSpec(ParamMode.LITERAL, required=False),
                "extract_on": ParamSpec(ParamMode.LITERAL, required=False, default="host", enum_values=("host", "device")),
            },
            primary_output_name="extracted_paths",
            executor_handler="extract_artifacts",
        ),
        handler=_handler("_extract_artifacts"),
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
        type=StepType.EXTRACT_ARCHIVE,
        spec=StepSpec(
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
        handler=_handler("_extract_archive"),
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
        type=StepType.COPY_FILES,
        spec=StepSpec(
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
        handler=_handler("_copy_files"),
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
        type=StepType.INSTALL_APK,
        spec=StepSpec(
            type_name=StepType.INSTALL_APK,
            params={
                "app": ParamSpec(ParamMode.REF),
                "replace_existing": ParamSpec(ParamMode.LITERAL, required=False, default=False),
            },
            executor_handler="install_apk",
        ),
        handler=_handler("_install_apk", takes_step=False),
        editor=StepEditorMetadata(
            label="Install APK",
            param_order=("app", "replace_existing"),
            ref_filters={"app": (RuntimeValueType.FILE_PATH,)},
            tooltip_key_prefix="steps.install_apk",
        ),
    ),
    StepPlugin(
        type=StepType.GRANT_PERMISSIONS,
        spec=StepSpec(
            type_name=StepType.GRANT_PERMISSIONS,
            params={
                "runtime": ParamSpec(ParamMode.LITERAL, required=False),
                "appops": ParamSpec(ParamMode.LITERAL, required=False),
                "policy": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="grant_permissions",
        ),
        handler=_handler("_grant_permissions"),
        validate=validate_grant_permissions,
        editor=StepEditorMetadata(
            label="Grant Permissions",
            param_order=("runtime", "appops", "policy"),
            tooltip_key_prefix="steps.grant_permissions",
        ),
    ),
    StepPlugin(
        type=StepType.LAUNCH_APP,
        spec=StepSpec(
            type_name=StepType.LAUNCH_APP,
            params={
                "package_name": ParamSpec(ParamMode.LITERAL),
                "activity": ParamSpec(ParamMode.LITERAL, required=False),
            },
            executor_handler="launch_app",
        ),
        handler=_handler("_launch_app", takes_step=False),
        validate=validate_package_name,
        editor=StepEditorMetadata(
            label="Launch App",
            param_order=("package_name", "activity"),
            tooltip_key_prefix="steps.launch_app",
        ),
    ),
    StepPlugin(
        type=StepType.WAIT,
        spec=StepSpec(
            type_name=StepType.WAIT,
            params={"duration_ms": ParamSpec(ParamMode.LITERAL)},
            executor_handler="wait",
        ),
        handler=_handler("_wait", takes_step=False),
        validate=validate_wait,
        editor=StepEditorMetadata(
            label="Wait",
            param_order=("duration_ms",),
            tooltip_key_prefix="steps.wait",
        ),
    ),
    StepPlugin(
        type=StepType.FORCE_STOP_APP,
        spec=StepSpec(
            type_name=StepType.FORCE_STOP_APP,
            params={"package_name": ParamSpec(ParamMode.LITERAL)},
            executor_handler="force_stop_app",
        ),
        handler=_handler("_force_stop_app", takes_step=False),
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

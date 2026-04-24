"""Shared tooltip copy for recipe editor field surfaces."""

from __future__ import annotations

from emuchef.domain import ArtifactCacheMode, InputRole, InputType, PERMISSION_POLICY_ON_FAILURE_VALUES
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
    "permissions.runtime.package": "Android package name the runtime permission grant applies to.",
    "permissions.runtime.permission": "Android runtime permission name to grant for the selected package.",
    "permissions.runtime.required": "When enabled, failure to apply this grant is treated as required permission work.",
    "permissions.runtime.rooted": "Optional rooted-state filter for this grant. Leave it at Any to apply regardless of device root state.",
    "permissions.runtime.android_api_min": "Optional minimum Android API level where this grant should apply.",
    "permissions.runtime.android_api_max": "Optional maximum Android API level where this grant should apply.",
    "permissions.appops.package": "Android package name the app-op grant applies to.",
    "permissions.appops.op": "Android app-op name to configure for the selected package.",
    "permissions.appops.mode": "Authored app-op mode value to apply, such as allow or ignore.",
    "permissions.appops.required": "When enabled, failure to apply this app-op is treated as required permission work.",
    "permissions.appops.rooted": "Optional rooted-state filter for this app-op. Leave it at Any to apply regardless of device root state.",
    "permissions.appops.android_api_min": "Optional minimum Android API level where this app-op should apply.",
    "permissions.appops.android_api_max": "Optional maximum Android API level where this app-op should apply.",
    "permissions.policy.on_failure": (
        "Policy outcome when permission work fails. "
        f"Accepted values: {_enum_values(*PERMISSION_POLICY_ON_FAILURE_VALUES)}."
    ),
    "permissions.policy.require_all": "When enabled, all applicable permission actions must succeed for the policy to pass.",
    "steps.id": (
        "This stable authored step id is used by dependencies and refs such as 'steps.<id>.outputs.<field>'. "
        "It stays read-only after creation because step rename with ref rewrite is not implemented."
    ),
    "steps.type": (
        "This selects the authored step kind. "
        "It stays read-only after creation because changing type would rewrite step structure and is not supported."
    ),
    "steps.name": "Human-readable step name shown in the editor and authored YAML.",
    "steps.user_toggleable": "When enabled, users may opt this step in or out at execution time.",
    "steps.description": "Optional author-facing description of what this step does or why it exists.",
    "steps.dependencies": "Ordered list of prerequisite step ids. New dependencies append to the authored list and are not re-sorted.",
    "steps.constraints.capabilities": "Runtime capabilities this step requires before execution can proceed.",
    "steps.constraints.conflicts_with": "Other steps that must not run alongside this step in the same execution plan.",
    "steps.skip_if": "Structured conditions checked immediately before execution. Matching conditions skip the step instead of running it.",
    "steps.verify": "Structured checks that run only after successful step execution.",
    "steps.resolve_artifacts.artifacts": "Artifacts to resolve directly by authored artifact id.",
    "steps.resolve_artifacts.artifact_groups": "Artifact groups to resolve by authored group id.",
    "steps.extract_artifacts.artifacts": "Artifacts whose resolved files should be extracted directly.",
    "steps.extract_artifacts.artifact_groups": "Artifact groups whose resolved files should be extracted.",
    "steps.extract_artifacts.extract_on": "Extraction target location. Accepted values: host or device.",
    "steps.extract_archive.archive": (
        "Choose an authored ref to the archive file to extract. "
        "Saved YAML keeps the explicit { ref: ... } value."
    ),
    "steps.extract_archive.extract_on": "Extraction target location. Accepted values: host or device.",
    "steps.extract_archive.dest": "Destination path used when archive extraction runs on the device.",
    "steps.extract_archive.device_temp_path": "Optional device-side temporary path used during device extraction.",
    "steps.extract_archive.cleanup": "When enabled, temporary extraction artifacts are cleaned up after the step finishes.",
    "steps.copy_files.source": (
        "Choose an authored ref to the source file, directory, or path list. "
        "Saved YAML keeps the explicit { ref: ... } value."
    ),
    "steps.copy_files.dest": "Literal destination path for the copy operation. Directory and exact-target behavior depends on the source shape.",
    "steps.copy_files.copy_policy": "Copy behavior to apply when the destination already contains content.",
    "steps.install_apk.app": (
        "Choose an authored ref to the APK file to install. "
        "Saved YAML keeps the explicit { ref: ... } value."
    ),
    "steps.install_apk.replace_existing": "When enabled, an existing installation may be replaced during APK install.",
    "steps.grant_permissions.note": "This step consumes the top-level permission plan. It is valid as a clean no-op when no permission actions apply.",
    "steps.launch_app.package_name": "Android package name to launch.",
    "steps.launch_app.activity": "Optional explicit activity name. Leave it blank to use normal launcher resolution behavior.",
    "steps.wait.duration_ms": "Integer wait duration in milliseconds.",
    "steps.force_stop_app.package_name": "Android package name to stop before continuing.",
    "steps.preserved_content": (
        "Unsupported authored step content is shown read-only here. "
        "It is preserved on save unless you explicitly replace it through a supported editor surface."
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
    "permissions.runtime.package": "Enter the Android package name this runtime permission applies to.",
    "permissions.runtime.permission": "Enter the Android runtime permission name to grant.",
    "permissions.appops.package": "Enter the Android package name this app-op applies to.",
    "permissions.appops.op": "Enter the Android app-op name to configure.",
    "permissions.appops.mode": "Enter the authored app-op mode value to apply.",
    "steps.id": (
        "Choose a stable authored step id. "
        "It becomes read-only after creation because step rename with ref rewrite is not implemented."
    ),
    "steps.type": (
        "Choose the authored step type. "
        "It becomes read-only after creation because changing type would rewrite step structure and is not supported."
    ),
    "steps.name": "Enter the human-readable step name shown in the editor and authored YAML.",
    "steps.condition.type": "Choose the structured condition type to add to this list.",
    "steps.condition.target": "Enter the path, package name, or other target value required by the selected condition type.",
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

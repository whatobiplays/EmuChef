"""Decode external JSON editor commands into core command dataclasses."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any, Callable

from emuchef.domain import RefParamValue
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    AddInputCommand,
    AddStepCommand,
    DeleteArtifactCommand,
    DeleteArtifactGroupCommand,
    DeleteInputCommand,
    DeleteStepCommand,
    DuplicateArtifactCommand,
    DuplicateArtifactGroupCommand,
    DuplicateInputCommand,
    DuplicateStepCommand,
    RemoveArtifactGroupMemberCommand,
    RecipeCommand,
    RenameArtifactCommand,
    RenameArtifactGroupCommand,
    RenameInputCommand,
    ReorderArtifactGroupCommand,
    ReorderArtifactGroupMemberCommand,
    ReorderStepCommand,
    SetOverviewFieldCommand,
    SetStepUserToggleableCommand,
    UpdateArtifactFieldCommand,
    UpdateInputFieldCommand,
    UpdateStepBasicsCommand,
    UpdateStepDependenciesCommand,
    UpdateStepParamsCommand,
)

from .errors import ApiError

Decoder = Callable[[Mapping[str, Any]], RecipeCommand]


def decode_recipe_command(payload: Mapping[str, Any] | object) -> RecipeCommand:
    if not isinstance(payload, Mapping):
        raise ApiError("invalid_command", "Command payload must be an object.", {"commandType": None})
    command_type = payload.get("type")
    if not isinstance(command_type, str) or not command_type:
        raise ApiError("invalid_command", "Command payload must include a string type.", {"commandType": command_type})
    decoder = _DECODERS.get(command_type)
    if decoder is None:
        raise ApiError("invalid_command", f"Unsupported command type: {command_type}", {"commandType": command_type})
    try:
        return decoder(payload)
    except ApiError:
        raise
    except (TypeError, ValueError) as exc:
        raise ApiError("invalid_command", f"Invalid {command_type} command payload: {exc}", {"commandType": command_type}) from exc


def _decode_set_overview_field(payload: Mapping[str, Any]) -> SetOverviewFieldCommand:
    field = _required_str(payload, "field")
    _require_one_of(field, "field", ("id", "name", "description"))
    return SetOverviewFieldCommand(
        field=field,
        value=_required(payload, "value"),
    )


def _decode_add_input(payload: Mapping[str, Any]) -> AddInputCommand:
    return AddInputCommand(input_id=_required_str(payload, "inputId"))


def _decode_rename_input(payload: Mapping[str, Any]) -> RenameInputCommand:
    return RenameInputCommand(
        input_id=_required_str(payload, "inputId"),
        new_input_id=_required_str(payload, "newInputId"),
    )


def _decode_delete_input(payload: Mapping[str, Any]) -> DeleteInputCommand:
    return DeleteInputCommand(input_id=_required_str(payload, "inputId"))


def _decode_duplicate_input(payload: Mapping[str, Any]) -> DuplicateInputCommand:
    return DuplicateInputCommand(
        source_input_id=_required_str(payload, "sourceInputId"),
        new_input_id=_required_str(payload, "newInputId"),
    )


def _decode_update_input_field(payload: Mapping[str, Any]) -> UpdateInputFieldCommand:
    field = _required_str(payload, "field")
    _require_one_of(
        field,
        "field",
        (
            "type",
            "role",
            "label",
            "description",
            "required",
            "multiple",
            "validation.must_exist",
            "validation.allowed_extensions",
            "validation.path_kind",
        ),
    )
    return UpdateInputFieldCommand(
        input_id=_required_str(payload, "inputId"),
        field=field,
        value=_required(payload, "value"),
    )


def _decode_add_artifact(payload: Mapping[str, Any]) -> AddArtifactCommand:
    return AddArtifactCommand(
        artifact_id=_required_str(payload, "artifactId"),
        url=_required_str(payload, "url"),
    )


def _decode_update_artifact_field(payload: Mapping[str, Any]) -> UpdateArtifactFieldCommand:
    field = _required_str(payload, "field")
    _require_one_of(field, "field", ("url", "cache"))
    return UpdateArtifactFieldCommand(
        artifact_id=_required_str(payload, "artifactId"),
        field=field,
        value=_required(payload, "value"),
    )


def _decode_rename_artifact(payload: Mapping[str, Any]) -> RenameArtifactCommand:
    return RenameArtifactCommand(
        artifact_id=_required_str(payload, "artifactId"),
        new_artifact_id=_required_str(payload, "newArtifactId"),
    )


def _decode_delete_artifact(payload: Mapping[str, Any]) -> DeleteArtifactCommand:
    return DeleteArtifactCommand(artifact_id=_required_str(payload, "artifactId"))


def _decode_duplicate_artifact(payload: Mapping[str, Any]) -> DuplicateArtifactCommand:
    return DuplicateArtifactCommand(
        source_artifact_id=_required_str(payload, "sourceArtifactId"),
        new_artifact_id=_required_str(payload, "newArtifactId"),
    )


def _decode_add_artifact_group(payload: Mapping[str, Any]) -> AddArtifactGroupCommand:
    return AddArtifactGroupCommand(group_id=_required_str(payload, "groupId"))


def _decode_rename_artifact_group(payload: Mapping[str, Any]) -> RenameArtifactGroupCommand:
    return RenameArtifactGroupCommand(
        group_id=_required_str(payload, "groupId"),
        new_group_id=_required_str(payload, "newGroupId"),
    )


def _decode_delete_artifact_group(payload: Mapping[str, Any]) -> DeleteArtifactGroupCommand:
    return DeleteArtifactGroupCommand(group_id=_required_str(payload, "groupId"))


def _decode_duplicate_artifact_group(payload: Mapping[str, Any]) -> DuplicateArtifactGroupCommand:
    return DuplicateArtifactGroupCommand(
        source_group_id=_required_str(payload, "sourceGroupId"),
        new_group_id=_required_str(payload, "newGroupId"),
    )


def _decode_reorder_artifact_group(payload: Mapping[str, Any]) -> ReorderArtifactGroupCommand:
    return ReorderArtifactGroupCommand(
        group_id=_required_str(payload, "groupId"),
        to_index=_required_index(payload, "toIndex"),
    )


def _decode_add_artifact_group_member(payload: Mapping[str, Any]) -> AddArtifactGroupMemberCommand:
    return AddArtifactGroupMemberCommand(
        group_id=_required_str(payload, "groupId"),
        artifact_id=_required_str(payload, "artifactId"),
        index=_optional_index(payload, "index"),
    )


def _decode_remove_artifact_group_member(payload: Mapping[str, Any]) -> RemoveArtifactGroupMemberCommand:
    return RemoveArtifactGroupMemberCommand(
        group_id=_required_str(payload, "groupId"),
        index=_required_index(payload, "index"),
    )


def _decode_reorder_artifact_group_member(payload: Mapping[str, Any]) -> ReorderArtifactGroupMemberCommand:
    return ReorderArtifactGroupMemberCommand(
        group_id=_required_str(payload, "groupId"),
        index=_required_index(payload, "index"),
        to_index=_required_index(payload, "toIndex"),
    )


def _decode_add_step(payload: Mapping[str, Any]) -> AddStepCommand:
    return AddStepCommand(
        step_id=_required_str(payload, "stepId"),
        step_type=_required_str(payload, "stepType"),
        name=_required_str(payload, "name"),
        index=_optional_index(payload, "index"),
    )


def _decode_delete_step(payload: Mapping[str, Any]) -> DeleteStepCommand:
    return DeleteStepCommand(step_id=_required_str(payload, "stepId"))


def _decode_duplicate_step(payload: Mapping[str, Any]) -> DuplicateStepCommand:
    return DuplicateStepCommand(
        source_step_id=_required_str(payload, "sourceStepId"),
        new_step_id=_required_str(payload, "newStepId"),
    )


def _decode_reorder_step(payload: Mapping[str, Any]) -> ReorderStepCommand:
    return ReorderStepCommand(
        step_id=_required_str(payload, "stepId"),
        to_index=_required_index(payload, "toIndex"),
    )


def _decode_update_step_basics(payload: Mapping[str, Any]) -> UpdateStepBasicsCommand:
    return UpdateStepBasicsCommand(
        step_id=_required_str(payload, "stepId"),
        name=_required_str(payload, "name"),
        description=_required_optional_str(payload, "description"),
    )


def _decode_set_step_user_toggleable(payload: Mapping[str, Any]) -> SetStepUserToggleableCommand:
    return SetStepUserToggleableCommand(
        step_id=_required_str(payload, "stepId"),
        user_toggleable=_required_bool(payload, "userToggleable"),
    )


def _decode_update_step_dependencies(payload: Mapping[str, Any]) -> UpdateStepDependenciesCommand:
    return UpdateStepDependenciesCommand(
        step_id=_required_str(payload, "stepId"),
        dependencies=_string_tuple(_required(payload, "dependencies"), field="dependencies"),
    )


def _decode_update_step_params(payload: Mapping[str, Any]) -> UpdateStepParamsCommand:
    params = _required(payload, "params")
    if not isinstance(params, Mapping):
        raise ApiError("invalid_command", "Command field 'params' must be an object.", {"field": "params"})
    return UpdateStepParamsCommand(
        step_id=_required_str(payload, "stepId"),
        params={str(name): _decode_authored_param_value(value) for name, value in params.items()},
    )


def _decode_authored_param_value(value: Any) -> Any:
    """Decode one top-level authored param value from the JSON command payload.

    Authored refs use the YAML/DTO shape {"ref": "..."} at the top level of an
    individual param. Nested objects and lists remain literal authored JSON
    values; the command codec does not recursively interpret arbitrary "ref"
    keys.
    """

    if isinstance(value, Mapping) and set(value.keys()) == {"ref"} and isinstance(value.get("ref"), str):
        return RefParamValue(ref=value["ref"])
    return value


def _required(payload: Mapping[str, Any], field: str) -> Any:
    if field not in payload:
        raise ApiError("invalid_command", f"Command payload is missing required field: {field}", {"field": field})
    return payload[field]


def _optional(payload: Mapping[str, Any], field: str) -> Any:
    return payload[field] if field in payload else None


def _required_str(payload: Mapping[str, Any], field: str) -> str:
    value = _required(payload, field)
    if not isinstance(value, str) or not value:
        raise ApiError("invalid_command", f"Command field {field!r} must be a non-empty string.", {"field": field})
    return value


def _required_index(payload: Mapping[str, Any], field: str) -> int:
    value = _required(payload, field)
    if not isinstance(value, int):
        raise ApiError("invalid_command", f"Command field {field!r} must be an integer.", {"field": field})
    return value


def _optional_index(payload: Mapping[str, Any], field: str) -> int | None:
    value = _optional(payload, field)
    if value is None:
        return None
    if not isinstance(value, int):
        raise ApiError("invalid_command", f"Command field {field!r} must be an integer.", {"field": field})
    return value


def _required_bool(payload: Mapping[str, Any], field: str) -> bool:
    value = _required(payload, field)
    if not isinstance(value, bool):
        raise ApiError("invalid_command", f"Command field {field!r} must be a boolean.", {"field": field})
    return value


def _required_optional_str(payload: Mapping[str, Any], field: str) -> str | None:
    value = _required(payload, field)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ApiError("invalid_command", f"Command field {field!r} must be a string or null.", {"field": field})
    return value


def _string_tuple(value: Any, *, field: str) -> tuple[str, ...]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        raise ApiError("invalid_command", f"Command field {field!r} must be a list.", {"field": field})
    for item in value:
        if not isinstance(item, str) or not item:
            raise ApiError("invalid_command", f"Command field {field!r} must contain only non-empty strings.", {"field": field})
    return tuple(value)


def _require_one_of(value: str, field: str, allowed_values: tuple[str, ...]) -> None:
    if value not in allowed_values:
        raise ApiError(
            "invalid_command",
            f"Command field {field!r} has unsupported value: {value}",
            {"field": field, "allowedValues": list(allowed_values)},
        )


_DECODERS: dict[str, Decoder] = {
    "SetOverviewField": _decode_set_overview_field,
    "AddInput": _decode_add_input,
    "RenameInput": _decode_rename_input,
    "DeleteInput": _decode_delete_input,
    "DuplicateInput": _decode_duplicate_input,
    "UpdateInputField": _decode_update_input_field,
    "AddArtifact": _decode_add_artifact,
    "UpdateArtifactField": _decode_update_artifact_field,
    "RenameArtifact": _decode_rename_artifact,
    "DeleteArtifact": _decode_delete_artifact,
    "DuplicateArtifact": _decode_duplicate_artifact,
    "AddArtifactGroup": _decode_add_artifact_group,
    "RenameArtifactGroup": _decode_rename_artifact_group,
    "DeleteArtifactGroup": _decode_delete_artifact_group,
    "DuplicateArtifactGroup": _decode_duplicate_artifact_group,
    "ReorderArtifactGroup": _decode_reorder_artifact_group,
    "AddArtifactGroupMember": _decode_add_artifact_group_member,
    "RemoveArtifactGroupMember": _decode_remove_artifact_group_member,
    "ReorderArtifactGroupMember": _decode_reorder_artifact_group_member,
    "AddStep": _decode_add_step,
    "DeleteStep": _decode_delete_step,
    "DuplicateStep": _decode_duplicate_step,
    "ReorderStep": _decode_reorder_step,
    "UpdateStepBasics": _decode_update_step_basics,
    "SetStepUserToggleable": _decode_set_step_user_toggleable,
    "UpdateStepDependencies": _decode_update_step_dependencies,
    "UpdateStepParams": _decode_update_step_params,
}

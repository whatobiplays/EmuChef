"""Command-based recipe mutations for the editor core."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from emuchef.domain import ArtifactCacheMode, InputDeclaration, InputRole, InputType, InputValidation, Recipe, RemoteFileArtifact


@dataclass(frozen=True, slots=True)
class SetOverviewFieldCommand:
    field: Literal["id", "name", "description"]
    value: str | None


@dataclass(frozen=True, slots=True)
class AddRecipeDependencyCommand:
    value: str
    index: int | None = None


@dataclass(frozen=True, slots=True)
class UpdateRecipeDependencyCommand:
    index: int
    value: str


@dataclass(frozen=True, slots=True)
class RemoveRecipeDependencyCommand:
    index: int


@dataclass(frozen=True, slots=True)
class MoveRecipeDependencyCommand:
    index: int
    to_index: int


@dataclass(frozen=True, slots=True)
class AddProvidedFeatureCommand:
    value: str
    index: int | None = None


@dataclass(frozen=True, slots=True)
class UpdateProvidedFeatureCommand:
    index: int
    value: str


@dataclass(frozen=True, slots=True)
class RemoveProvidedFeatureCommand:
    index: int


@dataclass(frozen=True, slots=True)
class MoveProvidedFeatureCommand:
    index: int
    to_index: int


@dataclass(frozen=True, slots=True)
class AddInputCommand:
    input_id: str


@dataclass(frozen=True, slots=True)
class UpdateInputFieldCommand:
    input_id: str
    field: Literal[
        "type",
        "role",
        "label",
        "description",
        "required",
        "multiple",
        "validation.must_exist",
        "validation.allowed_extensions",
        "validation.path_kind",
    ]
    value: object


@dataclass(frozen=True, slots=True)
class DeleteInputCommand:
    input_id: str


@dataclass(frozen=True, slots=True)
class DuplicateInputCommand:
    source_input_id: str
    new_input_id: str


@dataclass(frozen=True, slots=True)
class AddArtifactCommand:
    artifact_id: str
    url: str


@dataclass(frozen=True, slots=True)
class UpdateArtifactFieldCommand:
    artifact_id: str
    field: Literal["url", "cache"]
    value: object


@dataclass(frozen=True, slots=True)
class DeleteArtifactCommand:
    artifact_id: str


@dataclass(frozen=True, slots=True)
class DuplicateArtifactCommand:
    source_artifact_id: str
    new_artifact_id: str


@dataclass(frozen=True, slots=True)
class AddArtifactGroupCommand:
    group_id: str


@dataclass(frozen=True, slots=True)
class DeleteArtifactGroupCommand:
    group_id: str


@dataclass(frozen=True, slots=True)
class ReorderArtifactGroupCommand:
    group_id: str
    to_index: int


@dataclass(frozen=True, slots=True)
class AddArtifactGroupMemberCommand:
    group_id: str
    artifact_id: str
    index: int | None = None


@dataclass(frozen=True, slots=True)
class RemoveArtifactGroupMemberCommand:
    group_id: str
    index: int


@dataclass(frozen=True, slots=True)
class ReorderArtifactGroupMemberCommand:
    group_id: str
    index: int
    to_index: int


RecipeCommand = (
    SetOverviewFieldCommand
    | AddRecipeDependencyCommand
    | UpdateRecipeDependencyCommand
    | RemoveRecipeDependencyCommand
    | MoveRecipeDependencyCommand
    | AddProvidedFeatureCommand
    | UpdateProvidedFeatureCommand
    | RemoveProvidedFeatureCommand
    | MoveProvidedFeatureCommand
    | AddInputCommand
    | UpdateInputFieldCommand
    | DeleteInputCommand
    | DuplicateInputCommand
    | AddArtifactCommand
    | UpdateArtifactFieldCommand
    | DeleteArtifactCommand
    | DuplicateArtifactCommand
    | AddArtifactGroupCommand
    | DeleteArtifactGroupCommand
    | ReorderArtifactGroupCommand
    | AddArtifactGroupMemberCommand
    | RemoveArtifactGroupMemberCommand
    | ReorderArtifactGroupMemberCommand
)


def apply_recipe_command(recipe: Recipe, command: RecipeCommand) -> tuple[Recipe, str]:
    """Apply a single editor command to a recipe."""

    if isinstance(command, SetOverviewFieldCommand):
        return _apply_overview_field(recipe, command), f"Update recipe {command.field}"
    if isinstance(command, AddRecipeDependencyCommand):
        return _replace_recipe_dependencies(
            recipe,
            _insert_string_item(recipe.recipe_dependencies, command.value, command.index),
        ), "Add recipe dependency"
    if isinstance(command, UpdateRecipeDependencyCommand):
        return _replace_recipe_dependencies(
            recipe,
            _update_string_item(recipe.recipe_dependencies, command.index, command.value),
        ), "Update recipe dependency"
    if isinstance(command, RemoveRecipeDependencyCommand):
        return _replace_recipe_dependencies(
            recipe,
            _remove_string_item(recipe.recipe_dependencies, command.index),
        ), "Remove recipe dependency"
    if isinstance(command, MoveRecipeDependencyCommand):
        return _replace_recipe_dependencies(
            recipe,
            _move_string_item(recipe.recipe_dependencies, command.index, command.to_index),
        ), "Reorder recipe dependency"
    if isinstance(command, AddProvidedFeatureCommand):
        return _replace_provided_features(
            recipe,
            _insert_string_item(recipe.provides.features, command.value, command.index),
        ), "Add provided feature"
    if isinstance(command, UpdateProvidedFeatureCommand):
        return _replace_provided_features(
            recipe,
            _update_string_item(recipe.provides.features, command.index, command.value),
        ), "Update provided feature"
    if isinstance(command, RemoveProvidedFeatureCommand):
        return _replace_provided_features(
            recipe,
            _remove_string_item(recipe.provides.features, command.index),
        ), "Remove provided feature"
    if isinstance(command, MoveProvidedFeatureCommand):
        return _replace_provided_features(
            recipe,
            _move_string_item(recipe.provides.features, command.index, command.to_index),
        ), "Reorder provided feature"
    if isinstance(command, AddInputCommand):
        return _add_input(recipe, command.input_id), "Add input"
    if isinstance(command, UpdateInputFieldCommand):
        return _update_input_field(recipe, command), f"Update input {command.input_id}"
    if isinstance(command, DeleteInputCommand):
        return _delete_input(recipe, command.input_id), "Delete input"
    if isinstance(command, DuplicateInputCommand):
        return _duplicate_input(recipe, command.source_input_id, command.new_input_id), "Duplicate input"
    if isinstance(command, AddArtifactCommand):
        return _add_artifact(recipe, command.artifact_id, command.url), "Add artifact"
    if isinstance(command, UpdateArtifactFieldCommand):
        return _update_artifact_field(recipe, command), f"Update artifact {command.artifact_id}"
    if isinstance(command, DeleteArtifactCommand):
        return _delete_artifact(recipe, command.artifact_id), "Delete artifact"
    if isinstance(command, DuplicateArtifactCommand):
        return _duplicate_artifact(recipe, command.source_artifact_id, command.new_artifact_id), "Duplicate artifact"
    if isinstance(command, AddArtifactGroupCommand):
        return _add_artifact_group(recipe, command.group_id), "Add artifact group"
    if isinstance(command, DeleteArtifactGroupCommand):
        return _delete_artifact_group(recipe, command.group_id), "Delete artifact group"
    if isinstance(command, ReorderArtifactGroupCommand):
        return _reorder_artifact_group(recipe, command.group_id, command.to_index), "Reorder artifact group"
    if isinstance(command, AddArtifactGroupMemberCommand):
        return _add_artifact_group_member(recipe, command.group_id, command.artifact_id, command.index), "Add group member"
    if isinstance(command, RemoveArtifactGroupMemberCommand):
        return _remove_artifact_group_member(recipe, command.group_id, command.index), "Remove group member"
    if isinstance(command, ReorderArtifactGroupMemberCommand):
        return _reorder_artifact_group_member(recipe, command.group_id, command.index, command.to_index), "Reorder group member"
    raise TypeError(f"Unsupported recipe command: {type(command).__name__}")


def _apply_overview_field(recipe: Recipe, command: SetOverviewFieldCommand) -> Recipe:
    if command.field == "description":
        description = _optional_text(command.value)
        return replace(recipe, description=description)
    value = _required_text(command.value, label=f"recipe {command.field}")
    return replace(recipe, **{command.field: value})


def _replace_recipe_dependencies(recipe: Recipe, values: tuple[str, ...]) -> Recipe:
    return replace(recipe, recipe_dependencies=values)


def _replace_provided_features(recipe: Recipe, values: tuple[str, ...]) -> Recipe:
    return replace(recipe, provides=replace(recipe.provides, features=values))


def _add_input(recipe: Recipe, input_id: str) -> Recipe:
    normalized_id = _normalize_identifier(input_id, label="input id")
    if normalized_id in recipe.inputs:
        raise ValueError(f"Input {normalized_id!r} already exists.")
    inputs = dict(recipe.inputs)
    inputs[normalized_id] = InputDeclaration(
        id=normalized_id,
        type=InputType.FILE,
        role=InputRole.GENERIC,
        label=normalized_id,
        description=None,
        required=False,
        multiple=False,
        validation=InputValidation(
            must_exist=False,
            allowed_extensions=(),
            path_kind=InputType.FILE,
        ),
        default=None,
        metadata={},
    )
    return replace(recipe, inputs=inputs)


def _update_input_field(recipe: Recipe, command: UpdateInputFieldCommand) -> Recipe:
    declaration = recipe.inputs.get(command.input_id)
    if declaration is None:
        raise ValueError(f"Unknown input id {command.input_id!r}.")

    if command.field == "type":
        updated = replace(declaration, type=_coerce_input_type(command.value))
    elif command.field == "role":
        updated = replace(declaration, role=_coerce_input_role(command.value))
    elif command.field == "label":
        updated = replace(declaration, label=str(command.value))
    elif command.field == "description":
        updated = replace(declaration, description=_optional_text(command.value))
    elif command.field == "required":
        updated = replace(declaration, required=bool(command.value))
    elif command.field == "multiple":
        updated = replace(declaration, multiple=bool(command.value))
    elif command.field == "validation.must_exist":
        updated = replace(
            declaration,
            validation=replace(declaration.validation, must_exist=bool(command.value)),
        )
    elif command.field == "validation.allowed_extensions":
        updated = replace(
            declaration,
            validation=replace(
                declaration.validation,
                allowed_extensions=_coerce_allowed_extensions(command.value),
            ),
        )
    elif command.field == "validation.path_kind":
        updated = replace(
            declaration,
            validation=replace(
                declaration.validation,
                path_kind=_coerce_optional_input_type(command.value),
            ),
        )
    else:
        raise ValueError(f"Unknown input field {command.field!r}.")

    return replace(recipe, inputs=_replace_mapping_value(recipe.inputs, command.input_id, updated))


def _delete_input(recipe: Recipe, input_id: str) -> Recipe:
    _require_known_mapping_key(recipe.inputs, input_id, label="input")
    return replace(
        recipe,
        inputs={key: value for key, value in recipe.inputs.items() if key != input_id},
    )


def _duplicate_input(recipe: Recipe, source_input_id: str, new_input_id: str) -> Recipe:
    source = recipe.inputs.get(source_input_id)
    if source is None:
        raise ValueError(f"Unknown input id {source_input_id!r}.")
    normalized_id = _normalize_identifier(new_input_id, label="input id")
    if normalized_id in recipe.inputs:
        raise ValueError(f"Input {normalized_id!r} already exists.")
    inputs = dict(recipe.inputs)
    inputs[normalized_id] = replace(source, id=normalized_id)
    return replace(recipe, inputs=inputs)


def _add_artifact(recipe: Recipe, artifact_id: str, url: str) -> Recipe:
    normalized_id = _normalize_identifier(artifact_id, label="artifact id")
    if normalized_id in recipe.artifacts:
        raise ValueError(f"Artifact {normalized_id!r} already exists.")
    artifacts = dict(recipe.artifacts)
    artifacts[normalized_id] = RemoteFileArtifact(
        id=normalized_id,
        url=_required_text(url, label="artifact url"),
        cache=ArtifactCacheMode.DEFAULT,
    )
    return replace(recipe, artifacts=artifacts)


def _update_artifact_field(recipe: Recipe, command: UpdateArtifactFieldCommand) -> Recipe:
    artifact = recipe.artifacts.get(command.artifact_id)
    if artifact is None:
        raise ValueError(f"Unknown artifact id {command.artifact_id!r}.")

    if command.field == "url":
        updated = replace(artifact, url=_required_text(command.value, label="artifact url"))
    elif command.field == "cache":
        updated = replace(artifact, cache=_coerce_artifact_cache(command.value))
    else:
        raise ValueError(f"Unknown artifact field {command.field!r}.")

    return replace(recipe, artifacts=_replace_mapping_value(recipe.artifacts, command.artifact_id, updated))


def _delete_artifact(recipe: Recipe, artifact_id: str) -> Recipe:
    _require_known_mapping_key(recipe.artifacts, artifact_id, label="artifact")
    artifacts = {key: value for key, value in recipe.artifacts.items() if key != artifact_id}
    artifact_groups = {
        group_id: tuple(member for member in members if member != artifact_id)
        for group_id, members in recipe.artifact_groups.items()
    }
    return replace(recipe, artifacts=artifacts, artifact_groups=artifact_groups)


def _duplicate_artifact(recipe: Recipe, source_artifact_id: str, new_artifact_id: str) -> Recipe:
    source = recipe.artifacts.get(source_artifact_id)
    if source is None:
        raise ValueError(f"Unknown artifact id {source_artifact_id!r}.")
    normalized_id = _normalize_identifier(new_artifact_id, label="artifact id")
    if normalized_id in recipe.artifacts:
        raise ValueError(f"Artifact {normalized_id!r} already exists.")
    artifacts = dict(recipe.artifacts)
    artifacts[normalized_id] = replace(source, id=normalized_id)
    return replace(recipe, artifacts=artifacts)


def _add_artifact_group(recipe: Recipe, group_id: str) -> Recipe:
    normalized_id = _normalize_identifier(group_id, label="artifact group id")
    if normalized_id in recipe.artifact_groups:
        raise ValueError(f"Artifact group {normalized_id!r} already exists.")
    artifact_groups = dict(recipe.artifact_groups)
    artifact_groups[normalized_id] = ()
    return replace(recipe, artifact_groups=artifact_groups)


def _delete_artifact_group(recipe: Recipe, group_id: str) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, group_id, label="artifact group")
    return replace(
        recipe,
        artifact_groups={key: value for key, value in recipe.artifact_groups.items() if key != group_id},
    )


def _reorder_artifact_group(recipe: Recipe, group_id: str, to_index: int) -> Recipe:
    items = list(recipe.artifact_groups.items())
    from_index = _index_for_key(items, group_id, label="artifact group")
    moved_items = _move_item(items, from_index, to_index)
    return replace(recipe, artifact_groups=dict(moved_items))


def _add_artifact_group_member(recipe: Recipe, group_id: str, artifact_id: str, index: int | None) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, group_id, label="artifact group")
    _require_known_mapping_key(recipe.artifacts, artifact_id, label="artifact")
    members = list(recipe.artifact_groups[group_id])
    if artifact_id in members:
        raise ValueError(f"Artifact {artifact_id!r} is already in group {group_id!r}.")
    insertion_index = len(members) if index is None else index
    _validate_insert_index(insertion_index, len(members), label="artifact group membership")
    members.insert(insertion_index, artifact_id)
    return replace(recipe, artifact_groups=_replace_mapping_value(recipe.artifact_groups, group_id, tuple(members)))


def _remove_artifact_group_member(recipe: Recipe, group_id: str, index: int) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, group_id, label="artifact group")
    members = list(recipe.artifact_groups[group_id])
    _validate_index(index, len(members), label="artifact group membership")
    del members[index]
    return replace(recipe, artifact_groups=_replace_mapping_value(recipe.artifact_groups, group_id, tuple(members)))


def _reorder_artifact_group_member(recipe: Recipe, group_id: str, index: int, to_index: int) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, group_id, label="artifact group")
    members = list(recipe.artifact_groups[group_id])
    moved_members = _move_item(members, index, to_index)
    return replace(recipe, artifact_groups=_replace_mapping_value(recipe.artifact_groups, group_id, tuple(moved_members)))


def _replace_mapping_value(mapping, key: str, value):
    _require_known_mapping_key(mapping, key, label="mapping entry")
    return {item_key: (value if item_key == key else item_value) for item_key, item_value in mapping.items()}


def _insert_string_item(values: tuple[str, ...], value: str, index: int | None) -> tuple[str, ...]:
    normalized_value = _normalize_identifier(value, label="list item")
    insertion_index = len(values) if index is None else index
    _validate_insert_index(insertion_index, len(values), label="list item")
    items = list(values)
    items.insert(insertion_index, normalized_value)
    return tuple(items)


def _update_string_item(values: tuple[str, ...], index: int, value: str) -> tuple[str, ...]:
    _validate_index(index, len(values), label="list item")
    items = list(values)
    items[index] = _normalize_identifier(value, label="list item")
    return tuple(items)


def _remove_string_item(values: tuple[str, ...], index: int) -> tuple[str, ...]:
    _validate_index(index, len(values), label="list item")
    items = list(values)
    del items[index]
    return tuple(items)


def _move_string_item(values: tuple[str, ...], index: int, to_index: int) -> tuple[str, ...]:
    return tuple(_move_item(list(values), index, to_index))


def _move_item(items: list, from_index: int, to_index: int) -> list:
    _validate_index(from_index, len(items), label="reorder source")
    _validate_index(to_index, len(items), label="reorder target")
    updated_items = list(items)
    item = updated_items.pop(from_index)
    updated_items.insert(to_index, item)
    return updated_items


def _index_for_key(items: list[tuple[str, object]], key: str, *, label: str) -> int:
    for index, (item_key, _) in enumerate(items):
        if item_key == key:
            return index
    raise ValueError(f"Unknown {label} {key!r}.")


def _require_known_mapping_key(mapping, key: str, *, label: str) -> None:
    if key not in mapping:
        raise ValueError(f"Unknown {label} {key!r}.")


def _validate_index(index: int, size: int, *, label: str) -> None:
    if index < 0 or index >= size:
        raise ValueError(f"{label} index {index} is out of range.")


def _validate_insert_index(index: int, size: int, *, label: str) -> None:
    if index < 0 or index > size:
        raise ValueError(f"{label} index {index} is out of range.")


def _normalize_identifier(value: object, *, label: str) -> str:
    normalized = str(value).strip()
    if not normalized:
        raise ValueError(f"{label} must not be empty.")
    return normalized


def _required_text(value: object, *, label: str) -> str:
    return _normalize_identifier(value, label=label)


def _optional_text(value: object) -> str | None:
    if value is None:
        return None
    text = str(value)
    if not text.strip():
        return None
    return text


def _coerce_input_type(value: object) -> InputType:
    if isinstance(value, InputType):
        return value
    return InputType(str(value))


def _coerce_optional_input_type(value: object) -> InputType | None:
    if value in (None, ""):
        return None
    return _coerce_input_type(value)


def _coerce_input_role(value: object) -> InputRole:
    if isinstance(value, InputRole):
        return value
    return InputRole(str(value))


def _coerce_artifact_cache(value: object) -> ArtifactCacheMode:
    if isinstance(value, ArtifactCacheMode):
        return value
    return ArtifactCacheMode(str(value))


def _coerce_allowed_extensions(value: object) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        parts = [item.strip() for item in value.split(",")]
        return tuple(item for item in parts if item)
    if isinstance(value, (list, tuple)):
        return tuple(_normalize_identifier(item, label="extension") for item in value if str(item).strip())
    raise ValueError("allowed_extensions must be a string, list, or tuple.")

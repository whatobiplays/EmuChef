"""Command-based recipe mutations for the editor core."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, replace
from typing import Literal

from emuchef.domain import (
    AuthoredParamValue,
    ArtifactCacheMode,
    InputDeclaration,
    InputRole,
    InputType,
    InputValidation,
    Recipe,
    RefKind,
    RefParamValue,
    RemoteFileArtifact,
    Step,
    StepCondition,
    StepConstraints,
    parse_reference,
)
from emuchef.steps import builtin_step_registry


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
class RenameInputCommand:
    input_id: str
    new_input_id: str


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
class RenameArtifactCommand:
    artifact_id: str
    new_artifact_id: str


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
class RenameArtifactGroupCommand:
    group_id: str
    new_group_id: str


@dataclass(frozen=True, slots=True)
class DeleteArtifactGroupCommand:
    group_id: str


@dataclass(frozen=True, slots=True)
class DuplicateArtifactGroupCommand:
    source_group_id: str
    new_group_id: str


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


@dataclass(frozen=True, slots=True)
class AddStepCommand:
    step_id: str
    step_type: str
    name: str
    index: int | None = None


@dataclass(frozen=True, slots=True)
class RenameRecipeIdCommand:
    new_recipe_id: str


@dataclass(frozen=True, slots=True)
class RenameStepCommand:
    step_id: str
    new_step_id: str


@dataclass(frozen=True, slots=True)
class UpdateStepBasicsCommand:
    step_id: str
    name: str
    description: str | None


@dataclass(frozen=True, slots=True)
class DeleteStepCommand:
    step_id: str


@dataclass(frozen=True, slots=True)
class DuplicateStepCommand:
    source_step_id: str
    new_step_id: str


@dataclass(frozen=True, slots=True)
class ReorderStepCommand:
    step_id: str
    to_index: int


@dataclass(frozen=True, slots=True)
class SetStepUserToggleableCommand:
    step_id: str
    user_toggleable: bool


@dataclass(frozen=True, slots=True)
class UpdateStepDependenciesCommand:
    step_id: str
    dependencies: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class UpdateStepParamsCommand:
    step_id: str
    params: Mapping[str, AuthoredParamValue]


@dataclass(frozen=True, slots=True)
class UpdateStepConstraintsCommand:
    step_id: str
    constraints: StepConstraints


@dataclass(frozen=True, slots=True)
class UpdateStepSkipIfCommand:
    step_id: str
    skip_if: tuple[StepCondition, ...]


@dataclass(frozen=True, slots=True)
class UpdateStepVerifyCommand:
    step_id: str
    verify: tuple[StepCondition, ...]


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
    | RenameInputCommand
    | UpdateInputFieldCommand
    | DeleteInputCommand
    | DuplicateInputCommand
    | AddArtifactCommand
    | RenameArtifactCommand
    | UpdateArtifactFieldCommand
    | DeleteArtifactCommand
    | DuplicateArtifactCommand
    | AddArtifactGroupCommand
    | RenameArtifactGroupCommand
    | DeleteArtifactGroupCommand
    | DuplicateArtifactGroupCommand
    | ReorderArtifactGroupCommand
    | AddArtifactGroupMemberCommand
    | RemoveArtifactGroupMemberCommand
    | ReorderArtifactGroupMemberCommand
    | AddStepCommand
    | RenameRecipeIdCommand
    | RenameStepCommand
    | UpdateStepBasicsCommand
    | DeleteStepCommand
    | DuplicateStepCommand
    | ReorderStepCommand
    | SetStepUserToggleableCommand
    | UpdateStepDependenciesCommand
    | UpdateStepParamsCommand
    | UpdateStepConstraintsCommand
    | UpdateStepSkipIfCommand
    | UpdateStepVerifyCommand
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
    if isinstance(command, RenameInputCommand):
        return _rename_input(recipe, command.input_id, command.new_input_id), f"Rename input {command.input_id}"
    if isinstance(command, UpdateInputFieldCommand):
        return _update_input_field(recipe, command), f"Update input {command.input_id}"
    if isinstance(command, DeleteInputCommand):
        return _delete_input(recipe, command.input_id), "Delete input"
    if isinstance(command, DuplicateInputCommand):
        return _duplicate_input(recipe, command.source_input_id, command.new_input_id), "Duplicate input"
    if isinstance(command, AddArtifactCommand):
        return _add_artifact(recipe, command.artifact_id, command.url), "Add artifact"
    if isinstance(command, RenameArtifactCommand):
        return _rename_artifact(recipe, command.artifact_id, command.new_artifact_id), f"Rename artifact {command.artifact_id}"
    if isinstance(command, UpdateArtifactFieldCommand):
        return _update_artifact_field(recipe, command), f"Update artifact {command.artifact_id}"
    if isinstance(command, DeleteArtifactCommand):
        return _delete_artifact(recipe, command.artifact_id), "Delete artifact"
    if isinstance(command, DuplicateArtifactCommand):
        return _duplicate_artifact(recipe, command.source_artifact_id, command.new_artifact_id), "Duplicate artifact"
    if isinstance(command, AddArtifactGroupCommand):
        return _add_artifact_group(recipe, command.group_id), "Add artifact group"
    if isinstance(command, RenameArtifactGroupCommand):
        return _rename_artifact_group(recipe, command.group_id, command.new_group_id), f"Rename artifact group {command.group_id}"
    if isinstance(command, DeleteArtifactGroupCommand):
        return _delete_artifact_group(recipe, command.group_id), "Delete artifact group"
    if isinstance(command, DuplicateArtifactGroupCommand):
        return (
            _duplicate_artifact_group(recipe, command.source_group_id, command.new_group_id),
            "Duplicate artifact group",
        )
    if isinstance(command, ReorderArtifactGroupCommand):
        return _reorder_artifact_group(recipe, command.group_id, command.to_index), "Reorder artifact group"
    if isinstance(command, AddArtifactGroupMemberCommand):
        return _add_artifact_group_member(recipe, command.group_id, command.artifact_id, command.index), "Add group member"
    if isinstance(command, RemoveArtifactGroupMemberCommand):
        return _remove_artifact_group_member(recipe, command.group_id, command.index), "Remove group member"
    if isinstance(command, ReorderArtifactGroupMemberCommand):
        return _reorder_artifact_group_member(recipe, command.group_id, command.index, command.to_index), "Reorder group member"
    if isinstance(command, AddStepCommand):
        return _add_step(recipe, command), "Add step"
    if isinstance(command, RenameRecipeIdCommand):
        return _rename_recipe_id(recipe, command.new_recipe_id), "Rename recipe id"
    if isinstance(command, RenameStepCommand):
        return _rename_step(recipe, command.step_id, command.new_step_id), f"Rename step {command.step_id}"
    if isinstance(command, UpdateStepBasicsCommand):
        return _update_step_basics(recipe, command), f"Update step {command.step_id}"
    if isinstance(command, DeleteStepCommand):
        return _delete_step(recipe, command.step_id), "Delete step"
    if isinstance(command, DuplicateStepCommand):
        return _duplicate_step(recipe, command.source_step_id, command.new_step_id), "Duplicate step"
    if isinstance(command, ReorderStepCommand):
        return _reorder_step(recipe, command.step_id, command.to_index), "Reorder step"
    if isinstance(command, SetStepUserToggleableCommand):
        return _set_step_user_toggleable(recipe, command.step_id, command.user_toggleable), "Toggle step"
    if isinstance(command, UpdateStepDependenciesCommand):
        return _update_step_dependencies(recipe, command), f"Update dependencies for {command.step_id}"
    if isinstance(command, UpdateStepParamsCommand):
        return _update_step_params(recipe, command), f"Update params for {command.step_id}"
    if isinstance(command, UpdateStepConstraintsCommand):
        return _update_step_constraints(recipe, command), f"Update constraints for {command.step_id}"
    if isinstance(command, UpdateStepSkipIfCommand):
        return _update_step_skip_if(recipe, command), f"Update skip_if for {command.step_id}"
    if isinstance(command, UpdateStepVerifyCommand):
        return _update_step_verify(recipe, command), f"Update verify for {command.step_id}"
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


def _rename_recipe_id(recipe: Recipe, new_recipe_id: str) -> Recipe:
    normalized_id = _normalize_identifier(new_recipe_id, label="recipe id")
    recipe_dependencies = tuple(normalized_id if dependency == recipe.id else dependency for dependency in recipe.recipe_dependencies)
    return replace(recipe, id=normalized_id, recipe_dependencies=recipe_dependencies)


def _rename_input(recipe: Recipe, input_id: str, new_input_id: str) -> Recipe:
    declaration = recipe.inputs.get(input_id)
    if declaration is None:
        raise ValueError(f"Unknown input id {input_id!r}.")
    normalized_id = _normalize_identifier(new_input_id, label="input id")
    if normalized_id != input_id and normalized_id in recipe.inputs:
        raise ValueError(f"Input {normalized_id!r} already exists.")
    inputs = _rename_mapping_key(recipe.inputs, input_id, normalized_id, replace(declaration, id=normalized_id))
    steps = tuple(_rewrite_step_refs(step, "input", input_id, normalized_id) for step in recipe.steps)
    return replace(recipe, inputs=inputs, steps=steps)


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
        steps=tuple(_remove_step_refs(step, "input", input_id) for step in recipe.steps),
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


def _rename_artifact(recipe: Recipe, artifact_id: str, new_artifact_id: str) -> Recipe:
    artifact = recipe.artifacts.get(artifact_id)
    if artifact is None:
        raise ValueError(f"Unknown artifact id {artifact_id!r}.")
    normalized_id = _normalize_identifier(new_artifact_id, label="artifact id")
    if normalized_id != artifact_id and normalized_id in recipe.artifacts:
        raise ValueError(f"Artifact {normalized_id!r} already exists.")
    artifacts = _rename_mapping_key(recipe.artifacts, artifact_id, normalized_id, replace(artifact, id=normalized_id))
    artifact_groups = {
        group_id: tuple(normalized_id if member == artifact_id else member for member in members)
        for group_id, members in recipe.artifact_groups.items()
    }
    steps = tuple(_rewrite_step_refs(step, "artifact", artifact_id, normalized_id) for step in recipe.steps)
    return replace(recipe, artifacts=artifacts, artifact_groups=artifact_groups, steps=steps)


def _delete_artifact(recipe: Recipe, artifact_id: str) -> Recipe:
    _require_known_mapping_key(recipe.artifacts, artifact_id, label="artifact")
    artifacts = {key: value for key, value in recipe.artifacts.items() if key != artifact_id}
    artifact_groups = {
        group_id: tuple(member for member in members if member != artifact_id)
        for group_id, members in recipe.artifact_groups.items()
    }
    steps = tuple(_remove_step_refs(step, "artifact", artifact_id) for step in recipe.steps)
    return replace(recipe, artifacts=artifacts, artifact_groups=artifact_groups, steps=steps)


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
    steps = tuple(_remove_artifact_group_selection(step, group_id) for step in recipe.steps)
    return replace(
        recipe,
        artifact_groups={key: value for key, value in recipe.artifact_groups.items() if key != group_id},
        steps=steps,
    )


def _rename_artifact_group(recipe: Recipe, group_id: str, new_group_id: str) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, group_id, label="artifact group")
    normalized_id = _normalize_identifier(new_group_id, label="artifact group id")
    if normalized_id != group_id and normalized_id in recipe.artifact_groups:
        raise ValueError(f"Artifact group {normalized_id!r} already exists.")
    artifact_groups = _rename_mapping_key(recipe.artifact_groups, group_id, normalized_id, recipe.artifact_groups[group_id])
    steps = tuple(_rewrite_artifact_group_selection(step, group_id, normalized_id) for step in recipe.steps)
    return replace(recipe, artifact_groups=artifact_groups, steps=steps)


def _duplicate_artifact_group(recipe: Recipe, source_group_id: str, new_group_id: str) -> Recipe:
    _require_known_mapping_key(recipe.artifact_groups, source_group_id, label="artifact group")
    normalized_id = _normalize_identifier(new_group_id, label="artifact group id")
    if normalized_id in recipe.artifact_groups:
        raise ValueError(f"Artifact group {normalized_id!r} already exists.")
    artifact_groups = dict(recipe.artifact_groups)
    artifact_groups[normalized_id] = tuple(recipe.artifact_groups[source_group_id])
    return replace(recipe, artifact_groups=artifact_groups)


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


def _add_step(recipe: Recipe, command: AddStepCommand) -> Recipe:
    step_id = _normalize_identifier(command.step_id, label="step id")
    if any(step.id == step_id for step in recipe.steps):
        raise ValueError(f"Step {step_id!r} already exists.")
    step_type = _coerce_supported_step_type(command.step_type)
    insertion_index = len(recipe.steps) if command.index is None else command.index
    _validate_insert_index(insertion_index, len(recipe.steps), label="step")
    steps = list(recipe.steps)
    steps.insert(
        insertion_index,
        Step(
            id=step_id,
            type=step_type,
            name=_required_text(command.name, label="step name"),
            user_toggleable=False,
            dependencies=(),
            constraints=StepConstraints(),
            skip_if=(),
            params={},
            verify=(),
            description=None,
        ),
    )
    return replace(recipe, steps=tuple(steps))


def _rename_step(recipe: Recipe, step_id: str, new_step_id: str) -> Recipe:
    source = _require_known_step(recipe, step_id)
    normalized_id = _normalize_identifier(new_step_id, label="step id")
    if normalized_id != step_id and any(step.id == normalized_id for step in recipe.steps):
        raise ValueError(f"Step {normalized_id!r} already exists.")
    steps: list[Step] = []
    for step in recipe.steps:
        updated = replace(source, id=normalized_id) if step.id == step_id else step
        updated = _rewrite_step_refs(updated, "step", step_id, normalized_id)
        steps.append(updated)
    return replace(recipe, steps=tuple(steps))


def _update_step_basics(recipe: Recipe, command: UpdateStepBasicsCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    updated = replace(
        step,
        name=_required_text(command.name, label="step name"),
        description=_optional_text(command.description),
    )
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _delete_step(recipe: Recipe, step_id: str) -> Recipe:
    _require_known_step(recipe, step_id)
    return replace(
        recipe,
        steps=tuple(_remove_step_refs(step, "step", step_id) for step in recipe.steps if step.id != step_id),
    )


def _duplicate_step(recipe: Recipe, source_step_id: str, new_step_id: str) -> Recipe:
    source = _require_known_step(recipe, source_step_id)
    normalized_id = _normalize_identifier(new_step_id, label="step id")
    if any(step.id == normalized_id for step in recipe.steps):
        raise ValueError(f"Step {normalized_id!r} already exists.")
    source_index = _step_index(recipe.steps, source_step_id)
    steps = list(recipe.steps)
    steps.insert(source_index + 1, replace(source, id=normalized_id))
    return replace(recipe, steps=tuple(steps))


def _reorder_step(recipe: Recipe, step_id: str, to_index: int) -> Recipe:
    steps = list(recipe.steps)
    moved_steps = _move_item(steps, _step_index(recipe.steps, step_id), to_index)
    return replace(recipe, steps=tuple(moved_steps))


def _set_step_user_toggleable(recipe: Recipe, step_id: str, user_toggleable: bool) -> Recipe:
    step = _require_known_step(recipe, step_id)
    updated = replace(step, user_toggleable=bool(user_toggleable))
    return replace(recipe, steps=_replace_step(recipe.steps, step_id, updated))


def _update_step_dependencies(recipe: Recipe, command: UpdateStepDependenciesCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    updated = replace(
        step,
        dependencies=_normalize_identifier_tuple(command.dependencies, label="step dependency id"),
    )
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _update_step_params(recipe: Recipe, command: UpdateStepParamsCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    normalized_params = _normalize_step_params(step.type, command.params)
    updated = replace(step, params=normalized_params)
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _update_step_constraints(recipe: Recipe, command: UpdateStepConstraintsCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    updated = replace(
        step,
        constraints=StepConstraints(
            capabilities=_normalize_identifier_tuple(
                command.constraints.capabilities,
                label="step capability",
            ),
            conflicts_with=_normalize_identifier_tuple(
                command.constraints.conflicts_with,
                label="step conflict",
            ),
        ),
    )
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _update_step_skip_if(recipe: Recipe, command: UpdateStepSkipIfCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    updated = replace(step, skip_if=tuple(command.skip_if))
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _update_step_verify(recipe: Recipe, command: UpdateStepVerifyCommand) -> Recipe:
    step = _require_known_step(recipe, command.step_id)
    updated = replace(step, verify=tuple(command.verify))
    return replace(recipe, steps=_replace_step(recipe.steps, command.step_id, updated))


def _replace_step(steps: tuple[Step, ...], step_id: str, updated_step: Step) -> tuple[Step, ...]:
    _step_index(steps, step_id)
    return tuple(updated_step if step.id == step_id else step for step in steps)


def _require_known_step(recipe: Recipe, step_id: str) -> Step:
    for step in recipe.steps:
        if step.id == step_id:
            return step
    raise ValueError(f"Unknown step {step_id!r}.")


def _step_index(steps: tuple[Step, ...], step_id: str) -> int:
    for index, step in enumerate(steps):
        if step.id == step_id:
            return index
    raise ValueError(f"Unknown step {step_id!r}.")


def _normalize_step_params(
    step_type: str,
    params: Mapping[str, AuthoredParamValue],
) -> dict[str, AuthoredParamValue]:
    normalized = dict(params)
    plugin = builtin_step_registry().get(step_type)
    if plugin is None:
        return normalized
    for param_name, param_spec in plugin.spec.params.items():
        if param_name not in normalized or param_spec.default is None:
            continue
        if _param_value_equals_default(normalized[param_name], param_spec.default):
            del normalized[param_name]
    return normalized


def _param_value_equals_default(value: AuthoredParamValue, default: object) -> bool:
    if isinstance(value, RefParamValue):
        return False
    if hasattr(value, "value"):
        return getattr(value, "value") == default
    return value == default


def _rewrite_step_refs(step: Step, target_kind: Literal["input", "artifact", "artifact_group", "step"], old_id: str, new_id: str) -> Step:
    updated = step
    params = dict(step.params)
    changed_params = False
    for param_name, value in step.params.items():
        if not _is_supported_step_param(step, param_name):
            continue
        if isinstance(value, RefParamValue):
            rewritten_ref = _rewrite_ref(value.ref, target_kind, old_id, new_id)
            if rewritten_ref is not None and rewritten_ref != value.ref:
                params[param_name] = replace(value, ref=rewritten_ref)
                changed_params = True
            continue
        if target_kind == "artifact" and param_name == "artifacts":
            rewritten_value, changed = _rewrite_string_sequence_value(value, old_id, new_id)
            if changed:
                params[param_name] = rewritten_value
                changed_params = True
            continue
        if target_kind == "artifact_group" and param_name == "artifact_groups":
            rewritten_value, changed = _rewrite_string_sequence_value(value, old_id, new_id)
            if changed:
                params[param_name] = rewritten_value
                changed_params = True
    if changed_params:
        updated = replace(updated, params=params)
    if target_kind == "step":
        dependencies = tuple(new_id if dependency == old_id else dependency for dependency in updated.dependencies)
        conflicts = tuple(new_id if conflict == old_id else conflict for conflict in updated.constraints.conflicts_with)
        if dependencies != updated.dependencies or conflicts != updated.constraints.conflicts_with:
            updated = replace(
                updated,
                dependencies=dependencies,
                constraints=replace(updated.constraints, conflicts_with=conflicts),
            )
    return updated


def _remove_step_refs(step: Step, target_kind: Literal["input", "artifact", "step"], target_id: str) -> Step:
    updated = step
    params = dict(step.params)
    changed_params = False
    for param_name, value in step.params.items():
        if not _is_supported_step_param(step, param_name):
            continue
        if isinstance(value, RefParamValue):
            if _ref_matches(value.ref, target_kind, target_id):
                del params[param_name]
                changed_params = True
            continue
        if target_kind == "artifact" and param_name == "artifacts":
            rewritten_value, changed = _remove_string_sequence_value(value, target_id)
            if changed:
                params[param_name] = rewritten_value
                changed_params = True
    if changed_params:
        updated = replace(updated, params=params)
    if target_kind == "step":
        dependencies = tuple(dependency for dependency in updated.dependencies if dependency != target_id)
        conflicts = tuple(conflict for conflict in updated.constraints.conflicts_with if conflict != target_id)
        if dependencies != updated.dependencies or conflicts != updated.constraints.conflicts_with:
            updated = replace(
                updated,
                dependencies=dependencies,
                constraints=replace(updated.constraints, conflicts_with=conflicts),
            )
    return updated


def _rewrite_artifact_group_selection(step: Step, group_id: str, new_group_id: str) -> Step:
    params = dict(step.params)
    value = params.get("artifact_groups")
    if not _is_supported_step_param(step, "artifact_groups") or value is None:
        return step
    rewritten_value, changed = _rewrite_string_sequence_value(value, group_id, new_group_id)
    if not changed:
        return step
    params["artifact_groups"] = rewritten_value
    return replace(step, params=params)


def _remove_artifact_group_selection(step: Step, group_id: str) -> Step:
    params = dict(step.params)
    value = params.get("artifact_groups")
    if not _is_supported_step_param(step, "artifact_groups") or value is None:
        return step
    rewritten_value, changed = _remove_string_sequence_value(value, group_id)
    if not changed:
        return step
    params["artifact_groups"] = rewritten_value
    return replace(step, params=params)


def _rewrite_ref(ref: str, target_kind: Literal["input", "artifact", "artifact_group", "step"], old_id: str, new_id: str) -> str | None:
    try:
        parsed = parse_reference(ref)
    except ValueError:
        return None
    if target_kind == "input" and parsed.kind is RefKind.INPUT and parsed.target_id == old_id:
        return f"inputs.{new_id}"
    if target_kind == "artifact" and parsed.kind is RefKind.ARTIFACT_FIELD and parsed.target_id == old_id:
        return f"artifacts.{new_id}.{parsed.field}"
    if target_kind == "step" and parsed.target_id == old_id:
        if parsed.kind is RefKind.STEP_SHORTHAND:
            return f"steps.{new_id}"
        if parsed.kind is RefKind.STEP_OUTPUT:
            return f"steps.{new_id}.outputs.{parsed.field}"
    return None


def _ref_matches(ref: str, target_kind: Literal["input", "artifact", "step"], target_id: str) -> bool:
    return _rewrite_ref(ref, target_kind, target_id, target_id) is not None


def _is_supported_step_param(step: Step, param_name: str) -> bool:
    plugin = builtin_step_registry().get(step.type)
    return plugin is not None and param_name in plugin.spec.params


def _rewrite_string_sequence_value(value: object, old_id: str, new_id: str) -> tuple[object, bool]:
    values = _coerce_string_sequence(value)
    if values is None:
        return value, False
    rewritten = [new_id if item == old_id else item for item in values]
    return rewritten, rewritten != values


def _remove_string_sequence_value(value: object, target_id: str) -> tuple[object, bool]:
    values = _coerce_string_sequence(value)
    if values is None:
        return value, False
    rewritten = [item for item in values if item != target_id]
    return rewritten, rewritten != values


def _coerce_string_sequence(value: object) -> list[str] | None:
    if hasattr(value, "value"):
        value = getattr(value, "value")
    if isinstance(value, (list, tuple)):
        return [str(item) for item in value]
    return None


def _rename_mapping_key(mapping, old_key: str, new_key: str, new_value):
    _require_known_mapping_key(mapping, old_key, label="mapping entry")
    return {new_key if item_key == old_key else item_key: (new_value if item_key == old_key else item_value) for item_key, item_value in mapping.items()}


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


def _coerce_supported_step_type(value: object) -> str:
    step_type = str(value)
    if step_type not in builtin_step_registry():
        raise ValueError(f"Unsupported step type {step_type!r}.")
    return step_type


def _normalize_identifier_tuple(values: tuple[str, ...], *, label: str) -> tuple[str, ...]:
    normalized = tuple(_normalize_identifier(value, label=label) for value in values)
    if len(set(normalized)) != len(normalized):
        raise ValueError(f"{label}s must be unique.")
    return normalized


def _coerce_allowed_extensions(value: object) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        parts = [item.strip() for item in value.split(",")]
        return tuple(item for item in parts if item)
    if isinstance(value, (list, tuple)):
        return tuple(_normalize_identifier(item, label="extension") for item in value if str(item).strip())
    raise ValueError("allowed_extensions must be a string, list, or tuple.")

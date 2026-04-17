"""Dedicated authored YAML validation helpers."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from emuchef.domain import (
    AppDefinition,
    DevicePlan,
    DeviceProfile,
    ErrorCode,
    ErrorMessage,
    InputDeclaration,
    Recipe,
    ValidationResult,
    ValidationStatus,
    WarningCode,
    WarningMessage,
)
from emuchef.planner.bindings import PlannerOverrideProblemKind, normalize_planner_overrides
from emuchef.planner.catalog import CatalogLoadError
from emuchef.planner.contracts import validate_step_contract, validate_step_references
from emuchef.planner.dependencies import expand_recipe_dependencies, validate_recipe_step_cycles

from .loader import (
    _load_yaml,
    _namespaced_inputs,
    _parse_app_definition,
    _parse_device_plan,
    _parse_device_profile,
    _parse_recipe,
)

_DIR_KINDS: tuple[tuple[str, str], ...] = (
    ("apps", "app_definition"),
    ("recipes", "recipe"),
    ("device_profiles", "device_profile"),
    ("device_plans", "device_plan"),
)

_PARSERS = {
    "app_definition": _parse_app_definition,
    "recipe": _parse_recipe,
    "device_profile": _parse_device_profile,
    "device_plan": _parse_device_plan,
}


@dataclass(frozen=True, slots=True)
class _ParsedItem:
    path: Path
    kind: str
    item: AppDefinition | Recipe | DeviceProfile | DevicePlan


@dataclass(frozen=True, slots=True)
class _ValidationCatalog:
    root_path: Path
    apps: dict[str, AppDefinition]
    recipes: dict[str, Recipe]
    device_profiles: dict[str, DeviceProfile]
    device_plans: dict[str, DevicePlan]
    binding_inputs: dict[str, InputDeclaration]
    object_files: dict[tuple[str, str], Path]
    paths: tuple[Path, ...]
    errors: tuple[ErrorMessage, ...]


def validate_authored_catalog(root: str | Path) -> ValidationResult:
    catalog = _collect_catalog(Path(root))
    errors = list(catalog.errors)
    errors.extend(_validate_catalog_cross_refs(catalog))
    return ValidationResult(
        status=_derive_status((), tuple(errors)),
        warnings=(),
        errors=tuple(errors),
        validated_paths=tuple(str(path.resolve()) for path in catalog.paths),
    )


def validate_authored_recipe(
    recipe: Recipe,
    *,
    path: str | Path,
    authored_root: str | Path | None = None,
) -> ValidationResult:
    recipe_path = Path(path).resolve()
    warnings: list[WarningMessage] = []
    errors = list(_validate_recipe_locally(recipe, file=recipe_path))

    if authored_root is None:
        warnings.append(
            WarningMessage(
                code=WarningCode.VALIDATION_CONTEXT_LIMITED,
                message="Cross-file validation was limited because no authored_root was provided.",
                details=_issue_details(
                    file=recipe_path,
                    object_kind="recipe",
                    object_id=recipe.id,
                ),
            )
        )
    else:
        catalog = _collect_catalog(Path(authored_root))
        errors.extend(
            _validate_item_with_catalog_context(
                _ParsedItem(path=recipe_path, kind="recipe", item=recipe),
                catalog,
            )
        )

    return ValidationResult(
        status=_derive_status(tuple(warnings), tuple(errors)),
        warnings=tuple(warnings),
        errors=tuple(errors),
        validated_paths=(str(recipe_path),),
    )


def validate_authored_path(path: str | Path, authored_root: str | Path | None = None) -> ValidationResult:
    target_path = Path(path)
    parsed_item, local_errors = _validate_single_file(target_path)
    warnings: list[WarningMessage] = []
    errors = list(local_errors)

    if authored_root is None:
        warnings.append(
            WarningMessage(
                code=WarningCode.VALIDATION_CONTEXT_LIMITED,
                message="Cross-file validation was limited because no authored_root was provided.",
                details=_issue_details(
                    file=target_path.resolve(),
                    object_kind=parsed_item.kind if parsed_item is not None else None,
                    object_id=parsed_item.item.id if parsed_item is not None else None,
                ),
            )
        )
    elif parsed_item is not None:
        catalog = _collect_catalog(Path(authored_root))
        errors.extend(_validate_item_with_catalog_context(parsed_item, catalog))

    return ValidationResult(
        status=_derive_status(tuple(warnings), tuple(errors)),
        warnings=tuple(warnings),
        errors=tuple(errors),
        validated_paths=(str(target_path.resolve()),),
    )


def _collect_catalog(root_path: Path) -> _ValidationCatalog:
    apps: dict[str, AppDefinition] = {}
    recipes: dict[str, Recipe] = {}
    device_profiles: dict[str, DeviceProfile] = {}
    device_plans: dict[str, DevicePlan] = {}
    object_files: dict[tuple[str, str], Path] = {}
    paths: list[Path] = []
    errors: list[ErrorMessage] = []

    for directory_name, expected_kind in _DIR_KINDS:
        directory = root_path / directory_name
        for path in sorted(directory.glob("*.y*ml")):
            paths.append(path.resolve())
            parsed_item, item_errors = _validate_single_file(path, expected_kind=expected_kind)
            errors.extend(item_errors)
            if parsed_item is None:
                continue
            destination = {
                "app_definition": apps,
                "recipe": recipes,
                "device_profile": device_profiles,
                "device_plan": device_plans,
            }[parsed_item.kind]
            item_id = parsed_item.item.id
            if item_id in destination:
                errors.append(
                    _error(
                        code={
                            "app_definition": ErrorCode.APP_ID_CONFLICT,
                            "recipe": ErrorCode.RECIPE_ID_CONFLICT,
                            "device_profile": ErrorCode.DEVICE_PROFILE_ID_CONFLICT,
                            "device_plan": ErrorCode.DEVICE_PLAN_ID_CONFLICT,
                        }[parsed_item.kind],
                        message=f"Duplicate {parsed_item.kind} id {item_id!r}.",
                        file=path.resolve(),
                        object_kind=parsed_item.kind,
                        object_id=item_id,
                        field="id",
                    )
                )
                continue
            destination[item_id] = parsed_item.item
            object_files[(parsed_item.kind, item_id)] = parsed_item.path

    binding_inputs, binding_errors = _build_binding_inputs(apps, recipes, object_files)
    errors.extend(binding_errors)
    return _ValidationCatalog(
        root_path=root_path.resolve(),
        apps=apps,
        recipes=recipes,
        device_profiles=device_profiles,
        device_plans=device_plans,
        binding_inputs=binding_inputs,
        object_files=object_files,
        paths=tuple(paths),
        errors=tuple(errors),
    )


def _validate_single_file(path: Path, expected_kind: str | None = None) -> tuple[_ParsedItem | None, tuple[ErrorMessage, ...]]:
    raw, raw_errors = _load_raw_mapping(path)
    if raw is None:
        return None, raw_errors

    kind = raw.get("kind")
    if kind not in _PARSERS:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=(
                    f"File {path.name!r} has invalid kind {kind!r}. "
                    f"Expected one of: {', '.join(sorted(_PARSERS))}."
                ),
                file=path.resolve(),
                object_kind=str(kind) if kind is not None else None,
                object_id=raw.get("id"),
                field="kind",
                kind=kind,
            ),
        )

    if int(raw.get("schema_version", -1)) != 1:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {path.name!r} has unsupported schema_version {raw.get('schema_version')!r}.",
                file=path.resolve(),
                object_kind=str(kind),
                object_id=raw.get("id"),
                field="schema_version",
                schema_version=raw.get("schema_version"),
            ),
        )

    if expected_kind is not None and kind != expected_kind:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {path.name!r} has kind {kind!r}, expected {expected_kind!r}.",
                file=path.resolve(),
                object_kind=str(kind),
                object_id=raw.get("id"),
                field="kind",
                kind=kind,
                expected_kind=expected_kind,
            ),
        )

    try:
        item = _PARSERS[kind](raw)
    except (KeyError, TypeError, ValueError) as exc:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {path.name!r} has an invalid schema shape: {exc}.",
                file=path.resolve(),
                object_kind=str(kind),
                object_id=raw.get("id"),
            ),
        )

    errors: list[ErrorMessage] = []
    if isinstance(item, Recipe):
        errors.extend(_validate_recipe_locally(item, file=path.resolve()))

    return _ParsedItem(path=path.resolve(), kind=kind, item=item), tuple(errors)


def _load_raw_mapping(path: Path) -> tuple[Mapping[str, Any] | None, tuple[ErrorMessage, ...]]:
    try:
        return _load_yaml(path), ()
    except FileNotFoundError:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {path!s} was not found.",
                file=path.resolve(),
            ),
        )
    except CatalogLoadError as exc:
        return None, tuple(_with_context(error, file=path.resolve()) for error in exc.errors)
    except yaml.YAMLError as exc:
        return None, (
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {path.name!r} could not be parsed as YAML: {exc}.",
                file=path.resolve(),
            ),
        )


def _validate_recipe_locally(recipe: Recipe, *, file: Path) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []
    errors.extend(_annotate_recipe_step_cycle_errors(file, recipe, validate_recipe_step_cycles(recipe)))
    for step_index, step in enumerate(recipe.steps):
        errors.extend(_annotate_step_contract_errors(file, recipe, step_index, validate_step_contract(recipe.id, step, recipe)))
        errors.extend(_annotate_step_contract_errors(file, recipe, step_index, validate_step_references(recipe, step)))
    return tuple(errors)


def _build_binding_inputs(
    apps: Mapping[str, AppDefinition],
    recipes: Mapping[str, Recipe],
    object_files: Mapping[tuple[str, str], Path],
) -> tuple[dict[str, InputDeclaration], tuple[ErrorMessage, ...]]:
    binding_inputs: dict[str, InputDeclaration] = {}
    errors: list[ErrorMessage] = []

    for input_id, declaration in _namespaced_inputs("app", apps).items():
        if input_id in binding_inputs:
            app = apps[input_id.split("/", 1)[0]]
            errors.append(
                _error(
                    code=ErrorCode.BINDING_REF_CONFLICT,
                    message=f"Binding ref {input_id!r} is declared more than once.",
                    file=object_files.get(("app_definition", app.id), Path(".")),
                    object_kind="app_definition",
                    object_id=app.id,
                    field=_input_field(app, declaration.id),
                    binding_ref=input_id,
                )
            )
            continue
        binding_inputs[input_id] = declaration

    for input_id, declaration in _namespaced_inputs("recipe", recipes).items():
        if input_id in binding_inputs:
            recipe = recipes[input_id.split("/", 1)[0]]
            errors.append(
                _error(
                    code=ErrorCode.BINDING_REF_CONFLICT,
                    message=f"Binding ref {input_id!r} is declared more than once.",
                    file=object_files.get(("recipe", recipe.id), Path(".")),
                    object_kind="recipe",
                    object_id=recipe.id,
                    field=_input_field(recipe, declaration.id),
                    binding_ref=input_id,
                )
            )
            continue
        binding_inputs[input_id] = declaration

    return binding_inputs, tuple(errors)


def _validate_catalog_cross_refs(catalog: _ValidationCatalog) -> tuple[ErrorMessage, ...]:
    errors: list[ErrorMessage] = []

    for recipe in catalog.recipes.values():
        for dependency_ref in recipe.recipe_dependencies:
            if dependency_ref not in catalog.recipes:
                errors.append(
                    _error(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe dependency {dependency_ref!r} was not found.",
                        file=catalog.object_files[("recipe", recipe.id)],
                        object_kind="recipe",
                        object_id=recipe.id,
                        field=_recipe_dependency_field(recipe, dependency_ref),
                        recipe_ref=recipe.id,
                        dependency_ref=dependency_ref,
                    )
                )
    errors.extend(_validate_execution_step_id_uniqueness(catalog.recipes, catalog.object_files))
    errors.extend(_validate_recipe_dependency_cycles(catalog.recipes))

    for device_plan in catalog.device_plans.values():
        if device_plan.device_profile_ref not in catalog.device_profiles:
            errors.append(
                _error(
                    code=ErrorCode.DEVICE_PROFILE_NOT_FOUND,
                    message=f"Device profile {device_plan.device_profile_ref!r} was not found.",
                    file=catalog.object_files[("device_plan", device_plan.id)],
                    object_kind="device_plan",
                    object_id=device_plan.id,
                    field="device_profile_ref",
                    device_plan_ref=device_plan.id,
                    device_profile_ref=device_plan.device_profile_ref,
                )
            )
        for recipe_selection in device_plan.recipes:
            if recipe_selection.recipe_ref not in catalog.recipes:
                errors.append(
                    _error(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe {recipe_selection.recipe_ref!r} referenced by a device plan was not found.",
                        file=catalog.object_files[("device_plan", device_plan.id)],
                        object_kind="device_plan",
                        object_id=device_plan.id,
                        field=_device_plan_recipe_field(device_plan, recipe_selection.recipe_ref),
                        device_plan_ref=device_plan.id,
                        recipe_ref=recipe_selection.recipe_ref,
                    )
                )
        errors.extend(
            _validate_device_plan_override_refs(
                device_plan,
                catalog.binding_inputs,
                file=catalog.object_files[("device_plan", device_plan.id)],
            )
        )

    # The current authored schema does not define an explicit app_ref field on steps.
    return tuple(errors)


def _validate_item_with_catalog_context(parsed_item: _ParsedItem, catalog: _ValidationCatalog) -> tuple[ErrorMessage, ...]:
    if parsed_item.kind == "recipe":
        recipe_maps = dict(catalog.recipes)
        object_files = dict(catalog.object_files)
        replaced_recipe_id = _find_recipe_id_for_path(catalog.object_files, parsed_item.path)
        if replaced_recipe_id is not None:
            recipe_maps.pop(replaced_recipe_id, None)
            object_files.pop(("recipe", replaced_recipe_id), None)

        errors: list[ErrorMessage] = []
        existing_path = object_files.get(("recipe", parsed_item.item.id))
        if existing_path is not None and existing_path != parsed_item.path:
            errors.append(
                _error(
                    code=ErrorCode.RECIPE_ID_CONFLICT,
                    message=f"Duplicate recipe id {parsed_item.item.id!r}.",
                    file=parsed_item.path,
                    object_kind="recipe",
                    object_id=parsed_item.item.id,
                    field="id",
                )
            )

        recipe_maps[parsed_item.item.id] = parsed_item.item
        object_files[("recipe", parsed_item.item.id)] = parsed_item.path
        _, binding_errors = _build_binding_inputs(catalog.apps, recipe_maps, object_files)
        errors.extend(binding_errors)
        recipe = parsed_item.item
        for dependency_ref in recipe.recipe_dependencies:
            if dependency_ref not in recipe_maps:
                errors.append(
                    _error(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe dependency {dependency_ref!r} was not found.",
                        file=parsed_item.path,
                        object_kind="recipe",
                        object_id=recipe.id,
                        field=_recipe_dependency_field(recipe, dependency_ref),
                        recipe_ref=recipe.id,
                        dependency_ref=dependency_ref,
                    )
                )
        errors.extend(
            _annotate_recipe_cycle_catalog_errors(
                parsed_item.path,
                recipe.id,
                _validate_recipe_dependency_cycles(recipe_maps, selected_recipe_refs=(recipe.id,)),
            )
        )
        return tuple(errors)

    if parsed_item.kind == "device_plan":
        device_profiles = dict(catalog.device_profiles)
        recipes = dict(catalog.recipes)
        device_plan = parsed_item.item
        errors: list[ErrorMessage] = []
        if device_plan.device_profile_ref not in device_profiles:
            errors.append(
                _error(
                    code=ErrorCode.DEVICE_PROFILE_NOT_FOUND,
                    message=f"Device profile {device_plan.device_profile_ref!r} was not found.",
                    file=parsed_item.path,
                    object_kind="device_plan",
                    object_id=device_plan.id,
                    field="device_profile_ref",
                    device_plan_ref=device_plan.id,
                    device_profile_ref=device_plan.device_profile_ref,
                )
            )
        for recipe_selection in device_plan.recipes:
            if recipe_selection.recipe_ref not in recipes:
                errors.append(
                    _error(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe {recipe_selection.recipe_ref!r} referenced by a device plan was not found.",
                        file=parsed_item.path,
                        object_kind="device_plan",
                        object_id=device_plan.id,
                        field=_device_plan_recipe_field(device_plan, recipe_selection.recipe_ref),
                        device_plan_ref=device_plan.id,
                        recipe_ref=recipe_selection.recipe_ref,
                    )
                )
        errors.extend(
            _validate_device_plan_override_refs(
                device_plan,
                catalog.binding_inputs,
                file=parsed_item.path,
            )
        )
        return tuple(errors)

    return ()


def _find_recipe_id_for_path(object_files: Mapping[tuple[str, str], Path], path: Path) -> str | None:
    target_path = path.resolve()
    for (kind, object_id), object_path in object_files.items():
        if kind == "recipe" and object_path.resolve() == target_path:
            return object_id
    return None


def _validate_execution_step_id_uniqueness(
    recipes: Mapping[str, Recipe],
    object_files: Mapping[tuple[str, str], Path],
) -> tuple[ErrorMessage, ...]:
    seen_execution_step_ids: set[str] = set()
    errors: list[ErrorMessage] = []
    for recipe in recipes.values():
        for step in recipe.steps:
            execution_step_id = f"{recipe.id}/{step.id}"
            if execution_step_id in seen_execution_step_ids:
                errors.append(
                    _error(
                        code=ErrorCode.STEP_ID_CONFLICT,
                        message=f"Execution step id {execution_step_id!r} is duplicated after namespacing.",
                        file=object_files.get(("recipe", recipe.id), Path(".")),
                        object_kind="recipe",
                        object_id=recipe.id,
                        field=_step_field(recipe, step.id, "id"),
                        step_id=execution_step_id,
                    )
                )
            seen_execution_step_ids.add(execution_step_id)
    return tuple(errors)


def _validate_recipe_dependency_cycles(
    recipes: Mapping[str, Recipe],
    selected_recipe_refs: tuple[str, ...] | None = None,
) -> tuple[ErrorMessage, ...]:
    selected = selected_recipe_refs or tuple(recipes)
    if not selected:
        return ()
    from emuchef.planner.catalog import AuthoredCatalog

    binding_inputs, _ = _build_binding_inputs({}, recipes, {})
    catalog = AuthoredCatalog(
        root_path=Path("."),
        apps={},
        recipes=recipes,
        device_profiles={},
        device_plans={},
        binding_inputs=binding_inputs,
        recipe_artifacts={},
    )
    _, errors = expand_recipe_dependencies(catalog, selected)
    return tuple(error for error in errors if error.code is ErrorCode.DEPENDENCY_CYCLE)


def _derive_status(
    warnings: tuple[WarningMessage, ...],
    errors: tuple[ErrorMessage, ...],
) -> ValidationStatus:
    if errors:
        return ValidationStatus.ERROR
    if warnings:
        return ValidationStatus.WARNING
    return ValidationStatus.SUCCESS


def _error(
    *,
    code: ErrorCode,
    message: str,
    file: Path,
    object_kind: str | None = None,
    object_id: str | None = None,
    field: str | None = None,
    **extra_details: Any,
) -> ErrorMessage:
    return ErrorMessage(
        code=code,
        message=message,
        details=_issue_details(
            file=file,
            object_kind=object_kind,
            object_id=object_id,
            field=field,
            **extra_details,
        ),
    )


def _issue_details(
    *,
    file: Path,
    object_kind: str | None = None,
    object_id: str | None = None,
    field: str | None = None,
    **extra_details: Any,
) -> dict[str, Any]:
    details: dict[str, Any] = {
        "file": str(file),
        "object_kind": object_kind,
        "object_id": object_id,
        "field": field,
    }
    details.update(extra_details)
    return details


def _with_context(
    error: ErrorMessage,
    *,
    file: Path,
    object_kind: str | None = None,
    object_id: str | None = None,
    field: str | None = None,
) -> ErrorMessage:
    details = dict(error.details)
    details.setdefault("file", str(file))
    details.setdefault("object_kind", object_kind)
    details.setdefault("object_id", object_id)
    details.setdefault("field", field)
    return ErrorMessage(code=error.code, message=error.message, details=details)


def _annotate_step_contract_errors(
    file: Path,
    recipe: Recipe,
    step_index: int,
    errors: tuple[ErrorMessage, ...],
) -> tuple[ErrorMessage, ...]:
    annotated: list[ErrorMessage] = []
    for error in errors:
        param = error.details.get("param")
        field = _step_field(recipe, recipe.steps[step_index].id, f"params.{param}") if param else None
        annotated.append(
            _with_context(
                error,
                file=file,
                object_kind="recipe",
                object_id=recipe.id,
                field=field,
            )
        )
    return tuple(annotated)


def _annotate_recipe_step_cycle_errors(
    file: Path,
    recipe: Recipe,
    errors: tuple[ErrorMessage, ...],
) -> tuple[ErrorMessage, ...]:
    return tuple(
        _with_context(error, file=file, object_kind="recipe", object_id=recipe.id, field="steps")
        for error in errors
    )


def _annotate_recipe_cycle_catalog_errors(
    file: Path,
    recipe_id: str,
    errors: tuple[ErrorMessage, ...],
) -> tuple[ErrorMessage, ...]:
    return tuple(
        _with_context(error, file=file, object_kind="recipe", object_id=recipe_id, field="recipe_dependencies")
        for error in errors
    )


def _annotate_recipe_permission_step_errors(
    file: Path,
    recipe: Recipe,
    errors: tuple[ErrorMessage, ...],
) -> tuple[ErrorMessage, ...]:
    annotated: list[ErrorMessage] = []
    for error in errors:
        step_id = error.details.get("step_id")
        field = _step_field(recipe, str(step_id), "type") if isinstance(step_id, str) else "permissions"
        annotated.append(_with_context(error, file=file, object_kind="recipe", object_id=recipe.id, field=field))
    return tuple(annotated)


def _recipe_dependency_field(recipe: Recipe, dependency_ref: str) -> str | None:
    try:
        index = recipe.recipe_dependencies.index(dependency_ref)
    except ValueError:
        return "recipe_dependencies"
    return f"recipe_dependencies[{index}]"


def _device_plan_recipe_field(device_plan: DevicePlan, recipe_ref: str) -> str | None:
    for index, selection in enumerate(device_plan.recipes):
        if selection.recipe_ref == recipe_ref:
            return f"recipes[{index}].recipe_ref"
    return "recipes"


def _validate_device_plan_override_refs(
    device_plan: DevicePlan,
    binding_inputs: Mapping[str, InputDeclaration],
    *,
    file: Path,
) -> tuple[ErrorMessage, ...]:
    _, problems = normalize_planner_overrides(
        device_plan.overrides,
        binding_inputs,
        allow_metadata_keys=True,
    )
    errors: list[ErrorMessage] = []
    for problem in problems:
        field = _device_plan_override_field(problem.key)
        if problem.kind is PlannerOverrideProblemKind.UNKNOWN_BINDING:
            binding_ref = problem.binding_ref or str(problem.key)
            errors.append(
                _error(
                    code=ErrorCode.BINDING_MISSING,
                    message=f"Device plan override {binding_ref!r} references unknown binding {binding_ref!r}.",
                    file=file,
                    object_kind="device_plan",
                    object_id=device_plan.id,
                    field=field,
                    device_plan_ref=device_plan.id,
                    override_key=str(problem.key),
                    binding_ref=binding_ref,
                )
            )
            continue
        errors.append(
            _error(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=(
                    f"Device plan override {problem.key!r} must be a normalized input ref "
                    "(inputs.<id> or <recipe>/<input>)."
                ),
                file=file,
                object_kind="device_plan",
                object_id=device_plan.id,
                field=field,
                device_plan_ref=device_plan.id,
                override_key=str(problem.key),
            )
        )
    return tuple(errors)


def _device_plan_override_field(override_key: object) -> str:
    if not isinstance(override_key, str):
        return "overrides"
    return f"overrides.{override_key}"


def _binding_field(recipe: Recipe, step, step_index: int, binding_ref: str) -> str | None:
    for param_name, value in step.params.items():
        if getattr(value, "ref", None) is not None and value.ref.full == binding_ref:
            return f"steps[{step_index}].params.{param_name}.ref"
    return f"steps[{step_index}].params"


def _step_index(recipe: Recipe, step_id: str) -> int:
    for index, step in enumerate(recipe.steps):
        if step.id == step_id:
            return index
    return 0


def _step_field(recipe: Recipe, step_id: str, suffix: str) -> str:
    return f"steps[{_step_index(recipe, step_id)}].{suffix}"


def _input_field(item: AppDefinition | Recipe, input_id: str) -> str:
    if isinstance(item.inputs, Mapping):
        for declared_id in item.inputs.keys():
            if declared_id == input_id:
                return f"inputs.{declared_id}"
        return "inputs"
    for index, declaration in enumerate(item.inputs):
        if declaration.id == input_id:
            return f"inputs[{index}].id"
    return "inputs"

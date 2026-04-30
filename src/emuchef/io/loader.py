"""Authored YAML loading and validation."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Any

import yaml

from emuchef.domain import (
    AppArtifactSupport,
    AppArtifacts,
    ArtifactCacheMode,
    AppConfigTarget,
    AppDefinition,
    AppInstallSource,
    AppPackage,
    AppProvisioning,
    AppTrackingSource,
    DeviceMatchCriteria,
    DevicePlan,
    DevicePlanRecipeSelection,
    DeviceProfile,
    ErrorCode,
    ErrorMessage,
    InputDeclaration,
    InputRole,
    InputType,
    InputValidation,
    LiteralParamValue,
    Recipe,
    RecipeProvides,
    RefParamValue,
    RemoteFileArtifact,
    RuntimeCapabilities,
    Step,
    StepCondition,
    StepConstraints,
    StepType,
    parse_reference,
)
from emuchef.planner.catalog import AuthoredCatalog, CatalogLoadError
from emuchef.planner.bindings import PlannerOverrideProblemKind, normalize_planner_overrides
from emuchef.planner.contracts import (
    validate_step_contract,
    validate_step_references,
)
from emuchef.planner.dependencies import validate_recipe_step_cycles


def load_authored_recipe(path: str | Path) -> Recipe:
    recipe_path = Path(path)
    data = _load_yaml(recipe_path)

    errors: list[ErrorMessage] = []
    try:
        schema_version = int(data.get("schema_version", -1))
    except (TypeError, ValueError):
        schema_version = -1
    if schema_version != 1:
        errors.append(
            ErrorMessage(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {recipe_path.name!r} has unsupported schema_version {data.get('schema_version')!r}.",
                details={"path": str(recipe_path), "schema_version": data.get("schema_version")},
            )
        )
    if data.get("kind") != "recipe":
        errors.append(
            ErrorMessage(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"File {recipe_path.name!r} has kind {data.get('kind')!r}, expected 'recipe'.",
                details={"path": str(recipe_path), "kind": data.get("kind"), "expected_kind": "recipe"},
            )
        )
    if errors:
        raise CatalogLoadError(tuple(errors))

    try:
        return _parse_recipe(data)
    except (KeyError, TypeError, ValueError) as exc:
        raise CatalogLoadError(
            (
                ErrorMessage(
                    code=ErrorCode.AUTHORED_DATA_INVALID,
                    message=f"File {recipe_path.name!r} has an invalid schema shape: {exc}.",
                    details={"path": str(recipe_path)},
                ),
            )
        ) from exc


def load_authored_catalog(root: str | Path) -> AuthoredCatalog:
    root_path = Path(root)
    apps = _load_directory(root_path / "apps", "app_definition", _parse_app_definition)
    recipes = _load_directory(root_path / "recipes", "recipe", _parse_recipe)
    device_profiles = _load_directory(root_path / "device_profiles", "device_profile", _parse_device_profile)
    device_plans = _load_directory(root_path / "device_plans", "device_plan", _parse_device_plan)

    errors: list[ErrorMessage] = []
    binding_inputs: dict[str, InputDeclaration] = {}
    recipe_artifacts = _namespaced_artifacts(recipes)

    for input_id, declaration in _namespaced_inputs("app", apps).items():
        if input_id in binding_inputs:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_REF_CONFLICT,
                    message=f"Binding ref {input_id!r} is declared more than once.",
                    details={"binding_ref": input_id},
                )
            )
        binding_inputs[input_id] = declaration

    for input_id, declaration in _namespaced_inputs("recipe", recipes).items():
        if input_id in binding_inputs:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_REF_CONFLICT,
                    message=f"Binding ref {input_id!r} is declared more than once.",
                    details={"binding_ref": input_id},
                )
            )
        binding_inputs[input_id] = declaration

    for recipe in recipes.values():
        for dependency_ref in recipe.recipe_dependencies:
            if dependency_ref not in recipes:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe dependency {dependency_ref!r} was not found.",
                        details={"recipe_ref": recipe.id, "dependency_ref": dependency_ref},
                    )
                )
        errors.extend(validate_recipe_step_cycles(recipe))
        for step in recipe.steps:
            errors.extend(validate_step_contract(recipe.id, step, recipe))
            errors.extend(validate_step_references(recipe, step))

    seen_execution_step_ids: set[str] = set()
    for recipe in recipes.values():
        for step in recipe.steps:
            execution_step_id = f"{recipe.id}/{step.id}"
            if execution_step_id in seen_execution_step_ids:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.STEP_ID_CONFLICT,
                        message=f"Execution step id {execution_step_id!r} is duplicated after namespacing.",
                        details={"step_id": execution_step_id},
                    )
                )
            seen_execution_step_ids.add(execution_step_id)

    for device_plan in device_plans.values():
        if device_plan.device_profile_ref not in device_profiles:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.DEVICE_PROFILE_NOT_FOUND,
                    message=f"Device profile {device_plan.device_profile_ref!r} was not found.",
                    details={"device_plan_ref": device_plan.id, "device_profile_ref": device_plan.device_profile_ref},
                )
            )
        for recipe_selection in device_plan.recipes:
            if recipe_selection.recipe_ref not in recipes:
                errors.append(
                    ErrorMessage(
                        code=ErrorCode.RECIPE_NOT_FOUND,
                        message=f"Recipe {recipe_selection.recipe_ref!r} referenced by a device plan was not found.",
                        details={"device_plan_ref": device_plan.id, "recipe_ref": recipe_selection.recipe_ref},
                    )
                )
        errors.extend(_validate_device_plan_overrides(device_plan, binding_inputs))

    if errors:
        raise CatalogLoadError(tuple(errors))

    return AuthoredCatalog(
        root_path=root_path.resolve(),
        apps=apps,
        recipes=recipes,
        device_profiles=device_profiles,
        device_plans=device_plans,
        binding_inputs=binding_inputs,
        recipe_artifacts=recipe_artifacts,
    )


def _load_directory(directory: Path, expected_kind: str, parser):
    items: dict[str, Any] = {}
    errors: list[ErrorMessage] = []
    for path in sorted(directory.glob("*.y*ml")):
        data = _load_yaml(path)
        if int(data.get("schema_version", -1)) != 1:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.AUTHORED_DATA_INVALID,
                    message=f"File {path.name!r} has unsupported schema_version {data.get('schema_version')!r}.",
                    details={"path": str(path), "schema_version": data.get("schema_version")},
                )
            )
            continue
        if data.get("kind") != expected_kind:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.AUTHORED_DATA_INVALID,
                    message=f"File {path.name!r} has kind {data.get('kind')!r}, expected {expected_kind!r}.",
                    details={"path": str(path), "kind": data.get("kind"), "expected_kind": expected_kind},
                )
            )
            continue
        try:
            item = parser(data)
        except (KeyError, TypeError, ValueError) as exc:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.AUTHORED_DATA_INVALID,
                    message=f"File {path.name!r} has an invalid schema shape: {exc}.",
                    details={"path": str(path)},
                )
            )
            continue
        item_id = item.id
        if item_id in items:
            code = {
                "app_definition": ErrorCode.APP_ID_CONFLICT,
                "recipe": ErrorCode.RECIPE_ID_CONFLICT,
                "device_profile": ErrorCode.DEVICE_PROFILE_ID_CONFLICT,
                "device_plan": ErrorCode.DEVICE_PLAN_ID_CONFLICT,
            }[expected_kind]
            errors.append(
                ErrorMessage(
                    code=code,
                    message=f"Duplicate {expected_kind} id {item_id!r}.",
                    details={"id": item_id, "path": str(path)},
                )
            )
            continue
        items[item_id] = item
    if errors:
        raise CatalogLoadError(tuple(errors))
    return items


def _load_yaml(path: Path) -> Mapping[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        loaded = yaml.safe_load(handle) or {}
    if not isinstance(loaded, dict):
        raise CatalogLoadError(
            (
                ErrorMessage(
                    code=ErrorCode.AUTHORED_DATA_INVALID,
                    message=f"File {path.name!r} must contain a top-level mapping.",
                    details={"path": str(path)},
                ),
            )
        )
    return loaded


def _parse_app_definition(data: Mapping[str, Any]) -> AppDefinition:
    return AppDefinition(
        id=str(data["id"]),
        name=str(data["name"]),
        description=_optional_str(data.get("description")),
        category=_optional_str(data.get("category")),
        package=AppPackage(
            primary=str(data["package"]["primary"]),
            aliases=tuple(str(item) for item in data["package"].get("aliases", [])),
        ),
        install_source=AppInstallSource(
            type=str(data["install_source"]["type"]),
            resolver=str(data["install_source"]["resolver"]),
            options=dict(data["install_source"].get("options", {})),
        ),
        tracking_source=AppTrackingSource(
            type=str(data["tracking_source"]["type"]),
            config_snapshot=str(data["tracking_source"]["config_snapshot"]),
            app_id=str(data["tracking_source"]["app_id"]),
        ),
        artifacts=AppArtifacts(
            apk=_parse_artifact_support(data["artifacts"]["apk"]),
            shared_storage_config=_parse_optional_artifact(data["artifacts"].get("shared_storage_config")),
            app_data_config=_parse_optional_artifact(data["artifacts"].get("app_data_config")),
            byo_apk=_parse_optional_artifact(data["artifacts"].get("byo_apk")),
        ),
        provisioning=AppProvisioning(
            launch_once_recommended=bool(data["provisioning"].get("launch_once_recommended", False)),
            shared_storage_paths=tuple(str(item) for item in data["provisioning"].get("shared_storage_paths", [])),
            app_data_paths=tuple(str(item) for item in data["provisioning"].get("app_data_paths", [])),
            config_targets=tuple(
                AppConfigTarget(id=str(item["id"]), type=str(item["type"]), path=str(item["path"]))
                for item in data["provisioning"].get("config_targets", [])
            ),
        ),
        inputs=tuple(_parse_input_declaration(item) for item in data.get("inputs", [])),
        metadata=dict(data.get("metadata", {})),
        schema_version=int(data["schema_version"]),
        kind=str(data["kind"]),
    )


def _parse_input_declaration(data: Mapping[str, Any], *, input_id: str | None = None) -> InputDeclaration:
    validation = data.get("validation") or {}
    declared_id = input_id or str(data["id"])
    return InputDeclaration(
        id=declared_id,
        type=InputType(str(data["type"])),
        role=InputRole(str(data.get("role", InputRole.GENERIC.value))),
        label=str(data.get("label", declared_id)),
        description=_optional_str(data.get("description")),
        required=bool(data.get("required", True)),
        multiple=bool(data.get("multiple", False)),
        validation=InputValidation(
            must_exist=bool(validation.get("must_exist", False)),
            allowed_extensions=tuple(str(item) for item in validation.get("allowed_extensions", [])),
            path_kind=InputType(str(validation["path_kind"]))
            if validation.get("path_kind") is not None
            else None,
        ),
        default=data.get("default"),
        metadata=dict(data.get("metadata", {})),
    )


def _parse_step(data: Mapping[str, Any]) -> Step:
    return Step(
        id=str(data["id"]),
        type=StepType(str(data["type"])),
        name=str(data["name"]),
        description=_optional_str(data.get("description")),
        user_toggleable=bool(data["user_toggleable"]),
        dependencies=tuple(str(item) for item in data.get("dependencies", [])),
        constraints=StepConstraints(
            capabilities=tuple(str(item) for item in data.get("constraints", {}).get("capabilities", [])),
            conflicts_with=tuple(str(item) for item in data.get("constraints", {}).get("conflicts_with", [])),
        ),
        skip_if=tuple(_parse_step_condition(item) for item in data.get("skip_if", [])),
        params={str(key): _parse_param_value(value) for key, value in data.get("params", {}).items()},
        verify=tuple(_parse_step_condition(item) for item in data.get("verify", [])),
    )


def _parse_recipe(data: Mapping[str, Any]) -> Recipe:
    if "permissions" in data:
        raise ValueError("top-level 'permissions' is no longer supported; author permissions under grant_permissions.params")
    inputs_data = data.get("inputs", {})
    if inputs_data is None:
        inputs_data = {}
    if not isinstance(inputs_data, Mapping):
        raise ValueError("recipe inputs must be a mapping")
    artifacts_data = data.get("artifacts", {})
    if artifacts_data is None:
        artifacts_data = {}
    if not isinstance(artifacts_data, Mapping):
        raise ValueError("recipe artifacts must be a mapping")
    artifact_groups = data.get("artifact_groups", {})
    if artifact_groups is None:
        artifact_groups = {}
    if not isinstance(artifact_groups, Mapping):
        raise ValueError("recipe artifact_groups must be a mapping")
    return Recipe(
        id=str(data["id"]),
        name=str(data["name"]),
        description=_optional_str(data.get("description")),
        recipe_dependencies=tuple(str(item) for item in data.get("recipe_dependencies", [])),
        provides=RecipeProvides(features=tuple(str(item) for item in data.get("provides", {}).get("features", []))),
        inputs={str(key): _parse_input_declaration(value, input_id=str(key)) for key, value in inputs_data.items()},
        artifacts={str(key): _parse_artifact_definition(str(key), value) for key, value in artifacts_data.items()},
        artifact_groups={str(key): tuple(str(item) for item in value) for key, value in artifact_groups.items()},
        steps=tuple(_parse_step(item) for item in data.get("steps", [])),
        schema_version=int(data["schema_version"]),
        kind=str(data["kind"]),
    )


def _parse_device_profile(data: Mapping[str, Any]) -> DeviceProfile:
    android_version = data.get("match", {}).get("android_version")
    return DeviceProfile(
        id=str(data["id"]),
        name=str(data["name"]),
        description=_optional_str(data.get("description")),
        match=DeviceMatchCriteria(
            manufacturer_contains=tuple(str(item) for item in data.get("match", {}).get("manufacturer_contains", [])),
            brand_contains=tuple(str(item) for item in data.get("match", {}).get("brand_contains", [])),
            model_patterns=tuple(str(item) for item in data.get("match", {}).get("model_patterns", [])),
            android_version=None
            if android_version is None
            else _parse_android_version_range(android_version),
        ),
        capability_defaults=RuntimeCapabilities(
            adb_available=bool(data["capability_defaults"]["adb_available"]),
            apk_install=bool(data["capability_defaults"]["apk_install"]),
            shared_storage_write=bool(data["capability_defaults"]["shared_storage_write"]),
            app_launch=bool(data["capability_defaults"]["app_launch"]),
            shell_command=bool(data["capability_defaults"]["shell_command"]),
            package_remove_for_user=bool(data["capability_defaults"]["package_remove_for_user"]),
            root_shell=bool(data["capability_defaults"]["root_shell"]),
            app_data_write=bool(data["capability_defaults"]["app_data_write"]),
        ),
        device_tags=tuple(str(item) for item in data.get("device_tags", [])),
        metadata=dict(data.get("metadata", {})),
        schema_version=int(data["schema_version"]),
        kind=str(data["kind"]),
    )


def _parse_device_plan(data: Mapping[str, Any]) -> DevicePlan:
    return DevicePlan(
        id=str(data["id"]),
        name=str(data["name"]),
        description=_optional_str(data.get("description")),
        device_profile_ref=str(data["device_profile_ref"]),
        recipes=tuple(
            DevicePlanRecipeSelection(
                recipe_ref=str(item["recipe_ref"]),
                selected_by_default=bool(item["selected_by_default"]),
            )
            for item in data.get("recipes", [])
        ),
        defaults=dict(data.get("defaults", {})),
        overrides=dict(data.get("overrides", {})),
        metadata=dict(data.get("metadata", {})),
        schema_version=int(data["schema_version"]),
        kind=str(data["kind"]),
    )


def _parse_android_version_range(data: Mapping[str, Any]):
    from emuchef.domain import AndroidVersionRange

    return AndroidVersionRange(
        min=int(data["min"]) if data.get("min") is not None else None,
        max=int(data["max"]) if data.get("max") is not None else None,
    )


def _parse_step_condition(data: Mapping[str, Any]) -> StepCondition:
    return StepCondition(type=str(data["type"]), params=dict(data.get("params", {})))


def _parse_param_value(value: Any):
    if isinstance(value, dict) and set(value.keys()) == {"ref"}:
        parse_reference(str(value["ref"]))
        return RefParamValue(ref=str(value["ref"]))
    return value


def _parse_artifact_definition(artifact_id: str, data: Mapping[str, Any]) -> RemoteFileArtifact:
    artifact_type = str(data["type"])
    if artifact_type != "remote_file":
        raise ValueError(f"Unsupported artifact type: {artifact_type!r}")
    return RemoteFileArtifact(
        id=artifact_id,
        url=str(data["url"]),
        cache=ArtifactCacheMode(str(data.get("cache", ArtifactCacheMode.DEFAULT.value))),
    )


def _parse_artifact_support(data: Mapping[str, Any]) -> AppArtifactSupport:
    return AppArtifactSupport(
        required=bool(data["required"]) if "required" in data else None,
        supported=bool(data["supported"]) if "supported" in data else None,
    )


def _parse_optional_artifact(data: Mapping[str, Any] | None) -> AppArtifactSupport | None:
    if data is None:
        return None
    return _parse_artifact_support(data)


def _namespaced_inputs(kind: str, items: Mapping[str, Any]) -> dict[str, InputDeclaration]:
    result: dict[str, InputDeclaration] = {}
    for item_id, item in items.items():
        declarations = item.inputs.values() if isinstance(item.inputs, Mapping) else item.inputs
        for declaration in declarations:
            full_ref = f"{item_id}/{declaration.id}"
            result[full_ref] = declaration
    return result


def _ensure_allowed_mapping_keys(data: Mapping[str, Any], allowed_keys: set[str], label: str) -> None:
    unexpected = sorted(str(key) for key in data.keys() if str(key) not in allowed_keys)
    if unexpected:
        raise ValueError(f"{label} contains unsupported keys: {unexpected}")


def _namespaced_artifacts(items: Mapping[str, Recipe]) -> dict[str, RemoteFileArtifact]:
    result: dict[str, RemoteFileArtifact] = {}
    for recipe_id, recipe in items.items():
        for artifact_id, artifact in recipe.artifacts.items():
            result[f"{recipe_id}/{artifact_id}"] = artifact
    return result


def _validate_device_plan_overrides(
    device_plan: DevicePlan,
    binding_inputs: Mapping[str, InputDeclaration],
) -> tuple[ErrorMessage, ...]:
    _, problems = normalize_planner_overrides(
        device_plan.overrides,
        binding_inputs,
        allow_metadata_keys=True,
    )
    errors: list[ErrorMessage] = []
    for problem in problems:
        if problem.kind is PlannerOverrideProblemKind.UNKNOWN_BINDING:
            binding_ref = problem.binding_ref or str(problem.key)
            errors.append(
                ErrorMessage(
                    code=ErrorCode.BINDING_MISSING,
                    message=f"Device plan override {binding_ref!r} references unknown binding {binding_ref!r}.",
                    details={
                        "device_plan_ref": device_plan.id,
                        "override_key": str(problem.key),
                        "binding_ref": binding_ref,
                    },
                )
            )
            continue
        errors.append(
            ErrorMessage(
                code=ErrorCode.AUTHORED_DATA_INVALID,
                message=f"Device plan override {problem.key!r} must target a known execution-plan input id.",
                details={
                    "device_plan_ref": device_plan.id,
                    "override_key": str(problem.key),
                },
            )
        )
    return tuple(errors)


def _optional_str(value: Any) -> str | None:
    if value is None:
        return None
    return str(value)

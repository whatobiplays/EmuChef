from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

import yaml

from emuchef.domain import (
    Availability,
    BoundParamValue,
    CopyPolicy,
    DeviceContext,
    DraftInputState,
    DraftPlan,
    DraftPlanSource,
    DraftRecipeState,
    DraftStepState,
    ErrorCode,
    LiteralParamValue,
    RuntimeCapabilities,
    Step,
    StepCondition,
    StepConstraints,
    StepType,
    parse_reference,
)
from emuchef.io import load_authored_catalog
from emuchef.planner import CatalogLoadError, Planner
from emuchef.planner.contracts import validate_step_contract
from emuchef.planner.draft_builder import build_draft_plan
from emuchef.planner.emitter import emit_execution_plan


def write_yaml(path: Path, payload: dict) -> None:
    path.write_text(yaml.safe_dump(payload), encoding="utf-8")


class PlannerCoreTests(unittest.TestCase):
    def test_happy_path_bind_and_emit(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )

            self.assertEqual([recipe.id for recipe in session.draft_plan.recipes], ["feature.copy_bios"])
            self.assertEqual(
                [(step.id, step.selected) for step in session.draft_plan.steps],
                [("feature.copy_bios/copy_bios_dir", True)],
            )

            update = session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))
            self.assertEqual(update.errors, ())
            self.assertEqual(update.changes.bound_input_ids, ("feature.copy_bios.$bios_source_dir",))

            result = session.emit_execution_plan()
            self.assertEqual(result.status.value, "success")
            self.assertEqual(result.errors, ())
            self.assertEqual(len(result.execution_plan.steps), 1)
            self.assertEqual(result.execution_plan.steps[0].params["input"], str(bios_dir))
            self.assertEqual(result.execution_plan.steps[0].params["copy_policy"], "sync")
            self.assertNotIn("overwrite", result.execution_plan.steps[0].params)

    def test_recipe_dependency_is_auto_included(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, include_dependency_recipe=True)

            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )

            recipes = {recipe.id: recipe for recipe in session.draft_plan.recipes}
            self.assertIn("app.obtainium.install", recipes)
            self.assertTrue(recipes["app.obtainium.install"].auto_included)

    def test_history_undo_redo_restores_snapshots(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )

            session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))
            self.assertEqual(session.draft_plan.inputs[0].value, str(bios_dir))
            session.unbind_input("feature.copy_bios.$bios_source_dir")
            self.assertIsNone(session.draft_plan.inputs[0].value)

            undo = session.undo()
            self.assertEqual(undo.draft_plan.inputs[0].value, str(bios_dir))
            redo = session.redo()
            self.assertIsNone(redo.draft_plan.inputs[0].value)

    def test_loader_rejects_invalid_bindable_param_shape(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, input_param="raw/path/not/allowed")

            with self.assertRaises(CatalogLoadError) as context:
                load_authored_catalog(authored_root)

            self.assertTrue(
                any(error.code.value == "param_contract_violation" for error in context.exception.errors),
                context.exception.errors,
            )

    def test_loader_rejects_overwrite_param_for_copy_step(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, use_legacy_overwrite=True)

            with self.assertRaises(CatalogLoadError) as context:
                load_authored_catalog(authored_root)

            self.assertTrue(
                any(error.details.get("param") == "overwrite" for error in context.exception.errors),
                context.exception.errors,
            )

    def test_copy_related_contract_accepts_valid_copy_policy(self) -> None:
        steps = (
            Step(
                id="copy_input",
                type=StepType.COPY_BYO_INPUT,
                name="Copy input",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "input": BoundParamValue(ref=parse_reference("feature.copy_bios.$bios_source_dir")),
                    "dest": LiteralParamValue(value="/sdcard/BIOS"),
                    "copy_policy": CopyPolicy.SYNC.value,
                },
                verify=(),
            ),
            Step(
                id="push_file",
                type=StepType.PUSH_FILE,
                name="Push file",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "source": LiteralParamValue(value="/tmp/example.cfg"),
                    "dest": LiteralParamValue(value="/sdcard/Config/example.cfg"),
                    "copy_policy": CopyPolicy.MERGE.value,
                },
                verify=(),
            ),
            Step(
                id="push_dir",
                type=StepType.PUSH_DIR,
                name="Push dir",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "source": LiteralParamValue(value="/tmp/config"),
                    "dest": LiteralParamValue(value="/sdcard/Config"),
                    "copy_policy": CopyPolicy.REPLACE.value,
                },
                verify=(),
            ),
        )

        for step in steps:
            self.assertEqual(validate_step_contract("example.recipe", step), ())

    def test_copy_related_contract_rejects_overwrite_param(self) -> None:
        steps = (
            Step(
                id="copy_input",
                type=StepType.COPY_BYO_INPUT,
                name="Copy input",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "input": BoundParamValue(ref=parse_reference("feature.copy_bios.$bios_source_dir")),
                    "dest": LiteralParamValue(value="/sdcard/BIOS"),
                    "overwrite": False,
                },
                verify=(),
            ),
            Step(
                id="push_file",
                type=StepType.PUSH_FILE,
                name="Push file",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "source": LiteralParamValue(value="/tmp/example.cfg"),
                    "dest": LiteralParamValue(value="/sdcard/Config/example.cfg"),
                    "overwrite": False,
                },
                verify=(),
            ),
            Step(
                id="push_dir",
                type=StepType.PUSH_DIR,
                name="Push dir",
                user_toggleable=True,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={
                    "source": LiteralParamValue(value="/tmp/config"),
                    "dest": LiteralParamValue(value="/sdcard/Config"),
                    "overwrite": False,
                },
                verify=(),
            ),
        )

        for step in steps:
            errors = validate_step_contract("example.recipe", step)
            self.assertTrue(any(error.details.get("param") == "overwrite" for error in errors), errors)

    def test_execution_plan_contains_no_refs(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )
            session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))
            result = session.emit_execution_plan()

            self.assertFalse(contains_ref(result.execution_plan.steps[0].params))

    def test_capability_pruning_removes_optional_incompatible_steps(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root)
            catalog = load_authored_catalog(authored_root)

            draft_plan = DraftPlan(
                id="draft.test",
                source=DraftPlanSource(
                    device_profile_ref="ayaneo.generic",
                    device_plan_ref="ayaneo.generic.base",
                    selected_recipe_refs=("feature.copy_bios",),
                ),
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
                runtime_capabilities=RuntimeCapabilities(
                    adb_available=True,
                    apk_install=True,
                    shared_storage_write=False,
                    app_launch=True,
                    shell_command=True,
                    package_remove_for_user=True,
                    root_shell=False,
                    app_data_write=False,
                ),
                recipes=(
                    DraftRecipeState(
                        id="feature.copy_bios",
                        selected=True,
                        auto_included=False,
                        user_toggleable=True,
                        availability=Availability.AVAILABLE,
                        reason=None,
                    ),
                ),
                steps=(
                    DraftStepState(
                        id="feature.copy_bios/copy_bios_dir",
                        recipe_ref="feature.copy_bios",
                        type=StepType.COPY_BYO_INPUT,
                        name="Copy BIOS folder",
                        selected=True,
                        user_toggleable=True,
                        availability=Availability.AVAILABLE,
                        reason=None,
                    ),
                ),
                inputs=(
                    DraftInputState(
                        id="feature.copy_bios.$bios_source_dir",
                        label="BIOS Folder",
                        required=True,
                        multiple=False,
                        resolved=True,
                        value=str(root),
                        required_by=("feature.copy_bios/copy_bios_dir",),
                    ),
                ),
                warnings=(),
            )

            result = emit_execution_plan(
                catalog=catalog,
                draft_plan=draft_plan,
                user_bindings={"feature.copy_bios.$bios_source_dir": str(root)},
                planner_overrides={},
                step_selection_overrides={},
                plan_id="plan.test",
            )

            self.assertEqual(result.status.value, "error")
            self.assertIsNone(result.execution_plan)
            self.assertEqual(result.errors[0].code, ErrorCode.EMPTY_EXECUTION_PLAN)

    def test_empty_execution_plan_emission_fails(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root)

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )
            update = session.deselect_recipe("feature.copy_bios")
            self.assertEqual(update.errors, ())

            result = session.emit_execution_plan()

            self.assertEqual(result.status.value, "error")
            self.assertIsNone(result.execution_plan)
            self.assertEqual(result.errors[0].code, ErrorCode.EMPTY_EXECUTION_PLAN)

    def test_deselect_non_toggleable_step_returns_error_without_state_change(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, include_non_toggleable_step=True)

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )
            before_can_undo = session.history.can_undo
            before_selected = tuple((step.id, step.selected) for step in session.draft_plan.steps)

            update = session.deselect_step("feature.copy_bios/required_setup")

            self.assertEqual(update.errors[0].code, ErrorCode.STEP_NOT_TOGGLEABLE)
            self.assertEqual(session.history.can_undo, before_can_undo)
            self.assertEqual(tuple((step.id, step.selected) for step in session.draft_plan.steps), before_selected)
            undo = session.undo()
            self.assertEqual(undo.errors[0].code, ErrorCode.INVALID_OPERATION)

    def test_conflict_unresolved_is_reported(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_conflicting_authored_tree(root)
            catalog = load_authored_catalog(authored_root)
            draft_plan, errors = build_draft_plan(
                catalog=catalog,
                draft_id="draft.conflict",
                device_plan_ref="ayaneo.generic.base",
                device_profile_ref="ayaneo.generic",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
                runtime_capabilities=catalog.device_profiles["ayaneo.generic"].capability_defaults,
                user_selected_recipe_refs=("feature.conflict",),
                step_selection_overrides={},
                user_bindings={},
                planner_overrides={},
            )
            self.assertIsNone(draft_plan)
            self.assertEqual(errors[0].code, ErrorCode.CONFLICT_UNRESOLVED)


def build_minimal_authored_tree(
    root: Path,
    input_param: object | None = None,
    include_dependency_recipe: bool = False,
    include_non_toggleable_step: bool = False,
    use_legacy_overwrite: bool = False,
) -> Path:
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (root / subdir).mkdir(parents=True, exist_ok=True)

    recipe_id = "feature.copy_bios"
    recipe_steps = []
    if include_non_toggleable_step:
        recipe_steps.append(
            {
                "id": "required_setup",
                "type": "launch_app",
                "name": "Required setup",
                "user_toggleable": False,
                "dependencies": [],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": {"value": "com.example.required"}},
                "verify": [],
            }
        )
    recipe_steps.append(
        {
            "id": "copy_bios_dir",
            "type": "copy_byo_input",
            "name": "Copy BIOS folder",
            "user_toggleable": True,
            "dependencies": ["required_setup"] if include_non_toggleable_step else [],
            "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
            "skip_if": [],
            "params": {
                "input": input_param if input_param is not None else {"ref": f"{recipe_id}.$bios_source_dir"},
                "dest": {"value": "/sdcard/BIOS"},
                **({"overwrite": False} if use_legacy_overwrite else {"copy_policy": "sync"}),
            },
            "verify": [],
        }
    )
    recipe_inputs = [
        {
            "id": "bios_source_dir",
            "type": "directory",
            "role": "bios",
            "label": "BIOS Folder",
            "required": True,
            "multiple": False,
            "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
            "default": None,
            "metadata": {},
        }
    ]

    recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": recipe_id,
        "name": "Copy BIOS Files",
        "recipe_dependencies": [],
        "provides": {"features": ["bios_copy"]},
        "inputs": recipe_inputs,
        "steps": recipe_steps,
    }
    profile = {
        "schema_version": 1,
        "kind": "device_profile",
        "id": "ayaneo.generic",
        "name": "AYANEO Generic",
        "match": {
            "manufacturer_contains": ["AYANEO", "ARBOR"],
            "brand_contains": ["AYANEO"],
            "model_patterns": ["(?i)ayaneo"],
            "android_version": {"min": 11},
        },
        "capability_defaults": {
            "adb_available": True,
            "apk_install": True,
            "shared_storage_write": True,
            "app_launch": True,
            "shell_command": True,
            "package_remove_for_user": True,
            "root_shell": False,
            "app_data_write": False,
        },
        "device_tags": ["handheld_android"],
        "metadata": {},
    }
    plan = {
        "schema_version": 1,
        "kind": "device_plan",
        "id": "ayaneo.generic.base",
        "name": "Base",
        "device_profile_ref": "ayaneo.generic",
        "recipes": [{"recipe_ref": recipe_id, "selected_by_default": True}],
        "defaults": {},
        "overrides": {},
        "metadata": {},
    }

    if include_dependency_recipe:
        recipe["id"] = "app.retroarch.provision"
        recipe["recipe_dependencies"] = ["app.obtainium.install"]
        recipe["inputs"] = []
        recipe["steps"] = [
            {
                "id": "launch_retroarch",
                "type": "launch_app",
                "name": "Launch RetroArch",
                "user_toggleable": True,
                "dependencies": [],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": {"value": "com.retroarch"}},
                "verify": [],
            }
        ]
        dependent_recipe = {
            "schema_version": 1,
            "kind": "recipe",
            "id": "app.obtainium.install",
            "name": "Install Obtainium",
            "recipe_dependencies": [],
            "provides": {"features": ["obtainium_install"]},
            "inputs": [],
            "steps": [
                {
                    "id": "install_obtainium",
                    "type": "install_apk",
                    "name": "Install Obtainium",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": ["apk_install"], "conflicts_with": []},
                    "skip_if": [],
                    "params": {"app": "sample_artifacts/obtainium.apk", "replace_existing": False},
                    "verify": [],
                }
            ],
        }
        plan["recipes"] = [{"recipe_ref": "app.retroarch.provision", "selected_by_default": True}]
        write_yaml(root / "recipes" / "obtainium.yaml", dependent_recipe)

    write_yaml(root / "recipes" / "copy_bios.yaml", recipe)
    write_yaml(root / "device_profiles" / "ayaneo.yaml", profile)
    write_yaml(root / "device_plans" / "ayaneo_base.yaml", plan)
    return root


def build_conflicting_authored_tree(root: Path) -> Path:
    authored_root = build_minimal_authored_tree(root)
    conflict_recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "feature.conflict",
        "name": "Conflict Recipe",
        "recipe_dependencies": [],
        "provides": {"features": []},
        "inputs": [],
        "steps": [
            {
                "id": "step_a",
                "type": "launch_app",
                "name": "Step A",
                "user_toggleable": True,
                "dependencies": [],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": ["step_b"]},
                "skip_if": [],
                "params": {"package_name": {"value": "com.example.a"}},
                "verify": [],
            },
            {
                "id": "step_b",
                "type": "launch_app",
                "name": "Step B",
                "user_toggleable": True,
                "dependencies": [],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": ["step_a"]},
                "skip_if": [],
                "params": {"package_name": {"value": "com.example.b"}},
                "verify": [],
            },
        ],
    }
    write_yaml(root / "recipes" / "conflict.yaml", conflict_recipe)
    return authored_root


def contains_ref(value: object) -> bool:
    if isinstance(value, dict):
        if "ref" in value:
            return True
        return any(contains_ref(item) for item in value.values())
    if isinstance(value, (list, tuple)):
        return any(contains_ref(item) for item in value)
    return False


if __name__ == "__main__":
    unittest.main()

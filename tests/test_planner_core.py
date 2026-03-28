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
    InputDeclaration,
    InputRole,
    InputType,
    InputValidation,
    LiteralParamValue,
    RuntimeCapabilities,
    Step,
    StepCondition,
    StepConstraints,
    StepType,
    parse_reference,
)
from emuchef.io import dump_yaml, load_authored_catalog, load_execution_plan_file
from emuchef.planner.bindings import validate_binding_value
from emuchef.planner import CatalogLoadError, Planner
from emuchef.planner.contracts import validate_step_contract
from emuchef.planner.draft_builder import build_draft_plan
from emuchef.planner.emitter import emit_execution_plan


def write_yaml(path: Path, payload: dict) -> None:
    path.write_text(yaml.safe_dump(payload), encoding="utf-8")


class PlannerCoreTests(unittest.TestCase):
    def test_loader_defaults_permissions_when_omitted(self) -> None:
        with TemporaryDirectory() as tmp:
            authored_root = build_minimal_authored_tree(Path(tmp))

            catalog = load_authored_catalog(authored_root)
            recipe = catalog.recipes["feature.copy_bios"]

            self.assertEqual(recipe.permissions.runtime, ())
            self.assertEqual(recipe.permissions.appops, ())
            self.assertEqual(recipe.permissions.manual, ())
            self.assertEqual(recipe.permissions.policy.on_failure, "warn")
            self.assertFalse(recipe.permissions.policy.require_all)

    def test_loader_parses_recipe_permissions(self) -> None:
        with TemporaryDirectory() as tmp:
            authored_root = build_minimal_authored_tree(Path(tmp), recipe_permissions=sample_permission_block())

            catalog = load_authored_catalog(authored_root)
            recipe = catalog.recipes["feature.copy_bios"]

            self.assertEqual(recipe.permissions.runtime[0].package_name, "com.retroarch.aarch64")
            self.assertEqual(recipe.permissions.runtime[0].name, "android.permission.POST_NOTIFICATIONS")
            self.assertFalse(recipe.permissions.runtime[0].required)
            self.assertEqual(recipe.permissions.runtime[0].when.android_api_min, 33)
            self.assertEqual(recipe.permissions.appops[0].op, "MANAGE_EXTERNAL_STORAGE")
            self.assertEqual(recipe.permissions.appops[0].mode, "allow")
            self.assertEqual(recipe.permissions.manual[0].manual_type, "folder_picker")
            self.assertEqual(
                recipe.permissions.manual[0].reason,
                "App requires SAF URI grant for ROM directory selection",
            )

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
            self.assertIsNone(result.execution_plan.permission_plan)

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

    def test_new_step_contracts_accept_valid_permission_wait_and_force_stop_steps(self) -> None:
        steps = (
            Step(
                id="grant_permissions",
                type=StepType.GRANT_PERMISSIONS,
                name="Grant permissions",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={},
                verify=(),
            ),
            Step(
                id="wait_for_bootstrap",
                type=StepType.WAIT,
                name="Wait",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"duration_ms": 1500},
                verify=(),
            ),
            Step(
                id="force_stop",
                type=StepType.FORCE_STOP_APP,
                name="Force stop",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"package_name": "com.retroarch.aarch64"},
                verify=(),
            ),
        )

        for step in steps:
            self.assertEqual(validate_step_contract("example.recipe", step), ())

    def test_wait_contract_rejects_invalid_duration_ms(self) -> None:
        invalid_steps = (
            Step(
                id="wait_zero",
                type=StepType.WAIT,
                name="Wait zero",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"duration_ms": 0},
                verify=(),
            ),
            Step(
                id="wait_negative",
                type=StepType.WAIT,
                name="Wait negative",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"duration_ms": -1},
                verify=(),
            ),
            Step(
                id="wait_wrapped",
                type=StepType.WAIT,
                name="Wait wrapped",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"duration_ms": LiteralParamValue(value=1000)},
                verify=(),
            ),
            Step(
                id="wait_non_integer",
                type=StepType.WAIT,
                name="Wait non integer",
                user_toggleable=False,
                dependencies=(),
                constraints=StepConstraints(),
                skip_if=(),
                params={"duration_ms": "1000"},
                verify=(),
            ),
        )

        for step in invalid_steps:
            errors = validate_step_contract("example.recipe", step)
            self.assertTrue(any(error.details.get("param") == "duration_ms" for error in errors), errors)

    def test_force_stop_app_contract_rejects_empty_package_name(self) -> None:
        step = Step(
            id="force_stop",
            type=StepType.FORCE_STOP_APP,
            name="Force stop",
            user_toggleable=False,
            dependencies=(),
            constraints=StepConstraints(),
            skip_if=(),
            params={"package_name": ""},
            verify=(),
        )

        errors = validate_step_contract("example.recipe", step)
        self.assertTrue(any(error.details.get("param") == "package_name" for error in errors), errors)

    def test_binding_validation_accepts_extension_without_dot(self) -> None:
        with TemporaryDirectory() as tmp:
            cfg_path = Path(tmp) / "retroarch.cfg"
            cfg_path.write_text("video_driver = gl", encoding="utf-8")
            declaration = InputDeclaration(
                id="app.retroarch.provision.$retroarch_cfg",
                type=InputType.FILE,
                role=InputRole.GENERIC,
                label="RetroArch config",
                required=False,
                multiple=False,
                validation=InputValidation(must_exist=True, allowed_extensions=("cfg",), path_kind=InputType.FILE),
            )

            self.assertEqual(
                validate_binding_value(declaration.id, declaration, str(cfg_path)),
                (),
            )

    def test_binding_validation_accepts_extension_with_dot(self) -> None:
        with TemporaryDirectory() as tmp:
            cfg_path = Path(tmp) / "retroarch.cfg"
            cfg_path.write_text("video_driver = gl", encoding="utf-8")
            declaration = InputDeclaration(
                id="app.retroarch.provision.$retroarch_cfg",
                type=InputType.FILE,
                role=InputRole.GENERIC,
                label="RetroArch config",
                required=False,
                multiple=False,
                validation=InputValidation(must_exist=True, allowed_extensions=(".cfg",), path_kind=InputType.FILE),
            )

            self.assertEqual(
                validate_binding_value(declaration.id, declaration, str(cfg_path)),
                (),
            )

    def test_binding_validation_accepts_mixed_case_extension_without_dot(self) -> None:
        with TemporaryDirectory() as tmp:
            cfg_path = Path(tmp) / "retroarch.cfg"
            cfg_path.write_text("video_driver = gl", encoding="utf-8")
            declaration = InputDeclaration(
                id="app.retroarch.provision.$retroarch_cfg",
                type=InputType.FILE,
                role=InputRole.GENERIC,
                label="RetroArch config",
                required=False,
                multiple=False,
                validation=InputValidation(must_exist=True, allowed_extensions=("CFG",), path_kind=InputType.FILE),
            )

            self.assertEqual(
                validate_binding_value(declaration.id, declaration, str(cfg_path)),
                (),
            )

    def test_binding_validation_rejects_real_extension_mismatch(self) -> None:
        with TemporaryDirectory() as tmp:
            txt_path = Path(tmp) / "retroarch.txt"
            txt_path.write_text("video_driver = gl", encoding="utf-8")
            declaration = InputDeclaration(
                id="app.retroarch.provision.$retroarch_cfg",
                type=InputType.FILE,
                role=InputRole.GENERIC,
                label="RetroArch config",
                required=False,
                multiple=False,
                validation=InputValidation(must_exist=True, allowed_extensions=("cfg",), path_kind=InputType.FILE),
            )

            errors = validate_binding_value(declaration.id, declaration, str(txt_path))

            self.assertEqual(len(errors), 1)
            self.assertEqual(errors[0].code, ErrorCode.BINDING_VALIDATION_FAILED)
            self.assertIn("unsupported extension", errors[0].message)

    def test_session_binding_accepts_current_retroarch_cfg_extension_style(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_retroarch_cfg_authored_tree(root)
            cfg_path = root / "retroarch.cfg"
            cfg_path.write_text("video_driver = gl", encoding="utf-8")

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket S mini", android_version=13),
            )

            update = session.bind_input("app.retroarch.provision.$retroarch_cfg", str(cfg_path))

            self.assertEqual(update.errors, ())
            self.assertEqual(update.changes.bound_input_ids, ("app.retroarch.provision.$retroarch_cfg",))

    def test_execution_plan_emits_permission_lifecycle_steps_in_order(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_retroarch_cfg_authored_tree(root)
            cfg_path = root / "retroarch.cfg"
            cfg_path.write_text("video_driver = gl", encoding="utf-8")

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(
                    manufacturer="AYANEO",
                    model="Pocket S mini",
                    android_version=13,
                    android_api_level=33,
                ),
            )
            session.bind_input("app.retroarch.provision.$retroarch_cfg", str(cfg_path))

            result = session.emit_execution_plan()

            self.assertEqual(result.status.value, "success")
            self.assertEqual(
                [step.type for step in result.execution_plan.steps],
                [
                    StepType.INSTALL_APK,
                    StepType.LAUNCH_APP,
                    StepType.WAIT,
                    StepType.FORCE_STOP_APP,
                    StepType.GRANT_PERMISSIONS,
                    StepType.COPY_BYO_INPUT,
                    StepType.LAUNCH_APP,
                ],
            )
            self.assertEqual(result.execution_plan.steps[2].params["duration_ms"], 1500)
            self.assertEqual(result.execution_plan.steps[3].params["package_name"], "com.retroarch.aarch64")
            self.assertEqual(result.execution_plan.steps[4].params, {})
            self.assertIsNotNone(result.execution_plan.permission_plan)

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

    def test_permission_plan_emits_actions_and_round_trips(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, recipe_permissions=sample_permission_block())
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(
                    manufacturer="AYANEO",
                    model="Pocket 4 Pro",
                    android_version=13,
                    android_api_level=33,
                ),
            )
            session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))

            result = session.emit_execution_plan()

            self.assertIsNotNone(result.execution_plan.permission_plan)
            actions = result.execution_plan.permission_plan.actions
            self.assertEqual(len(actions), 3)

            runtime_action, appop_action, manual_action = actions
            self.assertEqual(runtime_action.status, "applicable")
            self.assertEqual(runtime_action.kind, "runtime_permission")
            self.assertEqual(runtime_action.permission, "android.permission.POST_NOTIFICATIONS")
            self.assertEqual(runtime_action.command[-2:], ("com.retroarch.aarch64", "android.permission.POST_NOTIFICATIONS"))
            self.assertEqual(runtime_action.source.recipe_id, "feature.copy_bios")
            self.assertEqual(runtime_action.source.section, "permissions.runtime[0]")

            self.assertEqual(appop_action.status, "skipped")
            self.assertEqual(appop_action.kind, "appop")
            self.assertEqual(appop_action.reason.code, "requires_root")
            self.assertEqual(appop_action.source.section, "permissions.appops[0]")
            self.assertEqual(appop_action.command[-3:], ("com.retroarch.aarch64", "MANAGE_EXTERNAL_STORAGE", "allow"))

            self.assertEqual(manual_action.status, "manual_required")
            self.assertEqual(manual_action.kind, "manual_requirement")
            self.assertEqual(manual_action.manual_type, "folder_picker")
            self.assertEqual(manual_action.reason.message, "App requires SAF URI grant for ROM directory selection")
            self.assertEqual(manual_action.source.section, "permissions.manual[0]")
            self.assertEqual(manual_action.command, ())

            plan_file = root / "plan.yaml"
            plan_file.write_text(dump_yaml(result.execution_plan), encoding="utf-8")
            payload = yaml.safe_load(plan_file.read_text(encoding="utf-8"))
            self.assertEqual(payload["device_context"]["android_api_level"], 33)
            self.assertEqual(payload["permission_plan"]["actions"][2]["command"], [])

            loaded_plan = load_execution_plan_file(plan_file)
            self.assertEqual(loaded_plan.device_context.android_api_level, 33)
            self.assertEqual(loaded_plan.permission_plan.actions[2].command, ())
            self.assertEqual(loaded_plan.permission_plan.actions[1].reason.code, "requires_root")

    def test_permission_plan_skips_when_android_api_level_is_out_of_range(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, recipe_permissions=sample_permission_block())
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(
                    manufacturer="AYANEO",
                    model="Pocket 4 Pro",
                    android_version=13,
                    android_api_level=32,
                ),
            )
            session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))

            result = session.emit_execution_plan()

            runtime_action = result.execution_plan.permission_plan.actions[0]
            self.assertEqual(runtime_action.status, "skipped")
            self.assertEqual(runtime_action.reason.code, "android_api_out_of_range")
            self.assertIn("API 32", runtime_action.reason.message)
            self.assertIn(">= 33", runtime_action.reason.message)

    def test_permission_plan_skips_when_android_api_level_is_unknown(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_minimal_authored_tree(root, recipe_permissions=sample_permission_block())
            bios_dir = root / "bios"
            bios_dir.mkdir()

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )
            session.bind_input("feature.copy_bios.$bios_source_dir", str(bios_dir))

            result = session.emit_execution_plan()

            runtime_action = result.execution_plan.permission_plan.actions[0]
            self.assertEqual(runtime_action.status, "skipped")
            self.assertEqual(runtime_action.reason.code, "missing_android_api_level")
            self.assertEqual(runtime_action.reason.message, "Device Android API level is unknown.")
            self.assertEqual(
                runtime_action.command,
                (
                    "adb",
                    "shell",
                    "pm",
                    "grant",
                    "com.retroarch.aarch64",
                    "android.permission.POST_NOTIFICATIONS",
                ),
            )
            self.assertEqual(runtime_action.source.section, "permissions.runtime[0]")

    def test_permission_plan_preserves_duplicate_equivalent_actions(self) -> None:
        with TemporaryDirectory() as tmp:
            authored_root = build_minimal_authored_tree(
                Path(tmp),
                include_dependency_recipe=True,
                recipe_permissions=duplicate_runtime_permission_block(),
                dependency_recipe_permissions=duplicate_runtime_permission_block(),
            )

            catalog = load_authored_catalog(authored_root)
            session = Planner(catalog).start_session(
                device_plan_ref="ayaneo.generic.base",
                device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
            )

            result = session.emit_execution_plan()

            self.assertIsNotNone(result.execution_plan.permission_plan)
            actions = result.execution_plan.permission_plan.actions
            self.assertEqual(len(actions), 2)
            self.assertEqual(actions[0].command, actions[1].command)
            self.assertNotEqual(actions[0].source.recipe_id, actions[1].source.recipe_id)
            self.assertEqual(
                [action.source.recipe_id for action in actions],
                [recipe.id for recipe in session.draft_plan.recipes],
            )

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
    recipe_permissions: dict | None = None,
    dependency_recipe_permissions: dict | None = None,
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
    if recipe_permissions is not None:
        recipe["permissions"] = recipe_permissions
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
        if recipe_permissions is not None:
            recipe["permissions"] = recipe_permissions
        if dependency_recipe_permissions is not None:
            dependent_recipe["permissions"] = dependency_recipe_permissions
        plan["recipes"] = [{"recipe_ref": "app.retroarch.provision", "selected_by_default": True}]
        write_yaml(root / "recipes" / "obtainium.yaml", dependent_recipe)

    write_yaml(root / "recipes" / "copy_bios.yaml", recipe)
    write_yaml(root / "device_profiles" / "ayaneo.yaml", profile)
    write_yaml(root / "device_plans" / "ayaneo_base.yaml", plan)
    return root


def build_retroarch_cfg_authored_tree(root: Path) -> Path:
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (root / subdir).mkdir(parents=True, exist_ok=True)

    recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "app.retroarch.provision",
        "name": "Provision RetroArch",
        "recipe_dependencies": [],
        "provides": {"features": ["retroarch_provision"]},
        "permissions": {
            "runtime": [
                {
                    "package_name": "com.retroarch.aarch64",
                    "name": "android.permission.POST_NOTIFICATIONS",
                    "required": False,
                    "when": {"android_api_min": 33},
                }
            ],
            "appops": [
                {
                    "package_name": "com.retroarch.aarch64",
                    "op": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                    "required": False,
                    "when": {"rooted": True},
                }
            ],
            "policy": {"on_failure": "warn", "require_all": False},
        },
        "inputs": [
            {
                "id": "retroarch_cfg",
                "type": "file",
                "role": "generic",
                "label": "RetroArch config",
                "required": False,
                "multiple": False,
                "validation": {"must_exist": True, "allowed_extensions": ["cfg"], "path_kind": "file"},
                "default": None,
                "metadata": {},
            }
        ],
        "steps": [
            {
                "id": "install_retroarch",
                "type": "install_apk",
                "name": "Install RetroArch",
                "user_toggleable": False,
                "dependencies": [],
                "constraints": {"capabilities": ["apk_install"], "conflicts_with": []},
                "skip_if": [],
                "params": {"app": "sample_artifacts/RetroArch_aarch64.apk", "replace_existing": False},
                "verify": [],
            },
            {
                "id": "launch_retroarch_bootstrap",
                "type": "launch_app",
                "name": "Launch RetroArch for bootstrap",
                "user_toggleable": False,
                "dependencies": ["install_retroarch"],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": {"value": "com.retroarch.aarch64"}},
                "verify": [],
            },
            {
                "id": "wait_for_retroarch_bootstrap",
                "type": "wait",
                "name": "Wait for RetroArch bootstrap",
                "user_toggleable": False,
                "dependencies": ["launch_retroarch_bootstrap"],
                "constraints": {"capabilities": [], "conflicts_with": []},
                "skip_if": [],
                "params": {"duration_ms": 1500},
                "verify": [],
            },
            {
                "id": "stop_retroarch_after_bootstrap",
                "type": "force_stop_app",
                "name": "Force stop RetroArch",
                "user_toggleable": False,
                "dependencies": ["wait_for_retroarch_bootstrap"],
                "constraints": {"capabilities": [], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": "com.retroarch.aarch64"},
                "verify": [],
            },
            {
                "id": "grant_retroarch_permissions",
                "type": "grant_permissions",
                "name": "Grant RetroArch permissions",
                "user_toggleable": False,
                "dependencies": ["stop_retroarch_after_bootstrap"],
                "constraints": {"capabilities": [], "conflicts_with": []},
                "skip_if": [],
                "params": {},
                "verify": [],
            },
            {
                "id": "seed_retroarch_cfg",
                "type": "copy_byo_input",
                "name": "Copy RetroArch config",
                "user_toggleable": True,
                "dependencies": ["grant_retroarch_permissions"],
                "constraints": {"capabilities": ["app_data_write"], "conflicts_with": []},
                "skip_if": [],
                "params": {
                    "input": {"ref": "app.retroarch.provision.$retroarch_cfg"},
                    "dest": {"value": "/sdcard/Android/data/com.retroarch.aarch64/files"},
                    "copy_policy": "sync",
                },
                "verify": [],
            },
            {
                "id": "launch_retroarch",
                "type": "launch_app",
                "name": "Launch RetroArch",
                "user_toggleable": True,
                "dependencies": ["grant_retroarch_permissions"],
                "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": {"value": "com.retroarch.aarch64"}},
                "verify": [],
            }
        ],
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
            "app_data_write": True,
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
        "recipes": [{"recipe_ref": "app.retroarch.provision", "selected_by_default": True}],
        "defaults": {},
        "overrides": {},
        "metadata": {},
    }

    write_yaml(root / "recipes" / "retroarch.yaml", recipe)
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


def sample_permission_block() -> dict:
    return {
        "runtime": [
            {
                "package_name": "com.retroarch.aarch64",
                "name": "android.permission.POST_NOTIFICATIONS",
                "required": False,
                "when": {"android_api_min": 33},
            }
        ],
        "appops": [
            {
                "package_name": "com.retroarch.aarch64",
                "op": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow",
                "required": False,
                "when": {"rooted": True},
            }
        ],
        "manual": [
            {
                "package_name": "org.citra.emu",
                "manual_type": "folder_picker",
                "reason": "App requires SAF URI grant for ROM directory selection",
                "required": True,
            }
        ],
        "policy": {"on_failure": "warn", "require_all": False},
    }


def duplicate_runtime_permission_block() -> dict:
    return {
        "runtime": [
            {
                "package_name": "com.example.shared",
                "name": "android.permission.POST_NOTIFICATIONS",
                "required": False,
            }
        ]
    }


if __name__ == "__main__":
    unittest.main()

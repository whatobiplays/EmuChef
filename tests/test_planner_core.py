from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef.domain import Availability, AvailabilityCode, DeviceContext, ErrorCode, LiteralParamValue, RefParamValue
from emuchef.io import load_authored_catalog, validate_authored_catalog
from emuchef.planner import Planner

from support import base_recipe, build_authored_tree


class PlannerCoreTests(unittest.TestCase):
    def test_loader_parses_map_inputs_artifacts_and_groups(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "config_file": {
                    "type": "file",
                    "role": "generic",
                    "label": "Config File",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": ["cfg"], "path_kind": "file"},
                }
            },
            artifacts={
                "app_apk": {"type": "remote_file", "url": "https://example.com/app.apk", "cache": "default"},
                "core_zip": {"type": "remote_file", "url": "https://example.com/core.zip", "cache": "default"},
            },
            artifact_groups={"core_group": ["core_zip"]},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["app_apk"], "artifact_groups": ["core_group"]},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            loaded = catalog.recipes["example.recipe"]
            self.assertEqual(tuple(loaded.inputs.keys()), ("config_file",))
            self.assertEqual(tuple(loaded.artifacts.keys()), ("app_apk", "core_zip"))
            self.assertEqual(loaded.artifact_groups["core_group"], ("core_zip",))

    def test_duplicate_artifacts_after_group_expansion_fail_validation(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "app_apk": {"type": "remote_file", "url": "https://example.com/app.apk"},
                "core_zip": {"type": "remote_file", "url": "https://example.com/core.zip"},
            },
            artifact_groups={"core_group": ["core_zip"]},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["core_zip"], "artifact_groups": ["core_group"]},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "error")
            self.assertTrue(
                any(error.code.value == "param_contract_violation" and "duplicate artifact ids" in error.message for error in result.errors),
                result.errors,
            )

    def test_step_output_ref_does_not_require_explicit_dependency(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"core_zip": {"type": "remote_file", "url": "https://example.com/core.zip"}},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["core_zip"]},
                    "verify": [],
                },
                {
                    "id": "extract",
                    "type": "extract_artifacts",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": ["resolve"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["core_zip"]},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": True,
                    "dependencies": [],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "steps.extract"}, "dest": "/sdcard/Test"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "success", result.errors)

    def test_execution_plan_injects_defaults_and_rewrites_shorthand_refs(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "app_apk": {"type": "remote_file", "url": "https://example.com/app.apk"},
                "core_zip": {"type": "remote_file", "url": "https://example.com/core.zip"},
            },
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["app_apk", "core_zip"]},
                    "verify": [],
                },
                {
                    "id": "extract",
                    "type": "extract_artifacts",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": ["resolve"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["core_zip"]},
                    "verify": [],
                },
                {
                    "id": "install",
                    "type": "install_apk",
                    "name": "Install",
                    "user_toggleable": False,
                    "dependencies": ["resolve"],
                    "constraints": {"capabilities": ["apk_install"], "conflicts_with": []},
                    "params": {"app": {"ref": "artifacts.app_apk.local_path"}},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": True,
                    "dependencies": ["extract"],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "steps.extract"}, "dest": "/sdcard/Test"},
                    "verify": [],
                },
                {
                    "id": "launch",
                    "type": "launch_app",
                    "name": "Launch",
                    "user_toggleable": True,
                    "dependencies": ["install"],
                    "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                    "params": {"package_name": "com.example.app", "activity": ".MainActivity"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )
            result = session.emit_execution_plan()
            self.assertEqual(result.status.value, "success", result.errors)
            plan = result.execution_plan
            assert plan is not None

            steps = {step.id: step for step in plan.steps}
            copy_step = steps["example.recipe/copy"]
            extract_step = steps["example.recipe/extract"]
            install_step = steps["example.recipe/install"]
            launch_step = steps["example.recipe/launch"]

            self.assertEqual(extract_step.params["extract_on"], LiteralParamValue(value="host"))
            self.assertEqual(copy_step.params["copy_policy"], LiteralParamValue(value="merge"))
            self.assertEqual(install_step.params["replace_existing"], LiteralParamValue(value=False))
            self.assertEqual(
                copy_step.params["source"],
                RefParamValue(ref="steps.example.recipe/extract.outputs.extracted_paths"),
            )
            self.assertEqual(launch_step.params["activity"], LiteralParamValue(value=".MainActivity"))

    def test_extract_archive_cleanup_defaults_true(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"}},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["archive_zip"]},
                    "verify": [],
                },
                {
                    "id": "extract",
                    "type": "extract_archive",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": ["resolve"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "archive": {"ref": "artifacts.archive_zip.local_path"},
                        "extract_on": "device",
                        "dest": "/sdcard/Extracted",
                    },
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )
            result = session.emit_execution_plan()
            self.assertEqual(result.status.value, "success", result.errors)
            plan = result.execution_plan
            assert plan is not None
            extract_step = next(step for step in plan.steps if step.id == "example.recipe/extract")
            self.assertEqual(extract_step.params["cleanup"], LiteralParamValue(value=True))

    def test_grant_permissions_without_local_permissions_is_valid(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "success", result.errors)
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )
            emitted = session.emit_execution_plan()
            self.assertEqual(emitted.status.value, "success", emitted.errors)
            assert emitted.execution_plan is not None
            grant_step = emitted.execution_plan.steps[0]
            self.assertEqual(grant_step.params, {})

    def test_grant_permissions_params_are_step_local_and_no_permission_plan_is_emitted(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "grant_early",
                    "type": "grant_permissions",
                    "name": "Grant Early",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "runtime": [
                            {
                                "package_name": "com.example.first",
                                "name": "android.permission.POST_NOTIFICATIONS",
                                "required": False,
                            }
                        ],
                        "policy": {"on_failure": "warn", "require_all": False},
                    },
                    "verify": [],
                },
                {
                    "id": "grant_late",
                    "type": "grant_permissions",
                    "name": "Grant Late",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "appops": [
                            {
                                "package_name": "com.example.second",
                                "op": "MANAGE_EXTERNAL_STORAGE",
                                "mode": "allow",
                                "required": False,
                            }
                        ],
                        "policy": {"on_failure": "fail", "require_all": True},
                    },
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )
            emitted = session.emit_execution_plan()

            self.assertEqual(emitted.status.value, "success", emitted.errors)
            plan = emitted.execution_plan
            assert plan is not None
            self.assertFalse(hasattr(plan, "permission_plan"))
            steps = {step.id: step for step in plan.steps}
            self.assertEqual(
                steps["example.recipe/grant_early"].params["runtime"].value[0]["package_name"],
                "com.example.first",
            )
            self.assertEqual(
                steps["example.recipe/grant_late"].params["appops"].value[0]["package_name"],
                "com.example.second",
            )
            self.assertEqual(
                steps["example.recipe/grant_late"].params["policy"].value,
                {"on_failure": "fail", "require_all": True},
            )

    def test_unbound_optional_input_prunes_direct_consumer_and_dependent_steps(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "optional_cfg": {
                    "type": "file",
                    "role": "generic",
                    "label": "Optional Config",
                    "description": "Optional config file.",
                    "required": False,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": ["cfg"], "path_kind": "file"},
                }
            },
            steps=[
                {
                    "id": "prepare",
                    "type": "wait",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 1},
                    "verify": [],
                },
                {
                    "id": "seed",
                    "type": "copy_files",
                    "name": "Seed",
                    "user_toggleable": True,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.optional_cfg"}, "dest": "/sdcard/Example.cfg"},
                    "verify": [],
                },
                {
                    "id": "launch",
                    "type": "launch_app",
                    "name": "Launch",
                    "user_toggleable": True,
                    "dependencies": ["seed"],
                    "constraints": {"capabilities": ["app_launch"], "conflicts_with": []},
                    "params": {"package_name": "com.example.app", "activity": ".MainActivity"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )

            steps = {step.id: step for step in session.draft_plan.steps}
            seed_step = steps["example.recipe/seed"]
            launch_step = steps["example.recipe/launch"]

            self.assertFalse(seed_step.selected)
            self.assertEqual(seed_step.availability, Availability.UNAVAILABLE)
            assert seed_step.reason is not None
            self.assertEqual(seed_step.reason.code, AvailabilityCode.OPTIONAL_INPUT_UNBOUND)
            self.assertEqual(seed_step.reason.details["input_id"], "example.recipe/optional_cfg")
            self.assertFalse(launch_step.selected)
            self.assertEqual(launch_step.availability, Availability.AVAILABLE)
            self.assertEqual(tuple(item.id for item in session.draft_plan.inputs), ())

            emitted = session.emit_execution_plan()
            self.assertEqual(emitted.status.value, "success", emitted.errors)
            assert emitted.execution_plan is not None
            self.assertEqual([step.id for step in emitted.execution_plan.steps], ["example.recipe/prepare"])
            self.assertEqual(emitted.execution_plan.inputs, ())

    def test_required_input_still_fails_when_unbound(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "required_cfg": {
                    "type": "file",
                    "role": "generic",
                    "label": "Required Config",
                    "description": "Required config file.",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": ["cfg"], "path_kind": "file"},
                }
            },
            steps=[
                {
                    "id": "seed",
                    "type": "copy_files",
                    "name": "Seed",
                    "user_toggleable": True,
                    "dependencies": [],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.required_cfg"}, "dest": "/sdcard/Example.cfg"},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )

            emitted = session.emit_execution_plan()
            self.assertEqual(emitted.status.value, "error")
            self.assertEqual(len(emitted.errors), 1)
            self.assertEqual(emitted.errors[0].code, ErrorCode.BINDING_MISSING)
            self.assertEqual(emitted.errors[0].details["input_id"], "example.recipe/required_cfg")

    def test_optional_input_binding_reenables_pruned_step_and_draft_input(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "optional_cfg": {
                    "type": "file",
                    "role": "generic",
                    "label": "Optional Config",
                    "description": "Optional config file.",
                    "required": False,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": ["cfg"], "path_kind": "file"},
                }
            },
            steps=[
                {
                    "id": "prepare",
                    "type": "wait",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 1},
                    "verify": [],
                },
                {
                    "id": "seed",
                    "type": "copy_files",
                    "name": "Seed",
                    "user_toggleable": True,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.optional_cfg"}, "dest": "/sdcard/Example.cfg"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            cfg = Path(tmp) / "optional.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )

            steps = {step.id: step for step in session.draft_plan.steps}
            seed_step = steps["example.recipe/seed"]
            self.assertFalse(seed_step.selected)
            self.assertEqual(seed_step.availability, Availability.UNAVAILABLE)
            self.assertEqual(tuple(item.id for item in session.draft_plan.inputs), ())

            update = session.bind_input("example.recipe/optional_cfg", str(cfg))
            self.assertFalse(update.errors)
            steps = {step.id: step for step in update.draft_plan.steps}
            seed_step = steps["example.recipe/seed"]
            self.assertTrue(seed_step.selected)
            self.assertEqual(seed_step.availability, Availability.AVAILABLE)
            self.assertEqual(tuple(item.id for item in update.draft_plan.inputs), ("example.recipe/optional_cfg",))

            emitted = session.emit_execution_plan()
            self.assertEqual(emitted.status.value, "success", emitted.errors)
            assert emitted.execution_plan is not None
            self.assertIn("example.recipe/seed", [step.id for step in emitted.execution_plan.steps])
            self.assertEqual(tuple(item.id for item in emitted.execution_plan.inputs), ("example.recipe/optional_cfg",))

    def test_optional_input_unbind_prunes_step_again_and_clears_active_binding(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "optional_cfg": {
                    "type": "file",
                    "role": "generic",
                    "label": "Optional Config",
                    "description": "Optional config file.",
                    "required": False,
                    "multiple": False,
                    "validation": {"must_exist": False, "allowed_extensions": ["cfg"], "path_kind": "file"},
                }
            },
            steps=[
                {
                    "id": "prepare",
                    "type": "wait",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 1},
                    "verify": [],
                },
                {
                    "id": "seed",
                    "type": "copy_files",
                    "name": "Seed",
                    "user_toggleable": True,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.optional_cfg"}, "dest": "/sdcard/Example.cfg"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            cfg = Path(tmp) / "optional.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )

            self.assertFalse(session.bind_input("example.recipe/optional_cfg", str(cfg)).errors)
            update = session.unbind_input("example.recipe/optional_cfg")
            self.assertFalse(update.errors)
            self.assertNotIn("example.recipe/optional_cfg", session._state.user_bindings)

            steps = {step.id: step for step in update.draft_plan.steps}
            seed_step = steps["example.recipe/seed"]
            self.assertFalse(seed_step.selected)
            self.assertEqual(seed_step.availability, Availability.UNAVAILABLE)
            self.assertEqual(tuple(item.id for item in update.draft_plan.inputs), ())

            emitted = session.emit_execution_plan()
            self.assertEqual(emitted.status.value, "success", emitted.errors)
            assert emitted.execution_plan is not None
            self.assertNotIn("example.recipe/seed", [step.id for step in emitted.execution_plan.steps])

    def test_bind_input_rejects_unknown_input_outside_active_session_context(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            catalog = load_authored_catalog(authored_root)
            planner = Planner(catalog)
            session = planner.start_session(
                "example.device_plan",
                DeviceContext(manufacturer="Example", model="Example", android_version=13, android_api_level=33, device_tags=()),
            )

            update = session.bind_input("missing.recipe/unknown_input", "/tmp/value.cfg")
            self.assertEqual(len(update.errors), 1)
            self.assertEqual(update.errors[0].code, ErrorCode.INPUT_NOT_FOUND)
            self.assertEqual(update.errors[0].details["input_id"], "missing.recipe/unknown_input")

    def test_sample_retroarch_flow_emits_grouped_artifacts_and_ordered_steps(self) -> None:
        with TemporaryDirectory() as tmp:
            cfg = Path(tmp) / "retroarch.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")

            catalog = load_authored_catalog("authored")
            planner = Planner(catalog)
            session = planner.start_session(
                "ayaneo.pocket_s_mini.base",
                DeviceContext(manufacturer="AYANEO", model="Pocket S mini", android_version=13, android_api_level=33, device_tags=()),
                runtime_capabilities=catalog.device_profiles["ayaneo.pocket_s_mini"].capability_defaults,
            )
            self.assertFalse(session.bind_input("app.retroarch.provision/retroarch_cfg", str(cfg)).errors)
            result = session.emit_execution_plan()
            self.assertEqual(result.status.value, "success", result.errors)
            plan = result.execution_plan
            assert plan is not None

            step_ids = [step.id for step in plan.steps]
            self.assertIn("app.retroarch.provision/resolve_artifacts", step_ids)
            self.assertIn("app.retroarch.provision/extract_cores", step_ids)
            self.assertIn("app.retroarch.provision/copy_cores", step_ids)
            self.assertIn("app.retroarch.provision/grant_retroarch_permissions", step_ids)
            self.assertLess(
                step_ids.index("app.retroarch.provision/extract_cores"),
                step_ids.index("app.retroarch.provision/copy_cores"),
            )
            self.assertLess(
                step_ids.index("app.retroarch.provision/grant_retroarch_permissions"),
                step_ids.index("app.retroarch.provision/seed_retroarch_cfg"),
            )
            self.assertLess(
                step_ids.index("app.retroarch.provision/copy_cores"),
                step_ids.index("app.retroarch.provision/seed_retroarch_cfg"),
            )
            self.assertTrue(
                any(artifact.id.endswith("/retroarch_apk") for artifact in plan.artifacts),
                plan.artifacts,
            )
            seed_step = next(step for step in plan.steps if step.id == "app.retroarch.provision/seed_retroarch_cfg")
            self.assertEqual(
                seed_step.params["dest"],
                LiteralParamValue(value="/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg"),
            )
            self.assertEqual(len(seed_step.verify), 1)
            self.assertEqual(seed_step.verify[0].type, "file_exists")
            self.assertEqual(
                seed_step.verify[0].params["path"],
                "/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg",
            )

    def test_retroarch_plan_without_optional_cfg_prunes_seed_but_keeps_launch(self) -> None:
        catalog = load_authored_catalog("authored")
        planner = Planner(catalog)
        session = planner.start_session(
            "ayaneo.konkr_pocket_fit.base",
            DeviceContext(manufacturer="AYANEO", model="Pocket FIT", android_version=14, android_api_level=34, device_tags=()),
            runtime_capabilities=catalog.device_profiles["ayaneo.konkr_pocket_fit"].capability_defaults,
        )

        steps = {step.id: step for step in session.draft_plan.steps}
        seed_step = steps["app.retroarch.provision/seed_retroarch_cfg"]
        launch_step = steps["app.retroarch.provision/launch_retroarch"]

        self.assertFalse(seed_step.selected)
        self.assertEqual(seed_step.availability, Availability.UNAVAILABLE)
        assert seed_step.reason is not None
        self.assertEqual(seed_step.reason.code, AvailabilityCode.OPTIONAL_INPUT_UNBOUND)
        self.assertEqual(seed_step.reason.details["input_id"], "app.retroarch.provision/retroarch_cfg")
        self.assertTrue(launch_step.selected)

        result = session.emit_execution_plan()
        self.assertEqual(result.status.value, "success", result.errors)
        plan = result.execution_plan
        assert plan is not None

        step_ids = [step.id for step in plan.steps]
        self.assertIn("app.retroarch.provision/launch_retroarch", step_ids)
        self.assertNotIn("app.retroarch.provision/seed_retroarch_cfg", step_ids)
        self.assertEqual(tuple(item.id for item in plan.inputs), ())


if __name__ == "__main__":
    unittest.main()

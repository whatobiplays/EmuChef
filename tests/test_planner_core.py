from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef.domain import DeviceContext, LiteralParamValue, RefParamValue
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
            permissions={
                "runtime": [
                    {
                        "package_name": "com.example.app",
                        "name": "android.permission.POST_NOTIFICATIONS",
                        "required": False,
                    }
                ]
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
            self.assertIsNotNone(plan.permission_plan)
            assert plan.permission_plan is not None
            self.assertFalse(any(hasattr(action, "command") for action in plan.permission_plan.actions))

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
            self.assertIsNone(emitted.execution_plan.permission_plan)

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
            self.assertIn("app.retroarch.provision/extract_selected_cores", step_ids)
            self.assertIn("app.retroarch.provision/grant_retroarch_permissions", step_ids)
            self.assertLess(
                step_ids.index("app.retroarch.provision/extract_selected_cores"),
                step_ids.index("app.retroarch.provision/copy_selected_cores"),
            )
            self.assertLess(
                step_ids.index("app.retroarch.provision/grant_retroarch_permissions"),
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


if __name__ == "__main__":
    unittest.main()

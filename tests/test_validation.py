from __future__ import annotations

import io
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory

import yaml

from emuchef.cli import main
from emuchef.io import validate_authored_catalog, validate_authored_path


class ValidationTests(unittest.TestCase):
    def test_valid_single_recipe_file(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root)
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "warning")
            self.assertEqual(result.errors, ())
            self.assertEqual(result.warnings[0].code.value, "validation_context_limited")

    def test_invalid_single_file_schema(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "invalid.yaml"
            write_yaml(
                path,
                {
                    "schema_version": 99,
                    "kind": "recipe",
                    "id": "bad.recipe",
                },
            )

            result = validate_authored_path(path)

            self.assertEqual(result.status.value, "error")
            self.assertEqual(result.errors[0].code.value, "authored_data_invalid")

    def test_invalid_step_contract(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, copy_step_params={"input": {"ref": "feature.copy_bios.$bios_source_dir"}})
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "param_contract_violation" for error in result.errors), result.errors)

    def test_wait_with_non_positive_duration_ms_fails(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                extra_steps=[
                    {
                        "id": "wait_invalid",
                        "type": "wait",
                        "name": "Wait",
                        "user_toggleable": False,
                        "dependencies": [],
                        "constraints": {"capabilities": [], "conflicts_with": []},
                        "skip_if": [],
                        "params": {"duration_ms": 0},
                        "verify": [],
                    }
                ],
            )
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "param_contract_violation" for error in result.errors), result.errors)

    def test_force_stop_app_with_empty_package_name_fails(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                extra_steps=[
                    {
                        "id": "stop_app",
                        "type": "force_stop_app",
                        "name": "Stop app",
                        "user_toggleable": False,
                        "dependencies": [],
                        "constraints": {"capabilities": [], "conflicts_with": []},
                        "skip_if": [],
                        "params": {"package_name": ""},
                        "verify": [],
                    }
                ],
            )
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "param_contract_violation" for error in result.errors), result.errors)

    def test_grant_permissions_without_local_permissions_fails(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                extra_steps=[
                    {
                        "id": "grant_permissions",
                        "type": "grant_permissions",
                        "name": "Grant permissions",
                        "user_toggleable": False,
                        "dependencies": [],
                        "constraints": {"capabilities": [], "conflicts_with": []},
                        "skip_if": [],
                        "params": {},
                        "verify": [],
                    }
                ],
            )
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "param_contract_violation" for error in result.errors), result.errors)

    def test_invalid_cross_recipe_dependency_ref(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, recipe_dependencies=["missing.recipe"])
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path, authored_root=authored_root)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "recipe_not_found" for error in result.errors), result.errors)

    def test_invalid_permission_when_range_is_rejected(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                permissions={
                    "runtime": [
                        {
                            "package_name": "com.example.app",
                            "name": "android.permission.POST_NOTIFICATIONS",
                            "when": {"android_api_min": 34, "android_api_max": 33},
                        }
                    ]
                },
            )
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path)

            self.assertEqual(result.status.value, "error")
            self.assertEqual(result.errors[0].code.value, "authored_data_invalid")
            self.assertIn("min 34 exceeds max 33", result.errors[0].message)

    def test_invalid_step_ref_reports_recipe_file_context(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                copy_step_params={
                    "input": {"ref": "feature.copy_bios.$missing_input"},
                    "dest": {"value": "/sdcard/BIOS"},
                    "copy_policy": "sync",
                },
            )
            recipe_path = authored_root / "recipes" / "feature_copy_bios.yaml"

            result = validate_authored_path(recipe_path, authored_root=authored_root)

            error = next(error for error in result.errors if error.code.value == "binding_missing")
            self.assertEqual(error.details["file"], str(recipe_path.resolve()))
            self.assertEqual(error.details["object_kind"], "recipe")
            self.assertEqual(error.details["object_id"], "feature.copy_bios")
            self.assertEqual(error.details["field"], "steps[0].params.input.ref")

    def test_device_plan_missing_device_profile_ref(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, device_profile_ref="missing.profile")
            plan_path = authored_root / "device_plans" / "example_device_plan.yaml"

            result = validate_authored_path(plan_path, authored_root=authored_root)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "device_profile_not_found" for error in result.errors), result.errors)

    def test_missing_recipe_in_device_plan_reports_device_plan_file_context(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, device_plan_recipe_ref="missing.recipe")
            plan_path = authored_root / "device_plans" / "example_device_plan.yaml"

            result = validate_authored_path(plan_path, authored_root=authored_root)

            error = next(error for error in result.errors if error.code.value == "recipe_not_found")
            self.assertEqual(error.details["file"], str(plan_path.resolve()))
            self.assertEqual(error.details["object_kind"], "device_plan")
            self.assertEqual(error.details["object_id"], "example.device_plan")
            self.assertEqual(error.details["field"], "recipes[0].recipe_ref")

    def test_recipe_dependency_cycle(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, add_cycle=True)

            result = validate_authored_catalog(authored_root)

            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "dependency_cycle" for error in result.errors), result.errors)

    def test_full_authored_catalog_success_case(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root)

            result = validate_authored_catalog(authored_root)

            self.assertEqual(result.status.value, "success")
            self.assertEqual(result.errors, ())
            self.assertGreaterEqual(len(result.validated_paths), 4)

    def test_validate_cli_summary_for_catalog(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root)

            rc, stdout, _ = run_cli(["validate", "--authored-root", str(authored_root)])

            self.assertEqual(rc, 0)
            self.assertIn("Validation status: success", stdout)

    def test_validate_cli_groups_issues_by_file(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(
                root,
                recipe_dependencies=["missing.recipe"],
                device_plan_recipe_ref="missing.recipe",
            )

            rc, stdout, _ = run_cli(["validate", "--authored-root", str(authored_root)])

            recipe_path = str((authored_root / "recipes" / "feature_copy_bios.yaml").resolve())
            device_plan_path = str((authored_root / "device_plans" / "example_device_plan.yaml").resolve())
            self.assertEqual(rc, 1)
            self.assertIn(recipe_path, stdout)
            self.assertIn(device_plan_path, stdout)
            self.assertIn("recipe_not_found: Recipe dependency 'missing.recipe' was not found.", stdout)
            self.assertIn("field: recipe_dependencies[0]", stdout)
            self.assertIn(
                "recipe_not_found: Recipe 'missing.recipe' referenced by a device plan was not found.",
                stdout,
            )
            self.assertIn("field: recipes[0].recipe_ref", stdout)

    def test_validate_cli_verbose_includes_structured_details(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_validation_tree(root, device_plan_recipe_ref="missing.recipe")
            plan_path = authored_root / "device_plans" / "example_device_plan.yaml"

            rc, stdout, _ = run_cli(["validate", str(plan_path), "--authored-root", str(authored_root), "--verbose"])

            self.assertEqual(rc, 1)
            self.assertIn("kind: validation_result", stdout)
            self.assertIn("details:", stdout)
            self.assertIn(f"file: {plan_path.resolve()}", stdout)
            self.assertIn("object_kind: device_plan", stdout)
            self.assertIn("object_id: example.device_plan", stdout)
            self.assertIn("field: recipes[0].recipe_ref", stdout)


def run_cli(argv: list[str]) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        rc = main(argv)
    return rc, stdout.getvalue(), stderr.getvalue()


def build_validation_tree(
    root: Path,
    *,
    copy_step_params: dict | None = None,
    extra_steps: list[dict] | None = None,
    permissions: dict | None = None,
    recipe_dependencies: list[str] | None = None,
    device_profile_ref: str = "example.device_profile",
    device_plan_recipe_ref: str = "feature.copy_bios",
    add_cycle: bool = False,
) -> Path:
    authored_root = root / "authored"
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (authored_root / subdir).mkdir(parents=True, exist_ok=True)

    write_yaml(
        authored_root / "apps" / "example_app.yaml",
        {
            "schema_version": 1,
            "kind": "app_definition",
            "id": "example.app",
            "name": "Example App",
            "description": "Example app",
            "category": "utility",
            "package": {"primary": "com.example.app", "aliases": []},
            "install_source": {"type": "local_file", "resolver": "none", "options": {"path": "sample_artifacts/example.apk"}},
            "tracking_source": {
                "type": "obtainium_config_snapshot",
                "config_snapshot": "vendor/obtainium/apps/example.json",
                "app_id": "example-app",
            },
            "artifacts": {
                "apk": {"required": True},
                "shared_storage_config": {"supported": False},
                "app_data_config": {"supported": False},
                "byo_apk": {"required": False},
            },
            "provisioning": {
                "launch_once_recommended": False,
                "shared_storage_paths": [],
                "app_data_paths": [],
                "config_targets": [],
            },
            "inputs": [],
            "metadata": {},
        },
    )
    write_yaml(
        authored_root / "device_profiles" / "example_device_profile.yaml",
        {
            "schema_version": 1,
            "kind": "device_profile",
            "id": "example.device_profile",
            "name": "Example Device Profile",
            "description": "Example profile",
            "match": {
                "manufacturer_contains": ["Example"],
                "brand_contains": ["ExampleBrand"],
                "model_patterns": ["(?i)example"],
                "android_version": {"min": 11},
            },
            "capability_defaults": {
                "adb_available": True,
                "apk_install": True,
                "shared_storage_write": True,
                "app_launch": True,
                "shell_command": True,
                "package_remove_for_user": False,
                "root_shell": False,
                "app_data_write": False,
            },
            "device_tags": ["example_tag"],
            "metadata": {},
        },
    )

    recipe_steps = list(extra_steps or [])
    recipe_steps.append(
        {
            "id": "copy_bios_dir",
            "type": "copy_byo_input",
            "name": "Copy BIOS folder",
            "user_toggleable": True,
            "dependencies": [],
            "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
            "skip_if": [],
            "params": copy_step_params
            or {
                "input": {"ref": "feature.copy_bios.$bios_source_dir"},
                "dest": {"value": "/sdcard/BIOS"},
                "copy_policy": "sync",
            },
            "verify": [],
        }
    )

    recipe_payload = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "feature.copy_bios",
        "name": "Copy BIOS Files",
        "description": "Copy BIOS",
        "recipe_dependencies": recipe_dependencies or [],
        "provides": {"features": ["bios_copy"]},
        "inputs": [
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
        ],
        "steps": recipe_steps,
    }
    if permissions is not None:
        recipe_payload["permissions"] = permissions
    write_yaml(authored_root / "recipes" / "feature_copy_bios.yaml", recipe_payload)

    if add_cycle:
        write_yaml(
            authored_root / "recipes" / "feature_other.yaml",
            {
                "schema_version": 1,
                "kind": "recipe",
                "id": "feature.other",
                "name": "Other Feature",
                "description": "Other recipe",
                "recipe_dependencies": ["feature.copy_bios"],
                "provides": {"features": ["other"]},
                "inputs": [],
                "steps": [],
            },
        )
        recipe_payload["recipe_dependencies"] = ["feature.other"]
        write_yaml(authored_root / "recipes" / "feature_copy_bios.yaml", recipe_payload)

    write_yaml(
        authored_root / "device_plans" / "example_device_plan.yaml",
        {
            "schema_version": 1,
            "kind": "device_plan",
            "id": "example.device_plan",
            "name": "Example Device Plan",
            "description": "Example plan",
            "device_profile_ref": device_profile_ref,
            "recipes": [{"recipe_ref": device_plan_recipe_ref, "selected_by_default": True}],
            "defaults": {},
            "overrides": {},
            "metadata": {},
        },
    )
    return authored_root


def write_yaml(path: Path, payload: dict) -> None:
    path.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")


if __name__ == "__main__":
    unittest.main()

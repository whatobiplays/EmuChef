from __future__ import annotations

from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
import contextlib
import unittest

from emuchef.cli import main
from emuchef.io import validate_authored_catalog, validate_authored_path

from support import base_recipe, build_authored_tree


class ValidationTests(unittest.TestCase):
    def test_valid_single_recipe_file_returns_warning_without_catalog_context(self) -> None:
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
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_path(authored_root / "recipes" / "example_recipe.yaml")
            self.assertEqual(result.status.value, "warning")
            self.assertEqual(len(result.errors), 0)
            self.assertEqual(len(result.warnings), 1)
            self.assertEqual(result.warnings[0].code.value, "validation_context_limited")

    def test_invalid_single_file_schema_reports_mapping_error(self) -> None:
        with TemporaryDirectory() as tmp:
            authored_root = Path(tmp) / "authored"
            (authored_root / "recipes").mkdir(parents=True)
            (authored_root / "recipes" / "bad_recipe.yaml").write_text(
                """
schema_version: 1
kind: recipe
id: bad.recipe
name: Bad Recipe
inputs:
  - id: config
    type: file
steps: []
""".strip(),
                encoding="utf-8",
            )
            result = validate_authored_path(authored_root / "recipes" / "bad_recipe.yaml")
            self.assertEqual(result.status.value, "error")
            self.assertIn("recipe inputs must be a mapping", result.errors[0].message)

    def test_invalid_step_contract_reports_file_context(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "wait",
                    "type": "wait",
                    "name": "Wait",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 0},
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "error")
            error = next(error for error in result.errors if error.code.value == "param_contract_violation")
            self.assertEqual(error.details["object_kind"], "recipe")
            self.assertEqual(error.details["object_id"], "example.recipe")
            self.assertEqual(error.details["field"], "steps[0].params.duration_ms")

    def test_invalid_cross_file_recipe_dependency_ref(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", recipe_dependencies=["missing.recipe"], steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "recipe_not_found" for error in result.errors), result.errors)

    def test_device_plan_with_missing_device_profile_ref(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            device_plan_path = authored_root / "device_plans" / "example_device_plan.yaml"
            device_plan_path.write_text(
                device_plan_path.read_text(encoding="utf-8").replace("example.device_profile", "missing.profile"),
                encoding="utf-8",
            )
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "error")
            self.assertTrue(any(error.code.value == "device_profile_not_found" for error in result.errors), result.errors)

    def test_full_authored_catalog_success_case(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"app_apk": {"type": "remote_file", "url": "https://example.com/app.apk"}},
            steps=[
                {
                    "id": "resolve",
                    "type": "resolve_artifacts",
                    "name": "Resolve",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": ["app_apk"]},
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
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            result = validate_authored_catalog(authored_root)
            self.assertEqual(result.status.value, "success", result.errors)

    def test_validate_cli_groups_issues_by_file(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", recipe_dependencies=["missing.recipe"], steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            stdout = StringIO()
            stderr = StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                rc = main(["validate", "--authored-root", str(authored_root)])
            self.assertEqual(rc, 1)
            output = stdout.getvalue()
            self.assertIn("Issues:", output)
            self.assertIn(str((authored_root / "recipes" / "example_recipe.yaml").resolve()), output)
            self.assertIn("recipe_not_found: Recipe dependency 'missing.recipe' was not found.", output)


if __name__ == "__main__":
    unittest.main()

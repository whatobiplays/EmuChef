from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import yaml

from emuchef.io import load_authored_catalog


class TemplateTests(unittest.TestCase):
    def test_loader_ignores_sibling_templates_directory(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = _build_minimal_authored_tree(root / "authored")
            templates_root = root / "templates" / "authored"
            templates_root.mkdir(parents=True, exist_ok=True)
            (templates_root / "recipe.template.yaml").write_text(
                yaml.safe_dump(
                    {
                        "schema_version": 999,
                        "kind": "recipe",
                        "id": "example.recipe.template",
                    }
                ),
                encoding="utf-8",
            )

            catalog = load_authored_catalog(authored_root)

            self.assertEqual(tuple(catalog.apps), ("example.app",))
            self.assertEqual(tuple(catalog.recipes), ("example.recipe",))
            self.assertEqual(tuple(catalog.device_profiles), ("example.device_profile",))
            self.assertEqual(tuple(catalog.device_plans), ("example.device_plan",))
            self.assertNotIn("example.recipe.template", catalog.recipes)

    def test_template_files_are_outside_authored_and_have_expected_top_level_schema(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        authored_root = repo_root / "authored"
        template_dir = repo_root / "templates" / "authored"
        expected = {
            "app_definition.template.yaml": "app_definition",
            "recipe.template.yaml": "recipe",
            "device_profile.template.yaml": "device_profile",
            "device_plan.template.yaml": "device_plan",
        }

        for filename, expected_kind in expected.items():
            path = template_dir / filename
            self.assertTrue(path.exists(), msg=f"Missing template file: {path}")
            self.assertNotIn(authored_root, path.parents)

            data = yaml.safe_load(path.read_text(encoding="utf-8"))
            self.assertIsInstance(data, dict)
            self.assertEqual(data["schema_version"], 1)
            self.assertEqual(data["kind"], expected_kind)
            if filename == "recipe.template.yaml":
                params = data["steps"][0]["params"]
                self.assertEqual(params["copy_policy"], "sync")
                self.assertNotIn("overwrite", params)


def _build_minimal_authored_tree(authored_root: Path) -> Path:
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (authored_root / subdir).mkdir(parents=True, exist_ok=True)

    _write_yaml(
        authored_root / "apps" / "example_app.yaml",
        {
            "schema_version": 1,
            "kind": "app_definition",
            "id": "example.app",
            "name": "Example App",
            "description": "Minimal app for loader tests.",
            "category": "utility",
            "package": {
                "primary": "com.example.app",
                "aliases": [],
            },
            "install_source": {
                "type": "local_file",
                "resolver": "none",
                "options": {
                    "path": "sample_artifacts/example-app.apk",
                },
            },
            "tracking_source": {
                "type": "obtainium_config_snapshot",
                "config_snapshot": "vendor/obtainium/apps/example-app.json",
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
    _write_yaml(
        authored_root / "recipes" / "example_recipe.yaml",
        {
            "schema_version": 1,
            "kind": "recipe",
            "id": "example.recipe",
            "name": "Example Recipe",
            "description": "Minimal recipe for loader tests.",
            "recipe_dependencies": [],
            "provides": {"features": ["example_feature"]},
            "inputs": [],
            "steps": [],
        },
    )
    _write_yaml(
        authored_root / "device_profiles" / "example_device_profile.yaml",
        {
            "schema_version": 1,
            "kind": "device_profile",
            "id": "example.device_profile",
            "name": "Example Device Profile",
            "description": "Minimal device profile for loader tests.",
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
    _write_yaml(
        authored_root / "device_plans" / "example_device_plan.yaml",
        {
            "schema_version": 1,
            "kind": "device_plan",
            "id": "example.device_plan",
            "name": "Example Device Plan",
            "description": "Minimal device plan for loader tests.",
            "device_profile_ref": "example.device_profile",
            "recipes": [
                {
                    "recipe_ref": "example.recipe",
                    "selected_by_default": True,
                }
            ],
            "defaults": {},
            "overrides": {},
            "metadata": {},
        },
    )
    return authored_root


def _write_yaml(path: Path, payload: dict) -> None:
    path.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")


if __name__ == "__main__":
    unittest.main()

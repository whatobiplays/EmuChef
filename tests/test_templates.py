from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import yaml

from emuchef.io import load_authored_catalog

from support import base_recipe, build_authored_tree


class TemplateTests(unittest.TestCase):
    def test_loader_ignores_sibling_templates_directory(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_authored_tree(root, recipes=[recipe])
            templates_root = root / "templates" / "authored"
            templates_root.mkdir(parents=True, exist_ok=True)
            (templates_root / "recipe.template.yaml").write_text(
                yaml.safe_dump({"schema_version": 999, "kind": "recipe", "id": "example.recipe.template"}),
                encoding="utf-8",
            )

            catalog = load_authored_catalog(authored_root)
            self.assertIn("example.recipe", catalog.recipes)
            self.assertNotIn("example.recipe.template", catalog.recipes)

    def test_template_files_match_current_schema_shape(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        template_dir = repo_root / "templates" / "authored"
        recipe_template = yaml.safe_load((template_dir / "recipe.template.yaml").read_text(encoding="utf-8"))

        self.assertEqual(recipe_template["schema_version"], 1)
        self.assertEqual(recipe_template["kind"], "recipe")
        self.assertIsInstance(recipe_template["inputs"], dict)
        self.assertIsInstance(recipe_template["artifacts"], dict)
        self.assertIsInstance(recipe_template["artifact_groups"], dict)
        self.assertEqual(recipe_template["steps"][0]["type"], "resolve_artifacts")
        self.assertIn("copy_files", {step["type"] for step in recipe_template["steps"]})
        self.assertNotIn("copy_byo_input", yaml.safe_dump(recipe_template))


if __name__ == "__main__":
    unittest.main()

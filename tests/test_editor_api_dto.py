from __future__ import annotations

import json
import sys
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef_editor.api.dto import (
    diagnostic_to_dto,
    document_to_dto,
    ref_index_to_dto,
    step_specs_to_dto,
)
from emuchef_editor.core.yaml.loader import load_recipe_document
from support import base_recipe, build_authored_tree


class EditorApiDtoTests(unittest.TestCase):
    def test_document_ref_index_step_specs_and_diagnostics_are_json_serializable(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"archive_zip": {"type": "remote_file", "url": "https://example.com/archive.zip"}},
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "runtime": [
                            {
                                "package_name": "com.example.app",
                                "name": "android.permission.POST_NOTIFICATIONS",
                                "required": False,
                            }
                        ]
                    },
                    "verify": [],
                },
                {
                    "id": "wait",
                    "type": "wait",
                    "name": "Wait",
                    "user_toggleable": False,
                    "dependencies": ["grant"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 0},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            document = load_recipe_document(authored_root / "recipes" / "example_recipe.yaml", authored_root=authored_root)

            document_dto = document_to_dto(document, document_id="doc-1")
            ref_index_dto = ref_index_to_dto(document.ref_index)
            step_specs_dto = step_specs_to_dto()
            diagnostic_dto = diagnostic_to_dto(document.validation_result.diagnostics[0])

            json.dumps(document_dto)
            json.dumps(ref_index_dto)
            json.dumps(step_specs_dto)
            json.dumps(diagnostic_dto)

            self.assertNotIn("stepSpecs", document_dto)
            self.assertNotIn("permissions", document_dto["recipe"])
            self.assertEqual(document_dto["recipe"]["steps"][0]["type"], "grant_permissions")
            self.assertIn("runtime", document_dto["recipe"]["steps"][0]["params"])
            self.assertIsInstance(diagnostic_dto["file"], str)

    def test_api_dto_imports_do_not_load_pyside_or_app_package(self) -> None:
        loaded_modules = set(sys.modules)

        self.assertFalse(any(name == "PySide6" or name.startswith("PySide6.") for name in loaded_modules))
        self.assertFalse(any(name == "emuchef_editor.app" or name.startswith("emuchef_editor.app.") for name in loaded_modules))


if __name__ == "__main__":
    unittest.main()

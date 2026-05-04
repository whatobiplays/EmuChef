from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

from support import base_recipe, build_authored_tree


class EditorApiServerTests(unittest.TestCase):
    def _run_server(self, request: str | dict, *, stdin: bool = False) -> dict:
        args = [sys.executable, "-m", "emuchef_editor.api.server"]
        input_text = None
        if stdin:
            input_text = request if isinstance(request, str) else json.dumps(request)
        else:
            args.append(request if isinstance(request, str) else json.dumps(request))
        completed = subprocess.run(
            args,
            cwd=Path(__file__).resolve().parents[1],
            input=input_text,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return json.loads(completed.stdout)

    def test_stateless_server_requests_work_through_entrypoint(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            step_specs = self._run_server({"type": "listStepSpecs"}, stdin=True)
            opened = self._run_server(
                {
                    "type": "openRecipe",
                    "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                }
            )
            validated = self._run_server(
                {
                    "type": "validateRecipePath",
                    "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                }
            )
            emitted = self._run_server(
                {
                    "type": "emitRecipeYamlFromPath",
                    "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                }
            )

            self.assertTrue(step_specs["ok"])
            self.assertIn("stepSpecs", step_specs["result"])
            self.assertTrue(opened["ok"])
            self.assertEqual(opened["result"]["document"]["recipe"]["id"], "example.recipe")
            self.assertTrue(validated["ok"])
            self.assertIn("diagnostics", validated["result"])
            self.assertTrue(emitted["ok"])
            self.assertIn("id: example.recipe", emitted["result"]["yaml"])

    def test_server_invalid_json_unknown_request_and_debug_behavior(self) -> None:
        invalid_json = self._run_server("{not-json", stdin=True)
        unknown = self._run_server({"type": "unknownRequest"})
        debug_failure = self._run_server(
            {
                "type": "openRecipe",
                "debug": True,
                "payload": {"path": "/path/that/does/not/exist.yaml"},
            }
        )
        normal_failure = self._run_server(
            {
                "type": "openRecipe",
                "payload": {"path": "/path/that/does/not/exist.yaml"},
            }
        )

        self.assertFalse(invalid_json["ok"])
        self.assertEqual(invalid_json["error"]["code"], "invalid_request")
        self.assertNotIn("debug", invalid_json)

        self.assertFalse(unknown["ok"])
        self.assertEqual(unknown["error"]["code"], "invalid_request")

        self.assertFalse(debug_failure["ok"])
        self.assertIn("debug", debug_failure)
        self.assertEqual(debug_failure["debug"]["requestType"], "openRecipe")
        self.assertTrue(debug_failure["debug"]["exceptionType"])
        self.assertTrue(debug_failure["debug"]["traceback"])

        self.assertFalse(normal_failure["ok"])
        self.assertNotIn("debug", normal_failure)


if __name__ == "__main__":
    unittest.main()

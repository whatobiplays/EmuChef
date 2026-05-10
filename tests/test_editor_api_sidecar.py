from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

from support import base_recipe, build_authored_tree


class SidecarProcess:
    def __init__(self) -> None:
        self.process = subprocess.Popen(
            [sys.executable, "-m", "emuchef_editor.api.server", "--sidecar"],
            cwd=Path(__file__).resolve().parents[1],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def request(self, request: dict) -> dict:
        return self.raw_line(json.dumps(request))

    def raw_line(self, line: str) -> dict:
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.process.stdin.write(line)
        self.process.stdin.write("\n")
        self.process.stdin.flush()
        response_line = self.process.stdout.readline()
        self.assert_running_stdout_line(response_line)
        return json.loads(response_line)

    def close(self) -> tuple[int, str]:
        if self.process.stdin is not None:
            self.process.stdin.close()
        returncode = self.process.wait(timeout=10)
        stdout_tail = self.process.stdout.read() if self.process.stdout is not None else ""
        stderr = self.process.stderr.read() if self.process.stderr is not None else ""
        if self.process.stdout is not None:
            self.process.stdout.close()
        if self.process.stderr is not None:
            self.process.stderr.close()
        if stdout_tail:
            raise AssertionError(f"sidecar wrote unexpected stdout after EOF: {stdout_tail!r}")
        return returncode, stderr

    def kill(self) -> None:
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=10)
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        if self.process.stdout is not None and not self.process.stdout.closed:
            self.process.stdout.close()
        if self.process.stderr is not None and not self.process.stderr.closed:
            self.process.stderr.close()

    @staticmethod
    def assert_running_stdout_line(response_line: str) -> None:
        if response_line == "":
            raise AssertionError("sidecar exited without writing a response line")


class EditorApiSidecarTests(unittest.TestCase):
    REQUIRED_HELLO_CAPABILITIES = {
        "listStepSpecs",
        "openRecipe",
        "getDocument",
        "applyRecipeCommand",
        "undo",
        "redo",
        "saveRecipe",
        "validate",
        "emitYaml",
        "getRefIndex",
    }
    OPTIONAL_HELLO_CAPABILITIES = {
        "createRecipeFromTemplate",
        "closeDocument",
        "saveRecipeAs",
    }

    def assert_hello_response(self, response: dict, request_id: str) -> None:
        self.assertEqual(response["id"], request_id)
        self.assertTrue(response["ok"])
        result = response["result"]
        self.assertEqual(result["protocolVersion"], 1)
        self.assertIsInstance(result["capabilities"], list)
        self.assertTrue(self.REQUIRED_HELLO_CAPABILITIES.issubset(set(result["capabilities"])))
        self.assertTrue(self.OPTIONAL_HELLO_CAPABILITIES.issubset(set(result["capabilities"])))
        self.assertNotIn("implementation", result)
        self.assertNotIn("implementationVersion", result)

    def test_hello_requires_no_document_and_can_repeat(self) -> None:
        sidecar = SidecarProcess()
        try:
            first = sidecar.request({"id": "hello-1", "type": "hello"})
            second = sidecar.request({"id": "hello-2", "type": "hello", "payload": {}})
            unknown_keys = sidecar.request({"id": "hello-3", "type": "hello", "payload": {"ignored": True}})
            recovered = sidecar.request({"id": "after-hello", "type": "listStepSpecs", "payload": {}})
            returncode, stderr = sidecar.close()
        finally:
            sidecar.kill()

        self.assertEqual(returncode, 0, stderr)
        self.assert_hello_response(first, "hello-1")
        self.assert_hello_response(second, "hello-2")
        self.assert_hello_response(unknown_keys, "hello-3")
        self.assertEqual(recovered["id"], "after-hello")
        self.assertTrue(recovered["ok"])

    def test_hello_rejects_non_object_payload(self) -> None:
        sidecar = SidecarProcess()
        try:
            malformed = sidecar.request({"id": "hello-bad", "type": "hello", "payload": []})
            recovered = sidecar.request({"id": "after-bad-hello", "type": "listStepSpecs", "payload": {}})
            returncode, stderr = sidecar.close()
        finally:
            sidecar.kill()

        self.assertEqual(returncode, 0, stderr)
        self.assertEqual(malformed["id"], "hello-bad")
        self.assertFalse(malformed["ok"])
        self.assertEqual(malformed["error"]["code"], "invalid_request")
        self.assertEqual(recovered["id"], "after-bad-hello")
        self.assertTrue(recovered["ok"])

    def test_subprocess_sidecar_preserves_document_session_across_requests(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            save_as_path = Path(tmp) / "saved_as.yaml"
            sidecar = SidecarProcess()
            try:
                step_specs = sidecar.request({"id": "req-001", "type": "listStepSpecs", "payload": {}})
                opened = sidecar.request(
                    {
                        "id": "req-002",
                        "type": "openRecipe",
                        "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                    }
                )
                document_id = opened["result"]["document"]["documentId"]
                original_yaml = opened["result"]["document"]["yaml"]

                fetched = sidecar.request(
                    {"id": "req-003", "type": "getDocument", "payload": {"documentId": document_id}}
                )
                changed = sidecar.request(
                    {
                        "id": "req-004",
                        "type": "applyRecipeCommand",
                        "payload": {
                            "documentId": document_id,
                            "command": {"type": "SetOverviewField", "field": "name", "value": "Sidecar Rename"},
                        },
                    }
                )
                undone = sidecar.request({"id": "req-005", "type": "undo", "payload": {"documentId": document_id}})
                redone = sidecar.request({"id": "req-006", "type": "redo", "payload": {"documentId": document_id}})
                validated = sidecar.request(
                    {"id": "req-007", "type": "validate", "payload": {"documentId": document_id}}
                )
                emitted = sidecar.request(
                    {"id": "req-008", "type": "emitYaml", "payload": {"documentId": document_id}}
                )
                ref_index = sidecar.request(
                    {"id": "req-009", "type": "getRefIndex", "payload": {"documentId": document_id}}
                )
                saved = sidecar.request(
                    {"id": "req-010", "type": "saveRecipe", "payload": {"documentId": document_id}}
                )
                saved_as = sidecar.request(
                    {
                        "id": "req-011",
                        "type": "saveRecipeAs",
                        "payload": {"documentId": document_id, "path": str(save_as_path)},
                    }
                )
                closed = sidecar.request(
                    {"id": "req-012", "type": "closeDocument", "payload": {"documentId": document_id}}
                )
                after_close = sidecar.request(
                    {"id": "req-013", "type": "getDocument", "payload": {"documentId": document_id}}
                )
                returncode, stderr = sidecar.close()
            finally:
                sidecar.kill()

            self.assertEqual(returncode, 0, stderr)
            for response in (
                step_specs,
                opened,
                fetched,
                changed,
                undone,
                redone,
                validated,
                emitted,
                ref_index,
                saved,
                saved_as,
                closed,
                after_close,
            ):
                self.assertIn("id", response)

            self.assertEqual(step_specs["id"], "req-001")
            self.assertTrue(step_specs["ok"])
            self.assertIn("stepSpecs", step_specs["result"])

            self.assertEqual(opened["id"], "req-002")
            self.assertTrue(opened["ok"])
            self.assertEqual(fetched["result"]["document"]["documentId"], document_id)
            self.assertTrue(changed["ok"])
            self.assertTrue(changed["result"]["commandResult"]["changed"])
            self.assertIn("name: Sidecar Rename", changed["result"]["document"]["yaml"])
            self.assertEqual(undone["result"]["document"]["yaml"], original_yaml)
            self.assertIn("name: Sidecar Rename", redone["result"]["document"]["yaml"])
            self.assertIn("diagnostics", validated["result"])
            self.assertNotIn("document", validated["result"])
            self.assertIn("yaml", emitted["result"])
            self.assertIn("refIndex", ref_index["result"])
            self.assertFalse(saved["result"]["document"]["dirty"])
            self.assertEqual(saved_as["result"]["document"]["path"], str(save_as_path.resolve()))
            self.assertTrue(save_as_path.exists())
            self.assertTrue(closed["ok"])
            self.assertFalse(after_close["ok"])
            self.assertEqual(after_close["error"]["code"], "unknown_document")

    def test_subprocess_sidecar_preserves_reordered_artifact_group_order(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifact_groups={
                "first_group": [],
                "second_group": [],
                "third_group": [],
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            sidecar = SidecarProcess()
            try:
                opened = sidecar.request(
                    {
                        "id": "open",
                        "type": "openRecipe",
                        "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                    }
                )
                document_id = opened["result"]["document"]["documentId"]
                changed = sidecar.request(
                    {
                        "id": "move",
                        "type": "applyRecipeCommand",
                        "payload": {
                            "documentId": document_id,
                            "command": {"type": "ReorderArtifactGroup", "groupId": "third_group", "toIndex": 0},
                        },
                    }
                )
                returncode, stderr = sidecar.close()
            finally:
                sidecar.kill()

            self.assertEqual(returncode, 0, stderr)
            self.assertTrue(changed["ok"])
            self.assertEqual(
                list(changed["result"]["document"]["recipe"]["artifactGroups"]),
                ["third_group", "first_group", "second_group"],
            )

    def test_sidecar_returns_structured_errors_and_continues_after_invalid_lines(self) -> None:
        sidecar = SidecarProcess()
        try:
            malformed = sidecar.raw_line("{not-json")
            unknown = sidecar.request({"id": "after-malformed", "type": "missingType", "payload": {}})
            recovered = sidecar.request({"id": "after-error", "type": "listStepSpecs", "payload": {}})
            returncode, stderr = sidecar.close()
        finally:
            sidecar.kill()

        self.assertEqual(returncode, 0, stderr)
        self.assertIsNone(malformed["id"])
        self.assertFalse(malformed["ok"])
        self.assertEqual(malformed["error"]["code"], "invalid_request")
        self.assertFalse(unknown["ok"])
        self.assertEqual(unknown["id"], "after-malformed")
        self.assertEqual(unknown["error"]["code"], "invalid_request")
        self.assertTrue(recovered["ok"])
        self.assertEqual(recovered["id"], "after-error")

    def test_debug_context_is_opt_in_for_sidecar_failures(self) -> None:
        sidecar = SidecarProcess()
        try:
            debug_failure = sidecar.request({"id": "debug", "type": "getDocument", "debug": True, "payload": {}})
            normal_failure = sidecar.request({"id": "normal", "type": "getDocument", "payload": {}})
            returncode, stderr = sidecar.close()
        finally:
            sidecar.kill()

        self.assertEqual(returncode, 0, stderr)
        self.assertFalse(debug_failure["ok"])
        self.assertEqual(debug_failure["error"]["code"], "invalid_request")
        self.assertEqual(debug_failure["debug"]["requestType"], "getDocument")
        self.assertEqual(debug_failure["debug"]["exceptionType"], "ApiError")
        self.assertTrue(debug_failure["debug"]["traceback"])
        self.assertFalse(normal_failure["ok"])
        self.assertNotIn("debug", normal_failure)

    def test_invalid_command_returns_invalid_command(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            sidecar = SidecarProcess()
            try:
                opened = sidecar.request(
                    {
                        "id": "open",
                        "type": "openRecipe",
                        "payload": {"path": str(recipe_path), "authoredRoot": str(authored_root)},
                    }
                )
                document_id = opened["result"]["document"]["documentId"]
                invalid = sidecar.request(
                    {
                        "id": "invalid-command",
                        "type": "applyRecipeCommand",
                        "payload": {"documentId": document_id, "command": {"type": "NoSuchCommand"}},
                    }
                )
                returncode, stderr = sidecar.close()
            finally:
                sidecar.kill()

            self.assertEqual(returncode, 0, stderr)
            self.assertFalse(invalid["ok"])
            self.assertEqual(invalid["id"], "invalid-command")
            self.assertEqual(invalid["error"]["code"], "invalid_command")

    def test_create_recipe_from_template_returns_session_document(self) -> None:
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_authored_tree(
                root,
                recipes=[],
                recipe_templates={"recipe.template.yaml": template},
            )
            destination_path = authored_root / "recipes" / "created_recipe.yaml"
            sidecar = SidecarProcess()
            try:
                created = sidecar.request(
                    {
                        "id": "create-template",
                        "type": "createRecipeFromTemplate",
                        "payload": {
                            "templatePath": str(root / "templates" / "authored" / "recipe.template.yaml"),
                            "destinationPath": str(destination_path),
                            "recipeId": "created.recipe",
                            "authoredRoot": str(authored_root),
                        },
                    }
                )
                document_id = created["result"]["document"]["documentId"]
                fetched = sidecar.request(
                    {"id": "get-created", "type": "getDocument", "payload": {"documentId": document_id}}
                )
                returncode, stderr = sidecar.close()
            finally:
                sidecar.kill()

            self.assertEqual(returncode, 0, stderr)
            self.assertTrue(created["ok"])
            self.assertEqual(created["result"]["document"]["recipe"]["id"], "created.recipe")
            self.assertEqual(created["result"]["document"]["path"], str(destination_path.resolve()))
            self.assertFalse(created["result"]["document"]["dirty"])
            self.assertTrue(destination_path.exists())
            self.assertEqual(fetched["result"]["document"]["documentId"], document_id)


if __name__ == "__main__":
    unittest.main()

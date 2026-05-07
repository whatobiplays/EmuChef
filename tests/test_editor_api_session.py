from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from emuchef_editor.api.session import DocumentSessionManager
from support import base_recipe, build_authored_tree


class EditorApiSessionTests(unittest.TestCase):
    def test_open_apply_undo_redo_save_save_as_and_unknown_document(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            manager = DocumentSessionManager()

            opened = manager.open_recipe(str(recipe_path), authored_root=str(authored_root))

            self.assertTrue(opened["ok"])
            document = opened["result"]["document"]
            original_yaml = document["yaml"]
            document_id = document["documentId"]
            self.assertEqual(document["path"], str(recipe_path.resolve()))
            self.assertEqual(document["authoredRoot"], str(authored_root.resolve()))
            self.assertFalse(document["dirty"])
            self.assertFalse(document["canUndo"])
            self.assertFalse(document["canRedo"])
            self.assertIn("recipe", document)
            self.assertIn("yaml", document)
            self.assertIn("diagnostics", document)
            self.assertIn("refIndex", document)
            self.assertNotIn("stepSpecs", document)
            self.assertNotIn("permissions", document["recipe"])

            changed = manager.apply_recipe_command(
                document_id,
                {"type": "SetOverviewField", "field": "name", "value": "Updated Recipe"},
            )

            self.assertTrue(changed["ok"])
            self.assertTrue(changed["result"]["commandResult"]["changed"])
            changed_document = changed["result"]["document"]
            self.assertTrue(changed_document["dirty"])
            self.assertNotEqual(changed_document["yaml"], original_yaml)
            self.assertIn("name: Updated Recipe", changed_document["yaml"])

            undone = manager.undo(document_id)
            self.assertTrue(undone["ok"])
            self.assertEqual(undone["result"]["document"]["yaml"], original_yaml)

            redone = manager.redo(document_id)
            self.assertTrue(redone["ok"])
            self.assertIn("name: Updated Recipe", redone["result"]["document"]["yaml"])

            saved = manager.save_recipe(document_id)
            self.assertTrue(saved["ok"])
            self.assertFalse(saved["result"]["document"]["dirty"])

            save_as_path = Path(tmp) / "saved_as.yaml"
            saved_as = manager.save_recipe_as(document_id, str(save_as_path))
            self.assertTrue(saved_as["ok"])
            self.assertEqual(saved_as["result"]["document"]["path"], str(save_as_path.resolve()))
            self.assertTrue(save_as_path.exists())

            validated = manager.validate(document_id)
            self.assertTrue(validated["ok"])
            self.assertIn("diagnostics", validated["result"])
            self.assertNotIn("document", validated["result"])

            unknown = manager.get_document("missing-document")
            self.assertFalse(unknown["ok"])
            self.assertEqual(unknown["error"]["code"], "unknown_document")

    def test_apply_invalid_command_returns_structured_invalid_command(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            manager = DocumentSessionManager()
            opened = manager.open_recipe(str(authored_root / "recipes" / "example_recipe.yaml"), authored_root=str(authored_root))
            document_id = opened["result"]["document"]["documentId"]

            response = manager.apply_recipe_command(document_id, {"type": "NoSuchCommand", "inputId": "missing"})

            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "invalid_command")

    def test_section_add_rename_and_duplicate_commands_update_document(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "source_zip": {"type": "remote_file", "url": "https://example.com/source.zip"},
                "member_zip": {"type": "remote_file", "url": "https://example.com/member.zip"},
            },
            artifact_groups={"bundle": ["member_zip"]},
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            manager = DocumentSessionManager()
            opened = manager.open_recipe(str(authored_root / "recipes" / "example_recipe.yaml"), authored_root=str(authored_root))
            document_id = opened["result"]["document"]["documentId"]

            commands = [
                {"type": "AddInput", "inputId": "new_input"},
                {"type": "RenameInput", "inputId": "new_input", "newInputId": "renamed_input"},
                {"type": "DuplicateInput", "sourceInputId": "renamed_input", "newInputId": "renamed_input_copy"},
                {"type": "AddArtifact", "artifactId": "new_artifact", "url": "https://example.com/new.zip"},
                {"type": "RenameArtifact", "artifactId": "new_artifact", "newArtifactId": "renamed_artifact"},
                {"type": "DuplicateArtifact", "sourceArtifactId": "renamed_artifact", "newArtifactId": "renamed_artifact_copy"},
                {"type": "AddArtifactGroup", "groupId": "new_bundle"},
                {"type": "RenameArtifactGroup", "groupId": "new_bundle", "newGroupId": "renamed_bundle"},
                {"type": "DuplicateArtifactGroup", "sourceGroupId": "bundle", "newGroupId": "bundle_copy"},
            ]

            for command in commands:
                response = manager.apply_recipe_command(document_id, command)
                self.assertTrue(response["ok"], command)

            document = manager.get_document(document_id)["result"]["document"]
            recipe_dto = document["recipe"]
            self.assertIn("renamed_input", recipe_dto["inputs"])
            self.assertIn("renamed_input_copy", recipe_dto["inputs"])
            self.assertIn("renamed_artifact", recipe_dto["artifacts"])
            self.assertIn("renamed_artifact_copy", recipe_dto["artifacts"])
            self.assertIn("renamed_bundle", recipe_dto["artifactGroups"])
            self.assertEqual(recipe_dto["artifactGroups"]["bundle_copy"], ["member_zip"])

    def test_step_lifecycle_commands_update_document_and_delete_step_safely(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "prepare",
                    "type": "extract_archive",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "steps.seed.outputs.copied_paths"}},
                    "verify": [],
                },
                {
                    "id": "consume",
                    "type": "copy_files",
                    "name": "Consume",
                    "user_toggleable": False,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": [], "conflicts_with": ["prepare"]},
                    "params": {"source": {"ref": "steps.prepare.outputs.extracted_path"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
                {
                    "id": "consume_shorthand",
                    "type": "copy_files",
                    "name": "Consume Shorthand",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "steps.prepare"}, "dest": "/sdcard/Example2"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            manager = DocumentSessionManager()
            opened = manager.open_recipe(
                str(authored_root / "recipes" / "example_recipe.yaml"),
                authored_root=str(authored_root),
            )
            document_id = opened["result"]["document"]["documentId"]

            commands = [
                {"type": "AddStep", "stepId": "pause", "stepType": "wait", "name": "Pause", "index": 1},
                {
                    "type": "UpdateStepBasics",
                    "stepId": "pause",
                    "name": "Wait for boot",
                    "description": "Delay before launch.",
                },
                {"type": "SetStepUserToggleable", "stepId": "pause", "userToggleable": True},
                {"type": "DuplicateStep", "sourceStepId": "pause", "newStepId": "pause_copy"},
                {"type": "ReorderStep", "stepId": "pause_copy", "toIndex": 0},
                {"type": "DeleteStep", "stepId": "prepare"},
            ]

            for command in commands:
                response = manager.apply_recipe_command(document_id, command)
                self.assertTrue(response["ok"], command)
                self.assertTrue(response["result"]["commandResult"]["changed"], command)
                self.assertEqual(response["result"]["document"]["documentId"], document_id)

            document = manager.get_document(document_id)["result"]["document"]
            steps = document["recipe"]["steps"]
            step_ids = [step["id"] for step in steps]
            self.assertNotIn("prepare", step_ids)
            self.assertEqual(step_ids[0], "pause_copy")

            pause_step = next(step for step in steps if step["id"] == "pause")
            self.assertEqual(pause_step["name"], "Wait for boot")
            self.assertEqual(pause_step["description"], "Delay before launch.")
            self.assertTrue(pause_step["userToggleable"])

            consume_step = next(step for step in steps if step["id"] == "consume")
            shorthand_step = next(step for step in steps if step["id"] == "consume_shorthand")
            self.assertEqual(consume_step["dependencies"], [])
            self.assertEqual(consume_step["constraints"]["conflictsWith"], [])
            self.assertNotIn("source", consume_step["params"])
            self.assertNotIn("source", shorthand_step["params"])
            self.assertNotIn("steps.prepare", document["yaml"])

    def test_update_step_params_command_returns_updated_document_without_dependency_synthesis(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "extract",
                    "type": "extract_artifacts",
                    "name": "Extract",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"artifacts": []},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"dest": "/sdcard/Old", "copy_policy": "sync"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            authored_root = build_authored_tree(Path(tmp), recipes=[recipe])
            manager = DocumentSessionManager()
            opened = manager.open_recipe(
                str(authored_root / "recipes" / "example_recipe.yaml"),
                authored_root=str(authored_root),
            )
            document_id = opened["result"]["document"]["documentId"]

            response = manager.apply_recipe_command(
                document_id,
                {
                    "type": "UpdateStepParams",
                    "stepId": "copy",
                    "params": {
                        "source": {"ref": "steps.extract.outputs.extracted_paths"},
                        "dest": "/sdcard/New",
                        "copy_policy": "sync",
                    },
                },
            )

            self.assertTrue(response["ok"], response)
            self.assertTrue(response["result"]["commandResult"]["changed"])
            document = response["result"]["document"]
            self.assertEqual(document["documentId"], document_id)
            self.assertTrue(document["dirty"])
            self.assertTrue(document["canUndo"])
            copy_step = next(step for step in document["recipe"]["steps"] if step["id"] == "copy")
            self.assertEqual(copy_step["params"]["source"], {"ref": "steps.extract.outputs.extracted_paths"})
            self.assertEqual(copy_step["params"]["dest"], "/sdcard/New")
            self.assertEqual(copy_step["params"]["copy_policy"], "sync")
            self.assertEqual(copy_step["dependencies"], [])
            self.assertIn("ref: steps.extract.outputs.extracted_paths", document["yaml"])
            self.assertIn("dest: /sdcard/New", document["yaml"])


if __name__ == "__main__":
    unittest.main()

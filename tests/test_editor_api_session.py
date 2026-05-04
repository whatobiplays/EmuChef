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

            response = manager.apply_recipe_command(document_id, {"type": "DeleteInput", "inputId": "missing"})

            self.assertFalse(response["ok"])
            self.assertEqual(response["error"]["code"], "invalid_command")


if __name__ == "__main__":
    unittest.main()

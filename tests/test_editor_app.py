from __future__ import annotations

import importlib.util
import os
import sys
import types
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import Mock, patch

from support import base_recipe, build_authored_tree

PYSIDE6_AVAILABLE = importlib.util.find_spec("PySide6") is not None

if PYSIDE6_AVAILABLE:
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QApplication, QComboBox, QFormLayout, QLineEdit, QSizePolicy, QWidget

    from emuchef_editor.app.app import main as editor_main
    from emuchef_editor.app.main_window import MainWindow
    from emuchef_editor.app.recipe_editor.common import TextEntryDialog, add_tooltipped_form_row
    from emuchef_editor.app.recipe_editor.tooltips import field_tooltip, prompt_tooltip


@unittest.skipUnless(PYSIDE6_AVAILABLE, "PySide6 is not installed in the local test environment.")
class EditorAppTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._app = QApplication.instance() or QApplication(["emuchef-editor-test"])

    def test_main_window_lists_recipe_files_and_opens_recipe(self) -> None:
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
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            self.assertIsNotNone(window.workspace_state)
            self.assertEqual(window.workspace_state.authored_root, authored_root.resolve())
            self.assertEqual(window._workspace_list.count(), 1)

            window.open_recipe_file(recipe_path)

            self.assertIsNotNone(window.current_document)
            self.assertEqual(window._editor_view._tabs.count(), 4)
            self.assertIn("schema_version: 1", window._yaml_preview.toPlainText())
            self.assertIn("example.recipe", window.windowTitle())
            self.assertEqual(window._diagnostics_view._tree.topLevelItemCount(), 0)

            window.close()

    def test_widget_edits_refresh_preview_dirty_state_and_undo_redo(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={
                "base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"},
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            overview_page = window._editor_view._overview_page
            overview_page._name_edit.setText("Updated Recipe")
            overview_page._name_edit.editingFinished.emit()

            self.assertEqual(window.current_document.working_recipe.name, "Updated Recipe")
            self.assertIn("name: Updated Recipe", window._yaml_preview.toPlainText())
            self.assertIn("*", window.windowTitle())
            self.assertTrue(window._undo_action.isEnabled())

            inputs_page = window._editor_view._inputs_page
            with patch.object(inputs_page, "_prompt_for_identifier", return_value="extra_input"):
                inputs_page._add_button.click()
            self.assertIn("extra_input", window.current_document.working_recipe.inputs)
            self.assertIn("extra_input:", window._yaml_preview.toPlainText())

            artifacts_page = window._editor_view._artifacts_page
            with patch.object(
                artifacts_page,
                "_prompt_for_new_artifact",
                return_value=("extra_zip", "https://example.com/extra.zip"),
            ):
                artifacts_page._add_button.click()
            self.assertIn("extra_zip", window.current_document.working_recipe.artifacts)
            self.assertIn("extra_zip:", window._yaml_preview.toPlainText())

            groups_page = window._editor_view._artifact_groups_page
            with patch.object(groups_page, "_prompt_for_identifier", return_value="bundle"):
                groups_page._add_group_button.click()
            with patch("emuchef_editor.app.recipe_editor.artifact_groups_page.QInputDialog.getItem", return_value=("extra_zip", True)):
                groups_page._add_member_button.click()
            self.assertEqual(window.current_document.working_recipe.artifact_groups["bundle"], ("extra_zip",))
            self.assertIn("bundle:", window._yaml_preview.toPlainText())

            window.save_current_document()
            self.assertNotIn("*", window.windowTitle())
            self.assertTrue(window._undo_action.isEnabled())
            self.assertFalse(window.current_document.is_dirty)

            window.undo_current_document()
            self.assertIn("*", window.windowTitle())
            self.assertTrue(window.current_document.is_dirty)
            self.assertTrue(window._redo_action.isEnabled())

            window.redo_current_document()
            self.assertNotIn("*", window.windowTitle())
            self.assertFalse(window.current_document.is_dirty)

            window.close()

    def test_opening_different_recipe_resets_document_history(self) -> None:
        first = base_recipe(recipe_id="first.recipe", steps=[])
        second = base_recipe(recipe_id="second.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[first, second])
            first_path = authored_root / "recipes" / "first_recipe.yaml"
            second_path = authored_root / "recipes" / "second_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(first_path)
            window._editor_view._overview_page._name_edit.setText("Renamed First")
            window._editor_view._overview_page._name_edit.editingFinished.emit()
            self.assertTrue(window._undo_action.isEnabled())

            window.open_recipe_file(second_path)
            self.assertEqual(window.current_document.working_recipe.id, "second.recipe")
            self.assertFalse(window._undo_action.isEnabled())
            self.assertFalse(window._redo_action.isEnabled())
            self.assertNotIn("*", window.windowTitle())

            window.close()

    def test_editor_forms_grow_fields_and_keep_label_alignment(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"}},
            artifact_groups={"bundle": ["base_zip"]},
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            for page in (
                window._editor_view._overview_page,
                window._editor_view._inputs_page,
                window._editor_view._artifacts_page,
                window._editor_view._artifact_groups_page,
            ):
                form = page._form
                self.assertEqual(form.fieldGrowthPolicy(), QFormLayout.FieldGrowthPolicy.AllNonFixedFieldsGrow)
                self.assertTrue(form.formAlignment() & Qt.AlignmentFlag.AlignLeft)
                self.assertTrue(form.formAlignment() & Qt.AlignmentFlag.AlignTop)
                self.assertTrue(form.labelAlignment() & Qt.AlignmentFlag.AlignRight)
                self.assertTrue(form.labelAlignment() & Qt.AlignmentFlag.AlignVCenter)

            for widget in (
                window._editor_view._overview_page._id_edit,
                window._editor_view._overview_page._name_edit,
                window._editor_view._inputs_page._id_value,
                window._editor_view._inputs_page._type_combo,
                window._editor_view._inputs_page._role_combo,
                window._editor_view._inputs_page._label_edit,
                window._editor_view._inputs_page._allowed_extensions_edit,
                window._editor_view._inputs_page._path_kind_combo,
                window._editor_view._artifacts_page._id_value,
                window._editor_view._artifacts_page._url_edit,
                window._editor_view._artifacts_page._cache_combo,
                window._editor_view._artifact_groups_page._group_id_value,
            ):
                self.assertIsInstance(widget, (QLineEdit, QComboBox))
                self.assertEqual(widget.sizePolicy().horizontalPolicy(), QSizePolicy.Policy.Expanding)

            window.close()

    def test_editor_fields_and_form_labels_expose_expected_tooltips(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            artifacts={"base_zip": {"type": "remote_file", "url": "https://example.com/base.zip"}},
            artifact_groups={"bundle": ["base_zip"]},
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            overview_page = window._editor_view._overview_page
            inputs_page = window._editor_view._inputs_page
            artifacts_page = window._editor_view._artifacts_page
            groups_page = window._editor_view._artifact_groups_page

            self.assertEqual(overview_page._id_edit.toolTip(), field_tooltip("overview.id"))
            self.assertEqual(
                overview_page._form.labelForField(overview_page._id_edit).toolTip(),
                field_tooltip("overview.id"),
            )
            self.assertEqual(overview_page._description_edit.toolTip(), field_tooltip("overview.description"))
            self.assertEqual(
                overview_page._recipe_dependencies._value_edit.toolTip(),
                field_tooltip("overview.recipe_dependencies"),
            )

            self.assertEqual(inputs_page._type_combo.toolTip(), field_tooltip("inputs.type"))
            self.assertEqual(
                inputs_page._form.labelForField(inputs_page._type_combo).toolTip(),
                field_tooltip("inputs.type"),
            )
            self.assertEqual(inputs_page._metadata_value.toolTip(), field_tooltip("inputs.metadata"))

            self.assertEqual(artifacts_page._kind_value.toolTip(), field_tooltip("artifacts.kind"))
            self.assertEqual(
                artifacts_page._form.labelForField(artifacts_page._kind_value).toolTip(),
                field_tooltip("artifacts.kind"),
            )
            self.assertEqual(artifacts_page._url_edit.toolTip(), field_tooltip("artifacts.url"))
            self.assertEqual(
                artifacts_page._form.labelForField(artifacts_page._url_edit).toolTip(),
                field_tooltip("artifacts.url"),
            )

            self.assertEqual(groups_page._group_id_value.toolTip(), field_tooltip("artifact_groups.id"))
            self.assertEqual(
                groups_page._form.labelForField(groups_page._group_id_value).toolTip(),
                field_tooltip("artifact_groups.id"),
            )

            window.close()

    def test_add_tooltipped_form_row_skips_missing_and_blank_tooltips(self) -> None:
        host = QWidget()
        form = QFormLayout(host)

        missing_field = QLineEdit()
        add_tooltipped_form_row(form, "Missing", missing_field, field_tooltip("missing.field"))
        self.assertEqual(missing_field.toolTip(), "")
        self.assertEqual(form.labelForField(missing_field).toolTip(), "")

        blank_field = QLineEdit()
        add_tooltipped_form_row(form, "Blank", blank_field, "   ")
        self.assertEqual(blank_field.toolTip(), "")
        self.assertEqual(form.labelForField(blank_field).toolTip(), "")

    def test_text_entry_dialog_applies_prompt_tooltip_and_returns_value(self) -> None:
        dialog = TextEntryDialog(
            title="Add Input",
            label="Input id",
            tooltip=prompt_tooltip("inputs.id"),
        )

        self.assertEqual(dialog.value_edit.toolTip(), prompt_tooltip("inputs.id"))
        self.assertEqual(
            dialog._form.labelForField(dialog.value_edit).toolTip(),
            prompt_tooltip("inputs.id"),
        )

        dialog.value_edit.setText("retroarch_cfg")
        dialog.accept()

        self.assertEqual(dialog.value(), "retroarch_cfg")

    def test_app_main_bootstraps_without_entering_real_event_loop(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            build_authored_tree(repo_root, recipes=[recipe])
            fake_app = Mock()
            fake_app.exec.return_value = 0
            fake_window = Mock()
            fake_window.show.return_value = None
            fake_module = types.SimpleNamespace(MainWindow=Mock(return_value=fake_window))

            with patch("PySide6.QtWidgets.QApplication.instance", return_value=fake_app), patch.dict(
                sys.modules,
                {"emuchef_editor.app.main_window": fake_module},
            ):
                rc = editor_main([str(repo_root)])

            self.assertEqual(rc, 0)
            fake_app.exec.assert_called_once()
            fake_module.MainWindow.assert_called_once_with(workspace_root=str(repo_root))
            fake_window.show.assert_called_once()


if __name__ == "__main__":
    unittest.main()

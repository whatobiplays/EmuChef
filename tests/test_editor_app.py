from __future__ import annotations

import importlib.util
import os
import sys
import types
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import Mock, patch

from emuchef.domain import RuntimePermissionGrant
from support import base_recipe, build_authored_tree

PYSIDE6_AVAILABLE = importlib.util.find_spec("PySide6") is not None

if PYSIDE6_AVAILABLE:
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QCloseEvent
    from PySide6.QtWidgets import (
        QApplication,
        QComboBox,
        QFormLayout,
        QLineEdit,
        QMessageBox,
        QSizePolicy,
        QWidget,
    )

    from emuchef_editor.app.app import main as editor_main
    from emuchef_editor.app.main_window import MainWindow
    from emuchef_editor.app.recipe_editor.common import TextEntryDialog, add_tooltipped_form_row
    from emuchef_editor.app.recipe_editor.new_recipe_dialog import NewRecipeDialog, NewRecipeRequest
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
            self.assertEqual(window._workspace_list.count(), 2)

            window.open_recipe_file(recipe_path)

            self.assertIsNotNone(window.current_document)
            self.assertEqual(window._editor_view._tabs.count(), 5)
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

    def test_permissions_tab_edits_refresh_preview_and_dirty_state(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            permissions={
                "runtime": [{"package_name": "com.example.app", "name": "READ_MEDIA_VIDEO"}],
                "policy": {"on_failure": "warn", "require_all": False},
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            self.assertEqual(window._editor_view._tabs.tabText(4), "Permissions")
            page = window._editor_view._permissions_page
            with patch.object(
                page,
                "_prompt_for_new_runtime_permission",
                return_value=RuntimePermissionGrant(
                    package_name="com.example.app",
                    name="POST_NOTIFICATIONS",
                ),
            ):
                page._add_runtime_button.click()

            page._policy_on_failure_combo.setCurrentText("fail")
            page._policy_require_all_check.setChecked(True)

            self.assertEqual(len(window.current_document.working_recipe.permissions.runtime), 2)
            self.assertEqual(window.current_document.working_recipe.permissions.policy.on_failure, "fail")
            self.assertTrue(window.current_document.working_recipe.permissions.policy.require_all)
            self.assertTrue(window.current_document.is_dirty)
            self.assertIn("POST_NOTIFICATIONS", window._yaml_preview.toPlainText())
            self.assertIn("on_failure: fail", window._yaml_preview.toPlainText())
            self.assertIn("*", window.windowTitle())

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_workspace_distinguishes_templates_and_template_activation_starts_new_recipe_flow(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        blank_template = base_recipe(recipe_id="blank.recipe", name="Blank Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={
                    "recipe.template.yaml": template,
                    "recipe.blank.template.yaml": blank_template,
                },
            )

            window = MainWindow(repo_root)

            item_kinds = [
                window._workspace_list.item(index).data(Qt.ItemDataRole.UserRole)["kind"]
                for index in range(window._workspace_list.count())
            ]
            self.assertEqual(item_kinds, ["header", "recipe", "header", "template", "template"])

            template_item = window._workspace_list.item(3)
            with patch.object(window, "_start_new_recipe_flow") as start_flow:
                window._open_item(template_item)

            start_flow.assert_called_once()
            self.assertEqual(
                start_flow.call_args.kwargs["preselected_template"],
                Path(template_item.data(Qt.ItemDataRole.UserRole)["path"]),
            )

            window.close()

    def test_new_recipe_dialog_preview_is_read_only_and_filename_tracks_recipe_id_until_edited(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        blank_template = base_recipe(recipe_id="blank.recipe", name="Blank Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={
                    "recipe.template.yaml": template,
                    "recipe.blank.template.yaml": blank_template,
                },
            )
            dialog = NewRecipeDialog(
                template_paths=(
                    repo_root / "templates" / "authored" / "recipe.blank.template.yaml",
                    repo_root / "templates" / "authored" / "recipe.template.yaml",
                ),
                authored_root=authored_root,
                preselected_template=repo_root / "templates" / "authored" / "recipe.blank.template.yaml",
            )

            self.assertTrue(dialog._template_preview.isReadOnly())
            self.assertIn("kind: recipe", dialog._template_preview.toPlainText())

            dialog._recipe_id_edit.setText("created.recipe")
            dialog._recipe_id_edit.editingFinished.emit()
            self.assertEqual(dialog._filename_edit.text(), "created.recipe.yaml")

            dialog._filename_edit.setText("custom.yaml")
            dialog._filename_edit.editingFinished.emit()
            dialog._recipe_id_edit.setText("changed.recipe")
            dialog._recipe_id_edit.editingFinished.emit()
            self.assertEqual(dialog._filename_edit.text(), "custom.yaml")

            dialog.close()

    def test_save_as_declined_overwrite_leaves_document_path_history_and_baseline_unchanged(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            existing_path = authored_root / "recipes" / "existing.yaml"
            existing_path.write_text("already here\n", encoding="utf-8")

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window._editor_view._overview_page._name_edit.setText("Updated Recipe")
            window._editor_view._overview_page._name_edit.editingFinished.emit()
            original_path = window.current_document.path
            original_yaml = window.current_document.to_yaml()

            with patch(
                "emuchef_editor.app.main_window.QFileDialog.getSaveFileName",
                return_value=(str(existing_path), "YAML Files (*.yaml)"),
            ), patch.object(window, "_confirm_overwrite", return_value=False):
                window.save_current_document_as()

            self.assertEqual(window.current_document.path, original_path)
            self.assertEqual(window.current_document.to_yaml(), original_yaml)
            self.assertTrue(window.current_document.can_undo)
            self.assertTrue(window.current_document.is_dirty)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_unsaved_changes_cancel_blocks_opening_other_recipe(self) -> None:
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

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Cancel,
            ):
                window.open_recipe_file(second_path)

            self.assertEqual(window.current_document.working_recipe.id, "first.recipe")
            self.assertTrue(window.current_document.is_dirty)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_start_new_recipe_flow_writes_destination_and_opens_clean_document(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={"recipe.template.yaml": template},
            )
            destination_path = authored_root / "recipes" / "created_recipe.yaml"
            template_path = repo_root / "templates" / "authored" / "recipe.template.yaml"

            window = MainWindow(repo_root)
            with patch("emuchef_editor.app.main_window.NewRecipeDialog") as dialog_cls:
                dialog = dialog_cls.return_value
                dialog.exec.return_value = NewRecipeDialog.DialogCode.Accepted
                dialog.request.return_value = NewRecipeRequest(
                    template_path=template_path,
                    destination_path=destination_path,
                    recipe_id="created.recipe",
                )
                window._start_new_recipe_flow()

            self.assertEqual(window.current_document.path, destination_path.resolve())
            self.assertEqual(window.current_document.working_recipe.id, "created.recipe")
            self.assertFalse(window.current_document.is_dirty)
            self.assertTrue(destination_path.exists())
            self.assertIn("id: created.recipe", destination_path.read_text(encoding="utf-8"))

            window.close()

    def test_failed_new_recipe_creation_leaves_current_document_unchanged(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[recipe],
                recipe_templates={"recipe.template.yaml": template},
            )
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            template_path = repo_root / "templates" / "authored" / "recipe.template.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window._editor_view._overview_page._name_edit.setText("Dirty Name")
            window._editor_view._overview_page._name_edit.editingFinished.emit()

            with patch("emuchef_editor.app.main_window.NewRecipeDialog") as dialog_cls, patch(
                "emuchef_editor.app.main_window.create_recipe_document_from_template",
                side_effect=OSError("write failed"),
            ):
                dialog = dialog_cls.return_value
                dialog.exec.return_value = NewRecipeDialog.DialogCode.Accepted
                dialog.request.return_value = NewRecipeRequest(
                    template_path=template_path,
                    destination_path=authored_root / "recipes" / "created_recipe.yaml",
                    recipe_id="created.recipe",
                )
                with patch.object(
                    window,
                    "_prompt_unsaved_changes",
                    return_value=QMessageBox.StandardButton.Discard,
                ):
                    window._start_new_recipe_flow()

            self.assertEqual(window.current_document.path, recipe_path.resolve())
            self.assertEqual(window.current_document.working_recipe.id, "example.recipe")
            self.assertTrue(window.current_document.is_dirty)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_save_as_success_switches_document_path_and_preserves_undo_history(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"
            save_as_path = authored_root / "recipes" / "saved_as_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window._editor_view._overview_page._name_edit.setText("Updated Recipe")
            window._editor_view._overview_page._name_edit.editingFinished.emit()
            self.assertTrue(window.current_document.can_undo)

            with patch(
                "emuchef_editor.app.main_window.QFileDialog.getSaveFileName",
                return_value=(str(save_as_path), "YAML Files (*.yaml)"),
            ):
                window.save_current_document_as()

            self.assertEqual(window.current_document.path, save_as_path.resolve())
            self.assertFalse(window.current_document.is_dirty)
            self.assertTrue(window.current_document.can_undo)
            self.assertTrue(save_as_path.exists())

            window.close()

    def test_start_new_recipe_flow_cancel_prompt_leaves_current_document_and_skips_dialog(self) -> None:
        first = base_recipe(recipe_id="first.recipe", steps=[])
        template = base_recipe(recipe_id="template.recipe", name="Template Recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(
                repo_root,
                recipes=[first],
                recipe_templates={"recipe.template.yaml": template},
            )
            first_path = authored_root / "recipes" / "first_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(first_path)
            window._editor_view._overview_page._name_edit.setText("Dirty First")
            window._editor_view._overview_page._name_edit.editingFinished.emit()

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Cancel,
            ), patch("emuchef_editor.app.main_window.NewRecipeDialog") as dialog_cls:
                window._start_new_recipe_flow()

            dialog_cls.assert_not_called()
            self.assertEqual(window.current_document.working_recipe.id, "first.recipe")
            self.assertTrue(window.current_document.is_dirty)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_close_event_respects_cancel_and_discard_unsaved_choices(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window._editor_view._overview_page._name_edit.setText("Dirty Name")
            window._editor_view._overview_page._name_edit.editingFinished.emit()

            cancel_event = QCloseEvent()
            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Cancel,
            ):
                window.closeEvent(cancel_event)
            self.assertFalse(cancel_event.isAccepted())

            discard_event = QCloseEvent()
            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.closeEvent(discard_event)
            self.assertTrue(discard_event.isAccepted())

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_opening_unsupported_permission_shape_surfaces_load_failure(self) -> None:
        recipe = base_recipe(recipe_id="example.recipe", steps=[])
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "bad_permissions.yaml"
            recipe_path.write_text(
                "\n".join(
                    [
                        "schema_version: 1",
                        "kind: recipe",
                        "id: bad.permissions",
                        "name: Bad Permissions",
                        "description: Invalid permission shape.",
                        "recipe_dependencies: []",
                        "provides:",
                        "  features: []",
                        "inputs: {}",
                        "artifacts: {}",
                        "artifact_groups: {}",
                        "permissions:",
                        "  manual:",
                        "    - unsupported",
                        "steps: []",
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            self.assertIsNone(window.current_document)
            self.assertEqual(window._yaml_preview.toPlainText(), "")
            self.assertGreater(window._diagnostics_view._tree.topLevelItemCount(), 0)
            self.assertIn("Recipe could not be loaded", window._editor_view._placeholder._title.text())

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

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
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

from __future__ import annotations

import importlib.util
import os
import sys
import types
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import Mock, patch

from emuchef.domain import RuntimePermissionGrant, StepCondition, StepType
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
            self.assertEqual(window._editor_view._tabs.count(), 6)
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

            self.assertEqual(window._editor_view._tabs.tabText(4), "Steps")
            self.assertEqual(window._editor_view._tabs.tabText(5), "Permissions")
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

    def test_steps_tab_supports_representative_widget_flows(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            steps=[],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)

            page = window._editor_view._steps_page
            self.assertEqual(window._editor_view._tabs.tabText(4), "Steps")
            window._editor_view._tabs.setCurrentIndex(4)

            with patch.object(
                page,
                "_prompt_for_new_step",
                return_value=(StepType.COPY_FILES, "copy_assets", "Copy Assets"),
            ):
                page._add_step_button.click()

            self.assertEqual(window.current_document.working_recipe.steps[0].id, "copy_assets")
            self.assertTrue(page._step_id_value.isReadOnly())
            self.assertTrue(page._step_type_value.isReadOnly())
            self.assertIsInstance(page._copy_source_combo, QComboBox)

            page._copy_source_combo.setCurrentIndex(page._copy_source_combo.findData("inputs.source_dir"))
            page._copy_dest_edit.setText("/sdcard/Example")
            page._copy_dest_edit.editingFinished.emit()
            page._copy_policy_combo.setCurrentIndex(page._copy_policy_combo.findData("sync"))
            page._step_user_toggleable_check.setChecked(True)

            with patch.object(
                page,
                "_prompt_for_new_step",
                return_value=(StepType.WAIT, "pause", "Pause"),
            ):
                page._add_step_button.click()

            page._select_step("pause")
            page._wait_duration_spin.setValue(1500)

            page._select_step("copy_assets")
            with patch("emuchef_editor.app.recipe_editor.steps_page.TextEntryDialog.prompt", return_value="copy_assets_dup"):
                page._duplicate_step_button.click()

            page._select_step("copy_assets_dup")
            page._move_up_button.click()

            page._select_step("copy_assets")
            with patch.object(page, "_confirm_delete_step", return_value=True):
                page._delete_step_button.click()

            self.assertEqual(
                tuple(step.id for step in window.current_document.working_recipe.steps),
                ("copy_assets_dup", "pause"),
            )
            self.assertTrue(window.current_document.working_recipe.steps[0].user_toggleable)
            self.assertIn("ref: inputs.source_dir", window._yaml_preview.toPlainText())
            self.assertIn("duration_ms: 1500", window._yaml_preview.toPlainText())
            self.assertTrue(window._undo_action.isEnabled())
            self.assertTrue(window.current_document.is_dirty)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_steps_page_shows_notes_and_locks_preserved_unsupported_lists(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {
                        "capabilities": ["shared_storage_write", "unsupported_capability"],
                        "conflicts_with": [],
                    },
                    "skip_if": [
                        {"type": "path_exists", "params": {"path": "/sdcard/Example"}},
                        {"type": "custom_skip", "params": {"foo": "bar"}},
                    ],
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            page = window._editor_view._steps_page
            window._editor_view._tabs.setCurrentIndex(4)

            page._select_step("grant")
            self.assertIs(page._params_stack.currentWidget(), page._grant_permissions_panel)
            self.assertIn("permission plan", page._grant_permissions_note_label.text())

            page._select_step("copy")
            self.assertIs(page._params_stack.currentWidget(), page._copy_files_panel)
            self.assertIn("destination directory", page._copy_help_label.text())
            self.assertFalse(page._constraints_preserved_view.isHidden())
            self.assertIn("unsupported_capability", page._constraints_preserved_view.toPlainText())
            self.assertFalse(page._capabilities_editor._remove_button.isEnabled())
            self.assertFalse(page._skip_if_editor._preserved_view.isHidden())
            self.assertIn("custom_skip", page._skip_if_editor._preserved_view.toPlainText())
            self.assertFalse(page._skip_if_editor._remove_button.isEnabled())

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_steps_page_surfaces_unresolved_ref_choices_without_clearing_them(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            steps=[
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "source": {"ref": "steps.missing.outputs.copied_paths"},
                        "dest": "/sdcard/Example",
                    },
                    "verify": [],
                }
            ],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            page = window._editor_view._steps_page
            window._editor_view._tabs.setCurrentIndex(4)
            page._select_step("copy")

            self.assertEqual(page._copy_source_combo.currentData(), "steps.missing.outputs.copied_paths")
            self.assertIn("[Unresolved]", page._copy_source_combo.currentText())
            page._step_name_edit.setText("Copy Updated")
            page._step_name_edit.editingFinished.emit()
            self.assertIn("ref: steps.missing.outputs.copied_paths", window._yaml_preview.toPlainText())

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_steps_page_dependency_card_and_auto_sizing_lists(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                }
            },
            steps=[
                {
                    "id": "prepare",
                    "type": "wait",
                    "name": "Prepare",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"duration_ms": 250},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": ["prepare"],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window.show()
            self._app.processEvents()

            page = window._editor_view._steps_page
            window._editor_view._tabs.setCurrentIndex(4)
            page._select_step("copy")
            self._app.processEvents()

            dependency_item = page._dependencies_list.item(0)
            self.assertIsNotNone(dependency_item)
            self.assertFalse(bool(dependency_item.flags() & Qt.ItemFlag.ItemIsUserCheckable))

            captured_labels = page._available_dependency_ids()

            with patch.object(page, "_prompt_for_dependency", return_value="grant"):
                page._add_dependency_button.click()
            self._app.processEvents()

            self.assertEqual(captured_labels, ("grant",))
            self.assertEqual(window.current_document.working_recipe.steps[1].dependencies, ("prepare", "grant"))
            self.assertEqual(tuple(page._dependencies_list.item(row).text() for row in range(page._dependencies_list.count())), ("prepare", "grant"))
            self.assertIn("- prepare", window._yaml_preview.toPlainText())
            self.assertIn("- grant", window._yaml_preview.toPlainText())

            page._dependencies_list.setCurrentRow(1)
            page._remove_dependency_button.click()
            self._app.processEvents()

            self.assertEqual(window.current_document.working_recipe.steps[1].dependencies, ("prepare",))
            self.assertEqual(tuple(page._dependencies_list.item(row).text() for row in range(page._dependencies_list.count())), ("prepare",))

            empty_capabilities_height = page._capabilities_editor._list.height()
            empty_conflicts_height = page._conflicts_editor._list.height()
            empty_skip_if_height = page._skip_if_editor._list.height()
            empty_verify_height = page._verify_editor._list.height()
            self.assertGreater(empty_capabilities_height, 0)
            self.assertGreater(empty_conflicts_height, 0)
            self.assertGreater(empty_skip_if_height, 0)
            self.assertGreater(empty_verify_height, 0)

            capability_labels = [label for _value, label in page._capabilities_editor._choices[:2]]
            self.assertGreaterEqual(len(capability_labels), 2)
            with patch(
                "emuchef_editor.app.recipe_editor.steps_page.QInputDialog.getItem",
                side_effect=[(capability_labels[0], True), (capability_labels[1], True)],
            ):
                page._capabilities_editor._add_button.click()
                page._capabilities_editor._add_button.click()
            self._app.processEvents()
            self.assertEqual(len(page._capabilities_editor.values()), 2)
            self.assertIn(page._capabilities_editor.values()[0], window._yaml_preview.toPlainText())
            self.assertGreater(page._capabilities_editor._list.height(), empty_capabilities_height)

            conflict_labels = [label for _value, label in page._conflicts_editor._choices[:2]]
            self.assertEqual(conflict_labels, ["prepare", "grant"])
            with patch(
                "emuchef_editor.app.recipe_editor.steps_page.QInputDialog.getItem",
                side_effect=[(conflict_labels[1], True), (conflict_labels[0], True)],
            ):
                page._conflicts_editor._add_button.click()
                page._conflicts_editor._add_button.click()
            self._app.processEvents()
            self.assertEqual(page._conflicts_editor.values(), ("grant", "prepare"))
            self.assertIn("conflicts_with:", window._yaml_preview.toPlainText())
            self.assertIn("- grant", window._yaml_preview.toPlainText())
            self.assertGreater(page._conflicts_editor._list.height(), empty_conflicts_height)

            condition = StepCondition(type="path_exists", params={"path": "/sdcard/Example"})
            second_condition = StepCondition(type="package_installed", params={"package_name": "com.example.app"})
            with patch(
                "emuchef_editor.app.recipe_editor.steps_page._ConditionDialog.prompt",
                side_effect=[condition, second_condition],
            ):
                page._skip_if_editor._add_button.click()
                page._skip_if_editor._add_button.click()
            self._app.processEvents()
            self.assertIn("skip_if:", window._yaml_preview.toPlainText())
            self.assertGreater(page._skip_if_editor._list.height(), empty_skip_if_height)

            verify_condition = StepCondition(type="file_exists", params={"path": "/sdcard/Example/config.ini"})
            second_verify_condition = StepCondition(type="path_exists", params={"path": "/sdcard/Example"})
            with patch(
                "emuchef_editor.app.recipe_editor.steps_page._ConditionDialog.prompt",
                side_effect=[verify_condition, second_verify_condition],
            ):
                page._verify_editor._add_button.click()
                page._verify_editor._add_button.click()
            self._app.processEvents()
            self.assertIn("verify:", window._yaml_preview.toPlainText())
            self.assertGreater(page._verify_editor._list.height(), empty_verify_height)

            with patch.object(
                window,
                "_prompt_unsaved_changes",
                return_value=QMessageBox.StandardButton.Discard,
            ):
                window.close()

    def test_steps_page_params_host_resizes_for_step_transitions_and_extract_archive_mode(self) -> None:
        recipe = base_recipe(
            recipe_id="example.recipe",
            inputs={
                "archive_file": {
                    "type": "file",
                    "role": "generic",
                    "label": "Archive File",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "file"},
                },
                "source_dir": {
                    "type": "directory",
                    "role": "generic",
                    "label": "Source Directory",
                    "required": True,
                    "multiple": False,
                    "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                },
            },
            steps=[
                {
                    "id": "grant",
                    "type": "grant_permissions",
                    "name": "Grant",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "verify": [],
                },
                {
                    "id": "copy",
                    "type": "copy_files",
                    "name": "Copy",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Example"},
                    "verify": [],
                },
                {
                    "id": "archive_plain",
                    "type": "extract_archive",
                    "name": "Extract Plain",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {"archive": {"ref": "inputs.archive_file"}, "extract_on": "host"},
                    "verify": [],
                },
                {
                    "id": "archive_preserved",
                    "type": "extract_archive",
                    "name": "Extract Preserved",
                    "user_toggleable": False,
                    "dependencies": [],
                    "constraints": {"capabilities": [], "conflicts_with": []},
                    "params": {
                        "archive": {"ref": "inputs.archive_file"},
                        "extract_on": "host",
                        "custom_behavior": "keep_me",
                    },
                    "verify": [],
                },
            ],
        )
        with TemporaryDirectory() as tmp:
            repo_root = Path(tmp)
            authored_root = build_authored_tree(repo_root, recipes=[recipe])
            recipe_path = authored_root / "recipes" / "example_recipe.yaml"

            window = MainWindow(repo_root)
            window.open_recipe_file(recipe_path)
            window.show()
            self._app.processEvents()

            page = window._editor_view._steps_page
            window._editor_view._tabs.setCurrentIndex(4)

            def params_height() -> int:
                page._detail_scroll.widget().adjustSize()
                self._app.processEvents()
                return page._params_section.height()

            page._select_step("archive_plain")
            self._app.processEvents()
            archive_host_height = params_height()

            page._extract_archive_extract_on_combo.setCurrentIndex(page._extract_archive_extract_on_combo.findData("device"))
            self._app.processEvents()
            archive_device_height = params_height()
            self.assertGreater(archive_device_height, archive_host_height)

            page._extract_archive_extract_on_combo.setCurrentIndex(page._extract_archive_extract_on_combo.findData("host"))
            self._app.processEvents()
            self.assertEqual(params_height(), archive_host_height)

            page._select_step("archive_preserved")
            self._app.processEvents()
            preserved_height = params_height()
            self.assertGreater(preserved_height, archive_host_height)
            self.assertFalse(page._params_preserved_view.isHidden())

            page._select_step("archive_plain")
            self._app.processEvents()
            self.assertEqual(params_height(), archive_host_height)
            self.assertTrue(page._params_preserved_view.isHidden())

            page._select_step("grant")
            self._app.processEvents()
            grant_height = params_height()

            page._select_step("copy")
            self._app.processEvents()
            copy_height = params_height()
            self.assertGreater(copy_height, grant_height)

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

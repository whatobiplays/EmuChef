"""Main window for the authored recipe editor."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import QFileSystemWatcher, Qt
from PySide6.QtGui import QAction, QCloseEvent, QKeySequence
from PySide6.QtWidgets import (
    QDialog,
    QFileDialog,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMessageBox,
    QSplitter,
    QTabWidget,
)

from emuchef.planner.catalog import CatalogLoadError
from emuchef_editor.core.documents.commands import RecipeCommand
from emuchef_editor.core.documents.recipe_document import RecipeDocument
from emuchef_editor.core.validation.validator_service import DiagnosticResult, ValidatorService
from emuchef_editor.core.yaml.loader import create_recipe_document_from_template, load_recipe_document

from .recipe_editor import NewRecipeDialog, RecipeEditor
from .shared.diagnostics_view import DiagnosticsView
from .shared.yaml_preview import YamlPreview
from .workspace.service import WorkspaceState, open_workspace


class MainWindow(QMainWindow):
    def __init__(self, workspace_root: str | Path | None = None) -> None:
        super().__init__()
        self._validator_service = ValidatorService()
        self._workspace_state: WorkspaceState | None = None
        self._current_document: RecipeDocument | None = None
        self._open_document_missing_from_workspace = False
        self._workspace_watcher = QFileSystemWatcher(self)
        self._workspace_watcher.directoryChanged.connect(self._on_workspace_directory_changed)

        self._workspace_list = QListWidget()
        self._workspace_list.itemActivated.connect(self._open_item)

        self._editor_view = RecipeEditor(self._apply_document_command)
        self._diagnostics_view = DiagnosticsView()
        self._yaml_preview = YamlPreview()

        right_tabs = QTabWidget()
        right_tabs.addTab(self._diagnostics_view, "Diagnostics")
        right_tabs.addTab(self._yaml_preview, "YAML Preview")

        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.addWidget(self._workspace_list)
        splitter.addWidget(self._editor_view)
        splitter.addWidget(right_tabs)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 2)
        splitter.setStretchFactor(2, 2)
        self.setCentralWidget(splitter)

        self._open_workspace_action = QAction("Open Workspace...", self)
        self._open_workspace_action.triggered.connect(self.open_workspace_dialog)
        self._new_recipe_action = QAction("New Recipe...", self)
        self._new_recipe_action.setShortcut(QKeySequence.StandardKey.New)
        self._new_recipe_action.triggered.connect(self.start_new_recipe_flow)
        self._save_action = QAction("Save", self)
        self._save_action.setShortcut(QKeySequence.StandardKey.Save)
        self._save_action.triggered.connect(self.save_current_document)
        self._save_action.setEnabled(False)
        self._save_as_action = QAction("Save As...", self)
        self._save_as_action.setShortcut(QKeySequence.StandardKey.SaveAs)
        self._save_as_action.triggered.connect(self.save_current_document_as)
        self._save_as_action.setEnabled(False)
        self._undo_action = QAction("Undo", self)
        self._undo_action.setShortcut(QKeySequence.StandardKey.Undo)
        self._undo_action.triggered.connect(self.undo_current_document)
        self._undo_action.setEnabled(False)
        self._redo_action = QAction("Redo", self)
        self._redo_action.setShortcut(QKeySequence.StandardKey.Redo)
        self._redo_action.triggered.connect(self.redo_current_document)
        self._redo_action.setEnabled(False)

        file_menu = self.menuBar().addMenu("File")
        file_menu.addAction(self._open_workspace_action)
        file_menu.addAction(self._new_recipe_action)
        file_menu.addAction(self._save_action)
        file_menu.addAction(self._save_as_action)
        edit_menu = self.menuBar().addMenu("Edit")
        edit_menu.addAction(self._undo_action)
        edit_menu.addAction(self._redo_action)

        toolbar = self.addToolBar("Main")
        toolbar.addAction(self._open_workspace_action)
        toolbar.addAction(self._new_recipe_action)
        toolbar.addAction(self._save_action)
        toolbar.addAction(self._save_as_action)
        toolbar.addAction(self._undo_action)
        toolbar.addAction(self._redo_action)

        self.statusBar().showMessage("Open a repo root or authored root to begin.")
        self.resize(1400, 800)
        self._refresh_window_title()

        if workspace_root is not None:
            self.open_workspace(workspace_root, guard_unsaved=False)

    @property
    def current_document(self) -> RecipeDocument | None:
        return self._current_document

    @property
    def workspace_state(self) -> WorkspaceState | None:
        return self._workspace_state

    def open_workspace_dialog(self) -> None:
        selected = QFileDialog.getExistingDirectory(self, "Open Workspace Root")
        if selected:
            self.open_workspace(selected)

    def open_workspace(self, root: str | Path, *, guard_unsaved: bool = True) -> None:
        if guard_unsaved and not self._confirm_unsaved_transition():
            return
        try:
            workspace_state = open_workspace(root)
        except ValueError as exc:
            self._workspace_state = None
            self._clear_workspace_watch_paths()
            self._workspace_list.clear()
            self._current_document = None
            self._open_document_missing_from_workspace = False
            self._editor_view.set_load_error(root, str(exc))
            self._clear_document_views()
            self.statusBar().showMessage(str(exc))
            self._refresh_window_title()
            return

        self._workspace_state = workspace_state
        self._current_document = None
        self._open_document_missing_from_workspace = False
        self._editor_view.clear()
        self._clear_document_views()
        self._refresh_workspace_from_disk(
            workspace_state=workspace_state,
            status_message=f"Loaded workspace {workspace_state.authored_root}",
        )

    def start_new_recipe_flow(self) -> None:
        self._start_new_recipe_flow()

    def open_recipe_file(self, recipe_path: str | Path, *, guard_unsaved: bool = True) -> None:
        if guard_unsaved and not self._confirm_unsaved_transition():
            return
        authored_root = self._workspace_state.authored_root if self._workspace_state is not None else None
        try:
            document = load_recipe_document(
                recipe_path,
                authored_root=authored_root,
                validator_service=self._validator_service,
            )
        except CatalogLoadError as exc:
            diagnostics = self._validator_service.validate_path(recipe_path, authored_root=authored_root)
            self._show_load_failure(recipe_path, exc, diagnostics)
            return

        self._current_document = document
        self._open_document_missing_from_workspace = False
        self._sync_current_document()
        self._select_workspace_path(document.path)
        self.statusBar().showMessage(f"Opened {document.path.name}")
        self._refresh_window_title()

    def save_current_document(self) -> bool:
        if self._current_document is None:
            return False
        try:
            self._current_document.save()
        except (OSError, ValueError) as exc:
            self.statusBar().showMessage(f"Save failed: {exc}")
            return False
        self._sync_current_document()
        self.statusBar().showMessage(f"Saved {self._current_document.path.name}")
        return True

    def save_current_document_as(self) -> bool:
        if self._current_document is None:
            return False
        selected, _filter = QFileDialog.getSaveFileName(
            self,
            "Save Recipe As",
            str(self._current_document.path),
            "YAML Files (*.yaml *.yml)",
        )
        if not selected:
            return False
        destination_path = Path(selected).resolve()
        if destination_path.exists() and not self._confirm_overwrite(destination_path):
            self.statusBar().showMessage("Save As canceled.")
            return False
        try:
            self._current_document.save_as(destination_path)
        except (OSError, ValueError) as exc:
            self.statusBar().showMessage(f"Save As failed: {exc}")
            return False
        self._refresh_workspace_from_disk(status_message=f"Saved {destination_path.name}")
        self._sync_current_document()
        return True

    def undo_current_document(self) -> None:
        if self._current_document is None:
            return
        if self._current_document.undo():
            self._sync_current_document()
            self.statusBar().showMessage(f"Undid change in {self._current_document.path.name}")

    def redo_current_document(self) -> None:
        if self._current_document is None:
            return
        if self._current_document.redo():
            self._sync_current_document()
            self.statusBar().showMessage(f"Redid change in {self._current_document.path.name}")

    def _open_item(self, item: QListWidgetItem) -> None:
        item_data = item.data(Qt.ItemDataRole.UserRole) or {}
        item_kind = item_data.get("kind")
        if item_kind == "recipe":
            self.open_recipe_file(item_data["path"])
        elif item_kind == "template":
            self._start_new_recipe_flow(preselected_template=Path(item_data["path"]))

    def _show_load_failure(
        self,
        recipe_path: str | Path,
        error: CatalogLoadError,
        diagnostics: DiagnosticResult,
    ) -> None:
        self._current_document = None
        self._open_document_missing_from_workspace = False
        self._editor_view.set_load_error(recipe_path, _format_catalog_load_error(error))
        self._diagnostics_view.set_result(diagnostics)
        self._yaml_preview.set_yaml("")
        self._save_action.setEnabled(False)
        self._save_as_action.setEnabled(False)
        self._undo_action.setEnabled(False)
        self._redo_action.setEnabled(False)
        self.statusBar().showMessage(f"Failed to load {Path(recipe_path).name}")
        self._refresh_window_title()

    def _apply_document_command(self, command: RecipeCommand) -> bool:
        if self._current_document is None:
            return False
        try:
            changed = self._current_document.apply_command(command)
        except ValueError as exc:
            self.statusBar().showMessage(str(exc))
            return False
        if changed:
            self._sync_current_document()
        return changed

    def _sync_current_document(self) -> None:
        if self._current_document is None:
            self._clear_document_views()
            return
        self._editor_view.set_document(self._current_document)
        self._diagnostics_view.set_result(self._current_document.validation_result)
        self._yaml_preview.set_yaml(self._current_document.to_yaml())
        self._save_action.setEnabled(True)
        self._save_as_action.setEnabled(True)
        self._undo_action.setEnabled(self._current_document.can_undo)
        self._redo_action.setEnabled(self._current_document.can_redo)
        self._refresh_window_title()

    def _clear_document_views(self) -> None:
        self._diagnostics_view.set_result(None)
        self._yaml_preview.set_yaml("")
        self._save_action.setEnabled(False)
        self._save_as_action.setEnabled(False)
        self._undo_action.setEnabled(False)
        self._redo_action.setEnabled(False)

    def _refresh_window_title(self) -> None:
        workspace_title = (
            str(self._workspace_state.authored_root)
            if self._workspace_state is not None
            else "No workspace"
        )
        document_title = (
            f"{self._current_document.working_recipe.id}{' *' if self._current_document.is_dirty else ''}"
            if self._current_document is not None
            else "No document"
        )
        self.setWindowTitle(f"EmuChef Editor | {workspace_title} | {document_title}")

    def closeEvent(self, event: QCloseEvent) -> None:  # type: ignore[override]
        if self._confirm_unsaved_transition():
            event.accept()
            return
        event.ignore()

    def _start_new_recipe_flow(self, *, preselected_template: Path | None = None) -> None:
        if self._workspace_state is None:
            self.statusBar().showMessage("Open a workspace before creating a new recipe.")
            return
        if not self._confirm_unsaved_transition():
            return
        if not self._workspace_state.template_files:
            self.statusBar().showMessage("No recipe templates were found for this workspace.")
            return
        dialog = NewRecipeDialog(
            template_paths=self._workspace_state.template_files,
            authored_root=self._workspace_state.authored_root,
            preselected_template=preselected_template,
            parent=self,
        )
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        try:
            request = dialog.request()
            if request.destination_path.exists() and not self._confirm_overwrite(request.destination_path):
                self.statusBar().showMessage("New recipe creation canceled.")
                return
            document = create_recipe_document_from_template(
                request.template_path,
                destination_path=request.destination_path,
                recipe_id=request.recipe_id,
                authored_root=self._workspace_state.authored_root,
                validator_service=self._validator_service,
            )
        except (CatalogLoadError, OSError, ValueError) as exc:
            self.statusBar().showMessage(f"New recipe creation failed: {exc}")
            return

        self._current_document = document
        self._open_document_missing_from_workspace = False
        self._refresh_workspace_from_disk(status_message=f"Created {document.path.name}")
        self._sync_current_document()

    def _populate_workspace_items(self) -> None:
        self._workspace_list.clear()
        if self._workspace_state is None:
            return
        self._add_workspace_header("Authored Recipes")
        for recipe_file in self._workspace_state.recipe_files:
            self._add_workspace_item(
                recipe_file.relative_to(self._workspace_state.authored_root).as_posix(),
                kind="recipe",
                path=recipe_file,
            )
        if self._workspace_state.template_files:
            self._add_workspace_header("Templates")
            for template_file in self._workspace_state.template_files:
                self._add_workspace_item(
                    template_file.relative_to(self._workspace_state.authored_root.parent).as_posix(),
                    kind="template",
                    path=template_file,
                )

    def _add_workspace_header(self, label: str) -> None:
        item = QListWidgetItem(label)
        item.setData(Qt.ItemDataRole.UserRole, {"kind": "header"})
        item.setFlags(Qt.ItemFlag.ItemIsEnabled)
        self._workspace_list.addItem(item)

    def _add_workspace_item(self, label: str, *, kind: str, path: Path) -> None:
        item = QListWidgetItem(label)
        item.setData(
            Qt.ItemDataRole.UserRole,
            {"kind": kind, "path": str(path.resolve())},
        )
        self._workspace_list.addItem(item)

    def _select_workspace_path(self, path: Path) -> None:
        target = path.resolve()
        for index in range(self._workspace_list.count()):
            item = self._workspace_list.item(index)
            item_data = item.data(Qt.ItemDataRole.UserRole) or {}
            if item_data.get("path") == str(target):
                self._workspace_list.setCurrentItem(item)
                return

    def _refresh_workspace_from_disk(
        self,
        *,
        workspace_state: WorkspaceState | None = None,
        status_message: str | None = None,
    ) -> None:
        selected_path = self._selected_workspace_path()
        if workspace_state is None:
            if self._workspace_state is None:
                return
            try:
                workspace_state = open_workspace(self._workspace_state.requested_root)
            except ValueError as exc:
                self._workspace_state = None
                self._clear_workspace_watch_paths()
                self._workspace_list.clear()
                self._open_document_missing_from_workspace = False
                self.statusBar().showMessage(str(exc))
                self._refresh_window_title()
                return

        self._workspace_state = workspace_state
        self._refresh_workspace_watch_paths()
        self._populate_workspace_items()

        if selected_path is not None and self._workspace_contains_path(selected_path):
            self._select_workspace_path(selected_path)
        elif self._current_document is not None and self._workspace_contains_path(self._current_document.path):
            self._select_workspace_path(self._current_document.path)
        else:
            self._clear_workspace_selection()

        self._update_open_document_workspace_presence(status_message=status_message)
        self._refresh_window_title()

    def _refresh_workspace_watch_paths(self) -> None:
        watcher_paths = list(self._workspace_watcher.directories()) + list(self._workspace_watcher.files())
        self._workspace_watcher.blockSignals(True)
        if watcher_paths:
            self._workspace_watcher.removePaths(watcher_paths)
        new_paths = self._workspace_watch_paths()
        if new_paths:
            self._workspace_watcher.addPaths(list(new_paths))
        self._workspace_watcher.blockSignals(False)

    def _clear_workspace_watch_paths(self) -> None:
        watcher_paths = list(self._workspace_watcher.directories()) + list(self._workspace_watcher.files())
        if watcher_paths:
            self._workspace_watcher.removePaths(watcher_paths)

    def _workspace_watch_paths(self) -> tuple[str, ...]:
        if self._workspace_state is None:
            return ()
        authored_root = self._workspace_state.authored_root
        repo_root = authored_root.parent
        candidate_paths = [
            authored_root / "recipes",
            repo_root / "templates",
            repo_root / "templates" / "authored",
        ]
        return tuple(str(path.resolve()) for path in candidate_paths if path.is_dir())

    def _selected_workspace_path(self) -> Path | None:
        item = self._workspace_list.currentItem()
        if item is None:
            return None
        item_data = item.data(Qt.ItemDataRole.UserRole) or {}
        path = item_data.get("path")
        return Path(path).resolve() if path is not None else None

    def _workspace_contains_path(self, path: Path) -> bool:
        if self._workspace_state is None:
            return False
        target = path.resolve()
        return target in self._workspace_state.recipe_files or target in self._workspace_state.template_files

    def _clear_workspace_selection(self) -> None:
        self._workspace_list.clearSelection()
        self._workspace_list.setCurrentRow(-1)

    def _update_open_document_workspace_presence(self, *, status_message: str | None = None) -> None:
        missing = False
        if self._current_document is not None and self._workspace_state is not None:
            recipes_root = self._workspace_state.authored_root / "recipes"
            if self._current_document.path.resolve().parent == recipes_root.resolve():
                missing = not self._workspace_contains_path(self._current_document.path)

        if missing and not self._open_document_missing_from_workspace and self._current_document is not None:
            self.statusBar().showMessage(
                f"Open document is no longer present in the workspace: {self._current_document.path.name}"
            )
        elif status_message is not None:
            self.statusBar().showMessage(status_message)

        self._open_document_missing_from_workspace = missing

    def _on_workspace_directory_changed(self, _path: str) -> None:
        self._refresh_workspace_from_disk()

    def _confirm_unsaved_transition(self) -> bool:
        if self._current_document is None or not self._current_document.is_dirty:
            return True
        response = self._prompt_unsaved_changes()
        if response == QMessageBox.StandardButton.Save:
            return self.save_current_document()
        if response == QMessageBox.StandardButton.Discard:
            return True
        return False

    def _prompt_unsaved_changes(self) -> QMessageBox.StandardButton:
        box = QMessageBox(self)
        box.setWindowTitle("Unsaved Changes")
        box.setText("Save changes before continuing?")
        save_button = box.addButton("Save", QMessageBox.ButtonRole.AcceptRole)
        discard_button = box.addButton("Don’t Save", QMessageBox.ButtonRole.DestructiveRole)
        cancel_button = box.addButton("Cancel", QMessageBox.ButtonRole.RejectRole)
        box.setDefaultButton(save_button)
        box.exec()
        clicked = box.clickedButton()
        if clicked is save_button:
            return QMessageBox.StandardButton.Save
        if clicked is discard_button:
            return QMessageBox.StandardButton.Discard
        if clicked is cancel_button:
            return QMessageBox.StandardButton.Cancel
        return QMessageBox.StandardButton.Cancel

    def _confirm_overwrite(self, destination_path: Path) -> bool:
        response = QMessageBox.question(
            self,
            "Overwrite Existing File?",
            f"{destination_path.name} already exists. Overwrite it?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        return response == QMessageBox.StandardButton.Yes


def _format_catalog_load_error(error: CatalogLoadError) -> str:
    return "\n".join(f"{issue.code.value}: {issue.message}" for issue in error.errors)

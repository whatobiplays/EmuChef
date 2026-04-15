"""Main window for the Milestone 1 recipe editor shell."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtGui import QAction
from PySide6.QtWidgets import (
    QFileDialog,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QSplitter,
    QTabWidget,
)

from emuchef.planner.catalog import CatalogLoadError
from emuchef_editor.core.documents.recipe_document import RecipeDocument
from emuchef_editor.core.validation.validator_service import DiagnosticResult, ValidatorService
from emuchef_editor.core.yaml.loader import load_recipe_document

from .recipe_editor.placeholder import RecipeEditorPlaceholder
from .shared.diagnostics_view import DiagnosticsView
from .shared.yaml_preview import YamlPreview
from .workspace.service import WorkspaceState, open_workspace


class MainWindow(QMainWindow):
    def __init__(self, workspace_root: str | Path | None = None) -> None:
        super().__init__()
        self._validator_service = ValidatorService()
        self._workspace_state: WorkspaceState | None = None
        self._current_document: RecipeDocument | None = None

        self._workspace_list = QListWidget()
        self._workspace_list.itemActivated.connect(self._open_item)

        self._editor_view = RecipeEditorPlaceholder()
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
        self._save_action = QAction("Save Canonical YAML", self)
        self._save_action.triggered.connect(self.save_current_document)
        self._save_action.setEnabled(False)

        file_menu = self.menuBar().addMenu("File")
        file_menu.addAction(self._open_workspace_action)
        file_menu.addAction(self._save_action)

        toolbar = self.addToolBar("Main")
        toolbar.addAction(self._open_workspace_action)
        toolbar.addAction(self._save_action)

        self.statusBar().showMessage("Open a repo root or authored root to begin.")
        self.resize(1400, 800)
        self._refresh_window_title()

        if workspace_root is not None:
            self.open_workspace(workspace_root)

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

    def open_workspace(self, root: str | Path) -> None:
        try:
            self._workspace_state = open_workspace(root)
        except ValueError as exc:
            self._workspace_state = None
            self._workspace_list.clear()
            self._current_document = None
            self._editor_view.set_load_error(root, str(exc))
            self._diagnostics_view.set_result(None)
            self._yaml_preview.set_yaml("")
            self._save_action.setEnabled(False)
            self.statusBar().showMessage(str(exc))
            self._refresh_window_title()
            return

        self._workspace_list.clear()
        for recipe_file in self._workspace_state.recipe_files:
            item = QListWidgetItem(recipe_file.relative_to(self._workspace_state.authored_root).as_posix())
            item.setData(Qt.ItemDataRole.UserRole, str(recipe_file))
            self._workspace_list.addItem(item)

        self._editor_view.clear()
        self._diagnostics_view.set_result(None)
        self._yaml_preview.set_yaml("")
        self._current_document = None
        self._save_action.setEnabled(False)
        self.statusBar().showMessage(f"Loaded workspace {self._workspace_state.authored_root}")
        self._refresh_window_title()

    def open_recipe_file(self, recipe_path: str | Path) -> None:
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
        self._editor_view.set_document(document)
        self._diagnostics_view.set_result(document.validation_result)
        self._yaml_preview.set_yaml(document.to_yaml())
        self._save_action.setEnabled(True)
        self.statusBar().showMessage(f"Opened {document.path.name}")
        self._refresh_window_title()

    def save_current_document(self) -> None:
        if self._current_document is None:
            return
        self._current_document.save()
        self._editor_view.set_document(self._current_document)
        self._diagnostics_view.set_result(self._current_document.validation_result)
        self._yaml_preview.set_yaml(self._current_document.to_yaml())
        self.statusBar().showMessage(f"Saved {self._current_document.path.name}")

    def _open_item(self, item: QListWidgetItem) -> None:
        recipe_path = item.data(Qt.ItemDataRole.UserRole)
        if recipe_path:
            self.open_recipe_file(recipe_path)

    def _show_load_failure(
        self,
        recipe_path: str | Path,
        error: CatalogLoadError,
        diagnostics: DiagnosticResult,
    ) -> None:
        self._current_document = None
        self._editor_view.set_load_error(recipe_path, _format_catalog_load_error(error))
        self._diagnostics_view.set_result(diagnostics)
        self._yaml_preview.set_yaml("")
        self._save_action.setEnabled(False)
        self.statusBar().showMessage(f"Failed to load {Path(recipe_path).name}")
        self._refresh_window_title()

    def _refresh_window_title(self) -> None:
        workspace_title = (
            str(self._workspace_state.authored_root)
            if self._workspace_state is not None
            else "No workspace"
        )
        document_title = (
            self._current_document.working_recipe.id
            if self._current_document is not None
            else "No document"
        )
        self.setWindowTitle(f"EmuChef Editor | {workspace_title} | {document_title}")


def _format_catalog_load_error(error: CatalogLoadError) -> str:
    return "\n".join(f"{issue.code.value}: {issue.message}" for issue in error.errors)

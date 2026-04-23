"""Tabbed recipe editor widget."""

from __future__ import annotations

from PySide6.QtWidgets import QStackedWidget, QTabWidget, QVBoxLayout, QWidget

from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .artifact_groups_page import ArtifactGroupsPage
from .artifacts_page import ArtifactsPage
from .inputs_page import InputsPage
from .overview_page import OverviewPage
from .placeholder import RecipeEditorPlaceholder
from .permissions_page import PermissionsPage


class RecipeEditor(QWidget):
    """Owns the center-pane editor pages and empty-state placeholder."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._placeholder = RecipeEditorPlaceholder()
        self._tabs = QTabWidget()
        self._overview_page = OverviewPage(command_handler)
        self._inputs_page = InputsPage(command_handler)
        self._artifacts_page = ArtifactsPage(command_handler)
        self._artifact_groups_page = ArtifactGroupsPage(command_handler)
        self._permissions_page = PermissionsPage(command_handler)
        self._tabs.addTab(self._overview_page, "Overview")
        self._tabs.addTab(self._inputs_page, "Inputs")
        self._tabs.addTab(self._artifacts_page, "Artifacts")
        self._tabs.addTab(self._artifact_groups_page, "Artifact Groups")
        self._tabs.addTab(self._permissions_page, "Permissions")

        self._stack = QStackedWidget()
        self._stack.addWidget(self._placeholder)
        self._stack.addWidget(self._tabs)

        layout = QVBoxLayout(self)
        layout.addWidget(self._stack)
        self.clear()

    def set_document(self, document: RecipeDocument) -> None:
        self._overview_page.set_document(document)
        self._inputs_page.set_document(document)
        self._artifacts_page.set_document(document)
        self._artifact_groups_page.set_document(document)
        self._permissions_page.set_document(document)
        self._stack.setCurrentWidget(self._tabs)

    def set_load_error(self, path, message: str) -> None:
        self._placeholder.set_load_error(path, message)
        self._stack.setCurrentWidget(self._placeholder)

    def clear(self) -> None:
        self._placeholder.clear()
        self._stack.setCurrentWidget(self._placeholder)

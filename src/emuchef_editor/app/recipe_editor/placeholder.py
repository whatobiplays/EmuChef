"""Placeholder center-pane widget for Milestone 1."""

from __future__ import annotations

from pathlib import Path

from PySide6.QtWidgets import QLabel, QVBoxLayout, QWidget

from emuchef_editor.core.documents.recipe_document import RecipeDocument


class RecipeEditorPlaceholder(QWidget):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._title = QLabel("No recipe selected.")
        self._details = QLabel("Open a workspace, then open a recipe from the left panel.")
        self._details.setWordWrap(True)
        self._state = QLabel("")
        self._state.setWordWrap(True)

        layout = QVBoxLayout(self)
        layout.addWidget(self._title)
        layout.addWidget(self._details)
        layout.addWidget(self._state)
        layout.addStretch(1)

    def set_document(self, document: RecipeDocument) -> None:
        self._title.setText(document.working_recipe.name)
        self._details.setText(
            "\n".join(
                [
                    f"Recipe ID: {document.working_recipe.id}",
                    f"Path: {document.path}",
                    f"Authored root: {document.authored_root if document.authored_root is not None else 'None'}",
                    "Recipe section editing widgets arrive in a later milestone.",
                ]
            )
        )
        self._state.setText(f"Dirty: {'yes' if document.is_dirty else 'no'}")

    def set_load_error(self, path: str | Path, message: str) -> None:
        self._title.setText("Recipe could not be loaded")
        self._details.setText(f"Path: {Path(path).resolve()}")
        self._state.setText(message)

    def clear(self) -> None:
        self._title.setText("No recipe selected.")
        self._details.setText("Open a workspace, then open a recipe from the left panel.")
        self._state.setText("")

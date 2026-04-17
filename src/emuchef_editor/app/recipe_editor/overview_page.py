"""Overview editor page for recipe metadata."""

from __future__ import annotations

from PySide6.QtCore import QSignalBlocker, Qt
from PySide6.QtWidgets import QFormLayout, QHBoxLayout, QLabel, QVBoxLayout, QWidget

from emuchef_editor.core.documents.commands import (
    AddProvidedFeatureCommand,
    AddRecipeDependencyCommand,
    MoveProvidedFeatureCommand,
    MoveRecipeDependencyCommand,
    RemoveProvidedFeatureCommand,
    RemoveRecipeDependencyCommand,
    SetOverviewFieldCommand,
    UpdateProvidedFeatureCommand,
    UpdateRecipeDependencyCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .common import (
    CommitPlainTextEdit,
    OrderedStringListEditor,
    configure_data_entry_form,
    create_expanding_line_edit,
    expand_form_field,
)


class OverviewPage(QWidget):
    """Edits recipe overview fields without touching nested YAML directly."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None

        self._id_edit = create_expanding_line_edit()
        self._name_edit = create_expanding_line_edit()
        self._description_edit = CommitPlainTextEdit()
        expand_form_field(self._description_edit)
        self._kind_label = QLabel("")
        self._schema_version_label = QLabel("")
        self._kind_label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self._schema_version_label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)

        self._form = configure_data_entry_form(QFormLayout())
        self._form.addRow("ID", self._id_edit)
        self._form.addRow("Name", self._name_edit)
        self._form.addRow("Kind", self._kind_label)
        self._form.addRow("Schema Version", self._schema_version_label)

        self._recipe_dependencies = OrderedStringListEditor(
            prompt_title="Add Recipe Dependency",
            prompt_label="Dependency recipe id",
        )
        self._provided_features = OrderedStringListEditor(
            prompt_title="Add Provided Feature",
            prompt_label="Feature name",
        )

        lists_row = QHBoxLayout()
        dependency_panel = QVBoxLayout()
        dependency_panel.addWidget(QLabel("Recipe Dependencies"))
        dependency_panel.addWidget(self._recipe_dependencies)
        feature_panel = QVBoxLayout()
        feature_panel.addWidget(QLabel("Provides Features"))
        feature_panel.addWidget(self._provided_features)
        lists_row.addLayout(dependency_panel)
        lists_row.addLayout(feature_panel)

        layout = QVBoxLayout(self)
        layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        layout.addLayout(self._form)
        layout.addWidget(QLabel("Description"))
        layout.addWidget(self._description_edit)
        layout.addLayout(lists_row)
        layout.addStretch(1)

        self._id_edit.editingFinished.connect(self._commit_id)
        self._name_edit.editingFinished.connect(self._commit_name)
        self._description_edit.committed.connect(self._commit_description)

        self._recipe_dependencies.add_requested.connect(self._add_dependency)
        self._recipe_dependencies.update_requested.connect(self._update_dependency)
        self._recipe_dependencies.remove_requested.connect(self._remove_dependency)
        self._recipe_dependencies.move_requested.connect(self._move_dependency)

        self._provided_features.add_requested.connect(self._add_feature)
        self._provided_features.update_requested.connect(self._update_feature)
        self._provided_features.remove_requested.connect(self._remove_feature)
        self._provided_features.move_requested.connect(self._move_feature)

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        recipe = document.working_recipe
        with QSignalBlocker(self._id_edit):
            self._id_edit.setText(recipe.id)
        with QSignalBlocker(self._name_edit):
            self._name_edit.setText(recipe.name)
        self._description_edit.set_committed_text(recipe.description or "")
        self._kind_label.setText(recipe.kind)
        self._schema_version_label.setText(str(recipe.schema_version))
        self._recipe_dependencies.set_items(recipe.recipe_dependencies)
        self._provided_features.set_items(recipe.provides.features)

    def _commit_id(self) -> None:
        if self._document is None:
            return
        if self._id_edit.text() != self._document.working_recipe.id:
            self._command_handler(SetOverviewFieldCommand(field="id", value=self._id_edit.text()))

    def _commit_name(self) -> None:
        if self._document is None:
            return
        if self._name_edit.text() != self._document.working_recipe.name:
            self._command_handler(SetOverviewFieldCommand(field="name", value=self._name_edit.text()))

    def _commit_description(self, value: str) -> None:
        if self._document is None:
            return
        if value != (self._document.working_recipe.description or ""):
            self._command_handler(SetOverviewFieldCommand(field="description", value=value))

    def _add_dependency(self, value: str) -> None:
        self._command_handler(AddRecipeDependencyCommand(value=value))

    def _update_dependency(self, index: int, value: str) -> None:
        self._command_handler(UpdateRecipeDependencyCommand(index=index, value=value))

    def _remove_dependency(self, index: int) -> None:
        self._command_handler(RemoveRecipeDependencyCommand(index=index))

    def _move_dependency(self, index: int, to_index: int) -> None:
        self._command_handler(MoveRecipeDependencyCommand(index=index, to_index=to_index))

    def _add_feature(self, value: str) -> None:
        self._command_handler(AddProvidedFeatureCommand(value=value))

    def _update_feature(self, index: int, value: str) -> None:
        self._command_handler(UpdateProvidedFeatureCommand(index=index, value=value))

    def _remove_feature(self, index: int) -> None:
        self._command_handler(RemoveProvidedFeatureCommand(index=index))

    def _move_feature(self, index: int, to_index: int) -> None:
        self._command_handler(MoveProvidedFeatureCommand(index=index, to_index=to_index))

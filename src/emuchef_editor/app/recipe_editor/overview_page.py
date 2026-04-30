"""Overview editor page for recipe metadata."""

from __future__ import annotations

from PySide6.QtCore import QSignalBlocker, Qt
from PySide6.QtWidgets import QFormLayout, QHBoxLayout, QLabel, QPushButton, QVBoxLayout, QWidget

from emuchef_editor.core.analysis.usages import UsageTarget, analyze_recipe_usages
from emuchef_editor.core.documents.commands import (
    AddProvidedFeatureCommand,
    AddRecipeDependencyCommand,
    MoveProvidedFeatureCommand,
    MoveRecipeDependencyCommand,
    RemoveProvidedFeatureCommand,
    RemoveRecipeDependencyCommand,
    RenameRecipeIdCommand,
    SetOverviewFieldCommand,
    UpdateProvidedFeatureCommand,
    UpdateRecipeDependencyCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .common import (
    CommitPlainTextEdit,
    OrderedStringListEditor,
    TextEntryDialog,
    add_tooltipped_form_row,
    apply_tooltip,
    configure_data_entry_form,
    create_expanding_line_edit,
    expand_form_field,
)
from .tooltips import field_tooltip, prompt_tooltip
from .usage_dialogs import FindUsagesDialog, confirm_preserved_content_warning


class OverviewPage(QWidget):
    """Edits recipe overview fields without touching nested YAML directly."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None

        self._id_edit = create_expanding_line_edit(read_only=True)
        self._name_edit = create_expanding_line_edit()
        self._description_edit = CommitPlainTextEdit()
        expand_form_field(self._description_edit)
        self._kind_label = QLabel("")
        self._schema_version_label = QLabel("")
        self._kind_label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self._schema_version_label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)

        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._form, "ID", self._id_edit, field_tooltip("overview.id"))
        add_tooltipped_form_row(self._form, "Name", self._name_edit, field_tooltip("overview.name"))
        add_tooltipped_form_row(self._form, "Kind", self._kind_label, field_tooltip("overview.kind"))
        add_tooltipped_form_row(
            self._form,
            "Schema Version",
            self._schema_version_label,
            field_tooltip("overview.schema_version"),
        )

        self._recipe_dependencies = OrderedStringListEditor(
            prompt_title="Add Recipe Dependency",
            prompt_label="Dependency recipe id",
            prompt_tooltip=prompt_tooltip("overview.recipe_dependencies"),
            field_tooltip=field_tooltip("overview.recipe_dependencies"),
        )
        self._provided_features = OrderedStringListEditor(
            prompt_title="Add Provided Feature",
            prompt_label="Feature name",
            prompt_tooltip=prompt_tooltip("overview.provides_features"),
            field_tooltip=field_tooltip("overview.provides_features"),
        )

        self._rename_id_button = QPushButton("Rename")
        self._find_id_usages_button = QPushButton("Find Usages")
        id_actions = QHBoxLayout()
        id_actions.addWidget(self._rename_id_button)
        id_actions.addWidget(self._find_id_usages_button)
        id_actions.addStretch(1)

        lists_row = QHBoxLayout()
        dependency_panel = QVBoxLayout()
        self._recipe_dependencies_label = QLabel("Recipe Dependencies")
        apply_tooltip(self._recipe_dependencies_label, field_tooltip("overview.recipe_dependencies"))
        dependency_panel.addWidget(self._recipe_dependencies_label)
        dependency_panel.addWidget(self._recipe_dependencies)
        feature_panel = QVBoxLayout()
        self._provided_features_label = QLabel("Provides Features")
        apply_tooltip(self._provided_features_label, field_tooltip("overview.provides_features"))
        feature_panel.addWidget(self._provided_features_label)
        feature_panel.addWidget(self._provided_features)
        lists_row.addLayout(dependency_panel)
        lists_row.addLayout(feature_panel)

        self._description_label = QLabel("Description")
        apply_tooltip(self._description_label, field_tooltip("overview.description"))
        apply_tooltip(self._description_edit, field_tooltip("overview.description"))

        layout = QVBoxLayout(self)
        layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        layout.addLayout(self._form)
        layout.addLayout(id_actions)
        layout.addWidget(self._description_label)
        layout.addWidget(self._description_edit)
        layout.addLayout(lists_row)
        layout.addStretch(1)

        self._rename_id_button.clicked.connect(self._rename_recipe_id)
        self._find_id_usages_button.clicked.connect(self._find_recipe_id_usages)
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

    def _rename_recipe_id(self) -> None:
        if self._document is None:
            return
        current_id = self._document.working_recipe.id
        new_id = TextEntryDialog.prompt(
            self,
            title="Rename Recipe ID",
            label="Recipe id",
            tooltip=prompt_tooltip("overview.id"),
            initial_value=current_id,
        )
        if new_id is None or new_id == current_id:
            return
        analysis = analyze_recipe_usages(self._document.working_recipe, UsageTarget(kind="recipe", id=current_id))
        if analysis.has_preserved_unsupported_content_warning and not confirm_preserved_content_warning(self, action="Renaming this recipe id"):
            return
        self._command_handler(RenameRecipeIdCommand(new_recipe_id=new_id))

    def _find_recipe_id_usages(self) -> None:
        if self._document is None:
            return
        recipe_id = self._document.working_recipe.id
        analysis = analyze_recipe_usages(self._document.working_recipe, UsageTarget(kind="recipe", id=recipe_id))
        FindUsagesDialog.show_usages(self, item_label=f"recipe {recipe_id}", analysis=analysis)

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

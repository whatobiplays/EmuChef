"""Inputs editor page."""

from __future__ import annotations

import yaml
from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from emuchef.domain import InputRole, InputType
from emuchef_editor.core.documents.commands import (
    AddInputCommand,
    DeleteInputCommand,
    DuplicateInputCommand,
    UpdateInputFieldCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .common import (
    CommitPlainTextEdit,
    TextEntryDialog,
    add_tooltipped_form_row,
    apply_tooltip,
    configure_data_entry_form,
    create_expanding_combo_box,
    create_expanding_line_edit,
    expand_form_field,
)
from .tooltips import field_tooltip, prompt_tooltip


class InputsPage(QWidget):
    """Edits recipe input declarations via document commands."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None
        self._loading = False
        self._selected_input_id: str | None = None

        self._list = QListWidget()
        self._list.currentItemChanged.connect(self._on_selection_changed)
        self._add_button = QPushButton("Add")
        self._delete_button = QPushButton("Delete")
        self._duplicate_button = QPushButton("Duplicate")

        list_buttons = QHBoxLayout()
        list_buttons.addWidget(self._add_button)
        list_buttons.addWidget(self._delete_button)
        list_buttons.addWidget(self._duplicate_button)

        left_panel = QWidget()
        left_layout = QVBoxLayout(left_panel)
        left_layout.addWidget(self._list)
        left_layout.addLayout(list_buttons)

        self._id_value = create_expanding_line_edit(read_only=True)
        self._type_combo = create_expanding_combo_box()
        for value in InputType:
            self._type_combo.addItem(value.value, value)
        self._role_combo = create_expanding_combo_box()
        for value in InputRole:
            self._role_combo.addItem(value.value, value)
        self._label_edit = create_expanding_line_edit()
        self._description_edit = CommitPlainTextEdit()
        expand_form_field(self._description_edit)
        self._required_check = QCheckBox()
        self._multiple_check = QCheckBox()
        self._must_exist_check = QCheckBox()
        self._allowed_extensions_edit = create_expanding_line_edit()
        self._path_kind_combo = create_expanding_combo_box()
        self._path_kind_combo.addItem("(none)", None)
        for value in InputType:
            self._path_kind_combo.addItem(value.value, value)
        self._default_value = QLabel("")
        self._default_value.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self._metadata_value = QLabel("")
        self._metadata_value.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self._default_label = QLabel("Default")
        self._metadata_label = QLabel("Metadata")
        self._default_value.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        self._metadata_value.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)

        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._form, "ID", self._id_value, field_tooltip("inputs.id"))
        add_tooltipped_form_row(self._form, "Type", self._type_combo, field_tooltip("inputs.type"))
        add_tooltipped_form_row(self._form, "Role", self._role_combo, field_tooltip("inputs.role"))
        add_tooltipped_form_row(self._form, "Label", self._label_edit, field_tooltip("inputs.label"))
        add_tooltipped_form_row(self._form, "Required", self._required_check, field_tooltip("inputs.required"))
        add_tooltipped_form_row(self._form, "Multiple", self._multiple_check, field_tooltip("inputs.multiple"))
        add_tooltipped_form_row(
            self._form,
            "Must Exist",
            self._must_exist_check,
            field_tooltip("inputs.must_exist"),
        )
        add_tooltipped_form_row(
            self._form,
            "Allowed Extensions",
            self._allowed_extensions_edit,
            field_tooltip("inputs.allowed_extensions"),
        )
        add_tooltipped_form_row(
            self._form,
            "Path Kind",
            self._path_kind_combo,
            field_tooltip("inputs.path_kind"),
        )
        add_tooltipped_form_row(self._form, self._default_label, self._default_value, field_tooltip("inputs.default"))
        add_tooltipped_form_row(
            self._form,
            self._metadata_label,
            self._metadata_value,
            field_tooltip("inputs.metadata"),
        )

        self._description_label = QLabel("Description")
        apply_tooltip(self._description_label, field_tooltip("inputs.description"))
        apply_tooltip(self._description_edit, field_tooltip("inputs.description"))

        right_panel = QWidget()
        right_layout = QVBoxLayout(right_panel)
        right_layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        right_layout.addLayout(self._form)
        right_layout.addWidget(self._description_label)
        right_layout.addWidget(self._description_edit)
        right_layout.addStretch(1)

        splitter = QSplitter()
        splitter.addWidget(left_panel)
        splitter.addWidget(right_panel)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 2)

        layout = QVBoxLayout(self)
        layout.addWidget(splitter)

        self._add_button.clicked.connect(self._add_input)
        self._delete_button.clicked.connect(self._delete_input)
        self._duplicate_button.clicked.connect(self._duplicate_input)
        self._type_combo.currentIndexChanged.connect(self._commit_type)
        self._role_combo.currentIndexChanged.connect(self._commit_role)
        self._label_edit.editingFinished.connect(self._commit_label)
        self._description_edit.committed.connect(self._commit_description)
        self._required_check.toggled.connect(self._commit_required)
        self._multiple_check.toggled.connect(self._commit_multiple)
        self._must_exist_check.toggled.connect(self._commit_must_exist)
        self._allowed_extensions_edit.editingFinished.connect(self._commit_allowed_extensions)
        self._path_kind_combo.currentIndexChanged.connect(self._commit_path_kind)

        self._refresh_detail_enabled()

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        current_id = self._selected_input_id if self._selected_input_id in document.working_recipe.inputs else None
        items = sorted(document.working_recipe.inputs)
        self._loading = True
        self._list.clear()
        for input_id in items:
            item = QListWidgetItem(input_id)
            item.setData(Qt.ItemDataRole.UserRole, input_id)
            self._list.addItem(item)
        self._loading = False

        if items:
            target_id = current_id or items[0]
            self._select_input(target_id)
        else:
            self._selected_input_id = None
            self._clear_detail()
        self._refresh_button_state()

    def _select_input(self, input_id: str) -> None:
        for row in range(self._list.count()):
            item = self._list.item(row)
            if item.data(Qt.ItemDataRole.UserRole) == input_id:
                self._list.setCurrentRow(row)
                return

    def _on_selection_changed(self, current: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        if self._loading:
            return
        self._selected_input_id = current.data(Qt.ItemDataRole.UserRole) if current is not None else None
        self._load_selected_input()

    def _load_selected_input(self) -> None:
        if self._document is None or self._selected_input_id is None:
            self._clear_detail()
            return
        declaration = self._document.working_recipe.inputs[self._selected_input_id]
        self._loading = True
        self._id_value.setText(declaration.id)
        self._type_combo.setCurrentIndex(self._type_combo.findData(declaration.type))
        self._role_combo.setCurrentIndex(self._role_combo.findData(declaration.role))
        self._label_edit.setText(declaration.label)
        self._description_edit.set_committed_text(declaration.description or "")
        self._required_check.setChecked(declaration.required)
        self._multiple_check.setChecked(declaration.multiple)
        self._must_exist_check.setChecked(declaration.validation.must_exist)
        self._allowed_extensions_edit.setText(", ".join(declaration.validation.allowed_extensions))
        self._path_kind_combo.setCurrentIndex(self._path_kind_combo.findData(declaration.validation.path_kind))
        default_text = _format_optional_yaml(declaration.default)
        metadata_text = _format_optional_yaml(dict(declaration.metadata) if declaration.metadata else None)
        self._default_value.setText(default_text)
        self._metadata_value.setText(metadata_text)
        self._default_label.setVisible(bool(default_text))
        self._default_value.setVisible(bool(default_text))
        self._metadata_label.setVisible(bool(metadata_text))
        self._metadata_value.setVisible(bool(metadata_text))
        self._loading = False
        self._refresh_detail_enabled()
        self._refresh_button_state()

    def _clear_detail(self) -> None:
        self._loading = True
        self._id_value.clear()
        self._type_combo.setCurrentIndex(0)
        self._role_combo.setCurrentIndex(0)
        self._label_edit.clear()
        self._description_edit.set_committed_text("")
        self._required_check.setChecked(False)
        self._multiple_check.setChecked(False)
        self._must_exist_check.setChecked(False)
        self._allowed_extensions_edit.clear()
        self._path_kind_combo.setCurrentIndex(0)
        self._default_value.clear()
        self._metadata_value.clear()
        self._default_label.setVisible(False)
        self._default_value.setVisible(False)
        self._metadata_label.setVisible(False)
        self._metadata_value.setVisible(False)
        self._loading = False
        self._refresh_detail_enabled()
        self._refresh_button_state()

    def _refresh_detail_enabled(self) -> None:
        enabled = self._selected_input_id is not None
        for widget in (
            self._type_combo,
            self._role_combo,
            self._label_edit,
            self._description_edit,
            self._required_check,
            self._multiple_check,
            self._must_exist_check,
            self._allowed_extensions_edit,
            self._path_kind_combo,
        ):
            widget.setEnabled(enabled)

    def _refresh_button_state(self) -> None:
        has_selection = self._selected_input_id is not None
        self._delete_button.setEnabled(has_selection)
        self._duplicate_button.setEnabled(has_selection)

    def _add_input(self) -> None:
        input_id = self._prompt_for_identifier("Add Input", "Input id", prompt_tooltip("inputs.id"))
        if input_id is None:
            return
        previous_selection = self._selected_input_id
        self._selected_input_id = input_id
        if not self._command_handler(AddInputCommand(input_id=input_id)):
            self._selected_input_id = previous_selection

    def _delete_input(self) -> None:
        if self._document is None or self._selected_input_id is None:
            return
        input_id = self._selected_input_id
        sorted_ids = sorted(self._document.working_recipe.inputs)
        current_index = sorted_ids.index(input_id)
        remaining = [item for item in sorted_ids if item != input_id]
        next_selection = remaining[min(current_index, len(remaining) - 1)] if remaining else None
        self._selected_input_id = next_selection
        if not self._command_handler(DeleteInputCommand(input_id=input_id)):
            self._selected_input_id = input_id

    def _duplicate_input(self) -> None:
        if self._selected_input_id is None:
            return
        source_input_id = self._selected_input_id
        new_input_id = self._prompt_for_identifier("Duplicate Input", "New input id", prompt_tooltip("inputs.id"))
        if new_input_id is None:
            return
        previous_selection = self._selected_input_id
        self._selected_input_id = new_input_id
        if not self._command_handler(
            DuplicateInputCommand(
                source_input_id=source_input_id,
                new_input_id=new_input_id,
            )
        ):
            self._selected_input_id = previous_selection

    def _commit_type(self) -> None:
        self._apply_field_update("type", self._type_combo.currentData())

    def _commit_role(self) -> None:
        self._apply_field_update("role", self._role_combo.currentData())

    def _commit_label(self) -> None:
        self._apply_field_update("label", self._label_edit.text())

    def _commit_description(self, value: str) -> None:
        self._apply_field_update("description", value)

    def _commit_required(self, value: bool) -> None:
        self._apply_field_update("required", value)

    def _commit_multiple(self, value: bool) -> None:
        self._apply_field_update("multiple", value)

    def _commit_must_exist(self, value: bool) -> None:
        self._apply_field_update("validation.must_exist", value)

    def _commit_allowed_extensions(self) -> None:
        self._apply_field_update("validation.allowed_extensions", self._allowed_extensions_edit.text())

    def _commit_path_kind(self) -> None:
        self._apply_field_update("validation.path_kind", self._path_kind_combo.currentData())

    def _apply_field_update(self, field: str, value: object) -> None:
        if self._loading or self._selected_input_id is None:
            return
        self._command_handler(UpdateInputFieldCommand(input_id=self._selected_input_id, field=field, value=value))

    def _prompt_for_identifier(self, title: str, label: str, tooltip: str | None) -> str | None:
        return TextEntryDialog.prompt(self, title=title, label=label, tooltip=tooltip)


def _format_optional_yaml(value: object) -> str:
    if value in (None, {}, ()):
        return ""
    return yaml.safe_dump(value, sort_keys=False, allow_unicode=True).strip()

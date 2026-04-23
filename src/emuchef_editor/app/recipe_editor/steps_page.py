"""Steps editor page."""

from __future__ import annotations

from collections.abc import Mapping

import yaml
from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QMessageBox,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from emuchef.domain import (
    AuthoredParamValue,
    RefParamValue,
    STEP_SPECS,
    Step,
    StepCondition,
    StepConstraints,
    StepType,
)
from emuchef_editor.core.documents.commands import (
    AddStepCommand,
    DeleteStepCommand,
    DuplicateStepCommand,
    ReorderStepCommand,
    SetStepUserToggleableCommand,
    UpdateStepBasicsCommand,
    UpdateStepConstraintsCommand,
    UpdateStepDependenciesCommand,
    UpdateStepParamsCommand,
    UpdateStepSkipIfCommand,
    UpdateStepVerifyCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument
from .common import (
    AutoSizingListWidget,
    AutoSizingPlainTextEdit,
    CommitPlainTextEdit,
    CurrentWidgetSizeStack,
    TextEntryDialog,
    add_tooltipped_form_row,
    apply_tooltip,
    configure_data_entry_form,
    create_expanding_combo_box,
    create_expanding_line_edit,
    expand_form_field,
)
from .step_metadata import (
    CONDITION_PARAM_FIELD,
    COPY_FILES_HELP,
    GRANT_PERMISSIONS_NOTE,
    KNOWN_CAPABILITIES,
    REF_VALUE_FILTERS,
    SUPPORTED_CONDITION_TYPES,
    SUPPORTED_EDITOR_STEP_TYPES,
)
from .tooltips import field_tooltip, prompt_tooltip


def _yaml_block(value: object) -> str:
    """Render a small YAML fragment for read-only preserved-content views."""

    if value in (None, {}, [], (), ""):
        return ""
    return yaml.safe_dump(value, sort_keys=False, allow_unicode=True).strip()


def _condition_label(condition: StepCondition) -> str:
    """Build a concise label for a supported or preserved condition row."""

    field_name, _label = CONDITION_PARAM_FIELD.get(condition.type, ("", ""))
    if field_name and field_name in condition.params:
        return f"{condition.type} · {condition.params[field_name]}"
    if condition.type in SUPPORTED_CONDITION_TYPES:
        return condition.type
    return f"[Preserved] {condition.type}"


class _OrderedChoiceEditor(QWidget):
    """Ordered structured selector for string ids."""

    changed = Signal(tuple)

    def __init__(
        self,
        *,
        prompt_title: str,
        prompt_label: str,
        field_tooltip_text: str | None = None,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._prompt_title = prompt_title
        self._prompt_label = prompt_label
        self._choices: tuple[tuple[str, str], ...] = ()
        self._loading = False
        self._locked = False
        self._pending_values: tuple[str, ...] | None = None
        self._pending_selection: str | None = None

        self._list = AutoSizingListWidget()
        apply_tooltip(self, field_tooltip_text)
        apply_tooltip(self._list, field_tooltip_text)
        self._list.currentRowChanged.connect(self._refresh_button_state)
        self._add_button = QPushButton("Add")
        self._remove_button = QPushButton("Remove")
        self._up_button = QPushButton("Move Up")
        self._down_button = QPushButton("Move Down")

        button_row = QHBoxLayout()
        button_row.addWidget(self._add_button)
        button_row.addWidget(self._remove_button)
        button_row.addWidget(self._up_button)
        button_row.addWidget(self._down_button)

        layout = QVBoxLayout(self)
        layout.addWidget(self._list)
        layout.addLayout(button_row)

        self._add_button.clicked.connect(self._add_item)
        self._remove_button.clicked.connect(self._remove_item)
        self._up_button.clicked.connect(self._move_up)
        self._down_button.clicked.connect(self._move_down)
        self._refresh_geometry()
        self._refresh_button_state()

    def set_state(
        self,
        items: tuple[str, ...],
        *,
        choices: tuple[tuple[str, str], ...],
        locked: bool = False,
    ) -> None:
        self._choices = choices
        self._locked = locked
        labels = {value: label for value, label in choices}
        current_value = self._pending_selection if self._pending_selection is not None else self.current_value()
        self._pending_values = None
        self._pending_selection = None
        self._loading = True
        self._list.clear()
        for value in items:
            item = QListWidgetItem(labels.get(value, f"[Preserved] {value}"))
            item.setData(Qt.ItemDataRole.UserRole, value)
            self._list.addItem(item)
        self._loading = False
        if self._list.count():
            target_row = 0
            if current_value is not None:
                for row in range(self._list.count()):
                    if self._list.item(row).data(Qt.ItemDataRole.UserRole) == current_value:
                        target_row = row
                        break
            self._list.setCurrentRow(target_row)
        self._refresh_geometry()
        self._refresh_button_state()

    def values(self) -> tuple[str, ...]:
        if self._pending_values is not None:
            return self._pending_values
        return tuple(self._list.item(row).data(Qt.ItemDataRole.UserRole) for row in range(self._list.count()))

    def current_value(self) -> str | None:
        item = self._list.currentItem()
        return item.data(Qt.ItemDataRole.UserRole) if item is not None else None

    def _refresh_button_state(self) -> None:
        row = self._list.currentRow()
        has_selection = row >= 0
        self._remove_button.setEnabled(has_selection and not self._locked)
        self._up_button.setEnabled(has_selection and row > 0 and not self._locked)
        self._down_button.setEnabled(has_selection and row < self._list.count() - 1 and not self._locked)
        available = [value for value, _label in self._choices if value not in self.values()]
        self._add_button.setEnabled(bool(available))

    def _add_item(self) -> None:
        available_choices = [(value, label) for value, label in self._choices if value not in self.values()]
        if not available_choices:
            return
        labels = [label for _value, label in available_choices]
        selected_label, accepted = QInputDialog.getItem(
            self,
            self._prompt_title,
            self._prompt_label,
            labels,
            0,
            False,
        )
        if not accepted:
            return
        selected_value = next(value for value, label in available_choices if label == selected_label)
        self._emit_change(self.values() + (selected_value,), selected_value)

    def _remove_item(self) -> None:
        if self._locked:
            return
        row = self._list.currentRow()
        if row < 0:
            return
        values = list(self.values())
        del values[row]
        next_selection = values[min(row, len(values) - 1)] if values else None
        self._emit_change(tuple(values), next_selection)

    def _move_up(self) -> None:
        if self._locked:
            return
        row = self._list.currentRow()
        if row <= 0:
            return
        values = list(self.values())
        values[row - 1], values[row] = values[row], values[row - 1]
        self._emit_change(tuple(values), values[row - 1])

    def _move_down(self) -> None:
        if self._locked:
            return
        row = self._list.currentRow()
        if row < 0 or row >= self._list.count() - 1:
            return
        values = list(self.values())
        values[row + 1], values[row] = values[row], values[row + 1]
        self._emit_change(tuple(values), values[row + 1])

    def _emit_change(self, values: tuple[str, ...], selected_value: str | None) -> None:
        self._pending_values = values
        self._pending_selection = selected_value
        self.changed.emit(values)

    def _refresh_geometry(self) -> None:
        """Recompute the composite editor height after the list content changes."""

        self._list.refresh_height()
        layout = self.layout()
        if layout is not None:
            layout.invalidate()
            layout.activate()
        self.updateGeometry()


class _ConditionDialog(QDialog):
    """Collects a supported step condition in one committed action."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Add Condition")

        self._type_combo = create_expanding_combo_box()
        for condition_type in SUPPORTED_CONDITION_TYPES:
            self._type_combo.addItem(condition_type, condition_type)
        self._value_edit = create_expanding_line_edit()
        self._value_label = QLabel("Path")
        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._form, "Type", self._type_combo, prompt_tooltip("steps.condition.type"))
        add_tooltipped_form_row(self._form, self._value_label, self._value_edit, prompt_tooltip("steps.condition.target"))

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.addLayout(self._form)
        layout.addWidget(buttons)

        self._type_combo.currentIndexChanged.connect(self._update_value_label)
        self._update_value_label()

    def condition(self) -> StepCondition:
        condition_type = str(self._type_combo.currentData())
        field_name, _field_label = CONDITION_PARAM_FIELD[condition_type]
        return StepCondition(type=condition_type, params={field_name: self._value_edit.text()})

    def _update_value_label(self) -> None:
        condition_type = str(self._type_combo.currentData())
        _field_name, field_label = CONDITION_PARAM_FIELD[condition_type]
        self._value_label.setText(field_label)
        apply_tooltip(self._value_label, prompt_tooltip("steps.condition.target"))

    @classmethod
    def prompt(cls, parent: QWidget | None = None) -> StepCondition | None:
        dialog = cls(parent)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return dialog.condition()


class _ConditionListEditor(QWidget):
    """Structured ordered editor for supported skip_if and verify conditions."""

    changed = Signal(tuple)

    def __init__(self, *, title: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._title = title
        self._conditions: list[StepCondition] = []
        self._loading = False
        self._locked = False
        tooltip = field_tooltip(f"steps.{title}")

        self._list = AutoSizingListWidget()
        apply_tooltip(self, tooltip)
        apply_tooltip(self._list, tooltip)
        self._list.currentRowChanged.connect(self._load_selected_condition)
        self._add_button = QPushButton("Add")
        self._remove_button = QPushButton("Remove")
        self._up_button = QPushButton("Move Up")
        self._down_button = QPushButton("Move Down")

        button_row = QHBoxLayout()
        button_row.addWidget(self._add_button)
        button_row.addWidget(self._remove_button)
        button_row.addWidget(self._up_button)
        button_row.addWidget(self._down_button)

        self._type_combo = create_expanding_combo_box()
        for condition_type in SUPPORTED_CONDITION_TYPES:
            self._type_combo.addItem(condition_type, condition_type)
        self._value_label = QLabel("Path")
        self._value_edit = create_expanding_line_edit()
        detail_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(detail_form, "Type", self._type_combo, tooltip)
        add_tooltipped_form_row(detail_form, self._value_label, self._value_edit, tooltip)

        self._preserved_label = QLabel("Preserved unsupported conditions")
        self._preserved_view = AutoSizingPlainTextEdit()
        self._preserved_view.setReadOnly(True)
        apply_tooltip(self._preserved_label, field_tooltip("steps.preserved_content"))
        apply_tooltip(self._preserved_view, field_tooltip("steps.preserved_content"))
        self._preserved_label.hide()
        self._preserved_view.hide()

        layout = QVBoxLayout(self)
        layout.addWidget(self._list)
        layout.addLayout(button_row)
        layout.addLayout(detail_form)
        layout.addWidget(self._preserved_label)
        layout.addWidget(self._preserved_view)

        self._add_button.clicked.connect(self._add_condition)
        self._remove_button.clicked.connect(self._remove_condition)
        self._up_button.clicked.connect(self._move_up)
        self._down_button.clicked.connect(self._move_down)
        self._type_combo.currentIndexChanged.connect(self._commit_selected_condition)
        self._value_edit.editingFinished.connect(self._commit_selected_condition)
        self._refresh_button_state()

    def set_conditions(self, conditions: tuple[StepCondition, ...]) -> None:
        current_row = self._list.currentRow()
        self._conditions = list(conditions)
        self._locked = any(condition.type not in SUPPORTED_CONDITION_TYPES for condition in self._conditions)
        preserved = [condition for condition in self._conditions if condition.type not in SUPPORTED_CONDITION_TYPES]
        self._loading = True
        self._list.clear()
        for condition in self._conditions:
            item = QListWidgetItem(_condition_label(condition))
            item.setData(Qt.ItemDataRole.UserRole, condition)
            self._list.addItem(item)
        self._loading = False
        if self._list.count():
            target_row = min(max(current_row, 0), self._list.count() - 1)
            self._list.setCurrentRow(target_row)
        else:
            self._type_combo.setEnabled(False)
            self._value_edit.setEnabled(False)
            self._type_combo.setCurrentIndex(0)
            self._value_edit.clear()
        preserved_text = _yaml_block([_serialize_condition(condition) for condition in preserved])
        self._preserved_label.setVisible(bool(preserved_text))
        self._preserved_view.setVisible(bool(preserved_text))
        self._preserved_view.setPlainText(preserved_text)
        self._preserved_view.refresh_height()
        self._list.refresh_height()
        self._refresh_button_state()

    def conditions(self) -> tuple[StepCondition, ...]:
        return tuple(self._conditions)

    def _selected_condition(self) -> tuple[int, StepCondition] | tuple[None, None]:
        row = self._list.currentRow()
        if row < 0 or row >= len(self._conditions):
            return None, None
        return row, self._conditions[row]

    def _load_selected_condition(self) -> None:
        if self._loading:
            return
        row, condition = self._selected_condition()
        supported = condition is not None and condition.type in SUPPORTED_CONDITION_TYPES
        self._type_combo.setEnabled(bool(supported))
        self._value_edit.setEnabled(bool(supported))
        if condition is None or not supported:
            self._value_edit.clear()
            self._refresh_button_state()
            return
        field_name, field_label = CONDITION_PARAM_FIELD[condition.type]
        self._loading = True
        self._type_combo.setCurrentIndex(self._type_combo.findData(condition.type))
        self._value_label.setText(field_label)
        self._value_edit.setText(str(condition.params.get(field_name, "")))
        self._loading = False
        self._refresh_button_state()

    def _refresh_button_state(self) -> None:
        row, condition = self._selected_condition()
        has_selection = row is not None
        self._remove_button.setEnabled(has_selection and not self._locked)
        self._up_button.setEnabled(has_selection and not self._locked and row > 0)
        self._down_button.setEnabled(has_selection and not self._locked and row < len(self._conditions) - 1)
        self._add_button.setEnabled(True)
        if condition is not None and condition.type in SUPPORTED_CONDITION_TYPES:
            field_name, field_label = CONDITION_PARAM_FIELD[condition.type]
            self._value_label.setText(field_label)

    def _add_condition(self) -> None:
        condition = _ConditionDialog.prompt(self)
        if condition is None:
            return
        self.changed.emit(self.conditions() + (condition,))

    def _remove_condition(self) -> None:
        if self._locked:
            return
        row, _condition = self._selected_condition()
        if row is None:
            return
        updated = list(self._conditions)
        del updated[row]
        self.changed.emit(tuple(updated))

    def _move_up(self) -> None:
        if self._locked:
            return
        row, _condition = self._selected_condition()
        if row is None or row <= 0:
            return
        updated = list(self._conditions)
        updated[row - 1], updated[row] = updated[row], updated[row - 1]
        self.changed.emit(tuple(updated))

    def _move_down(self) -> None:
        if self._locked:
            return
        row, _condition = self._selected_condition()
        if row is None or row >= len(self._conditions) - 1:
            return
        updated = list(self._conditions)
        updated[row + 1], updated[row] = updated[row], updated[row + 1]
        self.changed.emit(tuple(updated))

    def _commit_selected_condition(self) -> None:
        if self._loading:
            return
        row, condition = self._selected_condition()
        if row is None or condition is None or condition.type not in SUPPORTED_CONDITION_TYPES:
            return
        condition_type = str(self._type_combo.currentData())
        field_name, _field_label = CONDITION_PARAM_FIELD[condition_type]
        updated = list(self._conditions)
        updated[row] = StepCondition(type=condition_type, params={field_name: self._value_edit.text()})
        self.changed.emit(tuple(updated))


class _NewStepDialog(QDialog):
    """Collects the minimum valid shape for a new authored step."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Add Step")

        self._type_combo = create_expanding_combo_box()
        for step_type in SUPPORTED_EDITOR_STEP_TYPES:
            self._type_combo.addItem(step_type.value, step_type)
        self._id_edit = create_expanding_line_edit()
        self._name_edit = create_expanding_line_edit()
        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._form, "Type", self._type_combo, prompt_tooltip("steps.type"))
        add_tooltipped_form_row(self._form, "Step id", self._id_edit, prompt_tooltip("steps.id"))
        add_tooltipped_form_row(self._form, "Name", self._name_edit, prompt_tooltip("steps.name"))

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.addLayout(self._form)
        layout.addWidget(buttons)

    def values(self) -> tuple[StepType, str, str]:
        return (
            self._type_combo.currentData(),
            self._id_edit.text(),
            self._name_edit.text(),
        )

    @classmethod
    def prompt(cls, parent: QWidget | None = None) -> tuple[StepType, str, str] | None:
        dialog = cls(parent)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return dialog.values()


class StepsPage(QWidget):
    """Edits authored recipe steps through structured document commands."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None
        self._selected_step_id: str | None = None
        self._pending_dependency_selection: str | None = None
        self._loading = False

        self._step_list = QListWidget()
        self._step_list.currentItemChanged.connect(self._on_selection_changed)
        self._add_step_button = QPushButton("Add")
        self._delete_step_button = QPushButton("Delete")
        self._duplicate_step_button = QPushButton("Duplicate")
        self._move_up_button = QPushButton("Move Up")
        self._move_down_button = QPushButton("Move Down")
        self._toggle_user_toggleable_button = QPushButton("Toggle User Toggleable")
        left_buttons = QHBoxLayout()
        for button in (
            self._add_step_button,
            self._delete_step_button,
            self._duplicate_step_button,
            self._move_up_button,
            self._move_down_button,
            self._toggle_user_toggleable_button,
        ):
            left_buttons.addWidget(button)
        left_panel = QWidget()
        left_layout = QVBoxLayout(left_panel)
        left_layout.addWidget(self._step_list)
        left_layout.addLayout(left_buttons)

        self._step_id_value = create_expanding_line_edit(read_only=True)
        self._step_type_value = create_expanding_line_edit(read_only=True)
        self._step_name_edit = create_expanding_line_edit()
        self._step_user_toggleable_check = QCheckBox()
        self._step_description_edit = CommitPlainTextEdit()
        expand_form_field(self._step_description_edit)
        self._basics_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._basics_form, "ID", self._step_id_value, field_tooltip("steps.id"))
        add_tooltipped_form_row(self._basics_form, "Type", self._step_type_value, field_tooltip("steps.type"))
        add_tooltipped_form_row(self._basics_form, "Name", self._step_name_edit, field_tooltip("steps.name"))
        add_tooltipped_form_row(
            self._basics_form,
            "User Toggleable",
            self._step_user_toggleable_check,
            field_tooltip("steps.user_toggleable"),
        )
        basics_section = QGroupBox("Basics")
        basics_layout = QVBoxLayout(basics_section)
        basics_layout.addLayout(self._basics_form)
        self._description_label = QLabel("Description")
        apply_tooltip(self._description_label, field_tooltip("steps.description"))
        apply_tooltip(self._step_description_edit, field_tooltip("steps.description"))
        basics_layout.addWidget(self._description_label)
        basics_layout.addWidget(self._step_description_edit)

        self._dependencies_list = AutoSizingListWidget()
        self._dependencies_list.currentRowChanged.connect(self._refresh_dependency_buttons)
        self._add_dependency_button = QPushButton("Add")
        self._remove_dependency_button = QPushButton("Remove")
        dependencies_section = QGroupBox("Dependencies")
        apply_tooltip(dependencies_section, field_tooltip("steps.dependencies"))
        apply_tooltip(self._dependencies_list, field_tooltip("steps.dependencies"))
        dependencies_layout = QVBoxLayout(dependencies_section)
        dependencies_layout.addWidget(self._dependencies_list)
        dependency_buttons = QHBoxLayout()
        dependency_buttons.addWidget(self._add_dependency_button)
        dependency_buttons.addWidget(self._remove_dependency_button)
        dependencies_layout.addLayout(dependency_buttons)

        self._params_stack = CurrentWidgetSizeStack()
        self._params_placeholder = QLabel("Select a step to edit params.")
        self._params_placeholder.setWordWrap(True)
        self._params_stack.addWidget(self._params_placeholder)
        self._params_stack_by_type: dict[StepType, QWidget] = {}
        self._build_param_panels()
        self._params_preserved_label = QLabel("Preserved unsupported params")
        self._params_preserved_view = AutoSizingPlainTextEdit()
        self._params_preserved_view.setReadOnly(True)
        apply_tooltip(self._params_preserved_label, field_tooltip("steps.preserved_content"))
        apply_tooltip(self._params_preserved_view, field_tooltip("steps.preserved_content"))
        self._params_preserved_label.hide()
        self._params_preserved_view.hide()
        params_section = QGroupBox("Params")
        self._params_section = params_section
        params_layout = QVBoxLayout(params_section)
        params_layout.addWidget(self._params_stack)
        params_layout.addWidget(self._params_preserved_label)
        params_layout.addWidget(self._params_preserved_view)

        self._capabilities_editor = _OrderedChoiceEditor(
            prompt_title="Add Capability",
            prompt_label="Capability",
            field_tooltip_text=field_tooltip("steps.constraints.capabilities"),
        )
        self._conflicts_editor = _OrderedChoiceEditor(
            prompt_title="Add Conflict",
            prompt_label="Step",
            field_tooltip_text=field_tooltip("steps.constraints.conflicts_with"),
        )
        self._capabilities_editor.changed.connect(self._commit_capabilities)
        self._conflicts_editor.changed.connect(self._commit_conflicts)
        self._skip_if_editor = _ConditionListEditor(title="skip_if")
        self._skip_if_editor.changed.connect(self._commit_skip_if)
        self._constraints_preserved_label = QLabel("Preserved unsupported constraints")
        self._constraints_preserved_view = AutoSizingPlainTextEdit()
        self._constraints_preserved_view.setReadOnly(True)
        apply_tooltip(self._constraints_preserved_label, field_tooltip("steps.preserved_content"))
        apply_tooltip(self._constraints_preserved_view, field_tooltip("steps.preserved_content"))
        self._constraints_preserved_label.hide()
        self._constraints_preserved_view.hide()
        constraints_section = QGroupBox("Constraints / Skip")
        constraints_layout = QVBoxLayout(constraints_section)
        self._capabilities_label = QLabel("Capabilities")
        apply_tooltip(self._capabilities_label, field_tooltip("steps.constraints.capabilities"))
        constraints_layout.addWidget(self._capabilities_label)
        constraints_layout.addWidget(self._capabilities_editor)
        self._conflicts_label = QLabel("Conflicts With")
        apply_tooltip(self._conflicts_label, field_tooltip("steps.constraints.conflicts_with"))
        constraints_layout.addWidget(self._conflicts_label)
        constraints_layout.addWidget(self._conflicts_editor)
        constraints_layout.addWidget(self._constraints_preserved_label)
        constraints_layout.addWidget(self._constraints_preserved_view)
        self._skip_if_label = QLabel("skip_if")
        apply_tooltip(self._skip_if_label, field_tooltip("steps.skip_if"))
        constraints_layout.addWidget(self._skip_if_label)
        constraints_layout.addWidget(self._skip_if_editor)

        self._verify_editor = _ConditionListEditor(title="verify")
        apply_tooltip(self._verify_editor, field_tooltip("steps.verify"))
        self._verify_editor.changed.connect(self._commit_verify)
        verify_section = QGroupBox("Verify")
        apply_tooltip(verify_section, field_tooltip("steps.verify"))
        verify_layout = QVBoxLayout(verify_section)
        verify_layout.addWidget(self._verify_editor)

        detail_host = QWidget()
        detail_layout = QVBoxLayout(detail_host)
        detail_layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        detail_layout.addWidget(basics_section)
        detail_layout.addWidget(dependencies_section)
        detail_layout.addWidget(params_section)
        detail_layout.addWidget(constraints_section)
        detail_layout.addWidget(verify_section)
        detail_layout.addStretch(1)

        self._detail_scroll = QScrollArea()
        self._detail_scroll.setWidgetResizable(True)
        self._detail_scroll.setWidget(detail_host)

        splitter = QSplitter()
        splitter.addWidget(left_panel)
        splitter.addWidget(self._detail_scroll)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 2)

        layout = QVBoxLayout(self)
        layout.addWidget(splitter)

        self._add_step_button.clicked.connect(self._add_step)
        self._delete_step_button.clicked.connect(self._delete_step)
        self._duplicate_step_button.clicked.connect(self._duplicate_step)
        self._move_up_button.clicked.connect(self._move_step_up)
        self._move_down_button.clicked.connect(self._move_step_down)
        self._toggle_user_toggleable_button.clicked.connect(self._toggle_user_toggleable)
        self._add_dependency_button.clicked.connect(self._add_dependency)
        self._remove_dependency_button.clicked.connect(self._remove_dependency)
        self._step_name_edit.editingFinished.connect(self._commit_basics)
        self._step_description_edit.committed.connect(self._commit_description)
        self._step_user_toggleable_check.toggled.connect(self._commit_user_toggleable)
        self._refresh_step_buttons()
        self._refresh_dependency_buttons()
        self._set_detail_enabled(False)

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        existing_selection = self._selected_step_id if any(step.id == self._selected_step_id for step in document.working_recipe.steps) else None
        self._loading = True
        self._step_list.clear()
        for step in document.working_recipe.steps:
            item = QListWidgetItem(f"{step.id} · {step.type.value}")
            item.setData(Qt.ItemDataRole.UserRole, step.id)
            self._step_list.addItem(item)
        self._loading = False
        if document.working_recipe.steps:
            self._select_step(existing_selection or document.working_recipe.steps[0].id)
        else:
            self._selected_step_id = None
            self._clear_detail()
        self._refresh_step_buttons()

    def _build_param_panels(self) -> None:
        self._resolve_artifacts_panel = QWidget()
        resolve_layout = QVBoxLayout(self._resolve_artifacts_panel)
        self._resolve_artifacts_editor = _OrderedChoiceEditor(
            prompt_title="Add Artifact",
            prompt_label="Artifact",
            field_tooltip_text=field_tooltip("steps.resolve_artifacts.artifacts"),
        )
        self._resolve_artifact_groups_editor = _OrderedChoiceEditor(
            prompt_title="Add Artifact Group",
            prompt_label="Group",
            field_tooltip_text=field_tooltip("steps.resolve_artifacts.artifact_groups"),
        )
        self._resolve_artifacts_editor.changed.connect(self._commit_step_params)
        self._resolve_artifact_groups_editor.changed.connect(self._commit_step_params)
        self._resolve_artifacts_label = QLabel("Artifacts")
        apply_tooltip(self._resolve_artifacts_label, field_tooltip("steps.resolve_artifacts.artifacts"))
        resolve_layout.addWidget(self._resolve_artifacts_label)
        resolve_layout.addWidget(self._resolve_artifacts_editor)
        self._resolve_artifact_groups_label = QLabel("Artifact Groups")
        apply_tooltip(self._resolve_artifact_groups_label, field_tooltip("steps.resolve_artifacts.artifact_groups"))
        resolve_layout.addWidget(self._resolve_artifact_groups_label)
        resolve_layout.addWidget(self._resolve_artifact_groups_editor)
        self._register_param_panel(StepType.RESOLVE_ARTIFACTS, self._resolve_artifacts_panel)

        self._extract_artifacts_panel = QWidget()
        extract_layout = QVBoxLayout(self._extract_artifacts_panel)
        self._extract_artifacts_editor = _OrderedChoiceEditor(
            prompt_title="Add Artifact",
            prompt_label="Artifact",
            field_tooltip_text=field_tooltip("steps.extract_artifacts.artifacts"),
        )
        self._extract_artifact_groups_editor = _OrderedChoiceEditor(
            prompt_title="Add Artifact Group",
            prompt_label="Group",
            field_tooltip_text=field_tooltip("steps.extract_artifacts.artifact_groups"),
        )
        self._extract_artifacts_extract_on_combo = create_expanding_combo_box()
        self._extract_artifacts_extract_on_combo.addItem("host", "host")
        self._extract_artifacts_extract_on_combo.addItem("device", "device")
        self._extract_artifacts_editor.changed.connect(self._commit_step_params)
        self._extract_artifact_groups_editor.changed.connect(self._commit_step_params)
        self._extract_artifacts_extract_on_combo.currentIndexChanged.connect(self._commit_step_params)
        self._extract_artifacts_label = QLabel("Artifacts")
        apply_tooltip(self._extract_artifacts_label, field_tooltip("steps.extract_artifacts.artifacts"))
        extract_layout.addWidget(self._extract_artifacts_label)
        extract_layout.addWidget(self._extract_artifacts_editor)
        self._extract_artifact_groups_label = QLabel("Artifact Groups")
        apply_tooltip(self._extract_artifact_groups_label, field_tooltip("steps.extract_artifacts.artifact_groups"))
        extract_layout.addWidget(self._extract_artifact_groups_label)
        extract_layout.addWidget(self._extract_artifact_groups_editor)
        extract_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            extract_form,
            "Extract On",
            self._extract_artifacts_extract_on_combo,
            field_tooltip("steps.extract_artifacts.extract_on"),
        )
        extract_layout.addLayout(extract_form)
        self._register_param_panel(StepType.EXTRACT_ARTIFACTS, self._extract_artifacts_panel)

        self._extract_archive_panel = QWidget()
        extract_archive_layout = QVBoxLayout(self._extract_archive_panel)
        self._extract_archive_archive_combo = create_expanding_combo_box()
        self._extract_archive_extract_on_combo = create_expanding_combo_box()
        self._extract_archive_extract_on_combo.addItem("host", "host")
        self._extract_archive_extract_on_combo.addItem("device", "device")
        self._extract_archive_dest_edit = create_expanding_line_edit()
        self._extract_archive_device_temp_path_edit = create_expanding_line_edit()
        self._extract_archive_cleanup_check = QCheckBox()
        self._extract_archive_dest_label = QLabel("Dest")
        self._extract_archive_temp_label = QLabel("Device Temp Path")
        extract_archive_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            extract_archive_form,
            "Archive",
            self._extract_archive_archive_combo,
            field_tooltip("steps.extract_archive.archive"),
        )
        add_tooltipped_form_row(
            extract_archive_form,
            "Extract On",
            self._extract_archive_extract_on_combo,
            field_tooltip("steps.extract_archive.extract_on"),
        )
        add_tooltipped_form_row(
            extract_archive_form,
            self._extract_archive_dest_label,
            self._extract_archive_dest_edit,
            field_tooltip("steps.extract_archive.dest"),
        )
        add_tooltipped_form_row(
            extract_archive_form,
            self._extract_archive_temp_label,
            self._extract_archive_device_temp_path_edit,
            field_tooltip("steps.extract_archive.device_temp_path"),
        )
        add_tooltipped_form_row(
            extract_archive_form,
            "Cleanup",
            self._extract_archive_cleanup_check,
            field_tooltip("steps.extract_archive.cleanup"),
        )
        extract_archive_layout.addLayout(extract_archive_form)
        self._extract_archive_archive_combo.currentIndexChanged.connect(self._commit_step_params)
        self._extract_archive_extract_on_combo.currentIndexChanged.connect(self._on_extract_archive_location_changed)
        self._extract_archive_dest_edit.editingFinished.connect(self._commit_step_params)
        self._extract_archive_device_temp_path_edit.editingFinished.connect(self._commit_step_params)
        self._extract_archive_cleanup_check.toggled.connect(self._commit_step_params)
        self._register_param_panel(StepType.EXTRACT_ARCHIVE, self._extract_archive_panel)

        self._copy_files_panel = QWidget()
        copy_layout = QVBoxLayout(self._copy_files_panel)
        self._copy_source_combo = create_expanding_combo_box()
        self._copy_dest_edit = create_expanding_line_edit()
        self._copy_policy_combo = create_expanding_combo_box()
        self._copy_policy_combo.addItem("merge", "merge")
        self._copy_policy_combo.addItem("sync", "sync")
        self._copy_help_label = QLabel(COPY_FILES_HELP)
        self._copy_help_label.setWordWrap(True)
        copy_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(copy_form, "Source", self._copy_source_combo, field_tooltip("steps.copy_files.source"))
        add_tooltipped_form_row(copy_form, "Dest", self._copy_dest_edit, field_tooltip("steps.copy_files.dest"))
        add_tooltipped_form_row(
            copy_form,
            "Copy Policy",
            self._copy_policy_combo,
            field_tooltip("steps.copy_files.copy_policy"),
        )
        copy_layout.addLayout(copy_form)
        apply_tooltip(self._copy_help_label, field_tooltip("steps.copy_files.dest"))
        copy_layout.addWidget(self._copy_help_label)
        self._copy_source_combo.currentIndexChanged.connect(self._commit_step_params)
        self._copy_dest_edit.editingFinished.connect(self._commit_step_params)
        self._copy_policy_combo.currentIndexChanged.connect(self._commit_step_params)
        self._register_param_panel(StepType.COPY_FILES, self._copy_files_panel)

        self._install_apk_panel = QWidget()
        install_layout = QVBoxLayout(self._install_apk_panel)
        self._install_apk_app_combo = create_expanding_combo_box()
        self._install_apk_replace_existing_check = QCheckBox()
        install_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            install_form,
            "App",
            self._install_apk_app_combo,
            field_tooltip("steps.install_apk.app"),
        )
        add_tooltipped_form_row(
            install_form,
            "Replace Existing",
            self._install_apk_replace_existing_check,
            field_tooltip("steps.install_apk.replace_existing"),
        )
        install_layout.addLayout(install_form)
        self._install_apk_app_combo.currentIndexChanged.connect(self._commit_step_params)
        self._install_apk_replace_existing_check.toggled.connect(self._commit_step_params)
        self._register_param_panel(StepType.INSTALL_APK, self._install_apk_panel)

        self._grant_permissions_panel = QWidget()
        grant_layout = QVBoxLayout(self._grant_permissions_panel)
        self._grant_permissions_note_label = QLabel(GRANT_PERMISSIONS_NOTE)
        self._grant_permissions_note_label.setWordWrap(True)
        apply_tooltip(self._grant_permissions_note_label, field_tooltip("steps.grant_permissions.note"))
        grant_layout.addWidget(self._grant_permissions_note_label)
        self._register_param_panel(StepType.GRANT_PERMISSIONS, self._grant_permissions_panel)

        self._launch_app_panel = QWidget()
        launch_layout = QVBoxLayout(self._launch_app_panel)
        self._launch_package_edit = create_expanding_line_edit()
        self._launch_activity_edit = create_expanding_line_edit()
        launch_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            launch_form,
            "Package Name",
            self._launch_package_edit,
            field_tooltip("steps.launch_app.package_name"),
        )
        add_tooltipped_form_row(
            launch_form,
            "Activity",
            self._launch_activity_edit,
            field_tooltip("steps.launch_app.activity"),
        )
        launch_layout.addLayout(launch_form)
        self._launch_package_edit.editingFinished.connect(self._commit_step_params)
        self._launch_activity_edit.editingFinished.connect(self._commit_step_params)
        self._register_param_panel(StepType.LAUNCH_APP, self._launch_app_panel)

        self._wait_panel = QWidget()
        wait_layout = QVBoxLayout(self._wait_panel)
        self._wait_duration_spin = QSpinBox()
        self._wait_duration_spin.setMinimum(0)
        self._wait_duration_spin.setMaximum(2_147_483_647)
        expand_form_field(self._wait_duration_spin)
        wait_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            wait_form,
            "Duration (ms)",
            self._wait_duration_spin,
            field_tooltip("steps.wait.duration_ms"),
        )
        wait_layout.addLayout(wait_form)
        self._wait_duration_spin.valueChanged.connect(self._commit_step_params)
        self._register_param_panel(StepType.WAIT, self._wait_panel)

        self._force_stop_panel = QWidget()
        force_stop_layout = QVBoxLayout(self._force_stop_panel)
        self._force_stop_package_edit = create_expanding_line_edit()
        force_stop_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            force_stop_form,
            "Package Name",
            self._force_stop_package_edit,
            field_tooltip("steps.force_stop_app.package_name"),
        )
        force_stop_layout.addLayout(force_stop_form)
        self._force_stop_package_edit.editingFinished.connect(self._commit_step_params)
        self._register_param_panel(StepType.FORCE_STOP_APP, self._force_stop_panel)

    def _register_param_panel(self, step_type: StepType, panel: QWidget) -> None:
        self._params_stack_by_type[step_type] = panel
        self._params_stack.addWidget(panel)

    def _on_selection_changed(self, current: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        if self._loading:
            return
        self._selected_step_id = current.data(Qt.ItemDataRole.UserRole) if current is not None else None
        self._load_selected_step()

    def _select_step(self, step_id: str) -> None:
        for row in range(self._step_list.count()):
            item = self._step_list.item(row)
            if item.data(Qt.ItemDataRole.UserRole) == step_id:
                self._step_list.setCurrentRow(row)
                return

    def _selected_step(self) -> Step | None:
        if self._document is None or self._selected_step_id is None:
            return None
        for step in self._document.working_recipe.steps:
            if step.id == self._selected_step_id:
                return step
        return None

    def _load_selected_step(self) -> None:
        step = self._selected_step()
        if step is None:
            self._clear_detail()
            return
        self._loading = True
        self._step_id_value.setText(step.id)
        self._step_type_value.setText(step.type.value)
        self._step_name_edit.setText(step.name)
        self._step_description_edit.set_committed_text(step.description or "")
        self._step_user_toggleable_check.setChecked(step.user_toggleable)
        self._populate_dependencies(step)
        self._populate_param_panel(step)
        self._populate_constraints(step)
        self._skip_if_editor.set_conditions(step.skip_if)
        self._verify_editor.set_conditions(step.verify)
        self._loading = False
        self._set_detail_enabled(True)
        self._refresh_step_buttons()

    def _populate_dependencies(self, step: Step) -> None:
        current_dependency = (
            self._pending_dependency_selection
            if self._pending_dependency_selection is not None
            else self._current_dependency()
        )
        self._pending_dependency_selection = None
        self._dependencies_list.clear()
        for dependency_id in step.dependencies:
            item = QListWidgetItem(dependency_id)
            item.setData(Qt.ItemDataRole.UserRole, dependency_id)
            item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsUserCheckable)
            self._dependencies_list.addItem(item)
        if step.dependencies:
            target_dependency = current_dependency if current_dependency in step.dependencies else step.dependencies[0]
            for row in range(self._dependencies_list.count()):
                if self._dependencies_list.item(row).data(Qt.ItemDataRole.UserRole) == target_dependency:
                    self._dependencies_list.setCurrentRow(row)
                    break
        self._dependencies_list.refresh_height()
        self._refresh_dependency_buttons()

    def _populate_param_panel(self, step: Step) -> None:
        panel = self._params_stack_by_type.get(step.type, self._params_placeholder)
        self._params_stack.setCurrentWidget(panel)
        supported_names = set(STEP_SPECS[step.type].params)
        preserved_params = {name: value for name, value in step.params.items() if name not in supported_names}
        preserved_text = _yaml_block(_serialize_param_mapping(preserved_params))
        self._params_preserved_label.setVisible(bool(preserved_text))
        self._params_preserved_view.setVisible(bool(preserved_text))
        self._params_preserved_view.setPlainText(preserved_text)
        self._params_preserved_view.refresh_height()

        artifact_choices = tuple((artifact_id, artifact_id) for artifact_id in self._document.working_recipe.artifacts) if self._document is not None else ()
        group_choices = tuple((group_id, group_id) for group_id in self._document.working_recipe.artifact_groups) if self._document is not None else ()

        if step.type is StepType.RESOLVE_ARTIFACTS:
            self._resolve_artifacts_editor.set_state(tuple(_coerce_string_list(step.params.get("artifacts"))), choices=artifact_choices)
            self._resolve_artifact_groups_editor.set_state(tuple(_coerce_string_list(step.params.get("artifact_groups"))), choices=group_choices)
        elif step.type is StepType.EXTRACT_ARTIFACTS:
            self._extract_artifacts_editor.set_state(tuple(_coerce_string_list(step.params.get("artifacts"))), choices=artifact_choices)
            self._extract_artifact_groups_editor.set_state(tuple(_coerce_string_list(step.params.get("artifact_groups"))), choices=group_choices)
            self._extract_artifacts_extract_on_combo.setCurrentIndex(
                self._extract_artifacts_extract_on_combo.findData(str(step.params.get("extract_on", "host")))
            )
        elif step.type is StepType.EXTRACT_ARCHIVE:
            self._populate_ref_combo(
                self._extract_archive_archive_combo,
                step.params.get("archive"),
                self._ref_candidates(step.type, "archive"),
            )
            extract_on = str(step.params.get("extract_on", "host"))
            self._extract_archive_extract_on_combo.setCurrentIndex(self._extract_archive_extract_on_combo.findData(extract_on))
            self._extract_archive_dest_edit.setText(str(step.params.get("dest", "")))
            self._extract_archive_device_temp_path_edit.setText(str(step.params.get("device_temp_path", "")))
            self._extract_archive_cleanup_check.setChecked(bool(step.params.get("cleanup", True)))
            self._update_extract_archive_device_fields(extract_on == "device")
        elif step.type is StepType.COPY_FILES:
            self._populate_ref_combo(self._copy_source_combo, step.params.get("source"), self._ref_candidates(step.type, "source"))
            self._copy_dest_edit.setText(str(step.params.get("dest", "")))
            self._copy_policy_combo.setCurrentIndex(self._copy_policy_combo.findData(str(step.params.get("copy_policy", "merge"))))
        elif step.type is StepType.INSTALL_APK:
            self._populate_ref_combo(self._install_apk_app_combo, step.params.get("app"), self._ref_candidates(step.type, "app"))
            self._install_apk_replace_existing_check.setChecked(bool(step.params.get("replace_existing", False)))
        elif step.type is StepType.LAUNCH_APP:
            self._launch_package_edit.setText(str(step.params.get("package_name", "")))
            self._launch_activity_edit.setText(str(step.params.get("activity", "")))
        elif step.type is StepType.WAIT:
            self._wait_duration_spin.setValue(int(step.params.get("duration_ms", 0) or 0))
        elif step.type is StepType.FORCE_STOP_APP:
            self._force_stop_package_edit.setText(str(step.params.get("package_name", "")))
        self._refresh_params_section_height()

    def _populate_constraints(self, step: Step) -> None:
        other_step_ids = tuple(candidate.id for candidate in self._document.working_recipe.steps if candidate.id != step.id) if self._document is not None else ()
        capabilities = tuple(step.constraints.capabilities)
        conflicts = tuple(step.constraints.conflicts_with)
        unsupported_capabilities = tuple(value for value in capabilities if value not in KNOWN_CAPABILITIES)
        unsupported_conflicts = tuple(value for value in conflicts if value not in other_step_ids)
        self._capabilities_editor.set_state(
            capabilities,
            choices=tuple((value, value) for value in KNOWN_CAPABILITIES),
            locked=bool(unsupported_capabilities),
        )
        self._conflicts_editor.set_state(
            conflicts,
            choices=tuple((value, value) for value in other_step_ids),
            locked=bool(unsupported_conflicts),
        )
        preserved: dict[str, list[str]] = {}
        if unsupported_capabilities:
            preserved["capabilities"] = list(unsupported_capabilities)
        if unsupported_conflicts:
            preserved["conflicts_with"] = list(unsupported_conflicts)
        preserved_text = _yaml_block(preserved)
        self._constraints_preserved_label.setVisible(bool(preserved_text))
        self._constraints_preserved_view.setVisible(bool(preserved_text))
        self._constraints_preserved_view.setPlainText(preserved_text)
        self._constraints_preserved_view.refresh_height()

    def _clear_detail(self) -> None:
        self._loading = True
        self._step_id_value.clear()
        self._step_type_value.clear()
        self._step_name_edit.clear()
        self._step_description_edit.set_committed_text("")
        self._step_user_toggleable_check.setChecked(False)
        self._dependencies_list.clear()
        self._pending_dependency_selection = None
        self._params_stack.setCurrentWidget(self._params_placeholder)
        self._params_preserved_label.hide()
        self._params_preserved_view.hide()
        self._params_preserved_view.clear()
        self._params_preserved_view.refresh_height()
        self._constraints_preserved_label.hide()
        self._constraints_preserved_view.hide()
        self._constraints_preserved_view.clear()
        self._constraints_preserved_view.refresh_height()
        self._skip_if_editor.set_conditions(())
        self._verify_editor.set_conditions(())
        self._loading = False
        self._dependencies_list.refresh_height()
        self._refresh_dependency_buttons()
        self._refresh_params_section_height()
        self._set_detail_enabled(False)
        self._refresh_step_buttons()

    def _set_detail_enabled(self, enabled: bool) -> None:
        self._detail_scroll.setEnabled(enabled)

    def _refresh_step_buttons(self) -> None:
        has_selection = self._selected_step() is not None
        selected_row = self._step_list.currentRow()
        self._delete_step_button.setEnabled(has_selection)
        self._duplicate_step_button.setEnabled(has_selection)
        self._move_up_button.setEnabled(has_selection and selected_row > 0)
        self._move_down_button.setEnabled(has_selection and selected_row >= 0 and selected_row < self._step_list.count() - 1)
        self._toggle_user_toggleable_button.setEnabled(has_selection)
        self._refresh_dependency_buttons()

    def _current_dependency(self) -> str | None:
        item = self._dependencies_list.currentItem()
        return item.data(Qt.ItemDataRole.UserRole) if item is not None else None

    def _available_dependency_ids(self, step: Step | None = None) -> tuple[str, ...]:
        target_step = self._selected_step() if step is None else step
        if target_step is None or self._document is None:
            return ()
        return tuple(
            candidate.id
            for candidate in self._document.working_recipe.steps
            if candidate.id != target_step.id and candidate.id not in target_step.dependencies
        )

    def _refresh_dependency_buttons(self) -> None:
        step = self._selected_step()
        has_selection = step is not None and self._dependencies_list.currentRow() >= 0
        self._remove_dependency_button.setEnabled(has_selection)
        self._add_dependency_button.setEnabled(bool(self._available_dependency_ids(step)))

    def _add_dependency(self) -> None:
        step = self._selected_step()
        if step is None:
            return
        available_dependencies = self._available_dependency_ids(step)
        if not available_dependencies:
            return
        selected_dependency = self._prompt_for_dependency(available_dependencies)
        if selected_dependency is None:
            return
        self._pending_dependency_selection = selected_dependency
        self._command_handler(
            UpdateStepDependenciesCommand(
                step_id=step.id,
                dependencies=step.dependencies + (selected_dependency,),
            )
        )

    def _remove_dependency(self) -> None:
        step = self._selected_step()
        row = self._dependencies_list.currentRow()
        if step is None or row < 0:
            return
        remaining = tuple(dependency_id for index, dependency_id in enumerate(step.dependencies) if index != row)
        self._pending_dependency_selection = remaining[min(row, len(remaining) - 1)] if remaining else None
        self._command_handler(UpdateStepDependenciesCommand(step_id=step.id, dependencies=remaining))

    def _refresh_params_section_height(self) -> None:
        current_panel = self._params_stack.currentWidget()
        if current_panel is not None:
            panel_layout = current_panel.layout()
            if panel_layout is not None:
                panel_layout.invalidate()
                panel_layout.activate()
            current_panel.adjustSize()
            current_panel.updateGeometry()
        self._params_stack.updateGeometry()
        self._params_stack.setFixedHeight(self._params_stack.sizeHint().height())
        self._params_preserved_view.refresh_height()
        self._params_section.adjustSize()
        detail_host = self._detail_scroll.widget()
        if detail_host is not None:
            detail_host.adjustSize()
            detail_host.updateGeometry()

    def _prompt_for_dependency(self, available_dependencies: tuple[str, ...]) -> str | None:
        menu = QMenu(self)
        actions = {menu.addAction(dependency_id): dependency_id for dependency_id in available_dependencies}
        selected_action = menu.exec(self._add_dependency_button.mapToGlobal(self._add_dependency_button.rect().bottomLeft()))
        return actions.get(selected_action)

    def _add_step(self) -> None:
        values = self._prompt_for_new_step()
        if values is None:
            return
        step_type, step_id, name = values
        selected_row = self._step_list.currentRow()
        index = selected_row + 1 if selected_row >= 0 else None
        previous_selection = self._selected_step_id
        self._selected_step_id = step_id
        if not self._command_handler(AddStepCommand(step_id=step_id, step_type=step_type, name=name, index=index)):
            self._selected_step_id = previous_selection

    def _delete_step(self) -> None:
        step = self._selected_step()
        if step is None or not self._confirm_delete_step(step):
            return
        current_row = self._step_list.currentRow()
        remaining = [candidate.id for candidate in self._document.working_recipe.steps if candidate.id != step.id] if self._document is not None else []
        self._selected_step_id = remaining[min(current_row, len(remaining) - 1)] if remaining else None
        if not self._command_handler(DeleteStepCommand(step_id=step.id)):
            self._selected_step_id = step.id

    def _duplicate_step(self) -> None:
        step = self._selected_step()
        if step is None:
            return
        new_step_id = TextEntryDialog.prompt(
            self,
            title="Duplicate Step",
            label="New step id",
            tooltip=prompt_tooltip("steps.id"),
        )
        if new_step_id is None:
            return
        previous_selection = self._selected_step_id
        self._selected_step_id = new_step_id
        if not self._command_handler(DuplicateStepCommand(source_step_id=step.id, new_step_id=new_step_id)):
            self._selected_step_id = previous_selection

    def _move_step_up(self) -> None:
        step = self._selected_step()
        row = self._step_list.currentRow()
        if step is None or row <= 0:
            return
        self._command_handler(ReorderStepCommand(step_id=step.id, to_index=row - 1))

    def _move_step_down(self) -> None:
        step = self._selected_step()
        row = self._step_list.currentRow()
        if step is None or row < 0 or row >= self._step_list.count() - 1:
            return
        self._command_handler(ReorderStepCommand(step_id=step.id, to_index=row + 1))

    def _toggle_user_toggleable(self) -> None:
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(
            SetStepUserToggleableCommand(step_id=step.id, user_toggleable=not step.user_toggleable)
        )

    def _commit_basics(self) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(
            UpdateStepBasicsCommand(
                step_id=step.id,
                name=self._step_name_edit.text(),
                description=self._step_description_edit.toPlainText(),
            )
        )

    def _commit_description(self, _value: str) -> None:
        self._commit_basics()

    def _commit_user_toggleable(self, value: bool) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(SetStepUserToggleableCommand(step_id=step.id, user_toggleable=value))

    def _commit_step_params(self) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(
            UpdateStepParamsCommand(step_id=step.id, params=self._build_supported_params(step))
        )

    def _commit_capabilities(self, capabilities: tuple[str, ...]) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(
            UpdateStepConstraintsCommand(
                step_id=step.id,
                constraints=StepConstraints(
                    capabilities=capabilities,
                    conflicts_with=self._conflicts_editor.values(),
                ),
            )
        )

    def _commit_conflicts(self, conflicts_with: tuple[str, ...]) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(
            UpdateStepConstraintsCommand(
                step_id=step.id,
                constraints=StepConstraints(
                    capabilities=self._capabilities_editor.values(),
                    conflicts_with=conflicts_with,
                ),
            )
        )

    def _commit_skip_if(self, conditions: tuple[StepCondition, ...]) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(UpdateStepSkipIfCommand(step_id=step.id, skip_if=conditions))

    def _commit_verify(self, conditions: tuple[StepCondition, ...]) -> None:
        if self._loading:
            return
        step = self._selected_step()
        if step is None:
            return
        self._command_handler(UpdateStepVerifyCommand(step_id=step.id, verify=conditions))

    def _build_supported_params(self, step: Step) -> dict[str, AuthoredParamValue]:
        spec = STEP_SPECS[step.type]
        params: dict[str, AuthoredParamValue] = {
            name: value for name, value in step.params.items() if name not in spec.params
        }
        if step.type is StepType.RESOLVE_ARTIFACTS:
            _set_non_empty_list_param(params, "artifacts", self._resolve_artifacts_editor.values())
            _set_non_empty_list_param(params, "artifact_groups", self._resolve_artifact_groups_editor.values())
            return params
        if step.type is StepType.EXTRACT_ARTIFACTS:
            _set_non_empty_list_param(params, "artifacts", self._extract_artifacts_editor.values())
            _set_non_empty_list_param(params, "artifact_groups", self._extract_artifact_groups_editor.values())
            params["extract_on"] = str(self._extract_artifacts_extract_on_combo.currentData())
            return params
        if step.type is StepType.EXTRACT_ARCHIVE:
            archive_ref = _combo_ref(self._extract_archive_archive_combo)
            if archive_ref is not None:
                params["archive"] = RefParamValue(ref=archive_ref)
            extract_on = str(self._extract_archive_extract_on_combo.currentData())
            params["extract_on"] = extract_on
            if extract_on == "device":
                if self._extract_archive_dest_edit.text().strip():
                    params["dest"] = self._extract_archive_dest_edit.text()
                if self._extract_archive_device_temp_path_edit.text().strip():
                    params["device_temp_path"] = self._extract_archive_device_temp_path_edit.text()
            params["cleanup"] = self._extract_archive_cleanup_check.isChecked()
            return params
        if step.type is StepType.COPY_FILES:
            source_ref = _combo_ref(self._copy_source_combo)
            if source_ref is not None:
                params["source"] = RefParamValue(ref=source_ref)
            if self._copy_dest_edit.text().strip():
                params["dest"] = self._copy_dest_edit.text()
            params["copy_policy"] = str(self._copy_policy_combo.currentData())
            return params
        if step.type is StepType.INSTALL_APK:
            app_ref = _combo_ref(self._install_apk_app_combo)
            if app_ref is not None:
                params["app"] = RefParamValue(ref=app_ref)
            params["replace_existing"] = self._install_apk_replace_existing_check.isChecked()
            return params
        if step.type is StepType.GRANT_PERMISSIONS:
            return params
        if step.type is StepType.LAUNCH_APP:
            if self._launch_package_edit.text().strip():
                params["package_name"] = self._launch_package_edit.text()
            if self._launch_activity_edit.text().strip():
                params["activity"] = self._launch_activity_edit.text()
            return params
        if step.type is StepType.WAIT:
            params["duration_ms"] = self._wait_duration_spin.value()
            return params
        if step.type is StepType.FORCE_STOP_APP:
            if self._force_stop_package_edit.text().strip():
                params["package_name"] = self._force_stop_package_edit.text()
        return params

    def _ref_candidates(self, step_type: StepType, param_name: str) -> tuple[tuple[str, str], ...]:
        if self._document is None:
            return ()
        allowed_types = REF_VALUE_FILTERS.get((step_type, param_name), ())
        choices: list[tuple[str, str]] = []
        for candidate in self._document.ref_index.candidates:
            if candidate.value_type not in allowed_types:
                continue
            choices.append((candidate.ref, candidate.label))
            if candidate.source_kind == "step_output":
                choices.append((candidate.ref, f"Step · {candidate.source_id} (primary output)"))
        return tuple(choices)

    def _populate_ref_combo(
        self,
        combo: QComboBox,
        current_value: AuthoredParamValue | None,
        choices: tuple[tuple[str, str], ...],
    ) -> None:
        current_ref = current_value.ref if isinstance(current_value, RefParamValue) else None
        seen_refs: set[str] = set()
        combo.blockSignals(True)
        combo.clear()
        combo.addItem("(none)", None)
        for ref, label in choices:
            if ref in seen_refs:
                continue
            combo.addItem(label, ref)
            seen_refs.add(ref)
        if current_ref is not None and current_ref not in seen_refs:
            combo.insertItem(1, f"[Unresolved] {current_ref}", current_ref)
        target_index = combo.findData(current_ref)
        combo.setCurrentIndex(target_index if target_index >= 0 else 0)
        combo.blockSignals(False)

    def _on_extract_archive_location_changed(self) -> None:
        if self._loading:
            return
        extract_on = str(self._extract_archive_extract_on_combo.currentData())
        self._update_extract_archive_device_fields(extract_on == "device")
        self._refresh_params_section_height()
        self._commit_step_params()

    def _update_extract_archive_device_fields(self, visible: bool) -> None:
        self._extract_archive_dest_edit.setVisible(visible)
        self._extract_archive_device_temp_path_edit.setVisible(visible)
        self._extract_archive_dest_label.setVisible(visible)
        self._extract_archive_temp_label.setVisible(visible)
        self._extract_archive_panel.adjustSize()
        self._extract_archive_panel.updateGeometry()

    def _prompt_for_new_step(self) -> tuple[StepType, str, str] | None:
        return _NewStepDialog.prompt(self)

    def _confirm_delete_step(self, step: Step) -> bool:
        response = QMessageBox.question(
            self,
            "Delete Step?",
            (
                f"Delete step {step.id!r}? Downstream refs and dependencies are not rewritten. "
                "Live diagnostics will surface any resulting breakage."
            ),
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        return response == QMessageBox.StandardButton.Yes


def _serialize_condition(condition: StepCondition) -> dict[str, object]:
    return {
        "type": condition.type,
        "params": dict(condition.params),
    }


def _serialize_param_mapping(params: Mapping[str, AuthoredParamValue]) -> dict[str, object]:
    serialized: dict[str, object] = {}
    for name, value in params.items():
        if isinstance(value, RefParamValue):
            serialized[name] = {"ref": value.ref}
        else:
            serialized[name] = value
    return serialized


def _coerce_string_list(value: object) -> tuple[str, ...]:
    if isinstance(value, (list, tuple)):
        return tuple(str(item) for item in value)
    return ()


def _set_non_empty_list_param(params: dict[str, AuthoredParamValue], key: str, values: tuple[str, ...]) -> None:
    if values:
        params[key] = list(values)


def _combo_ref(combo: QComboBox) -> str | None:
    value = combo.currentData()
    return str(value) if value not in (None, "") else None

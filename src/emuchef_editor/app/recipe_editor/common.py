"""Shared widgets for recipe editor pages."""

from __future__ import annotations

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLineEdit,
    QListWidget,
    QSizePolicy,
    QPushButton,
    QPlainTextEdit,
    QVBoxLayout,
    QWidget,
)


class CommitPlainTextEdit(QPlainTextEdit):
    """Plain-text editor that emits a commit event on focus loss."""

    committed = Signal(str)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._committed_text = ""

    def set_committed_text(self, text: str) -> None:
        self._committed_text = text
        self.setPlainText(text)

    def focusOutEvent(self, event) -> None:  # type: ignore[override]
        current_text = self.toPlainText()
        super().focusOutEvent(event)
        if current_text != self._committed_text:
            self.committed.emit(current_text)


def configure_data_entry_form(form: QFormLayout) -> QFormLayout:
    """Apply the shared alignment and growth policy for recipe editor forms."""

    form.setFieldGrowthPolicy(QFormLayout.FieldGrowthPolicy.AllNonFixedFieldsGrow)
    form.setFormAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
    form.setLabelAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
    return form


def expand_form_field(widget: QWidget) -> QWidget:
    """Ensure a form field grows to the available horizontal space."""

    widget.setSizePolicy(QSizePolicy.Policy.Expanding, widget.sizePolicy().verticalPolicy())
    return widget


def apply_tooltip(widget: QWidget, tooltip: str) -> QWidget:
    """Attach shared tooltip copy to a widget."""

    widget.setToolTip(tooltip)
    return widget


def add_tooltipped_form_row(
    form: QFormLayout,
    label: str | QWidget,
    field: QWidget,
    tooltip: str,
) -> None:
    """Add a form row and apply the same tooltip to its label and field."""

    apply_tooltip(field, tooltip)
    if isinstance(label, QWidget):
        apply_tooltip(label, tooltip)
    form.addRow(label, field)
    label_widget = form.labelForField(field)
    if label_widget is not None:
        apply_tooltip(label_widget, tooltip)


def create_expanding_line_edit(*, read_only: bool = False) -> QLineEdit:
    """Create a line edit that fills the form field column."""

    line_edit = QLineEdit()
    line_edit.setReadOnly(read_only)
    expand_form_field(line_edit)
    return line_edit


def create_expanding_combo_box() -> QComboBox:
    """Create a combo box that fills the form field column."""

    combo_box = QComboBox()
    expand_form_field(combo_box)
    return combo_box


class TextEntryDialog(QDialog):
    """Small modal dialog for single-value editor prompts with tooltip support."""

    def __init__(
        self,
        *,
        title: str,
        label: str,
        tooltip: str,
        initial_value: str = "",
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setWindowTitle(title)

        self._form = configure_data_entry_form(QFormLayout())
        self._value_edit = create_expanding_line_edit()
        self._value_edit.setText(initial_value)
        add_tooltipped_form_row(self._form, label, self._value_edit, tooltip)

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.addLayout(self._form)
        layout.addWidget(buttons)

    @property
    def value_edit(self) -> QLineEdit:
        """Expose the prompt field for tests and call sites that need direct access."""

        return self._value_edit

    def value(self) -> str:
        """Return the current dialog field value."""

        return self._value_edit.text()

    @classmethod
    def prompt(
        cls,
        parent: QWidget | None,
        *,
        title: str,
        label: str,
        tooltip: str,
        initial_value: str = "",
    ) -> str | None:
        """Show the prompt dialog and return the entered value when accepted."""

        dialog = cls(
            title=title,
            label=label,
            tooltip=tooltip,
            initial_value=initial_value,
            parent=parent,
        )
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return dialog.value()


class OrderedStringListEditor(QWidget):
    """Structured editor for an ordered list of strings."""

    add_requested = Signal(str)
    update_requested = Signal(int, str)
    remove_requested = Signal(int)
    move_requested = Signal(int, int)

    def __init__(
        self,
        *,
        prompt_title: str,
        prompt_label: str,
        prompt_tooltip: str = "",
        field_tooltip: str = "",
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._prompt_title = prompt_title
        self._prompt_label = prompt_label
        self._prompt_tooltip = prompt_tooltip
        self._loading = False

        self._list = QListWidget()
        self._list.currentRowChanged.connect(self._on_selection_changed)

        self._add_button = QPushButton("Add")
        self._remove_button = QPushButton("Remove")
        self._up_button = QPushButton("Move Up")
        self._down_button = QPushButton("Move Down")
        self._value_edit = QLineEdit()
        self._value_edit.setEnabled(False)
        self._value_edit.editingFinished.connect(self._commit_selected_value)
        if field_tooltip:
            apply_tooltip(self._value_edit, field_tooltip)

        button_row = QHBoxLayout()
        button_row.addWidget(self._add_button)
        button_row.addWidget(self._remove_button)
        button_row.addWidget(self._up_button)
        button_row.addWidget(self._down_button)

        layout = QVBoxLayout(self)
        layout.addWidget(self._list)
        layout.addLayout(button_row)
        layout.addWidget(self._value_edit)

        self._add_button.clicked.connect(self._add_item)
        self._remove_button.clicked.connect(self._remove_item)
        self._up_button.clicked.connect(self._move_up)
        self._down_button.clicked.connect(self._move_down)
        self._refresh_button_state()

    def set_items(self, items: tuple[str, ...]) -> None:
        previous_row = self._list.currentRow()
        self._loading = True
        self._list.clear()
        for item in items:
            self._list.addItem(item)
        self._loading = False

        if items:
            target_row = min(max(previous_row, 0), len(items) - 1)
            self._list.setCurrentRow(target_row)
        else:
            self._value_edit.clear()
            self._value_edit.setEnabled(False)
        self._refresh_button_state()

    def current_row(self) -> int:
        return self._list.currentRow()

    def set_current_row(self, row: int) -> None:
        if 0 <= row < self._list.count():
            self._list.setCurrentRow(row)

    def _add_item(self) -> None:
        value = self._prompt_for_value()
        if value is not None:
            self.add_requested.emit(value)

    def _remove_item(self) -> None:
        row = self._list.currentRow()
        if row >= 0:
            self.remove_requested.emit(row)

    def _move_up(self) -> None:
        row = self._list.currentRow()
        if row > 0:
            self.move_requested.emit(row, row - 1)

    def _move_down(self) -> None:
        row = self._list.currentRow()
        if row >= 0 and row < self._list.count() - 1:
            self.move_requested.emit(row, row + 1)

    def _commit_selected_value(self) -> None:
        row = self._list.currentRow()
        if row < 0:
            return
        current_item = self._list.item(row)
        if current_item is None:
            return
        updated_text = self._value_edit.text()
        if updated_text != current_item.text():
            self.update_requested.emit(row, updated_text)

    def _on_selection_changed(self, row: int) -> None:
        if self._loading:
            return
        item = self._list.item(row)
        self._value_edit.setEnabled(item is not None)
        self._value_edit.setText(item.text() if item is not None else "")
        self._refresh_button_state()

    def _refresh_button_state(self) -> None:
        row = self._list.currentRow()
        has_selection = row >= 0
        self._remove_button.setEnabled(has_selection)
        self._up_button.setEnabled(has_selection and row > 0)
        self._down_button.setEnabled(has_selection and row < self._list.count() - 1)

    def _prompt_for_value(self) -> str | None:
        return TextEntryDialog.prompt(
            self,
            title=self._prompt_title,
            label=self._prompt_label,
            tooltip=self._prompt_tooltip,
        )

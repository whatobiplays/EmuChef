"""Shared widgets for recipe editor pages."""

from __future__ import annotations

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QHBoxLayout,
    QInputDialog,
    QLineEdit,
    QListWidget,
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
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._prompt_title = prompt_title
        self._prompt_label = prompt_label
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
        value, accepted = QInputDialog.getText(self, self._prompt_title, self._prompt_label)
        if not accepted:
            return None
        return value

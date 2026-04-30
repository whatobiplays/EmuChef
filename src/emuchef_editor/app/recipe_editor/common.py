"""Shared widgets for recipe editor pages.

These helpers keep editor pages structurally consistent without pushing UI
behavior into the core document layer. Widgets here focus on presentation and
commit semantics only.
"""

from __future__ import annotations

from PySide6.QtCore import QRect, QSize, Qt, Signal
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLayout,
    QLayoutItem,
    QLineEdit,
    QListWidget,
    QSizePolicy,
    QPushButton,
    QPlainTextEdit,
    QStackedWidget,
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


class AutoSizingListWidget(QListWidget):
    """List widget that grows with its content and stays visible when empty."""

    def __init__(self, *, minimum_visible_rows: int = 1, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._minimum_visible_rows = max(1, minimum_visible_rows)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)

        model = self.model()
        model.rowsInserted.connect(self._sync_height)
        model.rowsRemoved.connect(self._sync_height)
        model.modelReset.connect(self._sync_height)
        model.dataChanged.connect(self._sync_height)
        self._sync_height()

    def minimumSizeHint(self) -> QSize:  # type: ignore[override]
        base = super().minimumSizeHint()
        return QSize(base.width(), self._target_height())

    def sizeHint(self) -> QSize:  # type: ignore[override]
        base = super().sizeHint()
        return QSize(base.width(), self._target_height())

    def refresh_height(self) -> None:
        """Recompute the fixed height after external item or visibility changes."""

        self._sync_height()

    def _sync_height(self, *_args) -> None:
        self.setFixedHeight(self._target_height())
        self.updateGeometry()

    def _target_height(self) -> int:
        visible_rows = max(self._minimum_visible_rows, self.count())
        row_height = self.sizeHintForRow(0)
        if row_height <= 0:
            row_height = self.fontMetrics().lineSpacing() + 8
        spacing_height = max(0, visible_rows - 1) * self.spacing()
        return (self.frameWidth() * 2) + (visible_rows * row_height) + spacing_height


class AutoSizingPlainTextEdit(QPlainTextEdit):
    """Read-only plain-text view that fits its visible content height."""

    def __init__(self, *, minimum_visible_lines: int = 1, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._minimum_visible_lines = max(1, minimum_visible_lines)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.document().contentsChanged.connect(self._sync_height)
        self._sync_height()

    def minimumSizeHint(self) -> QSize:  # type: ignore[override]
        base = super().minimumSizeHint()
        return QSize(base.width(), self._target_height())

    def sizeHint(self) -> QSize:  # type: ignore[override]
        base = super().sizeHint()
        return QSize(base.width(), self._target_height())

    def refresh_height(self) -> None:
        """Recompute the fixed height after text or visibility changes."""

        self._sync_height()

    def _sync_height(self) -> None:
        self.setFixedHeight(self._target_height())
        self.updateGeometry()

    def _target_height(self) -> int:
        line_count = max(self._minimum_visible_lines, self.blockCount())
        line_height = self.fontMetrics().lineSpacing()
        document_margins = int(self.document().documentMargin() * 2)
        return (self.frameWidth() * 2) + document_margins + (line_count * line_height)


class CurrentWidgetSizeStack(QStackedWidget):
    """Stacked widget whose vertical size follows the active page only."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.currentChanged.connect(self._on_current_changed)

    def minimumSizeHint(self) -> QSize:  # type: ignore[override]
        widget = self.currentWidget()
        return widget.minimumSizeHint() if widget is not None else super().minimumSizeHint()

    def sizeHint(self) -> QSize:  # type: ignore[override]
        widget = self.currentWidget()
        return widget.sizeHint() if widget is not None else super().sizeHint()

    def _on_current_changed(self, _index: int) -> None:
        self.updateGeometry()


class FlowLayout(QLayout):
    """Layout child widgets left-to-right and wrap rows when width is limited.

    Qt's box layouts report a minimum width equal to the sum of every child in a
    row. Editor action bars use this layout when the actions should stay
    reachable without forcing the surrounding splitter pane to remain that wide.
    """

    def __init__(
        self,
        parent: QWidget | None = None,
        *,
        margin: int = 0,
        spacing: int = 6,
    ) -> None:
        super().__init__(parent)
        self._items: list[QLayoutItem] = []
        self.setContentsMargins(margin, margin, margin, margin)
        self.setSpacing(spacing)

    def addItem(self, item: QLayoutItem) -> None:  # type: ignore[override]
        self._items.append(item)
        self.invalidate()

    def count(self) -> int:  # type: ignore[override]
        return len(self._items)

    def itemAt(self, index: int) -> QLayoutItem | None:  # type: ignore[override]
        if 0 <= index < len(self._items):
            return self._items[index]
        return None

    def takeAt(self, index: int) -> QLayoutItem | None:  # type: ignore[override]
        if 0 <= index < len(self._items):
            item = self._items.pop(index)
            self.invalidate()
            return item
        return None

    def expandingDirections(self) -> Qt.Orientation:  # type: ignore[override]
        return Qt.Orientation(0)

    def hasHeightForWidth(self) -> bool:  # type: ignore[override]
        return True

    def heightForWidth(self, width: int) -> int:  # type: ignore[override]
        return self._do_layout(QRect(0, 0, width, 0), test_only=True)

    def setGeometry(self, rect: QRect) -> None:  # type: ignore[override]
        super().setGeometry(rect)
        self._do_layout(rect, test_only=False)

    def sizeHint(self) -> QSize:  # type: ignore[override]
        left, top, right, bottom = self.getContentsMargins()
        visible_items = [item for item in self._items if not item.isEmpty()]
        if not visible_items:
            return QSize(left + right, top + bottom)
        spacing = max(0, self.spacing())
        width = left + right + sum(item.sizeHint().width() for item in visible_items)
        width += spacing * max(0, len(visible_items) - 1)
        height = top + bottom + max(item.sizeHint().height() for item in visible_items)
        return QSize(width, height)

    def minimumSize(self) -> QSize:  # type: ignore[override]
        left, top, right, bottom = self.getContentsMargins()
        size = QSize()
        for item in self._items:
            if not item.isEmpty():
                size = size.expandedTo(item.minimumSize())
        size += QSize(left + right, top + bottom)
        return size

    def _do_layout(self, rect: QRect, *, test_only: bool) -> int:
        left, top, right, bottom = self.getContentsMargins()
        effective_rect = rect.adjusted(left, top, -right, -bottom)
        x = effective_rect.x()
        y = effective_rect.y()
        line_height = 0
        spacing = max(0, self.spacing())
        max_x = effective_rect.x() + max(0, effective_rect.width())

        for item in self._items:
            if item.isEmpty():
                continue
            item_size = item.sizeHint()
            if line_height > 0 and x + item_size.width() > max_x:
                x = effective_rect.x()
                y += line_height + spacing
                line_height = 0
            if not test_only:
                item.setGeometry(QRect(x, y, item_size.width(), item_size.height()))
            x += item_size.width() + spacing
            line_height = max(line_height, item_size.height())

        return (y + line_height) - rect.y() + bottom


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


def apply_tooltip(widget: QWidget, tooltip: str | None) -> QWidget:
    """Attach shared tooltip copy to a widget, clearing it when the text is blank."""

    normalized = tooltip.strip() if tooltip is not None else ""
    widget.setToolTip(normalized)
    return widget


def add_tooltipped_form_row(
    form: QFormLayout,
    label: str | QWidget,
    field: QWidget,
    tooltip: str | None,
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
        tooltip: str | None,
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
        tooltip: str | None,
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
        prompt_tooltip: str | None = None,
        field_tooltip: str | None = None,
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

"""Small dialogs for in-file usage inspection and destructive delete review."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QLabel,
    QMessageBox,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from emuchef_editor.core.analysis.usages import UsageAnalysis

DeleteDecision = Literal["cancel", "find_usages", "delete"]

PRESERVED_CONTENT_WARNING = (
    "Additional usages may exist in preserved unsupported content. That content is not rewritten or included here."
)


@dataclass(frozen=True, slots=True)
class UsageDialogContext:
    title: str
    item_label: str
    analysis: UsageAnalysis


class FindUsagesDialog(QDialog):
    """Read-only grouped in-file usage display."""

    def __init__(self, context: UsageDialogContext, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle(context.title)

        layout = QVBoxLayout(self)
        layout.addWidget(QLabel(f"Usages for {context.item_label}"))
        self._tree = _build_usage_tree(context.analysis)
        layout.addWidget(self._tree)
        if context.analysis.has_preserved_unsupported_content_warning:
            note = QLabel(PRESERVED_CONTENT_WARNING)
            note.setWordWrap(True)
            layout.addWidget(note)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

    @classmethod
    def show_usages(cls, parent: QWidget | None, *, item_label: str, analysis: UsageAnalysis) -> None:
        dialog = cls(UsageDialogContext(title="Find Usages", item_label=item_label, analysis=analysis), parent)
        dialog.exec()


class DeleteWithUsagesDialog(QDialog):
    """Destructive confirmation that shows supported structured usages first."""

    def __init__(self, context: UsageDialogContext, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle(context.title)
        self._decision: DeleteDecision = "cancel"

        layout = QVBoxLayout(self)
        label = QLabel(f"Delete {context.item_label}?")
        label.setWordWrap(True)
        layout.addWidget(label)
        self._tree = _build_usage_tree(context.analysis)
        layout.addWidget(self._tree)
        if context.analysis.has_preserved_unsupported_content_warning:
            note = QLabel(PRESERVED_CONTENT_WARNING)
            note.setWordWrap(True)
            layout.addWidget(note)

        buttons = QDialogButtonBox()
        self._cancel_button = buttons.addButton("Cancel", QDialogButtonBox.ButtonRole.RejectRole)
        self._find_usages_button = buttons.addButton("Find Usages", QDialogButtonBox.ButtonRole.ActionRole)
        self._delete_button = buttons.addButton("Delete Anyway", QDialogButtonBox.ButtonRole.DestructiveRole)
        self._cancel_button.clicked.connect(self.reject)
        self._find_usages_button.clicked.connect(self._choose_find_usages)
        self._delete_button.clicked.connect(self._choose_delete)
        layout.addWidget(buttons)

    @property
    def decision(self) -> DeleteDecision:
        return self._decision

    @classmethod
    def prompt(cls, parent: QWidget | None, *, item_label: str, analysis: UsageAnalysis) -> DeleteDecision:
        dialog = cls(UsageDialogContext(title="Confirm Delete", item_label=item_label, analysis=analysis), parent)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return "cancel"
        return dialog.decision

    def _choose_find_usages(self) -> None:
        self._decision = "find_usages"
        self.accept()

    def _choose_delete(self) -> None:
        self._decision = "delete"
        self.accept()


def _build_usage_tree(analysis: UsageAnalysis) -> QTreeWidget:
    tree = QTreeWidget()
    tree.setHeaderLabels(("Group", "Usage"))
    if not analysis.groups:
        QTreeWidgetItem(tree, ("No supported in-file usages", ""))
    for group in analysis.groups:
        parent = QTreeWidgetItem(tree, (group.title, ""))
        for usage in group.usages:
            QTreeWidgetItem(parent, ("", usage.summary))
        parent.setExpanded(True)
    tree.resizeColumnToContents(0)
    return tree


def confirm_preserved_content_warning(parent: QWidget | None, *, action: str) -> bool:
    response = QMessageBox.question(
        parent,
        "Preserved Unsupported Content",
        f"{action} will not rewrite preserved unsupported step content. Continue?",
        QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
        QMessageBox.StandardButton.No,
    )
    return response == QMessageBox.StandardButton.Yes

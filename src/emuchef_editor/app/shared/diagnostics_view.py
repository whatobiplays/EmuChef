"""Diagnostics pane widget."""

from __future__ import annotations

from PySide6.QtWidgets import QLabel, QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget

from emuchef_editor.core.validation.validator_service import DiagnosticResult


class DiagnosticsView(QWidget):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._summary = QLabel("No diagnostics.")
        self._tree = QTreeWidget()
        self._tree.setColumnCount(5)
        self._tree.setHeaderLabels(["Severity", "Code", "Message", "Object", "Field"])

        layout = QVBoxLayout(self)
        layout.addWidget(self._summary)
        layout.addWidget(self._tree)

    def set_result(self, result: DiagnosticResult | None) -> None:
        self._tree.clear()
        if result is None:
            self._summary.setText("No diagnostics.")
            return

        self._summary.setText(
            f"Status: {result.status.value} | Diagnostics: {len(result.diagnostics)}"
        )
        for diagnostic in result.diagnostics:
            item = QTreeWidgetItem(
                [
                    diagnostic.severity,
                    diagnostic.code,
                    diagnostic.message,
                    _object_label(diagnostic.object_kind, diagnostic.object_id),
                    diagnostic.field or "",
                ]
            )
            if diagnostic.file is not None:
                item.setToolTip(2, diagnostic.file)
            self._tree.addTopLevelItem(item)
        for column in range(self._tree.columnCount()):
            self._tree.resizeColumnToContents(column)


def _object_label(object_kind: str | None, object_id: str | None) -> str:
    if object_kind and object_id:
        return f"{object_kind}:{object_id}"
    if object_kind:
        return object_kind
    return object_id or ""

"""Read-only YAML preview widget."""

from __future__ import annotations

from PySide6.QtGui import QFontDatabase
from PySide6.QtWidgets import QPlainTextEdit


class YamlPreview(QPlainTextEdit):
    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.setReadOnly(True)
        fixed_font = QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont)
        self.setFont(fixed_font)

    def set_yaml(self, yaml_text: str) -> None:
        self.setPlainText(yaml_text)

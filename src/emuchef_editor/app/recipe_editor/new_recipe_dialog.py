"""Dialog for creating a new recipe from a template."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QMessageBox,
    QPushButton,
    QPlainTextEdit,
    QVBoxLayout,
    QWidget,
)

from .common import configure_data_entry_form, create_expanding_combo_box, create_expanding_line_edit


@dataclass(frozen=True, slots=True)
class NewRecipeRequest:
    template_path: Path
    destination_path: Path
    recipe_id: str


class NewRecipeDialog(QDialog):
    """Collects template, destination, and initial recipe id for new recipes."""

    def __init__(
        self,
        *,
        template_paths: tuple[Path, ...],
        authored_root: Path,
        preselected_template: Path | None = None,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setWindowTitle("New Recipe")
        self._template_paths = tuple(path.resolve() for path in template_paths)
        self._filename_follows_recipe_id = True

        self._template_combo = create_expanding_combo_box()
        for template_path in self._template_paths:
            self._template_combo.addItem(template_path.name, template_path)
        self._recipe_id_edit = create_expanding_line_edit()
        self._destination_dir_edit = create_expanding_line_edit()
        self._destination_dir_edit.setText(str(authored_root.resolve() / "recipes"))
        self._filename_edit = create_expanding_line_edit()
        self._template_preview = QPlainTextEdit()
        self._template_preview.setReadOnly(True)

        destination_row = QHBoxLayout()
        destination_row.addWidget(self._destination_dir_edit)
        browse_button = QPushButton("Browse...")
        browse_button.clicked.connect(self._browse_for_destination_directory)
        destination_row.addWidget(browse_button)
        destination_widget = QWidget()
        destination_widget.setLayout(destination_row)

        form = configure_data_entry_form(QFormLayout())
        form.addRow("Template", self._template_combo)
        form.addRow("Recipe ID", self._recipe_id_edit)
        form.addRow("Destination Directory", destination_widget)
        form.addRow("Filename", self._filename_edit)

        preview_label = QLabel("Template Preview")
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)

        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(preview_label)
        layout.addWidget(self._template_preview)
        layout.addWidget(buttons)

        self._template_combo.currentIndexChanged.connect(self._load_template_preview)
        self._recipe_id_edit.editingFinished.connect(self._recipe_id_committed)
        self._filename_edit.editingFinished.connect(self._filename_committed)

        if self._template_paths:
            selected_template = preselected_template.resolve() if preselected_template is not None else self._template_paths[0]
            selected_index = self._template_combo.findData(selected_template)
            self._template_combo.setCurrentIndex(max(selected_index, 0))
        self._recipe_id_edit.setText("new.recipe")
        self._recipe_id_committed()
        self._load_template_preview()
        self.resize(900, 700)

    def request(self) -> NewRecipeRequest:
        """Return the current dialog state as a file-creation request."""

        template_path = self._template_combo.currentData()
        if template_path is None:
            raise ValueError("A recipe template must be selected.")
        recipe_id = self._recipe_id_edit.text().strip()
        if not recipe_id:
            raise ValueError("Recipe ID must not be empty.")
        destination_directory = Path(self._destination_dir_edit.text().strip()).resolve()
        filename = self._filename_edit.text().strip()
        if not filename:
            raise ValueError("Filename must not be empty.")
        return NewRecipeRequest(
            template_path=Path(template_path).resolve(),
            destination_path=destination_directory / filename,
            recipe_id=recipe_id,
        )

    def accept(self) -> None:  # type: ignore[override]
        try:
            self.request()
        except ValueError as exc:
            QMessageBox.warning(self, "Invalid New Recipe Settings", str(exc))
            return
        super().accept()

    def _browse_for_destination_directory(self) -> None:
        selected = QFileDialog.getExistingDirectory(self, "Choose Destination Directory", self._destination_dir_edit.text())
        if selected:
            self._destination_dir_edit.setText(selected)

    def _load_template_preview(self) -> None:
        template_path = self._template_combo.currentData()
        if template_path is None:
            self._template_preview.setPlainText("")
            return
        self._template_preview.setPlainText(Path(template_path).read_text(encoding="utf-8"))

    def _recipe_id_committed(self) -> None:
        if self._filename_follows_recipe_id:
            self._filename_edit.setText(_suggested_filename(self._recipe_id_edit.text()))

    def _filename_committed(self) -> None:
        self._filename_follows_recipe_id = self._filename_edit.text().strip() == _suggested_filename(self._recipe_id_edit.text())


def _suggested_filename(recipe_id: str) -> str:
    normalized = recipe_id.strip()
    return f"{normalized}.yaml" if normalized else ""

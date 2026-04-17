"""Artifacts editor page."""

from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
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

from emuchef.domain import ArtifactCacheMode
from emuchef_editor.core.documents.commands import (
    AddArtifactCommand,
    DeleteArtifactCommand,
    DuplicateArtifactCommand,
    UpdateArtifactFieldCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .common import (
    TextEntryDialog,
    add_tooltipped_form_row,
    configure_data_entry_form,
    create_expanding_combo_box,
    create_expanding_line_edit,
)
from .tooltips import field_tooltip, prompt_tooltip


class ArtifactsPage(QWidget):
    """Edits recipe artifacts via core document commands."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None
        self._loading = False
        self._selected_artifact_id: str | None = None

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
        self._kind_value = QLabel("")
        self._kind_value.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self._url_edit = create_expanding_line_edit()
        self._cache_combo = create_expanding_combo_box()
        for value in ArtifactCacheMode:
            self._cache_combo.addItem(value.value, value)

        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._form, "ID", self._id_value, field_tooltip("artifacts.id"))
        add_tooltipped_form_row(self._form, "Kind", self._kind_value, field_tooltip("artifacts.kind"))
        add_tooltipped_form_row(self._form, "URL", self._url_edit, field_tooltip("artifacts.url"))
        add_tooltipped_form_row(self._form, "Cache", self._cache_combo, field_tooltip("artifacts.cache"))

        right_panel = QWidget()
        right_layout = QVBoxLayout(right_panel)
        right_layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        right_layout.addLayout(self._form)
        right_layout.addStretch(1)

        splitter = QSplitter()
        splitter.addWidget(left_panel)
        splitter.addWidget(right_panel)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 2)

        layout = QVBoxLayout(self)
        layout.addWidget(splitter)

        self._add_button.clicked.connect(self._add_artifact)
        self._delete_button.clicked.connect(self._delete_artifact)
        self._duplicate_button.clicked.connect(self._duplicate_artifact)
        self._url_edit.editingFinished.connect(self._commit_url)
        self._cache_combo.currentIndexChanged.connect(self._commit_cache)
        self._refresh_detail_enabled()

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        current_id = self._selected_artifact_id if self._selected_artifact_id in document.working_recipe.artifacts else None
        items = sorted(document.working_recipe.artifacts)
        self._loading = True
        self._list.clear()
        for artifact_id in items:
            item = QListWidgetItem(artifact_id)
            item.setData(Qt.ItemDataRole.UserRole, artifact_id)
            self._list.addItem(item)
        self._loading = False

        if items:
            self._select_artifact(current_id or items[0])
        else:
            self._selected_artifact_id = None
            self._clear_detail()
        self._refresh_button_state()

    def _select_artifact(self, artifact_id: str) -> None:
        for row in range(self._list.count()):
            item = self._list.item(row)
            if item.data(Qt.ItemDataRole.UserRole) == artifact_id:
                self._list.setCurrentRow(row)
                return

    def _on_selection_changed(self, current: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        if self._loading:
            return
        self._selected_artifact_id = current.data(Qt.ItemDataRole.UserRole) if current is not None else None
        self._load_selected_artifact()

    def _load_selected_artifact(self) -> None:
        if self._document is None or self._selected_artifact_id is None:
            self._clear_detail()
            return
        artifact = self._document.working_recipe.artifacts[self._selected_artifact_id]
        self._loading = True
        self._id_value.setText(artifact.id)
        self._kind_value.setText(artifact.type.value)
        self._url_edit.setText(artifact.url)
        self._cache_combo.setCurrentIndex(self._cache_combo.findData(artifact.cache))
        self._loading = False
        self._refresh_detail_enabled()
        self._refresh_button_state()

    def _clear_detail(self) -> None:
        self._loading = True
        self._id_value.clear()
        self._kind_value.clear()
        self._url_edit.clear()
        self._cache_combo.setCurrentIndex(0)
        self._loading = False
        self._refresh_detail_enabled()
        self._refresh_button_state()

    def _refresh_detail_enabled(self) -> None:
        enabled = self._selected_artifact_id is not None
        self._url_edit.setEnabled(enabled)
        self._cache_combo.setEnabled(enabled)

    def _refresh_button_state(self) -> None:
        has_selection = self._selected_artifact_id is not None
        self._delete_button.setEnabled(has_selection)
        self._duplicate_button.setEnabled(has_selection)

    def _add_artifact(self) -> None:
        artifact = self._prompt_for_new_artifact()
        if artifact is None:
            return
        artifact_id, _url = artifact
        previous_selection = self._selected_artifact_id
        self._selected_artifact_id = artifact_id
        if not self._command_handler(AddArtifactCommand(artifact_id=artifact_id, url=_url)):
            self._selected_artifact_id = previous_selection

    def _delete_artifact(self) -> None:
        if self._document is None or self._selected_artifact_id is None:
            return
        artifact_id = self._selected_artifact_id
        sorted_ids = sorted(self._document.working_recipe.artifacts)
        current_index = sorted_ids.index(artifact_id)
        remaining = [item for item in sorted_ids if item != artifact_id]
        self._selected_artifact_id = remaining[min(current_index, len(remaining) - 1)] if remaining else None
        self._command_handler(DeleteArtifactCommand(artifact_id=artifact_id))

    def _duplicate_artifact(self) -> None:
        if self._selected_artifact_id is None:
            return
        new_id = self._prompt_for_identifier("Duplicate Artifact", "New artifact id", prompt_tooltip("artifacts.id"))
        if new_id is None:
            return
        source_artifact_id = self._selected_artifact_id
        previous_selection = self._selected_artifact_id
        self._selected_artifact_id = new_id
        if not self._command_handler(
            DuplicateArtifactCommand(source_artifact_id=source_artifact_id, new_artifact_id=new_id)
        ):
            self._selected_artifact_id = previous_selection

    def _commit_url(self) -> None:
        if self._loading or self._selected_artifact_id is None:
            return
        self._command_handler(
            UpdateArtifactFieldCommand(
                artifact_id=self._selected_artifact_id,
                field="url",
                value=self._url_edit.text(),
            )
        )

    def _commit_cache(self) -> None:
        if self._loading or self._selected_artifact_id is None:
            return
        self._command_handler(
            UpdateArtifactFieldCommand(
                artifact_id=self._selected_artifact_id,
                field="cache",
                value=self._cache_combo.currentData(),
            )
        )

    def _prompt_for_identifier(self, title: str, label: str, tooltip: str | None) -> str | None:
        return TextEntryDialog.prompt(self, title=title, label=label, tooltip=tooltip)

    def _prompt_for_new_artifact(self) -> tuple[str, str] | None:
        dialog = QDialog(self)
        dialog.setWindowTitle("Add Artifact")
        form = configure_data_entry_form(QFormLayout(dialog))
        artifact_id_edit = create_expanding_line_edit()
        url_edit = create_expanding_line_edit()
        add_tooltipped_form_row(form, "Artifact id", artifact_id_edit, prompt_tooltip("artifacts.id"))
        add_tooltipped_form_row(form, "URL", url_edit, prompt_tooltip("artifacts.url"))
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addWidget(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return artifact_id_edit.text(), url_edit.text()

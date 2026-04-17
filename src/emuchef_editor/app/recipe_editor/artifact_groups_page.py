"""Artifact groups editor page."""

from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QFormLayout,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from emuchef_editor.core.documents.commands import (
    AddArtifactGroupCommand,
    AddArtifactGroupMemberCommand,
    DeleteArtifactGroupCommand,
    RemoveArtifactGroupMemberCommand,
    ReorderArtifactGroupCommand,
    ReorderArtifactGroupMemberCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument


class ArtifactGroupsPage(QWidget):
    """Edits artifact groups and ordered membership."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None
        self._loading = False
        self._selected_group_id: str | None = None
        self._selected_member_index: int = -1

        self._group_list = QListWidget()
        self._group_list.currentItemChanged.connect(self._on_group_selection_changed)
        self._add_group_button = QPushButton("Add")
        self._delete_group_button = QPushButton("Delete")
        self._move_group_up_button = QPushButton("Move Up")
        self._move_group_down_button = QPushButton("Move Down")

        group_buttons = QHBoxLayout()
        group_buttons.addWidget(self._add_group_button)
        group_buttons.addWidget(self._delete_group_button)
        group_buttons.addWidget(self._move_group_up_button)
        group_buttons.addWidget(self._move_group_down_button)

        left_panel = QWidget()
        left_layout = QVBoxLayout(left_panel)
        left_layout.addWidget(self._group_list)
        left_layout.addLayout(group_buttons)

        self._group_id_value = QLineEdit()
        self._group_id_value.setReadOnly(True)
        self._member_list = QListWidget()
        self._member_list.currentRowChanged.connect(self._on_member_selection_changed)
        self._add_member_button = QPushButton("Add")
        self._remove_member_button = QPushButton("Remove")
        self._move_member_up_button = QPushButton("Move Up")
        self._move_member_down_button = QPushButton("Move Down")

        member_buttons = QHBoxLayout()
        member_buttons.addWidget(self._add_member_button)
        member_buttons.addWidget(self._remove_member_button)
        member_buttons.addWidget(self._move_member_up_button)
        member_buttons.addWidget(self._move_member_down_button)

        right_panel = QWidget()
        right_layout = QVBoxLayout(right_panel)
        form = QFormLayout()
        form.addRow("Group ID", self._group_id_value)
        right_layout.addLayout(form)
        right_layout.addWidget(QLabel("Members"))
        right_layout.addWidget(self._member_list)
        right_layout.addLayout(member_buttons)

        splitter = QSplitter()
        splitter.addWidget(left_panel)
        splitter.addWidget(right_panel)
        splitter.setStretchFactor(0, 1)
        splitter.setStretchFactor(1, 2)

        layout = QVBoxLayout(self)
        layout.addWidget(splitter)

        self._add_group_button.clicked.connect(self._add_group)
        self._delete_group_button.clicked.connect(self._delete_group)
        self._move_group_up_button.clicked.connect(self._move_group_up)
        self._move_group_down_button.clicked.connect(self._move_group_down)
        self._add_member_button.clicked.connect(self._add_member)
        self._remove_member_button.clicked.connect(self._remove_member)
        self._move_member_up_button.clicked.connect(self._move_member_up)
        self._move_member_down_button.clicked.connect(self._move_member_down)

        self._refresh_button_state()

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        group_ids = tuple(document.working_recipe.artifact_groups.keys())
        current_id = self._selected_group_id if self._selected_group_id in document.working_recipe.artifact_groups else None
        self._loading = True
        self._group_list.clear()
        for group_id in group_ids:
            item = QListWidgetItem(group_id)
            item.setData(Qt.ItemDataRole.UserRole, group_id)
            self._group_list.addItem(item)
        self._loading = False

        if group_ids:
            self._select_group(current_id or group_ids[0])
        else:
            self._selected_group_id = None
            self._group_id_value.clear()
            self._member_list.clear()
        self._refresh_button_state()

    def _select_group(self, group_id: str) -> None:
        for row in range(self._group_list.count()):
            item = self._group_list.item(row)
            if item.data(Qt.ItemDataRole.UserRole) == group_id:
                self._group_list.setCurrentRow(row)
                return

    def _on_group_selection_changed(self, current: QListWidgetItem | None, _previous: QListWidgetItem | None) -> None:
        if self._loading:
            return
        self._selected_group_id = current.data(Qt.ItemDataRole.UserRole) if current is not None else None
        self._selected_member_index = -1
        self._load_selected_group()

    def _load_selected_group(self) -> None:
        self._loading = True
        self._member_list.clear()
        if self._document is None or self._selected_group_id is None:
            self._group_id_value.clear()
            self._loading = False
            self._refresh_button_state()
            return
        self._group_id_value.setText(self._selected_group_id)
        members = self._document.working_recipe.artifact_groups[self._selected_group_id]
        for member in members:
            self._member_list.addItem(member)
        if members:
            row = min(max(self._selected_member_index, 0), len(members) - 1)
            self._member_list.setCurrentRow(row)
        self._loading = False
        self._refresh_button_state()

    def _on_member_selection_changed(self, row: int) -> None:
        if self._loading:
            return
        self._selected_member_index = row
        self._refresh_button_state()

    def _refresh_button_state(self) -> None:
        group_row = self._group_list.currentRow()
        has_group = self._selected_group_id is not None
        member_row = self._member_list.currentRow()
        has_member = member_row >= 0
        self._delete_group_button.setEnabled(has_group)
        self._move_group_up_button.setEnabled(has_group and group_row > 0)
        self._move_group_down_button.setEnabled(has_group and group_row < self._group_list.count() - 1)
        self._add_member_button.setEnabled(has_group and bool(self._available_members()))
        self._remove_member_button.setEnabled(has_member)
        self._move_member_up_button.setEnabled(has_member and member_row > 0)
        self._move_member_down_button.setEnabled(has_member and member_row < self._member_list.count() - 1)

    def _add_group(self) -> None:
        group_id = self._prompt_for_identifier("Add Artifact Group", "Group id")
        if group_id is None:
            return
        previous_selection = self._selected_group_id
        self._selected_group_id = group_id
        if not self._command_handler(AddArtifactGroupCommand(group_id=group_id)):
            self._selected_group_id = previous_selection

    def _delete_group(self) -> None:
        if self._document is None or self._selected_group_id is None:
            return
        group_id = self._selected_group_id
        group_ids = tuple(self._document.working_recipe.artifact_groups.keys())
        current_index = group_ids.index(group_id)
        remaining = [item for item in group_ids if item != group_id]
        self._selected_group_id = remaining[min(current_index, len(remaining) - 1)] if remaining else None
        self._selected_member_index = -1
        self._command_handler(DeleteArtifactGroupCommand(group_id=group_id))

    def _move_group_up(self) -> None:
        if self._selected_group_id is None:
            return
        row = self._group_list.currentRow()
        self._command_handler(ReorderArtifactGroupCommand(group_id=self._selected_group_id, to_index=row - 1))

    def _move_group_down(self) -> None:
        if self._selected_group_id is None:
            return
        row = self._group_list.currentRow()
        self._command_handler(ReorderArtifactGroupCommand(group_id=self._selected_group_id, to_index=row + 1))

    def _add_member(self) -> None:
        if self._selected_group_id is None:
            return
        choices = self._available_members()
        if not choices:
            return
        artifact_id, accepted = QInputDialog.getItem(
            self,
            "Add Artifact Group Member",
            "Artifact",
            choices,
            0,
            editable=False,
        )
        if not accepted:
            return
        insertion_index = self._member_list.count()
        self._selected_member_index = insertion_index
        self._command_handler(
            AddArtifactGroupMemberCommand(
                group_id=self._selected_group_id,
                artifact_id=artifact_id,
                index=insertion_index,
            )
        )

    def _remove_member(self) -> None:
        if self._selected_group_id is None:
            return
        row = self._member_list.currentRow()
        if row < 0:
            return
        remaining_count = self._member_list.count() - 1
        self._selected_member_index = min(row, remaining_count - 1) if remaining_count > 0 else -1
        self._command_handler(RemoveArtifactGroupMemberCommand(group_id=self._selected_group_id, index=row))

    def _move_member_up(self) -> None:
        if self._selected_group_id is None:
            return
        row = self._member_list.currentRow()
        if row <= 0:
            return
        self._selected_member_index = row - 1
        self._command_handler(
            ReorderArtifactGroupMemberCommand(
                group_id=self._selected_group_id,
                index=row,
                to_index=row - 1,
            )
        )

    def _move_member_down(self) -> None:
        if self._selected_group_id is None:
            return
        row = self._member_list.currentRow()
        if row < 0 or row >= self._member_list.count() - 1:
            return
        self._selected_member_index = row + 1
        self._command_handler(
            ReorderArtifactGroupMemberCommand(
                group_id=self._selected_group_id,
                index=row,
                to_index=row + 1,
            )
        )

    def _available_members(self) -> list[str]:
        if self._document is None or self._selected_group_id is None:
            return []
        members = set(self._document.working_recipe.artifact_groups[self._selected_group_id])
        return [artifact_id for artifact_id in sorted(self._document.working_recipe.artifacts) if artifact_id not in members]

    def _prompt_for_identifier(self, title: str, label: str) -> str | None:
        value, accepted = QInputDialog.getText(self, title, label)
        if not accepted:
            return None
        return value

"""Permissions editor page."""

from __future__ import annotations

from dataclasses import dataclass

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QPushButton,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from emuchef.domain import AppOpGrant, PermissionWhen, RuntimePermissionGrant
from emuchef_editor.core.documents.commands import (
    AddAppOpCommand,
    AddRuntimePermissionCommand,
    DeleteAppOpCommand,
    DeleteRuntimePermissionCommand,
    UpdateAppOpCommand,
    UpdatePermissionPolicyFieldCommand,
    UpdateRuntimePermissionCommand,
)
from emuchef_editor.core.documents.recipe_document import RecipeDocument

from .common import (
    add_tooltipped_form_row,
    configure_data_entry_form,
    create_expanding_combo_box,
    create_expanding_line_edit,
)


@dataclass(frozen=True, slots=True)
class _WhenFields:
    rooted: bool | None
    android_api_min: int | None
    android_api_max: int | None


class PermissionsPage(QWidget):
    """Edits the top-level declarative permission surface."""

    def __init__(self, command_handler, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._command_handler = command_handler
        self._document: RecipeDocument | None = None
        self._loading = False
        self._selected_runtime_index = -1
        self._selected_appop_index = -1

        self._runtime_list = QListWidget()
        self._runtime_list.currentRowChanged.connect(self._on_runtime_selection_changed)
        self._add_runtime_button = QPushButton("Add")
        self._delete_runtime_button = QPushButton("Delete")
        runtime_button_row = QHBoxLayout()
        runtime_button_row.addWidget(self._add_runtime_button)
        runtime_button_row.addWidget(self._delete_runtime_button)
        runtime_left = QWidget()
        runtime_left_layout = QVBoxLayout(runtime_left)
        runtime_left_layout.addWidget(self._runtime_list)
        runtime_left_layout.addLayout(runtime_button_row)

        self._runtime_package_edit = create_expanding_line_edit()
        self._runtime_name_edit = create_expanding_line_edit()
        self._runtime_required_check = QCheckBox()
        self._runtime_rooted_combo = create_expanding_combo_box()
        self._runtime_rooted_combo.addItem("Any", None)
        self._runtime_rooted_combo.addItem("Yes", True)
        self._runtime_rooted_combo.addItem("No", False)
        self._runtime_api_min_edit = create_expanding_line_edit()
        self._runtime_api_max_edit = create_expanding_line_edit()
        self._runtime_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._runtime_form, "Package", self._runtime_package_edit, None)
        add_tooltipped_form_row(self._runtime_form, "Permission", self._runtime_name_edit, None)
        add_tooltipped_form_row(self._runtime_form, "Required", self._runtime_required_check, None)
        add_tooltipped_form_row(self._runtime_form, "Rooted", self._runtime_rooted_combo, None)
        add_tooltipped_form_row(self._runtime_form, "Android API Min", self._runtime_api_min_edit, None)
        add_tooltipped_form_row(self._runtime_form, "Android API Max", self._runtime_api_max_edit, None)
        runtime_right = QWidget()
        runtime_right_layout = QVBoxLayout(runtime_right)
        runtime_right_layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        runtime_right_layout.addLayout(self._runtime_form)
        runtime_right_layout.addStretch(1)
        runtime_splitter = QSplitter()
        runtime_splitter.addWidget(runtime_left)
        runtime_splitter.addWidget(runtime_right)
        runtime_splitter.setStretchFactor(0, 1)
        runtime_splitter.setStretchFactor(1, 2)
        runtime_section = QGroupBox("Runtime Permissions")
        runtime_section_layout = QVBoxLayout(runtime_section)
        runtime_section_layout.addWidget(runtime_splitter)

        self._appops_list = QListWidget()
        self._appops_list.currentRowChanged.connect(self._on_appop_selection_changed)
        self._add_appop_button = QPushButton("Add")
        self._delete_appop_button = QPushButton("Delete")
        appop_button_row = QHBoxLayout()
        appop_button_row.addWidget(self._add_appop_button)
        appop_button_row.addWidget(self._delete_appop_button)
        appop_left = QWidget()
        appop_left_layout = QVBoxLayout(appop_left)
        appop_left_layout.addWidget(self._appops_list)
        appop_left_layout.addLayout(appop_button_row)

        self._appop_package_edit = create_expanding_line_edit()
        self._appop_name_edit = create_expanding_line_edit()
        self._appop_mode_edit = create_expanding_line_edit()
        self._appop_required_check = QCheckBox()
        self._appop_rooted_combo = create_expanding_combo_box()
        self._appop_rooted_combo.addItem("Any", None)
        self._appop_rooted_combo.addItem("Yes", True)
        self._appop_rooted_combo.addItem("No", False)
        self._appop_api_min_edit = create_expanding_line_edit()
        self._appop_api_max_edit = create_expanding_line_edit()
        self._appop_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._appop_form, "Package", self._appop_package_edit, None)
        add_tooltipped_form_row(self._appop_form, "App Op", self._appop_name_edit, None)
        add_tooltipped_form_row(self._appop_form, "Mode", self._appop_mode_edit, None)
        add_tooltipped_form_row(self._appop_form, "Required", self._appop_required_check, None)
        add_tooltipped_form_row(self._appop_form, "Rooted", self._appop_rooted_combo, None)
        add_tooltipped_form_row(self._appop_form, "Android API Min", self._appop_api_min_edit, None)
        add_tooltipped_form_row(self._appop_form, "Android API Max", self._appop_api_max_edit, None)
        appop_right = QWidget()
        appop_right_layout = QVBoxLayout(appop_right)
        appop_right_layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        appop_right_layout.addLayout(self._appop_form)
        appop_right_layout.addStretch(1)
        appop_splitter = QSplitter()
        appop_splitter.addWidget(appop_left)
        appop_splitter.addWidget(appop_right)
        appop_splitter.setStretchFactor(0, 1)
        appop_splitter.setStretchFactor(1, 2)
        appop_section = QGroupBox("App Ops")
        appop_section_layout = QVBoxLayout(appop_section)
        appop_section_layout.addWidget(appop_splitter)

        self._policy_on_failure_combo = create_expanding_combo_box()
        self._policy_on_failure_combo.setEditable(True)
        self._policy_on_failure_combo.addItems(["warn", "fail"])
        self._policy_require_all_check = QCheckBox()
        self._policy_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(self._policy_form, "On Failure", self._policy_on_failure_combo, None)
        add_tooltipped_form_row(self._policy_form, "Require All", self._policy_require_all_check, None)
        policy_section = QGroupBox("Policy")
        policy_section_layout = QVBoxLayout(policy_section)
        policy_section_layout.addLayout(self._policy_form)

        layout = QVBoxLayout(self)
        layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        layout.addWidget(runtime_section)
        layout.addWidget(appop_section)
        layout.addWidget(policy_section)
        layout.addStretch(1)

        self._add_runtime_button.clicked.connect(self._add_runtime_permission)
        self._delete_runtime_button.clicked.connect(self._delete_runtime_permission)
        self._runtime_package_edit.editingFinished.connect(self._commit_runtime_permission)
        self._runtime_name_edit.editingFinished.connect(self._commit_runtime_permission)
        self._runtime_required_check.toggled.connect(self._commit_runtime_permission)
        self._runtime_rooted_combo.currentIndexChanged.connect(self._commit_runtime_permission)
        self._runtime_api_min_edit.editingFinished.connect(self._commit_runtime_permission)
        self._runtime_api_max_edit.editingFinished.connect(self._commit_runtime_permission)

        self._add_appop_button.clicked.connect(self._add_appop)
        self._delete_appop_button.clicked.connect(self._delete_appop)
        self._appop_package_edit.editingFinished.connect(self._commit_appop)
        self._appop_name_edit.editingFinished.connect(self._commit_appop)
        self._appop_mode_edit.editingFinished.connect(self._commit_appop)
        self._appop_required_check.toggled.connect(self._commit_appop)
        self._appop_rooted_combo.currentIndexChanged.connect(self._commit_appop)
        self._appop_api_min_edit.editingFinished.connect(self._commit_appop)
        self._appop_api_max_edit.editingFinished.connect(self._commit_appop)

        self._policy_on_failure_combo.currentTextChanged.connect(self._commit_policy_on_failure)
        self._policy_require_all_check.toggled.connect(self._commit_policy_require_all)

        self._refresh_button_state()

    def set_document(self, document: RecipeDocument) -> None:
        self._document = document
        self._populate_runtime_list()
        self._populate_appop_list()
        self._load_policy()
        self._refresh_button_state()

    def _populate_runtime_list(self) -> None:
        runtime = self._document.working_recipe.permissions.runtime if self._document is not None else ()
        target_index = self._selected_runtime_index if 0 <= self._selected_runtime_index < len(runtime) else (0 if runtime else -1)
        self._loading = True
        self._runtime_list.clear()
        for permission in runtime:
            self._runtime_list.addItem(f"{permission.package_name} · {permission.name}")
        self._loading = False
        if target_index >= 0:
            self._runtime_list.setCurrentRow(target_index)
        else:
            self._selected_runtime_index = -1
            self._clear_runtime_detail()

    def _populate_appop_list(self) -> None:
        appops = self._document.working_recipe.permissions.appops if self._document is not None else ()
        target_index = self._selected_appop_index if 0 <= self._selected_appop_index < len(appops) else (0 if appops else -1)
        self._loading = True
        self._appops_list.clear()
        for appop in appops:
            self._appops_list.addItem(f"{appop.package_name} · {appop.op} ({appop.mode})")
        self._loading = False
        if target_index >= 0:
            self._appops_list.setCurrentRow(target_index)
        else:
            self._selected_appop_index = -1
            self._clear_appop_detail()

    def _load_policy(self) -> None:
        if self._document is None:
            return
        policy = self._document.working_recipe.permissions.policy
        self._loading = True
        if self._policy_on_failure_combo.findText(policy.on_failure) < 0:
            self._policy_on_failure_combo.addItem(policy.on_failure)
        self._policy_on_failure_combo.setCurrentText(policy.on_failure)
        self._policy_require_all_check.setChecked(policy.require_all)
        self._loading = False

    def _on_runtime_selection_changed(self, row: int) -> None:
        if self._loading:
            return
        self._selected_runtime_index = row
        self._load_runtime_detail()

    def _on_appop_selection_changed(self, row: int) -> None:
        if self._loading:
            return
        self._selected_appop_index = row
        self._load_appop_detail()

    def _load_runtime_detail(self) -> None:
        if self._document is None or self._selected_runtime_index < 0:
            self._clear_runtime_detail()
            return
        permission = self._document.working_recipe.permissions.runtime[self._selected_runtime_index]
        self._loading = True
        self._runtime_package_edit.setText(permission.package_name)
        self._runtime_name_edit.setText(permission.name)
        self._runtime_required_check.setChecked(permission.required)
        when = permission.when or PermissionWhen()
        self._runtime_rooted_combo.setCurrentIndex(self._runtime_rooted_combo.findData(when.rooted))
        self._runtime_api_min_edit.setText("" if when.android_api_min is None else str(when.android_api_min))
        self._runtime_api_max_edit.setText("" if when.android_api_max is None else str(when.android_api_max))
        self._loading = False
        self._refresh_button_state()

    def _load_appop_detail(self) -> None:
        if self._document is None or self._selected_appop_index < 0:
            self._clear_appop_detail()
            return
        appop = self._document.working_recipe.permissions.appops[self._selected_appop_index]
        self._loading = True
        self._appop_package_edit.setText(appop.package_name)
        self._appop_name_edit.setText(appop.op)
        self._appop_mode_edit.setText(appop.mode)
        self._appop_required_check.setChecked(appop.required)
        when = appop.when or PermissionWhen()
        self._appop_rooted_combo.setCurrentIndex(self._appop_rooted_combo.findData(when.rooted))
        self._appop_api_min_edit.setText("" if when.android_api_min is None else str(when.android_api_min))
        self._appop_api_max_edit.setText("" if when.android_api_max is None else str(when.android_api_max))
        self._loading = False
        self._refresh_button_state()

    def _clear_runtime_detail(self) -> None:
        self._loading = True
        self._runtime_package_edit.clear()
        self._runtime_name_edit.clear()
        self._runtime_required_check.setChecked(True)
        self._runtime_rooted_combo.setCurrentIndex(0)
        self._runtime_api_min_edit.clear()
        self._runtime_api_max_edit.clear()
        self._loading = False
        self._refresh_button_state()

    def _clear_appop_detail(self) -> None:
        self._loading = True
        self._appop_package_edit.clear()
        self._appop_name_edit.clear()
        self._appop_mode_edit.clear()
        self._appop_required_check.setChecked(True)
        self._appop_rooted_combo.setCurrentIndex(0)
        self._appop_api_min_edit.clear()
        self._appop_api_max_edit.clear()
        self._loading = False
        self._refresh_button_state()

    def _refresh_button_state(self) -> None:
        has_runtime = self._selected_runtime_index >= 0
        has_appop = self._selected_appop_index >= 0
        self._delete_runtime_button.setEnabled(has_runtime)
        self._delete_appop_button.setEnabled(has_appop)
        for widget in (
            self._runtime_package_edit,
            self._runtime_name_edit,
            self._runtime_required_check,
            self._runtime_rooted_combo,
            self._runtime_api_min_edit,
            self._runtime_api_max_edit,
        ):
            widget.setEnabled(has_runtime)
        for widget in (
            self._appop_package_edit,
            self._appop_name_edit,
            self._appop_mode_edit,
            self._appop_required_check,
            self._appop_rooted_combo,
            self._appop_api_min_edit,
            self._appop_api_max_edit,
        ):
            widget.setEnabled(has_appop)

    def _add_runtime_permission(self) -> None:
        permission = self._prompt_for_new_runtime_permission()
        if permission is None:
            return
        target_index = len(self._document.working_recipe.permissions.runtime) if self._document is not None else 0
        self._selected_runtime_index = target_index
        if not self._command_handler(AddRuntimePermissionCommand(permission=permission)):
            self._selected_runtime_index = max(target_index - 1, -1)

    def _delete_runtime_permission(self) -> None:
        if self._document is None or self._selected_runtime_index < 0:
            return
        index = self._selected_runtime_index
        remaining_count = len(self._document.working_recipe.permissions.runtime) - 1
        self._selected_runtime_index = min(index, remaining_count - 1) if remaining_count > 0 else -1
        self._command_handler(DeleteRuntimePermissionCommand(index=index))

    def _commit_runtime_permission(self) -> None:
        if self._loading or self._selected_runtime_index < 0:
            return
        permission = RuntimePermissionGrant(
            package_name=self._runtime_package_edit.text(),
            name=self._runtime_name_edit.text(),
            required=self._runtime_required_check.isChecked(),
            when=_build_when(
                rooted=self._runtime_rooted_combo.currentData(),
                android_api_min=self._runtime_api_min_edit.text(),
                android_api_max=self._runtime_api_max_edit.text(),
            ),
        )
        self._command_handler(
            UpdateRuntimePermissionCommand(
                index=self._selected_runtime_index,
                permission=permission,
            )
        )

    def _add_appop(self) -> None:
        appop = self._prompt_for_new_appop()
        if appop is None:
            return
        target_index = len(self._document.working_recipe.permissions.appops) if self._document is not None else 0
        self._selected_appop_index = target_index
        if not self._command_handler(AddAppOpCommand(appop=appop)):
            self._selected_appop_index = max(target_index - 1, -1)

    def _delete_appop(self) -> None:
        if self._document is None or self._selected_appop_index < 0:
            return
        index = self._selected_appop_index
        remaining_count = len(self._document.working_recipe.permissions.appops) - 1
        self._selected_appop_index = min(index, remaining_count - 1) if remaining_count > 0 else -1
        self._command_handler(DeleteAppOpCommand(index=index))

    def _commit_appop(self) -> None:
        if self._loading or self._selected_appop_index < 0:
            return
        appop = AppOpGrant(
            package_name=self._appop_package_edit.text(),
            op=self._appop_name_edit.text(),
            mode=self._appop_mode_edit.text(),
            required=self._appop_required_check.isChecked(),
            when=_build_when(
                rooted=self._appop_rooted_combo.currentData(),
                android_api_min=self._appop_api_min_edit.text(),
                android_api_max=self._appop_api_max_edit.text(),
            ),
        )
        self._command_handler(UpdateAppOpCommand(index=self._selected_appop_index, appop=appop))

    def _commit_policy_on_failure(self, value: str) -> None:
        if self._loading or self._document is None:
            return
        if value != self._document.working_recipe.permissions.policy.on_failure:
            self._command_handler(UpdatePermissionPolicyFieldCommand(field="on_failure", value=value))

    def _commit_policy_require_all(self, value: bool) -> None:
        if self._loading or self._document is None:
            return
        if value != self._document.working_recipe.permissions.policy.require_all:
            self._command_handler(UpdatePermissionPolicyFieldCommand(field="require_all", value=value))

    def _prompt_for_new_runtime_permission(self) -> RuntimePermissionGrant | None:
        dialog = _RuntimePermissionDialog(parent=self)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return dialog.permission()

    def _prompt_for_new_appop(self) -> AppOpGrant | None:
        dialog = _AppOpDialog(parent=self)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return None
        return dialog.appop()


class _RuntimePermissionDialog(QDialog):
    """Collects the minimum valid runtime-permission shape before insertion."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Add Runtime Permission")
        self._package_edit = create_expanding_line_edit()
        self._name_edit = create_expanding_line_edit()
        form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(form, "Package", self._package_edit, None)
        add_tooltipped_form_row(form, "Permission", self._name_edit, None)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(buttons)

    def permission(self) -> RuntimePermissionGrant:
        return RuntimePermissionGrant(
            package_name=self._package_edit.text(),
            name=self._name_edit.text(),
        )


class _AppOpDialog(QDialog):
    """Collects the minimum valid app-op shape before insertion."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Add App Op")
        self._package_edit = create_expanding_line_edit()
        self._op_edit = create_expanding_line_edit()
        self._mode_edit = create_expanding_line_edit()
        form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(form, "Package", self._package_edit, None)
        add_tooltipped_form_row(form, "App Op", self._op_edit, None)
        add_tooltipped_form_row(form, "Mode", self._mode_edit, None)
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(form)
        layout.addWidget(buttons)

    def appop(self) -> AppOpGrant:
        return AppOpGrant(
            package_name=self._package_edit.text(),
            op=self._op_edit.text(),
            mode=self._mode_edit.text(),
        )


def _build_when(*, rooted: bool | None, android_api_min: str, android_api_max: str) -> PermissionWhen | None:
    api_min = _optional_int(android_api_min)
    api_max = _optional_int(android_api_max)
    if rooted is None and api_min is None and api_max is None:
        return None
    return PermissionWhen(rooted=rooted, android_api_min=api_min, android_api_max=api_max)


def _optional_int(value: str) -> int | None:
    text = value.strip()
    if not text:
        return None
    return int(text)

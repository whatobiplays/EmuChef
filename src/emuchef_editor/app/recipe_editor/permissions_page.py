"""Step-local permission parameter editor widgets."""

from __future__ import annotations

from dataclasses import dataclass
from collections.abc import Mapping, Sequence

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QSplitter,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from emuchef.domain import (
    AppOpGrant,
    AuthoredParamValue,
    PERMISSION_POLICY_ON_FAILURE_VALUES,
    PermissionPolicy,
    PermissionWhen,
    RuntimePermissionGrant,
)

from .common import (
    AutoSizingListWidget,
    add_tooltipped_form_row,
    configure_data_entry_form,
    create_expanding_combo_box,
    create_expanding_line_edit,
)
from .tooltips import field_tooltip, prompt_tooltip


@dataclass(frozen=True, slots=True)
class _WhenFields:
    rooted: bool | None
    android_api_min: int | None
    android_api_max: int | None


class GrantPermissionParamsEditor(QWidget):
    """Edits the `grant_permissions.params` runtime, app-op, and policy fields."""

    changed = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._loading = False
        self._selected_runtime_index = -1
        self._selected_appop_index = -1
        self._runtime_permissions: tuple[RuntimePermissionGrant, ...] = ()
        self._appops: tuple[AppOpGrant, ...] = ()
        self._policy = PermissionPolicy()
        self._policy_authored = False

        self._runtime_list = AutoSizingListWidget()
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
        add_tooltipped_form_row(
            self._runtime_form,
            "Package",
            self._runtime_package_edit,
            field_tooltip("steps.grant_permissions.runtime.package_name"),
        )
        add_tooltipped_form_row(
            self._runtime_form,
            "Permission",
            self._runtime_name_edit,
            field_tooltip("steps.grant_permissions.runtime.name"),
        )
        add_tooltipped_form_row(
            self._runtime_form,
            "Required",
            self._runtime_required_check,
            field_tooltip("steps.grant_permissions.runtime.required"),
        )
        add_tooltipped_form_row(
            self._runtime_form,
            "Rooted",
            self._runtime_rooted_combo,
            field_tooltip("steps.grant_permissions.when.rooted"),
        )
        add_tooltipped_form_row(
            self._runtime_form,
            "Android API Min",
            self._runtime_api_min_edit,
            field_tooltip("steps.grant_permissions.when.android_api_min"),
        )
        add_tooltipped_form_row(
            self._runtime_form,
            "Android API Max",
            self._runtime_api_max_edit,
            field_tooltip("steps.grant_permissions.when.android_api_max"),
        )
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

        self._appops_list = AutoSizingListWidget()
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
        add_tooltipped_form_row(
            self._appop_form,
            "Package",
            self._appop_package_edit,
            field_tooltip("steps.grant_permissions.appops.package_name"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "App Op",
            self._appop_name_edit,
            field_tooltip("steps.grant_permissions.appops.op"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "Mode",
            self._appop_mode_edit,
            field_tooltip("steps.grant_permissions.appops.mode"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "Required",
            self._appop_required_check,
            field_tooltip("steps.grant_permissions.appops.required"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "Rooted",
            self._appop_rooted_combo,
            field_tooltip("steps.grant_permissions.when.rooted"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "Android API Min",
            self._appop_api_min_edit,
            field_tooltip("steps.grant_permissions.when.android_api_min"),
        )
        add_tooltipped_form_row(
            self._appop_form,
            "Android API Max",
            self._appop_api_max_edit,
            field_tooltip("steps.grant_permissions.when.android_api_max"),
        )
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
        self._policy_on_failure_combo.addItems(PERMISSION_POLICY_ON_FAILURE_VALUES)
        self._policy_require_all_check = QCheckBox()
        self._policy_form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            self._policy_form,
            "On Failure",
            self._policy_on_failure_combo,
            field_tooltip("steps.grant_permissions.policy.on_failure"),
        )
        add_tooltipped_form_row(
            self._policy_form,
            "Require All",
            self._policy_require_all_check,
            field_tooltip("steps.grant_permissions.policy.require_all"),
        )
        policy_section = QGroupBox("Policy")
        policy_section_layout = QVBoxLayout(policy_section)
        policy_section_layout.addLayout(self._policy_form)

        layout = QVBoxLayout(self)
        layout.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        layout.addWidget(runtime_section)
        layout.addWidget(appop_section)
        layout.addWidget(policy_section)

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

    def set_params(self, params: Mapping[str, AuthoredParamValue]) -> None:
        self._runtime_permissions = tuple(_parse_runtime_permission(item) for item in _mapping_sequence(params.get("runtime")))
        self._appops = tuple(_parse_appop(item) for item in _mapping_sequence(params.get("appops")))
        self._policy_authored = isinstance(params.get("policy"), Mapping)
        self._policy = _parse_policy(params.get("policy"))
        self._populate_runtime_list()
        self._populate_appop_list()
        self._load_policy()
        self._refresh_button_state()

    def params(self) -> dict[str, AuthoredParamValue]:
        params: dict[str, AuthoredParamValue] = {}
        if self._runtime_permissions:
            params["runtime"] = [_runtime_permission_payload(permission) for permission in self._runtime_permissions]
        if self._appops:
            params["appops"] = [_appop_payload(appop) for appop in self._appops]
        if self._policy_authored or self._policy != PermissionPolicy():
            params["policy"] = _policy_payload(self._policy)
        return params

    def _populate_runtime_list(self) -> None:
        target_index = (
            self._selected_runtime_index
            if 0 <= self._selected_runtime_index < len(self._runtime_permissions)
            else (0 if self._runtime_permissions else -1)
        )
        self._loading = True
        self._runtime_list.clear()
        for permission in self._runtime_permissions:
            self._runtime_list.addItem(f"{permission.package_name} · {permission.name}")
        self._loading = False
        if target_index >= 0:
            self._runtime_list.setCurrentRow(target_index)
        else:
            self._selected_runtime_index = -1
            self._clear_runtime_detail()
        self._runtime_list.refresh_height()

    def _populate_appop_list(self) -> None:
        target_index = self._selected_appop_index if 0 <= self._selected_appop_index < len(self._appops) else (0 if self._appops else -1)
        self._loading = True
        self._appops_list.clear()
        for appop in self._appops:
            self._appops_list.addItem(f"{appop.package_name} · {appop.op} ({appop.mode})")
        self._loading = False
        if target_index >= 0:
            self._appops_list.setCurrentRow(target_index)
        else:
            self._selected_appop_index = -1
            self._clear_appop_detail()
        self._appops_list.refresh_height()

    def _load_policy(self) -> None:
        self._loading = True
        self._policy_on_failure_combo.clear()
        self._policy_on_failure_combo.addItems(PERMISSION_POLICY_ON_FAILURE_VALUES)
        if self._policy_on_failure_combo.findText(self._policy.on_failure) < 0:
            self._policy_on_failure_combo.addItem(self._policy.on_failure)
        self._policy_on_failure_combo.setCurrentText(self._policy.on_failure)
        self._policy_require_all_check.setChecked(self._policy.require_all)
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
        if self._selected_runtime_index < 0 or self._selected_runtime_index >= len(self._runtime_permissions):
            self._clear_runtime_detail()
            return
        permission = self._runtime_permissions[self._selected_runtime_index]
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
        if self._selected_appop_index < 0 or self._selected_appop_index >= len(self._appops):
            self._clear_appop_detail()
            return
        appop = self._appops[self._selected_appop_index]
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
        self._runtime_permissions = self._runtime_permissions + (permission,)
        self._selected_runtime_index = len(self._runtime_permissions) - 1
        self._populate_runtime_list()
        self._emit_changed()

    def _delete_runtime_permission(self) -> None:
        if self._selected_runtime_index < 0 or self._selected_runtime_index >= len(self._runtime_permissions):
            return
        index = self._selected_runtime_index
        self._runtime_permissions = tuple(
            permission for permission_index, permission in enumerate(self._runtime_permissions) if permission_index != index
        )
        self._selected_runtime_index = min(index, len(self._runtime_permissions) - 1) if self._runtime_permissions else -1
        self._populate_runtime_list()
        self._emit_changed()

    def _commit_runtime_permission(self) -> None:
        if self._loading or self._selected_runtime_index < 0 or self._selected_runtime_index >= len(self._runtime_permissions):
            return
        permissions = list(self._runtime_permissions)
        permissions[self._selected_runtime_index] = RuntimePermissionGrant(
            package_name=self._runtime_package_edit.text(),
            name=self._runtime_name_edit.text(),
            required=self._runtime_required_check.isChecked(),
            when=_build_when(
                rooted=self._runtime_rooted_combo.currentData(),
                android_api_min=self._runtime_api_min_edit.text(),
                android_api_max=self._runtime_api_max_edit.text(),
            ),
        )
        self._runtime_permissions = tuple(permissions)
        self._emit_changed()

    def _add_appop(self) -> None:
        appop = self._prompt_for_new_appop()
        if appop is None:
            return
        self._appops = self._appops + (appop,)
        self._selected_appop_index = len(self._appops) - 1
        self._populate_appop_list()
        self._emit_changed()

    def _delete_appop(self) -> None:
        if self._selected_appop_index < 0 or self._selected_appop_index >= len(self._appops):
            return
        index = self._selected_appop_index
        self._appops = tuple(appop for appop_index, appop in enumerate(self._appops) if appop_index != index)
        self._selected_appop_index = min(index, len(self._appops) - 1) if self._appops else -1
        self._populate_appop_list()
        self._emit_changed()

    def _commit_appop(self) -> None:
        if self._loading or self._selected_appop_index < 0 or self._selected_appop_index >= len(self._appops):
            return
        appops = list(self._appops)
        appops[self._selected_appop_index] = AppOpGrant(
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
        self._appops = tuple(appops)
        self._emit_changed()

    def _commit_policy_on_failure(self, value: str) -> None:
        if self._loading:
            return
        self._policy_authored = True
        self._policy = PermissionPolicy(on_failure=value, require_all=self._policy.require_all)
        self._emit_changed()

    def _commit_policy_require_all(self, value: bool) -> None:
        if self._loading:
            return
        self._policy_authored = True
        self._policy = PermissionPolicy(on_failure=self._policy.on_failure, require_all=value)
        self._emit_changed()

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

    def _emit_changed(self) -> None:
        if not self._loading:
            self.changed.emit()


class _RuntimePermissionDialog(QDialog):
    """Collects the minimum valid runtime-permission shape before insertion."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Add Runtime Permission")
        self._package_edit = create_expanding_line_edit()
        self._name_edit = create_expanding_line_edit()
        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            self._form,
            "Package",
            self._package_edit,
            prompt_tooltip("steps.grant_permissions.runtime.package_name"),
        )
        add_tooltipped_form_row(
            self._form,
            "Permission",
            self._name_edit,
            prompt_tooltip("steps.grant_permissions.runtime.name"),
        )
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(self._form)
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
        self._form = configure_data_entry_form(QFormLayout())
        add_tooltipped_form_row(
            self._form,
            "Package",
            self._package_edit,
            prompt_tooltip("steps.grant_permissions.appops.package_name"),
        )
        add_tooltipped_form_row(
            self._form,
            "App Op",
            self._op_edit,
            prompt_tooltip("steps.grant_permissions.appops.op"),
        )
        add_tooltipped_form_row(
            self._form,
            "Mode",
            self._mode_edit,
            prompt_tooltip("steps.grant_permissions.appops.mode"),
        )
        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout = QVBoxLayout(self)
        layout.addLayout(self._form)
        layout.addWidget(buttons)

    def appop(self) -> AppOpGrant:
        return AppOpGrant(
            package_name=self._package_edit.text(),
            op=self._op_edit.text(),
            mode=self._mode_edit.text(),
        )


def _parse_runtime_permission(data: Mapping[str, object]) -> RuntimePermissionGrant:
    return RuntimePermissionGrant(
        package_name=str(data.get("package_name", "")),
        name=str(data.get("name", "")),
        required=bool(data.get("required", True)),
        when=_parse_when(data.get("when")),
    )


def _parse_appop(data: Mapping[str, object]) -> AppOpGrant:
    return AppOpGrant(
        package_name=str(data.get("package_name", "")),
        op=str(data.get("op", "")),
        mode=str(data.get("mode", "")),
        required=bool(data.get("required", True)),
        when=_parse_when(data.get("when")),
    )


def _parse_policy(data: object) -> PermissionPolicy:
    if not isinstance(data, Mapping):
        return PermissionPolicy()
    return PermissionPolicy(
        on_failure=str(data.get("on_failure", PermissionPolicy().on_failure)),
        require_all=bool(data.get("require_all", PermissionPolicy().require_all)),
    )


def _parse_when(data: object) -> PermissionWhen | None:
    if not isinstance(data, Mapping):
        return None
    return _normalize_when(
        PermissionWhen(
            rooted=data.get("rooted") if isinstance(data.get("rooted"), bool) else None,
            android_api_min=_optional_int_value(data.get("android_api_min")),
            android_api_max=_optional_int_value(data.get("android_api_max")),
        )
    )


def _runtime_permission_payload(permission: RuntimePermissionGrant) -> dict[str, object]:
    payload: dict[str, object] = {
        "package_name": permission.package_name,
        "name": permission.name,
        "required": permission.required,
    }
    when = _permission_when_payload(permission.when)
    if when:
        payload["when"] = when
    return payload


def _appop_payload(appop: AppOpGrant) -> dict[str, object]:
    payload: dict[str, object] = {
        "package_name": appop.package_name,
        "op": appop.op,
        "mode": appop.mode,
        "required": appop.required,
    }
    when = _permission_when_payload(appop.when)
    if when:
        payload["when"] = when
    return payload


def _policy_payload(policy: PermissionPolicy) -> dict[str, object]:
    return {
        "on_failure": policy.on_failure,
        "require_all": policy.require_all,
    }


def _permission_when_payload(when: PermissionWhen | None) -> dict[str, object]:
    normalized = _normalize_when(when)
    if normalized is None:
        return {}
    payload: dict[str, object] = {}
    if normalized.rooted is not None:
        payload["rooted"] = normalized.rooted
    if normalized.android_api_min is not None:
        payload["android_api_min"] = normalized.android_api_min
    if normalized.android_api_max is not None:
        payload["android_api_max"] = normalized.android_api_max
    return payload


def _build_when(*, rooted: bool | None, android_api_min: str, android_api_max: str) -> PermissionWhen | None:
    api_min = _optional_int(android_api_min)
    api_max = _optional_int(android_api_max)
    return _normalize_when(PermissionWhen(rooted=rooted, android_api_min=api_min, android_api_max=api_max))


def _normalize_when(when: PermissionWhen | None) -> PermissionWhen | None:
    if when is None:
        return None
    if when.rooted is None and when.android_api_min is None and when.android_api_max is None:
        return None
    return when


def _mapping_sequence(value: object) -> tuple[Mapping[str, object], ...]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        return ()
    return tuple(item for item in value if isinstance(item, Mapping))


def _optional_int(value: str) -> int | None:
    text = value.strip()
    if not text:
        return None
    return int(text)


def _optional_int_value(value: object) -> int | None:
    if value is None or isinstance(value, bool):
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None

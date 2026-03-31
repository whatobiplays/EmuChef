"""skip_if and verify evaluation."""

from __future__ import annotations

from emuchef.domain import StepCondition

from .adb import AdbInterface


def evaluate_condition(adb: AdbInterface, condition: StepCondition) -> bool:
    if condition.type == "package_installed":
        return adb.package_installed(str(condition.params["package_name"]))
    if condition.type == "path_exists":
        return adb.path_exists(str(condition.params["path"]))
    if condition.type == "file_exists":
        path = str(condition.params["path"])
        return adb.path_exists(path) and not adb.path_is_dir(path)
    raise ValueError(f"Unsupported condition type: {condition.type}")

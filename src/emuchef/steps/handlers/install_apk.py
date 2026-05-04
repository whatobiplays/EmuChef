"""Execution handler for the ``install_apk`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

from emuchef.domain import ExecutionStep, RuntimeValue, RuntimeValueType
from emuchef.executor.runtime_values import require_runtime_value
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    app = require_runtime_value(resolved_params["app"])
    if app.type is not RuntimeValueType.FILE_PATH or app.location != "host":
        raise ValueError("install_apk requires a host-side file_path runtime value.")
    apk_path = Path(str(app.value))
    if apk_path.suffix.lower() != ".apk":
        raise ValueError(f"install_apk requires an .apk file, got: {apk_path}")
    if not apk_path.exists():
        raise FileNotFoundError(f"APK file not found: {apk_path}")
    context.adb.install_apk(apk_path, replace_existing=bool(resolved_params.get("replace_existing", False)))
    return {}

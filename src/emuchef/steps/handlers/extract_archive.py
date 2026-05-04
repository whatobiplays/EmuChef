"""Execution handler for the ``extract_archive`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

from emuchef.domain import ExecutionStep, RuntimeValue, RuntimeValueType
from emuchef.executor.artifact_io import extract_zip_to_directory
from emuchef.executor.runtime_values import require_runtime_value
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    archive = require_runtime_value(resolved_params["archive"])
    extract_on = str(resolved_params.get("extract_on", "host"))
    cleanup = bool(resolved_params.get("cleanup", True))

    if extract_on == "device":
        dest = str(resolved_params["dest"])
        device_archive_path = str(resolved_params.get("device_temp_path") or f"/data/local/tmp/emuchef/{step.id.replace('/', '_')}.zip")
        if archive.location == "host":
            context.adb.push(Path(str(archive.value)), device_archive_path)
        else:
            device_archive_path = str(archive.value)
        context.adb.mkdir_p(dest)
        context.adb.run_plan_command(("adb", "shell", "unzip", "-o", device_archive_path, "-d", dest))
        if cleanup and archive.location == "host":
            context.adb.remove_file(device_archive_path)
        return {"extracted_path": RuntimeValue(type=RuntimeValueType.DIRECTORY_PATH, value=dest, location="device")}

    if archive.location != "host":
        raise ValueError("Host extraction requires a host-side archive path.")
    archive_path = Path(str(archive.value))
    extract_root = context.workdir / ".emuchef_runtime" / "extract" / step.id.replace("/", "_")
    members = extract_zip_to_directory(archive_path, extract_root)
    if len(members) == 1:
        member = members[0]
        runtime_type = RuntimeValueType.DIRECTORY_PATH if member.is_dir() else RuntimeValueType.FILE_PATH
        return {"extracted_path": RuntimeValue(type=runtime_type, value=str(member), location="host")}
    return {"extracted_path": RuntimeValue(type=RuntimeValueType.DIRECTORY_PATH, value=str(extract_root), location="host")}

"""Execution handler for the ``extract_artifacts`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

from emuchef.domain import ExecutionStep, RuntimeValue, RuntimeValueType
from emuchef.executor.artifact_io import extract_zip_to_directory
from emuchef.executor.runtime_values import literal_string_list
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    artifact_ids = literal_string_list(resolved_params.get("artifacts"))
    extract_on = str(resolved_params.get("extract_on", "host"))

    if extract_on == "device":
        base_dir = f"/data/local/tmp/emuchef/{step.id.replace('/', '_')}"
        outputs: list[str] = []
        for artifact_id in artifact_ids:
            artifact_state = context.state.artifacts[artifact_id]
            archive_path = Path(artifact_state.local_path or "")
            device_archive_path = f"{base_dir}/{archive_path.name}"
            extract_dir = f"{base_dir}/{artifact_id.rsplit('/', 1)[-1]}"
            context.adb.mkdir_p(base_dir)
            context.adb.push(archive_path, device_archive_path)
            context.adb.mkdir_p(extract_dir)
            context.adb.run_plan_command(("adb", "shell", "unzip", "-o", device_archive_path, "-d", extract_dir))
            outputs.append(extract_dir)
        return {"extracted_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=outputs, location="device")}

    extract_root = context.workdir / ".emuchef_runtime" / "extract" / step.id.replace("/", "_")
    outputs: list[str] = []
    for artifact_id in artifact_ids:
        artifact_state = context.state.artifacts[artifact_id]
        archive_path = Path(artifact_state.local_path or "")
        artifact_dir = extract_root / artifact_id.rsplit("/", 1)[-1]
        members = extract_zip_to_directory(archive_path, artifact_dir)
        outputs.extend(str(member) for member in members)
    return {"extracted_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=outputs, location="host")}

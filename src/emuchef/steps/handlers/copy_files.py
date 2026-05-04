"""Execution handler for the ``copy_files`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import CopyPolicy, ErrorCode, ExecutionStep, RuntimeValue, RuntimeValueType
from emuchef.executor.adb import is_app_private_path
from emuchef.executor.copy_helpers import (
    copy_device_source,
    copy_host_source,
    copy_host_source_to_app_private,
    supports_app_data_write,
)
from emuchef.executor.runtime_values import require_runtime_value
from emuchef.executor.step_runtime import ExecutionContext, StepExecutionError


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    source = require_runtime_value(resolved_params["source"])
    dest = str(resolved_params["dest"])
    copy_policy = CopyPolicy(str(resolved_params.get("copy_policy", CopyPolicy.MERGE.value)))
    app_private_dest = is_app_private_path(dest)

    if app_private_dest and not supports_app_data_write(context.runtime_capabilities):
        raise StepExecutionError(
            ErrorCode.APP_DATA_WRITE_UNAVAILABLE,
            (
                f"Destination {dest!r} requires root-backed app_data_write support, but runtime capabilities "
                "do not provide both app_data_write and root_shell."
            ),
        )

    if app_private_dest and source.location == "host":
        copied = copy_host_source_to_app_private(context, step, source, dest, copy_policy)
    elif source.location == "device":
        copied = copy_device_source(context, source, dest, copy_policy)
    else:
        copied = copy_host_source(context, source, dest, copy_policy)
    return {"copied_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=copied, location="device")}

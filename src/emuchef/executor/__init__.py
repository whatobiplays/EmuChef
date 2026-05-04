"""Executor exports."""

from .adb import AdbCommandError, AdbResolutionError, DetectedDevice, DryRunAdb, SubprocessAdb, resolve_adb_executable
from .result import (
    ExecutionProgressEvent,
    ExecutionRunResult,
    ProgressPhase,
    ProgressStatus,
    StepRunRecord,
    StepRunStatus,
)


def __getattr__(name: str):
    if name == "ExecutorRunner":
        from .runner import ExecutorRunner

        return ExecutorRunner
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

__all__ = [
    "AdbCommandError",
    "AdbResolutionError",
    "DetectedDevice",
    "DryRunAdb",
    "ExecutionProgressEvent",
    "ExecutionRunResult",
    "ExecutorRunner",
    "ProgressPhase",
    "ProgressStatus",
    "StepRunRecord",
    "StepRunStatus",
    "SubprocessAdb",
    "resolve_adb_executable",
]

#!/usr/bin/env python3
"""Smoke direct live ADB getprop probing through the Rust shadow binary.

This optional developer tool invokes a supplied shadow planner executable with
explicit live-probe arguments and emits deterministic JSON evidence. It is a
manual route check only: it does not discover devices, write artifacts, or reuse
other smoke helpers.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath
from typing import Mapping, Sequence


REPORT_KIND = "rust_shadow_live_adb_probe_smoke"
REPORT_SCHEMA_VERSION = 1
CASE_ID = "live_adb_getprop_shadow_probe"
DEVICE_CONTEXT_FIELDS = ("manufacturer", "model", "android_version", "android_api_level")
PROBE_FAILURE_STDERR = {
    "Error: adb_probe_unavailable\n": "adb_probe_unavailable",
    "Error: adb_probe_failed\n": "adb_probe_failed",
}


@dataclass(frozen=True, slots=True)
class CaseResult:
    id: str
    status: str
    exit_class: str
    stdout_class: str
    stderr_class: str
    planning_status: str | None
    device_context_fields_present: tuple[str, ...]
    device_profile_mismatch_seen: bool
    failure_class: str | None = None


def build_shadow_command(
    *,
    rust_planner_bin: str,
    authored_root: str,
    device_plan: str,
    adb_path: str,
    serial: str,
) -> list[str]:
    return [
        rust_planner_bin,
        "--authored-root",
        authored_root,
        "--device-plan",
        device_plan,
        "--probe-adb-getprop",
        "--adb-path",
        adb_path,
        "--serial",
        serial,
    ]


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer-supplied executable paths are passed as a
    # structured argv list with shell=False; this tool does not compose shell text.
    return subprocess.run(
        list(argv),
        check=False,
        text=True,
        capture_output=True,
    )


def run_smoke_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    adb_path: str,
    serial: str,
) -> dict[str, object]:
    command = build_shadow_command(
        rust_planner_bin=rust_planner_bin,
        authored_root=authored_root,
        device_plan=device_plan,
        adb_path=adb_path,
        serial=serial,
    )
    try:
        completed = run_process(command)
    except OSError:
        result = process_start_failure_result()
    else:
        result = classify_completed_process(completed)

    return build_report(
        authored_root=authored_root,
        device_plan=device_plan,
        rust_planner_bin=rust_planner_bin,
        adb_path=adb_path,
        serial_supplied=True,
        case_results=[result],
    )


def classify_completed_process(completed: subprocess.CompletedProcess[str]) -> CaseResult:
    probe_failure_class = _probe_failure_class(completed)
    if probe_failure_class is not None:
        return _failure_result(
            exit_class=probe_failure_class,
            stdout_class="empty",
            stderr_class=probe_failure_class,
            failure_class=probe_failure_class,
        )

    if completed.returncode == 2 or "usage: emuchef-plan-shadow" in completed.stderr:
        return _failure_result(
            exit_class="usage_failure",
            stdout_class=_classify_non_json_stdout(completed.stdout),
            stderr_class="usage_failure",
            failure_class="usage_failure",
        )

    if completed.returncode not in {0, 1}:
        return _failure_result(
            exit_class="unexpected_exit",
            stdout_class=_classify_non_json_stdout(completed.stdout),
            stderr_class=_classify_stderr(completed.stderr),
            failure_class="unexpected_exit",
        )

    if completed.stderr.strip():
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class=_classify_non_json_stdout(completed.stdout),
            stderr_class="text",
            failure_class="stderr_text",
        )

    payload, stdout_class = _parse_planning_json(completed.stdout)
    if payload is None:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class=stdout_class,
            stderr_class="empty",
            failure_class=stdout_class,
        )

    if "kind" in payload and payload.get("kind") != "planning_result":
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="json_not_planning_result",
            stderr_class="empty",
            failure_class="json_not_planning_result",
        )

    planning_status = payload.get("status")
    if planning_status not in {"success", "warning"}:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status=str(planning_status) if planning_status is not None else None,
            failure_class="unexpected_planning_status",
        )

    warnings, warnings_valid = _warning_codes(payload)
    if not warnings_valid:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status=planning_status,
            failure_class="warnings_not_stable",
        )

    device_context = _device_context(payload)
    if device_context is None:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status=planning_status,
            failure_class="device_context_missing",
        )

    fields_present = _device_context_fields_present(device_context)
    if "manufacturer" not in fields_present or "model" not in fields_present:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status=planning_status,
            device_context_fields_present=fields_present,
            failure_class="device_context_required_fields_missing",
        )
    if "android_version" not in fields_present and "android_api_level" not in fields_present:
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status=planning_status,
            device_context_fields_present=fields_present,
            failure_class="device_context_android_version_missing",
        )

    return CaseResult(
        id=CASE_ID,
        status="pass",
        exit_class="success_or_warning",
        stdout_class="planning_result_json",
        stderr_class="empty",
        planning_status=planning_status,
        device_context_fields_present=fields_present,
        device_profile_mismatch_seen="device_profile_mismatch" in warnings,
    )


def process_start_failure_result() -> CaseResult:
    return _failure_result(
        exit_class="shadow_process_start_failed",
        stdout_class="not_started",
        stderr_class="not_started",
        failure_class="shadow_process_start_failed",
    )


def build_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    adb_path: str,
    serial_supplied: bool,
    case_results: Sequence[CaseResult],
) -> dict[str, object]:
    passed = sum(1 for result in case_results if result.status == "pass")
    failed = len(case_results) - passed
    command_metadata = {
        "probe_flag_present": True,
        "adb_path_supplied": bool(adb_path),
        "serial_supplied": bool(serial_supplied),
    }
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "pass" if failed == 0 else "fail",
        "inputs": {
            "authored_root": _stable_authored_root(authored_root),
            "device_plan": device_plan,
            "rust_planner_bin": _stable_basename(rust_planner_bin),
            "adb_path": _stable_adb_path(adb_path),
            "serial_supplied": bool(serial_supplied),
        },
        "cases": [
            _case_report_item(result, command_metadata=command_metadata)
            for result in case_results
        ],
        "summary": {
            "passed": passed,
            "failed": failed,
        },
    }


def smoke_exit_code(report: Mapping[str, object]) -> int:
    summary = report.get("summary")
    if not isinstance(summary, Mapping):
        return 1
    return 0 if summary.get("failed") == 0 else 1


def dumps_report(report: Mapping[str, object]) -> str:
    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="smoke_rust_shadow_live_adb_probe.py",
        description="Smoke direct Rust shadow live ADB getprop probing.",
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    parser.add_argument("--rust-planner-bin", required=True)
    parser.add_argument("--adb-path", required=True)
    parser.add_argument("--serial", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    report = run_smoke_report(
        authored_root=args.authored_root,
        device_plan=args.device_plan,
        rust_planner_bin=args.rust_planner_bin,
        adb_path=args.adb_path,
        serial=args.serial,
    )
    sys.stdout.write(dumps_report(report))
    return smoke_exit_code(report)


def _case_report_item(
    result: CaseResult,
    *,
    command_metadata: Mapping[str, bool],
) -> dict[str, object]:
    item: dict[str, object] = {
        "id": result.id,
        "status": result.status,
        "exit_class": result.exit_class,
        "stdout_class": result.stdout_class,
        "stderr_class": result.stderr_class,
        "planning_status": result.planning_status,
        "device_context_fields_present": list(result.device_context_fields_present),
        "device_profile_mismatch_seen": result.device_profile_mismatch_seen,
        "command_metadata": dict(command_metadata),
    }
    if result.failure_class is not None:
        item["failure_class"] = result.failure_class
    return item


def _failure_result(
    *,
    exit_class: str,
    stdout_class: str,
    stderr_class: str,
    failure_class: str,
    planning_status: str | None = None,
    device_context_fields_present: Sequence[str] = (),
) -> CaseResult:
    return CaseResult(
        id=CASE_ID,
        status="fail",
        exit_class=exit_class,
        stdout_class=stdout_class,
        stderr_class=stderr_class,
        planning_status=planning_status,
        device_context_fields_present=tuple(device_context_fields_present),
        device_profile_mismatch_seen=False,
        failure_class=failure_class,
    )


def _probe_failure_class(completed: subprocess.CompletedProcess[str]) -> str | None:
    if completed.returncode == 0 or completed.stdout.strip():
        return None
    return PROBE_FAILURE_STDERR.get(completed.stderr)


def _parse_planning_json(stdout_text: str) -> tuple[Mapping[str, object] | None, str]:
    if not stdout_text.strip():
        return None, "stdout_empty"
    try:
        payload = json.loads(stdout_text)
    except json.JSONDecodeError:
        return None, "stdout_text"
    if not isinstance(payload, Mapping):
        return None, "json_not_object"
    return payload, "planning_result_json"


def _classify_non_json_stdout(stdout_text: str) -> str:
    if not stdout_text.strip():
        return "empty"
    try:
        payload = json.loads(stdout_text)
    except json.JSONDecodeError:
        return "stdout_text"
    return "json_object" if isinstance(payload, Mapping) else "json_not_object"


def _classify_stderr(stderr_text: str) -> str:
    return "empty" if not stderr_text.strip() else "text"


def _warning_codes(payload: Mapping[str, object]) -> tuple[set[str], bool]:
    if "warnings" not in payload:
        return set(), True
    warnings = payload.get("warnings")
    if not isinstance(warnings, list):
        return set(), False
    codes: set[str] = set()
    for warning in warnings:
        if not isinstance(warning, Mapping):
            return set(), False
        code = warning.get("code")
        if not isinstance(code, str) or not code:
            return set(), False
        codes.add(code)
    return codes, True


def _device_context(payload: Mapping[str, object]) -> Mapping[str, object] | None:
    execution_plan = payload.get("execution_plan")
    if not isinstance(execution_plan, Mapping):
        return None
    device_context = execution_plan.get("device_context")
    return device_context if isinstance(device_context, Mapping) else None


def _device_context_fields_present(device_context: Mapping[str, object]) -> tuple[str, ...]:
    present: list[str] = []
    manufacturer = device_context.get("manufacturer")
    if isinstance(manufacturer, str) and manufacturer.strip():
        present.append("manufacturer")
    model = device_context.get("model")
    if isinstance(model, str) and model.strip():
        present.append("model")
    for key in ("android_version", "android_api_level"):
        if key in device_context and device_context[key] is not None:
            present.append(key)
    return tuple(present)


def _stable_basename(value: str) -> str:
    if "\\" in value:
        return PureWindowsPath(value).name or "<path>"
    return Path(value).name or value or "<path>"


def _stable_adb_path(value: str) -> str:
    if "\\" in value:
        return PureWindowsPath(value).name or "<path>"
    if "/" in value:
        return Path(value).name or "<path>"
    return value or "<path>"


def _stable_authored_root(value: str) -> str:
    path = Path(value)
    if path.name == "authored":
        return "authored"
    if path.is_absolute():
        return path.name or "<path>"
    return value


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Smoke Python rust-experimental live ADB probe forwarding.

This optional developer tool invokes the Python planner route with wrapper
probe arguments and emits deterministic JSON evidence. It does not discover
devices, inspect supplied probe inputs, or reuse other smoke helpers.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath
from typing import Mapping, Sequence


REPORT_KIND = "rust_experimental_live_adb_probe_smoke"
REPORT_SCHEMA_VERSION = 1
CASE_ID = "rust_experimental_live_adb_probe_forwarding"
WARNING_CODE = "device_profile_mismatch"
PROBE_FAILURE_MARKERS = {
    "Error: adb_probe_unavailable": "adb_probe_unavailable",
    "Error: adb_probe_failed": "adb_probe_failed",
}


@dataclass(frozen=True, slots=True)
class CaseResult:
    id: str
    status: str
    exit_class: str
    stdout_class: str
    stderr_class: str
    planning_status: str | None
    device_profile_mismatch_seen: bool
    failure_class: str | None = None


def build_cli_command(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    adb_path: str,
    serial: str,
) -> list[str]:
    return [
        sys.executable,
        "-m",
        "emuchef",
        "plan",
        "--planner-backend",
        "rust-experimental",
        "--rust-planner-bin",
        rust_planner_bin,
        "--authored-root",
        authored_root,
        "--device-plan",
        device_plan,
        "--rust-probe-adb-getprop",
        "--rust-adb-path",
        adb_path,
        "--rust-serial",
        serial,
    ]


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer-supplied inputs are passed as structured argv;
    # this tool does not compose shell text.
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
    command = build_cli_command(
        authored_root=authored_root,
        device_plan=device_plan,
        rust_planner_bin=rust_planner_bin,
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
    probe_failure_class = _probe_failure_class(completed.stderr)
    if probe_failure_class is not None:
        return _failure_result(
            exit_class=probe_failure_class,
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class=probe_failure_class,
            failure_class=probe_failure_class,
        )

    if completed.returncode == 2 or _looks_like_usage_failure(completed.stderr):
        return _failure_result(
            exit_class="usage_failure",
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class="usage_failure",
            failure_class="usage_failure",
        )

    if completed.returncode not in {0, 1}:
        return _failure_result(
            exit_class="unexpected_exit",
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class=_classify_stderr(completed.stderr),
            failure_class="unexpected_exit",
        )

    if completed.stderr.strip():
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class="text",
            failure_class="stderr_text",
        )

    stdout_class, planning_status = _classify_python_route_stdout(completed.stdout)
    if stdout_class == "python_compatible" and planning_status in {"success", "warning"}:
        return CaseResult(
            id=CASE_ID,
            status="pass",
            exit_class="success_or_warning",
            stdout_class=stdout_class,
            stderr_class="empty",
            planning_status=planning_status,
            device_profile_mismatch_seen=WARNING_CODE in completed.stdout,
        )

    failure_class = "raw_json_stdout" if stdout_class == "raw_json_stdout" else "rust_experimental_live_probe_route_failed"
    return _failure_result(
        exit_class="success_or_warning",
        stdout_class=stdout_class,
        stderr_class="empty",
        planning_status=planning_status,
        failure_class=failure_class,
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
        "python_route": True,
        "rust_experimental": True,
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
        prog="smoke_rust_experimental_live_adb_probe.py",
        description="Smoke Python rust-experimental live ADB probe forwarding.",
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
) -> CaseResult:
    return CaseResult(
        id=CASE_ID,
        status="fail",
        exit_class=exit_class,
        stdout_class=stdout_class,
        stderr_class=stderr_class,
        planning_status=planning_status,
        device_profile_mismatch_seen=False,
        failure_class=failure_class,
    )


def _probe_failure_class(stderr_text: str) -> str | None:
    for marker, failure_class in PROBE_FAILURE_MARKERS.items():
        if marker in stderr_text:
            return failure_class
    return None


def _looks_like_usage_failure(stderr_text: str) -> bool:
    return "usage:" in stderr_text or "unrecognized arguments:" in stderr_text


def _classify_python_route_stdout(stdout_text: str) -> tuple[str, str | None]:
    if not stdout_text.strip():
        return "stdout_empty", None
    try:
        json.loads(stdout_text)
    except json.JSONDecodeError:
        pass
    else:
        return "raw_json_stdout", None

    planning_status = _extract_planning_status(stdout_text)
    if planning_status in {"success", "warning"} and _has_python_compatible_indicator(stdout_text):
        return "python_compatible", planning_status
    return "stdout_text", planning_status


def _classify_failure_stdout(stdout_text: str) -> str:
    stdout_class, _planning_status = _classify_python_route_stdout(stdout_text)
    return stdout_class if stdout_class != "python_compatible" else "python_compatible"


def _classify_stderr(stderr_text: str) -> str:
    return "empty" if not stderr_text.strip() else "text"


def _extract_planning_status(stdout_text: str) -> str | None:
    for pattern in (
        r"^Planning status:\s*([A-Za-z_]+)\s*$",
        r"^status:\s*([A-Za-z_]+)\s*$",
    ):
        match = re.search(pattern, stdout_text, flags=re.MULTILINE)
        if match is not None:
            return match.group(1)
    return None


def _has_python_compatible_indicator(stdout_text: str) -> bool:
    if "Planning status:" in stdout_text:
        return True
    return bool(
        re.search(r"^status:\s*(success|warning)\s*$", stdout_text, flags=re.MULTILINE)
        and (
            "kind: planning_result" in stdout_text
            or "execution_plan:" in stdout_text
            or "warnings:" in stdout_text
        )
    )


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
    if "\\" in value:
        name = PureWindowsPath(value).name
        if name == "authored":
            return "authored"
        return name or "<path>"
    path = Path(value)
    if path.name == "authored":
        return "authored"
    if path.is_absolute():
        return path.name or "<path>"
    return value


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Smoke Python production-equivalent live ADB probe forwarding.

This optional developer tool invokes the Python planner route with wrapper
probe arguments and emits deterministic JSON evidence. It keeps probe input
inspection and device selection outside the smoke runner.
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


REPORT_KIND = "rust_production_equivalent_live_adb_probe_smoke"
REPORT_SCHEMA_VERSION = 1
CASE_ID = "rust_production_equivalent_live_adb_probe_forwarding"
WARNING_CODE = "device_profile_mismatch"
PROCESS_START_FAILURE = "production_equivalent_process_start_failed"
USAGE_FAILURE = "production_equivalent_usage_failed"
UNEXPECTED_EXIT = "production_equivalent_unexpected_exit"
OUTPUT_INCOMPATIBLE = "production_equivalent_output_incompatible"
STDERR_TEXT = "stderr_text"
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
    manufacturer: str | None = None,
    model: str | None = None,
    android_version: str | None = None,
    device_tags: Sequence[str] = (),
    bindings: Sequence[str] = (),
) -> list[str]:
    route = [
        sys.executable,
        "-m",
        "emuchef",
        "plan",
        "--planner-backend",
        "rust-production-equivalent",
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
    if manufacturer is not None:
        route.extend(["--manufacturer", manufacturer])
    if model is not None:
        route.extend(["--model", model])
    if android_version is not None:
        route.extend(["--android-version", android_version])
    for device_tag in device_tags:
        route.extend(["--device-tag", device_tag])
    for binding in bindings:
        route.extend(["--bind", binding])
    return route


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer-supplied inputs are passed as structured argv;
    # this smoke runner does not compose shell text.
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
    manufacturer: str | None = None,
    model: str | None = None,
    android_version: str | None = None,
    device_tags: Sequence[str] = (),
    bindings: Sequence[str] = (),
) -> dict[str, object]:
    route = build_cli_command(
        authored_root=authored_root,
        device_plan=device_plan,
        rust_planner_bin=rust_planner_bin,
        adb_path=adb_path,
        serial=serial,
        manufacturer=manufacturer,
        model=model,
        android_version=android_version,
        device_tags=device_tags,
        bindings=bindings,
    )
    try:
        completed = run_process(route)
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
        manufacturer=manufacturer,
        model=model,
        android_version=android_version,
        device_tags=device_tags,
        bindings=bindings,
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
            exit_class=USAGE_FAILURE,
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class=USAGE_FAILURE,
            failure_class=USAGE_FAILURE,
        )

    if completed.returncode not in {0, 1}:
        return _failure_result(
            exit_class=UNEXPECTED_EXIT,
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class=_classify_stderr(completed.stderr),
            failure_class=UNEXPECTED_EXIT,
        )

    if completed.stderr.strip():
        return _failure_result(
            exit_class="success_or_warning",
            stdout_class=_classify_failure_stdout(completed.stdout),
            stderr_class="text",
            failure_class=STDERR_TEXT,
        )

    stdout_class, planning_status = _classify_python_route_stdout(completed.stdout)
    if stdout_class == "python_compatible":
        return CaseResult(
            id=CASE_ID,
            status="passed",
            exit_class="success_or_warning",
            stdout_class=stdout_class,
            stderr_class="empty",
            planning_status=planning_status,
            device_profile_mismatch_seen=WARNING_CODE in completed.stdout,
        )

    return _failure_result(
        exit_class="success_or_warning",
        stdout_class=stdout_class,
        stderr_class="empty",
        planning_status=planning_status,
        failure_class=OUTPUT_INCOMPATIBLE,
    )


def process_start_failure_result() -> CaseResult:
    return _failure_result(
        exit_class=PROCESS_START_FAILURE,
        stdout_class="not_started",
        stderr_class="not_started",
        failure_class=PROCESS_START_FAILURE,
    )


def build_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    adb_path: str,
    serial_supplied: bool,
    manufacturer: str | None,
    model: str | None,
    android_version: str | None,
    device_tags: Sequence[str],
    bindings: Sequence[str],
    case_results: Sequence[CaseResult],
) -> dict[str, object]:
    passed = sum(1 for result in case_results if result.status == "passed")
    failed = sum(1 for result in case_results if result.status == "failed")
    skipped = sum(1 for result in case_results if result.status == "skipped")
    route_metadata = {
        "python_route": True,
        "rust_production_equivalent": True,
        "live_probe_requested": True,
        "probe_flag_present": True,
        "adb_path_supplied": bool(adb_path),
        "serial_supplied": bool(serial_supplied),
    }
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "passed" if failed == 0 else "failed",
        "inputs": {
            "authored_root": _stable_authored_root(authored_root),
            "device_plan": device_plan,
            "rust_planner_bin": _stable_basename(rust_planner_bin),
            "adb_path": _stable_adb_path(adb_path),
            "serial_supplied": bool(serial_supplied),
            "live_probe_requested": True,
            "context_overrides": {
                "manufacturer_supplied": manufacturer is not None,
                "model_supplied": model is not None,
                "android_version_supplied": android_version is not None,
                "device_tag_count": len(device_tags),
            },
            "binding_count": len(bindings),
        },
        "cases": [_case_report_item(result, route_metadata=route_metadata) for result in case_results],
        "summary": {
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
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
        prog="smoke_rust_production_equivalent_live_adb_probe.py",
        description="Smoke Python production-equivalent live ADB probe forwarding.",
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    parser.add_argument("--rust-planner-bin", required=True)
    parser.add_argument("--adb-path", required=True)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--manufacturer")
    parser.add_argument("--model")
    parser.add_argument("--android-version")
    parser.add_argument("--device-tag", action="append", default=[])
    parser.add_argument("--bind", action="append", default=[])
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
        manufacturer=args.manufacturer,
        model=args.model,
        android_version=args.android_version,
        device_tags=tuple(args.device_tag),
        bindings=tuple(args.bind),
    )
    sys.stdout.write(dumps_report(report))
    return smoke_exit_code(report)


def _case_report_item(
    result: CaseResult,
    *,
    route_metadata: Mapping[str, bool],
) -> dict[str, object]:
    item: dict[str, object] = {
        "id": result.id,
        "status": result.status,
        "exit_class": result.exit_class,
        "stdout_class": result.stdout_class,
        "stderr_class": result.stderr_class,
        "planning_status": result.planning_status,
        "device_profile_mismatch_seen": result.device_profile_mismatch_seen,
        "route_metadata": dict(route_metadata),
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
        status="failed",
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
    if _has_concise_success_or_warning(stdout_text, planning_status):
        return "python_compatible", planning_status
    if _has_yaml_planning_result(stdout_text) and planning_status in {None, "success", "warning"}:
        return "python_compatible", planning_status
    return "stdout_text", planning_status


def _classify_failure_stdout(stdout_text: str) -> str:
    stdout_class, _planning_status = _classify_python_route_stdout(stdout_text)
    return stdout_class


def _classify_stderr(stderr_text: str) -> str:
    return "empty" if not stderr_text.strip() else "text"


def _extract_planning_status(stdout_text: str) -> str | None:
    for pattern in (
        r"^Planning status:\s*([A-Za-z_]+)\s*$",
        r"^status:\s*([A-Za-z_]+)\s*$",
    ):
        match = re.search(pattern, stdout_text, flags=re.MULTILINE)
        if match is not None:
            return match.group(1).lower()
    return None


def _has_concise_success_or_warning(stdout_text: str, planning_status: str | None) -> bool:
    return "Planning status:" in stdout_text and planning_status in {"success", "warning"}


def _has_yaml_planning_result(stdout_text: str) -> bool:
    return bool(
        re.search(r"^kind:\s*planning_result\s*$", stdout_text, flags=re.MULTILINE)
        and re.search(r"^execution_plan:\s*$", stdout_text, flags=re.MULTILINE)
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
    if value == "authored":
        return "authored"
    if "/" in value:
        return Path(value).name or "<path>"
    return value or "<path>"


if __name__ == "__main__":
    raise SystemExit(main())

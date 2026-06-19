#!/usr/bin/env python3
"""Smoke production-equivalent detected-facts mismatch-warning parity.

This optional developer tool creates temporary detected-facts fixtures, invokes
the Python planner route for the explicit production-equivalent backend, and
emits a deterministic JSON report. It is fixture-backed only: selected device
facts come from local JSON files created by this process, not host device
inspection.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath
from typing import Mapping, Sequence


REPORT_KIND = "rust_production_equivalent_mismatch_warning_smoke"
REPORT_SCHEMA_VERSION = 1
DEVICE_PLAN_ID = "ayaneo.pocket_s_mini.base"
WARNING_CODE = "device_profile_mismatch"

PROCESS_START_FAILURE = "production_equivalent_process_start_failed"
USAGE_FAILURE = "production_equivalent_usage_failed"
UNEXPECTED_EXIT = "production_equivalent_unexpected_exit"
OUTPUT_INCOMPATIBLE = "production_equivalent_output_incompatible"
EXPECTED_WARNING_MISSING = "expected_mismatch_warning_missing"
UNEXPECTED_WARNING_PRESENT = "unexpected_mismatch_warning_present"


@dataclass(frozen=True, slots=True)
class SmokeCase:
    id: str
    fixture_name: str
    fixture_payload: Mapping[str, object]
    expected_exit_code: int
    expected_mismatch_warning: bool


@dataclass(frozen=True, slots=True)
class CaseResult:
    id: str
    status: str
    expected_mismatch_warning: bool
    mismatch_warning_seen: bool
    planning_status: str | None
    stdout_class: str
    failure_class: str | None


MATCHING_FIXTURE = {
    "serial": "P8AK-MATCHED",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

MANUFACTURER_MISMATCH_FIXTURE = {
    "serial": "P8AK-MANUFACTURER-MISMATCH",
    "manufacturer": "Valve",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

MODEL_MISMATCH_FIXTURE = {
    "serial": "P8AK-MODEL-MISMATCH",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "Steam Deck",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

ANDROID_MINIMUM_MISMATCH_FIXTURE = {
    "serial": "P8AK-ANDROID-MINIMUM-MISMATCH",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 12,
    "android_api_level": 32,
    "device_tags": ["detected_handheld"],
}

ANDROID_MINIMUM_MATCH_FIXTURE = {
    "serial": "P8AK-ANDROID-MINIMUM-MATCH",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

SMOKE_CASES = (
    SmokeCase(
        id="matched_profile",
        fixture_name="matched_profile.json",
        fixture_payload=MATCHING_FIXTURE,
        expected_exit_code=0,
        expected_mismatch_warning=False,
    ),
    SmokeCase(
        id="manufacturer_mismatch",
        fixture_name="manufacturer_mismatch.json",
        fixture_payload=MANUFACTURER_MISMATCH_FIXTURE,
        expected_exit_code=1,
        expected_mismatch_warning=True,
    ),
    SmokeCase(
        id="model_mismatch",
        fixture_name="model_mismatch.json",
        fixture_payload=MODEL_MISMATCH_FIXTURE,
        expected_exit_code=1,
        expected_mismatch_warning=True,
    ),
    SmokeCase(
        id="android_minimum_mismatch",
        fixture_name="android_minimum_mismatch.json",
        fixture_payload=ANDROID_MINIMUM_MISMATCH_FIXTURE,
        expected_exit_code=1,
        expected_mismatch_warning=True,
    ),
    SmokeCase(
        id="android_minimum_match",
        fixture_name="android_minimum_match.json",
        fixture_payload=ANDROID_MINIMUM_MATCH_FIXTURE,
        expected_exit_code=0,
        expected_mismatch_warning=False,
    ),
)


def build_cli_command(
    *,
    python_executable: str,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    fixture_path: Path,
) -> list[str]:
    return [
        python_executable,
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
        "--rust-detected-facts-json",
        str(fixture_path),
    ]


def run_process(argv: Sequence[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer inputs are passed as structured argv with the
    # platform shell disabled; this runner does not compose shell text.
    return subprocess.run(
        list(argv),
        cwd=str(cwd),
        check=False,
        text=True,
        capture_output=True,
    )


def run_case(
    case: SmokeCase,
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    python_executable: str,
    temp_root: Path,
    repo_root: Path,
) -> CaseResult:
    fixture_path = temp_root / case.fixture_name
    fixture_path.write_text(dumps_fixture(case.fixture_payload), encoding="utf-8")
    command = build_cli_command(
        python_executable=python_executable,
        authored_root=authored_root,
        device_plan=device_plan,
        rust_planner_bin=rust_planner_bin,
        fixture_path=fixture_path,
    )

    try:
        completed = run_process(command, cwd=repo_root)
    except OSError:
        return process_start_failure_result(case)

    return classify_completed_process(case, completed)


def run_smoke_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    repo_root: Path,
) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="emuchef-p8ak-production-mismatch-") as temp_dir:
        temp_root = Path(temp_dir)
        results = [
            run_case(
                case,
                authored_root=authored_root,
                device_plan=device_plan,
                rust_planner_bin=rust_planner_bin,
                python_executable=sys.executable,
                temp_root=temp_root,
                repo_root=repo_root,
            )
            for case in SMOKE_CASES
        ]
    return build_report(
        authored_root=authored_root,
        device_plan=device_plan,
        rust_planner_bin=rust_planner_bin,
        case_results=results,
    )


def classify_completed_process(
    case: SmokeCase,
    completed: subprocess.CompletedProcess[str],
) -> CaseResult:
    stdout_class, planning_status = classify_stdout(completed.stdout)
    mismatch_warning_seen = WARNING_CODE in completed.stdout

    if completed.returncode == 2 or _looks_like_usage_failure(completed.stderr):
        return _case_failure(
            case,
            mismatch_warning_seen=mismatch_warning_seen,
            planning_status=planning_status,
            stdout_class=stdout_class,
            failure_class=USAGE_FAILURE,
        )

    if completed.returncode != case.expected_exit_code:
        return _case_failure(
            case,
            mismatch_warning_seen=mismatch_warning_seen,
            planning_status=planning_status,
            stdout_class=stdout_class,
            failure_class=UNEXPECTED_EXIT,
        )

    if completed.stderr.strip() or stdout_class != "python_compatible":
        return _case_failure(
            case,
            mismatch_warning_seen=mismatch_warning_seen,
            planning_status=planning_status,
            stdout_class=stdout_class,
            failure_class=OUTPUT_INCOMPATIBLE,
        )

    if case.expected_mismatch_warning and not mismatch_warning_seen:
        return _case_failure(
            case,
            mismatch_warning_seen=False,
            planning_status=planning_status,
            stdout_class=stdout_class,
            failure_class=EXPECTED_WARNING_MISSING,
        )

    if not case.expected_mismatch_warning and mismatch_warning_seen:
        return _case_failure(
            case,
            mismatch_warning_seen=True,
            planning_status=planning_status,
            stdout_class=stdout_class,
            failure_class=UNEXPECTED_WARNING_PRESENT,
        )

    return CaseResult(
        id=case.id,
        status="passed",
        expected_mismatch_warning=case.expected_mismatch_warning,
        mismatch_warning_seen=mismatch_warning_seen,
        planning_status=planning_status,
        stdout_class=stdout_class,
        failure_class=None,
    )


def process_start_failure_result(case: SmokeCase) -> CaseResult:
    return _case_failure(
        case,
        mismatch_warning_seen=False,
        planning_status=None,
        stdout_class="process_not_started",
        failure_class=PROCESS_START_FAILURE,
    )


def build_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    case_results: Sequence[CaseResult],
) -> dict[str, object]:
    passed = sum(1 for result in case_results if result.status == "passed")
    failed = sum(1 for result in case_results if result.status == "failed")
    skipped = sum(1 for result in case_results if result.status == "skipped")
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "inputs": {
            "authored_root": _stable_input_path(authored_root),
            "device_plan": device_plan,
            "rust_planner_bin": _stable_basename(rust_planner_bin),
            "route_backend": "rust-production-equivalent",
            "route_output_mode": "python-compatible",
            "detected_facts_source": "temporary_fixture_json",
            "case_count": len(case_results),
        },
        "cases": [_case_report_item(result) for result in case_results],
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


def dumps_fixture(payload: Mapping[str, object]) -> str:
    return json.dumps(payload, indent=2, sort_keys=False) + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="smoke_rust_production_equivalent_mismatch_warning.py",
        description="Smoke production-equivalent detected-facts mismatch-warning parity.",
    )
    parser.add_argument("--rust-planner-bin", required=True)
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    report = run_smoke_report(
        authored_root=args.authored_root,
        device_plan=args.device_plan,
        rust_planner_bin=args.rust_planner_bin,
        repo_root=Path.cwd(),
    )
    sys.stdout.write(dumps_report(report))
    return smoke_exit_code(report)


def classify_stdout(stdout: str) -> tuple[str, str | None]:
    if not stdout.strip():
        return "stdout_empty", None
    try:
        json.loads(stdout)
    except json.JSONDecodeError:
        pass
    else:
        return "raw_json_stdout", None

    planning_status = _extract_planning_status(stdout)
    if "Planning status:" in stdout and planning_status in {"success", "warning"}:
        return "python_compatible", planning_status
    if _has_yaml_planning_result(stdout) and planning_status in {None, "success", "warning"}:
        return "python_compatible", planning_status
    return "stdout_text", planning_status


def _case_failure(
    case: SmokeCase,
    *,
    mismatch_warning_seen: bool,
    planning_status: str | None,
    stdout_class: str,
    failure_class: str,
) -> CaseResult:
    return CaseResult(
        id=case.id,
        status="failed",
        expected_mismatch_warning=case.expected_mismatch_warning,
        mismatch_warning_seen=mismatch_warning_seen,
        planning_status=planning_status,
        stdout_class=stdout_class,
        failure_class=failure_class,
    )


def _case_report_item(result: CaseResult) -> dict[str, object]:
    return {
        "id": result.id,
        "status": result.status,
        "expected_mismatch_warning": result.expected_mismatch_warning,
        "mismatch_warning_seen": result.mismatch_warning_seen,
        "planning_status": result.planning_status,
        "stdout_class": result.stdout_class,
        "failure_class": result.failure_class,
    }


def _looks_like_usage_failure(stderr_text: str) -> bool:
    return "usage:" in stderr_text or "unrecognized arguments:" in stderr_text


def _extract_planning_status(stdout_text: str) -> str | None:
    for pattern in (
        r"^Planning status:\s*([A-Za-z_]+)\s*$",
        r"^status:\s*([A-Za-z_]+)\s*$",
    ):
        match = re.search(pattern, stdout_text, flags=re.MULTILINE)
        if match is not None:
            return match.group(1).lower()
    return None


def _has_yaml_planning_result(stdout_text: str) -> bool:
    return bool(
        re.search(r"^kind:\s*planning_result\s*$", stdout_text, flags=re.MULTILINE)
        and re.search(r"^execution_plan:\s*$", stdout_text, flags=re.MULTILINE)
    )


def _stable_basename(value: str) -> str:
    if "\\" in value:
        return PureWindowsPath(value).name or value or "<path>"
    return Path(value).name or value or "<path>"


def _stable_input_path(value: str) -> str:
    if "\\" in value:
        name = PureWindowsPath(value).name
        if name == "authored":
            return "authored"
        return name or "<path>"
    path = Path(value)
    if path.name == "authored":
        return "authored"
    return path.name or value or "<path>"


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Smoke the Rust shadow detected-facts fixture harness directly.

This optional developer tool writes temporary detected-facts fixtures, invokes a
supplied ``emuchef-plan-shadow`` binary, and emits deterministic JSON evidence.
It does not route through the Python planner command, execute plans, probe
devices, or wire the fixture path into normal checks.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


REPORT_KIND = "rust_detected_facts_fixture_smoke"
REPORT_SCHEMA_VERSION = 1
DEVICE_PLAN_ID = "ayaneo.pocket_s_mini.base"


@dataclass(frozen=True, slots=True)
class ExplicitContext:
    manufacturer: str
    model: str
    android_version: int
    device_tags: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class SmokeCase:
    id: str
    fixture_name: str
    fixture_payload: Mapping[str, object]
    expected_exit_code: int
    expected_exit_class: str
    expected_result_status: str
    expected_warning_code: str | None
    expected_device_context: Mapping[str, object]
    explicit_context: ExplicitContext | None = None


@dataclass(frozen=True, slots=True)
class CaseResult:
    id: str
    status: str
    expected_warning_code: str | None
    actual_warning_codes: list[str]
    stdout_class: str
    exit_class: str
    stderr_class: str
    failure_summary: str | None = None


MATCHING_FIXTURE = {
    "serial": "P8S-MATCH",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

MISMATCHING_FIXTURE = {
    "serial": "P8S-MISMATCH",
    "manufacturer": "Valve",
    "brand": "Valve",
    "model": "Steam Deck",
    "android_version": 12,
    "android_api_level": 32,
    "device_tags": ["detected_mismatch"],
}

EXPLICIT_OVERRIDE = ExplicitContext(
    manufacturer="AYANEO",
    model="AYANEO Pocket S mini",
    android_version=13,
    device_tags=("explicit_handheld",),
)

SMOKE_CASES = (
    SmokeCase(
        id="matching_detected_facts",
        fixture_name="matching.json",
        fixture_payload=MATCHING_FIXTURE,
        expected_exit_code=0,
        expected_exit_class="success",
        expected_result_status="success",
        expected_warning_code=None,
        expected_device_context={
            "manufacturer": "AYANEO",
            "model": "AYANEO Pocket S mini",
            "android_version": 13,
            "android_api_level": 33,
            "device_tags": ["detected_handheld"],
        },
    ),
    SmokeCase(
        id="mismatching_detected_facts",
        fixture_name="mismatching.json",
        fixture_payload=MISMATCHING_FIXTURE,
        expected_exit_code=1,
        expected_exit_class="warning",
        expected_result_status="warning",
        expected_warning_code="device_profile_mismatch",
        expected_device_context={
            "manufacturer": "Valve",
            "model": "Steam Deck",
            "android_version": 12,
            "android_api_level": 32,
            "device_tags": ["detected_mismatch"],
        },
    ),
    SmokeCase(
        id="explicit_context_overrides_emitted_context",
        fixture_name="explicit-context.json",
        fixture_payload=MISMATCHING_FIXTURE,
        expected_exit_code=1,
        expected_exit_class="warning",
        expected_result_status="warning",
        expected_warning_code="device_profile_mismatch",
        expected_device_context={
            "manufacturer": "AYANEO",
            "model": "AYANEO Pocket S mini",
            "android_version": 13,
            "android_api_level": 32,
            "device_tags": ["explicit_handheld"],
        },
        explicit_context=EXPLICIT_OVERRIDE,
    ),
)


def build_shadow_command(
    *,
    rust_planner_bin: str,
    authored_root: str,
    fixture_path: Path,
    explicit_context: ExplicitContext | None = None,
) -> list[str]:
    command = [
        rust_planner_bin,
        "--authored-root",
        authored_root,
        "--device-plan",
        DEVICE_PLAN_ID,
        "--detected-facts-json",
        str(fixture_path),
    ]
    if explicit_context is not None:
        command.extend(
            [
                "--manufacturer",
                explicit_context.manufacturer,
                "--model",
                explicit_context.model,
                "--android-version",
                str(explicit_context.android_version),
            ]
        )
        for device_tag in explicit_context.device_tags:
            command.extend(["--device-tag", device_tag])
    return command


def run_process(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(argv),
        check=False,
        text=True,
        capture_output=True,
    )


def run_case(
    case: SmokeCase,
    *,
    authored_root: str,
    rust_planner_bin: str,
    temp_root: Path,
) -> CaseResult:
    fixture_path = temp_root / case.fixture_name
    fixture_path.write_text(json.dumps(case.fixture_payload, indent=2) + "\n", encoding="utf-8")
    command = build_shadow_command(
        rust_planner_bin=rust_planner_bin,
        authored_root=authored_root,
        fixture_path=fixture_path,
        explicit_context=case.explicit_context,
    )

    try:
        completed = run_process(command)
    except OSError:
        return CaseResult(
            id=case.id,
            status="fail",
            expected_warning_code=case.expected_warning_code,
            actual_warning_codes=[],
            stdout_class="process_not_started",
            exit_class="process_error",
            stderr_class="stderr_text",
            failure_summary="process did not start",
        )

    payload, stdout_class = _parse_planning_result(completed.stdout)
    actual_warning_codes = _warning_codes(payload)
    exit_class = _classify_exit(completed.returncode, case.expected_exit_code, case.expected_exit_class)
    stderr_class = "stderr_empty" if not completed.stderr.strip() else "stderr_text"
    passed = _case_passed(
        case=case,
        completed=completed,
        payload=payload,
        stdout_class=stdout_class,
        actual_warning_codes=actual_warning_codes,
        exit_class=exit_class,
        stderr_class=stderr_class,
    )

    return CaseResult(
        id=case.id,
        status="pass" if passed else "fail",
        expected_warning_code=case.expected_warning_code,
        actual_warning_codes=actual_warning_codes,
        stdout_class=stdout_class,
        exit_class=exit_class,
        stderr_class=stderr_class,
        failure_summary=None if passed else _stable_failure_summary(case, completed, stdout_class, exit_class, stderr_class),
    )


def run_smoke_report(*, authored_root: str, rust_planner_bin: str) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="emuchef-p8s-detected-facts-") as temp_dir:
        temp_root = Path(temp_dir)
        results = [
            run_case(
                case,
                authored_root=authored_root,
                rust_planner_bin=rust_planner_bin,
                temp_root=temp_root,
            )
            for case in SMOKE_CASES
        ]
    return build_report(
        authored_root=authored_root,
        rust_planner_bin=rust_planner_bin,
        case_results=results,
    )


def build_report(
    *,
    authored_root: str,
    rust_planner_bin: str,
    case_results: Sequence[CaseResult],
) -> dict[str, object]:
    passed = sum(1 for result in case_results if result.status == "pass")
    failed = len(case_results) - passed
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "pass" if failed == 0 else "fail",
        "inputs": {
            "authored_root": _stable_input_path(authored_root),
            "rust_planner_bin": _stable_basename(rust_planner_bin),
        },
        "cases": [
            {
                "id": result.id,
                "status": result.status,
                "expected_warning_code": result.expected_warning_code,
                "actual_warning_codes": result.actual_warning_codes,
                "stdout_class": result.stdout_class,
                "exit_class": result.exit_class,
                "stderr_class": result.stderr_class,
            }
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
        prog="smoke_rust_detected_facts_fixture.py",
        description="Smoke a supplied Rust shadow planner detected-facts fixture harness.",
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--rust-planner-bin", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    report = run_smoke_report(
        authored_root=args.authored_root,
        rust_planner_bin=args.rust_planner_bin,
    )
    sys.stdout.write(dumps_report(report))
    return smoke_exit_code(report)


def _parse_planning_result(stdout: str) -> tuple[Mapping[str, object] | None, str]:
    if not stdout.strip():
        return None, "stdout_empty"
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError:
        return None, "stdout_text"
    if not isinstance(payload, Mapping) or payload.get("kind") != "planning_result":
        return None, "json_not_planning_result"
    return payload, "planning_result_json"


def _warning_codes(payload: Mapping[str, object] | None) -> list[str]:
    if payload is None:
        return []
    warnings = payload.get("warnings")
    if not isinstance(warnings, list):
        return []
    return [
        str(warning["code"])
        for warning in warnings
        if isinstance(warning, Mapping) and isinstance(warning.get("code"), str)
    ]


def _classify_exit(actual: int, expected: int, expected_class: str) -> str:
    if actual != expected:
        return "unexpected_exit"
    return expected_class


def _case_passed(
    *,
    case: SmokeCase,
    completed: subprocess.CompletedProcess[str],
    payload: Mapping[str, object] | None,
    stdout_class: str,
    actual_warning_codes: Sequence[str],
    exit_class: str,
    stderr_class: str,
) -> bool:
    if completed.returncode != case.expected_exit_code:
        return False
    if stdout_class != "planning_result_json" or payload is None:
        return False
    if stderr_class != "stderr_empty":
        return False
    if payload.get("status") != case.expected_result_status:
        return False
    if _expected_warning_codes(case.expected_warning_code) != list(actual_warning_codes):
        return False
    execution_plan = payload.get("execution_plan")
    if not isinstance(execution_plan, Mapping):
        return False
    return execution_plan.get("device_context") == case.expected_device_context and exit_class != "unexpected_exit"


def _expected_warning_codes(expected_warning_code: str | None) -> list[str]:
    return [] if expected_warning_code is None else [expected_warning_code]


def _stable_failure_summary(
    case: SmokeCase,
    completed: subprocess.CompletedProcess[str],
    stdout_class: str,
    exit_class: str,
    stderr_class: str,
) -> str:
    parts = [
        f"expected_exit={case.expected_exit_code}",
        f"actual_exit={completed.returncode}",
        f"stdout_class={stdout_class}",
        f"exit_class={exit_class}",
        f"stderr_class={stderr_class}",
    ]
    return "; ".join(parts)


def _stable_basename(value: str) -> str:
    return Path(value).name or "<path>"


def _stable_input_path(value: str) -> str:
    path = Path(value)
    if path.is_absolute():
        return path.name or "<path>"
    return value


if __name__ == "__main__":
    raise SystemExit(main())

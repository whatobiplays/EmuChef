#!/usr/bin/env python3
"""Smoke Python rust-experimental detected-facts fixture forwarding.

This optional developer tool writes temporary detected-facts fixtures, invokes
``python3 -m emuchef plan --planner-backend rust-experimental`` with a supplied
``emuchef-plan-shadow`` binary, and emits deterministic JSON evidence. It does
not change default planning, probe devices, execute plans, or wire the fixture
route into readiness or normal runtime checks.
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


REPORT_KIND = "rust_experimental_detected_facts_fixture_smoke"
REPORT_SCHEMA_VERSION = 1
DEVICE_PLAN_ID = "ayaneo.pocket_s_mini.base"
WARNING_CODE = "device_profile_mismatch"


@dataclass(frozen=True, slots=True)
class SmokeCase:
    id: str
    fixture_name: str
    fixture_payload: Mapping[str, object]
    expected_exit_code: int
    expected_exit_class: str
    expected_warning_code: str | None
    output_file: bool = False


@dataclass(frozen=True, slots=True)
class CaseResult:
    id: str
    status: str
    expected_warning_code: str | None
    warning_observed: bool
    stdout_class: str
    exit_class: str
    stderr_class: str
    raw_rust_json_seen: bool
    output_file_written: bool | None = None
    output_warning_observed: bool | None = None
    failure_summary: str | None = None


MATCHING_FIXTURE = {
    "serial": "P8U-MATCH",
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}

MISMATCHING_FIXTURE = {
    "serial": "P8U-MISMATCH",
    "manufacturer": "Valve",
    "brand": "Valve",
    "model": "Steam Deck",
    "android_version": 12,
    "android_api_level": 32,
    "device_tags": ["detected_mismatch"],
}

SMOKE_CASES = (
    SmokeCase(
        id="matching_detected_facts_route",
        fixture_name="matching.json",
        fixture_payload=MATCHING_FIXTURE,
        expected_exit_code=0,
        expected_exit_class="success",
        expected_warning_code=None,
    ),
    SmokeCase(
        id="mismatching_detected_facts_route",
        fixture_name="mismatching.json",
        fixture_payload=MISMATCHING_FIXTURE,
        expected_exit_code=1,
        expected_exit_class="warning",
        expected_warning_code=WARNING_CODE,
        output_file=True,
    ),
)


def build_cli_command(
    *,
    python_executable: str,
    authored_root: str,
    rust_planner_bin: str,
    fixture_path: Path,
    output_path: Path | None = None,
) -> list[str]:
    command = [
        python_executable,
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
        DEVICE_PLAN_ID,
        "--rust-detected-facts-json",
        str(fixture_path),
    ]
    if output_path is not None:
        command.extend(["--output", str(output_path)])
    return command


def run_process(argv: Sequence[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer-supplied executable paths are passed as a
    # structured argv list with shell=False; the tool does not compose shell text.
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
    rust_planner_bin: str,
    python_executable: str,
    temp_root: Path,
    repo_root: Path,
) -> CaseResult:
    fixture_path = temp_root / case.fixture_name
    fixture_path.write_text(json.dumps(case.fixture_payload, indent=2) + "\n", encoding="utf-8")
    output_path = temp_root / f"{case.id}.planning-result.yaml" if case.output_file else None
    command = build_cli_command(
        python_executable=python_executable,
        authored_root=authored_root,
        rust_planner_bin=rust_planner_bin,
        fixture_path=fixture_path,
        output_path=output_path,
    )

    try:
        completed = run_process(command, cwd=repo_root)
    except OSError:
        return CaseResult(
            id=case.id,
            status="fail",
            expected_warning_code=case.expected_warning_code,
            warning_observed=False,
            stdout_class="process_not_started",
            exit_class="process_error",
            stderr_class="stderr_text",
            raw_rust_json_seen=False,
            output_file_written=False if case.output_file else None,
            output_warning_observed=False if case.output_file else None,
            failure_summary="process did not start",
        )

    stdout_class = classify_stdout(completed.stdout)
    stderr_class = classify_stderr(completed.stderr)
    exit_class = _classify_exit(completed.returncode, case.expected_exit_code, case.expected_exit_class)
    raw_rust_json_seen = stdout_class == "stdout_json"
    output_text = _read_output_file(output_path)
    output_file_written = output_path.exists() if output_path is not None else None
    output_warning_observed = _contains_warning(output_text, case.expected_warning_code) if output_path is not None else None
    warning_observed = _contains_warning(completed.stdout, case.expected_warning_code)
    passed = _case_passed(
        case=case,
        completed=completed,
        stdout_class=stdout_class,
        stderr_class=stderr_class,
        raw_rust_json_seen=raw_rust_json_seen,
        warning_observed=warning_observed,
        output_file_written=output_file_written,
        output_warning_observed=output_warning_observed,
    )

    return CaseResult(
        id=case.id,
        status="pass" if passed else "fail",
        expected_warning_code=case.expected_warning_code,
        warning_observed=warning_observed,
        stdout_class=stdout_class,
        exit_class=exit_class,
        stderr_class=stderr_class,
        raw_rust_json_seen=raw_rust_json_seen,
        output_file_written=output_file_written,
        output_warning_observed=output_warning_observed,
        failure_summary=None if passed else _stable_failure_summary(completed, stdout_class, exit_class, stderr_class),
    )


def run_smoke_report(
    *,
    authored_root: str,
    rust_planner_bin: str,
    python_executable: str,
    repo_root: Path,
) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="emuchef-p8u-rust-experimental-facts-") as temp_dir:
        temp_root = Path(temp_dir)
        results = [
            run_case(
                case,
                authored_root=authored_root,
                rust_planner_bin=rust_planner_bin,
                python_executable=python_executable,
                temp_root=temp_root,
                repo_root=repo_root,
            )
            for case in SMOKE_CASES
        ]
    return build_report(
        authored_root=authored_root,
        rust_planner_bin=rust_planner_bin,
        python_executable=python_executable,
        case_results=results,
    )


def build_report(
    *,
    authored_root: str,
    rust_planner_bin: str,
    python_executable: str,
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
            "python_executable": _stable_basename(python_executable),
            "route_backend": "rust-experimental",
            "route_output_mode": "python-compatible",
        },
        "cases": [_case_report_item(result) for result in case_results],
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
        prog="smoke_rust_experimental_detected_facts_fixture.py",
        description="Smoke Python rust-experimental detected-facts fixture forwarding.",
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--rust-planner-bin", required=True)
    parser.add_argument("--python-executable", default="python3")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    report = run_smoke_report(
        authored_root=args.authored_root,
        rust_planner_bin=args.rust_planner_bin,
        python_executable=args.python_executable,
        repo_root=Path.cwd(),
    )
    sys.stdout.write(dumps_report(report))
    return smoke_exit_code(report)


def classify_stdout(stdout: str) -> str:
    if not stdout.strip():
        return "stdout_empty"
    try:
        json.loads(stdout)
    except json.JSONDecodeError:
        if "Planning status:" in stdout:
            return "python_summary"
        if "kind: planning_result" in stdout:
            return "python_yaml"
        return "stdout_text"
    return "stdout_json"


def classify_stderr(stderr: str) -> str:
    return "stderr_empty" if not stderr.strip() else "stderr_text"


def _case_report_item(result: CaseResult) -> dict[str, object]:
    item: dict[str, object] = {
        "id": result.id,
        "status": result.status,
        "expected_warning_code": result.expected_warning_code,
        "warning_observed": result.warning_observed,
        "stdout_class": result.stdout_class,
        "exit_class": result.exit_class,
        "stderr_class": result.stderr_class,
        "raw_rust_json_seen": result.raw_rust_json_seen,
    }
    if result.output_file_written is not None:
        item["output_file_written"] = result.output_file_written
    if result.output_warning_observed is not None:
        item["output_warning_observed"] = result.output_warning_observed
    return item


def _case_passed(
    *,
    case: SmokeCase,
    completed: subprocess.CompletedProcess[str],
    stdout_class: str,
    stderr_class: str,
    raw_rust_json_seen: bool,
    warning_observed: bool,
    output_file_written: bool | None,
    output_warning_observed: bool | None,
) -> bool:
    if completed.returncode != case.expected_exit_code:
        return False
    if stdout_class != "python_summary" or raw_rust_json_seen:
        return False
    if stderr_class != "stderr_empty":
        return False
    if case.expected_warning_code is None:
        return not warning_observed
    if not warning_observed:
        return False
    if case.output_file:
        return output_file_written is True and output_warning_observed is True
    return True


def _classify_exit(actual: int, expected: int, expected_class: str) -> str:
    if actual != expected:
        return "unexpected_exit"
    return expected_class


def _contains_warning(text: str, warning_code: str | None) -> bool:
    if warning_code is None:
        return WARNING_CODE in text
    return warning_code in text


def _read_output_file(path: Path | None) -> str:
    if path is None or not path.exists():
        return ""
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def _stable_failure_summary(
    completed: subprocess.CompletedProcess[str],
    stdout_class: str,
    exit_class: str,
    stderr_class: str,
) -> str:
    return (
        f"exit_class={exit_class}; stdout_class={stdout_class}; "
        f"stderr_class={stderr_class}; returncode={completed.returncode}"
    )


def _stable_basename(value: str) -> str:
    return Path(value).name or value


def _stable_input_path(value: str) -> str:
    path = Path(value)
    if path.name == "authored":
        return "authored"
    return path.name or value


if __name__ == "__main__":
    raise SystemExit(main())

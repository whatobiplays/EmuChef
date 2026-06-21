#!/usr/bin/env python3
"""Smoke launcher-injected Rust planner entrypoint behavior.

This developer smoke verifies that ``emuchef plan`` starts the executable
supplied through ``--rust-planner-bin`` when using the explicit
``rust-production-equivalent`` route. The supplied executable may be an
argv0-observing wrapper around the real Rust planner binary; this tool proves
the launcher-supplied entrypoint was invoked, not that the wrapped binary path
was used directly.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Mapping, Sequence


REPORT_KIND = "rust_launcher_injected_planner_smoke"
REPORT_SCHEMA_VERSION = 1
PLANNER_BACKEND = "rust-production-equivalent"
OBSERVATION_ENV_VAR = "EMUCHEF_P8BA_ARGV0_OBSERVATION_PATH"

DETECTED_FACTS_FIXTURE = {
    "manufacturer": "AYANEO",
    "brand": "AYANEO",
    "model": "AYANEO Pocket S mini",
    "android_version": 13,
    "android_api_level": 33,
    "device_tags": ["detected_handheld"],
}


@dataclass(frozen=True, slots=True)
class LauncherPathValidation:
    path_was_absolute: bool
    path_exists: bool
    path_is_file: bool
    path_executable: bool

    @property
    def passed(self) -> bool:
        return self.path_was_absolute and self.path_exists and self.path_is_file and self.path_executable


@dataclass(frozen=True, slots=True)
class PlannerRunObservation:
    process_started: bool
    plan_succeeded: bool
    observed_argv0: str | None


def validate_launcher_path(raw_path: str) -> LauncherPathValidation:
    """Validate the launcher-supplied planner path without expanding it.

    The raw value must already be absolute. This intentionally rejects relative
    paths before user-home, symlink, or working-directory normalization can hide
    the form originally supplied by the launcher layer.
    """

    path = Path(raw_path)
    path_was_absolute = path.is_absolute()
    if not path_was_absolute:
        return LauncherPathValidation(
            path_was_absolute=False,
            path_exists=False,
            path_is_file=False,
            path_executable=False,
        )

    try:
        path_exists = path.exists()
        path_is_file = path.is_file() if path_exists else False
        path_executable = path_is_file and os.access(path, os.X_OK)
    except OSError:
        path_exists = False
        path_is_file = False
        path_executable = False

    return LauncherPathValidation(
        path_was_absolute=path_was_absolute,
        path_exists=path_exists,
        path_is_file=path_is_file,
        path_executable=path_executable,
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
        PLANNER_BACKEND,
        "--rust-planner-bin",
        rust_planner_bin,
        "--authored-root",
        authored_root,
        "--device-plan",
        device_plan,
        "--rust-detected-facts-json",
        str(fixture_path),
    ]


def run_process(
    command: Sequence[str],
    *,
    cwd: Path,
    observation_path: Path,
) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: Developer inputs are passed as structured arguments with
    # the platform shell disabled; this runner does not compose shell text.
    child_env = os.environ.copy()
    child_env[OBSERVATION_ENV_VAR] = str(observation_path)
    return subprocess.run(
        list(command),
        cwd=str(cwd),
        check=False,
        text=True,
        capture_output=True,
        env=child_env,
    )


def run_help_process(command: Sequence[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(command),
        cwd=str(cwd),
        check=False,
        text=True,
        capture_output=True,
    )


def check_explicit_python_backend_available(*, python_executable: str, repo_root: Path) -> bool:
    """Return whether CLI help still exposes explicit Python planner fallback."""

    try:
        completed = run_help_process([python_executable, "-m", "emuchef", "plan", "--help"], cwd=repo_root)
    except OSError:
        return False
    help_text = f"{completed.stdout}\n{completed.stderr}"
    return completed.returncode == 0 and "--planner-backend" in help_text and "python" in help_text


def run_smoke_report(
    *,
    authored_root: str,
    device_plan: str,
    rust_planner_bin: str,
    repo_root: Path,
    generated_at: str | None = None,
) -> dict[str, object]:
    validation = validate_launcher_path(rust_planner_bin)
    python_bypass_available = check_explicit_python_backend_available(
        python_executable=sys.executable,
        repo_root=repo_root,
    )
    observation = PlannerRunObservation(process_started=False, plan_succeeded=False, observed_argv0=None)

    if validation.passed:
        with tempfile.TemporaryDirectory(prefix="emuchef-p8ba-launcher-planner-") as temp_dir:
            temp_root = Path(temp_dir)
            fixture_path = temp_root / "detected-facts.json"
            observation_path = temp_root / "argv0-observation.json"
            fixture_path.write_text(dumps_fixture(DETECTED_FACTS_FIXTURE), encoding="utf-8")
            command = build_cli_command(
                python_executable=sys.executable,
                authored_root=authored_root,
                device_plan=device_plan,
                rust_planner_bin=rust_planner_bin,
                fixture_path=fixture_path,
            )
            try:
                completed = run_process(command, cwd=repo_root, observation_path=observation_path)
            except OSError:
                observation = PlannerRunObservation(
                    process_started=False,
                    plan_succeeded=False,
                    observed_argv0=None,
                )
            else:
                observation = PlannerRunObservation(
                    process_started=True,
                    plan_succeeded=completed.returncode == 0,
                    observed_argv0=load_observed_argv0(observation_path),
                )

    return build_report(
        generated_at=generated_at or utc_timestamp(),
        device_plan=device_plan,
        validation=validation,
        python_bypass_available=python_bypass_available,
        observation=observation,
        rust_planner_bin=rust_planner_bin,
    )


def build_report(
    *,
    generated_at: str,
    device_plan: str,
    validation: LauncherPathValidation,
    python_bypass_available: bool,
    observation: PlannerRunObservation,
    rust_planner_bin: str,
) -> dict[str, object]:
    argv0_corresponds = observation.observed_argv0 == rust_planner_bin
    no_implicit_fallback_sources_used = validation.passed and observation.process_started and argv0_corresponds
    checks = [
        _check(
            "launcher_supplied_path_absolute",
            validation.path_was_absolute,
            "launcher_path_not_absolute",
        ),
        _check("launcher_supplied_path_exists", validation.path_exists, "launcher_path_missing"),
        _check("launcher_supplied_path_file", validation.path_is_file, "launcher_path_not_file"),
        _check(
            "launcher_supplied_path_executable",
            validation.path_executable,
            "launcher_path_not_executable",
        ),
        _check(
            "argv0_corresponds_to_launcher_path",
            argv0_corresponds,
            "launcher_entrypoint_not_observed",
        ),
        _check(
            "known_fixture_plan_succeeded",
            observation.plan_succeeded,
            "known_fixture_plan_failed",
        ),
        _check(
            "explicit_python_backend_bypass_available",
            python_bypass_available,
            "explicit_python_backend_not_exposed",
        ),
        _check(
            "no_implicit_fallback_sources_used",
            no_implicit_fallback_sources_used,
            "explicit_launcher_entrypoint_not_observed",
        ),
    ]
    passed = sum(1 for check in checks if check["passed"] is True)
    failed = len(checks) - passed
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "generated_at": generated_at,
        "summary": {
            "passed": passed,
            "failed": failed,
        },
        "inputs": {
            "planner_backend": PLANNER_BACKEND,
            "device_plan": device_plan,
            "launcher_supplied_planner_path": True,
            "path_was_absolute": validation.path_was_absolute,
            "path_exists": validation.path_exists,
            "path_is_file": validation.path_is_file,
            "path_executable": validation.path_executable,
            "argv0_corresponds_to_launcher_path": argv0_corresponds,
            "explicit_python_bypass_checked": True,
            "explicit_python_bypass_check_mode": "cli_help_static",
            "detected_facts_source": "temporary_fixture_json",
            "launcher_entrypoint_observation": "external_wrapper",
        },
        "checks": checks,
        "redaction": {
            "full_paths_omitted": True,
            "process_invocation_omitted": True,
            "process_output_omitted": True,
            "runtime_context_omitted": True,
            "device_identifiers_omitted": True,
            "sensitive_values_omitted": True,
        },
        "artifacts": {
            "argv0_basename": stable_basename(observation.observed_argv0),
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


def load_observed_argv0(observation_path: Path) -> str | None:
    try:
        payload = json.loads(observation_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, Mapping):
        return None
    value = payload.get("argv0")
    if not isinstance(value, str) or not value:
        return None
    return value


def stable_basename(value: str | None) -> str | None:
    if value is None:
        return None
    return Path(value).name or None


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="smoke_launcher_injected_planner.py",
        description="Smoke launcher-injected Rust planner entrypoint behavior.",
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    parser.add_argument("--rust-planner-bin", required=True)
    parser.add_argument("--output-report", required=True)
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
    Path(args.output_report).write_text(dumps_report(report), encoding="utf-8")
    return smoke_exit_code(report)


def _check(name: str, passed: bool, failure_code: str) -> dict[str, object]:
    result: dict[str, object] = {
        "name": name,
        "passed": bool(passed),
    }
    if not passed:
        result["failure_code"] = failure_code
    return result


if __name__ == "__main__":
    raise SystemExit(main())

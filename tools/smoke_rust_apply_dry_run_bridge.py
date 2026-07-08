#!/usr/bin/env python3
"""Smoke the Python-to-Rust apply dry-run bridge.

This developer smoke validates that ``emuchef apply --dry-run`` can delegate to
the Rust backend through either an explicit ``--rust-apply-bin`` or the default
packaged route. It captures only stable output metadata, not full process logs,
so the resulting report can be used as readiness evidence without embedding
command output.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import sys
from pathlib import Path
from typing import Mapping, Sequence


REPORT_KIND = "rust_apply_dry_run_bridge_smoke"
REPORT_SCHEMA_VERSION = 1
EXPLICIT_ROUTE = "explicit_rust_apply_bin"
DEFAULT_PACKAGED_ROUTE = "default_packaged"


def run_smoke_report(
    *,
    rust_apply_bin: str | None,
    plan_file: str,
    use_default_packaged_route: bool = False,
) -> dict[str, object]:
    """Validate inputs, invoke the Python CLI bridge, and return a JSON report."""

    rust_apply_path = Path(rust_apply_bin).expanduser().resolve() if rust_apply_bin is not None else None
    plan_path = Path(plan_file).expanduser().resolve()
    route, route_reason = _resolve_route(
        rust_apply_bin=rust_apply_bin,
        use_default_packaged_route=use_default_packaged_route,
    )
    checks = [
        _check(
            "route_choice_valid",
            route is not None,
            route_reason or "route_choice_invalid",
        ),
        _check("plan_file_exists", plan_path.exists(), "plan_file_missing"),
        _check("plan_file_is_file", plan_path.is_file(), "plan_file_not_file"),
    ]
    if route == EXPLICIT_ROUTE and rust_apply_path is not None:
        checks[1:1] = [
            _check("rust_apply_bin_exists", rust_apply_path.exists(), "rust_apply_bin_missing"),
            _check("rust_apply_bin_is_file", rust_apply_path.is_file(), "rust_apply_bin_not_file"),
            _check(
                "rust_apply_bin_executable",
                rust_apply_path.is_file() and os.access(rust_apply_path, os.X_OK),
                "rust_apply_bin_not_executable",
            ),
        ]
    validation_passed = all(check["status"] == "pass" for check in checks)
    command = build_logical_command(route=route, rust_apply_bin=rust_apply_path, plan_file=plan_path)

    returncode: int | None = None
    stdout_present = False
    stderr_present = False
    bridge_reason = "input_validation_failed"

    if validation_passed:
        returncode, stdout_present, stderr_present, bridge_reason = invoke_python_cli_bridge(
            route=route,
            rust_apply_bin=rust_apply_path,
            plan_file=plan_path,
        )

    bridge_passed = returncode == 0
    checks.append(
        _check(
            "python_bridge_invocation_succeeded",
            bridge_passed,
            bridge_reason,
        )
    )
    passed = all(check["status"] == "pass" for check in checks)

    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "route": route,
        "status": "passed" if passed else "failed",
        "inputs": {
            "rust_apply_bin": str(rust_apply_path) if rust_apply_path is not None else None,
            "plan_file": str(plan_path),
        },
        "command": command,
        "result": {
            "returncode": returncode,
            "stdout_present": stdout_present,
            "stderr_present": stderr_present,
        },
        "checks": checks,
    }


def _resolve_route(*, rust_apply_bin: str | None, use_default_packaged_route: bool) -> tuple[str | None, str | None]:
    """Return the selected route and a stable rejection reason, if invalid."""

    explicit_requested = rust_apply_bin is not None
    if explicit_requested and use_default_packaged_route:
        return None, "mutually_exclusive_route_choice"
    if explicit_requested:
        return EXPLICIT_ROUTE, None
    if use_default_packaged_route:
        return DEFAULT_PACKAGED_ROUTE, None
    return None, "missing_route_choice"


def build_logical_command(*, route: str | None, rust_apply_bin: Path | None, plan_file: Path) -> list[str]:
    """Return the stable CLI command shape represented by this smoke."""

    command = [
        "emuchef",
        "apply",
        "--plan-file",
        str(plan_file),
        "--dry-run",
    ]
    if route == EXPLICIT_ROUTE and rust_apply_bin is not None:
        command.extend(["--rust-apply-bin", str(rust_apply_bin)])
    return command


def invoke_python_cli_bridge(
    *,
    route: str | None,
    rust_apply_bin: Path | None,
    plan_file: Path,
) -> tuple[int, bool, bool, str]:
    """Invoke ``emuchef.cli.main`` and capture only output presence metadata."""

    from emuchef.cli import main as cli_main

    argv = [
        "apply",
        "--plan-file",
        str(plan_file),
        "--dry-run",
    ]
    if route == EXPLICIT_ROUTE and rust_apply_bin is not None:
        argv.extend(["--rust-apply-bin", str(rust_apply_bin)])

    stdout = io.StringIO()
    stderr = io.StringIO()
    try:
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            returncode = cli_main(argv)
    except Exception:
        return 1, bool(stdout.getvalue()), True, "python_bridge_invocation_raised"

    return (
        returncode,
        bool(stdout.getvalue()),
        bool(stderr.getvalue()),
        "bridge_returncode_nonzero" if returncode != 0 else "python_bridge_invocation_succeeded",
    )


def smoke_exit_code(report: Mapping[str, object]) -> int:
    """Return the process exit code represented by a smoke report."""

    if report.get("status") == "passed":
        return 0
    result = report.get("result")
    if isinstance(result, Mapping):
        returncode = result.get("returncode")
        if isinstance(returncode, int) and not isinstance(returncode, bool) and returncode > 0:
            return returncode
    return 1


def dumps_report(report: Mapping[str, object]) -> str:
    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="smoke_rust_apply_dry_run_bridge.py",
        description="Smoke the Python CLI Rust apply dry-run bridge.",
    )
    parser.add_argument("--rust-apply-bin")
    parser.add_argument("--use-default-packaged-route", action="store_true")
    parser.add_argument("--plan-file", required=True)
    parser.add_argument("--output-report", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    report = run_smoke_report(
        rust_apply_bin=args.rust_apply_bin,
        plan_file=args.plan_file,
        use_default_packaged_route=args.use_default_packaged_route,
    )
    output_report = Path(args.output_report)
    output_report.parent.mkdir(parents=True, exist_ok=True)
    output_report.write_text(dumps_report(report), encoding="utf-8")
    return smoke_exit_code(report)


def _check(check_id: str, passed: bool, reason: str) -> dict[str, object]:
    check: dict[str, object] = {
        "id": check_id,
        "status": "pass" if passed else "fail",
    }
    if not passed:
        check["reason"] = reason
    return check


if __name__ == "__main__":
    raise SystemExit(main())

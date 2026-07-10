#!/usr/bin/env python3
"""Static readiness report for the canonical Rust CLI and runtime cutover.

This developer-only gate consolidates static prerequisites, supplied manual
evidence reports, advisory manual evidence commands, and intentionally remaining
blockers. It does not execute the comparison matrix, smoke runner, Cargo, npm,
ADB, planner code, or runtime paths.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Callable


REPORT_KIND = "rust_runtime_cutover_readiness_check"
REPORT_SCHEMA_VERSION = 2
HISTORICAL_P8_REPORT_SCHEMA_VERSION = 1
SCENARIO_MATRIX_SCHEMA_VERSION = 1
DEVICE_CONTEXT_FIELDS = ("manufacturer", "model", "android_version", "device_tags")
DEVICE_CONTEXT_FIELD_SET = set(DEVICE_CONTEXT_FIELDS)
P8AJ_EVIDENCE_ID = "p8aj_live_probe"
P8AK_EVIDENCE_ID = "p8ak_mismatch_warning"
P8BC_EVIDENCE_ID = "p8bc_launcher_injected_planner"
P8BU_EVIDENCE_ID = "p8bu_rust_apply_dry_run_bridge"
P8AJ_REPORT_KIND = "rust_production_equivalent_live_adb_probe_smoke"
P8AK_REPORT_KIND = "rust_production_equivalent_mismatch_warning_smoke"
P8BC_REPORT_KIND = "rust_launcher_injected_planner_smoke"
P8BU_REPORT_KIND = "rust_apply_dry_run_bridge_smoke"
P8BU_EXPLICIT_ROUTE = "explicit_rust_apply_bin"
P8BU_DEFAULT_PACKAGED_ROUTE = "default_packaged"
DEFAULT_CLI_BACKEND_BLOCKER_ID = "default_cli_backend_still_python"
EXPLICIT_RUST_APPLY_DRY_RUN_CAPABILITY_ID = "explicit_rust_apply_dry_run_bridge"
DEFAULT_RUST_APPLY_DRY_RUN_CAPABILITY_ID = "default_rust_apply_dry_run_route"
RUST_APPLY_DRY_RUN_BRIDGE_EVIDENCE_ID = "rust_apply_dry_run_bridge_evidence"
REAL_DEVICE_PROBING_BLOCKER_ID = "real_device_probing_not_cut_over"
MISMATCH_WARNING_BLOCKER_ID = "detected_device_profile_mismatch_warning_not_cut_over"
PACKAGED_LAUNCHER_INJECTION_BLOCKER_ID = "packaged_launcher_injection_evidence_not_accepted"
PACKAGED_RELEASE_BLOCKER_ID = "packaged_release_not_ready"
REAL_DEVICE_PLAN_EVIDENCE_BLOCKER_ID = "real_device_plan_probe_evidence"
MISMATCH_WARNING_EVIDENCE_BLOCKER_ID = "device_profile_mismatch_warning_evidence"
REAL_DEVICE_APPLY_EVIDENCE_BLOCKER_ID = "real_device_apply_evidence"
NETWORK_ARTIFACT_BLOCKER_ID = "network_artifact_downloads_not_cut_over"
P8AK_REQUIRED_PASSING_CASES = (
    "matched_profile",
    "manufacturer_mismatch",
    "model_mismatch",
    "android_minimum_mismatch",
    "android_minimum_match",
)
SENSITIVE_EVIDENCE_KEYS = {
    "command",
    "argv",
    "raw_command",
    "environment",
    "env",
    "stdout",
    "stderr",
    "raw_stdout",
    "raw_stderr",
    "serial",
    "device_serial",
    "planner_path",
    "absolute_path",
    "launcher_supplied_absolute_path",
    "cwd",
    "home",
}
P8BC_REQUIRED_TOP_LEVEL_KEYS = (
    "kind",
    "schema_version",
    "generated_at",
    "summary",
    "inputs",
    "checks",
    "redaction",
    "artifacts",
)
P8BC_REQUIRED_CHECKS = (
    "launcher_supplied_path_absolute",
    "launcher_supplied_path_exists",
    "launcher_supplied_path_file",
    "launcher_supplied_path_executable",
    "argv0_corresponds_to_launcher_path",
    "known_fixture_plan_succeeded",
    "no_implicit_fallback_sources_used",
)
P8BC_REQUIRED_REDACTION_FLAGS = (
    "full_paths_omitted",
    "process_invocation_omitted",
    "process_output_omitted",
    "runtime_context_omitted",
    "device_identifiers_omitted",
    "sensitive_values_omitted",
)
P8BC_REQUIRED_INPUT_VALUES = {
    "planner_backend": "rust-production-equivalent",
    "launcher_supplied_planner_path": True,
    "path_was_absolute": True,
    "path_exists": True,
    "path_executable": True,
    "argv0_corresponds_to_launcher_path": True,
}
P8BU_REQUIRED_TOP_LEVEL_KEYS = (
    "inputs",
    "command",
    "result",
    "checks",
)
P8BU_REQUIRED_CHECKS = (
    "rust_apply_bin_exists",
    "plan_file_exists",
    "python_bridge_invocation_succeeded",
)
P8BU_DEFAULT_REQUIRED_CHECKS = (
    "plan_file_exists",
    "python_bridge_invocation_succeeded",
)
P8BU_BASE_REQUIRED_COMMAND_TOKENS = (
    "apply",
    "--plan-file",
    "--dry-run",
)
P8BU_EXPLICIT_REQUIRED_COMMAND_TOKENS = (*P8BU_BASE_REQUIRED_COMMAND_TOKENS, "--rust-apply-bin")

REQUIRED_ARTIFACTS = (
    "tools/compare_rust_python_plan.py",
    "tools/smoke_rust_shadow_cli_matrix.py",
    "docs/rust-planner-cutover-readiness.md",
    "docs/rust-planner-parity-boundary.md",
    "docs/rust-cli-executor-parity.md",
    "docs/adr/0002-rust-planner-cli-output-compatibility.md",
    "src/emuchef/cli.py",
    "tests/test_cli.py",
    "tests/test_compare_rust_python_plan.py",
    "tests/test_smoke_rust_shadow_cli_matrix.py",
)

READINESS_DOC_REFERENCES = (
    ("plan_parity_scenarios", "tools/plan_parity_scenarios.json"),
    ("compare_rust_python_plan", "tools/compare_rust_python_plan.py"),
    ("smoke_rust_shadow_cli_matrix", "tools/smoke_rust_shadow_cli_matrix.py"),
    ("output_compatibility_adr", "docs/adr/0002-rust-planner-cli-output-compatibility.md"),
    ("rust_shadow", "rust-shadow"),
    ("rust_experimental", "rust-experimental"),
    ("python_planner", "Python planner"),
    ("default", "default"),
    ("executor_apply", "executor/apply"),
    ("adb", "ADB"),
    ("tauri", "Tauri"),
    ("python_planner_deletion", "Python planner deletion"),
)

CLI_BACKEND_TOKENS = (
    ("planner_backend", "--planner-backend"),
    ("python_compatible_output", "python-compatible"),
    ("rust_shadow", "rust-shadow"),
    ("rust_experimental", "rust-experimental"),
    ("rust_planner_bin", "--rust-planner-bin"),
    ("rust_shadow_output", "--rust-shadow-output"),
)

HISTORICAL_P8_MANUAL_EVIDENCE = (
    {
        "id": "p7p_python_rust_comparison_matrix",
        "command": (
            "python3 tools/compare_rust_python_plan.py "
            "--scenario-matrix tools/plan_parity_scenarios.json "
            "--authored-root authored"
        ),
    },
    {
        "id": "p8h_rust_experimental_matrix_smoke",
        "command": (
            "python3 tools/smoke_rust_shadow_cli_matrix.py "
            "--scenario-matrix tools/plan_parity_scenarios.json "
            "--authored-root authored "
            "--rust-planner-bin <path-to-emuchef-plan-shadow> "
            "--planner-backend rust-experimental"
        ),
    },
    {
        "id": "focused_python_tests",
        "command": (
            "python3 -m unittest tests.test_cli && "
            "python3 -m unittest tests.test_smoke_rust_shadow_cli_matrix && "
            "python3 -m unittest tests.test_compare_rust_python_plan"
        ),
    },
    {
        "id": "rust_tauri_checks",
        "command": (
            "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml shadow && "
            "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml planner && "
            "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml && "
            "cd apps/config-editor/src-tauri && cargo test && "
            "cd ../ && npm run check:rust-runtime"
        ),
    },
    {
        "id": "p8aj_rust_production_equivalent_live_probe_smoke",
        "command": (
            "PYTHONPATH=src rtk python3 tools/smoke_rust_production_equivalent_live_adb_probe.py "
            "--rust-planner-bin <path-to-emuchef-plan-shadow> "
            "--adb-path <path-to-adb> "
            "--serial <device-serial> "
            "--authored-root authored "
            "--device-plan <device-plan>"
        ),
    },
    {
        "id": "p8ak_rust_production_equivalent_mismatch_warning_smoke",
        "command": (
            "PYTHONPATH=src rtk python3 tools/smoke_rust_production_equivalent_mismatch_warning.py "
            "--rust-planner-bin <path-to-emuchef-plan-shadow> "
            "--authored-root authored "
            "--device-plan ayaneo.pocket_s_mini.base"
        ),
    },
    {
        "id": "p8bc_launcher_injected_planner_smoke",
        "command": (
            "PYTHONPATH=src rtk python3 tools/smoke_launcher_injected_planner.py "
            "--authored-root authored "
            "--device-plan ayaneo.pocket_s_mini.base "
            "--rust-planner-bin <absolute-path-to-launcher-supplied-planner> "
            "--output-report <path-to-output-report>"
        ),
    },
    {
        "id": P8BU_EVIDENCE_ID,
        "command": (
            "PYTHONPATH=src rtk python3 tools/smoke_rust_apply_dry_run_bridge.py "
            "--use-default-packaged-route "
            "--plan-file tests/fixtures/apply_dry_run/minimal_execution_plan.yaml "
            "--output-report <path-to-output-report>"
        ),
    },
)

REQUIRED_MANUAL_EVIDENCE = (
    {
        "id": REAL_DEVICE_PLAN_EVIDENCE_BLOCKER_ID,
        "command": (
            "emuchef plan --authored-root authored --device-plan <device-plan> "
            "--adb <path-to-adb> --serial <device-serial>"
        ),
    },
    {
        "id": MISMATCH_WARNING_EVIDENCE_BLOCKER_ID,
        "command": (
            "emuchef plan --authored-root authored --device-plan <mismatched-device-plan> "
            "--adb <path-to-adb> --serial <device-serial>"
        ),
    },
    {
        "id": REAL_DEVICE_APPLY_EVIDENCE_BLOCKER_ID,
        "command": (
            "emuchef apply --plan-file <device-safe-plan> --adb <path-to-adb> "
            "--serial <device-serial>"
        ),
    },
)

REMAINING_BLOCKERS = (
    {
        "id": DEFAULT_CLI_BACKEND_BLOCKER_ID,
        "status": "resolved",
    },
    {
        "id": EXPLICIT_RUST_APPLY_DRY_RUN_CAPABILITY_ID,
        "status": "resolved",
    },
    {
        "id": DEFAULT_RUST_APPLY_DRY_RUN_CAPABILITY_ID,
        "status": "resolved",
    },
    {
        "id": "executor_apply_not_cut_over",
        "status": "resolved",
    },
    {
        "id": REAL_DEVICE_PROBING_BLOCKER_ID,
        "status": "resolved",
    },
    {
        "id": MISMATCH_WARNING_BLOCKER_ID,
        "status": "resolved",
    },
    {
        "id": PACKAGED_LAUNCHER_INJECTION_BLOCKER_ID,
        "status": "resolved",
    },
    {
        "id": REAL_DEVICE_PLAN_EVIDENCE_BLOCKER_ID,
        "status": "missing",
    },
    {
        "id": MISMATCH_WARNING_EVIDENCE_BLOCKER_ID,
        "status": "missing",
    },
    {
        "id": REAL_DEVICE_APPLY_EVIDENCE_BLOCKER_ID,
        "status": "missing",
    },
    {
        "id": NETWORK_ARTIFACT_BLOCKER_ID,
        "status": "blocked",
    },
    {
        "id": PACKAGED_RELEASE_BLOCKER_ID,
        "status": "blocked",
    },
)


def _status_explanation() -> dict[str, Any]:
    """Return descriptive context for the intentionally blocked top-level status."""

    return {
        "top_level_status": "blocked",
        "evidence_accepted_is_not_release_ready": True,
        "evidence_accepted_meaning": (
            "Accepted schema-v1 P8 evidence validates historical report shape only; "
            "it does not satisfy current schema-v2 blockers."
        ),
        "top_level_blocked_reason": (
            "Code-level local/BYO Rust runtime cutover is complete. Top-level readiness remains "
            "blocked only by manual device evidence, network artifact support required by current "
            "authored recipes, and release/distribution work."
        ),
        "blocking_categories": [
            "manual_device_evidence",
            "network_artifacts",
            "release_distribution",
        ],
        "code_level_local_runtime_cutover": "resolved",
    }


def build_readiness_report(
    *,
    repo_root: Path,
    authored_root: Path,
    scenario_matrix: Path,
    p8aj_live_probe_report: Path | None = None,
    p8ak_mismatch_warning_report: Path | None = None,
    p8bc_launcher_injected_planner_report: Path | None = None,
    p8bu_rust_apply_dry_run_bridge_report: Path | None = None,
) -> dict[str, Any]:
    """Build the deterministic static readiness report.

    `status` intentionally remains `blocked` for manual device evidence,
    network-backed authored artifacts, and release/distribution work. Static
    implementation checks independently prove the local/BYO Rust runtime cutover.
    """

    repo_root = repo_root.resolve()
    matrix_path = _resolve_input_path(repo_root, scenario_matrix)
    authored_path = _resolve_input_path(repo_root, authored_root)

    payload, matrix_checks = _scenario_matrix_checks(matrix_path, authored_path)
    historical_checks = [
        *matrix_checks,
        *_required_artifact_checks(repo_root),
        *_readiness_doc_reference_checks(repo_root),
        *_cli_backend_token_checks(repo_root),
        *_cli_default_route_checks(repo_root),
        *_cli_explicit_rust_apply_dry_run_checks(repo_root),
        *_cli_default_rust_apply_dry_run_checks(repo_root),
        *_cli_packaged_rust_backend_candidate_checks(repo_root),
    ]
    static_checks = _runtime_cutover_checks(repo_root)
    historical_p8_evidence = {
        P8AJ_EVIDENCE_ID: _evaluate_evidence_report(
            repo_root=repo_root,
            report_path=p8aj_live_probe_report,
            evidence_id=P8AJ_EVIDENCE_ID,
            blocker_id="historical_real_device_plan_probe_evidence",
            validator=_p8aj_evidence_rejection_reasons,
        ),
        P8AK_EVIDENCE_ID: _evaluate_evidence_report(
            repo_root=repo_root,
            report_path=p8ak_mismatch_warning_report,
            evidence_id=P8AK_EVIDENCE_ID,
            blocker_id="historical_device_profile_mismatch_warning_evidence",
            validator=_p8ak_evidence_rejection_reasons,
        ),
        P8BC_EVIDENCE_ID: _evaluate_evidence_report(
            repo_root=repo_root,
            report_path=p8bc_launcher_injected_planner_report,
            evidence_id=P8BC_EVIDENCE_ID,
            blocker_id="historical_packaged_launcher_injection_evidence",
            validator=_p8bc_evidence_rejection_reasons,
        ),
        P8BU_EVIDENCE_ID: _evaluate_evidence_report(
            repo_root=repo_root,
            report_path=p8bu_rust_apply_dry_run_bridge_report,
            evidence_id=P8BU_EVIDENCE_ID,
            blocker_id="historical_rust_apply_dry_run_bridge_evidence",
            validator=_p8bu_evidence_rejection_reasons,
            sensitive_payload_exemption_kind=P8BU_REPORT_KIND,
        ),
    }

    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "blocked",
        "status_explanation": _status_explanation(),
        "inputs": {
            "authored_root": _display_path(authored_root),
            "scenario_matrix": _display_path(scenario_matrix),
        },
        "static_checks": static_checks,
        "historical_checks": historical_checks,
        "historical_p8_evidence_classification": {
            "classification": "historical_manual_only",
            "current_product_readiness_effect": False,
            "accepted_reports_do_not_resolve_implementation_cutover": True,
        },
        "historical_p8_evidence": historical_p8_evidence,
        "required_manual_evidence": [dict(item) for item in REQUIRED_MANUAL_EVIDENCE],
        "historical_manual_evidence": [dict(item) for item in HISTORICAL_P8_MANUAL_EVIDENCE],
        "remaining_blockers": [dict(item) for item in REMAINING_BLOCKERS],
    }


def dumps_report(report: dict[str, Any]) -> str:
    """Serialize a report without environment-specific or timing metadata."""

    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def static_checks_pass(report: dict[str, Any]) -> bool:
    """Return whether all static prerequisites passed."""

    return all(check.get("status") == "pass" for check in report.get("static_checks", []))


def _evaluate_evidence_report(
    *,
    repo_root: Path,
    report_path: Path | None,
    evidence_id: str,
    blocker_id: str,
    validator: Callable[[dict[str, Any]], list[str]],
    sensitive_payload_exemption_kind: str | None = None,
) -> dict[str, Any]:
    if report_path is None:
        return _evidence_item(
            status="missing",
            evidence_id=evidence_id,
            blocker_id=blocker_id,
            reasons=["evidence report path was not supplied"],
        )

    payload, read_reasons = _read_evidence_json(_resolve_input_path(repo_root, report_path))
    if read_reasons:
        return _evidence_item(
            status="rejected",
            evidence_id=evidence_id,
            blocker_id=blocker_id,
            reasons=read_reasons,
        )

    if payload is None:
        return _evidence_item(
            status="rejected",
            evidence_id=evidence_id,
            blocker_id=blocker_id,
            reasons=["evidence report root must be a JSON object"],
        )

    # P8BU intentionally records command tokens and absolute input paths because
    # the smoke proves the apply dry-run bridge route. Keep that exception tied
    # to the exact report kind so older evidence schemas retain their stricter
    # metadata-only policy.
    exempt_from_sensitive_payload_filters = (
        sensitive_payload_exemption_kind is not None and payload.get("kind") == sensitive_payload_exemption_kind
    )
    reasons = []
    if not exempt_from_sensitive_payload_filters:
        reasons.extend(_sensitive_evidence_rejection_reasons(payload))
        reasons.extend(_local_path_value_rejection_reasons(payload))
    reasons.extend(validator(payload))
    return _evidence_item(
        status="accepted" if not reasons else "rejected",
        evidence_id=evidence_id,
        blocker_id=blocker_id,
        reasons=reasons,
    )


def _evidence_item(*, status: str, evidence_id: str, blocker_id: str, reasons: list[str]) -> dict[str, Any]:
    return {
        "status": status,
        "evidence_id": evidence_id,
        "blocker_id": blocker_id,
        "reasons": reasons,
    }


def _read_evidence_json(path: Path) -> tuple[dict[str, Any] | None, list[str]]:
    if not path.exists():
        return None, ["evidence report file is missing"]
    try:
        # SECURITY-REVIEW: Evidence paths are explicitly supplied by a developer.
        # The gate only parses JSON and validates metadata; it never executes
        # commands or imports smoke/planner modules.
        payload = json.loads(path.read_text(encoding="utf-8"))
    except OSError:
        return None, ["evidence report could not be read"]
    except json.JSONDecodeError:
        return None, ["evidence report must be valid JSON"]
    if not isinstance(payload, dict):
        return None, ["evidence report root must be a JSON object"]
    return payload, []


def _sensitive_evidence_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    sensitive_paths = _sensitive_evidence_key_paths(payload)
    if not sensitive_paths:
        return []
    return [f"evidence report contains sensitive field: {path}" for path in sensitive_paths]


def _sensitive_evidence_key_paths(value: object, *, prefix: str = "report") -> list[str]:
    paths: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            key_text = str(key)
            child_path = f"{prefix}.{key_text}"
            if key_text.lower() in SENSITIVE_EVIDENCE_KEYS:
                paths.append(child_path)
            paths.extend(_sensitive_evidence_key_paths(child, prefix=child_path))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            paths.extend(_sensitive_evidence_key_paths(child, prefix=f"{prefix}[{index}]"))
    return paths


def _local_path_value_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    path_values = _local_path_value_paths(payload)
    if not path_values:
        return []
    return [f"evidence report contains local path-looking value: {path}" for path in path_values]


def _local_path_value_paths(value: object, *, prefix: str = "report") -> list[str]:
    paths: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            paths.extend(_local_path_value_paths(child, prefix=f"{prefix}.{key}"))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            paths.extend(_local_path_value_paths(child, prefix=f"{prefix}[{index}]"))
    elif isinstance(value, str) and _looks_like_full_local_path(value):
        paths.append(prefix)
    return paths


def _looks_like_full_local_path(value: str) -> bool:
    return (
        value.startswith("/")
        or value.startswith("~/")
        or _looks_like_windows_drive_path(value)
        or value.startswith("\\\\")
    )


def _looks_like_windows_drive_path(value: str) -> bool:
    return len(value) >= 3 and value[0].isalpha() and value[1] == ":" and value[2] in ("/", "\\")


def _p8aj_evidence_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    reasons = []
    if payload.get("kind") != P8AJ_REPORT_KIND:
        reasons.append(f"kind must be {P8AJ_REPORT_KIND}")
    if payload.get("schema_version") != HISTORICAL_P8_REPORT_SCHEMA_VERSION:
        reasons.append(f"schema_version must be {HISTORICAL_P8_REPORT_SCHEMA_VERSION}")
    reasons.extend(_optional_status_rejection_reasons(payload))

    summary = payload.get("summary")
    if not isinstance(summary, dict) or not _json_int_equals(summary.get("failed"), 0):
        reasons.append("summary.failed must be 0")

    inputs = payload.get("inputs")
    if not isinstance(inputs, dict) or inputs.get("live_probe_requested") is not True:
        reasons.append("inputs.live_probe_requested must be true")
    return reasons


def _p8ak_evidence_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    reasons = []
    if payload.get("kind") != P8AK_REPORT_KIND:
        reasons.append(f"kind must be {P8AK_REPORT_KIND}")
    if payload.get("schema_version") != HISTORICAL_P8_REPORT_SCHEMA_VERSION:
        reasons.append(f"schema_version must be {HISTORICAL_P8_REPORT_SCHEMA_VERSION}")
    reasons.extend(_optional_status_rejection_reasons(payload))

    summary = payload.get("summary")
    if not isinstance(summary, dict) or not _json_int_equals(summary.get("failed"), 0):
        reasons.append("summary.failed must be 0")

    cases = payload.get("cases")
    if not isinstance(cases, list):
        reasons.append("cases must be a list")
    else:
        passing_case_ids = {
            case.get("id")
            for case in cases
            if isinstance(case, dict) and case.get("status") == "passed" and isinstance(case.get("id"), str)
        }
        missing_cases = [case_id for case_id in P8AK_REQUIRED_PASSING_CASES if case_id not in passing_case_ids]
        if missing_cases:
            reasons.append(f"missing passing cases: {', '.join(missing_cases)}")
    return reasons


def _p8bc_evidence_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    reasons = []
    for key in P8BC_REQUIRED_TOP_LEVEL_KEYS:
        if key not in payload:
            reasons.append(f"missing required top-level key: {key}")

    if payload.get("kind") != P8BC_REPORT_KIND:
        reasons.append(f"kind must be {P8BC_REPORT_KIND}")
    if payload.get("schema_version") != HISTORICAL_P8_REPORT_SCHEMA_VERSION:
        reasons.append(f"schema_version must be {HISTORICAL_P8_REPORT_SCHEMA_VERSION}")

    summary = payload.get("summary")
    if not isinstance(summary, dict) or not _json_int_equals(summary.get("failed"), 0):
        reasons.append("summary.failed must be 0")

    inputs = payload.get("inputs")
    if not isinstance(inputs, dict):
        reasons.append("inputs must be an object")
    else:
        for key, expected in P8BC_REQUIRED_INPUT_VALUES.items():
            if inputs.get(key) != expected:
                reasons.append(f"inputs.{key} must be {_json_literal(expected)}")

    checks = payload.get("checks")
    if not isinstance(checks, list):
        reasons.append("checks must be a list")
    else:
        required_check_states = _p8bc_required_check_states(checks)
        for check_name in P8BC_REQUIRED_CHECKS:
            check_state = required_check_states.get(check_name)
            if check_state is None:
                reasons.append(f"missing required check: {check_name}")
            elif check_state is not True:
                reasons.append(f"required check must pass: {check_name}")

    redaction = payload.get("redaction")
    if not isinstance(redaction, dict):
        reasons.append("redaction must be an object")
    else:
        for key in P8BC_REQUIRED_REDACTION_FLAGS:
            if redaction.get(key) is not True:
                reasons.append(f"redaction.{key} must be true")

    artifacts = payload.get("artifacts")
    if not isinstance(artifacts, dict):
        reasons.append("artifacts must be an object")
    else:
        argv0_basename = artifacts.get("argv0_basename")
        if not isinstance(argv0_basename, str) or not argv0_basename or "/" in argv0_basename or "\\" in argv0_basename:
            reasons.append("artifacts.argv0_basename must be a non-empty basename")

    return reasons


def _p8bu_evidence_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    reasons = []
    for key in P8BU_REQUIRED_TOP_LEVEL_KEYS:
        if key not in payload:
            reasons.append(f"missing required top-level key: {key}")

    if payload.get("kind") != P8BU_REPORT_KIND:
        reasons.append(f"kind must be {P8BU_REPORT_KIND}")
    if payload.get("schema_version") != HISTORICAL_P8_REPORT_SCHEMA_VERSION:
        reasons.append(f"schema_version must be {HISTORICAL_P8_REPORT_SCHEMA_VERSION}")
    if payload.get("status") != "passed":
        reasons.append("status must be passed")

    route = payload.get("route")
    if route is None:
        reasons.append("route is required")
    elif route not in (P8BU_EXPLICIT_ROUTE, P8BU_DEFAULT_PACKAGED_ROUTE):
        reasons.append(f"route must be {P8BU_EXPLICIT_ROUTE} or {P8BU_DEFAULT_PACKAGED_ROUTE}")

    inputs = payload.get("inputs")
    if not isinstance(inputs, dict):
        reasons.append("inputs must be an object")
    elif route == P8BU_DEFAULT_PACKAGED_ROUTE and inputs.get("rust_apply_bin") is not None:
        reasons.append("inputs.rust_apply_bin must be null for default_packaged")

    command = payload.get("command")
    if not isinstance(command, list) or not all(isinstance(token, str) for token in command):
        reasons.append("command must be a list of strings")
    else:
        reasons.extend(_p8bu_command_rejection_reasons(command, route=route if isinstance(route, str) else None))

    result = payload.get("result")
    if not isinstance(result, dict):
        reasons.append("result must be an object")
    elif not _json_int_equals(result.get("returncode"), 0):
        reasons.append("result.returncode must be 0")

    checks = payload.get("checks")
    if not isinstance(checks, list):
        reasons.append("checks must be a list")
    else:
        required_check_ids = _p8bu_required_checks_for_route(route if isinstance(route, str) else None)
        required_check_states = _p8bu_required_check_states(checks, required_check_ids=required_check_ids)
        for check_id in required_check_ids:
            check_state = required_check_states.get(check_id)
            if check_state is None:
                reasons.append(f"missing required check: {check_id}")
            elif check_state is not True:
                reasons.append(f"required check must pass: {check_id}")

    return reasons


def _p8bu_command_rejection_reasons(command: list[str], *, route: str | None) -> list[str]:
    reasons = []
    required_tokens = (
        P8BU_EXPLICIT_REQUIRED_COMMAND_TOKENS
        if route == P8BU_EXPLICIT_ROUTE
        else P8BU_BASE_REQUIRED_COMMAND_TOKENS
    )
    for token in required_tokens:
        if token not in command:
            reasons.append(f"command must contain {token}")
    if route == P8BU_DEFAULT_PACKAGED_ROUTE and "--rust-apply-bin" in command:
        reasons.append("default_packaged command must not contain --rust-apply-bin")
    options_with_values = ("--plan-file", "--rust-apply-bin") if route == P8BU_EXPLICIT_ROUTE else ("--plan-file",)
    for option in options_with_values:
        if option in command and not _command_option_has_value(command, option):
            reasons.append(f"command must include a value after {option}")
    return reasons


def _command_option_has_value(command: list[str], option: str) -> bool:
    return any(
        index + 1 < len(command)
        and isinstance(command[index + 1], str)
        and bool(command[index + 1])
        and not command[index + 1].startswith("--")
        for index, token in enumerate(command)
        if token == option
    )


def _p8bu_required_checks_for_route(route: str | None) -> tuple[str, ...]:
    if route == P8BU_DEFAULT_PACKAGED_ROUTE:
        return P8BU_DEFAULT_REQUIRED_CHECKS
    if route == P8BU_EXPLICIT_ROUTE:
        return P8BU_REQUIRED_CHECKS
    return ()


def _p8bu_required_check_states(
    checks: list[object],
    *,
    required_check_ids: tuple[str, ...],
) -> dict[str, bool | None]:
    states: dict[str, bool | None] = {}
    for check in checks:
        if not isinstance(check, dict):
            continue
        check_id = check.get("id")
        if isinstance(check_id, str) and check_id in required_check_ids:
            status = check.get("status")
            states[check_id] = True if status == "pass" else False if isinstance(status, str) else None
    return states


def _optional_status_rejection_reasons(payload: dict[str, Any]) -> list[str]:
    status = payload.get("status")
    if status is None or status == "passed":
        return []
    return ["status must be passed when present"]


def _p8bc_required_check_states(checks: list[object]) -> dict[str, bool | None]:
    states: dict[str, bool | None] = {}
    for check in checks:
        if not isinstance(check, dict):
            continue
        name = check.get("name")
        if isinstance(name, str) and name in P8BC_REQUIRED_CHECKS:
            states[name] = check.get("passed") if isinstance(check.get("passed"), bool) else None
    return states


def _json_literal(value: object) -> str:
    return json.dumps(value)


def _json_int_equals(value: object, expected: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value == expected


def _runtime_cutover_checks(repo_root: Path) -> list[dict[str, Any]]:
    """Inspect only current product runtime source and packaging contracts."""

    cargo = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/Cargo.toml")
    main_rs = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/src/main.rs")
    cli_rs = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/src/cli.rs")
    executor_rs = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/src/executor.rs")
    adb_rs = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/src/executor/adb.rs")
    plan_shadow_rs = _read_text_if_available(repo_root / "crates/emuchef-rust-backend/src/plan_shadow.rs")
    pyproject = _read_text_if_available(repo_root / "pyproject.toml")
    package_json = _read_text_if_available(repo_root / "apps/config-editor/package.json")
    tauri_config = _read_text_if_available(repo_root / "apps/config-editor/src-tauri/tauri.conf.json")
    sidecar_client = _read_text_if_available(repo_root / "apps/config-editor/src-tauri/src/sidecar_client.rs")
    packaging = _read_text_if_available(repo_root / "apps/config-editor/scripts/sidecar-packaging.mjs")
    prepare = _read_text_if_available(repo_root / "apps/config-editor/scripts/prepare-rust-sidecar.mjs")

    python_execution_tokens = (
        'Command::new("python',
        'Command::new("python3',
        '"python.exe"',
        '"python3.exe"',
        "src/emuchef/cli.py",
    )
    rust_runtime_text = "\n".join((main_rs, cli_rs, executor_rs, adb_rs, plan_shadow_rs))
    runtime_python_tokens = [token for token in python_execution_tokens if token in rust_runtime_text]
    package_python_tokens = [
        token
        for token in ("python ", "python3 ", "python.exe", "python3.exe", "src/emuchef/cli.py")
        if token in "\n".join((package_json, tauri_config, sidecar_client, packaging, prepare))
    ]

    canonical_tokens = (
        'default-run = "emuchef"',
        'name = "emuchef"',
        'path = "src/main.rs"',
    )
    python_legacy_only = (
        'emuchef-python-legacy = "emuchef.cli:main"' in pyproject
        and 'emuchef = "emuchef.cli:main"' not in pyproject
    )

    return [
        _check(
            "canonical_cli_is_rust_binary",
            _source_has_tokens(cargo, canonical_tokens)
            and "emuchef_rust_backend::run_with_args_and_input" in main_rs,
            _missing_source_tokens_details(cargo + main_rs, (*canonical_tokens, "run_with_args_and_input")),
        ),
        _check(
            "python_cli_not_default_entrypoint",
            python_legacy_only,
            None
            if python_legacy_only
            else {"required": "emuchef-python-legacy only", "forbidden": "Python-owned emuchef"},
        ),
        _check(
            "default_plan_runtime_has_no_python_execution",
            'Some("plan") => run_plan' in cli_rs
            and "planning_result_with_adb_runner" in cli_rs
            and not runtime_python_tokens,
            None if not runtime_python_tokens else {"forbidden_tokens": runtime_python_tokens},
        ),
        _check(
            "default_validate_runtime_has_no_python_execution",
            'Some("validate") => run_validate' in cli_rs
            and "validation::validate_recipe_path_result" in cli_rs
            and not runtime_python_tokens,
            None if not runtime_python_tokens else {"forbidden_tokens": runtime_python_tokens},
        ),
        _check(
            "default_apply_runtime_has_no_python_execution",
            'Some("apply") => run_apply' in cli_rs
            and "execute_apply_plan" in cli_rs
            and not runtime_python_tokens,
            None if not runtime_python_tokens else {"forbidden_tokens": runtime_python_tokens},
        ),
        _check(
            "rust_cli_apply_accepts_non_dry_run",
            "if config.dry_run" in cli_rs
            and "Rust Phase 6S apply supports only --dry-run." not in cli_rs
            and "execute_apply_plan(&plan, adapters, false)" in cli_rs,
        ),
        _check(
            "rust_cli_apply_uses_real_adb_device_for_non_dry_run",
            "RealAdbDevice::new(adb, serial)" in cli_rs
            and "with_device_and_sandbox_roots" in cli_rs,
        ),
        _check(
            "rust_cli_apply_preserves_dry_run_runner",
            "ExecutorAdapters::with_sandbox_roots" in cli_rs
            and "execute_apply_plan(&plan, adapters, true)" in cli_rs,
        ),
        _check(
            "rust_cli_apply_forwards_adb_and_serial",
            '"--plan-file" | "--adb" | "--serial"' in cli_rs
            and "factory(config.adb, config.serial)" in cli_rs,
        ),
        _check(
            "tauri_runtime_has_no_python_dependency",
            '"externalBin": ["binaries/emuchef"]' in tauri_config
            and '"emuchef"' in sidecar_client
            and not package_python_tokens,
            None if not package_python_tokens else {"forbidden_tokens": package_python_tokens},
        ),
        _check(
            "packaged_sidecar_contract_ready",
            'BINARY_BASENAME = "emuchef"' in packaging
            and '"--bin", BINARY_BASENAME' in prepare
            and '"externalBin": ["binaries/emuchef"]' in tauri_config,
        ),
        _check(
            "packaged_runtime_does_not_invoke_python",
            not package_python_tokens,
            None if not package_python_tokens else {"forbidden_tokens": package_python_tokens},
        ),
    ]


def _scenario_matrix_checks(matrix_path: Path, authored_path: Path) -> tuple[dict[str, Any] | None, list[dict[str, Any]]]:
    checks: list[dict[str, Any]] = []
    payload: dict[str, Any] | None = None

    matrix_exists = matrix_path.exists()
    checks.append(_check("scenario_matrix_exists", matrix_exists, _missing_path_details(matrix_path, matrix_exists)))

    json_valid = False
    if matrix_exists:
        try:
            # SECURITY-REVIEW: The scenario matrix path is developer supplied.
            # The tool only deserializes JSON values and validates their shape.
            raw_payload = json.loads(matrix_path.read_text(encoding="utf-8"))
        except OSError as exc:
            checks.append(_check("scenario_matrix_json_valid", False, {"error": str(exc)}))
        except json.JSONDecodeError as exc:
            checks.append(_check("scenario_matrix_json_valid", False, {"error": str(exc)}))
        else:
            json_valid = isinstance(raw_payload, dict)
            payload = raw_payload if isinstance(raw_payload, dict) else None
            details = None if json_valid else {"error": "root must be a JSON object"}
            checks.append(_check("scenario_matrix_json_valid", json_valid, details))
    else:
        checks.append(_check("scenario_matrix_json_valid", False, {"error": "scenario matrix file is missing"}))

    scenarios = payload.get("scenarios") if payload is not None else None
    schema_version = payload.get("schema_version") if payload is not None else None
    checks.append(
        _check(
            "scenario_matrix_schema_version",
            schema_version == SCENARIO_MATRIX_SCHEMA_VERSION,
            None if schema_version == SCENARIO_MATRIX_SCHEMA_VERSION else {"actual": schema_version},
        )
    )

    scenarios_non_empty = isinstance(scenarios, list) and bool(scenarios)
    checks.append(
        _check(
            "scenario_matrix_scenarios_non_empty",
            scenarios_non_empty,
            None if scenarios_non_empty else {"error": "scenarios must be a non-empty list"},
        )
    )

    scenario_field_errors = _scenario_field_errors(scenarios)
    checks.append(
        _check(
            "scenario_matrix_scenario_fields",
            not scenario_field_errors,
            None if not scenario_field_errors else {"errors": scenario_field_errors},
        )
    )
    checks.extend(_explicit_context_checks(scenarios))

    scenario_ids = _scenario_string_values(scenarios, "id")
    duplicate_ids = _duplicates(scenario_ids)
    checks.append(
        _check(
            "scenario_matrix_unique_ids",
            bool(scenario_ids) and not duplicate_ids,
            None if scenario_ids and not duplicate_ids else {"duplicate_ids": duplicate_ids},
        )
    )

    scenario_device_plans = _scenario_string_values(scenarios, "device_plan")
    checked_in_device_plan_ids = _checked_in_device_plan_ids(authored_path)
    matrix_device_plan_ids = set(scenario_device_plans)
    missing_device_plans = sorted(device_plan_id for device_plan_id in checked_in_device_plan_ids if device_plan_id not in matrix_device_plan_ids)
    checks.append(
        _check(
            "scenario_matrix_covers_checked_in_device_plans",
            not missing_device_plans,
            None if not missing_device_plans else {"missing_device_plans": missing_device_plans},
        )
    )

    return payload, checks


def _required_artifact_checks(repo_root: Path) -> list[dict[str, Any]]:
    checks = []
    for relative_path in REQUIRED_ARTIFACTS:
        path = repo_root / relative_path
        exists = path.exists()
        checks.append(_check(f"required_artifact_{_stable_id(relative_path)}", exists, _missing_path_details(path, exists)))
    return checks


def _readiness_doc_reference_checks(repo_root: Path) -> list[dict[str, Any]]:
    path = repo_root / "docs" / "rust-planner-cutover-readiness.md"
    text = _read_text_if_available(path)
    checks = []
    for token_id, token in READINESS_DOC_REFERENCES:
        has_token = token in text
        details = None if has_token else {"missing_token": token}
        checks.append(_check(f"readiness_doc_reference_{token_id}", has_token, details))
    return checks


def _cli_backend_token_checks(repo_root: Path) -> list[dict[str, Any]]:
    path = repo_root / "src" / "emuchef" / "cli.py"
    text = _read_text_if_available(path)
    checks = []
    for token_id, token in CLI_BACKEND_TOKENS:
        has_token = token in text
        details = None if has_token else {"missing_token": token}
        checks.append(_check(f"cli_backend_token_{token_id}", has_token, details))
    return checks


def _cli_default_route_checks(repo_root: Path) -> list[dict[str, Any]]:
    path = repo_root / "src" / "emuchef" / "cli.py"
    text = _read_text_if_available(path)
    backend_block = _source_call_block(text, '"--planner-backend"')
    default_omitted = bool(backend_block) and "default=" not in backend_block

    default_resolution_tokens = (
        '_DEFAULT_RUST_PLANNER_BACKEND = "rust-production-equivalent"',
        "return args.planner_backend or _DEFAULT_RUST_PLANNER_BACKEND",
    )
    rust_bin_required_tokens = (
        "if args.rust_planner_bin:",
        "packaged_candidate = _packaged_rust_planner_bin_candidate(args)",
        "if packaged_candidate is not None:",
        "def _packaged_rust_planner_bin_candidate(args",
        "return _packaged_rust_backend_bin_candidate()",
        "if args.planner_backend is None:",
        "--rust-planner-bin is required when default Rust planner routing is active.",
    )

    return [
        _check(
            "cli_default_backend_is_omitted",
            default_omitted,
            _default_backend_omitted_details(backend_block, default_omitted),
        ),
        _check(
            "cli_default_backend_resolves_to_rust_production_equivalent",
            _source_has_tokens(text, default_resolution_tokens),
            _missing_source_tokens_details(text, default_resolution_tokens),
        ),
        _check(
            "cli_explicit_python_backend_not_exposed",
            '"python"' not in backend_block,
            None
            if '"python"' not in backend_block
            else {"unexpected_token": '"python"', "source_block": "--planner-backend"},
        ),
        _check(
            "cli_default_rust_requires_planner_bin",
            _source_has_tokens(text, rust_bin_required_tokens),
            _missing_source_tokens_details(text, rust_bin_required_tokens),
        ),
    ]


def _cli_explicit_rust_apply_dry_run_checks(repo_root: Path) -> list[dict[str, Any]]:
    path = repo_root / "src" / "emuchef" / "cli.py"
    text = _read_text_if_available(path)
    main_block = _source_function_block(text, "main")
    dispatch_scope = main_block or text
    run_apply_block = _source_function_block(text, "_run_apply")
    run_rust_apply_block = _source_function_block(text, "_run_rust_apply_dry_run")
    build_command_block = _source_function_block(text, "_build_rust_apply_command")
    resolve_bin_block = _source_function_block(text, "_resolve_rust_apply_bin")
    validate_dry_run_block = _source_function_block(text, "_validate_rust_apply_dry_run_args")
    validate_candidate_block = _source_function_block(text, "_validate_rust_apply_bin_candidate")
    explicit_candidate_block = _source_function_block(text, "_explicit_rust_apply_bin_candidate")
    args_validation_scope = "\n".join((resolve_bin_block, validate_dry_run_block))
    binary_validation_scope = "\n".join((resolve_bin_block, explicit_candidate_block, validate_candidate_block))

    option_block = _source_call_block(text, '"--rust-apply-bin"')
    option_present = bool(option_block) and "apply_parser.add_argument" in option_block

    dispatch_tokens = (
        "if _should_route_apply_dry_run_to_rust(args)",
        "return _run_apply(args)",
    )
    adb_resolution_tokens = (
        "setattr(",
        '"_resolved_adb"',
        "resolve_adb_executable(",
    )
    dispatch_before_adb = _source_tokens_before(dispatch_scope, dispatch_tokens, adb_resolution_tokens)

    branch_tokens = (
        "if args.rust_apply_bin is not None or args.dry_run",
        "return _run_rust_apply_dry_run(args)",
        "_build_adb(args)",
    )
    dry_run_tokens = (
        "if not args.dry_run",
        "--rust-apply-bin requires --dry-run.",
    )
    unsupported_rejection_tokens = (
        '"--adb"',
        "args.adb is not None",
        '"--verbose"',
        "args.verbose",
        '"--debug"',
        "args.debug",
        "--rust-apply-bin does not support",
    )
    unsupported_forwarding_tokens = (
        '"--adb"',
        "args.adb",
        '"--verbose"',
        "args.verbose",
        '"--debug"',
        "args.debug",
    )
    binary_validation_tokens = (
        "Path(args.rust_apply_bin).expanduser()",
        ".exists()",
        ".is_file()",
        "os.access(",
        "os.X_OK",
    )
    command_tokens = (
        "str(rust_apply_bin)",
        '"apply"',
        '"--plan-file"',
        "args.plan_file",
        '"--dry-run"',
    )
    serial_tokens = (
        "if args.serial is not None",
        "command.extend",
        '"--serial"',
        "args.serial",
    )
    subprocess_tokens = (
        "subprocess" + ".run(",
        "check=False",
        "text=True",
        "capture_output=True",
    )
    output_tokens = (
        "sys.stdout.write(completed.stdout)",
        "sys.stderr.write(completed.stderr)",
        "return completed.returncode",
    )

    rejects_unsupported_flags = _source_has_tokens(
        args_validation_scope,
        unsupported_rejection_tokens,
    ) and not _source_has_any_compact_tokens(build_command_block, unsupported_forwarding_tokens)

    return [
        _check(
            "cli_explicit_rust_apply_dry_run_option_present",
            option_present,
            None if option_present else {"missing_token": '"--rust-apply-bin"', "source_block": "apply_parser"},
        ),
        _check(
            "cli_explicit_rust_apply_dispatch_before_adb_resolution",
            dispatch_before_adb,
            _ordered_source_tokens_details(dispatch_scope, dispatch_tokens, adb_resolution_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_branch_present",
            _source_has_tokens(run_apply_block, branch_tokens),
            _missing_source_tokens_details(run_apply_block, branch_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_requires_dry_run",
            _source_has_tokens(args_validation_scope, dry_run_tokens),
            _missing_source_tokens_details(args_validation_scope, dry_run_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_rejects_python_only_flags",
            rejects_unsupported_flags,
            _unsupported_apply_flags_details(
                args_validation_scope,
                build_command_block,
                unsupported_rejection_tokens,
                unsupported_forwarding_tokens,
            ),
        ),
        _check(
            "cli_explicit_rust_apply_validates_binary_path",
            _source_has_compact_tokens(binary_validation_scope, binary_validation_tokens),
            _missing_compact_source_tokens_details(binary_validation_scope, binary_validation_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_builds_expected_command",
            _source_has_ordered_tokens(build_command_block, command_tokens),
            _missing_or_unordered_source_tokens_details(build_command_block, command_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_forwards_serial_conditionally",
            _source_has_tokens(build_command_block, serial_tokens),
            _missing_source_tokens_details(build_command_block, serial_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_uses_static_subprocess_contract",
            _source_has_tokens(run_rust_apply_block, subprocess_tokens),
            _missing_source_tokens_details(run_rust_apply_block, subprocess_tokens),
        ),
        _check(
            "cli_explicit_rust_apply_preserves_subprocess_output",
            _source_has_compact_tokens(run_rust_apply_block, output_tokens),
            _missing_compact_source_tokens_details(run_rust_apply_block, output_tokens),
        ),
    ]


def _cli_default_rust_apply_dry_run_checks(repo_root: Path) -> list[dict[str, Any]]:
    path = repo_root / "src" / "emuchef" / "cli.py"
    text = _read_text_if_available(path)
    main_block = _source_function_block(text, "main")
    dispatch_scope = main_block or text
    should_route_block = _source_function_block(text, "_should_route_apply_dry_run_to_rust")
    run_apply_block = _source_function_block(text, "_run_apply")
    resolve_bin_block = _source_function_block(text, "_resolve_rust_apply_bin")
    packaged_candidate_block = _source_function_block(text, "_packaged_rust_apply_bin_candidate")

    route_tokens = (
        'args.command == "apply"',
        "args.rust_apply_bin is not None",
        "args.dry_run is True",
    )
    dispatch_tokens = (
        "if _should_route_apply_dry_run_to_rust(args)",
        "return _run_apply(args)",
    )
    adb_resolution_tokens = (
        "setattr(",
        '"_resolved_adb"',
        "resolve_adb_executable(",
    )
    requires_binary_tokens = (
        "rust_apply_bin = _packaged_rust_apply_bin_candidate(args)",
        "if rust_apply_bin is not None",
        "--rust-apply-bin is required when default Rust apply dry-run routing is active.",
    )
    no_fallback_branch_tokens = (
        "if args.rust_apply_bin is not None or args.dry_run",
        "return _run_rust_apply_dry_run(args)",
    )
    non_dry_run_python_tokens = (
        "plan_path = Path(args.plan_file)",
        "load_execution_plan_file(plan_path)",
        "_build_adb(args)",
        "ExecutorRunner(",
    )

    route_present = _source_has_tokens(should_route_block, route_tokens)
    dispatch_before_adb = _source_tokens_before(dispatch_scope, dispatch_tokens, adb_resolution_tokens)
    requires_binary = _source_has_tokens(resolve_bin_block, requires_binary_tokens)
    packaged_seam_present = (
        "def _packaged_rust_apply_bin_candidate(args" in packaged_candidate_block
        and "return _packaged_rust_backend_bin_candidate()" in packaged_candidate_block
    )
    no_python_fallback = _source_has_tokens(run_apply_block, no_fallback_branch_tokens) and "DryRunAdb() if args.dry_run" not in run_apply_block
    preserves_non_dry_run = _source_has_tokens(run_apply_block, no_fallback_branch_tokens) and _source_has_tokens(
        run_apply_block,
        non_dry_run_python_tokens,
    )

    return [
        _check(
            "cli_default_rust_apply_dry_run_route_present",
            route_present,
            _missing_source_tokens_details(should_route_block, route_tokens),
        ),
        _check(
            "cli_default_rust_apply_dry_run_route_before_adb_resolution",
            dispatch_before_adb,
            _ordered_source_tokens_details(dispatch_scope, dispatch_tokens, adb_resolution_tokens),
        ),
        _check(
            "cli_default_rust_apply_dry_run_requires_binary",
            requires_binary and packaged_seam_present,
            _default_rust_apply_requires_binary_details(
                resolve_bin_block,
                packaged_candidate_block,
                requires_binary_tokens,
                packaged_seam_present,
            ),
        ),
        _check(
            "cli_default_rust_apply_dry_run_has_no_python_fallback",
            no_python_fallback,
            _default_rust_apply_no_fallback_details(run_apply_block, no_fallback_branch_tokens),
        ),
        _check(
            "cli_default_rust_apply_dry_run_preserves_non_dry_run_apply_boundary",
            preserves_non_dry_run,
            _missing_source_tokens_details(run_apply_block, (*no_fallback_branch_tokens, *non_dry_run_python_tokens)),
        ),
    ]


def _cli_packaged_rust_backend_candidate_checks(repo_root: Path) -> list[dict[str, Any]]:
    cli_path = repo_root / "src" / "emuchef" / "cli.py"
    cli_text = _read_text_if_available(cli_path)
    helper_block = _source_function_block(cli_text, "_packaged_rust_backend_bin_candidate")
    planner_candidate_block = _source_function_block(cli_text, "_packaged_rust_planner_bin_candidate")
    apply_candidate_block = _source_function_block(cli_text, "_packaged_rust_apply_bin_candidate")
    smoke_text = _read_text_if_available(repo_root / "tools" / "smoke_rust_apply_dry_run_bridge.py")

    helper_tokens = (
        "def _packaged_rust_backend_bin_candidate(",
        "platform.system()",
        "platform.machine()",
        "apps",
        "config-editor",
        "src-tauri",
        "binaries",
        "emuchef-",
    )
    forbidden_helper_tokens = (
        "os.environ",
        "getenv(",
        "shutil.which",
        "subprocess" + ".",
        "Popen(",
        "os.system",
        "cargo ",
        "npm ",
        "node ",
        "target/debug",
        "target/release",
        ".iterdir(",
        ".rglob(",
        "glob(",
        "os.listdir",
        "os.walk",
    )
    forbidden_present = [token for token in forbidden_helper_tokens if token in helper_block]
    planner_tokens = (
        "def _packaged_rust_planner_bin_candidate(args",
        "return _packaged_rust_backend_bin_candidate()",
    )
    apply_tokens = (
        "def _packaged_rust_apply_bin_candidate(args",
        "return _packaged_rust_backend_bin_candidate()",
    )
    smoke_tokens = (
        "--use-default-packaged-route",
        "default_packaged",
        "route",
        "--rust-apply-bin",
    )

    return [
        _check(
            "cli_packaged_rust_backend_candidate_contract_present",
            _source_has_tokens(helper_block, helper_tokens),
            _missing_source_tokens_details(helper_block, helper_tokens),
        ),
        _check(
            "cli_packaged_rust_backend_candidate_no_shell_or_path_lookup",
            not forbidden_present,
            None if not forbidden_present else {"forbidden_tokens": forbidden_present},
        ),
        _check(
            "cli_packaged_rust_planner_uses_packaged_backend_candidate",
            _source_has_tokens(planner_candidate_block, planner_tokens),
            _missing_source_tokens_details(planner_candidate_block, planner_tokens),
        ),
        _check(
            "cli_packaged_rust_apply_uses_packaged_backend_candidate",
            _source_has_tokens(apply_candidate_block, apply_tokens),
            _missing_source_tokens_details(apply_candidate_block, apply_tokens),
        ),
        _check(
            "cli_default_packaged_apply_dry_run_smoke_supported",
            _source_has_tokens(smoke_text, smoke_tokens),
            _missing_source_tokens_details(smoke_text, smoke_tokens),
        ),
    ]


def _default_backend_omitted_details(backend_block: str, passed: bool) -> dict[str, str] | None:
    if passed:
        return None
    if not backend_block:
        return {"missing_token": '"--planner-backend"'}
    return {"unexpected_token": "default="}


def _default_rust_apply_requires_binary_details(
    resolve_bin_block: str,
    packaged_candidate_block: str,
    required_tokens: tuple[str, ...],
    packaged_seam_present: bool,
) -> dict[str, list[str] | str] | None:
    missing_tokens = _missing_source_tokens(resolve_bin_block, required_tokens)
    details: dict[str, list[str] | str] = {}
    if missing_tokens:
        details["missing_tokens"] = missing_tokens
    if not packaged_seam_present:
        details["packaged_candidate"] = "packaged apply resolver seam must call shared packaged backend candidate"
    return details or None


def _default_rust_apply_no_fallback_details(
    run_apply_block: str,
    branch_tokens: tuple[str, ...],
) -> dict[str, list[str] | str] | None:
    details: dict[str, list[str] | str] = {}
    missing_tokens = _missing_source_tokens(run_apply_block, branch_tokens)
    if missing_tokens:
        details["missing_tokens"] = missing_tokens
    if "DryRunAdb() if args.dry_run" in run_apply_block:
        details["unexpected_token"] = "DryRunAdb() if args.dry_run"
    return details or None


def _source_call_block(text: str, marker: str) -> str:
    lines = text.splitlines()
    for marker_index, line in enumerate(lines):
        if marker not in line:
            continue
        start_index = marker_index
        while start_index > 0 and "add_argument(" not in lines[start_index]:
            start_index -= 1
        block_lines: list[str] = []
        paren_balance = 0
        for block_line in lines[start_index:]:
            block_lines.append(block_line)
            paren_balance += block_line.count("(") - block_line.count(")")
            if block_lines and paren_balance <= 0:
                break
        return "\n".join(block_lines)
    return ""


def _source_function_block(text: str, function_name: str) -> str:
    lines = text.splitlines()
    marker = f"def {function_name}("
    for start_index, line in enumerate(lines):
        if not line.startswith(marker):
            continue
        block_lines = [line]
        for block_line in lines[start_index + 1 :]:
            if block_line.startswith("def ") or block_line.startswith("class "):
                break
            block_lines.append(block_line)
        return "\n".join(block_lines)
    return ""


def _source_tokens_before(text: str, first_tokens: tuple[str, ...], second_tokens: tuple[str, ...]) -> bool:
    first_start = _ordered_token_end_index(text, first_tokens)
    second_start = _ordered_token_start_index(text, second_tokens)
    return first_start is not None and second_start is not None and first_start <= second_start


def _ordered_source_tokens_details(
    text: str,
    first_tokens: tuple[str, ...],
    second_tokens: tuple[str, ...],
) -> dict[str, list[str] | str] | None:
    if _source_tokens_before(text, first_tokens, second_tokens):
        return None
    details: dict[str, list[str] | str] = {}
    missing_first = _missing_source_tokens(text, first_tokens)
    missing_second = _missing_source_tokens(text, second_tokens)
    if missing_first:
        details["missing_first_tokens"] = missing_first
    if missing_second:
        details["missing_second_tokens"] = missing_second
    if not details:
        details["ordering"] = "first token group must appear before second token group"
    return details


def _source_has_tokens(text: str, tokens: tuple[str, ...]) -> bool:
    return not _missing_source_tokens(text, tokens)


def _source_has_compact_tokens(text: str, tokens: tuple[str, ...]) -> bool:
    return not _missing_compact_source_tokens(text, tokens)


def _source_has_any_compact_tokens(text: str, tokens: tuple[str, ...]) -> bool:
    compact_text = _compact_source(text)
    return any(_compact_source(token) in compact_text for token in tokens)


def _source_has_ordered_tokens(text: str, tokens: tuple[str, ...]) -> bool:
    return _ordered_token_end_index(text, tokens) is not None


def _missing_source_tokens_details(text: str, tokens: tuple[str, ...]) -> dict[str, list[str]] | None:
    missing_tokens = _missing_source_tokens(text, tokens)
    if not missing_tokens:
        return None
    return {"missing_tokens": missing_tokens}


def _missing_compact_source_tokens_details(text: str, tokens: tuple[str, ...]) -> dict[str, list[str]] | None:
    missing_tokens = _missing_compact_source_tokens(text, tokens)
    if not missing_tokens:
        return None
    return {"missing_tokens": missing_tokens}


def _missing_or_unordered_source_tokens_details(text: str, tokens: tuple[str, ...]) -> dict[str, list[str] | str] | None:
    if _source_has_ordered_tokens(text, tokens):
        return None
    missing_tokens = _missing_source_tokens(text, tokens)
    if missing_tokens:
        return {"missing_tokens": missing_tokens}
    return {"ordering": "tokens must appear in the expected order"}


def _unsupported_apply_flags_details(
    resolve_bin_block: str,
    build_command_block: str,
    rejection_tokens: tuple[str, ...],
    forwarding_tokens: tuple[str, ...],
) -> dict[str, list[str]] | None:
    missing_rejection_tokens = _missing_source_tokens(resolve_bin_block, rejection_tokens)
    forwarded_tokens = [
        token
        for token in forwarding_tokens
        if _compact_source(token) in _compact_source(build_command_block)
    ]
    details: dict[str, list[str]] = {}
    if missing_rejection_tokens:
        details["missing_rejection_tokens"] = missing_rejection_tokens
    if forwarded_tokens:
        details["unexpected_command_tokens"] = forwarded_tokens
    return details or None


def _missing_source_tokens(text: str, tokens: tuple[str, ...]) -> list[str]:
    return [token for token in tokens if token not in text]


def _missing_compact_source_tokens(text: str, tokens: tuple[str, ...]) -> list[str]:
    compact_text = _compact_source(text)
    return [token for token in tokens if _compact_source(token) not in compact_text]


def _ordered_token_start_index(text: str, tokens: tuple[str, ...]) -> int | None:
    position = 0
    first_index: int | None = None
    for token in tokens:
        index = text.find(token, position)
        if index < 0:
            return None
        if first_index is None:
            first_index = index
        position = index + len(token)
    return first_index


def _ordered_token_end_index(text: str, tokens: tuple[str, ...]) -> int | None:
    position = 0
    for token in tokens:
        index = text.find(token, position)
        if index < 0:
            return None
        position = index + len(token)
    return position


def _compact_source(text: str) -> str:
    return "".join(text.split())


def _scenario_field_errors(scenarios: object) -> list[str]:
    if not isinstance(scenarios, list):
        return ["scenarios must be a list before scenario fields can be validated"]

    errors: list[str] = []
    for index, scenario in enumerate(scenarios):
        prefix = f"scenarios[{index}]"
        if not isinstance(scenario, dict):
            errors.append(f"{prefix} must be an object")
            continue
        if not _non_empty_string(scenario.get("id")):
            errors.append(f"{prefix}.id must be a non-empty string")
        if not _non_empty_string(scenario.get("device_plan")):
            errors.append(f"{prefix}.device_plan must be a non-empty string")
        if scenario.get("expected_classification") != "match":
            errors.append(f"{prefix}.expected_classification must be match")
        if not isinstance(scenario.get("bindings"), list):
            errors.append(f"{prefix}.bindings must be a list")
        if not isinstance(scenario.get("known_gap_ids"), list):
            errors.append(f"{prefix}.known_gap_ids must be a list")
        if "device_context" in scenario:
            errors.extend(_device_context_field_errors(scenario["device_context"], prefix=prefix))
    return errors


def _explicit_context_checks(scenarios: object) -> list[dict[str, Any]]:
    schema_details = {"fields": list(DEVICE_CONTEXT_FIELDS)}
    present, present_details = _explicit_context_scenario_present(scenarios)
    valid, valid_details = _explicit_context_scenario_valid(scenarios)
    return [
        _check("explicit_context_supported_by_matrix_schema", True, schema_details),
        _check("explicit_context_scenario_present", present, present_details),
        _check("explicit_context_scenario_valid", valid, valid_details),
    ]


def _explicit_context_scenario_present(scenarios: object) -> tuple[bool, dict[str, Any] | None]:
    if not isinstance(scenarios, list):
        return False, {"error": "scenarios must be a list before explicit device context coverage can be checked"}
    scenario_ids: list[str] = []
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            continue
        device_context = scenario.get("device_context")
        if isinstance(device_context, dict) and _device_context_has_meaningful_explicit_field(device_context):
            scenario_ids.append(_scenario_id_or_index(scenario, index))
    if scenario_ids:
        return True, None
    return False, {
        "error": "at least one scenario must include device_context with at least one explicit context field",
        "fields": list(DEVICE_CONTEXT_FIELDS),
    }


def _explicit_context_scenario_valid(scenarios: object) -> tuple[bool, dict[str, Any] | None]:
    if not isinstance(scenarios, list):
        return False, {"errors": ["scenarios must be a list before explicit device context coverage can be checked"]}

    candidate_errors: list[str] = []
    valid_scenario_ids: list[str] = []
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict) or "device_context" not in scenario:
            continue
        scenario_id = _scenario_id_or_index(scenario, index)
        field_prefix = f"scenarios[{index}]"
        device_context = scenario["device_context"]
        errors = _device_context_field_errors(device_context, prefix=field_prefix)
        if errors:
            candidate_errors.extend(errors)
            continue
        if isinstance(device_context, dict) and _device_context_has_meaningful_explicit_field(device_context):
            valid_scenario_ids.append(scenario_id)

    if valid_scenario_ids:
        return True, None
    if candidate_errors:
        return False, {"errors": candidate_errors}
    return False, {
        "errors": ["no scenario includes valid device_context with at least one explicit context field"],
        "fields": list(DEVICE_CONTEXT_FIELDS),
    }


def _device_context_field_errors(device_context: object, *, prefix: str) -> list[str]:
    field = f"{prefix}.device_context"
    if not isinstance(device_context, dict):
        return [f"{field} must be an object"]

    errors: list[str] = []
    for key in device_context:
        if key not in DEVICE_CONTEXT_FIELD_SET:
            errors.append(f"{field} contains unsupported field: {key}")

    for key in ("manufacturer", "model"):
        if key in device_context and not _non_empty_string(device_context.get(key)):
            errors.append(f"{field}.{key} must be a non-empty string")

    if "android_version" in device_context:
        android_version = device_context["android_version"]
        if isinstance(android_version, bool) or not isinstance(android_version, int) or android_version < 0:
            errors.append(f"{field}.android_version must be a non-negative integer")

    if "device_tags" in device_context:
        raw_tags = device_context["device_tags"]
        if not isinstance(raw_tags, list) or not raw_tags:
            errors.append(f"{field}.device_tags must be a non-empty list")
        else:
            for index, value in enumerate(raw_tags):
                if not _non_empty_string(value):
                    errors.append(f"{field}.device_tags[{index}] must be a non-empty string")

    return errors


def _device_context_has_meaningful_explicit_field(device_context: dict[str, object]) -> bool:
    if _non_empty_string(device_context.get("manufacturer")):
        return True
    if _non_empty_string(device_context.get("model")):
        return True
    android_version = device_context.get("android_version")
    if isinstance(android_version, int) and not isinstance(android_version, bool) and android_version >= 0:
        return True
    raw_tags = device_context.get("device_tags")
    if isinstance(raw_tags, list) and raw_tags and all(_non_empty_string(value) for value in raw_tags):
        return True
    return False


def _scenario_id_or_index(scenario: dict[str, Any], index: int) -> str:
    scenario_id = scenario.get("id")
    return scenario_id if _non_empty_string(scenario_id) else f"scenarios[{index}]"


def _scenario_string_values(scenarios: object, key: str) -> list[str]:
    if not isinstance(scenarios, list):
        return []
    values = []
    for scenario in scenarios:
        if isinstance(scenario, dict) and isinstance(scenario.get(key), str) and scenario[key]:
            values.append(scenario[key])
    return values


def _checked_in_device_plan_ids(authored_path: Path) -> list[str]:
    device_plan_root = authored_path / "device_plans"
    ids = {
        path.stem
        for pattern in ("*.yaml", "*.yml")
        for path in device_plan_root.glob(pattern)
        if path.name != ".gitkeep"
    }
    return sorted(ids)


def _duplicates(values: list[str]) -> list[str]:
    seen: set[str] = set()
    duplicate_values: set[str] = set()
    for value in values:
        if value in seen:
            duplicate_values.add(value)
        seen.add(value)
    return sorted(duplicate_values)


def _check(check_id: str, passed: bool, details: dict[str, Any] | None = None) -> dict[str, Any]:
    check = {
        "id": check_id,
        "status": "pass" if passed else "fail",
    }
    if details:
        check["details"] = details
    return check


def _missing_path_details(path: Path, exists: bool) -> dict[str, str] | None:
    if exists:
        return None
    return {"path": str(path)}


def _read_text_if_available(path: Path) -> str:
    try:
        # SECURITY-REVIEW: This reads repository-local developer-supplied paths
        # as text for stable token checks only; contents are not executed.
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def _non_empty_string(value: object) -> bool:
    return isinstance(value, str) and bool(value)


def _resolve_input_path(repo_root: Path, path: Path) -> Path:
    if path.is_absolute():
        return path
    return repo_root / path


def _display_path(path: Path) -> str:
    return path.as_posix()


def _stable_id(value: str) -> str:
    return "".join(char.lower() if char.isalnum() else "_" for char in value).strip("_")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="check_rust_planner_cutover_readiness.py",
        description="Emit a static Rust planner default-cutover readiness report.",
    )
    parser.add_argument("--authored-root", default="authored")
    parser.add_argument("--scenario-matrix", default="tools/plan_parity_scenarios.json")
    parser.add_argument("--p8aj-live-probe-report")
    parser.add_argument("--p8ak-mismatch-warning-report")
    parser.add_argument("--p8bc-launcher-injected-planner-report")
    parser.add_argument("--p8bu-rust-apply-dry-run-bridge-report")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    report = build_readiness_report(
        repo_root=Path.cwd(),
        authored_root=Path(args.authored_root),
        scenario_matrix=Path(args.scenario_matrix),
        p8aj_live_probe_report=Path(args.p8aj_live_probe_report) if args.p8aj_live_probe_report else None,
        p8ak_mismatch_warning_report=Path(args.p8ak_mismatch_warning_report) if args.p8ak_mismatch_warning_report else None,
        p8bc_launcher_injected_planner_report=(
            Path(args.p8bc_launcher_injected_planner_report)
            if args.p8bc_launcher_injected_planner_report
            else None
        ),
        p8bu_rust_apply_dry_run_bridge_report=(
            Path(args.p8bu_rust_apply_dry_run_bridge_report)
            if args.p8bu_rust_apply_dry_run_bridge_report
            else None
        ),
    )
    sys.stdout.write(dumps_report(report))
    return 0 if static_checks_pass(report) else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Static readiness report for future Rust planner default-cutover PRs.

This developer-only gate consolidates static prerequisites, advisory manual
evidence commands, and intentionally remaining blockers. It does not execute the
comparison matrix, smoke runner, Cargo, npm, ADB, planner code, or runtime paths.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


REPORT_KIND = "rust_planner_cutover_readiness_check"
REPORT_SCHEMA_VERSION = 1
SCENARIO_MATRIX_SCHEMA_VERSION = 1
DEVICE_CONTEXT_FIELDS = ("manufacturer", "model", "android_version", "device_tags")
DEVICE_CONTEXT_FIELD_SET = set(DEVICE_CONTEXT_FIELDS)

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
    ("python", "python"),
    ("rust_shadow", "rust-shadow"),
    ("rust_experimental", "rust-experimental"),
    ("rust_planner_bin", "--rust-planner-bin"),
    ("rust_shadow_output", "--rust-shadow-output"),
)

REQUIRED_MANUAL_EVIDENCE = (
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
)

REMAINING_BLOCKERS = (
    {
        "id": "default_cli_backend_still_python",
        "status": "blocked",
    },
    {
        "id": "executor_apply_not_cut_over",
        "status": "blocked",
    },
    {
        "id": "real_device_probing_not_cut_over",
        "status": "blocked",
    },
    {
        "id": "detected_device_profile_mismatch_warning_not_cut_over",
        "status": "blocked",
    },
    {
        "id": "python_planner_deletion_not_ready",
        "status": "blocked",
    },
)


def build_readiness_report(
    *,
    repo_root: Path,
    authored_root: Path,
    scenario_matrix: Path,
) -> dict[str, Any]:
    """Build the deterministic static readiness report.

    `status` intentionally remains `blocked` because static prerequisites alone
    are not enough to make Rust the default planner backend.
    """

    repo_root = repo_root.resolve()
    matrix_path = _resolve_input_path(repo_root, scenario_matrix)
    authored_path = _resolve_input_path(repo_root, authored_root)

    payload, matrix_checks = _scenario_matrix_checks(matrix_path, authored_path)
    static_checks = [
        *matrix_checks,
        *_required_artifact_checks(repo_root),
        *_readiness_doc_reference_checks(repo_root),
        *_cli_backend_token_checks(repo_root),
    ]

    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "blocked",
        "inputs": {
            "authored_root": _display_path(authored_root),
            "scenario_matrix": _display_path(scenario_matrix),
        },
        "static_checks": static_checks,
        "required_manual_evidence": [dict(item) for item in REQUIRED_MANUAL_EVIDENCE],
        "remaining_blockers": [dict(item) for item in REMAINING_BLOCKERS],
    }


def dumps_report(report: dict[str, Any]) -> str:
    """Serialize a report without environment-specific or timing metadata."""

    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def static_checks_pass(report: dict[str, Any]) -> bool:
    """Return whether all static prerequisites passed."""

    return all(check.get("status") == "pass" for check in report.get("static_checks", []))


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
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    report = build_readiness_report(
        repo_root=Path.cwd(),
        authored_root=Path(args.authored_root),
        scenario_matrix=Path(args.scenario_matrix),
    )
    sys.stdout.write(dumps_report(report))
    return 0 if static_checks_pass(report) else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Compare Python planner API output with Rust shadow planner output.

This is a developer-only reporting harness. It does not call the Python CLI,
probe devices, invoke ADB, execute plans, download artifacts, or regenerate
checked-in fixture/golden data.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections import Counter, OrderedDict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any


EMUCHEF_IMPORTED_AT_MODULE_LOAD = "emuchef" in sys.modules
REPORT_SCHEMA_VERSION = 1
SCENARIO_MATRIX_SCHEMA_VERSION = 1
MISSING = object()

# Narrow P7P definition. This does not claim Python CLI parity, real-device
# parity, executor/apply parity, artifact/network/materialization parity, full
# schema parity, future scenario parity, or Rust planner cutover readiness.
MATCH_CLASSIFICATION_DEFINITION = (
    "The dev-only comparison harness found no unclassified differences for "
    "the compared fields under the supplied planner-only bindings and shared "
    "planner context."
)

CLASSIFICATIONS = (
    "match",
    "rust_missing",
    "python_missing",
    "value_mismatch",
    "known_gap",
    "intentional_shape_difference",
    "unsupported",
)

NON_MATCH_CLASSIFICATIONS = {
    "rust_missing",
    "python_missing",
    "value_mismatch",
    "known_gap",
    "unsupported",
}

NORMALIZATIONS = (
    "json_object_key_order_ignored",
    "python_planner_api_result_serialized_as_json",
    "warning_error_shape_compares_code_and_detail_keys",
    "permission_plan_compares_presence_only",
)

RETROARCH_APP_DATA_DEPENDENCIES = {
    "copy_assets",
    "copy_autoconfig",
    "copy_cheats",
    "copy_database_rdb",
    "copy_info",
    "copy_overlays",
    "copy_shaders_glsl",
    "copy_cores",
    "copy_core_system_files",
}


@dataclass(frozen=True, slots=True)
class CommandSpec:
    argv: list[str]
    mode: str


@dataclass(frozen=True, slots=True)
class ProcessResult:
    exit_code: int
    stdout: str
    stderr: str


@dataclass(frozen=True, slots=True)
class ParsedPlanningResult:
    result: dict[str, Any] | None
    issue: dict[str, Any] | None


@dataclass(frozen=True, slots=True)
class KnownGapRule:
    path: str
    classification: str
    code: str
    description: str


@dataclass(frozen=True, slots=True)
class PlanParityBindingSpec:
    ref: str
    kind: str
    suffix: str | None = None


@dataclass(frozen=True, slots=True)
class PlanParityScenario:
    id: str
    device_plan: str
    expected_classification: str
    bindings: tuple[PlanParityBindingSpec, ...]
    notes: str
    known_gap_ids: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class PlanParityScenarioMatrix:
    schema_version: int
    scenarios: tuple[PlanParityScenario, ...]


@dataclass(frozen=True, slots=True)
class PreparedScenarioBindings:
    raw_cli_binds: list[str]
    report_bindings: list[dict[str, str]]


@dataclass(frozen=True, slots=True)
class MatrixScenarioResult:
    scenario: PlanParityScenario
    binding_specs: list[dict[str, str]]
    comparison_report: Mapping[str, Any]


def parse_bindings(raw_bindings: Sequence[str]) -> OrderedDict[str, object]:
    """Parse repeated CLI bind arguments using the planner-visible ref syntax."""

    grouped: OrderedDict[str, list[str]] = OrderedDict()
    for raw_binding in raw_bindings:
        binding_ref, raw_value = _parse_binding(raw_binding)
        grouped.setdefault(binding_ref, []).append(raw_value)
    return OrderedDict(
        (binding_ref, values[0] if len(values) == 1 else values)
        for binding_ref, values in grouped.items()
    )


def _parse_binding(raw_binding: str) -> tuple[str, str]:
    if "=" not in raw_binding:
        raise ValueError(
            f"Invalid --bind value: {raw_binding!r}. "
            "Expected <recipe_ref>/<input_id>=<value>."
        )
    binding_ref, raw_value = raw_binding.split("=", 1)
    if binding_ref.count("/") != 1:
        raise ValueError(
            f"Invalid --bind value: {raw_binding!r}. "
            "Expected <recipe_ref>/<input_id>=<value>."
        )
    recipe_ref, input_id = binding_ref.split("/", 1)
    if not recipe_ref or not input_id:
        raise ValueError(
            f"Invalid --bind value: {raw_binding!r}. "
            "Expected <recipe_ref>/<input_id>=<value>."
        )
    return binding_ref, raw_value


def load_scenario_matrix(path: Path) -> PlanParityScenarioMatrix:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise ValueError(f"Could not read scenario matrix {path}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Scenario matrix {path} is not valid JSON: {exc}") from exc
    return parse_scenario_matrix_payload(payload, source=str(path))


def parse_scenario_matrix_payload(
    payload: object,
    *,
    source: str,
) -> PlanParityScenarioMatrix:
    if not isinstance(payload, Mapping):
        raise ValueError(f"{source}: root must be a JSON object")
    schema_version = payload.get("schema_version")
    if schema_version != SCENARIO_MATRIX_SCHEMA_VERSION:
        raise ValueError(f"{source}: schema_version must be 1")
    raw_scenarios = payload.get("scenarios")
    if not isinstance(raw_scenarios, list):
        raise ValueError(f"{source}: scenarios must be a list")

    scenarios: list[PlanParityScenario] = []
    seen_ids: set[str] = set()
    for index, raw_scenario in enumerate(raw_scenarios):
        scenario = _parse_scenario(raw_scenario, source=source, index=index)
        if scenario.id in seen_ids:
            raise ValueError(f"{source}: scenarios[{index}].id duplicates {scenario.id!r}")
        seen_ids.add(scenario.id)
        scenarios.append(scenario)

    return PlanParityScenarioMatrix(
        schema_version=SCENARIO_MATRIX_SCHEMA_VERSION,
        scenarios=tuple(scenarios),
    )


def _parse_scenario(
    raw_scenario: object,
    *,
    source: str,
    index: int,
) -> PlanParityScenario:
    prefix = f"{source}: scenarios[{index}]"
    if not isinstance(raw_scenario, Mapping):
        raise ValueError(f"{prefix} must be an object")

    scenario_id = _required_string(raw_scenario, "id", f"{prefix}.id")
    device_plan = _required_string(raw_scenario, "device_plan", f"{prefix}.device_plan")
    expected_classification = _required_string(
        raw_scenario,
        "expected_classification",
        f"{prefix}.expected_classification",
    )
    if expected_classification not in CLASSIFICATIONS:
        raise ValueError(
            f"{prefix}.expected_classification must be one of: {', '.join(CLASSIFICATIONS)}"
        )

    raw_bindings = raw_scenario.get("bindings", MISSING)
    if not isinstance(raw_bindings, list):
        raise ValueError(f"{prefix}.bindings must be a list")
    bindings = tuple(
        _parse_binding_spec(raw_binding, source=source, scenario_index=index, binding_index=binding_index)
        for binding_index, raw_binding in enumerate(raw_bindings)
    )

    known_gap_ids = _string_tuple(
        raw_scenario.get("known_gap_ids", []),
        f"{prefix}.known_gap_ids",
    )
    notes = raw_scenario.get("notes", "")
    if not isinstance(notes, str):
        raise ValueError(f"{prefix}.notes must be a string")

    return PlanParityScenario(
        id=scenario_id,
        device_plan=device_plan,
        expected_classification=expected_classification,
        bindings=bindings,
        notes=notes,
        known_gap_ids=known_gap_ids,
    )


def _parse_binding_spec(
    raw_binding: object,
    *,
    source: str,
    scenario_index: int,
    binding_index: int,
) -> PlanParityBindingSpec:
    prefix = f"{source}: scenarios[{scenario_index}].bindings[{binding_index}]"
    if not isinstance(raw_binding, Mapping):
        raise ValueError(f"{prefix} must be an object")
    binding_ref = _required_string(raw_binding, "ref", f"{prefix}.ref")
    _validate_binding_ref(binding_ref, field=f"{prefix}.ref")
    kind = _required_string(raw_binding, "kind", f"{prefix}.kind")
    if kind not in {"directory", "file"}:
        raise ValueError(f"{prefix}.kind must be one of: directory, file")
    suffix = raw_binding.get("suffix")
    if suffix is not None and not isinstance(suffix, str):
        raise ValueError(f"{prefix}.suffix must be a string")
    if kind == "directory":
        if suffix is not None:
            raise ValueError(f"{prefix}.suffix is only valid for file bindings")
        return PlanParityBindingSpec(ref=binding_ref, kind=kind, suffix=None)
    if suffix not in {".apk", ".cfg"}:
        raise ValueError(f"{prefix}.suffix must be one of: .apk, .cfg")
    return PlanParityBindingSpec(ref=binding_ref, kind=kind, suffix=suffix)


def _required_string(raw: Mapping[str, object], key: str, field: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def _string_tuple(raw: object, field: str) -> tuple[str, ...]:
    if not isinstance(raw, list):
        raise ValueError(f"{field} must be a list")
    result: list[str] = []
    for index, value in enumerate(raw):
        if not isinstance(value, str) or not value:
            raise ValueError(f"{field}[{index}] must be a non-empty string")
        result.append(value)
    return tuple(result)


def _validate_binding_ref(binding_ref: str, *, field: str) -> None:
    if binding_ref.count("/") != 1:
        raise ValueError(f"{field} must use <recipe_ref>/<input_id>")
    recipe_ref, input_id = binding_ref.split("/", 1)
    if not recipe_ref or not input_id:
        raise ValueError(f"{field} must use <recipe_ref>/<input_id>")


def prepare_scenario_bindings(
    scenario: PlanParityScenario,
    temp_root: Path,
) -> PreparedScenarioBindings:
    raw_cli_binds: list[str] = []
    report_bindings: list[dict[str, str]] = []
    scenario_root = temp_root / _stable_path_token(scenario.id)
    scenario_root.mkdir(parents=True, exist_ok=True)

    for index, binding in enumerate(scenario.bindings):
        path = _binding_temp_path(scenario_root, index, binding)
        if binding.kind == "directory":
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"")
        raw_cli_binds.append(f"{binding.ref}={path}")
        report_binding = {
            "ref": binding.ref,
            "kind": binding.kind,
        }
        if binding.suffix is not None:
            report_binding["suffix"] = binding.suffix
        report_bindings.append(report_binding)

    return PreparedScenarioBindings(
        raw_cli_binds=raw_cli_binds,
        report_bindings=report_bindings,
    )


def _binding_temp_path(
    scenario_root: Path,
    index: int,
    binding: PlanParityBindingSpec,
) -> Path:
    name = f"{index:02d}_{_stable_path_token(binding.ref)}"
    if binding.kind == "directory":
        return scenario_root / name
    assert binding.suffix is not None
    return scenario_root / f"{name}{binding.suffix}"


def _stable_path_token(value: str) -> str:
    return "".join(char if char.isalnum() or char in "._-" else "_" for char in value)


def build_python_worker_command(
    *,
    python_executable: str,
    script_path: Path,
    authored_root: str,
    device_plan: str,
    binds: Sequence[str],
) -> CommandSpec:
    argv = [
        python_executable,
        str(script_path),
        "__python-planner-worker",
        "--authored-root",
        authored_root,
        "--device-plan",
        device_plan,
    ]
    for raw_binding in binds:
        argv.extend(["--bind", raw_binding])
    return CommandSpec(argv=argv, mode="python_planner_worker")


def build_rust_command(
    *,
    authored_root: str,
    device_plan: str,
    binds: Sequence[str],
    rust_bin: str | None,
    cargo_offline: bool,
    repo_root: Path,
) -> CommandSpec:
    if rust_bin:
        argv = [rust_bin]
        mode = "prebuilt_binary"
    else:
        argv = ["cargo", "run"]
        if cargo_offline:
            argv.append("--offline")
        argv.extend(
            [
                "--quiet",
                "--manifest-path",
                str(repo_root / "crates/emuchef-rust-backend/Cargo.toml"),
                "--bin",
                "emuchef-plan-shadow",
                "--",
            ]
        )
        mode = "cargo_offline" if cargo_offline else "cargo"
    argv.extend(["--authored-root", authored_root, "--device-plan", device_plan])
    for raw_binding in binds:
        argv.extend(["--bind", raw_binding])
    return CommandSpec(argv=argv, mode=mode)


def parse_process_planning_result(*, side: str, process: ProcessResult) -> ParsedPlanningResult:
    if not process.stdout.strip():
        return ParsedPlanningResult(
            result=None,
            issue=_unsupported_issue(
                side=side,
                reason="process_stdout_empty",
                exit_code=process.exit_code,
                stderr=process.stderr,
            ),
        )
    try:
        payload = json.loads(process.stdout)
    except json.JSONDecodeError as exc:
        return ParsedPlanningResult(
            result=None,
            issue=_unsupported_issue(
                side=side,
                reason="process_stdout_not_json",
                exit_code=process.exit_code,
                stderr=process.stderr,
                detail=str(exc),
            ),
        )
    if not isinstance(payload, dict) or payload.get("kind") != "planning_result":
        return ParsedPlanningResult(
            result=None,
            issue=_unsupported_issue(
                side=side,
                reason="process_stdout_not_planning_result",
                exit_code=process.exit_code,
                stderr=process.stderr,
            ),
        )
    return ParsedPlanningResult(result=payload, issue=None)


def _unsupported_issue(
    *,
    side: str,
    reason: str,
    exit_code: int,
    stderr: str,
    detail: str | None = None,
) -> dict[str, Any]:
    issue: dict[str, Any] = {
        "path": f"process.{side}",
        "classification": "unsupported",
        "side": side,
        "reason": reason,
        "exit_code": exit_code,
    }
    if stderr:
        issue["stderr"] = stderr
    if detail:
        issue["detail"] = detail
    return issue


def compare_results(
    python_result: Mapping[str, Any],
    rust_result: Mapping[str, Any],
    *,
    known_gap_rules: Sequence[KnownGapRule],
) -> list[dict[str, Any]]:
    python_normalized = normalize_planning_result(python_result)
    rust_normalized = normalize_planning_result(rust_result)
    rules_by_path = {rule.path: rule for rule in known_gap_rules}
    comparisons = [
        _compare_path(path, python_normalized.get(path, MISSING), rust_normalized.get(path, MISSING), rules_by_path)
        for path in _comparison_paths()
    ]
    comparisons.append(
        {
            "path": "normalization.object_key_order",
            "classification": "intentional_shape_difference",
            "description": "JSON object key order is ignored; list order remains semantic.",
        }
    )
    return comparisons


def normalize_planning_result(result: Mapping[str, Any]) -> dict[str, Any]:
    execution_plan = result.get("execution_plan", MISSING)
    normalized: dict[str, Any] = {
        "top_level.status": result.get("status", MISSING),
        "warnings.shape": _message_shape(result.get("warnings", MISSING)),
        "errors.shape": _message_shape(result.get("errors", MISSING)),
    }
    if not isinstance(execution_plan, Mapping):
        normalized.update(
            {
                "execution_plan.present": execution_plan is not None and execution_plan is not MISSING,
                "source.selected_recipe_refs": MISSING,
                "source.expanded_recipe_refs": MISSING,
                "execution_plan.step_count": MISSING,
                "execution_plan.step_ids": MISSING,
                "execution_plan.step_types": MISSING,
                "execution_plan.dependencies": MISSING,
                "execution_plan.params": MISSING,
                "execution_plan.permission_plan_present": MISSING,
            }
        )
        return normalized

    source = execution_plan.get("source", {})
    steps = execution_plan.get("steps", MISSING)
    step_items = steps if isinstance(steps, list) else []
    normalized.update(
        {
            "execution_plan.present": True,
            "source.selected_recipe_refs": _list_value(source, "selected_recipe_refs"),
            "source.expanded_recipe_refs": _list_value(source, "expanded_recipe_refs"),
            "execution_plan.step_count": len(step_items) if isinstance(steps, list) else MISSING,
            "execution_plan.step_ids": [step.get("id") for step in step_items if isinstance(step, Mapping)],
            "execution_plan.step_types": _step_map(step_items, "type"),
            "execution_plan.dependencies": _step_map(step_items, "dependencies"),
            "execution_plan.params": _step_map(step_items, "params", normalize=True),
            "execution_plan.permission_plan_present": True if "permission_plan" in execution_plan else MISSING,
        }
    )
    return normalized


def _comparison_paths() -> tuple[str, ...]:
    return (
        "top_level.status",
        "source.selected_recipe_refs",
        "source.expanded_recipe_refs",
        "execution_plan.present",
        "execution_plan.step_count",
        "execution_plan.step_ids",
        "execution_plan.step_types",
        "execution_plan.dependencies",
        "execution_plan.params",
        "warnings.shape",
        "errors.shape",
        "execution_plan.permission_plan_present",
    )


def _compare_path(
    path: str,
    python_value: Any,
    rust_value: Any,
    rules_by_path: Mapping[str, KnownGapRule],
) -> dict[str, Any]:
    if python_value is MISSING and rust_value is MISSING:
        classification = "match"
    elif python_value is MISSING:
        classification = "python_missing"
    elif rust_value is MISSING:
        classification = "rust_missing"
    elif python_value == rust_value:
        classification = "match"
    else:
        classification = "value_mismatch"

    item: dict[str, Any] = {
        "path": path,
        "classification": classification,
    }
    if classification != "match":
        item["python"] = _jsonable_missing(python_value)
        item["rust"] = _jsonable_missing(rust_value)
    if classification != "match" and path in rules_by_path:
        rule = rules_by_path[path]
        item.update(
            {
                "classification": rule.classification,
                "known_gap_code": rule.code,
                "description": rule.description,
            }
        )
    return item


def _list_value(source: object, key: str) -> Any:
    if not isinstance(source, Mapping):
        return MISSING
    value = source.get(key, MISSING)
    return value if isinstance(value, list) else MISSING


def _step_map(steps: Sequence[object], key: str, *, normalize: bool = False) -> OrderedDict[str, Any]:
    result: OrderedDict[str, Any] = OrderedDict()
    for step in steps:
        if not isinstance(step, Mapping) or not isinstance(step.get("id"), str):
            continue
        value = step.get(key, MISSING)
        result[step["id"]] = normalize_json_value(value) if normalize and value is not MISSING else value
    return result


def _message_shape(messages: Any) -> Any:
    if messages is MISSING:
        return MISSING
    if not isinstance(messages, list):
        return MISSING
    shape = []
    for message in messages:
        if not isinstance(message, Mapping):
            continue
        details = message.get("details", {})
        shape.append(
            {
                "code": message.get("code"),
                "detail_keys": sorted(details.keys()) if isinstance(details, Mapping) else [],
            }
        )
    return shape


def normalize_json_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        return OrderedDict((key, normalize_json_value(value[key])) for key in sorted(value))
    if isinstance(value, list):
        return [normalize_json_value(item) for item in value]
    return value


def build_report(
    *,
    authored_root: str,
    device_plan: str,
    bindings: OrderedDict[str, object],
    python_result: Mapping[str, Any],
    rust_result: Mapping[str, Any],
    python_mode: str,
    rust_mode: str,
    known_gap_rules: Sequence[KnownGapRule],
) -> dict[str, Any]:
    comparisons = compare_results(python_result, rust_result, known_gap_rules=known_gap_rules)
    diagnostics = diagnose_results(python_result, rust_result)
    known_gaps = [
        {
            "path": item["path"],
            "code": item.get("known_gap_code"),
            "description": item.get("description"),
        }
        for item in comparisons
        if item["classification"] == "known_gap"
    ]
    return {
        "kind": "rust_python_planner_parity_report",
        "schema_version": REPORT_SCHEMA_VERSION,
        "inputs": {
            "comparison": "Python planner API vs Rust shadow planner",
            "authored_root": authored_root,
            "device_plan": device_plan,
            "binding_keys": list(bindings.keys()),
        },
        "metadata": {
            "python_worker_mode": python_mode,
            "rust_command_mode": rust_mode,
            "normalizations": list(NORMALIZATIONS),
        },
        "summary": _summary(comparisons),
        "comparisons": comparisons,
        "known_gaps": known_gaps,
        "diagnostics": diagnostics,
    }


def build_unsupported_report(
    *,
    authored_root: str,
    device_plan: str,
    bindings: OrderedDict[str, object],
    python_mode: str,
    rust_mode: str,
    issues: Sequence[dict[str, Any]],
) -> dict[str, Any]:
    comparisons = list(issues)
    return {
        "kind": "rust_python_planner_parity_report",
        "schema_version": REPORT_SCHEMA_VERSION,
        "inputs": {
            "comparison": "Python planner API vs Rust shadow planner",
            "authored_root": authored_root,
            "device_plan": device_plan,
            "binding_keys": list(bindings.keys()),
        },
        "metadata": {
            "python_worker_mode": python_mode,
            "rust_command_mode": rust_mode,
            "normalizations": list(NORMALIZATIONS),
        },
        "summary": _summary(comparisons),
        "comparisons": comparisons,
        "known_gaps": [],
        "diagnostics": [],
    }


def build_matrix_report(
    *,
    authored_root: str,
    matrix_path: str,
    matrix: PlanParityScenarioMatrix,
    scenario_results: Sequence[MatrixScenarioResult],
) -> dict[str, Any]:
    scenarios: list[dict[str, Any]] = []
    known_gaps: list[dict[str, Any]] = []
    expectation_statuses: list[str] = []
    expected_classifications: list[str] = []
    actual_classifications: list[str] = []
    aggregate_mismatch_counts: Counter[str] = Counter()

    for result in scenario_results:
        scenario = result.scenario
        comparison_summary = result.comparison_report.get("summary", {})
        actual_classification = comparison_summary.get("classification", "unsupported")
        expected_classification = scenario.expected_classification
        expectation_status = (
            "pass" if actual_classification == expected_classification else "fail"
        )
        expectation_statuses.append(expectation_status)
        expected_classifications.append(expected_classification)
        actual_classifications.append(actual_classification)

        counts = comparison_summary.get("counts", {})
        mismatch_buckets = _mismatch_buckets(counts)
        aggregate_mismatch_counts.update(mismatch_buckets)

        for known_gap in result.comparison_report.get("known_gaps", []):
            gap_item = {
                "scenario_id": scenario.id,
                "device_plan": scenario.device_plan,
            }
            if isinstance(known_gap, Mapping):
                for key in ("path", "code", "description"):
                    if key in known_gap:
                        gap_item[key] = known_gap[key]
            known_gaps.append(gap_item)

        scenarios.append(
            {
                "scenario_id": scenario.id,
                "device_plan": scenario.device_plan,
                "bindings": result.binding_specs,
                "expected_classification": expected_classification,
                "actual_classification": actual_classification,
                "expectation_status": expectation_status,
                "known_gap_ids": list(scenario.known_gap_ids),
                "notes": scenario.notes,
                "summary_counts": _ordered_count_mapping(counts),
                "mismatch_buckets": mismatch_buckets,
                "known_gaps": result.comparison_report.get("known_gaps", []),
                "diagnostics": result.comparison_report.get("diagnostics", []),
            }
        )

    expectation_counts = Counter(expectation_statuses)
    return {
        "kind": "rust_python_planner_parity_matrix_report",
        "schema_version": REPORT_SCHEMA_VERSION,
        "matrix_schema_version": matrix.schema_version,
        "inputs": {
            "comparison": "Python planner API vs Rust shadow planner",
            "authored_root": authored_root,
            "scenario_matrix": matrix_path,
        },
        "metadata": {
            "match_classification": MATCH_CLASSIFICATION_DEFINITION,
            "normalizations": list(NORMALIZATIONS),
            "exit_semantics": (
                "Matrix mode exits 0 only when every scenario actual classification "
                "matches its expected classification."
            ),
        },
        "summary": {
            "scenario_count": len(scenarios),
            "expectation_counts": {
                "pass": expectation_counts.get("pass", 0),
                "fail": expectation_counts.get("fail", 0),
            },
            "expected_classification_counts": _classification_counts(expected_classifications),
            "actual_classification_counts": _classification_counts(actual_classifications),
            "mismatch_buckets": _nonzero_ordered_counts(aggregate_mismatch_counts),
        },
        "scenarios": scenarios,
        "known_gaps": known_gaps,
    }


def matrix_exit_code(report: Mapping[str, Any]) -> int:
    summary = report.get("summary", {})
    expectation_counts = summary.get("expectation_counts", {})
    if not isinstance(expectation_counts, Mapping):
        return 1
    return 0 if expectation_counts.get("fail") == 0 else 1


def _classification_counts(classifications: Sequence[str]) -> OrderedDict[str, int]:
    counts = Counter(classifications)
    return OrderedDict((name, counts.get(name, 0)) for name in CLASSIFICATIONS)


def _ordered_count_mapping(counts: object) -> OrderedDict[str, int]:
    if not isinstance(counts, Mapping):
        counts = {}
    return OrderedDict((name, int(counts.get(name, 0))) for name in CLASSIFICATIONS)


def _mismatch_buckets(counts: object) -> OrderedDict[str, int]:
    ordered_counts = _ordered_count_mapping(counts)
    return OrderedDict(
        (name, count)
        for name, count in ordered_counts.items()
        if name in NON_MATCH_CLASSIFICATIONS and count
    )


def _nonzero_ordered_counts(counts: Counter[str]) -> OrderedDict[str, int]:
    return OrderedDict(
        (name, counts.get(name, 0))
        for name in CLASSIFICATIONS
        if counts.get(name, 0)
    )


def diagnose_results(python_result: Mapping[str, Any], rust_result: Mapping[str, Any]) -> list[dict[str, Any]]:
    if _is_retroarch_app_data_write_rust_bug(python_result, rust_result):
        return [
            {
                "classification": "rust_planner_bug",
                "category": "rust_optional_step_pruning_dependency_bug",
                "description": (
                    "Python planner API succeeds under the shared synthetic/profile-derived "
                    "context by pruning RetroArch launch and app-data copy steps. Rust shadow "
                    "planner returns unknown_step_dependency for launch_retroarch dependencies "
                    "that were not emitted because app_data_write is false."
                ),
            }
        ]
    return []


def _is_retroarch_app_data_write_rust_bug(
    python_result: Mapping[str, Any],
    rust_result: Mapping[str, Any],
) -> bool:
    python_plan = python_result.get("execution_plan")
    if python_result.get("status") != "success" or not isinstance(python_plan, Mapping):
        return False
    if rust_result.get("status") != "error" or rust_result.get("execution_plan") is not None:
        return False
    rust_errors = rust_result.get("errors")
    if not isinstance(rust_errors, list) or not rust_errors:
        return False
    matching_dependencies = set()
    for error in rust_errors:
        if not isinstance(error, Mapping) or error.get("code") != "unknown_step_dependency":
            return False
        details = error.get("details", {})
        if not isinstance(details, Mapping):
            return False
        if details.get("recipe_ref") != "app.retroarch.provision" or details.get("step_id") != "launch_retroarch":
            return False
        dependency = details.get("dependency")
        if dependency not in RETROARCH_APP_DATA_DEPENDENCIES:
            return False
        matching_dependencies.add(dependency)
    python_step_ids = {
        step.get("id")
        for step in python_plan.get("steps", [])
        if isinstance(step, Mapping)
    }
    return (
        bool(matching_dependencies)
        and "app.retroarch.provision/launch_retroarch" not in python_step_ids
        and not any(f"app.retroarch.provision/{dependency}" in python_step_ids for dependency in matching_dependencies)
    )


def _summary(comparisons: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    counts = Counter(item["classification"] for item in comparisons)
    ordered_counts = OrderedDict((name, counts.get(name, 0)) for name in CLASSIFICATIONS)
    return {
        "classification": _overall_classification(counts),
        "counts": ordered_counts,
    }


def _overall_classification(counts: Counter[str]) -> str:
    if counts.get("unsupported", 0):
        return "unsupported"
    if counts.get("value_mismatch", 0):
        return "value_mismatch"
    if counts.get("rust_missing", 0):
        return "rust_missing"
    if counts.get("python_missing", 0):
        return "python_missing"
    if counts.get("known_gap", 0):
        return "known_gap"
    return "match"


def dumps_report(report: Mapping[str, Any]) -> str:
    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def run_process(spec: CommandSpec, *, cwd: Path) -> ProcessResult:
    completed = subprocess.run(
        spec.argv,
        cwd=str(cwd),
        check=False,
        text=True,
        capture_output=True,
    )
    return ProcessResult(
        exit_code=completed.returncode,
        stdout=completed.stdout,
        stderr=completed.stderr,
    )


def _jsonable_missing(value: Any) -> Any:
    if value is MISSING:
        return {"missing": True}
    if isinstance(value, OrderedDict):
        return dict(value)
    return value


def _repo_root_from_script() -> Path:
    return Path(__file__).resolve().parents[1]


def _cargo_offline_default() -> bool:
    value = os.environ.get("EMUCHEF_PLAN_COMPARE_CARGO_OFFLINE")
    if value is None:
        return True
    return value.strip().lower() not in {"0", "false", "no", "off"}


def run_single_comparison_report(
    *,
    authored_root: str,
    device_plan: str,
    raw_binds: Sequence[str],
    rust_bin: str | None,
    python_executable: str,
    cargo_online: bool,
    repo_root: Path,
    script_path: Path,
) -> dict[str, Any]:
    bindings = parse_bindings(raw_binds)
    python_spec = build_python_worker_command(
        python_executable=python_executable,
        script_path=script_path,
        authored_root=authored_root,
        device_plan=device_plan,
        binds=raw_binds,
    )
    rust_spec = build_rust_command(
        authored_root=authored_root,
        device_plan=device_plan,
        binds=raw_binds,
        rust_bin=rust_bin,
        cargo_offline=_cargo_offline_default() and not cargo_online,
        repo_root=repo_root,
    )

    python_process = run_process(python_spec, cwd=repo_root)
    rust_process = run_process(rust_spec, cwd=repo_root)
    python_parsed = parse_process_planning_result(side="python", process=python_process)
    rust_parsed = parse_process_planning_result(side="rust", process=rust_process)
    issues = [
        issue
        for issue in (python_parsed.issue, rust_parsed.issue)
        if issue is not None
    ]
    if issues:
        return build_unsupported_report(
            authored_root=authored_root,
            device_plan=device_plan,
            bindings=bindings,
            python_mode=python_spec.mode,
            rust_mode=rust_spec.mode,
            issues=issues,
        )

    assert python_parsed.result is not None
    assert rust_parsed.result is not None
    return build_report(
        authored_root=authored_root,
        device_plan=device_plan,
        bindings=bindings,
        python_result=python_parsed.result,
        rust_result=rust_parsed.result,
        python_mode=python_spec.mode,
        rust_mode=rust_spec.mode,
        known_gap_rules=[],
    )


def run_scenario_matrix_report(
    *,
    authored_root: str,
    matrix_path: Path,
    matrix: PlanParityScenarioMatrix,
    rust_bin: str | None,
    python_executable: str,
    cargo_online: bool,
    repo_root: Path,
    script_path: Path,
) -> dict[str, Any]:
    scenario_results: list[MatrixScenarioResult] = []
    with tempfile.TemporaryDirectory(prefix="emuchef-plan-parity-") as temp_dir:
        temp_root = Path(temp_dir)
        for scenario in matrix.scenarios:
            prepared = prepare_scenario_bindings(scenario, temp_root)
            comparison_report = run_single_comparison_report(
                authored_root=authored_root,
                device_plan=scenario.device_plan,
                raw_binds=prepared.raw_cli_binds,
                rust_bin=rust_bin,
                python_executable=python_executable,
                cargo_online=cargo_online,
                repo_root=repo_root,
                script_path=script_path,
            )
            scenario_results.append(
                MatrixScenarioResult(
                    scenario=scenario,
                    binding_specs=prepared.report_bindings,
                    comparison_report=comparison_report,
                )
            )

    return build_matrix_report(
        authored_root=authored_root,
        matrix_path=str(matrix_path),
        matrix=matrix,
        scenario_results=scenario_results,
    )


def compare_main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Compare Python planner API output with Rust shadow planner output."
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan")
    parser.add_argument("--scenario-matrix")
    parser.add_argument("--bind", action="append", default=[])
    parser.add_argument("--rust-bin")
    parser.add_argument("--python-executable", default=sys.executable)
    parser.add_argument(
        "--cargo-online",
        action="store_true",
        help="Allow Cargo network/index access instead of the default --offline mode.",
    )
    args = parser.parse_args(argv)

    if bool(args.device_plan) == bool(args.scenario_matrix):
        parser.error("exactly one of --device-plan or --scenario-matrix is required")
    if args.scenario_matrix and args.bind:
        parser.error("--bind is only supported with --device-plan")

    repo_root = _repo_root_from_script()
    script_path = Path(__file__).resolve()

    if args.scenario_matrix:
        try:
            matrix_path = Path(args.scenario_matrix)
            matrix = load_scenario_matrix(matrix_path)
        except ValueError as exc:
            parser.error(str(exc))
        report = run_scenario_matrix_report(
            authored_root=args.authored_root,
            matrix_path=matrix_path,
            matrix=matrix,
            rust_bin=args.rust_bin,
            python_executable=args.python_executable,
            cargo_online=args.cargo_online,
            repo_root=repo_root,
            script_path=script_path,
        )
        sys.stdout.write(dumps_report(report))
        return matrix_exit_code(report)

    try:
        report = run_single_comparison_report(
            authored_root=args.authored_root,
            device_plan=args.device_plan,
            raw_binds=args.bind,
            rust_bin=args.rust_bin,
            python_executable=args.python_executable,
            cargo_online=args.cargo_online,
            repo_root=repo_root,
            script_path=script_path,
        )
    except ValueError as exc:
        parser.error(str(exc))
    sys.stdout.write(dumps_report(report))
    return 0 if report["summary"]["classification"] == "match" else 1


def python_worker_main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Internal worker for Python planner API comparison."
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    parser.add_argument("--bind", action="append", default=[])
    args = parser.parse_args(argv)

    repo_root = _repo_root_from_script()
    src_root = repo_root / "src"
    if src_root.exists():
        sys.path.insert(0, str(src_root))

    try:
        result = _run_python_planner_api(
            authored_root=args.authored_root,
            device_plan_ref=args.device_plan,
            bindings=parse_bindings(args.bind),
        )
    except Exception as exc:  # pragma: no cover - exercised through process-level reports.
        sys.stdout.write(
            json.dumps(
                {
                    "kind": "python_planner_worker_error",
                    "code": "python_worker_failed",
                    "message": str(exc),
                },
                indent=2,
                sort_keys=False,
            )
            + "\n"
        )
        return 1

    sys.stdout.write(json.dumps(result, indent=2, sort_keys=False) + "\n")
    return 0 if result.get("status") == "success" else 1


def _run_python_planner_api(
    *,
    authored_root: str,
    device_plan_ref: str,
    bindings: OrderedDict[str, object],
) -> dict[str, Any]:
    # Imports stay inside worker mode so the comparison module remains usable
    # without planner dependencies installed.
    from emuchef.domain import DeviceContext
    from emuchef.io import load_authored_catalog
    from emuchef.io.serde import to_primitive
    from emuchef.planner import Planner

    catalog = load_authored_catalog(authored_root)
    device_plan = catalog.device_plans[device_plan_ref]
    device_profile = catalog.device_profiles[device_plan.device_profile_ref]
    session = Planner(catalog).start_session(
        device_plan_ref=device_plan_ref,
        device_context=_synthetic_device_context(device_profile),
        runtime_capabilities=device_profile.capability_defaults,
        plan_id=f"plan.shadow.{device_plan_ref}.001",
    )
    for binding_ref, value in bindings.items():
        update = session.bind_input(binding_ref, value)
        if update.errors:
            return {
                "kind": "python_planner_worker_error",
                "code": "bind_update_failed",
                "errors": to_primitive(update.errors),
            }
    return to_primitive(session.emit_execution_plan())


def _synthetic_device_context(device_profile: object) -> object:
    from emuchef.domain import DeviceContext

    match = device_profile.match
    android_version = 0
    if match.android_version is not None and match.android_version.min is not None:
        android_version = match.android_version.min
    manufacturer = (
        match.manufacturer_contains[0]
        if match.manufacturer_contains
        else f"profile:{device_profile.id}"
    )
    model = device_profile.name or f"profile:{device_profile.id}"
    return DeviceContext(
        manufacturer=manufacturer,
        model=model,
        android_version=android_version,
        android_api_level=None,
        device_tags=device_profile.device_tags,
    )


def main(argv: Sequence[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == "__python-planner-worker":
        return python_worker_main(argv[1:])
    return compare_main(argv)


if __name__ == "__main__":
    raise SystemExit(main())

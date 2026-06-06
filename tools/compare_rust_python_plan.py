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
from collections import Counter, OrderedDict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any


EMUCHEF_IMPORTED_AT_MODULE_LOAD = "emuchef" in sys.modules
REPORT_SCHEMA_VERSION = 1
MISSING = object()

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


def compare_main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Compare Python planner API output with Rust shadow planner output."
    )
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--device-plan", required=True)
    parser.add_argument("--bind", action="append", default=[])
    parser.add_argument("--rust-bin")
    parser.add_argument("--python-executable", default=sys.executable)
    parser.add_argument(
        "--cargo-online",
        action="store_true",
        help="Allow Cargo network/index access instead of the default --offline mode.",
    )
    args = parser.parse_args(argv)

    try:
        bindings = parse_bindings(args.bind)
    except ValueError as exc:
        parser.error(str(exc))

    repo_root = _repo_root_from_script()
    script_path = Path(__file__).resolve()
    python_spec = build_python_worker_command(
        python_executable=args.python_executable,
        script_path=script_path,
        authored_root=args.authored_root,
        device_plan=args.device_plan,
        binds=args.bind,
    )
    rust_spec = build_rust_command(
        authored_root=args.authored_root,
        device_plan=args.device_plan,
        binds=args.bind,
        rust_bin=args.rust_bin,
        cargo_offline=_cargo_offline_default() and not args.cargo_online,
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
        report = build_unsupported_report(
            authored_root=args.authored_root,
            device_plan=args.device_plan,
            bindings=bindings,
            python_mode=python_spec.mode,
            rust_mode=rust_spec.mode,
            issues=issues,
        )
    else:
        assert python_parsed.result is not None
        assert rust_parsed.result is not None
        report = build_report(
            authored_root=args.authored_root,
            device_plan=args.device_plan,
            bindings=bindings,
            python_result=python_parsed.result,
            rust_result=rust_parsed.result,
            python_mode=python_spec.mode,
            rust_mode=rust_spec.mode,
            known_gap_rules=[],
        )
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

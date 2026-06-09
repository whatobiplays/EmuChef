#!/usr/bin/env python3
"""Smoke the explicit Python CLI route to the Rust shadow planner.

This developer-only tool proves that the opt-in ``emuchef plan
--planner-backend rust-shadow`` route can invoke a supplied shadow planner
binary across the current scenario matrix. It does not compare planner output,
execute plans, probe devices, materialize artifacts, or regenerate fixtures.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


REPORT_SCHEMA_VERSION = 1
SCENARIO_MATRIX_SCHEMA_VERSION = 1
FAILURE_TEXT_LIMIT = 240
ROUTE_OUTPUT_MODES = ("passthrough", "python-compatible")


@dataclass(frozen=True, slots=True)
class PlanParityBindingSpec:
    ref: str
    kind: str
    suffix: str | None = None


@dataclass(frozen=True, slots=True)
class PlanParityScenario:
    id: str
    device_plan: str
    expected_route_exit_code: int
    bindings: tuple[PlanParityBindingSpec, ...]


@dataclass(frozen=True, slots=True)
class PlanParityScenarioMatrix:
    schema_version: int
    scenarios: tuple[PlanParityScenario, ...]


@dataclass(frozen=True, slots=True)
class PreparedScenarioBindings:
    raw_cli_binds: list[str]
    report_bindings: list[dict[str, str]]


@dataclass(frozen=True, slots=True)
class ScenarioSmokeResult:
    scenario: PlanParityScenario
    report_bindings: list[dict[str, str]]
    expected_route_exit_code: int
    actual_exit_code: int
    stdout_classification: str
    stderr_classification: str
    expected_stdout_class: str = "not_enforced"
    command_classification: str = "rust_shadow_passthrough_implicit"
    failure_summary: str | None = None


def load_scenario_matrix(path: Path) -> PlanParityScenarioMatrix:
    # SECURITY-REVIEW: This reads a developer-supplied matrix path only; no
    # external content is deserialized beyond JSON values validated below.
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
    expected_route_exit_code = raw_scenario.get("expected_route_exit_code", 0)
    if not isinstance(expected_route_exit_code, int):
        raise ValueError(f"{prefix}.expected_route_exit_code must be an integer")

    raw_bindings = raw_scenario.get("bindings")
    if not isinstance(raw_bindings, list):
        raise ValueError(f"{prefix}.bindings must be a list")
    bindings = tuple(
        _parse_binding_spec(raw_binding, source=source, scenario_index=index, binding_index=binding_index)
        for binding_index, raw_binding in enumerate(raw_bindings)
    )

    return PlanParityScenario(
        id=scenario_id,
        device_plan=device_plan,
        expected_route_exit_code=expected_route_exit_code,
        bindings=bindings,
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
    return PlanParityBindingSpec(ref=binding_ref, kind=kind, suffix=suffix or "")


def _required_string(raw: Mapping[str, object], key: str, field: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


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
    # SECURITY-REVIEW: Placeholder resources are created only under the
    # caller-owned temporary directory and are never used for artifact execution.
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
        if binding.suffix:
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
    return scenario_root / f"{name}{binding.suffix}"


def _stable_path_token(value: str) -> str:
    return "".join(char if char.isalnum() or char in "._-" else "_" for char in value)


def build_cli_command(
    *,
    python_executable: str,
    authored_root: str,
    rust_planner_bin: str,
    scenario: PlanParityScenario,
    raw_binds: Sequence[str],
    route_output_mode: str = "passthrough",
) -> list[str]:
    _validate_route_output_mode(route_output_mode)
    command = [
        python_executable,
        "-m",
        "emuchef",
        "plan",
        "--planner-backend",
        "rust-shadow",
    ]
    if route_output_mode == "python-compatible":
        command.extend(["--rust-shadow-output", "python-compatible"])
    command.extend(
        [
            "--rust-planner-bin",
            rust_planner_bin,
            "--authored-root",
            authored_root,
            "--device-plan",
            scenario.device_plan,
        ]
    )
    for raw_bind in raw_binds:
        command.extend(["--bind", raw_bind])
    return command


def run_process(argv: Sequence[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    # SECURITY-REVIEW: This developer tool runs a structured argv list with
    # shell=False to exercise the explicit local CLI route only.
    return subprocess.run(
        list(argv),
        cwd=str(cwd),
        check=False,
        text=True,
        capture_output=True,
    )


def run_scenario_matrix_report(
    *,
    scenario_matrix: Path,
    authored_root: str,
    rust_planner_bin: str,
    python_executable: str,
    repo_root: Path,
    route_output_mode: str = "passthrough",
) -> dict[str, object]:
    _validate_route_output_mode(route_output_mode)
    matrix = load_scenario_matrix(scenario_matrix)
    scenario_results: list[ScenarioSmokeResult] = []

    with tempfile.TemporaryDirectory(prefix="emuchef-rust-shadow-cli-") as temp_dir:
        temp_root = Path(temp_dir)
        for scenario in matrix.scenarios:
            prepared = prepare_scenario_bindings(scenario, temp_root)
            command = build_cli_command(
                python_executable=python_executable,
                authored_root=authored_root,
                rust_planner_bin=rust_planner_bin,
                scenario=scenario,
                raw_binds=prepared.raw_cli_binds,
                route_output_mode=route_output_mode,
            )
            completed = run_process(command, cwd=repo_root)
            scenario_results.append(
                _scenario_result(
                    scenario=scenario,
                    report_bindings=prepared.report_bindings,
                    completed=completed,
                    temp_root=temp_root,
                    route_output_mode=route_output_mode,
                )
            )

    return build_report(
        scenario_matrix=str(scenario_matrix),
        authored_root=authored_root,
        rust_planner_bin=rust_planner_bin,
        python_executable=python_executable,
        route_output_mode=route_output_mode,
        matrix=matrix,
        scenario_results=scenario_results,
    )


def _scenario_result(
    *,
    scenario: PlanParityScenario,
    report_bindings: list[dict[str, str]],
    completed: subprocess.CompletedProcess[str],
    temp_root: Path,
    route_output_mode: str,
) -> ScenarioSmokeResult:
    expected = scenario.expected_route_exit_code
    actual = int(completed.returncode)
    stdout_classification = classify_stdout(completed.stdout, route_output_mode=route_output_mode)
    expected_stdout_class = _expected_stdout_class(
        route_output_mode=route_output_mode,
        expected_route_exit_code=expected,
    )
    failure_summary = None
    if actual != expected or not _stdout_expectation_met(
        stdout_classification=stdout_classification,
        expected_stdout_class=expected_stdout_class,
        expected_route_exit_code=expected,
        actual_exit_code=actual,
    ):
        failure_summary = _failure_summary(
            expected=expected,
            actual=actual,
            expected_stdout_class=expected_stdout_class,
            stdout_classification=stdout_classification,
            stdout=completed.stdout,
            stderr=completed.stderr,
            temp_root=temp_root,
        )
    return ScenarioSmokeResult(
        scenario=scenario,
        report_bindings=report_bindings,
        expected_route_exit_code=expected,
        actual_exit_code=actual,
        stdout_classification=stdout_classification,
        expected_stdout_class=expected_stdout_class,
        stderr_classification=classify_stderr(completed.stderr),
        command_classification=_command_classification(route_output_mode),
        failure_summary=failure_summary,
    )


def classify_stdout(stdout: str, *, route_output_mode: str = "passthrough") -> str:
    _validate_route_output_mode(route_output_mode)
    if not stdout.strip():
        return "stdout_empty"
    try:
        json.loads(stdout)
    except json.JSONDecodeError:
        if route_output_mode == "python-compatible":
            if "Planning status:" in stdout:
                return "python_summary"
            if "kind: planning_result" in stdout:
                return "python_yaml"
        return "stdout_text"
    return "stdout_json"


def classify_stderr(stderr: str) -> str:
    return "stderr_empty" if not stderr.strip() else "stderr_text"


def build_report(
    *,
    scenario_matrix: str,
    authored_root: str,
    rust_planner_bin: str,
    python_executable: str,
    route_output_mode: str,
    matrix: PlanParityScenarioMatrix,
    scenario_results: Sequence[ScenarioSmokeResult],
) -> dict[str, object]:
    _validate_route_output_mode(route_output_mode)
    scenario_items: list[dict[str, object]] = []
    failure_items: list[dict[str, object]] = []
    pass_count = 0

    for result in scenario_results:
        status = "pass" if _scenario_smoke_passed(result) else "fail"
        if status == "pass":
            pass_count += 1
        scenario_item: dict[str, object] = {
            "scenario_id": result.scenario.id,
            "device_plan": result.scenario.device_plan,
            "bindings": result.report_bindings,
            "expected_route_exit_code": result.expected_route_exit_code,
            "actual_exit_code": result.actual_exit_code,
            "status": status,
            "stdout_classification": result.stdout_classification,
            "expected_stdout_class": result.expected_stdout_class,
            "stderr_classification": result.stderr_classification,
            "command_classification": result.command_classification,
        }
        if result.failure_summary is not None:
            scenario_item["failure_summary"] = result.failure_summary
            failure_items.append(
                {
                    "scenario_id": result.scenario.id,
                    "device_plan": result.scenario.device_plan,
                    "summary": result.failure_summary,
                }
            )
        scenario_items.append(scenario_item)

    fail_count = len(scenario_results) - pass_count
    return {
        "kind": "rust_shadow_cli_matrix_smoke_report",
        "schema_version": REPORT_SCHEMA_VERSION,
        "inputs": {
            "scenario_matrix": scenario_matrix,
            "authored_root": authored_root,
            "rust_planner_bin": _stable_basename(rust_planner_bin),
            "python_executable": _stable_basename(python_executable),
            "route_output_mode": route_output_mode,
        },
        "summary": {
            "total_scenarios": len(matrix.scenarios),
            "pass_count": pass_count,
            "fail_count": fail_count,
        },
        "scenarios": scenario_items,
        "failures": failure_items,
    }


def matrix_exit_code(report: Mapping[str, object]) -> int:
    summary = report.get("summary", {})
    if not isinstance(summary, Mapping):
        return 1
    return 0 if summary.get("fail_count") == 0 else 1


def dumps_report(report: Mapping[str, object]) -> str:
    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def _stable_basename(value: str) -> str:
    return Path(value).name or value


def _failure_summary(
    *,
    expected: int,
    actual: int,
    expected_stdout_class: str,
    stdout_classification: str,
    stdout: str,
    stderr: str,
    temp_root: Path,
) -> str:
    parts = [f"expected exit {expected}, actual exit {actual}"]
    if expected_stdout_class != "not_enforced" and stdout_classification != expected_stdout_class:
        parts.append(f"expected stdout {expected_stdout_class}, actual {stdout_classification}")
    if stdout.strip():
        parts.append(f"stdout: {_bounded_output(stdout, temp_root=temp_root)}")
    if stderr.strip():
        parts.append(f"stderr: {_bounded_output(stderr, temp_root=temp_root)}")
    return "; ".join(parts)


def _bounded_output(text: str, *, temp_root: Path) -> str:
    normalized = " ".join(text.split())
    normalized = normalized.replace(str(temp_root), "$TEMP")
    normalized = normalized.replace(tempfile.gettempdir(), "$TEMP")
    normalized = normalized.replace("/tmp", "$TEMP")
    if len(normalized) > FAILURE_TEXT_LIMIT:
        return normalized[: FAILURE_TEXT_LIMIT - 3] + "..."
    return normalized


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="smoke_rust_shadow_cli_matrix.py",
        description="Smoke the explicit Python CLI rust-shadow planner route across a scenario matrix."
    )
    parser.add_argument("--scenario-matrix", required=True)
    parser.add_argument("--authored-root", required=True)
    parser.add_argument("--rust-planner-bin")
    parser.add_argument(
        "--rust-shadow-output",
        choices=ROUTE_OUTPUT_MODES,
        default="passthrough",
    )
    parser.add_argument("--python-executable", default=sys.executable)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else 2
    if not args.rust_planner_bin:
        sys.stderr.write("smoke_rust_shadow_cli_matrix.py: error: --rust-planner-bin is required\n")
        return 2

    try:
        report = run_scenario_matrix_report(
            scenario_matrix=Path(args.scenario_matrix),
            authored_root=args.authored_root,
            rust_planner_bin=args.rust_planner_bin,
            python_executable=args.python_executable,
            repo_root=Path.cwd(),
            route_output_mode=args.rust_shadow_output,
        )
    except ValueError as exc:
        parser.error(str(exc))
        return 2
    sys.stdout.write(dumps_report(report))
    return matrix_exit_code(report)


def _validate_route_output_mode(route_output_mode: str) -> None:
    if route_output_mode not in ROUTE_OUTPUT_MODES:
        raise ValueError(f"route_output_mode must be one of: {', '.join(ROUTE_OUTPUT_MODES)}")


def _expected_stdout_class(
    *,
    route_output_mode: str,
    expected_route_exit_code: int,
) -> str:
    if route_output_mode == "python-compatible" and expected_route_exit_code == 0:
        return "python_summary"
    return "not_enforced"


def _stdout_expectation_met(
    *,
    stdout_classification: str,
    expected_stdout_class: str,
    expected_route_exit_code: int,
    actual_exit_code: int,
) -> bool:
    if expected_stdout_class == "not_enforced":
        return True
    if expected_route_exit_code != 0 or actual_exit_code != 0:
        return True
    return stdout_classification == expected_stdout_class


def _command_classification(route_output_mode: str) -> str:
    if route_output_mode == "python-compatible":
        return "rust_shadow_python_compatible_explicit"
    return "rust_shadow_passthrough_implicit"


def _scenario_smoke_passed(result: ScenarioSmokeResult) -> bool:
    if result.actual_exit_code != result.expected_route_exit_code:
        return False
    return _stdout_expectation_met(
        stdout_classification=result.stdout_classification,
        expected_stdout_class=result.expected_stdout_class,
        expected_route_exit_code=result.expected_route_exit_code,
        actual_exit_code=result.actual_exit_code,
    )


if __name__ == "__main__":
    raise SystemExit(main())

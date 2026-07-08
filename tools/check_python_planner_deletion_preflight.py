#!/usr/bin/env python3
"""Static preflight for the Python planner deletion sequence.

The preflight inventories tracked public ``emuchef plan`` fallback, import,
runtime, readiness, and test surfaces that still intentionally keep the Python
planner reachable. It reads checked-in source files as text only; it does not
import EmuChef modules, execute planner code, invoke subprocesses, probe
devices, or require Rust binaries.
"""

from __future__ import annotations

import argparse
import ast
import json
import sys
from pathlib import Path
from typing import Any, Callable


REPORT_KIND = "python_planner_deletion_preflight"
REPORT_SCHEMA_VERSION = 1

CLI_PATH = Path("src/emuchef/cli.py")
READINESS_GATE_PATH = Path("tools/check_rust_planner_cutover_readiness.py")
TEST_CLI_PATH = Path("tests/test_cli.py")
TEST_READINESS_PATH = Path("tests/test_check_rust_planner_cutover_readiness.py")
TEST_LAUNCHER_SMOKE_PATH = Path("tests/test_smoke_launcher_injected_planner.py")

DELETION_STEP_ORDER = (
    "remove explicit Python planner backend from CLI",
    "remove _run_python_plan runtime path",
    "remove CLI runtime imports from emuchef.planner",
    "remove _run_plan fallback routing to Python planning",
    "move still-needed shared types/helpers out of planner-owned modules",
    "update tests that assert Python fallback availability",
    "update readiness gate assertions for Python planner deletion blocker",
)

SURFACE_DELETION_STEPS = {
    "cli_imports_emuchef_planner": (
        "remove CLI runtime imports from emuchef.planner",
        "move still-needed shared types/helpers out of planner-owned modules",
    ),
    "cli_explicit_python_backend": ("remove explicit Python planner backend from CLI",),
    "cli_run_python_plan_function": ("remove _run_python_plan runtime path",),
    "cli_run_plan_routes_to_python_plan": ("remove _run_plan fallback routing to Python planning",),
    "readiness_gate_python_deletion_blocker": (
        "update readiness gate assertions for Python planner deletion blocker",
    ),
    "test_cli_explicit_python_backend_behavior": (
        "update tests that assert Python fallback availability",
    ),
    "test_readiness_python_backend_or_deletion_assertions": (
        "update tests that assert Python fallback availability",
        "update readiness gate assertions for Python planner deletion blocker",
    ),
    "test_launcher_smoke_python_help_exposure": (
        "update tests that assert Python fallback availability",
    ),
}


def build_preflight_report(repo_root: Path) -> dict[str, Any]:
    """Build the deterministic static Python planner deletion preflight report."""

    remaining_surfaces = _remaining_python_planner_surfaces(repo_root)
    return {
        "kind": REPORT_KIND,
        "schema_version": REPORT_SCHEMA_VERSION,
        "status": "blocked" if remaining_surfaces else "ready",
        "remaining_python_planner_surfaces": remaining_surfaces,
        "required_deletion_steps": _required_deletion_steps(remaining_surfaces),
    }


def dumps_report(report: dict[str, Any]) -> str:
    """Serialize a preflight report without timestamps or host-specific data."""

    return json.dumps(report, indent=2, sort_keys=False) + "\n"


def preflight_passed(report: dict[str, Any]) -> bool:
    """Return whether the preflight found no remaining Python planner surfaces."""

    return report.get("status") == "ready" and not report.get("remaining_python_planner_surfaces")


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    report = build_preflight_report(Path(args.repo_root))
    sys.stdout.write(dumps_report(report))
    return 0 if preflight_passed(report) else 1


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="check_python_planner_deletion_preflight.py",
        description="Emit a static Python planner deletion preflight report.",
    )
    parser.add_argument("--repo-root", default=".")
    return parser.parse_args(argv)


def _remaining_python_planner_surfaces(repo_root: Path) -> list[dict[str, str]]:
    checks: tuple[tuple[str, Path, Callable[[Path], bool]], ...] = (
        ("cli_imports_emuchef_planner", CLI_PATH, _cli_imports_emuchef_planner),
        ("cli_explicit_python_backend", CLI_PATH, _cli_explicit_python_backend),
        ("cli_run_python_plan_function", CLI_PATH, _cli_run_python_plan_function),
        ("cli_run_plan_routes_to_python_plan", CLI_PATH, _cli_run_plan_routes_to_python_plan),
        (
            "readiness_gate_python_deletion_blocker",
            READINESS_GATE_PATH,
            _readiness_gate_python_deletion_blocker,
        ),
        ("test_cli_explicit_python_backend_behavior", TEST_CLI_PATH, _test_cli_explicit_python_backend_behavior),
        (
            "test_readiness_python_backend_or_deletion_assertions",
            TEST_READINESS_PATH,
            _test_readiness_python_backend_or_deletion_assertions,
        ),
        (
            "test_launcher_smoke_python_help_exposure",
            TEST_LAUNCHER_SMOKE_PATH,
            _test_launcher_smoke_python_help_exposure,
        ),
    )

    surfaces: list[dict[str, str]] = []
    for surface_id, relative_path, detector in checks:
        path = repo_root / relative_path
        if detector(path):
            surfaces.append(
                {
                    "id": surface_id,
                    "status": "present",
                    "path": str(relative_path),
                }
            )
    return surfaces


def _required_deletion_steps(remaining_surfaces: list[dict[str, str]]) -> list[str]:
    active_steps: set[str] = set()
    for surface in remaining_surfaces:
        active_steps.update(SURFACE_DELETION_STEPS.get(surface["id"], ()))
    return [step for step in DELETION_STEP_ORDER if step in active_steps]


def _cli_imports_emuchef_planner(path: Path) -> bool:
    tree = _parse_python(path)
    if tree is None:
        return False

    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == "emuchef.planner" or alias.name.startswith("emuchef.planner."):
                    return True
        elif isinstance(node, ast.ImportFrom) and node.module is not None:
            if node.module == "emuchef.planner" or node.module.startswith("emuchef.planner."):
                return True
    return False


def _cli_explicit_python_backend(path: Path) -> bool:
    tree = _parse_python(path)
    if tree is None:
        return False

    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and _is_add_argument_call(node, "--planner-backend"):
            choices = _keyword_value(node, "choices")
            if choices is not None and "python" in _literal_string_sequence(choices):
                return True
    return False


def _cli_run_python_plan_function(path: Path) -> bool:
    tree = _parse_python(path)
    if tree is None:
        return False
    return any(isinstance(node, ast.FunctionDef) and node.name == "_run_python_plan" for node in ast.walk(tree))


def _cli_run_plan_routes_to_python_plan(path: Path) -> bool:
    tree = _parse_python(path)
    if tree is None:
        return False

    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == "_run_plan":
            return _function_calls(node, "_run_python_plan")
    return False


def _readiness_gate_python_deletion_blocker(path: Path) -> bool:
    return _text_has_any_token(path, ("python_planner_deletion_not_ready",))


def _test_cli_explicit_python_backend_behavior(path: Path) -> bool:
    return _text_has_any_token(
        path,
        (
            "test_plan_explicit_python_backend",
            '"--planner-backend",\n                    "python"',
            '"--planner-backend",\n                    "python",',
        ),
    )


def _test_readiness_python_backend_or_deletion_assertions(path: Path) -> bool:
    return _text_has_any_token(
        path,
        (
            "cli_explicit_python_backend_available",
            "python_planner_deletion_not_ready",
        ),
    )


def _test_launcher_smoke_python_help_exposure(path: Path) -> bool:
    return _text_has_any_token(
        path,
        (
            "check_explicit_python_backend_available",
            "explicit_python_backend_bypass_available",
        ),
    )


def _parse_python(path: Path) -> ast.AST | None:
    text = _read_text_if_available(path)
    if not text:
        return None
    try:
        return ast.parse(text)
    except SyntaxError:
        return None


def _is_add_argument_call(node: ast.Call, argument_name: str) -> bool:
    if not isinstance(node.func, ast.Attribute) or node.func.attr != "add_argument":
        return False
    return any(isinstance(arg, ast.Constant) and arg.value == argument_name for arg in node.args)


def _keyword_value(node: ast.Call, keyword_name: str) -> ast.AST | None:
    for keyword in node.keywords:
        if keyword.arg == keyword_name:
            return keyword.value
    return None


def _literal_string_sequence(node: ast.AST) -> tuple[str, ...]:
    if not isinstance(node, (ast.Tuple, ast.List, ast.Set)):
        return ()
    values: list[str] = []
    for element in node.elts:
        if isinstance(element, ast.Constant) and isinstance(element.value, str):
            values.append(element.value)
    return tuple(values)


def _function_calls(function_node: ast.FunctionDef, function_name: str) -> bool:
    for node in ast.walk(function_node):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == function_name:
            return True
    return False


def _text_has_any_token(path: Path, tokens: tuple[str, ...]) -> bool:
    text = _read_text_if_available(path)
    return any(token in text for token in tokens)


def _read_text_if_available(path: Path) -> str:
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())

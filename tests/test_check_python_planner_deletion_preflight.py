from __future__ import annotations

import ast
import builtins
import importlib.util
import json
import re
import tempfile
import unittest
from io import StringIO
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOL_PATH = REPO_ROOT / "tools" / "check_python_planner_deletion_preflight.py"


CLI_WITH_PYTHON_SURFACES = """
from emuchef.planner import Planner, SelectRecipe

def configure(plan_parser):
    plan_parser.add_argument(
        "--planner-backend",
        choices=("python", "rust-shadow", "rust-production-equivalent"),
    )

def _run_plan(args):
    if args.planner_backend == "rust-production-equivalent":
        return _run_rust_shadow_plan(args)
    return _run_python_plan(args)

def _run_python_plan(args):
    return 0
"""

CLI_WITHOUT_PYTHON_SURFACES = """
def configure(plan_parser):
    plan_parser.add_argument(
        "--planner-backend",
        choices=("rust-shadow", "rust-production-equivalent"),
    )

def _run_plan(args):
    return _run_rust_shadow_plan(args)
"""

CLI_WITH_PYTHON_COMPATIBLE_ONLY = """
def configure(plan_parser):
    plan_parser.add_argument(
        "--planner-backend",
        choices=("rust-shadow", "python-compatible", "rust-production-equivalent"),
    )
"""


def import_preflight_module(module_name: str = "check_python_planner_deletion_preflight"):
    spec = importlib.util.spec_from_file_location(module_name, TOOL_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {TOOL_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_text(root: Path, relative_path: str, text: str = "") -> None:
    path = root / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def make_repo(
    root: Path,
    *,
    cli_text: str = CLI_WITH_PYTHON_SURFACES,
    readiness_text: str = 'REMAINING_BLOCKERS = ({"id": "python_planner_deletion_not_ready"},)',
    test_cli_text: str = 'def test_plan_explicit_python_backend_uses_python_summary_without_rust_process_or_binary(): pass',
    test_readiness_text: str = 'self.assertEqual(blockers["python_planner_deletion_not_ready"], "blocked")',
    test_launcher_text: str = "check_explicit_python_backend_available(repo_root=REPO_ROOT)",
    cutover_doc_text: str = "Explicit `--planner-backend python` remains available until Python planner deletion.",
    context_text: str = "Python planner deletion remains blocked while `--planner-backend python` remains available.",
) -> None:
    write_text(root, "src/emuchef/cli.py", cli_text)
    write_text(root, "tools/check_rust_planner_cutover_readiness.py", readiness_text)
    write_text(root, "tests/test_cli.py", test_cli_text)
    write_text(root, "tests/test_check_rust_planner_cutover_readiness.py", test_readiness_text)
    write_text(root, "tests/test_smoke_launcher_injected_planner.py", test_launcher_text)
    write_text(root, "docs/rust-planner-cutover-readiness.md", cutover_doc_text)
    write_text(root, "CONTEXT.md", context_text)


def surface_ids(report: dict) -> list[str]:
    return [surface["id"] for surface in report["remaining_python_planner_surfaces"]]


class CheckPythonPlannerDeletionPreflightTests(unittest.TestCase):
    def test_detects_exact_python_backend_choice_without_matching_python_compatible(self) -> None:
        preflight = import_preflight_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(root, cli_text=CLI_WITH_PYTHON_COMPATIBLE_ONLY)

            report = preflight.build_preflight_report(root)

        self.assertNotIn("cli_explicit_python_backend", surface_ids(report))

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(root, cli_text=CLI_WITH_PYTHON_SURFACES)

            report = preflight.build_preflight_report(root)

        self.assertIn("cli_explicit_python_backend", surface_ids(report))

    def test_detects_python_planner_runtime_cli_surfaces(self) -> None:
        preflight = import_preflight_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(root)

            report = preflight.build_preflight_report(root)

        ids = surface_ids(report)
        self.assertIn("cli_imports_emuchef_planner", ids)
        self.assertIn("cli_run_python_plan_function", ids)
        self.assertIn("cli_run_plan_routes_to_python_plan", ids)

    def test_emits_deterministic_json_and_reports_blocked_with_deletion_steps(self) -> None:
        preflight = import_preflight_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(root)

            first_report = preflight.build_preflight_report(root)
            second_report = preflight.build_preflight_report(root)
            first_json = preflight.dumps_report(first_report)
            second_json = preflight.dumps_report(second_report)

        self.assertEqual(first_report["kind"], "python_planner_deletion_preflight")
        self.assertEqual(first_report["schema_version"], 1)
        self.assertEqual(first_report["status"], "blocked")
        self.assertFalse(preflight.preflight_passed(first_report))
        self.assertEqual(first_json, second_json)
        self.assertEqual(json.loads(first_json), first_report)
        self.assertIn("remove explicit Python planner backend from CLI", first_report["required_deletion_steps"])
        self.assertIn("update tests that assert Python fallback availability", first_report["required_deletion_steps"])

    def test_reports_ready_when_no_surfaces_remain(self) -> None:
        preflight = import_preflight_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(
                root,
                cli_text=CLI_WITHOUT_PYTHON_SURFACES,
                readiness_text="REMAINING_BLOCKERS = ()",
                test_cli_text="def test_rust_route(): pass",
                test_readiness_text="self.assertEqual(blockers, {})",
                test_launcher_text="def test_launcher_path(): pass",
                cutover_doc_text="Rust planner deletion cleanup is complete.",
                context_text="Rust planning is the only runtime planner implementation.",
            )

            report = preflight.build_preflight_report(root)

        self.assertEqual(report["status"], "ready")
        self.assertTrue(preflight.preflight_passed(report))
        self.assertEqual(report["remaining_python_planner_surfaces"], [])
        self.assertEqual(report["required_deletion_steps"], [])

    def test_main_returns_nonzero_for_blocked_and_zero_for_ready(self) -> None:
        preflight = import_preflight_module()
        with tempfile.TemporaryDirectory() as blocked_dir, tempfile.TemporaryDirectory() as ready_dir:
            blocked_root = Path(blocked_dir)
            ready_root = Path(ready_dir)
            make_repo(blocked_root)
            make_repo(
                ready_root,
                cli_text=CLI_WITHOUT_PYTHON_SURFACES,
                readiness_text="REMAINING_BLOCKERS = ()",
                test_cli_text="def test_rust_route(): pass",
                test_readiness_text="self.assertEqual(blockers, {})",
                test_launcher_text="def test_launcher_path(): pass",
                cutover_doc_text="Rust planner deletion cleanup is complete.",
                context_text="Rust planning is the only runtime planner implementation.",
            )
            blocked_stdout = StringIO()
            ready_stdout = StringIO()

            with patch("sys.stdout", blocked_stdout):
                blocked_code = preflight.main(["--repo-root", str(blocked_root)])
            with patch("sys.stdout", ready_stdout):
                ready_code = preflight.main(["--repo-root", str(ready_root)])

        self.assertEqual(blocked_code, 1)
        self.assertEqual(json.loads(blocked_stdout.getvalue())["status"], "blocked")
        self.assertEqual(ready_code, 0)
        self.assertEqual(json.loads(ready_stdout.getvalue())["status"], "ready")

    def test_does_not_import_emuchef_modules(self) -> None:
        original_import = builtins.__import__

        def guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
            if name == "emuchef" or name.startswith("emuchef."):
                raise AssertionError(f"preflight must not import {name}")
            return original_import(name, globals, locals, fromlist, level)

        with patch("builtins.__import__", side_effect=guarded_import):
            import_preflight_module("check_python_planner_deletion_preflight_guarded")

        source = TOOL_PATH.read_text(encoding="utf-8")
        self.assertIsNone(re.search(r"^\s*import\s+emuchef\b", source, flags=re.MULTILINE))
        self.assertIsNone(re.search(r"^\s*from\s+emuchef\b", source, flags=re.MULTILINE))

    def test_does_not_import_or_call_subprocess_apis(self) -> None:
        source = TOOL_PATH.read_text(encoding="utf-8")
        tree = ast.parse(source)

        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    self.assertNotEqual(alias.name.split(".")[0], "subprocess")
            elif isinstance(node, ast.ImportFrom) and node.module is not None:
                self.assertNotEqual(node.module.split(".")[0], "subprocess")
            elif isinstance(node, ast.Attribute):
                self.assertNotEqual(getattr(node.value, "id", None), "subprocess")

    def test_does_not_read_local_paths(self) -> None:
        preflight = import_preflight_module()
        read_paths: list[Path] = []
        original_read_text = Path.read_text

        def tracked_read_text(path: Path, *args, **kwargs):
            read_paths.append(path)
            return original_read_text(path, *args, **kwargs)

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_repo(root)
            write_text(root, ".local/poison.txt", "must not be read")

            with patch.object(Path, "read_text", tracked_read_text):
                report = preflight.build_preflight_report(root)

        self.assertEqual(report["status"], "blocked")
        self.assertTrue(read_paths)
        self.assertTrue(all(".local" not in path.parts for path in read_paths), read_paths)
        self.assertNotIn(".local", TOOL_PATH.read_text(encoding="utf-8"))

    def test_real_repo_integration_reports_current_surfaces(self) -> None:
        preflight = import_preflight_module()

        report = preflight.build_preflight_report(REPO_ROOT)

        self.assertEqual(report["status"], "blocked")
        self.assertEqual(
            surface_ids(report),
            [
                "docs_cutover_python_fallback_or_deletion_readiness",
                "context_python_fallback_or_deletion_readiness",
            ],
        )


if __name__ == "__main__":
    unittest.main()

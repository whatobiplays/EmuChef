from __future__ import annotations

import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOL_PATH = REPO_ROOT / "tools" / "check_rust_planner_cutover_readiness.py"


READINESS_DOC_TEXT = """
tools/plan_parity_scenarios.json
tools/compare_rust_python_plan.py
tools/smoke_rust_shadow_cli_matrix.py
docs/adr/0002-rust-planner-cli-output-compatibility.md
rust-shadow
rust-experimental
Python planner
default
executor/apply
ADB
Tauri
Python planner deletion
"""

CLI_TEXT = """
plan_parser.add_argument("--planner-backend", choices=("python", "rust-shadow", "rust-experimental"), default="python")
plan_parser.add_argument("--rust-planner-bin")
plan_parser.add_argument("--rust-shadow-output")
"""


def import_readiness_module():
    spec = importlib.util.spec_from_file_location("check_rust_planner_cutover_readiness", TOOL_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {TOOL_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_text(root: Path, relative_path: str, text: str = "") -> None:
    path = root / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def scenario_payload(*device_plan_ids: str, classification: str = "match") -> dict:
    return {
        "schema_version": 1,
        "scenarios": [
            {
                "id": device_plan_id.replace(".", "_"),
                "device_plan": device_plan_id,
                "expected_classification": classification,
                "bindings": [],
                "known_gap_ids": [],
            }
            for device_plan_id in device_plan_ids
        ],
    }


def make_synthetic_repo(
    root: Path,
    *,
    device_plan_ids: tuple[str, ...] = ("example.one", "example.two"),
    matrix_payload: dict | None = None,
    matrix_text: str | None = None,
    readiness_doc_text: str = READINESS_DOC_TEXT,
    cli_text: str = CLI_TEXT,
    include_matrix: bool = True,
) -> None:
    for device_plan_id in device_plan_ids:
        write_text(root, f"authored/device_plans/{device_plan_id}.yaml", "not parsed by readiness gate\n")
    write_text(root, "authored/device_plans/.gitkeep", "")

    required_files = [
        "tools/compare_rust_python_plan.py",
        "tools/smoke_rust_shadow_cli_matrix.py",
        "docs/rust-planner-parity-boundary.md",
        "docs/rust-cli-executor-parity.md",
        "docs/adr/0002-rust-planner-cli-output-compatibility.md",
        "tests/test_cli.py",
        "tests/test_compare_rust_python_plan.py",
        "tests/test_smoke_rust_shadow_cli_matrix.py",
    ]
    for relative_path in required_files:
        write_text(root, relative_path, "required artifact\n")

    write_text(root, "docs/rust-planner-cutover-readiness.md", readiness_doc_text)
    write_text(root, "src/emuchef/cli.py", cli_text)

    if include_matrix:
        if matrix_text is None:
            payload = matrix_payload if matrix_payload is not None else scenario_payload(*device_plan_ids)
            matrix_text = json.dumps(payload, indent=2)
        write_text(root, "tools/plan_parity_scenarios.json", matrix_text)


def check_by_id(report: dict, check_id: str) -> dict:
    matches = [check for check in report["static_checks"] if check["id"] == check_id]
    if len(matches) != 1:
        raise AssertionError(f"Expected one check with id {check_id!r}, found {len(matches)}")
    return matches[0]


class CheckRustPlannerCutoverReadinessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.readiness = import_readiness_module()

    def build_report(self, root: Path) -> dict:
        return self.readiness.build_readiness_report(
            repo_root=root,
            authored_root=Path("authored"),
            scenario_matrix=Path("tools/plan_parity_scenarios.json"),
        )

    def test_happy_path_static_report_with_required_files_and_docs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        self.assertEqual(report["kind"], "rust_planner_cutover_readiness_check")
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(
            report["inputs"],
            {
                "authored_root": "authored",
                "scenario_matrix": "tools/plan_parity_scenarios.json",
            },
        )
        self.assertTrue(all(check["status"] == "pass" for check in report["static_checks"]))
        self.assertEqual(check_by_id(report, "scenario_matrix_covers_checked_in_device_plans")["status"], "pass")

    def test_missing_scenario_matrix_reports_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root, include_matrix=False)

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "scenario_matrix_exists")["status"], "fail")

    def test_malformed_scenario_matrix_reports_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root, matrix_text="{not json")

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "scenario_matrix_json_valid")["status"], "fail")

    def test_scenario_matrix_missing_checked_in_device_plan_reports_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root, matrix_payload=scenario_payload("example.one"))

            report = self.build_report(root)

        check = check_by_id(report, "scenario_matrix_covers_checked_in_device_plans")
        self.assertEqual(check["status"], "fail")
        self.assertIn("example.two", check["details"]["missing_device_plans"])

    def test_scenario_matrix_non_match_expected_classification_reports_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(
                root,
                device_plan_ids=("example.one",),
                matrix_payload=scenario_payload("example.one", classification="known_gap"),
            )

            report = self.build_report(root)

        check = check_by_id(report, "scenario_matrix_scenario_fields")
        self.assertEqual(check["status"], "fail")
        self.assertIn("expected_classification", check["details"]["errors"][0])

    def test_readiness_doc_missing_required_references_reports_failed_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root, readiness_doc_text="Python planner\nADB\n")

            report = self.build_report(root)

        self.assertEqual(
            check_by_id(report, "readiness_doc_reference_rust_experimental")["status"],
            "fail",
        )
        self.assertEqual(
            check_by_id(report, "readiness_doc_reference_python_planner_deletion")["status"],
            "fail",
        )

    def test_backend_choices_missing_rust_experimental_reports_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root, cli_text=CLI_TEXT.replace('"rust-experimental"', '"rust-missing"'))

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "cli_backend_token_rust_experimental")["status"], "fail")

    def test_report_is_deterministic_for_identical_synthetic_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            first = self.readiness.dumps_report(self.build_report(root))
            second = self.readiness.dumps_report(self.build_report(root))

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first), json.loads(second))

    def test_report_includes_required_manual_evidence_commands_without_executing_them(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        commands = {item["id"]: item["command"] for item in report["required_manual_evidence"]}
        self.assertIn("p7p_python_rust_comparison_matrix", commands)
        self.assertIn("tools/compare_rust_python_plan.py", commands["p7p_python_rust_comparison_matrix"])
        self.assertIn("p8h_rust_experimental_matrix_smoke", commands)
        self.assertIn("--planner-backend rust-experimental", commands["p8h_rust_experimental_matrix_smoke"])
        self.assertIn("focused_python_tests", commands)
        self.assertIn("rust_tauri_checks", commands)

    def test_report_status_remains_blocked_when_static_checks_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        self.assertTrue(all(check["status"] == "pass" for check in report["static_checks"]))
        self.assertEqual(report["status"], "blocked")
        self.assertTrue(all(blocker["status"] == "blocked" for blocker in report["remaining_blockers"]))

    def test_source_has_no_forbidden_imports(self) -> None:
        source = TOOL_PATH.read_text(encoding="utf-8")

        forbidden_import_patterns = [
            r"^\s*import\s+emuchef\b",
            r"^\s*from\s+emuchef\b",
            r"^\s*import\s+yaml\b",
            r"^\s*from\s+yaml\b",
            r"^\s*import\s+tools\.compare_rust_python_plan\b",
            r"^\s*from\s+tools\.compare_rust_python_plan\b",
            r"^\s*import\s+tools\.smoke_rust_shadow_cli_matrix\b",
            r"^\s*from\s+tools\.smoke_rust_shadow_cli_matrix\b",
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

    def test_source_has_no_subprocess_calls(self) -> None:
        source = TOOL_PATH.read_text(encoding="utf-8")

        self.assertNotIn("import subprocess", source)
        self.assertNotIn("subprocess.", source)

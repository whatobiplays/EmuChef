from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOL_PATH = REPO_ROOT / "tools" / "check_rust_planner_cutover_readiness.py"
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_launcher_injected_planner.py"
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
_DEFAULT_RUST_PLANNER_BACKEND = "rust-production-equivalent"
plan_parser.add_argument(
    "--planner-backend",
    choices=("rust-shadow", "rust-experimental", "rust-production-equivalent"),
    help=(
        "Omit this option to use default Rust-owned planning through "
        "rust-production-equivalent; python-compatible output is the route formatter."
    ),
)
plan_parser.add_argument("--rust-planner-bin")
plan_parser.add_argument("--rust-shadow-output")

def _effective_plan_backend(args):
    return args.planner_backend or _DEFAULT_RUST_PLANNER_BACKEND

def _validate_rust_shadow_plan_args(args):

def _resolve_rust_planner_bin(args):
    _validate_rust_shadow_plan_args(args)
    if args.rust_planner_bin:
        rust_planner_bin = Path(args.rust_planner_bin).expanduser()
        if not rust_planner_bin.exists():
            raise ValueError(f"Rust shadow planner binary does not exist: {args.rust_planner_bin}")
        if not rust_planner_bin.is_file() or not os.access(rust_planner_bin, os.X_OK):
            raise ValueError(f"Rust shadow planner binary is not executable: {args.rust_planner_bin}")
        return rust_planner_bin

    packaged_candidate = _packaged_rust_planner_bin_candidate(args)
    if packaged_candidate is not None:
        return packaged_candidate

    if args.planner_backend is None:
        raise ValueError("--rust-planner-bin is required when default Rust planner routing is active.")
    raise ValueError(f"--rust-planner-bin is required when --planner-backend {_effective_plan_backend(args)} is selected.")


def _packaged_rust_planner_bin_candidate(args):
    return None
"""

EXPLICIT_DEVICE_CONTEXT = {
    "manufacturer": "Example",
    "model": "Example Device",
    "android_version": 13,
    "device_tags": ["handheld", "landscape"],
}


def import_readiness_module():
    spec = importlib.util.spec_from_file_location("check_rust_planner_cutover_readiness", TOOL_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {TOOL_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def import_smoke_module():
    module_name = "smoke_launcher_injected_planner_for_readiness_tests"
    spec = importlib.util.spec_from_file_location(module_name, SMOKE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {SMOKE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def write_text(root: Path, relative_path: str, text: str = "") -> None:
    path = root / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_json(root: Path, relative_path: str, payload: dict) -> Path:
    path = root / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return Path(relative_path)


def accepted_p8aj_report() -> dict:
    return {
        "kind": "rust_production_equivalent_live_adb_probe_smoke",
        "schema_version": 1,
        "inputs": {
            "live_probe_requested": True,
            "serial_supplied": True,
        },
        "cases": [
            {
                "id": "rust_production_equivalent_live_adb_probe_forwarding",
                "status": "passed",
                "stdout_class": "python_compatible",
                "stderr_class": "empty",
            }
        ],
        "summary": {
            "passed": 1,
            "failed": 0,
            "skipped": 0,
        },
    }


def accepted_p8ak_report() -> dict:
    return {
        "kind": "rust_production_equivalent_mismatch_warning_smoke",
        "schema_version": 1,
        "inputs": {
            "route_backend": "rust-production-equivalent",
            "route_output_mode": "python-compatible",
        },
        "cases": [
            {"id": "matched_profile", "status": "passed", "stdout_class": "python_compatible"},
            {"id": "manufacturer_mismatch", "status": "passed", "stdout_class": "python_compatible"},
            {"id": "model_mismatch", "status": "passed", "stdout_class": "python_compatible"},
            {"id": "android_minimum_mismatch", "status": "passed", "stdout_class": "python_compatible"},
            {"id": "android_minimum_match", "status": "passed", "stdout_class": "python_compatible"},
        ],
        "summary": {
            "passed": 5,
            "failed": 0,
            "skipped": 0,
        },
    }


def accepted_p8bc_report() -> dict:
    return {
        "kind": "rust_launcher_injected_planner_smoke",
        "schema_version": 1,
        "generated_at": "2026-06-21T00:00:00Z",
        "summary": {
            "passed": 7,
            "failed": 0,
        },
        "inputs": {
            "planner_backend": "rust-production-equivalent",
            "launcher_supplied_planner_path": True,
            "path_was_absolute": True,
            "path_exists": True,
            "path_is_file": True,
            "path_executable": True,
            "argv0_corresponds_to_launcher_path": True,
            "detected_facts_source": "temporary_fixture_json",
            "launcher_entrypoint_observation": "external_wrapper",
        },
        "checks": [
            {"name": "launcher_supplied_path_absolute", "passed": True},
            {"name": "launcher_supplied_path_exists", "passed": True},
            {"name": "launcher_supplied_path_file", "passed": True},
            {"name": "launcher_supplied_path_executable", "passed": True},
            {"name": "argv0_corresponds_to_launcher_path", "passed": True},
            {"name": "known_fixture_plan_succeeded", "passed": True},
            {"name": "no_implicit_fallback_sources_used", "passed": True},
        ],
        "redaction": {
            "full_paths_omitted": True,
            "process_invocation_omitted": True,
            "process_output_omitted": True,
            "runtime_context_omitted": True,
            "device_identifiers_omitted": True,
            "sensitive_values_omitted": True,
        },
        "artifacts": {
            "argv0_basename": "emuchef-plan-shadow",
        },
    }


def scenario_payload(
    *device_plan_ids: str,
    classification: str = "match",
    include_explicit_context: bool = True,
) -> dict:
    scenarios = [
        {
            "id": device_plan_id.replace(".", "_"),
            "device_plan": device_plan_id,
            "expected_classification": classification,
            "bindings": [],
            "known_gap_ids": [],
        }
        for device_plan_id in device_plan_ids
    ]
    if include_explicit_context and scenarios:
        scenarios[-1]["device_context"] = {
            **EXPLICIT_DEVICE_CONTEXT,
            "device_tags": list(EXPLICIT_DEVICE_CONTEXT["device_tags"]),
        }
    return {
        "schema_version": 1,
        "scenarios": scenarios,
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


def blocker_status(report: dict, blocker_id: str) -> str:
    matches = [blocker for blocker in report["remaining_blockers"] if blocker["id"] == blocker_id]
    if len(matches) != 1:
        raise AssertionError(f"Expected one blocker with id {blocker_id!r}, found {len(matches)}")
    return matches[0]["status"]


def retired_python_deletion_blocker_id() -> str:
    return "_".join(("python", "planner", "deletion", "not", "ready"))


def expected_status_explanation() -> dict:
    return {
        "top_level_status": "blocked",
        "evidence_accepted_is_not_release_ready": True,
        "evidence_accepted_meaning": (
            "Accepted evidence can satisfy scoped evidence blockers; it does not imply top-level readiness."
        ),
        "top_level_blocked_reason": (
            "Top-level readiness remains blocked while executor/apply, packaged release, "
            "and unsatisfied evidence-dependent blockers remain blocked."
        ),
        "blocking_categories": [
            "executor_apply",
            "evidence_dependent_cutover",
            "packaged_release",
        ],
    }


def executable_file(path: Path) -> Path:
    path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    path.chmod(0o700)
    return path


class CheckRustPlannerCutoverReadinessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.readiness = import_readiness_module()

    def build_report(
        self,
        root: Path,
        *,
        p8aj_live_probe_report: Path | None = None,
        p8ak_mismatch_warning_report: Path | None = None,
        p8bc_launcher_injected_planner_report: Path | None = None,
    ) -> dict:
        return self.readiness.build_readiness_report(
            repo_root=root,
            authored_root=Path("authored"),
            scenario_matrix=Path("tools/plan_parity_scenarios.json"),
            p8aj_live_probe_report=p8aj_live_probe_report,
            p8ak_mismatch_warning_report=p8ak_mismatch_warning_report,
            p8bc_launcher_injected_planner_report=p8bc_launcher_injected_planner_report,
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
        self.assertEqual(check_by_id(report, "explicit_context_supported_by_matrix_schema")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_scenario_present")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_scenario_valid")["status"], "pass")
        self.assertEqual(check_by_id(report, "cli_default_backend_is_omitted")["status"], "pass")
        self.assertEqual(
            check_by_id(report, "cli_default_backend_resolves_to_rust_production_equivalent")["status"],
            "pass",
        )
        self.assertEqual(check_by_id(report, "cli_explicit_python_backend_not_exposed")["status"], "pass")
        self.assertEqual(check_by_id(report, "cli_default_rust_requires_planner_bin")["status"], "pass")
        blockers = {blocker["id"]: blocker["status"] for blocker in report["remaining_blockers"]}
        self.assertNotIn(retired_python_deletion_blocker_id(), blockers)

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

    def test_scenario_matrix_allows_additional_context_scenario_for_same_device_plan(self) -> None:
        payload = {
            "schema_version": 1,
            "scenarios": [
                {
                    "id": "example_one_base",
                    "device_plan": "example.one",
                    "expected_classification": "match",
                    "bindings": [],
                    "known_gap_ids": [],
                },
                {
                    "id": "example_one_explicit_context",
                    "device_plan": "example.one",
                    "expected_classification": "match",
                    "bindings": [],
                    "known_gap_ids": [],
                    "device_context": {
                        "manufacturer": "Example",
                        "model": "Example Device",
                        "android_version": 13,
                        "device_tags": ["handheld", "landscape"],
                    },
                },
            ],
        }
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(
                root,
                device_plan_ids=("example.one",),
                matrix_payload=payload,
            )

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "scenario_matrix_scenario_fields")["status"], "pass")
        self.assertEqual(check_by_id(report, "scenario_matrix_covers_checked_in_device_plans")["status"], "pass")
        self.assertTrue(all(check["id"] != "scenario_matrix_unique_device_plans" for check in report["static_checks"]))

    def test_scenario_matrix_invalid_device_context_reports_failed_check(self) -> None:
        invalid_contexts = [
            (None, "device_context must be an object"),
            ({"manufacturer": ""}, "device_context.manufacturer must be a non-empty string"),
            ({"model": ""}, "device_context.model must be a non-empty string"),
            ({"android_version": -1}, "device_context.android_version must be a non-negative integer"),
            ({"android_version": True}, "device_context.android_version must be a non-negative integer"),
            ({"device_tags": []}, "device_context.device_tags must be a non-empty list"),
            ({"device_tags": ["handheld", ""]}, "device_context.device_tags[1] must be a non-empty string"),
            ({"serial": "SERIAL"}, "device_context contains unsupported field: serial"),
            ({"adb": True}, "device_context contains unsupported field: adb"),
        ]

        for device_context, message in invalid_contexts:
            with self.subTest(device_context=device_context):
                payload = scenario_payload("example.one")
                payload["scenarios"][0]["device_context"] = device_context
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(
                        root,
                        device_plan_ids=("example.one",),
                        matrix_payload=payload,
                    )

                    report = self.build_report(root)

                check = check_by_id(report, "scenario_matrix_scenario_fields")
                self.assertEqual(check["status"], "fail")
                self.assertTrue(
                    any(message in error for error in check["details"]["errors"]),
                    check["details"]["errors"],
                )
                self.assertEqual(check_by_id(report, "explicit_context_scenario_valid")["status"], "fail")
                self.assertEqual(check_by_id(report, "explicit_context_scenario_present")["status"], "fail")

    def test_explicit_context_static_check_fails_when_no_scenario_has_device_context(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(
                root,
                matrix_payload=scenario_payload(
                    "example.one",
                    "example.two",
                    include_explicit_context=False,
                ),
            )

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "scenario_matrix_scenario_fields")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_supported_by_matrix_schema")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_scenario_present")["status"], "fail")
        self.assertEqual(check_by_id(report, "explicit_context_scenario_valid")["status"], "fail")

    def test_explicit_context_static_check_fails_when_context_has_no_explicit_fields(self) -> None:
        payload = scenario_payload("example.one")
        payload["scenarios"][0]["device_context"] = {}
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(
                root,
                device_plan_ids=("example.one",),
                matrix_payload=payload,
            )

            report = self.build_report(root)

        self.assertEqual(check_by_id(report, "scenario_matrix_scenario_fields")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_supported_by_matrix_schema")["status"], "pass")
        self.assertEqual(check_by_id(report, "explicit_context_scenario_present")["status"], "fail")
        explicit_check = check_by_id(report, "explicit_context_scenario_valid")
        self.assertEqual(explicit_check["status"], "fail")
        self.assertIn("at least one explicit context field", explicit_check["details"]["errors"][0])

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

    def test_default_route_static_checks_fail_when_route_markers_are_missing(self) -> None:
        exposed_python_choice = CLI_TEXT.replace(
            'choices=("rust-shadow", "rust-experimental", "rust-production-equivalent")',
            'choices=("python", "rust-shadow", "rust-experimental", "rust-production-equivalent")',
        )
        self.assertIn("python-compatible", CLI_TEXT)
        self.assertNotIn('"python"', CLI_TEXT)
        self.assertIn('"python"', exposed_python_choice)
        cases = [
            (
                "cli_default_backend_is_omitted",
                CLI_TEXT.replace('    help=(', '    default="python",\n    help=(', 1),
            ),
            (
                "cli_default_backend_resolves_to_rust_production_equivalent",
                CLI_TEXT.replace(
                    "return args.planner_backend or _DEFAULT_RUST_PLANNER_BACKEND",
                    'return args.planner_backend or "python"',
                ),
            ),
            (
                "cli_explicit_python_backend_not_exposed",
                exposed_python_choice,
            ),
            (
                "cli_default_rust_requires_planner_bin",
                CLI_TEXT.replace(
                    "--rust-planner-bin is required when default Rust planner routing is active.",
                    "--rust-planner-bin is required for explicit Rust routes.",
                ),
            ),
        ]
        for check_id, cli_text in cases:
            with self.subTest(check_id=check_id):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root, cli_text=cli_text)

                    report = self.build_report(root)

                self.assertEqual(check_by_id(report, check_id)["status"], "fail")

    def test_report_is_deterministic_for_identical_synthetic_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            first = self.readiness.dumps_report(self.build_report(root))
            second = self.readiness.dumps_report(self.build_report(root))

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first), json.loads(second))

    def test_current_static_readiness_report_is_deterministic_without_fixture_update(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)
            serialized = self.readiness.dumps_report(report)
            second_serialized = self.readiness.dumps_report(self.build_report(root))

        self.assertEqual(serialized, second_serialized)
        self.assertEqual(json.loads(serialized), report)
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_release_not_ready"), "blocked")
        self.assertNotIn(
            retired_python_deletion_blocker_id(),
            {blocker["id"] for blocker in report["remaining_blockers"]},
        )
        for leaked_token in (".local", "stdout", "stderr", "environment", "/tmp/", "/private/", "/Users/", "C:\\"):
            with self.subTest(leaked_token=leaked_token):
                self.assertNotIn(leaked_token, serialized)

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
        self.assertIn("p8aj_rust_production_equivalent_live_probe_smoke", commands)
        self.assertIn(
            "tools/smoke_rust_production_equivalent_live_adb_probe.py",
            commands["p8aj_rust_production_equivalent_live_probe_smoke"],
        )
        self.assertIn("p8ak_rust_production_equivalent_mismatch_warning_smoke", commands)
        self.assertIn(
            "tools/smoke_rust_production_equivalent_mismatch_warning.py",
            commands["p8ak_rust_production_equivalent_mismatch_warning_smoke"],
        )
        self.assertIn("p8bc_launcher_injected_planner_smoke", commands)
        p8bc_command = commands["p8bc_launcher_injected_planner_smoke"]
        self.assertIn("tools/smoke_launcher_injected_planner.py", p8bc_command)
        self.assertIn("--rust-planner-bin <absolute-path-to-launcher-supplied-planner>", p8bc_command)
        self.assertIn("--output-report <path-to-output-report>", p8bc_command)
        self.assertNotIn(".local", p8bc_command)
        self.assertNotIn("/Users/", p8bc_command)
        self.assertNotIn("/tmp/", p8bc_command)
        self.assertNotIn("stdout", p8bc_command.lower())
        self.assertNotIn("stderr", p8bc_command.lower())
        self.assertNotIn("environment", p8bc_command.lower())

    def test_status_explanation_is_present_without_p8bc_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        self.assertEqual(list(report).index("status_explanation"), list(report).index("status") + 1)
        self.assertEqual(report["status_explanation"], expected_status_explanation())

    def test_valid_p8aj_p8ak_and_p8bc_reports_under_local_evidence_paths_accept_scoped_blockers_only(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            live_report = write_json(root, ".local/evidence/p8aj-live-probe.json", accepted_p8aj_report())
            mismatch_report = write_json(root, ".local/evidence/p8ak-mismatch-warning.json", accepted_p8ak_report())
            launcher_report = write_json(
                root,
                ".local/evidence/p8bc-launcher-injected-planner.json",
                accepted_p8bc_report(),
            )

            report = self.build_report(
                root,
                p8aj_live_probe_report=live_report,
                p8ak_mismatch_warning_report=mismatch_report,
                p8bc_launcher_injected_planner_report=launcher_report,
            )

        evidence = report["production_equivalent_evidence"]
        self.assertEqual(evidence["p8aj_live_probe"]["status"], "accepted")
        self.assertEqual(evidence["p8ak_mismatch_warning"]["status"], "accepted")
        self.assertEqual(evidence["p8bc_launcher_injected_planner"]["status"], "accepted")
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "evidence_accepted")
        self.assertEqual(
            blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"),
            "evidence_accepted",
        )
        self.assertEqual(
            blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"),
            "evidence_accepted",
        )
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_release_not_ready"), "blocked")
        self.assertEqual(report["status"], "blocked")

    def test_missing_p8aj_and_p8ak_evidence_keeps_production_equivalent_blockers_blocked(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        evidence = report["production_equivalent_evidence"]
        self.assertEqual(evidence["p8aj_live_probe"]["status"], "missing")
        self.assertEqual(evidence["p8aj_live_probe"]["evidence_id"], "p8aj_live_probe")
        self.assertEqual(evidence["p8aj_live_probe"]["blocker_id"], "real_device_probing_not_cut_over")
        self.assertTrue(evidence["p8aj_live_probe"]["reasons"])
        self.assertEqual(evidence["p8ak_mismatch_warning"]["status"], "missing")
        self.assertEqual(
            evidence["p8ak_mismatch_warning"]["blocker_id"],
            "detected_device_profile_mismatch_warning_not_cut_over",
        )
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"), "blocked")
        self.assertEqual(report["status"], "blocked")

    def test_supplied_missing_evidence_paths_are_rejected_without_accepting_blockers(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(
                root,
                p8aj_live_probe_report=Path(".local/evidence/p8aj-live-probe.json"),
                p8ak_mismatch_warning_report=Path(".local/evidence/p8ak-mismatch-warning.json"),
                p8bc_launcher_injected_planner_report=Path(".local/evidence/p8bc-launcher-injected-planner.json"),
            )

        evidence = report["production_equivalent_evidence"]
        self.assertEqual(evidence["p8aj_live_probe"]["status"], "rejected")
        self.assertEqual(evidence["p8ak_mismatch_warning"]["status"], "rejected")
        self.assertEqual(evidence["p8bc_launcher_injected_planner"]["status"], "rejected")
        self.assertTrue(
            all(
                item["reasons"] == ["evidence report file is missing"]
                for item in evidence.values()
            )
        )
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"), "blocked")

    def test_p8aj_accepted_and_p8ak_missing_updates_only_live_probe_blocker(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            live_report = write_json(root, "reports/p8aj.json", accepted_p8aj_report())

            report = self.build_report(root, p8aj_live_probe_report=live_report)

        evidence = report["production_equivalent_evidence"]
        self.assertEqual(evidence["p8aj_live_probe"]["status"], "accepted")
        self.assertEqual(evidence["p8aj_live_probe"]["reasons"], [])
        self.assertEqual(evidence["p8ak_mismatch_warning"]["status"], "missing")
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "evidence_accepted")
        self.assertEqual(blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"), "blocked")
        self.assertEqual(report["status"], "blocked")

    def test_p8aj_missing_and_p8ak_accepted_updates_only_mismatch_warning_blocker(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            mismatch_report = write_json(root, "reports/p8ak.json", accepted_p8ak_report())

            report = self.build_report(root, p8ak_mismatch_warning_report=mismatch_report)

        evidence = report["production_equivalent_evidence"]
        self.assertEqual(evidence["p8aj_live_probe"]["status"], "missing")
        self.assertEqual(evidence["p8ak_mismatch_warning"]["status"], "accepted")
        self.assertEqual(evidence["p8ak_mismatch_warning"]["reasons"], [])
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "blocked")
        self.assertEqual(
            blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"),
            "evidence_accepted",
        )
        self.assertEqual(report["status"], "blocked")

    def test_accepted_p8aj_and_p8ak_reports_mark_both_evidence_blockers_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            live_report = write_json(root, "reports/p8aj.json", accepted_p8aj_report())
            mismatch_report = write_json(root, "reports/p8ak.json", accepted_p8ak_report())

            report = self.build_report(
                root,
                p8aj_live_probe_report=live_report,
                p8ak_mismatch_warning_report=mismatch_report,
            )

        self.assertEqual(report["production_equivalent_evidence"]["p8aj_live_probe"]["status"], "accepted")
        self.assertEqual(report["production_equivalent_evidence"]["p8ak_mismatch_warning"]["status"], "accepted")
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "evidence_accepted")
        self.assertEqual(
            blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"),
            "evidence_accepted",
        )
        self.assertEqual(blocker_status(report, "default_cli_backend_still_python"), "resolved")
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertNotIn(
            retired_python_deletion_blocker_id(),
            {blocker["id"] for blocker in report["remaining_blockers"]},
        )
        self.assertEqual(report["status"], "blocked")

    def test_failed_or_incomplete_evidence_reports_do_not_mark_blockers_accepted(self) -> None:
        p8aj_failed = accepted_p8aj_report()
        p8aj_failed["summary"]["failed"] = 1
        p8aj_failed_status = accepted_p8aj_report()
        p8aj_failed_status["status"] = "failed"
        p8aj_not_live = accepted_p8aj_report()
        p8aj_not_live["inputs"]["live_probe_requested"] = False
        p8ak_failed = accepted_p8ak_report()
        p8ak_failed["summary"]["failed"] = 1
        p8ak_failed_status = accepted_p8ak_report()
        p8ak_failed_status["status"] = "failed"
        p8ak_missing_case = accepted_p8ak_report()
        p8ak_missing_case["cases"] = [
            case for case in p8ak_missing_case["cases"] if case["id"] != "android_minimum_match"
        ]

        cases = [
            ("p8aj_failed", "reports/p8aj.json", p8aj_failed, "real_device_probing_not_cut_over"),
            ("p8aj_failed_status", "reports/p8aj.json", p8aj_failed_status, "real_device_probing_not_cut_over"),
            ("p8aj_not_live", "reports/p8aj.json", p8aj_not_live, "real_device_probing_not_cut_over"),
            (
                "p8ak_failed",
                "reports/p8ak.json",
                p8ak_failed,
                "detected_device_profile_mismatch_warning_not_cut_over",
            ),
            (
                "p8ak_failed_status",
                "reports/p8ak.json",
                p8ak_failed_status,
                "detected_device_profile_mismatch_warning_not_cut_over",
            ),
            (
                "p8ak_missing_case",
                "reports/p8ak.json",
                p8ak_missing_case,
                "detected_device_profile_mismatch_warning_not_cut_over",
            ),
        ]
        for name, relative_path, payload, blocker_id in cases:
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, relative_path, payload)
                    kwargs = (
                        {"p8aj_live_probe_report": report_path}
                        if "p8aj" in name
                        else {"p8ak_mismatch_warning_report": report_path}
                    )

                    report = self.build_report(root, **kwargs)

                evidence_key = "p8aj_live_probe" if "p8aj" in name else "p8ak_mismatch_warning"
                self.assertEqual(report["production_equivalent_evidence"][evidence_key]["status"], "rejected")
                self.assertTrue(report["production_equivalent_evidence"][evidence_key]["reasons"])
                self.assertEqual(blocker_status(report, blocker_id), "blocked")
                self.assertEqual(report["status"], "blocked")

    def test_valid_p8bc_report_is_accepted_for_launcher_injection_evidence_only(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            report_path = write_json(root, "reports/p8bc.json", accepted_p8bc_report())

            report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

        evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(evidence["status"], "accepted")
        self.assertEqual(evidence["evidence_id"], "p8bc_launcher_injected_planner")
        self.assertEqual(evidence["blocker_id"], "packaged_launcher_injection_evidence_not_accepted")
        self.assertEqual(evidence["reasons"], [])
        self.assertEqual(
            blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"),
            "evidence_accepted",
        )
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_release_not_ready"), "blocked")
        self.assertNotIn(
            retired_python_deletion_blocker_id(),
            {blocker["id"] for blocker in report["remaining_blockers"]},
        )
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(report["status_explanation"], expected_status_explanation())

    def test_invalid_p8bc_identity_summary_and_shape_are_rejected(self) -> None:
        cases = [
            ("wrong_kind", {"kind": "rust_production_equivalent_mismatch_warning_smoke"}),
            ("wrong_schema_version", {"schema_version": 2}),
            ("summary_failed", {"summary": {"passed": 7, "failed": 1}}),
        ]
        for name, replacement in cases:
            with self.subTest(name=name):
                payload = accepted_p8bc_report()
                payload.update(replacement)
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8bc.json", payload)

                    report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(evidence["reasons"])
                self.assertEqual(blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"), "blocked")
                self.assertEqual(report["status"], "blocked")

    def test_missing_p8bc_top_level_key_is_rejected(self) -> None:
        payload = accepted_p8bc_report()
        del payload["redaction"]
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            report_path = write_json(root, "reports/p8bc.json", payload)

            report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

        evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(evidence["status"], "rejected")
        self.assertTrue(any("missing required top-level key: redaction" in reason for reason in evidence["reasons"]))

    def test_missing_or_failed_p8bc_required_check_is_rejected(self) -> None:
        missing_check = accepted_p8bc_report()
        missing_check["checks"] = [
            check
            for check in missing_check["checks"]
            if check["name"] != "no_implicit_fallback_sources_used"
        ]
        failed_check = accepted_p8bc_report()
        failed_check["checks"][0]["passed"] = False

        cases = [
            ("missing_check", missing_check),
            ("failed_check", failed_check),
        ]
        for name, payload in cases:
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8bc.json", payload)

                    report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(evidence["reasons"])

    def test_missing_or_false_p8bc_required_input_and_redaction_values_are_rejected(self) -> None:
        missing_input = accepted_p8bc_report()
        del missing_input["inputs"]["launcher_supplied_planner_path"]
        false_input = accepted_p8bc_report()
        false_input["inputs"]["path_executable"] = False
        missing_redaction_flag = accepted_p8bc_report()
        del missing_redaction_flag["redaction"]["sensitive_values_omitted"]
        false_redaction_flag = accepted_p8bc_report()
        false_redaction_flag["redaction"]["process_output_omitted"] = False

        cases = [
            ("missing_input", missing_input, "inputs.launcher_supplied_planner_path"),
            ("false_input", false_input, "inputs.path_executable"),
            ("missing_redaction_flag", missing_redaction_flag, "redaction.sensitive_values_omitted"),
            ("false_redaction_flag", false_redaction_flag, "redaction.process_output_omitted"),
        ]
        for name, payload, reason_token in cases:
            with self.subTest(name=name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8bc.json", payload)

                    report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(any(reason_token in reason for reason in evidence["reasons"]))

    def test_p8bc_denylisted_key_rejection_is_recursive_and_exact_match_only(self) -> None:
        rejected = accepted_p8bc_report()
        rejected["checks"][0]["details"] = {"raw_command": "do-not-store"}
        accepted = accepted_p8bc_report()
        accepted["artifacts"]["argv0_basename"] = "emuchef-plan-shadow"
        accepted["inputs"]["argv0_corresponds_to_launcher_path"] = True

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            rejected_path = write_json(root, "reports/rejected.json", rejected)
            accepted_path = write_json(root, "reports/accepted.json", accepted)

            rejected_report = self.build_report(root, p8bc_launcher_injected_planner_report=rejected_path)
            accepted_report = self.build_report(root, p8bc_launcher_injected_planner_report=accepted_path)

        rejected_evidence = rejected_report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        accepted_evidence = accepted_report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(rejected_evidence["status"], "rejected")
        self.assertTrue(any("report.checks[0].details.raw_command" in reason for reason in rejected_evidence["reasons"]))
        self.assertEqual(accepted_evidence["status"], "accepted")
        self.assertEqual(accepted_evidence["reasons"], [])

    def test_sensitive_evidence_keys_are_rejected_across_supplied_payloads(self) -> None:
        for key in (
            "command",
            "argv",
            "stdout",
            "stderr",
            "env",
            "serial",
            "planner_path",
            "absolute_path",
            "cwd",
            "home",
        ):
            with self.subTest(key=key):
                payload = accepted_p8aj_report()
                payload["cases"][0][key] = "do-not-store"
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8aj.json", payload)

                    report = self.build_report(root, p8aj_live_probe_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8aj_live_probe"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(any(f"report.cases[0].{key}" in reason for reason in evidence["reasons"]))

    def test_p8bc_local_path_values_are_rejected_but_safe_schema_values_and_basenames_are_allowed(self) -> None:
        path_values = [
            "/Users/example/Projects/EmuChef",
            "~/Projects/EmuChef",
            "C:\\Users\\example\\EmuChef",
            "C:/Users/example/EmuChef",
            "\\\\server\\share\\EmuChef",
        ]
        for value in path_values:
            with self.subTest(value=value):
                payload = accepted_p8bc_report()
                payload["inputs"]["non_sensitive_extra"] = value
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8bc.json", payload)

                    report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(any("local path-looking value" in reason for reason in evidence["reasons"]))

        accepted = accepted_p8bc_report()
        accepted["inputs"]["non_sensitive_extra"] = "temporary_fixture_json"
        accepted["artifacts"]["safe_basename"] = "emuchef-plan-shadow"
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            report_path = write_json(root, "reports/p8bc.json", accepted)

            report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

        evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(evidence["status"], "accepted")

    def test_p8bc_argv0_basename_must_be_safe_basename(self) -> None:
        cases = [
            ("slash", "nested/emuchef-plan-shadow"),
            ("backslash", "nested\\emuchef-plan-shadow"),
            ("empty", ""),
        ]
        for name, argv0_basename in cases:
            with self.subTest(name=name):
                payload = accepted_p8bc_report()
                payload["artifacts"]["argv0_basename"] = argv0_basename
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    make_synthetic_repo(root)
                    report_path = write_json(root, "reports/p8bc.json", payload)

                    report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

                evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
                self.assertEqual(evidence["status"], "rejected")
                self.assertTrue(any("artifacts.argv0_basename" in reason for reason in evidence["reasons"]))

    def test_p8bc_non_sensitive_extra_field_is_ignored(self) -> None:
        payload = accepted_p8bc_report()
        payload["inputs"]["additional_classification"] = "external_wrapper"
        payload["checks"][0]["classification"] = "temporary_fixture_json"
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            report_path = write_json(root, "reports/p8bc.json", payload)

            report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

        evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(evidence["status"], "accepted")
        self.assertEqual(evidence["reasons"], [])

    def test_generated_p8bc_smoke_report_shape_is_accepted_by_readiness_gate(self) -> None:
        smoke = import_smoke_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            launcher_wrapper = executable_file(root / "emuchef-plan-shadow")

            def fake_run_process(command, *, cwd, observation_path):
                observation_path.write_text(json.dumps({"argv0": str(launcher_wrapper)}), encoding="utf-8")
                return subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout="Planning status: success\nExecution plan: plan.shadow.test\n",
                    stderr="",
                )

            with patch.object(smoke, "run_process", side_effect=fake_run_process):
                payload = smoke.run_smoke_report(
                    authored_root=str(root / "authored"),
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin=str(launcher_wrapper),
                    repo_root=REPO_ROOT,
                    generated_at="2026-06-21T00:00:00Z",
                )
            report_path = write_json(root, ".local/evidence/p8bc-launcher-injected-planner.json", payload)

            report = self.build_report(root, p8bc_launcher_injected_planner_report=report_path)

        evidence = report["production_equivalent_evidence"]["p8bc_launcher_injected_planner"]
        self.assertEqual(evidence["status"], "accepted")
        self.assertEqual(evidence["reasons"], [])
        self.assertEqual(
            blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"),
            "evidence_accepted",
        )

    def test_sensitive_evidence_fields_are_rejected_without_rejecting_classification_fields(self) -> None:
        accepted_with_classification_fields = accepted_p8aj_report()
        rejected_with_raw_stdout = accepted_p8aj_report()
        rejected_with_raw_stdout["cases"][0]["stdout"] = "Planning status: success\n"
        rejected_with_serial = accepted_p8ak_report()
        rejected_with_serial["cases"][0]["serial"] = "RAW-SERIAL"

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)
            accepted_path = write_json(root, "reports/accepted.json", accepted_with_classification_fields)
            raw_stdout_path = write_json(root, "reports/raw_stdout.json", rejected_with_raw_stdout)
            raw_serial_path = write_json(root, "reports/raw_serial.json", rejected_with_serial)

            accepted_report = self.build_report(root, p8aj_live_probe_report=accepted_path)
            raw_stdout_report = self.build_report(root, p8aj_live_probe_report=raw_stdout_path)
            raw_serial_report = self.build_report(root, p8ak_mismatch_warning_report=raw_serial_path)

        self.assertEqual(accepted_report["production_equivalent_evidence"]["p8aj_live_probe"]["status"], "accepted")
        self.assertEqual(raw_stdout_report["production_equivalent_evidence"]["p8aj_live_probe"]["status"], "rejected")
        self.assertEqual(
            raw_serial_report["production_equivalent_evidence"]["p8ak_mismatch_warning"]["status"],
            "rejected",
        )

    def test_parser_accepts_optional_production_equivalent_evidence_report_paths(self) -> None:
        args = self.readiness.parse_args(
            [
                "--p8aj-live-probe-report",
                "reports/p8aj.json",
                "--p8ak-mismatch-warning-report",
                "reports/p8ak.json",
                "--p8bc-launcher-injected-planner-report",
                "reports/p8bc.json",
            ]
        )

        self.assertEqual(args.p8aj_live_probe_report, "reports/p8aj.json")
        self.assertEqual(args.p8ak_mismatch_warning_report, "reports/p8ak.json")
        self.assertEqual(args.p8bc_launcher_injected_planner_report, "reports/p8bc.json")

    def test_report_status_remains_blocked_when_static_checks_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        self.assertTrue(all(check["status"] == "pass" for check in report["static_checks"]))
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(blocker_status(report, "default_cli_backend_still_python"), "resolved")
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "real_device_probing_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "detected_device_profile_mismatch_warning_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_launcher_injection_evidence_not_accepted"), "blocked")
        self.assertEqual(blocker_status(report, "packaged_release_not_ready"), "blocked")
        self.assertNotIn(
            retired_python_deletion_blocker_id(),
            {blocker["id"] for blocker in report["remaining_blockers"]},
        )

    def test_report_includes_narrowed_context_blockers_and_resolved_default_backend_history(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        blockers = {blocker["id"]: blocker["status"] for blocker in report["remaining_blockers"]}
        self.assertEqual(blockers["default_cli_backend_still_python"], "resolved")
        self.assertEqual(blockers["real_device_probing_not_cut_over"], "blocked")
        self.assertEqual(blockers["detected_device_profile_mismatch_warning_not_cut_over"], "blocked")
        self.assertEqual(blockers["packaged_launcher_injection_evidence_not_accepted"], "blocked")
        self.assertEqual(blockers["packaged_release_not_ready"], "blocked")
        self.assertEqual(blockers["executor_apply_not_cut_over"], "blocked")
        self.assertNotIn(retired_python_deletion_blocker_id(), blockers)
        self.assertNotIn("real_device_context_probing_not_cut_over", blockers)

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
            r"^\s*import\s+tools\.smoke_rust_production_equivalent_live_adb_probe\b",
            r"^\s*from\s+tools\.smoke_rust_production_equivalent_live_adb_probe\b",
            r"^\s*import\s+tools\.smoke_rust_production_equivalent_mismatch_warning\b",
            r"^\s*from\s+tools\.smoke_rust_production_equivalent_mismatch_warning\b",
            r"^\s*import\s+tools\.smoke_launcher_injected_planner\b",
            r"^\s*from\s+tools\.smoke_launcher_injected_planner\b",
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

    def test_source_has_no_subprocess_calls(self) -> None:
        source = TOOL_PATH.read_text(encoding="utf-8")

        self.assertNotIn("import subprocess", source)
        self.assertNotIn("subprocess.", source)

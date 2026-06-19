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
    ) -> dict:
        return self.readiness.build_readiness_report(
            repo_root=root,
            authored_root=Path("authored"),
            scenario_matrix=Path("tools/plan_parity_scenarios.json"),
            p8aj_live_probe_report=p8aj_live_probe_report,
            p8ak_mismatch_warning_report=p8ak_mismatch_warning_report,
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
        self.assertEqual(report["status"], "blocked")

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
        self.assertEqual(blocker_status(report, "default_cli_backend_still_python"), "blocked")
        self.assertEqual(blocker_status(report, "executor_apply_not_cut_over"), "blocked")
        self.assertEqual(blocker_status(report, "python_planner_deletion_not_ready"), "blocked")
        self.assertEqual(report["status"], "blocked")

    def test_failed_or_incomplete_evidence_reports_do_not_mark_blockers_accepted(self) -> None:
        p8aj_failed = accepted_p8aj_report()
        p8aj_failed["summary"]["failed"] = 1
        p8aj_not_live = accepted_p8aj_report()
        p8aj_not_live["inputs"]["live_probe_requested"] = False
        p8ak_failed = accepted_p8ak_report()
        p8ak_failed["summary"]["failed"] = 1
        p8ak_missing_case = accepted_p8ak_report()
        p8ak_missing_case["cases"] = [
            case for case in p8ak_missing_case["cases"] if case["id"] != "android_minimum_match"
        ]

        cases = [
            ("p8aj_failed", "reports/p8aj.json", p8aj_failed, "real_device_probing_not_cut_over"),
            ("p8aj_not_live", "reports/p8aj.json", p8aj_not_live, "real_device_probing_not_cut_over"),
            (
                "p8ak_failed",
                "reports/p8ak.json",
                p8ak_failed,
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
            ]
        )

        self.assertEqual(args.p8aj_live_probe_report, "reports/p8aj.json")
        self.assertEqual(args.p8ak_mismatch_warning_report, "reports/p8ak.json")

    def test_report_status_remains_blocked_when_static_checks_pass(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        self.assertTrue(all(check["status"] == "pass" for check in report["static_checks"]))
        self.assertEqual(report["status"], "blocked")
        self.assertTrue(all(blocker["status"] == "blocked" for blocker in report["remaining_blockers"]))

    def test_report_includes_narrowed_context_blockers_and_existing_cutover_blockers(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            make_synthetic_repo(root)

            report = self.build_report(root)

        blockers = {blocker["id"]: blocker["status"] for blocker in report["remaining_blockers"]}
        self.assertEqual(blockers["default_cli_backend_still_python"], "blocked")
        self.assertEqual(blockers["real_device_probing_not_cut_over"], "blocked")
        self.assertEqual(blockers["detected_device_profile_mismatch_warning_not_cut_over"], "blocked")
        self.assertEqual(blockers["executor_apply_not_cut_over"], "blocked")
        self.assertEqual(blockers["python_planner_deletion_not_ready"], "blocked")
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
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

    def test_source_has_no_subprocess_calls(self) -> None:
        source = TOOL_PATH.read_text(encoding="utf-8")

        self.assertNotIn("import subprocess", source)
        self.assertNotIn("subprocess.", source)

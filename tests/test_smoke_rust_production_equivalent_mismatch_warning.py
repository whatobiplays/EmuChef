from __future__ import annotations

import ast
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
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_production_equivalent_mismatch_warning.py"


def import_smoke_module():
    module_name = "smoke_rust_production_equivalent_mismatch_warning"
    spec = importlib.util.spec_from_file_location(module_name, SMOKE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {SMOKE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def python_summary(*, status: str = "success", warning_code: str | None = None) -> str:
    lines = [
        f"Planning status: {status}",
        "Execution plan: plan.shadow.ayaneo.pocket_s_mini.base.001",
        "Runnable steps:",
        "- app.retroarch.provision/configure",
    ]
    if warning_code is not None:
        lines.extend(
            [
                "Warnings:",
                f"- {warning_code}: Selected profile does not match.",
            ]
        )
    return "\n".join(lines) + "\n"


class SmokeRustProductionEquivalentMismatchWarningPresenceTests(unittest.TestCase):
    def test_tool_path_exists(self) -> None:
        self.assertTrue(SMOKE_PATH.exists())


@unittest.skipUnless(SMOKE_PATH.exists(), "smoke tool is not implemented yet")
class SmokeRustProductionEquivalentMismatchWarningTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_python_route_command_with_fixture_flag_only(self) -> None:
        command = self.smoke.build_cli_command(
            python_executable="python3",
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            fixture_path=Path("/tmp/facts.json"),
        )

        self.assertEqual(command[:4], ["python3", "-m", "emuchef", "plan"])
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-production-equivalent")
        self.assertEqual(command[command.index("--rust-planner-bin") + 1], "/tmp/emuchef-plan-shadow")
        self.assertEqual(command[command.index("--authored-root") + 1], "authored")
        self.assertEqual(command[command.index("--device-plan") + 1], "ayaneo.pocket_s_mini.base")
        self.assertEqual(command[command.index("--rust-detected-facts-json") + 1], "/tmp/facts.json")
        self.assertNotEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertNotIn("--detected-facts-json", command)
        self.assertNotIn("--rust-probe-adb-getprop", command)
        self.assertNotIn("--rust-adb-path", command)
        self.assertNotIn("--rust-serial", command)
        self.assertNotIn("--probe-adb-getprop", command)
        self.assertNotIn("--adb-path", command)
        self.assertNotIn("--serial", command)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb", command)

    def test_smoke_cases_use_expected_pocket_s_mini_warning_matrix(self) -> None:
        cases = {case.id: case for case in self.smoke.SMOKE_CASES}

        self.assertEqual(
            tuple(cases),
            (
                "matched_profile",
                "manufacturer_mismatch",
                "model_mismatch",
                "android_minimum_mismatch",
                "android_minimum_match",
            ),
        )
        self.assertFalse(cases["matched_profile"].expected_mismatch_warning)
        self.assertTrue(cases["manufacturer_mismatch"].expected_mismatch_warning)
        self.assertTrue(cases["model_mismatch"].expected_mismatch_warning)
        self.assertTrue(cases["android_minimum_mismatch"].expected_mismatch_warning)
        self.assertFalse(cases["android_minimum_match"].expected_mismatch_warning)
        self.assertEqual(cases["matched_profile"].fixture_payload["manufacturer"], "AYANEO")
        self.assertEqual(cases["manufacturer_mismatch"].fixture_payload["manufacturer"], "Valve")
        self.assertEqual(cases["model_mismatch"].fixture_payload["model"], "Steam Deck")
        self.assertEqual(cases["android_minimum_mismatch"].fixture_payload["android_version"], 12)
        self.assertEqual(cases["android_minimum_match"].fixture_payload["android_version"], 13)

    def test_fixture_json_shape_is_deterministic_and_expected(self) -> None:
        payloads = [case.fixture_payload for case in self.smoke.SMOKE_CASES]
        expected_keys = [
            "serial",
            "manufacturer",
            "brand",
            "model",
            "android_version",
            "android_api_level",
            "device_tags",
        ]

        for payload in payloads:
            with self.subTest(payload=payload["serial"]):
                self.assertEqual(list(payload), expected_keys)
                self.assertIsInstance(payload["device_tags"], list)
                self.assertNotIn("android_version_max", payload)
                self.assertNotIn("adb_path", payload)
                self.assertNotIn("getprop", payload)

        first = self.smoke.dumps_fixture(payloads[0])
        second = self.smoke.dumps_fixture(payloads[0])
        self.assertEqual(first, second)
        self.assertTrue(first.endswith("\n"))

    def test_run_case_writes_fixture_and_invokes_python_route(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=python_summary(), stderr="")

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[0]
            with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    temp_root=Path(temp_dir),
                    repo_root=REPO_ROOT,
                )
            fixture_path = Path(run_process.call_args.args[0][run_process.call_args.args[0].index("--rust-detected-facts-json") + 1])
            fixture_payload = json.loads(fixture_path.read_text(encoding="utf-8"))

        self.assertEqual(result.status, "passed")
        self.assertEqual(result.stdout_class, "python_compatible")
        self.assertFalse(result.expected_mismatch_warning)
        self.assertFalse(result.mismatch_warning_seen)
        self.assertEqual(fixture_payload, case.fixture_payload)
        command = run_process.call_args.args[0]
        self.assertEqual(command[:4], ["python3", "-m", "emuchef", "plan"])
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-production-equivalent")
        self.assertNotEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertNotIn("--rust-probe-adb-getprop", command)
        self.assertNotIn("--rust-adb-path", command)
        self.assertNotIn("--rust-serial", command)

    def test_expected_warning_cases_pass_with_mismatch_warning(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout=python_summary(status="warning", warning_code="device_profile_mismatch"),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            for case in self.smoke.SMOKE_CASES:
                if not case.expected_mismatch_warning:
                    continue
                with self.subTest(case=case.id):
                    with patch.object(self.smoke, "run_process", return_value=completed):
                        result = self.smoke.run_case(
                            case,
                            authored_root="authored",
                            device_plan="ayaneo.pocket_s_mini.base",
                            rust_planner_bin="/tmp/emuchef-plan-shadow",
                            python_executable="python3",
                            temp_root=Path(temp_dir),
                            repo_root=REPO_ROOT,
                        )

                    self.assertEqual(result.status, "passed")
                    self.assertTrue(result.expected_mismatch_warning)
                    self.assertTrue(result.mismatch_warning_seen)
                    self.assertEqual(result.planning_status, "warning")
                    self.assertIsNone(result.failure_class)

    def test_no_warning_cases_pass_without_mismatch_warning(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=python_summary(), stderr="")

        with tempfile.TemporaryDirectory() as temp_dir:
            for case in self.smoke.SMOKE_CASES:
                if case.expected_mismatch_warning:
                    continue
                with self.subTest(case=case.id):
                    with patch.object(self.smoke, "run_process", return_value=completed):
                        result = self.smoke.run_case(
                            case,
                            authored_root="authored",
                            device_plan="ayaneo.pocket_s_mini.base",
                            rust_planner_bin="/tmp/emuchef-plan-shadow",
                            python_executable="python3",
                            temp_root=Path(temp_dir),
                            repo_root=REPO_ROOT,
                        )

                    self.assertEqual(result.status, "passed")
                    self.assertFalse(result.expected_mismatch_warning)
                    self.assertFalse(result.mismatch_warning_seen)
                    self.assertEqual(result.planning_status, "success")
                    self.assertIsNone(result.failure_class)

    def test_raw_rust_json_stdout_fails_as_incompatible(self) -> None:
        result = self.smoke.classify_completed_process(
            self.smoke.SMOKE_CASES[0],
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{"kind":"planning_result","status":"success"}\n',
                stderr="",
            ),
        )

        self.assertEqual(result.status, "failed")
        self.assertEqual(result.stdout_class, "raw_json_stdout")
        self.assertEqual(result.failure_class, "production_equivalent_output_incompatible")

    def test_missing_expected_warning_and_unexpected_warning_classify_distinctly(self) -> None:
        missing = self.smoke.classify_completed_process(
            self.smoke.SMOKE_CASES[1],
            subprocess.CompletedProcess(args=[], returncode=1, stdout=python_summary(status="warning"), stderr=""),
        )
        unexpected = self.smoke.classify_completed_process(
            self.smoke.SMOKE_CASES[0],
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=python_summary(status="success", warning_code="device_profile_mismatch"),
                stderr="",
            ),
        )

        self.assertEqual(missing.status, "failed")
        self.assertEqual(missing.failure_class, "expected_mismatch_warning_missing")
        self.assertFalse(missing.mismatch_warning_seen)
        self.assertEqual(unexpected.status, "failed")
        self.assertEqual(unexpected.failure_class, "unexpected_mismatch_warning_present")
        self.assertTrue(unexpected.mismatch_warning_seen)

    def test_usage_unexpected_exit_and_process_start_failures_classify_distinctly(self) -> None:
        cases = [
            (
                self.smoke.SMOKE_CASES[0],
                subprocess.CompletedProcess(args=[], returncode=2, stdout="", stderr="usage: emuchef plan [-h]\n"),
                "production_equivalent_usage_failed",
            ),
            (
                self.smoke.SMOKE_CASES[0],
                subprocess.CompletedProcess(args=[], returncode=3, stdout=python_summary(), stderr=""),
                "production_equivalent_unexpected_exit",
            ),
            (
                self.smoke.SMOKE_CASES[1],
                subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout=python_summary(status="warning", warning_code="device_profile_mismatch"),
                    stderr="",
                ),
                "production_equivalent_unexpected_exit",
            ),
        ]
        for case, completed, expected_class in cases:
            with self.subTest(expected_class=expected_class):
                result = self.smoke.classify_completed_process(case, completed)
                self.assertEqual(result.status, "failed")
                self.assertEqual(result.failure_class, expected_class)

        start_failure = self.smoke.process_start_failure_result(self.smoke.SMOKE_CASES[0])
        self.assertEqual(start_failure.status, "failed")
        self.assertEqual(start_failure.failure_class, "production_equivalent_process_start_failed")

    def test_run_smoke_report_returns_all_cases_and_scrubbed_report(self) -> None:
        def fake_run_process(command, *, cwd):
            fixture_path = Path(command[command.index("--rust-detected-facts-json") + 1])
            fixture_payload = json.loads(fixture_path.read_text(encoding="utf-8"))
            has_warning = fixture_payload["serial"] in {
                "P8AK-MANUFACTURER-MISMATCH",
                "P8AK-MODEL-MISMATCH",
                "P8AK-ANDROID-MINIMUM-MISMATCH",
            }
            return subprocess.CompletedProcess(
                args=[],
                returncode=1 if has_warning else 0,
                stdout=python_summary(
                    status="warning" if has_warning else "success",
                    warning_code="device_profile_mismatch" if has_warning else None,
                ),
                stderr="",
            )

        with patch.object(self.smoke, "run_process", side_effect=fake_run_process):
            report = self.smoke.run_smoke_report(
                authored_root="/Users/example/Projects/EmuChef/authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/Users/example/target/debug/emuchef-plan-shadow",
                repo_root=REPO_ROOT,
            )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["kind"], "rust_production_equivalent_mismatch_warning_smoke")
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["route_backend"], "rust-production-equivalent")
        self.assertEqual(report["inputs"]["route_output_mode"], "python-compatible")
        self.assertEqual(report["inputs"]["detected_facts_source"], "temporary_fixture_json")
        self.assertEqual(report["summary"], {"passed": 5, "failed": 0, "skipped": 0})
        self.assertEqual([case["id"] for case in report["cases"]], [case.id for case in self.smoke.SMOKE_CASES])
        for case in report["cases"]:
            self.assertEqual(
                set(case),
                {
                    "id",
                    "status",
                    "expected_mismatch_warning",
                    "mismatch_warning_seen",
                    "planning_status",
                    "stdout_class",
                    "failure_class",
                },
            )
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn(tempfile.gettempdir(), serialized)
        self.assertNotIn("P8AK-", serialized)
        self.assertNotIn("raw", serialized.lower())
        self.assertNotIn("stdout", serialized.lower().replace("stdout_class", ""))
        self.assertNotIn("stderr", serialized.lower())
        self.assertNotIn("command", serialized.lower())
        self.assertNotIn("argv", serialized.lower())
        self.assertNotIn("environment", serialized.lower())
        self.assertEqual(self.smoke.smoke_exit_code(report), 0)

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        results = [
            self.smoke.CaseResult(
                id="matched_profile",
                status="passed",
                expected_mismatch_warning=False,
                mismatch_warning_seen=False,
                planning_status="success",
                stdout_class="python_compatible",
                failure_class=None,
            )
        ]

        first = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                case_results=results,
            )
        )
        second = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                case_results=results,
            )
        )

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first)["summary"], {"passed": 1, "failed": 0, "skipped": 0})

    def test_windows_style_paths_are_scrubbed(self) -> None:
        result = self.smoke.CaseResult(
            id="matched_profile",
            status="passed",
            expected_mismatch_warning=False,
            mismatch_warning_seen=False,
            planning_status="success",
            stdout_class="python_compatible",
            failure_class=None,
        )
        report = self.smoke.build_report(
            authored_root=r"C:\Users\example\Projects\EmuChef\authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin=r"C:\Users\example\target\debug\emuchef-plan-shadow.exe",
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow.exe")
        self.assertNotIn("C:\\Users", serialized)
        self.assertNotIn("target\\debug", serialized)
        self.assertNotIn("example", serialized)

    def test_source_is_stdlib_only_and_not_reusing_other_smoke_identities(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")
        tree = ast.parse(source)
        allowed_modules = {
            "__future__",
            "argparse",
            "dataclasses",
            "json",
            "pathlib",
            "re",
            "subprocess",
            "sys",
            "tempfile",
            "typing",
        }

        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    self.assertIn(alias.name.split(".")[0], allowed_modules)
            elif isinstance(node, ast.ImportFrom):
                self.assertIsNotNone(node.module)
                self.assertIn(node.module.split(".")[0], allowed_modules)

        forbidden_patterns = [
            r"^\s*import\s+emuchef\b",
            r"^\s*from\s+emuchef\b",
            r"^\s*import\s+yaml\b",
            r"^\s*from\s+yaml\b",
            r"^\s*import\s+tools\.",
            r"^\s*from\s+tools\.",
            r"^\s*import\s+crates\b",
            r"^\s*from\s+crates\b",
            r"^\s*import\s+tauri\b",
            r"^\s*from\s+tauri\b",
        ]
        for pattern in forbidden_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

        forbidden_substrings = [
            "shell=True",
            "cargo",
            "adb devices",
            "rust-experimental",
            "--rust-probe-adb-getprop",
            "--rust-adb-path",
            "--rust-serial",
            "--probe-adb-getprop",
            "--adb-path",
            "--serial",
            "--rust-shadow-output",
            ".expanduser(",
            ".exists(",
            ".is_file(",
            ".resolve(",
            ".stat(",
            "os.access",
            "SubprocessAdb",
            "resolve_adb_executable",
            "_run_apply",
            "executor/apply",
            "Tauri",
            "protocol",
            "rust_production_equivalent_live_adb_probe_smoke",
            "rust_experimental_detected_facts_fixture_smoke",
        ]
        for token in forbidden_substrings:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_experimental_live_adb_probe.py"


def import_smoke_module():
    module_name = "smoke_rust_experimental_live_adb_probe"
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


class SmokeRustExperimentalLiveAdbProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_python_route_command_with_wrapper_probe_flags(self) -> None:
        command = self.smoke.build_cli_command(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            adb_path="adb",
            serial="SERIAL123",
        )

        self.assertEqual(
            command,
            [
                sys.executable,
                "-m",
                "emuchef",
                "plan",
                "--planner-backend",
                "rust-experimental",
                "--rust-planner-bin",
                "/tmp/emuchef-plan-shadow",
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
                "--rust-probe-adb-getprop",
                "--rust-adb-path",
                "adb",
                "--rust-serial",
                "SERIAL123",
            ],
        )
        self.assertEqual(command[:4], [sys.executable, "-m", "emuchef", "plan"])
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-experimental")
        self.assertIn("--rust-probe-adb-getprop", command)
        self.assertIn("--rust-adb-path", command)
        self.assertIn("--rust-serial", command)
        self.assertNotIn("--probe-adb-getprop", command)
        self.assertNotIn("--adb-path", command)
        self.assertNotIn("--serial", command)
        self.assertNotIn("--detected-facts-json", command)
        self.assertNotIn("--rust-detected-facts-json", command)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb devices", " ".join(command))

    def test_run_smoke_report_invokes_python_route_not_shadow_binary_directly(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(),
            stderr="",
        )

        with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
            report = self.smoke.run_smoke_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                adb_path="adb",
                serial="SERIAL123",
            )

        self.assertEqual(self.smoke.smoke_exit_code(report), 0)
        run_process.assert_called_once()
        command = run_process.call_args.args[0]
        self.assertEqual(command[:4], [sys.executable, "-m", "emuchef", "plan"])
        self.assertEqual(command[command.index("--rust-adb-path") + 1], "adb")
        self.assertEqual(command[command.index("--rust-serial") + 1], "SERIAL123")
        self.assertNotEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertNotIn("--probe-adb-getprop", command)
        self.assertNotIn("--adb-path", command)
        self.assertNotIn("--serial", command)
        self.assertNotIn("cargo", command)

    def test_report_scrubs_serial_and_posix_paths(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(),
            stderr="",
        )

        with patch.object(self.smoke, "run_process", return_value=completed):
            report = self.smoke.run_smoke_report(
                authored_root="/Users/example/Projects/EmuChef/authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/Users/example/target/debug/emuchef-plan-shadow",
                adb_path="/Users/example/Library/Android/sdk/platform-tools/adb",
                serial="SERIAL123",
            )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["adb_path"], "adb")
        self.assertTrue(report["inputs"]["serial_supplied"])
        self.assertNotIn("SERIAL123", serialized)
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn("Library/Android", serialized)
        self.assertNotIn("target/debug", serialized)
        self.assertNotIn("command", report)
        self.assertNotIn("argv", serialized)

    def test_report_scrubs_windows_style_paths_with_pure_windows_path(self) -> None:
        result = self.smoke.CaseResult(
            id="rust_experimental_live_adb_probe_forwarding",
            status="pass",
            exit_class="success_or_warning",
            stdout_class="python_compatible",
            stderr_class="empty",
            planning_status="success",
            device_profile_mismatch_seen=False,
            failure_class=None,
        )

        report = self.smoke.build_report(
            authored_root=r"C:\Users\example\Projects\EmuChef\authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin=r"C:\Users\example\target\debug\emuchef-plan-shadow.exe",
            adb_path=r"C:\Users\example\AppData\Local\Android\Sdk\platform-tools\adb.exe",
            serial_supplied=True,
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow.exe")
        self.assertEqual(report["inputs"]["adb_path"], "adb.exe")
        self.assertNotIn("C:\\Users", serialized)
        self.assertNotIn("AppData", serialized)
        self.assertNotIn("platform-tools", serialized)
        self.assertNotIn("target\\debug", serialized)
        self.assertNotIn("example", serialized)

    def test_concise_success_output_passes(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(status="success"),
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.exit_class, "success_or_warning")
        self.assertEqual(result.stdout_class, "python_compatible")
        self.assertEqual(result.stderr_class, "empty")
        self.assertEqual(result.planning_status, "success")
        self.assertFalse(result.device_profile_mismatch_seen)

    def test_warning_device_profile_mismatch_output_passes_as_route_evidence(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout=python_summary(status="warning", warning_code="device_profile_mismatch"),
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.planning_status, "warning")
        self.assertTrue(result.device_profile_mismatch_seen)

    def test_structured_yaml_status_output_passes(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="kind: planning_result\nstatus: success\nexecution_plan:\n  id: plan.shadow.example.001\n",
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.stdout_class, "python_compatible")
        self.assertEqual(result.planning_status, "success")

    def test_raw_rust_json_stdout_fails(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout='{"kind":"planning_result","status":"success"}\n',
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.stdout_class, "raw_json_stdout")
        self.assertEqual(result.failure_class, "raw_json_stdout")

    def test_stable_adb_probe_unavailable_failure_returns_nonzero(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout="",
            stderr="Error: adb_probe_unavailable\nError: route output was unavailable.\n",
        )

        result = self.smoke.classify_completed_process(completed)
        report = self.smoke.build_report(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            adb_path="adb",
            serial_supplied=True,
            case_results=[result],
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "adb_probe_unavailable")
        self.assertEqual(result.stderr_class, "adb_probe_unavailable")
        self.assertEqual(self.smoke.smoke_exit_code(report), 1)
        self.assertNotIn("Error:", self.smoke.dumps_report(report))

    def test_stable_adb_probe_failed_failure_returns_nonzero(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout="",
            stderr="Error: adb_probe_failed\n",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "adb_probe_failed")
        self.assertEqual(result.stderr_class, "adb_probe_failed")

    def test_usage_failure_returns_nonzero(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=2,
            stdout="",
            stderr="usage: emuchef plan [-h]\n",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "usage_failure")
        self.assertEqual(result.stderr_class, "usage_failure")

    def test_stderr_text_fails_without_leaking_stderr_content(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(),
            stderr="host path /Users/example and serial SERIAL123 should not leak\n",
        )

        result = self.smoke.classify_completed_process(completed)
        report = self.smoke.build_report(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            adb_path="adb",
            serial_supplied=True,
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.failure_class, "stderr_text")
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn("SERIAL123", serialized)
        self.assertNotIn("should not leak", serialized)

    def test_unexpected_exit_fails(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=3,
            stdout=python_summary(),
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "unexpected_exit")

    def test_process_start_failure_is_distinct_from_adb_probe_failures(self) -> None:
        result = self.smoke.process_start_failure_result()

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "shadow_process_start_failed")
        self.assertEqual(result.failure_class, "shadow_process_start_failed")
        self.assertNotEqual(result.exit_class, "adb_probe_unavailable")
        self.assertNotEqual(result.exit_class, "adb_probe_failed")

    def test_run_smoke_report_classifies_process_start_failure(self) -> None:
        with patch.object(self.smoke, "run_process", side_effect=OSError):
            report = self.smoke.run_smoke_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                adb_path="adb",
                serial="SERIAL123",
            )

        case = report["cases"][0]
        self.assertEqual(case["status"], "fail")
        self.assertEqual(case["failure_class"], "shadow_process_start_failed")
        self.assertEqual(self.smoke.smoke_exit_code(report), 1)

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        result = self.smoke.CaseResult(
            id="rust_experimental_live_adb_probe_forwarding",
            status="pass",
            exit_class="success_or_warning",
            stdout_class="python_compatible",
            stderr_class="empty",
            planning_status="success",
            device_profile_mismatch_seen=False,
            failure_class=None,
        )

        first = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                adb_path="adb",
                serial_supplied=True,
                case_results=[result],
            )
        )
        second = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                adb_path="adb",
                serial_supplied=True,
                case_results=[result],
            )
        )

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first)["summary"], {"passed": 1, "failed": 0})

    def test_source_has_no_forbidden_imports_or_harness_reuse(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")

        forbidden_import_patterns = [
            r"^\s*import\s+emuchef\b",
            r"^\s*from\s+emuchef\b",
            r"^\s*import\s+yaml\b",
            r"^\s*from\s+yaml\b",
            r"^\s*import\s+tools\.compare_rust_python_plan\b",
            r"^\s*from\s+tools\.compare_rust_python_plan\b",
            r"^\s*import\s+tools\.smoke_rust_shadow_cli_matrix\b",
            r"^\s*from\s+tools\.smoke_rust_shadow_cli_matrix\b",
            r"^\s*import\s+tools\.smoke_rust_detected_facts_fixture\b",
            r"^\s*from\s+tools\.smoke_rust_detected_facts_fixture\b",
            r"^\s*import\s+tools\.smoke_rust_shadow_live_adb_probe\b",
            r"^\s*from\s+tools\.smoke_rust_shadow_live_adb_probe\b",
            r"^\s*import\s+tools\.smoke_rust_experimental_detected_facts_fixture\b",
            r"^\s*from\s+tools\.smoke_rust_experimental_detected_facts_fixture\b",
            r"^\s*import\s+crates\b",
            r"^\s*from\s+crates\b",
            r"^\s*import\s+tauri\b",
            r"^\s*from\s+tauri\b",
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

    def test_source_does_not_invoke_forbidden_routes_or_raw_flags(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")

        forbidden_substrings = [
            "shell=True",
            "cargo",
            "adb devices",
            "emuchef-plan-shadow",
            "--detected-facts-json",
            "--rust-detected-facts-json",
            "--planner-backend rust-shadow",
            "--rust-shadow-output",
            "compare_rust_python_plan",
            "smoke_rust_shadow_cli_matrix",
            "smoke_rust_detected_facts_fixture",
            "smoke_rust_shadow_live_adb_probe",
            "smoke_rust_experimental_detected_facts_fixture",
        ]
        for token in forbidden_substrings:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

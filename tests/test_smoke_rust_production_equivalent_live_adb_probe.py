from __future__ import annotations

import ast
import importlib.util
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_production_equivalent_live_adb_probe.py"


def import_smoke_module():
    module_name = "smoke_rust_production_equivalent_live_adb_probe"
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


class SmokeRustProductionEquivalentLiveAdbProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_exact_python_cli_command_with_wrapper_probe_flags(self) -> None:
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
                "rust-production-equivalent",
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
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-production-equivalent")
        self.assertNotEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertNotIn("--probe-adb-getprop", command)
        self.assertNotIn("--adb-path", command)
        self.assertNotIn("--serial", command)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb devices", " ".join(command))

    def test_optional_context_flags_and_bindings_are_forwarded_exactly(self) -> None:
        command = self.smoke.build_cli_command(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            adb_path="~/Android SDK/platform-tools/adb",
            serial="SERIAL $UNCHANGED",
            manufacturer="AYANEO",
            model="Pocket S Mini",
            android_version="13",
            device_tags=("handheld", "landscape"),
            bindings=(
                "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
                "app.retroarch.provision/retroarch_cfg=/tmp/retroarch.cfg",
            ),
        )

        self.assertEqual(
            command[-14:],
            [
                "--manufacturer",
                "AYANEO",
                "--model",
                "Pocket S Mini",
                "--android-version",
                "13",
                "--device-tag",
                "handheld",
                "--device-tag",
                "landscape",
                "--bind",
                "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
                "--bind",
                "app.retroarch.provision/retroarch_cfg=/tmp/retroarch.cfg",
            ],
        )
        self.assertEqual(command[command.index("--rust-adb-path") + 1], "~/Android SDK/platform-tools/adb")
        self.assertEqual(command[command.index("--rust-serial") + 1], "SERIAL $UNCHANGED")

    def test_run_smoke_report_invokes_python_route_not_rust_binary_directly(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=python_summary(), stderr="")

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
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-production-equivalent")
        self.assertNotEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertNotIn("cargo", command)
        self.assertTrue(report["inputs"]["live_probe_requested"])

    def test_report_scrubs_serial_paths_and_host_values(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=python_summary(), stderr="")

        with patch.object(self.smoke, "run_process", return_value=completed):
            report = self.smoke.run_smoke_report(
                authored_root="/Users/example/Projects/EmuChef/authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/Users/example/target/debug/emuchef-plan-shadow",
                adb_path="/Users/example/Library/Android/sdk/platform-tools/adb",
                serial="SERIAL123",
                manufacturer="AYANEO",
                model="Pocket S Mini",
                android_version="13",
                device_tags=("handheld",),
                bindings=("app.retroarch.provision/retroarch_cfg=/Users/example/retroarch.cfg",),
            )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["adb_path"], "adb")
        self.assertTrue(report["inputs"]["serial_supplied"])
        self.assertTrue(report["inputs"]["live_probe_requested"])
        self.assertEqual(
            report["inputs"]["context_overrides"],
            {
                "manufacturer_supplied": True,
                "model_supplied": True,
                "android_version_supplied": True,
                "device_tag_count": 1,
            },
        )
        self.assertEqual(report["inputs"]["binding_count"], 1)
        self.assertNotIn("SERIAL123", serialized)
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn("Library/Android", serialized)
        self.assertNotIn("target/debug", serialized)
        self.assertNotIn("retroarch.cfg", serialized)
        self.assertNotIn("command", report)
        self.assertNotIn("argv", serialized)

    def test_report_scrubs_windows_style_paths_with_pure_windows_path(self) -> None:
        result = self.smoke.CaseResult(
            id="rust_production_equivalent_live_adb_probe_forwarding",
            status="passed",
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
            manufacturer=None,
            model=None,
            android_version=None,
            device_tags=(),
            bindings=(),
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

    def test_concise_python_compatible_success_and_warning_pass(self) -> None:
        cases = [
            (0, python_summary(status="success"), "success", False),
            (1, python_summary(status="warning", warning_code="device_profile_mismatch"), "warning", True),
        ]
        for returncode, stdout, expected_status, expected_warning in cases:
            with self.subTest(expected_status=expected_status):
                completed = subprocess.CompletedProcess(args=[], returncode=returncode, stdout=stdout, stderr="")

                result = self.smoke.classify_completed_process(completed)

                self.assertEqual(result.status, "passed")
                self.assertEqual(result.exit_class, "success_or_warning")
                self.assertEqual(result.stdout_class, "python_compatible")
                self.assertEqual(result.stderr_class, "empty")
                self.assertEqual(result.planning_status, expected_status)
                self.assertEqual(result.device_profile_mismatch_seen, expected_warning)

    def test_yaml_compatible_success_passes(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="kind: planning_result\nstatus: success\nexecution_plan:\n  id: plan.shadow.example.001\n",
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "passed")
        self.assertEqual(result.stdout_class, "python_compatible")
        self.assertEqual(result.planning_status, "success")

    def test_raw_rust_json_stdout_fails_as_incompatible(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout='{"kind":"planning_result","status":"success"}\n',
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "failed")
        self.assertEqual(result.stdout_class, "raw_json_stdout")
        self.assertEqual(result.failure_class, "production_equivalent_output_incompatible")

    def test_stable_adb_probe_failures_are_classified_without_leaking_stderr(self) -> None:
        cases = [
            ("Error: adb_probe_unavailable\nError: route output was unavailable.\n", "adb_probe_unavailable"),
            ("Error: adb_probe_failed\n", "adb_probe_failed"),
        ]
        for stderr, expected_class in cases:
            with self.subTest(expected_class=expected_class):
                completed = subprocess.CompletedProcess(args=[], returncode=1, stdout="", stderr=stderr)

                result = self.smoke.classify_completed_process(completed)
                report = self.smoke.build_report(
                    authored_root="authored",
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    adb_path="adb",
                    serial_supplied=True,
                    manufacturer=None,
                    model=None,
                    android_version=None,
                    device_tags=(),
                    bindings=(),
                    case_results=[result],
                )

                self.assertEqual(result.status, "failed")
                self.assertEqual(result.exit_class, expected_class)
                self.assertEqual(result.stderr_class, expected_class)
                self.assertEqual(result.failure_class, expected_class)
                self.assertEqual(self.smoke.smoke_exit_code(report), 1)
                self.assertNotIn("Error:", self.smoke.dumps_report(report))

    def test_usage_unexpected_exit_stderr_text_and_process_start_failures_are_distinct(self) -> None:
        cases = [
            (
                subprocess.CompletedProcess(args=[], returncode=2, stdout="", stderr="usage: emuchef plan [-h]\n"),
                "production_equivalent_usage_failed",
            ),
            (
                subprocess.CompletedProcess(args=[], returncode=3, stdout=python_summary(), stderr=""),
                "production_equivalent_unexpected_exit",
            ),
            (
                subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout=python_summary(),
                    stderr="host path /Users/example and serial SERIAL123 should not leak\n",
                ),
                "stderr_text",
            ),
        ]
        for completed, expected_class in cases:
            with self.subTest(expected_class=expected_class):
                result = self.smoke.classify_completed_process(completed)

                self.assertEqual(result.status, "failed")
                self.assertEqual(result.failure_class, expected_class)

        start_failure = self.smoke.process_start_failure_result()
        self.assertEqual(start_failure.status, "failed")
        self.assertEqual(start_failure.failure_class, "production_equivalent_process_start_failed")
        self.assertNotEqual(start_failure.failure_class, "adb_probe_unavailable")
        self.assertNotEqual(start_failure.failure_class, "adb_probe_failed")

    def test_run_smoke_report_classifies_process_start_failure_without_leaking_exception(self) -> None:
        with patch.object(self.smoke, "run_process", side_effect=OSError("SERIAL123 /Users/example")):
            report = self.smoke.run_smoke_report(
                authored_root="authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                adb_path="adb",
                serial="SERIAL123",
            )

        serialized = self.smoke.dumps_report(report)
        case = report["cases"][0]
        self.assertEqual(case["status"], "failed")
        self.assertEqual(case["failure_class"], "production_equivalent_process_start_failed")
        self.assertEqual(self.smoke.smoke_exit_code(report), 1)
        self.assertNotIn("SERIAL123", serialized)
        self.assertNotIn("/Users/example", serialized)

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        result = self.smoke.CaseResult(
            id="rust_production_equivalent_live_adb_probe_forwarding",
            status="passed",
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
                manufacturer=None,
                model=None,
                android_version=None,
                device_tags=(),
                bindings=(),
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
                manufacturer=None,
                model=None,
                android_version=None,
                device_tags=(),
                bindings=(),
                case_results=[result],
            )
        )

        self.assertEqual(first, second)
        payload = json.loads(first)
        self.assertEqual(payload["summary"], {"passed": 1, "failed": 0, "skipped": 0})
        self.assertTrue(payload["inputs"]["live_probe_requested"])

    def test_source_uses_stdlib_only_and_no_forbidden_routes(self) -> None:
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
            "typing",
        }

        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    self.assertIn(alias.name.split(".")[0], allowed_modules)
            elif isinstance(node, ast.ImportFrom):
                self.assertIsNotNone(node.module)
                self.assertIn(node.module.split(".")[0], allowed_modules)

        forbidden_import_patterns = [
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
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

        forbidden_substrings = [
            "shell=True",
            "cargo",
            "adb devices",
            "rust-experimental",
            "--probe-adb-getprop",
            "--rust-shadow-output",
            "--rust-detected-facts-json",
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
        ]
        for token in forbidden_substrings:
            with self.subTest(token=token):
                self.assertNotIn(token, source)

    def test_report_identity_does_not_reuse_rust_experimental_names(self) -> None:
        result = self.smoke.CaseResult(
            id="rust_production_equivalent_live_adb_probe_forwarding",
            status="passed",
            exit_class="success_or_warning",
            stdout_class="python_compatible",
            stderr_class="empty",
            planning_status="success",
            device_profile_mismatch_seen=False,
            failure_class=None,
        )
        report = self.smoke.build_report(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            adb_path="adb",
            serial_supplied=True,
            manufacturer=None,
            model=None,
            android_version=None,
            device_tags=(),
            bindings=(),
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["kind"], "rust_production_equivalent_live_adb_probe_smoke")
        self.assertNotIn("rust_experimental", serialized)
        self.assertNotIn("rust-experimental", serialized)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import ast
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_launcher_injected_planner.py"

DENYLISTED_KEYS = {
    "command",
    "argv",
    "raw_command",
    "stdout",
    "stderr",
    "raw_stdout",
    "raw_stderr",
    "environment",
    "env",
    "serial",
    "device_serial",
    "planner_path",
    "absolute_path",
    "launcher_supplied_absolute_path",
    "cwd",
    "home",
}


def import_smoke_module():
    module_name = "smoke_launcher_injected_planner"
    spec = importlib.util.spec_from_file_location(module_name, SMOKE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {SMOKE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def executable_file(path: Path) -> Path:
    path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    path.chmod(0o700)
    return path


def observed_success(launcher_path: Path):
    def fake_run_process(command, *, cwd, observation_path):
        observation_path.write_text(json.dumps({"argv0": str(launcher_path)}), encoding="utf-8")
        return subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="Planning status: success\nExecution plan: plan.shadow.test\n",
            stderr="",
        )

    return fake_run_process


def assert_no_denylisted_keys(testcase: unittest.TestCase, value: object) -> None:
    if isinstance(value, dict):
        for key, nested in value.items():
            testcase.assertNotIn(key.lower(), DENYLISTED_KEYS)
            assert_no_denylisted_keys(testcase, nested)
    elif isinstance(value, list):
        for item in value:
            assert_no_denylisted_keys(testcase, item)


def checks_by_name(report: dict[str, object]) -> dict[str, dict[str, object]]:
    checks = report["checks"]
    if not isinstance(checks, list):
        raise AssertionError("report checks must be a list")
    result = {}
    for check in checks:
        if not isinstance(check, dict):
            raise AssertionError("report check entries must be objects")
        result[str(check["name"])] = check
    return result


class SmokeLauncherInjectedPlannerPresenceTests(unittest.TestCase):
    def test_tool_path_exists(self) -> None:
        self.assertTrue(SMOKE_PATH.exists())


class SmokeLauncherInjectedPlannerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_cli_command_with_explicit_production_backend_and_fixture(self) -> None:
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
        self.assertNotIn("--planner-backend python", " ".join(command))
        self.assertNotIn("--rust-probe-adb-getprop", command)
        self.assertNotIn("--rust-adb-path", command)
        self.assertNotIn("--rust-serial", command)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb", command)

    def test_path_validation_rejects_relative_missing_directories_and_non_executable_files(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            missing = temp_root / "missing-plan-shadow"
            directory = temp_root / "planner-dir"
            directory.mkdir()
            non_executable = temp_root / "emuchef-plan-shadow"
            non_executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            non_executable.chmod(0o600)
            executable = executable_file(temp_root / "good-plan-shadow")

            relative = self.smoke.validate_launcher_path("relative/emuchef-plan-shadow")
            missing_result = self.smoke.validate_launcher_path(str(missing))
            directory_result = self.smoke.validate_launcher_path(str(directory))
            non_executable_result = self.smoke.validate_launcher_path(str(non_executable))
            executable_result = self.smoke.validate_launcher_path(str(executable))

        self.assertFalse(relative.path_was_absolute)
        self.assertFalse(relative.path_exists)
        self.assertFalse(relative.path_is_file)
        self.assertFalse(relative.path_executable)

        self.assertTrue(missing_result.path_was_absolute)
        self.assertFalse(missing_result.path_exists)
        self.assertFalse(missing_result.path_is_file)
        self.assertFalse(missing_result.path_executable)

        self.assertTrue(directory_result.path_was_absolute)
        self.assertTrue(directory_result.path_exists)
        self.assertFalse(directory_result.path_is_file)
        self.assertFalse(directory_result.path_executable)

        self.assertTrue(non_executable_result.path_was_absolute)
        self.assertTrue(non_executable_result.path_exists)
        self.assertTrue(non_executable_result.path_is_file)
        self.assertFalse(non_executable_result.path_executable)

        self.assertTrue(executable_result.path_was_absolute)
        self.assertTrue(executable_result.path_exists)
        self.assertTrue(executable_result.path_is_file)
        self.assertTrue(executable_result.path_executable)

    def test_success_report_has_required_shape_checks_and_redacted_wrapper_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            launcher_wrapper = executable_file(temp_root / "emuchef-plan-shadow")
            with patch.object(self.smoke, "run_process", side_effect=observed_success(launcher_wrapper)):
                report = self.smoke.run_smoke_report(
                    authored_root=f"{temp_dir}/authored",
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin=str(launcher_wrapper),
                    repo_root=REPO_ROOT,
                    generated_at="2026-06-21T00:00:00Z",
                )

        serialized = self.smoke.dumps_report(report)
        check_map = checks_by_name(report)

        self.assertEqual(report["kind"], "rust_launcher_injected_planner_smoke")
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(
            set(report),
            {"kind", "schema_version", "generated_at", "summary", "inputs", "checks", "redaction", "artifacts"},
        )
        self.assertEqual(report["summary"], {"passed": 7, "failed": 0})
        self.assertEqual(
            set(check_map),
            {
                "launcher_supplied_path_absolute",
                "launcher_supplied_path_exists",
                "launcher_supplied_path_file",
                "launcher_supplied_path_executable",
                "argv0_corresponds_to_launcher_path",
                "known_fixture_plan_succeeded",
                "no_implicit_fallback_sources_used",
            },
        )
        self.assertTrue(all(check["passed"] is True for check in check_map.values()))
        self.assertEqual(report["inputs"]["planner_backend"], "rust-production-equivalent")
        self.assertEqual(report["inputs"]["path_was_absolute"], True)
        self.assertEqual(report["inputs"]["path_exists"], True)
        self.assertEqual(report["inputs"]["path_executable"], True)
        self.assertEqual(report["artifacts"]["argv0_basename"], "emuchef-plan-shadow")
        self.assertEqual(self.smoke.smoke_exit_code(report), 0)
        assert_no_denylisted_keys(self, report)
        self.assertNotIn(temp_dir, serialized)
        self.assertNotIn(str(REPO_ROOT), serialized)
        self.assertNotIn("raw", serialized.lower())
        self.assertNotIn("stdout", serialized.lower())
        self.assertNotIn("stderr", serialized.lower())
        self.assertNotIn("command", serialized.lower())
        self.assertNotIn("argv", serialized.lower().replace("argv0_basename", "").replace("argv0_corresponds", ""))
        self.assertNotIn("environment", serialized.lower())
        self.assertNotIn("serial", serialized.lower())

    def test_validation_failure_emits_report_without_running_planner_or_leaking_path(self) -> None:
        with patch.object(self.smoke, "run_process") as run_process:
            report = self.smoke.run_smoke_report(
                authored_root="/Users/example/Projects/EmuChef/authored",
                device_plan="ayaneo.pocket_s_mini.base",
                rust_planner_bin="relative/emuchef-plan-shadow",
                repo_root=REPO_ROOT,
                generated_at="2026-06-21T00:00:00Z",
            )

        serialized = self.smoke.dumps_report(report)
        check_map = checks_by_name(report)

        run_process.assert_not_called()
        self.assertEqual(report["kind"], "rust_launcher_injected_planner_smoke")
        self.assertEqual(self.smoke.smoke_exit_code(report), 1)
        self.assertFalse(check_map["launcher_supplied_path_absolute"]["passed"])
        self.assertFalse(check_map["known_fixture_plan_succeeded"]["passed"])
        self.assertFalse(check_map["argv0_corresponds_to_launcher_path"]["passed"])
        assert_no_denylisted_keys(self, report)
        self.assertNotIn("relative/emuchef-plan-shadow", serialized)
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn("command", serialized.lower())
        self.assertNotIn("stdout", serialized.lower())
        self.assertNotIn("stderr", serialized.lower())

    def test_planner_failure_report_omits_raw_output_and_command_values(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            launcher_wrapper = executable_file(temp_root / "emuchef-plan-shadow")

            def fake_failure(command, *, cwd, observation_path):
                observation_path.write_text(json.dumps({"argv0": str(launcher_wrapper)}), encoding="utf-8")
                return subprocess.CompletedProcess(
                    args=["do-not-report"],
                    returncode=2,
                    stdout=f"raw stdout with {temp_dir}\n",
                    stderr=f"raw stderr with {REPO_ROOT}\n",
                )

            with patch.object(self.smoke, "run_process", side_effect=fake_failure):
                report = self.smoke.run_smoke_report(
                    authored_root=f"{temp_dir}/authored",
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin=str(launcher_wrapper),
                    repo_root=REPO_ROOT,
                    generated_at="2026-06-21T00:00:00Z",
                )

        serialized = self.smoke.dumps_report(report)
        check_map = checks_by_name(report)

        self.assertEqual(self.smoke.smoke_exit_code(report), 1)
        self.assertTrue(check_map["argv0_corresponds_to_launcher_path"]["passed"])
        self.assertFalse(check_map["known_fixture_plan_succeeded"]["passed"])
        self.assertTrue(check_map["no_implicit_fallback_sources_used"]["passed"])
        assert_no_denylisted_keys(self, report)
        self.assertNotIn("do-not-report", serialized)
        self.assertNotIn(temp_dir, serialized)
        self.assertNotIn(str(REPO_ROOT), serialized)
        self.assertNotIn("raw stdout", serialized)
        self.assertNotIn("raw stderr", serialized)
        self.assertNotIn("stdout", serialized.lower())
        self.assertNotIn("stderr", serialized.lower())
        self.assertNotIn("command", serialized.lower())

    def test_observed_argv0_mismatch_fails_boolean_only_without_reporting_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            launcher_wrapper = executable_file(temp_root / "emuchef-plan-shadow")
            other_wrapper = executable_file(temp_root / "other-plan-shadow")

            def fake_mismatch(command, *, cwd, observation_path):
                observation_path.write_text(json.dumps({"argv0": str(other_wrapper)}), encoding="utf-8")
                return subprocess.CompletedProcess(args=[], returncode=0, stdout="Planning status: success\n", stderr="")

            with patch.object(self.smoke, "run_process", side_effect=fake_mismatch):
                report = self.smoke.run_smoke_report(
                    authored_root=f"{temp_dir}/authored",
                    device_plan="ayaneo.pocket_s_mini.base",
                    rust_planner_bin=str(launcher_wrapper),
                    repo_root=REPO_ROOT,
                    generated_at="2026-06-21T00:00:00Z",
                )

        serialized = self.smoke.dumps_report(report)
        check_map = checks_by_name(report)
        argv0_check = check_map["argv0_corresponds_to_launcher_path"]

        self.assertIs(argv0_check["passed"], False)
        self.assertEqual(report["inputs"]["argv0_corresponds_to_launcher_path"], False)
        self.assertIsInstance(report["inputs"]["argv0_corresponds_to_launcher_path"], bool)
        self.assertEqual(report["artifacts"]["argv0_basename"], "other-plan-shadow")
        self.assertEqual(self.smoke.smoke_exit_code(report), 1)
        assert_no_denylisted_keys(self, report)
        self.assertNotIn(str(launcher_wrapper), serialized)
        self.assertNotIn(str(other_wrapper), serialized)
        self.assertNotIn(temp_dir, serialized)

    def test_main_writes_report_file_and_returns_nonzero_for_failed_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_report = Path(temp_dir) / "report.json"
            with patch.object(self.smoke, "run_process") as run_process:
                exit_code = self.smoke.main(
                    [
                        "--authored-root",
                        f"{temp_dir}/authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                        "--rust-planner-bin",
                        "relative/emuchef-plan-shadow",
                        "--output-report",
                        str(output_report),
                    ]
                )

            payload = json.loads(output_report.read_text(encoding="utf-8"))

        self.assertEqual(exit_code, 1)
        self.assertEqual(payload["kind"], "rust_launcher_injected_planner_smoke")
        run_process.assert_not_called()
        assert_no_denylisted_keys(self, payload)

    def test_source_is_stdlib_only_and_not_using_forbidden_runtime_areas(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")
        tree = ast.parse(source)
        allowed_modules = {
            "__future__",
            "argparse",
            "dataclasses",
            "datetime",
            "json",
            "os",
            "pathlib",
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
            "--planner-backend python",
            "--rust-probe-adb-getprop",
            "--rust-adb-path",
            "--rust-serial",
            "--probe-adb-getprop",
            "--adb-path",
            "--serial",
            ".expanduser(",
            ".resolve(",
            "tools/check_rust_planner_cutover_readiness.py",
            "tests/test_check_rust_planner_cutover_readiness.py",
            "crates/emuchef-rust-backend/src",
            "apps/config-editor",
            ".local",
        ]
        for token in forbidden_substrings:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

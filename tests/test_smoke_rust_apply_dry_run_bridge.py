from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_apply_dry_run_bridge.py"
APPLY_DRY_RUN_FIXTURE = REPO_ROOT / "tests" / "fixtures" / "apply_dry_run" / "minimal_execution_plan.yaml"
ARGV_CAPTURE_ENV = "EMUCHEF_P8BU_ARGV_PATH"


def import_smoke_module():
    module_name = "smoke_rust_apply_dry_run_bridge"
    spec = importlib.util.spec_from_file_location(module_name, SMOKE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {SMOKE_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def write_executable(path: Path, body: str) -> Path:
    path.write_text(body, encoding="utf-8")
    path.chmod(0o700)
    return path


def check_by_id(report: dict[str, object], check_id: str) -> dict[str, object]:
    checks = report.get("checks")
    if not isinstance(checks, list):
        raise AssertionError("report checks must be a list")
    matches = [check for check in checks if isinstance(check, dict) and check.get("id") == check_id]
    if len(matches) != 1:
        raise AssertionError(f"Expected one check with id {check_id!r}, found {len(matches)}")
    return matches[0]


class SmokeRustApplyDryRunBridgeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def run_smoke(self, *, rust_apply_bin: Path, plan_file: Path, output_report: Path) -> tuple[int, dict]:
        rc = self.smoke.main(
            [
                "--rust-apply-bin",
                str(rust_apply_bin),
                "--plan-file",
                str(plan_file),
                "--output-report",
                str(output_report),
            ]
        )
        return rc, json.loads(output_report.read_text(encoding="utf-8"))

    def test_missing_rust_apply_binary_path_produces_failed_report_and_nonzero_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            missing_bin = temp_root / "missing-emuchef-rust-backend"
            plan_file = temp_root / "plan.yaml"
            plan_file.write_text("kind: execution_plan\n", encoding="utf-8")
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=missing_bin,
                plan_file=plan_file,
                output_report=output_report,
            )

        self.assertNotEqual(rc, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIsNone(report["result"]["returncode"])
        self.assertEqual(check_by_id(report, "rust_apply_bin_exists")["status"], "fail")
        self.assertIn("reason", check_by_id(report, "rust_apply_bin_exists"))
        self.assertEqual(check_by_id(report, "python_bridge_invocation_succeeded")["status"], "fail")

    def test_directory_rust_apply_binary_path_produces_failed_report_and_nonzero_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            directory_bin = temp_root / "emuchef-rust-backend-dir"
            directory_bin.mkdir()
            plan_file = temp_root / "plan.yaml"
            plan_file.write_text("kind: execution_plan\n", encoding="utf-8")
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=directory_bin,
                plan_file=plan_file,
                output_report=output_report,
            )

        self.assertNotEqual(rc, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIsNone(report["result"]["returncode"])
        self.assertEqual(check_by_id(report, "rust_apply_bin_is_file")["status"], "fail")
        self.assertIn("reason", check_by_id(report, "rust_apply_bin_is_file"))

    def test_non_executable_rust_apply_binary_path_produces_failed_report_and_nonzero_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            non_executable_bin = temp_root / "emuchef-rust-backend"
            non_executable_bin.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            non_executable_bin.chmod(0o600)
            plan_file = temp_root / "plan.yaml"
            plan_file.write_text("kind: execution_plan\n", encoding="utf-8")
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=non_executable_bin,
                plan_file=plan_file,
                output_report=output_report,
            )

        self.assertNotEqual(rc, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIsNone(report["result"]["returncode"])
        self.assertEqual(check_by_id(report, "rust_apply_bin_executable")["status"], "fail")
        self.assertIn("reason", check_by_id(report, "rust_apply_bin_executable"))

    def test_missing_plan_file_produces_failed_report_and_nonzero_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            rust_apply_bin = write_executable(temp_root / "emuchef-rust-backend", "#!/bin/sh\nexit 0\n")
            missing_plan = temp_root / "missing-plan.yaml"
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=rust_apply_bin,
                plan_file=missing_plan,
                output_report=output_report,
            )

        self.assertNotEqual(rc, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIsNone(report["result"]["returncode"])
        self.assertEqual(check_by_id(report, "plan_file_exists")["status"], "fail")
        self.assertIn("reason", check_by_id(report, "plan_file_exists"))

    def test_successful_invocation_uses_python_cli_bridge_and_writes_passed_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            argv_path = temp_root / "argv.txt"
            rust_apply_bin = write_executable(
                temp_root / "emuchef-rust-backend",
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$EMUCHEF_P8BU_ARGV_PATH\"\nprintf 'rust dry run ok\\n'\nexit 0\n",
            )
            plan_file = temp_root / "plan.yaml"
            plan_file.write_text("kind: execution_plan\n", encoding="utf-8")
            output_report = temp_root / "report.json"

            with patch.dict(os.environ, {ARGV_CAPTURE_ENV: str(argv_path)}):
                rc, report = self.run_smoke(
                    rust_apply_bin=rust_apply_bin,
                    plan_file=plan_file,
                    output_report=output_report,
                )
            observed_argv = argv_path.read_text(encoding="utf-8").splitlines()

        self.assertEqual(rc, 0)
        self.assertEqual(report["kind"], "rust_apply_dry_run_bridge_smoke")
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["inputs"]["rust_apply_bin"], str(rust_apply_bin.resolve()))
        self.assertEqual(report["inputs"]["plan_file"], str(plan_file.resolve()))
        self.assertEqual(observed_argv, ["apply", "--plan-file", str(plan_file.resolve()), "--dry-run"])
        self.assertEqual(report["command"][:2], ["emuchef", "apply"])
        self.assertIn("--plan-file", report["command"])
        self.assertIn(str(plan_file.resolve()), report["command"])
        self.assertIn("--dry-run", report["command"])
        self.assertIn("--rust-apply-bin", report["command"])
        self.assertIn(str(rust_apply_bin.resolve()), report["command"])
        self.assertEqual(report["result"]["returncode"], 0)
        self.assertTrue(report["result"]["stdout_present"])
        self.assertFalse(report["result"]["stderr_present"])
        self.assertTrue(all(check["status"] == "pass" for check in report["checks"]))

    def test_checked_in_fixture_works_as_smoke_plan_file_input(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            rust_apply_bin = write_executable(
                temp_root / "emuchef-rust-backend",
                "#!/bin/sh\nprintf 'fixture dry run ok\\n'\nexit 0\n",
            )
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=rust_apply_bin,
                plan_file=APPLY_DRY_RUN_FIXTURE,
                output_report=output_report,
            )

        self.assertTrue(APPLY_DRY_RUN_FIXTURE.exists())
        self.assertEqual(rc, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["result"]["returncode"], 0)
        self.assertIn("apply", report["command"])
        self.assertIn("--plan-file", report["command"])
        self.assertIn("--dry-run", report["command"])
        self.assertIn("--rust-apply-bin", report["command"])
        self.assertIn(
            "tests/fixtures/apply_dry_run/minimal_execution_plan.yaml",
            report["inputs"]["plan_file"],
        )

    def test_failed_bridge_invocation_preserves_return_code_and_writes_failed_report(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            rust_apply_bin = write_executable(
                temp_root / "emuchef-rust-backend",
                "#!/bin/sh\nprintf 'rust dry run failed\\n' >&2\nexit 2\n",
            )
            plan_file = temp_root / "plan.yaml"
            plan_file.write_text("kind: execution_plan\n", encoding="utf-8")
            output_report = temp_root / "report.json"

            rc, report = self.run_smoke(
                rust_apply_bin=rust_apply_bin,
                plan_file=plan_file,
                output_report=output_report,
            )

        self.assertEqual(rc, 2)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["result"]["returncode"], 2)
        self.assertFalse(report["result"]["stdout_present"])
        self.assertTrue(report["result"]["stderr_present"])
        invocation_check = check_by_id(report, "python_bridge_invocation_succeeded")
        self.assertEqual(invocation_check["status"], "fail")
        self.assertIn("reason", invocation_check)


if __name__ == "__main__":
    unittest.main()

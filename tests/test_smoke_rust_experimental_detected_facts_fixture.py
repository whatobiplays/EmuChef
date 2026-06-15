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
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_experimental_detected_facts_fixture.py"


def import_smoke_module():
    module_name = "smoke_rust_experimental_detected_facts_fixture"
    spec = importlib.util.spec_from_file_location(
        module_name,
        SMOKE_PATH,
    )
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


class SmokeRustExperimentalDetectedFactsFixturePresenceTests(unittest.TestCase):
    def test_tool_path_exists(self) -> None:
        self.assertTrue(SMOKE_PATH.exists())


@unittest.skipUnless(SMOKE_PATH.exists(), "smoke tool is not implemented yet")
class SmokeRustExperimentalDetectedFactsFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_python_route_command_with_rust_experimental_fixture_flag(self) -> None:
        command = self.smoke.build_cli_command(
            python_executable="python3",
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            fixture_path=Path("/tmp/facts.json"),
        )

        self.assertEqual(
            command[:4],
            ["python3", "-m", "emuchef", "plan"],
        )
        self.assertEqual(command[command.index("--planner-backend") + 1], "rust-experimental")
        self.assertEqual(command[command.index("--rust-planner-bin") + 1], "/tmp/emuchef-plan-shadow")
        self.assertEqual(command[command.index("--rust-detected-facts-json") + 1], "/tmp/facts.json")
        self.assertEqual(command[command.index("--authored-root") + 1], "authored")
        self.assertEqual(command[command.index("--device-plan") + 1], "ayaneo.pocket_s_mini.base")
        self.assertNotIn("--detected-facts-json", command)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb", command)
        self.assertNotIn("apply", command)
        self.assertNotIn("--sidecar", command)
        self.assertNotIn("tauri", " ".join(command).lower())
        self.assertNotIn("protocol", " ".join(command).lower())
        self.assertNotIn("PYTHONPATH=src", command)

    def test_builds_mismatch_command_with_temp_output_path_when_requested(self) -> None:
        command = self.smoke.build_cli_command(
            python_executable="/venv/bin/python",
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            fixture_path=Path("/tmp/facts.json"),
            output_path=Path("/tmp/planning-result.yaml"),
        )

        self.assertEqual(command[:4], ["/venv/bin/python", "-m", "emuchef", "plan"])
        self.assertEqual(command[command.index("--output") + 1], "/tmp/planning-result.yaml")
        self.assertIn("--rust-detected-facts-json", command)
        self.assertNotIn("--detected-facts-json", command)
        self.assertNotIn("PYTHONPATH=src", command)

    def test_matching_fixture_case_passes_with_python_compatible_summary(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[0]
            with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    temp_root=Path(temp_dir),
                    repo_root=REPO_ROOT,
                )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.stdout_class, "python_summary")
        self.assertEqual(result.exit_class, "success")
        self.assertFalse(result.raw_rust_json_seen)
        self.assertFalse(result.warning_observed)
        self.assertEqual(run_process.call_args.args[0][:4], ["python3", "-m", "emuchef", "plan"])

    def test_raw_rust_json_stdout_fails_for_python_route_smoke(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout='{"kind":"planning_result","status":"success"}\n',
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[0]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    temp_root=Path(temp_dir),
                    repo_root=REPO_ROOT,
                )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.stdout_class, "stdout_json")
        self.assertTrue(result.raw_rust_json_seen)

    def test_mismatching_fixture_warning_route_passes_with_output_file_validation(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout=python_summary(status="warning", warning_code="device_profile_mismatch"),
            stderr="",
        )

        def fake_run_process(command, *, cwd):
            output_path = Path(command[command.index("--output") + 1])
            output_path.write_text(
                "kind: planning_result\nstatus: warning\nwarnings:\n- code: device_profile_mismatch\n",
                encoding="utf-8",
            )
            return completed

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[1]
            with patch.object(self.smoke, "run_process", side_effect=fake_run_process) as run_process:
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    temp_root=Path(temp_dir),
                    repo_root=REPO_ROOT,
                )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.stdout_class, "python_summary")
        self.assertEqual(result.exit_class, "warning")
        self.assertTrue(result.warning_observed)
        self.assertTrue(result.output_file_written)
        self.assertTrue(result.output_warning_observed)
        command = run_process.call_args.args[0]
        self.assertIn("--output", command)
        self.assertIn("--rust-detected-facts-json", command)
        self.assertNotIn("--detected-facts-json", command)

    def test_warning_exit_behavior_follows_current_route_semantics(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=python_summary(status="warning", warning_code="device_profile_mismatch"),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[1]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    temp_root=Path(temp_dir),
                    repo_root=REPO_ROOT,
                )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "unexpected_exit")

    def test_report_omits_temp_paths_absolute_inputs_and_volatile_output(self) -> None:
        result = self.smoke.CaseResult(
            id="failing",
            status="fail",
            expected_warning_code="device_profile_mismatch",
            warning_observed=False,
            stdout_class="stdout_text",
            exit_class="warning",
            stderr_class="stderr_text",
            raw_rust_json_seen=False,
            output_file_written=True,
            output_warning_observed=False,
            failure_summary="stdout: full volatile output from /tmp/example",
        )
        report = self.smoke.build_report(
            authored_root="/Users/example/Projects/EmuChef/authored",
            rust_planner_bin="/Users/example/target/debug/emuchef-plan-shadow",
            python_executable="/Users/example/.venv/bin/python3",
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["python_executable"], "python3")
        self.assertEqual(report["inputs"]["route_backend"], "rust-experimental")
        self.assertEqual(report["inputs"]["route_output_mode"], "python-compatible")
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn(tempfile.gettempdir(), serialized)
        self.assertNotIn("full volatile output", serialized)
        self.assertNotIn("PYTHONPATH", serialized)

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        results = [
            self.smoke.CaseResult(
                id="matching_detected_facts_route",
                status="pass",
                expected_warning_code=None,
                warning_observed=False,
                stdout_class="python_summary",
                exit_class="success",
                stderr_class="stderr_empty",
                raw_rust_json_seen=False,
                output_file_written=None,
                output_warning_observed=None,
            )
        ]

        first = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                python_executable="python3",
                case_results=results,
            )
        )
        second = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                python_executable="python3",
                case_results=results,
            )
        )

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first)["summary"], {"passed": 1, "failed": 0})

    def test_any_case_failure_returns_nonzero(self) -> None:
        report = self.smoke.build_report(
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            python_executable="python3",
            case_results=[
                self.smoke.CaseResult(
                    id="failing",
                    status="fail",
                    expected_warning_code=None,
                    warning_observed=False,
                    stdout_class="stdout_text",
                    exit_class="success",
                    stderr_class="stderr_empty",
                    raw_rust_json_seen=False,
                    output_file_written=None,
                    output_warning_observed=None,
                )
            ],
        )

        self.assertEqual(self.smoke.smoke_exit_code(report), 1)

    def test_source_has_no_forbidden_top_level_imports_or_harness_reuse(self) -> None:
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
            r"^\s*import\s+crates\b",
            r"^\s*from\s+crates\b",
            r"^\s*import\s+tauri\b",
            r"^\s*from\s+tauri\b",
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))


if __name__ == "__main__":
    unittest.main()

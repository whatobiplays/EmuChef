from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_shadow_cli_matrix.py"


def import_smoke_module():
    spec = importlib.util.spec_from_file_location("smoke_rust_shadow_cli_matrix", SMOKE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"Could not load module spec for {SMOKE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def scenario_payload(*scenarios: dict) -> dict:
    return {
        "schema_version": 1,
        "scenarios": list(scenarios),
    }


class SmokeRustShadowCliMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_loads_checked_in_scenario_matrix_without_running_processes(self) -> None:
        with patch.object(self.smoke, "run_process") as run_process:
            matrix = self.smoke.load_scenario_matrix(REPO_ROOT / "tools" / "plan_parity_scenarios.json")

        self.assertEqual(matrix.schema_version, 1)
        self.assertEqual(
            [scenario.id for scenario in matrix.scenarios],
            [
                "ayaneo_konkr_pocket_fit_base",
                "ayaneo_pocket_s_mini_base",
                "ayaneo_generic_base",
                "ayaneo_pocket_air_mini_base",
                "ayaneo_pocket_s2_base",
            ],
        )
        run_process.assert_not_called()

    def test_module_docs_describe_both_explicit_rust_routes(self) -> None:
        module_docs = self.smoke.__doc__ or ""

        self.assertIn("rust-shadow", module_docs)
        self.assertIn("rust-experimental", module_docs)
        self.assertIn("Python-compatible", module_docs)

    def test_builds_cli_command_for_scenario_without_bindings(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="no_bindings",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(),
        )

        command = self.smoke.build_cli_command(
            python_executable="/venv/bin/python",
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            scenario=scenario,
            raw_binds=[],
        )

        self.assertEqual(
            command,
            [
                "/venv/bin/python",
                "-m",
                "emuchef",
                "plan",
                "--planner-backend",
                "rust-shadow",
                "--rust-planner-bin",
                "/tmp/emuchef-plan-shadow",
                "--authored-root",
                "authored",
                "--device-plan",
                "example.plan",
            ],
        )
        self.assertNotIn("--rust-shadow-output", command)

    def test_builds_cli_command_for_python_compatible_output_mode(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="compatible",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(),
        )

        command = self.smoke.build_cli_command(
            python_executable="/venv/bin/python",
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            scenario=scenario,
            raw_binds=[],
            route_output_mode="python-compatible",
        )

        self.assertIn("--rust-shadow-output", command)
        self.assertEqual(
            command[command.index("--rust-shadow-output") + 1],
            "python-compatible",
        )

    def test_builds_cli_command_for_rust_experimental_backend(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="experimental",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(),
        )

        command = self.smoke.build_cli_command(
            python_executable="/venv/bin/python",
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            scenario=scenario,
            raw_binds=[],
            route_backend="rust-experimental",
        )

        self.assertEqual(
            command,
            [
                "/venv/bin/python",
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
                "example.plan",
            ],
        )
        self.assertNotIn("--rust-shadow-output", command)

    def test_constructs_directory_binding_placeholder_for_cli_only(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="directory_binding",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(
                self.smoke.PlanParityBindingSpec(
                    ref="feature.copy_bios/bios_source_dir",
                    kind="directory",
                    suffix=None,
                ),
            ),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            prepared = self.smoke.prepare_scenario_bindings(scenario, Path(temp_dir))
            bind_value = prepared.raw_cli_binds[0].split("=", 1)[1]

            self.assertTrue(Path(bind_value).is_dir())
            self.assertEqual(
                prepared.report_bindings,
                [{"ref": "feature.copy_bios/bios_source_dir", "kind": "directory"}],
            )
            self.assertNotIn(temp_dir, json.dumps(prepared.report_bindings))

    def test_constructs_apk_file_binding_placeholder_for_cli_only(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="apk_binding",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(
                self.smoke.PlanParityBindingSpec(
                    ref="app.xaniteog.install/xaniteog_apk",
                    kind="file",
                    suffix=".apk",
                ),
            ),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            prepared = self.smoke.prepare_scenario_bindings(scenario, Path(temp_dir))
            bind_value = prepared.raw_cli_binds[0].split("=", 1)[1]

            self.assertTrue(Path(bind_value).is_file())
            self.assertEqual(Path(bind_value).suffix, ".apk")
            self.assertEqual(
                prepared.report_bindings,
                [{"ref": "app.xaniteog.install/xaniteog_apk", "kind": "file", "suffix": ".apk"}],
            )
            self.assertNotIn(temp_dir, json.dumps(prepared.report_bindings))

    def test_repeated_bind_order_is_preserved(self) -> None:
        scenario = self.smoke.PlanParityScenario(
            id="ordered_bindings",
            device_plan="example.plan",
            expected_route_exit_code=0,
            bindings=(
                self.smoke.PlanParityBindingSpec("feature.copy_bios/bios_source_dir", "directory", None),
                self.smoke.PlanParityBindingSpec("app.xaniteog.install/xaniteog_apk", "file", ".apk"),
                self.smoke.PlanParityBindingSpec("app.retroarch.provision/retroarch_cfg", "file", ".cfg"),
            ),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            prepared = self.smoke.prepare_scenario_bindings(scenario, Path(temp_dir))
            command = self.smoke.build_cli_command(
                python_executable="python3",
                authored_root="authored",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                scenario=scenario,
                raw_binds=prepared.raw_cli_binds,
            )

        bind_args = [command[index + 1] for index, value in enumerate(command) if value == "--bind"]
        self.assertEqual(
            [raw.split("=", 1)[0] for raw in bind_args],
            [
                "feature.copy_bios/bios_source_dir",
                "app.xaniteog.install/xaniteog_apk",
                "app.retroarch.provision/retroarch_cfg",
            ],
        )

    def test_report_aggregation_is_deterministic_and_omits_noisy_paths(self) -> None:
        matrix = self.smoke.PlanParityScenarioMatrix(
            schema_version=1,
            scenarios=(
                self.smoke.PlanParityScenario("passing", "pass.plan", 0, ()),
                self.smoke.PlanParityScenario("failing", "fail.plan", 0, ()),
            ),
        )
        results = [
            self.smoke.ScenarioSmokeResult(
                scenario=matrix.scenarios[0],
                report_bindings=[],
                expected_route_exit_code=0,
                actual_exit_code=0,
                stdout_classification="stdout_json",
                expected_stdout_class="not_enforced",
                stderr_classification="stderr_empty",
                command_classification="rust_shadow_passthrough_implicit",
            ),
            self.smoke.ScenarioSmokeResult(
                scenario=matrix.scenarios[1],
                report_bindings=[],
                expected_route_exit_code=0,
                actual_exit_code=2,
                stdout_classification="stdout_empty",
                expected_stdout_class="not_enforced",
                stderr_classification="stderr_text",
                command_classification="rust_shadow_passthrough_implicit",
                failure_summary="exit code 2; stderr: failed",
            ),
        ]

        first = self.smoke.dumps_report(
            self.smoke.build_report(
                scenario_matrix="tools/plan_parity_scenarios.json",
                authored_root="authored",
                rust_planner_bin="/abs/path/to/emuchef-plan-shadow",
                python_executable="/abs/path/to/python3",
                route_backend="rust-shadow",
                route_output_mode="passthrough",
                matrix=matrix,
                scenario_results=results,
            )
        )
        second = self.smoke.dumps_report(
            self.smoke.build_report(
                scenario_matrix="tools/plan_parity_scenarios.json",
                authored_root="authored",
                rust_planner_bin="/abs/path/to/emuchef-plan-shadow",
                python_executable="/abs/path/to/python3",
                route_backend="rust-shadow",
                route_output_mode="passthrough",
                matrix=matrix,
                scenario_results=results,
            )
        )
        report = json.loads(first)

        self.assertEqual(first, second)
        self.assertEqual(report["kind"], "rust_shadow_cli_matrix_smoke_report")
        self.assertEqual(report["summary"], {"total_scenarios": 2, "pass_count": 1, "fail_count": 1})
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["python_executable"], "python3")
        self.assertEqual(report["inputs"]["route_output_mode"], "passthrough")
        self.assertEqual(report["scenarios"][0]["expected_stdout_class"], "not_enforced")
        self.assertEqual(report["scenarios"][0]["command_classification"], "rust_shadow_passthrough_implicit")
        self.assertNotIn("/abs/path", first)
        self.assertNotIn(tempfile.gettempdir(), first)

    def test_run_matrix_exits_zero_when_all_synthetic_runs_pass(self) -> None:
        payload = scenario_payload(
            {"id": "one", "device_plan": "one.plan", "bindings": []},
            {"id": "two", "device_plan": "two.plan", "bindings": []},
        )
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout='{"kind":"planning_result"}\n',
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            matrix_path = Path(temp_dir) / "matrix.json"
            matrix_path.write_text(json.dumps(payload), encoding="utf-8")
            with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
                report = self.smoke.run_scenario_matrix_report(
                    scenario_matrix=matrix_path,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    repo_root=Path("/repo"),
                )

        self.assertEqual(self.smoke.matrix_exit_code(report), 0)
        self.assertEqual(report["summary"]["pass_count"], 2)
        self.assertEqual(report["inputs"]["route_output_mode"], "passthrough")
        self.assertEqual(run_process.call_count, 2)
        for call in run_process.call_args_list:
            argv = call.args[0]
            self.assertNotIn("cargo", argv)
            self.assertNotIn("compare_rust_python_plan", " ".join(argv))
            self.assertNotIn("__python-planner-worker", argv)
            self.assertNotIn("--rust-shadow-output", argv)

    def test_python_compatible_matrix_requires_summary_stdout_for_success(self) -> None:
        payload = scenario_payload(
            {"id": "one", "device_plan": "one.plan", "bindings": []},
            {"id": "two", "device_plan": "two.plan", "bindings": []},
        )
        completed = [
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="Planning status: success\nExecution plan: plan.shadow.one\n",
                stderr="",
            ),
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{"kind":"planning_result","status":"success"}\n',
                stderr="",
            ),
        ]

        with tempfile.TemporaryDirectory() as temp_dir:
            matrix_path = Path(temp_dir) / "matrix.json"
            matrix_path.write_text(json.dumps(payload), encoding="utf-8")
            with patch.object(self.smoke, "run_process", side_effect=completed) as run_process:
                report = self.smoke.run_scenario_matrix_report(
                    scenario_matrix=matrix_path,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    repo_root=Path("/repo"),
                    route_output_mode="python-compatible",
                )

        self.assertEqual(self.smoke.matrix_exit_code(report), 1)
        self.assertEqual(report["inputs"]["route_output_mode"], "python-compatible")
        self.assertEqual(report["summary"], {"total_scenarios": 2, "pass_count": 1, "fail_count": 1})
        self.assertEqual(report["scenarios"][0]["stdout_classification"], "python_summary")
        self.assertEqual(report["scenarios"][0]["expected_stdout_class"], "python_summary")
        self.assertEqual(report["scenarios"][0]["status"], "pass")
        self.assertEqual(report["scenarios"][1]["stdout_classification"], "stdout_json")
        self.assertEqual(report["scenarios"][1]["expected_stdout_class"], "python_summary")
        self.assertEqual(report["scenarios"][1]["status"], "fail")
        self.assertIn("expected stdout python_summary, actual stdout_json", report["scenarios"][1]["failure_summary"])
        for call in run_process.call_args_list:
            argv = call.args[0]
            self.assertEqual(
                argv[argv.index("--rust-shadow-output") + 1],
                "python-compatible",
            )

    def test_rust_experimental_matrix_uses_implicit_python_compatible_output(self) -> None:
        payload = scenario_payload(
            {"id": "one", "device_plan": "one.plan", "bindings": []},
            {"id": "two", "device_plan": "two.plan", "bindings": []},
        )
        completed = [
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="Planning status: success\nExecution plan: plan.shadow.one\n",
                stderr="",
            ),
            subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{"kind":"planning_result","status":"success"}\n',
                stderr="",
            ),
        ]

        with tempfile.TemporaryDirectory() as temp_dir:
            matrix_path = Path(temp_dir) / "matrix.json"
            matrix_path.write_text(json.dumps(payload), encoding="utf-8")
            with patch.object(self.smoke, "run_process", side_effect=completed) as run_process:
                report = self.smoke.run_scenario_matrix_report(
                    scenario_matrix=matrix_path,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    repo_root=Path("/repo"),
                    route_backend="rust-experimental",
                )

        self.assertEqual(self.smoke.matrix_exit_code(report), 1)
        self.assertEqual(report["inputs"]["route_backend"], "rust-experimental")
        self.assertEqual(report["inputs"]["route_output_mode"], "python-compatible")
        self.assertEqual(report["summary"], {"total_scenarios": 2, "pass_count": 1, "fail_count": 1})
        self.assertEqual(report["scenarios"][0]["stdout_classification"], "python_summary")
        self.assertEqual(report["scenarios"][0]["expected_stdout_class"], "python_summary")
        self.assertEqual(report["scenarios"][0]["command_classification"], "rust_experimental_python_compatible_implicit")
        self.assertEqual(report["scenarios"][1]["stdout_classification"], "stdout_json")
        self.assertEqual(report["scenarios"][1]["expected_stdout_class"], "python_summary")
        self.assertEqual(report["scenarios"][1]["status"], "fail")
        for call in run_process.call_args_list:
            argv = call.args[0]
            self.assertEqual(
                argv[argv.index("--planner-backend") + 1],
                "rust-experimental",
            )
            self.assertNotIn("--rust-shadow-output", argv)

    def test_rust_experimental_matrix_exits_zero_when_all_synthetic_runs_emit_summary(self) -> None:
        payload = scenario_payload(
            {"id": "one", "device_plan": "one.plan", "bindings": []},
            {"id": "two", "device_plan": "two.plan", "bindings": []},
        )
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="Planning status: success\nExecution plan: plan.shadow.synthetic\n",
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            matrix_path = Path(temp_dir) / "matrix.json"
            matrix_path.write_text(json.dumps(payload), encoding="utf-8")
            with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
                report = self.smoke.run_scenario_matrix_report(
                    scenario_matrix=matrix_path,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    repo_root=Path("/repo"),
                    route_backend="rust-experimental",
                )

        self.assertEqual(self.smoke.matrix_exit_code(report), 0)
        self.assertEqual(report["inputs"]["route_backend"], "rust-experimental")
        self.assertEqual(report["inputs"]["route_output_mode"], "python-compatible")
        self.assertEqual(report["summary"], {"total_scenarios": 2, "pass_count": 2, "fail_count": 0})
        for scenario in report["scenarios"]:
            self.assertEqual(scenario["expected_route_exit_code"], 0)
            self.assertEqual(scenario["actual_exit_code"], 0)
            self.assertEqual(scenario["stdout_classification"], "python_summary")
            self.assertEqual(scenario["expected_stdout_class"], "python_summary")
            self.assertEqual(scenario["command_classification"], "rust_experimental_python_compatible_implicit")
            self.assertEqual(scenario["status"], "pass")
        for call in run_process.call_args_list:
            argv = call.args[0]
            self.assertEqual(argv[argv.index("--planner-backend") + 1], "rust-experimental")
            self.assertIn("--rust-planner-bin", argv)
            self.assertNotIn("--rust-shadow-output", argv)

    def test_run_matrix_exits_nonzero_when_any_synthetic_run_fails(self) -> None:
        payload = scenario_payload(
            {"id": "one", "device_plan": "one.plan", "bindings": []},
            {"id": "two", "device_plan": "two.plan", "bindings": []},
        )
        completed = [
            subprocess.CompletedProcess(args=[], returncode=0, stdout='{"kind":"planning_result"}\n', stderr=""),
            subprocess.CompletedProcess(args=[], returncode=2, stdout="", stderr="volatile temp /tmp/example failure\n"),
        ]

        with tempfile.TemporaryDirectory() as temp_dir:
            matrix_path = Path(temp_dir) / "matrix.json"
            matrix_path.write_text(json.dumps(payload), encoding="utf-8")
            with patch.object(self.smoke, "run_process", side_effect=completed):
                report = self.smoke.run_scenario_matrix_report(
                    scenario_matrix=matrix_path,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    python_executable="python3",
                    repo_root=Path("/repo"),
                )

        self.assertEqual(self.smoke.matrix_exit_code(report), 1)
        self.assertEqual(report["summary"]["fail_count"], 1)
        self.assertEqual(report["failures"][0]["scenario_id"], "two")
        self.assertNotIn("/tmp/example", json.dumps(report["failures"]))

    def test_parser_accepts_python_compatible_output_mode(self) -> None:
        parser = self.smoke.build_parser()

        args = parser.parse_args(
            [
                "--scenario-matrix",
                "tools/plan_parity_scenarios.json",
                "--authored-root",
                "authored",
                "--rust-planner-bin",
                "/tmp/emuchef-plan-shadow",
                "--rust-shadow-output",
                "python-compatible",
            ]
        )

        self.assertEqual(args.rust_shadow_output, "python-compatible")

    def test_parser_accepts_rust_experimental_backend(self) -> None:
        parser = self.smoke.build_parser()

        args = parser.parse_args(
            [
                "--scenario-matrix",
                "tools/plan_parity_scenarios.json",
                "--authored-root",
                "authored",
                "--planner-backend",
                "rust-experimental",
                "--rust-planner-bin",
                "/tmp/emuchef-plan-shadow",
            ]
        )

        self.assertEqual(args.planner_backend, "rust-experimental")
        self.assertIsNone(args.rust_shadow_output)

    def test_rust_experimental_rejects_rust_shadow_output_mode(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()

        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = self.smoke.main(
                [
                    "--scenario-matrix",
                    "tools/plan_parity_scenarios.json",
                    "--authored-root",
                    "authored",
                    "--planner-backend",
                    "rust-experimental",
                    "--rust-planner-bin",
                    "/tmp/emuchef-plan-shadow",
                    "--rust-shadow-output",
                    "passthrough",
                ]
            )

        self.assertEqual(exit_code, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("--rust-shadow-output is only valid with --planner-backend rust-shadow", stderr.getvalue())

    def test_python_compatible_yaml_like_stdout_is_classified_but_not_default_success(self) -> None:
        self.assertEqual(
            self.smoke.classify_stdout(
                "kind: planning_result\nstatus: success\nexecution_plan:\n  id: plan.shadow.one\n",
                route_output_mode="python-compatible",
            ),
            "python_yaml",
        )

    def test_missing_rust_planner_bin_arg_uses_stable_error(self) -> None:
        stdout = io.StringIO()
        stderr = io.StringIO()

        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = self.smoke.main(
                [
                    "--scenario-matrix",
                    "tools/plan_parity_scenarios.json",
                    "--authored-root",
                    "authored",
                ]
            )

        self.assertEqual(exit_code, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("--rust-planner-bin is required", stderr.getvalue())

    def test_source_has_no_forbidden_top_level_imports_or_harness_reuse(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")

        forbidden = [
            "import emuchef",
            "from emuchef",
            "import yaml",
            "from yaml",
            "compare_rust_python_plan",
            "__python-planner-worker",
            "cargo",
        ]
        for token in forbidden:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

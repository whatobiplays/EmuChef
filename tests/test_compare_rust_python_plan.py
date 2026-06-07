from __future__ import annotations

import importlib
import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from collections import OrderedDict
from pathlib import Path
from unittest.mock import patch


harness = importlib.import_module("tools.compare_rust_python_plan")


def planning_result(
    *,
    status: str = "success",
    steps: list[dict] | None = None,
    errors: list[dict] | None = None,
    warnings: list[dict] | None = None,
    selected: list[str] | None = None,
    expanded: list[str] | None = None,
    permission_plan: object = harness.MISSING,
) -> dict:
    execution_plan = None
    if status != "error":
        execution_plan = {
            "id": "plan.shadow.example.001",
            "source": {
                "device_profile_ref": "example.profile",
                "device_plan_ref": "example.plan",
                "selected_recipe_refs": selected or ["example.recipe"],
                "expanded_recipe_refs": expanded or ["example.recipe"],
            },
            "steps": steps or [],
            "schema_version": 1,
            "kind": "execution_plan",
        }
        if permission_plan is not harness.MISSING:
            execution_plan["permission_plan"] = permission_plan
    return {
        "status": status,
        "warnings": warnings or [],
        "errors": errors or [],
        "execution_plan": execution_plan,
        "schema_version": 1,
        "kind": "planning_result",
    }


def minimal_report(
    device_plan: str,
    classification: str,
    *,
    counts: dict[str, int] | None = None,
    known_gaps: list[dict] | None = None,
) -> dict:
    full_counts = OrderedDict((name, 0) for name in harness.CLASSIFICATIONS)
    if counts is None:
        full_counts["match"] = 12
        full_counts["intentional_shape_difference"] = 1
    else:
        for name, value in counts.items():
            full_counts[name] = value
    return {
        "kind": "rust_python_planner_parity_report",
        "schema_version": harness.REPORT_SCHEMA_VERSION,
        "inputs": {
            "comparison": "Python planner API vs Rust shadow planner",
            "authored_root": "authored",
            "device_plan": device_plan,
            "binding_keys": [],
        },
        "metadata": {
            "python_worker_mode": "python_planner_worker",
            "rust_command_mode": "cargo_offline",
            "normalizations": list(harness.NORMALIZATIONS),
        },
        "summary": {
            "classification": classification,
            "counts": full_counts,
        },
        "comparisons": [],
        "known_gaps": known_gaps or [],
        "diagnostics": [],
    }


class CompareRustPythonPlanTests(unittest.TestCase):
    def test_imports_without_emuchef_or_yaml(self) -> None:
        self.assertFalse(harness.EMUCHEF_IMPORTED_AT_MODULE_LOAD)

    def test_parse_bindings_groups_repeated_refs_in_order(self) -> None:
        bindings = harness.parse_bindings(
            [
                "recipe.one/input=/tmp/one",
                "recipe.two/input=value=with=equals",
                "recipe.one/input=/tmp/two",
            ]
        )

        self.assertEqual(
            bindings,
            OrderedDict(
                [
                    ("recipe.one/input", ["/tmp/one", "/tmp/two"]),
                    ("recipe.two/input", "value=with=equals"),
                ]
            ),
        )

    def test_parse_bindings_rejects_malformed_refs(self) -> None:
        for raw in [
            "missing-equals",
            "/input=/tmp/value",
            "recipe/=/tmp/value",
            "recipe/input/extra=/tmp/value",
        ]:
            with self.subTest(raw=raw):
                with self.assertRaisesRegex(ValueError, "Expected <recipe_ref>/<input_id>=<value>"):
                    harness.parse_bindings([raw])

    def test_build_rust_command_defaults_to_offline_cargo(self) -> None:
        spec = harness.build_rust_command(
            authored_root="authored",
            device_plan="example.plan",
            binds=["recipe/input=value"],
            rust_bin=None,
            cargo_offline=True,
            repo_root=Path("/repo"),
        )

        self.assertEqual(spec.mode, "cargo_offline")
        self.assertEqual(
            spec.argv,
            [
                "cargo",
                "run",
                "--offline",
                "--quiet",
                "--manifest-path",
                "/repo/crates/emuchef-rust-backend/Cargo.toml",
                "--bin",
                "emuchef-plan-shadow",
                "--",
                "--authored-root",
                "authored",
                "--device-plan",
                "example.plan",
                "--bind",
                "recipe/input=value",
            ],
        )

    def test_build_rust_command_allows_online_cargo_or_prebuilt_binary(self) -> None:
        online = harness.build_rust_command(
            authored_root="authored",
            device_plan="example.plan",
            binds=[],
            rust_bin=None,
            cargo_offline=False,
            repo_root=Path("/repo"),
        )
        prebuilt = harness.build_rust_command(
            authored_root="authored",
            device_plan="example.plan",
            binds=[],
            rust_bin="/tmp/emuchef-plan-shadow",
            cargo_offline=True,
            repo_root=Path("/repo"),
        )

        self.assertEqual(online.mode, "cargo")
        self.assertNotIn("--offline", online.argv)
        self.assertEqual(prebuilt.mode, "prebuilt_binary")
        self.assertEqual(prebuilt.argv[:5], ["/tmp/emuchef-plan-shadow", "--authored-root", "authored", "--device-plan", "example.plan"])

    def test_build_python_worker_command_uses_hidden_worker_mode(self) -> None:
        spec = harness.build_python_worker_command(
            python_executable="/venv/bin/python",
            script_path=Path("/repo/tools/compare_rust_python_plan.py"),
            authored_root="authored",
            device_plan="example.plan",
            binds=["recipe/input=value"],
        )

        self.assertEqual(spec.mode, "python_planner_worker")
        self.assertEqual(
            spec.argv,
            [
                "/venv/bin/python",
                "/repo/tools/compare_rust_python_plan.py",
                "__python-planner-worker",
                "--authored-root",
                "authored",
                "--device-plan",
                "example.plan",
                "--bind",
                "recipe/input=value",
            ],
        )

    def test_matching_results_compare_equal_deterministically(self) -> None:
        step = {
            "id": "example.recipe/step",
            "type": "wait",
            "dependencies": [],
            "params": {"duration_ms": {"value": 1}},
        }
        report = harness.build_report(
            authored_root="authored",
            device_plan="example.plan",
            bindings=OrderedDict([("example.recipe/input", "/tmp/value")]),
            python_result=planning_result(steps=[step]),
            rust_result=planning_result(steps=[step]),
            python_mode="python_planner_worker",
            rust_mode="cargo_offline",
            known_gap_rules=[],
        )
        first = harness.dumps_report(report)
        second = harness.dumps_report(report)

        self.assertEqual(first, second)
        self.assertEqual(report["summary"]["classification"], "match")
        self.assertTrue(all(item["classification"] in {"match", "intentional_shape_difference"} for item in report["comparisons"]))
        self.assertEqual(json.loads(first), report)

    def test_value_and_presence_mismatches_are_classified(self) -> None:
        python = planning_result(
            steps=[
                {
                    "id": "example.recipe/one",
                    "type": "wait",
                    "dependencies": [],
                    "params": {"duration_ms": {"value": 1}},
                }
            ],
            permission_plan={},
        )
        rust = planning_result(
            steps=[
                {
                    "id": "example.recipe/one",
                    "type": "launch_app",
                    "dependencies": [],
                    "params": {"package_name": {"value": "example"}},
                }
            ]
        )

        comparisons = harness.compare_results(python, rust, known_gap_rules=[])
        by_path = {item["path"]: item["classification"] for item in comparisons}

        self.assertEqual(by_path["execution_plan.step_types"], "value_mismatch")
        self.assertEqual(by_path["execution_plan.params"], "value_mismatch")
        self.assertEqual(by_path["execution_plan.permission_plan_present"], "rust_missing")

    def test_known_gap_rules_are_explicit_and_do_not_apply_by_default(self) -> None:
        python = planning_result(selected=["python.only"])
        rust = planning_result(selected=["rust.only"])
        rule = harness.KnownGapRule(
            path="source.selected_recipe_refs",
            classification="known_gap",
            code="fixture_known_gap",
            description="Synthetic test-only known gap.",
        )

        without_rule = harness.compare_results(python, rust, known_gap_rules=[])
        with_rule = harness.compare_results(python, rust, known_gap_rules=[rule])

        self.assertEqual(
            next(item for item in without_rule if item["path"] == "source.selected_recipe_refs")["classification"],
            "value_mismatch",
        )
        self.assertEqual(
            next(item for item in with_rule if item["path"] == "source.selected_recipe_refs")["classification"],
            "known_gap",
        )

    def test_retroarch_app_data_write_divergence_is_reported_as_rust_bug_not_known_gap(self) -> None:
        python = planning_result(
            steps=[
                {"id": "app.retroarch.provision/install_retroarch", "type": "install_apk", "dependencies": [], "params": {}},
                {"id": "feature.copy_bios/copy_bios_dir", "type": "copy_files", "dependencies": [], "params": {}},
                {"id": "app.xaniteog.install/install_xaniteog", "type": "install_apk", "dependencies": [], "params": {}},
            ],
            selected=["app.retroarch.provision", "feature.copy_bios", "app.xaniteog.install"],
            expanded=["app.retroarch.provision", "feature.copy_bios", "app.xaniteog.install"],
        )
        rust = planning_result(
            status="error",
            errors=[
                {
                    "code": "unknown_step_dependency",
                    "message": "Step 'launch_retroarch' depends on unknown or non-emitted step 'copy_assets'.",
                    "details": {
                        "recipe_ref": "app.retroarch.provision",
                        "step_id": "launch_retroarch",
                        "dependency": "copy_assets",
                    },
                }
            ],
        )

        report = harness.build_report(
            authored_root="authored",
            device_plan="ayaneo.pocket_s2.base",
            bindings=OrderedDict(
                [
                    ("feature.copy_bios/bios_source_dir", "/tmp/emuchef-p7n-bios"),
                    ("app.xaniteog.install/xaniteog_apk", "/tmp/emuchef-p7n-xaniteog.apk"),
                ]
            ),
            python_result=python,
            rust_result=rust,
            python_mode="python_planner_worker",
            rust_mode="cargo_offline",
            known_gap_rules=[],
        )

        self.assertEqual(report["summary"]["classification"], "value_mismatch")
        self.assertEqual(report["diagnostics"][0]["classification"], "rust_planner_bug")
        self.assertEqual(report["diagnostics"][0]["category"], "rust_optional_step_pruning_dependency_bug")
        self.assertEqual(report["known_gaps"], [])

    def test_process_failures_without_planning_json_are_unsupported(self) -> None:
        result = harness.parse_process_planning_result(
            side="rust",
            process=harness.ProcessResult(exit_code=101, stdout="", stderr="cargo failed"),
        )

        self.assertEqual(result.result, None)
        self.assertEqual(result.issue["classification"], "unsupported")
        self.assertEqual(result.issue["side"], "rust")

    def test_scenario_matrix_payload_preserves_ordered_binding_specs(self) -> None:
        matrix = harness.parse_scenario_matrix_payload(
            {
                "schema_version": 1,
                "scenarios": [
                    {
                        "id": "ayaneo_pocket_s2_base",
                        "device_plan": "ayaneo.pocket_s2.base",
                        "expected_classification": "match",
                        "known_gap_ids": [],
                        "notes": "Current checked-in scenario is expected to match under planner-only bindings.",
                        "bindings": [
                            {
                                "ref": "feature.copy_bios/bios_source_dir",
                                "kind": "directory",
                            },
                            {
                                "ref": "app.xaniteog.install/xaniteog_apk",
                                "kind": "file",
                                "suffix": ".apk",
                            },
                        ],
                    }
                ],
            },
            source="synthetic.json",
        )

        self.assertEqual(matrix.schema_version, 1)
        self.assertEqual([scenario.id for scenario in matrix.scenarios], ["ayaneo_pocket_s2_base"])
        scenario = matrix.scenarios[0]
        self.assertEqual(scenario.device_plan, "ayaneo.pocket_s2.base")
        self.assertEqual(scenario.expected_classification, "match")
        self.assertEqual(scenario.known_gap_ids, ())
        self.assertEqual(
            [(binding.ref, binding.kind, binding.suffix) for binding in scenario.bindings],
            [
                ("feature.copy_bios/bios_source_dir", "directory", None),
                ("app.xaniteog.install/xaniteog_apk", "file", ".apk"),
            ],
        )

    def test_scenario_matrix_rejects_malformed_definitions_with_stable_errors(self) -> None:
        malformed_payloads = [
            (
                {"schema_version": 2, "scenarios": []},
                "schema_version must be 1",
            ),
            (
                {"schema_version": 1, "scenarios": [{"id": "missing_fields"}]},
                "scenarios[0].device_plan must be a non-empty string",
            ),
            (
                {
                    "schema_version": 1,
                    "scenarios": [
                        {
                            "id": "bad_bindings",
                            "device_plan": "example.plan",
                            "expected_classification": "match",
                            "bindings": {
                                "feature.copy_bios/bios_source_dir": {"kind": "directory"}
                            },
                        }
                    ],
                },
                "scenarios[0].bindings must be a list",
            ),
            (
                {
                    "schema_version": 1,
                    "scenarios": [
                        {
                            "id": "bad_suffix",
                            "device_plan": "example.plan",
                            "expected_classification": "match",
                            "bindings": [
                                {
                                    "ref": "app.xaniteog.install/xaniteog_apk",
                                    "kind": "file",
                                    "suffix": ".zip",
                                }
                            ],
                        }
                    ],
                },
                "scenarios[0].bindings[0].suffix must be one of: .apk, .cfg",
            ),
        ]

        for payload, message in malformed_payloads:
            with self.subTest(message=message):
                with self.assertRaises(ValueError) as context:
                    harness.parse_scenario_matrix_payload(payload, source="synthetic.json")
                self.assertIn(message, str(context.exception))

    def test_temp_binding_specs_create_resources_without_report_path_leaks(self) -> None:
        scenario = harness.PlanParityScenario(
            id="example",
            device_plan="example.plan",
            expected_classification="match",
            bindings=(
                harness.PlanParityBindingSpec(
                    ref="feature.copy_bios/bios_source_dir",
                    kind="directory",
                    suffix=None,
                ),
                harness.PlanParityBindingSpec(
                    ref="app.xaniteog.install/xaniteog_apk",
                    kind="file",
                    suffix=".apk",
                ),
                harness.PlanParityBindingSpec(
                    ref="app.retroarch.provision/retroarch_cfg",
                    kind="file",
                    suffix=".cfg",
                ),
            ),
            notes="Synthetic test scenario.",
            known_gap_ids=(),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            prepared = harness.prepare_scenario_bindings(scenario, Path(temp_dir))
            bind_values = [raw.split("=", 1)[1] for raw in prepared.raw_cli_binds]

            self.assertTrue(Path(bind_values[0]).is_dir())
            self.assertTrue(Path(bind_values[1]).is_file())
            self.assertEqual(Path(bind_values[1]).suffix, ".apk")
            self.assertTrue(Path(bind_values[2]).is_file())
            self.assertEqual(Path(bind_values[2]).suffix, ".cfg")
            self.assertEqual(
                prepared.report_bindings,
                [
                    {"ref": "feature.copy_bios/bios_source_dir", "kind": "directory"},
                    {"ref": "app.xaniteog.install/xaniteog_apk", "kind": "file", "suffix": ".apk"},
                    {"ref": "app.retroarch.provision/retroarch_cfg", "kind": "file", "suffix": ".cfg"},
                ],
            )
            self.assertNotIn(temp_dir, json.dumps(prepared.report_bindings))

    def test_matrix_report_aggregates_synthetic_scenarios_and_expectations(self) -> None:
        matrix = harness.PlanParityScenarioMatrix(
            schema_version=1,
            scenarios=(
                harness.PlanParityScenario(
                    id="matching",
                    device_plan="matching.plan",
                    expected_classification="match",
                    bindings=(),
                    notes="Expected to match.",
                    known_gap_ids=(),
                ),
                harness.PlanParityScenario(
                    id="expected_gap",
                    device_plan="gap.plan",
                    expected_classification="known_gap",
                    bindings=(),
                    notes="Future scenarios may intentionally expect a known gap.",
                    known_gap_ids=("future_gap",),
                ),
                harness.PlanParityScenario(
                    id="unexpected_gap",
                    device_plan="unexpected.plan",
                    expected_classification="match",
                    bindings=(),
                    notes="Synthetic expectation failure.",
                    known_gap_ids=(),
                ),
            ),
        )
        match_report = minimal_report("matching.plan", "match")
        known_gap_report = minimal_report(
            "gap.plan",
            "known_gap",
            counts={"known_gap": 1, "match": 11, "intentional_shape_difference": 1},
            known_gaps=[
                {
                    "path": "execution_plan.params",
                    "code": "future_gap",
                    "description": "Synthetic future known gap.",
                }
            ],
        )
        unexpected_report = minimal_report(
            "unexpected.plan",
            "known_gap",
            counts={"known_gap": 1, "match": 11, "intentional_shape_difference": 1},
        )

        report = harness.build_matrix_report(
            authored_root="authored",
            matrix_path="tools/plan_parity_scenarios.json",
            matrix=matrix,
            scenario_results=[
                harness.MatrixScenarioResult(
                    scenario=matrix.scenarios[0],
                    binding_specs=[],
                    comparison_report=match_report,
                ),
                harness.MatrixScenarioResult(
                    scenario=matrix.scenarios[1],
                    binding_specs=[],
                    comparison_report=known_gap_report,
                ),
                harness.MatrixScenarioResult(
                    scenario=matrix.scenarios[2],
                    binding_specs=[],
                    comparison_report=unexpected_report,
                ),
            ],
        )

        self.assertEqual(report["kind"], "rust_python_planner_parity_matrix_report")
        self.assertEqual(report["summary"]["scenario_count"], 3)
        self.assertEqual(report["summary"]["expectation_counts"], {"pass": 2, "fail": 1})
        self.assertEqual(report["summary"]["expected_classification_counts"]["match"], 2)
        self.assertEqual(report["summary"]["actual_classification_counts"]["known_gap"], 2)
        self.assertEqual(report["summary"]["mismatch_buckets"], {"known_gap": 2})
        self.assertEqual(report["scenarios"][0]["expectation_status"], "pass")
        self.assertEqual(report["scenarios"][1]["expectation_status"], "pass")
        self.assertEqual(report["scenarios"][2]["expectation_status"], "fail")
        self.assertEqual(report["known_gaps"][0]["scenario_id"], "expected_gap")
        self.assertEqual(harness.matrix_exit_code(report), 1)

    def test_matrix_json_output_is_deterministic_for_identical_synthetic_inputs(self) -> None:
        matrix = harness.PlanParityScenarioMatrix(
            schema_version=1,
            scenarios=(
                harness.PlanParityScenario(
                    id="matching",
                    device_plan="matching.plan",
                    expected_classification="match",
                    bindings=(),
                    notes="Expected to match.",
                    known_gap_ids=(),
                ),
            ),
        )
        scenario_result = harness.MatrixScenarioResult(
            scenario=matrix.scenarios[0],
            binding_specs=[],
            comparison_report=minimal_report("matching.plan", "match"),
        )

        first = harness.dumps_report(
            harness.build_matrix_report(
                authored_root="authored",
                matrix_path="tools/plan_parity_scenarios.json",
                matrix=matrix,
                scenario_results=[scenario_result],
            )
        )
        second = harness.dumps_report(
            harness.build_matrix_report(
                authored_root="authored",
                matrix_path="tools/plan_parity_scenarios.json",
                matrix=matrix,
                scenario_results=[scenario_result],
            )
        )

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first)["summary"]["expectation_counts"], {"pass": 1, "fail": 0})

    def test_compare_main_single_scenario_behavior_remains_single_report(self) -> None:
        process = harness.ProcessResult(
            exit_code=0,
            stdout=json.dumps(planning_result()) + "\n",
            stderr="",
        )

        with patch.object(harness, "run_process", return_value=process) as run_process:
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                exit_code = harness.compare_main(
                    [
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "example.plan",
                        "--bind",
                        "example.recipe/input=value",
                        "--rust-bin",
                        "/tmp/emuchef-plan-shadow",
                    ]
                )

        report = json.loads(stdout.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(report["kind"], "rust_python_planner_parity_report")
        self.assertEqual(report["inputs"]["device_plan"], "example.plan")
        self.assertEqual(report["inputs"]["binding_keys"], ["example.recipe/input"])
        self.assertEqual(run_process.call_count, 2)


if __name__ == "__main__":
    unittest.main()

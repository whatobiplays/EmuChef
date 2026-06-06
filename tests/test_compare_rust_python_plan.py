from __future__ import annotations

import importlib
import json
import unittest
from collections import OrderedDict
from pathlib import Path


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


if __name__ == "__main__":
    unittest.main()

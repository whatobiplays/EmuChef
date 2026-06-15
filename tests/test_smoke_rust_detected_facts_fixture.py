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
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_detected_facts_fixture.py"


def import_smoke_module():
    module_name = "smoke_rust_detected_facts_fixture"
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


def planning_result_json(
    *,
    status: str = "success",
    warnings: list[dict] | None = None,
    device_context: dict | None = None,
) -> str:
    return json.dumps(
        {
            "kind": "planning_result",
            "schema_version": 1,
            "status": status,
            "warnings": warnings or [],
            "errors": [],
            "execution_plan": {
                "id": "plan.shadow.ayaneo.pocket_s_mini.base.001",
                "schema_version": 1,
                "kind": "execution_plan",
                "source": {
                    "device_plan_ref": "ayaneo.pocket_s_mini.base",
                    "device_profile_ref": "ayaneo.pocket_s_mini",
                    "selected_recipe_refs": ["app.retroarch.provision"],
                    "expanded_recipe_refs": ["app.retroarch.provision"],
                },
                "device_context": device_context
                or {
                    "manufacturer": "AYANEO",
                    "model": "AYANEO Pocket S mini",
                    "android_version": 13,
                    "android_api_level": 33,
                    "device_tags": ["detected_handheld"],
                },
                "runtime_capabilities": {},
                "inputs": [],
                "artifacts": [],
                "steps": [],
            },
        },
        indent=2,
    )


class SmokeRustDetectedFactsFixturePresenceTests(unittest.TestCase):
    def test_tool_path_exists(self) -> None:
        self.assertTrue(SMOKE_PATH.exists())


@unittest.skipUnless(SMOKE_PATH.exists(), "smoke tool is not implemented yet")
class SmokeRustDetectedFactsFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_direct_shadow_commands_with_detected_facts_fixture(self) -> None:
        command = self.smoke.build_shadow_command(
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            authored_root="authored",
            fixture_path=Path("/tmp/facts.json"),
        )

        self.assertEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertIn("--detected-facts-json", command)
        self.assertEqual(command[command.index("--detected-facts-json") + 1], "/tmp/facts.json")
        self.assertEqual(command[command.index("--authored-root") + 1], "authored")
        self.assertEqual(command[command.index("--device-plan") + 1], "ayaneo.pocket_s_mini.base")
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb", command)
        self.assertNotIn("-m", command)
        self.assertNotIn("emuchef", command)
        self.assertNotIn("compare_rust_python_plan", " ".join(command))
        self.assertNotIn("smoke_rust_shadow_cli_matrix", " ".join(command))

    def test_explicit_context_case_appends_context_overrides(self) -> None:
        command = self.smoke.build_shadow_command(
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            authored_root="authored",
            fixture_path=Path("/tmp/facts.json"),
            explicit_context=self.smoke.ExplicitContext(
                manufacturer="AYANEO",
                model="AYANEO Pocket S mini",
                android_version=13,
                device_tags=("explicit_handheld",),
            ),
        )

        self.assertEqual(command[command.index("--manufacturer") + 1], "AYANEO")
        self.assertEqual(command[command.index("--model") + 1], "AYANEO Pocket S mini")
        self.assertEqual(command[command.index("--android-version") + 1], "13")
        self.assertEqual(command[command.index("--device-tag") + 1], "explicit_handheld")

    def test_matching_fixture_case_passes_with_no_mismatch_warning(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[0]
            with patch.object(self.smoke, "run_process", return_value=completed) as run_process:
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    temp_root=Path(temp_dir),
                )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.actual_warning_codes, [])
        self.assertEqual(result.stdout_class, "planning_result_json")
        self.assertEqual(result.exit_class, "success")
        self.assertEqual(run_process.call_args.args[0][0], "/tmp/emuchef-plan-shadow")

    def test_mismatching_fixture_case_passes_with_one_mismatch_warning(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout=planning_result_json(
                status="warning",
                warnings=[
                    {
                        "code": "device_profile_mismatch",
                        "message": "Selected profile does not match.",
                        "details": {"manufacturer": "Valve"},
                    }
                ],
                device_context={
                    "manufacturer": "Valve",
                    "model": "Steam Deck",
                    "android_version": 12,
                    "android_api_level": 32,
                    "device_tags": ["detected_mismatch"],
                },
            ),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[1]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    temp_root=Path(temp_dir),
                )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.actual_warning_codes, ["device_profile_mismatch"])
        self.assertEqual(result.stdout_class, "planning_result_json")
        self.assertEqual(result.exit_class, "warning")

    def test_warning_exit_behavior_follows_current_shadow_mapping(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(
                status="warning",
                warnings=[{"code": "device_profile_mismatch", "message": "Mismatch.", "details": {}}],
                device_context={
                    "manufacturer": "Valve",
                    "model": "Steam Deck",
                    "android_version": 12,
                    "android_api_level": 32,
                    "device_tags": ["detected_mismatch"],
                },
            ),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[1]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    temp_root=Path(temp_dir),
                )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "unexpected_exit")

    def test_explicit_context_case_keeps_warning_on_fixture_facts(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout=planning_result_json(
                status="warning",
                warnings=[
                    {
                        "code": "device_profile_mismatch",
                        "message": "Selected profile does not match.",
                        "details": {"manufacturer": "Valve"},
                    }
                ],
                device_context={
                    "manufacturer": "AYANEO",
                    "model": "AYANEO Pocket S mini",
                    "android_version": 13,
                    "android_api_level": 32,
                    "device_tags": ["explicit_handheld"],
                },
            ),
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[2]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    temp_root=Path(temp_dir),
                )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.actual_warning_codes, ["device_profile_mismatch"])

    def test_raw_invalid_stdout_classifies_as_failure(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="not json\n",
            stderr="",
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            case = self.smoke.SMOKE_CASES[0]
            with patch.object(self.smoke, "run_process", return_value=completed):
                result = self.smoke.run_case(
                    case,
                    authored_root="authored",
                    rust_planner_bin="/tmp/emuchef-plan-shadow",
                    temp_root=Path(temp_dir),
                )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.stdout_class, "stdout_text")

    def test_report_omits_temp_paths_absolute_inputs_and_full_volatile_output(self) -> None:
        result = self.smoke.CaseResult(
            id="failing",
            status="fail",
            expected_warning_code=None,
            actual_warning_codes=[],
            stdout_class="stdout_text",
            exit_class="success",
            stderr_class="stderr_text",
            failure_summary="stdout: very long volatile detail",
        )
        report = self.smoke.build_report(
            authored_root="/Users/example/Projects/EmuChef/authored",
            rust_planner_bin="/Users/example/target/debug/emuchef-plan-shadow",
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn(tempfile.gettempdir(), serialized)
        self.assertNotIn("very long volatile detail", serialized)

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        results = [
            self.smoke.CaseResult(
                id="matching_detected_facts",
                status="pass",
                expected_warning_code=None,
                actual_warning_codes=[],
                stdout_class="planning_result_json",
                exit_class="success",
                stderr_class="stderr_empty",
            )
        ]

        first = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                case_results=results,
            )
        )
        second = self.smoke.dumps_report(
            self.smoke.build_report(
                authored_root="authored",
                rust_planner_bin="/tmp/emuchef-plan-shadow",
                case_results=results,
            )
        )

        self.assertEqual(first, second)
        self.assertEqual(json.loads(first)["summary"], {"passed": 1, "failed": 0})

    def test_any_case_failure_returns_nonzero(self) -> None:
        report = self.smoke.build_report(
            authored_root="authored",
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            case_results=[
                self.smoke.CaseResult(
                    id="failing",
                    status="fail",
                    expected_warning_code=None,
                    actual_warning_codes=[],
                    stdout_class="stdout_text",
                    exit_class="success",
                    stderr_class="stderr_empty",
                )
            ],
        )

        self.assertEqual(self.smoke.smoke_exit_code(report), 1)

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
            r"^\s*import\s+crates\b",
            r"^\s*from\s+crates\b",
            r"^\s*import\s+tauri\b",
            r"^\s*from\s+tauri\b",
        ]
        for pattern in forbidden_import_patterns:
            with self.subTest(pattern=pattern):
                self.assertIsNone(re.search(pattern, source, flags=re.MULTILINE))

        forbidden_tokens = [
            "cargo",
            "adb ",
            "adb.exe",
            "-m emuchef",
            "compare_rust_python_plan",
            "smoke_rust_shadow_cli_matrix",
        ]
        for token in forbidden_tokens:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

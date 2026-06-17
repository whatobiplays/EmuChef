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
SMOKE_PATH = REPO_ROOT / "tools" / "smoke_rust_shadow_live_adb_probe.py"


def import_smoke_module():
    module_name = "smoke_rust_shadow_live_adb_probe"
    spec = importlib.util.spec_from_file_location(module_name, SMOKE_PATH)
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
    include_kind: bool = False,
) -> str:
    payload = {
        "schema_version": 1,
        "status": status,
        "warnings": warnings or [],
        "errors": [],
        "execution_plan": {
            "id": "plan.shadow.ayaneo.pocket_s_mini.base.001",
            "schema_version": 1,
            "source": {
                "device_plan_ref": "ayaneo.pocket_s_mini.base",
                "device_profile_ref": "ayaneo.pocket_s_mini",
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
    }
    if include_kind:
        payload["kind"] = "planning_result"
    return json.dumps(payload, indent=2)


class SmokeRustShadowLiveAdbProbeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.smoke = import_smoke_module()

    def test_builds_direct_shadow_command_with_probe_adb_and_serial(self) -> None:
        command = self.smoke.build_shadow_command(
            rust_planner_bin="/tmp/emuchef-plan-shadow",
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            adb_path="/opt/android/platform-tools/adb",
            serial="SERIAL123",
        )

        self.assertEqual(
            command,
            [
                "/tmp/emuchef-plan-shadow",
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
                "--probe-adb-getprop",
                "--adb-path",
                "/opt/android/platform-tools/adb",
                "--serial",
                "SERIAL123",
            ],
        )
        command_text = " ".join(command)
        self.assertEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertIn("--probe-adb-getprop", command)
        self.assertIn("--adb-path", command)
        self.assertIn("--serial", command)
        self.assertNotIn("python", command)
        self.assertNotIn("-m", command)
        self.assertNotIn("emuchef plan", command_text)
        self.assertNotIn("cargo", command)
        self.assertNotIn("adb devices", command_text)
        self.assertNotIn("--planner-backend", command)
        self.assertNotIn("--rust-shadow-output", command)
        self.assertNotIn("--rust-detected-facts-json", command)

    def test_run_smoke_report_invokes_supplied_rust_binary_directly(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(),
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
        self.assertEqual(command[0], "/tmp/emuchef-plan-shadow")
        self.assertEqual(command[command.index("--adb-path") + 1], "adb")
        self.assertEqual(command[command.index("--serial") + 1], "SERIAL123")
        self.assertNotIn("python", command)
        self.assertNotIn("cargo", command)

    def test_report_uses_scrubbed_inputs_and_command_metadata_only(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(),
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
        case = report["cases"][0]

        self.assertEqual(report["inputs"]["authored_root"], "authored")
        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow")
        self.assertEqual(report["inputs"]["adb_path"], "adb")
        self.assertTrue(report["inputs"]["serial_supplied"])
        self.assertEqual(
            case["command_metadata"],
            {
                "probe_flag_present": True,
                "adb_path_supplied": True,
                "serial_supplied": True,
            },
        )
        self.assertNotIn("SERIAL123", serialized)
        self.assertNotIn("/Users/example", serialized)
        self.assertNotIn("Library/Android", serialized)
        self.assertNotIn("command", report)
        self.assertNotIn("argv", serialized)
        self.assertNotIn("raw non-json text", serialized)
        self.assertNotIn("Error: adb_probe", serialized)

    def test_report_scrubs_windows_style_paths_to_basenames_on_posix(self) -> None:
        result = self.smoke.CaseResult(
            id="live_adb_getprop_shadow_probe",
            status="pass",
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status="success",
            device_context_fields_present=("manufacturer", "model", "android_version", "android_api_level"),
            device_profile_mismatch_seen=False,
            failure_class=None,
        )

        report = self.smoke.build_report(
            authored_root="authored",
            device_plan="ayaneo.pocket_s_mini.base",
            rust_planner_bin=r"C:\Users\example\target\debug\emuchef-plan-shadow.exe",
            adb_path=r"C:\Users\example\AppData\Local\Android\Sdk\platform-tools\adb.exe",
            serial_supplied=True,
            case_results=[result],
        )
        serialized = self.smoke.dumps_report(report)

        self.assertEqual(report["inputs"]["rust_planner_bin"], "emuchef-plan-shadow.exe")
        self.assertEqual(report["inputs"]["adb_path"], "adb.exe")
        self.assertNotIn("C:\\Users", serialized)
        self.assertNotIn("AppData", serialized)
        self.assertNotIn("platform-tools", serialized)
        self.assertNotIn("target\\debug", serialized)
        self.assertNotIn("example", serialized)

    def test_success_planning_result_without_kind_field_passes(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(include_kind=False),
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.exit_class, "success_or_warning")
        self.assertEqual(result.stdout_class, "planning_result_json")
        self.assertEqual(result.stderr_class, "empty")
        self.assertEqual(result.planning_status, "success")
        self.assertEqual(
            result.device_context_fields_present,
            ("manufacturer", "model", "android_version", "android_api_level"),
        )

    def test_warning_device_profile_mismatch_passes_as_route_evidence(self) -> None:
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

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.planning_status, "warning")
        self.assertTrue(result.device_profile_mismatch_seen)

    def test_missing_device_context_fails(self) -> None:
        payload = json.loads(planning_result_json())
        del payload["execution_plan"]["device_context"]
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=json.dumps(payload), stderr="")

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.failure_class, "device_context_missing")

    def test_missing_manufacturer_or_model_fails(self) -> None:
        invalid_contexts = [
            {"model": "AYANEO Pocket S mini", "android_version": 13},
            {"manufacturer": "AYANEO", "model": "", "android_api_level": 33},
        ]

        for device_context in invalid_contexts:
            with self.subTest(device_context=device_context):
                completed = subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout=planning_result_json(device_context=device_context),
                    stderr="",
                )

                result = self.smoke.classify_completed_process(completed)

                self.assertEqual(result.status, "fail")
                self.assertEqual(result.failure_class, "device_context_required_fields_missing")

    def test_missing_android_version_and_api_level_fails(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=planning_result_json(
                device_context={
                    "manufacturer": "AYANEO",
                    "model": "AYANEO Pocket S mini",
                    "device_tags": ["detected_handheld"],
                }
            ),
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.failure_class, "device_context_android_version_missing")

    def test_stable_adb_probe_unavailable_failure_returns_nonzero(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=1,
            stdout="",
            stderr="Error: adb_probe_unavailable\n",
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
            stderr="usage: emuchef-plan-shadow --authored-root <path>\n",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "usage_failure")
        self.assertEqual(result.stderr_class, "usage_failure")

    def test_shadow_process_start_failure_is_distinct_from_adb_probe_failures(self) -> None:
        result = self.smoke.process_start_failure_result()

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.exit_class, "shadow_process_start_failed")
        self.assertEqual(result.failure_class, "shadow_process_start_failed")
        self.assertNotEqual(result.exit_class, "adb_probe_unavailable")
        self.assertNotEqual(result.exit_class, "adb_probe_failed")

    def test_raw_non_json_stdout_fails(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="raw non-json text\n",
            stderr="",
        )

        result = self.smoke.classify_completed_process(completed)

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.stdout_class, "stdout_text")

    def test_report_is_deterministic_for_identical_fake_inputs(self) -> None:
        result = self.smoke.CaseResult(
            id="live_adb_getprop_shadow_probe",
            status="pass",
            exit_class="success_or_warning",
            stdout_class="planning_result_json",
            stderr_class="empty",
            planning_status="success",
            device_context_fields_present=("manufacturer", "model", "android_version", "android_api_level"),
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

    def test_source_does_not_invoke_cargo_adb_devices_or_python_routes(self) -> None:
        source = SMOKE_PATH.read_text(encoding="utf-8")

        forbidden_tokens = [
            "cargo",
            "adb devices",
            "-m emuchef",
            "emuchef plan",
            "--planner-backend",
            "--rust-shadow-output",
            "--rust-detected-facts-json",
            "compare_rust_python_plan",
            "smoke_rust_shadow_cli_matrix",
            "smoke_rust_detected_facts_fixture",
            "smoke_rust_experimental_detected_facts_fixture",
            "executor/apply",
            "Tauri",
            "protocol",
        ]
        for token in forbidden_tokens:
            with self.subTest(token=token):
                self.assertNotIn(token, source)


if __name__ == "__main__":
    unittest.main()

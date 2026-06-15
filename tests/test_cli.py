from __future__ import annotations

import json
from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
import contextlib
import subprocess
import unittest
from unittest.mock import patch

from emuchef.cli import main
from emuchef.domain import (
    DeviceContext,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    LiteralParamValue,
    RuntimeCapabilities,
)
from emuchef.executor import DetectedDevice
from emuchef.io import dump_yaml, load_authored_catalog
from emuchef.planner import Planner


class CliTests(unittest.TestCase):
    def _shadow_bin(self, tmp: str) -> str:
        shadow_bin = Path(tmp) / "emuchef-plan-shadow"
        shadow_bin.write_text("#!/bin/sh\n", encoding="utf-8")
        shadow_bin.chmod(0o755)
        return str(shadow_bin)

    def _rust_shadow_planning_result_json(
        self,
        *,
        status: str = "success",
        execution_plan: dict | None = None,
        warnings: list[dict] | None = None,
        errors: list[dict] | None = None,
    ) -> str:
        if execution_plan is None and status != "error":
            execution_plan = {
                "id": "plan.shadow.example.001",
                "source": {
                    "device_profile_ref": "example.device_profile",
                    "device_plan_ref": "example.device_plan",
                    "selected_recipe_refs": ["example.recipe"],
                    "expanded_recipe_refs": ["example.recipe"],
                },
                "device_context": {
                    "manufacturer": "Example",
                    "model": "Example",
                    "android_version": 13,
                    "android_api_level": None,
                    "device_tags": [],
                },
                "runtime_capabilities": {
                    "adb_available": True,
                    "apk_install": True,
                    "shared_storage_write": True,
                    "app_launch": True,
                    "shell_command": True,
                    "package_remove_for_user": False,
                    "root_shell": True,
                    "app_data_write": True,
                },
                "inputs": [],
                "artifacts": [],
                "steps": [
                    {
                        "id": "example.recipe/wait",
                        "recipe_ref": "example.recipe",
                        "type": "wait",
                        "name": "Wait",
                        "dependencies": [],
                        "constraints": {"capabilities": [], "conflicts_with": []},
                        "params": {"duration_ms": {"value": 10}},
                        "skip_if": [],
                        "verify": [],
                    }
                ],
                "schema_version": 1,
                "kind": "execution_plan",
            }
        payload = {
            "status": status,
            "warnings": warnings or [],
            "errors": errors or [],
            "execution_plan": execution_plan,
            "schema_version": 1,
            "kind": "planning_result",
        }
        return json.dumps(payload, indent=2, sort_keys=False) + "\n"

    def test_validate_command_succeeds_for_bundled_catalog(self) -> None:
        stdout = StringIO()
        stderr = StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            rc = main(["validate", "--authored-root", "authored"])
        self.assertEqual(rc, 0, stderr.getvalue())
        self.assertIn("Validation status: success", stdout.getvalue())

    def test_plan_verbose_emits_normalized_execution_plan(self) -> None:
        with TemporaryDirectory() as tmp:
            cfg = Path(tmp) / "retroarch.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")

            detected = DetectedDevice(
                serial="SERIAL",
                manufacturer="AYANEO",
                brand="AYANEO",
                model="Pocket S mini",
                android_version=13,
                android_api_level=33,
                root_available=True,
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable", return_value="adb"),
                patch("emuchef.cli.SubprocessAdb.detect_device", return_value=detected),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                        "--bind",
                        f"app.retroarch.provision/retroarch_cfg={cfg}",
                        "--verbose",
                    ]
                )
            self.assertEqual(rc, 0, stderr.getvalue())
            output = stdout.getvalue()
            self.assertIn("kind: planning_result", output)
            self.assertIn("artifacts:", output)
            self.assertIn("ref: inputs.app.retroarch.provision/retroarch_cfg", output)
            self.assertIn("type: resolve_artifacts", output)

    def test_plan_allows_missing_optional_retroarch_cfg(self) -> None:
        detected = DetectedDevice(
            serial="SERIAL",
            manufacturer="AYANEO",
            brand="AYANEO",
            model="Pocket FIT",
            android_version=14,
            android_api_level=34,
            root_available=True,
        )
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable", return_value="adb"),
            patch("emuchef.cli.SubprocessAdb.detect_device", return_value=detected),
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.konkr_pocket_fit.base",
                    "--verbose",
                ]
            )
        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("status: success", output)
        self.assertNotIn("binding_missing", output)
        self.assertIn("id: app.retroarch.provision/launch_retroarch", output)
        self.assertNotIn("id: app.retroarch.provision/seed_retroarch_cfg", output)

    def test_plan_default_uses_python_planner_without_rust_process(self) -> None:
        detected = DetectedDevice(
            serial="SERIAL",
            manufacturer="AYANEO",
            brand="AYANEO",
            model="Pocket FIT",
            android_version=14,
            android_api_level=34,
            root_available=True,
        )
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable", return_value="adb"),
            patch("emuchef.cli.SubprocessAdb.detect_device", return_value=detected),
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.konkr_pocket_fit.base",
                ]
            )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("Planning status: success", output)
        self.assertNotIn("kind: planning_result", output)
        run.assert_not_called()

    def test_plan_explicit_python_backend_uses_python_summary_without_rust_process(self) -> None:
        detected = DetectedDevice(
            serial="SERIAL",
            manufacturer="AYANEO",
            brand="AYANEO",
            model="Pocket FIT",
            android_version=14,
            android_api_level=34,
            root_available=True,
        )
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable", return_value="adb"),
            patch("emuchef.cli.SubprocessAdb.detect_device", return_value=detected),
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--planner-backend",
                    "python",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.konkr_pocket_fit.base",
                ]
            )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("Planning status: success", output)
        self.assertNotIn("kind: planning_result", output)
        run.assert_not_called()

    def test_plan_python_backend_rejects_rust_shadow_output(self) -> None:
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--planner-backend",
                    "python",
                    "--rust-shadow-output",
                    "python-compatible",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.konkr_pocket_fit.base",
                ]
            )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("--rust-shadow-output is only valid with --planner-backend rust-shadow", stderr.getvalue())
        resolve_adb.assert_not_called()
        run.assert_not_called()

    def test_plan_rust_shadow_requires_explicit_binary_path(self) -> None:
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--planner-backend",
                    "rust-shadow",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.pocket_s_mini.base",
                ]
            )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("--rust-planner-bin is required", stderr.getvalue())
        resolve_adb.assert_not_called()
        run.assert_not_called()

    def test_plan_rust_shadow_invokes_explicit_binary_and_forwards_raw_binds(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{\n  "kind": "planning_result",\n  "status": "success"\n}\n',
                stderr="rust shadow diagnostic\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--bind",
                        "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
                        "--bind",
                        "app.xaniteog.install/xaniteog_apk=/tmp/app.apk",
                        "--bind",
                        "feature.copy_bios/bios_source_dir=/tmp/second",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        self.assertEqual(stdout.getvalue(), completed.stdout)
        self.assertEqual(stderr.getvalue(), completed.stderr)
        self.assertNotIn("Planning status:", stdout.getvalue())
        resolve_adb.assert_not_called()
        detect_device.assert_not_called()
        run.assert_called_once()
        self.assertEqual(
            run.call_args.args[0],
            [
                shadow_bin,
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s2.base",
                "--bind",
                "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
                "--bind",
                "app.xaniteog.install/xaniteog_apk=/tmp/app.apk",
                "--bind",
                "feature.copy_bios/bios_source_dir=/tmp/second",
            ],
        )
        self.assertEqual(run.call_args.kwargs["check"], False)
        self.assertEqual(run.call_args.kwargs["text"], True)
        self.assertEqual(run.call_args.kwargs["capture_output"], True)
        self.assertNotIn("cargo", run.call_args.args[0])
        self.assertNotIn("--sidecar", run.call_args.args[0])

    def test_plan_rust_shadow_forwards_explicit_device_context_flags(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{\n  "kind": "planning_result",\n  "status": "success"\n}\n',
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
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
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        self.assertEqual(stdout.getvalue(), completed.stdout)
        self.assertEqual(stderr.getvalue(), "")
        resolve_adb.assert_not_called()
        detect_device.assert_not_called()
        run.assert_called_once()
        self.assertEqual(
            run.call_args.args[0],
            [
                shadow_bin,
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
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
            ],
        )
        self.assertNotIn("cargo", run.call_args.args[0])
        self.assertNotIn("adb", run.call_args.args[0])
        self.assertNotIn("--sidecar", run.call_args.args[0])

    def test_plan_rejects_detected_facts_fixture_option_before_rust_forwarding(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            fixture = Path(tmp) / "facts.json"
            fixture.write_text("{}", encoding="utf-8")
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run") as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
                self.assertRaises(SystemExit) as raised,
            ):
                main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                        "--detected-facts-json",
                        str(fixture),
                    ]
                )

        self.assertEqual(raised.exception.code, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("unrecognized arguments: --detected-facts-json", stderr.getvalue())
        resolve_adb.assert_not_called()
        run.assert_not_called()

    def test_plan_rust_shadow_passes_through_json_and_exit_code_for_planner_error(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=7,
                stdout='{\n  "kind": "planning_result",\n  "status": "error"\n}\n',
                stderr="rust planner error detail\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 7)
        self.assertEqual(stdout.getvalue(), completed.stdout)
        self.assertEqual(stderr.getvalue(), completed.stderr)
        self.assertNotIn("Planning status:", stdout.getvalue())
        resolve_adb.assert_not_called()

    def test_plan_rust_shadow_explicit_passthrough_output_mode_preserves_raw_process_io(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=7,
                stdout=self._rust_shadow_planning_result_json(
                    status="error",
                    execution_plan=None,
                    errors=[
                        {
                            "code": "binding_missing",
                            "message": "Required input is missing.",
                            "details": {"input_id": "example.recipe/input"},
                        }
                    ],
                ),
                stderr="rust planner error detail\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "passthrough",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 7)
        self.assertEqual(stdout.getvalue(), completed.stdout)
        self.assertEqual(stderr.getvalue(), completed.stderr)
        self.assertNotIn("Planning status:", stdout.getvalue())
        resolve_adb.assert_not_called()

    def test_plan_rust_shadow_python_compatible_success_prints_concise_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(
                    warnings=[
                        {
                            "code": "device_profile_mismatch",
                            "message": "Selected profile does not match.",
                            "details": {"device_profile_ref": "example.device_profile"},
                        }
                    ],
                ),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli._build_session") as build_session,
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("Planning status: success", output)
        self.assertIn("Execution plan: plan.shadow.example.001", output)
        self.assertIn("Runnable steps:", output)
        self.assertIn("- example.recipe/wait", output)
        self.assertIn("Warnings:", output)
        self.assertIn("- device_profile_mismatch: Selected profile does not match.", output)
        self.assertNotIn("kind: planning_result", output)
        self.assertEqual(stderr.getvalue(), "")
        build_session.assert_not_called()
        resolve_adb.assert_not_called()
        detect_device.assert_not_called()
        run.assert_called_once()
        self.assertNotIn("cargo", run.call_args.args[0])
        self.assertNotIn("adb", run.call_args.args[0])
        self.assertNotIn("--sidecar", run.call_args.args[0])

    def test_plan_rust_shadow_python_compatible_verbose_emits_structured_yaml(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--verbose",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("status: success", output)
        self.assertIn("execution_plan:", output)
        self.assertIn("id: plan.shadow.example.001", output)
        self.assertIn("schema_version: 1", output)
        self.assertIn("kind: planning_result", output)
        self.assertNotIn("Planning status:", output)
        resolve_adb.assert_not_called()

    def test_plan_rust_shadow_python_compatible_output_writes_yaml_and_prints_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            output_path = Path(tmp) / "planning-result.yaml"
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--output",
                        str(output_path),
                    ]
                )

            self.assertEqual(rc, 0, stderr.getvalue())
            summary = stdout.getvalue()
            self.assertIn("Planning status: success", summary)
            self.assertIn(f"Wrote planning result: {output_path.resolve()}", summary)
            self.assertIn("Execution plan: plan.shadow.example.001", summary)
            self.assertNotIn("execution_plan:", summary)
            yaml_output = output_path.read_text(encoding="utf-8")
            self.assertIn("status: success", yaml_output)
            self.assertIn("execution_plan:", yaml_output)
            self.assertIn("kind: planning_result", yaml_output)
            resolve_adb.assert_not_called()

    def test_plan_rust_shadow_python_compatible_formats_planner_error_and_preserves_rust_exit_code(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=7,
                stdout=self._rust_shadow_planning_result_json(
                    status="error",
                    execution_plan=None,
                    errors=[
                        {
                            "code": "binding_missing",
                            "message": "Required input is missing.",
                            "details": {"input_id": "example.recipe/input"},
                        }
                    ],
                ),
                stderr="rust planner validation detail\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 7)
        output = stdout.getvalue()
        self.assertIn("Planning status: error", output)
        self.assertIn("Errors:", output)
        self.assertIn("- binding_missing: Required input is missing.", output)
        self.assertIn("rust planner validation detail", stderr.getvalue())
        self.assertNotIn("Rust shadow planner failed", stderr.getvalue())
        resolve_adb.assert_not_called()

    def test_plan_rust_shadow_python_compatible_zero_exit_without_execution_plan_returns_nonzero(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(
                    status="error",
                    execution_plan=None,
                    errors=[
                        {
                            "code": "empty_execution_plan",
                            "message": "Execution plan emission produced no runnable steps.",
                            "details": {"plan_id": "plan.shadow.example.001"},
                        }
                    ],
                ),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 1)
        self.assertIn("Planning status: error", stdout.getvalue())
        self.assertIn("empty_execution_plan", stdout.getvalue())

    def test_plan_rust_shadow_python_compatible_invalid_json_fails_clearly(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="not json\n",
                stderr="rust diagnostic\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("rust diagnostic", stderr.getvalue())
        self.assertIn("python-compatible", stderr.getvalue())
        self.assertIn("not valid JSON", stderr.getvalue())

    def test_plan_rust_shadow_python_compatible_process_failure_without_json_fails_clearly(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=2,
                stdout="",
                stderr="rust usage details\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-shadow-output",
                        "python-compatible",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("rust usage details", stderr.getvalue())
        self.assertIn("python-compatible", stderr.getvalue())
        self.assertIn("did not emit PlanningResult JSON", stderr.getvalue())

    def test_plan_rust_experimental_requires_explicit_binary_path(self) -> None:
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--planner-backend",
                    "rust-experimental",
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.pocket_s_mini.base",
                ]
            )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("--rust-planner-bin is required", stderr.getvalue())
        self.assertIn("rust-experimental", stderr.getvalue())
        resolve_adb.assert_not_called()
        run.assert_not_called()

    def test_plan_rust_experimental_invokes_shadow_command_and_prints_concise_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(
                    warnings=[
                        {
                            "code": "device_profile_mismatch",
                            "message": "Selected profile does not match.",
                            "details": {"device_profile_ref": "example.device_profile"},
                        }
                    ],
                ),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli._build_session") as build_session,
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("emuchef.cli._run_apply") as run_apply,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--bind",
                        "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("Planning status: success", output)
        self.assertIn("Execution plan: plan.shadow.example.001", output)
        self.assertIn("Runnable steps:", output)
        self.assertIn("- example.recipe/wait", output)
        self.assertIn("Warnings:", output)
        self.assertNotIn("kind: planning_result", output)
        self.assertEqual(stderr.getvalue(), "")
        build_session.assert_not_called()
        resolve_adb.assert_not_called()
        detect_device.assert_not_called()
        run_apply.assert_not_called()
        run.assert_called_once()
        self.assertEqual(
            run.call_args.args[0],
            [
                shadow_bin,
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s2.base",
                "--bind",
                "feature.copy_bios/bios_source_dir=/tmp/bios=with=equals",
            ],
        )
        self.assertNotIn("cargo", run.call_args.args[0])
        self.assertNotIn("adb", run.call_args.args[0])
        self.assertNotIn("--sidecar", run.call_args.args[0])

    def test_plan_rust_experimental_forwards_explicit_device_context_without_python_planner_or_adb(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli._build_session") as build_session,
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("emuchef.cli._run_apply") as run_apply,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
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
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        self.assertIn("Planning status: success", stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        build_session.assert_not_called()
        resolve_adb.assert_not_called()
        detect_device.assert_not_called()
        run_apply.assert_not_called()
        run.assert_called_once()
        self.assertEqual(
            run.call_args.args[0],
            [
                shadow_bin,
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
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
            ],
        )
        self.assertNotIn("cargo", run.call_args.args[0])
        self.assertNotIn("adb", run.call_args.args[0])
        self.assertNotIn("--sidecar", run.call_args.args[0])

    def test_plan_rust_experimental_verbose_emits_structured_yaml(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--verbose",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        output = stdout.getvalue()
        self.assertIn("status: success", output)
        self.assertIn("execution_plan:", output)
        self.assertIn("id: plan.shadow.example.001", output)
        self.assertIn("kind: planning_result", output)
        self.assertNotIn("Planning status:", output)
        resolve_adb.assert_not_called()

    def test_plan_rust_experimental_output_writes_yaml_and_prints_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            output_path = Path(tmp) / "planning-result.yaml"
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout=self._rust_shadow_planning_result_json(),
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                        "--output",
                        str(output_path),
                    ]
                )

            self.assertEqual(rc, 0, stderr.getvalue())
            summary = stdout.getvalue()
            self.assertIn("Planning status: success", summary)
            self.assertIn(f"Wrote planning result: {output_path.resolve()}", summary)
            self.assertIn("Execution plan: plan.shadow.example.001", summary)
            self.assertNotIn("execution_plan:", summary)
            yaml_output = output_path.read_text(encoding="utf-8")
            self.assertIn("status: success", yaml_output)
            self.assertIn("execution_plan:", yaml_output)
            self.assertIn("kind: planning_result", yaml_output)
            resolve_adb.assert_not_called()

    def test_plan_rust_experimental_rejects_rust_shadow_output(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            for output_mode in ("passthrough", "python-compatible"):
                with self.subTest(output_mode=output_mode):
                    stdout = StringIO()
                    stderr = StringIO()
                    with (
                        patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                        patch("subprocess.run") as run,
                        contextlib.redirect_stdout(stdout),
                        contextlib.redirect_stderr(stderr),
                    ):
                        rc = main(
                            [
                                "plan",
                                "--planner-backend",
                                "rust-experimental",
                                "--rust-shadow-output",
                                output_mode,
                                "--rust-planner-bin",
                                shadow_bin,
                                "--authored-root",
                                "authored",
                                "--device-plan",
                                "ayaneo.pocket_s2.base",
                            ]
                        )

                    self.assertEqual(rc, 1)
                    self.assertEqual(stdout.getvalue(), "")
                    self.assertIn("--rust-shadow-output is only valid with --planner-backend rust-shadow", stderr.getvalue())
                    resolve_adb.assert_not_called()
                    run.assert_not_called()

    def test_plan_rust_experimental_still_rejects_adb_and_serial(self) -> None:
        unsupported_args = [
            ["--adb", "/does/not/exist"],
            ["--serial", "SERIAL"],
        ]
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            for extra_args in unsupported_args:
                with self.subTest(extra_args=extra_args):
                    stdout = StringIO()
                    stderr = StringIO()
                    with (
                        patch("emuchef.cli._build_session") as build_session,
                        patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                        patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                        patch("subprocess.run") as run,
                        contextlib.redirect_stdout(stdout),
                        contextlib.redirect_stderr(stderr),
                    ):
                        rc = main(
                            [
                                "plan",
                                "--planner-backend",
                                "rust-experimental",
                                "--rust-planner-bin",
                                shadow_bin,
                                "--authored-root",
                                "authored",
                                "--device-plan",
                                "ayaneo.pocket_s_mini.base",
                                *extra_args,
                            ]
                        )

                    self.assertEqual(rc, 1)
                    self.assertEqual(stdout.getvalue(), "")
                    self.assertIn("is not supported with --planner-backend rust-experimental", stderr.getvalue())
                    self.assertIn(extra_args[0], stderr.getvalue())
                    build_session.assert_not_called()
                    resolve_adb.assert_not_called()
                    detect_device.assert_not_called()
                    run.assert_not_called()

    def test_plan_rust_experimental_invalid_json_matches_python_compatible_mode(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="not json\n",
                stderr="rust diagnostic\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("rust diagnostic", stderr.getvalue())
        self.assertIn("python-compatible", stderr.getvalue())
        self.assertIn("not valid JSON", stderr.getvalue())

    def test_plan_rust_experimental_process_failure_without_json_matches_python_compatible_mode(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=2,
                stdout="",
                stderr="rust usage details\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-experimental",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s2.base",
                    ]
                )

        self.assertEqual(rc, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("rust usage details", stderr.getvalue())
        self.assertIn("python-compatible", stderr.getvalue())
        self.assertIn("did not emit PlanningResult JSON", stderr.getvalue())

    def test_plan_rust_shadow_rejects_python_only_options(self) -> None:
        unsupported_args = [
            ["--verbose"],
            ["--debug"],
            ["--ops", "ops.yaml"],
            ["--output", "plan.yaml"],
            ["--adb", "/does/not/exist"],
            ["--serial", "SERIAL"],
        ]
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            for extra_args in unsupported_args:
                with self.subTest(extra_args=extra_args):
                    stdout = StringIO()
                    stderr = StringIO()
                    with (
                        patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                        patch("subprocess.run") as run,
                        contextlib.redirect_stdout(stdout),
                        contextlib.redirect_stderr(stderr),
                    ):
                        rc = main(
                            [
                                "plan",
                                "--planner-backend",
                                "rust-shadow",
                                "--rust-planner-bin",
                                shadow_bin,
                                "--authored-root",
                                "authored",
                                "--device-plan",
                                "ayaneo.pocket_s_mini.base",
                                *extra_args,
                            ]
                        )

                    self.assertEqual(rc, 1)
                    self.assertEqual(stdout.getvalue(), "")
                    self.assertIn("is not supported with --planner-backend rust-shadow", stderr.getvalue())
                    self.assertIn(extra_args[0], stderr.getvalue())
                    resolve_adb.assert_not_called()
                    run.assert_not_called()

    def test_rust_planner_output_compatibility_adr_is_referenced_by_readiness_doc(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        adr_matches = sorted(
            (repo_root / "docs" / "adr").glob("*-rust-planner-cli-output-compatibility.md")
        )
        readiness_path = repo_root / "docs" / "rust-planner-cutover-readiness.md"

        self.assertEqual(
            len(adr_matches),
            1,
            "Exactly one Rust planner CLI output compatibility ADR should exist.",
        )
        adr_path = adr_matches[0]
        readiness_text = readiness_path.read_text(encoding="utf-8")
        adr_ref = adr_path.relative_to(repo_root).as_posix()

        self.assertIn(adr_ref, readiness_text)
        for token in [
            "rust-shadow",
            "JSON passthrough",
            "--output",
            "--verbose",
            "--format json",
            "output compatibility",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, readiness_text)

    def test_plan_rust_shadow_does_not_build_python_planner_session_or_detect_device(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{"kind":"planning_result","status":"success"}\n',
                stderr="",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli._build_session") as build_session,
                patch("emuchef.cli.SubprocessAdb.detect_device") as detect_device,
                patch("subprocess.run", return_value=completed) as run,
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                    ]
                )

        self.assertEqual(rc, 0, stderr.getvalue())
        self.assertEqual(stdout.getvalue(), completed.stdout)
        self.assertEqual(stderr.getvalue(), "")
        build_session.assert_not_called()
        detect_device.assert_not_called()
        run.assert_called_once()

    def test_plan_rust_shadow_missing_binary_path_is_clear_error(self) -> None:
        missing_bin = "/definitely/missing/emuchef-plan-shadow"
        stdout = StringIO()
        stderr = StringIO()
        with (
            patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
            patch("subprocess.run") as run,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            rc = main(
                [
                    "plan",
                    "--planner-backend",
                    "rust-shadow",
                    "--rust-planner-bin",
                    missing_bin,
                    "--authored-root",
                    "authored",
                    "--device-plan",
                    "ayaneo.pocket_s_mini.base",
                ]
            )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("Rust shadow planner binary does not exist", stderr.getvalue())
        self.assertIn(missing_bin, stderr.getvalue())
        resolve_adb.assert_not_called()
        run.assert_not_called()

    def test_plan_rust_shadow_failed_start_uses_stable_error_prefix(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", side_effect=OSError("platform-specific detail")),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                    ]
                )

        self.assertEqual(rc, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("Error: failed to start Rust shadow planner", stderr.getvalue())
        self.assertIn(shadow_bin, stderr.getvalue())
        resolve_adb.assert_not_called()

    def test_plan_rust_shadow_process_failure_without_json_uses_stable_error_prefix(self) -> None:
        with TemporaryDirectory() as tmp:
            shadow_bin = self._shadow_bin(tmp)
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=2,
                stdout="",
                stderr="rust usage details\n",
            )
            stdout = StringIO()
            stderr = StringIO()
            with (
                patch("emuchef.cli.resolve_adb_executable") as resolve_adb,
                patch("subprocess.run", return_value=completed),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                rc = main(
                    [
                        "plan",
                        "--planner-backend",
                        "rust-shadow",
                        "--rust-planner-bin",
                        shadow_bin,
                        "--authored-root",
                        "authored",
                        "--device-plan",
                        "ayaneo.pocket_s_mini.base",
                    ]
                )

        self.assertEqual(rc, 2)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("Error: Rust shadow planner failed with exit code 2.", stderr.getvalue())
        self.assertIn("rust usage details", stderr.getvalue())
        resolve_adb.assert_not_called()

    def test_apply_dry_run_shows_progress_and_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            plan = ExecutionPlan(
                id="plan.test",
                source=ExecutionPlanSource(
                    device_profile_ref="example.device_profile",
                    device_plan_ref="example.device_plan",
                    selected_recipe_refs=("example.recipe",),
                    expanded_recipe_refs=("example.recipe",),
                ),
                device_context=DeviceContext(
                    manufacturer="Example",
                    model="Example",
                    android_version=13,
                    android_api_level=33,
                    device_tags=(),
                ),
                runtime_capabilities=RuntimeCapabilities(
                    adb_available=True,
                    apk_install=True,
                    shared_storage_write=True,
                    app_launch=True,
                    shell_command=True,
                    package_remove_for_user=False,
                    root_shell=True,
                    app_data_write=True,
                ),
                inputs=(),
                artifacts=(),
                steps=(
                    ExecutionStep(
                        id="example.recipe/wait",
                        recipe_ref="example.recipe",
                        type="wait",
                        name="Wait",
                        params={"duration_ms": LiteralParamValue(value=10)},
                    ),
                ),
            )
            plan_path = Path(tmp) / "plan.yaml"
            dump_yaml(plan, path=plan_path)

            stdout = StringIO()
            stderr = StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                rc = main(["apply", "--plan-file", str(plan_path), "--dry-run"])
            self.assertEqual(rc, 0, stderr.getvalue())
            output = stdout.getvalue()
            self.assertIn("[1/1] Wait: executing (dry-run)", output)
            self.assertIn("Dry run: success", output)
            self.assertIn("- total: 1", output)
            self.assertIn("- succeeded: 1", output)
            self.assertIn("- blocked: 0", output)

    def test_apply_reports_blocked_steps_separately(self) -> None:
        with TemporaryDirectory() as tmp:
            plan = ExecutionPlan(
                id="plan.test",
                source=ExecutionPlanSource(
                    device_profile_ref="example.device_profile",
                    device_plan_ref="example.device_plan",
                    selected_recipe_refs=("example.recipe",),
                    expanded_recipe_refs=("example.recipe",),
                ),
                device_context=DeviceContext(
                    manufacturer="Example",
                    model="Example",
                    android_version=13,
                    android_api_level=33,
                    device_tags=(),
                ),
                runtime_capabilities=RuntimeCapabilities(
                    adb_available=True,
                    apk_install=True,
                    shared_storage_write=True,
                    app_launch=True,
                    shell_command=True,
                    package_remove_for_user=False,
                    root_shell=True,
                    app_data_write=True,
                ),
                inputs=(),
                artifacts=(),
                steps=(
                    ExecutionStep(
                        id="example.recipe/fail",
                        recipe_ref="example.recipe",
                        type="wait",
                        name="Fail",
                        params={"duration_ms": LiteralParamValue(value=0)},
                    ),
                    ExecutionStep(
                        id="example.recipe/downstream",
                        recipe_ref="example.recipe",
                        type="wait",
                        name="Downstream",
                        dependencies=("example.recipe/fail",),
                        params={"duration_ms": LiteralParamValue(value=1)},
                    ),
                ),
            )
            plan_path = Path(tmp) / "plan.yaml"
            dump_yaml(plan, path=plan_path)

            stdout = StringIO()
            stderr = StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                rc = main(["apply", "--plan-file", str(plan_path), "--dry-run"])
            self.assertEqual(rc, 1, stderr.getvalue())
            output = stdout.getvalue()
            self.assertIn("[2/2] Downstream: blocked", output)
            self.assertIn("Dry run: failed", output)
            self.assertIn("- blocked: 1", output)

    def test_apply_permission_summary_contains_only_runtime_and_appop_reporting(self) -> None:
        with TemporaryDirectory() as tmp:
            plan = ExecutionPlan(
                id="plan.test",
                source=ExecutionPlanSource(
                    device_profile_ref="example.device_profile",
                    device_plan_ref="example.device_plan",
                    selected_recipe_refs=("example.recipe",),
                    expanded_recipe_refs=("example.recipe",),
                ),
                device_context=DeviceContext(
                    manufacturer="Example",
                    model="Example",
                    android_version=13,
                    android_api_level=33,
                    device_tags=(),
                ),
                runtime_capabilities=RuntimeCapabilities(
                    adb_available=True,
                    apk_install=True,
                    shared_storage_write=True,
                    app_launch=True,
                    shell_command=True,
                    package_remove_for_user=False,
                    root_shell=True,
                    app_data_write=True,
                ),
                inputs=(),
                artifacts=(),
                steps=(
                    ExecutionStep(
                        id="example.recipe/grant",
                        recipe_ref="example.recipe",
                        type="grant_permissions",
                        name="Grant",
                        params={
                            "runtime": LiteralParamValue(
                                value=[
                                    {
                                        "package_name": "com.example.app",
                                        "name": "android.permission.POST_NOTIFICATIONS",
                                        "required": False,
                                    }
                                ]
                            ),
                            "appops": LiteralParamValue(
                                value=[
                                    {
                                        "package_name": "com.example.app",
                                        "op": "MANAGE_EXTERNAL_STORAGE",
                                        "mode": "allow",
                                        "required": False,
                                        "when": {"rooted": False},
                                    }
                                ]
                            ),
                        },
                    ),
                ),
            )
            plan_path = Path(tmp) / "plan.yaml"
            dump_yaml(plan, path=plan_path)

            stdout = StringIO()
            stderr = StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                rc = main(["apply", "--plan-file", str(plan_path), "--dry-run"])
            self.assertEqual(rc, 0, stderr.getvalue())
            output = stdout.getvalue()
            self.assertIn("Permission actions:", output)
            self.assertIn("- executed: 1", output)
            self.assertIn("- not_applicable: 1", output)
            self.assertIn("- failed: 0", output)


if __name__ == "__main__":
    unittest.main()

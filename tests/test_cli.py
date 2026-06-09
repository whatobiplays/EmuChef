from __future__ import annotations

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

    def test_plan_rust_shadow_rejects_python_only_options(self) -> None:
        unsupported_args = [
            ["--verbose"],
            ["--debug"],
            ["--ops", "ops.yaml"],
            ["--output", "plan.yaml"],
            ["--adb", "/does/not/exist"],
            ["--serial", "SERIAL"],
            ["--manufacturer", "AYANEO"],
            ["--model", "Pocket"],
            ["--android-version", "14"],
            ["--device-tag", "handheld"],
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

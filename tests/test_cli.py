from __future__ import annotations

from io import StringIO
from pathlib import Path
from tempfile import TemporaryDirectory
import contextlib
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
    StepType,
)
from emuchef.executor import DetectedDevice
from emuchef.io import dump_yaml, load_authored_catalog
from emuchef.planner import Planner


class CliTests(unittest.TestCase):
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
                        type=StepType.WAIT,
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
                        type=StepType.WAIT,
                        name="Fail",
                        params={"duration_ms": LiteralParamValue(value=0)},
                    ),
                    ExecutionStep(
                        id="example.recipe/downstream",
                        recipe_ref="example.recipe",
                        type=StepType.WAIT,
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
                        type=StepType.GRANT_PERMISSIONS,
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

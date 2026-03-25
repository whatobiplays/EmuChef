from __future__ import annotations

import subprocess
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from emuchef.domain import (
    CopyPolicy,
    DeviceContext,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    ResolvedInputValue,
    RuntimeCapabilities,
    StepCondition,
    StepType,
)
from emuchef.executor import DryRunAdb, ExecutorRunner, ProgressPhase, ProgressStatus, SubprocessAdb


class ExecutorCoreTests(unittest.TestCase):
    def test_copy_byo_input_directory_merge_preserves_destination_and_uses_push(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_dir = _build_nested_source_tree(root / "bios")
            adb = DryRunAdb()
            adb.remote_paths.update({"/sdcard/BIOS", "/sdcard/BIOS/existing.bin"})

            runner = ExecutorRunner(adb=adb, workdir=root)
            result = runner.run(_single_step_plan(_copy_byo_input_step(source_dir, CopyPolicy.MERGE.value)))

            self.assertTrue(result.success)
            self.assertIn(("mkdir_p", "/sdcard/BIOS"), adb.commands)
            self.assertIn(("push", str(source_dir / "nested"), "/sdcard/BIOS/nested"), adb.commands)
            self.assertNotIn(("remove_tree", "/sdcard/BIOS"), adb.commands)
            self.assertIn("/sdcard/BIOS/existing.bin", adb.remote_paths)

    def test_copy_byo_input_directory_sync_uses_push_sync(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_dir = _build_nested_source_tree(root / "bios")
            adb = DryRunAdb()

            runner = ExecutorRunner(adb=adb, workdir=root)
            result = runner.run(_single_step_plan(_copy_byo_input_step(source_dir, CopyPolicy.SYNC.value)))

            self.assertTrue(result.success)
            self.assertIn(("push_sync", str(source_dir / "nested"), "/sdcard/BIOS/nested"), adb.commands)
            self.assertNotIn(("remove_tree", "/sdcard/BIOS"), adb.commands)

    def test_copy_byo_input_directory_replace_clears_destination_first(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_dir = _build_nested_source_tree(root / "bios")
            adb = DryRunAdb()
            adb.remote_paths.update({"/sdcard/BIOS", "/sdcard/BIOS/existing.bin"})

            runner = ExecutorRunner(adb=adb, workdir=root)
            result = runner.run(_single_step_plan(_copy_byo_input_step(source_dir, CopyPolicy.REPLACE.value)))

            self.assertTrue(result.success)
            self.assertEqual(adb.commands[:2], [("remove_tree", "/sdcard/BIOS"), ("mkdir_p", "/sdcard/BIOS")])
            self.assertNotIn("/sdcard/BIOS/existing.bin", adb.remote_paths)

    def test_push_dir_sync_uses_push_sync(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            source_dir = _build_nested_source_tree(root / "config")
            adb = DryRunAdb()

            step = ExecutionStep(
                id="example.recipe/push_config",
                recipe_ref="example.recipe",
                type=StepType.PUSH_DIR,
                name="Push config directory",
                params={"source": str(source_dir), "dest": "/sdcard/Config", "copy_policy": CopyPolicy.SYNC.value},
                skip_if=(),
                verify=(),
            )
            runner = ExecutorRunner(adb=adb, workdir=root)
            result = runner.run(_single_step_plan(step))

            self.assertTrue(result.success)
            self.assertIn(("mkdir_p", "/sdcard/Config"), adb.commands)
            self.assertIn(("push_sync", str(source_dir / "nested"), "/sdcard/Config/nested"), adb.commands)

    def test_runner_emits_progress_events_in_order(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            apk_path = root / "retroarch.apk"
            apk_path.write_bytes(b"")
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")

            plan = build_execution_plan(apk_path, bios_dir)
            events = []
            runner = ExecutorRunner(adb=DryRunAdb(), workdir=root)

            result = runner.run(plan, progress_callback=events.append)

            self.assertTrue(result.success)
            self.assertEqual(
                [event.phase for event in events[:4]],
                [
                    ProgressPhase.CHECKING_SKIP_CONDITIONS,
                    ProgressPhase.EXECUTING,
                    ProgressPhase.VERIFYING,
                    ProgressPhase.FINISHED,
                ],
            )
            self.assertEqual(events[0].step_index, 1)
            self.assertEqual(events[0].total_steps, 3)
            self.assertEqual(events[3].status, ProgressStatus.SUCCEEDED)

    def test_runner_progress_reports_skipped_step(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            apk_path = root / "retroarch.apk"
            apk_path.write_bytes(b"")
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")

            plan = build_execution_plan(apk_path, bios_dir)
            adb = DryRunAdb()
            adb.installed_packages.add("com.example.skip")
            events = []
            runner = ExecutorRunner(adb=adb, workdir=root)

            result = runner.run(plan, progress_callback=events.append)

            self.assertTrue(result.success)
            self.assertEqual(result.steps[-1].status.value, "skipped")
            self.assertEqual(events[-1].step_id, "app.retroarch.provision/launch_retroarch")
            self.assertEqual(events[-1].phase, ProgressPhase.FINISHED)
            self.assertEqual(events[-1].status, ProgressStatus.SKIPPED)

    def test_runner_progress_reports_failed_step(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            missing_apk = root / "missing.apk"
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")

            plan = build_execution_plan(missing_apk, bios_dir)
            events = []
            runner = ExecutorRunner(adb=DryRunAdb(), workdir=root)

            result = runner.run(plan, progress_callback=events.append)

            self.assertFalse(result.success)
            self.assertEqual(result.steps[0].status.value, "failed")
            self.assertEqual(events[-1].phase, ProgressPhase.FINISHED)
            self.assertEqual(events[-1].status, ProgressStatus.FAILED)
            self.assertIn("APK file not found", events[-1].message or "")

    def test_subprocess_adb_mkdir_p_uses_direct_shell_args(self) -> None:
        seen: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            seen.append(args)
            return subprocess.CompletedProcess(args=args, returncode=0, stdout="", stderr="")

        adb = SubprocessAdb(runner=runner)
        adb.mkdir_p("/sdcard/BIOS")
        adb.mkdir_p("/sdcard/BIOS Files")

        self.assertEqual(seen[0], ["adb", "shell", "mkdir", "-p", "/sdcard/BIOS"])
        self.assertEqual(seen[1], ["adb", "shell", "mkdir", "-p", "/sdcard/BIOS Files"])

    def test_runner_executes_supported_alpha_steps(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            apk_path = root / "retroarch.apk"
            apk_path.write_bytes(b"")
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")

            plan = build_execution_plan(apk_path, bios_dir)
            adb = DryRunAdb()
            runner = ExecutorRunner(adb=adb, workdir=root)
            result = runner.run(plan)

            self.assertTrue(result.success)
            self.assertEqual(result.total_steps, 3)
            self.assertEqual([record.status.value for record in result.steps], ["executed", "executed", "executed"])
            self.assertIn("/sdcard/BIOS/scph1001.bin", adb.remote_paths)

    def test_runner_rejects_plan_with_ref_payload(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            plan = build_execution_plan(root / "retroarch.apk", root)
            plan = ExecutionPlan(
                id=plan.id,
                source=plan.source,
                device_context=plan.device_context,
                runtime_capabilities=plan.runtime_capabilities,
                inputs_resolved=plan.inputs_resolved,
                steps=(
                    ExecutionStep(
                        id=plan.steps[0].id,
                        recipe_ref=plan.steps[0].recipe_ref,
                        type=plan.steps[0].type,
                        name=plan.steps[0].name,
                        params={"app": {"ref": "bad.$ref"}, "replace_existing": False},
                        skip_if=plan.steps[0].skip_if,
                        verify=plan.steps[0].verify,
                    ),
                ),
            )
            runner = ExecutorRunner(adb=DryRunAdb(), workdir=root)
            with self.assertRaises(ValueError):
                runner.run(plan)


def build_execution_plan(apk_path: Path, bios_dir: Path) -> ExecutionPlan:
    capabilities = RuntimeCapabilities(
        adb_available=True,
        apk_install=True,
        shared_storage_write=True,
        app_launch=True,
        shell_command=True,
        package_remove_for_user=True,
        root_shell=False,
        app_data_write=False,
    )
    return ExecutionPlan(
        id="plan.test",
        source=ExecutionPlanSource(
            device_profile_ref="ayaneo.generic",
            device_plan_ref="ayaneo.generic.base",
            selected_recipe_refs=("app.retroarch.provision", "feature.copy_bios"),
            expanded_recipe_refs=("app.retroarch.provision", "feature.copy_bios"),
        ),
        device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
        runtime_capabilities=capabilities,
        inputs_resolved=(ResolvedInputValue(id="feature.copy_bios.$bios_source_dir", value=str(bios_dir)),),
        steps=(
            ExecutionStep(
                id="app.retroarch.provision/install_retroarch",
                recipe_ref="app.retroarch.provision",
                type=StepType.INSTALL_APK,
                name="Install RetroArch",
                params={"app": str(apk_path), "replace_existing": False},
                skip_if=(),
                verify=(),
            ),
            ExecutionStep(
                id="feature.copy_bios/copy_bios_dir",
                recipe_ref="feature.copy_bios",
                type=StepType.COPY_BYO_INPUT,
                name="Copy BIOS folder",
                params={"input": str(bios_dir), "dest": "/sdcard/BIOS", "copy_policy": "sync"},
                skip_if=(),
                verify=(StepCondition(type="path_exists", params={"path": "/sdcard/BIOS/scph1001.bin"}),),
            ),
            ExecutionStep(
                id="app.retroarch.provision/launch_retroarch",
                recipe_ref="app.retroarch.provision",
                type=StepType.LAUNCH_APP,
                name="Launch RetroArch",
                params={"package_name": "com.retroarch"},
                skip_if=(StepCondition(type="package_installed", params={"package_name": "com.example.skip"}),),
                verify=(),
            ),
        ),
    )

def _single_step_plan(step: ExecutionStep) -> ExecutionPlan:
    capabilities = RuntimeCapabilities(
        adb_available=True,
        apk_install=True,
        shared_storage_write=True,
        app_launch=True,
        shell_command=True,
        package_remove_for_user=True,
        root_shell=False,
        app_data_write=False,
    )
    return ExecutionPlan(
        id="plan.single",
        source=ExecutionPlanSource(
            device_profile_ref="ayaneo.generic",
            device_plan_ref="ayaneo.generic.base",
            selected_recipe_refs=(step.recipe_ref,),
            expanded_recipe_refs=(step.recipe_ref,),
        ),
        device_context=DeviceContext(manufacturer="AYANEO", model="Pocket 4 Pro", android_version=13),
        runtime_capabilities=capabilities,
        inputs_resolved=(),
        steps=(step,),
    )


def _copy_byo_input_step(source_dir: Path, copy_policy: str) -> ExecutionStep:
    return ExecutionStep(
        id="feature.copy_bios/copy_bios_dir",
        recipe_ref="feature.copy_bios",
        type=StepType.COPY_BYO_INPUT,
        name="Copy BIOS folder",
        params={"input": str(source_dir), "dest": "/sdcard/BIOS", "copy_policy": copy_policy},
        skip_if=(),
        verify=(),
    )


def _build_nested_source_tree(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    (path / "scph1001.bin").write_text("bios", encoding="utf-8")
    nested = path / "nested"
    nested.mkdir()
    (nested / "inner.cfg").write_text("cfg", encoding="utf-8")
    return path


if __name__ == "__main__":
    unittest.main()

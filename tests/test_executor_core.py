from __future__ import annotations

from pathlib import Path
from tempfile import TemporaryDirectory
import shlex
import ssl
import subprocess
import unittest
import zipfile
from unittest.mock import patch

from emuchef.domain import (
    ArtifactCacheMode,
    ArtifactType,
    DeviceContext,
    ExecutionArtifact,
    ExecutionInputValue,
    ExecutionPermissionPlan,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    LiteralParamValue,
    PermissionPlanAction,
    PermissionPlanReason,
    PermissionPlanSource,
    PermissionPolicy,
    RefParamValue,
    RuntimeCapabilities,
    RuntimeValue,
    RuntimeValueType,
    StepCondition,
    StepConstraints,
    StepRuntimeState,
    StepRuntimeStatus,
    StepType,
    ExecutionState,
    ArtifactRuntimeState,
    ArtifactRuntimeStatus,
)
from emuchef.executor import DryRunAdb, ExecutorRunner, ProgressPhase, ProgressStatus, SubprocessAdb
from emuchef.executor.resolver import RefResolutionError, resolve_runtime_ref


def _runtime_capabilities() -> RuntimeCapabilities:
    return RuntimeCapabilities(
        adb_available=True,
        apk_install=True,
        shared_storage_write=True,
        app_launch=True,
        shell_command=True,
        package_remove_for_user=False,
        root_shell=True,
        app_data_write=True,
    )


def _device_context() -> DeviceContext:
    return DeviceContext(
        manufacturer="Example",
        model="Example",
        android_version=13,
        android_api_level=33,
        device_tags=(),
    )


def _base_plan(*, inputs=(), artifacts=(), steps=(), permission_plan=None, runtime_capabilities=None) -> ExecutionPlan:
    return ExecutionPlan(
        id="plan.test",
        source=ExecutionPlanSource(
            device_profile_ref="example.device_profile",
            device_plan_ref="example.device_plan",
            selected_recipe_refs=("example.recipe",),
            expanded_recipe_refs=("example.recipe",),
        ),
        device_context=_device_context(),
        runtime_capabilities=runtime_capabilities or _runtime_capabilities(),
        inputs=tuple(inputs),
        artifacts=tuple(artifacts),
        steps=tuple(steps),
        permission_plan=permission_plan,
    )


class ExecutorCoreTests(unittest.TestCase):
    def test_resolver_handles_inputs_artifacts_and_step_outputs(self) -> None:
        state = ExecutionState(
            inputs={"example.recipe/config": RuntimeValue(type=RuntimeValueType.FILE_PATH, value="/tmp/config.cfg", location="host")},
            artifacts={
                "example.recipe/archive": ArtifactRuntimeState(
                    artifact_id="example.recipe/archive",
                    status=ArtifactRuntimeStatus.RESOLVED,
                    local_path="/tmp/archive.zip",
                    resolved_url="file:///tmp/archive.zip",
                    filename="archive.zip",
                    cache_hit=True,
                )
            },
            steps={
                "example.recipe/extract": StepRuntimeState(
                    step_id="example.recipe/extract",
                    status=StepRuntimeStatus.SUCCEEDED,
                    outputs={
                        "extracted_paths": RuntimeValue(
                            type=RuntimeValueType.PATH_LIST,
                            value=["/tmp/out/a", "/tmp/out/b"],
                            location="host",
                        )
                    },
                )
            },
        )
        self.assertEqual(
            resolve_runtime_ref(state, "inputs.example.recipe/config"),
            RuntimeValue(type=RuntimeValueType.FILE_PATH, value="/tmp/config.cfg", location="host"),
        )
        self.assertEqual(
            resolve_runtime_ref(state, "artifacts.example.recipe/archive.local_path"),
            RuntimeValue(type=RuntimeValueType.FILE_PATH, value="/tmp/archive.zip", location="host"),
        )
        self.assertEqual(
            resolve_runtime_ref(state, "steps.example.recipe/extract.outputs.extracted_paths"),
            RuntimeValue(type=RuntimeValueType.PATH_LIST, value=["/tmp/out/a", "/tmp/out/b"], location="host"),
        )

    def test_resolver_raises_when_step_output_is_unavailable(self) -> None:
        state = ExecutionState(
            inputs={},
            artifacts={},
            steps={"example.recipe/extract": StepRuntimeState(step_id="example.recipe/extract", status=StepRuntimeStatus.SKIPPED)},
        )
        with self.assertRaises(RefResolutionError) as context:
            resolve_runtime_ref(state, "steps.example.recipe/extract.outputs.extracted_paths")
        self.assertEqual(context.exception.code.value, "step_output_unavailable")

    def test_permission_actions_are_not_auto_executed_without_grant_step(self) -> None:
        plan = _base_plan(
            steps=(
                ExecutionStep(
                    id="example.recipe/wait",
                    recipe_ref="example.recipe",
                    type=StepType.WAIT,
                    name="Wait",
                    params={"duration_ms": LiteralParamValue(value=1)},
                ),
            ),
            permission_plan=ExecutionPermissionPlan(
                actions=(
                    PermissionPlanAction(
                        status="applicable",
                        kind="runtime_permission",
                        package_name="com.example.app",
                        permission="android.permission.POST_NOTIFICATIONS",
                        required=False,
                        source=PermissionPlanSource(recipe_id="example.recipe", section="permissions.runtime[0]"),
                    ),
                ),
                policies={"example.recipe": PermissionPolicy()},
            ),
        )
        adb = DryRunAdb()
        result = ExecutorRunner(adb=adb, sleep_fn=lambda _: None).run(plan)
        self.assertTrue(result.success)
        self.assertEqual(result.permission_results, ())
        self.assertFalse(any(command[0] == "run_plan_command" for command in adb.commands), adb.commands)

    def test_grant_permissions_executes_only_matching_recipe_actions(self) -> None:
        plan = _base_plan(
            steps=(
                ExecutionStep(
                    id="example.recipe/grant",
                    recipe_ref="example.recipe",
                    type=StepType.GRANT_PERMISSIONS,
                    name="Grant",
                ),
            ),
            permission_plan=ExecutionPermissionPlan(
                actions=(
                    PermissionPlanAction(
                        status="applicable",
                        kind="runtime_permission",
                        package_name="com.example.app",
                        permission="android.permission.POST_NOTIFICATIONS",
                        required=False,
                        source=PermissionPlanSource(recipe_id="example.recipe", section="permissions.runtime[0]"),
                    ),
                    PermissionPlanAction(
                        status="not_applicable",
                        kind="appop",
                        package_name="com.example.app",
                        op="MANAGE_EXTERNAL_STORAGE",
                        desired_mode="allow",
                        required=False,
                        source=PermissionPlanSource(recipe_id="example.recipe", section="permissions.appops[0]"),
                        reason=PermissionPlanReason(code="requires_root", message="Device is not rooted."),
                    ),
                    PermissionPlanAction(
                        status="applicable",
                        kind="runtime_permission",
                        package_name="com.other.app",
                        permission="android.permission.CAMERA",
                        required=False,
                        source=PermissionPlanSource(recipe_id="other.recipe", section="permissions.runtime[0]"),
                    ),
                ),
                policies={"example.recipe": PermissionPolicy(on_failure="warn", require_all=False)},
            ),
        )
        adb = DryRunAdb()
        result = ExecutorRunner(adb=adb).run(plan)
        self.assertTrue(result.success, result)
        self.assertEqual([record.status.value for record in result.permission_results], ["executed", "not_applicable"])
        self.assertEqual([record.kind for record in result.permission_results], ["runtime_permission", "appop"])
        self.assertTrue(any(command[:4] == ("run_plan_command", "adb", "shell", "pm") for command in adb.commands), adb.commands)
        self.assertFalse(any("com.other.app" in command for command in adb.commands), adb.commands)

    def test_grant_permissions_succeeds_cleanly_with_zero_actions(self) -> None:
        plan = _base_plan(
            steps=(
                ExecutionStep(
                    id="example.recipe/grant",
                    recipe_ref="example.recipe",
                    type=StepType.GRANT_PERMISSIONS,
                    name="Grant",
                ),
            ),
            permission_plan=ExecutionPermissionPlan(actions=(), policies={"example.recipe": PermissionPolicy()}),
        )
        adb = DryRunAdb()
        result = ExecutorRunner(adb=adb).run(plan)
        self.assertTrue(result.success, result)
        self.assertEqual(result.permission_results, ())
        self.assertFalse(any(command[0] == "run_plan_command" for command in adb.commands), adb.commands)

    def test_wait_uses_millisecond_sleep_with_float_seconds(self) -> None:
        observed: list[float] = []
        plan = _base_plan(
            steps=(
                ExecutionStep(
                    id="example.recipe/wait",
                    recipe_ref="example.recipe",
                    type=StepType.WAIT,
                    name="Wait",
                    params={"duration_ms": LiteralParamValue(value=1500)},
                ),
            )
        )
        result = ExecutorRunner(adb=DryRunAdb(), sleep_fn=observed.append).run(plan)
        self.assertTrue(result.success)
        self.assertEqual(observed, [1.5])

    def test_grouped_extract_and_copy_flow_succeeds(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zip_a = tmp_path / "a.zip"
            zip_b = tmp_path / "b.zip"
            self._write_zip(zip_a, {"core_a.so": "alpha"})
            self._write_zip(zip_b, {"core_b.so": "beta"})

            plan = _base_plan(
                artifacts=(
                    ExecutionArtifact(id="example.recipe/a_zip", type=ArtifactType.REMOTE_FILE, url=zip_a.resolve().as_uri(), cache=ArtifactCacheMode.NONE),
                    ExecutionArtifact(id="example.recipe/b_zip", type=ArtifactType.REMOTE_FILE, url=zip_b.resolve().as_uri(), cache=ArtifactCacheMode.NONE),
                ),
                steps=(
                    ExecutionStep(
                        id="example.recipe/resolve",
                        recipe_ref="example.recipe",
                        type=StepType.RESOLVE_ARTIFACTS,
                        name="Resolve",
                        params={"artifacts": LiteralParamValue(value=["example.recipe/a_zip", "example.recipe/b_zip"])},
                    ),
                    ExecutionStep(
                        id="example.recipe/extract",
                        recipe_ref="example.recipe",
                        type=StepType.EXTRACT_ARTIFACTS,
                        name="Extract",
                        dependencies=("example.recipe/resolve",),
                        params={
                            "artifacts": LiteralParamValue(value=["example.recipe/a_zip", "example.recipe/b_zip"]),
                            "extract_on": LiteralParamValue(value="host"),
                        },
                    ),
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        dependencies=("example.recipe/extract",),
                        params={
                            "source": RefParamValue(ref="steps.example.recipe/extract.outputs.extracted_paths"),
                            "dest": LiteralParamValue(value="/sdcard/RetroArch/cores"),
                            "copy_policy": LiteralParamValue(value="sync"),
                        },
                    ),
                ),
            )
            adb = DryRunAdb()
            result = ExecutorRunner(adb=adb, workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            self.assertEqual([record.status.value for record in result.steps], ["executed", "executed", "executed"])
            pushed = [command for command in adb.commands if command[0] in {"push", "push_sync"}]
            self.assertEqual(len(pushed), 2, adb.commands)

    def test_copy_single_file_uses_exact_target_when_dest_is_not_directory(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            cfg = tmp_path / "retroarch.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")
            target = "/sdcard/Android/data/com.example.app/files/retroarch.cfg"
            plan = _base_plan(
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.FILE_PATH,
                                    value=str(cfg),
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value=target),
                        },
                        verify=(StepCondition(type="file_exists", params={"path": target}),),
                    ),
                ),
            )
            adb = DryRunAdb()
            result = ExecutorRunner(adb=adb, workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            self.assertTrue(any(command[:2] == ("push", str(cfg)) and command[2] == target for command in adb.commands), adb.commands)

    def test_copy_single_file_uses_dest_basename_when_dest_exists_as_directory(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            cfg = tmp_path / "retroarch.cfg"
            cfg.write_text("video_driver = gl\n", encoding="utf-8")
            dest_dir = "/sdcard/Android/data/com.example.app/files"
            expected_target = f"{dest_dir}/retroarch.cfg"
            plan = _base_plan(
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.FILE_PATH,
                                    value=str(cfg),
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value=dest_dir),
                        },
                        verify=(StepCondition(type="file_exists", params={"path": expected_target}),),
                    ),
                ),
            )
            adb = DryRunAdb()
            adb.remote_paths.add(dest_dir)
            adb.remote_dirs.add(dest_dir)
            result = ExecutorRunner(adb=adb, workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            self.assertTrue(
                any(command[:2] == ("push", str(cfg)) and command[2] == expected_target for command in adb.commands),
                adb.commands,
            )

    def test_copy_host_path_list_to_app_private_uses_staging_and_privileged_copy(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            core_a = tmp_path / "core_a.so"
            core_b = tmp_path / "core_b.so"
            core_a.write_text("alpha", encoding="utf-8")
            core_b.write_text("beta", encoding="utf-8")
            plan = _base_plan(
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.PATH_LIST,
                                    value=[str(core_a), str(core_b)],
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value="/data/user/0/com.example.app/cores"),
                            "copy_policy": LiteralParamValue(value="sync"),
                        },
                        verify=(StepCondition(type="path_exists", params={"path": "/data/user/0/com.example.app/cores"}),),
                    ),
                ),
            )
            adb = DryRunAdb()
            result = ExecutorRunner(adb=adb, workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            self.assertTrue(any(command[0] == "copy_on_device" and command[-1] == "True" for command in adb.commands), adb.commands)
            self.assertTrue(any(command[:2] == ("mkdir_p", "/data/user/0/com.example.app/cores") and command[2] == "True" for command in adb.commands), adb.commands)
            self.assertTrue(any(command[:2] == ("path_exists", "/data/user/0/com.example.app/cores") and command[2] == "True" for command in adb.commands), adb.commands)
            self.assertFalse(any(command[0] == "push_sync" for command in adb.commands), adb.commands)

    def test_copy_host_file_to_app_private_exact_target_succeeds_with_verify(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            core = tmp_path / "gambatte_libretro_android.so"
            core.write_text("alpha", encoding="utf-8")
            plan = _base_plan(
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.FILE_PATH,
                                    value=str(core),
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value="/data/user/0/com.example.app/cores/gambatte_libretro_android.so"),
                            "copy_policy": LiteralParamValue(value="replace"),
                        },
                        verify=(StepCondition(type="file_exists", params={"path": "/data/user/0/com.example.app/cores/gambatte_libretro_android.so"}),),
                    ),
                ),
            )
            adb = DryRunAdb()
            result = ExecutorRunner(adb=adb, workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            self.assertTrue(
                any(
                    command[:3] == (
                        "remove_file",
                        "/data/user/0/com.example.app/cores/gambatte_libretro_android.so",
                        "True",
                    )
                    for command in adb.commands
                ),
                adb.commands,
            )
            self.assertTrue(
                any(
                    command[:2] == (
                        "path_is_dir",
                        "/data/user/0/com.example.app/cores/gambatte_libretro_android.so",
                    )
                    and command[2] == "True"
                    for command in adb.commands
                ),
                adb.commands,
            )

    def test_copy_device_source_to_app_private_uses_privileged_device_copy(self) -> None:
        plan = _base_plan(
            steps=(
                ExecutionStep(
                    id="example.recipe/copy",
                    recipe_ref="example.recipe",
                    type=StepType.COPY_FILES,
                    name="Copy",
                    params={
                        "source": LiteralParamValue(
                            value=RuntimeValue(
                                type=RuntimeValueType.DIRECTORY_PATH,
                                value="/data/local/tmp/emuchef/extracted",
                                location="device",
                            )
                        ),
                        "dest": LiteralParamValue(value="/data/user/0/com.example.app/cores"),
                    },
                ),
            ),
        )
        adb = DryRunAdb()
        adb.remote_paths.add("/data/local/tmp/emuchef/extracted")
        adb.remote_dirs.add("/data/local/tmp/emuchef/extracted")
        result = ExecutorRunner(adb=adb).run(plan)
        self.assertTrue(result.success, result)
        self.assertTrue(
            any(
                command[:3] == ("copy_on_device", "/data/local/tmp/emuchef/extracted/.", "/data/user/0/com.example.app/cores")
                and command[4] == "True"
                for command in adb.commands
            ),
            adb.commands,
        )

    def test_app_private_copy_fails_cleanly_without_root_backed_capability(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            core = tmp_path / "core.so"
            core.write_text("alpha", encoding="utf-8")
            limited_capabilities = RuntimeCapabilities(
                adb_available=True,
                apk_install=True,
                shared_storage_write=True,
                app_launch=True,
                shell_command=True,
                package_remove_for_user=False,
                root_shell=False,
                app_data_write=True,
            )
            plan = _base_plan(
                runtime_capabilities=limited_capabilities,
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.FILE_PATH,
                                    value=str(core),
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value="/data/user/0/com.example.app/cores/core.so"),
                        },
                    ),
                ),
            )
            result = ExecutorRunner(adb=DryRunAdb(), workdir=tmp_path).run(plan)
            self.assertFalse(result.success)
            self.assertEqual([record.status.value for record in result.steps], ["failed"])
            self.assertIn("app_data_write_unavailable", result.steps[0].message or "")

    def test_failed_dependency_blocks_downstream_step(self) -> None:
        with TemporaryDirectory() as tmp:
            file_path = Path(tmp) / "config.cfg"
            file_path.write_text("config", encoding="utf-8")
            plan = _base_plan(
                inputs=(ExecutionInputValue(id="example.recipe/config", value=RuntimeValue(type=RuntimeValueType.FILE_PATH, value=str(file_path), location="host")),),
                steps=(
                    ExecutionStep(
                        id="example.recipe/fail",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Fail",
                        params={
                            "source": RefParamValue(ref="inputs.example.recipe/missing"),
                            "dest": LiteralParamValue(value="/sdcard/fail.cfg"),
                        },
                    ),
                    ExecutionStep(
                        id="example.recipe/downstream",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Downstream",
                        dependencies=("example.recipe/fail",),
                        params={
                            "source": RefParamValue(ref="inputs.example.recipe/config"),
                            "dest": LiteralParamValue(value="/sdcard/downstream.cfg"),
                        },
                    ),
                ),
            )
            result = ExecutorRunner(adb=DryRunAdb(), workdir=tmp).run(plan)
            self.assertEqual([record.status.value for record in result.steps], ["failed", "blocked"])

    def test_blocked_step_does_not_attempt_param_resolution(self) -> None:
        plan = _base_plan(
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
                    type=StepType.COPY_FILES,
                    name="Downstream",
                    dependencies=("example.recipe/fail",),
                    params={
                        "source": RefParamValue(ref="inputs.example.recipe/missing"),
                        "dest": LiteralParamValue(value="/sdcard/downstream.cfg"),
                    },
                ),
            ),
        )
        result = ExecutorRunner(adb=DryRunAdb()).run(plan)
        self.assertEqual([record.status.value for record in result.steps], ["failed", "blocked"])
        self.assertIn("dependency blocked", result.steps[1].message or "")

    def test_skipped_dependency_does_not_auto_block_downstream_step(self) -> None:
        with TemporaryDirectory() as tmp:
            config_path = Path(tmp) / "config.cfg"
            config_path.write_text("config", encoding="utf-8")
            adb = DryRunAdb()
            adb.installed_packages.add("com.example.skip")
            plan = _base_plan(
                inputs=(ExecutionInputValue(id="example.recipe/config", value=RuntimeValue(type=RuntimeValueType.FILE_PATH, value=str(config_path), location="host")),),
                steps=(
                    ExecutionStep(
                        id="example.recipe/skipped",
                        recipe_ref="example.recipe",
                        type=StepType.LAUNCH_APP,
                        name="Skipped",
                        skip_if=(StepCondition(type="package_installed", params={"package_name": "com.example.skip"}),),
                        params={"package_name": LiteralParamValue(value="com.example.skip")},
                    ),
                    ExecutionStep(
                        id="example.recipe/copy",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy",
                        dependencies=("example.recipe/skipped",),
                        params={
                            "source": RefParamValue(ref="inputs.example.recipe/config"),
                            "dest": LiteralParamValue(value="/sdcard/config.cfg"),
                        },
                        verify=(StepCondition(type="file_exists", params={"path": "/sdcard/config.cfg"}),),
                    ),
                ),
            )
            result = ExecutorRunner(adb=adb, workdir=tmp).run(plan)
            self.assertEqual([record.status.value for record in result.steps], ["skipped", "executed"])

    def test_progress_events_report_blocked_status(self) -> None:
        events = []
        plan = _base_plan(
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
                    params={"duration_ms": LiteralParamValue(value=10)},
                ),
            ),
        )
        ExecutorRunner(adb=DryRunAdb(), sleep_fn=lambda _: None).run(plan, progress_callback=events.append)
        finished_events = [event for event in events if event.phase is ProgressPhase.FINISHED]
        self.assertEqual([event.status for event in finished_events], [ProgressStatus.FAILED, ProgressStatus.BLOCKED])

    def test_artifact_tls_failure_is_typed_and_blocks_downstream_permissions(self) -> None:
        plan = _base_plan(
            artifacts=(
                ExecutionArtifact(
                    id="example.recipe/archive",
                    type=ArtifactType.REMOTE_FILE,
                    url="https://example.invalid/archive.zip",
                    cache=ArtifactCacheMode.NONE,
                ),
            ),
            steps=(
                ExecutionStep(
                    id="example.recipe/resolve",
                    recipe_ref="example.recipe",
                    type=StepType.RESOLVE_ARTIFACTS,
                    name="Resolve",
                    params={"artifacts": LiteralParamValue(value=["example.recipe/archive"])},
                ),
                ExecutionStep(
                    id="example.recipe/grant",
                    recipe_ref="example.recipe",
                    type=StepType.GRANT_PERMISSIONS,
                    name="Grant",
                    dependencies=("example.recipe/resolve",),
                ),
            ),
            permission_plan=ExecutionPermissionPlan(
                actions=(
                    PermissionPlanAction(
                        status="applicable",
                        kind="runtime_permission",
                        package_name="com.example.app",
                        permission="android.permission.POST_NOTIFICATIONS",
                        required=False,
                        source=PermissionPlanSource(recipe_id="example.recipe", section="permissions.runtime[0]"),
                    ),
                ),
                policies={"example.recipe": PermissionPolicy()},
            ),
        )
        with patch(
            "emuchef.executor.step_handlers.urllib.request.urlopen",
            side_effect=ssl.SSLCertVerificationError("certificate verify failed"),
        ):
            result = ExecutorRunner(adb=DryRunAdb()).run(plan)
        self.assertFalse(result.success)
        self.assertEqual([record.status.value for record in result.steps], ["failed", "blocked"])
        self.assertIn("tls_verification_failed", result.steps[0].message or "")
        self.assertEqual(result.permission_results, ())

    def test_artifact_download_failure_is_typed(self) -> None:
        plan = _base_plan(
            artifacts=(
                ExecutionArtifact(
                    id="example.recipe/archive",
                    type=ArtifactType.REMOTE_FILE,
                    url="https://example.invalid/archive.zip",
                    cache=ArtifactCacheMode.NONE,
                ),
            ),
            steps=(
                ExecutionStep(
                    id="example.recipe/resolve",
                    recipe_ref="example.recipe",
                    type=StepType.RESOLVE_ARTIFACTS,
                    name="Resolve",
                    params={"artifacts": LiteralParamValue(value=["example.recipe/archive"])},
                ),
            ),
        )
        with patch(
            "emuchef.executor.step_handlers.urllib.request.urlopen",
            side_effect=OSError("connection reset"),
        ):
            result = ExecutorRunner(adb=DryRunAdb()).run(plan)
        self.assertFalse(result.success)
        self.assertEqual([record.status.value for record in result.steps], ["failed"])
        self.assertIn("artifact_download_failed", result.steps[0].message or "")

    def test_launch_app_with_explicit_activity_uses_am_start(self) -> None:
        calls: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            return subprocess.CompletedProcess(args, 0, "", "")

        adb = SubprocessAdb(executable="adb", runner=runner)
        adb.launch_app("com.example.app", ".MainActivity")
        self.assertEqual(
            calls,
            [["adb", "shell", "am", "start", "-n", "com.example.app/.MainActivity"]],
        )

    def test_launch_app_prefers_resolved_activity_before_monkey(self) -> None:
        calls: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            if args[1:] == ["shell", "cmd", "package", "resolve-activity", "--brief", "com.example.app"]:
                return subprocess.CompletedProcess(args, 0, "com.example.app/.MainActivity\n", "")
            return subprocess.CompletedProcess(args, 0, "", "")

        adb = SubprocessAdb(executable="adb", runner=runner)
        adb.launch_app("com.example.app")
        self.assertEqual(
            calls,
            [
                ["adb", "shell", "cmd", "package", "resolve-activity", "--brief", "com.example.app"],
                ["adb", "shell", "am", "start", "-n", "com.example.app/.MainActivity"],
            ],
        )

    def test_launch_app_falls_back_to_monkey_when_resolution_is_unavailable(self) -> None:
        calls: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            if args[1:] in (
                ["shell", "cmd", "package", "resolve-activity", "--brief", "com.example.app"],
                ["shell", "pm", "resolve-activity", "--brief", "com.example.app"],
            ):
                return subprocess.CompletedProcess(args, 1, "", "not found")
            return subprocess.CompletedProcess(args, 0, "", "")

        adb = SubprocessAdb(executable="adb", runner=runner)
        adb.launch_app("com.example.app")
        self.assertEqual(
            calls[-1],
            ["adb", "shell", "monkey", "-p", "com.example.app", "-c", "android.intent.category.LAUNCHER", "1"],
        )

    def test_copy_on_device_privileged_quotes_spaced_paths_for_su(self) -> None:
        calls: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            return subprocess.CompletedProcess(args, 0, "", "")

        source = "/data/local/tmp/emuchef/Amstrad - CPC.rdb"
        dest = "/data/user/0/com.example.app/database/rdb"
        adb = SubprocessAdb(executable="adb", runner=runner)
        adb.copy_on_device(source, dest, privileged=True)
        self.assertEqual(
            calls,
            [["adb", "shell", shlex.join(["su", "-c", shlex.join(["cp", source, dest])])]],
        )

    def test_mkdir_p_quotes_spaced_paths_for_adb_shell(self) -> None:
        calls: list[list[str]] = []

        def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
            calls.append(args)
            return subprocess.CompletedProcess(args, 0, "", "")

        path = "/sdcard/RetroArch/My Config"
        adb = SubprocessAdb(executable="adb", runner=runner)
        adb.mkdir_p(path)
        self.assertEqual(calls, [["adb", "shell", shlex.join(["mkdir", "-p", path])]])

    def test_executor_copy_to_app_private_quotes_spaced_staged_filename(self) -> None:
        with TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            database_dir = tmp_path / "database"
            database_dir.mkdir()
            database = database_dir / "Amstrad - CPC.rdb"
            database.write_text("db", encoding="utf-8")
            calls: list[list[str]] = []

            def runner(args: list[str]) -> subprocess.CompletedProcess[str]:
                calls.append(args)
                return subprocess.CompletedProcess(args, 0, "", "")

            plan = _base_plan(
                steps=(
                    ExecutionStep(
                        id="example.recipe/copy_database_rdb",
                        recipe_ref="example.recipe",
                        type=StepType.COPY_FILES,
                        name="Copy database",
                        params={
                            "source": LiteralParamValue(
                                value=RuntimeValue(
                                    type=RuntimeValueType.DIRECTORY_PATH,
                                    value=str(database_dir),
                                    location="host",
                                )
                            ),
                            "dest": LiteralParamValue(value="/data/user/0/com.example.app/database/rdb"),
                        },
                    ),
                ),
            )

            result = ExecutorRunner(adb=SubprocessAdb(executable="adb", runner=runner), workdir=tmp_path).run(plan)
            self.assertTrue(result.success, result)
            expected_copy = [
                "adb",
                "shell",
                shlex.join(
                    [
                        "su",
                        "-c",
                        shlex.join(
                            [
                                "cp",
                                "/data/local/tmp/emuchef/example.recipe_copy_database_rdb/Amstrad - CPC.rdb",
                                "/data/user/0/com.example.app/database/rdb",
                            ]
                        ),
                    ]
                ),
            ]
            self.assertIn(expected_copy, calls)

    @staticmethod
    def _write_zip(path: Path, entries: dict[str, str]) -> None:
        with zipfile.ZipFile(path, "w") as handle:
            for name, contents in entries.items():
                handle.writestr(name, contents)


if __name__ == "__main__":
    unittest.main()

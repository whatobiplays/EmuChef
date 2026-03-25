from __future__ import annotations

import io
import os
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

import yaml

from emuchef.cli import main
from emuchef.domain import (
    DeviceContext,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    ResolvedInputValue,
    RuntimeCapabilities,
    StepCondition,
    StepType,
)
from emuchef.executor import AdbResolutionError, DetectedDevice
from emuchef.io import dump_yaml


def write_yaml(path: Path, payload: dict) -> None:
    path.write_text(yaml.safe_dump(payload), encoding="utf-8")


class CliTests(unittest.TestCase):
    def test_draft_warns_when_selected_device_plan_profile_mismatches_connected_device(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    self.serial = serial
                    self.executable = executable

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="OtherCorp",
                        model="Other Handheld",
                        android_version=14,
                        root_available=False,
                        brand="OtherBrand",
                    )

            with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                rc, stdout, _ = run_cli(
                    [
                        "draft",
                        "--authored-root",
                        str(authored_root),
                        "--device-plan",
                        "ayaneo.generic.base",
                    ]
                )

            self.assertEqual(rc, 0)
            self.assertIn("Warnings:", stdout)
            self.assertIn("device_profile_mismatch:", stdout)

    def test_plan_omits_profile_mismatch_warning_when_connected_device_matches(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)
            ops_file = root / "ops.yaml"
            write_yaml(
                ops_file,
                [
                    {"type": "deselect_recipe", "recipe_ref": "feature.copy_bios"},
                ],
            )

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    self.serial = serial
                    self.executable = executable

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="AYANEO",
                        model="Pocket 4 Pro",
                        android_version=13,
                        root_available=False,
                        brand="AYANEO",
                    )

            with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                rc, stdout, _ = run_cli(
                    [
                        "plan",
                        "--authored-root",
                        str(authored_root),
                        "--device-plan",
                        "ayaneo.generic.base",
                        "--ops",
                        str(ops_file),
                    ]
                )

            self.assertEqual(rc, 0)
            self.assertIn("Planning status: success", stdout)
            self.assertNotIn("device_profile_mismatch", stdout)

    def test_draft_rejects_device_profile_ref_for_device_plan(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)

            rc, _, stderr = run_cli(
                [
                    "draft",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                ]
            )

            self.assertEqual(rc, 1)
            self.assertIn("Unknown device plan: ayaneo.generic", stderr)
            self.assertIn("is a device profile, not a device plan", stderr)
            self.assertIn("Matching device plans: ayaneo.generic.base", stderr)

    def test_draft_summary_shows_selected_auto_included_unavailable_and_unresolved(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)

            rc, stdout, _ = run_cli(
                [
                    "draft",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic.base",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                ]
            )

            self.assertEqual(rc, 0)
            self.assertIn("Selected recipes:", stdout)
            self.assertIn("- app.main", stdout)
            self.assertIn("- feature.copy_bios", stdout)
            self.assertIn("Auto-included recipes:", stdout)
            self.assertIn("- app.dep", stdout)
            self.assertIn("Selected steps:", stdout)
            self.assertIn("- app.main/install_main", stdout)
            self.assertIn("Unavailable steps:", stdout)
            self.assertIn("app.main/push_extra_config: This step requires app_data_write", stdout)
            self.assertIn("Unresolved required inputs:", stdout)
            self.assertIn("feature.copy_bios.$bios_source_dir (BIOS Folder)", stdout)

    def test_draft_ops_file_replay_uses_planner_operations(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)
            ops_file = root / "ops.yaml"
            write_yaml(
                ops_file,
                [
                    {"type": "deselect_recipe", "recipe_ref": "feature.copy_bios"},
                ],
            )

            rc, stdout, _ = run_cli(
                [
                    "draft",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic.base",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                    "--ops",
                    str(ops_file),
                ]
            )

            self.assertEqual(rc, 0)
            self.assertNotIn("- feature.copy_bios", stdout)
            self.assertIn("Auto-included recipes:", stdout)
            self.assertIn("- app.dep", stdout)
            self.assertIn("Unresolved required inputs:\n- (none)", stdout)

    def test_plan_emits_absolute_artifact_paths(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            rc, stdout, _ = run_cli(
                [
                    "plan",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic.base",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                    "--bind",
                    f"feature.copy_bios.$bios_source_dir={bios_dir}",
                    "--verbose",
                ]
            )

            self.assertEqual(rc, 0)
            payload = yaml.safe_load(stdout)
            steps = {step["id"]: step for step in payload["execution_plan"]["steps"]}
            self.assertEqual(
                steps["app.dep/install_dep"]["params"]["app"],
                str((root / "assets" / "dep.apk").resolve()),
            )
            self.assertEqual(
                steps["app.main/install_main"]["params"]["app"],
                str((root / "assets" / "main.apk").resolve()),
            )
            self.assertEqual(steps["feature.copy_bios/copy_bios_dir"]["params"]["copy_policy"], "sync")

    def test_detect_command_outputs_summary_and_verbose_yaml(self) -> None:
        fake_device = DetectedDevice(
            serial="emulator-5554",
            manufacturer="AYANEO",
            model="Pocket 4 Pro",
            android_version=13,
            root_available=False,
            brand="AYANEO",
        )

        class FakeAdb:
            def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                self.serial = serial
                self.executable = executable

            def detect_device(self) -> DetectedDevice:
                return fake_device

        with patch("emuchef.cli.SubprocessAdb", FakeAdb):
            rc, stdout, _ = run_cli(["detect"])
            self.assertEqual(rc, 0)
            self.assertIn("Serial: emulator-5554", stdout)
            self.assertIn("Root available: no", stdout)

            rc, stdout, _ = run_cli(["detect", "--verbose"])
            self.assertEqual(rc, 0)
            self.assertIn("serial: emulator-5554", stdout)
            self.assertIn("android_version: 13", stdout)

    def test_detect_profiles_matches_pocket_air_mini_when_manufacturer_is_arbor(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    self.serial = serial
                    self.executable = executable

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="ARBOR",
                        model="Pocket Air Mini",
                        android_version=13,
                        root_available=False,
                        brand="AYANEO",
                    )

            with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                rc, stdout, _ = run_cli(
                    [
                        "detect-profiles",
                        "--authored-root",
                        str(authored_root),
                    ]
                )

            self.assertEqual(rc, 0)
            self.assertIn("Matching device profiles:", stdout)
            self.assertIn("- ayaneo.generic", stdout)
            self.assertIn("- ayaneo.pocket_air_mini", stdout)
            self.assertNotIn("- other.generic", stdout)

    def test_detect_profiles_verbose_includes_mismatch_reasons(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    self.serial = serial
                    self.executable = executable

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="AYANEO",
                        model="Pocket Air Mini",
                        android_version=13,
                        root_available=False,
                        brand="AYANEO",
                    )

            with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                rc, stdout, _ = run_cli(
                    [
                        "detect-profiles",
                        "--authored-root",
                        str(authored_root),
                        "--verbose",
                    ]
                )

            self.assertEqual(rc, 0)
            self.assertIn("profile_id: other.generic", stdout)
            self.assertIn("did not contain any of: OtherCorp", stdout)
            self.assertIn("did not contain any of: OtherBrand", stdout)

    def test_cli_adb_flag_overrides_env(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            cli_adb = make_executable(root / "cli-adb")
            env_adb = make_executable(root / "env-adb")
            seen: list[str] = []

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    seen.append(executable)

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="AYANEO",
                        model="Pocket 4 Pro",
                        android_version=13,
                        root_available=False,
                        brand="AYANEO",
                    )

            with patch.dict(os.environ, {"EMUCHEF_ADB": str(env_adb)}, clear=False):
                with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                    rc, _, _ = run_cli(["detect", "--adb", str(cli_adb)])

            self.assertEqual(rc, 0)
            self.assertEqual(seen[-1], str(cli_adb.resolve()))

    def test_env_adb_works(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            env_adb = make_executable(root / "env-adb")
            seen: list[str] = []

            class FakeAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    seen.append(executable)

                def detect_device(self) -> DetectedDevice:
                    return DetectedDevice(
                        serial="device-1",
                        manufacturer="AYANEO",
                        model="Pocket 4 Pro",
                        android_version=13,
                        root_available=False,
                        brand="AYANEO",
                    )

            with patch.dict(os.environ, {"EMUCHEF_ADB": str(env_adb)}, clear=False):
                with patch("emuchef.cli.SubprocessAdb", FakeAdb):
                    rc, _, _ = run_cli(["detect"])

            self.assertEqual(rc, 0)
            self.assertEqual(seen[-1], str(env_adb.resolve()))

    def test_invalid_explicit_adb_path_returns_adb_not_found(self) -> None:
        rc, _, stderr = run_cli(["detect", "--adb", "/does/not/exist/adb"])

        self.assertEqual(rc, 1)
        self.assertIn("adb_not_found:", stderr)
        self.assertIn("--adb", stderr)
        self.assertIn("EMUCHEF_ADB", stderr)
        self.assertIn("PATH", stderr)

    def test_detect_and_apply_fail_without_usable_adb(self) -> None:
        class MissingDetectAdb:
            def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                pass

            def detect_device(self) -> DetectedDevice:
                raise AdbResolutionError(
                    "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                )

        with patch("emuchef.cli.SubprocessAdb", MissingDetectAdb):
            rc, _, stderr = run_cli(["detect"])
            self.assertEqual(rc, 1)
            self.assertIn("adb_not_found:", stderr)

        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            apk_path = root / "retroarch.apk"
            apk_path.write_bytes(b"")
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")
            plan_file = root / "plan.yaml"
            plan_file.write_text(dump_yaml(build_execution_plan(apk_path, bios_dir)), encoding="utf-8")

            class MissingApplyAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    pass

                def install_apk(self, apk_path: Path, replace_existing: bool = False) -> None:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

                def push(self, source: Path, dest: str) -> None:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

                def mkdir_p(self, path: str) -> None:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

                def path_exists(self, path: str) -> bool:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

                def package_installed(self, package_name: str) -> bool:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

                def launch_app(self, package_name: str, activity: str | None = None) -> None:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

            with patch("emuchef.cli.SubprocessAdb", MissingApplyAdb):
                rc, _, stderr = run_cli(["apply", "--plan-file", str(plan_file)])

            self.assertEqual(rc, 1)
            self.assertIn("adb_not_found:", stderr)

    def test_draft_and_plan_still_work_offline(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            class OfflineAdb:
                def __init__(self, serial: str | None = None, executable: str = "adb") -> None:
                    pass

                def detect_device(self) -> DetectedDevice:
                    raise AdbResolutionError(
                        "The configured ADB executable could not be started. Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."
                    )

            with patch("emuchef.cli.SubprocessAdb", OfflineAdb):
                rc, stdout, _ = run_cli(
                    [
                        "draft",
                        "--authored-root",
                        str(authored_root),
                        "--device-plan",
                        "ayaneo.generic.base",
                    ]
                )
                self.assertEqual(rc, 0)
                self.assertIn("Draft:", stdout)

                rc, stdout, _ = run_cli(
                    [
                        "plan",
                        "--authored-root",
                        str(authored_root),
                        "--device-plan",
                        "ayaneo.generic.base",
                        "--ops",
                        str(write_ops(root / "ops.yaml", [{"type": "deselect_recipe", "recipe_ref": "feature.copy_bios"}])),
                    ]
                )
                self.assertEqual(rc, 0)
                self.assertIn("Planning status: success", stdout)

    def test_apply_dry_run_prints_live_progress_and_summary(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            apk_path = root / "retroarch.apk"
            apk_path.write_bytes(b"")
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")
            plan_file = root / "plan.yaml"
            plan_file.write_text(dump_yaml(build_execution_plan(apk_path, bios_dir)), encoding="utf-8")

            rc, stdout, _ = run_cli(["apply", "--plan-file", str(plan_file), "--dry-run"])

            self.assertEqual(rc, 0)
            self.assertIn("[1/3] Install RetroArch: checking skip conditions", stdout)
            self.assertIn("[1/3] Install RetroArch: executing (dry-run)", stdout)
            self.assertIn("[1/3] Install RetroArch: succeeded", stdout)
            self.assertIn("Dry run: success", stdout)
            self.assertIn("- total: 3", stdout)
            self.assertIn("- succeeded: 3", stdout)
            self.assertIn("- skipped: 0", stdout)
            self.assertIn("- failed: 0", stdout)
            self.assertIn("- not run: 0", stdout)

    def test_apply_summary_reports_failure_and_not_run_counts(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            missing_apk = root / "missing.apk"
            bios_dir = root / "bios"
            bios_dir.mkdir()
            (bios_dir / "scph1001.bin").write_text("bios", encoding="utf-8")
            plan_file = root / "plan.yaml"
            plan_file.write_text(dump_yaml(build_execution_plan(missing_apk, bios_dir)), encoding="utf-8")

            rc, stdout, _ = run_cli(["apply", "--plan-file", str(plan_file), "--dry-run"])

            self.assertEqual(rc, 1)
            self.assertIn("[1/3] Install RetroArch: failed", stdout)
            self.assertIn("Dry run: failed", stdout)
            self.assertIn("- total: 3", stdout)
            self.assertIn("- succeeded: 0", stdout)
            self.assertIn("- skipped: 0", stdout)
            self.assertIn("- failed: 1", stdout)
            self.assertIn("- not run: 2", stdout)

    def test_verbose_and_debug_flags_do_not_break_commands(self) -> None:
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            authored_root = build_cli_project_tree(root)
            bios_dir = root / "bios"
            bios_dir.mkdir()

            rc, stdout, _ = run_cli(
                [
                    "plan",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic.base",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                    "--bind",
                    f"feature.copy_bios.$bios_source_dir={bios_dir}",
                    "--verbose",
                ]
            )
            self.assertEqual(rc, 0)
            self.assertIn("kind: planning_result", stdout)

            rc, stdout, stderr = run_cli(
                [
                    "draft",
                    "--authored-root",
                    str(authored_root),
                    "--device-plan",
                    "ayaneo.generic.base",
                    "--manufacturer",
                    "AYANEO",
                    "--model",
                    "Pocket 4 Pro",
                    "--android-version",
                    "13",
                    "--debug",
                ]
            )
            self.assertEqual(rc, 0)
            self.assertIn("Draft:", stdout)
            self.assertIn("DEBUG", stderr)


def run_cli(argv: list[str]) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        rc = main(argv)
    return rc, stdout.getvalue(), stderr.getvalue()


def make_executable(path: Path) -> Path:
    path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    path.chmod(0o755)
    return path


def write_ops(path: Path, payload: list[dict]) -> Path:
    write_yaml(path, payload)
    return path


def build_cli_project_tree(root: Path) -> Path:
    authored_root = root / "authored"
    for subdir in ("apps", "recipes", "device_profiles", "device_plans"):
        (authored_root / subdir).mkdir(parents=True, exist_ok=True)

    assets_dir = root / "assets"
    assets_dir.mkdir()
    (assets_dir / "main.apk").write_bytes(b"")
    (assets_dir / "dep.apk").write_bytes(b"")

    profile = {
        "schema_version": 1,
        "kind": "device_profile",
        "id": "ayaneo.generic",
        "name": "AYANEO Generic",
        "match": {
            "manufacturer_contains": ["AYANEO", "ARBOR"],
            "brand_contains": ["AYANEO"],
            "model_patterns": ["(?i)(ayaneo|pocket)"],
            "android_version": {"min": 11},
        },
        "capability_defaults": {
            "adb_available": True,
            "apk_install": True,
            "shared_storage_write": True,
            "app_launch": True,
            "shell_command": True,
            "package_remove_for_user": True,
            "root_shell": False,
            "app_data_write": False,
        },
        "device_tags": ["handheld_android"],
        "metadata": {},
    }
    pocket_air_profile = {
        "schema_version": 1,
        "kind": "device_profile",
        "id": "ayaneo.pocket_air_mini",
        "name": "AYANEO Pocket Air Mini",
        "match": {
            "manufacturer_contains": ["AYANEO", "ARBOR"],
            "brand_contains": ["AYANEO"],
            "model_patterns": ["(?i)pocket air mini", "GT78-VN"],
            "android_version": {"min": 13},
        },
        "capability_defaults": profile["capability_defaults"],
        "device_tags": ["handheld_android", "pocket_air_mini"],
        "metadata": {},
    }
    other_profile = {
        "schema_version": 1,
        "kind": "device_profile",
        "id": "other.generic",
        "name": "Other Generic",
        "match": {
            "manufacturer_contains": ["OtherCorp"],
            "brand_contains": ["OtherBrand"],
            "model_patterns": ["(?i)other"],
            "android_version": {"min": 14},
        },
        "capability_defaults": profile["capability_defaults"],
        "device_tags": ["other_device"],
        "metadata": {},
    }
    plan = {
        "schema_version": 1,
        "kind": "device_plan",
        "id": "ayaneo.generic.base",
        "name": "Base",
        "device_profile_ref": "ayaneo.generic",
        "recipes": [
            {"recipe_ref": "app.main", "selected_by_default": True},
            {"recipe_ref": "feature.copy_bios", "selected_by_default": True},
        ],
        "defaults": {},
        "overrides": {},
        "metadata": {},
    }
    main_recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "app.main",
        "name": "Main App",
        "recipe_dependencies": ["app.dep"],
        "provides": {"features": ["main"]},
        "inputs": [],
        "steps": [
            {
                "id": "install_main",
                "type": "install_apk",
                "name": "Install Main App",
                "user_toggleable": False,
                "dependencies": [],
                "constraints": {"capabilities": ["apk_install"], "conflicts_with": []},
                "skip_if": [],
                "params": {"app": "assets/main.apk", "replace_existing": False},
                "verify": [],
            },
            {
                "id": "push_extra_config",
                "type": "launch_app",
                "name": "Unavailable extra step",
                "user_toggleable": True,
                "dependencies": [],
                "constraints": {"capabilities": ["app_data_write"], "conflicts_with": []},
                "skip_if": [],
                "params": {"package_name": {"value": "com.example.unavailable"}},
                "verify": [],
            },
        ],
    }
    dependency_recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "app.dep",
        "name": "Dependency App",
        "recipe_dependencies": [],
        "provides": {"features": ["dep"]},
        "inputs": [],
        "steps": [
            {
                "id": "install_dep",
                "type": "install_apk",
                "name": "Install Dependency",
                "user_toggleable": False,
                "dependencies": [],
                "constraints": {"capabilities": ["apk_install"], "conflicts_with": []},
                "skip_if": [],
                "params": {"app": "assets/dep.apk", "replace_existing": False},
                "verify": [],
            }
        ],
    }
    bios_recipe = {
        "schema_version": 1,
        "kind": "recipe",
        "id": "feature.copy_bios",
        "name": "Copy BIOS Files",
        "recipe_dependencies": [],
        "provides": {"features": ["bios_copy"]},
        "inputs": [
            {
                "id": "bios_source_dir",
                "type": "directory",
                "role": "bios",
                "label": "BIOS Folder",
                "required": True,
                "multiple": False,
                "validation": {"must_exist": True, "allowed_extensions": [], "path_kind": "directory"},
                "default": None,
                "metadata": {},
            }
        ],
        "steps": [
            {
                "id": "copy_bios_dir",
                "type": "copy_byo_input",
                "name": "Copy BIOS folder",
                "user_toggleable": True,
                "dependencies": [],
                "constraints": {"capabilities": ["shared_storage_write"], "conflicts_with": []},
                "skip_if": [],
                "params": {
                    "input": {"ref": "feature.copy_bios.$bios_source_dir"},
                    "dest": {"value": "/sdcard/BIOS"},
                    "copy_policy": "sync",
                },
                "verify": [],
            }
        ],
    }

    write_yaml(authored_root / "device_profiles" / "ayaneo.yaml", profile)
    write_yaml(authored_root / "device_profiles" / "ayaneo_pocket_air_mini.yaml", pocket_air_profile)
    write_yaml(authored_root / "device_profiles" / "other.yaml", other_profile)
    write_yaml(authored_root / "device_plans" / "ayaneo_base.yaml", plan)
    write_yaml(authored_root / "recipes" / "app_main.yaml", main_recipe)
    write_yaml(authored_root / "recipes" / "app_dep.yaml", dependency_recipe)
    write_yaml(authored_root / "recipes" / "copy_bios.yaml", bios_recipe)
    return authored_root


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


if __name__ == "__main__":
    unittest.main()

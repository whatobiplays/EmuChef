"""Thin ADB abstraction for real and dry-run execution."""

from __future__ import annotations

import logging
import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from collections.abc import Sequence
from typing import Protocol

from emuchef.domain import ErrorCode

logger = logging.getLogger(__name__)


class AdbRunner(Protocol):
    def __call__(self, args: list[str]) -> subprocess.CompletedProcess[str]:
        ...


@dataclass(frozen=True, slots=True)
class AdbCommandResult:
    args: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str


@dataclass(frozen=True, slots=True)
class DetectedDevice:
    serial: str
    manufacturer: str
    model: str
    android_version: int
    root_available: bool
    android_api_level: int | None = None
    brand: str | None = None


class AdbCommandError(RuntimeError):
    def __init__(self, result: AdbCommandResult) -> None:
        message = f"ADB command failed ({result.returncode}): {' '.join(result.args)}"
        if result.stderr:
            message = f"{message}\n{result.stderr.strip()}"
        super().__init__(message)
        self.result = result


class AdbResolutionError(ValueError):
    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.code = ErrorCode.ADB_NOT_FOUND


class AdbInterface(Protocol):
    def install_apk(self, apk_path: Path, replace_existing: bool = False) -> None:
        ...

    def push(self, source: Path, dest: str) -> None:
        ...

    def push_sync(self, source: Path, dest: str) -> None:
        ...

    def mkdir_p(self, path: str) -> None:
        ...

    def remove_file(self, path: str) -> None:
        ...

    def remove_tree(self, path: str) -> None:
        ...

    def path_exists(self, path: str) -> bool:
        ...

    def package_installed(self, package_name: str) -> bool:
        ...

    def launch_app(self, package_name: str, activity: str | None = None) -> None:
        ...

    def force_stop_app(self, package_name: str) -> None:
        ...

    def run_plan_command(self, command: Sequence[str]) -> None:
        ...


def _default_runner(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, check=False, text=True, capture_output=True)


class SubprocessAdb:
    def __init__(self, serial: str | None = None, executable: str = "adb", runner: AdbRunner | None = None) -> None:
        self._serial = serial
        self._executable = executable
        self._runner = runner or _default_runner

    def install_apk(self, apk_path: Path, replace_existing: bool = False) -> None:
        args = ["install"]
        if replace_existing:
            args.append("-r")
        args.append(str(apk_path))
        self._run(args)

    def push(self, source: Path, dest: str) -> None:
        self._run(["push", str(source), dest])

    def push_sync(self, source: Path, dest: str) -> None:
        self._run(["push", "--sync", str(source), dest])

    def mkdir_p(self, path: str) -> None:
        self._run(["shell", "mkdir", "-p", path])

    def remove_file(self, path: str) -> None:
        self._run(["shell", "rm", "-f", path])

    def remove_tree(self, path: str) -> None:
        self._run(["shell", "rm", "-rf", path])

    def path_exists(self, path: str) -> bool:
        return self._run(["shell", "test", "-e", path], check=False).returncode == 0

    def package_installed(self, package_name: str) -> bool:
        result = self._run(["shell", "pm", "path", package_name], check=False)
        return result.returncode == 0 and "package:" in result.stdout

    def launch_app(self, package_name: str, activity: str | None = None) -> None:
        if activity:
            self._run(["shell", "am", "start", "-n", f"{package_name}/{activity}"])
            return
        self._run(["shell", "monkey", "-p", package_name, "-c", "android.intent.category.LAUNCHER", "1"])

    def force_stop_app(self, package_name: str) -> None:
        self._run(["shell", "am", "force-stop", package_name])

    def run_plan_command(self, command: Sequence[str]) -> None:
        command_args = list(command)
        if not command_args:
            raise ValueError("Plan command must not be empty.")
        if command_args[0] != "adb":
            raise ValueError(f"Plan command must start with 'adb': {command_args!r}")

        tail = command_args[1:]
        if self._serial is not None and not _command_has_serial_flag(tail):
            tail = ["-s", self._serial, *tail]
        self._run_raw([self._executable, *tail])

    def detect_device(self) -> DetectedDevice:
        serial = self._serial or self._select_single_device_serial()
        manufacturer = self._getprop(serial, "ro.product.manufacturer")
        brand = self._getprop(serial, "ro.product.brand")
        model = self._getprop(serial, "ro.product.model")
        release = self._getprop(serial, "ro.build.version.release")
        sdk = self._getprop(serial, "ro.build.version.sdk")
        root_available = self._run_with_serial(serial, ["shell", "su", "-c", "true"], check=False).returncode == 0
        detected = DetectedDevice(
            serial=serial,
            manufacturer=manufacturer or "Unknown",
            model=model or "Unknown",
            android_version=_parse_android_version(release),
            android_api_level=_parse_android_api_level(sdk),
            root_available=root_available,
            brand=brand or None,
        )
        logger.debug("Detected device: %s", detected)
        return detected

    def _run(self, args: list[str], check: bool = True) -> AdbCommandResult:
        return self._run_with_serial(self._serial, args, check=check)

    def _run_with_serial(self, serial: str | None, args: list[str], check: bool = True) -> AdbCommandResult:
        full_args = [self._executable]
        if serial:
            full_args.extend(["-s", serial])
        full_args.extend(args)
        return self._run_raw(full_args, check=check)

    def _run_raw(self, full_args: list[str], check: bool = True) -> AdbCommandResult:
        logger.debug("ADB command: %s", " ".join(full_args))
        try:
            completed = self._runner(full_args)
        except FileNotFoundError as exc:
            raise AdbResolutionError(_adb_not_found_message("The configured ADB executable could not be started.")) from exc
        result = AdbCommandResult(
            args=tuple(full_args),
            returncode=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )
        if check and result.returncode != 0:
            raise AdbCommandError(result)
        return result

    def _select_single_device_serial(self) -> str:
        result = self._run_with_serial(None, ["devices"], check=False)
        devices = tuple(
            line.split("\t", 1)[0]
            for line in result.stdout.splitlines()
            if "\tdevice" in line and not line.startswith("List of devices attached")
        )
        if not devices:
            raise ValueError("No connected ADB devices were detected.")
        if len(devices) > 1:
            raise ValueError("Multiple connected ADB devices were detected. Pass --serial.")
        return devices[0]

    def _getprop(self, serial: str, prop: str) -> str:
        result = self._run_with_serial(serial, ["shell", "getprop", prop], check=False)
        return result.stdout.strip()


class DryRunAdb:
    """In-memory stub ADB for smoke tests and dry-run CLI usage."""

    def __init__(
        self,
        serial: str = "DRY-RUN",
        manufacturer: str = "Unknown",
        model: str = "Unknown",
        android_version: int = 0,
        android_api_level: int | None = None,
        root_available: bool = False,
    ) -> None:
        self.installed_packages: set[str] = set()
        self.remote_paths: set[str] = set()
        self.commands: list[tuple[str, ...]] = []
        self._detected_device = DetectedDevice(
            serial=serial,
            manufacturer=manufacturer,
            model=model,
            android_version=android_version,
            android_api_level=android_api_level,
            root_available=root_available,
        )

    def install_apk(self, apk_path: Path, replace_existing: bool = False) -> None:
        self.commands.append(("install_apk", str(apk_path), str(replace_existing)))

    def push(self, source: Path, dest: str) -> None:
        self.commands.append(("push", str(source), dest))
        self._record_push(source, dest)

    def push_sync(self, source: Path, dest: str) -> None:
        self.commands.append(("push_sync", str(source), dest))
        self._record_push(source, dest)

    def mkdir_p(self, path: str) -> None:
        self.commands.append(("mkdir_p", path))
        self.remote_paths.add(path)

    def remove_file(self, path: str) -> None:
        self.commands.append(("remove_file", path))
        self.remote_paths.discard(path)

    def remove_tree(self, path: str) -> None:
        self.commands.append(("remove_tree", path))
        path_prefix = f"{path.rstrip('/')}/"
        self.remote_paths = {
            remote_path
            for remote_path in self.remote_paths
            if remote_path != path and not remote_path.startswith(path_prefix)
        }

    def path_exists(self, path: str) -> bool:
        self.commands.append(("path_exists", path))
        return path in self.remote_paths

    def package_installed(self, package_name: str) -> bool:
        self.commands.append(("package_installed", package_name))
        return package_name in self.installed_packages

    def launch_app(self, package_name: str, activity: str | None = None) -> None:
        self.commands.append(("launch_app", package_name, activity or ""))

    def force_stop_app(self, package_name: str) -> None:
        self.commands.append(("force_stop_app", package_name))

    def run_plan_command(self, command: Sequence[str]) -> None:
        self.commands.append(("run_plan_command", *tuple(command)))

    def detect_device(self) -> DetectedDevice:
        self.commands.append(("detect_device",))
        return self._detected_device

    def _record_push(self, source: Path, dest: str) -> None:
        self.remote_paths.add(dest)
        if not source.is_dir():
            return
        for child in source.rglob("*"):
            remote_path = str(PurePosixPath(dest) / child.relative_to(source).as_posix())
            self.remote_paths.add(remote_path)


def _parse_android_version(raw_value: str) -> int:
    match = re.search(r"\d+", raw_value)
    if match is None:
        return 0
    return int(match.group(0))


def _parse_android_api_level(raw_value: str) -> int | None:
    value = raw_value.strip()
    if not value:
        return None
    try:
        return int(value)
    except ValueError:
        return None


def _command_has_serial_flag(args: Sequence[str]) -> bool:
    return len(args) >= 2 and args[0] == "-s"


def resolve_adb_executable(
    cli_value: str | None = None,
    env: dict[str, str] | None = None,
    config_value: str | None = None,
) -> str:
    source_env = env if env is not None else dict(os.environ)

    if cli_value is not None:
        return _validate_explicit_adb_path(cli_value, "CLI --adb")
    if source_env.get("EMUCHEF_ADB"):
        return _validate_explicit_adb_path(source_env["EMUCHEF_ADB"], "EMUCHEF_ADB")
    if config_value:
        return _validate_explicit_adb_path(config_value, "config")
    return "adb"


def _validate_explicit_adb_path(raw_value: str, source_name: str) -> str:
    path = Path(raw_value).expanduser()
    if not path.exists():
        raise AdbResolutionError(
            _adb_not_found_message(f"{source_name} points to {raw_value!r}, but that path does not exist.")
        )
    if not path.is_file() or not os.access(path, os.X_OK):
        raise AdbResolutionError(
            _adb_not_found_message(f"{source_name} points to {raw_value!r}, but it is not executable.")
        )
    return str(path.resolve())


def _adb_not_found_message(detail: str) -> str:
    return f"{detail} Configure ADB with --adb, EMUCHEF_ADB, or ensure adb is available on PATH."

"""Supported step executors for the current slice."""

from __future__ import annotations

from pathlib import Path, PurePosixPath

from emuchef.domain import CopyPolicy, ExecutionStep, StepType

from .adb import AdbInterface


def execute_step(adb: AdbInterface, step: ExecutionStep, workdir: Path) -> None:
    if step.type is StepType.INSTALL_APK:
        _install_apk(adb, step, workdir)
        return
    if step.type is StepType.COPY_BYO_INPUT:
        _copy_byo_input(adb, step, workdir)
        return
    if step.type is StepType.PUSH_FILE:
        _push_file(adb, step, workdir)
        return
    if step.type is StepType.PUSH_DIR:
        _push_dir(adb, step, workdir)
        return
    if step.type is StepType.LAUNCH_APP:
        _launch_app(adb, step)
        return
    raise ValueError(f"Unsupported step type: {step.type.value}")


def _install_apk(adb: AdbInterface, step: ExecutionStep, workdir: Path) -> None:
    app_value = step.params["app"]
    replace_existing = bool(step.params.get("replace_existing", False))
    apk_path = _resolve_local_path(str(app_value), workdir)
    if not apk_path.exists():
        raise FileNotFoundError(f"APK file not found: {apk_path}")
    adb.install_apk(apk_path, replace_existing=replace_existing)


def _copy_byo_input(adb: AdbInterface, step: ExecutionStep, workdir: Path) -> None:
    source = _resolve_local_path(str(step.params["input"]), workdir)
    dest_dir = str(step.params["dest"])
    copy_policy = _parse_copy_policy(step.params.get("copy_policy"))
    if not source.exists():
        raise FileNotFoundError(f"copy_byo_input source not found: {source}")

    if source.is_file():
        _copy_file_to_dir(adb, source, dest_dir, copy_policy)
        return

    _copy_directory_contents(adb, source, dest_dir, copy_policy)


def _push_file(adb: AdbInterface, step: ExecutionStep, workdir: Path) -> None:
    source = _resolve_local_path(str(step.params["source"]), workdir)
    dest = str(step.params["dest"])
    copy_policy = _parse_copy_policy(step.params.get("copy_policy"))
    if not source.exists():
        raise FileNotFoundError(f"push_file source not found: {source}")
    if not source.is_file():
        raise ValueError(f"push_file source must be a file: {source}")

    parent_dir = str(PurePosixPath(dest).parent)
    if parent_dir not in {"", "."}:
        adb.mkdir_p(parent_dir)
    if copy_policy is CopyPolicy.REPLACE:
        adb.remove_file(dest)
    adb.push(source, dest)


def _push_dir(adb: AdbInterface, step: ExecutionStep, workdir: Path) -> None:
    source = _resolve_local_path(str(step.params["source"]), workdir)
    dest_dir = str(step.params["dest"])
    copy_policy = _parse_copy_policy(step.params.get("copy_policy"))
    if not source.exists():
        raise FileNotFoundError(f"push_dir source not found: {source}")
    if not source.is_dir():
        raise ValueError(f"push_dir source must be a directory: {source}")

    _copy_directory_contents(adb, source, dest_dir, copy_policy)


def _copy_file_to_dir(adb: AdbInterface, source: Path, dest_dir: str, copy_policy: CopyPolicy) -> None:
    adb.mkdir_p(dest_dir)
    dest_path = str(PurePosixPath(dest_dir) / source.name)
    if copy_policy is CopyPolicy.REPLACE:
        adb.remove_file(dest_path)
    adb.push(source, dest_path)


def _copy_directory_contents(adb: AdbInterface, source: Path, dest_dir: str, copy_policy: CopyPolicy) -> None:
    if copy_policy is CopyPolicy.REPLACE:
        adb.remove_tree(dest_dir)
    adb.mkdir_p(dest_dir)

    for child in sorted(source.iterdir()):
        dest_path = str(PurePosixPath(dest_dir) / child.name)
        if copy_policy is CopyPolicy.SYNC:
            adb.push_sync(child, dest_path)
        else:
            adb.push(child, dest_path)


def _launch_app(adb: AdbInterface, step: ExecutionStep) -> None:
    package_name = str(step.params["package_name"])
    activity = step.params.get("activity")
    adb.launch_app(package_name, str(activity) if activity is not None else None)


def _resolve_local_path(raw_path: str, workdir: Path) -> Path:
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        raise ValueError(f"Execution plan local path must be absolute: {raw_path!r}")
    return path


def _parse_copy_policy(raw_value) -> CopyPolicy:
    if raw_value is None:
        raise ValueError("Copy step params must include copy_policy.")
    try:
        return CopyPolicy(str(raw_value))
    except ValueError as exc:
        raise ValueError(f"Unsupported copy_policy: {raw_value!r}") from exc

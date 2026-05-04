"""Shared file-copy helpers for device and app-private destinations."""

from __future__ import annotations

from pathlib import Path, PurePosixPath

from emuchef.domain import CopyPolicy, ExecutionStep, RuntimeCapabilities, RuntimeValue, RuntimeValueType

from .step_runtime import ExecutionContext


def copy_host_source(context: ExecutionContext, source: RuntimeValue, dest: str, copy_policy: CopyPolicy) -> list[str]:
    if source.type is RuntimeValueType.DIRECTORY_PATH:
        return _copy_host_directory_contents(context, Path(str(source.value)), dest, copy_policy)
    if source.type is RuntimeValueType.PATH_LIST:
        return _copy_host_path_list(context, [Path(str(item)) for item in list(source.value)], dest, copy_policy)
    if source.type is RuntimeValueType.FILE_PATH:
        return [_copy_host_file(context, Path(str(source.value)), dest, copy_policy)]
    raise ValueError(f"copy_files does not support source runtime type {source.type.value!r}.")


def copy_device_source(context: ExecutionContext, source: RuntimeValue, dest: str, copy_policy: CopyPolicy) -> list[str]:
    if source.type is RuntimeValueType.DIRECTORY_PATH:
        if copy_policy is CopyPolicy.REPLACE:
            context.adb.remove_tree(dest)
        context.adb.mkdir_p(dest)
        context.adb.copy_on_device(f"{source.value}/.", dest, recursive=True)
        return [dest]
    if source.type is RuntimeValueType.PATH_LIST:
        if copy_policy is CopyPolicy.REPLACE:
            context.adb.remove_tree(dest)
        context.adb.mkdir_p(dest)
        copied: list[str] = []
        for item in list(source.value):
            context.adb.copy_on_device(str(item), dest, recursive=True)
            copied.append(str(PurePosixPath(dest) / PurePosixPath(str(item)).name))
        return copied
    if source.type is RuntimeValueType.FILE_PATH:
        target = dest
        if context.adb.path_is_dir(dest):
            target = str(PurePosixPath(dest) / PurePosixPath(str(source.value)).name)
        else:
            parent_dir = str(PurePosixPath(target).parent)
            if parent_dir not in {"", "."}:
                context.adb.mkdir_p(parent_dir)
        if copy_policy is CopyPolicy.REPLACE:
            context.adb.remove_file(target)
        context.adb.copy_on_device(str(source.value), target)
        return [target]
    raise ValueError(f"copy_files does not support device source runtime type {source.type.value!r}.")


def copy_host_source_to_app_private(
    context: ExecutionContext,
    step: ExecutionStep,
    source: RuntimeValue,
    dest: str,
    copy_policy: CopyPolicy,
) -> list[str]:
    stage_root = f"/data/local/tmp/emuchef/{step.id.replace('/', '_')}"
    context.adb.remove_tree(stage_root)
    context.adb.mkdir_p(stage_root)
    try:
        if source.type is RuntimeValueType.DIRECTORY_PATH:
            return _copy_host_directory_to_app_private(context, Path(str(source.value)), dest, copy_policy, stage_root)
        if source.type is RuntimeValueType.PATH_LIST:
            return _copy_host_path_list_to_app_private(
                context,
                [Path(str(item)) for item in list(source.value)],
                dest,
                copy_policy,
                stage_root,
            )
        if source.type is RuntimeValueType.FILE_PATH:
            return [_copy_host_file_to_app_private(context, Path(str(source.value)), dest, copy_policy, stage_root)]
        raise ValueError(f"copy_files does not support source runtime type {source.type.value!r}.")
    finally:
        context.adb.remove_tree(stage_root)


def supports_app_data_write(capabilities: RuntimeCapabilities) -> bool:
    return capabilities.app_data_write and capabilities.root_shell


def _copy_host_directory_contents(context: ExecutionContext, source: Path, dest_dir: str, copy_policy: CopyPolicy) -> list[str]:
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_tree(dest_dir)
    context.adb.mkdir_p(dest_dir)
    copied: list[str] = []
    for child in sorted(source.iterdir()):
        dest_path = str(PurePosixPath(dest_dir) / child.name)
        if copy_policy is CopyPolicy.SYNC:
            context.adb.push_sync(child, dest_path)
        else:
            context.adb.push(child, dest_path)
        copied.append(dest_path)
    return copied


def _copy_host_path_list(context: ExecutionContext, sources: list[Path], dest_dir: str, copy_policy: CopyPolicy) -> list[str]:
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_tree(dest_dir)
    context.adb.mkdir_p(dest_dir)
    copied: list[str] = []
    for source in sources:
        dest_path = str(PurePosixPath(dest_dir) / source.name)
        if copy_policy is CopyPolicy.SYNC:
            context.adb.push_sync(source, dest_path)
        else:
            context.adb.push(source, dest_path)
        copied.append(dest_path)
    return copied


def _copy_host_file(context: ExecutionContext, source: Path, dest: str, copy_policy: CopyPolicy) -> str:
    target = dest
    if context.adb.path_is_dir(dest):
        target = str(PurePosixPath(dest) / source.name)
    else:
        parent_dir = str(PurePosixPath(target).parent)
        if parent_dir not in {"", "."}:
            context.adb.mkdir_p(parent_dir)
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_file(target)
    if copy_policy is CopyPolicy.SYNC:
        context.adb.push_sync(source, target)
    else:
        context.adb.push(source, target)
    return target


def _copy_host_directory_to_app_private(
    context: ExecutionContext,
    source: Path,
    dest_dir: str,
    copy_policy: CopyPolicy,
    stage_root: str,
) -> list[str]:
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_tree(dest_dir)
    context.adb.mkdir_p(dest_dir)
    copied: list[str] = []
    for child in sorted(source.iterdir()):
        staged_path = str(PurePosixPath(stage_root) / child.name)
        context.adb.push(child, staged_path)
        context.adb.copy_on_device(staged_path, dest_dir, recursive=child.is_dir(), privileged=True)
        copied.append(str(PurePosixPath(dest_dir) / child.name))
    return copied


def _copy_host_path_list_to_app_private(
    context: ExecutionContext,
    sources: list[Path],
    dest_dir: str,
    copy_policy: CopyPolicy,
    stage_root: str,
) -> list[str]:
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_tree(dest_dir)
    context.adb.mkdir_p(dest_dir)
    copied: list[str] = []
    for source in sources:
        staged_path = str(PurePosixPath(stage_root) / source.name)
        context.adb.push(source, staged_path)
        context.adb.copy_on_device(staged_path, dest_dir, recursive=source.is_dir(), privileged=True)
        copied.append(str(PurePosixPath(dest_dir) / source.name))
    return copied


def _copy_host_file_to_app_private(
    context: ExecutionContext,
    source: Path,
    dest: str,
    copy_policy: CopyPolicy,
    stage_root: str,
) -> str:
    staged_path = str(PurePosixPath(stage_root) / source.name)
    context.adb.push(source, staged_path)
    target = dest
    if context.adb.path_is_dir(dest):
        target = str(PurePosixPath(dest) / source.name)
    else:
        parent_dir = str(PurePosixPath(target).parent)
        if parent_dir not in {"", "."}:
            context.adb.mkdir_p(parent_dir)
    if copy_policy is CopyPolicy.REPLACE:
        context.adb.remove_file(target)
    context.adb.copy_on_device(staged_path, target, privileged=True)
    return target

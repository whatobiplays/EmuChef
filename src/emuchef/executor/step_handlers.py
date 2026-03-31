"""Supported step executors for the current slice."""

from __future__ import annotations

import hashlib
import ssl
import shutil
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from emuchef.domain import (
    ArtifactCacheMode,
    ArtifactRuntimeStatus,
    CopyPolicy,
    ErrorCode,
    ExecutionArtifact,
    ExecutionState,
    ExecutionStep,
    RuntimeCapabilities,
    RuntimeValue,
    RuntimeValueType,
    StepType,
)

from .adb import AdbInterface, is_app_private_path


class StepExecutionError(RuntimeError):
    def __init__(self, code: ErrorCode, message: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass(slots=True)
class ExecutionContext:
    adb: AdbInterface
    workdir: Path
    artifacts_by_id: Mapping[str, ExecutionArtifact]
    state: ExecutionState
    runtime_capabilities: RuntimeCapabilities
    sleep_fn: Callable[[float], None]


def execute_step(
    context: ExecutionContext,
    step: ExecutionStep,
    resolved_params: Mapping[str, object],
) -> dict[str, RuntimeValue]:
    if step.type is StepType.RESOLVE_ARTIFACTS:
        _resolve_artifacts(context, step, resolved_params)
        return {}
    if step.type is StepType.EXTRACT_ARTIFACTS:
        return _extract_artifacts(context, step, resolved_params)
    if step.type is StepType.EXTRACT_ARCHIVE:
        return _extract_archive(context, step, resolved_params)
    if step.type is StepType.COPY_FILES:
        return _copy_files(context, step, resolved_params)
    if step.type is StepType.INSTALL_APK:
        _install_apk(context, resolved_params)
        return {}
    if step.type is StepType.LAUNCH_APP:
        _launch_app(context, resolved_params)
        return {}
    if step.type is StepType.WAIT:
        _wait(context, resolved_params)
        return {}
    if step.type is StepType.FORCE_STOP_APP:
        _force_stop_app(context, resolved_params)
        return {}
    raise ValueError(f"Unsupported step type: {step.type.value}")


def _resolve_artifacts(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> None:
    artifact_ids = _literal_string_list(resolved_params.get("artifacts"))
    downloads_root = context.workdir / ".emuchef_runtime" / "downloads"
    cache_root = context.workdir / ".emuchef_cache" / "artifacts"
    downloads_root.mkdir(parents=True, exist_ok=True)
    cache_root.mkdir(parents=True, exist_ok=True)

    for artifact_id in artifact_ids:
        artifact = context.artifacts_by_id[artifact_id]
        parsed = urllib.parse.urlparse(artifact.url)
        filename = Path(parsed.path).name or f"{artifact_id.rsplit('/', 1)[-1]}.bin"
        if artifact.cache is ArtifactCacheMode.DEFAULT:
            local_path = cache_root / f"{hashlib.sha256(artifact.url.encode('utf-8')).hexdigest()}-{filename}"
            cache_hit = local_path.exists()
        else:
            local_path = downloads_root / f"{hashlib.sha256((artifact_id + artifact.url).encode('utf-8')).hexdigest()}-{filename}"
            cache_hit = False
        state = context.state.artifacts[artifact_id]
        try:
            if not local_path.exists():
                _download_to_path(artifact_id, artifact.url, local_path)
            state.status = ArtifactRuntimeStatus.RESOLVED
            state.local_path = str(local_path)
            state.resolved_url = artifact.url
            state.filename = filename
            state.cache_hit = cache_hit
            state.error = None
        except Exception as exc:
            state.status = ArtifactRuntimeStatus.FAILED
            state.error = str(exc)
            raise


def _extract_artifacts(
    context: ExecutionContext,
    step: ExecutionStep,
    resolved_params: Mapping[str, object],
) -> dict[str, RuntimeValue]:
    artifact_ids = _literal_string_list(resolved_params.get("artifacts"))
    extract_on = str(resolved_params.get("extract_on", "host"))

    if extract_on == "device":
        base_dir = f"/data/local/tmp/emuchef/{step.id.replace('/', '_')}"
        outputs: list[str] = []
        for artifact_id in artifact_ids:
            artifact_state = context.state.artifacts[artifact_id]
            archive_path = Path(artifact_state.local_path or "")
            device_archive_path = f"{base_dir}/{archive_path.name}"
            extract_dir = f"{base_dir}/{artifact_id.rsplit('/', 1)[-1]}"
            context.adb.mkdir_p(base_dir)
            context.adb.push(archive_path, device_archive_path)
            context.adb.mkdir_p(extract_dir)
            context.adb.run_plan_command(("adb", "shell", "unzip", "-o", device_archive_path, "-d", extract_dir))
            outputs.append(extract_dir)
        return {"extracted_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=outputs, location="device")}

    extract_root = context.workdir / ".emuchef_runtime" / "extract" / step.id.replace("/", "_")
    outputs: list[str] = []
    for artifact_id in artifact_ids:
        artifact_state = context.state.artifacts[artifact_id]
        archive_path = Path(artifact_state.local_path or "")
        artifact_dir = extract_root / artifact_id.rsplit("/", 1)[-1]
        members = _extract_zip_to_directory(archive_path, artifact_dir)
        outputs.extend(str(member) for member in members)
    return {"extracted_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=outputs, location="host")}


def _extract_archive(
    context: ExecutionContext,
    step: ExecutionStep,
    resolved_params: Mapping[str, object],
) -> dict[str, RuntimeValue]:
    archive = _require_runtime_value(resolved_params["archive"])
    extract_on = str(resolved_params.get("extract_on", "host"))
    cleanup = bool(resolved_params.get("cleanup", True))

    if extract_on == "device":
        dest = str(resolved_params["dest"])
        device_archive_path = str(resolved_params.get("device_temp_path") or f"/data/local/tmp/emuchef/{step.id.replace('/', '_')}.zip")
        if archive.location == "host":
            context.adb.push(Path(str(archive.value)), device_archive_path)
        else:
            device_archive_path = str(archive.value)
        context.adb.mkdir_p(dest)
        context.adb.run_plan_command(("adb", "shell", "unzip", "-o", device_archive_path, "-d", dest))
        if cleanup and archive.location == "host":
            context.adb.remove_file(device_archive_path)
        return {"extracted_path": RuntimeValue(type=RuntimeValueType.DIRECTORY_PATH, value=dest, location="device")}

    if archive.location != "host":
        raise ValueError("Host extraction requires a host-side archive path.")
    archive_path = Path(str(archive.value))
    extract_root = context.workdir / ".emuchef_runtime" / "extract" / step.id.replace("/", "_")
    members = _extract_zip_to_directory(archive_path, extract_root)
    if len(members) == 1:
        member = members[0]
        runtime_type = RuntimeValueType.DIRECTORY_PATH if member.is_dir() else RuntimeValueType.FILE_PATH
        return {"extracted_path": RuntimeValue(type=runtime_type, value=str(member), location="host")}
    return {"extracted_path": RuntimeValue(type=RuntimeValueType.DIRECTORY_PATH, value=str(extract_root), location="host")}


def _copy_files(
    context: ExecutionContext,
    step: ExecutionStep,
    resolved_params: Mapping[str, object],
) -> dict[str, RuntimeValue]:
    source = _require_runtime_value(resolved_params["source"])
    dest = str(resolved_params["dest"])
    copy_policy = CopyPolicy(str(resolved_params.get("copy_policy", CopyPolicy.MERGE.value)))
    app_private_dest = is_app_private_path(dest)

    if app_private_dest and not _supports_app_data_write(context.runtime_capabilities):
        raise StepExecutionError(
            ErrorCode.APP_DATA_WRITE_UNAVAILABLE,
            (
                f"Destination {dest!r} requires root-backed app_data_write support, but runtime capabilities "
                "do not provide both app_data_write and root_shell."
            ),
        )

    if app_private_dest and source.location == "host":
        copied = _copy_host_source_to_app_private(context, step, source, dest, copy_policy)
    elif source.location == "device":
        copied = _copy_device_source(context, source, dest, copy_policy)
    else:
        copied = _copy_host_source(context, source, dest, copy_policy)
    return {"copied_paths": RuntimeValue(type=RuntimeValueType.PATH_LIST, value=copied, location="device")}


def _install_apk(context: ExecutionContext, resolved_params: Mapping[str, object]) -> None:
    app = _require_runtime_value(resolved_params["app"])
    if app.type is not RuntimeValueType.FILE_PATH or app.location != "host":
        raise ValueError("install_apk requires a host-side file_path runtime value.")
    apk_path = Path(str(app.value))
    if apk_path.suffix.lower() != ".apk":
        raise ValueError(f"install_apk requires an .apk file, got: {apk_path}")
    if not apk_path.exists():
        raise FileNotFoundError(f"APK file not found: {apk_path}")
    context.adb.install_apk(apk_path, replace_existing=bool(resolved_params.get("replace_existing", False)))


def _launch_app(context: ExecutionContext, resolved_params: Mapping[str, object]) -> None:
    package_name = str(resolved_params["package_name"])
    activity = resolved_params.get("activity")
    context.adb.launch_app(package_name, str(activity) if activity is not None else None)


def _wait(context: ExecutionContext, resolved_params: Mapping[str, object]) -> None:
    raw_duration = resolved_params["duration_ms"]
    if isinstance(raw_duration, bool) or not isinstance(raw_duration, int) or raw_duration <= 0:
        raise ValueError(f"wait step requires a positive integer duration_ms: {raw_duration!r}")
    context.sleep_fn(raw_duration / 1000.0)


def _force_stop_app(context: ExecutionContext, resolved_params: Mapping[str, object]) -> None:
    package_name = str(resolved_params["package_name"])
    if not package_name.strip():
        raise ValueError("force_stop_app step requires a non-empty package_name.")
    context.adb.force_stop_app(package_name)


def _copy_host_source(context: ExecutionContext, source: RuntimeValue, dest: str, copy_policy: CopyPolicy) -> list[str]:
    if source.type is RuntimeValueType.DIRECTORY_PATH:
        return _copy_host_directory_contents(context, Path(str(source.value)), dest, copy_policy)
    if source.type is RuntimeValueType.PATH_LIST:
        return _copy_host_path_list(context, [Path(str(item)) for item in list(source.value)], dest, copy_policy)
    if source.type is RuntimeValueType.FILE_PATH:
        return [_copy_host_file(context, Path(str(source.value)), dest, copy_policy)]
    raise ValueError(f"copy_files does not support source runtime type {source.type.value!r}.")


def _copy_device_source(context: ExecutionContext, source: RuntimeValue, dest: str, copy_policy: CopyPolicy) -> list[str]:
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


def _copy_host_source_to_app_private(
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


def _extract_zip_to_directory(archive_path: Path, dest_dir: Path) -> list[Path]:
    dest_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive_path, "r") as handle:
        handle.extractall(dest_dir)
    children = sorted(dest_dir.iterdir())
    return children or [dest_dir]


def _download_to_path(artifact_id: str, url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    try:
        with urllib.request.urlopen(url) as source, dest.open("wb") as target:
            shutil.copyfileobj(source, target)
    except ssl.SSLCertVerificationError as exc:
        raise StepExecutionError(
            ErrorCode.TLS_VERIFICATION_FAILED,
            _tls_verification_error_message(artifact_id, url),
        ) from exc
    except urllib.error.URLError as exc:
        if _is_tls_verification_error(exc):
            raise StepExecutionError(
                ErrorCode.TLS_VERIFICATION_FAILED,
                _tls_verification_error_message(artifact_id, url),
            ) from exc
        reason = exc.reason if exc.reason is not None else exc
        raise StepExecutionError(
            ErrorCode.ARTIFACT_DOWNLOAD_FAILED,
            f"Failed to download artifact {artifact_id!r} from {url!r}: {reason}",
        ) from exc
    except Exception as exc:
        raise StepExecutionError(
            ErrorCode.ARTIFACT_DOWNLOAD_FAILED,
            f"Failed to download artifact {artifact_id!r} from {url!r}: {exc}",
        ) from exc


def _is_tls_verification_error(exc: urllib.error.URLError) -> bool:
    if isinstance(exc.reason, ssl.SSLCertVerificationError):
        return True
    return "CERTIFICATE_VERIFY_FAILED" in str(exc.reason)


def _tls_verification_error_message(artifact_id: str, url: str) -> str:
    return (
        f"TLS verification failed while downloading artifact {artifact_id!r} from {url!r}. "
        "Your Python installation could not verify the server certificate. "
        "On macOS Python.org builds, run Install Certificates.command to install or update the trust store."
    )


def _literal_string_list(value: object | None) -> list[str]:
    if value is None:
        return []
    if isinstance(value, RuntimeValue):
        value = value.value
    return [str(item) for item in list(value)]


def _require_runtime_value(value: object) -> RuntimeValue:
    if not isinstance(value, RuntimeValue):
        raise ValueError(f"Expected a resolved runtime value, got: {value!r}")
    return value


def _supports_app_data_write(capabilities: RuntimeCapabilities) -> bool:
    return capabilities.app_data_write and capabilities.root_shell

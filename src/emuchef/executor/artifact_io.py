"""Artifact download and archive extraction helpers used by step handlers."""

from __future__ import annotations

import shutil
import ssl
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

from emuchef.domain import ErrorCode

from .step_runtime import StepExecutionError


def extract_zip_to_directory(archive_path: Path, dest_dir: Path) -> list[Path]:
    dest_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive_path, "r") as handle:
        handle.extractall(dest_dir)
    children = sorted(dest_dir.iterdir())
    return children or [dest_dir]


def download_to_path(artifact_id: str, url: str, dest: Path) -> None:
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

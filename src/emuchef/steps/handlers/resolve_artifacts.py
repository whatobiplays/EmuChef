"""Execution handler for the ``resolve_artifacts`` built-in step."""

from __future__ import annotations

import hashlib
import urllib.parse
from collections.abc import Mapping
from pathlib import Path

from emuchef.domain import ArtifactCacheMode, ArtifactRuntimeStatus, ExecutionStep, RuntimeValue
from emuchef.executor.artifact_io import download_to_path
from emuchef.executor.runtime_values import literal_string_list
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    artifact_ids = literal_string_list(resolved_params.get("artifacts"))
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
                download_to_path(artifact_id, artifact.url, local_path)
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
    return {}

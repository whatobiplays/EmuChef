//! Sidecar metadata for one app-managed artifact-cache payload.
//!
//! The payload remains authoritative for execution. Metadata is optional support
//! information written beside a newly published default-cache payload. A failed
//! metadata write never fails execution and never leaves an older sidecar that
//! could falsely describe the new payload.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

pub(crate) const METADATA_SCHEMA_VERSION: u64 = 1;
pub(crate) const METADATA_SUFFIX: &str = ".emuchef-cache.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactCacheMetadata {
    pub schema_version: u64,
    pub payload_file_name: String,
    pub artifact_label: String,
    pub source_kind: String,
    pub source_fingerprint: String,
    pub payload_size_bytes: u64,
    pub payload_modified_nanos: Option<u128>,
}

/// Build validated metadata without retaining a raw source URL.
pub(crate) fn prepare_metadata(
    payload_path: &Path,
    artifact_id: &str,
    source_url: &str,
    payload_size_bytes: u64,
    payload_modified_nanos: Option<u128>,
) -> Option<ArtifactCacheMetadata> {
    let payload_file_name = payload_path.file_name()?.to_str()?.to_string();
    let source_kind = source_url
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .filter(|scheme| matches!(*scheme, "file" | "http" | "https"))
        .unwrap_or("unknown")
        .to_string();
    let source_fingerprint = Sha256::digest(source_url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let metadata = ArtifactCacheMetadata {
        schema_version: METADATA_SCHEMA_VERSION,
        payload_file_name,
        artifact_label: safe_artifact_label(artifact_id),
        source_kind,
        source_fingerprint,
        payload_size_bytes,
        payload_modified_nanos,
    };
    serde_json::to_vec(&metadata)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ArtifactCacheMetadata>(&bytes).ok())
}

/// Atomically publish metadata after its payload has been promoted.
///
/// Any stale destination is removed before the new sidecar is attempted. If
/// publication fails, callers intentionally retain a usable but unindexed
/// payload and this function guarantees best-effort removal of misleading
/// metadata and temporary files.
pub(crate) fn publish_metadata(payload_path: &Path, metadata: &ArtifactCacheMetadata) -> bool {
    let Some(metadata_path) = metadata_path(payload_path) else {
        return false;
    };
    let _ = fs::remove_file(&metadata_path);
    let Some(parent) = metadata_path.parent() else {
        return false;
    };
    let bytes = match serde_json::to_vec(metadata) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let mut temporary = match TempFileBuilder::new()
        .prefix(".emuchef-cache-metadata-")
        .suffix(".partial")
        .tempfile_in(parent)
    {
        Ok(temporary) => temporary,
        Err(_) => return false,
    };
    if temporary.write_all(&bytes).is_err()
        || temporary.as_file_mut().flush().is_err()
        || temporary.as_file().sync_all().is_err()
    {
        let _ = temporary.close();
        return false;
    }
    match temporary.persist_noclobber(&metadata_path) {
        Ok(_) => true,
        Err(error) => {
            let _ = error.file.close();
            let _ = fs::remove_file(metadata_path);
            false
        }
    }
}

pub(crate) fn metadata_path(payload_path: &Path) -> Option<PathBuf> {
    let file_name = payload_path.file_name()?.to_str()?;
    Some(payload_path.with_file_name(format!(".{file_name}{METADATA_SUFFIX}")))
}

fn safe_artifact_label(artifact_id: &str) -> String {
    let filtered = artifact_id
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
        })
        .take(160)
        .collect::<String>();
    if filtered.is_empty() {
        "artifact".to_string()
    } else {
        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_contains_no_raw_url_and_publishes_as_one_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("payload.bin");
        fs::write(&payload, b"bytes").unwrap();
        let metadata = prepare_metadata(
            &payload,
            "recipe/artifact",
            "https://user:password@example.invalid/a?token=secret#fragment",
            5,
            Some(123),
        )
        .unwrap();
        assert!(publish_metadata(&payload, &metadata));
        let path = metadata_path(&payload).unwrap();
        let encoded = fs::read_to_string(path).unwrap();
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("example.invalid"));
        assert_eq!(
            serde_json::from_str::<ArtifactCacheMetadata>(&encoded).unwrap(),
            metadata
        );
    }

    #[test]
    fn publishing_replaces_stale_metadata_or_leaves_no_misleading_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("payload.bin");
        fs::write(&payload, b"new").unwrap();
        let sidecar = metadata_path(&payload).unwrap();
        fs::write(&sidecar, b"stale").unwrap();
        let metadata =
            prepare_metadata(&payload, "recipe/new", "file:///new", 3, Some(456)).unwrap();
        assert!(publish_metadata(&payload, &metadata));
        assert_eq!(
            serde_json::from_slice::<ArtifactCacheMetadata>(&fs::read(sidecar).unwrap()).unwrap(),
            metadata
        );
    }

    #[test]
    fn metadata_publication_failure_keeps_payload_usable_and_unindexed() {
        let temp = tempfile::tempdir().unwrap();
        let payload = temp.path().join("payload.bin");
        fs::write(&payload, b"usable").unwrap();
        let sidecar = metadata_path(&payload).unwrap();
        fs::create_dir(&sidecar).unwrap();
        let metadata =
            prepare_metadata(&payload, "recipe/artifact", "file:///source", 6, None).unwrap();

        assert!(!publish_metadata(&payload, &metadata));
        assert_eq!(fs::read(&payload).unwrap(), b"usable");
        assert!(fs::read(&sidecar).is_err());
    }
}

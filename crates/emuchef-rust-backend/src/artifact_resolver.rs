//! Artifact routing, destination selection, and compatibility-preserving names.
//!
//! The resolver is crate-private because execution plans remain the product
//! interface. It preserves the original URL bytes when deriving cache keys.

use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::artifact_transport::{ArtifactTransport, LocalFileTransport};
use crate::executor::SandboxRoots;

/// Inputs required to resolve one execution-plan artifact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArtifactResolveRequest<'a> {
    pub artifact_id: &'a str,
    pub url: &'a str,
    pub cache_mode: &'a str,
}

/// Filesystem result recorded in executor runtime state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedArtifact {
    pub local_path: PathBuf,
    pub filename: String,
    pub cache_hit: bool,
}

/// Initial resolver error boundary; commit 2 replaces this wrapper with typed
/// artifact failure variants while retaining the same executor integration.
#[derive(Debug)]
pub(crate) struct ArtifactResolveError {
    message: String,
}

impl ArtifactResolveError {
    pub(crate) fn new(message: String) -> Self {
        Self { message }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

/// Resolve artifacts within the executor's authoritative sandbox roots.
#[derive(Debug)]
pub(crate) struct ArtifactResolver<'a> {
    sandbox: &'a SandboxRoots,
    local_transport: LocalFileTransport,
}

impl<'a> ArtifactResolver<'a> {
    pub(crate) fn new(sandbox: &'a SandboxRoots) -> Self {
        Self {
            sandbox,
            local_transport: LocalFileTransport,
        }
    }

    pub(crate) fn resolve(
        &self,
        request: ArtifactResolveRequest<'_>,
    ) -> Result<ResolvedArtifact, ArtifactResolveError> {
        let filename = artifact_filename(request.artifact_id, request.url);
        let local_filename =
            artifact_local_filename(request.artifact_id, request.url, request.cache_mode);
        let cache_hit;
        let local_path = if request.cache_mode == "default" {
            let path = self.sandbox.cache_root.join(&local_filename);
            cache_hit = path.exists();
            path
        } else {
            cache_hit = false;
            self.sandbox
                .runtime_root
                .join("downloads")
                .join(&local_filename)
        };

        self.sandbox
            .ensure_runtime_or_cache_write(&local_path)
            .map_err(|failure| ArtifactResolveError::new(failure.message))?;
        if !local_path.exists() {
            let Some(source_path) = file_url_to_path(request.url) else {
                return Err(ArtifactResolveError::new(format!(
                    "network_artifact_download_unsupported: Network artifact downloads are not supported for artifact {:?} from {:?}; use a file:// source.",
                    request.artifact_id, request.url
                )));
            };
            self.sandbox
                .ensure_read_allowed(&source_path)
                .map_err(|failure| ArtifactResolveError::new(failure.message))?;
            fs::create_dir_all(
                local_path
                    .parent()
                    .expect("artifact destination has a parent"),
            )
            .map_err(|error| ArtifactResolveError::new(error.to_string()))?;
            self.local_transport.download(&source_path, &local_path)?;
        }

        Ok(ResolvedArtifact {
            local_path,
            filename,
            cache_hit,
        })
    }
}

/// Derive the deterministic local name without normalizing the source URL.
pub(crate) fn artifact_local_filename(artifact_id: &str, url: &str, cache: &str) -> String {
    let filename = artifact_filename(artifact_id, url);
    let hash_input = if cache == "default" {
        url.to_string()
    } else {
        format!("{artifact_id}{url}")
    };
    let digest = Sha256::digest(hash_input.as_bytes());
    let digest_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{digest_hex}-{filename}")
}

fn artifact_filename(artifact_id: &str, url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path_with_query = if after_scheme.starts_with('/') {
        after_scheme
    } else {
        after_scheme
            .find('/')
            .map(|index| &after_scheme[index..])
            .unwrap_or("")
    };
    let path = path_with_query.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .map(percent_decode)
        .unwrap_or_else(|| {
            format!(
                "{}.bin",
                artifact_id.rsplit('/').next().unwrap_or(artifact_id)
            )
        })
}

/// Convert the existing absolute file-URL forms without changing compatibility.
pub(crate) fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.find('/').map(|index| &rest[index..])?
    };
    Some(PathBuf::from(percent_decode(path)))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

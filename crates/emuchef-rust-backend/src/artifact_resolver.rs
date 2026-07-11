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

/// Typed artifact failures converted to stable messages only by the executor.
#[derive(Debug)]
#[allow(dead_code)] // Transport and publication commits activate the remaining typed variants.
pub(crate) enum ArtifactResolveError {
    UrlInvalid,
    SchemeUnsupported { scheme: String },
    SourceNotFound,
    DownloadFailed,
    HttpStatus { status: u16 },
    RedirectLimitExceeded { redirects: usize },
    RedirectDowngradeRejected,
    ConnectTimeout,
    RequestTimeout,
    TlsVerificationFailed,
    ResponseIncomplete,
    ResponseTooLarge,
    CacheWriteFailed,
    CachePublishFailed,
    PartialCleanupFailed { primary: Box<ArtifactResolveError> },
    SandboxRejected,
}

impl ArtifactResolveError {
    /// Stable code embedded in executor messages without changing protocol fields.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UrlInvalid => "artifact_url_invalid",
            Self::SchemeUnsupported { .. } => "artifact_scheme_unsupported",
            Self::SourceNotFound => "artifact_source_not_found",
            Self::DownloadFailed => "artifact_download_failed",
            Self::HttpStatus { .. } => "artifact_http_status",
            Self::RedirectLimitExceeded { .. } => "artifact_redirect_limit_exceeded",
            Self::RedirectDowngradeRejected => "artifact_redirect_downgrade_rejected",
            Self::ConnectTimeout => "artifact_connect_timeout",
            Self::RequestTimeout => "artifact_request_timeout",
            Self::TlsVerificationFailed => "artifact_tls_verification_failed",
            Self::ResponseIncomplete => "artifact_response_incomplete",
            Self::ResponseTooLarge => "artifact_response_too_large",
            Self::CacheWriteFailed => "artifact_cache_write_failed",
            Self::CachePublishFailed => "artifact_cache_publish_failed",
            Self::PartialCleanupFailed { .. } => "artifact_partial_cleanup_failed",
            Self::SandboxRejected => "artifact_sandbox_rejected",
        }
    }

    /// Render one stable, credential-safe executor-facing failure message.
    pub(crate) fn executor_message(&self, request: ArtifactResolveRequest<'_>) -> String {
        if let Self::PartialCleanupFailed { primary } = self {
            return format!(
                "{}; artifact_partial_cleanup_failed: temporary artifact cleanup failed",
                primary.executor_message(request)
            );
        }

        let scheme = url_scheme(request.url).unwrap_or("unknown");
        let source = redacted_url(request.url)
            .map(|url| format!(" from {url}"))
            .unwrap_or_default();
        let detail = match self {
            Self::UrlInvalid => "has an invalid artifact URL".to_string(),
            Self::SchemeUnsupported { scheme } => {
                format!("uses unsupported URL scheme {scheme:?}")
            }
            Self::SourceNotFound => "references a local source that does not exist".to_string(),
            Self::DownloadFailed => "could not be downloaded".to_string(),
            Self::HttpStatus { status } => format!("returned HTTP {status}"),
            Self::RedirectLimitExceeded { redirects } => {
                format!("exceeded the redirect limit after {redirects} redirects")
            }
            Self::RedirectDowngradeRejected => {
                "attempted a rejected HTTPS-to-HTTP redirect".to_string()
            }
            Self::ConnectTimeout => "timed out while connecting".to_string(),
            Self::RequestTimeout => "exceeded the total request deadline".to_string(),
            Self::TlsVerificationFailed => "failed TLS verification".to_string(),
            Self::ResponseIncomplete => "returned an incomplete response".to_string(),
            Self::ResponseTooLarge => "exceeded the supported byte counter".to_string(),
            Self::CacheWriteFailed => "could not be written to artifact storage".to_string(),
            Self::CachePublishFailed => "could not be published to artifact storage".to_string(),
            Self::SandboxRejected => "was rejected by the filesystem sandbox".to_string(),
            Self::PartialCleanupFailed { .. } => unreachable!("handled above"),
        };
        format!(
            "{}: Artifact {:?} ({scheme}) {detail}{source}",
            self.code(),
            request.artifact_id
        )
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
            .map_err(|_| ArtifactResolveError::SandboxRejected)?;
        if !local_path.exists() {
            let Some(source_path) = file_url_to_path(request.url) else {
                return Err(ArtifactResolveError::SchemeUnsupported {
                    scheme: url_scheme(request.url).unwrap_or("unknown").to_string(),
                });
            };
            self.sandbox
                .ensure_read_allowed(&source_path)
                .map_err(|_| ArtifactResolveError::SandboxRejected)?;
            if !source_path.is_file() {
                return Err(ArtifactResolveError::SourceNotFound);
            }
            fs::create_dir_all(
                local_path
                    .parent()
                    .expect("artifact destination has a parent"),
            )
            .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
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

fn url_scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once(':')?;
    (!scheme.is_empty()).then_some(scheme)
}

fn redacted_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let without_sensitive_suffix = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = without_sensitive_suffix
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((without_sensitive_suffix, String::new()));
    let host = authority.rsplit('@').next().unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ArtifactResolveRequest<'static> {
        ArtifactResolveRequest {
            artifact_id: "example.recipe/archive",
            url: "https://user:password@example.com/archive.zip?token=secret#private",
            cache_mode: "default",
        }
    }

    #[test]
    fn every_artifact_error_has_a_stable_code_and_message() {
        let failures = vec![
            ArtifactResolveError::UrlInvalid,
            ArtifactResolveError::SchemeUnsupported {
                scheme: "ftp".to_string(),
            },
            ArtifactResolveError::SourceNotFound,
            ArtifactResolveError::DownloadFailed,
            ArtifactResolveError::HttpStatus { status: 404 },
            ArtifactResolveError::RedirectLimitExceeded { redirects: 6 },
            ArtifactResolveError::RedirectDowngradeRejected,
            ArtifactResolveError::ConnectTimeout,
            ArtifactResolveError::RequestTimeout,
            ArtifactResolveError::TlsVerificationFailed,
            ArtifactResolveError::ResponseIncomplete,
            ArtifactResolveError::ResponseTooLarge,
            ArtifactResolveError::CacheWriteFailed,
            ArtifactResolveError::CachePublishFailed,
            ArtifactResolveError::SandboxRejected,
        ];

        for failure in failures {
            let message = failure.executor_message(request());
            assert!(message.starts_with(failure.code()));
            assert!(message.contains("example.recipe/archive"));
            assert!(!message.contains("user"));
            assert!(!message.contains("password"));
            assert!(!message.contains("secret"));
            assert!(!message.contains("private"));
        }
    }

    #[test]
    fn cleanup_failure_preserves_primary_code_and_adds_secondary_code() {
        let failure = ArtifactResolveError::PartialCleanupFailed {
            primary: Box::new(ArtifactResolveError::RequestTimeout),
        };
        let message = failure.executor_message(request());
        assert!(message.starts_with("artifact_request_timeout"));
        assert!(message.contains("artifact_partial_cleanup_failed"));
    }

    #[test]
    fn redacted_url_keeps_location_but_removes_credentials_query_and_fragment() {
        assert_eq!(
            redacted_url(request().url).as_deref(),
            Some("https://example.com/archive.zip")
        );
    }
}

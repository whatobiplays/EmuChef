//! Artifact routing, destination selection, and compatibility-preserving names.
//!
//! The resolver is crate-private because execution plans remain the product
//! interface. It preserves the original URL bytes when deriving cache keys.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::{Builder as TempFileBuilder, NamedTempFile, TempPath};
use url::Url;

use crate::artifact_store::{prepare_metadata, publish_metadata};
use crate::artifact_transport::{
    ArtifactTransport, DownloadMetadata, HttpArtifactTransport, HttpClientConfig,
    LocalFileTransport, RedirectPolicy,
};
use crate::executor::SandboxRoots;

/// Inputs required to resolve one execution-plan artifact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArtifactResolveRequest<'a> {
    pub artifact_id: &'a str,
    pub type_name: &'a str,
    pub url: &'a str,
    pub cache_mode: &'a str,
}

/// Filesystem result recorded in executor runtime state.
#[derive(Debug)]
pub(crate) struct ResolvedArtifact {
    pub local_path: PathBuf,
    pub filename: String,
    pub cache_hit: bool,
    pub calculated_sha256: Option<String>,
    pub redacted_final_url: Option<String>,
    pub verified_path_guard: Option<Arc<TempPath>>,
}

/// Typed artifact failures converted to stable messages only by the executor.
#[derive(Debug)]
pub(crate) enum ArtifactResolveError {
    TypeUnsupported,
    CacheModeUnsupported,
    UrlInvalid,
    SchemeUnsupported { scheme: String },
    SourceNotFound,
    SourceWrongKind,
    SourceUnreadable,
    DownloadFailed,
    HttpStatus { status: u16 },
    RedirectLimitExceeded { redirects: usize },
    RedirectDowngradeRejected,
    RedirectPolicyRejected,
    ConnectTimeout,
    RequestTimeout,
    TlsVerificationFailed,
    ResponseIncomplete,
    ResponseTooLarge,
    Sha256Unavailable,
    ExpectedSha256Invalid,
    ExpectedSha256Mismatch,
    ArtifactChangedDuringResolution,
    CacheWriteFailed,
    CachePublishFailed,
    PartialCleanupFailed { primary: Box<ArtifactResolveError> },
    SandboxRejected,
}

impl ArtifactResolveError {
    /// Stable code embedded in executor messages without changing protocol fields.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::TypeUnsupported => "artifact_type_unsupported",
            Self::CacheModeUnsupported => "artifact_cache_mode_unsupported",
            Self::UrlInvalid => "artifact_url_invalid",
            Self::SchemeUnsupported { .. } => "artifact_scheme_unsupported",
            Self::SourceNotFound => "artifact_source_not_found",
            Self::SourceWrongKind => "artifact_source_wrong_kind",
            Self::SourceUnreadable => "artifact_source_unreadable",
            Self::DownloadFailed => "artifact_download_failed",
            Self::HttpStatus { .. } => "artifact_http_status",
            Self::RedirectLimitExceeded { .. } => "artifact_redirect_limit_exceeded",
            Self::RedirectDowngradeRejected => "artifact_redirect_downgrade_rejected",
            Self::RedirectPolicyRejected => "artifact_redirect_policy_rejected",
            Self::ConnectTimeout => "artifact_connect_timeout",
            Self::RequestTimeout => "artifact_request_timeout",
            Self::TlsVerificationFailed => "artifact_tls_verification_failed",
            Self::ResponseIncomplete => "artifact_response_incomplete",
            Self::ResponseTooLarge => "artifact_response_too_large",
            Self::Sha256Unavailable => "artifact_sha256_unavailable",
            Self::ExpectedSha256Invalid => "artifact_sha256_invalid",
            Self::ExpectedSha256Mismatch => "artifact_sha256_mismatch",
            Self::ArtifactChangedDuringResolution => "artifact_changed_during_resolution",
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
            Self::TypeUnsupported => "uses an unsupported artifact type".to_string(),
            Self::CacheModeUnsupported => "uses an unsupported cache mode".to_string(),
            Self::UrlInvalid => "has an invalid artifact URL".to_string(),
            Self::SchemeUnsupported { scheme } => {
                format!("uses unsupported URL scheme {scheme:?}")
            }
            Self::SourceNotFound => "references a local source that does not exist".to_string(),
            Self::SourceWrongKind => {
                "references a local source that is not a regular file".to_string()
            }
            Self::SourceUnreadable => "references a local source that is not readable".to_string(),
            Self::DownloadFailed => "could not be downloaded".to_string(),
            Self::HttpStatus { status } => format!("returned HTTP {status}"),
            Self::RedirectLimitExceeded { redirects } => {
                format!("exceeded the redirect limit after {redirects} redirects")
            }
            Self::RedirectDowngradeRejected => {
                "attempted a rejected HTTPS-to-HTTP redirect".to_string()
            }
            Self::RedirectPolicyRejected => {
                "redirected outside the public HTTPS source policy".to_string()
            }
            Self::ConnectTimeout => "timed out while connecting".to_string(),
            Self::RequestTimeout => "exceeded the total request deadline".to_string(),
            Self::TlsVerificationFailed => "failed TLS verification".to_string(),
            Self::ResponseIncomplete => "returned an incomplete response".to_string(),
            Self::ResponseTooLarge => "exceeded the supported byte counter".to_string(),
            Self::Sha256Unavailable => "could not be read for SHA-256 verification".to_string(),
            Self::ExpectedSha256Invalid => "contains an invalid expected SHA-256".to_string(),
            Self::ExpectedSha256Mismatch => "does not match the expected SHA-256".to_string(),
            Self::ArtifactChangedDuringResolution => {
                "changed while the artifact was being materialized".to_string()
            }
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

#[derive(Debug)]
enum AdmittedArtifactSource {
    CacheHit,
    LocalFile(PathBuf),
    Http(Url),
}

#[derive(Clone, Copy, Debug)]
enum ArtifactResolvePolicy<'a> {
    RecipeRemoteFile,
    DirectUrl { expected_sha256: Option<&'a str> },
}

/// Non-mutating result shared by start admission and runtime resolution.
#[derive(Debug)]
pub(crate) struct AdmittedArtifact {
    final_path: PathBuf,
    filename: String,
    default_cache: bool,
    source: AdmittedArtifactSource,
}

type SourceReadabilityCheck = fn(&Path) -> io::Result<()>;

/// Resolve and admit artifacts within the executor's authoritative sandbox roots.
#[derive(Debug)]
pub(crate) struct ArtifactResolver<'a> {
    sandbox: &'a SandboxRoots,
    local_transport: LocalFileTransport,
    http_transport: Option<HttpArtifactTransport>,
    source_readability_check: SourceReadabilityCheck,
}

impl<'a> ArtifactResolver<'a> {
    pub(crate) fn new(sandbox: &'a SandboxRoots) -> Self {
        Self {
            sandbox,
            local_transport: LocalFileTransport,
            http_transport: None,
            source_readability_check: check_source_readable,
        }
    }

    #[cfg(test)]
    fn with_source_readability_check(
        sandbox: &'a SandboxRoots,
        source_readability_check: SourceReadabilityCheck,
    ) -> Self {
        Self {
            source_readability_check,
            ..Self::new(sandbox)
        }
    }

    /// Classify one artifact without network access or filesystem mutation.
    ///
    /// A structurally valid authoritative default-cache file is accepted before
    /// parsing its original URL. Cold sources repeat the canonical URL, local
    /// source, destination, and sandbox checks used immediately before runtime
    /// resolution performs any transfer or publication work.
    pub(crate) fn admit(
        &self,
        request: ArtifactResolveRequest<'_>,
    ) -> Result<AdmittedArtifact, ArtifactResolveError> {
        self.admit_with_policy(request, ArtifactResolvePolicy::RecipeRemoteFile)
    }

    /// Classify an App Definition-owned direct URL under its stricter source
    /// policy, including when a cache entry already exists.
    pub(crate) fn admit_direct_url(
        &self,
        request: ArtifactResolveRequest<'_>,
        expected_sha256: Option<&str>,
    ) -> Result<AdmittedArtifact, ArtifactResolveError> {
        self.admit_with_policy(
            request,
            ArtifactResolvePolicy::DirectUrl { expected_sha256 },
        )
    }

    fn admit_with_policy(
        &self,
        request: ArtifactResolveRequest<'_>,
        policy: ArtifactResolvePolicy<'_>,
    ) -> Result<AdmittedArtifact, ArtifactResolveError> {
        validate_artifact_definition(request)?;
        let direct_url = match policy {
            ArtifactResolvePolicy::RecipeRemoteFile => None,
            ArtifactResolvePolicy::DirectUrl { expected_sha256 } => {
                validate_expected_sha256(expected_sha256)?;
                Some(
                    crate::authored_models::parse_public_https_url(request.url)
                        .ok_or(ArtifactResolveError::UrlInvalid)?,
                )
            }
        };
        let filename = artifact_filename(request.artifact_id, request.url);
        let local_filename =
            artifact_local_filename(request.artifact_id, request.url, request.cache_mode);
        let default_cache = request.cache_mode == "default";
        let final_path = if default_cache {
            self.sandbox.cache_root.join(&local_filename)
        } else {
            self.sandbox
                .runtime_root
                .join("downloads")
                .join(&local_filename)
        };

        self.sandbox
            .ensure_runtime_or_cache_write(&final_path)
            .map_err(|_| ArtifactResolveError::SandboxRejected)?;
        let existing = existing_regular_file(&final_path)?;
        if default_cache && existing {
            return Ok(AdmittedArtifact {
                final_path,
                filename,
                default_cache,
                source: AdmittedArtifactSource::CacheHit,
            });
        }

        let parsed_url = match direct_url {
            Some(url) => url,
            None => {
                let url = Url::parse(request.url).map_err(|_| ArtifactResolveError::UrlInvalid)?;
                validate_source_url(&url)?;
                url
            }
        };
        let source = match parsed_url.scheme() {
            "file" => {
                let source_path = file_url_to_path(request.url)
                    .filter(|path| path.is_absolute())
                    .ok_or(ArtifactResolveError::UrlInvalid)?;
                self.sandbox
                    .ensure_read_allowed(&source_path)
                    .map_err(|_| ArtifactResolveError::SandboxRejected)?;
                validate_local_source(&source_path, self.source_readability_check)?;
                AdmittedArtifactSource::LocalFile(source_path)
            }
            "http" | "https" => AdmittedArtifactSource::Http(parsed_url),
            _ => unreachable!("source scheme was validated"),
        };
        Ok(AdmittedArtifact {
            final_path,
            filename,
            default_cache,
            source,
        })
    }

    /// Check artifact policy and the selected local storage root for a source
    /// whose URL will be discovered only after execution begins.
    ///
    /// This check intentionally performs no network request, destination
    /// creation, or placeholder-URL validation. The discovered URL is passed
    /// through `resolve` later, where the ordinary URL and sandbox admission
    /// checks run before any download or publication work.
    pub(crate) fn admit_late_bound(
        &self,
        type_name: &str,
        cache_mode: &str,
    ) -> Result<(), ArtifactResolveError> {
        if type_name != "remote_file" {
            return Err(ArtifactResolveError::TypeUnsupported);
        }
        if !matches!(cache_mode, "default" | "none") {
            return Err(ArtifactResolveError::CacheModeUnsupported);
        }
        let storage_root = if cache_mode == "default" {
            &self.sandbox.cache_root
        } else {
            &self.sandbox.runtime_root
        };
        self.sandbox
            .ensure_runtime_or_cache_write(storage_root)
            .map_err(|_| ArtifactResolveError::SandboxRejected)?;
        match fs::symlink_metadata(storage_root) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(ArtifactResolveError::SandboxRejected)
            }
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(ArtifactResolveError::CachePublishFailed),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(ArtifactResolveError::CachePublishFailed),
        }
    }

    pub(crate) fn resolve(
        &mut self,
        request: ArtifactResolveRequest<'_>,
    ) -> Result<ResolvedArtifact, ArtifactResolveError> {
        self.resolve_with_policy(request, ArtifactResolvePolicy::RecipeRemoteFile)
    }

    /// Resolve an App Definition-owned direct URL and retain a verified,
    /// per-run snapshot for downstream path consumers.
    pub(crate) fn resolve_direct_url(
        &mut self,
        request: ArtifactResolveRequest<'_>,
        expected_sha256: Option<&str>,
    ) -> Result<ResolvedArtifact, ArtifactResolveError> {
        self.resolve_with_policy(
            request,
            ArtifactResolvePolicy::DirectUrl { expected_sha256 },
        )
    }

    fn resolve_with_policy(
        &mut self,
        request: ArtifactResolveRequest<'_>,
        policy: ArtifactResolvePolicy<'_>,
    ) -> Result<ResolvedArtifact, ArtifactResolveError> {
        let admitted = self.admit_with_policy(request, policy)?;
        let is_direct_url = matches!(policy, ArtifactResolvePolicy::DirectUrl { .. });
        let expected_sha256 = match policy {
            ArtifactResolvePolicy::RecipeRemoteFile => None,
            ArtifactResolvePolicy::DirectUrl { expected_sha256 } => expected_sha256,
        };
        if matches!(admitted.source, AdmittedArtifactSource::CacheHit) {
            if is_direct_url {
                let (verified_path_guard, calculated_sha256) =
                    snapshot_verified_file(&admitted.final_path, self.sandbox, expected_sha256)?;
                return Ok(ResolvedArtifact {
                    local_path: verified_path_guard.to_path_buf(),
                    filename: admitted.filename,
                    cache_hit: true,
                    calculated_sha256: Some(calculated_sha256),
                    redacted_final_url: None,
                    verified_path_guard: Some(verified_path_guard),
                });
            }
            return Ok(ResolvedArtifact {
                local_path: admitted.final_path,
                filename: admitted.filename,
                cache_hit: true,
                calculated_sha256: None,
                redacted_final_url: None,
                verified_path_guard: None,
            });
        }

        let parent = admitted
            .final_path
            .parent()
            .expect("artifact destination has a parent");
        fs::create_dir_all(parent).map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
        self.sandbox
            .ensure_runtime_or_cache_write(&admitted.final_path)
            .map_err(|_| ArtifactResolveError::SandboxRejected)?;
        let mut partial = TempFileBuilder::new()
            .prefix(".emuchef-artifact-")
            .suffix(".partial")
            .tempfile_in(parent)
            .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;

        let redirect_policy = if is_direct_url {
            RedirectPolicy::PublicHttps
        } else {
            RedirectPolicy::Existing
        };
        let transfer_result = match admitted.source {
            AdmittedArtifactSource::LocalFile(source_path) => self
                .local_transport
                .download(&source_path, partial.as_file_mut()),
            AdmittedArtifactSource::Http(parsed_url) => {
                let transport = self.http_transport()?;
                if is_direct_url {
                    transport.download_with_policy(
                        &parsed_url,
                        partial.as_file_mut(),
                        redirect_policy,
                    )
                } else {
                    transport.download(&parsed_url, partial.as_file_mut())
                }
            }
            AdmittedArtifactSource::CacheHit => unreachable!("cache hit returned above"),
        };
        let transfer_metadata = match transfer_result {
            Ok(metadata) => metadata,
            Err(error) => return Err(cleanup_partial(partial, error, false)),
        };

        let downloaded_sha256 = if is_direct_url {
            if partial.as_file_mut().flush().is_err() {
                return Err(cleanup_partial(
                    partial,
                    ArtifactResolveError::CacheWriteFailed,
                    false,
                ));
            }
            let calculated_sha256 = match calculate_file_sha256(partial.path()) {
                Ok(calculated_sha256) => calculated_sha256,
                Err(error) => return Err(cleanup_partial(partial, error, false)),
            };
            if let Err(error) = verify_expected_sha256(expected_sha256, &calculated_sha256) {
                return Err(cleanup_partial(partial, error, false));
            }
            Some(calculated_sha256)
        } else {
            None
        };

        let payload_fingerprint = partial.as_file().metadata().ok().map(|metadata| {
            let modified_nanos = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos());
            (metadata.len(), modified_nanos)
        });
        let prepared_metadata = if admitted.default_cache {
            payload_fingerprint.and_then(|(size, modified_nanos)| {
                prepare_metadata(
                    &admitted.final_path,
                    request.artifact_id,
                    request.url,
                    size,
                    modified_nanos,
                )
            })
        } else {
            None
        };
        let (published_path, cache_hit) =
            finish_partial(partial, &admitted.final_path, admitted.default_cache, None)?;
        if !cache_hit {
            if let Some(metadata) = prepared_metadata.as_ref() {
                // Metadata is optional support state. Failure intentionally
                // leaves the payload usable and unindexed.
                let _ = publish_metadata(&published_path, metadata);
            }
        }

        if is_direct_url {
            let snapshot = snapshot_verified_file(&published_path, self.sandbox, expected_sha256);
            if !admitted.default_cache {
                let _ = fs::remove_file(&published_path);
            }
            let (verified_path_guard, calculated_sha256) = snapshot?;
            if !cache_hit && downloaded_sha256.as_deref() != Some(calculated_sha256.as_str()) {
                return Err(ArtifactResolveError::ArtifactChangedDuringResolution);
            }
            let redacted_final_url = if cache_hit {
                None
            } else {
                redacted_download_url(&transfer_metadata)
            };
            return Ok(ResolvedArtifact {
                local_path: verified_path_guard.to_path_buf(),
                filename: admitted.filename,
                cache_hit,
                calculated_sha256: Some(calculated_sha256),
                redacted_final_url,
                verified_path_guard: Some(verified_path_guard),
            });
        }

        Ok(ResolvedArtifact {
            local_path: published_path,
            filename: admitted.filename,
            cache_hit,
            calculated_sha256: None,
            redacted_final_url: None,
            verified_path_guard: None,
        })
    }

    fn http_transport(&mut self) -> Result<&HttpArtifactTransport, ArtifactResolveError> {
        if self.http_transport.is_none() {
            self.http_transport = Some(HttpArtifactTransport::new(HttpClientConfig::default())?);
        }
        self.http_transport
            .as_ref()
            .ok_or(ArtifactResolveError::DownloadFailed)
    }
}

fn validate_artifact_definition(
    request: ArtifactResolveRequest<'_>,
) -> Result<(), ArtifactResolveError> {
    if request.type_name != "remote_file" {
        return Err(ArtifactResolveError::TypeUnsupported);
    }
    if !matches!(request.cache_mode, "default" | "none") {
        return Err(ArtifactResolveError::CacheModeUnsupported);
    }
    Ok(())
}

fn check_source_readable(path: &Path) -> io::Result<()> {
    File::open(path).map(drop)
}

fn validate_expected_sha256(expected_sha256: Option<&str>) -> Result<(), ArtifactResolveError> {
    if expected_sha256.is_some_and(|value| !crate::authored_models::is_lowercase_sha256(value)) {
        return Err(ArtifactResolveError::ExpectedSha256Invalid);
    }
    Ok(())
}

fn verify_expected_sha256(
    expected_sha256: Option<&str>,
    calculated_sha256: &str,
) -> Result<(), ArtifactResolveError> {
    if expected_sha256.is_some_and(|expected| expected != calculated_sha256) {
        return Err(ArtifactResolveError::ExpectedSha256Mismatch);
    }
    Ok(())
}

fn calculate_file_sha256(path: &Path) -> Result<String, ArtifactResolveError> {
    let mut file = File::open(path).map_err(|_| ArtifactResolveError::Sha256Unavailable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| ArtifactResolveError::Sha256Unavailable)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_digest(digest.finalize().as_slice()))
}

fn snapshot_verified_file(
    source_path: &Path,
    sandbox: &SandboxRoots,
    expected_sha256: Option<&str>,
) -> Result<(Arc<TempPath>, String), ArtifactResolveError> {
    let verified_root = sandbox.runtime_root.join("verified-artifacts");
    sandbox
        .ensure_runtime_or_cache_write(&verified_root)
        .map_err(|_| ArtifactResolveError::SandboxRejected)?;
    match fs::symlink_metadata(&verified_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ArtifactResolveError::SandboxRejected);
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(ArtifactResolveError::CacheWriteFailed);
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(ArtifactResolveError::CacheWriteFailed),
    }
    fs::create_dir_all(&verified_root).map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    sandbox
        .ensure_runtime_or_cache_write(&verified_root)
        .map_err(|_| ArtifactResolveError::SandboxRejected)?;

    let source_metadata =
        fs::symlink_metadata(source_path).map_err(|_| ArtifactResolveError::Sha256Unavailable)?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Err(ArtifactResolveError::SandboxRejected);
    }
    let mut source =
        File::open(source_path).map_err(|_| ArtifactResolveError::Sha256Unavailable)?;
    let mut snapshot = TempFileBuilder::new()
        .prefix(".emuchef-verified-")
        .suffix(".apk")
        .tempfile_in(&verified_root)
        .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| ArtifactResolveError::Sha256Unavailable)?;
        if count == 0 {
            break;
        }
        snapshot
            .as_file_mut()
            .write_all(&buffer[..count])
            .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    }
    snapshot
        .as_file_mut()
        .flush()
        .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    snapshot
        .as_file()
        .sync_all()
        .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    let calculated_sha256 = calculate_file_sha256(snapshot.path())?;
    verify_expected_sha256(expected_sha256, &calculated_sha256)?;
    Ok((Arc::new(snapshot.into_temp_path()), calculated_sha256))
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_local_source(
    path: &Path,
    source_readability_check: SourceReadabilityCheck,
) -> Result<(), ArtifactResolveError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(ArtifactResolveError::SourceWrongKind),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ArtifactResolveError::SourceNotFound)
        }
        Err(_) => return Err(ArtifactResolveError::SourceUnreadable),
    }
    source_readability_check(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ArtifactResolveError::SourceNotFound
        } else {
            ArtifactResolveError::SourceUnreadable
        }
    })
}

fn validate_source_url(url: &Url) -> Result<(), ArtifactResolveError> {
    match url.scheme() {
        "file" if file_url_to_path(url.as_str()).is_some_and(|path| path.is_absolute()) => Ok(()),
        "file" => Err(ArtifactResolveError::UrlInvalid),
        "http" | "https" if url.has_host() => Ok(()),
        "http" | "https" => Err(ArtifactResolveError::UrlInvalid),
        scheme => Err(ArtifactResolveError::SchemeUnsupported {
            scheme: scheme.to_string(),
        }),
    }
}

fn existing_regular_file(path: &Path) -> Result<bool, ArtifactResolveError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ArtifactResolveError::SandboxRejected)
        }
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(ArtifactResolveError::CachePublishFailed),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ArtifactResolveError::CachePublishFailed),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicationFault {
    Sync,
    Publish,
    Cleanup,
}

fn finish_partial(
    mut partial: NamedTempFile,
    final_path: &Path,
    default_cache: bool,
    fault: Option<PublicationFault>,
) -> Result<(PathBuf, bool), ArtifactResolveError> {
    if fault == Some(PublicationFault::Cleanup) {
        return Err(cleanup_partial(
            partial,
            ArtifactResolveError::CachePublishFailed,
            true,
        ));
    }
    if fault == Some(PublicationFault::Sync)
        || partial.as_file_mut().flush().is_err()
        || partial.as_file().sync_all().is_err()
    {
        return Err(cleanup_partial(
            partial,
            ArtifactResolveError::CacheWriteFailed,
            false,
        ));
    }
    if fault == Some(PublicationFault::Publish) {
        return Err(cleanup_partial(
            partial,
            ArtifactResolveError::CachePublishFailed,
            false,
        ));
    }
    publish_partial(partial, final_path, default_cache)
}

fn publish_partial(
    mut partial: NamedTempFile,
    deterministic_path: &Path,
    default_cache: bool,
) -> Result<(PathBuf, bool), ArtifactResolveError> {
    let unique_token = partial
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact")
        .trim_start_matches(".emuchef-artifact-")
        .trim_end_matches(".partial")
        .to_string();
    let mut destination = deterministic_path.to_path_buf();
    loop {
        match partial.persist_noclobber(&destination) {
            Ok(_) => return Ok((destination, false)),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists && default_cache => {
                let cleanup_error = error.file.close().err();
                if !existing_regular_file(&destination)? {
                    return Err(ArtifactResolveError::CachePublishFailed);
                }
                if cleanup_error.is_some() {
                    return Err(ArtifactResolveError::PartialCleanupFailed {
                        primary: Box::new(ArtifactResolveError::CachePublishFailed),
                    });
                }
                return Ok((destination, true));
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                partial = error.file;
                let filename = deterministic_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("artifact");
                destination =
                    deterministic_path.with_file_name(format!("{filename}.{unique_token}"));
                if existing_regular_file(&destination)? {
                    return Err(cleanup_partial(
                        partial,
                        ArtifactResolveError::CachePublishFailed,
                        false,
                    ));
                }
            }
            Err(error) => {
                return Err(cleanup_partial(
                    error.file,
                    ArtifactResolveError::CachePublishFailed,
                    false,
                ));
            }
        }
    }
}

fn cleanup_partial(
    partial: NamedTempFile,
    primary: ArtifactResolveError,
    simulate_failure: bool,
) -> ArtifactResolveError {
    if simulate_failure || partial.close().is_err() {
        ArtifactResolveError::PartialCleanupFailed {
            primary: Box::new(primary),
        }
    } else {
        primary
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

pub(crate) fn redacted_url(url: &str) -> Option<String> {
    let mut parsed = Url::parse(url).ok()?;
    parsed.set_username("").ok()?;
    parsed.set_password(None).ok()?;
    parsed.set_query(None);
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

fn redacted_download_url(metadata: &DownloadMetadata) -> Option<String> {
    metadata
        .observed_final_url
        .as_deref()
        .and_then(redacted_url)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    fn sandbox(root: &Path) -> SandboxRoots {
        SandboxRoots {
            runtime_root: root.join("runtime"),
            cache_root: root.join("cache"),
            fake_device_root: root.join("device"),
            read_only_roots: vec![root.to_path_buf()],
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        hex_digest(&Sha256::digest(bytes))
    }

    struct HttpsFixture {
        address: SocketAddr,
        certificate_der: Vec<u8>,
        requests: Arc<AtomicUsize>,
        stop: Arc<std::sync::atomic::AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl HttpsFixture {
        fn spawn(handler: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> Self {
            use rcgen::{string::Ia5String, CertificateParams, KeyPair, SanType};
            use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
            use rustls::{ServerConfig, ServerConnection, StreamOwned};

            const HOST: &str = "downloads.example.test";
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let mut params = CertificateParams::default();
            params.subject_alt_names = vec![SanType::DnsName(
                Ia5String::try_from(HOST).expect("fixture DNS name is valid"),
            )];
            let key = KeyPair::generate().unwrap();
            let certificate = params.self_signed(&key).unwrap();
            let certificate_der = certificate.der().to_vec();
            let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
            let config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate.der().clone()], private_key)
                .unwrap();
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let thread_requests = Arc::clone(&requests);
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let handler = Arc::new(handler);
            let config = Arc::new(config);
            let thread = thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            let connection = ServerConnection::new(Arc::clone(&config)).unwrap();
                            let mut stream = StreamOwned::new(connection, stream);
                            let mut request = [0u8; 4096];
                            let Ok(count) = stream.read(&mut request) else {
                                continue;
                            };
                            let request = String::from_utf8_lossy(&request[..count]);
                            let target = request
                                .lines()
                                .next()
                                .and_then(|line| line.split_whitespace().nth(1))
                                .unwrap_or("/");
                            thread_requests.fetch_add(1, Ordering::Relaxed);
                            let response = handler(target);
                            let _ = stream.write_all(&response);
                            let _ = stream.flush();
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                address,
                certificate_der,
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn url(&self, path: &str) -> String {
            format!(
                "https://downloads.example.test:{}{path}",
                self.address.port()
            )
        }

        fn request_count(&self) -> usize {
            self.requests.load(Ordering::Relaxed)
        }

        fn resolver<'a>(
            &self,
            sandbox: &'a SandboxRoots,
            policy_address: IpAddr,
        ) -> ArtifactResolver<'a> {
            let mut resolver = ArtifactResolver::new(sandbox);
            resolver.http_transport = Some(
                HttpArtifactTransport::with_test_root_and_host_mapping(
                    HttpClientConfig {
                        connect_timeout: Duration::from_secs(1),
                        total_timeout: Duration::from_secs(3),
                        use_system_proxy: false,
                    },
                    &self.certificate_der,
                    "downloads.example.test",
                    policy_address,
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                )
                .unwrap(),
            );
            resolver
        }

        fn stop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    impl Drop for HttpsFixture {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn tls_response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn public_test_address() -> IpAddr {
        "1.1.1.1".parse().unwrap()
    }

    #[test]
    fn direct_https_resolver_downloads_fresh_bytes_with_and_without_trusted_sha256() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        let body = b"fresh HTTPS APK bytes";
        let fixture = HttpsFixture::spawn(move |_| tls_response("200 OK", "", body));
        let expected_sha256 = sha256(body);

        for (path, trusted_sha256) in [
            ("/without-checksum.apk", None),
            ("/with-checksum.apk", Some(expected_sha256.as_str())),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let roots = sandbox(temp.path());
            let url = fixture.url(path);
            let resolved = fixture
                .resolver(&roots, public_test_address())
                .resolve_direct_url(
                    ArtifactResolveRequest {
                        artifact_id: ARTIFACT_ID,
                        type_name: "remote_file",
                        url: &url,
                        cache_mode: "default",
                    },
                    trusted_sha256,
                )
                .unwrap();

            assert!(!resolved.cache_hit);
            assert_eq!(fs::read(&resolved.local_path).unwrap(), body);
            assert_eq!(
                resolved.calculated_sha256.as_deref(),
                Some(expected_sha256.as_str())
            );
            assert_eq!(
                resolved.redacted_final_url.as_deref(),
                Some(
                    format!(
                        "https://downloads.example.test:{}{path}",
                        fixture.address.port()
                    )
                    .as_str()
                )
            );
            assert!(roots
                .cache_root
                .join(artifact_local_filename(ARTIFACT_ID, &url, "default"))
                .is_file());
        }
        assert_eq!(fixture.request_count(), 2);
    }

    #[test]
    fn direct_https_checksum_mismatch_cleans_partial_before_cache_publication() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        let fixture = HttpsFixture::spawn(|_| tls_response("200 OK", "", b"wrong artifact"));
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let url = fixture.url("/app.apk");
        let expected_sha256 = sha256(b"trusted artifact");

        let error = fixture
            .resolver(&roots, public_test_address())
            .resolve_direct_url(
                ArtifactResolveRequest {
                    artifact_id: ARTIFACT_ID,
                    type_name: "remote_file",
                    url: &url,
                    cache_mode: "default",
                },
                Some(&expected_sha256),
            )
            .unwrap_err();

        assert_eq!(error.code(), "artifact_sha256_mismatch");
        assert_eq!(fixture.request_count(), 1);
        assert_eq!(fs::read_dir(&roots.cache_root).unwrap().count(), 0);
    }

    #[test]
    fn direct_https_cache_none_keeps_only_the_verified_runtime_snapshot() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        let body = b"uncached HTTPS APK bytes";
        let fixture = HttpsFixture::spawn(move |_| tls_response("200 OK", "", body));
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let url = fixture.url("/app.apk");

        let resolved = fixture
            .resolver(&roots, public_test_address())
            .resolve_direct_url(
                ArtifactResolveRequest {
                    artifact_id: ARTIFACT_ID,
                    type_name: "remote_file",
                    url: &url,
                    cache_mode: "none",
                },
                None,
            )
            .unwrap();

        assert!(!resolved.cache_hit);
        assert_eq!(fs::read(&resolved.local_path).unwrap(), body);
        assert!(resolved
            .local_path
            .starts_with(roots.runtime_root.join("verified-artifacts")));
        assert_eq!(
            fs::read_dir(roots.runtime_root.join("downloads"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(fixture.request_count(), 1);
    }

    #[test]
    fn direct_https_resolver_records_and_redacts_a_signed_redirect_destination() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        const SIGNED_PATH: &str =
            "/signed.apk?X-Amz-Credential=private-signed-value&X-Amz-Signature=sig";
        let fixture = HttpsFixture::spawn(|target| match target {
            "/start" => tls_response("302 Found", &format!("Location: {SIGNED_PATH}\r\n"), b""),
            SIGNED_PATH => tls_response("200 OK", "", b"redirected APK bytes"),
            _ => tls_response("404 Not Found", "", b""),
        });
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let url = fixture.url("/start");

        let resolved = fixture
            .resolver(&roots, public_test_address())
            .resolve_direct_url(
                ArtifactResolveRequest {
                    artifact_id: ARTIFACT_ID,
                    type_name: "remote_file",
                    url: &url,
                    cache_mode: "default",
                },
                None,
            )
            .unwrap();

        assert_eq!(fixture.request_count(), 2);
        assert_eq!(
            resolved.redacted_final_url.as_deref(),
            Some(
                format!(
                    "https://downloads.example.test:{}/signed.apk",
                    fixture.address.port()
                )
                .as_str()
            )
        );
        assert!(!resolved
            .redacted_final_url
            .as_deref()
            .unwrap()
            .contains("private-signed-value"));
    }

    #[test]
    fn direct_https_rejects_private_dns_answers_before_connecting() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        for answer in ["10.20.30.40", "0.1.2.3"] {
            let fixture =
                HttpsFixture::spawn(|_| tls_response("200 OK", "", b"must not be fetched"));
            let temp = tempfile::tempdir().unwrap();
            let roots = sandbox(temp.path());
            let url = fixture.url("/app.apk");

            let error = fixture
                .resolver(&roots, answer.parse().unwrap())
                .resolve_direct_url(
                    ArtifactResolveRequest {
                        artifact_id: ARTIFACT_ID,
                        type_name: "remote_file",
                        url: &url,
                        cache_mode: "default",
                    },
                    None,
                )
                .unwrap_err();

            assert_eq!(
                error.code(),
                "artifact_redirect_policy_rejected",
                "DNS answer {answer} must be rejected"
            );
            assert_eq!(fixture.request_count(), 0, "DNS answer {answer}");
            assert_eq!(fs::read_dir(&roots.cache_root).unwrap().count(), 0);
        }
    }

    #[test]
    fn direct_https_cache_publication_race_hashes_and_reports_the_winning_file() {
        const ARTIFACT_ID: &str = "app.obtainium.install/obtainium_apk";
        const DOWNLOADED: &[u8] = b"response that lost publication";
        const CACHE_WINNER: &[u8] = b"concurrent verified cache winner";
        let request_arrived = Arc::new(std::sync::Barrier::new(2));
        let response_released = Arc::new(std::sync::Barrier::new(2));
        let fixture = HttpsFixture::spawn({
            let request_arrived = Arc::clone(&request_arrived);
            let response_released = Arc::clone(&response_released);
            move |_| {
                request_arrived.wait();
                response_released.wait();
                tls_response("200 OK", "", DOWNLOADED)
            }
        });
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let url = fixture.url("/race.apk");
        let cache_path =
            roots
                .cache_root
                .join(artifact_local_filename(ARTIFACT_ID, &url, "default"));
        let mut resolver = fixture.resolver(&roots, public_test_address());

        let resolved = std::thread::scope(|scope| {
            let request = ArtifactResolveRequest {
                artifact_id: ARTIFACT_ID,
                type_name: "remote_file",
                url: &url,
                cache_mode: "default",
            };
            let resolution = scope.spawn(move || resolver.resolve_direct_url(request, None));
            request_arrived.wait();
            fs::write(&cache_path, CACHE_WINNER).unwrap();
            response_released.wait();
            resolution.join().unwrap().unwrap()
        });

        assert!(resolved.cache_hit);
        assert_eq!(fs::read(&resolved.local_path).unwrap(), CACHE_WINNER);
        assert_eq!(fs::read(&cache_path).unwrap(), CACHE_WINNER);
        assert_eq!(
            resolved.calculated_sha256.as_deref(),
            Some(sha256(CACHE_WINNER).as_str())
        );
        assert!(resolved.verified_path_guard.is_some());
        assert_eq!(resolved.redacted_final_url, None);
        assert_eq!(fixture.request_count(), 1);
    }

    fn spawn_http_server(
        bodies: Vec<&'static [u8]>,
    ) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let thread_requests = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request).unwrap();
                thread_requests.fetch_add(1, Ordering::Relaxed);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        (format!("http://{address}"), requests, thread)
    }

    fn request() -> ArtifactResolveRequest<'static> {
        ArtifactResolveRequest {
            artifact_id: "example.recipe/archive",
            type_name: "remote_file",
            url: "https://user:password@example.com/archive.zip?token=secret#private",
            cache_mode: "default",
        }
    }

    #[test]
    fn every_artifact_error_has_a_stable_code_and_message() {
        let failures = vec![
            ArtifactResolveError::TypeUnsupported,
            ArtifactResolveError::CacheModeUnsupported,
            ArtifactResolveError::UrlInvalid,
            ArtifactResolveError::SchemeUnsupported {
                scheme: "ftp".to_string(),
            },
            ArtifactResolveError::SourceNotFound,
            ArtifactResolveError::SourceWrongKind,
            ArtifactResolveError::SourceUnreadable,
            ArtifactResolveError::DownloadFailed,
            ArtifactResolveError::HttpStatus { status: 404 },
            ArtifactResolveError::RedirectLimitExceeded { redirects: 6 },
            ArtifactResolveError::RedirectDowngradeRejected,
            ArtifactResolveError::RedirectPolicyRejected,
            ArtifactResolveError::ConnectTimeout,
            ArtifactResolveError::RequestTimeout,
            ArtifactResolveError::TlsVerificationFailed,
            ArtifactResolveError::ResponseIncomplete,
            ArtifactResolveError::ResponseTooLarge,
            ArtifactResolveError::Sha256Unavailable,
            ArtifactResolveError::ExpectedSha256Invalid,
            ArtifactResolveError::ExpectedSha256Mismatch,
            ArtifactResolveError::ArtifactChangedDuringResolution,
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

    #[test]
    fn direct_url_cache_hit_rejects_source_policy_before_accepting_cached_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        fs::create_dir_all(&roots.cache_root).unwrap();
        let url = "https://user:secret@example.com/app.apk?token=private#fragment";
        let artifact_id = "app.obtainium.install/obtainium_apk";
        let cached_path =
            roots
                .cache_root
                .join(artifact_local_filename(artifact_id, url, "default"));
        fs::write(&cached_path, b"apparently cached").unwrap();

        let error = ArtifactResolver::new(&roots)
            .resolve_direct_url(
                ArtifactResolveRequest {
                    artifact_id,
                    type_name: "remote_file",
                    url,
                    cache_mode: "default",
                },
                None,
            )
            .unwrap_err();

        assert_eq!(error.code(), "artifact_url_invalid");
        assert!(!error
            .executor_message(ArtifactResolveRequest {
                artifact_id,
                type_name: "remote_file",
                url,
                cache_mode: "default",
            })
            .contains("private"));
        assert!(!roots.runtime_root.join("verified-artifacts").exists());
    }

    #[test]
    fn direct_url_admission_accepts_signed_https_with_cache_none() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let url = "https://downloads.example.com/app.apk?signature=signed-value";
        let admitted = ArtifactResolver::new(&roots)
            .admit_direct_url(
                ArtifactResolveRequest {
                    artifact_id: "app.obtainium.install/obtainium_apk",
                    type_name: "remote_file",
                    url,
                    cache_mode: "none",
                },
                None,
            )
            .unwrap();

        assert!(!admitted.default_cache);
        assert!(admitted
            .final_path
            .starts_with(roots.runtime_root.join("downloads")));
        let AdmittedArtifactSource::Http(parsed_url) = admitted.source else {
            panic!("a direct public HTTPS URL should use the HTTP transport");
        };
        assert_eq!(parsed_url.as_str(), url);
    }

    #[test]
    fn direct_url_admission_rejects_invalid_expected_sha_before_cache_acceptance() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        fs::create_dir_all(&roots.cache_root).unwrap();
        let url = "https://downloads.example.com/app.apk?signature=signed-value";
        let artifact_id = "app.obtainium.install/obtainium_apk";
        let cached_path =
            roots
                .cache_root
                .join(artifact_local_filename(artifact_id, url, "default"));
        fs::write(&cached_path, b"cached bytes").unwrap();
        let invalid_sha256 = "A".repeat(64);

        let error = ArtifactResolver::new(&roots)
            .admit_direct_url(
                ArtifactResolveRequest {
                    artifact_id,
                    type_name: "remote_file",
                    url,
                    cache_mode: "default",
                },
                Some(&invalid_sha256),
            )
            .unwrap_err();

        assert_eq!(error.code(), "artifact_sha256_invalid");
        assert!(!roots.runtime_root.join("verified-artifacts").exists());
    }

    #[test]
    fn direct_url_cache_hit_rejects_corrupted_bytes_and_uses_a_private_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        fs::create_dir_all(&roots.cache_root).unwrap();
        let url = "https://downloads.example.com/app.apk?signature=signed-value";
        let artifact_id = "app.obtainium.install/obtainium_apk";
        let cached_path =
            roots
                .cache_root
                .join(artifact_local_filename(artifact_id, url, "default"));
        fs::write(&cached_path, b"corrupt cached artifact").unwrap();
        let expected_sha256 = sha256(b"trusted artifact");
        let request = ArtifactResolveRequest {
            artifact_id,
            type_name: "remote_file",
            url,
            cache_mode: "default",
        };
        let mut resolver = ArtifactResolver::new(&roots);
        let error = resolver
            .resolve_direct_url(request, Some(&expected_sha256))
            .unwrap_err();
        assert_eq!(error.code(), "artifact_sha256_mismatch");
        assert!(!roots
            .runtime_root
            .join("verified-artifacts")
            .read_dir()
            .is_ok_and(|mut entries| entries.next().is_some()));

        fs::write(&cached_path, b"trusted artifact").unwrap();
        let resolved = resolver
            .resolve_direct_url(request, Some(&expected_sha256))
            .unwrap();
        assert!(resolved.cache_hit);
        assert_eq!(
            resolved.calculated_sha256.as_deref(),
            Some(expected_sha256.as_str())
        );
        assert_eq!(fs::read(&resolved.local_path).unwrap(), b"trusted artifact");
        assert!(resolved.verified_path_guard.is_some());

        fs::write(&cached_path, b"changed shared cache").unwrap();
        assert_eq!(fs::read(&resolved.local_path).unwrap(), b"trusted artifact");
    }

    #[test]
    fn concurrent_cache_publication_hashes_the_winning_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        fs::create_dir_all(&roots.cache_root).unwrap();
        let destination = roots.cache_root.join("artifact.apk");
        let candidates: [&[u8]; 2] = [b"first concurrent artifact", b"second concurrent artifact"];
        let barrier = Arc::new(std::sync::Barrier::new(candidates.len()));
        let writers = candidates
            .iter()
            .copied()
            .map(|bytes| {
                let destination = destination.clone();
                let cache_root = roots.cache_root.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut partial = TempFileBuilder::new()
                        .prefix(".emuchef-artifact-")
                        .suffix(".partial")
                        .tempfile_in(cache_root)
                        .unwrap();
                    partial.write_all(bytes).unwrap();
                    barrier.wait();
                    finish_partial(partial, &destination, true, None).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let outcomes = writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            outcomes.iter().filter(|(_, cache_hit)| !cache_hit).count(),
            1
        );
        assert!(outcomes.iter().all(|(path, _)| path == &destination));
        let (verified_path, calculated_sha256) =
            snapshot_verified_file(&destination, &roots, None).unwrap();
        let winner_bytes = fs::read(verified_path.as_ref()).unwrap();
        assert!(candidates.contains(&winner_bytes.as_slice()));
        assert_eq!(calculated_sha256, sha256(&winner_bytes));

        let losing_bytes = candidates
            .iter()
            .copied()
            .find(|candidate| *candidate != winner_bytes.as_slice())
            .unwrap();
        let losing_checksum = sha256(losing_bytes);
        assert_eq!(
            verify_expected_sha256(Some(&losing_checksum), &calculated_sha256)
                .unwrap_err()
                .code(),
            "artifact_sha256_mismatch"
        );
    }

    #[test]
    fn download_url_redaction_removes_credentials_query_and_fragment() {
        let metadata = DownloadMetadata {
            bytes_written: 12,
            content_length: Some(12),
            observed_final_url: Some(
                "https://user:private@cdn.example.com/app.apk?signature=signed-value#fragment"
                    .to_string(),
            ),
        };

        assert_eq!(
            redacted_download_url(&metadata).as_deref(),
            Some("https://cdn.example.com/app.apk")
        );
        assert_eq!(
            redacted_download_url(&metadata).as_deref(),
            Some("https://cdn.example.com/app.apk")
        );
    }

    #[test]
    fn malformed_url_credentials_are_not_echoed_in_redacted_diagnostics() {
        assert_eq!(
            redacted_url("https://user:private@example.com/app.apk?token=private#fragment")
                .as_deref(),
            Some("https://example.com/app.apk")
        );
        assert_eq!(redacted_url("https://user:private"), None);
    }

    #[test]
    fn admission_classifies_cold_http_local_files_and_authoritative_cache_hits_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let resolver = ArtifactResolver::new(&roots);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/artifact.bin", listener.local_addr().unwrap());
        let admitted = resolver
            .admit(ArtifactResolveRequest {
                artifact_id: "example/http",
                type_name: "remote_file",
                url: &url,
                cache_mode: "none",
            })
            .unwrap();
        assert!(matches!(admitted.source, AdmittedArtifactSource::Http(_)));
        assert!(resolver.http_transport.is_none());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!roots.runtime_root.exists());
        assert!(!roots.cache_root.exists());

        let source = temp.path().join("source.bin");
        fs::write(&source, b"source").unwrap();
        let file_url = format!("file://{}", source.display());
        let admitted = resolver
            .admit(ArtifactResolveRequest {
                artifact_id: "example/local",
                type_name: "remote_file",
                url: &file_url,
                cache_mode: "none",
            })
            .unwrap();
        assert!(matches!(
            admitted.source,
            AdmittedArtifactSource::LocalFile(path) if path == source
        ));
        assert!(!roots.runtime_root.exists());

        fs::create_dir_all(&roots.cache_root).unwrap();
        let malformed_url = "not a valid URL";
        let cached_path = roots.cache_root.join(artifact_local_filename(
            "example/cached",
            malformed_url,
            "default",
        ));
        fs::write(&cached_path, b"cached").unwrap();
        let mut before = fs::read_dir(&roots.cache_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        before.sort();
        let admitted = resolver
            .admit(ArtifactResolveRequest {
                artifact_id: "example/cached",
                type_name: "remote_file",
                url: malformed_url,
                cache_mode: "default",
            })
            .unwrap();
        assert!(matches!(admitted.source, AdmittedArtifactSource::CacheHit));
        assert_eq!(admitted.final_path, cached_path);
        assert_eq!(fs::read(&cached_path).unwrap(), b"cached");
        let mut after = fs::read_dir(&roots.cache_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        after.sort();
        assert_eq!(after, before);
    }

    #[test]
    fn admission_rejects_unsupported_definitions_before_using_cache_or_source() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        fs::create_dir_all(&roots.cache_root).unwrap();
        let cached_path = roots.cache_root.join(artifact_local_filename(
            "example/cached",
            "not a valid URL",
            "default",
        ));
        fs::write(cached_path, b"cached").unwrap();
        let resolver = ArtifactResolver::new(&roots);

        let unsupported_type = ArtifactResolveRequest {
            artifact_id: "example/cached",
            type_name: "archive",
            url: "not a valid URL",
            cache_mode: "default",
        };
        assert_eq!(
            resolver.admit(unsupported_type).unwrap_err().code(),
            "artifact_type_unsupported"
        );
        let unsupported_cache = ArtifactResolveRequest {
            type_name: "remote_file",
            cache_mode: "forever",
            ..unsupported_type
        };
        assert_eq!(
            resolver.admit(unsupported_cache).unwrap_err().code(),
            "artifact_cache_mode_unsupported"
        );
    }

    #[test]
    fn admission_classifies_local_source_failures_without_permission_assumptions() {
        fn permission_denied(_: &Path) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }

        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let source = temp.path().join("source.bin");
        fs::write(&source, b"source").unwrap();
        let source_url = format!("file://{}", source.display());
        let request = ArtifactResolveRequest {
            artifact_id: "example/local",
            type_name: "remote_file",
            url: &source_url,
            cache_mode: "none",
        };
        let unreadable = ArtifactResolver::with_source_readability_check(&roots, permission_denied)
            .admit(request)
            .unwrap_err();
        assert_eq!(unreadable.code(), "artifact_source_unreadable");

        let directory = temp.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let directory_url = format!("file://{}", directory.display());
        let resolver = ArtifactResolver::new(&roots);
        let wrong_kind = resolver
            .admit(ArtifactResolveRequest {
                url: &directory_url,
                ..request
            })
            .unwrap_err();
        assert_eq!(wrong_kind.code(), "artifact_source_wrong_kind");

        let missing = temp.path().join("missing.bin");
        let missing_url = format!("file://{}", missing.display());
        let missing = resolver
            .admit(ArtifactResolveRequest {
                url: &missing_url,
                ..request
            })
            .unwrap_err();
        assert_eq!(missing.code(), "artifact_source_not_found");

        let restricted_root = temp.path().join("restricted");
        let restricted_roots = SandboxRoots {
            runtime_root: restricted_root.join("runtime"),
            cache_root: restricted_root.join("cache"),
            fake_device_root: restricted_root.join("device"),
            read_only_roots: Vec::new(),
        };
        let rejected = ArtifactResolver::new(&restricted_roots)
            .admit(request)
            .unwrap_err();
        assert_eq!(rejected.code(), "artifact_sandbox_rejected");
        assert!(!roots.runtime_root.exists());
        assert!(!roots.cache_root.exists());
    }

    #[test]
    fn late_bound_admission_checks_source_type_cache_and_storage_root_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let resolver = ArtifactResolver::new(&roots);

        resolver.admit_late_bound("remote_file", "default").unwrap();
        assert!(matches!(
            resolver.admit_late_bound("app_artifact", "default"),
            Err(ArtifactResolveError::TypeUnsupported)
        ));
        assert!(matches!(
            resolver.admit_late_bound("remote_file", "invalid"),
            Err(ArtifactResolveError::CacheModeUnsupported)
        ));
        assert!(!roots.runtime_root.exists());
        assert!(!roots.cache_root.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = temp.path().join("outside");
            fs::create_dir(&outside).unwrap();
            symlink(&outside, &roots.cache_root).unwrap();
            assert!(matches!(
                resolver.admit_late_bound("remote_file", "default"),
                Err(ArtifactResolveError::SandboxRejected)
            ));
            assert!(fs::read_dir(&outside).unwrap().next().is_none());
        }
    }

    #[test]
    fn admission_rejects_non_file_cache_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let request = ArtifactResolveRequest {
            artifact_id: "example/cached",
            type_name: "remote_file",
            url: "https://example.com/artifact.bin",
            cache_mode: "default",
        };
        let final_path = roots.cache_root.join(artifact_local_filename(
            request.artifact_id,
            request.url,
            request.cache_mode,
        ));
        fs::create_dir_all(&final_path).unwrap();
        let error = ArtifactResolver::new(&roots).admit(request).unwrap_err();
        assert_eq!(error.code(), "artifact_cache_publish_failed");
    }

    #[test]
    fn local_files_publish_atomically_and_default_cache_hits_skip_url_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.bin");
        fs::write(&source, b"artifact-bytes").unwrap();
        let roots = sandbox(temp.path());
        let url = format!("file://{}", source.display());
        let mut resolver = ArtifactResolver::new(&roots);
        let first = resolver
            .resolve(ArtifactResolveRequest {
                artifact_id: "example/artifact",
                type_name: "remote_file",
                url: &url,
                cache_mode: "default",
            })
            .unwrap();
        assert!(!first.cache_hit);
        assert_eq!(fs::read(&first.local_path).unwrap(), b"artifact-bytes");
        assert!(fs::read_dir(&roots.cache_root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("partial")));

        let invalid_url = "not a valid URL";
        let cached_path = roots.cache_root.join(artifact_local_filename(
            "example/cached",
            invalid_url,
            "default",
        ));
        fs::write(&cached_path, b"existing").unwrap();
        let cached = resolver
            .resolve(ArtifactResolveRequest {
                artifact_id: "example/cached",
                type_name: "remote_file",
                url: invalid_url,
                cache_mode: "default",
            })
            .unwrap();
        assert!(cached.cache_hit);
        assert_eq!(fs::read(cached.local_path).unwrap(), b"existing");
    }

    #[test]
    fn cache_none_always_copies_and_uses_unique_collision_path() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.bin");
        fs::write(&source, b"one").unwrap();
        let roots = sandbox(temp.path());
        let url = format!("file://{}", source.display());
        let request = ArtifactResolveRequest {
            artifact_id: "example/artifact",
            type_name: "remote_file",
            url: &url,
            cache_mode: "none",
        };
        let mut resolver = ArtifactResolver::new(&roots);
        let first = resolver.resolve(request).unwrap();
        fs::write(&source, b"two").unwrap();
        let second = resolver.resolve(request).unwrap();
        assert!(!first.cache_hit);
        assert!(!second.cache_hit);
        assert_ne!(first.local_path, second.local_path);
        assert_eq!(fs::read(first.local_path).unwrap(), b"one");
        assert_eq!(fs::read(second.local_path).unwrap(), b"two");
    }

    #[test]
    fn publication_faults_map_to_write_publish_and_cleanup_errors() {
        let temp = tempfile::tempdir().unwrap();
        for (fault, expected) in [
            (PublicationFault::Sync, "artifact_cache_write_failed"),
            (PublicationFault::Publish, "artifact_cache_publish_failed"),
            (PublicationFault::Cleanup, "artifact_partial_cleanup_failed"),
        ] {
            let mut partial = TempFileBuilder::new()
                .prefix(".emuchef-artifact-")
                .suffix(".partial")
                .tempfile_in(temp.path())
                .unwrap();
            partial.write_all(b"bytes").unwrap();
            let error = finish_partial(
                partial,
                &temp.path().join(format!("final-{expected}")),
                true,
                Some(fault),
            )
            .unwrap_err();
            if fault == PublicationFault::Cleanup {
                assert!(matches!(
                    error,
                    ArtifactResolveError::PartialCleanupFailed { .. }
                ));
            } else {
                assert_eq!(error.code(), expected);
            }
        }
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("partial")));
    }

    #[test]
    fn default_http_cache_uses_one_request_then_works_with_server_offline() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let (base_url, requests, server) = spawn_http_server(vec![b"network-bytes"]);
        let url = format!("{base_url}/encoded%20artifact.apk?token=one#fragment");
        let request = ArtifactResolveRequest {
            artifact_id: "example/network",
            type_name: "remote_file",
            url: &url,
            cache_mode: "default",
        };
        let mut resolver = ArtifactResolver::new(&roots);
        let first = resolver.resolve(request).unwrap();
        server.join().unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert_eq!(first.filename, "encoded artifact.apk");
        assert_eq!(fs::read(&first.local_path).unwrap(), b"network-bytes");

        let second = resolver.resolve(request).unwrap();
        assert!(second.cache_hit);
        assert_eq!(second.local_path, first.local_path);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert_eq!(fs::read(second.local_path).unwrap(), b"network-bytes");
    }

    #[test]
    fn raw_query_and_fragment_bytes_remain_part_of_compatible_cache_keys() {
        let query_one =
            artifact_local_filename("example/a", "https://host/file?value=1", "default");
        let query_two =
            artifact_local_filename("example/a", "https://host/file?value=2", "default");
        let fragment_one = artifact_local_filename("example/a", "https://host/file#one", "default");
        let fragment_two = artifact_local_filename("example/a", "https://host/file#two", "default");
        assert_ne!(query_one, query_two);
        assert_ne!(fragment_one, fragment_two);
        assert!(query_one.ends_with("-file"));
        assert!(fragment_one.ends_with("-file"));
    }

    #[test]
    fn malformed_and_unsupported_urls_fail_before_creating_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let mut resolver = ArtifactResolver::new(&roots);
        for (url, expected) in [
            ("http://[::1", "artifact_url_invalid"),
            ("ftp://example.com/file", "artifact_scheme_unsupported"),
        ] {
            let error = resolver
                .resolve(ArtifactResolveRequest {
                    artifact_id: "example/invalid",
                    type_name: "remote_file",
                    url,
                    cache_mode: "none",
                })
                .unwrap_err();
            assert_eq!(error.code(), expected);
        }
        assert!(!roots.runtime_root.join("downloads").exists());
    }

    #[test]
    fn cache_none_http_resolution_makes_a_request_on_every_invocation() {
        let temp = tempfile::tempdir().unwrap();
        let roots = sandbox(temp.path());
        let (base_url, requests, server) = spawn_http_server(vec![b"first", b"second"]);
        let url = format!("{base_url}/artifact.bin");
        let request = ArtifactResolveRequest {
            artifact_id: "example/network",
            type_name: "remote_file",
            url: &url,
            cache_mode: "none",
        };
        let mut resolver = ArtifactResolver::new(&roots);
        let first = resolver.resolve(request).unwrap();
        let second = resolver.resolve(request).unwrap();
        server.join().unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert!(!first.cache_hit);
        assert!(!second.cache_hit);
        assert_ne!(first.local_path, second.local_path);
        assert_eq!(fs::read(first.local_path).unwrap(), b"first");
        assert_eq!(fs::read(second.local_path).unwrap(), b"second");
    }
}

//! Artifact routing, destination selection, and compatibility-preserving names.
//!
//! The resolver is crate-private because execution plans remain the product
//! interface. It preserves the original URL bytes when deriving cache keys.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::{Builder as TempFileBuilder, NamedTempFile};
use url::Url;

use crate::artifact_store::{prepare_metadata, publish_metadata};
use crate::artifact_transport::{
    ArtifactTransport, HttpArtifactTransport, HttpClientConfig, LocalFileTransport,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedArtifact {
    pub local_path: PathBuf,
    pub filename: String,
    pub cache_hit: bool,
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

#[derive(Debug)]
enum AdmittedArtifactSource {
    CacheHit,
    LocalFile(PathBuf),
    Http(Url),
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
        validate_artifact_definition(request)?;
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

        let parsed_url = Url::parse(request.url).map_err(|_| ArtifactResolveError::UrlInvalid)?;
        validate_source_url(&parsed_url)?;
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

    pub(crate) fn resolve(
        &mut self,
        request: ArtifactResolveRequest<'_>,
    ) -> Result<ResolvedArtifact, ArtifactResolveError> {
        let admitted = self.admit(request)?;
        if matches!(admitted.source, AdmittedArtifactSource::CacheHit) {
            return Ok(ResolvedArtifact {
                local_path: admitted.final_path,
                filename: admitted.filename,
                cache_hit: true,
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

        let transfer_result = match admitted.source {
            AdmittedArtifactSource::LocalFile(source_path) => self
                .local_transport
                .download(&source_path, partial.as_file_mut()),
            AdmittedArtifactSource::Http(parsed_url) => self
                .http_transport()?
                .download(&parsed_url, partial.as_file_mut()),
            AdmittedArtifactSource::CacheHit => unreachable!("cache hit returned above"),
        };
        if let Err(error) = transfer_result {
            return Err(cleanup_partial(partial, error, false));
        }

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
        let (local_path, cache_hit) =
            finish_partial(partial, &admitted.final_path, admitted.default_cache, None)?;
        if !cache_hit {
            if let Some(metadata) = prepared_metadata.as_ref() {
                // Metadata is optional support state. Failure intentionally
                // leaves the payload usable and unindexed.
                let _ = publish_metadata(&local_path, metadata);
            }
        }

        Ok(ResolvedArtifact {
            local_path,
            filename: admitted.filename,
            cache_hit,
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
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn sandbox(root: &Path) -> SandboxRoots {
        SandboxRoots {
            runtime_root: root.join("runtime"),
            cache_root: root.join("cache"),
            fake_device_root: root.join("device"),
            read_only_roots: vec![root.to_path_buf()],
        }
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

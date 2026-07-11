//! Transport primitives used by artifact resolution.
//!
//! The executor deliberately does not perform source I/O itself. Transport
//! implementations copy or download bytes while the resolver owns destination
//! selection and sandbox policy.

use std::error::Error;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT_ENCODING, LOCATION};
use reqwest::redirect::Policy;
use url::Url;

use crate::artifact_resolver::ArtifactResolveError;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const TOTAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_REDIRECTS: usize = 5;
const USER_AGENT: &str = "EmuChef/0.1";
const COPY_BUFFER_SIZE: usize = 64 * 1024;

/// Successful transfer metadata used to validate response completeness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DownloadMetadata {
    pub bytes_written: u64,
    pub content_length: Option<u64>,
}

/// Transfer an artifact source to a resolver-selected destination.
pub(crate) trait ArtifactTransport {
    fn download(
        &self,
        source: &Path,
        destination: &mut dyn Write,
    ) -> Result<DownloadMetadata, ArtifactResolveError>;
}

/// Local-file transport preserving the executor's existing copy semantics.
#[derive(Debug, Default)]
pub(crate) struct LocalFileTransport;

impl ArtifactTransport for LocalFileTransport {
    fn download(
        &self,
        source: &Path,
        destination: &mut dyn Write,
    ) -> Result<DownloadMetadata, ArtifactResolveError> {
        let mut source = fs::File::open(source).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ArtifactResolveError::SourceNotFound
            } else {
                ArtifactResolveError::DownloadFailed
            }
        })?;
        let mut bytes_written = 0u64;
        let mut buffer = [0u8; COPY_BUFFER_SIZE];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|_| ArtifactResolveError::DownloadFailed)?;
            if count == 0 {
                break;
            }
            bytes_written = bytes_written
                .checked_add(count as u64)
                .ok_or(ArtifactResolveError::ResponseTooLarge)?;
            destination
                .write_all(&buffer[..count])
                .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
        }
        Ok(DownloadMetadata {
            bytes_written,
            content_length: Some(bytes_written),
        })
    }
}

/// HTTP client configuration. Production callers use fixed policy constants;
/// unit tests can use shorter deadlines without changing product behavior.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpClientConfig {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub use_system_proxy: bool,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            total_timeout: TOTAL_REQUEST_TIMEOUT,
            use_system_proxy: true,
        }
    }
}

/// Serial blocking HTTP transport with strict Rustls verification.
#[derive(Debug)]
pub(crate) struct HttpArtifactTransport {
    client: Client,
    total_timeout: Duration,
}

impl HttpArtifactTransport {
    pub(crate) fn new(config: HttpClientConfig) -> Result<Self, ArtifactResolveError> {
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(config.connect_timeout)
            .user_agent(USER_AGENT);
        if !config.use_system_proxy {
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|_| ArtifactResolveError::DownloadFailed)?;
        Ok(Self {
            client,
            total_timeout: config.total_timeout,
        })
    }

    pub(crate) fn download(
        &self,
        initial_url: &Url,
        destination: &mut dyn Write,
    ) -> Result<DownloadMetadata, ArtifactResolveError> {
        let deadline = Instant::now()
            .checked_add(self.total_timeout)
            .ok_or(ArtifactResolveError::RequestTimeout)?;
        let mut current_url = initial_url.clone();
        let mut redirects = 0usize;
        let mut visited = vec![current_url.as_str().to_string()];

        loop {
            let remaining = remaining(deadline)?;
            let response = self
                .client
                .get(current_url.clone())
                .header(ACCEPT_ENCODING, "identity")
                .timeout(remaining)
                .send()
                .map_err(|error| classify_reqwest_error(&error))?;

            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .ok_or(ArtifactResolveError::DownloadFailed)?
                    .to_str()
                    .map_err(|_| ArtifactResolveError::DownloadFailed)?;
                let next_url = current_url
                    .join(location)
                    .map_err(|_| ArtifactResolveError::DownloadFailed)?;
                validate_redirect(&current_url, &next_url, &mut redirects, &mut visited)?;
                current_url = next_url;
                continue;
            }

            if !response.status().is_success() {
                return Err(ArtifactResolveError::HttpStatus {
                    status: response.status().as_u16(),
                });
            }
            return stream_response(response, destination, deadline);
        }
    }
}

fn validate_redirect(
    current_url: &Url,
    next_url: &Url,
    redirects: &mut usize,
    visited: &mut Vec<String>,
) -> Result<(), ArtifactResolveError> {
    if !matches!(next_url.scheme(), "http" | "https") {
        return Err(ArtifactResolveError::SchemeUnsupported {
            scheme: next_url.scheme().to_string(),
        });
    }
    if current_url.scheme() == "https" && next_url.scheme() == "http" {
        return Err(ArtifactResolveError::RedirectDowngradeRejected);
    }
    *redirects += 1;
    if *redirects > MAX_REDIRECTS || visited.iter().any(|url| url == next_url.as_str()) {
        return Err(ArtifactResolveError::RedirectLimitExceeded {
            redirects: *redirects,
        });
    }
    visited.push(next_url.as_str().to_string());
    Ok(())
}

fn stream_response(
    mut response: Response,
    destination: &mut dyn Write,
    deadline: Instant,
) -> Result<DownloadMetadata, ArtifactResolveError> {
    let content_length = response.content_length();
    let mut bytes_written = 0u64;
    let mut buffer = [0u8; COPY_BUFFER_SIZE];
    loop {
        remaining(deadline)?;
        let count = response.read(&mut buffer).map_err(classify_read_error)?;
        if count == 0 {
            break;
        }
        bytes_written = bytes_written
            .checked_add(count as u64)
            .ok_or(ArtifactResolveError::ResponseTooLarge)?;
        destination
            .write_all(&buffer[..count])
            .map_err(|_| ArtifactResolveError::CacheWriteFailed)?;
    }
    if content_length.is_some_and(|expected| expected != bytes_written) {
        return Err(ArtifactResolveError::ResponseIncomplete);
    }
    Ok(DownloadMetadata {
        bytes_written,
        content_length,
    })
}

fn remaining(deadline: Instant) -> Result<Duration, ArtifactResolveError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(ArtifactResolveError::RequestTimeout)
}

fn classify_reqwest_error(error: &reqwest::Error) -> ArtifactResolveError {
    if error.is_timeout() {
        return if error.is_connect() {
            ArtifactResolveError::ConnectTimeout
        } else {
            ArtifactResolveError::RequestTimeout
        };
    }
    let mut source = error.source();
    while let Some(cause) = source {
        if matches!(
            cause.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(_))
        ) {
            return ArtifactResolveError::TlsVerificationFailed;
        }
        source = cause.source();
    }
    ArtifactResolveError::DownloadFailed
}

fn classify_read_error(error: io::Error) -> ArtifactResolveError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ArtifactResolveError::RequestTimeout,
        io::ErrorKind::UnexpectedEof => ArtifactResolveError::ResponseIncomplete,
        _ => ArtifactResolveError::DownloadFailed,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn production_http_policy_uses_locked_timeouts() {
        let config = HttpClientConfig::default();
        assert_eq!(config.connect_timeout, Duration::from_secs(15));
        assert_eq!(config.total_timeout, Duration::from_secs(5 * 60));
        assert!(config.use_system_proxy);
    }

    #[test]
    fn blocking_transport_downloads_exact_local_http_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = [0u8; 2048];
            let _ = stream
                .read(&mut request)
                .expect("request should be readable");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n\0\x01apk",
                )
                .expect("response should be written");
        });
        let transport = HttpArtifactTransport::new(HttpClientConfig {
            connect_timeout: Duration::from_secs(1),
            total_timeout: Duration::from_secs(2),
            use_system_proxy: false,
        })
        .expect("client should build");
        let mut output = Vec::new();
        let metadata = transport
            .download(
                &Url::parse(&format!("http://{address}/artifact.apk")).unwrap(),
                &mut output,
            )
            .expect("download should succeed");
        server.join().expect("server should stop");
        assert_eq!(output, b"\0\x01apk");
        assert_eq!(metadata.bytes_written, 5);
        assert_eq!(metadata.content_length, Some(5));
    }

    #[test]
    fn redirect_policy_rejects_downgrades_loops_and_sixth_redirect() {
        let https = Url::parse("https://example.com/start").unwrap();
        let http = Url::parse("http://example.com/next").unwrap();
        let mut redirects = 0;
        let mut visited = vec![https.as_str().to_string()];
        assert!(matches!(
            validate_redirect(&https, &http, &mut redirects, &mut visited),
            Err(ArtifactResolveError::RedirectDowngradeRejected)
        ));

        let next = Url::parse("https://example.com/next").unwrap();
        let mut redirects = 5;
        let mut visited = vec![https.as_str().to_string()];
        assert!(matches!(
            validate_redirect(&https, &next, &mut redirects, &mut visited),
            Err(ArtifactResolveError::RedirectLimitExceeded { redirects: 6 })
        ));

        let mut redirects = 0;
        let mut visited = vec![https.as_str().to_string()];
        assert!(matches!(
            validate_redirect(&https, &https, &mut redirects, &mut visited),
            Err(ArtifactResolveError::RedirectLimitExceeded { redirects: 1 })
        ));
    }

    #[test]
    fn local_transport_maps_destination_write_failure_without_exposing_raw_error() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"bytes").unwrap();
        let error = LocalFileTransport
            .download(&source, &mut FailingWriter)
            .unwrap_err();
        assert_eq!(error.code(), "artifact_cache_write_failed");
    }
}

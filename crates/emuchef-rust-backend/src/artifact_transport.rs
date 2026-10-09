//! Transport primitives used by artifact resolution.
//!
//! The executor deliberately does not perform source I/O itself. Transport
//! implementations copy or download bytes while the resolver owns destination
//! selection and sandbox policy.

use std::error::Error;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadMetadata {
    pub bytes_written: u64,
    pub content_length: Option<u64>,
    /// Absolute response URL observed by the HTTP client after manual redirects.
    pub observed_final_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RedirectPolicy {
    Existing,
    PublicHttps,
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
            observed_final_url: None,
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
    client_config: HttpClientConfig,
    #[cfg(test)]
    test_root_der: Option<Vec<u8>>,
    #[cfg(test)]
    test_host_mapping: Option<TestHostMapping>,
}

#[cfg(test)]
#[derive(Debug)]
struct TestHostMapping {
    host: String,
    policy_address: IpAddr,
    connection_address: IpAddr,
}

impl HttpArtifactTransport {
    pub(crate) fn new(config: HttpClientConfig) -> Result<Self, ArtifactResolveError> {
        Self::from_builder(Client::builder(), config)
    }

    fn from_builder(
        builder: reqwest::blocking::ClientBuilder,
        config: HttpClientConfig,
    ) -> Result<Self, ArtifactResolveError> {
        let mut builder = builder
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
            client_config: config,
            #[cfg(test)]
            test_root_der: None,
            #[cfg(test)]
            test_host_mapping: None,
        })
    }

    #[cfg(test)]
    fn with_test_root(
        config: HttpClientConfig,
        certificate_der: &[u8],
    ) -> Result<Self, ArtifactResolveError> {
        let certificate = reqwest::Certificate::from_der(certificate_der)
            .map_err(|_| ArtifactResolveError::TlsVerificationFailed)?;
        let mut transport =
            Self::from_builder(Client::builder().add_root_certificate(certificate), config)?;
        transport.test_root_der = Some(certificate_der.to_vec());
        Ok(transport)
    }

    #[cfg(test)]
    pub(crate) fn with_test_root_and_host_mapping(
        config: HttpClientConfig,
        certificate_der: &[u8],
        host: &str,
        policy_address: IpAddr,
        connection_address: IpAddr,
    ) -> Result<Self, ArtifactResolveError> {
        let certificate = reqwest::Certificate::from_der(certificate_der)
            .map_err(|_| ArtifactResolveError::TlsVerificationFailed)?;
        let builder = Client::builder()
            .resolve(host, SocketAddr::new(connection_address, 0))
            .add_root_certificate(certificate);
        let mut transport = Self::from_builder(builder, config)?;
        transport.test_root_der = Some(certificate_der.to_vec());
        transport.test_host_mapping = Some(TestHostMapping {
            host: host.to_string(),
            policy_address,
            connection_address,
        });
        Ok(transport)
    }

    pub(crate) fn download(
        &self,
        initial_url: &Url,
        destination: &mut dyn Write,
    ) -> Result<DownloadMetadata, ArtifactResolveError> {
        self.download_with_policy(initial_url, destination, RedirectPolicy::Existing)
    }

    pub(crate) fn download_with_policy(
        &self,
        initial_url: &Url,
        destination: &mut dyn Write,
        redirect_policy: RedirectPolicy,
    ) -> Result<DownloadMetadata, ArtifactResolveError> {
        if redirect_policy == RedirectPolicy::PublicHttps
            && crate::authored_models::parse_public_https_url(initial_url.as_str()).is_none()
        {
            return Err(ArtifactResolveError::RedirectPolicyRejected);
        }
        let deadline = Instant::now()
            .checked_add(self.total_timeout)
            .ok_or(ArtifactResolveError::RequestTimeout)?;
        let mut current_url = initial_url.clone();
        let mut redirects = 0usize;
        let mut visited = vec![current_url.as_str().to_string()];

        loop {
            let remaining = remaining(deadline)?;
            let client = match redirect_policy {
                RedirectPolicy::Existing => self.client.clone(),
                RedirectPolicy::PublicHttps => self.public_https_client(&current_url)?,
            };
            let response = client
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
                validate_redirect(
                    &current_url,
                    &next_url,
                    &mut redirects,
                    &mut visited,
                    redirect_policy,
                )?;
                current_url = next_url;
                continue;
            }

            if !response.status().is_success() {
                return Err(ArtifactResolveError::HttpStatus {
                    status: response.status().as_u16(),
                });
            }
            let observed_final_url = response.url().as_str().to_string();
            return stream_response(response, destination, deadline, observed_final_url);
        }
    }

    fn public_https_client(&self, url: &Url) -> Result<Client, ArtifactResolveError> {
        let host = url
            .host_str()
            .ok_or(ArtifactResolveError::RedirectPolicyRejected)?;
        let addresses = self.public_https_addresses(url)?;
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(self.client_config.connect_timeout)
            .user_agent(USER_AGENT);
        if !self.client_config.use_system_proxy {
            builder = builder.no_proxy();
        }
        #[cfg(test)]
        if let Some(certificate_der) = self.test_root_der.as_deref() {
            let certificate = reqwest::Certificate::from_der(certificate_der)
                .map_err(|_| ArtifactResolveError::TlsVerificationFailed)?;
            builder = builder.add_root_certificate(certificate);
        }
        if matches!(url.host(), Some(url::Host::Domain(_))) {
            let addresses = self.public_https_connection_addresses(host, addresses);
            builder = builder.resolve_to_addrs(host, &addresses);
        }
        builder
            .build()
            .map_err(|_| ArtifactResolveError::DownloadFailed)
    }

    fn public_https_addresses(&self, url: &Url) -> Result<Vec<SocketAddr>, ArtifactResolveError> {
        #[cfg(test)]
        if let Some(mapping) = self
            .test_host_mapping
            .as_ref()
            .filter(|mapping| mapping.host == url.host_str().unwrap_or_default())
        {
            if !crate::authored_models::is_public_ip(mapping.policy_address) {
                return Err(ArtifactResolveError::RedirectPolicyRejected);
            }
            return Ok(vec![SocketAddr::new(mapping.policy_address, 0)]);
        }

        match url
            .host()
            .ok_or(ArtifactResolveError::RedirectPolicyRejected)?
        {
            url::Host::Ipv4(address) => {
                let address = IpAddr::V4(address);
                if crate::authored_models::is_public_ip(address) {
                    Ok(vec![SocketAddr::new(address, 0)])
                } else {
                    Err(ArtifactResolveError::RedirectPolicyRejected)
                }
            }
            url::Host::Ipv6(address) => {
                let address = IpAddr::V6(address);
                if crate::authored_models::is_public_ip(address) {
                    Ok(vec![SocketAddr::new(address, 0)])
                } else {
                    Err(ArtifactResolveError::RedirectPolicyRejected)
                }
            }
            url::Host::Domain(host) => {
                let resolved = (host, 0)
                    .to_socket_addrs()
                    .map_err(|_| ArtifactResolveError::DownloadFailed)?
                    .collect::<Vec<_>>();
                if resolved.is_empty()
                    || resolved
                        .iter()
                        .any(|address| !crate::authored_models::is_public_ip(address.ip()))
                {
                    return Err(ArtifactResolveError::RedirectPolicyRejected);
                }
                Ok(resolved
                    .into_iter()
                    .map(|address| SocketAddr::new(address.ip(), 0))
                    .collect())
            }
        }
    }

    fn public_https_connection_addresses(
        &self,
        _host: &str,
        validated_addresses: Vec<SocketAddr>,
    ) -> Vec<SocketAddr> {
        #[cfg(test)]
        if let Some(mapping) = self
            .test_host_mapping
            .as_ref()
            .filter(|mapping| mapping.host == _host)
        {
            return vec![SocketAddr::new(mapping.connection_address, 0)];
        }
        validated_addresses
    }
}

fn validate_redirect(
    current_url: &Url,
    next_url: &Url,
    redirects: &mut usize,
    visited: &mut Vec<String>,
    policy: RedirectPolicy,
) -> Result<(), ArtifactResolveError> {
    if !matches!(next_url.scheme(), "http" | "https") {
        return Err(ArtifactResolveError::SchemeUnsupported {
            scheme: next_url.scheme().to_string(),
        });
    }
    if current_url.scheme() == "https" && next_url.scheme() == "http" {
        return Err(ArtifactResolveError::RedirectDowngradeRejected);
    }
    if policy == RedirectPolicy::PublicHttps
        && crate::authored_models::parse_public_https_url(next_url.as_str()).is_none()
    {
        return Err(ArtifactResolveError::RedirectPolicyRejected);
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
    observed_final_url: String,
) -> Result<DownloadMetadata, ArtifactResolveError> {
    let content_length = response.content_length();
    let mut bytes_written = 0u64;
    let mut buffer = [0u8; COPY_BUFFER_SIZE];
    loop {
        remaining(deadline)?;
        let count = match response.read(&mut buffer) {
            Ok(count) => count,
            Err(error) => {
                let classified = classify_read_error(error);
                if matches!(classified, ArtifactResolveError::RequestTimeout) {
                    return Err(classified);
                }
                if content_length.is_some_and(|expected| bytes_written < expected) {
                    return Err(ArtifactResolveError::ResponseIncomplete);
                }
                return Err(classified);
            }
        };
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
        observed_final_url: Some(observed_final_url),
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
    if has_invalid_certificate(error) {
        return ArtifactResolveError::TlsVerificationFailed;
    }
    ArtifactResolveError::DownloadFailed
}

fn has_invalid_certificate(error: &(dyn Error + 'static)) -> bool {
    if matches!(
        error.downcast_ref::<rustls::Error>(),
        Some(rustls::Error::InvalidCertificate(_))
    ) {
        return true;
    }
    if let Some(io_error) = error.downcast_ref::<io::Error>() {
        if io_error
            .get_ref()
            .is_some_and(|source| has_invalid_certificate(source))
        {
            return true;
        }
    }
    error.source().is_some_and(has_invalid_certificate)
}

fn has_reqwest_timeout(error: &(dyn Error + 'static)) -> bool {
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_timeout)
    {
        return true;
    }
    if let Some(io_error) = error.downcast_ref::<io::Error>() {
        if io_error
            .get_ref()
            .is_some_and(|source| has_reqwest_timeout(source))
        {
            return true;
        }
    }
    error.source().is_some_and(has_reqwest_timeout)
}

fn classify_read_error(error: io::Error) -> ArtifactResolveError {
    if has_reqwest_timeout(&error) {
        return ArtifactResolveError::RequestTimeout;
    }
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ArtifactResolveError::RequestTimeout,
        io::ErrorKind::UnexpectedEof => ArtifactResolveError::ResponseIncomplete,
        _ => ArtifactResolveError::DownloadFailed,
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};

    use super::*;

    struct TestServer {
        base_url: String,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(handler: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let thread_requests = Arc::clone(&requests);
            let thread_stop = Arc::clone(&stop);
            let handler = Arc::new(handler);
            let thread = thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            thread_requests.fetch_add(1, Ordering::Relaxed);
                            let mut request = [0u8; 4096];
                            let count = stream.read(&mut request).unwrap_or(0);
                            let request = String::from_utf8_lossy(&request[..count]);
                            let target = request
                                .lines()
                                .next()
                                .and_then(|line| line.split_whitespace().nth(1))
                                .unwrap_or("/");
                            let response = handler(target);
                            let _ = stream.write_all(&response);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base_url: format!("http://{address}"),
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn url(&self, path: &str) -> Url {
            Url::parse(&format!("{}{path}", self.base_url)).unwrap()
        }

        fn request_count(&self) -> usize {
            self.requests.load(Ordering::Relaxed)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn test_config() -> HttpClientConfig {
        HttpClientConfig {
            connect_timeout: Duration::from_secs(1),
            total_timeout: Duration::from_secs(2),
            use_system_proxy: false,
        }
    }

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
            validate_redirect(
                &https,
                &http,
                &mut redirects,
                &mut visited,
                RedirectPolicy::Existing,
            ),
            Err(ArtifactResolveError::RedirectDowngradeRejected)
        ));

        let next = Url::parse("https://example.com/next").unwrap();
        let mut redirects = 5;
        let mut visited = vec![https.as_str().to_string()];
        assert!(matches!(
            validate_redirect(
                &https,
                &next,
                &mut redirects,
                &mut visited,
                RedirectPolicy::Existing,
            ),
            Err(ArtifactResolveError::RedirectLimitExceeded { redirects: 6 })
        ));

        let mut redirects = 0;
        let mut visited = vec![https.as_str().to_string()];
        assert!(matches!(
            validate_redirect(
                &https,
                &https,
                &mut redirects,
                &mut visited,
                RedirectPolicy::Existing,
            ),
            Err(ArtifactResolveError::RedirectLimitExceeded { redirects: 1 })
        ));
    }

    #[test]
    fn public_https_redirect_policy_allows_signed_urls_and_rejects_credentials_and_local_hosts() {
        let current = Url::parse("https://downloads.example.com/start").unwrap();
        let signed = Url::parse("https://cdn.example.net/download.apk?token=signed-value").unwrap();
        let mut redirects = 0;
        let mut visited = vec![current.as_str().to_string()];
        validate_redirect(
            &current,
            &signed,
            &mut redirects,
            &mut visited,
            RedirectPolicy::PublicHttps,
        )
        .expect("signed CDN query parameters are valid public HTTPS redirects");

        let globally_routable = Url::parse("https://8.8.8.8/download.apk").unwrap();
        validate_redirect(
            &current,
            &globally_routable,
            &mut 0,
            &mut vec![current.as_str().to_string()],
            RedirectPolicy::PublicHttps,
        )
        .expect("globally routable literal IPv4 redirect destinations are allowed");

        for target in [
            "https://user:secret@cdn.example.net/download.apk",
            "https://cdn.example.net/download.apk#private",
            "https://localhost/download.apk",
            "https://127.0.0.1/download.apk",
            "https://10.1.2.3/download.apk",
            "https://0.1.2.3/download.apk",
            "https://169.254.12.34/download.apk",
            "https://[fc00::1]/download.apk",
            "https://[fe80::1]/download.apk",
        ] {
            let target = Url::parse(target).unwrap();
            let mut redirects = 0;
            let mut visited = vec![current.as_str().to_string()];
            assert!(matches!(
                validate_redirect(
                    &current,
                    &target,
                    &mut redirects,
                    &mut visited,
                    RedirectPolicy::PublicHttps,
                ),
                Err(ArtifactResolveError::RedirectPolicyRejected)
            ));
        }

        let downgrade = Url::parse("http://cdn.example.net/download.apk").unwrap();
        assert!(matches!(
            validate_redirect(
                &current,
                &downgrade,
                &mut 0,
                &mut vec![current.as_str().to_string()],
                RedirectPolicy::PublicHttps,
            ),
            Err(ArtifactResolveError::RedirectDowngradeRejected)
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

    #[test]
    fn local_server_covers_binary_empty_large_chunked_query_and_encoded_paths() {
        let large = vec![0x5a; 512 * 1024];
        let expected_large = large.clone();
        let server = TestServer::spawn(move |target| {
            match target {
            "/binary" => response("200 OK", "", b"\0\x01\xffapk"),
            "/empty" => response("200 OK", "", b""),
            "/large" => response("200 OK", "", &large),
            "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n".to_vec(),
            "/query?version=one" => response("200 OK", "", b"one"),
            "/query?version=two" => response("200 OK", "", b"two"),
            "/encoded%20name.apk" => response("200 OK", "", b"encoded"),
            _ => response("404 Not Found", "", b"missing"),
        }
        });
        let transport = HttpArtifactTransport::new(test_config()).unwrap();
        for (path, expected) in [
            ("/binary", b"\0\x01\xffapk".as_slice()),
            ("/empty", b"".as_slice()),
            ("/chunked", b"abcdef".as_slice()),
            ("/query?version=one", b"one".as_slice()),
            ("/query?version=two", b"two".as_slice()),
            ("/encoded%20name.apk", b"encoded".as_slice()),
        ] {
            let mut output = Vec::new();
            transport.download(&server.url(path), &mut output).unwrap();
            assert_eq!(output, expected);
        }
        let mut output = Vec::new();
        transport
            .download(&server.url("/large"), &mut output)
            .unwrap();
        assert_eq!(output, expected_large);
        assert_eq!(server.request_count(), 7);
    }

    #[test]
    fn local_server_covers_redirects_statuses_and_truncation() {
        let server = TestServer::spawn(|target| {
            if let Some(index) = target.strip_prefix("/chain/") {
                let index = index.parse::<usize>().unwrap();
                if index == 5 {
                    return response("200 OK", "", b"redirected");
                }
                return response(
                    "302 Found",
                    &format!("Location: /chain/{}\r\n", index + 1),
                    b"ignored",
                );
            }
            match target {
                "/redirect301" => response("301 Moved Permanently", "Location: /ok\r\n", b""),
                "/redirect-secret" => response(
                    "302 Found",
                    "Location: /ok?token=private-signed-value\r\n",
                    b"",
                ),
                "/ok?token=private-signed-value" => response("200 OK", "", b"ok"),
                "/ok" => response("200 OK", "", b"ok"),
                "/loop" => response("302 Found", "Location: /loop\r\n", b""),
                "/malformed" => b"HTTP/1.1 302 Found\r\nLocation: http://[::1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                "/unsupported" => response("302 Found", "Location: ftp://example.com/file\r\n", b""),
                "/404" => response("404 Not Found", "", b"secret response body"),
                "/500" => response("500 Internal Server Error", "", b"secret response body"),
                "/truncated" => b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort".to_vec(),
                _ => response("404 Not Found", "", b""),
            }
        });
        let transport = HttpArtifactTransport::new(test_config()).unwrap();
        let mut output = Vec::new();
        let first_redirect = server.url("/redirect301");
        let metadata = transport.download(&first_redirect, &mut output).unwrap();
        assert_eq!(output, b"ok");
        assert_eq!(
            metadata.observed_final_url.as_deref(),
            Some(server.url("/ok").as_str())
        );
        output.clear();
        let metadata = transport
            .download(&server.url("/redirect-secret"), &mut output)
            .unwrap();
        let observed_final_url = metadata
            .observed_final_url
            .as_deref()
            .expect("HTTP transport should report the response URL it observed");
        assert!(observed_final_url.ends_with("/ok?token=private-signed-value"));
        let redacted = crate::artifact_resolver::redacted_url(observed_final_url)
            .expect("the final URL should have a safe redacted projection");
        assert!(redacted.ends_with("/ok"));
        assert!(!redacted.contains("private-signed-value"));
        assert!(!redacted.contains("token"));
        output.clear();
        transport
            .download(&server.url("/chain/0"), &mut output)
            .unwrap();
        assert_eq!(output, b"redirected");

        for (path, code) in [
            ("/loop", "artifact_redirect_limit_exceeded"),
            ("/malformed", "artifact_download_failed"),
            ("/unsupported", "artifact_scheme_unsupported"),
            ("/404", "artifact_http_status"),
            ("/500", "artifact_http_status"),
            ("/truncated", "artifact_response_incomplete"),
        ] {
            let error = transport
                .download(&server.url(path), &mut Vec::new())
                .unwrap_err();
            assert_eq!(error.code(), code, "path: {path}");
        }
    }

    #[test]
    fn delayed_response_uses_the_shared_request_deadline() {
        let server = TestServer::spawn(|_| {
            thread::sleep(Duration::from_millis(250));
            response("200 OK", "", b"late")
        });
        let transport = HttpArtifactTransport::new(HttpClientConfig {
            total_timeout: Duration::from_millis(50),
            ..test_config()
        })
        .unwrap();
        let error = transport
            .download(&server.url("/delayed"), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "artifact_request_timeout");
    }

    #[test]
    fn delayed_body_uses_the_shared_request_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(250));
            let _ = stream.write_all(b"late");
        });
        let transport = HttpArtifactTransport::new(HttpClientConfig {
            total_timeout: Duration::from_millis(50),
            ..test_config()
        })
        .unwrap();
        let error = transport
            .download(
                &Url::parse(&format!("http://{address}/delayed-body")).unwrap(),
                &mut Vec::new(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "artifact_request_timeout");
        server.join().unwrap();
    }

    fn spawn_tls_server(response_bytes: Vec<u8>) -> (String, Vec<u8>, JoinHandle<()>) {
        use std::net::{IpAddr, Ipv4Addr};

        use rcgen::{CertificateParams, KeyPair, SanType};
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
        use rustls::{ServerConfig, ServerConnection, StreamOwned};

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut params = CertificateParams::default();
        params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
        let key = KeyPair::generate().unwrap();
        let certificate = params.self_signed(&key).unwrap();
        let certificate_der = certificate.der().to_vec();
        let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.der().clone()], private_key)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(Arc::new(config)).unwrap();
            let mut stream = StreamOwned::new(connection, stream);
            let mut request = [0u8; 4096];
            if stream.read(&mut request).is_ok() {
                let _ = stream.write_all(&response_bytes);
            }
        });
        (
            format!("https://127.0.0.1:{}/artifact", address.port()),
            certificate_der,
            thread,
        )
    }

    #[test]
    fn local_tls_is_strict_supports_test_root_and_rejects_downgrade() {
        let (url, _certificate, thread) = spawn_tls_server(response("200 OK", "", b"tls"));
        let transport = HttpArtifactTransport::new(test_config()).unwrap();
        let error = transport
            .download(&Url::parse(&url).unwrap(), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "artifact_tls_verification_failed");
        thread.join().unwrap();

        let (url, certificate, thread) = spawn_tls_server(response("200 OK", "", b"tls"));
        let transport = HttpArtifactTransport::with_test_root(test_config(), &certificate).unwrap();
        let mut output = Vec::new();
        transport
            .download(&Url::parse(&url).unwrap(), &mut output)
            .unwrap();
        assert_eq!(output, b"tls");
        thread.join().unwrap();

        let (url, certificate, thread) = spawn_tls_server(response(
            "302 Found",
            "Location: http://127.0.0.1:9/downgrade\r\n",
            b"",
        ));
        let transport = HttpArtifactTransport::with_test_root(test_config(), &certificate).unwrap();
        let error = transport
            .download(&Url::parse(&url).unwrap(), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "artifact_redirect_downgrade_rejected");
        thread.join().unwrap();

        let (url, certificate, thread) = spawn_tls_server(response("200 OK", "", b"tls"));
        let wrong_host_url = url.replace("127.0.0.1", "localhost");
        let transport = HttpArtifactTransport::with_test_root(test_config(), &certificate).unwrap();
        let error = transport
            .download(&Url::parse(&wrong_host_url).unwrap(), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "artifact_tls_verification_failed");
        thread.join().unwrap();
    }
}

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tempfile::TempDir;

const TEST_BINARY_ENV: &str = "EMUCHEF_TEST_BINARY";

struct LocalServer {
    base_url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalServer {
    fn spawn(status: &'static str, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut request = [0u8; 4096];
                        let _ = stream.read(&mut request).unwrap();
                        thread_requests.fetch_add(1, Ordering::Relaxed);
                        let headers = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream.write_all(headers.as_bytes()).unwrap();
                        stream.write_all(body).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
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

    fn url(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.base_url)
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct AuthoredFixture {
    _temp: TempDir,
    root: PathBuf,
    recipe: PathBuf,
    plan: PathBuf,
}

impl AuthoredFixture {
    fn new(artifact_url: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("authored");
        fs::create_dir_all(root.join("recipes")).unwrap();
        fs::create_dir_all(root.join("device_profiles")).unwrap();
        fs::create_dir_all(root.join("device_plans")).unwrap();
        let recipe = root.join("recipes/network.test.yaml");
        fs::write(
            &recipe,
            format!(
                r#"schema_version: 1
kind: recipe
id: network.test
name: Network Test
description: Process-level HTTP artifact fixture.
recipe_dependencies: []
provides:
  features: []
inputs: {{}}
artifacts:
  payload:
    type: remote_file
    url: {artifact_url}
    cache: default
artifact_groups: {{}}
steps:
  - id: resolve
    type: resolve_artifacts
    name: Resolve
    user_toggleable: false
    dependencies: []
    constraints: {{capabilities: [], conflicts_with: []}}
    skip_if: []
    params:
      artifacts: [payload]
    verify: []
  - id: downstream
    type: wait
    name: Downstream
    user_toggleable: false
    dependencies: [resolve]
    constraints: {{capabilities: [], conflicts_with: []}}
    skip_if: []
    params:
      duration_ms: 1
    verify: []
  - id: unrelated
    type: wait
    name: Unrelated
    user_toggleable: false
    dependencies: []
    constraints: {{capabilities: [], conflicts_with: []}}
    skip_if: []
    params:
      duration_ms: 1
    verify: []
"#
            ),
        )
        .unwrap();
        fs::write(
            root.join("device_profiles/test.profile.yaml"),
            r#"schema_version: 1
kind: device_profile
id: test.profile
name: Test Profile
match:
  manufacturer_contains: [Example]
  brand_contains: []
  model_patterns: ['.*']
  android_version: {min: 1, max: 99}
capability_defaults:
  adb_available: true
  apk_install: true
  shared_storage_write: true
  app_launch: true
  shell_command: true
  package_remove_for_user: false
  root_shell: false
  app_data_write: false
device_tags: [test]
"#,
        )
        .unwrap();
        fs::write(
            root.join("device_plans/test.plan.yaml"),
            r#"schema_version: 1
kind: device_plan
id: test.plan
name: Test Plan
device_profile_ref: test.profile
recipes:
  - recipe_ref: network.test
    selected_by_default: true
defaults: {}
overrides: {}
"#,
        )
        .unwrap();
        let plan = temp.path().join("execution-plan.yaml");
        Self {
            _temp: temp,
            root,
            recipe,
            plan,
        }
    }

    fn validate_and_plan(&self) {
        let validate = run(
            self._temp.path(),
            &[
                "validate",
                "--authored-root",
                self.root.to_str().unwrap(),
                self.recipe.to_str().unwrap(),
            ],
        );
        assert_success(&validate, "validate");
        let plan = run(
            self._temp.path(),
            &[
                "plan",
                "--authored-root",
                self.root.to_str().unwrap(),
                "--device-plan",
                "test.plan",
                "--manufacturer",
                "Example",
                "--model",
                "Example Model",
                "--android-version",
                "13",
                "--output",
                self.plan.to_str().unwrap(),
            ],
        );
        assert_success(&plan, "plan");
        let plan_yaml = fs::read_to_string(&self.plan).unwrap();
        assert!(plan_yaml.contains("kind: execution_plan"));
        assert!(plan_yaml.contains("network.test/payload"));
    }

    fn apply(&self) -> Output {
        run(
            self._temp.path(),
            &[
                "apply",
                "--plan-file",
                self.plan.to_str().unwrap(),
                "--dry-run",
            ],
        )
    }
}

fn run(current_dir: &Path, args: &[&str]) -> Output {
    Command::new(test_binary())
        .current_dir(current_dir)
        .args(args)
        .output()
        .expect("emuchef process should run")
}

fn test_binary() -> PathBuf {
    let binary = std::env::var_os(TEST_BINARY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_emuchef")));
    assert!(
        binary.is_absolute(),
        "{TEST_BINARY_ENV} must be an absolute path"
    );
    assert!(
        binary.is_file(),
        "test binary must be a regular file: {}",
        binary.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            binary.metadata().unwrap().permissions().mode() & 0o111,
            0,
            "test binary must be executable: {}",
            binary.display()
        );
    }
    binary
}

fn assert_success(output: &Output, command: &str) {
    assert!(
        output.status.success(),
        "{command} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_no_partials(root: &Path) {
    if !root.exists() {
        return;
    }
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_no_partials(&path);
        } else {
            assert!(!path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("partial"));
        }
    }
}

fn file_snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, current: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        if !current.exists() {
            return;
        }
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }

    let mut files = Vec::new();
    visit(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn spawn_self_signed_tls_server() -> (String, JoinHandle<()>) {
    use std::net::{IpAddr, Ipv4Addr};

    use rcgen::{CertificateParams, KeyPair, SanType};
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
    let key = KeyPair::generate().unwrap();
    let certificate = params.self_signed(&key).unwrap();
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
            let response =
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret";
            let _ = stream.write_all(response);
        }
    });
    (
        format!(
            "https://127.0.0.1:{}/artifact.apk?token=tls-secret",
            address.port()
        ),
        thread,
    )
}

#[test]
fn product_cli_downloads_then_uses_live_and_offline_warm_cache() {
    let mut server = LocalServer::spawn("200 OK", b"\0\x01network-apk");
    let fixture = AuthoredFixture::new(&server.url("/artifact.apk?token=secret#fragment"));
    fixture.validate_and_plan();

    let cold = fixture.apply();
    assert_success(&cold, "cold apply");
    assert_eq!(server.request_count(), 1);
    let cold_cache = file_snapshot(&fixture._temp.path().join(".emuchef_cache"));
    assert!(!cold_cache.is_empty());
    let warm = fixture.apply();
    assert_success(&warm, "warm apply");
    assert_eq!(server.request_count(), 1);
    assert_eq!(
        file_snapshot(&fixture._temp.path().join(".emuchef_cache")),
        cold_cache
    );
    server.stop();
    let offline = fixture.apply();
    assert_success(&offline, "offline warm-cache apply");
    assert_eq!(server.request_count(), 1);
    assert_eq!(
        file_snapshot(&fixture._temp.path().join(".emuchef_cache")),
        cold_cache
    );
    assert_no_partials(fixture._temp.path());
}

#[test]
fn product_cli_redacts_network_failure_and_preserves_executor_semantics() {
    let server = LocalServer::spawn("404 Not Found", b"secret response body");
    let fixture = AuthoredFixture::new(&server.url("/artifact.apk?token=super-secret"));
    fixture.validate_and_plan();

    let failed = fixture.apply();
    assert_eq!(failed.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&failed.stdout);
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stdout.contains("Dry run: failed"), "stdout: {stdout}");
    assert!(stdout.contains("blocked"), "stdout: {stdout}");
    assert!(stdout.contains("Unrelated"), "stdout: {stdout}");
    assert!(stderr.contains("artifact_http_status"));
    assert!(stderr.contains("HTTP 404"));
    assert!(!stderr.contains("super-secret"));
    assert!(!stderr.contains("secret response body"));
    assert_no_partials(fixture._temp.path());
    assert!(file_snapshot(&fixture._temp.path().join(".emuchef_cache")).is_empty());
}

#[test]
fn product_cli_rejects_self_signed_tls_without_publication_or_secret_leakage() {
    let (url, server) = spawn_self_signed_tls_server();
    let fixture = AuthoredFixture::new(&url);
    fixture.validate_and_plan();

    let failed = fixture.apply();
    assert_eq!(failed.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        stderr.contains("artifact_tls_verification_failed"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("tls-secret"));
    assert!(!stderr.contains("secret response"));
    assert!(file_snapshot(&fixture._temp.path().join(".emuchef_cache")).is_empty());
    assert_no_partials(fixture._temp.path());
    server.join().unwrap();
}

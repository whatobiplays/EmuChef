//! Lifecycle and compatibility negotiation for the canonical Rust sidecar.

use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_io::{block_on, Timer};
use async_process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use futures_lite::future::{self, poll_fn};
use futures_lite::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde::Serialize;
use serde_json::{json, Value};

const REQUIRED_CAPABILITIES: &[&str] = &[
    "describeCatalog",
    "listAdbDevices",
    "probeDevice",
    "qualifyDevice",
    "checkRoot",
    "matchDevice",
    "describeConfiguration",
    "planConfiguration",
    "startExecution",
    "getExecution",
    "getExecutionEvents",
    "cancelExecution",
    "launchExecutionApp",
    "openUserConfiguration",
    "createUserConfiguration",
    "getUserConfigurationDocument",
    "saveUserConfiguration",
    "saveUserConfigurationAs",
    "setUserConfigurationBinding",
    "removeUserConfigurationBinding",
    "setUserConfigurationSelectedRecipes",
    "setUserConfigurationDevicePlan",
    "validateUserConfiguration",
    "closeUserConfiguration",
];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Records whether the last owned sidecar process reached a confirmed terminal
/// state. `NotRequired` means no child existed for that lifecycle attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidecarCleanup {
    NotRequired,
    Confirmed,
    Uncertain,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RuntimeStatusDto {
    Starting,
    Ready {
        #[serde(rename = "protocolVersion")]
        protocol_version: u64,
        #[serde(rename = "catalogVersion")]
        catalog_version: Option<String>,
    },
    Unsupported {
        error: RuntimeErrorDto,
    },
    Failed {
        error: RuntimeErrorDto,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeErrorDto {
    code: &'static str,
    message: String,
    actions: Vec<&'static str>,
}

pub struct SidecarState {
    inner: Mutex<SidecarClient>,
}

impl SidecarState {
    /// Construct the end-user runtime with its trusted app-owned cache root.
    ///
    /// This explicit path is supplied only by the Tauri app. Backend defaults,
    /// CLI behavior, and other sidecar embedders remain unchanged.
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            inner: Mutex::new(SidecarClient::new(cache_root)),
        }
    }

    pub fn initialize(&self) {
        let mut client = self.inner.lock().expect("sidecar mutex poisoned");
        client.generation = client.generation.saturating_add(1).max(1);
        client.initialize();
    }

    pub fn generation(&self) -> u64 {
        self.try_generation().expect("sidecar mutex poisoned")
    }

    /// Return the runtime generation without panicking when process
    /// infrastructure is unavailable.
    pub fn try_generation(&self) -> Result<u64, ()> {
        self.inner
            .lock()
            .map(|client| client.generation)
            .map_err(|_| ())
    }

    pub fn status(&self) -> RuntimeStatusDto {
        self.inner
            .lock()
            .expect("sidecar mutex poisoned")
            .status
            .clone()
    }

    /// Report whether the current process generation lost its protocol
    /// session after startup. Callers use this to discard all authority that
    /// depended on the terminated in-memory runtime.
    pub(crate) fn runtime_session_was_lost(&self) -> bool {
        self.inner
            .lock()
            .map(|client| {
                client.session_lost
                    || matches!(
                        &client.status,
                        RuntimeStatusDto::Failed { error }
                            if error.code == "runtime_session_lost"
                    )
            })
            .unwrap_or(false)
    }

    /// Return fixed, path-free runtime compatibility data for support export.
    pub fn diagnostics(&self) -> Value {
        let status = match self.status() {
            RuntimeStatusDto::Starting => json!({ "status": "starting" }),
            RuntimeStatusDto::Ready {
                protocol_version,
                catalog_version,
            } => json!({
                "status": "ready",
                "protocolVersion": protocol_version,
                "catalogVersion": catalog_version,
            }),
            RuntimeStatusDto::Unsupported { error } => json!({
                "status": "unsupported",
                "error": { "code": error.code, "actions": error.actions },
            }),
            RuntimeStatusDto::Failed { error } => json!({
                "status": "failed",
                "error": { "code": error.code, "actions": error.actions },
            }),
        };
        json!({
            "status": status,
            "requiredCapabilities": REQUIRED_CAPABILITIES,
        })
    }

    pub fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
        self.inner
            .lock()
            .map_err(|_| "Rust runtime state is unavailable.".to_string())?
            .request(request_type, payload)
    }
}

struct SidecarClient {
    process: Option<SidecarProcess>,
    status: RuntimeStatusDto,
    session_lost: bool,
    last_cleanup: SidecarCleanup,
    next_request_id: u64,
    cache_root: PathBuf,
    generation: u64,
}

impl SidecarClient {
    fn new(cache_root: PathBuf) -> Self {
        Self {
            process: None,
            status: RuntimeStatusDto::Starting,
            session_lost: false,
            last_cleanup: SidecarCleanup::NotRequired,
            next_request_id: 1,
            cache_root,
            generation: 0,
        }
    }

    fn initialize(&mut self) {
        self.stop();
        // A new generation starts with no cleanup obligation. Any process
        // created below will replace this evidence if startup later fails.
        self.last_cleanup = SidecarCleanup::NotRequired;
        self.session_lost = false;
        self.status = RuntimeStatusDto::Starting;
        let result = self.start_and_negotiate();
        self.apply_start_result(result);
    }

    fn apply_start_result(&mut self, result: Result<u64, StartFailure>) {
        match result {
            Ok(protocol_version) => {
                self.status = RuntimeStatusDto::Ready {
                    protocol_version,
                    catalog_version: Some("phase1-bundled-1".to_string()),
                };
            }
            Err(StartFailure::Unsupported(message)) => {
                self.status = RuntimeStatusDto::Unsupported {
                    error: RuntimeErrorDto {
                        code: "runtime_unsupported",
                        message,
                        actions: vec!["retry"],
                    },
                };
                self.stop();
            }
            Err(StartFailure::SessionLost(message)) => {
                self.session_lost = true;
                self.status = RuntimeStatusDto::Failed {
                    error: RuntimeErrorDto {
                        code: "runtime_start_failed",
                        message,
                        actions: vec!["retry"],
                    },
                };
                self.stop();
            }
            Err(StartFailure::Failed { message, cleanup }) => {
                self.last_cleanup = cleanup;
                self.status = RuntimeStatusDto::Failed {
                    error: RuntimeErrorDto {
                        code: "runtime_start_failed",
                        message,
                        actions: vec!["retry"],
                    },
                };
                self.stop();
            }
        }
    }

    fn start_and_negotiate(&mut self) -> Result<u64, StartFailure> {
        let process = start_process(&self.cache_root, self.generation).map_err(|failure| {
            StartFailure::Failed {
                message: failure.message,
                cleanup: failure.cleanup,
            }
        })?;
        self.process = Some(process);
        self.negotiate_existing_process()
    }

    fn negotiate_existing_process(&mut self) -> Result<u64, StartFailure> {
        let hello = self
            .raw_request("hello", json!({}))
            .map_err(StartFailure::SessionLost)?;
        let result = startup_result(&hello)?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                StartFailure::Unsupported("Rust runtime omitted its protocol version.".to_string())
            })?;
        let extension_ok = result
            .get("protocolExtensions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|extension| {
                extension.get("id").and_then(Value::as_str) == Some("phase0_end_user_runtime")
                    && extension
                        .get("version")
                        .and_then(Value::as_u64)
                        .is_some_and(|version| version >= 1)
            });
        if !extension_ok {
            return Err(StartFailure::Unsupported(
                "Rust runtime does not provide the Phase 0 end-user extension.".to_string(),
            ));
        }
        let negotiation = self
            .raw_request("negotiateCapabilities", capability_negotiation_payload())
            .map_err(StartFailure::SessionLost)?;
        let result = startup_result(&negotiation)?;
        if result.get("compatible").and_then(Value::as_bool) != Some(true) {
            return Err(StartFailure::Unsupported(
                "Rust runtime is missing one or more required end-user product operations."
                    .to_string(),
            ));
        }
        Ok(protocol_version)
    }

    #[cfg(test)]
    fn initialize_with_process(&mut self, process: SidecarProcess) {
        self.stop();
        self.last_cleanup = SidecarCleanup::NotRequired;
        self.session_lost = false;
        self.status = RuntimeStatusDto::Starting;
        self.process = Some(process);
        let result = self.negotiate_existing_process();
        self.apply_start_result(result);
    }

    fn request(&mut self, request_type: &str, payload: Value) -> Result<Value, String> {
        if matches!(
            &self.status,
            RuntimeStatusDto::Failed { error } if error.code == "runtime_session_lost"
        ) {
            return Err(runtime_session_lost_error());
        }
        if !matches!(self.status, RuntimeStatusDto::Ready { .. }) {
            return Err("Rust runtime is not ready.".to_string());
        }
        let envelope = match self.raw_request(request_type, payload) {
            Ok(envelope) => envelope,
            Err(_) => return Err(self.fail_runtime_session()),
        };
        match envelope.get("ok").and_then(Value::as_bool) {
            Some(true) => envelope
                .get("result")
                .cloned()
                .ok_or_else(|| self.fail_runtime_session()),
            Some(false) => {
                let Some(error) = envelope.get("error").filter(|error| {
                    error.get("code").and_then(Value::as_str).is_some()
                        && error.get("message").and_then(Value::as_str).is_some()
                }) else {
                    return Err(self.fail_runtime_session());
                };
                match serde_json::to_string(error) {
                    Ok(error) => Err(error),
                    Err(_) => Err(self.fail_runtime_session()),
                }
            }
            None => Err(self.fail_runtime_session()),
        }
    }

    fn fail_runtime_session(&mut self) -> String {
        // A broken or structurally invalid response proves this in-memory
        // protocol session cannot answer future requests. Stop the child and
        // retain a stable failure code for every later request in this process
        // generation so all dependent native authority can be invalidated.
        self.status = RuntimeStatusDto::Failed {
            error: RuntimeErrorDto {
                code: "runtime_session_lost",
                message: "The local app service stopped responding.".to_string(),
                actions: vec!["retry"],
            },
        };
        self.session_lost = true;
        self.stop();
        runtime_session_lost_error()
    }

    fn raw_request(&mut self, request_type: &str, payload: Value) -> Result<Value, String> {
        let request_id = format!("app-{}", self.next_request_id);
        self.next_request_id += 1;
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| "Rust runtime process is not running.".to_string())?;
        if process.generation() != self.generation {
            return Err("Rust runtime response belonged to a stale generation.".to_string());
        }
        let request = json!({ "id": request_id, "type": request_type, "payload": payload });
        let mut encoded = serde_json::to_vec(&request)
            .map_err(|_| "Rust runtime request could not be encoded.".to_string())?;
        encoded.push(b'\n');
        let frame = process.request_frame(&encoded, REQUEST_TIMEOUT)?;
        let response: Value = serde_json::from_slice(trim_frame(&frame))
            .map_err(|_| "Rust runtime response was not valid JSON.".to_string())?;
        if response.get("id").and_then(Value::as_str) != Some(&request_id) {
            return Err("Rust runtime response id did not match its request.".to_string());
        }
        Ok(response)
    }

    fn stop(&mut self) -> SidecarCleanup {
        if let Some(process) = self.process.take() {
            self.last_cleanup = process.stop();
        }
        self.last_cleanup
    }
}

fn runtime_session_lost_error() -> String {
    json!({
        "code": "runtime_session_lost",
        "message": "The local app service session was lost.",
    })
    .to_string()
}

/// Build the backend-defined capability negotiation payload used at startup.
fn capability_negotiation_payload() -> Value {
    json!({
        "requiredCapabilities": REQUIRED_CAPABILITIES,
        "optionalCapabilities": [],
    })
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        let _cleanup = self.stop();
    }
}

enum StartFailure {
    Unsupported(String),
    SessionLost(String),
    Failed {
        message: String,
        cleanup: SidecarCleanup,
    },
}

struct StartProcessFailure {
    message: String,
    cleanup: SidecarCleanup,
}

enum SidecarProcess {
    Child {
        generation: u64,
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    },
    #[cfg(test)]
    Scripted {
        generation: u64,
        stdin: Vec<u8>,
        stdout: std::io::Cursor<Vec<u8>>,
        observer: std::sync::Arc<ScriptedProcessObserver>,
    },
}

#[cfg(test)]
struct ScriptedProcessObserver {
    stopped: std::sync::atomic::AtomicBool,
    transport_accesses: std::sync::atomic::AtomicUsize,
    cleanup: SidecarCleanup,
}

#[cfg(test)]
impl Default for ScriptedProcessObserver {
    fn default() -> Self {
        Self {
            stopped: std::sync::atomic::AtomicBool::new(false),
            transport_accesses: std::sync::atomic::AtomicUsize::new(0),
            cleanup: SidecarCleanup::Confirmed,
        }
    }
}

impl SidecarProcess {
    fn generation(&self) -> u64 {
        match self {
            Self::Child { generation, .. } => *generation,
            #[cfg(test)]
            Self::Scripted { generation, .. } => *generation,
        }
    }

    /// Send one request and read one bounded JSONL frame in the caller-owned
    /// future tree. No reader task or detached executor work outlives this
    /// method.
    fn request_frame(&mut self, request: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
        match self {
            Self::Child { stdin, stdout, .. } => {
                let result = block_on(async {
                    let mut timer = Box::pin(Timer::after(timeout));
                    let mut exchange =
                        Box::pin(async {
                            stdin.write_all(request).await.map_err(|_| {
                                "Rust runtime request could not be sent.".to_string()
                            })?;
                            stdin.flush().await.map_err(|_| {
                                "Rust runtime request could not be sent.".to_string()
                            })?;
                            read_frame(stdout).await
                        });
                    poll_fn(|context| {
                        if let std::task::Poll::Ready(result) = exchange.as_mut().poll(context) {
                            return std::task::Poll::Ready(result);
                        }
                        if let std::task::Poll::Ready(_) = timer.as_mut().poll(context) {
                            return std::task::Poll::Ready(Err(
                                "Rust runtime response timed out.".to_string()
                            ));
                        }
                        std::task::Poll::Pending
                    })
                    .await
                });
                result
            }
            #[cfg(test)]
            Self::Scripted {
                stdin,
                stdout,
                observer,
                ..
            } => {
                observer
                    .transport_accesses
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                stdin.extend_from_slice(request);
                let mut frame = Vec::new();
                std::io::Read::read_to_end(stdout, &mut frame)
                    .map_err(|_| "Rust runtime response could not be read.".to_string())?;
                if frame.len() > MAX_FRAME_BYTES {
                    return Err("Rust runtime response exceeded its frame limit.".to_string());
                }
                Ok(frame)
            }
        }
    }

    fn stop(self) -> SidecarCleanup {
        match self {
            Self::Child {
                mut child,
                stdin,
                stdout,
                ..
            } => {
                // Dropping the pipes first closes stdin and releases all local
                // readers before the bounded child reap begins.
                drop(stdin);
                drop(stdout);
                cleanup_child(&mut child)
            }
            #[cfg(test)]
            Self::Scripted { observer, .. } => {
                observer
                    .stopped
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                observer.cleanup
            }
        }
    }
}

/// Read exactly one newline-terminated frame without ever growing beyond the
/// protocol limit. Buffered bytes after the newline remain available for the
/// next request.
async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut frame = Vec::with_capacity(1024);
    loop {
        let buffer = reader
            .fill_buf()
            .await
            .map_err(|_| "Rust runtime response could not be read.".to_string())?;
        if buffer.is_empty() {
            return Err(if frame.is_empty() {
                "Rust runtime response reached EOF.".to_string()
            } else {
                "Rust runtime response ended with a partial frame.".to_string()
            });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(buffer.len(), |position| position + 1);
        if frame.len().saturating_add(consumed) > MAX_FRAME_BYTES {
            return Err("Rust runtime response exceeded its frame limit.".to_string());
        }
        frame.extend_from_slice(&buffer[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(frame);
        }
    }
}

fn trim_frame(frame: &[u8]) -> &[u8] {
    frame.strip_suffix(b"\n").unwrap_or(frame)
}

trait CleanupProcess {
    fn try_exit(&mut self) -> bool;
    fn kill(&mut self) -> bool;
    fn reap_with_deadline(&mut self) -> bool;
}

impl CleanupProcess for Child {
    fn try_exit(&mut self) -> bool {
        self.try_status().ok().flatten().is_some()
    }

    fn kill(&mut self) -> bool {
        Child::kill(self).is_ok()
    }

    fn reap_with_deadline(&mut self) -> bool {
        block_on(future::race(
            async { Child::status(self).await.is_ok() },
            async {
                Timer::after(CLEANUP_TIMEOUT).await;
                false
            },
        ))
    }
}

/// Apply the bounded sidecar cleanup contract to any privately owned process.
///
/// A failed kill does not decide the result by itself: a later status check or
/// bounded reap can still establish that the exact child exited. Dropping pipe
/// handles is intentionally not treated as process termination evidence.
fn cleanup_owned_process(process: &mut impl CleanupProcess) -> SidecarCleanup {
    if process.try_exit() {
        return SidecarCleanup::Confirmed;
    }

    let kill_succeeded = process.kill();
    // A status check proves the exact child exited regardless of whether the
    // termination request itself succeeded.
    if process.try_exit() {
        return SidecarCleanup::Confirmed;
    }

    let reaped = process.reap_with_deadline();
    match (kill_succeeded, reaped) {
        (true, true) | (false, true) => SidecarCleanup::Confirmed,
        // A successful kill without exit/reap evidence is still uncertain.
        (true, false) | (false, false) => SidecarCleanup::Uncertain,
    }
}

fn cleanup_child(child: &mut Child) -> SidecarCleanup {
    cleanup_owned_process(child)
}

fn start_process(
    cache_root: &Path,
    generation: u64,
) -> Result<SidecarProcess, StartProcessFailure> {
    let (program, cwd) = resolve_sidecar().map_err(|message| StartProcessFailure {
        message,
        cleanup: SidecarCleanup::NotRequired,
    })?;
    let mut command = sidecar_command(&program, cache_root);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|_| StartProcessFailure {
        message: "The bundled Rust runtime could not be started.".to_string(),
        cleanup: SidecarCleanup::NotRequired,
    })?;
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            drop(child.stdout.take());
            let cleanup = cleanup_child(&mut child);
            return Err(StartProcessFailure {
                message: "Rust runtime stdin was unavailable.".to_string(),
                cleanup,
            });
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            drop(stdin);
            let cleanup = cleanup_child(&mut child);
            return Err(StartProcessFailure {
                message: "Rust runtime stdout was unavailable.".to_string(),
                cleanup,
            });
        }
    };
    Ok(SidecarProcess::Child {
        generation,
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

fn sidecar_command(program: &Path, cache_root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .arg("--sidecar")
        .arg("--cache-root")
        .arg(cache_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn resolve_sidecar() -> Result<(PathBuf, Option<PathBuf>), String> {
    if cfg!(debug_assertions) {
        let root =
            repo_root().ok_or_else(|| "The EmuChef repository root was not found.".to_string())?;
        let candidates = [
            root.join("crates/emuchef-rust-backend/target/debug/emuchef"),
            root.join("target/debug/emuchef"),
        ];
        let binary = candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| "Build the Rust runtime with npm run sidecar:dev.".to_string())?;
        return Ok((binary, Some(root)));
    }
    let current = env::current_exe()
        .map_err(|_| "Application executable location is unavailable.".to_string())?;
    let binary = current
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(if cfg!(windows) {
            "emuchef.exe"
        } else {
            "emuchef"
        });
    if !binary.is_file() {
        return Err("The packaged Rust runtime is missing.".to_string());
    }
    Ok((binary, None))
}

fn repo_root() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|candidate| {
            candidate
                .join("crates/emuchef-rust-backend/Cargo.toml")
                .is_file()
        })
        .map(Path::to_path_buf)
}

fn startup_result(envelope: &Value) -> Result<&Value, StartFailure> {
    match envelope.get("ok").and_then(Value::as_bool) {
        Some(true) => envelope.get("result").ok_or_else(|| {
            StartFailure::SessionLost(
                "Rust runtime compatibility response omitted result data.".to_string(),
            )
        }),
        Some(false) => Err(StartFailure::Unsupported(
            "Rust runtime rejected compatibility negotiation.".to_string(),
        )),
        None => Err(StartFailure::SessionLost(
            "Rust runtime compatibility response was structurally invalid.".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn scripted_process(response: Option<&[u8]>) -> (SidecarProcess, Arc<ScriptedProcessObserver>) {
        scripted_process_with_cleanup(response, SidecarCleanup::Confirmed)
    }

    fn scripted_process_with_cleanup(
        response: Option<&[u8]>,
        cleanup: SidecarCleanup,
    ) -> (SidecarProcess, Arc<ScriptedProcessObserver>) {
        // The observer is immutable after construction so the scripted seam
        // can expose cleanup truth without adding a second transport path.
        let observer = Arc::new(ScriptedProcessObserver {
            cleanup,
            ..ScriptedProcessObserver::default()
        });
        (
            SidecarProcess::Scripted {
                generation: 0,
                stdin: Vec::new(),
                stdout: std::io::Cursor::new(response.unwrap_or_default().to_vec()),
                observer: Arc::clone(&observer),
            },
            observer,
        )
    }

    struct ScriptedCleanupProcess {
        status_results: Vec<bool>,
        status_index: usize,
        kill_result: bool,
        reap_result: bool,
        kill_calls: usize,
        reap_calls: usize,
    }

    impl ScriptedCleanupProcess {
        fn new(status_results: &[bool], kill_result: bool, reap_result: bool) -> Self {
            Self {
                status_results: status_results.to_vec(),
                status_index: 0,
                kill_result,
                reap_result,
                kill_calls: 0,
                reap_calls: 0,
            }
        }
    }

    impl CleanupProcess for ScriptedCleanupProcess {
        fn try_exit(&mut self) -> bool {
            let result = self
                .status_results
                .get(self.status_index)
                .copied()
                .unwrap_or(false);
            self.status_index += 1;
            result
        }

        fn kill(&mut self) -> bool {
            self.kill_calls += 1;
            self.kill_result
        }

        fn reap_with_deadline(&mut self) -> bool {
            self.reap_calls += 1;
            self.reap_result
        }
    }

    fn negotiate(payload: Value, request_id: &str) -> Value {
        let request = json!({
            "id": request_id,
            "type": "negotiateCapabilities",
            "payload": payload,
        });
        let output = emuchef_rust_backend::jsonl::process_jsonl(&format!("{request}\n"));
        serde_json::from_str(output.trim()).expect("negotiation response should be valid JSON")
    }

    #[test]
    fn end_user_operations_are_all_negotiated() {
        assert_eq!(
            REQUIRED_CAPABILITIES,
            [
                "describeCatalog",
                "listAdbDevices",
                "probeDevice",
                "qualifyDevice",
                "checkRoot",
                "matchDevice",
                "describeConfiguration",
                "planConfiguration",
                "startExecution",
                "getExecution",
                "getExecutionEvents",
                "cancelExecution",
                "launchExecutionApp",
                "openUserConfiguration",
                "createUserConfiguration",
                "getUserConfigurationDocument",
                "saveUserConfiguration",
                "saveUserConfigurationAs",
                "setUserConfigurationBinding",
                "removeUserConfigurationBinding",
                "setUserConfigurationSelectedRecipes",
                "setUserConfigurationDevicePlan",
                "validateUserConfiguration",
                "closeUserConfiguration",
            ]
        );
    }

    #[test]
    fn fatal_protocol_loss_stops_the_process_and_marks_the_runtime_failed() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process_with_cleanup(None, SidecarCleanup::Uncertain);
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let accesses_after_loss = observer.transport_accesses.load(Ordering::SeqCst);

        let error: Value = serde_json::from_str(&error).expect("session loss should be structured");
        assert_eq!(error["code"], "runtime_session_lost");
        assert!(matches!(client.status, RuntimeStatusDto::Failed { .. }));
        assert!(client.session_lost);
        assert!(client.process.is_none());
        assert_eq!(client.last_cleanup, SidecarCleanup::Uncertain);
        assert!(observer.stopped.load(Ordering::SeqCst));

        let repeated = client.request("getExecution", json!({})).unwrap_err();
        let repeated: Value =
            serde_json::from_str(&repeated).expect("lost status should remain structured");
        assert_eq!(repeated["code"], "runtime_session_lost");
        assert_eq!(
            observer.transport_accesses.load(Ordering::SeqCst),
            accesses_after_loss
        );
    }

    #[test]
    fn malformed_success_envelope_invalidates_the_runtime_session() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process(Some(b"{\"id\":\"app-1\",\"ok\":true}\n"));
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let accesses_after_loss = observer.transport_accesses.load(Ordering::SeqCst);
        let repeated = client.request("getExecution", json!({})).unwrap_err();

        for error in [error, repeated] {
            let error: Value =
                serde_json::from_str(&error).expect("protocol loss should remain structured");
            assert_eq!(error["code"], "runtime_session_lost");
        }
        assert!(matches!(client.status, RuntimeStatusDto::Failed { .. }));
        assert!(client.process.is_none());
        assert!(observer.stopped.load(Ordering::SeqCst));
        assert_eq!(
            observer.transport_accesses.load(Ordering::SeqCst),
            accesses_after_loss
        );
    }

    #[test]
    fn valid_backend_error_does_not_invalidate_the_runtime_session() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process(Some(
            b"{\"id\":\"app-1\",\"ok\":false,\"error\":{\"code\":\"unknown_execution\",\"message\":\"Execution not found.\"}}\n",
        ));
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();

        let error: Value =
            serde_json::from_str(&error).expect("backend error should be structured");
        assert_eq!(
            error,
            json!({
                "code": "unknown_execution",
                "message": "Execution not found."
            })
        );
        assert!(matches!(client.status, RuntimeStatusDto::Ready { .. }));
        assert!(client.process.is_some());
        assert_eq!(client.last_cleanup, SidecarCleanup::NotRequired);
        assert!(!observer.stopped.load(Ordering::SeqCst));
        assert_ne!(error["code"], "runtime_session_lost");
    }

    #[test]
    fn malformed_error_envelope_invalidates_the_runtime_session_persistently() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process(Some(
            b"{\"id\":\"app-1\",\"ok\":false,\"error\":{\"code\":\"unknown_execution\"}}\n",
        ));
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let accesses_after_loss = observer.transport_accesses.load(Ordering::SeqCst);
        let repeated = client.request("getExecution", json!({})).unwrap_err();

        for error in [error, repeated] {
            let error: Value =
                serde_json::from_str(&error).expect("protocol loss should remain structured");
            assert_eq!(error["code"], "runtime_session_lost");
        }
        assert!(matches!(client.status, RuntimeStatusDto::Failed { .. }));
        assert!(client.process.is_none());
        assert!(observer.stopped.load(Ordering::SeqCst));
        assert_eq!(
            observer.transport_accesses.load(Ordering::SeqCst),
            accesses_after_loss
        );
    }

    #[test]
    fn frame_reader_is_incremental_bounded_and_preserves_buffered_frames() {
        let bytes = b"{\"id\":\"one\"}\n{\"id\":\"two\"}\n";
        let mut reader = futures_lite::io::BufReader::new(&bytes[..]);
        let frames = block_on(async {
            let first = read_frame(&mut reader).await.unwrap();
            let second = read_frame(&mut reader).await.unwrap();
            (first, second)
        });
        assert_eq!(trim_frame(&frames.0), br#"{"id":"one"}"#);
        assert_eq!(trim_frame(&frames.1), br#"{"id":"two"}"#);
    }

    #[test]
    fn frame_reader_rejects_partial_eof_and_oversize_frames() {
        let mut partial = futures_lite::io::BufReader::new(&b"partial"[..]);
        assert!(block_on(read_frame(&mut partial))
            .unwrap_err()
            .contains("partial frame"));

        let oversized_bytes = vec![b'x'; MAX_FRAME_BYTES + 1];
        let mut oversized = futures_lite::io::BufReader::new(&oversized_bytes[..]);
        assert!(block_on(read_frame(&mut oversized))
            .unwrap_err()
            .contains("frame limit"));
    }

    #[test]
    fn response_id_mismatch_invalidates_the_current_generation() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process(Some(
            br#"{"id":"stale-generation","ok":true,"result":{}}
"#,
        ));
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let error: Value = serde_json::from_str(&error).unwrap();
        assert_eq!(error["code"], "runtime_session_lost");
        assert!(client.process.is_none());
        assert!(observer.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn stale_process_generation_is_rejected_before_transport_access() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        client.generation = 1;
        client.status = RuntimeStatusDto::Ready {
            protocol_version: 1,
            catalog_version: None,
        };
        let (process, observer) = scripted_process(Some(
            br#"{"id":"app-1","ok":true,"result":{}}
"#,
        ));
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let error: Value = serde_json::from_str(&error).unwrap();
        assert_eq!(error["code"], "runtime_session_lost");
        assert_eq!(observer.transport_accesses.load(Ordering::SeqCst), 0);
        assert!(observer.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn startup_negotiation_payload_uses_the_real_backend_contract() {
        let payload = capability_negotiation_payload();
        assert_eq!(
            payload["requiredCapabilities"],
            serde_json::to_value(REQUIRED_CAPABILITIES).unwrap()
        );
        assert_eq!(
            payload["requiredCapabilities"].as_array().unwrap().len(),
            24
        );
        assert_eq!(payload["optionalCapabilities"], json!([]));
        assert!(payload.get("required").is_none());
        assert!(payload.get("optional").is_none());

        let supported = negotiate(payload.clone(), "all-required-supported");
        assert_eq!(supported["ok"], true, "{supported:#}");
        assert_eq!(supported["result"]["compatible"], true, "{supported:#}");
        assert_eq!(
            supported["result"]["enabledRequired"]
                .as_array()
                .unwrap()
                .len(),
            24
        );

        let mut missing_execution = payload;
        missing_execution["requiredCapabilities"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|capability| capability.as_str() == Some("cancelExecution"))
            .expect("cancelExecution should be required")
            .clone_from(&json!("missingExecutionCapability"));
        let incompatible = negotiate(missing_execution, "missing-required-execution");
        assert_eq!(incompatible["ok"], true, "{incompatible:#}");
        assert_eq!(
            incompatible["result"]["compatible"], false,
            "{incompatible:#}"
        );
        assert_eq!(
            incompatible["result"]["unsupportedRequired"],
            json!(["missingExecutionCapability"])
        );
    }

    #[test]
    fn startup_result_distinguishes_incompatibility_from_invalid_transport_data() {
        assert!(matches!(
            startup_result(&json!({ "ok": false })),
            Err(StartFailure::Unsupported(_))
        ));
        assert!(matches!(
            startup_result(&json!({ "ok": true })),
            Err(StartFailure::SessionLost(_))
        ));
        assert!(matches!(
            startup_result(&json!({ "result": {} })),
            Err(StartFailure::SessionLost(_))
        ));
    }

    #[test]
    fn cleanup_confirms_a_child_that_already_exited_without_killing_it() {
        let mut process = ScriptedCleanupProcess::new(&[true], true, false);

        assert_eq!(
            cleanup_owned_process(&mut process),
            SidecarCleanup::Confirmed
        );
        assert_eq!(process.kill_calls, 0);
        assert_eq!(process.reap_calls, 0);
    }

    #[test]
    fn cleanup_confirms_successful_kill_and_bounded_reap() {
        let mut process = ScriptedCleanupProcess::new(&[false, false], true, true);

        assert_eq!(
            cleanup_owned_process(&mut process),
            SidecarCleanup::Confirmed
        );
        assert_eq!(process.kill_calls, 1);
        assert_eq!(process.reap_calls, 1);
    }

    #[test]
    fn cleanup_confirms_exit_after_kill_failure_when_status_proves_termination() {
        let mut process = ScriptedCleanupProcess::new(&[false, true], false, false);

        assert_eq!(
            cleanup_owned_process(&mut process),
            SidecarCleanup::Confirmed
        );
        assert_eq!(process.kill_calls, 1);
        assert_eq!(process.reap_calls, 0);
    }

    #[test]
    fn cleanup_is_uncertain_when_bounded_reap_does_not_complete() {
        let mut process = ScriptedCleanupProcess::new(&[false, false], true, false);

        assert_eq!(
            cleanup_owned_process(&mut process),
            SidecarCleanup::Uncertain
        );
        assert_eq!(process.kill_calls, 1);
        assert_eq!(process.reap_calls, 1);
    }

    #[test]
    fn startup_transport_loss_retains_uncertain_cleanup_without_changing_public_status() {
        let mut client = SidecarClient::new(PathBuf::from("unused-test-cache"));
        let (process, observer) = scripted_process_with_cleanup(None, SidecarCleanup::Uncertain);

        client.initialize_with_process(process);

        assert!(matches!(
            client.status,
            RuntimeStatusDto::Failed { ref error } if error.code == "runtime_start_failed"
        ));
        assert!(client.session_lost);
        assert!(client.process.is_none());
        assert_eq!(client.last_cleanup, SidecarCleanup::Uncertain);
        assert!(observer.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn end_user_command_injects_only_the_trusted_cache_root() {
        let command = sidecar_command(Path::new("emuchef"), Path::new("/trusted/app/cache"));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(args, ["--sidecar", "--cache-root", "/trusted/app/cache"]);
    }

    #[test]
    fn diagnostics_omit_runtime_error_messages() {
        let state = SidecarState {
            inner: Mutex::new(SidecarClient {
                process: None,
                status: RuntimeStatusDto::Failed {
                    error: RuntimeErrorDto {
                        code: "runtime_start_failed",
                        message: "/Users/alice/private/runtime failed".to_string(),
                        actions: vec!["retry"],
                    },
                },
                session_lost: false,
                last_cleanup: SidecarCleanup::NotRequired,
                next_request_id: 1,
                cache_root: PathBuf::from("/trusted/app/cache"),
                generation: 1,
            }),
        };
        let diagnostics = state.diagnostics();
        assert_eq!(diagnostics["status"]["status"], "failed");
        assert_eq!(
            diagnostics["status"]["error"],
            json!({ "code": "runtime_start_failed", "actions": ["retry"] })
        );
        assert!(!diagnostics.to_string().contains("/Users/alice"));
    }
}

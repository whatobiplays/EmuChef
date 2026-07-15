use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::Mutex,
};

use serde_json::{json, Map, Value};

const SUPPORTED_PROTOCOL_VERSION: i64 = 1;
const REQUIRED_CAPABILITIES: &[&str] = &[
    "listStepSpecs",
    "openRecipe",
    "getDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "saveRecipe",
    // The Tauri command is registered, so compatibility must fail before any
    // command can forward to a backend that does not support it.
    "saveRecipeAs",
    "validate",
    "emitYaml",
    "getRefIndex",
    "setDocumentAuthoredRoot",
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
    "emitUserConfigurationYaml",
    "setUserConfigurationAuthoredRoot",
    "closeUserConfiguration",
    "describeConfiguration",
    "planConfiguration",
    "listAdbDevices",
    "probeDevice",
    "inspectApk",
    "generateAppRecipeDraft",
    "generateDeviceProfileDraft",
    "checkGeneratedCatalogCollisions",
    "ping",
];
const RUST_BACKEND_MANIFEST: &str = "crates/emuchef-rust-backend/Cargo.toml";
const TAURI_MANIFEST: &str = "apps/config-editor/src-tauri/Cargo.toml";
const STDERR_EXCERPT_BYTES: u64 = 4096;

/// Shared Tauri state for the persistent Rust sidecar.
///
/// Sidecar request handling is deliberately serial. The mutex is
/// held for the full send-line/read-line exchange so requests cannot interleave
/// on the JSONL protocol.
pub struct SidecarState {
    client: Mutex<SidecarClient>,
}

impl Default for SidecarState {
    fn default() -> Self {
        Self::new(SidecarRuntime::Dev)
    }
}

impl SidecarState {
    pub fn new(runtime: SidecarRuntime) -> Self {
        Self {
            client: Mutex::new(SidecarClient::new(runtime)),
        }
    }

    pub fn status(&self) -> Result<Value, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "Sidecar client lock is poisoned".to_string())?;
        Ok(client.status())
    }

    pub fn restart(&self) -> Result<Value, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "Sidecar client lock is poisoned".to_string())?;
        client.restart()
    }

    pub fn request(&self, request_type: &str, payload: Option<Value>) -> Result<Value, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "Sidecar client lock is poisoned".to_string())?;
        client.request(request_type, payload)
    }
}

pub struct SidecarClient {
    next_request_id: u64,
    state: ProcessState,
    runtime: SidecarRuntime,
    binary_path_override: Option<PathBuf>,
}

/// Selects the sidecar binary layout used by the Tauri backend bridge.
///
/// Development builds intentionally keep the repo-local resolver so
/// tests and `tauri dev` use deterministic Cargo outputs. Packaged builds use
/// the Tauri externalBin directory beside the app executable and never fall back
/// to development paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidecarRuntime {
    Dev,
    Packaged { bundled_dir: Option<PathBuf> },
}

impl SidecarRuntime {
    pub fn for_current_process() -> Self {
        if cfg!(debug_assertions) {
            Self::Dev
        } else {
            Self::Packaged {
                bundled_dir: current_exe_start(),
            }
        }
    }
}

enum ProcessState {
    NotStarted,
    Running(RunningSidecar),
    Exited {
        last_error: Option<String>,
        compatibility: Option<ProtocolCompatibility>,
    },
    Incompatible {
        last_error: String,
        protocol_version: Option<i64>,
        capabilities: Vec<String>,
    },
}

struct RunningSidecar {
    process: SidecarProcess,
    compatibility: ProtocolCompatibility,
}

struct SidecarProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Option<ChildStderr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolCompatibility {
    protocol_version: i64,
    capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HelloCompatibilityError {
    message: String,
    protocol_version: Option<i64>,
    capabilities: Vec<String>,
}

impl SidecarClient {
    pub fn new(runtime: SidecarRuntime) -> Self {
        Self {
            next_request_id: 1,
            state: ProcessState::NotStarted,
            runtime,
            binary_path_override: None,
        }
    }

    #[cfg(test)]
    fn with_binary_path_for_test(path: impl Into<PathBuf>) -> Self {
        Self {
            next_request_id: 1,
            state: ProcessState::NotStarted,
            runtime: SidecarRuntime::Dev,
            binary_path_override: Some(path.into()),
        }
    }

    #[cfg(test)]
    fn with_runtime_for_test(runtime: SidecarRuntime) -> Self {
        Self {
            next_request_id: 1,
            state: ProcessState::NotStarted,
            runtime,
            binary_path_override: None,
        }
    }

    pub fn status(&mut self) -> Value {
        json!({
            "ok": true,
            "result": self.status_result(),
        })
    }

    fn status_result(&mut self) -> Value {
        match &mut self.state {
            ProcessState::NotStarted => json!({
                "running": false,
                "pid": null,
                "state": "notStarted",
                "compatible": null,
                "protocolVersion": null,
                "capabilities": [],
                "lastError": null,
            }),
            ProcessState::Exited {
                last_error,
                compatibility,
            } => json!({
                "running": false,
                "pid": null,
                "state": "exited",
                "compatible": compatibility.as_ref().map(|_| true),
                "protocolVersion": compatibility.as_ref().map(|item| item.protocol_version),
                "capabilities": compatibility
                    .as_ref()
                    .map(|item| item.capabilities.clone())
                    .unwrap_or_default(),
                "lastError": last_error,
            }),
            ProcessState::Incompatible {
                last_error,
                protocol_version,
                capabilities,
            } => json!({
                "running": false,
                "pid": null,
                "state": "incompatible",
                "compatible": false,
                "protocolVersion": protocol_version,
                "capabilities": capabilities,
                "lastError": last_error,
            }),
            ProcessState::Running(running) => match running.process.child.try_wait() {
                Ok(Some(_status)) => {
                    let compatibility = running.compatibility.clone();
                    self.state = ProcessState::Exited {
                        last_error: None,
                        compatibility: Some(compatibility.clone()),
                    };
                    json!({
                        "running": false,
                        "pid": null,
                        "state": "exited",
                        "compatible": true,
                        "protocolVersion": compatibility.protocol_version,
                        "capabilities": compatibility.capabilities,
                        "lastError": null,
                    })
                }
                Ok(None) => json!({
                    "running": true,
                    "pid": running.process.child.id(),
                    "state": "running",
                    "compatible": true,
                    "protocolVersion": running.compatibility.protocol_version,
                    "capabilities": running.compatibility.capabilities.clone(),
                    "lastError": null,
                }),
                Err(err) => json!({
                    "running": false,
                    "pid": null,
                    "state": "error",
                    "compatible": true,
                    "protocolVersion": running.compatibility.protocol_version,
                    "capabilities": running.compatibility.capabilities.clone(),
                    "lastError": format!("Failed to inspect backend sidecar process: {err}"),
                }),
            },
        }
    }

    pub fn restart(&mut self) -> Result<Value, String> {
        self.stop_owned_process();
        let mut process = start_sidecar(self.binary_path_override.as_deref(), &self.runtime)?;

        match self.perform_hello_handshake(&mut process) {
            Ok(compatibility) => {
                self.state = ProcessState::Running(RunningSidecar {
                    process,
                    compatibility,
                });
            }
            Err(err) => {
                let err = stop_after_failed_handshake(process, err);
                self.state = ProcessState::Incompatible {
                    last_error: err.message,
                    protocol_version: err.protocol_version,
                    capabilities: err.capabilities,
                };
            }
        }

        Ok(json!({
            "ok": true,
            "result": {
                "status": self.status_result(),
                "documentSessionsPreserved": false,
            },
        }))
    }

    pub fn request(&mut self, request_type: &str, payload: Option<Value>) -> Result<Value, String> {
        self.ensure_running()?;

        let request_id = self.allocate_request_id();
        let request = build_sidecar_request(&request_id, request_type, payload);

        let mut mark_exited: Option<(String, ProtocolCompatibility)> = None;
        let response = match &mut self.state {
            ProcessState::Running(running) => {
                match exchange_sidecar_request(&mut running.process, &request) {
                    Ok(response_line) => parse_sidecar_response_line(&response_line, &request_id),
                    Err(err) => {
                        mark_exited = Some((err.clone(), running.compatibility.clone()));
                        Err(err)
                    }
                }
            }
            ProcessState::NotStarted
            | ProcessState::Exited { .. }
            | ProcessState::Incompatible { .. } => {
                unreachable!("ensure_running failed to start sidecar")
            }
        };

        if let Some((last_error, compatibility)) = mark_exited {
            self.state = ProcessState::Exited {
                last_error: Some(last_error),
                compatibility: Some(compatibility),
            };
        }

        response
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        match &mut self.state {
            ProcessState::NotStarted => {
                let mut process =
                    start_sidecar(self.binary_path_override.as_deref(), &self.runtime)?;
                match self.perform_hello_handshake(&mut process) {
                    Ok(compatibility) => {
                        self.state = ProcessState::Running(RunningSidecar {
                            process,
                            compatibility,
                        });
                        Ok(())
                    }
                    Err(err) => {
                        let err = stop_after_failed_handshake(process, err);
                        self.state = ProcessState::Incompatible {
                            last_error: err.message.clone(),
                            protocol_version: err.protocol_version,
                            capabilities: err.capabilities,
                        };
                        Err(err.message)
                    }
                }
            }
            ProcessState::Running(running) => match running.process.child.try_wait() {
                Ok(Some(status)) => {
                    let compatibility = running.compatibility.clone();
                    let last_error = format!(
                        "Backend sidecar exited unexpectedly with status {status}; explicitly restart the sidecar and reopen the recipe to create a new document session."
                    );
                    self.state = ProcessState::Exited {
                        last_error: Some(last_error.clone()),
                        compatibility: Some(compatibility),
                    };
                    Err(last_error)
                }
                Ok(None) => Ok(()),
                Err(err) => Err(format!("Failed to inspect backend sidecar process: {err}")),
            },
            ProcessState::Exited { .. } => Err(
                "Backend sidecar has exited and is not restarted automatically; explicitly restart the sidecar and reopen the recipe to create a new document session."
                    .to_string(),
            ),
            ProcessState::Incompatible { last_error, .. } => {
                Err(format!("Backend sidecar is incompatible: {last_error}"))
            }
        }
    }

    fn perform_hello_handshake(
        &mut self,
        process: &mut SidecarProcess,
    ) -> Result<ProtocolCompatibility, HelloCompatibilityError> {
        let request_id = self.allocate_request_id();
        let request = build_sidecar_request(&request_id, "hello", None);
        let response_line = exchange_sidecar_request(process, &request).map_err(|err| {
            hello_error(format!(
                "Backend hello transport failed: {err}. {}",
                rust_sidecar_start_guidance()
            ))
        })?;
        let response = parse_sidecar_response_line(&response_line, &request_id)
            .map_err(|err| hello_error(format!("Backend hello response was malformed: {err}")))?;
        validate_hello_response(&response)
    }

    fn allocate_request_id(&mut self) -> String {
        let request_id = format!("req-{}", self.next_request_id);
        self.next_request_id += 1;
        request_id
    }

    fn stop_owned_process(&mut self) {
        let state = std::mem::replace(&mut self.state, ProcessState::NotStarted);
        if let ProcessState::Running(mut running) = state {
            stop_sidecar_process(&mut running.process);
        }
    }

    #[cfg(test)]
    fn mark_exited_for_test(&mut self) {
        self.state = ProcessState::Exited {
            last_error: Some("Backend sidecar exited.".to_string()),
            compatibility: None,
        };
    }

    #[cfg(test)]
    fn mark_incompatible_for_test(
        &mut self,
        last_error: String,
        protocol_version: Option<i64>,
        capabilities: Vec<String>,
    ) {
        self.state = ProcessState::Incompatible {
            last_error,
            protocol_version,
            capabilities,
        };
    }
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        if let ProcessState::Running(running) = &mut self.state {
            stop_sidecar_process(&mut running.process);
        }
    }
}

fn stop_sidecar_process(process: &mut SidecarProcess) {
    let _ = process.child.kill();
    let _ = process.child.wait();
}

fn exchange_sidecar_request(
    process: &mut SidecarProcess,
    request: &Value,
) -> Result<String, String> {
    let request_json = serde_json::to_string(request)
        .map_err(|err| format!("Failed to serialize sidecar request: {err}"))?;

    process
        .stdin
        .write_all(request_json.as_bytes())
        .and_then(|_| process.stdin.write_all(b"\n"))
        .and_then(|_| process.stdin.flush())
        .map_err(|err| format!("Failed to write to backend sidecar stdin: {err}"))?;

    let mut response_line = String::new();
    let bytes_read = process
        .stdout
        .read_line(&mut response_line)
        .map_err(|err| format!("Failed to read backend sidecar stdout: {err}"))?;
    if bytes_read == 0 {
        return Err(
            "Backend sidecar exited before writing a response; explicitly restart the sidecar and reopen the recipe to create a new document session."
                .to_string(),
        );
    }
    Ok(response_line)
}

fn validate_hello_response(
    response: &Value,
) -> Result<ProtocolCompatibility, HelloCompatibilityError> {
    match response.get("ok").and_then(Value::as_bool) {
        Some(false) => {
            let error = response.get("error").and_then(Value::as_object);
            let code = error
                .and_then(|item| item.get("code"))
                .and_then(Value::as_str);
            let message = error
                .and_then(|item| item.get("message"))
                .and_then(Value::as_str);
            let detail = match (code, message) {
                (Some(code), Some(message)) => format!("{code}: {message}"),
                (Some(code), None) => code.to_string(),
                (None, Some(message)) => message.to_string(),
                (None, None) => "unknown API error".to_string(),
            };
            Err(hello_error(format!("Backend hello failed: {detail}")))
        }
        Some(true) => {
            let result = response
                .get("result")
                .and_then(Value::as_object)
                .ok_or_else(|| hello_error("Backend hello response was malformed.".to_string()))?;
            let protocol_version = result
                .get("protocolVersion")
                .and_then(Value::as_i64)
                .ok_or_else(|| hello_error("Backend hello response was malformed.".to_string()))?;
            let capabilities = result
                .get("capabilities")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    hello_error_with_metadata(
                        "Backend hello response was malformed.".to_string(),
                        Some(protocol_version),
                        Vec::new(),
                    )
                })?
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_string).ok_or_else(|| {
                        hello_error_with_metadata(
                            "Backend hello response was malformed.".to_string(),
                            Some(protocol_version),
                            Vec::new(),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            if protocol_version != SUPPORTED_PROTOCOL_VERSION {
                return Err(hello_error_with_metadata(
                    format!(
                        "Backend protocol version {protocol_version} is not supported. This editor supports protocol version {SUPPORTED_PROTOCOL_VERSION}."
                    ),
                    Some(protocol_version),
                    capabilities,
                ));
            }

            let missing = REQUIRED_CAPABILITIES
                .iter()
                .copied()
                .filter(|required| !capabilities.iter().any(|capability| capability == required))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(hello_error_with_metadata(
                    format!(
                        "Backend is missing required capabilities: {}.",
                        missing.join(", ")
                    ),
                    Some(protocol_version),
                    capabilities,
                ));
            }

            Ok(ProtocolCompatibility {
                protocol_version,
                capabilities,
            })
        }
        None => Err(hello_error(
            "Backend hello response was malformed.".to_string(),
        )),
    }
}

fn hello_error(message: String) -> HelloCompatibilityError {
    hello_error_with_metadata(message, None, Vec::new())
}

fn hello_error_with_metadata(
    message: String,
    protocol_version: Option<i64>,
    capabilities: Vec<String>,
) -> HelloCompatibilityError {
    HelloCompatibilityError {
        message,
        protocol_version,
        capabilities,
    }
}

fn stop_after_failed_handshake(
    mut process: SidecarProcess,
    mut err: HelloCompatibilityError,
) -> HelloCompatibilityError {
    stop_sidecar_process(&mut process);
    if let Some(stderr) = stderr_excerpt(process.stderr.take()) {
        err.message = format!("{} Backend stderr: {stderr}", err.message);
    }
    err
}

fn stderr_excerpt(stderr: Option<ChildStderr>) -> Option<String> {
    let stderr = stderr?;
    let mut limited = stderr.take(STDERR_EXCERPT_BYTES);
    let mut text = String::new();
    if limited.read_to_string(&mut text).is_err() {
        return None;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn build_sidecar_request(
    request_id: &str,
    request_type: &str,
    payload: Option<Value>,
) -> Value {
    let mut request = Map::new();
    request.insert("id".to_string(), Value::String(request_id.to_string()));
    request.insert("type".to_string(), Value::String(request_type.to_string()));
    request.insert(
        "payload".to_string(),
        payload.unwrap_or_else(|| Value::Object(Map::new())),
    );
    Value::Object(request)
}

pub fn parse_sidecar_response_line(line: &str, expected_request_id: &str) -> Result<Value, String> {
    let response: Value = serde_json::from_str(line.trim_end())
        .map_err(|err| format!("Backend sidecar stdout line was not valid JSON: {err}"))?;
    if !response.is_object() {
        return Err("Backend sidecar response must be a JSON object".to_string());
    }

    let response_id = response
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Backend sidecar response is missing string id".to_string())?;
    if response_id != expected_request_id {
        return Err(format!(
            "Backend sidecar response id mismatch: expected {expected_request_id}, received {response_id}"
        ));
    }

    match response.get("ok").and_then(Value::as_bool) {
        Some(true) if response.get("result").is_some() => Ok(response),
        Some(false) if response.get("error").is_some() => Ok(response),
        Some(_) => Err("Backend sidecar response is missing result/error data".to_string()),
        None => Err("Backend sidecar response is missing boolean ok field".to_string()),
    }
}

fn start_sidecar(
    binary_path_override: Option<&Path>,
    runtime: &SidecarRuntime,
) -> Result<SidecarProcess, String> {
    let spec = rust_sidecar_command_spec(binary_path_override, runtime)?;
    let mut command = spec.command();

    let mut child = command.spawn().map_err(|err| {
        format!(
            "Failed to start Rust sidecar at '{}': {err}. {}",
            spec.program.display(),
            rust_sidecar_start_guidance()
        )
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture Rust sidecar stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture Rust sidecar stdout".to_string())?;
    let stderr = child.stderr.take();

    Ok(SidecarProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        stderr,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SidecarCommandSpec {
    program: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl SidecarCommandSpec {
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command
    }
}

fn rust_sidecar_command_spec(
    binary_path_override: Option<&Path>,
    runtime: &SidecarRuntime,
) -> Result<SidecarCommandSpec, String> {
    match binary_path_override {
        Some(path) => Ok(SidecarCommandSpec {
            program: path.to_path_buf(),
            args: vec!["--sidecar".to_string()],
            cwd: discover_repo_root(),
        }),
        None => match runtime {
            SidecarRuntime::Dev => {
                let repo_root = discover_repo_root();
                let program = resolve_dev_rust_sidecar_binary(repo_root.as_deref())?;
                Ok(SidecarCommandSpec {
                    program,
                    args: vec!["--sidecar".to_string()],
                    cwd: repo_root,
                })
            }
            SidecarRuntime::Packaged { bundled_dir } => {
                let program = resolve_packaged_rust_sidecar_binary(bundled_dir.as_deref())?;
                Ok(SidecarCommandSpec {
                    program,
                    args: vec!["--sidecar".to_string()],
                    cwd: None,
                })
            }
        },
    }
}

fn resolve_dev_rust_sidecar_binary(repo_root: Option<&Path>) -> Result<PathBuf, String> {
    let Some(repo_root) = repo_root else {
        return Err(format!(
            "Could not locate the EmuChef repository root. {}",
            rust_sidecar_start_guidance()
        ));
    };
    let candidates = rust_sidecar_binary_candidates(repo_root);
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .ok_or_else(|| {
            let searched = candidates
                .iter()
                .map(|path| format!("'{}'", path.display()))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Rust sidecar binary was not found. Searched: {searched}. {}",
                dev_sidecar_start_guidance()
            )
        })
}

fn resolve_packaged_rust_sidecar_binary(bundled_dir: Option<&Path>) -> Result<PathBuf, String> {
    let Some(bundled_dir) = bundled_dir else {
        return Err(format!(
            "Tauri bundled sidecar directory was not available while resolving the bundled Rust sidecar. {}",
            packaged_sidecar_start_guidance()
        ));
    };
    let candidate = bundled_dir.join(rust_backend_binary_name());
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "The bundled Rust sidecar binary was not found at '{}'. {}",
            candidate.display(),
            packaged_sidecar_start_guidance()
        ))
    }
}

fn rust_sidecar_binary_candidates(repo_root: &Path) -> Vec<PathBuf> {
    let binary = rust_backend_binary_name();
    vec![
        repo_root
            .join("crates")
            .join("emuchef-rust-backend")
            .join("target")
            .join("debug")
            .join(binary),
        repo_root.join("target").join("debug").join(binary),
    ]
}

fn rust_backend_binary_name() -> &'static str {
    if cfg!(windows) {
        "emuchef.exe"
    } else {
        "emuchef"
    }
}

fn rust_sidecar_start_guidance() -> &'static str {
    "For development, run `npm run sidecar:dev` from apps/config-editor or `cargo build --manifest-path crates/emuchef-rust-backend/Cargo.toml` from the repository root. For packaged builds, run the Tauri build flow so `npm run sidecar:build` prepares the bundled Rust sidecar."
}

fn dev_sidecar_start_guidance() -> &'static str {
    "Build the local development sidecar with `npm run sidecar:dev` from apps/config-editor, or run `cargo build --manifest-path crates/emuchef-rust-backend/Cargo.toml` from the repository root."
}

fn packaged_sidecar_start_guidance() -> &'static str {
    "Run `npm run sidecar:build` before packaging, or use `npm run tauri build` so Tauri prepares and bundles the Rust sidecar via externalBin."
}

pub(crate) fn discover_repo_root() -> Option<PathBuf> {
    let starts = [current_exe_start(), env::current_dir().ok()];
    starts
        .into_iter()
        .flatten()
        .find_map(|start| walk_up_for_repo_root(&start))
}

fn current_exe_start() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    if exe.is_dir() {
        Some(exe)
    } else {
        exe.parent().map(Path::to_path_buf)
    }
}

fn walk_up_for_repo_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        if candidate.join(RUST_BACKEND_MANIFEST).is_file()
            && candidate.join(TAURI_MANIFEST).is_file()
        {
            Some(candidate.to_path_buf())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempRecipe {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempRecipe {
        fn copy_fixture(name: &str) -> Self {
            static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
            let root = repo_root_for_test();
            let fixture = root
                .join("crates")
                .join("emuchef-rust-backend")
                .join("tests")
                .join("fixtures")
                .join("recipes")
                .join(name);
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "emuchef-tauri-sidecar-{}-{unique}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("temp directory should be created");
            let path = dir.join(name);
            fs::copy(fixture, &path).expect("fixture should copy to temp path");
            Self { dir, path }
        }
    }

    impl Drop for TempRecipe {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn repo_root_for_test() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|candidate| {
                candidate.join(RUST_BACKEND_MANIFEST).is_file()
                    && candidate.join(TAURI_MANIFEST).is_file()
            })
            .expect("test should run inside the EmuChef repo")
            .to_path_buf()
    }

    fn rust_backend_binary_for_test() -> PathBuf {
        let root = repo_root_for_test();
        let manifest = root.join(RUST_BACKEND_MANIFEST);
        let status = Command::new("cargo")
            .arg("build")
            .arg("--manifest-path")
            .arg(&manifest)
            .status()
            .expect("cargo build should start for Rust sidecar test binary");
        assert!(status.success(), "Rust sidecar test binary should build");

        rust_sidecar_binary_candidates(&root)
            .into_iter()
            .find(|candidate| candidate.is_file())
            .expect("Rust sidecar binary should exist after cargo build")
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emuchef-tauri-{prefix}-{}-{unique}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp directory should be created");
        dir
    }

    fn assert_no_python_or_uv_spawn(spec: &SidecarCommandSpec) {
        let mut tokens = vec![spec.program.to_string_lossy().to_lowercase()];
        tokens.extend(spec.args.iter().map(|arg| arg.to_lowercase()));
        for forbidden in ["python", "python3", "uv"] {
            assert!(
                !tokens.iter().any(|token| token == forbidden),
                "sidecar command must not spawn {forbidden}: {tokens:?}"
            );
        }
    }

    #[test]
    fn builds_sidecar_request_with_opaque_id_and_payload() {
        let request = build_sidecar_request(
            "req-42",
            "openRecipe",
            Some(json!({
                "path": "/tmp/example.yaml",
                "authoredRoot": null,
            })),
        );

        assert_eq!(
            request,
            json!({
                "id": "req-42",
                "type": "openRecipe",
                "payload": {
                    "path": "/tmp/example.yaml",
                    "authoredRoot": null,
                },
            })
        );
    }

    #[test]
    fn parses_api_failure_envelope_as_successful_transport_response() {
        let response = parse_sidecar_response_line(
            r#"{"id":"req-1","ok":false,"error":{"code":"unknown_document","message":"missing","details":{}}}"#,
            "req-1",
        )
        .expect("API failure envelopes should not be transport errors");

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "unknown_document");
    }

    #[test]
    fn preserves_nested_document_object_order_when_parsing_sidecar_responses() {
        let response = parse_sidecar_response_line(
            r#"{"id":"req-1","ok":true,"result":{"document":{"recipe":{"artifactGroups":{"third_group":[],"first_group":[],"second_group":[]}}}}}"#,
            "req-1",
        )
        .expect("valid sidecar response should parse");

        let group_ids = response["result"]["document"]["recipe"]["artifactGroups"]
            .as_object()
            .expect("artifact groups should be a JSON object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();

        assert_eq!(group_ids, ["third_group", "first_group", "second_group"]);
    }

    #[test]
    fn rejects_mismatched_response_id() {
        let err = parse_sidecar_response_line(r#"{"id":"other","ok":true,"result":{}}"#, "req-1")
            .expect_err("mismatched response ids should be transport errors");

        assert!(err.contains("response id"));
    }

    #[test]
    fn rejects_malformed_response_envelope() {
        let err = parse_sidecar_response_line("not json", "req-1")
            .expect_err("malformed response should be rejected");

        assert!(err.contains("valid JSON"));
    }

    #[test]
    fn rust_sidecar_command_uses_only_backend_binary_and_sidecar_arg() {
        let binary = PathBuf::from("/tmp/emuchef");
        let spec = rust_sidecar_command_spec(Some(&binary), &SidecarRuntime::Dev)
            .expect("override should build spec");

        assert_eq!(spec.program, binary);
        assert_eq!(spec.args, vec!["--sidecar"]);
        assert_no_python_or_uv_spawn(&spec);
    }

    #[test]
    fn rust_sidecar_resolver_checks_crate_local_and_repo_root_targets() {
        let candidates = rust_sidecar_binary_candidates(Path::new("/repo"));

        assert_eq!(
            candidates,
            vec![
                Path::new("/repo")
                    .join("crates/emuchef-rust-backend/target/debug")
                    .join(rust_backend_binary_name()),
                Path::new("/repo")
                    .join("target/debug")
                    .join(rust_backend_binary_name()),
            ]
        );
    }

    #[test]
    fn packaged_sidecar_resolver_uses_bundled_directory_binary() {
        let bundled_dir = unique_temp_dir("packaged-bundled-dir");
        let binary = bundled_dir.join(rust_backend_binary_name());
        fs::write(&binary, b"packaged sidecar").expect("packaged binary should be writable");

        let spec = rust_sidecar_command_spec(
            None,
            &SidecarRuntime::Packaged {
                bundled_dir: Some(bundled_dir.clone()),
            },
        )
        .expect("packaged sidecar binary should resolve");

        assert_eq!(spec.program, binary);
        assert_eq!(spec.args, vec!["--sidecar"]);
        assert_eq!(spec.cwd, None);
        assert_no_python_or_uv_spawn(&spec);
    }

    #[test]
    fn packaged_sidecar_resolver_does_not_fall_back_to_dev_targets() {
        let bundled_dir = unique_temp_dir("missing-packaged-bundled-dir");

        let err = rust_sidecar_command_spec(
            None,
            &SidecarRuntime::Packaged {
                bundled_dir: Some(bundled_dir.clone()),
            },
        )
        .expect_err("packaged mode must require the bundled sidecar binary");

        assert!(err.contains("bundled Rust sidecar binary was not found"));
        assert!(err.contains(&bundled_dir.display().to_string()));
        assert!(!err.contains("target/debug"));
    }

    #[test]
    fn packaged_sidecar_resolver_reports_missing_bundled_directory() {
        let err = rust_sidecar_command_spec(None, &SidecarRuntime::Packaged { bundled_dir: None })
            .expect_err("packaged mode should fail clearly without a bundled directory");

        assert!(err.contains("Tauri bundled sidecar directory was not available"));
        assert!(!err.contains("target/debug"));
    }

    #[test]
    fn dev_sidecar_resolver_does_not_require_packaged_bundle_layout() {
        let repo_root = unique_temp_dir("dev-repo");
        let binary = repo_root
            .join("crates")
            .join("emuchef-rust-backend")
            .join("target")
            .join("debug")
            .join(rust_backend_binary_name());
        fs::create_dir_all(binary.parent().expect("binary should have parent"))
            .expect("dev target directory should be created");
        fs::write(&binary, b"dev sidecar").expect("dev binary should be writable");

        let resolved = resolve_dev_rust_sidecar_binary(Some(&repo_root))
            .expect("dev resolver should not require packaged resources");

        assert_eq!(resolved, binary);
    }

    #[test]
    fn missing_dev_sidecar_error_points_to_sidecar_dev_script() {
        let repo_root = unique_temp_dir("missing-dev-repo");

        let err = resolve_dev_rust_sidecar_binary(Some(&repo_root))
            .expect_err("missing dev sidecar should fail clearly");

        assert!(err.contains("Rust sidecar binary was not found"));
        assert!(err.contains("npm run sidecar:dev"));
        assert!(err.contains("cargo build --manifest-path"));
    }

    fn copy_sidecar_to_packaged_bundled_dir(source_binary: &Path) -> (PathBuf, PathBuf) {
        let bundled_dir = unique_temp_dir("packaged-sidecar-smoke");
        let packaged_binary = bundled_dir.join(rust_backend_binary_name());
        fs::copy(source_binary, &packaged_binary)
            .expect("sidecar binary should copy into simulated resource dir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&packaged_binary)
                .expect("packaged binary metadata should be readable")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&packaged_binary, permissions)
                .expect("packaged binary permissions should be set");
            let mode = fs::metadata(&packaged_binary)
                .expect("packaged binary metadata should be readable")
                .permissions()
                .mode();
            assert_ne!(
                mode & 0o111,
                0,
                "packaged Rust sidecar should be executable on Unix/macOS"
            );
        }

        assert!(packaged_binary.is_file());
        assert_eq!(
            packaged_binary.file_name().and_then(|name| name.to_str()),
            Some(rust_backend_binary_name())
        );
        (bundled_dir, packaged_binary)
    }

    fn run_editor_sidecar_smoke_sequence(client: &mut SidecarClient) {
        let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
        let save_as_path = temp_recipe.dir.join("tauri_smoke_saved_as.yaml");

        let specs_before_start = client
            .request("listStepSpecs", None)
            .expect("stateless listStepSpecs should start the Rust sidecar");
        assert_eq!(specs_before_start["ok"], true);
        assert!(specs_before_start["result"]["stepSpecs"].is_array());

        let hello = client.request("hello", None).expect("hello should succeed");
        assert_eq!(hello["ok"], true);
        assert_eq!(hello["result"]["protocolVersion"], 1);
        assert!(hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("saveRecipeAs")));
        assert!(hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("setDocumentAuthoredRoot")));

        let opened = client
            .request(
                "openRecipe",
                Some(json!({"path": temp_recipe.path, "authoredRoot": null})),
            )
            .expect("openRecipe should succeed");
        assert_eq!(opened["ok"], true);
        let document_id = opened["result"]["document"]["documentId"]
            .as_str()
            .unwrap()
            .to_string();

        let validated_path = client
            .request(
                "validateRecipePath",
                Some(json!({"path": temp_recipe.path, "authoredRoot": null})),
            )
            .expect("stateless validateRecipePath should work after sidecar startup");
        assert_eq!(validated_path["ok"], true);

        let emitted_from_path = client
            .request(
                "emitRecipeYamlFromPath",
                Some(json!({"path": temp_recipe.path, "authoredRoot": null})),
            )
            .expect("stateless emitRecipeYamlFromPath should work after sidecar startup");
        assert_eq!(emitted_from_path["ok"], true);
        assert!(emitted_from_path["result"]["yaml"]
            .as_str()
            .unwrap()
            .contains("id: phase6e.minimal"));

        let fetched = client
            .request("getDocument", Some(json!({"documentId": document_id})))
            .expect("getDocument should succeed");
        assert_eq!(fetched["ok"], true);

        let context_updated = client
            .request(
                "setDocumentAuthoredRoot",
                Some(json!({"documentId": document_id, "authoredRoot": null})),
            )
            .expect("setDocumentAuthoredRoot should succeed");
        assert_eq!(context_updated["ok"], true);
        assert_eq!(
            context_updated["result"]["document"]["documentId"],
            document_id
        );

        let changed = client
            .request(
                "applyRecipeCommand",
                Some(json!({
                    "documentId": document_id,
                    "command": {"type": "SetOverviewField", "field": "name", "value": "Tauri Rust Smoke"}
                })),
            )
            .expect("applyRecipeCommand should succeed");
        assert_eq!(changed["ok"], true);
        assert_eq!(changed["result"]["commandResult"]["changed"], true);

        let validated = client
            .request("validate", Some(json!({"documentId": document_id})))
            .expect("validate should succeed");
        assert_eq!(validated["ok"], true);
        assert!(validated["result"]["diagnostics"].as_array().is_some());

        let emitted = client
            .request("emitYaml", Some(json!({"documentId": document_id})))
            .expect("emitYaml should succeed");
        assert_eq!(emitted["ok"], true);
        assert!(emitted["result"]["yaml"]
            .as_str()
            .unwrap()
            .contains("name: Tauri Rust Smoke"));

        let saved = client
            .request("saveRecipe", Some(json!({"documentId": document_id})))
            .expect("saveRecipe should succeed");
        assert_eq!(saved["ok"], true);
        assert_eq!(saved["result"]["document"]["dirty"], false);

        let saved_as = client
            .request(
                "saveRecipeAs",
                Some(json!({"documentId": document_id, "path": save_as_path})),
            )
            .expect("saveRecipeAs should succeed");
        assert_eq!(saved_as["ok"], true);
        assert_eq!(saved_as["result"]["document"]["documentId"], document_id);
        assert!(save_as_path.exists());

        let closed = client
            .request("closeDocument", Some(json!({"documentId": document_id})))
            .expect("closeDocument should succeed");
        assert_eq!(closed["ok"], true);
    }

    #[test]
    fn validates_successful_hello_with_extra_capabilities() {
        let capabilities = REQUIRED_CAPABILITIES
            .iter()
            .copied()
            .chain(["futureCapability"])
            .collect::<Vec<_>>();
        let compatibility = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": 1,
                "capabilities": capabilities
            }
        }))
        .expect("required capabilities should be accepted");

        assert_eq!(compatibility.protocol_version, 1);
        assert!(compatibility
            .capabilities
            .iter()
            .any(|capability| capability == "futureCapability"));
    }

    #[test]
    fn rejects_unsupported_hello_protocol_version() {
        let capabilities = REQUIRED_CAPABILITIES;
        let err = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": 2,
                "capabilities": capabilities
            }
        }))
        .expect_err("unsupported protocol version should fail compatibility");

        assert_eq!(err.protocol_version, Some(2));
        assert!(err.message.contains("protocol version 2"));
        assert!(err.message.contains("protocol version 1"));
    }

    #[test]
    fn rejects_hello_missing_required_capabilities() {
        let err = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": 1,
                "capabilities": [
                    "listStepSpecs",
                    "openRecipe",
                    "getDocument",
                    "undo",
                    "redo",
                    "saveRecipe",
                    "validate",
                    "emitYaml"
                ]
            }
        }))
        .expect_err("missing required capabilities should fail compatibility");

        assert_eq!(err.protocol_version, Some(1));
        assert!(err.message.contains("applyRecipeCommand"));
        assert!(err.message.contains("saveRecipeAs"));
        assert!(err.message.contains("getRefIndex"));
        assert!(err.message.contains("setDocumentAuthoredRoot"));
        assert!(err.message.contains("describeConfiguration"));
        assert!(err.message.contains("ping"));
    }

    #[test]
    fn rejects_malformed_hello_response() {
        let err = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": "1",
                "capabilities": ["listStepSpecs"]
            }
        }))
        .expect_err("malformed hello response should fail compatibility");

        assert!(err.message.contains("malformed"));
    }

    #[test]
    fn rejects_hello_api_error_envelope() {
        let err = validate_hello_response(&json!({
            "id": "req-1",
            "ok": false,
            "error": {
                "code": "invalid_request",
                "message": "bad hello",
                "details": {}
            }
        }))
        .expect_err("hello ok:false should fail compatibility");

        assert!(err.message.contains("Backend hello failed"));
        assert!(err.message.contains("invalid_request"));
        assert!(err.message.contains("bad hello"));
    }

    #[test]
    fn status_does_not_start_sidecar() {
        let mut client = SidecarClient::new(SidecarRuntime::Dev);

        assert_eq!(
            client.status(),
            json!({
                "ok": true,
                "result": {
                    "running": false,
                    "pid": null,
                    "state": "notStarted",
                    "compatible": null,
                    "protocolVersion": null,
                    "capabilities": [],
                    "lastError": null,
                },
            })
        );
    }

    #[test]
    fn incompatible_sidecar_status_reports_compatibility_failure() {
        let mut client = SidecarClient::new(SidecarRuntime::Dev);
        client.mark_incompatible_for_test(
            "Backend is missing required capabilities: getRefIndex.".to_string(),
            Some(1),
            vec!["listStepSpecs".to_string()],
        );

        assert_eq!(
            client.status(),
            json!({
                "ok": true,
                "result": {
                    "running": false,
                    "pid": null,
                    "state": "incompatible",
                    "compatible": false,
                    "protocolVersion": 1,
                    "capabilities": ["listStepSpecs"],
                    "lastError": "Backend is missing required capabilities: getRefIndex.",
                },
            })
        );

        let err = client
            .request("listStepSpecs", None)
            .expect_err("incompatible sidecar should block normal requests");
        assert!(err.contains("missing required capabilities"));
    }

    #[test]
    fn exited_sidecar_is_not_restarted_for_requests() {
        let mut client = SidecarClient::new(SidecarRuntime::Dev);
        client.mark_exited_for_test();

        let err = client
            .request("listStepSpecs", None)
            .expect_err("exited sidecar should not auto-restart");

        assert!(err.contains("exited"));
    }

    #[test]
    fn restart_starts_fresh_sidecar_from_not_started() {
        let binary = rust_backend_binary_for_test();
        let mut client = SidecarClient::with_binary_path_for_test(binary);

        let restarted = client
            .restart()
            .expect("restart should start a fresh Rust sidecar");

        assert_eq!(restarted["ok"], true);
        assert_eq!(restarted["result"]["documentSessionsPreserved"], false);
        assert_eq!(restarted["result"]["status"]["running"], true);
        assert_eq!(restarted["result"]["status"]["state"], "running");
        assert!(restarted["result"]["status"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("ping")));
    }

    #[test]
    fn explicit_restart_replaces_running_process_without_preserving_document_sessions() {
        let binary = rust_backend_binary_for_test();
        let mut client = SidecarClient::with_binary_path_for_test(binary);
        let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");

        let opened = client
            .request(
                "openRecipe",
                Some(json!({"path": temp_recipe.path, "authoredRoot": null})),
            )
            .expect("openRecipe should start the sidecar and open a document");
        let old_document_id = opened["result"]["document"]["documentId"]
            .as_str()
            .unwrap()
            .to_string();
        let old_status = client.status();
        let old_pid = old_status["result"]["pid"]
            .as_u64()
            .expect("running sidecar should report a pid");

        let restarted = client
            .restart()
            .expect("restart should replace the running sidecar");
        let new_pid = restarted["result"]["status"]["pid"]
            .as_u64()
            .expect("restarted sidecar should report a pid");

        assert_ne!(new_pid, old_pid);
        assert_eq!(restarted["result"]["documentSessionsPreserved"], false);

        let fetched = client
            .request("getDocument", Some(json!({"documentId": old_document_id})))
            .expect("unknown document is an API envelope, not a transport failure");
        assert_eq!(fetched["ok"], false);
        assert_eq!(fetched["error"]["code"], "unknown_document");
    }

    #[test]
    fn explicit_restart_replaces_exited_state_without_enabling_normal_auto_restart() {
        let binary = rust_backend_binary_for_test();
        let mut client = SidecarClient::with_binary_path_for_test(binary);
        client.mark_exited_for_test();

        let normal_request_error = client
            .request("ping", None)
            .expect_err("normal requests must not auto-restart an exited sidecar");
        assert!(normal_request_error.contains("exited"));

        let restarted = client
            .restart()
            .expect("explicit restart should replace exited state");
        assert_eq!(restarted["ok"], true);
        assert_eq!(restarted["result"]["status"]["state"], "running");
        assert_eq!(restarted["result"]["documentSessionsPreserved"], false);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_restart_returns_envelope_for_new_incompatible_sidecar() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_temp_dir("incompatible-sidecar");
        let binary = dir.join("fake-incompatible-sidecar");
        fs::write(
            &binary,
            "#!/bin/sh\nIFS= read line\nprintf '%s\\n' '{\"id\":\"req-1\",\"ok\":true,\"result\":{\"protocolVersion\":1,\"capabilities\":[]}}'\n",
        )
        .expect("fake incompatible sidecar should be writable");
        let mut permissions = fs::metadata(&binary)
            .expect("fake incompatible sidecar metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions)
            .expect("fake incompatible sidecar should be executable");

        let mut client = SidecarClient::with_binary_path_for_test(binary);

        let restarted = client
            .restart()
            .expect("compatibility failure should still produce a restart envelope");

        assert_eq!(restarted["ok"], true);
        assert_eq!(restarted["result"]["documentSessionsPreserved"], false);
        assert_eq!(restarted["result"]["status"]["state"], "incompatible");
        assert_eq!(restarted["result"]["status"]["compatible"], false);
        assert!(restarted["result"]["status"]["lastError"]
            .as_str()
            .unwrap()
            .contains("missing required capabilities"));
    }

    #[test]
    fn actual_rust_sidecar_process_handles_editor_smoke_sequence() {
        let binary = rust_backend_binary_for_test();
        let mut client = SidecarClient::with_binary_path_for_test(binary);
        run_editor_sidecar_smoke_sequence(&mut client);
    }

    #[test]
    fn packaged_bundled_sidecar_process_handles_editor_smoke_sequence() {
        let binary = rust_backend_binary_for_test();
        let (bundled_dir, _packaged_binary) = copy_sidecar_to_packaged_bundled_dir(&binary);
        let mut client = SidecarClient::with_runtime_for_test(SidecarRuntime::Packaged {
            bundled_dir: Some(bundled_dir),
        });

        run_editor_sidecar_smoke_sequence(&mut client);
    }
}

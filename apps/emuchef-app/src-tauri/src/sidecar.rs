//! Lifecycle and compatibility negotiation for the canonical Rust sidecar.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

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
        matches!(
            self.status(),
            RuntimeStatusDto::Failed { error } if error.code == "runtime_session_lost"
        )
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
    next_request_id: u64,
    cache_root: PathBuf,
    generation: u64,
}

impl SidecarClient {
    fn new(cache_root: PathBuf) -> Self {
        Self {
            process: None,
            status: RuntimeStatusDto::Starting,
            next_request_id: 1,
            cache_root,
            generation: 0,
        }
    }

    fn initialize(&mut self) {
        self.stop();
        self.status = RuntimeStatusDto::Starting;
        match self.start_and_negotiate() {
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
            Err(StartFailure::Failed(message)) => {
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
        self.process = Some(start_process(&self.cache_root).map_err(StartFailure::Failed)?);
        let hello = self
            .raw_request("hello", json!({}))
            .map_err(StartFailure::Failed)?;
        let result = successful_result(&hello).map_err(StartFailure::Unsupported)?;
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
            .map_err(StartFailure::Failed)?;
        let result = successful_result(&negotiation).map_err(StartFailure::Unsupported)?;
        if result.get("compatible").and_then(Value::as_bool) != Some(true) {
            return Err(StartFailure::Unsupported(
                "Rust runtime is missing one or more required end-user product operations."
                    .to_string(),
            ));
        }
        Ok(protocol_version)
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
        let request = json!({ "id": request_id, "type": request_type, "payload": payload });
        serde_json::to_writer(process.writer(), &request)
            .map_err(|_| "Rust runtime request could not be encoded.".to_string())?;
        process
            .writer()
            .write_all(b"\n")
            .and_then(|_| process.writer().flush())
            .map_err(|_| "Rust runtime request could not be sent.".to_string())?;
        let mut line = String::new();
        process
            .reader()
            .read_line(&mut line)
            .map_err(|_| "Rust runtime response could not be read.".to_string())?;
        let response: Value = serde_json::from_str(line.trim_end())
            .map_err(|_| "Rust runtime response was not valid JSON.".to_string())?;
        if response.get("id").and_then(Value::as_str) != Some(&request_id) {
            return Err("Rust runtime response id did not match its request.".to_string());
        }
        Ok(response)
    }

    fn stop(&mut self) {
        if let Some(process) = self.process.take() {
            process.stop();
        }
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
        self.stop();
    }
}

enum StartFailure {
    Unsupported(String),
    Failed(String),
}

enum SidecarProcess {
    Child {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    },
    #[cfg(test)]
    Scripted {
        stdin: Vec<u8>,
        stdout: std::io::Cursor<Vec<u8>>,
        observer: std::sync::Arc<ScriptedProcessObserver>,
    },
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedProcessObserver {
    stopped: std::sync::atomic::AtomicBool,
    transport_accesses: std::sync::atomic::AtomicUsize,
}

impl SidecarProcess {
    fn writer(&mut self) -> &mut dyn Write {
        match self {
            Self::Child { stdin, .. } => stdin,
            #[cfg(test)]
            Self::Scripted {
                stdin, observer, ..
            } => {
                observer
                    .transport_accesses
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                stdin
            }
        }
    }

    fn reader(&mut self) -> &mut dyn BufRead {
        match self {
            Self::Child { stdout, .. } => stdout,
            #[cfg(test)]
            Self::Scripted {
                stdout, observer, ..
            } => {
                observer
                    .transport_accesses
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                stdout
            }
        }
    }

    fn stop(self) {
        match self {
            Self::Child { mut child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(test)]
            Self::Scripted { observer, .. } => observer
                .stopped
                .store(true, std::sync::atomic::Ordering::SeqCst),
        }
    }
}

fn start_process(cache_root: &Path) -> Result<SidecarProcess, String> {
    let (program, cwd) = resolve_sidecar()?;
    let mut command = sidecar_command(&program, cache_root);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "The bundled Rust runtime could not be started.".to_string())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Rust runtime stdin was unavailable.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Rust runtime stdout was unavailable.".to_string())?;
    Ok(SidecarProcess::Child {
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

fn successful_result(envelope: &Value) -> Result<&Value, String> {
    if envelope.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("Rust runtime rejected compatibility negotiation.".to_string());
    }
    envelope
        .get("result")
        .ok_or_else(|| "Rust runtime compatibility response omitted result data.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn scripted_process(response: Option<&[u8]>) -> (SidecarProcess, Arc<ScriptedProcessObserver>) {
        let observer = Arc::new(ScriptedProcessObserver::default());
        (
            SidecarProcess::Scripted {
                stdin: Vec::new(),
                stdout: std::io::Cursor::new(response.unwrap_or_default().to_vec()),
                observer: Arc::clone(&observer),
            },
            observer,
        )
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
        let (process, observer) = scripted_process(None);
        client.process = Some(process);

        let error = client.request("getExecution", json!({})).unwrap_err();
        let accesses_after_loss = observer.transport_accesses.load(Ordering::SeqCst);

        let error: Value = serde_json::from_str(&error).expect("session loss should be structured");
        assert_eq!(error["code"], "runtime_session_lost");
        assert!(matches!(client.status, RuntimeStatusDto::Failed { .. }));
        assert!(client.process.is_none());
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

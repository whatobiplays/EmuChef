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
        client.initialize();
    }

    pub fn status(&self) -> RuntimeStatusDto {
        self.inner
            .lock()
            .expect("sidecar mutex poisoned")
            .status
            .clone()
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
}

impl SidecarClient {
    fn new(cache_root: PathBuf) -> Self {
        Self {
            process: None,
            status: RuntimeStatusDto::Starting,
            next_request_id: 1,
            cache_root,
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
        if !matches!(self.status, RuntimeStatusDto::Ready { .. }) {
            return Err("Rust runtime is not ready.".to_string());
        }
        let envelope = self.raw_request(request_type, payload)?;
        if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
            envelope
                .get("result")
                .cloned()
                .ok_or_else(|| "Rust runtime response omitted result data.".to_string())
        } else {
            let error = envelope.get("error").cloned().unwrap_or_else(|| {
                json!({ "code": "runtime_request_failed", "message": "Rust runtime request failed." })
            });
            Err(serde_json::to_string(&error)
                .unwrap_or_else(|_| "Rust runtime request failed.".to_string()))
        }
    }

    fn raw_request(&mut self, request_type: &str, payload: Value) -> Result<Value, String> {
        let request_id = format!("app-{}", self.next_request_id);
        self.next_request_id += 1;
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| "Rust runtime process is not running.".to_string())?;
        let request = json!({ "id": request_id, "type": request_type, "payload": payload });
        serde_json::to_writer(&mut process.stdin, &request)
            .map_err(|_| "Rust runtime request could not be encoded.".to_string())?;
        process
            .stdin
            .write_all(b"\n")
            .and_then(|_| process.stdin.flush())
            .map_err(|_| "Rust runtime request could not be sent.".to_string())?;
        let mut line = String::new();
        process
            .stdout
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
        if let Some(mut process) = self.process.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }
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

struct SidecarProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
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
    Ok(SidecarProcess {
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
    fn startup_negotiation_payload_uses_the_real_backend_contract() {
        let payload = capability_negotiation_payload();
        assert_eq!(
            payload["requiredCapabilities"],
            serde_json::to_value(REQUIRED_CAPABILITIES).unwrap()
        );
        assert_eq!(
            payload["requiredCapabilities"].as_array().unwrap().len(),
            22
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
            22
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

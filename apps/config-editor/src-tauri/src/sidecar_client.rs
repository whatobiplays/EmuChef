use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::Mutex,
};

use serde_json::{json, Map, Value};

use crate::python_bridge::{configured_python_command, discover_repo_root, python_path_for_repo};

const SUPPORTED_PROTOCOL_VERSION: i64 = 1;
const REQUIRED_CAPABILITIES: &[&str] = &[
    "listStepSpecs",
    "openRecipe",
    "getDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "saveRecipe",
    "validate",
    "emitYaml",
    "getRefIndex",
];
const STDERR_EXCERPT_BYTES: u64 = 4096;

/// Shared Tauri state for the persistent Python sidecar.
///
/// Phase 3A keeps sidecar request handling deliberately serial. The mutex is
/// held for the full send-line/read-line exchange so requests cannot interleave
/// on the JSONL protocol.
pub struct SidecarState {
    client: Mutex<SidecarClient>,
}

impl Default for SidecarState {
    fn default() -> Self {
        Self {
            client: Mutex::new(SidecarClient::new()),
        }
    }
}

impl SidecarState {
    pub fn status(&self) -> Result<Value, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "Sidecar client lock is poisoned".to_string())?;
        Ok(client.status())
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
    pub fn new() -> Self {
        Self {
            next_request_id: 1,
            state: ProcessState::NotStarted,
        }
    }

    pub fn status(&mut self) -> Value {
        match &mut self.state {
            ProcessState::NotStarted => json!({
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
            }),
            ProcessState::Exited {
                last_error,
                compatibility,
            } => json!({
                "ok": true,
                "result": {
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
                },
            }),
            ProcessState::Incompatible {
                last_error,
                protocol_version,
                capabilities,
            } => json!({
                "ok": true,
                "result": {
                    "running": false,
                    "pid": null,
                    "state": "incompatible",
                    "compatible": false,
                    "protocolVersion": protocol_version,
                    "capabilities": capabilities,
                    "lastError": last_error,
                },
            }),
            ProcessState::Running(running) => match running.process.child.try_wait() {
                Ok(Some(_status)) => {
                    let compatibility = running.compatibility.clone();
                    self.state = ProcessState::Exited {
                        last_error: None,
                        compatibility: Some(compatibility.clone()),
                    };
                    json!({
                        "ok": true,
                        "result": {
                            "running": false,
                            "pid": null,
                            "state": "exited",
                            "compatible": true,
                            "protocolVersion": compatibility.protocol_version,
                            "capabilities": compatibility.capabilities,
                            "lastError": null,
                        },
                    })
                }
                Ok(None) => json!({
                    "ok": true,
                    "result": {
                        "running": true,
                        "pid": running.process.child.id(),
                        "state": "running",
                        "compatible": true,
                        "protocolVersion": running.compatibility.protocol_version,
                        "capabilities": running.compatibility.capabilities.clone(),
                        "lastError": null,
                    },
                }),
                Err(err) => json!({
                    "ok": true,
                    "result": {
                        "running": false,
                        "pid": null,
                        "state": "error",
                        "compatible": true,
                        "protocolVersion": running.compatibility.protocol_version,
                        "capabilities": running.compatibility.capabilities.clone(),
                        "lastError": format!("Failed to inspect backend sidecar process: {err}"),
                    },
                }),
            },
        }
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
                let mut process = start_sidecar()?;
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
                        "Backend sidecar exited unexpectedly with status {status}; restart the Tauri app to create a new sidecar session."
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
                "Backend sidecar has exited and is not restarted automatically; restart the Tauri app to create a new session."
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
                python_start_guidance()
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
            let _ = running.process.child.kill();
            let _ = running.process.child.wait();
        }
    }
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
            "Backend sidecar exited before writing a response; restart the Tauri app to create a new sidecar session."
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
    let _ = process.child.kill();
    let _ = process.child.wait();
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

fn start_sidecar() -> Result<SidecarProcess, String> {
    let python = configured_python_command();
    let repo_root = discover_repo_root();

    let mut command = Command::new(&python);
    command
        .arg("-m")
        .arg("emuchef_editor.api.server")
        .arg("--sidecar")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(root) = repo_root.as_ref() {
        command.current_dir(root);
        command.env("PYTHONPATH", python_path_for_repo(root)?);
    }

    let mut child = command.spawn().map_err(|err| {
        format!(
            "Failed to start Python sidecar with '{python}': {err}. {}",
            python_start_guidance()
        )
    })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture Python sidecar stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture Python sidecar stdout".to_string())?;
    let stderr = child.stderr.take();

    Ok(SidecarProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        stderr,
    })
}

fn python_start_guidance() -> &'static str {
    "Set EMUCHEF_PYTHON to the Python interpreter to use. That Python must be able to import the local emuchef_editor package, usually through the repo src/ directory during development."
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn validates_successful_hello_with_extra_capabilities() {
        let compatibility = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": 1,
                "capabilities": [
                    "listStepSpecs",
                    "openRecipe",
                    "getDocument",
                    "applyRecipeCommand",
                    "undo",
                    "redo",
                    "saveRecipe",
                    "validate",
                    "emitYaml",
                    "getRefIndex",
                    "futureCapability"
                ]
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
        let err = validate_hello_response(&json!({
            "id": "req-1",
            "ok": true,
            "result": {
                "protocolVersion": 2,
                "capabilities": [
                    "listStepSpecs",
                    "openRecipe",
                    "getDocument",
                    "applyRecipeCommand",
                    "undo",
                    "redo",
                    "saveRecipe",
                    "validate",
                    "emitYaml",
                    "getRefIndex"
                ]
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
        assert!(err.message.contains("getRefIndex"));
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
        let mut client = SidecarClient::new();

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
        let mut client = SidecarClient::new();
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
        let mut client = SidecarClient::new();
        client.mark_exited_for_test();

        let err = client
            .request("listStepSpecs", None)
            .expect_err("exited sidecar should not auto-restart");

        assert!(err.contains("exited"));
    }
}

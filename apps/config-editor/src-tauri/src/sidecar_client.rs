use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Mutex,
};

use serde_json::{json, Map, Value};

use crate::python_bridge::{configured_python_command, discover_repo_root, python_path_for_repo};

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
    Exited,
}

struct RunningSidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
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
                },
            }),
            ProcessState::Exited => json!({
                "ok": true,
                "result": {
                    "running": false,
                    "pid": null,
                    "state": "exited",
                },
            }),
            ProcessState::Running(running) => match running.child.try_wait() {
                Ok(Some(_status)) => {
                    self.state = ProcessState::Exited;
                    json!({
                        "ok": true,
                        "result": {
                            "running": false,
                            "pid": null,
                            "state": "exited",
                        },
                    })
                }
                Ok(None) => json!({
                    "ok": true,
                    "result": {
                        "running": true,
                        "pid": running.child.id(),
                    },
                }),
                Err(err) => json!({
                    "ok": true,
                    "result": {
                        "running": false,
                        "pid": null,
                        "state": "error",
                        "message": format!("Failed to inspect sidecar process: {err}"),
                    },
                }),
            },
        }
    }

    pub fn request(&mut self, request_type: &str, payload: Option<Value>) -> Result<Value, String> {
        self.ensure_running()?;

        let request_id = self.allocate_request_id();
        let request = build_sidecar_request(&request_id, request_type, payload);
        let request_json = serde_json::to_string(&request)
            .map_err(|err| format!("Failed to serialize sidecar request: {err}"))?;

        let mut response_line = String::new();
        let transport_result = match &mut self.state {
            ProcessState::Running(running) => running
                .stdin
                .write_all(request_json.as_bytes())
                .and_then(|_| running.stdin.write_all(b"\n"))
                .and_then(|_| running.stdin.flush())
                .map_err(|err| format!("Failed to write to Python sidecar stdin: {err}"))
                .and_then(|_| {
                    running
                        .stdout
                        .read_line(&mut response_line)
                        .map_err(|err| format!("Failed to read Python sidecar stdout: {err}"))
                }),
            ProcessState::NotStarted | ProcessState::Exited => {
                unreachable!("ensure_running failed to start sidecar")
            }
        };

        match transport_result {
            Ok(0) => {
                self.state = ProcessState::Exited;
                Err("Python sidecar exited before writing a response; restart the Tauri app to create a new sidecar session.".to_string())
            }
            Ok(_) => parse_sidecar_response_line(&response_line, &request_id),
            Err(err) => {
                self.state = ProcessState::Exited;
                Err(err)
            }
        }
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        match &mut self.state {
            ProcessState::NotStarted => {
                self.state = ProcessState::Running(start_sidecar()?);
                Ok(())
            }
            ProcessState::Running(running) => match running.child.try_wait() {
                Ok(Some(status)) => {
                    self.state = ProcessState::Exited;
                    Err(format!(
                        "Python sidecar exited unexpectedly with status {status}; restart the Tauri app to create a new sidecar session."
                    ))
                }
                Ok(None) => Ok(()),
                Err(err) => Err(format!("Failed to inspect Python sidecar process: {err}")),
            },
            ProcessState::Exited => Err(
                "Python sidecar has exited and Phase 3A does not restart it automatically; restart the Tauri app to create a new session."
                    .to_string(),
            ),
        }
    }

    fn allocate_request_id(&mut self) -> String {
        let request_id = format!("req-{}", self.next_request_id);
        self.next_request_id += 1;
        request_id
    }

    #[cfg(test)]
    fn mark_exited_for_test(&mut self) {
        self.state = ProcessState::Exited;
    }
}

impl Drop for SidecarClient {
    fn drop(&mut self) {
        if let ProcessState::Running(running) = &mut self.state {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
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
        .map_err(|err| format!("Python sidecar stdout line was not valid JSON: {err}"))?;
    if !response.is_object() {
        return Err("Python sidecar response must be a JSON object".to_string());
    }

    let response_id = response
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Python sidecar response is missing string id".to_string())?;
    if response_id != expected_request_id {
        return Err(format!(
            "Python sidecar response id mismatch: expected {expected_request_id}, received {response_id}"
        ));
    }

    match response.get("ok").and_then(Value::as_bool) {
        Some(true) if response.get("result").is_some() => Ok(response),
        Some(false) if response.get("error").is_some() => Ok(response),
        Some(_) => Err("Python sidecar response is missing result/error data".to_string()),
        None => Err("Python sidecar response is missing boolean ok field".to_string()),
    }
}

fn start_sidecar() -> Result<RunningSidecar, String> {
    let python = configured_python_command();
    let repo_root = discover_repo_root();

    let mut command = Command::new(&python);
    command
        .arg("-m")
        .arg("emuchef_editor.api.server")
        .arg("--sidecar")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if let Some(root) = repo_root.as_ref() {
        command.current_dir(root);
        command.env("PYTHONPATH", python_path_for_repo(root)?);
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("Failed to start Python sidecar with '{python}': {err}. {}", python_start_guidance()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to capture Python sidecar stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture Python sidecar stdout".to_string())?;

    Ok(RunningSidecar {
        child,
        stdin,
        stdout: BufReader::new(stdout),
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
    fn status_does_not_start_sidecar() {
        let mut client = SidecarClient::new();

        assert_eq!(
            client.status(),
            json!({
                "ok": true,
                "result": {
                    "running": false,
                    "pid": null,
                },
            })
        );
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

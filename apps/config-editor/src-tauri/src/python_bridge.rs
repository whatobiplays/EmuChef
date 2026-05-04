use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Map, Value};

const PYPROJECT_FILE: &str = "pyproject.toml";
const API_SERVER_FILE: &str = "src/emuchef_editor/api/server.py";

/// Build the stable one-shot Python API request envelope.
///
/// The Python editor API intentionally keeps camelCase request names and payload
/// fields. Rust command names stay snake_case for Tauri, but this bridge must
/// not translate the Python API contract.
pub fn build_request(request_type: &str, payload: Option<Value>) -> Value {
    let mut request = Map::new();
    request.insert("type".to_string(), Value::String(request_type.to_string()));
    if let Some(payload) = payload {
        request.insert("payload".to_string(), payload);
    }
    Value::Object(request)
}

pub fn run_request(request: Value) -> Result<Value, String> {
    let request_json = serde_json::to_string(&request)
        .map_err(|err| format!("Failed to serialize Python API request: {err}"))?;
    let python = env::var("EMUCHEF_PYTHON").unwrap_or_else(|_| "python".to_string());
    let repo_root = discover_repo_root();

    let mut command = Command::new(&python);
    command
        .arg("-m")
        .arg("emuchef_editor.api.server")
        .arg(request_json);

    if let Some(root) = repo_root.as_ref() {
        command.current_dir(root);
        command.env("PYTHONPATH", python_path_for_repo(root)?);
    }

    let output = command
        .output()
        .map_err(|err| format!("Failed to start Python API process with '{python}': {err}"))?;

    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("Python API stdout was not valid UTF-8: {err}"))?;
    match parse_stdout_envelope(&stdout) {
        Ok(envelope) => Ok(envelope),
        Err(parse_err) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let status = output.status;
            let mut message =
                format!("Python API process did not return a usable JSON envelope: {parse_err}");
            if !status.success() {
                message.push_str(&format!("; exit status: {status}"));
            }
            if !stderr.is_empty() {
                message.push_str(&format!("; stderr: {stderr}"));
            }
            Err(message)
        }
    }
}

pub fn parse_stdout_envelope(stdout: &str) -> Result<Value, String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err("stdout was empty".to_string());
    }

    let envelope: Value =
        serde_json::from_str(trimmed).map_err(|err| format!("stdout was not valid JSON: {err}"))?;
    if !envelope.is_object() {
        return Err("stdout JSON envelope must be an object".to_string());
    }
    match envelope.get("ok").and_then(Value::as_bool) {
        Some(true) if envelope.get("result").is_some() => Ok(envelope),
        Some(false) if envelope.get("error").is_some() => Ok(envelope),
        Some(_) => Err("stdout JSON envelope is missing result/error data".to_string()),
        None => Err("stdout JSON envelope is missing boolean ok field".to_string()),
    }
}

fn discover_repo_root() -> Option<PathBuf> {
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
        if candidate.join(PYPROJECT_FILE).is_file() && candidate.join(API_SERVER_FILE).is_file() {
            Some(candidate.to_path_buf())
        } else {
            None
        }
    })
}

fn python_path_for_repo(repo_root: &Path) -> Result<std::ffi::OsString, String> {
    let mut paths = vec![repo_root.join("src")];
    if let Some(existing) = env::var_os("PYTHONPATH") {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths).map_err(|err| format!("Failed to construct PYTHONPATH: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_open_recipe_request_with_camel_case_python_api_contract() {
        let request = build_request(
            "openRecipe",
            Some(json!({
                "path": "/tmp/example.yaml",
                "authoredRoot": null,
            })),
        );

        assert_eq!(
            request,
            json!({
                "type": "openRecipe",
                "payload": {
                    "path": "/tmp/example.yaml",
                    "authoredRoot": null,
                },
            })
        );
    }

    #[test]
    fn parses_api_failure_envelope_as_successful_bridge_output() {
        let envelope = parse_stdout_envelope(
            r#"
            {"ok": false, "error": {"code": "load_failed", "message": "bad recipe", "details": {"path": "missing.yaml"}}}
            "#,
        )
        .expect("valid API envelope should parse");

        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["message"], "bad recipe");
    }

    #[test]
    fn rejects_stdout_with_non_json_log_lines() {
        let err = parse_stdout_envelope("log line\n{\"ok\": true, \"result\": {}}")
            .expect_err("stdout log lines should be treated as transport errors");

        assert!(err.contains("valid JSON"));
    }
}

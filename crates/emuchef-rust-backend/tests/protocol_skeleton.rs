use std::io::Write;
use std::process::{Command, Stdio};

use emuchef_rust_backend::{jsonl, run_with_args_and_input};
use serde_json::{json, Value};

fn parse_stdout_json(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one stdout JSON line: {stdout:?}"
    );
    serde_json::from_str(lines[0]).expect("stdout line should be valid JSON")
}

fn one_shot_response(request: &str) -> Value {
    let output = run_with_args_and_input(&[request.to_string()], "");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    parse_stdout_json(&output.stdout)
}

fn one_shot_args_response(args: &[&str]) -> Value {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let output = run_with_args_and_input(&args, "");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    parse_stdout_json(&output.stdout)
}

fn sidecar_responses(input: &str) -> Vec<Value> {
    jsonl::process_jsonl(input)
        .lines()
        .map(|line| serde_json::from_str(line).expect("sidecar output line should be valid JSON"))
        .collect()
}

fn assert_invalid_request(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
    assert_eq!(response["error"]["details"], json!({}));
}

fn assert_hello_result(result: &Value) {
    assert_eq!(result["protocolVersion"], 1);
    assert_eq!(result["capabilities"], json!([]));
    assert!(!result["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("hello")));
    assert!(result.get("implementation").is_none());
    assert!(result.get("implementationVersion").is_none());
}

fn assert_hello_response(response: &Value) {
    assert_eq!(response["ok"], true);
    assert_hello_result(&response["result"]);
}

#[test]
fn one_shot_hello_accepts_omitted_payload() {
    assert_hello_response(&one_shot_response(r#"{"type":"hello"}"#));
}

#[test]
fn one_shot_hello_accepts_empty_object_payload() {
    assert_hello_response(&one_shot_response(r#"{"type":"hello","payload":{}}"#));
}

#[test]
fn one_shot_hello_accepts_unknown_object_payload_keys() {
    assert_hello_response(&one_shot_response(
        r#"{"type":"hello","payload":{"ignored":true}}"#,
    ));
}

#[test]
fn one_shot_hello_rejects_non_object_payload() {
    assert_invalid_request(&one_shot_response(r#"{"type":"hello","payload":[]}"#));
}

#[test]
fn one_shot_unknown_request_returns_invalid_request() {
    assert_invalid_request(&one_shot_response(r#"{"type":"unknown"}"#));
}

#[test]
fn one_shot_missing_request_type_returns_invalid_request() {
    assert_invalid_request(&one_shot_response(r#"{"payload":{}}"#));
}

#[test]
fn one_shot_non_string_request_type_returns_invalid_request() {
    assert_invalid_request(&one_shot_response(r#"{"type":123}"#));
}

#[test]
fn one_shot_empty_request_type_returns_invalid_request() {
    assert_invalid_request(&one_shot_response(r#"{"type":""}"#));
}

#[test]
fn one_shot_malformed_json_returns_invalid_request() {
    assert_invalid_request(&one_shot_response("{not-json"));
}

#[test]
fn one_shot_missing_argument_returns_invalid_request() {
    assert_invalid_request(&one_shot_args_response(&[]));
}

#[test]
fn one_shot_extra_arguments_return_invalid_request() {
    assert_invalid_request(&one_shot_args_response(&[
        r#"{"type":"hello"}"#,
        r#"{"type":"hello"}"#,
    ]));
}

#[test]
fn sidecar_hello_echoes_id() {
    let responses = sidecar_responses(r#"{"id":"hello-1","type":"hello"}"#);
    assert_eq!(responses[0]["id"], "hello-1");
    assert_hello_response(&responses[0]);
}

#[test]
fn sidecar_hello_accepts_omitted_payload() {
    assert_hello_response(&sidecar_responses(r#"{"id":"hello-1","type":"hello"}"#)[0]);
}

#[test]
fn sidecar_hello_accepts_empty_object_payload() {
    assert_hello_response(&sidecar_responses(r#"{"id":"hello-1","type":"hello","payload":{}}"#)[0]);
}

#[test]
fn sidecar_hello_accepts_unknown_object_payload_keys() {
    assert_hello_response(
        &sidecar_responses(r#"{"id":"hello-1","type":"hello","payload":{"ignored":true}}"#)[0],
    );
}

#[test]
fn sidecar_hello_with_non_object_payload_returns_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"hello-bad","type":"hello","payload":[]}"#)[0];
    assert_eq!(response["id"], "hello-bad");
    assert_invalid_request(response);
}

#[test]
fn sidecar_missing_id_returns_null_id_invalid_request() {
    let response = &sidecar_responses(r#"{"type":"hello"}"#)[0];
    assert_eq!(response["id"], Value::Null);
    assert_invalid_request(response);
}

#[test]
fn sidecar_non_string_id_returns_null_id_invalid_request() {
    let response = &sidecar_responses(r#"{"id":123,"type":"hello"}"#)[0];
    assert_eq!(response["id"], Value::Null);
    assert_invalid_request(response);
}

#[test]
fn sidecar_empty_id_returns_null_id_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"","type":"hello"}"#)[0];
    assert_eq!(response["id"], Value::Null);
    assert_invalid_request(response);
}

#[test]
fn sidecar_missing_request_type_returns_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"missing-type","payload":{}}"#)[0];
    assert_eq!(response["id"], "missing-type");
    assert_invalid_request(response);
}

#[test]
fn sidecar_non_string_request_type_returns_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"bad-type","type":123}"#)[0];
    assert_eq!(response["id"], "bad-type");
    assert_invalid_request(response);
}

#[test]
fn sidecar_empty_request_type_returns_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"empty-type","type":""}"#)[0];
    assert_eq!(response["id"], "empty-type");
    assert_invalid_request(response);
}

#[test]
fn sidecar_unknown_request_returns_invalid_request() {
    let response = &sidecar_responses(r#"{"id":"unknown","type":"unknown"}"#)[0];
    assert_eq!(response["id"], "unknown");
    assert_invalid_request(response);
}

#[test]
fn sidecar_malformed_json_returns_null_id_invalid_request() {
    let response = &sidecar_responses("{not-json")[0];
    assert_eq!(response["id"], Value::Null);
    assert_invalid_request(response);
}

#[test]
fn sidecar_blank_line_returns_null_id_invalid_request() {
    let response = &sidecar_responses("\n")[0];
    assert_eq!(response["id"], Value::Null);
    assert_invalid_request(response);
}

#[test]
fn sidecar_continues_after_invalid_request_and_handles_next_hello() {
    let responses = sidecar_responses(
        "{\"id\":\"bad\",\"type\":\"unknown\"}\n{\"id\":\"hello-2\",\"type\":\"hello\"}\n",
    );
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], "bad");
    assert_invalid_request(&responses[0]);
    assert_eq!(responses[1]["id"], "hello-2");
    assert_hello_response(&responses[1]);
}

#[test]
fn mixed_sidecar_plus_extra_args_is_process_usage_error() {
    let output = run_with_args_and_input(
        &["--sidecar".to_string(), r#"{"type":"hello"}"#.to_string()],
        "",
    );
    assert_ne!(output.exit_code, 0);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.contains("usage"));
}

#[test]
fn process_one_shot_stdout_contains_exactly_one_json_envelope() {
    let output = Command::new(env!("CARGO_BIN_EXE_emuchef-rust-backend"))
        .arg(r#"{"type":"hello"}"#)
        .output()
        .expect("process should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_hello_response(&parse_stdout_json(
        &String::from_utf8(output.stdout).unwrap(),
    ));
}

#[test]
fn process_jsonl_sidecar_responds_line_by_line_with_json_only_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_emuchef-rust-backend"))
        .arg("--sidecar")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sidecar should start");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        writeln!(stdin, r#"{{"id":"hello-1","type":"hello","payload":{{}}}}"#).unwrap();
        writeln!(stdin, r#"{{"id":"hello-2","type":"hello"}}"#).unwrap();
    }

    let output = child
        .wait_with_output()
        .expect("sidecar should exit on EOF");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout should contain JSONL only"))
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], "hello-1");
    assert_eq!(responses[1]["id"], "hello-2");
    assert_hello_response(&responses[0]);
    assert_hello_response(&responses[1]);
}

#[test]
fn process_mixed_sidecar_extra_arg_writes_no_stdout_and_non_zero_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_emuchef-rust-backend"))
        .arg("--sidecar")
        .arg(r#"{"type":"hello"}"#)
        .output()
        .expect("process should run");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr).unwrap().contains("usage"));
}

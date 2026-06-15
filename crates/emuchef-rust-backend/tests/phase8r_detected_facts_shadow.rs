use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should resolve from crate manifest dir")
}

fn repo_authored_root() -> PathBuf {
    repo_root().join("authored")
}

fn run_shadow(device_plan: &str, extra_args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_emuchef-plan-shadow"));
    command
        .arg("--authored-root")
        .arg(repo_authored_root())
        .arg("--device-plan")
        .arg(device_plan);
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().expect("shadow planner process should run")
}

fn stdout_json(output: &Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout should be valid JSON: {error}\nstdout:\n{stdout}"))
}

fn write_fixture(temp: &TempDir, name: &str, payload: &str) -> String {
    let path = temp.path().join(name);
    fs::write(&path, payload).expect("fixture should be written");
    path.to_string_lossy().into_owned()
}

fn matching_fixture_json() -> &'static str {
    r#"{
  "serial": "P8R-MATCH",
  "manufacturer": "AYANEO",
  "brand": "AYANEO",
  "model": "AYANEO Pocket S mini",
  "android_version": 13,
  "android_api_level": 33,
  "device_tags": ["detected_handheld"]
}"#
}

fn mismatching_fixture_json() -> &'static str {
    r#"{
  "serial": "P8R-MISMATCH",
  "manufacturer": "Valve",
  "brand": "Valve",
  "model": "Steam Deck",
  "android_version": 12,
  "android_api_level": 32,
  "device_tags": ["detected_mismatch"]
}"#
}

#[test]
fn detected_facts_shadow_matching_profile_emits_success_without_mismatch_warning() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "matching.json", matching_fixture_json());
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let result = stdout_json(&output);
    assert_eq!(result["status"], "success");
    assert_eq!(result["warnings"], json!([]));
    assert_eq!(
        result["execution_plan"]["device_context"],
        json!({
            "manufacturer": "AYANEO",
            "model": "AYANEO Pocket S mini",
            "android_version": 13,
            "android_api_level": 33,
            "device_tags": ["detected_handheld"],
        })
    );
}

#[test]
fn detected_facts_shadow_mismatching_profile_emits_warning_result_with_shadow_exit_mapping() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "mismatching.json", mismatching_fixture_json());
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let result = stdout_json(&output);
    let warnings = result["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert_eq!(result["status"], "warning");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "device_profile_mismatch");
    assert_eq!(
        result["execution_plan"]["device_context"]["device_tags"],
        json!(["detected_mismatch"])
    );
}

#[test]
fn detected_facts_shadow_preserves_bind_behavior() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "matching.json", matching_fixture_json());
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &[
            "--detected-facts-json",
            &fixture,
            "--bind",
            "app.retroarch.provision/retroarch_cfg=/tmp/p8r-retroarch.cfg",
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = stdout_json(&output);
    assert!(result["execution_plan"]["inputs"]
        .as_array()
        .expect("inputs should be an array")
        .iter()
        .any(|input| {
            input["id"] == "app.retroarch.provision/retroarch_cfg"
                && input["value"]["value"] == "/tmp/p8r-retroarch.cfg"
        }));
}

#[test]
fn detected_facts_shadow_allows_explicit_context_to_override_emitted_context_only() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "mismatching.json", mismatching_fixture_json());
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &[
            "--detected-facts-json",
            &fixture,
            "--manufacturer",
            "AYANEO",
            "--model",
            "AYANEO Pocket S mini",
            "--android-version",
            "13",
            "--device-tag",
            "explicit_handheld",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let result = stdout_json(&output);
    assert_eq!(result["status"], "warning");
    assert_eq!(result["warnings"][0]["code"], "device_profile_mismatch");
    assert_eq!(
        result["warnings"][0]["details"]["manufacturer"],
        json!("Valve")
    );
    assert_eq!(
        result["execution_plan"]["device_context"],
        json!({
            "manufacturer": "AYANEO",
            "model": "AYANEO Pocket S mini",
            "android_version": 13,
            "android_api_level": 32,
            "device_tags": ["explicit_handheld"],
        })
    );
}

#[test]
fn detected_facts_shadow_missing_file_writes_stable_process_error_without_stdout() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = temp.path().join("missing.json");
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture.to_string_lossy()],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Error: detected_facts_fixture_read_failed"));
    assert!(stderr.contains("missing.json"));
    assert!(!stderr.contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn detected_facts_shadow_invalid_json_writes_stable_process_error_without_stdout() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "invalid.json", "{not json");
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Error: detected_facts_fixture_invalid"));
    assert!(stderr.contains("invalid.json"));
    assert!(!stderr.contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn detected_facts_shadow_invalid_field_type_writes_stable_process_error_without_stdout() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(
        &temp,
        "invalid-field-type.json",
        r#"{"android_version": "13"}"#,
    );
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Error: detected_facts_fixture_invalid"));
    assert!(stderr.contains("invalid-field-type.json"));
    assert!(!stderr.contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn detected_facts_shadow_unknown_field_writes_stable_process_error_without_stdout() {
    let temp = TempDir::new().expect("temp dir should be created");
    let fixture = write_fixture(&temp, "unknown-field.json", r#"{"codename": "odin"}"#);
    let output = run_shadow(
        "ayaneo.pocket_s_mini.base",
        &["--detected-facts-json", &fixture],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Error: detected_facts_fixture_invalid"));
    assert!(stderr.contains("unknown-field.json"));
    assert!(stderr.contains("unknown field"));
    assert!(!stderr.contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn detected_facts_shadow_source_has_no_live_behavior_dependencies() {
    let source = include_str!("../src/plan_shadow.rs");
    let code_without_line_comments = source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("//!"))
        })
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [
        "std::process",
        "Command::new",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "ureq",
        "hyper",
        "adb ",
        "adb.exe",
    ] {
        assert!(
            !code_without_line_comments.contains(forbidden),
            "shadow fixture harness must not contain live behavior marker {forbidden:?}"
        );
    }
}

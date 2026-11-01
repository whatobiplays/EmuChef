//! Protocol-level contract for passive device storage volume discovery.
//!
//! These tests exercise the request dispatch boundary only. They never require
//! an Android device because the selected ADB executable does not exist, so the
//! inventory command fails before any device command can run.

use serde_json::{json, Value};

const MISSING_ADB: &str = "/nonexistent/emuchef-t04-adb";

#[test]
fn one_shot_dispatch_reaches_device_volume_discovery() {
    let response = emuchef_rust_backend::request::handle_one_shot_value(json!({
        "type": "listDeviceVolumes",
        "payload": { "adbPath": MISSING_ADB, "serial": "T04-SERIAL" },
    }));

    assert_dispatch_reached_the_handler(&response);
}

#[test]
fn sidecar_dispatch_reaches_device_volume_discovery_and_preserves_the_request_id() {
    let response = emuchef_rust_backend::jsonl::process_jsonl(&format!(
        "{}\n",
        json!({
            "id": "t04-volume-request",
            "type": "listDeviceVolumes",
            "payload": { "adbPath": MISSING_ADB, "serial": "T04-SERIAL" },
        })
    ));
    let response: Value = serde_json::from_str(response.trim()).expect("valid JSONL response");

    assert_eq!(response["id"], "t04-volume-request");
    assert_dispatch_reached_the_handler(&response);
}

#[test]
fn device_volume_discovery_requires_an_adb_path_and_serial() {
    let missing_serial = emuchef_rust_backend::request::handle_one_shot_value(json!({
        "type": "listDeviceVolumes",
        "payload": { "adbPath": MISSING_ADB },
    }));
    assert_eq!(missing_serial["ok"], false);
    assert_eq!(missing_serial["error"]["code"], "invalid_request");
    assert!(!missing_serial.to_string().contains("Unknown request type"));

    let missing_adb = emuchef_rust_backend::request::handle_one_shot_value(json!({
        "type": "listDeviceVolumes",
        "payload": { "serial": "T04-SERIAL" },
    }));
    assert_eq!(missing_adb["ok"], false);
    assert_eq!(missing_adb["error"]["code"], "invalid_request");
}

#[test]
fn device_volume_discovery_errors_never_echo_the_configured_adb_path() {
    let response = emuchef_rust_backend::request::handle_one_shot_value(json!({
        "type": "listDeviceVolumes",
        "payload": { "adbPath": MISSING_ADB, "serial": "T04-SERIAL" },
    }));

    let rendered = response.to_string();
    assert!(
        !rendered.contains(MISSING_ADB),
        "rendered error: {rendered}"
    );
    assert!(
        !rendered.contains("T04-SERIAL"),
        "rendered error: {rendered}"
    );
}

fn assert_dispatch_reached_the_handler(response: &Value) {
    assert_eq!(response["ok"], false, "response: {response}");
    assert_eq!(
        response["error"]["code"], "command_failed",
        "response: {response}"
    );
    assert_eq!(
        response["error"]["message"], "ADB device inventory could not be started.",
        "response: {response}"
    );
    assert_eq!(
        response["error"]["details"]["reason"], "adb_inventory_unavailable",
        "response: {response}"
    );
}

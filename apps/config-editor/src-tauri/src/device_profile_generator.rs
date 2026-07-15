//! Trusted Config Editor boundary for connected-device profile generation.
//!
//! Exact ADB serials and native authored-root paths live only in this module's
//! process-memory registry. React receives session-scoped opaque handles, safe
//! device facts, canonical draft data, and relative display metadata.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::sidecar_client::SidecarState;

const ADB_EXECUTABLE: &str = "adb";
const SAFE_ROOT_LABEL: &str = "Selected authored root";
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct GeneratorRegistry {
    next_handle: u64,
    sessions: HashMap<String, GeneratorSession>,
}

#[derive(Default)]
struct GeneratorSession {
    devices: HashMap<String, TrustedDevice>,
    roots: HashMap<String, TrustedRoot>,
}

struct TrustedDevice {
    serial: String,
    facts: Option<Value>,
}

#[derive(Clone)]
struct TrustedRoot {
    root: PathBuf,
    profile_directory: PathBuf,
}

/// Process-memory state for one-window device-profile generator sessions.
#[derive(Default)]
pub struct DeviceProfileGeneratorState {
    registry: Mutex<GeneratorRegistry>,
}

impl DeviceProfileGeneratorState {
    /// Invalidate every opaque handle, including after a sidecar restart.
    pub fn clear(&self) -> Result<(), String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Device-profile generator state is unavailable.".to_string())?;
        registry.sessions.clear();
        Ok(())
    }
}

impl GeneratorRegistry {
    fn allocate_handle(&mut self, prefix: &str) -> String {
        self.next_handle = self.next_handle.saturating_add(1);
        format!("{prefix}-{}", self.next_handle)
    }

    fn begin_session(&mut self) -> String {
        let handle = self.allocate_handle("generator");
        self.sessions
            .insert(handle.clone(), GeneratorSession::default());
        handle
    }

    fn session_mut(&mut self, handle: &str) -> Result<&mut GeneratorSession, Value> {
        self.sessions
            .get_mut(handle)
            .ok_or_else(expired_session_error)
    }

    fn session(&self, handle: &str) -> Result<&GeneratorSession, Value> {
        self.sessions.get(handle).ok_or_else(expired_session_error)
    }
}

#[tauri::command]
/// Start an isolated generator session and return its opaque handle.
pub fn begin_device_profile_generator(
    state: State<'_, DeviceProfileGeneratorState>,
) -> Result<Value, String> {
    let mut registry = lock_registry(&state)?;
    let session_handle = registry.begin_session();
    Ok(success(json!({ "sessionHandle": session_handle })))
}

#[tauri::command]
/// Select and retain one authored root without exposing its native path.
pub async fn choose_device_profile_authored_root(
    app: AppHandle,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    {
        let registry = lock_registry(&state)?;
        if let Err(error) = registry.session(&session_handle) {
            return Ok(error);
        }
    }
    let Some(selection) = app.dialog().file().blocking_pick_folder() else {
        return Ok(success(json!({ "cancelled": true })));
    };
    let selected_path = match selection.into_path() {
        Ok(path) => path,
        Err(_) => return Ok(invalid_root_error()),
    };
    let trusted_root = match validate_authored_root(&selected_path) {
        Ok(root) => root,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let root_handle = registry.allocate_handle("root");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.roots.insert(root_handle.clone(), trusted_root);
    Ok(success(json!({
        "cancelled": false,
        "rootHandle": root_handle,
        "label": SAFE_ROOT_LABEL,
    })))
}

#[tauri::command]
/// Refresh ADB inventory and replace exact serials with session-scoped handles.
pub fn list_device_profile_generator_devices(
    sidecar: State<'_, SidecarState>,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    {
        let registry = lock_registry(&state)?;
        if let Err(error) = registry.session(&session_handle) {
            return Ok(error);
        }
    }
    let response = request_sidecar(
        &sidecar,
        "listAdbDevices",
        Some(json!({ "adbPath": ADB_EXECUTABLE })),
    )?;
    let result = match success_result(&response) {
        Some(result) => result,
        None => return Ok(response),
    };
    let devices = match result.get("devices").and_then(Value::as_array) {
        Some(devices) => devices,
        None => return Ok(generator_protocol_error()),
    };

    let mut registry = lock_registry(&state)?;
    let mut projected = Vec::new();
    let mut trusted = Vec::new();
    for device in devices {
        let Some(serial) = device.get("serial").and_then(Value::as_str) else {
            continue;
        };
        let handle = registry.allocate_handle("device");
        projected.push(json!({
            "deviceHandle": handle,
            "state": device.get("state").cloned().unwrap_or(Value::Null),
            "model": device.get("model").cloned().unwrap_or(Value::Null),
        }));
        trusted.push((
            handle,
            TrustedDevice {
                serial: serial.to_string(),
                facts: None,
            },
        ));
    }
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.devices.clear();
    session.devices.extend(trusted);
    Ok(success(json!({
        "state": result.get("state").cloned().unwrap_or(Value::Null),
        "devices": projected,
    })))
}

#[tauri::command]
/// Probe one session-owned device and return only the safe fact projection.
pub fn probe_device_profile_generator_device(
    sidecar: State<'_, SidecarState>,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
    device_handle: String,
) -> Result<Value, String> {
    let serial = {
        let registry = lock_registry(&state)?;
        match resolve_device(&registry, &session_handle, &device_handle) {
            Ok(device) => device.serial.clone(),
            Err(error) => return Ok(error),
        }
    };
    let response = request_sidecar(
        &sidecar,
        "probeDevice",
        Some(json!({ "adbPath": ADB_EXECUTABLE, "serial": serial })),
    )?;
    let result = match success_result(&response) {
        Some(result) => result,
        None => return Ok(response),
    };
    let facts = project_safe_facts(result);
    let mut registry = lock_registry(&state)?;
    let device = match resolve_device_mut(&mut registry, &session_handle, &device_handle) {
        Ok(device) => device,
        Err(error) => return Ok(error),
    };
    device.facts = Some(facts.clone());
    Ok(success(json!({ "facts": facts })))
}

#[tauri::command]
/// Generate or revalidate a profile draft using stored safe probe facts.
pub fn generate_device_profile_draft(
    sidecar: State<'_, SidecarState>,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
    device_handle: String,
    profile: Option<Value>,
) -> Result<Value, String> {
    let facts = match stored_facts(&state, &session_handle, &device_handle)? {
        Ok(facts) => facts,
        Err(error) => return Ok(error),
    };
    let mut payload = Map::new();
    payload.insert("facts".to_string(), facts);
    if let Some(profile) = profile {
        payload.insert("profile".to_string(), profile);
    }
    request_sidecar(
        &sidecar,
        "generateDeviceProfileDraft",
        Some(Value::Object(payload)),
    )
}

#[tauri::command]
/// Run side-effect-free collision analysis against a session-owned root.
pub fn check_device_profile_collisions(
    sidecar: State<'_, SidecarState>,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
    device_handle: String,
    root_handle: String,
    profile: Value,
) -> Result<Value, String> {
    let (facts, root) =
        match stored_generation_context(&state, &session_handle, &device_handle, &root_handle)? {
            Ok(context) => context,
            Err(error) => return Ok(error),
        };
    request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(json!({
            "authoredRoot": root.root,
            "facts": facts,
            "profile": profile,
        })),
    )
}

#[tauri::command]
/// Revalidate, rescan, and safely publish one new canonical profile file.
pub fn save_generated_device_profile(
    sidecar: State<'_, SidecarState>,
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
    device_handle: String,
    root_handle: String,
    profile: Value,
) -> Result<Value, String> {
    let (facts, root) =
        match stored_generation_context(&state, &session_handle, &device_handle, &root_handle)? {
            Ok(context) => context,
            Err(error) => return Ok(error),
        };

    let validation = request_sidecar(
        &sidecar,
        "generateDeviceProfileDraft",
        Some(json!({ "facts": facts.clone(), "profile": profile.clone() })),
    )?;
    let draft = match success_result(&validation) {
        Some(result) => result,
        None => return Ok(validation),
    };
    let Some(canonical_yaml) = draft.get("canonicalYaml").and_then(Value::as_str) else {
        return Ok(api_error(
            "device_profile_invalid",
            "The device profile must pass validation before it can be saved.",
            json!({ "reason": "validation_failed" }),
        ));
    };
    let file_name = match draft
        .get("destination")
        .and_then(|destination| destination.get("fileName"))
        .and_then(Value::as_str)
    {
        Some(file_name) if safe_file_name(file_name) => file_name.to_string(),
        _ => return Ok(invalid_destination_error()),
    };

    let collisions = request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(json!({
            "authoredRoot": root.root,
            "facts": facts,
            "profile": profile,
        })),
    )?;
    let collision_result = match success_result(&collisions) {
        Some(result) => result,
        None => return Ok(collisions),
    };
    if collision_result.get("blocking").and_then(Value::as_bool) != Some(false) {
        return Ok(api_error(
            "device_profile_collision_blocking",
            "Blocking device-profile collisions must be resolved before saving.",
            json!({
                "collisions": collision_result
                    .get("collisions")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            }),
        ));
    }

    if let Err(error) = publish_create_new(&root, &file_name, canonical_yaml.as_bytes()) {
        return Ok(error);
    }
    Ok(success(json!({
        "fileName": file_name,
        "displayPath": format!("device_profiles/{file_name}"),
    })))
}

#[tauri::command]
/// Expire a generator session and every device or root handle scoped to it.
pub fn cancel_device_profile_generator(
    state: State<'_, DeviceProfileGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    let mut registry = lock_registry(&state)?;
    if registry.sessions.remove(&session_handle).is_none() {
        return Ok(expired_session_error());
    }
    Ok(success(json!({})))
}

fn lock_registry(
    state: &DeviceProfileGeneratorState,
) -> Result<std::sync::MutexGuard<'_, GeneratorRegistry>, String> {
    state
        .registry
        .lock()
        .map_err(|_| "Device-profile generator state is unavailable.".to_string())
}

fn resolve_device<'a>(
    registry: &'a GeneratorRegistry,
    session_handle: &str,
    device_handle: &str,
) -> Result<&'a TrustedDevice, Value> {
    registry
        .session(session_handle)?
        .devices
        .get(device_handle)
        .ok_or_else(invalid_handle_error)
}

fn resolve_device_mut<'a>(
    registry: &'a mut GeneratorRegistry,
    session_handle: &str,
    device_handle: &str,
) -> Result<&'a mut TrustedDevice, Value> {
    registry
        .session_mut(session_handle)?
        .devices
        .get_mut(device_handle)
        .ok_or_else(invalid_handle_error)
}

fn resolve_root<'a>(
    registry: &'a GeneratorRegistry,
    session_handle: &str,
    root_handle: &str,
) -> Result<&'a TrustedRoot, Value> {
    registry
        .session(session_handle)?
        .roots
        .get(root_handle)
        .ok_or_else(invalid_handle_error)
}

fn stored_facts(
    state: &DeviceProfileGeneratorState,
    session_handle: &str,
    device_handle: &str,
) -> Result<Result<Value, Value>, String> {
    let registry = lock_registry(state)?;
    let device = match resolve_device(&registry, session_handle, device_handle) {
        Ok(device) => device,
        Err(error) => return Ok(Err(error)),
    };
    Ok(match device.facts.clone() {
        Some(facts) => Ok(facts),
        None => Err(api_error(
            "device_not_probed",
            "Read the selected device information before generating a profile.",
            json!({}),
        )),
    })
}

fn stored_generation_context(
    state: &DeviceProfileGeneratorState,
    session_handle: &str,
    device_handle: &str,
    root_handle: &str,
) -> Result<Result<(Value, TrustedRoot), Value>, String> {
    let registry = lock_registry(state)?;
    let device = match resolve_device(&registry, session_handle, device_handle) {
        Ok(device) => device,
        Err(error) => return Ok(Err(error)),
    };
    let Some(facts) = device.facts.clone() else {
        return Ok(Err(api_error(
            "device_not_probed",
            "Read the selected device information before generating a profile.",
            json!({}),
        )));
    };
    let root = match resolve_root(&registry, session_handle, root_handle) {
        Ok(root) => root.clone(),
        Err(error) => return Ok(Err(error)),
    };
    Ok(Ok((facts, root)))
}

fn project_safe_facts(result: &Value) -> Value {
    json!({
        "manufacturer": result.get("manufacturer").cloned().unwrap_or(Value::Null),
        "brand": result.get("brand").cloned().unwrap_or(Value::Null),
        "model": result.get("model").cloned().unwrap_or(Value::Null),
        "product": result.get("product").cloned().unwrap_or(Value::Null),
        "device": result.get("device").cloned().unwrap_or(Value::Null),
        "board": result.get("board").cloned().unwrap_or(Value::Null),
        "hardware": result.get("hardware").cloned().unwrap_or(Value::Null),
        "abis": result.get("abis").cloned().unwrap_or_else(|| json!([])),
        "androidVersion": result.get("android_version").cloned().unwrap_or(Value::Null),
        "androidApiLevel": result.get("android_api_level").cloned().unwrap_or(Value::Null),
    })
}

fn validate_authored_root(path: &Path) -> Result<TrustedRoot, Value> {
    let root = fs::canonicalize(path).map_err(|_| invalid_root_error())?;
    if !root.is_dir() {
        return Err(invalid_root_error());
    }
    let profile_directory =
        fs::canonicalize(root.join("device_profiles")).map_err(|_| invalid_root_error())?;
    if !profile_directory.is_dir() || !profile_directory.starts_with(&root) {
        return Err(invalid_root_error());
    }
    Ok(TrustedRoot {
        root,
        profile_directory,
    })
}

fn safe_file_name(file_name: &str) -> bool {
    let path = Path::new(file_name);
    path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
        && file_name.ends_with(".yaml")
}

fn publish_create_new(root: &TrustedRoot, file_name: &str, bytes: &[u8]) -> Result<(), Value> {
    if !root.profile_directory.starts_with(&root.root) || !safe_file_name(file_name) {
        return Err(invalid_destination_error());
    }
    let destination = root.profile_directory.join(file_name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(destination_exists_error()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(save_failed_error()),
    }

    let temp_path = create_temp_sibling(&root.profile_directory, file_name, bytes)?;
    let guard = TempFileGuard(temp_path.clone());
    match fs::hard_link(&temp_path, &destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(destination_exists_error());
        }
        Err(_) => return Err(save_failed_error()),
    }
    let _ = fs::remove_file(&temp_path);
    drop(guard);
    sync_directory_when_supported(&root.profile_directory);
    Ok(())
}

fn create_temp_sibling(directory: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, Value> {
    for _ in 0..32 {
        let nonce = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".{file_name}.tmp-{nonce}"));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(save_failed_error()),
        };
        if file.write_all(bytes).is_err() || file.flush().is_err() || file.sync_all().is_err() {
            let _ = fs::remove_file(&path);
            return Err(save_failed_error());
        }
        return Ok(path);
    }
    Err(save_failed_error())
}

#[cfg(unix)]
fn sync_directory_when_supported(directory: &Path) {
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory_when_supported(_directory: &Path) {}

struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn request_sidecar(
    sidecar: &SidecarState,
    request_type: &str,
    payload: Option<Value>,
) -> Result<Value, String> {
    sidecar
        .request(request_type, payload)
        .map(strip_transport_id)
}

fn strip_transport_id(mut response: Value) -> Value {
    if let Some(object) = response.as_object_mut() {
        object.remove("id");
    }
    response
}

fn success_result(response: &Value) -> Option<&Value> {
    (response.get("ok").and_then(Value::as_bool) == Some(true))
        .then(|| response.get("result"))
        .flatten()
}

fn success(result: Value) -> Value {
    json!({ "ok": true, "result": result })
}

fn api_error(code: &str, message: &str, details: Value) -> Value {
    json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message,
            "details": details,
        }
    })
}

fn expired_session_error() -> Value {
    api_error(
        "device_profile_generator_session_expired",
        "The device-profile generator session has expired. Start the generator again.",
        json!({}),
    )
}

fn invalid_handle_error() -> Value {
    api_error(
        "device_profile_generator_handle_invalid",
        "The selected generator item is no longer available.",
        json!({}),
    )
}

fn invalid_root_error() -> Value {
    api_error(
        "device_profile_authored_root_invalid",
        "Select an authored root with an accessible device_profiles directory.",
        json!({}),
    )
}

fn invalid_destination_error() -> Value {
    api_error(
        "device_profile_destination_invalid",
        "The generated device-profile destination is invalid.",
        json!({}),
    )
}

fn destination_exists_error() -> Value {
    api_error(
        "device_profile_destination_exists",
        "The generated device-profile destination already exists.",
        json!({}),
    )
}

fn save_failed_error() -> Value {
    api_error(
        "device_profile_save_failed",
        "The generated device profile could not be saved safely.",
        json!({}),
    )
}

fn generator_protocol_error() -> Value {
    api_error(
        "device_profile_generator_protocol_invalid",
        "The device-profile generator received an invalid backend response.",
        json!({}),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emuchef-tauri-{label}-{nonce}"));
        fs::create_dir_all(root.join("device_profiles")).unwrap();
        root
    }

    #[test]
    fn opaque_handles_are_scoped_to_their_generator_session() {
        let mut registry = GeneratorRegistry::default();
        let first = registry.begin_session();
        let second = registry.begin_session();
        let device_handle = registry.allocate_handle("device");
        registry.session_mut(&first).unwrap().devices.insert(
            device_handle.clone(),
            TrustedDevice {
                serial: "SECRET-SERIAL".to_string(),
                facts: None,
            },
        );
        assert!(resolve_device(&registry, &first, &device_handle).is_ok());
        assert!(resolve_device(&registry, &second, &device_handle).is_err());
        registry.sessions.remove(&first);
        assert!(resolve_device(&registry, &first, &device_handle).is_err());
    }

    #[test]
    fn opaque_root_handles_are_scoped_to_their_generator_session() {
        let root_path = temp_root("root-handle-scope");
        let trusted = validate_authored_root(&root_path).unwrap();
        let mut registry = GeneratorRegistry::default();
        let first = registry.begin_session();
        let second = registry.begin_session();
        let root_handle = registry.allocate_handle("root");
        registry
            .session_mut(&first)
            .unwrap()
            .roots
            .insert(root_handle.clone(), trusted);
        assert!(resolve_root(&registry, &first, &root_handle).is_ok());
        assert!(resolve_root(&registry, &second, &root_handle).is_err());
        registry.sessions.remove(&first);
        assert!(resolve_root(&registry, &first, &root_handle).is_err());
        fs::remove_dir_all(root_path).unwrap();
    }

    #[test]
    fn safe_fact_projection_omits_serial_and_uses_camel_case() {
        let projection = project_safe_facts(&json!({
            "serial": "SECRET-SERIAL",
            "manufacturer": "AYANEO",
            "android_version": 13,
            "android_api_level": 33,
            "abis": ["arm64-v8a"],
        }));
        let serialized = serde_json::to_string(&projection).unwrap();
        assert!(!serialized.contains("SECRET-SERIAL"));
        assert!(!serialized.contains("serial"));
        assert_eq!(projection["androidVersion"], 13);
        assert_eq!(projection["androidApiLevel"], 33);
    }

    #[test]
    fn safe_publication_is_complete_and_never_clobbers() {
        let root_path = temp_root("publish");
        let trusted = validate_authored_root(&root_path).unwrap();
        publish_create_new(&trusted, "new.profile.yaml", b"complete: true\n").unwrap();
        assert_eq!(
            fs::read_to_string(root_path.join("device_profiles/new.profile.yaml")).unwrap(),
            "complete: true\n"
        );
        let error =
            publish_create_new(&trusted, "new.profile.yaml", b"replacement: true\n").unwrap_err();
        assert_eq!(error["error"]["code"], "device_profile_destination_exists");
        assert_eq!(
            fs::read_to_string(root_path.join("device_profiles/new.profile.yaml")).unwrap(),
            "complete: true\n"
        );
        fs::remove_dir_all(root_path).unwrap();
    }

    #[test]
    fn unsafe_destination_names_are_rejected() {
        let root_path = temp_root("unsafe-destination");
        let trusted = validate_authored_root(&root_path).unwrap();
        for file_name in ["../escape.yaml", "nested/escape.yaml", "profile.yml"] {
            let error = publish_create_new(&trusted, file_name, b"unsafe").unwrap_err();
            assert_eq!(error["error"]["code"], "device_profile_destination_invalid");
        }
        fs::remove_dir_all(root_path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn authored_root_rejects_device_profile_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root_path = temp_root("symlink-root");
        let outside = temp_root("symlink-outside");
        fs::remove_dir(root_path.join("device_profiles")).unwrap();
        symlink(
            outside.join("device_profiles"),
            root_path.join("device_profiles"),
        )
        .unwrap();
        assert!(validate_authored_root(&root_path).is_err());
        fs::remove_dir_all(root_path).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}

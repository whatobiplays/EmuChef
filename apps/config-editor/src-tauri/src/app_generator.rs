//! Trusted local-APK generator boundary for the Config Editor.
//!
//! Native APK and authored-root paths remain in process memory behind
//! session-scoped handles. Only safe inspection metadata, labels, authored drafts,
//! diagnostics, canonical previews, and relative destination metadata cross into React.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{
    HeaderMap, ACCEPT, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER, USER_AGENT,
};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tempfile::TempDir;
use url::Url;

use crate::app_sources::capabilities_for_mode;
use crate::sidecar_client::SidecarState;

const MAX_APK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SAFE_APK_LABEL: &str = "Selected local APK";
const SAFE_ROOT_LABEL: &str = "Selected authored root";
const PREFERENCES_FILE_NAME: &str = "app-generator-preferences.json";
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 5;
const HTTP_USER_AGENT: &str = "EmuChef-Config-Editor/0.1";
const GITHUB_API_ROOT: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
/// Reject retry hints beyond one day so untrusted headers cannot produce absurd guidance.
const MAX_RATE_LIMIT_ADVISORY_SECONDS: u64 = 24 * 60 * 60;
static NEXT_PUBLICATION_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppGeneratorPreferences {
    authored_root: Option<PathBuf>,
}

#[derive(Default)]
struct GeneratorRegistry {
    next_handle: u64,
    sessions: HashMap<String, GeneratorSession>,
    /// Sticky publication outcomes keyed by native operation identity.
    ///
    /// A record exists before its sidecar request is sent, so a lost or
    /// interrupted response keeps its truthful UNKNOWN state instead of
    /// being reported as a confirmed failure.
    publications: HashMap<String, PublicationRecord>,
}

#[derive(Default)]
struct GeneratorSession {
    apks: HashMap<String, TrustedApk>,
    roots: HashMap<String, TrustedRoot>,
    remote_sources: HashMap<String, TrustedRemoteSource>,
    remote_assets: HashMap<String, TrustedRemoteAsset>,
    temp_directory: Option<TempDir>,
    /// Root handle most recently chosen for this session. List and selection
    /// fall back to it when a request carries no explicit root handle.
    active_root: Option<String>,
    /// Root-bound existing-App selections by native handle.
    existing_apps: HashMap<String, TrustedExistingApp>,
    /// Immutable reviewed App/Recipe pairs by native review handle.
    reviews: HashMap<String, ReviewRecord>,
}

#[derive(Clone, Debug)]
struct TrustedApk {
    path: PathBuf,
    identity: FileIdentity,
    inspection: Option<Value>,
    inspection_handle: Option<String>,
    facts: Option<Value>,
    /// Retained asset handle this APK was downloaded from, when it came from a
    /// remote source. Draft resolution requires the selected asset to match it,
    /// because two independently valid handles do not prove that the inspected
    /// bytes are the bytes the generated recipe will install.
    remote_asset_handle: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PermissionSelectionRequest {
    inspection_handle: String,
    #[serde(default)]
    runtime_permissions: Vec<RuntimePermissionIdentity>,
    #[serde(default)]
    app_ops: Vec<AppOpPermissionIdentity>,
}

#[derive(Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RuntimePermissionIdentity {
    permission_name: String,
}

#[derive(Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AppOpPermissionIdentity {
    permission_name: String,
    operation_name: String,
    mode: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CanonicalPermissionAutomation {
    package_name: String,
    runtime_permissions: Vec<CanonicalRuntimePermission>,
    app_ops: Vec<CanonicalAppOpPermission>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CanonicalRuntimePermission {
    permission_name: String,
    requires_root: bool,
    android_api_min: u32,
    android_api_max: Option<u32>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CanonicalAppOpPermission {
    permission_name: String,
    operation_name: String,
    mode: String,
    requires_root: bool,
    android_api_min: u32,
    android_api_max: Option<u32>,
}

#[derive(Clone)]
struct TrustedRemoteSource {
    mode: String,
    provider: Option<String>,
    base_url: Option<String>,
    repository: Option<String>,
    release_tag: Option<String>,
    direct_url: Option<String>,
    release_analysis: Option<Vec<TrustedRemoteRelease>>,
}

#[derive(Clone)]
struct TrustedRemoteRelease {
    tag: String,
    prerelease: bool,
    asset_file_names: Vec<String>,
}

#[derive(Clone)]
struct TrustedRemoteAsset {
    source_handle: String,
    download_url: String,
    file_name: String,
    size: u64,
    release_tag: Option<String>,
}

#[derive(Clone, Debug)]
struct TrustedRoot {
    root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    length: u64,
    modified_nanos: u128,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

/// One existing-App selection captured by the backend and bound to a trusted root.
#[derive(Clone, Debug)]
struct TrustedExistingApp {
    /// Canonical authored root the selection is bound to.
    root_path: PathBuf,
    /// Opaque backend snapshot handle; never leaves native process memory.
    snapshot_id: String,
    /// Catalog-relative App Definition path.
    app_path: String,
    package_id: Option<String>,
}

/// An existing-App selection together with the native handle that names it.
#[derive(Clone, Debug)]
struct BoundExistingApp {
    handle: String,
    selection: TrustedExistingApp,
}

/// Immutable dependency fingerprint of one reviewed draft.
///
/// Publication re-checks every captured fact before the sidecar request is sent.
/// A changed APK file, a replaced inspection, a re-analyzed remote source, or a
/// replaced existing-App selection invalidates the review instead of publishing
/// a pair the author did not review.
#[derive(Clone, Debug)]
enum ReviewBinding {
    Local {
        apk_handle: String,
        apk_identity: FileIdentity,
        inspection_handle: String,
    },
    Remote {
        apk_handle: String,
        apk_identity: FileIdentity,
        inspection_handle: String,
        asset_handle: String,
        strategy: String,
        asset_pattern: Option<String>,
        include_prereleases: bool,
        trusted_sha256: Option<String>,
        /// Deterministic serialization of the trusted remote source payload.
        source_fingerprint: String,
    },
}

impl ReviewBinding {
    /// Native APK handle both binding kinds retain.
    fn apk_handle(&self) -> &str {
        match self {
            ReviewBinding::Local { apk_handle, .. } => apk_handle.as_str(),
            ReviewBinding::Remote { apk_handle, .. } => apk_handle.as_str(),
        }
    }
}

/// One immutable reviewed App/Recipe pair retained for publication.
///
/// Publication consumes only this capture: the save command accepts no mutable
/// App, Recipe, mapping, or permission inputs from the frontend.
#[derive(Clone)]
struct ReviewRecord {
    request_id: i64,
    /// Opaque backend review handle; never leaves native process memory.
    backend_review_id: Option<String>,
    binding: ReviewBinding,
    existing_app: Option<BoundExistingApp>,
    /// Canonical authored root the review was generated against.
    root_path: PathBuf,
    /// The reviewed candidate retained blocking diagnostics.
    blocking: bool,
    /// Catalog-relative destinations captured from the review result.
    app_relative_path: Option<String>,
    recipe_relative_path: Option<String>,
}

/// Confirmation state of one publication attempt.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    /// Recorded before the sidecar request is sent; a lost response keeps this state.
    Submitted,
    Saved,
    Failed,
}

/// Sticky registry-level publication memory for one native operation identity.
///
/// Publication records survive session cancellation and sidecar restarts so a
/// lost response stays an unconfirmed outcome instead of being reported as a
/// confirmed failure or silently retried.
#[derive(Clone)]
struct PublicationRecord {
    state: PublicationState,
    recorded_at: String,
    /// Confirmed publication outcome reported to the author, when one arrived.
    detail: Option<Value>,
    /// Canonical authored root the attempt was reviewed against.
    root_path: PathBuf,
}

/// Narrow request boundary over the persistent sidecar.
///
/// Routing review and publication flow through this trait keeps the trusted
/// lifecycle testable without a running sidecar process.
trait AppGeneratorTransport {
    fn request(&self, operation: &str, payload: Value) -> Result<Value, String>;
}

/// Production transport backed by the shared persistent sidecar.
struct SidecarTransport<'a> {
    sidecar: &'a SidecarState,
}

impl AppGeneratorTransport for SidecarTransport<'_> {
    fn request(&self, operation: &str, payload: Value) -> Result<Value, String> {
        request_sidecar(self.sidecar, operation, Some(payload))
    }
}

/// Process-memory state for local-APK generator sessions.
#[derive(Default)]
pub struct AppGeneratorState {
    registry: Mutex<GeneratorRegistry>,
}

impl AppGeneratorState {
    /// Invalidate all trusted handles, including after a sidecar restart.
    ///
    /// Sticky publication records are deliberately preserved: a sidecar restart
    /// erases the backend's publication memory, and dropping the native records
    /// would turn an unconfirmed attempt into an apparent "no record" that the
    /// author could mistake for a failed publication.
    pub fn clear(&self) -> Result<(), String> {
        let mut registry = lock_registry(self)?;
        registry.sessions.clear();
        Ok(())
    }
}

impl GeneratorRegistry {
    fn allocate(&mut self, prefix: &str) -> String {
        self.next_handle = self.next_handle.saturating_add(1);
        format!("{prefix}-{}", self.next_handle)
    }

    fn allocate_publication(&mut self) -> String {
        let sequence = NEXT_PUBLICATION_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        publication_operation_id(SystemTime::now(), std::process::id(), sequence)
    }

    fn begin(&mut self) -> String {
        let handle = self.allocate("app-generator");
        self.sessions
            .insert(handle.clone(), GeneratorSession::default());
        handle
    }

    fn session(&self, handle: &str) -> Result<&GeneratorSession, Value> {
        self.sessions.get(handle).ok_or_else(expired_session_error)
    }

    fn session_mut(&mut self, handle: &str) -> Result<&mut GeneratorSession, Value> {
        self.sessions
            .get_mut(handle)
            .ok_or_else(expired_session_error)
    }
}

fn publication_operation_id(created_at: SystemTime, process_id: u32, sequence: u64) -> String {
    let created_at_nanos = created_at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("publication-{created_at_nanos:x}-{process_id:x}-{sequence:x}")
}

#[tauri::command]
pub fn get_config_editor_authored_root(app: AppHandle) -> Result<Value, String> {
    let preferences = load_preferences(&app);
    let authored_root = preferences
        .authored_root
        .filter(|path| validate_authored_root(path).is_ok());
    Ok(success(json!({ "authoredRoot": authored_root })))
}

#[tauri::command]
pub fn set_config_editor_authored_root(
    app: AppHandle,
    authored_root: Option<String>,
) -> Result<Value, String> {
    let canonical = match authored_root {
        Some(path) => match validate_authored_root(Path::new(&path)) {
            Ok(root) => Some(root.root),
            Err(error) => return Ok(error),
        },
        None => None,
    };
    update_preferences(&app, |preferences| {
        preferences.authored_root = canonical.clone();
    });
    Ok(success(json!({ "authoredRoot": canonical })))
}

#[tauri::command]
pub fn begin_app_generator(
    app: AppHandle,
    state: State<'_, AppGeneratorState>,
) -> Result<Value, String> {
    let preferences = load_preferences(&app);
    let mut registry = lock_registry(&state)?;
    let session_handle = registry.begin();
    let mut root_handle = None;
    let mut root_label = None;

    if let Some(path) = preferences.authored_root.as_deref() {
        if let Ok(root) = validate_authored_root(path) {
            let handle = registry.allocate("app-root");
            if let Ok(session) = registry.session_mut(&session_handle) {
                session.roots.insert(handle.clone(), root);
                session.active_root = Some(handle.clone());
                root_handle = Some(handle);
                root_label = Some(SAFE_ROOT_LABEL);
            }
        }
    }

    Ok(success(json!({
        "sessionHandle": session_handle,
        "rootHandle": root_handle,
        "rootLabel": root_label,
    })))
}

#[tauri::command]
pub async fn choose_app_generator_apk(
    app: AppHandle,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    require_session(&state, &session_handle)?;
    let Some(selection) = app
        .dialog()
        .file()
        .add_filter("Android APK", &["apk"])
        .blocking_pick_file()
    else {
        return Ok(success(json!({ "cancelled": true })));
    };
    let path = match selection.into_path() {
        Ok(path) => path,
        Err(_) => return Ok(invalid_apk_error()),
    };
    let (path, identity) = match validate_apk(&path) {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let apk_handle = registry.allocate("apk");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.remote_sources.clear();
    session.remote_assets.clear();
    session.temp_directory = None;
    session.apks.clear();
    session.apks.insert(
        apk_handle.clone(),
        TrustedApk {
            path,
            identity,
            inspection: None,
            inspection_handle: None,
            facts: None,
            remote_asset_handle: None,
        },
    );
    Ok(success(json!({
        "cancelled": false,
        "apkHandle": apk_handle,
        "label": SAFE_APK_LABEL,
    })))
}

#[tauri::command]
pub fn set_app_generator_authored_root(
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    authored_root: String,
) -> Result<Value, String> {
    require_session(&state, &session_handle)?;
    let root = match validate_authored_root(Path::new(&authored_root)) {
        Ok(root) => root,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let root_handle = registry.allocate("app-root");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.roots.insert(root_handle.clone(), root);
    session.active_root = Some(root_handle.clone());
    Ok(success(json!({
        "cancelled": false,
        "rootHandle": root_handle,
        "label": SAFE_ROOT_LABEL,
    })))
}

#[tauri::command]
pub async fn choose_app_generator_authored_root(
    app: AppHandle,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    require_session(&state, &session_handle)?;
    let Some(selection) = app.dialog().file().blocking_pick_folder() else {
        return Ok(success(json!({ "cancelled": true })));
    };
    let path = match selection.into_path() {
        Ok(path) => path,
        Err(_) => return Ok(invalid_root_error()),
    };
    let root = match validate_authored_root(&path) {
        Ok(root) => root,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let root_handle = registry.allocate("app-root");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    let persisted_root = root.root.clone();
    session.roots.insert(root_handle.clone(), root);
    session.active_root = Some(root_handle.clone());
    Ok(success(json!({
        "cancelled": false,
        "rootHandle": root_handle,
        "label": SAFE_ROOT_LABEL,
        "path": persisted_root,
    })))
}

#[tauri::command]
pub fn analyze_app_generator_source(
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    mode: String,
    source_url: String,
    include_prereleases: bool,
) -> Result<Value, String> {
    require_session(&state, &session_handle)?;
    let normalized = match normalize_remote_source(&mode, &source_url) {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let client = match remote_http_client() {
        Ok(client) => client,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let source_handle = registry.allocate("remote-source");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.remote_sources.clear();
    session.remote_assets.clear();
    session.apks.clear();
    session.temp_directory = None;
    let mut trusted_assets = Vec::new();
    let mut trusted_release_analysis = None;
    let result = if mode == "direct_apk" {
        let file_name = normalized
            .url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|value| value.to_ascii_lowercase().ends_with(".apk"))
            .map(str::to_string)
            .unwrap_or_else(|| "Remote APK".to_string());
        let asset_handle = registry.allocate("remote-asset");
        let asset = TrustedRemoteAsset {
            source_handle: source_handle.clone(),
            download_url: normalized.url.to_string(),
            file_name: file_name.clone(),
            size: 0,
            release_tag: None,
        };
        trusted_assets.push((asset_handle.clone(), asset));
        json!({
            "sourceHandle": source_handle,
            "mode": mode,
            "normalizedUrl": normalized.url,
            "repository": Value::Null,
            "capabilities": capabilities_for_mode(&mode),
            "releases": [],
            "assets": [{
                "assetHandle": asset_handle,
                "fileName": file_name,
                "size": Value::Null,
                "contentType": Value::Null,
                "releaseTag": Value::Null,
                "releaseName": Value::Null,
                "prerelease": false,
                "publishedAt": Value::Null,
            }],
            "preselectedAssetHandle": asset_handle,
        })
    } else {
        drop(registry);
        let analyzed = match normalized.provider.as_deref() {
            Some("github") => {
                analyze_github_source(&client, &mode, &normalized, include_prereleases)
            }
            Some("gitlab") => {
                analyze_gitlab_source(&client, &mode, &normalized, include_prereleases)
            }
            Some("forgejo") => {
                analyze_forgejo_source(&client, &mode, &normalized, include_prereleases)
            }
            _ => Err(provider_url_error("remote provider")),
        };
        let analyzed = match analyzed {
            Ok(value) => value,
            Err(error) => return Ok(error),
        };
        registry = lock_registry(&state)?;
        let mut releases = Vec::new();
        let mut flat_assets = Vec::new();
        let retain_release_analysis = mode == "github_repository";
        let mut trusted_releases = Vec::new();
        for release in analyzed.releases {
            let mut release_assets = Vec::new();
            let mut trusted_asset_file_names = Vec::new();
            for asset in release.assets {
                let asset_handle = registry.allocate("remote-asset");
                trusted_asset_file_names.push(asset.file_name.clone());
                let safe = json!({
                    "assetHandle": asset_handle,
                    "fileName": asset.file_name,
                    "size": asset.size,
                    "contentType": asset.content_type,
                    "releaseTag": release.tag,
                    "releaseName": release.name,
                    "prerelease": release.prerelease,
                    "publishedAt": release.published_at,
                });
                release_assets.push(safe.clone());
                flat_assets.push(safe);
                trusted_assets.push((
                    asset_handle,
                    TrustedRemoteAsset {
                        source_handle: source_handle.clone(),
                        download_url: asset.download_url,
                        file_name: asset.file_name,
                        size: asset.size,
                        release_tag: Some(release.tag.clone()),
                    },
                ));
            }
            if retain_release_analysis {
                trusted_releases.push(TrustedRemoteRelease {
                    tag: release.tag.clone(),
                    prerelease: release.prerelease,
                    asset_file_names: trusted_asset_file_names,
                });
            }
            if retain_release_analysis || !release_assets.is_empty() {
                releases.push(json!({
                    "tag": release.tag,
                    "name": release.name,
                    "prerelease": release.prerelease,
                    "publishedAt": release.published_at,
                    "assets": release_assets,
                }));
            }
        }
        if retain_release_analysis {
            trusted_release_analysis = Some(trusted_releases);
        }
        let eligible_assets = flat_assets
            .iter()
            .filter(|asset| {
                include_prereleases
                    || asset.get("prerelease").and_then(Value::as_bool) != Some(true)
            })
            .collect::<Vec<_>>();
        let preselected = (eligible_assets.len() == 1)
            .then(|| eligible_assets[0].get("assetHandle").cloned())
            .flatten();
        json!({
            "sourceHandle": source_handle,
            "mode": mode,
            "normalizedUrl": normalized.url,
            "repository": analyzed.repository,
            "capabilities": capabilities_for_mode(&mode),
            "releases": releases,
            "assets": flat_assets,
            "preselectedAssetHandle": preselected,
        })
    };
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.remote_sources.insert(
        source_handle.clone(),
        TrustedRemoteSource {
            mode: mode.clone(),
            provider: normalized.provider.clone(),
            base_url: normalized.base_url.clone(),
            repository: normalized.repository.clone(),
            release_tag: normalized.release_tag.clone(),
            direct_url: (mode == "direct_apk").then(|| normalized.url.to_string()),
            release_analysis: trusted_release_analysis,
        },
    );
    session.remote_assets.extend(trusted_assets);
    Ok(success(result))
}

#[tauri::command]
pub async fn download_app_generator_remote_apk(
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    asset_handle: String,
) -> Result<Value, String> {
    let (asset, path) = {
        let mut registry = lock_registry(&state)?;
        let session = match registry.session_mut(&session_handle) {
            Ok(session) => session,
            Err(error) => return Ok(error),
        };
        let Some(asset) = session.remote_assets.get(&asset_handle).cloned() else {
            return Ok(invalid_handle_error());
        };
        if session.temp_directory.is_none() {
            session.temp_directory = tempfile::Builder::new()
                .prefix("emuchef-app-generator-")
                .tempdir()
                .ok();
        }
        let Some(directory) = session.temp_directory.as_ref() else {
            return Ok(remote_source_error(
                "remote_workspace_failed",
                "A temporary download workspace could not be created.",
            ));
        };
        (asset, directory.path().join("selected.apk"))
    };

    let download_asset = asset.clone();
    let download_path = path.clone();
    let downloaded = tauri::async_runtime::spawn_blocking(move || {
        // A retained address was accepted when it was captured; re-checking it
        // immediately before the request keeps this path enforcing the address
        // policy even if a future change stores a source without normalization.
        let download_url = Url::parse(&download_asset.download_url)
            .ok()
            .filter(safe_remote_url)
            .ok_or_else(|| {
                remote_source_error(
                    "remote_url_invalid",
                    "The APK download address is not an accepted public HTTPS address.",
                )
            })?;
        let client = remote_http_client()?;
        let response = client
            .get(download_url)
            .header(USER_AGENT, HTTP_USER_AGENT)
            .send()
            .map_err(|_| {
                remote_source_error("remote_download_failed", "The APK could not be downloaded.")
            })?;
        let final_url = response.url().clone();
        if final_url.scheme() != "https" || !safe_remote_url(&final_url) {
            return Err(remote_source_error(
                "remote_redirect_unsafe",
                "The APK download redirected to an unsafe address.",
            ));
        }
        if !response.status().is_success() {
            return Err(remote_source_error(
                "remote_download_failed",
                "The APK download did not succeed.",
            ));
        }
        let content_length = response.content_length().or_else(|| {
            response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
        });
        if download_asset.size > MAX_APK_BYTES
            || content_length.is_some_and(|value| value == 0 || value > MAX_APK_BYTES)
        {
            return Err(remote_source_error(
                "remote_apk_size_invalid",
                "The selected APK is empty or larger than 2 GiB.",
            ));
        }
        if clearly_non_apk_content_type(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
        ) {
            return Err(remote_source_error(
                "remote_content_type_invalid",
                "The selected download is not an APK file.",
            ));
        }
        let file_name =
            response_file_name(&response).unwrap_or_else(|| download_asset.file_name.clone());
        if !file_name.to_ascii_lowercase().ends_with(".apk") {
            return Err(remote_source_error(
                "remote_apk_name_invalid",
                "The selected download does not identify an APK file.",
            ));
        }
        let _ = fs::remove_file(&download_path);
        if let Err(error) = stream_apk_response(response, &download_path) {
            let _ = fs::remove_file(&download_path);
            return Err(error);
        }
        let (path, identity) = validate_apk(&download_path)?;
        Ok::<_, Value>((path, identity, file_name, final_url))
    })
    .await;

    let (path, identity, file_name, final_url) = match downloaded {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return Ok(error),
        Err(_) => {
            return Ok(remote_source_error(
                "remote_download_failed",
                "The APK download could not be completed.",
            ))
        }
    };

    retain_downloaded_remote_apk(
        &state,
        &session_handle,
        &asset_handle,
        &asset,
        DownloadedRemoteApk {
            path,
            identity,
            file_name,
            final_url,
        },
    )
}

/// One completed APK download retained by the native boundary.
///
/// The transient post-redirect URL belongs to the labelled download result only:
/// it is never written to session state, review payloads, or the catalog.
struct DownloadedRemoteApk {
    path: PathBuf,
    identity: FileIdentity,
    file_name: String,
    final_url: Url,
}

/// Retain one completed remote download and build its author-facing result.
///
/// The retained records keep the durable asset identity only: the source record
/// keeps the author's original address, the asset record keeps the provider's
/// release asset URL, and the APK record keeps the asset handle it came from.
/// The transient post-redirect URL is returned for display and is never stored,
/// so a later review maps the original durable source instead of a signed or
/// CDN address.
fn retain_downloaded_remote_apk(
    state: &AppGeneratorState,
    session_handle: &str,
    asset_handle: &str,
    asset: &TrustedRemoteAsset,
    downloaded: DownloadedRemoteApk,
) -> Result<Value, String> {
    let mut registry = lock_registry(state)?;
    let apk_handle = registry.allocate("apk");
    let session = match registry.session_mut(session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.apks.insert(
        apk_handle.clone(),
        TrustedApk {
            path: downloaded.path,
            identity: downloaded.identity,
            inspection: None,
            inspection_handle: None,
            facts: None,
            remote_asset_handle: Some(asset_handle.to_string()),
        },
    );
    let source = session.remote_sources.get(&asset.source_handle).cloned();
    let file_name = downloaded.file_name;
    Ok(success(json!({
        "apkHandle": apk_handle,
        "label": file_name.clone(),
        "source": {
            "mode": source.as_ref().map(|value| value.mode.as_str()).unwrap_or("direct_apk"),
            "strategy": "pinned_remote_asset",
            "downloadUrl": downloaded.final_url,
            "provider": source.as_ref().and_then(|value| value.provider.clone()),
            "baseUrl": source.as_ref().and_then(|value| value.base_url.clone()),
            "repository": source.as_ref().and_then(|value| value.repository.clone()),
            "releaseTag": asset.release_tag.clone(),
            "assetName": file_name,
            "assetPattern": Value::Null,
            "includePrereleases": false,
        }
    })))
}

#[derive(Clone)]
struct NormalizedRemoteSource {
    url: Url,
    provider: Option<String>,
    base_url: Option<String>,
    repository: Option<String>,
    release_tag: Option<String>,
}

struct GitHubAnalysis {
    repository: Value,
    releases: Vec<GitHubRelease>,
}
struct GitHubRelease {
    tag: String,
    name: Option<String>,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}
struct GitHubAsset {
    file_name: String,
    size: u64,
    content_type: Option<String>,
    download_url: String,
}

/// Normalize one author-entered remote source address.
///
/// Every mode requires a public HTTPS address without credentials or a
/// fragment. A durable query string is part of the author's original address,
/// so a direct APK address keeps it; provider repository and release addresses
/// are canonicalized from their path segments, and a query there is refused
/// instead of being silently dropped.
fn normalize_remote_source(mode: &str, input: &str) -> Result<NormalizedRemoteSource, Value> {
    let mut url = Url::parse(input.trim())
        .map_err(|_| remote_source_error("remote_url_invalid", "Enter a valid HTTPS address."))?;
    let durable_query = mode == "direct_apk";
    if !safe_remote_url(&url)
        || url.fragment().is_some()
        || (!durable_query && url.query().is_some())
    {
        return Err(remote_source_error(
            "remote_url_invalid",
            if durable_query {
                "Enter a valid public HTTPS address without credentials or a fragment."
            } else {
                "Enter a valid public HTTPS address without credentials, query parameters, or a fragment."
            },
        ));
    }
    url.set_fragment(None);
    if mode == "direct_apk" {
        return Ok(NormalizedRemoteSource {
            url,
            provider: None,
            base_url: None,
            repository: None,
            release_tag: None,
        });
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let segments = url
        .path_segments()
        .map(|items| items.filter(|item| !item.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let (provider, base_url, repository, release_tag, normalized_url) = match mode {
        "github_repository" | "github_release" => {
            if host != "github.com" {
                return Err(provider_url_error("GitHub"));
            }
            let release = mode.ends_with("_release");
            let (repository, tag) = parse_owner_repository_release(&segments, release)?;
            let normalized = if let Some(tag) = &tag {
                format!("https://github.com/{repository}/releases/tag/{tag}")
            } else {
                format!("https://github.com/{repository}")
            };
            (
                "github",
                "https://github.com".to_string(),
                repository,
                tag,
                normalized,
            )
        }
        "gitlab_repository" | "gitlab_release" => {
            if host != "gitlab.com" {
                return Err(provider_url_error("GitLab"));
            }
            let release = mode.ends_with("_release");
            let (repository, tag) = parse_gitlab_source(&segments, release)?;
            let normalized = if let Some(tag) = &tag {
                format!("https://gitlab.com/{repository}/-/releases/{tag}")
            } else {
                format!("https://gitlab.com/{repository}")
            };
            (
                "gitlab",
                "https://gitlab.com".to_string(),
                repository,
                tag,
                normalized,
            )
        }
        "forgejo_repository" | "forgejo_release" => {
            let release = mode.ends_with("_release");
            let (repository, tag) = parse_owner_repository_release(&segments, release)?;
            let base = format!("https://{host}");
            let normalized = if let Some(tag) = &tag {
                format!("{base}/{repository}/releases/tag/{tag}")
            } else {
                format!("{base}/{repository}")
            };
            ("forgejo", base, repository, tag, normalized)
        }
        _ => return Err(provider_url_error("remote provider")),
    };
    Ok(NormalizedRemoteSource {
        url: Url::parse(&normalized_url).map_err(|_| provider_url_error(provider))?,
        provider: Some(provider.to_string()),
        base_url: Some(base_url),
        repository: Some(repository),
        release_tag,
    })
}

fn parse_owner_repository_release(
    segments: &[&str],
    release: bool,
) -> Result<(String, Option<String>), Value> {
    let (owner, repository, tag) = if release {
        if segments.len() != 5 || segments[2] != "releases" || segments[3] != "tag" {
            return Err(provider_url_error("release provider"));
        }
        (segments[0], segments[1], Some(segments[4].to_string()))
    } else {
        if segments.len() != 2 {
            return Err(provider_url_error("repository provider"));
        }
        (segments[0], segments[1].trim_end_matches(".git"), None)
    };
    if !valid_repository_component(owner)
        || !valid_repository_component(repository)
        || tag.as_deref().is_some_and(str::is_empty)
    {
        return Err(provider_url_error("repository provider"));
    }
    Ok((format!("{owner}/{repository}"), tag))
}

fn parse_gitlab_source(
    segments: &[&str],
    release: bool,
) -> Result<(String, Option<String>), Value> {
    let (repository_segments, tag) = if release {
        let Some(marker) = segments
            .windows(2)
            .position(|pair| pair == ["-", "releases"])
        else {
            return Err(provider_url_error("GitLab"));
        };
        if marker < 2 || marker + 2 >= segments.len() || marker + 3 != segments.len() {
            return Err(provider_url_error("GitLab"));
        }
        (&segments[..marker], Some(segments[marker + 2].to_string()))
    } else {
        (segments, None)
    };
    if repository_segments.len() < 2
        || repository_segments.len() > 20
        || !repository_segments
            .iter()
            .all(|component| valid_repository_component(component))
        || tag.as_deref().is_some_and(str::is_empty)
    {
        return Err(provider_url_error("GitLab"));
    }
    let mut parts = repository_segments.to_vec();
    if let Some(last) = parts.last_mut() {
        *last = last.trim_end_matches(".git");
    }
    Ok((parts.join("/"), tag))
}

fn valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn provider_url_error(provider: &str) -> Value {
    remote_source_error(
        "provider_url_invalid",
        &format!("Enter a supported public {provider} repository or release address."),
    )
}

/// Whether one resolved address is publicly reachable.
///
/// The accepted source contract admits durable public HTTPS endpoints only, so
/// loopback, private, link-local, shared, reserved, and documentation ranges are
/// refused, including their IPv4-mapped IPv6 forms and IPv6 literals.
fn public_unicast_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => public_ipv4_address(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => public_ipv4_address(mapped),
            None => public_ipv6_address(v6),
        },
    }
}

/// Whether one IPv4 address is inside the ranges the source policy admits.
///
/// The stable standard predicates cover loopback, private, link-local,
/// broadcast, documentation, and multicast addresses. The remaining exclusions
/// are ranges without a stable predicate: `0.0.0.0/8`, carrier-grade NAT
/// `100.64.0.0/10`, IETF protocol assignments `192.0.0.0/24`, the deprecated
/// 6to4 relay anycast `192.88.99.0/24`, benchmarking `198.18.0.0/15`, and
/// reserved `240.0.0.0/4`.
fn public_ipv4_address(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_private()
        && !address.is_link_local()
        && !address.is_broadcast()
        && !address.is_documentation()
        && !address.is_multicast()
        && octets[0] != 0
        && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        && !(octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        && !(octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        && !(octets[0] == 198 && (18..=19).contains(&octets[1]))
        && octets[0] < 240
}

/// Whether one IPv6 address is inside the ranges the source policy admits.
///
/// IPv4-mapped addresses are classified as their embedded IPv4 address before
/// this helper runs. The remaining exclusions are the unspecified and loopback
/// addresses, multicast, unique-local `fc00::/7`, unicast link-local
/// `fe80::/10`, documentation `2001:db8::/32`, deprecated site-local
/// `fec0::/10`, the deprecated IPv4-compatible `::/96` form, and the Teredo
/// `2001::/32` and 6to4 `2002::/16` transition prefixes. Uncommon IANA
/// special-purpose registrations without a stable predicate stay outside this
/// bounded list.
fn public_ipv6_address(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && !address.is_unique_local()
        && !address.is_unicast_link_local()
        && !(segments[0] == 0x2001 && (segments[1] == 0x0000 || segments[1] == 0x0db8))
        && segments[0] & 0xffc0 != 0xfec0
        && segments[0] != 0x2002
        && segments[..6].iter().any(|segment| *segment != 0)
}

/// Whether one resolved address set contains only public addresses.
///
/// Every answer for a name must be public: a host that resolves to any internal
/// address is refused instead of racing the client's own connection choice.
fn resolved_public_addresses(addresses: &[SocketAddr]) -> bool {
    !addresses.is_empty()
        && addresses
            .iter()
            .all(|address| public_unicast_address(address.ip()))
}

/// Resolve one host and require every answer to be publicly reachable.
fn resolve_public_host(host: &str) -> Result<Vec<SocketAddr>, &'static str> {
    let resolved = (host, 0u16)
        .to_socket_addrs()
        .map_err(|_| "remote_host_unresolved")?
        .collect::<Vec<_>>();
    if resolved_public_addresses(&resolved) {
        Ok(resolved)
    } else {
        Err("remote_host_not_public")
    }
}

fn safe_remote_url(url: &Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return false;
    }
    let literal = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(address) = literal.parse::<IpAddr>() {
        return public_unicast_address(address);
    }
    true
}

/// Whether one redirect hop stays inside the accepted source policy.
///
/// The chain must stay within the bounded hop budget and every hop must be a
/// public HTTPS URL, so a public address that redirects to an internal one is
/// refused before the hop is followed.
fn redirect_target_allowed(previous_hops: usize, next: &Url) -> bool {
    previous_hops < MAX_REDIRECTS && next.scheme() == "https" && safe_remote_url(next)
}

/// DNS resolver limited to publicly reachable addresses.
///
/// Validating inside resolution keeps the checked address set identical to the
/// set the client connects to. A preflight lookup followed by an independent
/// connect could resolve differently on the second lookup and reach an internal
/// endpoint, so this resolver is the only resolver the remote client uses: it
/// applies to the original request and to every redirect hop the client follows.
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let resolved = tauri::async_runtime::spawn_blocking(move || resolve_public_host(&host))
                .await
                .map_err(|_| {
                    Box::new(io::Error::other("host resolution task failed"))
                        as Box<dyn std::error::Error + Send + Sync>
                })?
                .map_err(|code| {
                    Box::new(io::Error::other(format!("host resolution refused: {code}")))
                        as Box<dyn std::error::Error + Send + Sync>
                })?;
            Ok(Box::new(resolved.into_iter()) as Addrs)
        })
    }
}

/// One guarded HTTP client for accepted public remote sources.
///
/// Every host resolves through [`PublicOnlyResolver`], and system or
/// environment proxy settings are ignored: a proxy would resolve and reach the
/// target on its own, outside the address policy this boundary enforces.
fn remote_http_client() -> Result<Client, Value> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(NETWORK_TIMEOUT)
        .no_proxy()
        .dns_resolver(Arc::new(PublicOnlyResolver))
        .redirect(Policy::custom(|attempt| {
            if !redirect_target_allowed(attempt.previous().len(), attempt.url()) {
                return attempt.stop();
            }
            attempt.follow()
        }))
        .build()
        .map_err(|_| {
            remote_source_error(
                "remote_client_failed",
                "Network access could not be initialized.",
            )
        })
}

fn analyze_github_source(
    client: &Client,
    mode: &str,
    source: &NormalizedRemoteSource,
    include_prereleases: bool,
) -> Result<GitHubAnalysis, Value> {
    analyze_github_source_with_api_root(
        client,
        mode,
        source,
        include_prereleases,
        GITHUB_API_ROOT,
        current_unix_epoch_seconds(),
    )
}

/// Analyze a GitHub source through an injected private API root.
///
/// Production callers always use [`GITHUB_API_ROOT`]. The injection point keeps
/// deterministic HTTP tests offline without exposing a runtime endpoint override.
fn analyze_github_source_with_api_root(
    client: &Client,
    mode: &str,
    source: &NormalizedRemoteSource,
    _include_prereleases: bool,
    api_root: &str,
    now_epoch_seconds: u64,
) -> Result<GitHubAnalysis, Value> {
    let repository = source.repository.as_deref().ok_or_else(|| {
        remote_source_error("github_url_invalid", "The GitHub repository is missing.")
    })?;
    let api_root = api_root.trim_end_matches('/');
    let repository_value = get_github_json_bounded(
        client,
        &format!("{api_root}/repos/{repository}"),
        now_epoch_seconds,
    )?;
    let releases_value = if mode == "github_release" {
        let tag = source.release_tag.as_deref().unwrap_or_default();
        Value::Array(vec![get_github_json_bounded(
            client,
            &format!("{api_root}/repos/{repository}/releases/tags/{tag}"),
            now_epoch_seconds,
        )?])
    } else {
        get_github_json_bounded(
            client,
            &format!("{api_root}/repos/{repository}/releases?per_page=30"),
            now_epoch_seconds,
        )?
    };
    let mut releases = Vec::new();
    for release in releases_value.as_array().into_iter().flatten() {
        if release.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let prerelease = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let mut assets = Vec::new();
        for asset in release
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = asset.get("name").and_then(Value::as_str) else {
                continue;
            };
            let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
            let Some(download_url) = asset.get("browser_download_url").and_then(Value::as_str)
            else {
                continue;
            };
            if !name.to_ascii_lowercase().ends_with(".apk") || size == 0 || size > MAX_APK_BYTES {
                continue;
            }
            let parsed = Url::parse(download_url).ok();
            if !parsed.as_ref().is_some_and(safe_remote_url) {
                continue;
            }
            assets.push(GitHubAsset {
                file_name: name.to_string(),
                size,
                content_type: asset
                    .get("content_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                download_url: download_url.to_string(),
            });
        }
        releases.push(GitHubRelease {
            tag: tag.to_string(),
            name: release
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            prerelease,
            published_at: release
                .get("published_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            assets,
        });
    }
    Ok(GitHubAnalysis {
        repository: json!({
            "fullName": repository_value.get("full_name").cloned().unwrap_or_else(|| Value::String(repository.to_string())),
            "name": repository_value.get("name").cloned().unwrap_or(Value::Null),
            "description": repository_value.get("description").cloned().unwrap_or(Value::Null),
            "htmlUrl": source.url,
        }),
        releases,
    })
}

fn analyze_gitlab_source(
    client: &Client,
    mode: &str,
    source: &NormalizedRemoteSource,
    include_prereleases: bool,
) -> Result<GitHubAnalysis, Value> {
    let repository = source
        .repository
        .as_deref()
        .ok_or_else(|| provider_url_error("GitLab"))?;
    let encoded = url::form_urlencoded::byte_serialize(repository.as_bytes()).collect::<String>();
    let repository_value = get_provider_json_bounded(
        client,
        &format!("https://gitlab.com/api/v4/projects/{encoded}"),
        "GitLab",
    )?;
    let releases_value = if mode == "gitlab_release" {
        let tag = source.release_tag.as_deref().unwrap_or_default();
        let encoded_tag = url::form_urlencoded::byte_serialize(tag.as_bytes()).collect::<String>();
        Value::Array(vec![get_provider_json_bounded(
            client,
            &format!("https://gitlab.com/api/v4/projects/{encoded}/releases/{encoded_tag}"),
            "GitLab",
        )?])
    } else {
        get_provider_json_bounded(
            client,
            &format!("https://gitlab.com/api/v4/projects/{encoded}/releases?per_page=30"),
            "GitLab",
        )?
    };
    let mut releases = Vec::new();
    for release in releases_value.as_array().into_iter().flatten() {
        if release.get("upcoming_release").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let name = release.get("name").and_then(Value::as_str);
        let prerelease = likely_provider_prerelease(tag, name.unwrap_or_default());
        if mode == "gitlab_repository" && prerelease && !include_prereleases {
            continue;
        }
        let mut assets = Vec::new();
        for asset in release
            .get("assets")
            .and_then(|value| value.get("links"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(file_name) = asset.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(download_url) = asset
                .get("direct_asset_url")
                .or_else(|| asset.get("url"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if !file_name.to_ascii_lowercase().ends_with(".apk")
                || !Url::parse(download_url)
                    .ok()
                    .as_ref()
                    .is_some_and(safe_remote_url)
            {
                continue;
            }
            assets.push(GitHubAsset {
                file_name: file_name.to_string(),
                size: 0,
                content_type: None,
                download_url: download_url.to_string(),
            });
        }
        releases.push(GitHubRelease {
            tag: tag.to_string(),
            name: name.map(str::to_string),
            prerelease,
            published_at: release
                .get("released_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            assets,
        });
    }
    Ok(GitHubAnalysis {
        repository: json!({
            "fullName": repository_value.get("path_with_namespace").cloned().unwrap_or_else(|| Value::String(repository.to_string())),
            "name": repository_value.get("name").cloned().unwrap_or(Value::Null),
            "description": repository_value.get("description").cloned().unwrap_or(Value::Null),
            "htmlUrl": source.url,
        }),
        releases,
    })
}

fn analyze_forgejo_source(
    client: &Client,
    mode: &str,
    source: &NormalizedRemoteSource,
    include_prereleases: bool,
) -> Result<GitHubAnalysis, Value> {
    let repository = source
        .repository
        .as_deref()
        .ok_or_else(|| provider_url_error("Forgejo"))?;
    let base_url = source
        .base_url
        .as_deref()
        .ok_or_else(|| provider_url_error("Forgejo"))?;
    let api_root = format!(
        "{}/api/v1/repos/{repository}",
        base_url.trim_end_matches('/')
    );
    let repository_value = get_provider_json_bounded(client, &api_root, "Forgejo")?;
    let releases_value = if mode == "forgejo_release" {
        let tag = source.release_tag.as_deref().unwrap_or_default();
        Value::Array(vec![get_provider_json_bounded(
            client,
            &format!("{api_root}/releases/tags/{tag}"),
            "Forgejo",
        )?])
    } else {
        get_provider_json_bounded(client, &format!("{api_root}/releases?limit=30"), "Forgejo")?
    };
    let expected_host = source.url.host_str().unwrap_or_default();
    let mut releases = Vec::new();
    for release in releases_value.as_array().into_iter().flatten() {
        if release.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let prerelease = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if mode == "forgejo_repository" && prerelease && !include_prereleases {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let mut assets = Vec::new();
        for asset in release
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(file_name) = asset.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(download_url) = asset.get("browser_download_url").and_then(Value::as_str)
            else {
                continue;
            };
            let parsed = Url::parse(download_url).ok();
            if !file_name.to_ascii_lowercase().ends_with(".apk")
                || !parsed.as_ref().is_some_and(safe_remote_url)
                || parsed.as_ref().and_then(Url::host_str) != Some(expected_host)
            {
                continue;
            }
            let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
            if size > MAX_APK_BYTES {
                continue;
            }
            assets.push(GitHubAsset {
                file_name: file_name.to_string(),
                size,
                content_type: asset
                    .get("content_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                download_url: download_url.to_string(),
            });
        }
        releases.push(GitHubRelease {
            tag: tag.to_string(),
            name: release
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            prerelease,
            published_at: release
                .get("published_at")
                .or_else(|| release.get("created_at"))
                .and_then(Value::as_str)
                .map(str::to_string),
            assets,
        });
    }
    Ok(GitHubAnalysis {
        repository: json!({
            "fullName": repository_value.get("full_name").cloned().unwrap_or_else(|| Value::String(repository.to_string())),
            "name": repository_value.get("name").cloned().unwrap_or(Value::Null),
            "description": repository_value.get("description").cloned().unwrap_or(Value::Null),
            "htmlUrl": source.url,
        }),
        releases,
    })
}

fn likely_provider_prerelease(tag: &str, name: &str) -> bool {
    let value = format!("{tag} {name}").to_ascii_lowercase();
    ["alpha", "beta", "preview", "prerelease", "pre-release"]
        .iter()
        .any(|marker| value.contains(marker))
        || value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| part == "rc")
}

fn get_provider_json_bounded(client: &Client, url: &str, provider: &str) -> Result<Value, Value> {
    let mut response = client
        .get(url)
        .header(USER_AGENT, HTTP_USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|_| provider_request_error(provider))?;
    if !response.status().is_success() {
        return Err(provider_request_error(provider));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES)
    {
        return Err(provider_response_error(provider));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| provider_response_error(provider))?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(provider_response_error(provider));
    }
    serde_json::from_slice(&bytes).map_err(|_| provider_response_error(provider))
}

fn provider_request_error(provider: &str) -> Value {
    remote_source_error(
        "provider_request_failed",
        &format!("{provider} information could not be retrieved."),
    )
}

fn provider_response_error(provider: &str) -> Value {
    remote_source_error(
        "provider_response_invalid",
        &format!("{provider} returned information the generator could not safely process."),
    )
}

fn get_github_json_bounded(
    client: &Client,
    url: &str,
    now_epoch_seconds: u64,
) -> Result<Value, Value> {
    let mut response = client
        .get(url)
        .header(USER_AGENT, HTTP_USER_AGENT)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .send()
        .map_err(|_| {
            remote_source_error(
                "github_transport_failed",
                "GitHub could not be reached. Check the network connection and try again.",
            )
        })?;
    if !response.status().is_success() {
        return Err(github_status_error(
            response.status(),
            response.headers(),
            now_epoch_seconds,
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES)
    {
        return Err(remote_source_error(
            "github_response_too_large",
            "GitHub returned more information than the generator can safely process.",
        ));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            remote_source_error(
                "github_response_failed",
                "GitHub information could not be read.",
            )
        })?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(remote_source_error(
            "github_response_too_large",
            "GitHub returned more information than the generator can safely process.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        remote_source_error(
            "github_response_invalid",
            "GitHub returned information in an unexpected format.",
        )
    })
}

/// Convert a non-success GitHub response into a stable, redacted product error.
///
/// Classification uses only the HTTP status and validated numeric rate-limit
/// headers. GitHub response bodies and raw header maps never cross this boundary.
fn github_status_error(status: StatusCode, headers: &HeaderMap, now_epoch_seconds: u64) -> Value {
    let rate_limit_status = matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    );
    let remaining = parse_unsigned_header(headers, "x-ratelimit-remaining");
    let retry_after = parse_unsigned_header(headers, RETRY_AFTER.as_str())
        .filter(|seconds| *seconds <= MAX_RATE_LIMIT_ADVISORY_SECONDS);
    let primary_rate_limit = rate_limit_status && remaining == Some(0);
    let secondary_rate_limit = rate_limit_status && retry_after.is_some();

    if status == StatusCode::TOO_MANY_REQUESTS || primary_rate_limit || secondary_rate_limit {
        let reset_wait = primary_rate_limit
            .then(|| parse_unsigned_header(headers, "x-ratelimit-reset"))
            .flatten()
            .and_then(|reset| reset.checked_sub(now_epoch_seconds))
            .filter(|seconds| *seconds > 0 && *seconds <= MAX_RATE_LIMIT_ADVISORY_SECONDS);
        return remote_source_error(
            "github_rate_limited",
            &github_rate_limit_message(retry_after, reset_wait),
        );
    }

    if status == StatusCode::NOT_FOUND {
        return remote_source_error(
            "github_repository_unavailable",
            "The GitHub repository was not found or is not publicly accessible.",
        );
    }

    remote_source_error(
        "github_service_failed",
        "GitHub could not complete the request. Try again later.",
    )
}

/// Parse one decimal response header without accepting signs, units, or overflow.
fn parse_unsigned_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn github_rate_limit_message(retry_after: Option<u64>, reset_wait: Option<u64>) -> String {
    if let Some(seconds) = retry_after.filter(|seconds| *seconds > 0) {
        return format!(
            "GitHub API requests are rate-limited. You may be able to try again in about {seconds} seconds."
        );
    }
    if let Some(seconds) = reset_wait {
        let minutes = seconds.div_ceil(60);
        let unit = if minutes == 1 { "minute" } else { "minutes" };
        return format!(
            "GitHub API requests are rate-limited. You may be able to try again in about {minutes} {unit}."
        );
    }
    "GitHub API requests are rate-limited. Try again later.".to_string()
}

fn current_unix_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn stream_apk_response(mut response: Response, path: &Path) -> Result<(), Value> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| {
            remote_source_error(
                "remote_workspace_failed",
                "The temporary APK could not be created.",
            )
        })?;
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = response.read(&mut buffer).map_err(|_| {
            remote_source_error(
                "remote_download_failed",
                "The APK download was interrupted.",
            )
        })?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > MAX_APK_BYTES {
            return Err(remote_source_error(
                "remote_apk_size_invalid",
                "The selected APK is larger than 2 GiB.",
            ));
        }
        file.write_all(&buffer[..count]).map_err(|_| {
            remote_source_error(
                "remote_workspace_failed",
                "The temporary APK could not be written.",
            )
        })?;
    }
    if total == 0 {
        return Err(remote_source_error(
            "remote_apk_size_invalid",
            "The selected APK is empty.",
        ));
    }
    file.sync_all().map_err(|_| {
        remote_source_error(
            "remote_workspace_failed",
            "The temporary APK could not be finalized.",
        )
    })
}

fn response_file_name(response: &Response) -> Option<String> {
    let header = response.headers().get(CONTENT_DISPOSITION)?.to_str().ok()?;
    header.split(';').map(str::trim).find_map(|part| {
        part.strip_prefix("filename=")
            .map(|value| value.trim_matches(['\"', '\'']).to_string())
    })
}

fn clearly_non_apk_content_type(value: Option<&str>) -> bool {
    value.is_some_and(|content_type| {
        let content_type = content_type.to_ascii_lowercase();
        content_type.starts_with("text/")
            || content_type.contains("html")
            || content_type.contains("json")
            || content_type.starts_with("image/")
    })
}

fn remote_source_error(code: &str, message: &str) -> Value {
    api_error(code, message, json!({}))
}

#[tauri::command]
pub fn inspect_app_generator_apk(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
) -> Result<Value, String> {
    let (apk_path, identity) = {
        let registry = lock_registry(&state)?;
        let session = match registry.session(&session_handle) {
            Ok(session) => session,
            Err(error) => return Ok(error),
        };
        let Some(apk) = session.apks.get(&apk_handle) else {
            return Ok(invalid_handle_error());
        };
        (apk.path.clone(), apk.identity.clone())
    };
    if current_file_identity(&apk_path).as_ref() != Some(&identity) {
        return Ok(apk_changed_error());
    }
    let response = request_sidecar(
        &sidecar,
        "inspectApk",
        Some(native_inspection_payload(&apk_path)),
    )?;
    let Some(inspection) = success_result(&response).cloned() else {
        return Ok(response);
    };
    let Some(facts) = facts_from_native_inspection(&inspection) else {
        return Ok(generator_protocol_error());
    };
    if current_file_identity(&apk_path).as_ref() != Some(&identity) {
        return Ok(apk_changed_error());
    }
    let mut registry = lock_registry(&state)?;
    let inspection_handle = registry.allocate("apk-inspection");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    let Some(apk) = session.apks.get_mut(&apk_handle) else {
        return Ok(invalid_handle_error());
    };
    apk.inspection = Some(inspection.clone());
    apk.inspection_handle = Some(inspection_handle.clone());
    apk.facts = Some(facts);
    let Some(mut public_inspection) = apk.inspection.clone() else {
        return Ok(generator_protocol_error());
    };
    let Some(public_inspection) = public_inspection.as_object_mut() else {
        return Ok(generator_protocol_error());
    };
    public_inspection.insert(
        "inspectionHandle".to_string(),
        Value::String(inspection_handle),
    );
    Ok(success(Value::Object(public_inspection.clone())))
}

fn native_inspection_payload(apk_path: &Path) -> Value {
    json!({ "apkPath": apk_path })
}

/// Build one non-committable live preview of a local-APK App and Recipe pair.
///
/// The preview shares the review request shape, including the retained
/// existing-App snapshot and the author's explicit identities, but retains no
/// native state and never writes to the catalog. An explicit appArtifactId or
/// recipeId is forwarded unchanged so the preview shows the exact identities a
/// later review would publish under; omitted identities keep the backend's
/// proposed defaults and are never renamed, suffixed, or replaced in between.
/// Permission automation is opt-in: `permissionAutomationEnabled` defaults to
/// false and preserves every declared permission set, while a disabled request
/// refuses any selected permission actions.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn generate_app_recipe_draft(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    permission_selection: Option<Value>,
    permission_automation_enabled: Option<bool>,
    regenerate_identifiers: bool,
    root_handle: Option<String>,
    request_id: i64,
    existing_app_handle: Option<String>,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    artifact_name: Option<String>,
) -> Result<Value, String> {
    // A live preview carries no review identity: the request id only orders the
    // author's preview revisions, and the display name never changes identities.
    let _ = (request_id, artifact_name);
    let transport = SidecarTransport { sidecar: &sidecar };
    let inputs = match resolve_draft_inputs(
        &state,
        &session_handle,
        &apk_handle,
        root_handle.as_deref(),
        false,
        existing_app_handle.as_deref(),
        permission_selection,
        permission_automation_enabled.unwrap_or(false),
        None,
    )? {
        Ok(inputs) => inputs,
        Err(error) => return Ok(error),
    };
    generate_draft_via_transport(
        &transport,
        inputs,
        "generateAppRecipeDraft",
        app_artifact_id,
        recipe_id,
        app,
        recipe,
        mappings,
        regenerate_identifiers,
    )
}

/// Build one non-committable live preview of a remote-source App and Recipe pair.
///
/// The trusted remote payload is rebuilt from the retained source and asset, so
/// only the author's asset choices and explicit identities cross the command
/// boundary; provider facts, release analysis, download URLs, and publisher
/// checksums stay native-owned.
/// Permission automation is opt-in: `permissionAutomationEnabled` defaults to
/// false and preserves every declared permission set, while a disabled request
/// refuses any selected permission actions.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn generate_remote_app_recipe_draft(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    asset_handle: String,
    strategy: String,
    asset_pattern: Option<String>,
    include_prereleases: bool,
    trusted_sha256: Option<String>,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    permission_selection: Option<Value>,
    permission_automation_enabled: Option<bool>,
    regenerate_identifiers: bool,
    root_handle: Option<String>,
    request_id: i64,
    existing_app_handle: Option<String>,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    artifact_name: Option<String>,
) -> Result<Value, String> {
    // A live preview carries no review identity: the request id only orders the
    // author's preview revisions, and the display name never changes identities.
    let _ = (request_id, artifact_name);
    let transport = SidecarTransport { sidecar: &sidecar };
    let remote = RemoteReviewSelection {
        asset_handle: &asset_handle,
        strategy: &strategy,
        asset_pattern: asset_pattern.as_deref(),
        include_prereleases,
        trusted_sha256: trusted_sha256.as_deref(),
    };
    let inputs = match resolve_draft_inputs(
        &state,
        &session_handle,
        &apk_handle,
        root_handle.as_deref(),
        false,
        existing_app_handle.as_deref(),
        permission_selection,
        permission_automation_enabled.unwrap_or(false),
        Some(remote),
    )? {
        Ok(inputs) => inputs,
        Err(error) => return Ok(error),
    };
    generate_draft_via_transport(
        &transport,
        inputs,
        "generateRemoteAppRecipeDraft",
        app_artifact_id,
        recipe_id,
        app,
        recipe,
        mappings,
        regenerate_identifiers,
    )
}

/// Refuse legacy direct publication of a remote-source App and Recipe pair.
///
/// The retired command accepted App, Recipe, mapping, and permission inputs at
/// save time, so nothing bound the published bytes to a reviewed candidate. It
/// stays registered so earlier callers receive an explicit review_required
/// refusal instead of an unknown-command error; publication now runs only
/// through save_app_generator_review, and no alternative write authority exists.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_generated_remote_app_recipe(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    asset_handle: String,
    strategy: String,
    asset_pattern: Option<String>,
    include_prereleases: bool,
    trusted_sha256: Option<String>,
    root_handle: String,
    app: Value,
    recipe: Value,
    mappings: Value,
    permission_selection: Option<Value>,
) -> Result<Value, String> {
    let _ = (
        sidecar,
        state,
        session_handle,
        apk_handle,
        asset_handle,
        strategy,
        asset_pattern,
        include_prereleases,
        trusted_sha256,
        root_handle,
        app,
        recipe,
        mappings,
        permission_selection,
    );
    Ok(retired_direct_publication_refusal())
}

#[derive(Debug)]
struct TrustedRemoteDraftContext {
    source: Value,
    release_analysis: Option<Value>,
    /// Deterministic serialization of the trusted payload pieces.
    ///
    /// Reviews retain this fingerprint and re-derive it before publication, so a
    /// re-analyzed repository or replaced asset invalidates the review instead of
    /// publishing a pair generated from different provider facts.
    source_fingerprint: String,
}

/// Build the trusted remote source payload from one retained source and asset.
///
/// Remote handling stays inside the native boundary: the sidecar receives only
/// normalized provider facts, never a signed redirect or CDN URL supplied by the
/// frontend, and a trusted publisher checksum is limited to pinned assets.
fn remote_source_context(
    source: &TrustedRemoteSource,
    asset: &TrustedRemoteAsset,
    strategy: &str,
    asset_pattern: Option<&str>,
    include_prereleases: bool,
    trusted_sha256: Option<&str>,
) -> Result<TrustedRemoteDraftContext, Value> {
    let release_analysis = if strategy == "latest_compatible_release" {
        if source.mode != "github_repository" || source.repository.is_none() {
            return Err(remote_source_error(
                "latest_release_source_unsupported",
                "Latest compatible release currently requires a GitHub repository source.",
            ));
        }
        let Some(releases) = source.release_analysis.as_ref() else {
            return Err(remote_source_error(
                "latest_release_analysis_missing",
                "Analyze the GitHub repository again before generating a latest-compatible recipe.",
            ));
        };
        Some(trusted_release_analysis_value(releases)?)
    } else {
        None
    };
    let mut payload = json!({
        "mode": source.mode,
        "strategy": strategy,
        "downloadUrl": source.direct_url.as_ref().unwrap_or(&asset.download_url),
        "provider": source.provider,
        "baseUrl": source.base_url,
        "repository": source.repository,
        "releaseTag": asset.release_tag.as_ref().or(source.release_tag.as_ref()),
        "assetName": asset.file_name,
        "assetPattern": asset_pattern,
        "includePrereleases": include_prereleases,
    });
    if let Some(trusted_sha256) = trusted_sha256 {
        payload["trustedSha256"] = Value::String(trusted_sha256.to_string());
    }
    let source_fingerprint = format!(
        "{payload}\u{1f}{}",
        release_analysis
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default()
    );
    Ok(TrustedRemoteDraftContext {
        source: payload,
        release_analysis,
        source_fingerprint,
    })
}

/// Resolve one retained remote asset into its trusted draft context.
///
/// The session owns every provider fact: the request only names the retained
/// asset and the author's strategy choices, and a strategy the generator cannot
/// install, a missing asset, or a missing source is refused here. Review
/// re-validation uses the same resolution, so a re-analyzed source changes the
/// fingerprint and invalidates an outstanding review instead of publishing a
/// pair generated from provider facts the author never saw.
fn retained_remote_context(
    session: &GeneratorSession,
    asset_handle: &str,
    strategy: &str,
    asset_pattern: Option<&str>,
    include_prereleases: bool,
    trusted_sha256: Option<&str>,
) -> Result<TrustedRemoteDraftContext, Value> {
    if !remote_strategy_supported(strategy) {
        return Err(remote_source_error(
            "remote_strategy_invalid",
            "Choose a supported installation method.",
        ));
    }
    let trusted_sha256 = trusted_sha256_request_value(strategy, trusted_sha256)?;
    let Some(asset) = session.remote_assets.get(asset_handle) else {
        return Err(invalid_handle_error());
    };
    let Some(source) = session.remote_sources.get(&asset.source_handle) else {
        return Err(invalid_handle_error());
    };
    remote_source_context(
        source,
        asset,
        strategy,
        asset_pattern,
        include_prereleases,
        trusted_sha256,
    )
}

fn trusted_release_analysis_value(releases: &[TrustedRemoteRelease]) -> Result<Value, Value> {
    let mut release_tags = HashSet::new();
    let mut serialized = Vec::with_capacity(releases.len());
    for release in releases {
        if release.tag.trim().is_empty() || !release_tags.insert(release.tag.as_str()) {
            return Err(remote_source_error(
                "latest_release_analysis_invalid",
                "The analyzed GitHub release information is ambiguous. Check the source again.",
            ));
        }
        let mut asset_names = HashSet::new();
        for asset_name in &release.asset_file_names {
            if asset_name.trim().is_empty()
                || !asset_name.to_ascii_lowercase().ends_with(".apk")
                || !asset_names.insert(asset_name.as_str())
            {
                return Err(remote_source_error(
                    "latest_release_analysis_invalid",
                    "The analyzed GitHub release information is ambiguous. Check the source again.",
                ));
            }
        }
        serialized.push(json!({
            "releaseTag": release.tag,
            "prerelease": release.prerelease,
            "assetFileNames": release.asset_file_names,
        }));
    }
    Ok(json!({ "releases": serialized }))
}

fn trusted_sha256_request_value<'a>(
    strategy: &str,
    trusted_sha256: Option<&'a str>,
) -> Result<Option<&'a str>, Value> {
    let trusted_sha256 = trusted_sha256.filter(|value| {
        !value
            .trim_matches(|character: char| character.is_ascii_whitespace())
            .is_empty()
    });
    if trusted_sha256.is_some() && strategy != "pinned_remote_asset" {
        return Err(remote_source_error(
            "apk_trusted_sha256_strategy_unsupported",
            "A trusted publisher SHA-256 is supported only for pinned remote APK assets.",
        ));
    }
    Ok(trusted_sha256)
}

#[tauri::command]
pub fn check_app_recipe_collisions() -> Result<Value, String> {
    Ok(api_error(
        "app_recipe_collision_review_requires_generation",
        "Review collisions by generating a fresh app-and-recipe draft.",
        json!({}),
    ))
}

/// Refuse legacy direct publication of a local-APK App and Recipe pair.
///
/// The retired command accepted App, Recipe, mapping, and permission inputs at
/// save time, so nothing bound the published bytes to a reviewed candidate. It
/// stays registered so earlier callers receive an explicit review_required
/// refusal instead of an unknown-command error; publication now runs only
/// through save_app_generator_review, and no alternative write authority exists.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_generated_app_recipe(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    root_handle: String,
    app: Value,
    recipe: Value,
    mappings: Value,
    permission_selection: Option<Value>,
) -> Result<Value, String> {
    let _ = (
        sidecar,
        state,
        session_handle,
        apk_handle,
        root_handle,
        app,
        recipe,
        mappings,
        permission_selection,
    );
    Ok(retired_direct_publication_refusal())
}

#[tauri::command]
pub fn cancel_app_generator(
    state: State<'_, AppGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    // Cancelling discards every session handle and review, because those
    // describe drafts the author abandoned. Sticky publication records are kept:
    // an unconfirmed attempt must remain checkable, and cancelling must never
    // turn an uncertain publication into an apparently never-attempted one.
    let mut registry = lock_registry(&state)?;
    if registry.sessions.remove(&session_handle).is_none() {
        return Ok(expired_session_error());
    }
    Ok(success(json!({})))
}

/// Remote source selection fields one preview or review request carries.
///
/// The values are the author's choices about which retained asset to use; every
/// provider fact, release analysis entry, and download URL comes from the
/// trusted native source state instead of the request.
struct RemoteReviewSelection<'a> {
    asset_handle: &'a str,
    strategy: &'a str,
    asset_pattern: Option<&'a str>,
    include_prereleases: bool,
    trusted_sha256: Option<&'a str>,
}

/// Trusted inputs one preview or review request resolved from native state.
#[derive(Debug)]
struct ResolvedDraftInputs {
    /// Authored root the request targets; a preview may run without one.
    root: Option<TrustedRoot>,
    facts: Value,
    permission_automation: Option<Value>,
    /// Explicit author opt-in for permission automation, forwarded as its own
    /// request field so the backend never infers intent from selected identities.
    ///
    /// An opt-in that selected no identities still travels: it allows the
    /// backend to reference the whole declared permission sets. A disabled
    /// request grants nothing and refuses nonempty selections.
    permission_automation_enabled: bool,
    existing_app: Option<BoundExistingApp>,
    /// Dependency fingerprint retained for review requests only.
    binding: Option<ReviewBinding>,
    remote: Option<ResolvedRemoteInputs>,
}

/// Trusted remote payload pieces derived from the retained source and asset.
#[derive(Debug)]
struct ResolvedRemoteInputs {
    source: Value,
    release_analysis: Option<Value>,
}

/// Resolve the authored root one existing-App request operates on.
///
/// The most recently selected root is the session's active root. An explicit
/// handle always wins, so a workflow that captured a handle keeps using the root
/// it captured, and a request cannot silently retarget another catalog.
fn resolve_session_root(
    state: &AppGeneratorState,
    session_handle: &str,
    root_handle: Option<&str>,
) -> Result<Result<(String, TrustedRoot), Value>, String> {
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(Err(error)),
    };
    let handle = match root_handle {
        Some(handle) => handle,
        None => match session.active_root.as_deref() {
            Some(handle) => handle,
            None => return Ok(Err(invalid_root_error())),
        },
    };
    Ok(session
        .roots
        .get(handle)
        .cloned()
        .map(|root| (handle.to_string(), root))
        .ok_or_else(invalid_handle_error))
}

/// Package name the trusted APK inspection recorded for the selected APK.
fn inspected_package_name(apk: &TrustedApk) -> Option<&str> {
    apk.inspection
        .as_ref()?
        .get("manifest")?
        .get("packageName")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Look up one native existing-App handle and enforce its trusted bindings.
///
/// The selection is bound to the root it was captured against, and the inspected
/// APK package must match the selected App Definition package: generation
/// installs the inspected APK into that App Definition, so a mismatch can never
/// produce an installable pair.
fn trusted_existing_app(
    session: &GeneratorSession,
    apk: &TrustedApk,
    handle: &str,
    expected_root: Option<&TrustedRoot>,
) -> Result<BoundExistingApp, Value> {
    let Some(selection) = session.existing_apps.get(handle) else {
        return Err(invalid_handle_error());
    };
    if let Some(root) = expected_root {
        if selection.root_path != root.root {
            return Err(existing_app_outside_root_error());
        }
    }
    if let (Some(inspected), Some(selected)) = (
        inspected_package_name(apk),
        selection
            .package_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        if inspected != selected {
            return Err(package_mismatch_error(inspected, selected));
        }
    }
    Ok(BoundExistingApp {
        handle: handle.to_string(),
        selection: selection.clone(),
    })
}

/// Resolve every trusted fact one preview or review request needs.
///
/// Nothing in this boundary is trusted from the frontend: APK facts, permission
/// automation bounds, remote provider payloads, and existing-App snapshots all
/// come from native session state captured through the inspection and analysis
/// flows. `require_root` is false for previews so an author can still see a
/// non-committable draft before choosing a root.
///
/// The `permission_automation_enabled` flag is the author's explicit opt-in for
/// permission automation. A disabled request preserves every declared App
/// permission set and adds no grant steps, so a nonempty identity selection is
/// refused as contradictory before any request reaches the backend.
#[allow(clippy::too_many_arguments)]
fn resolve_draft_inputs(
    state: &AppGeneratorState,
    session_handle: &str,
    apk_handle: &str,
    root_handle: Option<&str>,
    require_root: bool,
    existing_app_handle: Option<&str>,
    permission_selection: Option<Value>,
    permission_automation_enabled: bool,
    remote_inputs: Option<RemoteReviewSelection<'_>>,
) -> Result<Result<ResolvedDraftInputs, Value>, String> {
    if let Some(selection) = &remote_inputs {
        if !remote_strategy_supported(selection.strategy) {
            return Ok(Err(remote_source_error(
                "remote_strategy_invalid",
                "Choose a supported installation method.",
            )));
        }
    }
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(Err(error)),
    };
    let root = match root_handle {
        Some(handle) => match session.roots.get(handle) {
            Some(root) => Some((handle.to_string(), root.clone())),
            None => return Ok(Err(invalid_handle_error())),
        },
        None => match session.active_root.as_deref().and_then(|handle| {
            session
                .roots
                .get(handle)
                .map(|root| (handle.to_string(), root.clone()))
        }) {
            Some(resolved) => Some(resolved),
            None if require_root => return Ok(Err(invalid_root_error())),
            None => None,
        },
    };
    let Some(apk) = session.apks.get(apk_handle) else {
        return Ok(Err(invalid_handle_error()));
    };
    // An inspected APK and a remote install source must come from the same
    // retained download. Two independently valid handles are not proof that the
    // bytes the author inspected are the bytes the recipe will install, so a
    // mismatched pair is refused before any review request reaches the backend.
    if let Some(selection) = &remote_inputs {
        if apk.remote_asset_handle.as_deref() != Some(selection.asset_handle) {
            return Ok(Err(remote_source_error(
                "remote_asset_mismatch",
                "The inspected APK does not come from the selected release asset. Download and inspect the selected APK again.",
            )));
        }
    }
    let Some(facts) = apk.facts.clone() else {
        return Ok(Err(inspection_required_error()));
    };
    let Some(inspection_handle) = apk.inspection_handle.clone() else {
        return Ok(Err(inspection_required_error()));
    };
    let existing_app = match existing_app_handle {
        Some(handle) => {
            match trusted_existing_app(session, apk, handle, root.as_ref().map(|(_, root)| root)) {
                Ok(bound) => Some(bound),
                Err(error) => return Ok(Err(error)),
            }
        }
        None => None,
    };
    if current_file_identity(&apk.path).as_ref() != Some(&apk.identity) {
        return Ok(Err(apk_changed_error()));
    }
    // Permission automation is derived from the trusted inspection of the APK the
    // draft installs. A local APK is that inspected artifact, and a remote source
    // is eligible when its strategy re-resolves the inspected artifact at
    // execution time. A user-provided remote artifact has no inspected identity
    // to attach the grants to, so it stays refused.
    let installs_inspected_apk = match &remote_inputs {
        Some(selection) => matches!(
            selection.strategy,
            "pinned_remote_asset" | "latest_compatible_release"
        ),
        None => true,
    };
    let permission_automation = match canonical_permission_automation(
        apk,
        permission_selection,
        installs_inspected_apk,
        permission_automation_enabled,
    ) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    let (remote, binding) = match &remote_inputs {
        Some(selection) => {
            let context = match retained_remote_context(
                session,
                selection.asset_handle,
                selection.strategy,
                selection.asset_pattern,
                selection.include_prereleases,
                selection.trusted_sha256,
            ) {
                Ok(context) => context,
                Err(error) => return Ok(Err(error)),
            };
            (
                Some(ResolvedRemoteInputs {
                    source: context.source,
                    release_analysis: context.release_analysis,
                }),
                ReviewBinding::Remote {
                    apk_handle: apk_handle.to_string(),
                    apk_identity: apk.identity.clone(),
                    inspection_handle: inspection_handle.clone(),
                    asset_handle: selection.asset_handle.to_string(),
                    strategy: selection.strategy.to_string(),
                    asset_pattern: selection.asset_pattern.map(str::to_string),
                    include_prereleases: selection.include_prereleases,
                    trusted_sha256: selection.trusted_sha256.map(str::to_string),
                    source_fingerprint: context.source_fingerprint,
                },
            )
        }
        None => (
            None,
            ReviewBinding::Local {
                apk_handle: apk_handle.to_string(),
                apk_identity: apk.identity.clone(),
                inspection_handle: inspection_handle.clone(),
            },
        ),
    };
    Ok(Ok(ResolvedDraftInputs {
        root: root.map(|(_, root)| root),
        facts,
        permission_automation,
        permission_automation_enabled,
        existing_app,
        binding: Some(binding),
        remote,
    }))
}

/// Supported installation strategies for remote app generation.
fn remote_strategy_supported(strategy: &str) -> bool {
    matches!(
        strategy,
        "pinned_remote_asset" | "latest_compatible_release" | "user_provided_apk"
    )
}

/// Copy explicit author-chosen identities into one request payload.
///
/// Omitted identifiers keep the backend's proposed default. The native boundary
/// never renames, suffixes, or assigns a different identity on its own, and a
/// blank value is treated as omitted instead of becoming an empty identifier.
fn insert_explicit_identity(
    payload: &mut Map<String, Value>,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
) {
    if let Some(value) = app_artifact_id.filter(|value| !value.trim().is_empty()) {
        payload.insert("appArtifactId".to_string(), Value::String(value));
    }
    if let Some(value) = recipe_id.filter(|value| !value.trim().is_empty()) {
        payload.insert("recipeId".to_string(), Value::String(value));
    }
}

/// Catalog-relative destination path one review destination carries.
fn destination_relative_path(object: &Map<String, Value>, kind: &str) -> Option<String> {
    object
        .get("destinations")?
        .get(kind)?
        .get("relativePath")?
        .as_str()
        .map(str::to_string)
}

/// Reject review results whose catalog paths are absolute or escape the root.
///
/// Every path the author sees stays catalog-relative; a backend payload that
/// carries a native absolute path is a protocol violation instead of something
/// the UI layer has to interpret.
fn validate_review_paths(object: &Map<String, Value>) -> Result<(), Value> {
    if let Some(destinations) = object.get("destinations").and_then(Value::as_object) {
        for kind in ["app", "recipe"] {
            if let Some(relative) = destinations
                .get(kind)
                .and_then(|destination| destination.get("relativePath"))
                .and_then(Value::as_str)
            {
                if !is_catalog_relative_path(relative) {
                    return Err(invalid_destination_error());
                }
            }
        }
    }
    if let Some(app_path) = object
        .get("target")
        .and_then(|target| target.get("appPath"))
        .and_then(Value::as_str)
    {
        if !is_catalog_relative_path(app_path) {
            return Err(invalid_destination_error());
        }
    }
    if let Some(affected) = object
        .get("changeImpact")
        .and_then(|impact| impact.get("affectedRecipes"))
        .and_then(Value::as_array)
    {
        for recipe in affected {
            if let Some(path) = recipe.get("recipePath").and_then(Value::as_str) {
                if !is_catalog_relative_path(path) {
                    return Err(invalid_destination_error());
                }
            }
        }
    }
    Ok(())
}

/// Build one immutable review, retain its native mapping, and map the result.
///
/// The backend review handle stays in native memory: the frontend receives a
/// native review handle plus the frozen review payload. The retained mapping is
/// the only thing save can publish, so no later request can alter the reviewed
/// App, Recipe, mappings, permissions, or identities. A backend response that
/// retained no review snapshot is reported as blocking, because native can
/// never publish a review the backend cannot pair.
#[allow(clippy::too_many_arguments)]
fn review_draft_via_transport(
    transport: &dyn AppGeneratorTransport,
    state: &AppGeneratorState,
    session_handle: &str,
    inputs: ResolvedDraftInputs,
    request_id: i64,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    artifact_name: Option<String>,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let (Some(binding), Some(root)) = (inputs.binding.clone(), inputs.root.clone()) else {
        return Ok(generator_protocol_error());
    };
    // Review and preview share one binding rule: an existing-App target and an
    // App Definition edit cannot describe the same draft, because the existing
    // snapshot already owns the App bytes the backend substitutes.
    if inputs.existing_app.is_some() && app.is_some() {
        return Ok(invalid_argument_error(
            "A draft cannot combine an App Definition edit with an existing-App selection.",
        ));
    }
    let operation = if inputs.remote.is_some() {
        "reviewRemoteAppGeneratorDraft"
    } else {
        "reviewAppGeneratorDraft"
    };
    let mut payload = Map::new();
    payload.insert("authoredRoot".to_string(), json!(root.root));
    payload.insert("requestId".to_string(), json!(request_id));
    payload.insert("facts".to_string(), inputs.facts.clone());
    if let Some(existing) = &inputs.existing_app {
        payload.insert(
            "existingAppSnapshotId".to_string(),
            Value::String(existing.selection.snapshot_id.clone()),
        );
    }
    insert_explicit_identity(&mut payload, app_artifact_id, recipe_id);
    if let Some(value) = artifact_name.filter(|value| !value.trim().is_empty()) {
        payload.insert("artifactName".to_string(), Value::String(value));
    }
    for (field, value) in [("app", app), ("recipe", recipe), ("mappings", mappings)] {
        if let Some(value) = value {
            payload.insert(field.to_string(), value);
        }
    }
    if let Some(value) = inputs.permission_automation.clone() {
        payload.insert("permissionAutomation".to_string(), value);
    }
    payload.insert(
        "permissionAutomationEnabled".to_string(),
        Value::Bool(inputs.permission_automation_enabled),
    );
    payload.insert(
        "regenerateIdentifiers".to_string(),
        Value::Bool(regenerate_identifiers),
    );
    if let Some(remote) = &inputs.remote {
        payload.insert("source".to_string(), remote.source.clone());
        if let Some(release_analysis) = &remote.release_analysis {
            payload.insert("releaseAnalysis".to_string(), release_analysis.clone());
        }
    }
    let response = transport.request(operation, Value::Object(payload))?;
    let Some(result) = success_result(&response).cloned() else {
        return Ok(response);
    };
    let Some(object) = result.as_object() else {
        return Ok(generator_protocol_error());
    };
    if object.get("requestId").and_then(Value::as_i64) != Some(request_id) {
        return Ok(generator_protocol_error());
    }
    let Some(blocking) = object.get("blocking").and_then(Value::as_bool) else {
        return Ok(generator_protocol_error());
    };
    if let Err(error) = validate_review_paths(object) {
        return Ok(error);
    }
    let backend_review_id = object
        .get("reviewId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let record = ReviewRecord {
        request_id,
        backend_review_id: backend_review_id.clone(),
        binding,
        existing_app: inputs.existing_app.clone(),
        root_path: root.root.clone(),
        // A review the backend could not retain a pair snapshot for can never be
        // published, so it counts as blocking whatever the diagnostics report.
        blocking: blocking || backend_review_id.is_none(),
        app_relative_path: destination_relative_path(object, "app"),
        recipe_relative_path: destination_relative_path(object, "recipe"),
    };
    let mut registry = lock_registry(state)?;
    let review_handle = registry.allocate("app-review");
    let session = match registry.session_mut(session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    // The public flag mirrors the save gate: a review the backend did not retain
    // can never be published, so the UI must not offer publication even when the
    // diagnostics come back clear.
    let effective_blocking = record.blocking;
    session.reviews.insert(review_handle.clone(), record);
    let mut public = object.clone();
    public.remove("reviewId");
    public.insert("reviewHandle".to_string(), Value::String(review_handle));
    public.insert("blocking".to_string(), Value::Bool(effective_blocking));
    Ok(success(Value::Object(public)))
}

/// Build one non-committable preview from resolved trusted inputs.
///
/// Previews share the review request shape, including the retained existing-App
/// snapshot, but retain nothing native-side and never write to the catalog.
#[allow(clippy::too_many_arguments)]
fn generate_draft_via_transport(
    transport: &dyn AppGeneratorTransport,
    inputs: ResolvedDraftInputs,
    operation: &str,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let ResolvedDraftInputs {
        root,
        facts,
        permission_automation,
        permission_automation_enabled,
        existing_app,
        remote,
        ..
    } = inputs;
    // Review and preview share one binding rule: an existing-App target and an
    // App Definition edit cannot describe the same draft, because the existing
    // snapshot already owns the App bytes the backend substitutes.
    if existing_app.is_some() && app.is_some() {
        return Ok(invalid_argument_error(
            "A draft cannot combine an App Definition edit with an existing-App selection.",
        ));
    }
    let mut payload = Map::new();
    payload.insert("facts".to_string(), facts);
    if let Some(existing) = &existing_app {
        // The preview resolves the retained snapshot exactly like a review
        // request: the frontend never supplies App Definition facts for an
        // existing-App target.
        payload.insert(
            "authoredRoot".to_string(),
            json!(existing.selection.root_path),
        );
        payload.insert(
            "existingAppSnapshotId".to_string(),
            Value::String(existing.selection.snapshot_id.clone()),
        );
    }
    insert_explicit_identity(&mut payload, app_artifact_id, recipe_id);
    for (field, value) in [("app", app), ("recipe", recipe), ("mappings", mappings)] {
        if let Some(value) = value {
            payload.insert(field.to_string(), value);
        }
    }
    if let Some(value) = permission_automation {
        payload.insert("permissionAutomation".to_string(), value);
    }
    payload.insert(
        "permissionAutomationEnabled".to_string(),
        Value::Bool(permission_automation_enabled),
    );
    payload.insert(
        "regenerateIdentifiers".to_string(),
        Value::Bool(regenerate_identifiers),
    );
    if let Some(remote) = &remote {
        payload.insert("source".to_string(), remote.source.clone());
        if let Some(release_analysis) = &remote.release_analysis {
            payload.insert("releaseAnalysis".to_string(), release_analysis.clone());
        }
    }
    request_generated_draft(transport, operation, Value::Object(payload), root.as_ref())
}

/// Pre-publication gate outcome for one save attempt.
enum SaveGate {
    Expired,
    Invalidated,
    Blocking(Box<ReviewRecord>),
}

/// Re-check every trusted fact one review was generated from.
///
/// Reviews are invalidated, never silently refreshed: a changed APK file, a
/// replaced inspection, a re-analyzed remote source, a dropped root, or a
/// replaced existing-App selection all mean the author must review again.
fn review_dependencies_current(
    session: &GeneratorSession,
    record: &ReviewRecord,
) -> Result<(), ()> {
    if !session
        .roots
        .values()
        .any(|root| root.root == record.root_path)
    {
        return Err(());
    }
    let Some(apk) = session.apks.get(record.binding.apk_handle()) else {
        return Err(());
    };
    let (apk_identity, inspection_handle) = match &record.binding {
        ReviewBinding::Local {
            apk_identity,
            inspection_handle,
            ..
        }
        | ReviewBinding::Remote {
            apk_identity,
            inspection_handle,
            ..
        } => (apk_identity, inspection_handle),
    };
    if &apk.identity != apk_identity
        || apk.inspection_handle.as_deref() != Some(inspection_handle.as_str())
        || current_file_identity(&apk.path).as_ref() != Some(apk_identity)
    {
        return Err(());
    }
    if let ReviewBinding::Remote {
        asset_handle,
        strategy,
        asset_pattern,
        include_prereleases,
        trusted_sha256,
        source_fingerprint,
        ..
    } = &record.binding
    {
        let Ok(context) = retained_remote_context(
            session,
            asset_handle,
            strategy,
            asset_pattern.as_deref(),
            *include_prereleases,
            trusted_sha256.as_deref(),
        ) else {
            return Err(());
        };
        if &context.source_fingerprint != source_fingerprint {
            return Err(());
        }
    }
    if let Some(bound) = &record.existing_app {
        let Some(current) = session.existing_apps.get(&bound.handle) else {
            return Err(());
        };
        if current.snapshot_id != bound.selection.snapshot_id
            || current.app_path != bound.selection.app_path
            || current.root_path != bound.selection.root_path
        {
            return Err(());
        }
    }
    Ok(())
}

/// Update the sticky publication record for one native operation identity.
fn update_publication(
    state: &AppGeneratorState,
    operation_id: &str,
    status: PublicationState,
    detail: Option<Value>,
) -> Result<(), String> {
    let mut registry = lock_registry(state)?;
    if let Some(record) = registry.publications.get_mut(operation_id) {
        record.state = status;
        record.detail = detail;
    }
    Ok(())
}

/// Open the freshly published Recipe document after a confirmed save.
///
/// The catalog write already committed, so a failure here only reports that the
/// new file could not be opened in the editor. Publication is never repeated
/// because of it.
fn open_generated_recipe(
    transport: &dyn AppGeneratorTransport,
    record: &ReviewRecord,
) -> Result<String, Value> {
    let Some(recipe_relative) = record.recipe_relative_path.as_deref() else {
        return Err(open_failed_error(
            "The saved Recipe destination is unknown, so the generated Recipe could not be opened.",
        ));
    };
    let recipe_path = record.root_path.join(recipe_relative);
    let response = transport
        .request(
            "openRecipe",
            json!({ "path": recipe_path, "authoredRoot": record.root_path }),
        )
        .map_err(|_| {
            open_failed_error(
                "The generated Recipe was saved, but the editor backend is unavailable to open it.",
            )
        })?;
    let Some(result) = success_result(&response) else {
        return Err(response
            .get("error")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| open_failed_error("The saved Recipe could not be opened.")));
    };
    result
        .get("document")
        .and_then(|document| document.get("documentId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            open_failed_error(
                "The saved Recipe was opened, but its document identity was not reported.",
            )
        })
}

/// Publish one reviewed App/Recipe pair for the author.
///
/// The operation identity is recorded locally before the request is sent, so a
/// lost response stays an unconfirmed outcome instead of a confirmed failure and
/// is never retried automatically. Pre-publication refusals write nothing and
/// therefore drop the local record.
fn publish_review_via_transport(
    transport: &dyn AppGeneratorTransport,
    state: &AppGeneratorState,
    session_handle: &str,
    review_handle: &str,
    request_id: i64,
) -> Result<Value, String> {
    let (operation_id, record) = {
        let mut registry = lock_registry(state)?;
        let gate = match registry.sessions.get(session_handle) {
            None => Err(SaveGate::Expired),
            Some(session) => match session.reviews.get(review_handle) {
                None => Err(SaveGate::Invalidated),
                Some(record) => {
                    if record.request_id != request_id
                        || review_dependencies_current(session, record).is_err()
                    {
                        Err(SaveGate::Invalidated)
                    } else if record.blocking || record.backend_review_id.is_none() {
                        Err(SaveGate::Blocking(Box::new(record.clone())))
                    } else {
                        Ok(record.clone())
                    }
                }
            },
        };
        match gate {
            Err(SaveGate::Expired) => return Ok(expired_session_error()),
            Err(SaveGate::Invalidated) => {
                if let Some(session) = registry.sessions.get_mut(session_handle) {
                    session.reviews.remove(review_handle);
                }
                return Ok(review_invalidated_error());
            }
            Err(SaveGate::Blocking(record)) => return Ok(review_blocking_error(&record)),
            Ok(record) => {
                let operation_id = registry.allocate_publication();
                registry.publications.insert(
                    operation_id.clone(),
                    PublicationRecord {
                        state: PublicationState::Submitted,
                        recorded_at: iso8601_now(),
                        detail: None,
                        root_path: record.root_path.clone(),
                    },
                );
                // The review is consumed by this attempt unless the backend
                // refuses before publication and keeps its retained pair.
                if let Some(session) = registry.sessions.get_mut(session_handle) {
                    session.reviews.remove(review_handle);
                }
                (operation_id, record)
            }
        }
    };
    let Some(backend_review_id) = record.backend_review_id.clone() else {
        return Ok(review_blocking_error(&record));
    };
    let response = match transport.request(
        "saveAppGeneratorReview",
        json!({
            "operationId": operation_id,
            "reviewId": backend_review_id,
            "requestId": request_id,
        }),
    ) {
        Ok(response) => response,
        Err(_) => {
            return Ok(success(unknown_publication_value(
                &operation_id,
                UNCONFIRMED_PUBLICATION_MESSAGE,
            )))
        }
    };
    if success_result(&response).is_none() {
        // A pre-publication refusal wrote no files. Blocking or unsaved-document
        // refusals leave the backend review intact, so the native handle survives
        // for an attempt after the author resolves the refusal.
        let refusal_code = response
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str);
        let retain_review = matches!(
            refusal_code,
            Some("review_blocking") | Some("dirty_app_document")
        );
        let mut registry = lock_registry(state)?;
        registry.publications.remove(&operation_id);
        if retain_review {
            if let Some(session) = registry.sessions.get_mut(session_handle) {
                if review_dependencies_current(session, &record).is_ok() {
                    session.reviews.insert(review_handle.to_string(), record);
                }
            }
        }
        return Ok(response);
    }
    let result = success_result(&response).cloned().unwrap_or(Value::Null);
    match result.get("outcome").and_then(Value::as_str) {
        Some("saved") => {
            let app_path = record.app_relative_path.clone().unwrap_or_default();
            let recipe_path = record.recipe_relative_path.clone().unwrap_or_default();
            let mut payload = json!({
                "outcome": "saved",
                "operationId": operation_id,
                "app": { "appPath": app_path },
                "recipe": {
                    "recipePath": recipe_path,
                    "documentId": Value::Null,
                },
                "createdFiles": catalog_relative_file_list(result.get("createdFiles"), &record.root_path),
                "replacedFiles": catalog_relative_file_list(result.get("replacedFiles"), &record.root_path),
            });
            // The confirmed outcome is recorded before the Recipe is opened:
            // status must never depend on the editor document opening.
            update_publication(
                state,
                &operation_id,
                PublicationState::Saved,
                Some(payload.clone()),
            )?;
            match open_generated_recipe(transport, &record) {
                Ok(document_id) => {
                    payload["recipe"]["documentId"] = Value::String(document_id);
                }
                Err(open_error) => {
                    payload["recipe"]["openError"] = open_error;
                }
            }
            update_publication(
                state,
                &operation_id,
                PublicationState::Saved,
                Some(payload.clone()),
            )?;
            Ok(success(payload))
        }
        Some("failed") => {
            let payload = failed_outcome_value(&operation_id, &record.root_path, &result);
            update_publication(
                state,
                &operation_id,
                PublicationState::Failed,
                Some(payload.clone()),
            )?;
            Ok(success(payload))
        }
        _ => {
            let message = result
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(UNCONFIRMED_PUBLICATION_MESSAGE);
            let payload = unknown_publication_value(&operation_id, message);
            update_publication(
                state,
                &operation_id,
                PublicationState::Submitted,
                Some(payload.clone()),
            )?;
            Ok(success(payload))
        }
    }
}

/// Map one failed publication result onto the frozen outcome shape.
fn failed_outcome_value(operation_id: &str, root: &Path, result: &Value) -> Value {
    let error = result
        .get("error")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(generator_protocol_error_value);
    let rollback_source = result.get("rollback").and_then(Value::as_object);
    let mut rollback = json!({
        "recipeRemoved": rollback_source
            .and_then(|rollback| rollback.get("recipeRemoved"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "recipeRecoveryRequired": rollback_source
            .and_then(|rollback| rollback.get("recipeRecoveryRequired"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    });
    if let Some(relative) = rollback_source
        .and_then(|rollback| rollback.get("recipePath"))
        .and_then(Value::as_str)
        .and_then(|raw| catalog_relative_from_raw(root, raw))
    {
        rollback["recipePath"] = Value::String(relative);
    }
    json!({
        "outcome": "failed",
        "operationId": operation_id,
        "error": error,
        "rollback": rollback,
    })
}

/// Frozen result for one unconfirmed publication attempt.
fn unknown_publication_value(operation_id: &str, message: &str) -> Value {
    json!({
        "outcome": "unknown",
        "operationId": operation_id,
        "message": message,
    })
}

/// Report the recorded status of one native publication operation.
///
/// A confirmed record answers directly. An unconfirmed attempt is offered to the
/// backend while the same sidecar process still holds its own publication
/// memory; an answer that is still unconfirmed keeps the outcome unknown, which
/// is never treated as a failed publication.
fn publication_status_via_transport(
    transport: &dyn AppGeneratorTransport,
    state: &AppGeneratorState,
    operation_id: &str,
) -> Result<Value, String> {
    let record = {
        let registry = lock_registry(state)?;
        registry.publications.get(operation_id).cloned()
    };
    let Some(record) = record else {
        let response = transport.request(
            "appGeneratorPublicationStatus",
            json!({ "operationId": operation_id }),
        );
        if let Ok(response) = response {
            if let Some(result) = success_result(&response) {
                return Ok(success(result.clone()));
            }
        }
        return Ok(success(publication_status_value(
            operation_id,
            "unknown",
            false,
            None,
            json!({ "reason": "no_record" }),
        )));
    };
    if matches!(
        record.state,
        PublicationState::Saved | PublicationState::Failed
    ) {
        return Ok(success(publication_status_value(
            operation_id,
            publication_state_name(record.state),
            true,
            Some(record.recorded_at.clone()),
            record.detail.clone().unwrap_or(Value::Null),
        )));
    }
    let response = transport.request(
        "appGeneratorPublicationStatus",
        json!({ "operationId": operation_id }),
    );
    let Ok(response) = response else {
        return Ok(success(unconfirmed_publication_status(
            operation_id,
            &record,
            "status_unavailable",
        )));
    };
    let Some(result) = success_result(&response) else {
        return Ok(success(unconfirmed_publication_status(
            operation_id,
            &record,
            "status_unavailable",
        )));
    };
    let backend_outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let backend_confirmed = result
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if backend_confirmed && matches!(backend_outcome, "saved" | "failed") {
        let detail = confirmed_status_detail(operation_id, &record, result);
        let status = if backend_outcome == "saved" {
            PublicationState::Saved
        } else {
            PublicationState::Failed
        };
        update_publication(state, operation_id, status, Some(detail.clone()))?;
        let recorded_at = result
            .get("recordedAt")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| record.recorded_at.clone());
        return Ok(success(publication_status_value(
            operation_id,
            backend_outcome,
            true,
            Some(recorded_at),
            detail,
        )));
    }
    let reason = result
        .get("detail")
        .and_then(|detail| detail.get("reason"))
        .and_then(Value::as_str)
        .filter(|reason| *reason == "no_record")
        .unwrap_or("unconfirmed");
    Ok(success(unconfirmed_publication_status(
        operation_id,
        &record,
        reason,
    )))
}

/// Map one confirmed backend record onto the frozen status detail shape.
fn confirmed_status_detail(
    operation_id: &str,
    record: &PublicationRecord,
    result: &Value,
) -> Value {
    let outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if outcome == "failed" {
        // A status lookup nests the recorded save payload under `detail`, while a
        // direct save response carries the failure fields at its top level. Both
        // shapes must keep the real error, the rollback facts, and the recovery
        // path instead of degrading to the protocol-error placeholder.
        let payload = match result.get("detail") {
            Some(detail) if detail.is_object() => detail,
            _ => result,
        };
        return failed_outcome_value(operation_id, &record.root_path, payload);
    }
    let detail = result.get("detail").cloned().unwrap_or(Value::Null);
    let app_path = detail
        .get("app")
        .and_then(|app| app.get("appPath"))
        .and_then(Value::as_str)
        .and_then(|raw| catalog_relative_from_raw(&record.root_path, raw))
        .unwrap_or_default();
    let recipe_path = detail
        .get("recipe")
        .and_then(|recipe| recipe.get("recipePath"))
        .and_then(Value::as_str)
        .and_then(|raw| catalog_relative_from_raw(&record.root_path, raw))
        .unwrap_or_default();
    json!({
        "outcome": "saved",
        "operationId": operation_id,
        "app": { "appPath": app_path },
        "recipe": { "recipePath": recipe_path },
        "createdFiles": catalog_relative_file_list(detail.get("createdFiles"), &record.root_path),
        "replacedFiles": catalog_relative_file_list(detail.get("replacedFiles"), &record.root_path),
    })
}

/// Build one status result for an operation that never confirmed an outcome.
fn unconfirmed_publication_status(
    operation_id: &str,
    record: &PublicationRecord,
    reason: &str,
) -> Value {
    publication_status_value(
        operation_id,
        "unknown",
        false,
        Some(record.recorded_at.clone()),
        json!({ "reason": reason }),
    )
}

/// Build one publication status result payload.
fn publication_status_value(
    operation_id: &str,
    outcome: &str,
    confirmed: bool,
    recorded_at: Option<String>,
    detail: Value,
) -> Value {
    json!({
        "operationId": operation_id,
        "outcome": outcome,
        "confirmed": confirmed,
        "recordedAt": recorded_at,
        "detail": detail,
    })
}

/// Name one confirmed publication state for status results.
fn publication_state_name(status: PublicationState) -> &'static str {
    match status {
        PublicationState::Saved => "saved",
        PublicationState::Failed => "failed",
        PublicationState::Submitted => "unknown",
    }
}

/// List the catalog App Definitions one authored root offers for extension.
///
/// The listing is catalog-relative and inventory-aware: an incomplete inventory
/// blocks selection, so the author never picks an App whose identity could not
/// be read completely.
#[tauri::command]
pub fn list_app_generator_existing_apps(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    root_handle: Option<String>,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    let (_, root) = match resolve_session_root(&state, &session_handle, root_handle.as_deref())? {
        Ok(resolved) => resolved,
        Err(error) => return Ok(error),
    };
    let response = transport.request(
        "listAppGeneratorExistingApps",
        json!({ "authoredRoot": root.root }),
    )?;
    let Some(result) = success_result(&response) else {
        return Ok(response);
    };
    let Some(apps) = result.get("apps").and_then(Value::as_array) else {
        return Ok(generator_protocol_error());
    };
    let paths_relative = apps.iter().all(|app| {
        app.get("appPath")
            .and_then(Value::as_str)
            .is_some_and(is_catalog_relative_path)
    });
    if !paths_relative {
        return Ok(generator_protocol_error());
    }
    Ok(response)
}

/// Capture one catalog App Definition as the target of existing-App generation.
///
/// The returned handle names a native record bound to the exact root the App was
/// captured under; the backend snapshot it wraps retains the App bytes and
/// filesystem identity the later review and publication validate against.
#[tauri::command]
pub fn select_app_generator_existing_app(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    app_path: String,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    if !is_catalog_relative_path(&app_path) {
        return Ok(existing_app_outside_root_error());
    }
    let (_, root) = match resolve_session_root(&state, &session_handle, None)? {
        Ok(resolved) => resolved,
        Err(error) => return Ok(error),
    };
    let response = transport.request(
        "selectAppGeneratorExistingApp",
        json!({ "authoredRoot": root.root, "appPath": app_path }),
    )?;
    let Some(result) = success_result(&response).cloned() else {
        return Ok(response);
    };
    let Some(public) = result.as_object().cloned() else {
        return Ok(generator_protocol_error());
    };
    let Some(snapshot_id) = public.get("appSnapshotId").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    let Some(echoed_path) = public.get("appPath").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    if !is_catalog_relative_path(echoed_path) {
        return Ok(generator_protocol_error());
    }
    let selection = TrustedExistingApp {
        root_path: root.root.clone(),
        snapshot_id: snapshot_id.to_string(),
        app_path: echoed_path.to_string(),
        package_id: public
            .get("packageId")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    let mut registry = lock_registry(&state)?;
    let app_handle = registry.allocate("existing-app");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    // A new selection replaces every earlier capture and every outstanding
    // review, because those describe a different target App Definition.
    session.existing_apps.clear();
    session.reviews.clear();
    session.existing_apps.insert(app_handle.clone(), selection);
    let mut mapped = public;
    mapped.remove("appSnapshotId");
    mapped.insert("appHandle".to_string(), Value::String(app_handle));
    Ok(success(Value::Object(mapped)))
}

/// Build one immutable review of a local-APK App and Recipe pair.
///
/// Every trusted input is resolved from native session state. The result retains
/// the exact chosen App Artifact ID and Recipe ID, and the sidecar review handle
/// it wraps stays private so publication cannot be redirected later.
/// Permission automation is opt-in: `permissionAutomationEnabled` defaults to
/// false and preserves every declared permission set, while a disabled request
/// refuses any selected permission actions.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn review_app_generator_draft(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    root_handle: String,
    request_id: i64,
    existing_app_handle: Option<String>,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    artifact_name: Option<String>,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    permission_selection: Option<Value>,
    permission_automation_enabled: Option<bool>,
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    let inputs = match resolve_draft_inputs(
        &state,
        &session_handle,
        &apk_handle,
        Some(&root_handle),
        true,
        existing_app_handle.as_deref(),
        permission_selection,
        permission_automation_enabled.unwrap_or(false),
        None,
    )? {
        Ok(inputs) => inputs,
        Err(error) => return Ok(error),
    };
    review_draft_via_transport(
        &transport,
        &state,
        &session_handle,
        inputs,
        request_id,
        app_artifact_id,
        recipe_id,
        artifact_name,
        app,
        recipe,
        mappings,
        regenerate_identifiers,
    )
}

/// Build one immutable review of a remote-source App and Recipe pair.
///
/// The trusted remote payload is rebuilt from the retained source and asset, so
/// the author's choices select an asset while the provider facts, release
/// analysis, download URL, and publisher checksum stay native-owned.
/// Permission automation is opt-in: `permissionAutomationEnabled` defaults to
/// false and preserves every declared permission set, while a disabled request
/// refuses any selected permission actions.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn review_remote_app_generator_draft(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    apk_handle: String,
    root_handle: String,
    asset_handle: String,
    strategy: String,
    asset_pattern: Option<String>,
    include_prereleases: bool,
    trusted_sha256: Option<String>,
    request_id: i64,
    existing_app_handle: Option<String>,
    app_artifact_id: Option<String>,
    recipe_id: Option<String>,
    artifact_name: Option<String>,
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    permission_selection: Option<Value>,
    permission_automation_enabled: Option<bool>,
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    let remote = RemoteReviewSelection {
        asset_handle: &asset_handle,
        strategy: &strategy,
        asset_pattern: asset_pattern.as_deref(),
        include_prereleases,
        trusted_sha256: trusted_sha256.as_deref(),
    };
    let inputs = match resolve_draft_inputs(
        &state,
        &session_handle,
        &apk_handle,
        Some(&root_handle),
        true,
        existing_app_handle.as_deref(),
        permission_selection,
        permission_automation_enabled.unwrap_or(false),
        Some(remote),
    )? {
        Ok(inputs) => inputs,
        Err(error) => return Ok(error),
    };
    review_draft_via_transport(
        &transport,
        &state,
        &session_handle,
        inputs,
        request_id,
        app_artifact_id,
        recipe_id,
        artifact_name,
        app,
        recipe,
        mappings,
        regenerate_identifiers,
    )
}

/// Publish exactly the pair one immutable review handle describes.
///
/// The request carries no App, Recipe, mapping, permission, or identity input:
/// the reviewed capture is the only publication authority, and a stale request
/// id is refused instead of publishing a newer draft under an older review.
#[tauri::command]
pub fn save_app_generator_review(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    review_handle: String,
    request_id: i64,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    publish_review_via_transport(
        &transport,
        &state,
        &session_handle,
        &review_handle,
        request_id,
    )
}

/// Report the recorded outcome of one publication operation identity.
#[tauri::command]
pub fn app_generator_publication_status(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    operation_id: String,
) -> Result<Value, String> {
    let transport = SidecarTransport { sidecar: &sidecar };
    publication_status_via_transport(&transport, &state, &operation_id)
}

fn validate_apk(path: &Path) -> Result<(PathBuf, FileIdentity), Value> {
    let selected_metadata = fs::symlink_metadata(path).map_err(|_| invalid_apk_error())?;
    if !selected_metadata.file_type().is_file() {
        return Err(invalid_apk_error());
    }
    let canonical = fs::canonicalize(path).map_err(|_| invalid_apk_error())?;
    let extension_valid = canonical
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("apk"));
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| invalid_apk_error())?;
    if !extension_valid || !metadata.file_type().is_file() || metadata.len() > MAX_APK_BYTES {
        return Err(invalid_apk_error());
    }
    let identity = file_identity(&metadata).ok_or_else(invalid_apk_error)?;
    Ok((canonical, identity))
}

fn validate_authored_root(path: &Path) -> Result<TrustedRoot, Value> {
    let root = fs::canonicalize(path).map_err(|_| invalid_root_error())?;
    if !root.is_dir() {
        return Err(invalid_root_error());
    }
    let apps_directory = fs::canonicalize(root.join("apps")).map_err(|_| invalid_root_error())?;
    let recipes_directory =
        fs::canonicalize(root.join("recipes")).map_err(|_| invalid_root_error())?;
    if !apps_directory.is_dir()
        || !recipes_directory.is_dir()
        || !apps_directory.starts_with(&root)
        || !recipes_directory.starts_with(&root)
    {
        return Err(invalid_root_error());
    }
    Ok(TrustedRoot { root })
}

fn current_file_identity(path: &Path) -> Option<FileIdentity> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .and_then(|metadata| file_identity(&metadata))
}

fn file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity {
            length: metadata.len(),
            modified_nanos,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Some(FileIdentity {
            length: metadata.len(),
            modified_nanos,
        })
    }
}

fn preferences_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join(PREFERENCES_FILE_NAME))
}

fn load_preferences(app: &AppHandle) -> AppGeneratorPreferences {
    preferences_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn update_preferences<F>(app: &AppHandle, update: F)
where
    F: FnOnce(&mut AppGeneratorPreferences),
{
    let Some(path) = preferences_path(app) else {
        return;
    };
    let mut preferences = load_preferences(app);
    update(&mut preferences);
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(&preferences) else {
        return;
    };
    let _ = fs::write(path, bytes);
}

fn require_session(state: &AppGeneratorState, handle: &str) -> Result<(), String> {
    let registry = lock_registry(state)?;
    registry
        .session(handle)
        .map(|_| ())
        .map_err(|_| "The app generator session has expired.".to_string())
}

/// Canonicalize one author-selected permission set against the trusted
/// inspection of the APK a draft installs.
///
/// `installs_inspected_apk` records whether the draft's installation source is
/// the inspected artifact itself: a local APK file, or a remote source whose
/// strategy re-resolves that artifact at execution time. Every identity, bound,
/// and candidate check below still runs against the retained inspection, so a
/// selection can never widen what the inspection authorizes.
///
/// The `enabled` flag is the request's explicit opt-in. A disabled request with
/// a nonempty selection is contradictory: canonicalizing those identities would
/// produce grant steps the author turned off, so it is refused instead.
fn canonical_permission_automation(
    apk: &TrustedApk,
    selection: Option<Value>,
    installs_inspected_apk: bool,
    enabled: bool,
) -> Result<Option<Value>, Value> {
    let Some(selection) = selection else {
        return Ok(None);
    };
    let selection = serde_json::from_value::<PermissionSelectionRequest>(selection)
        .map_err(|_| permission_selection_error("altered"))?;
    if selection.runtime_permissions.is_empty() && selection.app_ops.is_empty() {
        return Ok(None);
    }
    if !enabled {
        return Err(permission_automation_disabled_error());
    }
    if !installs_inspected_apk {
        return Err(permission_strategy_error());
    }
    if apk.inspection_handle.as_deref() != Some(selection.inspection_handle.as_str()) {
        return Err(permission_selection_error("stale"));
    }
    if current_file_identity(&apk.path).as_ref() != Some(&apk.identity) {
        return Err(apk_changed_error());
    }
    let inspection = apk
        .inspection
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(inspection_required_error)?;
    let package_name = inspection
        .get("manifest")
        .and_then(Value::as_object)
        .and_then(|manifest| manifest.get("packageName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| permission_selection_error("altered"))?
        .to_string();

    let runtime_candidates = inspection
        .get("runtimeGrantCandidates")
        .and_then(Value::as_array)
        .ok_or_else(|| permission_selection_error("altered"))?;
    let app_op_candidates = inspection
        .get("appOpCandidates")
        .and_then(Value::as_array)
        .ok_or_else(|| permission_selection_error("altered"))?;

    let mut runtime_identities = HashSet::new();
    let mut runtime_permissions = Vec::new();
    for identity in selection.runtime_permissions {
        if identity.permission_name.trim().is_empty() {
            return Err(permission_selection_error("altered"));
        }
        if !runtime_identities.insert(identity.permission_name.clone()) {
            return Err(permission_selection_error("duplicate"));
        }
        let matches = runtime_candidates
            .iter()
            .filter_map(Value::as_object)
            .filter(|candidate| {
                candidate.get("permissionName").and_then(Value::as_str)
                    == Some(identity.permission_name.as_str())
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => return Err(permission_selection_error("unknown")),
            [candidate] => {
                let requires_root = candidate
                    .get("requiresRoot")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| permission_selection_error("altered"))?;
                let (android_api_min, android_api_max) = trusted_candidate_api_bounds(candidate)?;
                runtime_permissions.push(CanonicalRuntimePermission {
                    permission_name: identity.permission_name,
                    requires_root,
                    android_api_min,
                    android_api_max,
                });
            }
            _ => return Err(permission_selection_error("ambiguous")),
        }
    }

    let mut app_op_identities = HashSet::new();
    let mut app_ops = Vec::new();
    for identity in selection.app_ops {
        if identity.permission_name.trim().is_empty()
            || identity.operation_name.trim().is_empty()
            || identity.mode != "allow"
        {
            return Err(permission_selection_error("altered"));
        }
        if !app_op_identities.insert((
            identity.permission_name.clone(),
            identity.operation_name.clone(),
            identity.mode.clone(),
        )) {
            return Err(permission_selection_error("duplicate"));
        }
        let permission_matches = app_op_candidates
            .iter()
            .filter_map(Value::as_object)
            .filter(|candidate| {
                candidate.get("permissionName").and_then(Value::as_str)
                    == Some(identity.permission_name.as_str())
            })
            .collect::<Vec<_>>();
        let matches = permission_matches
            .iter()
            .copied()
            .filter(|candidate| {
                candidate.get("operationName").and_then(Value::as_str)
                    == Some(identity.operation_name.as_str())
                    && candidate.get("mode").and_then(Value::as_str) == Some(identity.mode.as_str())
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] if permission_matches.is_empty() => {
                return Err(permission_selection_error("unknown"));
            }
            [] => return Err(permission_selection_error("altered")),
            [candidate] => {
                let requires_root = candidate
                    .get("requiresRoot")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| permission_selection_error("altered"))?;
                let (android_api_min, android_api_max) = trusted_candidate_api_bounds(candidate)?;
                app_ops.push(CanonicalAppOpPermission {
                    permission_name: identity.permission_name,
                    operation_name: identity.operation_name,
                    mode: identity.mode,
                    requires_root,
                    android_api_min,
                    android_api_max,
                });
            }
            _ => return Err(permission_selection_error("ambiguous")),
        }
    }

    runtime_permissions.sort_by(|left, right| left.permission_name.cmp(&right.permission_name));
    app_ops.sort_by(|left, right| {
        left.operation_name
            .cmp(&right.operation_name)
            .then_with(|| left.mode.cmp(&right.mode))
            .then_with(|| left.permission_name.cmp(&right.permission_name))
    });
    serde_json::to_value(CanonicalPermissionAutomation {
        package_name,
        runtime_permissions,
        app_ops,
    })
    .map(Some)
    .map_err(|_| generator_protocol_error())
}

fn trusted_candidate_api_bounds(
    candidate: &serde_json::Map<String, Value>,
) -> Result<(u32, Option<u32>), Value> {
    let minimum = candidate
        .get("androidApiMin")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| permission_selection_error("altered"))?;
    let maximum = match candidate.get("androidApiMax") {
        Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| permission_selection_error("altered"))?,
        ),
        None => return Err(permission_selection_error("altered")),
    };
    if maximum.is_some_and(|maximum| maximum < minimum) {
        return Err(permission_selection_error("altered"));
    }
    Ok((minimum, maximum))
}

/// Convert native manifest metadata into the deliberately smaller facts model
/// consumed by the existing draft generators. Fields that native inspection
/// does not prove remain unavailable. This product accepts one standalone APK,
/// so split/base are admission assumptions rather than inspection evidence.
fn facts_from_native_inspection(inspection: &Value) -> Option<Value> {
    let manifest = inspection.get("manifest")?.as_object()?;
    let package_name = manifest
        .get("packageName")?
        .as_str()
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let version_code = nullable_string(manifest, "versionCode")?;
    let version_name = nullable_string(manifest, "versionName")?;
    let min_sdk =
        nullable_string(manifest, "minSdkVersion")?.and_then(|value| value.parse::<i64>().ok());
    let target_sdk =
        nullable_string(manifest, "targetSdkVersion")?.and_then(|value| value.parse::<i64>().ok());
    let requested_permissions = inspection.get("permissions")?.as_array()?.clone();
    let calculated_sha256 = inspection.get("calculatedSha256")?.as_str()?;
    let checksum_status = inspection.get("checksumStatus")?.as_str()?;
    let signature_verification = inspection.get("signatureVerification")?.as_str()?;

    Some(json!({
        "packageName": package_name,
        "applicationLabel": Value::Null,
        "versionCode": version_code,
        "versionName": version_name,
        "minSdk": min_sdk,
        "targetSdk": target_sdk,
        "abis": [],
        "launcherActivities": [],
        "requestedPermissions": requested_permissions,
        "calculatedSha256": calculated_sha256,
        "checksumStatus": checksum_status,
        "signatureVerification": signature_verification,
        "debuggable": Value::Null,
        "split": false,
        "base": true,
    }))
}

fn nullable_string(object: &Map<String, Value>, field: &str) -> Option<Option<String>> {
    match object.get(field)? {
        Value::Null => Some(None),
        Value::String(value) => Some(Some(value.clone())),
        _ => None,
    }
}

fn lock_registry(
    state: &AppGeneratorState,
) -> Result<std::sync::MutexGuard<'_, GeneratorRegistry>, String> {
    state
        .registry
        .lock()
        .map_err(|_| "App generator state is unavailable.".to_string())
}

fn request_generated_draft(
    transport: &dyn AppGeneratorTransport,
    operation: &str,
    payload: Value,
    root: Option<&TrustedRoot>,
) -> Result<Value, String> {
    let mut response = transport.request(operation, payload)?;
    let Some(draft) = success_result(&response).cloned() else {
        return Ok(response);
    };
    let collisions = match root {
        Some(root) => {
            let collision_payload = match collision_payload_from_generated_draft(root, &draft) {
                Ok(payload) => payload,
                Err(_) => return Ok(generator_protocol_error()),
            };
            let collision_response =
                transport.request("checkGeneratedCatalogCollisions", collision_payload)?;
            let Some(collisions) = success_result(&collision_response).cloned() else {
                return Ok(collision_response);
            };
            collisions
        }
        None => Value::Null,
    };
    let Some(result) = response.get_mut("result").and_then(Value::as_object_mut) else {
        return Ok(generator_protocol_error());
    };
    result.insert("collisions".to_string(), collisions);
    Ok(response)
}

fn collision_payload_from_generated_draft(
    root: &TrustedRoot,
    draft: &Value,
) -> Result<Value, String> {
    let app = draft
        .get("app")
        .cloned()
        .ok_or_else(|| "Generated app-and-recipe draft is missing its app.".to_string())?;
    let recipe = draft
        .get("recipe")
        .cloned()
        .ok_or_else(|| "Generated app-and-recipe draft is missing its recipe.".to_string())?;
    Ok(json!({
        "authoredRoot": root.root,
        "app": app,
        "recipe": recipe,
    }))
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
    json!({ "ok": false, "error": { "code": code, "message": message, "details": details } })
}
fn expired_session_error() -> Value {
    api_error(
        "app_generator_session_expired",
        "The app generator session has expired. Start the generator again.",
        json!({}),
    )
}
fn invalid_handle_error() -> Value {
    api_error(
        "app_generator_handle_invalid",
        "The selected generator item is no longer available.",
        json!({}),
    )
}
fn invalid_apk_error() -> Value {
    api_error(
        "apk_invalid",
        "Choose a regular .apk file no larger than 2 GiB.",
        json!({}),
    )
}
fn apk_changed_error() -> Value {
    api_error(
        "apk_changed",
        "The selected APK disappeared or changed. Choose and inspect it again.",
        json!({}),
    )
}
fn invalid_root_error() -> Value {
    api_error(
        "app_generator_root_invalid",
        "Choose an authored root with existing apps and recipes directories.",
        json!({}),
    )
}
fn invalid_destination_error() -> Value {
    api_error(
        "app_recipe_destination_invalid",
        "The generated destination is invalid.",
        json!({}),
    )
}
fn inspection_required_error() -> Value {
    api_error(
        "apk_inspection_required",
        "Inspect the selected APK before generating drafts.",
        json!({}),
    )
}
fn permission_selection_error(reason: &str) -> Value {
    api_error(
        "app_generator_permission_selection_invalid",
        "The selected permissions no longer match the current APK inspection. Inspect the APK again and review permissions.",
        json!({ "reason": reason }),
    )
}
/// Refusal for selected permission actions on a request that disabled
/// permission automation.
///
/// The error code matches the other permission-selection refusals so the
/// frontend keeps one display path, while `details.reason` records the specific
/// contradiction: the draft asked for no permission automation and still listed
/// actions to grant.
fn permission_automation_disabled_error() -> Value {
    api_error(
        "app_generator_permission_selection_invalid",
        "Permission automation is disabled for this draft, but permission actions were selected. Enable permission automation or clear the selection.",
        json!({ "reason": "disabled" }),
    )
}
fn permission_strategy_error() -> Value {
    api_error(
        "apk_permission_automation_strategy_unsupported",
        "Permission automation is supported only when the draft installs the inspected APK: a local APK or a pinned or latest-compatible remote artifact.",
        json!({}),
    )
}
fn generator_protocol_error() -> Value {
    api_error(
        "app_generator_protocol_error",
        "The generator backend returned an incomplete response.",
        json!({}),
    )
}
/// Protocol-violation error object used where only the error value is kept.
fn generator_protocol_error_value() -> Value {
    json!({
        "code": "app_generator_protocol_error",
        "message": "The generator backend returned an incomplete response.",
        "details": {},
    })
}
/// Refusal for every retired direct-publication command.
///
/// Generated files may only be published through an immutable review handle,
/// so the legacy writers can no longer act as an alternative write authority.
fn review_required_error() -> Value {
    api_error(
        "review_required",
        "Generated files can only be published through an immutable review. Review the draft and save the review.",
        json!({}),
    )
}
/// Refusal shared by the retired direct-publication commands.
///
/// The retired commands accepted mutable App, Recipe, mapping, and permission
/// inputs at save time, so nothing bound the published bytes to a reviewed
/// candidate. Both commands resolve to exactly this refusal, which keeps
/// publication authority with the immutable review handle.
fn retired_direct_publication_refusal() -> Value {
    review_required_error()
}
/// Refusal for an App Definition outside the authored root this request targets.
fn existing_app_outside_root_error() -> Value {
    api_error(
        "existing_app_outside_root",
        "Choose an App Definition path inside the current authored root.",
        json!({}),
    )
}
/// Refusal for an inspected APK whose package differs from the selected App.
///
/// Generation installs the inspected APK into the selected App Definition, so a
/// package mismatch could never produce a working install; the pairing is
/// rejected before any review or publication state exists.
fn package_mismatch_error(inspected: &str, selected: &str) -> Value {
    api_error(
        "existing_app_package_mismatch",
        "The inspected APK package does not match the selected App Definition package.",
        json!({ "inspectedPackageId": inspected, "selectedPackageId": selected }),
    )
}
/// Refusal for one request that combines mutually exclusive inputs.
fn invalid_argument_error(message: &str) -> Value {
    api_error("invalid_argument", message, json!({}))
}
/// Error object for a committed save whose Recipe document could not be opened.
fn open_failed_error(message: &str) -> Value {
    json!({
        "code": "app_recipe_saved_open_failed",
        "message": message,
        "details": {},
    })
}
/// Refusal for a save attempt whose review no longer matches its trusted inputs.
fn review_invalidated_error() -> Value {
    api_error(
        "review_invalidated",
        "This review no longer matches the selected APK, root, inspection, source, or App snapshot. Generate a new review.",
        json!({}),
    )
}
/// Refusal for a save attempt whose reviewed candidate still blocks publication.
///
/// Nothing was written, and the retained review survives the attempt, so the
/// author can resolve the reported problem and save the same review again.
fn review_blocking_error(record: &ReviewRecord) -> Value {
    api_error(
        "review_blocking",
        "The review still has blocking problems, so nothing was published.",
        json!({ "requestId": record.request_id }),
    )
}
/// Frozen message for one unconfirmed publication attempt.
///
/// The wording is part of the published UI contract: it tells the author that
/// the catalog may already contain the generated files and that repeating the
/// save can duplicate them.
const UNCONFIRMED_PUBLICATION_MESSAGE: &str =
    "Publication status unknown — the catalog may already contain these files. Do not repeat the save.";
/// True when one path is a non-empty catalog-relative path.
///
/// Catalog paths are relative POSIX-style paths below the authored root, so
/// absolute paths, parent-directory escapes, empty names, and Windows
/// separators are all rejected instead of being interpreted by the UI.
fn is_catalog_relative_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') {
        return false;
    }
    let candidate = Path::new(path);
    !candidate.is_absolute()
        && candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}
/// Reduce one backend-reported path to its catalog-relative form.
///
/// Backend evidence sometimes carries absolute native paths. The UI contract
/// only ever exposes catalog-relative paths, so a path outside the authored
/// root yields None instead of leaking host filesystem layout.
fn catalog_relative_from_raw(root: &Path, raw: &str) -> Option<String> {
    let raw_path = Path::new(raw);
    let relative = if raw_path.is_absolute() {
        raw_path.strip_prefix(root).ok()?
    } else {
        raw_path
    };
    let relative = relative.to_str()?;
    is_catalog_relative_path(relative).then(|| relative.to_string())
}
/// Reduce one backend-reported file list to catalog-relative paths.
///
/// Entries that cannot be expressed relative to the authored root are omitted:
/// the list is author-facing evidence, and a host absolute path must never
/// reach a result payload.
fn catalog_relative_file_list(value: Option<&Value>, root: &Path) -> Value {
    let Some(entries) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        entries
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|raw| catalog_relative_from_raw(root, raw))
            .map(Value::String)
            .collect(),
    )
}
/// Current UTC time formatted as an ISO 8601 timestamp.
fn iso8601_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    iso8601_from_epoch_seconds(seconds)
}
/// Format one Unix timestamp in seconds as an ISO 8601 UTC timestamp.
fn iso8601_from_epoch_seconds(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3_600,
        (time_of_day % 3_600) / 60,
        time_of_day % 60
    )
}
/// Convert a day count since 1970-01-01 into a civil (year, month, day).
///
/// This is Howard Hinnant's civil_from_days date algorithm: it is exact for
/// every timestamp a system clock can produce and needs no date library.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        (month_prime + 3) as u32
    } else {
        (month_prime - 9) as u32
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn publication_operation_ids_do_not_repeat_after_registry_restart() {
        let first = GeneratorRegistry::default().allocate_publication();
        let after_restart = GeneratorRegistry::default().allocate_publication();

        assert_ne!(first, after_restart);
        assert!(first.starts_with("publication-"));
        assert!(after_restart.starts_with("publication-"));
    }

    #[test]
    fn publication_operation_ids_distinguish_a_restart_with_reused_pid_and_sequence() {
        let before_restart = publication_operation_id(UNIX_EPOCH + Duration::from_nanos(1), 7, 1);
        let after_restart = publication_operation_id(UNIX_EPOCH + Duration::from_nanos(2), 7, 1);

        assert_ne!(before_restart, after_restart);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CapturedGitHubRequest {
        path: String,
        user_agent: Option<String>,
        accept: Option<String>,
        api_version: Option<String>,
    }

    enum FakeResponseFraming {
        ContentLength,
        Chunked,
        DeclaredLength(usize),
    }

    struct FakeGitHubResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        framing: FakeResponseFraming,
    }

    impl FakeGitHubResponse {
        fn json(status: u16, body: Value) -> Self {
            Self {
                status,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: serde_json::to_vec(&body).unwrap(),
                framing: FakeResponseFraming::ContentLength,
            }
        }

        fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.to_string(), value.to_string()));
            self
        }
    }

    fn spawn_fake_github(
        responses: Vec<FakeGitHubResponse>,
    ) -> (String, Arc<Mutex<Vec<CapturedGitHubRequest>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_root = format!("http://{}", listener.local_addr().unwrap());
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_for_thread = Arc::clone(&captured);
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request = capture_github_request(&mut stream);
                captured_for_thread.lock().unwrap().push(request);
                write_fake_response(&mut stream, response);
            }
        });
        (api_root, captured)
    }

    fn capture_github_request(stream: &mut std::net::TcpStream) -> CapturedGitHubRequest {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        while bytes.len() < 16 * 1024 && !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8(bytes).unwrap();
        let mut lines = request.split("\r\n");
        let path = lines
            .next()
            .and_then(|line| line.split_ascii_whitespace().nth(1))
            .unwrap_or_default()
            .to_string();
        let mut captured = CapturedGitHubRequest {
            path,
            user_agent: None,
            accept: None,
            api_version: None,
        };
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let target = if name.eq_ignore_ascii_case("user-agent") {
                &mut captured.user_agent
            } else if name.eq_ignore_ascii_case("accept") {
                &mut captured.accept
            } else if name.eq_ignore_ascii_case("x-github-api-version") {
                &mut captured.api_version
            } else {
                continue;
            };
            *target = Some(value.trim().to_string());
        }
        captured
    }

    fn write_fake_response(stream: &mut std::net::TcpStream, response: FakeGitHubResponse) {
        let mut head = format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", response.status);
        for (name, value) in response.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        match response.framing {
            FakeResponseFraming::ContentLength => {
                head.push_str(&format!("Content-Length: {}\r\n\r\n", response.body.len()));
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&response.body).unwrap();
            }
            FakeResponseFraming::Chunked => {
                head.push_str("Transfer-Encoding: chunked\r\n\r\n");
                stream.write_all(head.as_bytes()).unwrap();
                stream
                    .write_all(format!("{:X}\r\n", response.body.len()).as_bytes())
                    .unwrap();
                stream.write_all(&response.body).unwrap();
                stream.write_all(b"\r\n0\r\n\r\n").unwrap();
            }
            FakeResponseFraming::DeclaredLength(length) => {
                head.push_str(&format!("Content-Length: {length}\r\n\r\n"));
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&response.body).unwrap();
            }
        }
    }

    fn fake_github_client() -> Client {
        Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
    }

    fn fake_github_error(response: FakeGitHubResponse, now_epoch_seconds: u64) -> Value {
        let (api_root, _) = spawn_fake_github(vec![response]);
        get_github_json_bounded(
            &fake_github_client(),
            &format!("{api_root}/metadata"),
            now_epoch_seconds,
        )
        .unwrap_err()
    }

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "emuchef-app-generator-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn permission_test_apk(label: &str) -> TrustedApk {
        let path = temp_path(label).with_extension("apk");
        fs::write(&path, b"trusted-apk").unwrap();
        let (_, identity) = validate_apk(&path).unwrap();
        TrustedApk {
            path,
            identity,
            inspection: Some(json!({
                "manifest": { "packageName": "com.example.inspected" },
                "runtimeGrantCandidates": [
                    {
                        "permissionName": "android.permission.CAMERA",
                        "requiresRoot": false,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                        "selected": false
                    }
                ],
                "appOpCandidates": [
                    {
                        "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                        "operationName": "MANAGE_EXTERNAL_STORAGE",
                        "mode": "allow",
                        "requiresRoot": true,
                        "androidApiMin": 30,
                        "androidApiMax": null,
                        "selected": false
                    }
                ]
            })),
            inspection_handle: Some("apk-inspection-current".to_string()),
            facts: Some(json!({ "packageName": "com.example.inspected" })),
            remote_asset_handle: None,
        }
    }

    fn permission_selection() -> Value {
        json!({
            "inspectionHandle": "apk-inspection-current",
            "runtimePermissions": [{ "permissionName": "android.permission.CAMERA" }],
            "appOps": [{
                "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                "operationName": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow"
            }]
        })
    }

    fn api_error_code(value: &Value) -> Option<&str> {
        value
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
    }

    fn api_error_message(value: &Value) -> Option<&str> {
        value
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
    }

    fn trusted_remote_test_state(
        release_analysis: Option<Vec<TrustedRemoteRelease>>,
    ) -> (AppGeneratorState, String, String) {
        let state = AppGeneratorState::default();
        let mut registry = lock_registry(&state).unwrap();
        let session_handle = registry.begin();
        let source_handle = registry.allocate("remote-source");
        let asset_handle = registry.allocate("remote-asset");
        let session = registry.session_mut(&session_handle).unwrap();
        session.remote_sources.insert(
            source_handle.clone(),
            TrustedRemoteSource {
                mode: "github_repository".to_string(),
                provider: Some("github".to_string()),
                base_url: Some("https://github.com".to_string()),
                repository: Some("example/project".to_string()),
                release_tag: None,
                direct_url: None,
                release_analysis,
            },
        );
        session.remote_assets.insert(
            asset_handle.clone(),
            TrustedRemoteAsset {
                source_handle,
                download_url: "https://github.com/example/project/releases/download/v2/app.apk"
                    .to_string(),
                file_name: "app-v2.apk".to_string(),
                size: 10,
                release_tag: Some("v2".to_string()),
            },
        );
        drop(registry);
        (state, session_handle, asset_handle)
    }

    /// Resolve one retained remote context the way the production boundary does.
    ///
    /// Reviews and previews resolve remote inputs from the session, so tests go
    /// through the same lookup and trusted-payload construction the generator
    /// uses instead of a parallel test-only path.
    fn remote_context_from_state(
        state: &AppGeneratorState,
        session_handle: &str,
        asset_handle: &str,
        strategy: &str,
        asset_pattern: Option<&str>,
        include_prereleases: bool,
        trusted_sha256: Option<&str>,
    ) -> Result<TrustedRemoteDraftContext, Value> {
        let registry = lock_registry(state).unwrap();
        let session = registry.session(session_handle).unwrap();
        retained_remote_context(
            session,
            asset_handle,
            strategy,
            asset_pattern,
            include_prereleases,
            trusted_sha256,
        )
    }

    #[test]
    fn trusted_release_snapshot_preserves_session_order_and_minimal_shape() {
        let releases = vec![
            TrustedRemoteRelease {
                tag: "second-by-name-first-by-provider".to_string(),
                prerelease: true,
                asset_file_names: vec!["z.apk".to_string(), "a.apk".to_string()],
            },
            TrustedRemoteRelease {
                tag: "first-by-name-second-by-provider".to_string(),
                prerelease: false,
                asset_file_names: Vec::new(),
            },
        ];
        let (state, session_handle, asset_handle) = trusted_remote_test_state(Some(releases));
        let trusted = remote_context_from_state(
            &state,
            &session_handle,
            &asset_handle,
            "latest_compatible_release",
            Some(r"\.apk$"),
            false,
            None,
        )
        .unwrap();
        assert_eq!(
            trusted.release_analysis,
            Some(json!({
                "releases": [
                    {
                        "releaseTag": "second-by-name-first-by-provider",
                        "prerelease": true,
                        "assetFileNames": ["z.apk", "a.apk"]
                    },
                    {
                        "releaseTag": "first-by-name-second-by-provider",
                        "prerelease": false,
                        "assetFileNames": []
                    }
                ]
            }))
        );
        let serialized = trusted.release_analysis.unwrap().to_string();
        assert!(!serialized.contains("download"));
        assert!(!serialized.contains("description"));
        assert!(!serialized.contains("path"));
    }

    #[test]
    fn trusted_release_snapshot_rejects_missing_ambiguous_and_stale_state() {
        let (missing_state, missing_session, missing_asset) = trusted_remote_test_state(None);
        let missing = remote_context_from_state(
            &missing_state,
            &missing_session,
            &missing_asset,
            "latest_compatible_release",
            Some(r"\.apk$"),
            false,
            None,
        )
        .unwrap_err();
        assert_eq!(
            api_error_code(&missing),
            Some("latest_release_analysis_missing")
        );

        for releases in [
            vec![TrustedRemoteRelease {
                tag: " ".to_string(),
                prerelease: false,
                asset_file_names: vec!["app.apk".to_string()],
            }],
            vec![
                TrustedRemoteRelease {
                    tag: "v1".to_string(),
                    prerelease: false,
                    asset_file_names: vec!["app.apk".to_string()],
                },
                TrustedRemoteRelease {
                    tag: "v1".to_string(),
                    prerelease: true,
                    asset_file_names: vec!["other.apk".to_string()],
                },
            ],
            vec![TrustedRemoteRelease {
                tag: "v1".to_string(),
                prerelease: false,
                asset_file_names: vec!["app.apk".to_string(), "app.apk".to_string()],
            }],
            vec![TrustedRemoteRelease {
                tag: "v1".to_string(),
                prerelease: false,
                asset_file_names: vec!["not-an-apk.zip".to_string()],
            }],
        ] {
            let (state, session, asset) = trusted_remote_test_state(Some(releases));
            let error = remote_context_from_state(
                &state,
                &session,
                &asset,
                "latest_compatible_release",
                Some(r"\.apk$"),
                false,
                None,
            )
            .unwrap_err();
            assert_eq!(
                api_error_code(&error),
                Some("latest_release_analysis_invalid")
            );
        }

        for pattern in [None, Some("   ")] {
            let (state, session, asset) =
                trusted_remote_test_state(Some(vec![TrustedRemoteRelease {
                    tag: "v1".to_string(),
                    prerelease: false,
                    asset_file_names: vec!["app.apk".to_string()],
                }]));
            let blank_pattern = remote_context_from_state(
                &state,
                &session,
                &asset,
                "latest_compatible_release",
                pattern,
                false,
                None,
            )
            .unwrap();
            assert!(blank_pattern.release_analysis.is_some());
            match pattern {
                None => assert_eq!(blank_pattern.source["assetPattern"], Value::Null),
                Some(value) => assert_eq!(
                    blank_pattern.source["assetPattern"],
                    Value::String(value.to_string())
                ),
            }
        }

        let (state, session, _) = trusted_remote_test_state(Some(Vec::new()));
        let stale = remote_context_from_state(
            &state,
            &session,
            "frontend-invented-asset",
            "latest_compatible_release",
            Some(r"\.apk$"),
            false,
            None,
        )
        .unwrap_err();
        assert_eq!(api_error_code(&stale), Some("app_generator_handle_invalid"));
    }

    #[test]
    fn pinned_and_user_provided_payloads_do_not_require_release_analysis() {
        for strategy in ["pinned_remote_asset", "user_provided_apk"] {
            let (state, session, asset) = trusted_remote_test_state(None);
            let trusted =
                remote_context_from_state(&state, &session, &asset, strategy, None, false, None)
                    .unwrap();
            assert!(trusted.release_analysis.is_none());
            assert_eq!(trusted.source["strategy"], strategy);
        }
    }

    #[test]
    fn permission_selection_is_canonicalized_from_stored_inspection_only() {
        let apk = permission_test_apk("permission-canonical");
        let canonical =
            canonical_permission_automation(&apk, Some(permission_selection()), true, true)
                .unwrap()
                .unwrap();
        assert_eq!(canonical["packageName"], "com.example.inspected");
        assert_eq!(
            canonical["runtimePermissions"],
            json!([{
                "permissionName": "android.permission.CAMERA",
                "requiresRoot": false,
                "androidApiMin": 23,
                "androidApiMax": null
            }])
        );
        assert_eq!(
            canonical["appOps"],
            json!([{
                "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                "operationName": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow",
                "requiresRoot": true,
                "androidApiMin": 30,
                "androidApiMax": null
            }])
        );
        assert!(!canonical.to_string().contains("selected"));
        fs::remove_file(apk.path).unwrap();
    }

    #[test]
    fn permission_selection_rejects_missing_malformed_zero_and_impossible_trusted_bounds() {
        for (label, minimum, maximum) in [
            ("missing", Value::Null, Value::Null),
            ("malformed", json!("23"), Value::Null),
            ("zero", json!(0), Value::Null),
            ("impossible", json!(30), json!(29)),
        ] {
            let mut apk = permission_test_apk(label);
            let candidate = apk.inspection.as_mut().unwrap()["runtimeGrantCandidates"][0]
                .as_object_mut()
                .unwrap();
            if label == "missing" {
                candidate.remove("androidApiMin");
            } else {
                candidate.insert("androidApiMin".to_string(), minimum);
            }
            candidate.insert("androidApiMax".to_string(), maximum);
            let selection = json!({
                "inspectionHandle": "apk-inspection-current",
                "runtimePermissions": [{ "permissionName": "android.permission.CAMERA" }],
                "appOps": []
            });
            let error =
                canonical_permission_automation(&apk, Some(selection), true, true).unwrap_err();
            assert_eq!(error["error"]["details"]["reason"], "altered");
            fs::remove_file(apk.path).unwrap();
        }
    }

    #[test]
    fn permission_selection_rejects_unknown_duplicate_altered_and_ambiguous_identities() {
        let mut apk = permission_test_apk("permission-invalid-identities");
        let cases = [
            (
                json!({
                    "inspectionHandle": "apk-inspection-current",
                    "runtimePermissions": [{ "permissionName": "android.permission.UNKNOWN" }],
                    "appOps": []
                }),
                "unknown",
            ),
            (
                json!({
                    "inspectionHandle": "apk-inspection-current",
                    "runtimePermissions": [
                        { "permissionName": "android.permission.CAMERA" },
                        { "permissionName": "android.permission.CAMERA" }
                    ],
                    "appOps": []
                }),
                "duplicate",
            ),
            (
                json!({
                    "inspectionHandle": "apk-inspection-current",
                    "runtimePermissions": [],
                    "appOps": [{
                        "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                        "operationName": "MANAGE_EXTERNAL_STORAGE",
                        "mode": "deny"
                    }]
                }),
                "altered",
            ),
        ];
        for (selection, reason) in cases {
            let error =
                canonical_permission_automation(&apk, Some(selection), true, true).unwrap_err();
            assert_eq!(
                api_error_code(&error),
                Some("app_generator_permission_selection_invalid")
            );
            assert_eq!(error["error"]["details"]["reason"], reason);
        }

        apk.inspection.as_mut().unwrap()["runtimeGrantCandidates"] = json!([
            {
                "permissionName": "android.permission.CAMERA",
                "requiresRoot": false,
                "androidApiMin": 23,
                "androidApiMax": null,
                "selected": false
            },
            {
                "permissionName": "android.permission.CAMERA",
                "requiresRoot": false,
                "androidApiMin": 23,
                "androidApiMax": null,
                "selected": false
            }
        ]);
        let error = canonical_permission_automation(
            &apk,
            Some(json!({
                "inspectionHandle": "apk-inspection-current",
                "runtimePermissions": [{ "permissionName": "android.permission.CAMERA" }],
                "appOps": []
            })),
            true,
            true,
        )
        .unwrap_err();
        assert_eq!(error["error"]["details"]["reason"], "ambiguous");
        fs::remove_file(apk.path).unwrap();
    }

    #[test]
    fn stale_inspection_handle_and_replaced_apk_identity_are_distinct_failures() {
        let apk = permission_test_apk("permission-stale");
        let mut stale = permission_selection();
        stale["inspectionHandle"] = json!("apk-inspection-previous");
        let stale_error =
            canonical_permission_automation(&apk, Some(stale), true, true).unwrap_err();
        assert_eq!(
            api_error_code(&stale_error),
            Some("app_generator_permission_selection_invalid")
        );
        assert_eq!(stale_error["error"]["details"]["reason"], "stale");

        fs::write(&apk.path, b"replacement-apk-with-new-identity").unwrap();
        let changed_error =
            canonical_permission_automation(&apk, Some(permission_selection()), true, true)
                .unwrap_err();
        assert_eq!(api_error_code(&changed_error), Some("apk_changed"));
        fs::remove_file(apk.path).unwrap();
    }

    #[test]
    fn empty_and_noneligible_permission_selections_have_stable_behavior() {
        let apk = permission_test_apk("permission-eligibility");
        assert_eq!(
            canonical_permission_automation(
                &apk,
                Some(json!({
                    "inspectionHandle": "stale-is-irrelevant-when-empty",
                    "runtimePermissions": [],
                    "appOps": []
                })),
                false,
                true,
            )
            .unwrap(),
            None
        );
        let error =
            canonical_permission_automation(&apk, Some(permission_selection()), false, true)
                .unwrap_err();
        assert_eq!(
            api_error_code(&error),
            Some("apk_permission_automation_strategy_unsupported")
        );
        fs::remove_file(apk.path).unwrap();
    }

    #[test]
    fn native_inspection_derives_only_conservative_generator_facts() {
        let inspection = json!({
            "manifest": {
                "packageName": "com.example.app",
                "versionCode": "42",
                "versionName": "1.2",
                "minSdkVersion": "23",
                "targetSdkVersion": "35",
            },
            "permissions": [
                {
                    "name": "android.permission.INTERNET",
                    "declarationKind": "uses_permission",
                    "maxSdkVersion": null,
                    "classification": "install_time",
                    "applicability": {
                        "status": "applicable",
                        "reason": null,
                        "maximumSdkVersion": null,
                        "introductionApi": null,
                        "minimumDeviceApi": null,
                        "minimumTargetSdk": null,
                        "targetSdkState": null
                    }
                },
                {
                    "name": "android.permission.CAMERA",
                    "declarationKind": "uses_permission_sdk_23",
                    "maxSdkVersion": "34",
                    "classification": "runtime_grantable",
                    "applicability": {
                        "status": "not_applicable",
                        "reason": "max_sdk_version_exceeded",
                        "maximumSdkVersion": 34,
                        "introductionApi": null,
                        "minimumDeviceApi": null,
                        "minimumTargetSdk": null,
                        "targetSdkState": null
                    }
                },
            ],
            "runtimeGrantCandidates": [],
            "appOpCandidates": [],
            "warnings": [{
                "code": "apk_permission_unknown",
                "message": "Review only.",
                "permissionName": "android.permission.EXAMPLE",
                "applicabilityReason": null,
            }],
            "calculatedSha256": "AB".repeat(32),
            "checksumStatus": "not_compared",
            "signatureVerification": "not_performed",
        });
        let facts = facts_from_native_inspection(&inspection).unwrap();
        assert_eq!(facts["packageName"], "com.example.app");
        assert_eq!(facts["versionCode"], "42");
        assert_eq!(facts["minSdk"], 23);
        assert_eq!(facts["targetSdk"], 35);
        assert_eq!(facts["requestedPermissions"], inspection["permissions"]);
        assert_eq!(facts["calculatedSha256"], "AB".repeat(32));
        assert_eq!(facts["checksumStatus"], "not_compared");
        assert_eq!(facts["signatureVerification"], "not_performed");
        assert_eq!(facts["applicationLabel"], Value::Null);
        assert_eq!(facts["launcherActivities"], json!([]));
        assert_eq!(facts["abis"], json!([]));
        assert_eq!(facts["debuggable"], Value::Null);
        assert_eq!(facts["split"], false);
        assert_eq!(facts["base"], true);
    }

    #[test]
    fn native_inspection_keeps_non_numeric_sdk_values_out_of_generator_facts() {
        let inspection = json!({
            "manifest": {
                "packageName": "com.example.preview",
                "versionCode": null,
                "versionName": null,
                "minSdkVersion": "VanillaIceCream",
                "targetSdkVersion": null,
            },
            "permissions": [],
            "calculatedSha256": "A".repeat(64),
            "checksumStatus": "not_compared",
            "signatureVerification": "not_performed",
        });
        let facts = facts_from_native_inspection(&inspection).unwrap();
        assert_eq!(facts["minSdk"], Value::Null);
        assert_eq!(facts["targetSdk"], Value::Null);
    }

    #[test]
    fn native_inspection_payload_uses_only_the_trusted_path() {
        let path = Path::new("/private/internal/selected.apk");
        assert_eq!(
            native_inspection_payload(path),
            json!({ "apkPath": "/private/internal/selected.apk" })
        );
        assert!(native_inspection_payload(path).get("analyzer").is_none());
        assert!(native_inspection_payload(path).get("facts").is_none());
    }

    #[test]
    fn trusted_apk_keeps_native_inspection_separate_from_generator_facts() {
        let path = temp_path("stored-inspection").with_extension("apk");
        fs::write(&path, b"apk").unwrap();
        let (_, identity) = validate_apk(&path).unwrap();
        let inspection = json!({
            "manifest": {
                "packageName": "com.example.app",
                "versionCode": null,
                "versionName": null,
                "minSdkVersion": null,
                "targetSdkVersion": null,
            },
            "permissions": [],
            "runtimeGrantCandidates": [],
            "appOpCandidates": [],
            "warnings": [],
            "calculatedSha256": "A".repeat(64),
            "checksumStatus": "not_compared",
            "signatureVerification": "not_performed",
        });
        let facts = facts_from_native_inspection(&inspection).unwrap();
        let trusted = TrustedApk {
            path: path.clone(),
            identity,
            inspection: Some(inspection.clone()),
            inspection_handle: Some("apk-inspection-test".to_string()),
            facts: Some(facts.clone()),
            remote_asset_handle: None,
        };
        assert_eq!(trusted.inspection, Some(inspection));
        assert_eq!(trusted.facts, Some(facts));
        assert_ne!(trusted.inspection, trusted.facts);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn incomplete_generated_draft_maps_to_protocol_error_envelope() {
        let trusted_root = TrustedRoot {
            root: PathBuf::from("/trusted/authored"),
        };
        let malformed = json!({ "app": { "id": "generated-app" } });
        let response = match collision_payload_from_generated_draft(&trusted_root, &malformed) {
            Ok(_) => panic!("malformed generated draft must fail"),
            Err(_) => generator_protocol_error(),
        };
        assert_eq!(
            api_error_code(&response),
            Some("app_generator_protocol_error")
        );
    }

    #[test]
    fn collision_payload_uses_only_generated_draft_and_trusted_root() {
        let trusted_root = TrustedRoot {
            root: PathBuf::from("/trusted/authored"),
        };
        let generated_app = json!({ "id": "generated-app", "metadata": { "trusted": true } });
        let generated_recipe = json!({ "id": "app.generated.install", "steps": [] });
        let payload = collision_payload_from_generated_draft(
            &trusted_root,
            &json!({
                "app": generated_app,
                "recipe": generated_recipe,
                "editableApp": { "id": "frontend-app" },
                "editableRecipe": { "id": "frontend-recipe" },
                "fingerprint": "frontend-fingerprint"
            }),
        )
        .unwrap();
        assert_eq!(payload["authoredRoot"], "/trusted/authored");
        assert_eq!(payload["app"]["id"], "generated-app");
        assert_eq!(payload["recipe"]["id"], "app.generated.install");
        assert!(payload.get("recipeId").is_none());
        assert!(payload.get("fingerprint").is_none());
        assert!(!payload.to_string().contains("frontend-app"));
        assert!(!payload.to_string().contains("frontend-recipe"));
    }

    #[test]
    fn direct_frontend_collision_review_is_rejected() {
        let response = check_app_recipe_collisions().unwrap();
        assert_eq!(
            api_error_code(&response),
            Some("app_recipe_collision_review_requires_generation")
        );
    }

    #[test]
    fn apk_and_root_validation_reject_unsafe_selections() {
        let directory = temp_path("validation");
        fs::create_dir_all(&directory).unwrap();

        let wrong_extension = directory.join("selected.zip");
        fs::write(&wrong_extension, b"not-an-apk").unwrap();
        assert!(validate_apk(&wrong_extension).is_err());
        assert!(validate_apk(&directory).is_err());

        assert!(validate_authored_root(&directory).is_err());
        fs::create_dir(directory.join("apps")).unwrap();
        fs::create_dir(directory.join("recipes")).unwrap();
        assert!(validate_authored_root(&directory).is_ok());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn handles_are_scoped_and_file_identity_detects_material_change() {
        let mut registry = GeneratorRegistry::default();
        let first = registry.begin();
        let second = registry.begin();
        let handle = registry.allocate("apk");
        let path = temp_path("identity").with_extension("apk");
        fs::write(&path, b"first").unwrap();
        let (_, identity) = validate_apk(&path).unwrap();
        registry.session_mut(&first).unwrap().apks.insert(
            handle.clone(),
            TrustedApk {
                path: path.clone(),
                identity: identity.clone(),
                inspection: None,
                inspection_handle: None,
                facts: None,
                remote_asset_handle: None,
            },
        );
        assert!(registry.session(&first).unwrap().apks.contains_key(&handle));
        assert!(!registry
            .session(&second)
            .unwrap()
            .apks
            .contains_key(&handle));
        fs::write(&path, b"second-longer").unwrap();
        assert_ne!(current_file_identity(&path), Some(identity));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn github_repository_analysis_uses_expected_paths_headers_and_assets() {
        let responses = vec![
            FakeGitHubResponse::json(
                200,
                json!({
                    "full_name": "azahar-emu/azahar",
                    "name": "azahar",
                    "description": "A public emulator repository"
                }),
            ),
            FakeGitHubResponse::json(
                200,
                json!([
                    {
                        "draft": false,
                        "prerelease": true,
                        "tag_name": "2121-beta",
                        "name": "Azahar 2121 beta",
                        "published_at": "2026-07-02T00:00:00Z",
                        "assets": []
                    },
                    {
                        "draft": false,
                        "prerelease": false,
                        "tag_name": "2120.1",
                        "name": "Azahar 2120.1",
                        "published_at": "2026-07-01T00:00:00Z",
                        "assets": [{
                            "name": "azahar-2120.1-android.apk",
                            "size": 42,
                            "content_type": "application/vnd.android.package-archive",
                            "browser_download_url": "https://github.com/azahar-emu/azahar/releases/download/2120.1/azahar.apk"
                        }]
                    }
                ]),
            ),
        ];
        let (api_root, captured) = spawn_fake_github(responses);
        let source =
            normalize_remote_source("github_repository", "https://github.com/azahar-emu/azahar")
                .unwrap();
        let analysis = analyze_github_source_with_api_root(
            &fake_github_client(),
            "github_repository",
            &source,
            false,
            &api_root,
            1_000,
        )
        .unwrap();

        assert_eq!(analysis.repository["fullName"], "azahar-emu/azahar");
        assert_eq!(analysis.releases.len(), 2);
        assert_eq!(analysis.releases[0].tag, "2121-beta");
        assert!(analysis.releases[0].prerelease);
        assert!(analysis.releases[0].assets.is_empty());
        assert_eq!(analysis.releases[1].assets.len(), 1);
        assert_eq!(
            analysis.releases[1].assets[0].file_name,
            "azahar-2120.1-android.apk"
        );
        let requests = captured.lock().unwrap();
        assert_eq!(
            requests.as_slice(),
            &[
                CapturedGitHubRequest {
                    path: "/repos/azahar-emu/azahar".to_string(),
                    user_agent: Some(HTTP_USER_AGENT.to_string()),
                    accept: Some("application/vnd.github+json".to_string()),
                    api_version: Some(GITHUB_API_VERSION.to_string()),
                },
                CapturedGitHubRequest {
                    path: "/repos/azahar-emu/azahar/releases?per_page=30".to_string(),
                    user_agent: Some(HTTP_USER_AGENT.to_string()),
                    accept: Some("application/vnd.github+json".to_string()),
                    api_version: Some(GITHUB_API_VERSION.to_string()),
                },
            ]
        );
    }

    #[test]
    fn github_exact_release_analysis_uses_expected_path_and_headers() {
        let responses = vec![
            FakeGitHubResponse::json(
                200,
                json!({ "full_name": "azahar-emu/azahar", "name": "azahar" }),
            ),
            FakeGitHubResponse::json(
                200,
                json!({
                    "draft": false,
                    "prerelease": false,
                    "tag_name": "2120.1",
                    "assets": []
                }),
            ),
        ];
        let (api_root, captured) = spawn_fake_github(responses);
        let source = normalize_remote_source(
            "github_release",
            "https://github.com/azahar-emu/azahar/releases/tag/2120.1",
        )
        .unwrap();
        analyze_github_source_with_api_root(
            &fake_github_client(),
            "github_release",
            &source,
            false,
            &api_root,
            1_000,
        )
        .unwrap();

        let requests = captured.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/repos/azahar-emu/azahar");
        assert_eq!(
            requests[1].path,
            "/repos/azahar-emu/azahar/releases/tags/2120.1"
        );
        for request in requests.iter() {
            assert_eq!(request.user_agent.as_deref(), Some(HTTP_USER_AGENT));
            assert_eq!(
                request.accept.as_deref(),
                Some("application/vnd.github+json")
            );
            assert_eq!(request.api_version.as_deref(), Some(GITHUB_API_VERSION));
        }
    }

    #[test]
    fn github_primary_rate_limit_uses_bounded_reset_advisory() {
        let error = fake_github_error(
            FakeGitHubResponse::json(403, json!({ "secret": "must-not-cross" }))
                .with_header("x-ratelimit-remaining", "0")
                .with_header("x-ratelimit-reset", "1061"),
            1_000,
        );
        assert_eq!(api_error_code(&error), Some("github_rate_limited"));
        assert_eq!(
            api_error_message(&error),
            Some(
                "GitHub API requests are rate-limited. You may be able to try again in about 2 minutes."
            )
        );
        assert!(!error.to_string().contains("must-not-cross"));
    }

    #[test]
    fn github_secondary_and_429_limits_prefer_retry_after() {
        for status in [403, 429] {
            let error = fake_github_error(
                FakeGitHubResponse::json(status, json!({ "secret": "must-not-cross" }))
                    .with_header("Retry-After", "120")
                    .with_header("x-ratelimit-remaining", "17")
                    .with_header("x-ratelimit-reset", "1001"),
                1_000,
            );
            assert_eq!(api_error_code(&error), Some("github_rate_limited"));
            assert_eq!(
                api_error_message(&error),
                Some(
                    "GitHub API requests are rate-limited. You may be able to try again in about 120 seconds."
                )
            );
            assert!(!error.to_string().contains("must-not-cross"));
        }
    }

    #[test]
    fn github_429_without_valid_metadata_remains_rate_limited() {
        for response in [
            FakeGitHubResponse::json(429, json!({ "secret": "must-not-cross" })),
            FakeGitHubResponse::json(429, json!({ "secret": "must-not-cross" }))
                .with_header("Retry-After", "86401")
                .with_header("x-ratelimit-remaining", "not-a-number"),
        ] {
            let error = fake_github_error(response, 1_000);
            assert_eq!(api_error_code(&error), Some("github_rate_limited"));
            assert_eq!(
                api_error_message(&error),
                Some("GitHub API requests are rate-limited. Try again later.")
            );
            assert!(!error.to_string().contains("must-not-cross"));
        }
    }

    #[test]
    fn github_invalid_rate_limit_metadata_is_ignored() {
        let cases = [
            (
                FakeGitHubResponse::json(403, json!({ "secret": "must-not-cross" }))
                    .with_header("x-ratelimit-remaining", "-1")
                    .with_header("Retry-After", "seconds"),
                "github_service_failed",
            ),
            (
                FakeGitHubResponse::json(403, json!({ "secret": "must-not-cross" }))
                    .with_header("Retry-After", "18446744073709551616"),
                "github_service_failed",
            ),
            (
                FakeGitHubResponse::json(403, json!({ "secret": "must-not-cross" }))
                    .with_header("Retry-After", "86401"),
                "github_service_failed",
            ),
        ];
        for (response, expected_code) in cases {
            let error = fake_github_error(response, 1_000);
            assert_eq!(api_error_code(&error), Some(expected_code));
            assert!(!error.to_string().contains("must-not-cross"));
        }

        for reset in ["999", "87401", "18446744073709551616", "not-a-number"] {
            let error = fake_github_error(
                FakeGitHubResponse::json(403, json!({ "secret": "must-not-cross" }))
                    .with_header("x-ratelimit-remaining", "0")
                    .with_header("x-ratelimit-reset", reset),
                1_000,
            );
            assert_eq!(api_error_code(&error), Some("github_rate_limited"));
            assert_eq!(
                api_error_message(&error),
                Some("GitHub API requests are rate-limited. Try again later.")
            );
            assert!(!error.to_string().contains("must-not-cross"));
        }
    }

    #[test]
    fn github_non_rate_limited_http_statuses_are_distinct_and_redacted() {
        for (status, expected_code) in [
            (403, "github_service_failed"),
            (404, "github_repository_unavailable"),
            (500, "github_service_failed"),
        ] {
            let error = fake_github_error(
                FakeGitHubResponse::json(status, json!({ "secret": "must-not-cross" })),
                1_000,
            );
            assert_eq!(api_error_code(&error), Some(expected_code));
            assert!(!error.to_string().contains("must-not-cross"));
            assert_eq!(error["error"]["details"], json!({}));
        }
    }

    #[test]
    fn github_malformed_and_oversized_responses_remain_bounded() {
        let malformed = fake_github_error(
            FakeGitHubResponse {
                status: 200,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: b"secret-malformed-body".to_vec(),
                framing: FakeResponseFraming::ContentLength,
            },
            1_000,
        );
        assert_eq!(api_error_code(&malformed), Some("github_response_invalid"));
        assert!(!malformed.to_string().contains("secret-malformed-body"));

        let declared = fake_github_error(
            FakeGitHubResponse {
                status: 200,
                headers: vec![],
                body: Vec::new(),
                framing: FakeResponseFraming::DeclaredLength(
                    usize::try_from(MAX_METADATA_BYTES).unwrap() + 1,
                ),
            },
            1_000,
        );
        assert_eq!(api_error_code(&declared), Some("github_response_too_large"));

        let streamed = fake_github_error(
            FakeGitHubResponse {
                status: 200,
                headers: vec![],
                body: vec![b'x'; usize::try_from(MAX_METADATA_BYTES).unwrap() + 1],
                framing: FakeResponseFraming::Chunked,
            },
            1_000,
        );
        assert_eq!(api_error_code(&streamed), Some("github_response_too_large"));
    }

    #[test]
    fn github_transport_and_response_read_failures_are_distinct_and_redacted() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(stream);
        });
        let transport = get_github_json_bounded(
            &fake_github_client(),
            &format!("http://{address}/secret-transport-path"),
            1_000,
        )
        .unwrap_err();
        assert_eq!(api_error_code(&transport), Some("github_transport_failed"));
        assert!(!transport.to_string().contains("secret-transport-path"));

        let read_failure = fake_github_error(
            FakeGitHubResponse {
                status: 200,
                headers: vec![],
                body: b"secret-truncated-body".to_vec(),
                framing: FakeResponseFraming::DeclaredLength(1_000),
            },
            1_000,
        );
        assert_eq!(
            api_error_code(&read_failure),
            Some("github_response_failed")
        );
        assert!(!read_failure.to_string().contains("secret-truncated-body"));
    }

    #[test]
    fn remote_source_normalization_accepts_supported_github_shapes() {
        let repository = normalize_remote_source(
            "github_repository",
            "https://github.com/example/project.git/",
        )
        .unwrap();
        assert_eq!(repository.repository.as_deref(), Some("example/project"));
        assert_eq!(
            repository.url.as_str(),
            "https://github.com/example/project"
        );

        let release = normalize_remote_source(
            "github_release",
            "https://github.com/example/project/releases/tag/v1.2.3",
        )
        .unwrap();
        assert_eq!(release.release_tag.as_deref(), Some("v1.2.3"));
    }

    #[test]
    fn remote_source_normalization_accepts_gitlab_and_forgejo_shapes() {
        let gitlab = normalize_remote_source(
            "gitlab_release",
            "https://gitlab.com/example/group/project/-/releases/v1.2.3",
        )
        .unwrap();
        assert_eq!(gitlab.provider.as_deref(), Some("gitlab"));
        assert_eq!(gitlab.repository.as_deref(), Some("example/group/project"));
        assert_eq!(gitlab.release_tag.as_deref(), Some("v1.2.3"));
        assert_eq!(
            gitlab.url.as_str(),
            "https://gitlab.com/example/group/project/-/releases/v1.2.3"
        );

        let forgejo = normalize_remote_source(
            "forgejo_release",
            "https://codeberg.org/example/project/releases/tag/v2.0.0",
        )
        .unwrap();
        assert_eq!(forgejo.provider.as_deref(), Some("forgejo"));
        assert_eq!(forgejo.base_url.as_deref(), Some("https://codeberg.org"));
        assert_eq!(forgejo.repository.as_deref(), Some("example/project"));
        assert_eq!(forgejo.release_tag.as_deref(), Some("v2.0.0"));
    }

    #[test]
    fn remote_source_normalization_rejects_unsafe_or_unsupported_addresses() {
        assert!(
            normalize_remote_source("github_repository", "http://github.com/example/project",)
                .is_err()
        );
        assert!(normalize_remote_source(
            "github_repository",
            "https://user@example.com/example/project",
        )
        .is_err());
        assert!(normalize_remote_source("direct_apk", "https://127.0.0.1/app.apk",).is_err());
        assert!(normalize_remote_source("direct_apk", "https://[::1]/app.apk").is_err());
        assert!(normalize_remote_source("direct_apk", "https://169.254.10.10/app.apk").is_err());
        assert!(normalize_remote_source("direct_apk", "https://localhost/app.apk").is_err());
        assert!(normalize_remote_source("direct_apk", "https://example.com/app.apk#frag").is_err());
        assert!(normalize_remote_source("direct_apk", "https://user@example.com/app.apk").is_err());
        // Provider addresses are canonicalized from their path segments, so a
        // query there has no durable meaning and is refused instead of dropped.
        assert!(normalize_remote_source(
            "github_repository",
            "https://github.com/example/project?tab=readme",
        )
        .is_err());
    }

    #[test]
    fn direct_url_can_defer_apk_filename_to_response_headers() {
        let source = normalize_remote_source("direct_apk", "https://example.com/download").unwrap();
        assert_eq!(source.url.as_str(), "https://example.com/download");
        // A durable query string is part of the author's original address.
        let query =
            normalize_remote_source("direct_apk", "https://example.com/download?variant=stable")
                .unwrap();
        assert_eq!(
            query.url.as_str(),
            "https://example.com/download?variant=stable"
        );
    }

    #[test]
    fn public_source_guard_refuses_internal_addresses_and_unsafe_redirect_hops() {
        for refused in [
            "http://mirror.example.com/app.apk",
            "https://user:secret@mirror.example.com/app.apk",
            "https://localhost/app.apk",
            "https://status.localhost/app.apk",
            "https://127.0.0.1/app.apk",
            "https://10.0.0.7/app.apk",
            "https://172.16.9.9/app.apk",
            "https://192.168.1.4/app.apk",
            "https://169.254.10.10/app.apk",
            "https://100.64.0.1/app.apk",
            "https://192.0.2.10/app.apk",
            "https://198.18.0.5/app.apk",
            "https://224.0.0.1/app.apk",
            "https://0.0.0.0/app.apk",
            "https://240.0.0.9/app.apk",
            "https://255.255.255.255/app.apk",
            "https://[::1]/app.apk",
            "https://[::]/app.apk",
            "https://[fe80::1]/app.apk",
            "https://[fc00::1]/app.apk",
            "https://[ff02::1]/app.apk",
            "https://[2001:db8::1]/app.apk",
            "https://[2001::1]/app.apk",
            "https://[2002:0a00::1]/app.apk",
            "https://[::ffff:10.0.0.1]/app.apk",
            "https://[::ffff:192.168.1.1]/app.apk",
        ] {
            let url = Url::parse(refused).unwrap();
            assert!(!safe_remote_url(&url), "{refused} must be refused");
        }
        for allowed in [
            "https://mirror.example.com/app.apk",
            "https://mirror.example.com/app.apk?channel=stable&signature=abc",
            "https://1.1.1.1/app.apk",
            "https://[2606:4700:4700::1111]/app.apk",
            "https://[2001:4860:4860::8888]/app.apk",
            "https://[::ffff:1.1.1.1]/app.apk",
        ] {
            let url = Url::parse(allowed).unwrap();
            assert!(safe_remote_url(&url), "{allowed} must be accepted");
        }
        // Every address a name answers with must be public, so a hostname that
        // resolves to an internal address is refused instead of racing the
        // client's own connection choice.
        assert!(resolved_public_addresses(&[SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            443,
        )]));
        assert!(!resolved_public_addresses(&[]));
        assert!(!resolved_public_addresses(&[
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 443),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)), 443),
        ]));
        assert!(!resolved_public_addresses(&[SocketAddr::new(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            443,
        )]));
        assert_eq!(
            resolve_public_host("localhost"),
            Err("remote_host_not_public")
        );
        // Redirect hops keep the original address policy and the bounded budget.
        let public_hop = Url::parse("https://mirror.example.com/app.apk?signature=abc").unwrap();
        assert!(redirect_target_allowed(0, &public_hop));
        assert!(redirect_target_allowed(MAX_REDIRECTS - 1, &public_hop));
        assert!(!redirect_target_allowed(MAX_REDIRECTS, &public_hop));
        for refused in [
            "https://127.0.0.1/app.apk",
            "https://[::1]/app.apk",
            "https://169.254.10.10/app.apk",
            "http://mirror.example.com/app.apk",
            "https://user:secret@mirror.example.com/app.apk",
            "https://localhost/app.apk",
        ] {
            assert!(!redirect_target_allowed(0, &Url::parse(refused).unwrap()));
        }
    }

    #[test]
    fn trusted_sha256_request_value_forwards_only_non_empty_pinned_values() {
        assert_eq!(
            trusted_sha256_request_value("pinned_remote_asset", Some(" AABB ")).unwrap(),
            Some(" AABB ")
        );
        assert_eq!(
            trusted_sha256_request_value("pinned_remote_asset", Some(" \t\r\n")).unwrap(),
            None
        );
        assert_eq!(
            trusted_sha256_request_value("latest_compatible_release", None).unwrap(),
            None
        );
    }

    #[test]
    fn trusted_sha256_request_value_rejects_unsupported_strategies() {
        for strategy in ["latest_compatible_release", "user_provided_apk"] {
            let error = trusted_sha256_request_value(strategy, Some(&"A".repeat(64)))
                .expect_err("non-pinned checksum should be rejected");
            assert_eq!(
                error["error"]["code"],
                "apk_trusted_sha256_strategy_unsupported"
            );
        }
    }

    /// Scripted sidecar transport for native review and publication tests.
    ///
    /// Every request is recorded verbatim so the tests can assert what the
    /// native boundary actually sent, while the handler decides the response or
    /// a transport failure per operation.
    type ScriptedTransportHandler = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

    struct ScriptedTransport {
        handler: Box<ScriptedTransportHandler>,
        requests: Mutex<Vec<(String, Value)>>,
    }

    impl ScriptedTransport {
        fn new<F>(handler: F) -> Self
        where
            F: Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static,
        {
            Self {
                handler: Box::new(handler),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests_for(&self, operation: &str) -> Vec<Value> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| name == operation)
                .map(|(_, payload)| payload.clone())
                .collect()
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    impl AppGeneratorTransport for ScriptedTransport {
        fn request(&self, operation: &str, payload: Value) -> Result<Value, String> {
            self.requests
                .lock()
                .unwrap()
                .push((operation.to_string(), payload.clone()));
            (self.handler)(operation, &payload)
        }
    }

    /// Create one temporary authored root with apps and recipes directories.
    fn temp_authored_root(label: &str) -> (PathBuf, TrustedRoot) {
        let path = temp_path(label);
        fs::create_dir_all(path.join("apps")).unwrap();
        fs::create_dir_all(path.join("recipes")).unwrap();
        let root = validate_authored_root(&path).unwrap();
        (path, root)
    }

    /// One prepared session with a trusted root, inspected APK, and one review.
    ///
    /// The fixture mirrors the state a successful review leaves behind, so save
    /// and status tests exercise the real gates instead of hand-built records.
    struct ReviewFixture {
        state: AppGeneratorState,
        session_handle: String,
        review_handle: String,
        root_path: PathBuf,
        canonical_root: PathBuf,
        apk_path: PathBuf,
        apk_identity: FileIdentity,
    }

    impl ReviewFixture {
        fn create(label: &str) -> Self {
            let (root_path, root) = temp_authored_root(label);
            let apk = permission_test_apk(label);
            let state = AppGeneratorState::default();
            let mut registry = lock_registry(&state).unwrap();
            let session_handle = registry.begin();
            let session = registry.session_mut(&session_handle).unwrap();
            session
                .roots
                .insert("root-handle".to_string(), root.clone());
            session.active_root = Some("root-handle".to_string());
            session.apks.insert("apk-handle".to_string(), apk.clone());
            session.reviews.insert(
                "app-review-handle".to_string(),
                ReviewRecord {
                    request_id: 7,
                    backend_review_id: Some("backend-review-handle".to_string()),
                    binding: ReviewBinding::Local {
                        apk_handle: "apk-handle".to_string(),
                        apk_identity: apk.identity.clone(),
                        inspection_handle: apk
                            .inspection_handle
                            .clone()
                            .expect("fixture APK retains an inspection handle"),
                    },
                    existing_app: None,
                    root_path: root.root.clone(),
                    blocking: false,
                    app_relative_path: Some("apps/example.yaml".to_string()),
                    recipe_relative_path: Some("recipes/app.example.install.yaml".to_string()),
                },
            );
            drop(registry);
            Self {
                state,
                session_handle,
                review_handle: "app-review-handle".to_string(),
                root_path,
                canonical_root: root.root,
                apk_path: apk.path,
                apk_identity: apk.identity,
            }
        }

        fn cleanup(self) {
            let _ = fs::remove_file(&self.apk_path);
            let _ = fs::remove_dir_all(&self.root_path);
        }
    }

    /// Build the trusted draft inputs the fixture's retained APK and root resolve.
    fn fixture_review_inputs(fixture: &ReviewFixture) -> ResolvedDraftInputs {
        let registry = lock_registry(&fixture.state).unwrap();
        let session = registry.session(&fixture.session_handle).unwrap();
        let apk = session.apks.get("apk-handle").unwrap();
        ResolvedDraftInputs {
            root: session.roots.get("root-handle").cloned(),
            facts: apk.facts.clone().expect("fixture APK retains facts"),
            permission_automation: None,
            permission_automation_enabled: false,
            existing_app: None,
            binding: Some(ReviewBinding::Local {
                apk_handle: "apk-handle".to_string(),
                apk_identity: apk.identity.clone(),
                inspection_handle: apk
                    .inspection_handle
                    .clone()
                    .expect("fixture APK retains an inspection handle"),
            }),
            remote: None,
        }
    }

    /// Attach one root-bound existing-App selection to trusted draft inputs.
    fn with_existing_app(
        mut inputs: ResolvedDraftInputs,
        fixture: &ReviewFixture,
    ) -> ResolvedDraftInputs {
        inputs.existing_app = Some(BoundExistingApp {
            handle: "existing-app-handle".to_string(),
            selection: TrustedExistingApp {
                root_path: fixture.canonical_root.clone(),
                snapshot_id: "backend-snapshot-1".to_string(),
                app_path: "apps/armsx1.yaml".to_string(),
                package_id: Some("com.example.inspected".to_string()),
            },
        });
        inputs
    }

    #[test]
    fn save_rejects_a_stale_request_id_without_publishing() {
        let fixture = ReviewFixture::create("publication-stale-request");
        let transport = ScriptedTransport::new(|operation, _| {
            panic!("a stale save must not reach the backend: {operation}")
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            8,
        )
        .unwrap();
        assert_eq!(api_error_code(&response), Some("review_invalidated"));
        assert_eq!(transport.request_count(), 0);
        // The stale attempt consumed the review, so only a new review can publish.
        let repeated = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        assert_eq!(api_error_code(&repeated), Some("review_invalidated"));
        fixture.cleanup();
    }

    #[test]
    fn save_publishes_only_the_retained_review_identity_and_consumes_the_handle() {
        let fixture = ReviewFixture::create("publication-identity");
        let transport = ScriptedTransport::new(|operation, _| match operation {
            "saveAppGeneratorReview" => Ok(success(json!({ "outcome": "saved" }))),
            "openRecipe" => Ok(success(
                json!({ "document": { "documentId": "document-1" } }),
            )),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        assert_eq!(result["outcome"], "saved");
        assert_eq!(result["app"]["appPath"], "apps/example.yaml");
        assert_eq!(
            result["recipe"]["recipePath"],
            "recipes/app.example.install.yaml"
        );
        assert_eq!(result["recipe"]["documentId"], "document-1");
        assert!(result["recipe"].get("openError").is_none());

        let saves = transport.requests_for("saveAppGeneratorReview");
        assert_eq!(saves.len(), 1);
        let payload = saves[0].as_object().unwrap();
        let mut keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["operationId", "requestId", "reviewId"]);
        assert_eq!(payload["reviewId"], "backend-review-handle");
        assert_eq!(payload["requestId"], 7);
        let operation_id = payload["operationId"].as_str().unwrap();

        // A confirmed saved record answers status from native memory.
        let status =
            publication_status_via_transport(&transport, &fixture.state, operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "saved");
        assert_eq!(status["confirmed"], true);
        assert_eq!(status["detail"]["recipe"]["documentId"], "document-1");
        assert!(transport
            .requests_for("appGeneratorPublicationStatus")
            .is_empty());

        // Repeating the consumed handle cannot publish a second pair.
        let repeated = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        assert_eq!(api_error_code(&repeated), Some("review_invalidated"));
        assert_eq!(transport.requests_for("saveAppGeneratorReview").len(), 1);
        fixture.cleanup();
    }

    #[test]
    fn lost_publication_response_stays_unknown_and_is_never_retried() {
        let fixture = ReviewFixture::create("publication-lost-response");
        let transport = ScriptedTransport::new(|operation, _| match operation {
            "saveAppGeneratorReview" => Err("sidecar exited during publication".to_string()),
            "appGeneratorPublicationStatus" => Err("sidecar restarting".to_string()),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        assert_eq!(result["outcome"], "unknown");
        assert_eq!(
            result["message"].as_str(),
            Some(UNCONFIRMED_PUBLICATION_MESSAGE)
        );
        let operation_id = result["operationId"].as_str().unwrap().to_string();
        assert_eq!(transport.requests_for("saveAppGeneratorReview").len(), 1);

        // The uncertain attempt stays uncertain instead of becoming a failure.
        let status =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "unknown");
        assert_eq!(status["confirmed"], false);
        assert_eq!(status["detail"]["reason"], "status_unavailable");
        let recorded_at = status["recordedAt"].as_str().unwrap();
        assert_eq!(recorded_at.len(), 20);
        assert!(recorded_at.ends_with('Z'));
        assert_eq!(transport.requests_for("saveAppGeneratorReview").len(), 1);
        fixture.cleanup();
    }

    #[test]
    fn sidecar_restart_clears_handles_but_preserves_unknown_publication_memory() {
        let fixture = ReviewFixture::create("publication-restart");
        let transport = ScriptedTransport::new(|operation, _| match operation {
            "saveAppGeneratorReview" => Err("transport closed".to_string()),
            "appGeneratorPublicationStatus" => Ok(success(json!({
                "operationId": "restarted",
                "outcome": "unknown",
                "confirmed": false,
                "recordedAt": Value::Null,
                "detail": { "reason": "no_record" },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        let operation_id = result["operationId"].as_str().unwrap().to_string();

        // A sidecar restart drops every session handle.
        fixture.state.clear().unwrap();
        let after_restart = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        assert_eq!(
            api_error_code(&after_restart),
            Some("app_generator_session_expired")
        );

        // A restart that forgot the operation reports no_record, which stays unknown.
        let status =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "unknown");
        assert_eq!(status["confirmed"], false);
        assert_eq!(status["detail"]["reason"], "no_record");
        assert!(status["recordedAt"].as_str().is_some());
        assert_eq!(transport.requests_for("saveAppGeneratorReview").len(), 1);
        fixture.cleanup();
    }

    #[test]
    fn saved_publication_survives_open_failure_and_status_never_requires_opening() {
        let fixture = ReviewFixture::create("publication-open-failure");
        let transport = ScriptedTransport::new(|operation, _| match operation {
            "saveAppGeneratorReview" => Ok(success(json!({
                "outcome": "saved",
                "createdFiles": [],
                "replacedFiles": [],
            }))),
            "openRecipe" => Ok(api_error(
                "app_recipe_open_failed",
                "The saved Recipe could not be opened.",
                json!({ "reason": "missing_catalog" }),
            )),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        assert_eq!(result["outcome"], "saved");
        assert_eq!(result["recipe"]["documentId"], Value::Null);
        assert_eq!(
            result["recipe"]["openError"]["code"],
            "app_recipe_open_failed"
        );
        assert_eq!(
            result["recipe"]["openError"]["details"]["reason"],
            "missing_catalog"
        );
        let operation_id = result["operationId"].as_str().unwrap();

        // The saved record never depends on the editor document opening.
        let status =
            publication_status_via_transport(&transport, &fixture.state, operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "saved");
        assert_eq!(status["confirmed"], true);
        assert_eq!(status["detail"]["recipe"]["documentId"], Value::Null);
        assert_eq!(
            status["detail"]["recipe"]["openError"]["code"],
            "app_recipe_open_failed"
        );
        assert!(transport
            .requests_for("appGeneratorPublicationStatus")
            .is_empty());
        assert_eq!(transport.requests_for("saveAppGeneratorReview").len(), 1);
        fixture.cleanup();
    }

    #[test]
    fn failed_publication_maps_rollback_relative_and_is_recorded() {
        let fixture = ReviewFixture::create("publication-failed");
        let absolute_recipe = fixture
            .canonical_root
            .join("recipes/app.example.install.yaml")
            .to_string_lossy()
            .to_string();
        let transport = ScriptedTransport::new(move |operation, _| match operation {
            "saveAppGeneratorReview" => Ok(success(json!({
                "outcome": "failed",
                "error": {
                    "code": "publication_denied",
                    "message": "The destination already exists.",
                    "details": {},
                },
                "rollback": {
                    "recipeRemoved": true,
                    "recipeRecoveryRequired": false,
                    "recipePath": absolute_recipe.clone(),
                },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        assert_eq!(result["outcome"], "failed");
        assert_eq!(result["error"]["code"], "publication_denied");
        assert_eq!(result["rollback"]["recipeRemoved"], true);
        assert_eq!(
            result["rollback"]["recipePath"],
            "recipes/app.example.install.yaml"
        );
        assert!(transport.requests_for("openRecipe").is_empty());
        let operation_id = result["operationId"].as_str().unwrap();

        let status =
            publication_status_via_transport(&transport, &fixture.state, operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "failed");
        assert_eq!(status["confirmed"], true);
        fixture.cleanup();
    }

    #[test]
    fn backend_confirmation_upgrades_an_uncertain_publication_record() {
        let fixture = ReviewFixture::create("publication-confirmation");
        let absolute_app = fixture
            .canonical_root
            .join("apps/example.yaml")
            .to_string_lossy()
            .to_string();
        let absolute_recipe = fixture
            .canonical_root
            .join("recipes/app.example.install.yaml")
            .to_string_lossy()
            .to_string();
        let transport = ScriptedTransport::new(move |operation, payload| match operation {
            "saveAppGeneratorReview" => Err("response lost after commit".to_string()),
            "appGeneratorPublicationStatus" => Ok(success(json!({
                "operationId": payload["operationId"].clone(),
                "outcome": "saved",
                "confirmed": true,
                "recordedAt": "2026-10-15T20:00:00Z",
                "detail": {
                    "app": { "appPath": absolute_app.clone() },
                    "recipe": { "recipePath": absolute_recipe.clone() },
                    "createdFiles": [absolute_app.clone()],
                    "replacedFiles": [],
                },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let operation_id = success_result(&response).unwrap()["operationId"]
            .as_str()
            .unwrap()
            .to_string();

        let status =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "saved");
        assert_eq!(status["confirmed"], true);
        assert_eq!(status["recordedAt"], "2026-10-15T20:00:00Z");
        assert_eq!(status["detail"]["app"]["appPath"], "apps/example.yaml");
        assert_eq!(
            status["detail"]["recipe"]["recipePath"],
            "recipes/app.example.install.yaml"
        );
        assert_eq!(
            status["detail"]["createdFiles"],
            json!(["apps/example.yaml"])
        );

        // The upgrade is sticky: a later query answers from native memory.
        let adopted =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let adopted = success_result(&adopted).unwrap();
        assert_eq!(adopted["outcome"], "saved");
        assert_eq!(
            transport
                .requests_for("appGeneratorPublicationStatus")
                .len(),
            1
        );
        fixture.cleanup();
    }

    #[test]
    fn lost_publication_response_upgrades_to_confirmed_failure_with_rollback_detail() {
        let fixture = ReviewFixture::create("publication-confirmation-failure");
        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "saveAppGeneratorReview" => Err("response lost after commit".to_string()),
            "appGeneratorPublicationStatus" => Ok(success(json!({
                "operationId": payload["operationId"].clone(),
                "outcome": "failed",
                "confirmed": true,
                "recordedAt": "2026-10-15T20:30:00Z",
                "detail": {
                    "ok": true,
                    "outcome": "failed",
                    "operationId": payload["operationId"].clone(),
                    "app": { "appPath": "apps/example.yaml" },
                    "recipe": { "recipePath": "recipes/app.example.install.yaml" },
                    "createdFiles": [],
                    "replacedFiles": [],
                    "rollback": {
                        "recipeRemoved": false,
                        "recipeRecoveryRequired": true,
                        "recipePath": "recipes/app.example.install.yaml",
                    },
                    "error": {
                        "code": "app_definition_publish_failed",
                        "message": "The App Definition could not be published atomically",
                        "details": { "reason": "app_definition_publish_failed" },
                    },
                    "message": Value::Null,
                },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &fixture.review_handle,
            7,
        )
        .unwrap();
        let operation_id = success_result(&response).unwrap()["operationId"]
            .as_str()
            .unwrap()
            .to_string();

        // The lost save response left the attempt unknown; the backend record
        // confirms the failure, so the status must surface its real detail.
        let status =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["outcome"], "failed");
        assert_eq!(status["confirmed"], true);
        assert_eq!(status["recordedAt"], "2026-10-15T20:30:00Z");
        assert_eq!(
            status["detail"]["error"]["code"],
            "app_definition_publish_failed"
        );
        assert_eq!(
            status["detail"]["error"]["message"],
            "The App Definition could not be published atomically"
        );
        assert_eq!(
            status["detail"]["error"]["details"]["reason"],
            "app_definition_publish_failed"
        );
        assert_eq!(status["detail"]["rollback"]["recipeRemoved"], false);
        assert_eq!(status["detail"]["rollback"]["recipeRecoveryRequired"], true);
        assert_eq!(
            status["detail"]["rollback"]["recipePath"],
            "recipes/app.example.install.yaml"
        );

        // The confirmed failure is sticky: a later query answers from native memory.
        let adopted =
            publication_status_via_transport(&transport, &fixture.state, &operation_id).unwrap();
        let adopted = success_result(&adopted).unwrap();
        assert_eq!(adopted["outcome"], "failed");
        assert_eq!(
            adopted["detail"]["rollback"]["recipeRecoveryRequired"],
            true
        );
        assert_eq!(
            transport
                .requests_for("appGeneratorPublicationStatus")
                .len(),
            1
        );
        fixture.cleanup();
    }

    #[test]
    fn status_uses_durable_backend_record_after_native_process_relaunch() {
        let state = AppGeneratorState::default();
        let transport = ScriptedTransport::new(|operation, _| match operation {
            "appGeneratorPublicationStatus" => Ok(success(json!({
                "operationId": "publication-42",
                "outcome": "saved",
                "confirmed": true,
                "recordedAt": null,
                "detail": { "outcome": "saved", "operationId": "publication-42" },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let status =
            publication_status_via_transport(&transport, &state, "publication-42").unwrap();
        let status = success_result(&status).unwrap();
        assert_eq!(status["operationId"], "publication-42");
        assert_eq!(status["outcome"], "saved");
        assert_eq!(status["confirmed"], true);
        assert_eq!(transport.request_count(), 1);
    }

    #[test]
    fn retired_direct_save_commands_refuse_with_review_required() {
        let refusal = retired_direct_publication_refusal();
        assert_eq!(refusal["ok"], false);
        assert_eq!(api_error_code(&refusal), Some("review_required"));
        assert!(api_error_message(&refusal)
            .expect("refusal carries a message")
            .contains("review"));
        assert!(refusal.get("result").is_none());
    }

    #[test]
    fn review_retains_immutable_mapping_and_rejects_backend_protocol_violations() {
        let fixture = ReviewFixture::create("review-mapping");
        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "reviewAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "reviewId": "backend-review-9",
                "destinations": {
                    "app": {
                        "fileName": "example.yaml",
                        "relativePath": "apps/example.yaml",
                    },
                    "recipe": {
                        "fileName": "app.example.install.yaml",
                        "relativePath": "recipes/app.example.install.yaml",
                    },
                },
                "target": { "mode": "new_app", "appPath": Value::Null },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = review_draft_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            fixture_review_inputs(&fixture),
            11,
            Some("fan.apk".to_string()),
            Some("app.fan.install".to_string()),
            Some("Fan APK".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        let review_handle = result["reviewHandle"].as_str().unwrap().to_string();
        assert!(review_handle.starts_with("app-review-"));
        assert!(result.get("reviewId").is_none());
        assert_eq!(result["requestId"], 11);

        let requests = transport.requests_for("reviewAppGeneratorDraft");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["appArtifactId"], "fan.apk");
        assert_eq!(requests[0]["recipeId"], "app.fan.install");
        assert_eq!(requests[0]["artifactName"], "Fan APK");
        assert!(requests[0].get("app").is_none());
        assert!(requests[0].get("recipe").is_none());
        assert!(requests[0].get("mappings").is_none());
        assert!(requests[0].get("permissionSelection").is_none());

        // The retained mapping, not a later request, is what publication consumes.
        {
            let registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session(&fixture.session_handle).unwrap();
            let record = session.reviews.get(&review_handle).unwrap();
            assert_eq!(
                record.backend_review_id.as_deref(),
                Some("backend-review-9")
            );
            assert_eq!(
                record.app_relative_path.as_deref(),
                Some("apps/example.yaml")
            );
            assert_eq!(
                record.recipe_relative_path.as_deref(),
                Some("recipes/app.example.install.yaml")
            );
            assert!(!record.blocking);
            assert_eq!(record.request_id, 11);
        }
        fixture.cleanup();
    }

    #[test]
    fn review_rejects_mismatched_request_ids_and_absolute_destinations() {
        let fixture = ReviewFixture::create("review-protocol");
        let mismatched = ScriptedTransport::new(|operation, _| match operation {
            "reviewAppGeneratorDraft" => Ok(success(json!({
                "requestId": 99,
                "blocking": false,
                "reviewId": "backend-review-x",
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = review_draft_via_transport(
            &mismatched,
            &fixture.state,
            &fixture.session_handle,
            fixture_review_inputs(&fixture),
            11,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            api_error_code(&response),
            Some("app_generator_protocol_error")
        );

        let absolute = ScriptedTransport::new(|operation, payload| match operation {
            "reviewAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "reviewId": "backend-review-y",
                "destinations": {
                    "app": { "relativePath": "/etc/apps/example.yaml" },
                },
                "target": { "mode": "new_app", "appPath": Value::Null },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = review_draft_via_transport(
            &absolute,
            &fixture.state,
            &fixture.session_handle,
            fixture_review_inputs(&fixture),
            11,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            api_error_code(&response),
            Some("app_recipe_destination_invalid")
        );

        // Neither protocol violation retained a review handle: the fixture's own
        // pre-seeded review is still the only record.
        {
            let registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session(&fixture.session_handle).unwrap();
            assert_eq!(session.reviews.len(), 1);
            assert!(session.reviews.contains_key("app-review-handle"));
        }
        fixture.cleanup();
    }

    #[test]
    fn review_without_a_retained_backend_pair_can_never_publish() {
        let fixture = ReviewFixture::create("review-unretained");
        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "reviewAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "destinations": {},
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = review_draft_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            fixture_review_inputs(&fixture),
            11,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let result = success_result(&response).unwrap();
        let review_handle = result["reviewHandle"].as_str().unwrap().to_string();
        assert_eq!(result["blocking"], true);

        let save = publish_review_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            &review_handle,
            11,
        )
        .unwrap();
        assert_eq!(api_error_code(&save), Some("review_blocking"));
        assert!(transport.requests_for("saveAppGeneratorReview").is_empty());
        fixture.cleanup();
    }

    #[test]
    fn review_dependency_changes_invalidate_the_retained_review() {
        let fixture = ReviewFixture::create("review-dependencies");
        let local_record = || ReviewRecord {
            request_id: 7,
            backend_review_id: Some("backend-review-handle".to_string()),
            binding: ReviewBinding::Local {
                apk_handle: "apk-handle".to_string(),
                apk_identity: fixture.apk_identity.clone(),
                inspection_handle: "apk-inspection-current".to_string(),
            },
            existing_app: Some(BoundExistingApp {
                handle: "existing-app-handle".to_string(),
                selection: TrustedExistingApp {
                    root_path: fixture.canonical_root.clone(),
                    snapshot_id: "backend-snapshot-1".to_string(),
                    app_path: "apps/armsx1.yaml".to_string(),
                    package_id: Some("com.example.inspected".to_string()),
                },
            }),
            root_path: fixture.canonical_root.clone(),
            blocking: false,
            app_relative_path: None,
            recipe_relative_path: None,
        };
        {
            let mut registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session_mut(&fixture.session_handle).unwrap();
            session.existing_apps.insert(
                "existing-app-handle".to_string(),
                TrustedExistingApp {
                    root_path: fixture.canonical_root.clone(),
                    snapshot_id: "backend-snapshot-1".to_string(),
                    app_path: "apps/armsx1.yaml".to_string(),
                    package_id: Some("com.example.inspected".to_string()),
                },
            );
            let record = local_record();
            assert!(review_dependencies_current(session, &record).is_ok());

            // A replaced inspection invalidates the review.
            session
                .apks
                .get_mut("apk-handle")
                .unwrap()
                .inspection_handle = Some("apk-inspection-replaced".to_string());
            assert!(review_dependencies_current(session, &record).is_err());
            session
                .apks
                .get_mut("apk-handle")
                .unwrap()
                .inspection_handle = Some("apk-inspection-current".to_string());

            // A replaced existing-App snapshot invalidates the review.
            session
                .existing_apps
                .get_mut("existing-app-handle")
                .unwrap()
                .snapshot_id = "backend-snapshot-2".to_string();
            assert!(review_dependencies_current(session, &record).is_err());
            session
                .existing_apps
                .get_mut("existing-app-handle")
                .unwrap()
                .snapshot_id = "backend-snapshot-1".to_string();

            // A dropped root invalidates the review.
            let root = session.roots.remove("root-handle").unwrap();
            assert!(review_dependencies_current(session, &record).is_err());
            session.roots.insert("root-handle".to_string(), root);
        }
        // A changed APK file invalidates the review.
        fs::write(&fixture.apk_path, b"replacement-apk-bytes").unwrap();
        {
            let registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session(&fixture.session_handle).unwrap();
            assert!(review_dependencies_current(session, &local_record()).is_err());
        }
        fixture.cleanup();

        // A re-analyzed remote source invalidates the review through its fingerprint.
        let (remote_state, remote_session, remote_asset) =
            trusted_remote_test_state(Some(vec![TrustedRemoteRelease {
                tag: "v1".to_string(),
                prerelease: false,
                asset_file_names: vec!["app.apk".to_string()],
            }]));
        let remote_apk = permission_test_apk("review-dependencies-remote");
        let (remote_root_path, remote_root) = temp_authored_root("review-dependencies-remote-root");
        let remote_context = remote_context_from_state(
            &remote_state,
            &remote_session,
            &remote_asset,
            "latest_compatible_release",
            None,
            false,
            None,
        )
        .unwrap();
        let remote_record = ReviewRecord {
            request_id: 7,
            backend_review_id: Some("backend-review-remote".to_string()),
            binding: ReviewBinding::Remote {
                apk_handle: "apk-handle".to_string(),
                apk_identity: remote_apk.identity.clone(),
                inspection_handle: "apk-inspection-current".to_string(),
                asset_handle: remote_asset.clone(),
                strategy: "latest_compatible_release".to_string(),
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
                source_fingerprint: remote_context.source_fingerprint.clone(),
            },
            existing_app: None,
            root_path: remote_root.root.clone(),
            blocking: false,
            app_relative_path: None,
            recipe_relative_path: None,
        };
        {
            let mut registry = lock_registry(&remote_state).unwrap();
            let session = registry.session_mut(&remote_session).unwrap();
            session
                .apks
                .insert("apk-handle".to_string(), remote_apk.clone());
            session
                .roots
                .insert("root-handle".to_string(), remote_root.clone());
            assert!(review_dependencies_current(session, &remote_record).is_ok());

            let source_handle = session
                .remote_assets
                .get(&remote_asset)
                .unwrap()
                .source_handle
                .clone();
            session
                .remote_sources
                .get_mut(&source_handle)
                .unwrap()
                .release_analysis = Some(vec![TrustedRemoteRelease {
                tag: "v3".to_string(),
                prerelease: false,
                asset_file_names: vec!["app.apk".to_string()],
            }]);
            assert!(review_dependencies_current(session, &remote_record).is_err());
        }
        let _ = fs::remove_file(&remote_apk.path);
        let _ = fs::remove_dir_all(&remote_root_path);
    }

    #[test]
    fn resolve_draft_inputs_derives_local_and_remote_permission_automation() {
        let fixture = ReviewFixture::create("resolve-permissions-local");
        // A local APK is the inspected artifact the draft installs, so the
        // selection canonicalizes against the stored inspection exactly like a
        // pinned remote artifact does.
        let baseline = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(json!({
                "inspectionHandle": "apk-inspection-current",
                "runtimePermissions": [{ "permissionName": "android.permission.CAMERA" }],
                "appOps": [],
            })),
            true,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            baseline.permission_automation,
            Some(json!({
                "packageName": "com.example.inspected",
                "runtimePermissions": [{
                    "permissionName": "android.permission.CAMERA",
                    "requiresRoot": false,
                    "androidApiMin": 23,
                    "androidApiMax": null,
                }],
                "appOps": [],
            }))
        );
        assert!(matches!(
            baseline.binding,
            Some(ReviewBinding::Local { .. })
        ));

        // Elevated selections keep the trusted root requirement and API bounds.
        let elevated = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(json!({
                "inspectionHandle": "apk-inspection-current",
                "runtimePermissions": [],
                "appOps": [{
                    "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "operationName": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                }],
            })),
            true,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            elevated.permission_automation,
            Some(json!({
                "packageName": "com.example.inspected",
                "runtimePermissions": [],
                "appOps": [{
                    "permissionName": "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "operationName": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                    "requiresRoot": true,
                    "androidApiMin": 30,
                    "androidApiMax": null,
                }],
            }))
        );

        // The empty selection means no permission automation and grants nothing;
        // the draft still resolves as a plain local binding.
        let empty = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(json!({
                "inspectionHandle": "apk-inspection-current",
                "runtimePermissions": [],
                "appOps": [],
            })),
            false,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(empty.permission_automation.is_none());
        assert!(empty.remote.is_none());
        assert!(matches!(empty.binding, Some(ReviewBinding::Local { .. })));
        fixture.cleanup();

        // A remote review derives the same canonical selection from the retained asset.
        let (remote_state, remote_session, remote_asset) = trusted_remote_test_state(None);
        let mut remote_apk = permission_test_apk("resolve-permissions-remote");
        // The download flow retains the originating asset on the inspected APK,
        // so a remote case binds the record exactly like a completed download.
        remote_apk.remote_asset_handle = Some(remote_asset.clone());
        let (remote_root_path, remote_root) = temp_authored_root("resolve-permissions-remote-root");
        {
            let mut registry = lock_registry(&remote_state).unwrap();
            let session = registry.session_mut(&remote_session).unwrap();
            session
                .apks
                .insert("apk-handle".to_string(), remote_apk.clone());
            session
                .roots
                .insert("root-handle".to_string(), remote_root.clone());
        }
        let remote_inputs = resolve_draft_inputs(
            &remote_state,
            &remote_session,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(permission_selection()),
            true,
            Some(RemoteReviewSelection {
                asset_handle: &remote_asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        let automation = remote_inputs
            .permission_automation
            .as_ref()
            .expect("eligible remote strategy derives permission automation");
        assert_eq!(automation["packageName"], "com.example.inspected");
        assert!(matches!(
            remote_inputs.binding,
            Some(ReviewBinding::Remote { .. })
        ));

        // A user-provided remote artifact is never the inspected APK, so the
        // selection is refused before generation or publication.
        let unsupported = resolve_draft_inputs(
            &remote_state,
            &remote_session,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(permission_selection()),
            true,
            Some(RemoteReviewSelection {
                asset_handle: &remote_asset,
                strategy: "user_provided_apk",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(
            api_error_code(&unsupported),
            Some("apk_permission_automation_strategy_unsupported")
        );
        let _ = fs::remove_file(&remote_apk.path);
        let _ = fs::remove_dir_all(&remote_root_path);
    }

    #[test]
    fn resolve_draft_inputs_refuses_malformed_and_untrusted_local_permission_evidence() {
        let fixture = ReviewFixture::create("resolve-permissions-untrusted");
        let cases = [
            ("malformed", json!({ "inspectionHandle": 9 }), "altered"),
            (
                "stale",
                json!({
                    "inspectionHandle": "apk-inspection-stale",
                    "runtimePermissions": [{ "permissionName": "android.permission.CAMERA" }],
                    "appOps": [],
                }),
                "stale",
            ),
            (
                "unknown",
                json!({
                    "inspectionHandle": "apk-inspection-current",
                    "runtimePermissions": [{ "permissionName": "android.permission.RECORD_AUDIO" }],
                    "appOps": [],
                }),
                "unknown",
            ),
            (
                "duplicate",
                json!({
                    "inspectionHandle": "apk-inspection-current",
                    "runtimePermissions": [
                        { "permissionName": "android.permission.CAMERA" },
                        { "permissionName": "android.permission.CAMERA" },
                    ],
                    "appOps": [],
                }),
                "duplicate",
            ),
        ];
        for (label, selection, reason) in cases {
            let refused = resolve_draft_inputs(
                &fixture.state,
                &fixture.session_handle,
                "apk-handle",
                Some("root-handle"),
                true,
                None,
                Some(selection),
                true,
                None,
            )
            .unwrap()
            .unwrap_err();
            assert_eq!(
                api_error_code(&refused),
                Some("app_generator_permission_selection_invalid"),
                "{label} selection carries the permission-selection error code"
            );
            assert_eq!(
                refused["error"]["details"]["reason"], reason,
                "{label} selection reports its refusal reason"
            );
        }
        fixture.cleanup();
    }

    #[test]
    fn permission_automation_optin_travels_on_local_preview_and_review_wires() {
        let fixture = ReviewFixture::create("optin-local-wire");
        // Zero selected identities in both directions: the request intent is the
        // flag alone, so it must survive canonicalization returning no actions.
        let disabled = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            false,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(!disabled.permission_automation_enabled);
        assert!(disabled.permission_automation.is_none());
        let enabled = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            true,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(enabled.permission_automation_enabled);
        assert!(enabled.permission_automation.is_none());

        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "generateAppRecipeDraft" => Ok(success(json!({
                "app": { "id": "generated.app" },
                "recipe": { "id": "generated.app.install" },
            }))),
            "checkGeneratedCatalogCollisions" => Ok(success(json!({ "collisions": [] }))),
            "reviewAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "reviewId": "backend-review-optin-local",
                "destinations": {
                    "app": { "fileName": "example.yaml", "relativePath": "apps/example.yaml" },
                    "recipe": {
                        "fileName": "app.example.install.yaml",
                        "relativePath": "recipes/app.example.install.yaml",
                    },
                },
                "target": { "mode": "new_app", "appPath": Value::Null },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let preview = generate_draft_via_transport(
            &transport,
            enabled,
            "generateAppRecipeDraft",
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&preview).is_some());
        let review = review_draft_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            disabled,
            5,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&review).is_some());

        let previews = transport.requests_for("generateAppRecipeDraft");
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0]["permissionAutomationEnabled"], true);
        assert!(previews[0].get("permissionAutomation").is_none());
        let reviews = transport.requests_for("reviewAppGeneratorDraft");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0]["permissionAutomationEnabled"], false);
        assert!(reviews[0].get("permissionAutomation").is_none());

        // The existing-App mode carries the same flag beside its retained snapshot.
        let mut existing = with_existing_app(fixture_review_inputs(&fixture), &fixture);
        existing.permission_automation_enabled = true;
        let response = review_draft_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            existing,
            6,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&response).is_some());
        let reviews = transport.requests_for("reviewAppGeneratorDraft");
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[1]["permissionAutomationEnabled"], true);
        assert_eq!(reviews[1]["existingAppSnapshotId"], "backend-snapshot-1");
        assert!(reviews[1].get("permissionAutomation").is_none());
        // Two reviews, one preview, and its collision check; nothing else.
        assert_eq!(transport.request_count(), 4);
        fixture.cleanup();
    }

    #[test]
    fn permission_automation_optin_travels_on_remote_preview_and_review_wires() {
        let (state, session_handle, asset) = trusted_remote_test_state(None);
        let apk = insert_retained_remote_apk(&state, &session_handle, &asset, "optin-remote-wire");
        let (root_path, root) = temp_authored_root("optin-remote-wire-root");
        {
            let mut registry = lock_registry(&state).unwrap();
            let session = registry.session_mut(&session_handle).unwrap();
            session
                .roots
                .insert("root-handle".to_string(), root.clone());
            session.existing_apps.insert(
                "existing-app-handle".to_string(),
                TrustedExistingApp {
                    root_path: root.root,
                    snapshot_id: "backend-snapshot-1".to_string(),
                    app_path: "apps/armsx1.yaml".to_string(),
                    package_id: Some("com.example.inspected".to_string()),
                },
            );
        }
        let opted_in = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            true,
            Some(RemoteReviewSelection {
                asset_handle: &asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(opted_in.permission_automation_enabled);
        assert!(opted_in.permission_automation.is_none());
        let opted_out = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle: &asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(!opted_out.permission_automation_enabled);
        assert!(opted_out.existing_app.is_some());

        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "generateRemoteAppRecipeDraft" => Ok(success(json!({
                "app": { "id": "generated.app" },
                "recipe": { "id": "generated.app.install" },
            }))),
            "checkGeneratedCatalogCollisions" => Ok(success(json!({ "collisions": [] }))),
            "reviewRemoteAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "reviewId": "backend-review-optin-remote",
                "destinations": {
                    "app": { "fileName": "example.yaml", "relativePath": "apps/example.yaml" },
                    "recipe": {
                        "fileName": "app.example.install.yaml",
                        "relativePath": "recipes/app.example.install.yaml",
                    },
                },
                "target": { "mode": "new_app", "appPath": Value::Null },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let preview = generate_draft_via_transport(
            &transport,
            opted_in,
            "generateRemoteAppRecipeDraft",
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&preview).is_some());
        let review = review_draft_via_transport(
            &transport,
            &state,
            &session_handle,
            opted_out,
            9,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&review).is_some());

        let previews = transport.requests_for("generateRemoteAppRecipeDraft");
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0]["permissionAutomationEnabled"], true);
        assert!(previews[0].get("permissionAutomation").is_none());
        // Only the durable asset address travels, and the flag rides beside it.
        assert_eq!(
            previews[0]["source"]["downloadUrl"],
            "https://github.com/example/project/releases/download/v2/app.apk"
        );
        let reviews = transport.requests_for("reviewRemoteAppGeneratorDraft");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0]["permissionAutomationEnabled"], false);
        assert!(reviews[0].get("permissionAutomation").is_none());
        assert_eq!(reviews[0]["existingAppSnapshotId"], "backend-snapshot-1");

        let _ = fs::remove_file(&apk.path);
        let _ = fs::remove_dir_all(&root_path);
    }

    #[test]
    fn disabled_permission_automation_refuses_a_nonempty_selection() {
        let fixture = ReviewFixture::create("optin-disabled-selection");
        let refused = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(permission_selection()),
            false,
            None,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(
            api_error_code(&refused),
            Some("app_generator_permission_selection_invalid")
        );
        assert_eq!(refused["error"]["details"]["reason"], "disabled");

        // The identical selection with the opt-in enabled still canonicalizes, so
        // the refusal is the disabled flag and not the identities themselves.
        let accepted = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(permission_selection()),
            true,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(accepted.permission_automation_enabled);
        assert!(accepted.permission_automation.is_some());
        fixture.cleanup();
    }

    #[test]
    fn permission_automation_optin_without_identities_needs_no_inspected_install_source() {
        let (state, session_handle, asset) = trusted_remote_test_state(None);
        let apk =
            insert_retained_remote_apk(&state, &session_handle, &asset, "optin-user-provided");
        let (root_path, root) = temp_authored_root("optin-user-provided-root");
        {
            let mut registry = lock_registry(&state).unwrap();
            let session = registry.session_mut(&session_handle).unwrap();
            session.roots.insert("root-handle".to_string(), root);
        }
        let remote = |strategy| RemoteReviewSelection {
            asset_handle: &asset,
            strategy,
            asset_pattern: None,
            include_prereleases: false,
            trusted_sha256: None,
        };
        // A user-provided remote artifact never installs the inspected bytes, so
        // newly inspected actions stay refused for that source.
        let refused = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            Some(permission_selection()),
            true,
            Some(remote("user_provided_apk")),
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(
            api_error_code(&refused),
            Some("apk_permission_automation_strategy_unsupported")
        );
        // An opt-in that selected no identities only lets the backend reference the
        // App's already declared sets, so it needs no inspected install source.
        let enabled = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            true,
            Some(remote("user_provided_apk")),
        )
        .unwrap()
        .unwrap();
        assert!(enabled.permission_automation_enabled);
        assert!(enabled.permission_automation.is_none());
        assert!(matches!(
            enabled.binding,
            Some(ReviewBinding::Remote { .. })
        ));
        let _ = fs::remove_file(&apk.path);
        let _ = fs::remove_dir_all(&root_path);
    }

    #[test]
    fn existing_app_binding_checks_root_and_package_identity() {
        let fixture = ReviewFixture::create("existing-app-binding");
        let (second_root_path, second_root) = temp_authored_root("existing-app-second-root");
        {
            let mut registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session_mut(&fixture.session_handle).unwrap();
            session.existing_apps.insert(
                "existing-app-handle".to_string(),
                TrustedExistingApp {
                    root_path: fixture.canonical_root.clone(),
                    snapshot_id: "backend-snapshot-1".to_string(),
                    app_path: "apps/armsx1.yaml".to_string(),
                    package_id: Some("com.example.inspected".to_string()),
                },
            );
            session
                .roots
                .insert("root-handle-2".to_string(), second_root);
        }
        let matched = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            matched.existing_app.unwrap().selection.snapshot_id,
            "backend-snapshot-1"
        );

        // A selection captured under another root is refused, never re-bound.
        let mismatch = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle-2"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            None,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(api_error_code(&mismatch), Some("existing_app_outside_root"));

        // A package that differs from the inspected APK can never install.
        {
            let mut registry = lock_registry(&fixture.state).unwrap();
            let session = registry.session_mut(&fixture.session_handle).unwrap();
            session
                .existing_apps
                .get_mut("existing-app-handle")
                .unwrap()
                .package_id = Some("com.example.other".to_string());
        }
        let package = resolve_draft_inputs(
            &fixture.state,
            &fixture.session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            None,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(
            api_error_code(&package),
            Some("existing_app_package_mismatch")
        );
        assert_eq!(
            package["error"]["details"]["inspectedPackageId"],
            "com.example.inspected"
        );
        assert_eq!(
            package["error"]["details"]["selectedPackageId"],
            "com.example.other"
        );
        let _ = fs::remove_dir_all(second_root_path);
        fixture.cleanup();
    }

    #[test]
    fn drafts_reject_combined_app_edits_and_existing_app_selections() {
        let fixture = ReviewFixture::create("combined-inputs");
        let transport = ScriptedTransport::new(|operation, _| {
            panic!("an invalid combination must not reach the backend: {operation}")
        });
        let preview = generate_draft_via_transport(
            &transport,
            with_existing_app(fixture_review_inputs(&fixture), &fixture),
            "generateAppRecipeDraft",
            None,
            None,
            Some(json!({ "id": "frontend-app" })),
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(api_error_code(&preview), Some("invalid_argument"));
        let review = review_draft_via_transport(
            &transport,
            &fixture.state,
            &fixture.session_handle,
            with_existing_app(fixture_review_inputs(&fixture), &fixture),
            11,
            None,
            None,
            None,
            Some(json!({ "id": "frontend-app" })),
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(api_error_code(&review), Some("invalid_argument"));
        assert_eq!(transport.request_count(), 0);
        fixture.cleanup();
    }

    #[test]
    fn catalog_paths_and_timestamps_stay_ui_safe() {
        let root = Path::new("/trusted/authored");
        assert!(is_catalog_relative_path("apps/example.yaml"));
        assert!(!is_catalog_relative_path(""));
        assert!(!is_catalog_relative_path("/etc/passwd"));
        assert!(!is_catalog_relative_path("../escape.yaml"));
        assert!(!is_catalog_relative_path("apps/../escape.yaml"));
        assert!(!is_catalog_relative_path(r"apps\example.yaml"));
        assert_eq!(
            catalog_relative_from_raw(root, "/trusted/authored/apps/example.yaml").as_deref(),
            Some("apps/example.yaml")
        );
        assert_eq!(
            catalog_relative_from_raw(root, "apps/example.yaml").as_deref(),
            Some("apps/example.yaml")
        );
        assert_eq!(
            catalog_relative_from_raw(root, "/elsewhere/apps/example.yaml"),
            None
        );
        assert_eq!(
            catalog_relative_file_list(
                Some(&json!([
                    "/trusted/authored/apps/example.yaml",
                    "/elsewhere/secret.yaml",
                    "recipes/app.example.install.yaml",
                ])),
                root,
            ),
            json!(["apps/example.yaml", "recipes/app.example.install.yaml"])
        );
        assert_eq!(catalog_relative_file_list(None, root), json!([]));
        assert_eq!(iso8601_from_epoch_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso8601_from_epoch_seconds(1_700_000_000),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(iso8601_now().len(), 20);
    }

    /// Insert one additional provider asset retained by the fixture's source.
    fn insert_remote_asset(
        state: &AppGeneratorState,
        session_handle: &str,
        source_handle: &str,
        download_url: &str,
    ) -> String {
        let mut registry = lock_registry(state).unwrap();
        let handle = registry.allocate("remote-asset");
        let session = registry.session_mut(session_handle).unwrap();
        session.remote_assets.insert(
            handle.clone(),
            TrustedRemoteAsset {
                source_handle: source_handle.to_string(),
                download_url: download_url.to_string(),
                file_name: "app.apk".to_string(),
                size: 11,
                release_tag: None,
            },
        );
        handle
    }

    /// Retain one inspected APK exactly like a completed remote download does.
    fn insert_retained_remote_apk(
        state: &AppGeneratorState,
        session_handle: &str,
        asset_handle: &str,
        label: &str,
    ) -> TrustedApk {
        let mut apk = permission_test_apk(label);
        apk.remote_asset_handle = Some(asset_handle.to_string());
        let mut registry = lock_registry(state).unwrap();
        let session = registry.session_mut(session_handle).unwrap();
        session.apks.insert("apk-handle".to_string(), apk.clone());
        apk
    }

    /// Run one review after a redirected download and report what was sent.
    ///
    /// The download completed at `final_url`, which differs from the retained
    /// durable address. Returns the review request payload the native boundary
    /// sent and the durable session facts serialized as JSON, so tests can
    /// prove what was persisted and what only passed through.
    fn review_after_redirected_download(
        state: &AppGeneratorState,
        session_handle: &str,
        asset_handle: &str,
        final_url: &str,
        request_id: i64,
    ) -> (Value, String) {
        let (asset, apk_path) = {
            let mut registry = lock_registry(state).unwrap();
            let session = registry.session_mut(session_handle).unwrap();
            let asset = session.remote_assets.get(asset_handle).unwrap().clone();
            if session.temp_directory.is_none() {
                session.temp_directory = tempfile::Builder::new()
                    .prefix("emuchef-app-generator-")
                    .tempdir()
                    .ok();
            }
            let path = session
                .temp_directory
                .as_ref()
                .expect("temporary download workspace exists")
                .path()
                .join("selected.apk");
            (asset, path)
        };
        fs::write(&apk_path, b"trusted-remote-apk").unwrap();
        let (apk_path, identity) = validate_apk(&apk_path).unwrap();
        let retained = retain_downloaded_remote_apk(
            state,
            session_handle,
            asset_handle,
            &asset,
            DownloadedRemoteApk {
                path: apk_path,
                identity,
                file_name: asset.file_name.clone(),
                final_url: Url::parse(final_url).unwrap(),
            },
        )
        .unwrap();
        let retained = success_result(&retained).unwrap();
        // The labelled download result may name the address it actually fetched;
        // nothing below persists it.
        assert_eq!(retained["source"]["downloadUrl"], final_url);
        let apk_handle = retained["apkHandle"].as_str().unwrap().to_string();
        // Store the trusted inspection exactly like the inspection command does.
        let inspection = json!({
            "manifest": {
                "packageName": "com.example.inspected",
                "versionCode": null,
                "versionName": null,
                "minSdkVersion": null,
                "targetSdkVersion": null,
            },
            "permissions": [],
            "runtimeGrantCandidates": [],
            "appOpCandidates": [],
            "warnings": [],
            "calculatedSha256": "A".repeat(64),
            "checksumStatus": "not_compared",
            "signatureVerification": "not_performed",
        });
        let facts = facts_from_native_inspection(&inspection).unwrap();
        {
            let mut registry = lock_registry(state).unwrap();
            let session = registry.session_mut(session_handle).unwrap();
            let apk = session.apks.get_mut(&apk_handle).unwrap();
            apk.inspection = Some(inspection);
            apk.facts = Some(facts);
            apk.inspection_handle = Some("apk-inspection-current".to_string());
        }
        let inputs = resolve_draft_inputs(
            state,
            session_handle,
            &apk_handle,
            Some("root-handle"),
            true,
            None,
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        let transport = ScriptedTransport::new(|operation, payload| match operation {
            "reviewRemoteAppGeneratorDraft" => Ok(success(json!({
                "requestId": payload["requestId"].clone(),
                "blocking": false,
                "reviewId": "backend-review-durable",
                "destinations": {
                    "app": { "fileName": "example.yaml", "relativePath": "apps/example.yaml" },
                    "recipe": {
                        "fileName": "app.example.install.yaml",
                        "relativePath": "recipes/app.example.install.yaml",
                    },
                },
                "target": { "mode": "new_app", "appPath": Value::Null },
            }))),
            other => panic!("unexpected operation {other}"),
        });
        let response = review_draft_via_transport(
            &transport,
            state,
            session_handle,
            inputs,
            request_id,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        assert!(success_result(&response).is_some());
        let requests = transport.requests_for("reviewRemoteAppGeneratorDraft");
        assert_eq!(requests.len(), 1);
        let durable = {
            let registry = lock_registry(state).unwrap();
            let session = registry.session(session_handle).unwrap();
            json!({
                "sources": session
                    .remote_sources
                    .values()
                    .map(|source| json!({
                        "mode": source.mode,
                        "directUrl": source.direct_url,
                        "repository": source.repository,
                    }))
                    .collect::<Vec<_>>(),
                "assets": session
                    .remote_assets
                    .values()
                    .map(|asset| json!({ "downloadUrl": asset.download_url }))
                    .collect::<Vec<_>>(),
                "apks": session
                    .apks
                    .values()
                    .map(|apk| json!({ "remoteAssetHandle": apk.remote_asset_handle }))
                    .collect::<Vec<_>>(),
            })
            .to_string()
        };
        (requests[0].clone(), durable)
    }

    #[test]
    fn remote_review_refuses_an_apk_retained_for_a_different_asset() {
        let (state, session_handle, retained_asset) = trusted_remote_test_state(None);
        let source_handle = {
            let registry = lock_registry(&state).unwrap();
            let session = registry.session(&session_handle).unwrap();
            session
                .remote_assets
                .get(&retained_asset)
                .unwrap()
                .source_handle
                .clone()
        };
        // A second, independently valid asset of the same source. Both handles
        // resolve on their own, so only the retained download relation proves
        // which bytes were inspected.
        let other_asset = insert_remote_asset(
            &state,
            &session_handle,
            &source_handle,
            "https://github.com/example/project/releases/download/v3/app.apk",
        );
        let apk = insert_retained_remote_apk(
            &state,
            &session_handle,
            &retained_asset,
            "swapped-asset-new-app",
        );
        let (root_path, root) = temp_authored_root("swapped-asset-new-app-root");
        {
            let mut registry = lock_registry(&state).unwrap();
            let session = registry.session_mut(&session_handle).unwrap();
            session.roots.insert("root-handle".to_string(), root);
        }
        let swapped = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle: &other_asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(api_error_code(&swapped), Some("remote_asset_mismatch"));

        // The retained pair still resolves: only the replacement relation fails.
        let accepted = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            None,
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle: &retained_asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            accepted.binding,
            Some(ReviewBinding::Remote { .. })
        ));

        let _ = fs::remove_file(&apk.path);
        let _ = fs::remove_dir_all(&root_path);
    }

    #[test]
    fn remote_review_refuses_a_swapped_asset_with_an_existing_app_selection() {
        let (state, session_handle, retained_asset) = trusted_remote_test_state(None);
        let source_handle = {
            let registry = lock_registry(&state).unwrap();
            let session = registry.session(&session_handle).unwrap();
            session
                .remote_assets
                .get(&retained_asset)
                .unwrap()
                .source_handle
                .clone()
        };
        let other_asset = insert_remote_asset(
            &state,
            &session_handle,
            &source_handle,
            "https://github.com/example/project/releases/download/v3/app.apk",
        );
        let apk = insert_retained_remote_apk(
            &state,
            &session_handle,
            &retained_asset,
            "swapped-asset-existing-app",
        );
        let (root_path, root) = temp_authored_root("swapped-asset-existing-app-root");
        {
            let mut registry = lock_registry(&state).unwrap();
            let session = registry.session_mut(&session_handle).unwrap();
            session
                .roots
                .insert("root-handle".to_string(), root.clone());
            session.existing_apps.insert(
                "existing-app-handle".to_string(),
                TrustedExistingApp {
                    root_path: root.root,
                    snapshot_id: "backend-snapshot-1".to_string(),
                    app_path: "apps/armsx1.yaml".to_string(),
                    package_id: Some("com.example.inspected".to_string()),
                },
            );
        }
        let swapped = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle: &other_asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(api_error_code(&swapped), Some("remote_asset_mismatch"));

        // The reviewed pair plus the existing-App selection still resolve.
        let accepted = resolve_draft_inputs(
            &state,
            &session_handle,
            "apk-handle",
            Some("root-handle"),
            true,
            Some("existing-app-handle"),
            None,
            false,
            Some(RemoteReviewSelection {
                asset_handle: &retained_asset,
                strategy: "pinned_remote_asset",
                asset_pattern: None,
                include_prereleases: false,
                trusted_sha256: None,
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            accepted
                .existing_app
                .as_ref()
                .expect("existing App binding resolves")
                .selection
                .snapshot_id,
            "backend-snapshot-1"
        );
        assert!(matches!(
            accepted.binding,
            Some(ReviewBinding::Remote { .. })
        ));

        let _ = fs::remove_file(&apk.path);
        let _ = fs::remove_dir_all(&root_path);
    }

    #[test]
    fn durable_original_address_survives_a_signed_redirect_in_the_review_payload() {
        // Scenario 1: a direct APK address whose durable query string must reach
        // the backend while the signed address the CDN served stays transient.
        let original = "https://mirror.example.com/releases/app.apk?channel=stable";
        let normalized = normalize_remote_source("direct_apk", original).unwrap();
        assert_eq!(normalized.url.as_str(), original);
        let state = AppGeneratorState::default();
        let (root_path, root) = temp_authored_root("durable-address-direct");
        let (session_handle, asset_handle) = {
            let mut registry = lock_registry(&state).unwrap();
            let session_handle = registry.begin();
            let source_handle = registry.allocate("remote-source");
            let asset_handle = registry.allocate("remote-asset");
            let session = registry.session_mut(&session_handle).unwrap();
            session.roots.insert("root-handle".to_string(), root);
            session.remote_sources.insert(
                source_handle.clone(),
                TrustedRemoteSource {
                    mode: "direct_apk".to_string(),
                    provider: None,
                    base_url: None,
                    repository: None,
                    release_tag: None,
                    direct_url: Some(original.to_string()),
                    release_analysis: None,
                },
            );
            session.remote_assets.insert(
                asset_handle.clone(),
                TrustedRemoteAsset {
                    source_handle,
                    download_url: original.to_string(),
                    file_name: "app.apk".to_string(),
                    size: 17,
                    release_tag: None,
                },
            );
            (session_handle, asset_handle)
        };
        let (payload, durable) = review_after_redirected_download(
            &state,
            &session_handle,
            &asset_handle,
            "https://cdn.example.net/signed/app.apk?token=transient-abc",
            31,
        );
        assert_eq!(payload["source"]["downloadUrl"], original);
        assert!(!payload.to_string().contains("cdn.example.net"));
        assert!(durable.contains(original));
        assert!(!durable.contains("cdn.example.net"));
        assert!(!durable.contains("transient-abc"));
        let _ = fs::remove_dir_all(&root_path);

        // Scenario 2: a provider release asset address stays the durable source.
        let (state, session_handle, asset_handle) = trusted_remote_test_state(None);
        let (provider_root_path, provider_root) = temp_authored_root("durable-address-provider");
        {
            let mut registry = lock_registry(&state).unwrap();
            let session = registry.session_mut(&session_handle).unwrap();
            session
                .roots
                .insert("root-handle".to_string(), provider_root);
        }
        let (payload, durable) = review_after_redirected_download(
            &state,
            &session_handle,
            &asset_handle,
            "https://objects.githubusercontent.com/signed/app.apk?token=transient-def",
            32,
        );
        assert_eq!(
            payload["source"]["downloadUrl"],
            "https://github.com/example/project/releases/download/v2/app.apk"
        );
        assert!(!payload
            .to_string()
            .contains("objects.githubusercontent.com"));
        assert!(durable.contains("https://github.com/example/project/releases/download/v2/app.apk"));
        assert!(!durable.contains("objects.githubusercontent.com"));
        let _ = fs::remove_dir_all(&provider_root_path);
    }
}

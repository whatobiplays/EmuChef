//! Trusted local-APK generator boundary for the Config Editor.
//!
//! Native APK and authored-root paths remain in process memory behind
//! session-scoped handles. Only safe inspection metadata, labels, authored drafts,
//! diagnostics, canonical previews, and relative destination metadata cross into React.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppGeneratorPreferences {
    authored_root: Option<PathBuf>,
}

#[derive(Default)]
struct GeneratorRegistry {
    next_handle: u64,
    sessions: HashMap<String, GeneratorSession>,
}

#[derive(Default)]
struct GeneratorSession {
    apks: HashMap<String, TrustedApk>,
    roots: HashMap<String, TrustedRoot>,
    remote_sources: HashMap<String, TrustedRemoteSource>,
    remote_assets: HashMap<String, TrustedRemoteAsset>,
    temp_directory: Option<TempDir>,
}

struct TrustedApk {
    path: PathBuf,
    identity: FileIdentity,
    inspection: Option<Value>,
    inspection_handle: Option<String>,
    facts: Option<Value>,
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
}

#[derive(Clone)]
struct TrustedRemoteAsset {
    source_handle: String,
    download_url: String,
    file_name: String,
    size: u64,
    release_tag: Option<String>,
}

#[derive(Clone)]
struct TrustedRoot {
    root: PathBuf,
    apps_directory: PathBuf,
    recipes_directory: PathBuf,
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

/// Process-memory state for local-APK generator sessions.
#[derive(Default)]
pub struct AppGeneratorState {
    registry: Mutex<GeneratorRegistry>,
}

impl AppGeneratorState {
    /// Invalidate all trusted handles, including after a sidecar restart.
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
        for release in analyzed.releases {
            let mut release_assets = Vec::new();
            for asset in release.assets {
                let asset_handle = registry.allocate("remote-asset");
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
            if !release_assets.is_empty() {
                releases.push(json!({
                    "tag": release.tag,
                    "name": release.name,
                    "prerelease": release.prerelease,
                    "publishedAt": release.published_at,
                    "assets": release_assets,
                }));
            }
        }
        let preselected = (flat_assets.len() == 1)
            .then(|| flat_assets[0].get("assetHandle").cloned())
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
        let client = remote_http_client()?;
        let response = client
            .get(&download_asset.download_url)
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

    let mut registry = lock_registry(&state)?;
    let apk_handle = registry.allocate("apk");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.apks.insert(
        apk_handle.clone(),
        TrustedApk {
            path,
            identity,
            inspection: None,
            inspection_handle: None,
            facts: None,
        },
    );
    let source = session.remote_sources.get(&asset.source_handle).cloned();
    Ok(success(json!({
        "apkHandle": apk_handle,
        "label": file_name,
        "source": {
            "mode": source.as_ref().map(|value| value.mode.as_str()).unwrap_or("direct_apk"),
            "strategy": "pinned_remote_asset",
            "downloadUrl": final_url,
            "provider": source.as_ref().and_then(|value| value.provider.clone()),
            "baseUrl": source.as_ref().and_then(|value| value.base_url.clone()),
            "repository": source.as_ref().and_then(|value| value.repository.clone()),
            "releaseTag": asset.release_tag,
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

fn normalize_remote_source(mode: &str, input: &str) -> Result<NormalizedRemoteSource, Value> {
    let mut url = Url::parse(input.trim())
        .map_err(|_| remote_source_error("remote_url_invalid", "Enter a valid HTTPS address."))?;
    if !safe_remote_url(&url) || url.fragment().is_some() || url.query().is_some() {
        return Err(remote_source_error(
            "remote_url_invalid",
            "Enter a valid public HTTPS address without credentials, query parameters, or a fragment.",
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
    if let Ok(address) = host.parse::<IpAddr>() {
        return !(address.is_loopback() || address.is_unspecified() || address.is_multicast());
    }
    true
}

fn remote_http_client() -> Result<Client, Value> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(NETWORK_TIMEOUT)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.stop();
            }
            let next = attempt.url();
            if next.scheme() != "https" || !safe_remote_url(next) {
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
    include_prereleases: bool,
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
        if mode == "github_repository" && prerelease && !include_prereleases {
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
    regenerate_identifiers: bool,
    root_handle: Option<String>,
) -> Result<Value, String> {
    let facts = match stored_facts(&state, &session_handle, &apk_handle)? {
        Ok(facts) => facts,
        Err(error) => return Ok(error),
    };
    let permission_automation = match stored_permission_automation(
        &state,
        &session_handle,
        &apk_handle,
        permission_selection,
        false,
    )? {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let mut payload = Map::new();
    payload.insert("facts".to_string(), facts);
    payload.insert(
        "regenerateIdentifiers".to_string(),
        Value::Bool(regenerate_identifiers),
    );
    if let Some(app) = app {
        payload.insert("app".to_string(), app);
    }
    if let Some(recipe) = recipe {
        payload.insert("recipe".to_string(), recipe);
    }
    if let Some(mappings) = mappings {
        payload.insert("mappings".to_string(), mappings);
    }
    if let Some(permission_automation) = permission_automation {
        payload.insert("permissionAutomation".to_string(), permission_automation);
    }
    let root = match root_handle {
        Some(root_handle) => match stored_root(&state, &session_handle, &root_handle)? {
            Ok(root) => Some(root),
            Err(error) => return Ok(error),
        },
        None => None,
    };
    request_generated_draft(
        &sidecar,
        "generateAppRecipeDraft",
        Value::Object(payload),
        root.as_ref(),
    )
}

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
    regenerate_identifiers: bool,
    root_handle: Option<String>,
) -> Result<Value, String> {
    let facts = match stored_facts(&state, &session_handle, &apk_handle)? {
        Ok(facts) => facts,
        Err(error) => return Ok(error),
    };
    let source = match trusted_remote_source_payload(
        &state,
        &session_handle,
        &asset_handle,
        &strategy,
        asset_pattern.as_deref(),
        include_prereleases,
        trusted_sha256.as_deref(),
    )? {
        Ok(source) => source,
        Err(error) => return Ok(error),
    };
    let eligible_strategy =
        source
            .get("strategy")
            .and_then(Value::as_str)
            .is_some_and(|strategy| {
                matches!(
                    strategy,
                    "pinned_remote_asset" | "latest_compatible_release"
                )
            });
    let permission_automation = match stored_permission_automation(
        &state,
        &session_handle,
        &apk_handle,
        permission_selection,
        eligible_strategy,
    )? {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let mut payload = Map::new();
    payload.insert("facts".to_string(), facts);
    payload.insert("source".to_string(), source);
    payload.insert(
        "regenerateIdentifiers".to_string(),
        Value::Bool(regenerate_identifiers),
    );
    if let Some(app) = app {
        payload.insert("app".to_string(), app);
    }
    if let Some(recipe) = recipe {
        payload.insert("recipe".to_string(), recipe);
    }
    if let Some(mappings) = mappings {
        payload.insert("mappings".to_string(), mappings);
    }
    if let Some(permission_automation) = permission_automation {
        payload.insert("permissionAutomation".to_string(), permission_automation);
    }
    let root = match root_handle {
        Some(root_handle) => match stored_root(&state, &session_handle, &root_handle)? {
            Ok(root) => Some(root),
            Err(error) => return Ok(error),
        },
        None => None,
    };
    request_generated_draft(
        &sidecar,
        "generateRemoteAppRecipeDraft",
        Value::Object(payload),
        root.as_ref(),
    )
}

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
    let (facts, apk_path, identity, root) = {
        let registry = lock_registry(&state)?;
        let session = match registry.session(&session_handle) {
            Ok(session) => session,
            Err(error) => return Ok(error),
        };
        let Some(apk) = session.apks.get(&apk_handle) else {
            return Ok(invalid_handle_error());
        };
        let Some(facts) = apk.facts.clone() else {
            return Ok(inspection_required_error());
        };
        let Some(root) = session.roots.get(&root_handle) else {
            return Ok(invalid_handle_error());
        };
        (facts, apk.path.clone(), apk.identity.clone(), root.clone())
    };
    if current_file_identity(&apk_path).as_ref() != Some(&identity) {
        return Ok(apk_changed_error());
    }
    let source = match trusted_remote_source_payload(
        &state,
        &session_handle,
        &asset_handle,
        &strategy,
        asset_pattern.as_deref(),
        include_prereleases,
        trusted_sha256.as_deref(),
    )? {
        Ok(source) => source,
        Err(error) => return Ok(error),
    };
    let eligible_strategy =
        source
            .get("strategy")
            .and_then(Value::as_str)
            .is_some_and(|strategy| {
                matches!(
                    strategy,
                    "pinned_remote_asset" | "latest_compatible_release"
                )
            });
    let permission_automation = match stored_permission_automation(
        &state,
        &session_handle,
        &apk_handle,
        permission_selection,
        eligible_strategy,
    )? {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };
    let mut validation_payload = json!({
        "facts": facts,
        "source": source,
        "app": app,
        "recipe": recipe,
        "mappings": mappings,
        "regenerateIdentifiers": false,
    });
    if let Some(permission_automation) = permission_automation {
        validation_payload
            .as_object_mut()
            .expect("validation payload is an object")
            .insert("permissionAutomation".to_string(), permission_automation);
    }
    let validation = request_sidecar(
        &sidecar,
        "generateRemoteAppRecipeDraft",
        Some(validation_payload),
    )?;
    let Some(draft) = success_result(&validation) else {
        return Ok(validation);
    };
    if draft.get("blocking").and_then(Value::as_bool) != Some(false) {
        return Ok(api_error(
            "app_recipe_invalid",
            "The app and recipe must pass validation before they can be saved.",
            json!({ "diagnostics": draft.get("diagnostics").cloned().unwrap_or_else(|| json!([])) }),
        ));
    }
    let Some(app_yaml) = draft.get("appCanonicalYaml").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    let Some(recipe_yaml) = draft.get("recipeCanonicalYaml").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    let Some(app_file) = destination_file(draft, "appDestination") else {
        return Ok(invalid_destination_error());
    };
    let Some(recipe_file) = destination_file(draft, "recipeDestination") else {
        return Ok(invalid_destination_error());
    };
    let collision_payload = match collision_payload_from_generated_draft(&root, draft) {
        Ok(payload) => payload,
        Err(_) => return Ok(generator_protocol_error()),
    };
    let collisions = request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(collision_payload),
    )?;
    let Some(collision_result) = success_result(&collisions) else {
        return Ok(collisions);
    };
    if collision_result.get("blocking").and_then(Value::as_bool) != Some(false) {
        return Ok(api_error(
            "app_recipe_collision_blocking",
            "Blocking app or recipe collisions must be resolved before saving.",
            json!({ "collisions": collision_result.get("collisions").cloned().unwrap_or_else(|| json!([])) }),
        ));
    }
    let (_, recipe_path) = match publish_pair_create_new(
        &root,
        &app_file,
        app_yaml.as_bytes(),
        &recipe_file,
        recipe_yaml.as_bytes(),
    ) {
        Ok(paths) => paths,
        Err(error) => return Ok(error),
    };
    let opened = request_sidecar(
        &sidecar,
        "openRecipe",
        Some(json!({ "path": recipe_path, "authoredRoot": root.root })),
    )?;
    let Some(opened_result) = success_result(&opened).cloned() else {
        return Ok(api_error(
            "app_recipe_saved_open_failed",
            "The app and recipe were saved, but the generated recipe could not be opened.",
            json!({ "appRelativePath": format!("apps/{app_file}"), "recipeRelativePath": format!("recipes/{recipe_file}") }),
        ));
    };
    let mut registry = lock_registry(&state)?;
    registry.sessions.remove(&session_handle);
    Ok(success(json!({
        "appFileName": app_file,
        "recipeFileName": recipe_file,
        "appRelativePath": format!("apps/{app_file}"),
        "recipeRelativePath": format!("recipes/{recipe_file}"),
        "openedRecipe": opened_result,
    })))
}

fn trusted_remote_source_payload(
    state: &AppGeneratorState,
    session_handle: &str,
    asset_handle: &str,
    strategy: &str,
    asset_pattern: Option<&str>,
    include_prereleases: bool,
    trusted_sha256: Option<&str>,
) -> Result<Result<Value, Value>, String> {
    if !matches!(
        strategy,
        "pinned_remote_asset" | "latest_compatible_release" | "user_provided_apk"
    ) {
        return Ok(Err(remote_source_error(
            "remote_strategy_invalid",
            "Choose a supported installation method.",
        )));
    }
    let trusted_sha256 = match trusted_sha256_request_value(strategy, trusted_sha256) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(Err(error)),
    };
    let Some(asset) = session.remote_assets.get(asset_handle) else {
        return Ok(Err(invalid_handle_error()));
    };
    let Some(source) = session.remote_sources.get(&asset.source_handle) else {
        return Ok(Err(invalid_handle_error()));
    };
    if strategy == "latest_compatible_release" {
        if source.mode != "github_repository" || source.repository.is_none() {
            return Ok(Err(remote_source_error(
                "latest_release_source_unsupported",
                "Latest compatible release currently requires a GitHub repository source.",
            )));
        }
        let Some(_pattern) = asset_pattern.filter(|value| !value.trim().is_empty()) else {
            return Ok(Err(remote_source_error(
                "latest_release_asset_pattern_invalid",
                "Enter an APK filename pattern for latest-release resolution.",
            )));
        };
    }
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
    Ok(Ok(payload))
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
    let (facts, apk_path, identity, root) = {
        let registry = lock_registry(&state)?;
        let session = match registry.session(&session_handle) {
            Ok(session) => session,
            Err(error) => return Ok(error),
        };
        let Some(apk) = session.apks.get(&apk_handle) else {
            return Ok(invalid_handle_error());
        };
        let Some(facts) = apk.facts.clone() else {
            return Ok(inspection_required_error());
        };
        let Some(root) = session.roots.get(&root_handle) else {
            return Ok(invalid_handle_error());
        };
        (facts, apk.path.clone(), apk.identity.clone(), root.clone())
    };
    if current_file_identity(&apk_path).as_ref() != Some(&identity) {
        return Ok(apk_changed_error());
    }

    let permission_automation = match stored_permission_automation(
        &state,
        &session_handle,
        &apk_handle,
        permission_selection,
        false,
    )? {
        Ok(value) => value,
        Err(error) => return Ok(error),
    };

    let mut validation_payload = json!({
        "facts": facts,
        "app": app,
        "recipe": recipe,
        "mappings": mappings,
        "regenerateIdentifiers": false,
    });
    if let Some(permission_automation) = permission_automation {
        validation_payload
            .as_object_mut()
            .expect("validation payload is an object")
            .insert("permissionAutomation".to_string(), permission_automation);
    }
    let validation = request_sidecar(&sidecar, "generateAppRecipeDraft", Some(validation_payload))?;
    let Some(draft) = success_result(&validation) else {
        return Ok(validation);
    };
    if draft.get("blocking").and_then(Value::as_bool) != Some(false) {
        return Ok(api_error(
            "app_recipe_invalid",
            "The app and recipe must pass validation before they can be saved.",
            json!({ "diagnostics": draft.get("diagnostics").cloned().unwrap_or_else(|| json!([])) }),
        ));
    }
    let Some(app_yaml) = draft.get("appCanonicalYaml").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    let Some(recipe_yaml) = draft.get("recipeCanonicalYaml").and_then(Value::as_str) else {
        return Ok(generator_protocol_error());
    };
    let Some(app_file) = destination_file(draft, "appDestination") else {
        return Ok(invalid_destination_error());
    };
    let Some(recipe_file) = destination_file(draft, "recipeDestination") else {
        return Ok(invalid_destination_error());
    };
    let collision_payload = match collision_payload_from_generated_draft(&root, draft) {
        Ok(payload) => payload,
        Err(_) => return Ok(generator_protocol_error()),
    };
    let collisions = request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(collision_payload),
    )?;
    let Some(collision_result) = success_result(&collisions) else {
        return Ok(collisions);
    };
    if collision_result.get("blocking").and_then(Value::as_bool) != Some(false) {
        return Ok(api_error(
            "app_recipe_collision_blocking",
            "Blocking app or recipe collisions must be resolved before saving.",
            json!({ "collisions": collision_result.get("collisions").cloned().unwrap_or_else(|| json!([])) }),
        ));
    }

    let (app_path, recipe_path) = match publish_pair_create_new(
        &root,
        &app_file,
        app_yaml.as_bytes(),
        &recipe_file,
        recipe_yaml.as_bytes(),
    ) {
        Ok(paths) => paths,
        Err(error) => return Ok(error),
    };
    let opened = request_sidecar(
        &sidecar,
        "openRecipe",
        Some(json!({ "path": recipe_path, "authoredRoot": root.root })),
    )?;
    let Some(opened_result) = success_result(&opened).cloned() else {
        return Ok(api_error(
            "app_recipe_saved_open_failed",
            "The app and recipe were saved, but the generated recipe could not be opened.",
            json!({ "appRelativePath": format!("apps/{app_file}"), "recipeRelativePath": format!("recipes/{recipe_file}") }),
        ));
    };
    let mut registry = lock_registry(&state)?;
    registry.sessions.remove(&session_handle);
    drop(app_path);
    Ok(success(json!({
        "appFileName": app_file,
        "recipeFileName": recipe_file,
        "appRelativePath": format!("apps/{app_file}"),
        "recipeRelativePath": format!("recipes/{recipe_file}"),
        "openedRecipe": opened_result,
    })))
}

#[tauri::command]
pub fn cancel_app_generator(
    state: State<'_, AppGeneratorState>,
    session_handle: String,
) -> Result<Value, String> {
    let mut registry = lock_registry(&state)?;
    if registry.sessions.remove(&session_handle).is_none() {
        return Ok(expired_session_error());
    }
    Ok(success(json!({})))
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
    Ok(TrustedRoot {
        root,
        apps_directory,
        recipes_directory,
    })
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

fn publish_pair_create_new(
    root: &TrustedRoot,
    app_file: &str,
    app_bytes: &[u8],
    recipe_file: &str,
    recipe_bytes: &[u8],
) -> Result<(PathBuf, PathBuf), Value> {
    publish_pair_create_new_with_hook(root, app_file, app_bytes, recipe_file, recipe_bytes, || {})
}

fn publish_pair_create_new_with_hook<F>(
    root: &TrustedRoot,
    app_file: &str,
    app_bytes: &[u8],
    recipe_file: &str,
    recipe_bytes: &[u8],
    before_recipe_publish: F,
) -> Result<(PathBuf, PathBuf), Value>
where
    F: FnOnce(),
{
    if !safe_file_name(app_file) || !safe_file_name(recipe_file) {
        return Err(invalid_destination_error());
    }
    let app_path = root.apps_directory.join(app_file);
    let recipe_path = root.recipes_directory.join(recipe_file);
    ensure_absent(&app_path)?;
    ensure_absent(&recipe_path)?;
    let app_temp = create_temp_sibling(&root.apps_directory, app_file, app_bytes)?;
    let recipe_temp = match create_temp_sibling(&root.recipes_directory, recipe_file, recipe_bytes)
    {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&app_temp);
            return Err(error);
        }
    };
    let app_guard = TempFileGuard(app_temp.clone());
    let recipe_guard = TempFileGuard(recipe_temp.clone());
    if let Err(error) = fs::hard_link(&app_temp, &app_path) {
        return Err(publication_error(error));
    }
    before_recipe_publish();
    if let Err(error) = fs::hard_link(&recipe_temp, &recipe_path) {
        let rollback = fs::remove_file(&app_path);
        if rollback.is_err() {
            return Err(api_error(
                "app_recipe_partial_publication",
                "The recipe could not be published and the newly published app definition could not be safely removed.",
                json!({}),
            ));
        }
        return Err(publication_error(error));
    }
    let _ = fs::remove_file(&app_temp);
    let _ = fs::remove_file(&recipe_temp);
    drop(app_guard);
    drop(recipe_guard);
    sync_directory_when_supported(&root.apps_directory);
    sync_directory_when_supported(&root.recipes_directory);
    Ok((app_path, recipe_path))
}

fn ensure_absent(path: &Path) -> Result<(), Value> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(destination_exists_error()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(save_failed_error()),
    }
}

fn create_temp_sibling(directory: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, Value> {
    for attempt in 0..32_u64 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = directory.join(format!(".{file_name}.emuchef-{nonce}-{attempt}.tmp"));
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|_| save_failed_error())?;
                file.sync_all().map_err(|_| save_failed_error())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(save_failed_error()),
        }
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

fn publication_error(error: io::Error) -> Value {
    if error.kind() == io::ErrorKind::AlreadyExists {
        destination_exists_error()
    } else {
        save_failed_error()
    }
}

fn destination_file(draft: &Value, field: &str) -> Option<String> {
    draft
        .get(field)?
        .get("fileName")?
        .as_str()
        .filter(|value| safe_file_name(value))
        .map(str::to_string)
}

fn safe_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && file_name.ends_with(".yaml")
        && !file_name.contains('/')
        && !file_name.contains('\\')
        && file_name != "."
        && file_name != ".."
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

fn stored_facts(
    state: &AppGeneratorState,
    session_handle: &str,
    apk_handle: &str,
) -> Result<Result<Value, Value>, String> {
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    let Some(apk) = session.apks.get(apk_handle) else {
        return Ok(Err(invalid_handle_error()));
    };
    Ok(apk.facts.clone().ok_or_else(inspection_required_error))
}

fn stored_permission_automation(
    state: &AppGeneratorState,
    session_handle: &str,
    apk_handle: &str,
    selection: Option<Value>,
    eligible_strategy: bool,
) -> Result<Result<Option<Value>, Value>, String> {
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    let Some(apk) = session.apks.get(apk_handle) else {
        return Ok(Err(invalid_handle_error()));
    };
    Ok(canonical_permission_automation(
        apk,
        selection,
        eligible_strategy,
    ))
}

fn canonical_permission_automation(
    apk: &TrustedApk,
    selection: Option<Value>,
    eligible_strategy: bool,
) -> Result<Option<Value>, Value> {
    let Some(selection) = selection else {
        return Ok(None);
    };
    let selection = serde_json::from_value::<PermissionSelectionRequest>(selection)
        .map_err(|_| permission_selection_error("altered"))?;
    if selection.runtime_permissions.is_empty() && selection.app_ops.is_empty() {
        return Ok(None);
    }
    if !eligible_strategy {
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

fn stored_root(
    state: &AppGeneratorState,
    session_handle: &str,
    root_handle: &str,
) -> Result<Result<TrustedRoot, Value>, String> {
    let registry = lock_registry(state)?;
    let session = match registry.session(session_handle) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    Ok(session
        .roots
        .get(root_handle)
        .cloned()
        .ok_or_else(invalid_handle_error))
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
    sidecar: &SidecarState,
    operation: &str,
    payload: Value,
    root: Option<&TrustedRoot>,
) -> Result<Value, String> {
    let mut response = request_sidecar(sidecar, operation, Some(payload))?;
    let Some(draft) = success_result(&response).cloned() else {
        return Ok(response);
    };
    let collisions = match root {
        Some(root) => {
            let collision_payload = match collision_payload_from_generated_draft(root, &draft) {
                Ok(payload) => payload,
                Err(_) => return Ok(generator_protocol_error()),
            };
            let collision_response = request_sidecar(
                sidecar,
                "checkGeneratedCatalogCollisions",
                Some(collision_payload),
            )?;
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
fn destination_exists_error() -> Value {
    api_error(
        "app_recipe_destination_exists",
        "A generated destination already exists; no files were overwritten.",
        json!({}),
    )
}
fn save_failed_error() -> Value {
    api_error(
        "app_recipe_save_failed",
        "The generated files could not be saved safely.",
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
fn permission_strategy_error() -> Value {
    api_error(
        "apk_permission_automation_strategy_unsupported",
        "Permission automation is supported only for pinned or latest-compatible remote APK recipes.",
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

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

    #[test]
    fn permission_selection_is_canonicalized_from_stored_inspection_only() {
        let apk = permission_test_apk("permission-canonical");
        let canonical = canonical_permission_automation(&apk, Some(permission_selection()), true)
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
            let error = canonical_permission_automation(&apk, Some(selection), true).unwrap_err();
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
            let error = canonical_permission_automation(&apk, Some(selection), true).unwrap_err();
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
        let stale_error = canonical_permission_automation(&apk, Some(stale), true).unwrap_err();
        assert_eq!(
            api_error_code(&stale_error),
            Some("app_generator_permission_selection_invalid")
        );
        assert_eq!(stale_error["error"]["details"]["reason"], "stale");

        fs::write(&apk.path, b"replacement-apk-with-new-identity").unwrap();
        let changed_error =
            canonical_permission_automation(&apk, Some(permission_selection()), true).unwrap_err();
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
            )
            .unwrap(),
            None
        );
        let error =
            canonical_permission_automation(&apk, Some(permission_selection()), false).unwrap_err();
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
            apps_directory: PathBuf::from("/trusted/authored/apps"),
            recipes_directory: PathBuf::from("/trusted/authored/recipes"),
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
            apps_directory: PathBuf::from("/trusted/authored/apps"),
            recipes_directory: PathBuf::from("/trusted/authored/recipes"),
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
    fn pair_publication_never_clobbers_and_rolls_back_first_file() {
        let root_path =
            std::env::temp_dir().join(format!("emuchef-app-generator-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root_path);
        fs::create_dir_all(root_path.join("apps")).unwrap();
        fs::create_dir_all(root_path.join("recipes")).unwrap();
        let root = validate_authored_root(&root_path).unwrap();
        publish_pair_create_new(
            &root,
            "example.yaml",
            b"app",
            "app.example.install.yaml",
            b"recipe",
        )
        .unwrap();
        assert_eq!(
            fs::read(root_path.join("apps/example.yaml")).unwrap(),
            b"app"
        );
        assert!(
            publish_pair_create_new(&root, "example.yaml", b"new", "other.yaml", b"new").is_err()
        );
        assert_eq!(
            fs::read(root_path.join("apps/example.yaml")).unwrap(),
            b"app"
        );

        let rollback_app = root_path.join("apps/rollback.yaml");
        let blocked_recipe = root_path.join("recipes/app.rollback.install.yaml");
        let result = publish_pair_create_new_with_hook(
            &root,
            "rollback.yaml",
            b"rollback-app",
            "app.rollback.install.yaml",
            b"rollback-recipe",
            || {
                fs::write(&blocked_recipe, b"concurrent-writer").unwrap();
            },
        );
        assert!(result.is_err());
        assert!(!rollback_app.exists());
        assert_eq!(fs::read(&blocked_recipe).unwrap(), b"concurrent-writer");
        fs::remove_dir_all(root_path).unwrap();
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
                json!([{
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
                }]),
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
        assert_eq!(analysis.releases.len(), 1);
        assert_eq!(analysis.releases[0].assets.len(), 1);
        assert_eq!(
            analysis.releases[0].assets[0].file_name,
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
        assert!(normalize_remote_source(
            "direct_apk",
            "https://example.com/download?variant=stable",
        )
        .is_err());
    }

    #[test]
    fn direct_url_can_defer_apk_filename_to_response_headers() {
        let source = normalize_remote_source("direct_apk", "https://example.com/download").unwrap();
        assert_eq!(source.url.as_str(), "https://example.com/download");
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
}

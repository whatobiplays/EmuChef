//! Trusted local-APK generator boundary for the Config Editor.
//!
//! Native APK, analyzer, and authored-root paths remain in process memory behind
//! session-scoped handles. Only safe facts, labels, authored drafts, diagnostics,
//! canonical previews, and relative destination metadata cross into React.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tempfile::TempDir;
use url::Url;

use crate::sidecar_client::SidecarState;

const MAX_APK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ANALYZER_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const ANALYZER_TIMEOUT: Duration = Duration::from_secs(30);
const SAFE_APK_LABEL: &str = "Selected local APK";
const SAFE_ROOT_LABEL: &str = "Selected authored root";
const PREFERENCES_FILE_NAME: &str = "app-generator-preferences.json";
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 5;
const HTTP_USER_AGENT: &str = "EmuChef-Config-Editor/0.1";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppGeneratorPreferences {
    analyzer_path: Option<PathBuf>,
    analyzer_kind: Option<String>,
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
    analyzers: HashMap<String, TrustedAnalyzer>,
    roots: HashMap<String, TrustedRoot>,
    remote_sources: HashMap<String, TrustedRemoteSource>,
    remote_assets: HashMap<String, TrustedRemoteAsset>,
    temp_directory: Option<TempDir>,
}

struct TrustedApk {
    path: PathBuf,
    identity: FileIdentity,
    facts: Option<Value>,
}

#[derive(Clone)]
struct TrustedRemoteSource {
    mode: String,
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
struct TrustedAnalyzer {
    path: PathBuf,
    kind: AnalyzerKind,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnalyzerKind {
    Apkanalyzer,
    Aapt2,
}

impl AnalyzerKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "apkanalyzer" => Some(Self::Apkanalyzer),
            "aapt2" => Some(Self::Aapt2),
            _ => None,
        }
    }

    fn protocol_name(self) -> &'static str {
        match self {
            Self::Apkanalyzer => "apkanalyzer",
            Self::Aapt2 => "aapt2",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Apkanalyzer => "Configured apkanalyzer",
            Self::Aapt2 => "Configured aapt2",
        }
    }
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
    let mut analyzer_handle = None;
    let mut analyzer_kind = None;
    let mut analyzer_label = None;
    let mut root_handle = None;
    let mut root_label = None;

    if let (Some(path), Some(kind_name)) = (
        preferences.analyzer_path.as_deref(),
        preferences.analyzer_kind.as_deref(),
    ) {
        if let Some(kind) = AnalyzerKind::parse(kind_name) {
            if let Ok(path) = validate_analyzer(path, kind) {
                let handle = registry.allocate("analyzer");
                if let Ok(session) = registry.session_mut(&session_handle) {
                    session
                        .analyzers
                        .insert(handle.clone(), TrustedAnalyzer { path, kind });
                    analyzer_handle = Some(handle);
                    analyzer_kind = Some(kind.protocol_name());
                    analyzer_label = Some(kind.display_label());
                }
            }
        }
    }

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
        "analyzerHandle": analyzer_handle,
        "analyzerKind": analyzer_kind,
        "analyzerLabel": analyzer_label,
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
pub async fn choose_app_generator_analyzer(
    app: AppHandle,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    analyzer_kind: String,
) -> Result<Value, String> {
    require_session(&state, &session_handle)?;
    let Some(kind) = AnalyzerKind::parse(&analyzer_kind) else {
        return Ok(invalid_analyzer_error());
    };
    let Some(selection) = app.dialog().file().blocking_pick_file() else {
        return Ok(success(json!({ "cancelled": true })));
    };
    let selected_path = match selection.into_path() {
        Ok(path) => path,
        Err(_) => return Ok(invalid_analyzer_error()),
    };
    let path = match validate_analyzer(&selected_path, kind) {
        Ok(path) => path,
        Err(error) => return Ok(error),
    };
    let mut registry = lock_registry(&state)?;
    let analyzer_handle = registry.allocate("analyzer");
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    session.analyzers.insert(
        analyzer_handle.clone(),
        TrustedAnalyzer {
            path: path.clone(),
            kind,
        },
    );
    update_preferences(&app, |preferences| {
        preferences.analyzer_path = Some(path);
        preferences.analyzer_kind = Some(kind.protocol_name().to_string());
    });
    Ok(success(json!({
        "cancelled": false,
        "analyzerHandle": analyzer_handle,
        "kind": kind.protocol_name(),
        "label": kind.display_label(),
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
        let analyzed = match analyze_github_source(&client, &mode, &normalized, include_prereleases)
        {
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
            "repository": source.as_ref().and_then(|value| value.repository.clone()),
            "releaseTag": asset.release_tag,
            "assetName": file_name,
        }
    })))
}

#[derive(Clone)]
struct NormalizedRemoteSource {
    url: Url,
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
    if !safe_remote_url(&url) || url.fragment().is_some() {
        return Err(remote_source_error(
            "remote_url_invalid",
            "Enter a valid public HTTPS address without credentials or a fragment.",
        ));
    }
    url.set_fragment(None);
    if mode == "direct_apk" {
        if url.query().is_some() {
            return Err(remote_source_error(
                "remote_url_invalid",
                "Enter a direct APK address without query parameters.",
            ));
        }
        return Ok(NormalizedRemoteSource {
            url,
            repository: None,
            release_tag: None,
        });
    }
    if !url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
        || url.query().is_some()
    {
        return Err(remote_source_error(
            "github_url_invalid",
            "Enter a public github.com repository or release address.",
        ));
    }
    let segments = url
        .path_segments()
        .map(|items| items.filter(|item| !item.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let (owner, repository, release_tag) = match mode {
        "github_repository" if segments.len() == 2 => {
            (segments[0], segments[1].trim_end_matches(".git"), None)
        }
        "github_release"
            if segments.len() == 5 && segments[2] == "releases" && segments[3] == "tag" =>
        {
            (segments[0], segments[1], Some(segments[4].to_string()))
        }
        _ => {
            return Err(remote_source_error(
                "github_url_invalid",
                "Enter a supported GitHub repository or release address.",
            ))
        }
    };
    if !valid_github_component(owner)
        || !valid_github_component(repository)
        || release_tag.as_deref().is_some_and(|tag| tag.is_empty())
    {
        return Err(remote_source_error(
            "github_url_invalid",
            "The GitHub owner, repository, or release tag is invalid.",
        ));
    }
    let repository_id = format!("{owner}/{repository}");
    let normalized_url = if let Some(tag) = &release_tag {
        Url::parse(&format!(
            "https://github.com/{repository_id}/releases/tag/{tag}"
        ))
        .unwrap()
    } else {
        Url::parse(&format!("https://github.com/{repository_id}")).unwrap()
    };
    Ok(NormalizedRemoteSource {
        url: normalized_url,
        repository: Some(repository_id),
        release_tag,
    })
}

fn valid_github_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
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
    let repository = source.repository.as_deref().ok_or_else(|| {
        remote_source_error("github_url_invalid", "The GitHub repository is missing.")
    })?;
    let repository_value = get_json_bounded(
        client,
        &format!("https://api.github.com/repos/{repository}"),
    )?;
    let releases_value = if mode == "github_release" {
        let tag = source.release_tag.as_deref().unwrap_or_default();
        Value::Array(vec![get_json_bounded(
            client,
            &format!("https://api.github.com/repos/{repository}/releases/tags/{tag}"),
        )?])
    } else {
        get_json_bounded(
            client,
            &format!("https://api.github.com/repos/{repository}/releases?per_page=30"),
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

fn get_json_bounded(client: &Client, url: &str) -> Result<Value, Value> {
    let mut response = client
        .get(url)
        .header(USER_AGENT, HTTP_USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| {
            remote_source_error(
                "github_request_failed",
                "GitHub information could not be retrieved.",
            )
        })?;
    if !response.status().is_success() {
        return Err(remote_source_error(
            "github_request_failed",
            "GitHub information could not be retrieved.",
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
    analyzer_handle: String,
) -> Result<Value, String> {
    let (apk_path, identity, analyzer) = {
        let registry = lock_registry(&state)?;
        let session = match registry.session(&session_handle) {
            Ok(session) => session,
            Err(error) => return Ok(error),
        };
        let Some(apk) = session.apks.get(&apk_handle) else {
            return Ok(invalid_handle_error());
        };
        let Some(analyzer) = session.analyzers.get(&analyzer_handle) else {
            return Ok(invalid_handle_error());
        };
        (apk.path.clone(), apk.identity.clone(), analyzer.clone())
    };
    if current_file_identity(&apk_path).as_ref() != Some(&identity) {
        return Ok(apk_changed_error());
    }
    let facts = match inspect_with_analyzer(&analyzer, &apk_path) {
        Ok(facts) => facts,
        Err(error) => return Ok(error),
    };
    let response = request_sidecar(
        &sidecar,
        "inspectApk",
        Some(json!({ "analyzer": analyzer.kind.protocol_name(), "facts": facts })),
    )?;
    let Some(result) = success_result(&response) else {
        return Ok(response);
    };
    if result.get("blocking").and_then(Value::as_bool) == Some(true) {
        return Ok(response);
    }
    let Some(safe_facts) = result.get("facts").cloned() else {
        return Ok(generator_protocol_error());
    };
    let mut registry = lock_registry(&state)?;
    let session = match registry.session_mut(&session_handle) {
        Ok(session) => session,
        Err(error) => return Ok(error),
    };
    let Some(apk) = session.apks.get_mut(&apk_handle) else {
        return Ok(invalid_handle_error());
    };
    apk.facts = Some(safe_facts);
    Ok(response)
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
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let facts = match stored_facts(&state, &session_handle, &apk_handle)? {
        Ok(facts) => facts,
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
    request_sidecar(
        &sidecar,
        "generateAppRecipeDraft",
        Some(Value::Object(payload)),
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
    app: Option<Value>,
    recipe: Option<Value>,
    mappings: Option<Value>,
    regenerate_identifiers: bool,
) -> Result<Value, String> {
    let facts = match stored_facts(&state, &session_handle, &apk_handle)? {
        Ok(facts) => facts,
        Err(error) => return Ok(error),
    };
    let source =
        match trusted_remote_source_payload(&state, &session_handle, &asset_handle, &strategy)? {
            Ok(source) => source,
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
    request_sidecar(
        &sidecar,
        "generateRemoteAppRecipeDraft",
        Some(Value::Object(payload)),
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
    root_handle: String,
    app: Value,
    recipe: Value,
    mappings: Value,
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
    let source =
        match trusted_remote_source_payload(&state, &session_handle, &asset_handle, &strategy)? {
            Ok(source) => source,
            Err(error) => return Ok(error),
        };
    let validation = request_sidecar(
        &sidecar,
        "generateRemoteAppRecipeDraft",
        Some(json!({
            "facts": facts,
            "source": source,
            "app": app,
            "recipe": recipe,
            "mappings": mappings,
            "regenerateIdentifiers": false,
        })),
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
    let Some(final_app) = draft.get("app").cloned() else {
        return Ok(generator_protocol_error());
    };
    let recipe_id = draft
        .get("recipe")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let collisions = request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(json!({ "authoredRoot": root.root, "app": final_app, "recipeId": recipe_id })),
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
) -> Result<Result<Value, Value>, String> {
    if !matches!(strategy, "pinned_remote_asset" | "user_provided_apk") {
        return Ok(Err(remote_source_error(
            "remote_strategy_invalid",
            "Choose a supported installation method.",
        )));
    }
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
    Ok(Ok(json!({
        "mode": source.mode,
        "strategy": strategy,
        "downloadUrl": source.direct_url.as_ref().unwrap_or(&asset.download_url),
        "repository": source.repository,
        "releaseTag": asset.release_tag.as_ref().or(source.release_tag.as_ref()),
        "assetName": asset.file_name,
    })))
}

#[tauri::command]
pub fn check_app_recipe_collisions(
    sidecar: State<'_, SidecarState>,
    state: State<'_, AppGeneratorState>,
    session_handle: String,
    root_handle: String,
    app: Value,
    recipe_id: String,
) -> Result<Value, String> {
    let root = match stored_root(&state, &session_handle, &root_handle)? {
        Ok(root) => root,
        Err(error) => return Ok(error),
    };
    request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(json!({ "authoredRoot": root.root, "app": app, "recipeId": recipe_id })),
    )
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

    let validation = request_sidecar(
        &sidecar,
        "generateAppRecipeDraft",
        Some(json!({
            "facts": facts,
            "app": app,
            "recipe": recipe,
            "mappings": mappings,
            "regenerateIdentifiers": false,
        })),
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
    let Some(final_app) = draft.get("app").cloned() else {
        return Ok(generator_protocol_error());
    };
    let recipe_id = draft
        .get("recipe")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let collisions = request_sidecar(
        &sidecar,
        "checkGeneratedCatalogCollisions",
        Some(json!({ "authoredRoot": root.root, "app": final_app, "recipeId": recipe_id })),
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

fn inspect_with_analyzer(analyzer: &TrustedAnalyzer, apk: &Path) -> Result<Value, Value> {
    match analyzer.kind {
        AnalyzerKind::Aapt2 => {
            let output = run_analyzer(&analyzer.path, &["dump", "badging"], apk)?;
            parse_aapt2_badging(&output)
        }
        AnalyzerKind::Apkanalyzer => {
            let summary = run_analyzer(&analyzer.path, &["apk", "summary"], apk)?;
            let manifest = run_analyzer(&analyzer.path, &["manifest", "print"], apk)?;
            let permissions = run_analyzer(&analyzer.path, &["manifest", "permissions"], apk)?;
            let debuggable = run_analyzer(&analyzer.path, &["manifest", "debuggable"], apk)?;
            let files = run_analyzer(&analyzer.path, &["files", "list"], apk)?;
            Ok(parse_apkanalyzer_outputs(
                &summary,
                &manifest,
                &permissions,
                &debuggable,
                &files,
            ))
        }
    }
}

fn run_analyzer(executable: &Path, args: &[&str], apk: &Path) -> Result<String, Value> {
    run_analyzer_with_limits(
        executable,
        args,
        apk,
        ANALYZER_TIMEOUT,
        MAX_ANALYZER_OUTPUT_BYTES,
    )
}

fn run_analyzer_with_limits(
    executable: &Path,
    args: &[&str],
    apk: &Path,
    timeout: Duration,
    output_limit: usize,
) -> Result<String, Value> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .arg(apk)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|_| analyzer_failed_error("analyzer_start_failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| analyzer_failed_error("analyzer_output_unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| analyzer_failed_error("analyzer_output_unavailable"))?;
    let stdout_thread = thread::spawn(move || read_bounded(stdout, output_limit));
    let stderr_thread = thread::spawn(move || read_bounded(stderr, output_limit));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(analyzer_failed_error("analyzer_timeout"));
            }
            Err(_) => return Err(analyzer_failed_error("analyzer_wait_failed")),
        }
    };
    let (stdout, stdout_truncated) = stdout_thread
        .join()
        .map_err(|_| analyzer_failed_error("analyzer_output_failed"))?;
    let (_, stderr_truncated) = stderr_thread
        .join()
        .map_err(|_| analyzer_failed_error("analyzer_output_failed"))?;
    if stdout_truncated || stderr_truncated {
        return Err(analyzer_failed_error("analyzer_output_limit"));
    }
    if !status.success() {
        return Err(analyzer_failed_error("analyzer_command_failed"));
    }
    String::from_utf8(stdout).map_err(|_| analyzer_failed_error("analyzer_output_invalid"))
}

fn read_bounded(mut reader: impl Read, output_limit: usize) -> (Vec<u8>, bool) {
    let mut retained = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = output_limit.saturating_sub(retained.len());
                let keep = remaining.min(count);
                retained.extend_from_slice(&buffer[..keep]);
                truncated |= keep < count;
            }
        }
    }
    (retained, truncated)
}

fn parse_aapt2_badging(output: &str) -> Result<Value, Value> {
    let package_line = output.lines().find(|line| line.starts_with("package:"));
    let package = package_line.and_then(|line| quoted_attribute(line, "name"));
    let version_code = package_line.and_then(|line| quoted_attribute(line, "versionCode"));
    let version_name = package_line.and_then(|line| quoted_attribute(line, "versionName"));
    let split_name = package_line.and_then(|line| quoted_attribute(line, "split"));
    let label = output
        .lines()
        .find(|line| line.starts_with("application:"))
        .and_then(|line| quoted_attribute(line, "label"));
    let min_sdk = prefixed_quoted_integer(output, "sdkVersion:");
    let target_sdk = prefixed_quoted_integer(output, "targetSdkVersion:");
    let launcher_activities = output
        .lines()
        .filter(|line| line.starts_with("launchable-activity:"))
        .filter_map(|line| quoted_attribute(line, "name"))
        .collect::<Vec<_>>();
    let requested_permissions = output
        .lines()
        .filter(|line| line.starts_with("uses-permission:"))
        .filter_map(|line| quoted_attribute(line, "name"))
        .collect::<Vec<_>>();
    let abis = output
        .lines()
        .find(|line| line.starts_with("native-code:"))
        .map(quoted_values)
        .unwrap_or_default();
    let debuggable = Some(
        output
            .lines()
            .any(|line| line.trim() == "application-debuggable"),
    );
    if package.is_none() {
        return Err(analyzer_failed_error("analyzer_output_malformed"));
    }
    Ok(json!({
        "packageName": package,
        "applicationLabel": label,
        "versionCode": version_code,
        "versionName": version_name,
        "minSdk": min_sdk,
        "targetSdk": target_sdk,
        "abis": abis,
        "launcherActivities": launcher_activities,
        "requestedPermissions": requested_permissions,
        "debuggable": debuggable,
        "split": split_name.is_some(),
        "base": split_name.is_none(),
        "certificateSha256": Value::Null,
    }))
}

fn parse_apkanalyzer_outputs(
    summary: &str,
    manifest: &str,
    permissions: &str,
    debuggable: &str,
    files: &str,
) -> Value {
    let summary_fields = summary.split_whitespace().collect::<Vec<_>>();
    let package = summary_fields.first().map(|value| (*value).to_string());
    let version_code = summary_fields.get(1).map(|value| (*value).to_string());
    let version_name = (summary_fields.len() > 2).then(|| summary_fields[2..].join(" "));
    let manifest_line = manifest.lines().find(|line| line.contains("<manifest"));
    let application_line = manifest.lines().find(|line| line.contains("<application"));
    let label = application_line
        .and_then(|line| xml_attribute(line, "android:label"))
        .filter(|value| !value.starts_with('@'));
    let min_sdk = xml_integer(manifest, "android:minSdkVersion");
    let target_sdk = xml_integer(manifest, "android:targetSdkVersion");
    let split = manifest_line
        .and_then(|line| xml_attribute(line, "split"))
        .is_some()
        || manifest.contains("android:isFeatureSplit=\"true\"");
    let launcher_activities = parse_manifest_launchers(manifest);
    let requested_permissions = permissions
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut abis = files
        .lines()
        .filter_map(|line| line.trim().strip_prefix("/lib/"))
        .filter_map(|tail| tail.split('/').next())
        .map(str::to_string)
        .collect::<Vec<_>>();
    abis.sort();
    abis.dedup();
    let debuggable = match debuggable.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    json!({
        "packageName": package,
        "applicationLabel": label,
        "versionCode": version_code,
        "versionName": version_name,
        "minSdk": min_sdk,
        "targetSdk": target_sdk,
        "abis": abis,
        "launcherActivities": launcher_activities,
        "requestedPermissions": requested_permissions,
        "debuggable": debuggable,
        "split": split,
        "base": !split,
        "certificateSha256": Value::Null,
    })
}

fn parse_manifest_launchers(manifest: &str) -> Vec<String> {
    let mut launchers = Vec::new();
    let mut activity: Option<String> = None;
    let mut has_main = false;
    let mut has_launcher = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("<activity ") || line.starts_with("<activity-alias ") {
            activity = xml_attribute(line, "android:name");
            has_main = false;
            has_launcher = false;
        } else if activity.is_some() && line.contains("android.intent.action.MAIN") {
            has_main = true;
        } else if activity.is_some() && line.contains("android.intent.category.LAUNCHER") {
            has_launcher = true;
        } else if (line.starts_with("</activity") || line.starts_with("</activity-alias"))
            && activity.is_some()
        {
            if has_main && has_launcher {
                launchers.push(activity.take().unwrap_or_default());
            } else {
                activity = None;
            }
        }
    }
    launchers.sort();
    launchers.dedup();
    launchers
}

fn quoted_attribute(line: &str, name: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let needle = format!("{name}={quote}");
        if let Some(start) = line.find(&needle) {
            let rest = &line[start + needle.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn quoted_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('\'') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('\'') else {
            break;
        };
        values.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    values
}

fn prefixed_quoted_integer(output: &str, prefix: &str) -> Option<i64> {
    output
        .lines()
        .find(|line| line.starts_with(prefix))
        .and_then(|line| quoted_values(line).first().cloned())
        .and_then(|value| value.parse().ok())
}

fn xml_attribute(line: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    Some(rest[..rest.find('"')?].to_string())
}

fn xml_integer(manifest: &str, name: &str) -> Option<i64> {
    manifest
        .lines()
        .find_map(|line| xml_attribute(line, name))
        .and_then(|value| value.parse().ok())
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

fn validate_analyzer(path: &Path, kind: AnalyzerKind) -> Result<PathBuf, Value> {
    let selected_metadata = fs::symlink_metadata(path).map_err(|_| invalid_analyzer_error())?;
    if !selected_metadata.file_type().is_file() {
        return Err(invalid_analyzer_error());
    }
    let canonical = fs::canonicalize(path).map_err(|_| invalid_analyzer_error())?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| invalid_analyzer_error())?;
    if !metadata.file_type().is_file() {
        return Err(invalid_analyzer_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(invalid_analyzer_error());
        }
    }
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let matches_kind = match kind {
        AnalyzerKind::Apkanalyzer => file_name == "apkanalyzer" || file_name == "apkanalyzer.exe",
        AnalyzerKind::Aapt2 => file_name == "aapt2" || file_name == "aapt2.exe",
    };
    if !matches_kind {
        return Err(invalid_analyzer_error());
    }
    Ok(canonical)
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
fn invalid_analyzer_error() -> Value {
    api_error(
        "apk_analyzer_invalid",
        "Choose a regular executable matching the selected analyzer type.",
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
fn generator_protocol_error() -> Value {
    api_error(
        "app_generator_protocol_error",
        "The generator backend returned an incomplete response.",
        json!({}),
    )
}
fn analyzer_failed_error(reason: &str) -> Value {
    api_error(
        "apk_analyzer_failed",
        "The configured APK analyzer could not inspect this APK.",
        json!({ "reason": reason }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn aapt2_badging_parser_extracts_safe_manifest_facts() {
        let output = "package: name='com.example.app' versionCode='42' versionName='1.2'\nuses-permission: name='android.permission.INTERNET'\nsdkVersion:'23'\ntargetSdkVersion:'35'\napplication: label='Example' icon='x'\nlaunchable-activity: name='com.example.app.MainActivity' label='' icon=''\nnative-code: 'arm64-v8a' 'x86_64'\n";
        let facts = parse_aapt2_badging(output).unwrap();
        assert_eq!(facts["packageName"], "com.example.app");
        assert_eq!(
            facts["launcherActivities"][0],
            "com.example.app.MainActivity"
        );
        assert_eq!(facts["split"], false);
    }

    #[test]
    fn apkanalyzer_parser_extracts_launchers_and_never_includes_paths() {
        let manifest = r#"<manifest package="com.example.app"><uses-sdk android:minSdkVersion="23" android:targetSdkVersion="35"/><application android:label="Example"><activity android:name=".Main"><intent-filter><action android:name="android.intent.action.MAIN"/><category android:name="android.intent.category.LAUNCHER"/></intent-filter></activity></application></manifest>"#;
        let facts = parse_apkanalyzer_outputs(
            "com.example.app 7 1.0",
            manifest,
            "android.permission.INTERNET",
            "false",
            "/lib/arm64-v8a/libx.so",
        );
        assert_eq!(facts["packageName"], "com.example.app");
        assert!(!facts.to_string().contains("/tmp/"));
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
    fn apk_analyzer_and_root_validation_reject_unsafe_selections() {
        let directory = temp_path("validation");
        fs::create_dir_all(&directory).unwrap();

        let wrong_extension = directory.join("selected.zip");
        fs::write(&wrong_extension, b"not-an-apk").unwrap();
        assert!(validate_apk(&wrong_extension).is_err());
        assert!(validate_apk(&directory).is_err());

        let wrong_analyzer = directory.join("arbitrary-tool");
        fs::write(&wrong_analyzer, b"tool").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wrong_analyzer, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(validate_analyzer(&wrong_analyzer, AnalyzerKind::Apkanalyzer).is_err());

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

    #[cfg(unix)]
    #[test]
    fn analyzer_runner_enforces_timeout_and_output_bounds_without_shell_interpolation() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temp_path("runner");
        fs::create_dir_all(&directory).unwrap();
        let large = directory.join("aapt2");
        fs::write(
            &large,
            "#!/bin/sh\nprintf '0123456789012345678901234567890123456789'\n",
        )
        .unwrap();
        fs::set_permissions(&large, fs::Permissions::from_mode(0o755)).unwrap();
        let error = run_analyzer_with_limits(
            &large,
            &["dump", "badging"],
            Path::new("literal;not-executed.apk"),
            Duration::from_secs(1),
            16,
        )
        .unwrap_err();
        assert_eq!(error["error"]["details"]["reason"], "analyzer_output_limit");

        let slow = directory.join("apkanalyzer");
        fs::write(&slow, "#!/bin/sh\nsleep 1\n").unwrap();
        fs::set_permissions(&slow, fs::Permissions::from_mode(0o755)).unwrap();
        let error = run_analyzer_with_limits(
            &slow,
            &["apk", "summary"],
            Path::new("literal.apk"),
            Duration::from_millis(25),
            1024,
        )
        .unwrap_err();
        assert_eq!(error["error"]["details"]["reason"], "analyzer_timeout");
        fs::remove_dir_all(directory).unwrap();
    }
}

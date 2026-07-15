//! Secure, user-triggered update discovery and manual DMG handoff.
//!
//! This module is the sole owner of update endpoints, metadata keys, response
//! policy, manifest validation, retained candidates, and external navigation.
//! The frontend receives display-only data and cannot supply a URL, signature,
//! key, path, or opener argument.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE};
use ring::signature::{UnparsedPublicKey, ED25519};
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use crate::commands::{safe_error, AppState};

const PRODUCTION_TRUST: &str = include_str!("../update-trust.json");
const MANIFEST_LIMIT: u64 = 64 * 1024;
const NOTES_LIMIT: usize = 16 * 1024;
const DMG_LIMIT: u64 = 512 * 1024 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_FRONTEND_GENERATION: u32 = 1_000_000;
const FUTURE_SKEW_SECONDS: i64 = 10 * 60;
const MAX_VALIDITY_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustDocument {
    schema_version: u64,
    configured: bool,
    manifest_url: Option<String>,
    dmg_origin: Option<String>,
    dmg_path_prefix: Option<String>,
    metadata_key_id: Option<String>,
    metadata_public_key: Option<String>,
}

#[derive(Clone, Debug)]
struct ConfiguredTrust {
    manifest_url: String,
    dmg_origin: String,
    dmg_path_prefix: String,
    metadata_key_id: String,
    metadata_public_key: [u8; 32],
    allow_http_for_tests: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u64,
    product: String,
    channel: String,
    platform: String,
    architecture: String,
    version: String,
    published_at: String,
    expires_at: String,
    notes: String,
    dmg_url: String,
    dmg_size_bytes: u64,
    dmg_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_macos_version: Option<String>,
    metadata_key_id: String,
    metadata_signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedManifest<'a> {
    schema_version: u64,
    product: &'a str,
    channel: &'a str,
    platform: &'a str,
    architecture: &'a str,
    version: &'a str,
    published_at: &'a str,
    expires_at: &'a str,
    notes: &'a str,
    dmg_url: &'a str,
    dmg_size_bytes: u64,
    dmg_sha256: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_macos_version: Option<&'a str>,
    metadata_key_id: &'a str,
}

impl Manifest {
    fn signed(&self) -> SignedManifest<'_> {
        SignedManifest {
            schema_version: self.schema_version,
            product: &self.product,
            channel: &self.channel,
            platform: &self.platform,
            architecture: &self.architecture,
            version: &self.version,
            published_at: &self.published_at,
            expires_at: &self.expires_at,
            notes: &self.notes,
            dmg_url: &self.dmg_url,
            dmg_size_bytes: self.dmg_size_bytes,
            dmg_sha256: &self.dmg_sha256,
            minimum_macos_version: self.minimum_macos_version.as_deref(),
            metadata_key_id: &self.metadata_key_id,
        }
    }

    fn canonical_signed_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&self.signed()).map_err(|_| manifest_error())
    }

    fn canonical_full_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|_| manifest_error())
    }
}

#[derive(Clone, Debug)]
struct ValidatedCandidate {
    manifest: Manifest,
    version: Version,
    expires_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusDto {
    state: &'static str,
    current_version: String,
    latest_version: Option<String>,
    published_at: Option<String>,
    expires_at: Option<String>,
    notes: Option<String>,
    dmg_size_bytes: Option<u64>,
    dmg_sha256: Option<String>,
    minimum_macos_version: Option<String>,
    minimum_macos_version_is_informational: bool,
    can_open_download: bool,
    message: Option<String>,
}

#[derive(Debug)]
struct RuntimeState {
    phase: &'static str,
    candidate: Option<ValidatedCandidate>,
    message: Option<String>,
    check_in_progress: bool,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            phase: "idle",
            candidate: None,
            message: None,
            check_in_progress: false,
        }
    }
}

/// Process-local update state. It never persists metadata or browser authority.
pub struct UpdateService {
    trust: Option<ConfiguredTrust>,
    state: Mutex<RuntimeState>,
}

impl UpdateService {
    pub fn from_production_document() -> Result<Self, String> {
        let trust: TrustDocument = serde_json::from_str(PRODUCTION_TRUST).map_err(|_| {
            safe_error(
                "update_trust_invalid",
                "Update discovery is unavailable because its trust configuration is invalid.",
            )
        })?;
        if trust.schema_version != 1 {
            return Err(safe_error(
                "update_trust_invalid",
                "Update discovery is unavailable because its trust configuration is invalid.",
            ));
        }
        if !trust.configured {
            if trust.manifest_url.is_some()
                || trust.dmg_origin.is_some()
                || trust.dmg_path_prefix.is_some()
                || trust.metadata_key_id.is_some()
                || trust.metadata_public_key.is_some()
            {
                return Err(safe_error(
                    "update_trust_invalid",
                    "Update discovery is unavailable because its trust configuration is invalid.",
                ));
            }
            return Ok(Self {
                trust: None,
                state: Mutex::new(RuntimeState::default()),
            });
        }
        let key = decode_fixed_hex::<32>(trust.metadata_public_key.as_deref().unwrap_or_default())
            .map_err(|_| {
                safe_error("update_trust_invalid", "Update discovery trust is invalid.")
            })?;
        let configured = ConfiguredTrust {
            manifest_url: trust.manifest_url.ok_or_else(manifest_error)?,
            dmg_origin: trust.dmg_origin.ok_or_else(manifest_error)?,
            dmg_path_prefix: trust.dmg_path_prefix.ok_or_else(manifest_error)?,
            metadata_key_id: trust.metadata_key_id.ok_or_else(manifest_error)?,
            metadata_public_key: key,
            allow_http_for_tests: false,
        };
        validate_trust(&configured)?;
        Ok(Self {
            trust: Some(configured),
            state: Mutex::new(RuntimeState::default()),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, RuntimeState>, String> {
        self.state.lock().map_err(|_| {
            safe_error(
                "update_state_unavailable",
                "Update status is temporarily unavailable.",
            )
        })
    }

    pub fn status(&self) -> Result<UpdateStatusDto, String> {
        if self.trust.is_none() {
            return Ok(unconfigured_status());
        }
        let state = self.lock()?;
        Ok(status_from_state(&state))
    }

    async fn check(&self) -> Result<UpdateStatusDto, String> {
        let Some(trust) = self.trust.clone() else {
            return Ok(unconfigured_status());
        };
        {
            let mut state = self.lock()?;
            if state.check_in_progress {
                return Err(safe_error(
                    "update_check_active",
                    "An update check is already in progress.",
                ));
            }
            state.check_in_progress = true;
            state.phase = "checking";
            state.candidate = None;
            state.message = None;
        }
        let mut check_lease = CheckLease {
            service: self,
            completed: false,
        };

        let result = fetch_and_validate(&trust, OffsetDateTime::now_utc()).await;
        let mut state = self.lock()?;
        state.check_in_progress = false;
        match result {
            Ok(Some(candidate)) => {
                state.phase = "update_available";
                state.candidate = Some(candidate);
                state.message = None;
            }
            Ok(None) => {
                state.phase = "up_to_date";
                state.candidate = None;
                state.message = Some("This version of EmuChef is up to date.".to_string());
            }
            Err(_) => {
                state.phase = "failed";
                state.candidate = None;
                state.message = Some(
                    "EmuChef could not validate update information. Local use is unaffected."
                        .to_string(),
                );
            }
        }
        check_lease.completed = true;
        Ok(status_from_state(&state))
    }

    fn candidate_for_open(&self, now: OffsetDateTime) -> Result<ValidatedCandidate, String> {
        let trust = self.trust.as_ref().ok_or_else(|| {
            safe_error(
                "updates_unconfigured",
                "Update discovery is not configured in this build.",
            )
        })?;
        let state = self.lock()?;
        let candidate = state.candidate.clone().ok_or_else(|| {
            safe_error(
                "update_candidate_unavailable",
                "Check for updates before opening a download.",
            )
        })?;
        validate_manifest_policy(&candidate.manifest, trust, now)?;
        if candidate.version <= current_version()? || candidate.expires_at <= now {
            return Err(safe_error(
                "update_candidate_stale",
                "The retained update is no longer eligible. Check again.",
            ));
        }
        Ok(candidate)
    }
}

/// Restores a non-fatal local state if Tauri cancels a manifest-check future.
struct CheckLease<'a> {
    service: &'a UpdateService,
    completed: bool,
}

impl Drop for CheckLease<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Ok(mut state) = self.service.state.lock() {
            state.check_in_progress = false;
            state.phase = "failed";
            state.candidate = None;
            state.message = Some(
                "The update check stopped before completion. Local use is unaffected.".to_string(),
            );
        }
    }
}

fn unconfigured_status() -> UpdateStatusDto {
    UpdateStatusDto {
        state: "unconfigured",
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: None,
        published_at: None,
        expires_at: None,
        notes: None,
        dmg_size_bytes: None,
        dmg_sha256: None,
        minimum_macos_version: None,
        minimum_macos_version_is_informational: true,
        can_open_download: false,
        message: Some("Update discovery is not configured in this build.".to_string()),
    }
}

fn status_from_state(state: &RuntimeState) -> UpdateStatusDto {
    let candidate = state.candidate.as_ref();
    UpdateStatusDto {
        state: state.phase,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: candidate.map(|value| value.manifest.version.clone()),
        published_at: candidate.map(|value| value.manifest.published_at.clone()),
        expires_at: candidate.map(|value| value.manifest.expires_at.clone()),
        notes: candidate.map(|value| value.manifest.notes.clone()),
        dmg_size_bytes: candidate.map(|value| value.manifest.dmg_size_bytes),
        dmg_sha256: candidate.map(|value| value.manifest.dmg_sha256.clone()),
        minimum_macos_version: candidate
            .and_then(|value| value.manifest.minimum_macos_version.clone()),
        minimum_macos_version_is_informational: true,
        can_open_download: candidate.is_some(),
        message: state.message.clone(),
    }
}

fn current_version() -> Result<Version, String> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| manifest_error())
}

fn validate_trust(trust: &ConfiguredTrust) -> Result<(), String> {
    let manifest_url = validate_https_url(&trust.manifest_url, trust.allow_http_for_tests)?;
    if manifest_url.query().is_some() || manifest_url.fragment().is_some() {
        return Err(manifest_error());
    }
    let origin = tauri::Url::parse(&trust.dmg_origin).map_err(|_| manifest_error())?;
    if origin.path() != "/" || origin.query().is_some() || origin.fragment().is_some() {
        return Err(manifest_error());
    }
    validate_https_url(&trust.dmg_origin, trust.allow_http_for_tests)?;
    validate_dmg_path_prefix(&trust.dmg_path_prefix)?;
    let key_id = trust.metadata_key_id.to_ascii_lowercase();
    if trust.metadata_key_id.is_empty()
        || key_id.starts_with("test-")
        || key_id.starts_with("test_")
        || key_id.starts_with("fixture-")
        || key_id.starts_with("fixture_")
    {
        return Err(manifest_error());
    }
    Ok(())
}

fn validate_https_url(value: &str, allow_http: bool) -> Result<tauri::Url, String> {
    let url = tauri::Url::parse(value).map_err(|_| manifest_error())?;
    if (url.scheme() != "https" && !(allow_http && url.scheme() == "http"))
        || url.username() != ""
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(manifest_error());
    }
    Ok(url)
}

fn validate_dmg_url(value: &str, trust: &ConfiguredTrust) -> Result<(), String> {
    validate_dmg_path_prefix(&trust.dmg_path_prefix)?;
    let url = validate_https_url(value, trust.allow_http_for_tests)?;
    let origin = tauri::Url::parse(&trust.dmg_origin).map_err(|_| manifest_error())?;
    let raw_path = raw_url_path(value).ok_or_else(manifest_error)?;
    if url.scheme() != origin.scheme()
        || url.host_str() != origin.host_str()
        || url.port_or_known_default() != origin.port_or_known_default()
        || raw_path != url.path()
        || !url.path().starts_with(&trust.dmg_path_prefix)
        || !url.path().ends_with(".dmg")
        || url.query().is_some()
        || url.fragment().is_some()
        || path_is_not_normalized(url.path(), false)
    {
        return Err(manifest_error());
    }
    Ok(())
}

fn validate_dmg_path_prefix(prefix: &str) -> Result<(), String> {
    if !prefix.starts_with('/')
        || !prefix.ends_with('/')
        || prefix.contains('?')
        || prefix.contains('#')
        || path_is_not_normalized(prefix, true)
    {
        return Err(manifest_error());
    }
    Ok(())
}

fn path_is_not_normalized(path: &str, require_trailing_slash: bool) -> bool {
    let lower = path.to_ascii_lowercase();
    !path.starts_with('/')
        || (require_trailing_slash && !path.ends_with('/'))
        || path.contains("//")
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%2e")
        || lower.contains("%25")
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
}

fn raw_url_path(value: &str) -> Option<&str> {
    let authority_start = value.find("://")? + 3;
    let path_start = value[authority_start..].find('/')? + authority_start;
    let path_and_suffix = &value[path_start..];
    let path_end = path_and_suffix
        .find(['?', '#'])
        .unwrap_or(path_and_suffix.len());
    Some(&path_and_suffix[..path_end])
}

fn strict_parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    validate_json_lexical_contract(bytes)?;
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| manifest_error())?;
    if manifest.canonical_full_bytes()? != bytes {
        return Err(manifest_error());
    }
    Ok(manifest)
}

fn validate_json_lexical_contract(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| manifest_error())?;
    if text.starts_with('\u{feff}') || bytes.is_empty() {
        return Err(manifest_error());
    }
    let mut keys = HashSet::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index + 1;
                index += 1;
                let mut escaped = false;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => {
                            escaped = true;
                            index += 2;
                        }
                        b'"' => break,
                        _ => index += 1,
                    }
                }
                if index >= bytes.len() {
                    return Err(manifest_error());
                }
                let mut after = index + 1;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    if escaped {
                        return Err(manifest_error());
                    }
                    let key = &text[start..index];
                    if !keys.insert(key) {
                        return Err(manifest_error());
                    }
                }
                index += 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && !matches!(
                        bytes[index],
                        b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t'
                    )
                {
                    index += 1;
                }
                let token = &text[start..index];
                if token.starts_with('-')
                    || !token.bytes().all(|byte| byte.is_ascii_digit())
                    || (token.len() > 1 && token.starts_with('0'))
                    || token
                        .parse::<u64>()
                        .map_or(true, |value| value > MAX_SAFE_INTEGER)
                {
                    return Err(manifest_error());
                }
            }
            _ => index += 1,
        }
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime, String> {
    if value.len() != 20
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
        || value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
        || !value.ends_with('Z')
    {
        return Err(manifest_error());
    }
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| manifest_error())
}

fn validate_manifest_policy(
    manifest: &Manifest,
    trust: &ConfiguredTrust,
    now: OffsetDateTime,
) -> Result<(Version, OffsetDateTime), String> {
    if manifest.schema_version != 1
        || manifest.product != "com.emuchef.desktop"
        || manifest.channel != "stable"
        || manifest.platform != "darwin"
        || manifest.architecture != "aarch64"
        || manifest.metadata_key_id != trust.metadata_key_id
        || manifest.notes.len() > NOTES_LIMIT
        || manifest.notes.contains('\0')
        || contains_local_path_leak(&manifest.notes)
        || manifest.dmg_size_bytes == 0
        || manifest.dmg_size_bytes > DMG_LIMIT
        || manifest.dmg_sha256.len() != 64
        || !manifest
            .dmg_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(manifest_error());
    }
    let version = Version::parse(&manifest.version).map_err(|_| manifest_error())?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(manifest_error());
    }
    let published = parse_timestamp(&manifest.published_at)?;
    let expires = parse_timestamp(&manifest.expires_at)?;
    if published > now + time::Duration::seconds(FUTURE_SKEW_SECONDS)
        || expires <= now
        || expires <= published
        || expires - published > time::Duration::seconds(MAX_VALIDITY_SECONDS)
    {
        return Err(manifest_error());
    }
    if let Some(minimum) = manifest.minimum_macos_version.as_deref() {
        if minimum.is_empty()
            || Version::parse(minimum).is_err()
            || minimum.contains('-')
            || minimum.contains('+')
        {
            return Err(manifest_error());
        }
    }
    validate_dmg_url(&manifest.dmg_url, trust)?;
    let signature = decode_fixed_hex::<64>(&manifest.metadata_signature)?;
    UnparsedPublicKey::new(&ED25519, trust.metadata_public_key)
        .verify(&manifest.canonical_signed_bytes()?, &signature)
        .map_err(|_| manifest_error())?;
    Ok((version, expires))
}

fn contains_local_path_leak(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("file://")
        || lower.contains("/users/")
        || lower.contains("/home/")
        || lower.contains("/private/")
        || lower.contains("/tmp/")
        || value.as_bytes().windows(3).any(|window| {
            window[0].is_ascii_alphabetic() && window[1] == b':' && window[2] == b'\\'
        })
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(manifest_error());
    }
    let decoded = hex::decode(value).map_err(|_| manifest_error())?;
    decoded.try_into().map_err(|_| manifest_error())
}

async fn fetch_and_validate(
    trust: &ConfiguredTrust,
    now: OffsetDateTime,
) -> Result<Option<ValidatedCandidate>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| transport_error())?;
    let response = client
        .get(&trust.manifest_url)
        .header(ACCEPT, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|_| transport_error())?;
    let expected = validate_response_headers(&response)?;
    let bytes = read_bounded(response, expected).await?;
    let manifest = strict_parse_manifest(&bytes)?;
    let (version, expires_at) = validate_manifest_policy(&manifest, trust, now)?;
    if version <= current_version()? {
        return Ok(None);
    }
    Ok(Some(ValidatedCandidate {
        manifest,
        version,
        expires_at,
    }))
}

fn validate_response_headers(response: &reqwest::Response) -> Result<u64, String> {
    validate_response_policy(response.status(), response.headers())
}

fn validate_response_policy(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> Result<u64, String> {
    if status != reqwest::StatusCode::OK {
        return Err(transport_error());
    }
    let content_types = headers.get_all(CONTENT_TYPE).iter().collect::<Vec<_>>();
    if content_types.len() != 1 {
        return Err(transport_error());
    }
    let content_type = content_types[0].to_str().map_err(|_| transport_error())?;
    let normalized = content_type.to_ascii_lowercase().replace(' ', "");
    if normalized != "application/json" && normalized != "application/json;charset=utf-8" {
        return Err(transport_error());
    }
    let encodings = headers.get_all(CONTENT_ENCODING).iter().collect::<Vec<_>>();
    if encodings.len() > 1 {
        return Err(transport_error());
    }
    if let Some(encoding) = encodings.first() {
        if !encoding
            .to_str()
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case("identity"))
        {
            return Err(transport_error());
        }
    }
    let lengths = headers
        .get_all(CONTENT_LENGTH)
        .iter()
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(transport_error)?;
    if lengths.len() != 1 || lengths[0] == 0 || lengths[0] > MANIFEST_LIMIT {
        return Err(transport_error());
    }
    Ok(lengths[0])
}

async fn read_bounded(mut response: reqwest::Response, expected: u64) -> Result<Vec<u8>, String> {
    let mut body = Vec::with_capacity(expected as usize);
    while let Some(chunk) = response.chunk().await.map_err(|_| transport_error())? {
        let next = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(transport_error)?;
        if next as u64 > expected || next as u64 > MANIFEST_LIMIT {
            return Err(transport_error());
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() || body.len() as u64 != expected {
        return Err(transport_error());
    }
    Ok(body)
}

fn manifest_error() -> String {
    safe_error(
        "update_manifest_invalid",
        "The update service returned information that EmuChef could not validate.",
    )
}

fn transport_error() -> String {
    safe_error(
        "update_service_unavailable",
        "EmuChef could not securely read update information. Local use is unaffected.",
    )
}

#[derive(Debug, Default)]
struct ActivityState {
    frontend: Option<FrontendSession>,
    native_dialogs: u32,
    cleanup_active: bool,
    execution_starts: u32,
    navigation_active: bool,
}

#[derive(Debug)]
struct FrontendSession {
    id: String,
    generation: u32,
    blocked: bool,
}

/// Shared lock-order root for activity that cannot overlap external navigation.
///
/// Callers acquire a short reservation here before any execution, cleanup, or
/// update-state lock. The mutex is released before native dialogs, cleanup,
/// execution work, or the operating-system opener runs; only a lease flag stays
/// active. This prevents races without holding an ordinary mutex across OS IPC.
#[derive(Default)]
pub struct ActivityGate {
    inner: Arc<Mutex<ActivityState>>,
}

impl ActivityGate {
    pub fn begin_frontend_session(&self) -> Result<FrontendSessionDto, String> {
        let mut state = self.lock()?;
        let id = Uuid::new_v4().to_string();
        state.frontend = Some(FrontendSession {
            id: id.clone(),
            generation: 0,
            blocked: true,
        });
        Ok(FrontendSessionDto {
            session_id: id,
            generation: 0,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, ActivityState>, String> {
        self.inner.lock().map_err(|_| {
            safe_error(
                "update_activity_unavailable",
                "External navigation is temporarily unavailable.",
            )
        })
    }

    fn update_frontend(&self, request: FrontendInteractionRequest) -> Result<(), String> {
        if request.generation > MAX_FRONTEND_GENERATION {
            return Err(activity_error());
        }
        let mut state = self.lock()?;
        let session = state.frontend.as_mut().ok_or_else(activity_error)?;
        if request.session_id != session.id || request.generation < session.generation {
            return Err(activity_error());
        }
        if request.generation == session.generation {
            if request.blocked == session.blocked {
                return Ok(());
            }
            return Err(activity_error());
        }
        session.generation = request.generation;
        session.blocked = request.blocked;
        Ok(())
    }

    fn end_frontend(&self, session_id: &str) -> Result<(), String> {
        let mut state = self.lock()?;
        if state
            .frontend
            .as_ref()
            .is_some_and(|session| session.id == session_id)
        {
            state.frontend = None;
        }
        Ok(())
    }

    pub fn reserve_navigation(&self) -> Result<ActivityLease, String> {
        let mut state = self.lock()?;
        let frontend_ready = state.frontend.as_ref().is_some_and(|value| !value.blocked);
        if !frontend_ready
            || state.native_dialogs != 0
            || state.cleanup_active
            || state.execution_starts != 0
            || state.navigation_active
        {
            return Err(activity_error());
        }
        state.navigation_active = true;
        Ok(ActivityLease::new(
            self.inner.clone(),
            LeaseKind::Navigation,
        ))
    }

    pub fn reserve_execution_start(&self) -> Result<ActivityLease, String> {
        let mut state = self.lock()?;
        if state.navigation_active || state.execution_starts == u32::MAX {
            return Err(activity_error());
        }
        state.execution_starts += 1;
        Ok(ActivityLease::new(
            self.inner.clone(),
            LeaseKind::ExecutionStart,
        ))
    }

    pub fn reserve_cleanup(&self) -> Result<ActivityLease, String> {
        let mut state = self.lock()?;
        if state.navigation_active || state.cleanup_active {
            return Err(activity_error());
        }
        state.cleanup_active = true;
        Ok(ActivityLease::new(self.inner.clone(), LeaseKind::Cleanup))
    }

    pub fn reserve_native_dialog(&self) -> Result<ActivityLease, String> {
        let mut state = self.lock()?;
        if state.navigation_active || state.native_dialogs == u32::MAX {
            return Err(activity_error());
        }
        state.native_dialogs += 1;
        Ok(ActivityLease::new(
            self.inner.clone(),
            LeaseKind::NativeDialog,
        ))
    }
}

fn activity_error() -> String {
    safe_error(
        "external_navigation_blocked",
        "Close active dialogs and wait for current work to finish before opening the update download.",
    )
}

#[derive(Debug)]
enum LeaseKind {
    Navigation,
    ExecutionStart,
    Cleanup,
    NativeDialog,
}

pub struct ActivityLease {
    state: Arc<Mutex<ActivityState>>,
    kind: Option<LeaseKind>,
}

impl ActivityLease {
    fn new(state: Arc<Mutex<ActivityState>>, kind: LeaseKind) -> Self {
        Self {
            state,
            kind: Some(kind),
        }
    }
}

impl Drop for ActivityLease {
    fn drop(&mut self) {
        let Some(kind) = self.kind.take() else { return };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        match kind {
            LeaseKind::Navigation => state.navigation_active = false,
            LeaseKind::ExecutionStart => {
                state.execution_starts = state.execution_starts.saturating_sub(1)
            }
            LeaseKind::Cleanup => state.cleanup_active = false,
            LeaseKind::NativeDialog => {
                state.native_dialogs = state.native_dialogs.saturating_sub(1)
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendSessionDto {
    session_id: String,
    generation: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendInteractionRequest {
    session_id: String,
    generation: u32,
    blocked: bool,
}

#[tauri::command]
pub fn get_update_status(state: State<'_, AppState>) -> Result<UpdateStatusDto, String> {
    state.updates.status()
}

#[tauri::command]
pub async fn check_for_updates(state: State<'_, AppState>) -> Result<UpdateStatusDto, String> {
    state.updates.check().await
}

#[tauri::command]
pub fn begin_update_interaction_session(
    state: State<'_, AppState>,
) -> Result<FrontendSessionDto, String> {
    state.update_activity.begin_frontend_session()
}

#[tauri::command]
pub fn set_update_interaction_state(
    request: FrontendInteractionRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.update_activity.update_frontend(request)
}

#[tauri::command]
pub fn end_update_interaction_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.update_activity.end_frontend(&session_id)
}

#[tauri::command]
pub fn open_update_download(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _navigation = state.update_activity.reserve_navigation()?;
    if state
        .executions
        .lock()
        .map_err(|_| activity_error())?
        .has_in_flight()
    {
        return Err(activity_error());
    }
    let candidate = state
        .updates
        .candidate_for_open(OffsetDateTime::now_utc())?;
    app.opener()
        .open_url(candidate.manifest.dmg_url, None::<&str>)
        .map_err(|_| {
            safe_error(
                "update_download_open_failed",
                "The validated update download could not be opened in the default browser.",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn signed_manifest(notes: &str, minimum: Option<&str>) -> (Manifest, ConfiguredTrust) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let mut manifest = Manifest {
            schema_version: 1,
            product: "com.emuchef.desktop".into(),
            channel: "stable".into(),
            platform: "darwin".into(),
            architecture: "aarch64".into(),
            version: "9.0.0".into(),
            published_at: "2026-07-14T20:00:00Z".into(),
            expires_at: "2026-07-20T20:00:00Z".into(),
            notes: notes.into(),
            dmg_url: "https://releases.example.test/emuchef/EmuChef-9.0.0.dmg".into(),
            dmg_size_bytes: 1024,
            dmg_sha256: "a".repeat(64),
            minimum_macos_version: minimum.map(str::to_string),
            metadata_key_id: "test-metadata-2026".into(),
            metadata_signature: String::new(),
        };
        manifest.metadata_signature = hex::encode(
            key_pair
                .sign(&manifest.canonical_signed_bytes().unwrap())
                .as_ref(),
        );
        let mut public = [0_u8; 32];
        public.copy_from_slice(key_pair.public_key().as_ref());
        let trust = ConfiguredTrust {
            manifest_url: "https://releases.example.test/emuchef/stable.json".into(),
            dmg_origin: "https://releases.example.test/".into(),
            dmg_path_prefix: "/emuchef/".into(),
            metadata_key_id: "test-metadata-2026".into(),
            metadata_public_key: public,
            allow_http_for_tests: false,
        };
        (manifest, trust)
    }

    fn production_trust_for_prefix(prefix: &str) -> ConfiguredTrust {
        ConfiguredTrust {
            manifest_url: "https://updates.example.test/emuchef/stable.json".into(),
            dmg_origin: "https://downloads.example.test/".into(),
            dmg_path_prefix: prefix.into(),
            metadata_key_id: "production-metadata-2026".into(),
            metadata_public_key: [1_u8; 32],
            allow_http_for_tests: false,
        }
    }

    #[test]
    fn production_is_strictly_unconfigured_and_local() {
        let service = UpdateService::from_production_document().unwrap();
        assert_eq!(service.status().unwrap().state, "unconfigured");
        assert!(service.trust.is_none());
        assert_eq!(
            tauri::async_runtime::block_on(service.check())
                .unwrap()
                .state,
            "unconfigured"
        );
    }

    #[test]
    fn cancelled_check_lease_returns_runtime_to_a_safe_local_state() {
        let service = UpdateService {
            trust: None,
            state: Mutex::new(RuntimeState {
                phase: "checking",
                candidate: None,
                message: None,
                check_in_progress: true,
            }),
        };
        drop(CheckLease {
            service: &service,
            completed: false,
        });
        let state = service.lock().unwrap();
        assert!(!state.check_in_progress);
        assert_eq!(state.phase, "failed");
        assert!(state.message.as_deref().unwrap().contains("stopped"));
    }

    #[test]
    fn canonical_bytes_cover_escaping_unicode_and_optional_order() {
        let (manifest, _) = signed_manifest("Quote \" slash \\\\ line\n café", Some("14.0.0"));
        let signed = String::from_utf8(manifest.canonical_signed_bytes().unwrap()).unwrap();
        assert!(signed.starts_with("{\"schemaVersion\":1,\"product\":\"com.emuchef.desktop\""));
        assert!(signed.contains("Quote \\\" slash \\\\\\\\ line\\n café"));
        assert!(signed.ends_with(
            "\"minimumMacosVersion\":\"14.0.0\",\"metadataKeyId\":\"test-metadata-2026\"}"
        ));
        assert!(!signed.ends_with('\n'));
        let (omitted, _) = signed_manifest("", None);
        assert!(
            !String::from_utf8(omitted.canonical_signed_bytes().unwrap())
                .unwrap()
                .contains("minimumMacosVersion")
        );
    }

    #[test]
    fn rust_canonical_bytes_match_the_shared_node_golden() {
        let manifest = Manifest {
            schema_version: 1,
            product: "com.emuchef.desktop".into(),
            channel: "stable".into(),
            platform: "darwin".into(),
            architecture: "aarch64".into(),
            version: "9.0.0".into(),
            published_at: "2026-07-14T20:00:00Z".into(),
            expires_at: "2026-07-20T20:00:00Z".into(),
            notes: "Quote \" slash \\\\ controls\n\t café 雪".into(),
            dmg_url: "https://downloads.example.test/emuchef/stable/EmuChef-9.0.0.dmg".into(),
            dmg_size_bytes: 123456,
            dmg_sha256: "a".repeat(64),
            minimum_macos_version: Some("14.0.0".into()),
            metadata_key_id: "test-metadata-2026".into(),
            metadata_signature: "0".repeat(128),
        };
        let expected =
            hex::decode(include_str!("../../tests/fixtures/update-manifest-canonical.hex").trim())
                .unwrap();
        assert_eq!(manifest.canonical_full_bytes().unwrap(), expected);
    }

    #[test]
    fn strict_parser_accepts_only_exact_canonical_full_bytes() {
        let (manifest, _) = signed_manifest("notes", None);
        let bytes = manifest.canonical_full_bytes().unwrap();
        assert_eq!(strict_parse_manifest(&bytes).unwrap().version, "9.0.0");
        let mut newline = bytes.clone();
        newline.push(b'\n');
        assert!(strict_parse_manifest(&newline).is_err());
        let spaced = String::from_utf8(bytes).unwrap().replacen(":1", ": 1", 1);
        assert!(strict_parse_manifest(spaced.as_bytes()).is_err());
    }

    #[test]
    fn lexical_contract_rejects_duplicate_escaped_and_inexact_numbers() {
        for bytes in [
            br#"{"schemaVersion":1,"schemaVersion":1}"#.as_slice(),
            br#"{"schema\u0056ersion":1}"#.as_slice(),
            br#"{"schemaVersion":1.0}"#.as_slice(),
            br#"{"schemaVersion":1e0}"#.as_slice(),
            br#"{"schemaVersion":-0}"#.as_slice(),
            br#"{"schemaVersion":01}"#.as_slice(),
            br#"{"schemaVersion":9007199254740992}"#.as_slice(),
        ] {
            assert!(validate_json_lexical_contract(bytes).is_err());
        }
        assert!(validate_json_lexical_contract(&[0xff]).is_err());
        assert!(validate_json_lexical_contract(b"\xef\xbb\xbf{}").is_err());
    }

    #[test]
    fn signature_target_time_version_and_url_are_bound() {
        let (manifest, trust) = signed_manifest("notes", None);
        let now = parse_timestamp("2026-07-15T00:00:00Z").unwrap();
        assert!(validate_manifest_policy(&manifest, &trust, now).is_ok());
        for mutate in [
            |value: &mut Manifest| value.product = "other".into(),
            |value: &mut Manifest| value.channel = "beta".into(),
            |value: &mut Manifest| value.architecture = "x86_64".into(),
            |value: &mut Manifest| value.dmg_url = "https://evil.test/file.dmg".into(),
            |value: &mut Manifest| value.notes = "tampered".into(),
        ] {
            let mut changed = manifest.clone();
            mutate(&mut changed);
            assert!(validate_manifest_policy(&changed, &trust, now).is_err());
        }
        let mut wrong_key = trust.clone();
        wrong_key.metadata_public_key = [7_u8; 32];
        assert!(validate_manifest_policy(&manifest, &wrong_key, now).is_err());
    }

    #[test]
    fn configured_dmg_prefixes_are_normalized_and_segment_bounded() {
        assert!(validate_trust(&production_trust_for_prefix("/emuchef/")).is_ok());
        for prefix in [
            "emuchef/",
            "/emuchef",
            "/emuchef/../stable/",
            "/emuchef/./stable/",
            "/emuchef/%2e%2e/",
            "/emuchef/%2Fstable/",
            "/emuchef/%5cstable/",
            "/emuchef/%252e%252e/",
            "/emuchef?channel=stable/",
            "/emuchef/#stable/",
            "/emuchef//stable/",
            "/emuchef\\stable/",
        ] {
            assert!(validate_trust(&production_trust_for_prefix(prefix)).is_err());
        }

        let trust = production_trust_for_prefix("/emuchef/");
        assert!(validate_dmg_url(
            "https://downloads.example.test/emuchef/nested/EmuChef-9.0.0.dmg",
            &trust,
        )
        .is_ok());
        for candidate in [
            "https://downloads.example.test/emuchef-evil/file.dmg",
            "https://downloads.example.test/emuchef2/file.dmg",
            "https://downloads.example.test/emuchef",
            "https://downloads.example.test/emuchef/%2e%2e/file.dmg",
            "https://downloads.example.test/emuchef/nested/../file.dmg",
            "https://downloads.example.test/emuchef%2ffile.dmg",
            "https://downloads.example.test/emuchef/%252e%252e/file.dmg",
            "https://downloads.example.test/emuchef//file.dmg",
        ] {
            assert!(validate_dmg_url(candidate, &trust).is_err());
        }
    }

    #[test]
    fn retained_candidate_is_revalidated_immediately_before_handoff() {
        let (manifest, trust) = signed_manifest("notes", None);
        let expires_at = parse_timestamp(&manifest.expires_at).unwrap();
        let candidate = ValidatedCandidate {
            version: Version::parse(&manifest.version).unwrap(),
            expires_at,
            manifest,
        };
        let service = UpdateService {
            trust: Some(trust),
            state: Mutex::new(RuntimeState {
                phase: "update_available",
                candidate: Some(candidate),
                message: None,
                check_in_progress: false,
            }),
        };
        assert!(service
            .candidate_for_open(parse_timestamp("2026-07-15T00:00:00Z").unwrap())
            .is_ok());
        assert!(service
            .candidate_for_open(parse_timestamp("2026-07-21T00:00:00Z").unwrap())
            .is_err());
    }

    #[test]
    fn response_policy_ignores_harmless_headers_and_rejects_security_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            "application/json; charset=utf-8".parse().unwrap(),
        );
        headers.insert(CONTENT_LENGTH, "2".parse().unwrap());
        headers.insert("x-request-id", "safe".parse().unwrap());
        assert_eq!(
            validate_response_policy(reqwest::StatusCode::OK, &headers).unwrap(),
            2
        );
        headers.insert(CONTENT_ENCODING, "gzip".parse().unwrap());
        assert!(validate_response_policy(reqwest::StatusCode::OK, &headers).is_err());
        headers.insert(CONTENT_ENCODING, "identity".parse().unwrap());
        assert!(validate_response_policy(reqwest::StatusCode::FOUND, &headers).is_err());
    }

    #[test]
    fn frontend_sessions_are_bounded_self_healing_and_navigation_safe() {
        let gate = ActivityGate::default();
        let session = gate.begin_frontend_session().unwrap();
        assert!(gate.reserve_navigation().is_err());
        gate.update_frontend(FrontendInteractionRequest {
            session_id: session.session_id.clone(),
            generation: 1,
            blocked: false,
        })
        .unwrap();
        gate.update_frontend(FrontendInteractionRequest {
            session_id: session.session_id.clone(),
            generation: 1,
            blocked: false,
        })
        .unwrap();
        assert!(gate
            .update_frontend(FrontendInteractionRequest {
                session_id: session.session_id.clone(),
                generation: 0,
                blocked: true,
            })
            .is_err());
        let navigation = gate.reserve_navigation().unwrap();
        assert!(gate.reserve_execution_start().is_err());
        assert!(gate.reserve_cleanup().is_err());
        assert!(gate.reserve_native_dialog().is_err());
        drop(navigation);
        let dialog = gate.reserve_native_dialog().unwrap();
        assert!(gate.reserve_navigation().is_err());
        drop(dialog);
        gate.end_frontend(&session.session_id).unwrap();
        assert!(gate.reserve_navigation().is_err());
        let replacement = gate.begin_frontend_session().unwrap();
        assert_ne!(replacement.session_id, session.session_id);
        assert!(gate
            .update_frontend(FrontendInteractionRequest {
                session_id: session.session_id,
                generation: 2,
                blocked: false,
            })
            .is_err());
    }

    fn serve_once(response: Vec<u8>) -> (String, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(&response).unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    fn fixture_trust_and_body(origin: &str) -> (ConfiguredTrust, Vec<u8>) {
        let (mut manifest, mut trust) = signed_manifest("fixture notes", None);
        manifest.dmg_url = format!("{origin}/emuchef/EmuChef-9.0.0.dmg");
        trust.manifest_url = format!("{origin}/emuchef/stable.json");
        trust.dmg_origin = format!("{origin}/");
        trust.allow_http_for_tests = true;
        // The URL is part of signed metadata, so re-sign with a fresh fixture key.
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        manifest.metadata_signature = hex::encode(
            pair.sign(&manifest.canonical_signed_bytes().unwrap())
                .as_ref(),
        );
        trust
            .metadata_public_key
            .copy_from_slice(pair.public_key().as_ref());
        (trust, manifest.canonical_full_bytes().unwrap())
    }

    #[test]
    fn fixture_transport_is_single_request_identity_only_and_direct_with_proxy_environment() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let (trust, body) = fixture_trust_and_body(&origin);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nContent-Encoding: identity\r\nX-Request-Id: harmless\r\nConnection: close\r\n\r\n",
            body.len()
        ).into_bytes();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(&response).unwrap();
            stream.write_all(&body).unwrap();
            request
        });
        let proxy_names = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy"];
        let prior = proxy_names.map(|name| (name, std::env::var_os(name)));
        for name in proxy_names {
            std::env::set_var(name, "http://127.0.0.1:1");
        }
        let result = tauri::async_runtime::block_on(fetch_and_validate(
            &trust,
            parse_timestamp("2026-07-15T00:00:00Z").unwrap(),
        ));
        for (name, value) in prior {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        assert!(result.unwrap().is_some());
        let request = String::from_utf8(server.join().unwrap())
            .unwrap()
            .to_ascii_lowercase();
        assert!(request.starts_with("get /emuchef/stable.json http/1.1"));
        assert!(request.contains("accept-encoding: identity"));
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("cookie:"));
    }

    #[test]
    fn compressed_and_malformed_framing_fail_closed() {
        for extra_headers in [
            "Content-Encoding: gzip\r\nContent-Length: 2\r\n",
            "Content-Length: 2\r\nContent-Length: 3\r\n",
            "",
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra_headers}Connection: close\r\n\r\n{{}}"
            ).into_bytes();
            let (origin, server) = serve_once(response);
            let (trust, _) = fixture_trust_and_body(&origin);
            let result = tauri::async_runtime::block_on(fetch_and_validate(
                &trust,
                parse_timestamp("2026-07-15T00:00:00Z").unwrap(),
            ));
            assert!(result.is_err());
            let _ = server.join();
        }
    }

    #[test]
    fn configured_read_timeout_rejects_a_stalled_manifest_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (trust, _) = fixture_trust_and_body(&origin);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n")
                .unwrap();
            thread::sleep(Duration::from_secs(6));
            let _ = stream.write_all(b"{}");
        });
        let started = std::time::Instant::now();
        let result = tauri::async_runtime::block_on(fetch_and_validate(
            &trust,
            parse_timestamp("2026-07-15T00:00:00Z").unwrap(),
        ));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(8));
        server.join().unwrap();
    }
}

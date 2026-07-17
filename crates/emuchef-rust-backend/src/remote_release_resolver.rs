//! Deterministic runtime resolution for supported remote release providers.

use std::io::Read;
use std::time::Duration;

use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde_json::Value;
use url::Url;

const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT_VALUE: &str = "EmuChef-Runtime/0.1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedRemoteRelease {
    pub download_url: String,
    pub asset_name: String,
    pub release_tag: String,
    pub published_at: Option<String>,
    pub size: Option<u64>,
}

pub(crate) fn resolve_github_latest(
    repository: &str,
    include_prereleases: bool,
    asset_pattern: &str,
) -> Result<ResolvedRemoteRelease, String> {
    validate_repository(repository)?;
    let matcher = Regex::new(asset_pattern)
        .map_err(|_| "remote_asset_pattern_invalid: APK filename pattern is invalid".to_string())?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            "remote_release_client_failed: Network access could not be initialized".to_string()
        })?;
    let endpoint = format!("https://api.github.com/repos/{repository}/releases?per_page=30");
    let releases = get_json_bounded(&client, &endpoint)?;
    let mut candidates = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release.get("draft").and_then(Value::as_bool) != Some(true))
        .filter(|release| {
            include_prereleases || release.get("prerelease").and_then(Value::as_bool) != Some(true)
        })
        .filter_map(|release| {
            let tag = release.get("tag_name")?.as_str()?.to_string();
            let published_at = release
                .get("published_at")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some((published_at, tag, release))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let Some((published_at, release_tag, release)) = candidates.into_iter().next() else {
        return Err("remote_release_not_found: No eligible GitHub release was found".to_string());
    };
    let matches = release
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            if !name.to_ascii_lowercase().ends_with(".apk") || !matcher.is_match(name) {
                return None;
            }
            let download_url = asset.get("browser_download_url")?.as_str()?;
            let parsed = Url::parse(download_url).ok()?;
            if parsed.scheme() != "https"
                || parsed.host_str() != Some("github.com")
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return None;
            }
            Some(ResolvedRemoteRelease {
                download_url: download_url.to_string(),
                asset_name: name.to_string(),
                release_tag: release_tag.clone(),
                published_at: published_at.clone(),
                size: asset.get("size").and_then(Value::as_u64),
            })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err(
            "remote_asset_no_match: Latest release contains no APK matching the saved pattern"
                .to_string(),
        ),
        _ => Err(
            "remote_asset_ambiguous: Latest release contains multiple APKs matching the saved pattern"
                .to_string(),
        ),
    }
}

fn validate_repository(repository: &str) -> Result<(), String> {
    let parts = repository.split('/').collect::<Vec<_>>();
    let valid = parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    if valid {
        Ok(())
    } else {
        Err("remote_repository_invalid: GitHub repository identity is invalid".to_string())
    }
}

fn get_json_bounded(client: &Client, url: &str) -> Result<Value, String> {
    let mut response = client
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|_| {
            "remote_release_request_failed: GitHub releases could not be retrieved".to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "remote_release_http_status: GitHub returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES)
    {
        return Err(
            "remote_release_response_too_large: GitHub metadata exceeded the limit".to_string(),
        );
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            "remote_release_response_failed: GitHub metadata could not be read".to_string()
        })?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(
            "remote_release_response_too_large: GitHub metadata exceeded the limit".to_string(),
        );
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        "remote_release_response_invalid: GitHub returned invalid metadata".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_validation_is_strict() {
        assert!(validate_repository("owner/project").is_ok());
        assert!(validate_repository("owner/nested/project").is_err());
        assert!(validate_repository("owner/project extra").is_err());
    }
}

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

/// Validate the supported stable GitHub APK-release policy without making a
/// provider request. Execution-start admission uses this before any execution
/// state is allocated.
pub(crate) fn validate_github_stable_release_policy(
    repository: &str,
    include_prereleases: bool,
    asset_pattern: &str,
) -> Result<(), String> {
    validate_repository(repository)?;
    if include_prereleases {
        return Err("remote_release_policy_unsupported: Stable releases only are supported".into());
    }
    if Regex::new(asset_pattern).is_err() {
        return Err("remote_asset_pattern_invalid: APK filename pattern is invalid".into());
    }
    Ok(())
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
    select_github_release_asset(&releases, include_prereleases, &matcher)
}

fn select_github_release_asset(
    releases: &Value,
    include_prereleases: bool,
    matcher: &Regex,
) -> Result<ResolvedRemoteRelease, String> {
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
    require_single_match(matches)
}

pub(crate) fn resolve_remote_latest(
    provider: &str,
    base_url: &str,
    repository: &str,
    include_prereleases: bool,
    asset_pattern: &str,
) -> Result<ResolvedRemoteRelease, String> {
    match provider {
        "github" => resolve_github_latest(repository, include_prereleases, asset_pattern),
        "gitlab" => resolve_gitlab_latest(repository, include_prereleases, asset_pattern),
        "forgejo" => {
            resolve_forgejo_latest(base_url, repository, include_prereleases, asset_pattern)
        }
        _ => Err("remote_provider_invalid: Unsupported release provider".to_string()),
    }
}

fn resolve_gitlab_latest(
    repository: &str,
    include_prereleases: bool,
    asset_pattern: &str,
) -> Result<ResolvedRemoteRelease, String> {
    validate_gitlab_repository(repository)?;
    let encoded_repository =
        url::form_urlencoded::byte_serialize(repository.as_bytes()).collect::<String>();
    let endpoint =
        format!("https://gitlab.com/api/v4/projects/{encoded_repository}/releases?per_page=30");
    let releases = get_provider_json_bounded(&endpoint, "GitLab")?;
    let matcher = compiled_asset_pattern(asset_pattern)?;
    let mut candidates = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release.get("upcoming_release").and_then(Value::as_bool) != Some(true))
        .filter_map(|release| {
            let tag = release.get("tag_name")?.as_str()?.to_string();
            let name = release
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !include_prereleases && likely_prerelease(&tag, name) {
                return None;
            }
            let published_at = release
                .get("released_at")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some((published_at, tag, release))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let Some((published_at, release_tag, release)) = candidates.into_iter().next() else {
        return Err("remote_release_not_found: No eligible GitLab release was found".to_string());
    };
    let matches = release
        .get("assets")
        .and_then(|assets| assets.get("links"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            if !name.to_ascii_lowercase().ends_with(".apk") || !matcher.is_match(name) {
                return None;
            }
            let download_url = asset
                .get("direct_asset_url")
                .or_else(|| asset.get("url"))?
                .as_str()?;
            safe_https_download(download_url)?;
            Some(ResolvedRemoteRelease {
                download_url: download_url.to_string(),
                asset_name: name.to_string(),
                release_tag: release_tag.clone(),
                published_at: published_at.clone(),
                size: None,
            })
        })
        .collect::<Vec<_>>();
    require_single_match(matches)
}

fn resolve_forgejo_latest(
    base_url: &str,
    repository: &str,
    include_prereleases: bool,
    asset_pattern: &str,
) -> Result<ResolvedRemoteRelease, String> {
    validate_repository(repository)?;
    let base = validate_forgejo_base_url(base_url)?;
    let endpoint = format!(
        "{}/api/v1/repos/{repository}/releases?limit=30",
        base.as_str().trim_end_matches('/')
    );
    let releases = get_provider_json_bounded(&endpoint, "Forgejo")?;
    let matcher = compiled_asset_pattern(asset_pattern)?;
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
                .or_else(|| release.get("created_at"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some((published_at, tag, release))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let Some((published_at, release_tag, release)) = candidates.into_iter().next() else {
        return Err("remote_release_not_found: No eligible Forgejo release was found".to_string());
    };
    let expected_host = base.host_str().unwrap_or_default();
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
            let parsed = safe_https_download(download_url)?;
            if parsed.host_str()? != expected_host {
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
    require_single_match(matches)
}

fn compiled_asset_pattern(asset_pattern: &str) -> Result<Regex, String> {
    Regex::new(asset_pattern)
        .map_err(|_| "remote_asset_pattern_invalid: APK filename pattern is invalid".to_string())
}

fn require_single_match(
    matches: Vec<ResolvedRemoteRelease>,
) -> Result<ResolvedRemoteRelease, String> {
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

fn validate_gitlab_repository(repository: &str) -> Result<(), String> {
    let parts = repository.split('/').collect::<Vec<_>>();
    let valid = (2..=20).contains(&parts.len())
        && parts.iter().all(|part| valid_repository_component(part));
    if valid {
        Ok(())
    } else {
        Err("remote_repository_invalid: GitLab repository identity is invalid".to_string())
    }
}

fn valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_forgejo_base_url(base_url: &str) -> Result<Url, String> {
    let parsed = Url::parse(base_url)
        .map_err(|_| "remote_base_url_invalid: Forgejo base URL is invalid".to_string())?;
    let valid = parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.host_str().is_some_and(|host| {
            !host.eq_ignore_ascii_case("localhost") && !host.ends_with(".localhost")
        });
    if valid {
        Ok(parsed)
    } else {
        Err("remote_base_url_invalid: Forgejo base URL is not allowed".to_string())
    }
}

fn safe_https_download(value: &str) -> Option<Url> {
    Url::parse(value).ok().filter(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some()
    })
}

fn likely_prerelease(tag: &str, name: &str) -> bool {
    let value = format!("{tag} {name}").to_ascii_lowercase();
    ["alpha", "beta", "preview", "prerelease", "pre-release"]
        .iter()
        .any(|marker| value.contains(marker))
        || value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| part == "rc")
}

fn get_provider_json_bounded(url: &str, provider_name: &str) -> Result<Value, String> {
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            "remote_release_client_failed: Network access could not be initialized".to_string()
        })?;
    let mut response = client
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/json")
        .send()
        .map_err(|_| {
            format!(
                "remote_release_request_failed: {provider_name} releases could not be retrieved"
            )
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "remote_release_http_status: {provider_name} returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES)
    {
        return Err(
            "remote_release_response_too_large: Release metadata exceeded the limit".to_string(),
        );
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            "remote_release_response_failed: Release metadata could not be read".to_string()
        })?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(
            "remote_release_response_too_large: Release metadata exceeded the limit".to_string(),
        );
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        "remote_release_response_invalid: Provider returned invalid metadata".to_string()
    })
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
    use serde_json::json;

    #[test]
    fn repository_validation_is_strict() {
        assert!(validate_repository("owner/project").is_ok());
        assert!(validate_repository("owner/nested/project").is_err());
        assert!(validate_repository("owner/project extra").is_err());
    }

    #[test]
    fn github_release_selection_rejects_zero_matching_apks() {
        let releases = json!([
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "published_at": "2026-01-01T00:00:00Z",
                "assets": [{
                    "name": "wanted-1.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v1/wanted-1.apk"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v2",
                "published_at": "2026-01-02T00:00:00Z",
                "assets": [{
                    "name": "other.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v2/other.apk"
                }]
            }
        ]);
        let matcher = Regex::new("^wanted-[0-9]+\\.apk$").unwrap();

        let error = select_github_release_asset(&releases, false, &matcher).unwrap_err();

        assert_eq!(
            error,
            "remote_asset_no_match: Latest release contains no APK matching the saved pattern"
        );
    }

    #[test]
    fn github_release_selection_rejects_multiple_matching_apks() {
        let releases = json!([{
            "draft": false,
            "prerelease": false,
            "tag_name": "v1",
            "published_at": "2026-01-01T00:00:00Z",
            "assets": [
                {
                    "name": "wanted-1.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v1/wanted-1.apk"
                },
                {
                    "name": "wanted-2.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v1/wanted-2.apk"
                }
            ]
        }]);
        let matcher = Regex::new("^wanted-[0-9]+\\.apk$").unwrap();

        let error = select_github_release_asset(&releases, false, &matcher).unwrap_err();

        assert_eq!(
            error,
            "remote_asset_ambiguous: Latest release contains multiple APKs matching the saved pattern"
        );
    }

    #[test]
    fn github_release_selection_chooses_one_matching_asset_from_latest_stable_release() {
        let releases = json!([
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "published_at": "2025-12-31T00:00:00Z",
                "assets": [{
                    "name": "wanted-1.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v1/wanted-1.apk",
                    "size": 12
                }]
            },
            {
                "draft": false,
                "prerelease": true,
                "tag_name": "v2-rc1",
                "published_at": "2026-02-01T00:00:00Z",
                "assets": [{
                    "name": "wanted-2.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v2/wanted-2.apk"
                }]
            },
            {
                "draft": true,
                "prerelease": false,
                "tag_name": "v3",
                "published_at": "2026-03-01T00:00:00Z",
                "assets": [{
                    "name": "wanted-3.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v3/wanted-3.apk"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v4",
                "published_at": "2026-01-31T00:00:00Z",
                "assets": [{
                    "name": "wanted-4.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v4/wanted-4.apk",
                    "size": 34
                }]
            }
        ]);
        let matcher = Regex::new("^wanted-[0-9]+\\.apk$").unwrap();

        let selected = select_github_release_asset(&releases, false, &matcher).unwrap();

        assert_eq!(selected.asset_name, "wanted-4.apk");
        assert_eq!(selected.release_tag, "v4");
        assert_eq!(selected.size, Some(34));
        assert_eq!(
            selected.download_url,
            "https://github.com/example/app/releases/download/v4/wanted-4.apk"
        );
    }
}

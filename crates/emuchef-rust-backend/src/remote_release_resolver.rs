//! Deterministic runtime resolution for supported remote release providers.

use std::io::Read;
use std::time::Duration;

use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, ACCEPT, LINK, USER_AGENT};
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use url::Url;

use crate::artifact_transport::{HttpArtifactTransport, MetadataResponse};

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
    pub created_at: Option<String>,
    pub timestamp_source: &'static str,
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
            let published_at = release.get("published_at").and_then(Value::as_str)?;
            let published_at_time = OffsetDateTime::parse(published_at, &Rfc3339).ok()?;
            Some((published_at_time, published_at.to_string(), tag, release))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.2.cmp(&left.2)));
    let Some((_, published_at, release_tag, release)) = candidates.into_iter().next() else {
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
                published_at: Some(published_at.clone()),
                created_at: None,
                timestamp_source: "published_at",
                size: asset.get("size").and_then(Value::as_u64),
            })
        })
        .collect::<Vec<_>>();
    require_single_match(matches)
}

pub(crate) fn resolve_remote_latest(
    transport: &HttpArtifactTransport,
    provider: &str,
    base_url: &str,
    repository: &str,
    artifact_kind: &str,
    include_prereleases: bool,
    asset_pattern: Option<&str>,
    invert_asset_pattern: bool,
) -> Result<ResolvedRemoteRelease, String> {
    validate_remote_release_policy(
        provider,
        base_url,
        repository,
        artifact_kind,
        asset_pattern,
        invert_asset_pattern,
    )?;
    let provider = ReleaseProvider::parse(provider)?;
    let endpoint = ProviderEndpoint::new(provider, base_url, repository)?;
    let matcher = asset_pattern.map(compiled_asset_pattern).transpose()?;
    resolve_release_pages(
        provider,
        &endpoint,
        artifact_kind,
        include_prereleases,
        matcher.as_ref(),
        invert_asset_pattern,
        |url| fetch_provider_page(transport, url),
    )
}

#[cfg(test)]
pub(crate) fn resolve_remote_latest_from_fixture(
    provider: &str,
    base_url: &str,
    repository: &str,
    artifact_kind: &str,
    include_prereleases: bool,
    asset_pattern: Option<&str>,
    invert_asset_pattern: bool,
    releases: &Value,
) -> Result<ResolvedRemoteRelease, String> {
    validate_remote_release_policy(
        provider,
        base_url,
        repository,
        artifact_kind,
        asset_pattern,
        invert_asset_pattern,
    )?;
    let provider = ReleaseProvider::parse(provider)?;
    let endpoint = ProviderEndpoint::new(provider, base_url, repository)?;
    let matcher = asset_pattern.map(compiled_asset_pattern).transpose()?;
    let releases = releases.as_array().cloned().ok_or_else(|| {
        "remote_release_response_invalid: Provider release metadata was not a list".to_string()
    })?;
    let first_url = endpoint.page_url(1)?;
    resolve_release_pages(
        provider,
        &endpoint,
        artifact_kind,
        include_prereleases,
        matcher.as_ref(),
        invert_asset_pattern,
        move |url| {
            if url != &first_url {
                return Err(
                    "remote_release_fixture_missing: No fixture remains for the requested page"
                        .to_string(),
                );
            }
            Ok(ReleasePage {
                releases: releases.clone(),
                headers: HeaderMap::new(),
                final_url: url.clone(),
            })
        },
    )
}

/// Validate a self-contained latest-release policy before execution begins.
pub(crate) fn validate_remote_release_policy(
    provider: &str,
    service_origin: &str,
    repository: &str,
    artifact_kind: &str,
    asset_pattern: Option<&str>,
    invert_asset_pattern: bool,
) -> Result<(), String> {
    if !matches!(provider, "github" | "gitlab" | "forgejo") {
        return Err("remote_provider_invalid: Unsupported release provider".to_string());
    }
    if !crate::authored_models::is_service_origin(service_origin) {
        return Err("remote_base_url_invalid: Release service origin is invalid".to_string());
    }
    if !matches!(artifact_kind, "apk" | "file") {
        return Err("app_artifact_kind_invalid: Release artifact kind is invalid".to_string());
    }
    if invert_asset_pattern && asset_pattern.is_none() {
        return Err(
            "remote_asset_pattern_invalid: Pattern inversion requires a filename pattern"
                .to_string(),
        );
    }
    if let Some(pattern) = asset_pattern {
        compiled_asset_pattern(pattern)?;
    }
    match provider {
        "gitlab" => validate_gitlab_repository(repository),
        "github" | "forgejo" => validate_repository(repository),
        _ => unreachable!("provider names were checked above"),
    }
}

const PAGE_SIZE: usize = 30;
const MAX_RELEASE_PAGES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseProvider {
    Github,
    Gitlab,
    Forgejo,
}

impl ReleaseProvider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            "forgejo" => Ok(Self::Forgejo),
            _ => Err("remote_provider_invalid: Unsupported release provider".to_string()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Forgejo => "forgejo",
        }
    }
}

struct ProviderEndpoint {
    api_origin: Url,
    path: String,
    fixed_query: Vec<(&'static str, String)>,
}

impl ProviderEndpoint {
    fn new(
        provider: ReleaseProvider,
        service_origin: &str,
        repository: &str,
    ) -> Result<Self, String> {
        let service = Url::parse(service_origin).map_err(|_| {
            "remote_base_url_invalid: Release service origin is invalid".to_string()
        })?;
        let (api_origin, path, fixed_query) = match provider {
            ReleaseProvider::Github => {
                let is_github_com = service.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("github.com")
                        && service.port_or_known_default() == Some(443)
                });
                let api_origin = if is_github_com {
                    Url::parse("https://api.github.com").map_err(|_| {
                        "remote_base_url_invalid: GitHub API origin is invalid".to_string()
                    })?
                } else {
                    service
                };
                (
                    api_origin,
                    if is_github_com {
                        format!("/repos/{repository}/releases")
                    } else {
                        format!("/api/v3/repos/{repository}/releases")
                    },
                    vec![("per_page", PAGE_SIZE.to_string())],
                )
            }
            ReleaseProvider::Gitlab => {
                let encoded =
                    url::form_urlencoded::byte_serialize(repository.as_bytes()).collect::<String>();
                (
                    service,
                    format!("/api/v4/projects/{encoded}/releases"),
                    vec![
                        ("per_page", PAGE_SIZE.to_string()),
                        ("order_by", "released_at".to_string()),
                        ("sort", "desc".to_string()),
                    ],
                )
            }
            ReleaseProvider::Forgejo => (
                service,
                format!("/api/v1/repos/{repository}/releases"),
                vec![("limit", PAGE_SIZE.to_string())],
            ),
        };
        Ok(Self {
            api_origin,
            path,
            fixed_query,
        })
    }

    fn page_url(&self, page: usize) -> Result<Url, String> {
        let mut url = self.api_origin.join(&self.path).map_err(|_| {
            "remote_release_endpoint_invalid: Provider API endpoint is invalid".to_string()
        })?;
        {
            let mut query = url.query_pairs_mut();
            for (key, value) in &self.fixed_query {
                query.append_pair(key, value);
            }
            query.append_pair("page", &page.to_string());
        }
        Ok(url)
    }

    fn validate_page_url(&self, url: &Url, expected_page: usize) -> Result<(), String> {
        let page = self.validate_pagination_url(url)?;
        if page != expected_page {
            return Err(
                "remote_release_endpoint_invalid: Pagination parameters are invalid".to_string(),
            );
        }
        Ok(())
    }

    fn validate_pagination_url(&self, url: &Url) -> Result<usize, String> {
        if !same_origin(&self.api_origin, url)
            || url.path() != self.path
            || url.username() != ""
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "remote_release_endpoint_invalid: Pagination endpoint is outside the provider API"
                    .to_string(),
            );
        }
        let mut query = std::collections::BTreeMap::new();
        for (key, value) in url.query_pairs() {
            if query.insert(key.to_string(), value.to_string()).is_some() {
                return Err(
                    "remote_release_endpoint_invalid: Pagination parameters are ambiguous"
                        .to_string(),
                );
            }
        }
        if query.len() != self.fixed_query.len() + 1
            || self
                .fixed_query
                .iter()
                .any(|(key, value)| query.get(*key).is_none_or(|actual| actual != value))
        {
            return Err(
                "remote_release_endpoint_invalid: Pagination parameters are invalid".to_string(),
            );
        }
        query
            .get("page")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|page| *page > 0)
            .ok_or_else(|| {
                "remote_release_endpoint_invalid: Pagination page is invalid".to_string()
            })
    }
}

#[derive(Clone)]
struct ReleasePage {
    releases: Vec<Value>,
    headers: HeaderMap,
    final_url: Url,
}

fn fetch_provider_page(
    transport: &HttpArtifactTransport,
    url: &Url,
) -> Result<ReleasePage, String> {
    let response = transport
        .get_public_metadata(url, MAX_METADATA_BYTES)
        .map_err(|error| match error.code() {
            "artifact_response_too_large" => {
                "remote_release_response_too_large: Provider metadata exceeded the limit".to_string()
            }
            "artifact_http_status" => {
                "remote_release_http_status: Provider returned an unsuccessful response".to_string()
            }
            "artifact_redirect_policy_rejected" | "artifact_tls_verification_failed" => {
                "remote_release_transport_rejected: Provider metadata did not satisfy public HTTPS policy".to_string()
            }
            _ => "remote_release_request_failed: Provider metadata could not be retrieved".to_string(),
        })?;
    parse_release_page(response)
}

fn parse_release_page(response: MetadataResponse) -> Result<ReleasePage, String> {
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| {
        "remote_release_response_invalid: Provider returned invalid release metadata".to_string()
    })?;
    let releases = value.as_array().cloned().ok_or_else(|| {
        "remote_release_response_invalid: Provider release metadata was not a list".to_string()
    })?;
    Ok(ReleasePage {
        releases,
        headers: response.headers,
        final_url: response.final_url,
    })
}

fn resolve_release_pages<F>(
    provider: ReleaseProvider,
    endpoint: &ProviderEndpoint,
    artifact_kind: &str,
    include_prereleases: bool,
    matcher: Option<&Regex>,
    invert_asset_pattern: bool,
    mut fetch_page: F,
) -> Result<ResolvedRemoteRelease, String>
where
    F: FnMut(&Url) -> Result<ReleasePage, String>,
{
    let mut page_number = 1usize;
    let mut page_url = endpoint.page_url(page_number)?;
    let mut best: Option<ReleaseCandidate> = None;
    let mut previous_gitlab_timestamp = None;

    loop {
        let page = fetch_page(&page_url)?;
        endpoint.validate_page_url(&page.final_url, page_number)?;
        let mut page_oldest = None;
        for release in &page.releases {
            if provider == ReleaseProvider::Gitlab {
                if let Some(timestamp) = gitlab_order_timestamp(release)? {
                    if previous_gitlab_timestamp.is_some_and(|previous| timestamp > previous) {
                        return Err("remote_release_order_ambiguous: GitLab releases were not ordered by released_at".to_string());
                    }
                    previous_gitlab_timestamp = Some(timestamp);
                    page_oldest = Some(
                        page_oldest
                            .map_or(timestamp, |oldest: OffsetDateTime| oldest.min(timestamp)),
                    );
                }
            }
            if let Some(candidate) = release_candidate(provider, release, include_prereleases)? {
                if best
                    .as_ref()
                    .is_none_or(|current| candidate_is_newer(&candidate, current))
                {
                    best = Some(candidate);
                }
            }
        }
        let latest_proven = provider == ReleaseProvider::Gitlab
            && best.as_ref().is_some_and(|candidate| {
                page_oldest.is_some_and(|oldest| candidate.timestamp > oldest)
            });
        let next = next_page_url(provider, &page, endpoint, page_number, latest_proven)?;
        if latest_proven || next.is_none() {
            break;
        }
        if page_number == MAX_RELEASE_PAGES {
            return Err("remote_release_history_incomplete: Release history exceeded the safe discovery bound".to_string());
        }
        page_number += 1;
        page_url = next.expect("checked that the next page exists");
    }

    let candidate = best.ok_or_else(|| {
        format!(
            "remote_release_not_found: No eligible {} release was found",
            provider.as_str()
        )
    })?;
    select_app_artifact_asset(
        provider,
        candidate,
        artifact_kind,
        matcher,
        invert_asset_pattern,
    )
}

#[derive(Clone)]
struct ReleaseCandidate {
    timestamp: OffsetDateTime,
    tie_tag: String,
    tie_key: String,
    release: Value,
    published_at: Option<String>,
    created_at: Option<String>,
    timestamp_source: &'static str,
}

fn release_candidate(
    provider: ReleaseProvider,
    release: &Value,
    include_prereleases: bool,
) -> Result<Option<ReleaseCandidate>, String> {
    match provider {
        ReleaseProvider::Github => {
            if release.get("draft").and_then(Value::as_bool) == Some(true)
                || (!include_prereleases
                    && release.get("prerelease").and_then(Value::as_bool) == Some(true))
            {
                return Ok(None);
            }
            let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
                return Ok(None);
            };
            let Some(published_at) = release.get("published_at").and_then(Value::as_str) else {
                return Ok(None);
            };
            let Ok(timestamp) = OffsetDateTime::parse(published_at, &Rfc3339) else {
                return Ok(None);
            };
            Ok(Some(make_candidate(
                release,
                timestamp,
                tag,
                Some(published_at.to_string()),
                None,
                "published_at",
            )))
        }
        ReleaseProvider::Gitlab => {
            if release.get("upcoming_release").and_then(Value::as_bool) == Some(true) {
                return Ok(None);
            }
            let Some(timestamp) = gitlab_order_timestamp(release)? else {
                return Ok(None);
            };
            let tag = required_release_tag(release, provider)?;
            let name = release
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !include_prereleases && likely_prerelease(tag, name) {
                return Ok(None);
            }
            let released_at = release
                .get("released_at")
                .and_then(Value::as_str)
                .expect("GitLab ordering validation requires released_at");
            Ok(Some(make_candidate(
                release,
                timestamp,
                tag,
                Some(released_at.to_string()),
                None,
                "released_at",
            )))
        }
        ReleaseProvider::Forgejo => {
            if release.get("draft").and_then(Value::as_bool) == Some(true)
                || (!include_prereleases
                    && release.get("prerelease").and_then(Value::as_bool) == Some(true))
            {
                return Ok(None);
            }
            let tag = required_release_tag(release, provider)?;
            let published_at = release.get("published_at").and_then(Value::as_str);
            let parsed_published =
                published_at.and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok());
            let created_at = release.get("created_at").and_then(Value::as_str);
            let parsed_created =
                created_at.and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok());
            let (timestamp, timestamp_source) = parsed_published
                .map(|timestamp| (timestamp, "published_at"))
                .or_else(|| parsed_created.map(|timestamp| (timestamp, "created_at")))
                .ok_or_else(|| {
                    "remote_release_timestamp_invalid: Forgejo release has no usable publication timestamp".to_string()
                })?;
            Ok(Some(make_candidate(
                release,
                timestamp,
                tag,
                parsed_published.map(|_| published_at.unwrap().to_string()),
                (timestamp_source == "created_at").then(|| created_at.unwrap().to_string()),
                timestamp_source,
            )))
        }
    }
}

fn gitlab_order_timestamp(release: &Value) -> Result<Option<OffsetDateTime>, String> {
    let Some(value) = release.get("released_at").and_then(Value::as_str) else {
        if release.get("upcoming_release").and_then(Value::as_bool) == Some(true) {
            return Ok(None);
        }
        return Err(
            "remote_release_timestamp_invalid: GitLab release has no released_at timestamp"
                .to_string(),
        );
    };
    OffsetDateTime::parse(value, &Rfc3339)
        .map(Some)
        .map_err(|_| {
            "remote_release_timestamp_invalid: GitLab released_at timestamp is invalid".to_string()
        })
}

fn required_release_tag<'a>(
    release: &'a Value,
    provider: ReleaseProvider,
) -> Result<&'a str, String> {
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| {
            format!(
                "remote_release_metadata_invalid: {} release has no tag name",
                provider.as_str()
            )
        })
}

fn make_candidate(
    release: &Value,
    timestamp: OffsetDateTime,
    tag: &str,
    published_at: Option<String>,
    created_at: Option<String>,
    timestamp_source: &'static str,
) -> ReleaseCandidate {
    ReleaseCandidate {
        timestamp,
        tie_tag: tag.to_string(),
        tie_key: release
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| format!("id:{id:020}"))
            .unwrap_or_else(|| serde_json::to_string(release).unwrap_or_default()),
        release: release.clone(),
        published_at,
        created_at,
        timestamp_source,
    }
}

fn candidate_is_newer(left: &ReleaseCandidate, right: &ReleaseCandidate) -> bool {
    left.timestamp > right.timestamp
        || (left.timestamp == right.timestamp
            && (left.tie_tag > right.tie_tag
                || (left.tie_tag == right.tie_tag && left.tie_key > right.tie_key)))
}

fn select_app_artifact_asset(
    provider: ReleaseProvider,
    candidate: ReleaseCandidate,
    artifact_kind: &str,
    matcher: Option<&Regex>,
    invert_asset_pattern: bool,
) -> Result<ResolvedRemoteRelease, String> {
    let assets = match provider {
        ReleaseProvider::Gitlab => candidate
            .release
            .get("assets")
            .and_then(|assets| assets.get("links")),
        ReleaseProvider::Github | ReleaseProvider::Forgejo => candidate.release.get("assets"),
    };
    let mut candidates = Vec::new();
    let mut invalid_matching_filename = false;
    for asset in assets.and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = asset.get("name").and_then(Value::as_str) else {
            // Without a filename, kind and pattern admission cannot be
            // determined. Treat the record as a possible match so it cannot
            // disappear beside a valid asset and alter exact-one selection.
            invalid_matching_filename = true;
            continue;
        };
        if artifact_kind == "apk" && !name.to_ascii_lowercase().ends_with(".apk") {
            continue;
        }
        let pattern_matches = matcher.is_none_or(|matcher| matcher.is_match(name));
        if pattern_matches == invert_asset_pattern {
            continue;
        }
        if name.trim().is_empty() {
            invalid_matching_filename = true;
            continue;
        }
        candidates.push((asset, name));
    }
    if invalid_matching_filename {
        return Err("remote_asset_invalid: Latest eligible release contains a matching asset with an invalid filename".to_string());
    }
    if candidates.is_empty() {
        return Err("remote_asset_no_match: Latest eligible release contains no asset matching the App Artifact policy".to_string());
    }

    let mut matches = Vec::with_capacity(candidates.len());
    let mut invalid_matching_metadata = false;
    for (asset, name) in candidates {
        let download_url = match provider {
            ReleaseProvider::Gitlab => asset
                .get("direct_asset_url")
                .or_else(|| asset.get("url"))
                .and_then(Value::as_str),
            ReleaseProvider::Github | ReleaseProvider::Forgejo => {
                asset.get("browser_download_url").and_then(Value::as_str)
            }
        };
        let Some(download_url) = download_url else {
            invalid_matching_metadata = true;
            continue;
        };
        if crate::authored_models::parse_public_https_url(download_url).is_none() {
            invalid_matching_metadata = true;
            continue;
        }
        matches.push(ResolvedRemoteRelease {
            download_url: download_url.to_string(),
            asset_name: name.to_string(),
            release_tag: candidate.tie_tag.clone(),
            published_at: candidate.published_at.clone(),
            created_at: candidate.created_at.clone(),
            timestamp_source: candidate.timestamp_source,
            size: asset.get("size").and_then(Value::as_u64),
        });
    }
    if invalid_matching_metadata {
        return Err("remote_asset_invalid: Latest eligible release contains a matching asset with invalid or unsafe download metadata".to_string());
    }
    match matches.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err("remote_asset_no_match: Latest eligible release contains no asset matching the App Artifact policy".to_string()),
        _ => Err("remote_asset_ambiguous: Latest eligible release contains multiple assets matching the App Artifact policy".to_string()),
    }
}

fn next_page_url(
    provider: ReleaseProvider,
    page: &ReleasePage,
    endpoint: &ProviderEndpoint,
    current_page: usize,
    latest_proven: bool,
) -> Result<Option<Url>, String> {
    let link_present = page.headers.get_all(LINK).iter().next().is_some();
    let link_next = if link_present {
        parse_next_link(&page.headers, &page.final_url, endpoint)?
    } else {
        None
    };
    let next = match provider {
        ReleaseProvider::Github => link_next,
        ReleaseProvider::Gitlab => {
            let header_present = page.headers.contains_key("x-next-page");
            let header_next = gitlab_next_page_header(&page.headers, endpoint)?;
            let total_has_next = gitlab_total_has_next(&page.headers, current_page)?;
            if link_present {
                let selected = match (link_next, header_next) {
                    (Some(link), Some(header)) => {
                        if endpoint.validate_pagination_url(&link)?
                            != endpoint.validate_pagination_url(&header)?
                        {
                            return Err("remote_release_history_ambiguous: GitLab pagination markers conflict".to_string());
                        }
                        Some(link)
                    }
                    (Some(_), None) if header_present => {
                        return Err(
                            "remote_release_history_ambiguous: GitLab pagination markers conflict"
                                .to_string(),
                        );
                    }
                    (None, Some(_)) => {
                        return Err(
                            "remote_release_history_ambiguous: GitLab pagination markers conflict"
                                .to_string(),
                        );
                    }
                    (link, _) => link,
                };
                if total_has_next.is_some_and(|has_next| has_next != selected.is_some()) {
                    return Err(
                        "remote_release_history_ambiguous: GitLab pagination markers conflict"
                            .to_string(),
                    );
                }
                selected
            } else if header_present {
                if total_has_next.is_some_and(|has_next| has_next != header_next.is_some()) {
                    return Err(
                        "remote_release_history_ambiguous: GitLab pagination markers conflict"
                            .to_string(),
                    );
                }
                header_next
            } else if let Some(has_next) = total_has_next {
                if has_next {
                    Some(endpoint.page_url(current_page + 1)?)
                } else {
                    None
                }
            } else if page.releases.len() < PAGE_SIZE {
                None
            } else if latest_proven {
                None
            } else {
                return Err("remote_release_history_ambiguous: GitLab did not provide a conclusive pagination marker".to_string());
            }
        }
        ReleaseProvider::Forgejo => {
            let total_has_next = page
                .headers
                .get("x-total-count")
                .map(|value| {
                    value
                        .to_str()
                        .ok()
                        .and_then(|value| value.parse::<usize>().ok())
                        .map(|total| total > current_page.saturating_mul(PAGE_SIZE))
                        .ok_or_else(|| {
                            "remote_release_endpoint_invalid: Forgejo total-count header is invalid"
                                .to_string()
                        })
                })
                .transpose()?;
            if link_present {
                match (link_next, total_has_next) {
                    (Some(link), Some(true)) => Some(link),
                    (Some(_), Some(false)) | (None, Some(true)) => {
                        return Err(
                            "remote_release_history_ambiguous: Forgejo pagination markers conflict"
                                .to_string(),
                        );
                    }
                    (link, _) => link,
                }
            } else if let Some(has_next) = total_has_next {
                if has_next {
                    if page.releases.len() != PAGE_SIZE {
                        return Err("remote_release_history_ambiguous: Forgejo result count conflicts with its pagination total".to_string());
                    }
                    Some(endpoint.page_url(current_page + 1)?)
                } else {
                    None
                }
            } else if page.releases.len() < PAGE_SIZE {
                None
            } else {
                return Err("remote_release_history_ambiguous: Forgejo did not provide a conclusive pagination marker".to_string());
            }
        }
    };
    if let Some(next) = &next {
        endpoint.validate_page_url(next, current_page + 1)?;
    }
    Ok(next)
}

fn gitlab_next_page_header(
    headers: &HeaderMap,
    endpoint: &ProviderEndpoint,
) -> Result<Option<Url>, String> {
    let Some(value) = headers.get("x-next-page") else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| {
        "remote_release_endpoint_invalid: GitLab pagination header is invalid".to_string()
    })?;
    if value.trim().is_empty() {
        return Ok(None);
    }
    let page_number = value
        .parse::<usize>()
        .map_err(|_| "remote_release_endpoint_invalid: GitLab next page is invalid".to_string())?;
    if page_number == 0 {
        return Err("remote_release_endpoint_invalid: GitLab next page is invalid".to_string());
    }
    Ok(Some(endpoint.page_url(page_number)?))
}

fn gitlab_total_has_next(headers: &HeaderMap, current_page: usize) -> Result<Option<bool>, String> {
    let parse_usize_header = |name: &'static str| -> Result<Option<usize>, String> {
        headers
            .get(name)
            .map(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or_else(|| {
                        "remote_release_endpoint_invalid: GitLab pagination header is invalid"
                            .to_string()
                    })
            })
            .transpose()
    };
    if parse_usize_header("x-page")?.is_some_and(|page| page != current_page)
        || parse_usize_header("x-per-page")?.is_some_and(|per_page| per_page != PAGE_SIZE)
    {
        return Err(
            "remote_release_endpoint_invalid: GitLab pagination headers do not match the request"
                .to_string(),
        );
    }
    let by_pages =
        parse_usize_header("x-total-pages")?.map(|total_pages| current_page < total_pages);
    let by_count =
        parse_usize_header("x-total")?.map(|total| total > current_page.saturating_mul(PAGE_SIZE));
    if by_pages
        .is_some_and(|has_next| by_count.is_some_and(|count_has_next| has_next != count_has_next))
    {
        return Err(
            "remote_release_history_ambiguous: GitLab pagination totals conflict".to_string(),
        );
    }
    Ok(by_pages.or(by_count))
}

fn parse_next_link(
    headers: &HeaderMap,
    current_url: &Url,
    endpoint: &ProviderEndpoint,
) -> Result<Option<Url>, String> {
    let mut next = None;
    for header in headers.get_all(LINK).iter() {
        let value = header.to_str().map_err(|_| {
            "remote_release_endpoint_invalid: Provider pagination link is invalid".to_string()
        })?;
        for item in value.split(',') {
            let (target, parameters) = item.trim().split_once(';').ok_or_else(|| {
                "remote_release_endpoint_invalid: Provider pagination link is malformed".to_string()
            })?;
            let is_next = parameters.split(';').any(|parameter| {
                parameter.trim().strip_prefix("rel=").is_some_and(|value| {
                    value
                        .trim_matches('"')
                        .split_ascii_whitespace()
                        .any(|relation| relation == "next")
                })
            });
            let target = target
                .trim()
                .strip_prefix('<')
                .and_then(|target| target.strip_suffix('>'))
                .ok_or_else(|| {
                    "remote_release_endpoint_invalid: Provider next link is malformed".to_string()
                })?;
            let parsed = current_url.join(target).map_err(|_| {
                "remote_release_endpoint_invalid: Provider next link is malformed".to_string()
            })?;
            endpoint.validate_pagination_url(&parsed)?;
            if !is_next {
                continue;
            }
            if next.replace(parsed).is_some() {
                return Err(
                    "remote_release_endpoint_invalid: Provider returned multiple next links"
                        .to_string(),
                );
            }
        }
    }
    Ok(next)
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str().is_some_and(|host| {
            right
                .host_str()
                .is_some_and(|other| host.eq_ignore_ascii_case(other))
        })
        && left.port_or_known_default() == right.port_or_known_default()
}

fn compiled_asset_pattern(asset_pattern: &str) -> Result<Regex, String> {
    Regex::new(asset_pattern)
        .map_err(|_| "remote_asset_pattern_invalid: Asset filename pattern is invalid".to_string())
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

fn likely_prerelease(tag: &str, name: &str) -> bool {
    let value = format!("{tag} {name}").to_ascii_lowercase();
    ["alpha", "beta", "preview", "prerelease", "pre-release"]
        .iter()
        .any(|marker| value.contains(marker))
        || value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| part == "rc")
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

    fn endpoint(provider: ReleaseProvider, origin: &str, repository: &str) -> ProviderEndpoint {
        ProviderEndpoint::new(provider, origin, repository).unwrap()
    }

    fn page(
        endpoint: &ProviderEndpoint,
        page_number: usize,
        releases: Value,
        next_page: Option<usize>,
    ) -> ReleasePage {
        let mut headers = HeaderMap::new();
        if let Some(next_page) = next_page {
            let next = endpoint.page_url(next_page).unwrap();
            headers.insert(LINK, format!("<{next}>; rel=\"next\"").parse().unwrap());
        }
        ReleasePage {
            releases: releases.as_array().unwrap().clone(),
            headers,
            final_url: endpoint.page_url(page_number).unwrap(),
        }
    }

    fn github_release(tag: &str, published_at: &str, asset_name: &str) -> Value {
        json!({
            "draft": false,
            "prerelease": false,
            "tag_name": tag,
            "published_at": published_at,
            "assets": [{
                "name": asset_name,
                "browser_download_url": format!("https://github.com/example/app/releases/download/{tag}/{asset_name}")
            }]
        })
    }

    #[test]
    fn repository_validation_is_strict() {
        assert!(validate_repository("owner/project").is_ok());
        assert!(validate_repository("owner/nested/project").is_err());
        assert!(validate_repository("owner/project extra").is_err());
    }

    #[test]
    fn provider_endpoints_derive_public_and_self_hosted_api_paths() {
        let github = endpoint(ReleaseProvider::Github, "https://github.com", "owner/repo");
        let github_url = github.page_url(1).unwrap();
        assert_eq!(github_url.host_str(), Some("api.github.com"));
        assert_eq!(github_url.path(), "/repos/owner/repo/releases");

        let enterprise = endpoint(
            ReleaseProvider::Github,
            "https://git.example.net",
            "owner/repo",
        );
        let enterprise_url = enterprise.page_url(1).unwrap();
        assert_eq!(enterprise_url.host_str(), Some("git.example.net"));
        assert_eq!(enterprise_url.path(), "/api/v3/repos/owner/repo/releases");

        let gitlab = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/subgroup/project",
        );
        let gitlab_url = gitlab.page_url(1).unwrap();
        assert_eq!(
            gitlab_url.path(),
            "/api/v4/projects/group%2Fsubgroup%2Fproject/releases"
        );
        assert_eq!(
            gitlab_url
                .query_pairs()
                .find(|(key, _)| key == "order_by")
                .unwrap()
                .1,
            "released_at"
        );

        let forgejo = endpoint(
            ReleaseProvider::Forgejo,
            "https://forge.example.net",
            "owner/repo",
        );
        assert_eq!(
            forgejo.page_url(1).unwrap().path(),
            "/api/v1/repos/owner/repo/releases"
        );
    }

    #[test]
    fn release_policy_accepts_optional_patterns_and_rejects_unbound_inversion() {
        assert!(validate_remote_release_policy(
            "forgejo",
            "https://forge.example.net",
            "owner/repo",
            "file",
            None,
            false,
        )
        .is_ok());
        assert_eq!(
            validate_remote_release_policy(
                "github",
                "https://github.com",
                "owner/repo",
                "apk",
                None,
                true,
            )
            .unwrap_err(),
            "remote_asset_pattern_invalid: Pattern inversion requires a filename pattern"
        );
        assert!(validate_remote_release_policy(
            "github",
            "https://github.com/api/v3",
            "owner/repo",
            "apk",
            None,
            false,
        )
        .is_err());
    }

    #[test]
    fn gitlab_stops_after_the_latest_eligible_release_is_proven() {
        let endpoint = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let mut requests = Vec::new();
        let selected = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |url| {
                requests.push(url.clone());
                Ok(page(
                    &endpoint,
                    1,
                    json!([
                        {
                            "tag_name": "v2",
                            "name": "Release 2",
                            "released_at": "2026-10-01T00:00:00Z",
                            "assets": {"links": [{"name":"app-v2.apk", "direct_asset_url":"https://gitlab.example.net/group/project/-/releases/v2/app-v2.apk"}]}
                        },
                        {
                            "tag_name": "v1",
                            "name": "Release 1",
                            "released_at": "2026-09-01T00:00:00Z",
                            "assets": {"links": [{"name":"app-v1.apk", "direct_asset_url":"https://gitlab.example.net/group/project/-/releases/v1/app-v1.apk"}]}
                        }
                    ]),
                    Some(2),
                ))
            },
        )
        .unwrap();

        assert_eq!(requests.len(), 1);
        assert_eq!(selected.release_tag, "v2");
        assert_eq!(selected.asset_name, "app-v2.apk");
    }

    #[test]
    fn gitlab_continues_across_timestamp_ties_and_uses_parsed_timestamps() {
        let endpoint = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let page_one = json!([
            {
                "tag_name":"v2", "name":"Release 2", "released_at":"2026-01-01T00:00:00Z",
                "assets":{"links":[{"name":"v2.apk","url":"https://gitlab.example.net/v2.apk"}]}
            },
            {
                "tag_name":"v1", "name":"Release 1", "released_at":"2026-01-01T00:00:00Z",
                "assets":{"links":[{"name":"v1.apk","url":"https://gitlab.example.net/v1.apk"}]}
            }
        ]);
        let page_two = json!([
            {
                "tag_name":"v3", "name":"Release 3", "released_at":"2026-01-01T00:00:00Z",
                "assets":{"links":[{"name":"v3.apk","url":"https://gitlab.example.net/v3.apk"}]}
            },
            {
                "tag_name":"v0", "name":"Release 0", "released_at":"2025-12-31T00:00:00Z",
                "assets":{"links":[{"name":"v0.apk","url":"https://gitlab.example.net/v0.apk"}]}
            }
        ]);
        let selected = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |url| {
                if url
                    .query_pairs()
                    .any(|(key, value)| key == "page" && value == "1")
                {
                    Ok(page(&endpoint, 1, page_one.clone(), Some(2)))
                } else {
                    Ok(page(&endpoint, 2, page_two.clone(), None))
                }
            },
        )
        .unwrap();
        assert_eq!(selected.release_tag, "v3");

        let timestamp_order = json!([
            {
                "tag_name":"chronologically-newer", "name":"Stable",
                "released_at":"2026-01-31T23:00:00Z",
                "assets":{"links":[{"name":"new.apk","url":"https://gitlab.example.net/new.apk"}]}
            },
            {
                "tag_name":"lexically-newer", "name":"Stable",
                "released_at":"2026-02-01T00:00:00+02:00",
                "assets":{"links":[{"name":"old.apk","url":"https://gitlab.example.net/old.apk"}]}
            }
        ]);
        let selected = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_url| Ok(page(&endpoint, 1, timestamp_order.clone(), None)),
        )
        .unwrap();
        assert_eq!(selected.release_tag, "chronologically-newer");
    }

    #[test]
    fn gitlab_total_pages_proves_exhaustion_for_a_full_last_page() {
        let endpoint = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let releases = (0..PAGE_SIZE)
            .map(|index| {
                let day = 31 - index;
                json!({
                    "tag_name": format!("v{index}"),
                    "name": "Stable",
                    "released_at": format!("2026-01-{day:02}T00:00:00Z"),
                    "assets": {"links": [{
                        "name": "app.apk",
                        "url": format!("https://gitlab.example.net/{index}.apk")
                    }]}
                })
            })
            .collect::<Vec<_>>();
        let mut final_page = page(&endpoint, 1, Value::Array(releases), None);
        final_page
            .headers
            .insert("x-total-pages", "1".parse().unwrap());

        let selected = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_| Ok(final_page.clone()),
        )
        .unwrap();
        assert_eq!(selected.release_tag, "v0");
    }

    #[test]
    fn gitlab_stops_after_a_full_descending_page_proves_the_latest_release_without_headers() {
        let endpoint = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let releases = (0..PAGE_SIZE)
            .map(|index| {
                let day = 31 - index;
                json!({
                    "tag_name": format!("v{index}"),
                    "name": "Stable",
                    "released_at": format!("2026-01-{day:02}T00:00:00Z"),
                    "assets": {"links": [{
                        "name": "app.apk",
                        "url": format!("https://gitlab.example.net/{index}.apk")
                    }]}
                })
            })
            .collect::<Vec<_>>();
        let mut requests = 0;

        let selected = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_| {
                requests += 1;
                Ok(page(&endpoint, 1, Value::Array(releases.clone()), None))
            },
        )
        .unwrap();

        assert_eq!(selected.release_tag, "v0");
        assert_eq!(requests, 1);
    }

    #[test]
    fn unordered_provider_pages_are_followed_and_the_latest_release_assets_are_authoritative() {
        let endpoint = endpoint(ReleaseProvider::Github, "https://github.com", "owner/repo");
        let mut requests = Vec::new();
        let selected = resolve_release_pages(
            ReleaseProvider::Github,
            &endpoint,
            "apk",
            false,
            Some(&Regex::new("^wanted\\.apk$").unwrap()),
            false,
            |url| {
                requests.push(url.clone());
                let page_number = url
                    .query_pairs()
                    .find(|(key, _)| key == "page")
                    .unwrap()
                    .1
                    .parse::<usize>()
                    .unwrap();
                if page_number == 1 {
                    Ok(page(
                        &endpoint,
                        1,
                        json!([github_release("v1", "2025-01-01T00:00:00Z", "wanted.apk")]),
                        Some(2),
                    ))
                } else {
                    Ok(page(
                        &endpoint,
                        2,
                        json!([github_release("v2", "2026-01-01T00:00:00Z", "other.apk")]),
                        None,
                    ))
                }
            },
        )
        .unwrap_err();
        assert_eq!(requests.len(), 2);
        assert!(selected.starts_with("remote_asset_no_match:"));
    }

    #[test]
    fn empty_exhausted_history_returns_not_found_and_foreign_next_links_fail_closed() {
        let endpoint = endpoint(ReleaseProvider::Github, "https://github.com", "owner/repo");
        let error = resolve_release_pages(
            ReleaseProvider::Github,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_url| Ok(page(&endpoint, 1, json!([]), None)),
        )
        .unwrap_err();
        assert!(error.starts_with("remote_release_not_found:"));

        let mut foreign = page(
            &endpoint,
            1,
            json!([github_release("v1", "2026-01-01T00:00:00Z", "app.apk")]),
            None,
        );
        foreign.headers.insert(
            LINK,
            "<https://attacker.example/repos/owner/repo/releases?per_page=30&page=2>; rel=\"next\""
                .parse()
                .unwrap(),
        );
        let error = resolve_release_pages(
            ReleaseProvider::Github,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_| Ok(foreign.clone()),
        )
        .unwrap_err();
        assert!(error.starts_with("remote_release_endpoint_invalid:"));

        let mut foreign_previous = page(
            &endpoint,
            1,
            json!([github_release("v1", "2026-01-01T00:00:00Z", "app.apk")]),
            None,
        );
        foreign_previous.headers.insert(
            LINK,
            "<https://attacker.example/repos/owner/repo/releases?per_page=30&page=1>; rel=\"prev\""
                .parse()
                .unwrap(),
        );
        let error = resolve_release_pages(
            ReleaseProvider::Github,
            &endpoint,
            "apk",
            false,
            None,
            false,
            |_| Ok(foreign_previous.clone()),
        )
        .unwrap_err();
        assert!(error.starts_with("remote_release_endpoint_invalid:"));
    }

    #[test]
    fn conflicting_provider_pagination_markers_fail_closed() {
        let gitlab = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let mut gitlab_page = page(
            &gitlab,
            1,
            json!([{
                "tag_name":"v1", "name":"Stable", "released_at":"2026-01-01T00:00:00Z",
                "assets":{"links":[{"name":"app.apk","url":"https://gitlab.example.net/app.apk"}]}
            }]),
            Some(2),
        );
        gitlab_page
            .headers
            .insert("x-next-page", "3".parse().unwrap());
        let error = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &gitlab,
            "apk",
            false,
            None,
            false,
            |_| Ok(gitlab_page.clone()),
        )
        .unwrap_err();
        assert!(error.starts_with("remote_release_history_ambiguous:"));

        let forgejo = endpoint(
            ReleaseProvider::Forgejo,
            "https://forgejo.example.net",
            "owner/repo",
        );
        let mut forgejo_page = page(
            &forgejo,
            1,
            json!([{
                "tag_name":"v1", "published_at":"2026-01-01T00:00:00Z",
                "assets":[{"name":"app.apk","browser_download_url":"https://forgejo.example.net/app.apk"}]
            }]),
            Some(2),
        );
        forgejo_page
            .headers
            .insert("x-total-count", "1".parse().unwrap());
        let error = resolve_release_pages(
            ReleaseProvider::Forgejo,
            &forgejo,
            "apk",
            false,
            None,
            false,
            |_| Ok(forgejo_page.clone()),
        )
        .unwrap_err();
        assert!(error.starts_with("remote_release_history_ambiguous:"));
    }

    #[test]
    fn release_discovery_fails_when_the_bound_expires_or_history_is_ambiguous() {
        let github = endpoint(ReleaseProvider::Github, "https://github.com", "owner/repo");
        let mut github_pages = 0usize;
        let error = resolve_release_pages(
            ReleaseProvider::Github,
            &github,
            "apk",
            false,
            None,
            false,
            |url| {
                github_pages += 1;
                let page_number = url
                    .query_pairs()
                    .find(|(key, _)| key == "page")
                    .unwrap()
                    .1
                    .parse::<usize>()
                    .unwrap();
                Ok(page(
                    &github,
                    page_number,
                    json!([github_release(
                        &format!("v{page_number}"),
                        &format!("2026-01-0{page_number}T00:00:00Z"),
                        "app.apk"
                    )]),
                    Some(page_number + 1),
                ))
            },
        )
        .unwrap_err();
        assert_eq!(github_pages, MAX_RELEASE_PAGES);
        assert!(error.starts_with("remote_release_history_incomplete:"));

        let gitlab = endpoint(
            ReleaseProvider::Gitlab,
            "https://gitlab.example.net",
            "group/project",
        );
        let mut gitlab_pages = 0usize;
        let error = resolve_release_pages(
            ReleaseProvider::Gitlab,
            &gitlab,
            "apk",
            false,
            None,
            false,
            |url| {
                gitlab_pages += 1;
                let page_number = url
                    .query_pairs()
                    .find(|(key, _)| key == "page")
                    .unwrap()
                    .1
                    .parse::<usize>()
                    .unwrap();
                Ok(page(
                    &gitlab,
                    page_number,
                    json!([{
                        "tag_name":"v1", "name":"Stable",
                        "released_at":"2026-01-01T00:00:00Z",
                        "assets":{"links":[{"name":"app.apk","url":"https://gitlab.example.net/app.apk"}]}
                    }]),
                    Some(page_number + 1),
                ))
            },
        )
        .unwrap_err();
        assert_eq!(gitlab_pages, MAX_RELEASE_PAGES);
        assert!(error.starts_with("remote_release_history_incomplete:"));
    }

    #[test]
    fn artifact_kind_optional_pattern_inversion_and_forgejo_timestamp_fallback_are_explicit() {
        let assets = json!([{
            "draft": false,
            "prerelease": false,
            "tag_name": "v1",
            "published_at": "bad-timestamp",
            "created_at": "2026-03-01T00:00:00Z",
            "assets": [
                {"name":"app-v1.apk", "browser_download_url":"https://forge.example.net/app-v1.apk"},
                {"name":"bundle.tar", "browser_download_url":"https://forge.example.net/bundle.tar"}
            ]
        }]);
        let selected = resolve_remote_latest_from_fixture(
            "forgejo",
            "https://forge.example.net",
            "owner/repo",
            "file",
            false,
            Some("^bundle\\.tar$"),
            false,
            &assets,
        )
        .unwrap();
        assert_eq!(selected.asset_name, "bundle.tar");
        assert_eq!(selected.timestamp_source, "created_at");
        assert_eq!(selected.published_at, None);
        assert_eq!(selected.created_at.as_deref(), Some("2026-03-01T00:00:00Z"));

        let selected = resolve_remote_latest_from_fixture(
            "forgejo",
            "https://forge.example.net",
            "owner/repo",
            "apk",
            false,
            Some("^app-.*\\.apk$"),
            true,
            &assets,
        )
        .unwrap_err();
        assert!(selected.starts_with("remote_asset_no_match:"));

        let gitlab_invalid_timestamp = json!([{
            "tag_name":"v1", "name":"Stable", "released_at":"invalid",
            "assets":{"links":[{"name":"app.apk","url":"https://gitlab.example.net/app.apk"}]}
        }]);
        assert!(resolve_remote_latest_from_fixture(
            "gitlab",
            "https://gitlab.example.net",
            "group/project",
            "apk",
            false,
            None,
            false,
            &gitlab_invalid_timestamp,
        )
        .unwrap_err()
        .starts_with("remote_release_timestamp_invalid:"));
    }

    #[test]
    fn matching_assets_with_invalid_urls_fail_closed_while_pattern_exclusions_are_ignored() {
        let resolve = |assets: Value| {
            let releases = json!([{
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "name": "Stable",
                "published_at": "2026-03-01T00:00:00Z",
                "assets": assets,
            }]);
            resolve_remote_latest_from_fixture(
                "forgejo",
                "https://forge.example.net",
                "owner/repo",
                "apk",
                false,
                Some("^app-(good|bad|missing)\\.apk$"),
                false,
                &releases,
            )
        };
        let invalid_metadata = "remote_asset_invalid: Latest eligible release contains a matching asset with invalid or unsafe download metadata";

        let error = resolve(json!([
            {"name":"app-good.apk", "browser_download_url":"https://forge.example.net/good.apk"},
            {"name":"app-bad.apk", "browser_download_url":"http://forge.example.net/bad.apk"}
        ]))
        .unwrap_err();
        assert_eq!(error, invalid_metadata);

        let error = resolve(json!([
            {"name":"app-bad.apk", "browser_download_url":"https://127.0.0.1/bad.apk"}
        ]))
        .unwrap_err();
        assert_eq!(error, invalid_metadata);

        let error = resolve(json!([{"name":"app-missing.apk"}])).unwrap_err();
        assert_eq!(error, invalid_metadata);

        let selected = resolve(json!([
            {"name":"app-good.apk", "browser_download_url":"https://forge.example.net/good.apk"},
            {"name":"app-excluded.apk", "browser_download_url":"http://forge.example.net/excluded.apk"}
        ]))
        .unwrap();
        assert_eq!(selected.asset_name, "app-good.apk");
        assert_eq!(selected.download_url, "https://forge.example.net/good.apk");
    }

    #[test]
    fn generic_file_assets_require_a_valid_filename_and_reject_ambiguous_metadata() {
        let resolve = |assets: Value| {
            let releases = json!([{
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "published_at": "2026-03-01T00:00:00Z",
                "assets": assets,
            }]);
            resolve_remote_latest_from_fixture(
                "forgejo",
                "https://forge.example.net",
                "owner/repo",
                "file",
                false,
                None,
                false,
                &releases,
            )
        };
        let invalid_filename = "remote_asset_invalid: Latest eligible release contains a matching asset with an invalid filename";

        for asset in [
            json!({"browser_download_url":"https://forge.example.net/unnamed.bin"}),
            json!({"name":"", "browser_download_url":"https://forge.example.net/empty.bin"}),
            json!({"name":" \t ", "browser_download_url":"https://forge.example.net/blank.bin"}),
        ] {
            assert_eq!(resolve(json!([asset])).unwrap_err(), invalid_filename);
        }

        let selected = resolve(json!([{
            "name":"data-archive.zip",
            "browser_download_url":"https://forge.example.net/data-archive.zip"
        }]))
        .unwrap();
        assert_eq!(selected.asset_name, "data-archive.zip");
        assert_eq!(
            selected.download_url,
            "https://forge.example.net/data-archive.zip"
        );

        let error = resolve(json!([
            {
                "name":"valid.zip",
                "browser_download_url":"https://forge.example.net/valid.zip"
            },
            {"browser_download_url":"https://forge.example.net/unclassified.bin"}
        ]))
        .unwrap_err();
        assert_eq!(error, invalid_filename);
    }

    #[test]
    fn invalid_filename_on_latest_release_does_not_fall_back_to_an_older_release() {
        let releases = json!([
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v2",
                "published_at": "2026-03-02T00:00:00Z",
                "assets": [{
                    "name":"",
                    "browser_download_url":"https://forge.example.net/v2-empty.bin"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "published_at": "2026-03-01T00:00:00Z",
                "assets": [{
                    "name":"older.zip",
                    "browser_download_url":"https://forge.example.net/older.zip"
                }]
            }
        ]);

        let error = resolve_remote_latest_from_fixture(
            "forgejo",
            "https://forge.example.net",
            "owner/repo",
            "file",
            false,
            None,
            false,
            &releases,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "remote_asset_invalid: Latest eligible release contains a matching asset with an invalid filename"
        );
    }

    #[test]
    fn gitlab_prerelease_classification_keeps_the_approved_marker_set() {
        for marker in [
            "alpha",
            "beta",
            "preview",
            "prerelease",
            "pre-release",
            "rc",
        ] {
            assert!(likely_prerelease(&format!("v1-{marker}"), ""), "{marker}");
        }
        assert!(!likely_prerelease("v1-candidate", "Release Candidate"));
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

    #[test]
    fn github_release_selection_ignores_malformed_published_dates() {
        let releases = json!([
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v1",
                "published_at": "2025-12-31T00:00:00Z",
                "assets": [{
                    "name": "wanted.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v1/wanted.apk"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v2",
                "published_at": "2026-02-01T00:00:00Z",
                "assets": [{
                    "name": "wanted.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/v2/wanted.apk"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v-invalid-date",
                "published_at": "9999-99-99T99:99:99Z",
                "assets": [{
                    "name": "wanted.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/invalid/wanted.apk"
                }]
            },
            {
                "draft": false,
                "prerelease": false,
                "tag_name": "v-missing-date",
                "published_at": null,
                "assets": [{
                    "name": "wanted.apk",
                    "browser_download_url": "https://github.com/example/app/releases/download/missing/wanted.apk"
                }]
            }
        ]);
        let matcher = Regex::new("^wanted\\.apk$").unwrap();

        let selected = select_github_release_asset(&releases, false, &matcher).unwrap();

        assert_eq!(selected.release_tag, "v2");
        assert_eq!(
            selected.published_at.as_deref(),
            Some("2026-02-01T00:00:00Z")
        );
    }

    #[test]
    fn github_release_selection_rejects_releases_without_a_valid_published_date() {
        let releases = json!([{
            "draft": false,
            "prerelease": false,
            "tag_name": "v-undated",
            "assets": [{
                "name": "wanted.apk",
                "browser_download_url": "https://github.com/example/app/releases/download/undated/wanted.apk"
            }]
        }]);
        let matcher = Regex::new("^wanted\\.apk$").unwrap();

        let error = select_github_release_asset(&releases, false, &matcher).unwrap_err();

        assert_eq!(
            error,
            "remote_release_not_found: No eligible GitHub release was found"
        );
    }
}

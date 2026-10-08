//! Side-effect-free app-definition and recipe generation for inspected remote APK sources.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::authored_models::{
    emit_app_definition_yaml, validate_app_definition, AppArtifactKind, AppArtifactSource,
    AppArtifactV1, AppDefinitionV1, AppPermissionSets, OrderedValueMap, ReleaseProvider,
    APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use crate::model::{
    InputDeclaration, InputValidation, OrderedMap, ParamValue, Recipe, RecipeArtifact,
    RecipeProvides, RemoteFileArtifact, Step, StepCondition, StepConstraints,
};
use crate::validation::normalize_expected_sha256;
use indexmap::IndexMap;

use super::apk::ApkInspectionFacts;
use super::app_recipe::{
    app_permission_sets, build_permission_step, collect_artifacts, collect_targets, error,
    generated_apk_inspection_metadata, pair_reviewed_artifact, parse_mapping,
    verified_launcher_activity, warning, AppArtifactEdit, AppTargetEdit, DraftDiagnostic,
    DraftSeverity, PermissionAutomationIssue, PermissionAutomationSelection,
    ReviewedArtifactPairing,
};
use super::identifiers::{normalize_identifier_component, recipe_local_token};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteAppRecipeDraftRequest {
    pub facts: ApkInspectionFacts,
    pub source: RemoteSource,
    #[serde(default)]
    release_analysis: Option<TrustedReleaseAnalysis>,
    #[serde(default)]
    pub permission_automation: Option<PermissionAutomationSelection>,
    #[serde(default)]
    pub app: Option<AppDefinitionV1>,
    #[serde(default)]
    pub recipe: Option<RemoteRecipeEdits>,
    #[serde(default)]
    pub mappings: Option<RemoteMappingEdits>,
    #[serde(default)]
    pub regenerate_identifiers: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteSource {
    pub mode: String,
    pub strategy: String,
    pub download_url: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub release_tag: Option<String>,
    #[serde(default)]
    pub asset_name: Option<String>,
    #[serde(default)]
    pub asset_pattern: Option<String>,
    #[serde(default)]
    pub include_prereleases: bool,
    #[serde(default)]
    pub trusted_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TrustedReleaseAnalysis {
    releases: Vec<TrustedRelease>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TrustedRelease {
    release_tag: String,
    prerelease: bool,
    asset_file_names: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GeneratedRecipeIds {
    recipe_id: String,
    input_id: String,
    feature_id: String,
    install_step_id: String,
    permission_step_id: String,
    launch_step_id: String,
    artifact_id: String,
    resolve_step_id: String,
    latest_resolve_step_id: String,
    download_step_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteRecipeEdits {
    ids: Option<GeneratedRecipeIds>,
    name: String,
    description: String,
    input_label: String,
    input_description: String,
    replace_existing: bool,
    launch_enabled: bool,
    launcher_activity: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteMappingEdits {
    artifacts: Vec<AppArtifactEdit>,
    targets: Vec<AppTargetEdit>,
    metadata: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceState {
    Verified,
    Derived,
    Missing,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FieldEvidence {
    field: String,
    state: EvidenceState,
    source: String,
    edited_from_proposal: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProposedDestination {
    file_name: Option<String>,
    relative_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAppRecipeDraft {
    app: AppDefinitionV1,
    /// Transient APK inspection evidence shown during draft review.
    ///
    /// The evidence never becomes part of the saved app definition, so the
    /// canonical YAML an author reviews matches the YAML that is written.
    apk_inspection: Option<Value>,
    recipe: Value,
    recipe_edits: RemoteRecipeEdits,
    app_canonical_yaml: Option<String>,
    recipe_canonical_yaml: Option<String>,
    app_destination: ProposedDestination,
    recipe_destination: ProposedDestination,
    evidence: Vec<FieldEvidence>,
    diagnostics: Vec<DraftDiagnostic>,
    blocking: bool,
}

pub(crate) fn generate_remote_app_recipe_draft(
    request: RemoteAppRecipeDraftRequest,
) -> RemoteAppRecipeDraft {
    let proposed = proposed_app(&request.facts, &request.source);
    let mut app = request.app.unwrap_or_else(|| proposed.clone());
    let mut diagnostics = Vec::new();
    if let Some(mappings) = request.mappings {
        apply_mapping_edits(&mut app, mappings, &mut diagnostics);
    }
    // The reviewed App Definition artifact is the authority for the download
    // policy duplicated below; the source selection only supplies the facts
    // that artifact leaves blank.
    let pairing = pair_reviewed_artifact(&app, &proposed, |artifact| {
        // The generated Recipe installs an APK, so only an APK artifact can
        // represent the reviewed source: a generic file with the same source
        // strategy must not pair, or it would satisfy the presence check while
        // the App Definition declares no artifact the Recipe installs.
        artifact.kind == AppArtifactKind::Apk
            && artifact_strategy_name(&request.source.strategy)
                .is_some_and(|expected| app_artifact_strategy_name(&artifact.source) == expected)
    });
    let source = match &pairing {
        ReviewedArtifactPairing::Paired(id) => {
            reconcile_remote_source(&mut app, id, &request.source)
        }
        ReviewedArtifactPairing::Missing | ReviewedArtifactPairing::Ambiguous => {
            request.source.clone()
        }
    };
    match &pairing {
        ReviewedArtifactPairing::Paired(id) => {
            if artifact_inversion(&app, id) == Some(true) {
                // The duplicated Recipe resolves a filename pattern by positive
                // match, so an inverted policy would install the assets its author
                // excluded instead of the ones they selected.
                diagnostics.push(error(
                    "latest_release_invert_unsupported",
                    "The generated recipe resolves filename patterns by positive match, so it cannot duplicate an inverted pattern yet. Clear the inversion to generate a matching App Definition and Recipe pair.",
                    "source.assetPattern",
                ));
            }
        }
        ReviewedArtifactPairing::Missing => {
            if matches!(
                request.source.strategy.as_str(),
                "pinned_remote_asset" | "latest_compatible_release" | "user_provided_apk"
            ) {
                // Replacing or removing the reviewed artifact would otherwise save
                // an App Definition without the remote download while the
                // duplicated Recipe still resolves and installs the selected
                // download address.
                diagnostics.push(error(
                    "remote_source_artifact_missing",
                    "The reviewed app definition no longer declares the remote artifact this source installs. Keep the remote artifact, or start a new draft for a user-provided APK.",
                    "source.strategy",
                ));
            }
        }
        ReviewedArtifactPairing::Ambiguous => {
            if matches!(
                request.source.strategy.as_str(),
                "pinned_remote_asset" | "latest_compatible_release" | "user_provided_apk"
            ) {
                // Several artifacts share the reviewed source strategy and none
                // of them is the artifact the draft proposed, so neither the
                // duplicated Recipe nor the App Definition can be attributed to
                // one reviewed artifact.
                diagnostics.push(error(
                    "remote_source_artifact_ambiguous",
                    "The reviewed app definition declares more than one artifact for this installation method, so the reviewed artifact is ambiguous. Keep exactly one artifact that represents this source.",
                    "source.strategy",
                ));
            }
        }
    }
    diagnostics.extend(validate_source(
        &source,
        &request.source,
        request.release_analysis.as_ref(),
    ));
    diagnostics.extend(permission_automation_diagnostics(
        request.permission_automation.as_ref(),
        &request.facts,
        &source,
    ));
    let automation_eligible = matches!(
        request.source.strategy.as_str(),
        "pinned_remote_asset" | "latest_compatible_release"
    );
    let apk_inspection = match generated_apk_inspection_metadata(
        &request.facts,
        request.permission_automation.as_ref(),
        automation_eligible,
    ) {
        Ok(metadata) => Some(metadata),
        Err(issues) => {
            diagnostics.extend(
                issues
                    .into_iter()
                    .map(|issue| error(issue.code, issue.message, &issue.field)),
            );
            None
        }
    };
    app.permission_sets = verified_permission_sets(
        request.permission_automation.as_ref(),
        &request.facts,
        automation_eligible,
    );
    let mut recipe_edits = request
        .recipe
        .unwrap_or_else(|| proposed_recipe_edits(&app, &source));
    if recipe_edits.ids.is_none() || request.regenerate_identifiers {
        recipe_edits.ids = Some(generated_ids(&app.id));
    }
    fill_empty_recipe_text(&mut recipe_edits, &app, &source);
    app.launcher_activity = verified_launcher_activity(
        &request.facts,
        recipe_edits.launch_enabled,
        recipe_edits.launcher_activity.as_deref(),
    );
    diagnostics.extend(app_diagnostics(&app));
    let recipe = build_recipe(
        &app,
        &request.facts,
        &source,
        &recipe_edits,
        request.permission_automation.as_ref(),
        &mut diagnostics,
    );
    diagnostics.extend(recipe_diagnostics(&recipe));
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.field.cmp(&right.field))
    });
    diagnostics.dedup();
    let app_has_error = diagnostics
        .iter()
        .any(|item| item.severity == DraftSeverity::Error && !item.field.starts_with("recipe."));
    let recipe_has_error = diagnostics
        .iter()
        .any(|item| item.severity == DraftSeverity::Error && item.field.starts_with("recipe."));
    let app_canonical_yaml = (!app_has_error)
        .then(|| emit_app_definition_yaml(&app).ok())
        .flatten();
    let recipe_canonical_yaml = (!recipe_has_error)
        .then(|| crate::yaml::emit_recipe_yaml(&recipe).ok())
        .flatten();
    let blocking = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DraftSeverity::Error);
    RemoteAppRecipeDraft {
        recipe: crate::dto::recipe_to_dto(&recipe),
        recipe_edits,
        app_canonical_yaml,
        recipe_canonical_yaml,
        app_destination: destination("apps", &app.id, !app_has_error),
        recipe_destination: destination("recipes", &recipe.id, !recipe_has_error),
        evidence: evidence(&request.facts, &request.source, &proposed, &app),
        diagnostics,
        blocking,
        app,
        apk_inspection,
    }
}

fn validate_source(
    source: &RemoteSource,
    selection: &RemoteSource,
    release_analysis: Option<&TrustedReleaseAnalysis>,
) -> Vec<DraftDiagnostic> {
    let mut diagnostics = Vec::new();
    if !matches!(
        source.mode.as_str(),
        "github_repository"
            | "github_release"
            | "gitlab_repository"
            | "gitlab_release"
            | "forgejo_repository"
            | "forgejo_release"
            | "direct_apk"
    ) {
        diagnostics.push(error(
            "remote_source_mode_invalid",
            "Choose a supported remote source.",
            "source.mode",
        ));
    }
    if !matches!(
        source.strategy.as_str(),
        "pinned_remote_asset" | "latest_compatible_release" | "user_provided_apk"
    ) {
        diagnostics.push(error(
            "remote_strategy_invalid",
            "Choose a supported installation method.",
            "source.strategy",
        ));
    }
    if source.strategy == "latest_compatible_release" {
        if !source.mode.ends_with("_repository")
            || source.repository.is_none()
            || source.provider.is_none()
            || source.base_url.is_none()
        {
            diagnostics.push(error(
                "latest_release_source_unsupported",
                "Latest compatible release requires a supported repository source.",
                "source.strategy",
            ));
        }
        if !matches!(
            source.provider.as_deref(),
            Some("github" | "gitlab" | "forgejo")
        ) {
            diagnostics.push(error(
                "latest_release_provider_unsupported",
                "Latest compatible release requires a supported release provider.",
                "source.provider",
            ));
        }
        // The runtime release resolver resolves GitHub and GitLab releases
        // from their official service origin, so an authored self-hosted
        // origin would save a Recipe that downloads from a different server.
        if let Some(origin) = canonical_provider_origin(source.provider.as_deref()) {
            if source
                .base_url
                .as_deref()
                .is_some_and(|value| !is_canonical_service_origin(value, origin))
            {
                diagnostics.push(error(
                    "latest_release_base_url_unsupported",
                    "The runtime release resolver resolves GitHub and GitLab releases from the provider official service origin, so a self-hosted origin cannot generate a matching recipe yet. Use the official origin, or a Forgejo source for a self-hosted server.",
                    "source.baseUrl",
                ));
            }
        }
        if normalize_asset_pattern(source.asset_pattern.as_deref())
            .is_some_and(|pattern| regex::Regex::new(&pattern).is_err())
        {
            diagnostics.push(error(
                "latest_release_asset_pattern_invalid",
                "Rust rejected the APK filename pattern. Use syntax supported by the runtime regex engine.",
                "source.assetPattern",
            ));
        }
        if source.mode == "github_repository" {
            if release_analysis.is_some() && !release_evidence_matches_selection(source, selection)
            {
                // The author retargeted the repository, provider, or service
                // origin of the reviewed artifact, so the trusted release
                // analysis belongs to a different source and cannot confirm
                // this filename policy.
                diagnostics.push(warning(
                    "latest_release_analysis_stale",
                    "The edited release source no longer matches the analyzed GitHub repository, so its releases could not confirm the filename policy. Analyze the edited source to check it.",
                    "source.assetPattern",
                ));
            } else {
                diagnostics.extend(release_pattern_diagnostics(source, release_analysis));
            }
        }
    }
    if let Some(trusted_sha256) = source.trusted_sha256.as_deref().filter(|value| {
        !value
            .trim_matches(|character: char| character.is_ascii_whitespace())
            .is_empty()
    }) {
        if source.strategy != "pinned_remote_asset" {
            diagnostics.push(error(
                "apk_trusted_sha256_strategy_unsupported",
                "A trusted publisher SHA-256 is supported only for pinned remote APK assets.",
                "source.trustedSha256",
            ));
        } else if normalize_expected_sha256(trusted_sha256).is_none() {
            diagnostics.push(error(
                "apk_trusted_sha256_invalid",
                "Trusted publisher SHA-256 must contain exactly 64 hexadecimal characters.",
                "source.trustedSha256",
            ));
        }
    }
    let valid_url = url::Url::parse(&source.download_url)
        .ok()
        .is_some_and(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        });
    if !valid_url {
        diagnostics.push(error(
            "remote_download_url_invalid",
            "The selected APK download address is not a valid HTTPS URL.",
            "source.downloadUrl",
        ));
    }
    diagnostics
}

/// Return whether a latest-release source still describes the analyzed GitHub
/// source that its trusted release analysis belongs to. An author may retarget
/// the provider, service origin, or repository of the reviewed artifact, which
/// leaves the session's release analysis describing a different source.
fn release_evidence_matches_selection(source: &RemoteSource, selection: &RemoteSource) -> bool {
    source.provider == selection.provider
        && source.base_url == selection.base_url
        && source.repository == selection.repository
}

fn release_pattern_diagnostics(
    source: &RemoteSource,
    release_analysis: Option<&TrustedReleaseAnalysis>,
) -> Vec<DraftDiagnostic> {
    let pattern = normalize_asset_pattern(source.asset_pattern.as_deref());
    let expression = match pattern {
        Some(pattern) => match regex::Regex::new(&pattern) {
            Ok(expression) => Some(expression),
            Err(_) => {
                return vec![error(
                    "latest_release_asset_pattern_invalid",
                    "Rust rejected the APK filename pattern. Use syntax supported by the runtime regex engine.",
                    "source.assetPattern",
                )]
            }
        },
        None => None,
    };
    let analysis = match release_analysis {
        Some(analysis) => analysis,
        None => {
            return vec![error(
                "latest_release_analysis_missing",
                "Trusted GitHub release analysis is required before latest-compatible generation.",
                "source.assetPattern",
            )]
        }
    };
    if analysis.releases.is_empty() {
        return vec![error(
            "latest_release_analysis_empty",
            "The trusted GitHub analysis contains no releases.",
            "source.assetPattern",
        )];
    }
    if !trusted_release_analysis_is_valid(analysis) {
        return vec![error(
            "latest_release_analysis_invalid",
            "The trusted GitHub release analysis is malformed or ambiguous.",
            "source.assetPattern",
        )];
    }
    let eligible_releases = analysis
        .releases
        .iter()
        .filter(|release| source.include_prereleases || !release.prerelease)
        .collect::<Vec<_>>();
    if eligible_releases.is_empty() {
        return vec![error(
            "latest_release_no_eligible_releases",
            "No trusted GitHub releases remain after applying the prerelease policy.",
            "source.assetPattern",
        )];
    }

    let mut diagnostics = Vec::new();
    for (index, release) in eligible_releases.into_iter().enumerate() {
        let mut matching_names = release
            .asset_file_names
            .iter()
            .filter(|file_name| {
                expression
                    .as_ref()
                    .is_none_or(|expression| expression.is_match(file_name))
            })
            .cloned()
            .collect::<Vec<_>>();
        matching_names.sort();
        let qualifier = if expression.is_some() {
            "matching the pattern"
        } else {
            "eligible"
        };
        match (index, matching_names.len()) {
            (0, 0) => diagnostics.push(error(
                "latest_release_current_no_match",
                &format!(
                    "The current release '{}' has no {qualifier} APK asset.",
                    release.release_tag,
                ),
                "source.assetPattern",
            )),
            (0, 1) => {}
            (0, count) => diagnostics.push(error(
                "latest_release_current_multiple_matches",
                &format!(
                    "The current release '{}' has {count} {qualifier} APK assets: {}.",
                    release.release_tag,
                    matching_names.join(", ")
                ),
                "source.assetPattern",
            )),
            (_, 0) => diagnostics.push(warning(
                "latest_release_historical_no_match",
                &format!(
                    "Older release '{}' has no {qualifier} APK asset.",
                    release.release_tag,
                ),
                "source.assetPattern",
            )),
            (_, 1) => {}
            (_, count) => diagnostics.push(warning(
                "latest_release_historical_multiple_matches",
                &format!(
                    "Older release '{}' has {count} {qualifier} APK assets: {}.",
                    release.release_tag,
                    matching_names.join(", ")
                ),
                "source.assetPattern",
            )),
        }
    }
    diagnostics
}

fn trusted_release_analysis_is_valid(analysis: &TrustedReleaseAnalysis) -> bool {
    let mut release_tags = HashSet::new();
    analysis.releases.iter().all(|release| {
        let mut asset_names = HashSet::new();
        !release.release_tag.trim().is_empty()
            && release_tags.insert(release.release_tag.as_str())
            && release.asset_file_names.iter().all(|asset_name| {
                !asset_name.trim().is_empty()
                    && asset_name.to_ascii_lowercase().ends_with(".apk")
                    && asset_names.insert(asset_name.as_str())
            })
    })
}

fn permission_automation_diagnostics(
    selection: Option<&PermissionAutomationSelection>,
    facts: &ApkInspectionFacts,
    source: &RemoteSource,
) -> Vec<DraftDiagnostic> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    let mut diagnostics = selection
        .validation_issues()
        .into_iter()
        .map(permission_automation_issue)
        .collect::<Vec<_>>();
    if !selection.is_empty() {
        if !matches!(
            source.strategy.as_str(),
            "pinned_remote_asset" | "latest_compatible_release"
        ) {
            diagnostics.push(error(
                "apk_permission_automation_strategy_unsupported",
                "Permission automation is supported only for package-enforced remote APK recipes.",
                "recipe.permissionAutomation",
            ));
        }
        if facts.package_name.as_deref() != Some(selection.package_name.as_str()) {
            diagnostics.push(error(
                "apk_permission_automation_package_mismatch",
                "Permission automation must use the package name from the inspected APK manifest.",
                "recipe.permissionAutomation.packageName",
            ));
        }
    }
    diagnostics
}

fn permission_automation_issue(issue: PermissionAutomationIssue) -> DraftDiagnostic {
    error(issue.code, issue.message, &issue.field)
}

/// Partition verified permission selections into the canonical app permission sets.
///
/// A selection contributes to the generated app definition only when its
/// strategy supports permission automation, its actions pass validation, and
/// its package identity matches the inspected APK manifest.
fn verified_permission_sets(
    selection: Option<&PermissionAutomationSelection>,
    facts: &ApkInspectionFacts,
    automation_eligible: bool,
) -> Option<AppPermissionSets> {
    let selection = selection.filter(|selection| automation_eligible && !selection.is_empty())?;
    if !selection.validation_issues().is_empty()
        || facts.package_name.as_deref() != Some(selection.package_name.as_str())
    {
        return None;
    }
    app_permission_sets(selection)
}

fn proposed_app(facts: &ApkInspectionFacts, source: &RemoteSource) -> AppDefinitionV1 {
    let id_source = facts
        .application_label
        .as_deref()
        .or(facts.package_name.as_deref())
        .unwrap_or_default();
    let name = facts
        .application_label
        .clone()
        .or_else(|| facts.package_name.clone())
        .unwrap_or_default();
    let mut artifacts = IndexMap::new();
    artifacts.insert(
        "apk".to_string(),
        AppArtifactV1 {
            kind: AppArtifactKind::Apk,
            name: (!name.trim().is_empty()).then(|| format!("{name} APK")),
            description: None,
            source: proposed_artifact_source(source),
        },
    );
    AppDefinitionV1 {
        schema_version: SCHEMA_VERSION_V1,
        kind: APP_DEFINITION_KIND.to_string(),
        id: normalize_identifier_component(id_source),
        name,
        description: None,
        category: None,
        package_id: facts.package_name.clone().unwrap_or_default(),
        artifacts,
        permission_sets: None,
        targets: IndexMap::new(),
        launcher_activity: None,
        metadata: OrderedValueMap::new(),
    }
}

/// The artifact source proposed for the selected remote source strategy.
///
/// Unsupported or incomplete selections fall back to the user-provided
/// strategy; a blocking source diagnostic prevents such a draft from being
/// emitted, so the fallback never reaches a saved document.
fn proposed_artifact_source(source: &RemoteSource) -> AppArtifactSource {
    match source.strategy.as_str() {
        "pinned_remote_asset" => AppArtifactSource::DirectUrl {
            url: source.download_url.clone(),
            sha256: source
                .trusted_sha256
                .as_deref()
                .and_then(normalize_expected_sha256)
                .map(|value| value.to_ascii_lowercase()),
        },
        "latest_compatible_release" => match release_provider(source.provider.as_deref()) {
            Some(provider) => AppArtifactSource::LatestRelease {
                provider,
                base_url: source.base_url.clone().unwrap_or_default(),
                repository: source.repository.clone().unwrap_or_default(),
                asset_pattern: normalize_asset_pattern(source.asset_pattern.as_deref()),
                invert_asset_pattern: None,
                prerelease: source.include_prereleases,
            },
            None => AppArtifactSource::UserProvided,
        },
        _ => AppArtifactSource::UserProvided,
    }
}

/// Normalize an authored APK filename pattern. Surrounding whitespace carries
/// no meaning, and a blank pattern means the release resolves by artifact kind
/// alone.
fn normalize_asset_pattern(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .map(str::to_string)
}

/// Release provider family name as stored in an App Definition.
fn release_provider_name(provider: &ReleaseProvider) -> &'static str {
    match provider {
        ReleaseProvider::Github => "github",
        ReleaseProvider::Gitlab => "gitlab",
        ReleaseProvider::Forgejo => "forgejo",
    }
}

/// App Definition artifact source strategy that represents one remote-source
/// strategy.
fn artifact_strategy_name(strategy: &str) -> Option<&'static str> {
    match strategy {
        "pinned_remote_asset" => Some("direct_url"),
        "latest_compatible_release" => Some("latest_release"),
        "user_provided_apk" => Some("user_provided"),
        _ => None,
    }
}

/// Artifact source strategy name as stored in one App Definition artifact.
fn app_artifact_strategy_name(source: &AppArtifactSource) -> &'static str {
    match source {
        AppArtifactSource::UserProvided => "user_provided",
        AppArtifactSource::DirectUrl { .. } => "direct_url",
        AppArtifactSource::LatestRelease { .. } => "latest_release",
    }
}

/// Return the filename-pattern inversion declared by one App Definition
/// artifact.
fn artifact_inversion(app: &AppDefinitionV1, id: &str) -> Option<bool> {
    match app.artifacts.get(id).map(|artifact| &artifact.source) {
        Some(AppArtifactSource::LatestRelease {
            invert_asset_pattern,
            ..
        }) => *invert_asset_pattern,
        _ => None,
    }
}

/// Reconcile the App Definition artifact that represents the selected remote
/// source with the source selection, and return the effective download policy
/// the generated Recipe duplicates.
///
/// The reviewed artifact is authoritative: facts edited there win, so the
/// Recipe duplicates the saved App Definition rather than the source step.
/// Blank artifact fields are filled from the source selection, so an
/// author-supplied checksum or filename pattern is retained no matter which
/// input produced it and the two generated documents cannot disagree.
fn reconcile_remote_source(
    app: &mut AppDefinitionV1,
    artifact_id: &str,
    source: &RemoteSource,
) -> RemoteSource {
    let Some(artifact) = app.artifacts.get_mut(artifact_id) else {
        return source.clone();
    };
    let mut effective = source.clone();
    match (&mut artifact.source, source.strategy.as_str()) {
        (AppArtifactSource::DirectUrl { url, sha256 }, "pinned_remote_asset") => {
            if url.trim() == source.download_url.trim() && sha256.is_none() {
                *sha256 = source
                    .trusted_sha256
                    .as_deref()
                    .and_then(normalize_expected_sha256)
                    .map(|value| value.to_ascii_lowercase());
            }
            effective.download_url = url.clone();
            if sha256.is_some() {
                effective.trusted_sha256 = sha256.clone();
            } else if url.trim() != source.download_url.trim() {
                // A retargeted artifact never inherits the checksum of the
                // selection it replaced.
                effective.trusted_sha256 = None;
            }
        }
        (
            AppArtifactSource::LatestRelease {
                provider,
                base_url,
                repository,
                asset_pattern,
                prerelease,
                ..
            },
            "latest_compatible_release",
        ) => {
            if repository.trim().is_empty() {
                if let Some(selection) = source.repository.as_deref() {
                    *repository = selection.to_string();
                }
            }
            if base_url.trim().is_empty() {
                if let Some(selection) = source.base_url.as_deref() {
                    *base_url = selection.to_string();
                }
            }
            // A blank pattern means kind-only resolution, so clearing the
            // artifact pattern must not restore the selection's old pattern.
            *asset_pattern = normalize_asset_pattern(asset_pattern.as_deref());
            effective.provider = Some(release_provider_name(provider).to_string());
            effective.base_url = Some(base_url.clone());
            effective.repository = Some(repository.clone());
            effective.asset_pattern = asset_pattern.clone();
            effective.include_prereleases = *prerelease;
        }
        _ => {}
    }
    effective
}

/// Return the service origin the runtime release resolver honors for one
/// provider family, or None for providers that resolve from the authored
/// origin.
fn canonical_provider_origin(provider: Option<&str>) -> Option<&'static str> {
    match provider {
        Some("github") => Some("https://github.com"),
        Some("gitlab") => Some("https://gitlab.com"),
        _ => None,
    }
}

/// Return whether an authored service origin names the canonical provider
/// origin. Trailing slashes and letter case carry no meaning.
fn is_canonical_service_origin(base_url: &str, origin: &str) -> bool {
    fn normalized(value: &str) -> String {
        value.trim().trim_end_matches('/').to_ascii_lowercase()
    }
    normalized(base_url) == normalized(origin)
}

fn release_provider(value: Option<&str>) -> Option<ReleaseProvider> {
    match value {
        Some("github") => Some(ReleaseProvider::Github),
        Some("gitlab") => Some(ReleaseProvider::Gitlab),
        Some("forgejo") => Some(ReleaseProvider::Forgejo),
        _ => None,
    }
}

fn generated_ids(app_id: &str) -> GeneratedRecipeIds {
    let local = recipe_local_token(app_id);
    GeneratedRecipeIds {
        recipe_id: format!("app.{app_id}.install"),
        input_id: format!("{local}_apk"),
        feature_id: format!("{local}_install"),
        install_step_id: format!("install_{local}"),
        permission_step_id: format!("grant_permissions_{local}"),
        launch_step_id: format!("launch_{local}"),
        artifact_id: format!("{local}_apk"),
        resolve_step_id: "resolve_artifacts".to_string(),
        latest_resolve_step_id: format!("resolve_latest_{local}"),
        download_step_id: format!("download_{local}"),
    }
}

fn proposed_recipe_edits(app: &AppDefinitionV1, source: &RemoteSource) -> RemoteRecipeEdits {
    let pinned = source.strategy == "pinned_remote_asset";
    let latest = source.strategy == "latest_compatible_release";
    RemoteRecipeEdits {
        ids: Some(generated_ids(&app.id)),
        name: format!("Install {}", app.name),
        description: if pinned {
            format!("Download and install {}.", app.name)
        } else if latest {
            format!(
                "Resolve the latest compatible release and install {}.",
                app.name
            )
        } else {
            format!("Install a user-provided {} APK.", app.name)
        },
        input_label: format!("{} APK", app.name),
        input_description: format!("Local {} APK to install.", app.name),
        ..RemoteRecipeEdits::default()
    }
}

fn fill_empty_recipe_text(
    edits: &mut RemoteRecipeEdits,
    app: &AppDefinitionV1,
    source: &RemoteSource,
) {
    if edits.name.trim().is_empty() {
        edits.name = format!("Install {}", app.name);
    }
    if edits.description.trim().is_empty() {
        edits.description = if source.strategy == "pinned_remote_asset" {
            format!("Download and install {}.", app.name)
        } else if source.strategy == "latest_compatible_release" {
            format!(
                "Resolve the latest compatible release and install {}.",
                app.name
            )
        } else {
            format!("Install a user-provided {} APK.", app.name)
        };
    }
    if edits.input_label.trim().is_empty() {
        edits.input_label = format!("{} APK", app.name);
    }
}

fn build_recipe(
    app: &AppDefinitionV1,
    facts: &ApkInspectionFacts,
    source: &RemoteSource,
    edits: &RemoteRecipeEdits,
    permission_automation: Option<&PermissionAutomationSelection>,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> Recipe {
    let ids = edits.ids.clone().unwrap_or_else(|| generated_ids(&app.id));
    let pinned = source.strategy == "pinned_remote_asset";
    let latest = source.strategy == "latest_compatible_release";
    let mut inputs = OrderedMap::new();
    let mut artifacts = OrderedMap::new();
    let mut steps = Vec::new();
    let app_ref;
    let install_dependencies;
    if pinned {
        artifacts.insert(
            ids.artifact_id.clone(),
            RecipeArtifact::RemoteFile(RemoteFileArtifact {
                url: source.download_url.clone(),
                cache: "default".to_string(),
            }),
        );
        let mut resolve_params = OrderedMap::new();
        resolve_params.insert(
            "artifacts".to_string(),
            ParamValue::Literal(Value::Array(vec![Value::String(ids.artifact_id.clone())])),
        );
        steps.push(Step {
            id: ids.resolve_step_id.clone(),
            type_name: "resolve_artifacts".to_string(),
            name: format!("Download {} APK", app.name),
            description: None,
            progress_note: Some(format!("Downloading {}", app.name)),
            user_toggleable: false,
            app_ref: None,
            dependencies: Vec::new(),
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: resolve_params,
            verify: Vec::new(),
        });
        app_ref = format!("artifacts.{}.local_path", ids.artifact_id);
        install_dependencies = vec![ids.resolve_step_id.clone()];
    } else if latest {
        let mut resolve_params = OrderedMap::new();
        resolve_params.insert(
            "provider".to_string(),
            ParamValue::Literal(Value::String(source.provider.clone().unwrap_or_default())),
        );
        resolve_params.insert(
            "base_url".to_string(),
            ParamValue::Literal(Value::String(source.base_url.clone().unwrap_or_default())),
        );
        resolve_params.insert(
            "repository".to_string(),
            ParamValue::Literal(Value::String(source.repository.clone().unwrap_or_default())),
        );
        resolve_params.insert(
            "include_prereleases".to_string(),
            ParamValue::Literal(Value::Bool(source.include_prereleases)),
        );
        resolve_params.insert(
            "asset_pattern".to_string(),
            ParamValue::Literal(Value::String(
                normalize_asset_pattern(source.asset_pattern.as_deref()).unwrap_or_default(),
            )),
        );
        steps.push(Step {
            id: ids.latest_resolve_step_id.clone(),
            type_name: "resolve_remote_release".to_string(),
            name: format!("Resolve latest {} release", app.name),
            description: Some(
                "Select the newest eligible provider release and require one matching APK."
                    .to_string(),
            ),
            progress_note: Some(format!("Resolving latest {} release", app.name)),
            user_toggleable: false,
            app_ref: None,
            dependencies: Vec::new(),
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: resolve_params,
            verify: Vec::new(),
        });
        let mut download_params = OrderedMap::new();
        download_params.insert(
            "url".to_string(),
            ParamValue::Ref(format!(
                "steps.{}.outputs.download_url",
                ids.latest_resolve_step_id
            )),
        );
        download_params.insert(
            "cache".to_string(),
            ParamValue::Literal(Value::String("default".to_string())),
        );
        steps.push(Step {
            id: ids.download_step_id.clone(),
            type_name: "download_remote_file".to_string(),
            name: format!("Download latest {} APK", app.name),
            description: None,
            progress_note: Some(format!("Downloading latest {} APK", app.name)),
            user_toggleable: false,
            app_ref: None,
            dependencies: vec![ids.latest_resolve_step_id.clone()],
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: download_params,
            verify: Vec::new(),
        });
        app_ref = format!("steps.{}.outputs.local_path", ids.download_step_id);
        install_dependencies = vec![ids.download_step_id.clone()];
    } else {
        inputs.insert(
            ids.input_id.clone(),
            InputDeclaration {
                type_name: "file".to_string(),
                role: "apk".to_string(),
                label: edits.input_label.trim().to_string(),
                description: present(&edits.input_description),
                required: true,
                multiple: false,
                validation: InputValidation {
                    must_exist: true,
                    allowed_extensions: vec!["apk".to_string()],
                    path_kind: Some("file".to_string()),
                    allowed_prefixes: Vec::new(),
                },
                default: Value::Null,
                options: Vec::new(),
                sensitive: false,
                advanced: false,
                metadata: OrderedMap::new(),
            },
        );
        app_ref = format!("inputs.{}", ids.input_id);
        install_dependencies = Vec::new();
    }
    let mut install_params = OrderedMap::new();
    install_params.insert("app".to_string(), ParamValue::Ref(app_ref));
    if pinned || latest {
        match facts.package_name.as_deref() {
            Some(package_name) if !package_name.trim().is_empty() => {
                install_params.insert(
                    "expected_package_name".to_string(),
                    ParamValue::Literal(Value::String(package_name.to_string())),
                );
            }
            _ => diagnostics.push(error(
                "apk_expected_package_name_unavailable",
                "Pinned and latest remote APK generation requires a package name verified by APK inspection.",
                "recipe.expectedPackageName",
            )),
        }
    }
    if pinned {
        if let Some(expected_sha256) = source
            .trusted_sha256
            .as_deref()
            .and_then(normalize_expected_sha256)
        {
            install_params.insert(
                "expected_sha256".to_string(),
                ParamValue::Literal(Value::String(expected_sha256)),
            );
        }
    }
    install_params.insert(
        "replace_existing".to_string(),
        ParamValue::Literal(Value::Bool(edits.replace_existing)),
    );
    let mut skip_params = OrderedMap::new();
    skip_params.insert(
        "package_name".to_string(),
        Value::String(app.package_id.clone()),
    );
    steps.push(Step {
        id: ids.install_step_id.clone(),
        type_name: "install_apk".to_string(),
        name: format!("Install {}", app.name),
        description: None,
        progress_note: Some(format!("Installing {} on the selected device", app.name)),
        user_toggleable: false,
        app_ref: None,
        dependencies: install_dependencies,
        constraints: StepConstraints {
            capabilities: vec!["apk_install".to_string()],
            conflicts_with: Vec::new(),
        },
        skip_if: vec![StepCondition {
            type_name: "package_installed".to_string(),
            params: skip_params,
        }],
        params: install_params,
        verify: Vec::new(),
    });
    let permission_step_id = permission_automation
        .filter(|selection| {
            !selection.is_empty()
                && matches!(
                    source.strategy.as_str(),
                    "pinned_remote_asset" | "latest_compatible_release"
                )
        })
        .map(|selection| {
            let step_id = ids.permission_step_id.clone();
            steps.push(build_permission_step(
                selection,
                step_id.clone(),
                ids.install_step_id.clone(),
                &app.name,
            ));
            step_id
        });
    if edits.launch_enabled {
        let launcher = edits
            .launcher_activity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if launcher.is_none()
            || !facts
                .launcher_activities
                .iter()
                .any(|value| Some(value.as_str()) == launcher)
        {
            diagnostics.push(error(
                "apk_launcher_unverified",
                "Launch-once generation requires a launcher component verified by APK inspection.",
                "recipe.launcherActivity",
            ));
        } else if let Some(launcher) = launcher {
            let mut params = OrderedMap::new();
            params.insert(
                "package_name".to_string(),
                ParamValue::Literal(Value::String(app.package_id.clone())),
            );
            params.insert(
                "activity".to_string(),
                ParamValue::Literal(Value::String(launcher.to_string())),
            );
            steps.push(Step {
                id: ids.launch_step_id.clone(),
                type_name: "launch_app".to_string(),
                name: format!("Launch {} once", app.name),
                description: Some("Launch the app once after installation.".to_string()),
                progress_note: Some(format!("Launching {} once", app.name)),
                user_toggleable: false,
                app_ref: None,
                dependencies: vec![permission_step_id
                    .clone()
                    .unwrap_or_else(|| ids.install_step_id.clone())],
                constraints: StepConstraints {
                    capabilities: vec!["app_launch".to_string()],
                    conflicts_with: Vec::new(),
                },
                skip_if: Vec::new(),
                params,
                verify: Vec::new(),
            });
        }
    }
    Recipe {
        schema_version: SCHEMA_VERSION_V1,
        kind: "recipe".to_string(),
        id: ids.recipe_id,
        name: edits.name.trim().to_string(),
        description: present(&edits.description),
        recipe_dependencies: Vec::new(),
        provides: RecipeProvides {
            features: vec![ids.feature_id],
        },
        inputs,
        artifacts,
        artifact_groups: OrderedMap::new(),
        steps,
    }
}

fn apply_mapping_edits(
    app: &mut AppDefinitionV1,
    mappings: RemoteMappingEdits,
    diagnostics: &mut Vec<DraftDiagnostic>,
) {
    app.artifacts = collect_artifacts(mappings.artifacts, diagnostics);
    app.targets = collect_targets(mappings.targets, diagnostics);
    if let Some(value) = parse_mapping("metadata", &mappings.metadata, diagnostics) {
        app.metadata = value;
    }
}

fn app_diagnostics(app: &AppDefinitionV1) -> Vec<DraftDiagnostic> {
    validate_app_definition(app)
        .into_iter()
        .map(|item| error(&item.code, &item.message, &item.field))
        .collect()
}

fn recipe_diagnostics(recipe: &Recipe) -> Vec<DraftDiagnostic> {
    let relative_path = format!("recipes/{}.yaml", recipe.id);
    crate::validation::validate_loaded_recipe_result(recipe, Path::new(&relative_path), None)
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let severity = match item.get("severity").and_then(Value::as_str) {
                Some("error") => DraftSeverity::Error,
                Some("warning") => DraftSeverity::Warning,
                _ => return None,
            };
            let code = item.get("code").and_then(Value::as_str)?.to_string();
            if code == "limited_validation_context" {
                return None;
            }
            Some(DraftDiagnostic {
                severity,
                code,
                message: item
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("The recipe is invalid.")
                    .to_string(),
                field: format!(
                    "recipe.{}",
                    item.get("field")
                        .and_then(Value::as_str)
                        .unwrap_or("recipe")
                ),
            })
        })
        .collect()
}

fn evidence(
    facts: &ApkInspectionFacts,
    source: &RemoteSource,
    proposed: &AppDefinitionV1,
    current: &AppDefinitionV1,
) -> Vec<FieldEvidence> {
    vec![
        field_evidence(
            "package_id",
            if facts.package_name.is_some() {
                EvidenceState::Verified
            } else {
                EvidenceState::Missing
            },
            "apk_manifest",
            current.package_id != proposed.package_id,
        ),
        field_evidence(
            "name",
            if facts.application_label.is_some() {
                EvidenceState::Verified
            } else {
                EvidenceState::Missing
            },
            "apk_manifest",
            current.name != proposed.name,
        ),
        field_evidence(
            "id",
            if proposed.id.is_empty() {
                EvidenceState::Missing
            } else {
                EvidenceState::Derived
            },
            "apk_identity",
            current.id != proposed.id,
        ),
        field_evidence(
            "category",
            EvidenceState::Missing,
            "author_required",
            current.category != proposed.category,
        ),
        field_evidence(
            "artifacts",
            EvidenceState::Verified,
            &source.mode,
            current.artifacts != proposed.artifacts,
        ),
        field_evidence(
            "metadata",
            EvidenceState::Missing,
            "author_optional",
            current.metadata != proposed.metadata,
        ),
    ]
}

fn field_evidence(
    field: &str,
    state: EvidenceState,
    source: &str,
    edited_from_proposal: bool,
) -> FieldEvidence {
    FieldEvidence {
        field: field.to_string(),
        state,
        source: source.to_string(),
        edited_from_proposal,
    }
}

fn destination(directory: &str, id: &str, valid: bool) -> ProposedDestination {
    if valid {
        let file_name = format!("{id}.yaml");
        ProposedDestination {
            relative_path: Some(format!("{directory}/{file_name}")),
            file_name: Some(file_name),
        }
    } else {
        ProposedDestination {
            file_name: None,
            relative_path: None,
        }
    }
}

fn present(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_models::AppOpMode;
    use crate::generation::app_recipe::{AppOpPermissionSelection, RuntimePermissionSelection};

    fn facts() -> ApkInspectionFacts {
        ApkInspectionFacts {
            package_name: Some("com.example.remote".to_string()),
            application_label: Some("Remote Example".to_string()),
            launcher_activities: vec!["com.example.remote/.MainActivity".to_string()],
            calculated_sha256: "B".repeat(64),
            checksum_status: "not_compared".to_string(),
            signature_verification: "not_performed".to_string(),
            split: Some(false),
            base: Some(true),
            ..ApkInspectionFacts::default()
        }
    }

    fn source() -> RemoteSource {
        RemoteSource {
            mode: "github_release".to_string(),
            strategy: "pinned_remote_asset".to_string(),
            download_url: "https://github.com/example/project/releases/download/v1/app.apk"
                .to_string(),
            provider: Some("github".to_string()),
            base_url: Some("https://github.com".to_string()),
            repository: Some("example/project".to_string()),
            release_tag: Some("v1".to_string()),
            asset_name: Some("app.apk".to_string()),
            asset_pattern: None,
            include_prereleases: false,
            trusted_sha256: None,
        }
    }

    fn trusted_release(
        release_tag: &str,
        prerelease: bool,
        asset_file_names: &[&str],
    ) -> TrustedRelease {
        TrustedRelease {
            release_tag: release_tag.to_string(),
            prerelease,
            asset_file_names: asset_file_names
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }

    fn release_analysis(releases: Vec<TrustedRelease>) -> TrustedReleaseAnalysis {
        TrustedReleaseAnalysis { releases }
    }

    fn unique_release_analysis() -> TrustedReleaseAnalysis {
        release_analysis(vec![trusted_release("v1", false, &["app-v1-arm64.apk"])])
    }

    fn latest_draft(
        pattern: &str,
        include_prereleases: bool,
        release_analysis: Option<TrustedReleaseAnalysis>,
    ) -> RemoteAppRecipeDraft {
        latest_draft_with_pattern(Some(pattern), include_prereleases, release_analysis)
    }

    fn latest_draft_with_pattern(
        pattern: Option<&str>,
        include_prereleases: bool,
        release_analysis: Option<TrustedReleaseAnalysis>,
    ) -> RemoteAppRecipeDraft {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = pattern.map(str::to_string);
        latest.include_prereleases = include_prereleases;
        let mut app = proposed_app(&facts(), &latest);
        app.category = Some("emulator".to_string());
        generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        })
    }

    fn permission_automation(
        runtime_permissions: Vec<RuntimePermissionSelection>,
        app_ops: Vec<AppOpPermissionSelection>,
    ) -> PermissionAutomationSelection {
        PermissionAutomationSelection {
            package_name: "com.example.remote".to_string(),
            runtime_permissions,
            app_ops,
        }
    }

    fn runtime_permission(name: &str, requires_root: bool) -> RuntimePermissionSelection {
        RuntimePermissionSelection {
            permission_name: name.to_string(),
            requires_root,
            android_api_min: 23,
            android_api_max: None,
        }
    }

    fn app_op(permission: &str, operation: &str, requires_root: bool) -> AppOpPermissionSelection {
        AppOpPermissionSelection {
            permission_name: permission.to_string(),
            operation_name: operation.to_string(),
            mode: "allow".to_string(),
            requires_root,
            android_api_min: 30,
            android_api_max: None,
        }
    }

    #[test]
    fn mixed_permission_automation_uses_inspected_package_and_launch_dependency() {
        let source = source();
        let mut app = proposed_app(&facts(), &source);
        app.category = Some("emulator".to_string());
        app.package_id = "com.example.edited".to_string();
        let mut edits = proposed_recipe_edits(&app, &source);
        edits.launch_enabled = true;
        edits.launcher_activity = Some("com.example.remote/.MainActivity".to_string());
        let selection = permission_automation(
            vec![
                runtime_permission("android.permission.RECORD_AUDIO", false),
                runtime_permission("android.permission.CAMERA", true),
            ],
            vec![
                app_op("android.permission.ZETA", "ZETA_OP", false),
                app_op(
                    "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "MANAGE_EXTERNAL_STORAGE",
                    true,
                ),
            ],
        );
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: Some(app),
            recipe: Some(edits),
            mappings: None,
            permission_automation: Some(selection),
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let steps = draft.recipe["steps"].as_array().unwrap();
        assert_eq!(
            steps
                .iter()
                .map(|step| step["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "resolve_artifacts",
                "install_apk",
                "grant_permissions",
                "launch_app"
            ]
        );
        let permission_step = &steps[2];
        assert_eq!(permission_step["id"], "grant_permissions_remote_example");
        assert_eq!(
            permission_step["dependencies"],
            serde_json::json!(["install_remote_example"])
        );
        assert_eq!(
            permission_step["constraints"]["capabilities"],
            serde_json::json!(["shell_command"])
        );
        assert_eq!(
            permission_step["params"]["policy"],
            serde_json::json!({ "on_failure": "warn", "require_all": false })
        );
        assert_eq!(
            permission_step["params"]["runtime"],
            serde_json::json!([
                {
                    "package_name": "com.example.remote",
                    "name": "android.permission.CAMERA",
                    "required": false,
                    "when": { "rooted": true, "android_api_min": 23 }
                },
                {
                    "package_name": "com.example.remote",
                    "name": "android.permission.RECORD_AUDIO",
                    "required": false,
                    "when": { "android_api_min": 23 }
                }
            ])
        );
        assert_eq!(
            permission_step["params"]["appops"],
            serde_json::json!([
                {
                    "package_name": "com.example.remote",
                    "op": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                    "required": false,
                    "when": { "rooted": true, "android_api_min": 30 }
                },
                {
                    "package_name": "com.example.remote",
                    "op": "ZETA_OP",
                    "mode": "allow",
                    "required": false,
                    "when": { "android_api_min": 30 }
                }
            ])
        );
        assert_eq!(
            steps[3]["dependencies"],
            serde_json::json!(["grant_permissions_remote_example"])
        );
        assert!(!permission_step.to_string().contains("com.example.edited"));
        assert!(!permission_step.to_string().contains("root_shell"));
        let evidence = draft
            .apk_inspection
            .as_ref()
            .expect("draft review keeps transient APK inspection evidence");
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
        let permission_sets = draft
            .app
            .permission_sets
            .as_ref()
            .expect("verified selections produce canonical permission sets");
        let baseline = permission_sets.baseline.as_ref().unwrap();
        assert_eq!(baseline.runtime.len(), 1);
        assert_eq!(
            baseline.runtime[0].permission,
            "android.permission.RECORD_AUDIO"
        );
        assert_eq!(baseline.runtime[0].android_api_min, Some(23));
        assert_eq!(baseline.app_ops.len(), 1);
        assert_eq!(baseline.app_ops[0].op, "ZETA_OP");
        let elevated = permission_sets.elevated.as_ref().unwrap();
        assert_eq!(elevated.runtime.len(), 1);
        assert_eq!(elevated.runtime[0].permission, "android.permission.CAMERA");
        assert_eq!(elevated.runtime[0].android_api_min, Some(23));
        assert_eq!(elevated.app_ops.len(), 1);
        assert_eq!(elevated.app_ops[0].op, "MANAGE_EXTERNAL_STORAGE");
        assert_eq!(elevated.app_ops[0].mode, AppOpMode::Allow);
        assert_eq!(elevated.app_ops[0].android_api_min, Some(30));
        let canonical = draft.app_canonical_yaml.clone().unwrap();
        assert!(canonical.contains("permission_sets:"));
        assert!(!canonical.contains("requires_root"));
        assert!(!canonical.contains("root_shell"));
        assert_eq!(
            evidence["selected_runtime_permissions"],
            serde_json::json!([
                { "permission_name": "android.permission.CAMERA", "requires_root": true },
                { "permission_name": "android.permission.RECORD_AUDIO", "requires_root": false }
            ])
        );
        assert_eq!(
            evidence["selected_app_ops"],
            serde_json::json!([
                {
                    "permission_name": "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "operation_name": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                    "requires_root": true
                },
                {
                    "permission_name": "android.permission.ZETA",
                    "operation_name": "ZETA_OP",
                    "mode": "allow",
                    "requires_root": false
                }
            ])
        );
    }

    #[test]
    fn runtime_only_and_app_op_only_each_generate_one_permission_step() {
        for (selection, expected_runtime, expected_app_ops) in [
            (
                permission_automation(
                    vec![runtime_permission("android.permission.CAMERA", false)],
                    Vec::new(),
                ),
                serde_json::json!([{
                    "package_name": "com.example.remote",
                    "name": "android.permission.CAMERA",
                    "required": false,
                    "when": { "android_api_min": 23 }
                }]),
                serde_json::json!([]),
            ),
            (
                permission_automation(
                    Vec::new(),
                    vec![app_op(
                        "android.permission.MANAGE_EXTERNAL_STORAGE",
                        "MANAGE_EXTERNAL_STORAGE",
                        true,
                    )],
                ),
                serde_json::json!([]),
                serde_json::json!([{
                    "package_name": "com.example.remote",
                    "op": "MANAGE_EXTERNAL_STORAGE",
                    "mode": "allow",
                    "required": false,
                    "when": { "rooted": true, "android_api_min": 30 }
                }]),
            ),
        ] {
            let source = source();
            let mut app = proposed_app(&facts(), &source);
            app.category = Some("emulator".to_string());
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
                release_analysis: None,
                app: Some(app),
                recipe: None,
                mappings: None,
                permission_automation: Some(selection),
                regenerate_identifiers: false,
            });
            assert!(!draft.blocking, "{:#?}", draft.diagnostics);
            let permission_steps = draft.recipe["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|step| step["type"] == "grant_permissions")
                .collect::<Vec<_>>();
            assert_eq!(permission_steps.len(), 1);
            let permission_step = permission_steps[0];
            assert_eq!(permission_step["params"]["runtime"], expected_runtime);
            assert_eq!(permission_step["params"]["appops"], expected_app_ops);
            assert_eq!(
                permission_step["constraints"]["capabilities"],
                serde_json::json!(["shell_command"])
            );
            assert!(!permission_step.to_string().contains("root_shell"));
            assert!(!draft.recipe.to_string().contains("root_shell"));
            assert!(!draft
                .recipe_canonical_yaml
                .as_deref()
                .unwrap()
                .contains("root_shell"));
        }
    }

    #[test]
    fn latest_compatible_release_generates_permission_step_after_install() {
        let mut source = source();
        source.mode = "github_repository".to_string();
        source.strategy = "latest_compatible_release".to_string();
        source.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &source);
        app.category = Some("emulator".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: Some(permission_automation(
                vec![runtime_permission("android.permission.CAMERA", false)],
                Vec::new(),
            )),
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(
            draft.recipe["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|step| step["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "resolve_remote_release",
                "download_remote_file",
                "install_apk",
                "grant_permissions"
            ]
        );
    }

    #[test]
    fn empty_permission_automation_preserves_existing_recipe_shape() {
        let source = source();
        let mut app = proposed_app(&facts(), &source);
        app.category = Some("emulator".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: Some(permission_automation(Vec::new(), Vec::new())),
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(!draft
            .recipe_canonical_yaml
            .unwrap()
            .contains("grant_permissions"));
    }

    #[test]
    fn invalid_duplicate_and_noneligible_permission_automation_blocks_generation() {
        let duplicate = permission_automation(
            vec![
                runtime_permission("android.permission.CAMERA", false),
                runtime_permission("android.permission.CAMERA", false),
            ],
            Vec::new(),
        );
        let mut user_provided = source();
        user_provided.strategy = "user_provided_apk".to_string();
        for (source, selection, expected_code) in [
            (source(), duplicate, "apk_permission_automation_duplicate"),
            (
                user_provided,
                permission_automation(
                    vec![runtime_permission("android.permission.CAMERA", false)],
                    Vec::new(),
                ),
                "apk_permission_automation_strategy_unsupported",
            ),
        ] {
            let mut app = proposed_app(&facts(), &source);
            app.category = Some("emulator".to_string());
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
                release_analysis: None,
                app: Some(app),
                recipe: None,
                mappings: None,
                permission_automation: Some(selection),
                regenerate_identifiers: false,
            });
            assert!(draft.blocking);
            assert!(draft
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected_code));
        }
    }

    #[test]
    fn pinned_source_generates_remote_artifact_and_resolve_step() {
        let mut app = proposed_app(&facts(), &source());
        app.category = Some("emulator".to_string());
        app.package_id = "com.example.edited".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: source(),
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("type: remote_file"));
        assert!(recipe.contains("type: resolve_artifacts"));
        assert!(recipe.contains("ref: artifacts.remote_example_apk.local_path"));
        assert!(recipe.contains("expected_package_name: com.example.remote"));
        assert!(!recipe.contains("expected_package_name: com.example.edited"));
        assert!(!recipe.contains("expected_sha256"));
    }

    #[test]
    fn blank_selection_pattern_normalizes_to_kind_only_policy() {
        let draft =
            latest_draft_with_pattern(Some("  \t "), false, Some(unique_release_analysis()));

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::LatestRelease {
                asset_pattern: None,
                ..
            }
        ));
        let resolve = draft.recipe["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["type"] == "resolve_remote_release")
            .expect("latest release resolve step");
        assert_eq!(resolve["params"]["asset_pattern"], "");
    }

    #[test]
    fn edited_artifact_download_policy_drives_the_duplicated_recipe() {
        let source = source();
        let mut app = proposed_app(&facts(), &source);
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::DirectUrl {
            url: "https://example.test/edited/app.apk".to_string(),
            sha256: Some("c".repeat(64)),
        };

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("https://example.test/edited/app.apk"));
        assert!(recipe_yaml.contains(&format!("expected_sha256: {}", "C".repeat(64))));
    }

    #[test]
    fn edited_artifact_release_policy_drives_the_duplicated_recipe() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::LatestRelease {
            provider: ReleaseProvider::Github,
            base_url: "https://github.com".to_string(),
            repository: "example/edited".to_string(),
            asset_pattern: Some("^app-v1.*\\.apk$".to_string()),
            invert_asset_pattern: None,
            prerelease: true,
        };

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let resolve = draft.recipe["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["type"] == "resolve_remote_release")
            .expect("latest release resolve step");
        assert_eq!(resolve["params"]["repository"], "example/edited");
        assert_eq!(resolve["params"]["asset_pattern"], "^app-v1.*\\.apk$");
        assert_eq!(resolve["params"]["include_prereleases"], true);
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("repository: example/edited"));
    }

    #[test]
    fn retargeted_release_repository_warns_instead_of_using_stale_evidence() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v1.*\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        // The trusted analysis describes example/project. The reviewed
        // artifact points at example/other with a pattern that no asset in the
        // analyzed repository matches, so judging that pattern against the
        // original analysis would report a false blocking mismatch.
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::LatestRelease {
            provider: ReleaseProvider::Github,
            base_url: "https://github.com".to_string(),
            repository: "example/other".to_string(),
            asset_pattern: Some("^unrelated-.*\\.apk$".to_string()),
            invert_asset_pattern: None,
            prerelease: false,
        };

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_analysis_stale"));
        assert!(!draft
            .diagnostics
            .iter()
            .any(|item| item.code.starts_with("latest_release_current_")));
    }

    #[test]
    fn clearing_the_reviewed_artifact_pattern_resolves_by_kind_alone() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        if let AppArtifactSource::LatestRelease { asset_pattern, .. } =
            &mut app.artifacts.get_mut("apk").unwrap().source
        {
            *asset_pattern = None;
        }

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::LatestRelease {
                asset_pattern: None,
                ..
            }
        ));
        let resolve = draft.recipe["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["type"] == "resolve_remote_release")
            .expect("latest release resolve step");
        assert_eq!(resolve["params"]["asset_pattern"], "");
    }

    #[test]
    fn replacing_the_reviewed_remote_artifact_blocks_generation() {
        let pinned = source();
        let mut app = proposed_app(&facts(), &pinned);
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::UserProvided;

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: pinned,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "remote_source_artifact_missing"));
    }

    #[test]
    fn inverting_the_reviewed_artifact_pattern_blocks_generation() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v1.*\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        if let AppArtifactSource::LatestRelease {
            invert_asset_pattern,
            ..
        } = &mut app.artifacts.get_mut("apk").unwrap().source
        {
            *invert_asset_pattern = Some(true);
        }

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_invert_unsupported"));
    }

    #[test]
    fn selection_checksum_fills_a_blank_artifact_download_source() {
        let mut selection = source();
        selection.trusted_sha256 = Some("d".repeat(64).to_ascii_uppercase());
        let app = proposed_app(&facts(), &source());

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: selection,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let expected = "d".repeat(64);
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl {
                sha256: Some(sha256),
                ..
            } if sha256 == &expected
        ));
        assert!(draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains(&format!(
                "expected_sha256: {}",
                expected.to_ascii_uppercase()
            )));
    }

    #[test]
    fn pinned_source_emits_only_explicit_valid_trusted_sha256() {
        let mut source = source();
        source.trusted_sha256 = Some(
            " \t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n".to_string(),
        );
        let mut app = proposed_app(&facts(), &source);
        app.category = Some("emulator".to_string());

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft.recipe_canonical_yaml.unwrap().contains(
            "expected_sha256: 0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"
        ));
        let evidence = draft.apk_inspection.as_ref().unwrap();
        assert_eq!(evidence["calculated_sha256"], "B".repeat(64));
        assert_eq!(evidence["checksum_status"], "not_compared");
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl {
                sha256: Some(sha256),
                ..
            } if sha256 == "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
    }

    #[test]
    fn pinned_source_rejects_invalid_trusted_sha256() {
        let mut source = source();
        source.trusted_sha256 = Some("sha256:not-trusted".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });

        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "apk_trusted_sha256_invalid"
                && diagnostic.field == "source.trustedSha256"
        }));
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap_or_default()
            .contains("expected_sha256"));
    }

    #[test]
    fn non_pinned_strategies_reject_non_empty_trusted_sha256() {
        for strategy in ["latest_compatible_release", "user_provided_apk"] {
            let mut source = source();
            source.strategy = strategy.to_string();
            source.trusted_sha256 = Some("A".repeat(64));
            if strategy == "latest_compatible_release" {
                source.mode = "github_repository".to_string();
                source.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
            }
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
                release_analysis: (strategy == "latest_compatible_release")
                    .then(unique_release_analysis),
                app: None,
                recipe: None,
                mappings: None,
                permission_automation: None,
                regenerate_identifiers: false,
            });

            assert!(draft.blocking, "{strategy}");
            assert!(draft.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "apk_trusted_sha256_strategy_unsupported"
                    && diagnostic.field == "source.trustedSha256"
            }));
            assert!(!draft
                .recipe_canonical_yaml
                .as_deref()
                .unwrap_or_default()
                .contains("expected_sha256"));
        }
    }

    #[test]
    fn all_remote_strategies_accept_blank_trusted_sha256_as_absent() {
        for strategy in [
            "pinned_remote_asset",
            "latest_compatible_release",
            "user_provided_apk",
        ] {
            let mut source = source();
            source.strategy = strategy.to_string();
            source.trusted_sha256 = Some(" \t\r\n".to_string());
            if strategy == "latest_compatible_release" {
                source.mode = "github_repository".to_string();
                source.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
            }
            let mut app = proposed_app(&facts(), &source);
            app.category = Some("emulator".to_string());
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
                release_analysis: (strategy == "latest_compatible_release")
                    .then(unique_release_analysis),
                app: Some(app),
                recipe: None,
                mappings: None,
                permission_automation: None,
                regenerate_identifiers: false,
            });

            assert!(!draft.blocking, "{strategy}: {:#?}", draft.diagnostics);
            assert!(!draft
                .recipe_canonical_yaml
                .unwrap()
                .contains("expected_sha256"));
        }
    }

    #[test]
    fn user_provided_strategy_uses_user_provided_artifact_source() {
        let mut source = source();
        source.strategy = "user_provided_apk".to_string();
        let app = proposed_app(&facts(), &source);
        assert_eq!(app.package_id, "com.example.remote");
        assert!(matches!(app.artifacts["apk"].kind, AppArtifactKind::Apk));
        assert_eq!(app.artifacts["apk"].source, AppArtifactSource::UserProvided);
        assert!(app.targets.is_empty());
        assert!(app.permission_sets.is_none());

        let mut app = app;
        app.category = Some("emulator".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(!recipe.contains("type: remote_file"));
        assert!(recipe.contains("ref: inputs.remote_example_apk"));
        assert!(!recipe.contains("expected_package_name"));
        assert!(!recipe.contains("expected_sha256"));
    }

    #[test]
    fn latest_strategy_generates_explicit_resolve_download_install_chain() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        latest.include_prereleases = true;
        let mut app = proposed_app(&facts(), &latest);
        app.category = Some("emulator".to_string());
        app.package_id = "com.example.edited".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("type: resolve_remote_release"));
        assert!(recipe.contains("type: download_remote_file"));
        assert!(recipe.contains("include_prereleases: true"));
        assert!(recipe.contains("ref: steps.download_remote_example.outputs.local_path"));
        assert!(recipe.contains("expected_package_name: com.example.remote"));
        assert!(!recipe.contains("expected_package_name: com.example.edited"));
        assert!(!recipe.contains("expected_sha256"));
    }

    #[test]
    fn pinned_and_latest_sources_block_without_inspected_package_name() {
        for strategy in ["pinned_remote_asset", "latest_compatible_release"] {
            let mut source = source();
            source.strategy = strategy.to_string();
            if strategy == "latest_compatible_release" {
                source.mode = "github_repository".to_string();
                source.asset_pattern = Some("^app\\.apk$".to_string());
            }
            let mut app = proposed_app(&facts(), &source);
            app.category = Some("emulator".to_string());
            let mut unavailable_facts = facts();
            unavailable_facts.package_name =
                (strategy == "latest_compatible_release").then(|| "   ".to_string());

            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: unavailable_facts,
                source,
                release_analysis: (strategy == "latest_compatible_release")
                    .then(unique_release_analysis),
                app: Some(app),
                recipe: None,
                mappings: None,
                permission_automation: None,
                regenerate_identifiers: false,
            });

            assert!(draft.blocking);
            assert!(draft.recipe_canonical_yaml.is_none());
            assert!(draft.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "apk_expected_package_name_unavailable"
                    && diagnostic.field == "recipe.expectedPackageName"
            }));
        }
    }

    #[test]
    fn gitlab_latest_strategy_preserves_provider_identity() {
        let mut latest = source();
        latest.mode = "gitlab_repository".to_string();
        latest.provider = Some("gitlab".to_string());
        latest.base_url = Some("https://gitlab.com".to_string());
        latest.repository = Some("example/group/project".to_string());
        latest.download_url = "https://gitlab.com/example/group/project".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        app.category = Some("emulator".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        match &draft.app.artifacts["apk"].source {
            AppArtifactSource::LatestRelease {
                provider,
                base_url,
                repository,
                ..
            } => {
                assert_eq!(*provider, ReleaseProvider::Gitlab);
                assert_eq!(base_url, "https://gitlab.com");
                assert_eq!(repository, "example/group/project");
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("provider: gitlab"));
        assert!(recipe.contains("base_url: https://gitlab.com"));
        assert!(recipe.contains("repository: example/group/project"));
    }

    #[test]
    fn forgejo_latest_strategy_preserves_custom_base_url() {
        let mut latest = source();
        latest.mode = "forgejo_repository".to_string();
        latest.provider = Some("forgejo".to_string());
        latest.base_url = Some("https://codeberg.org".to_string());
        latest.repository = Some("example/project".to_string());
        latest.download_url = "https://codeberg.org/example/project".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        app.category = Some("emulator".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        match &draft.app.artifacts["apk"].source {
            AppArtifactSource::LatestRelease {
                provider,
                base_url,
                repository,
                ..
            } => {
                assert_eq!(*provider, ReleaseProvider::Forgejo);
                assert_eq!(base_url, "https://codeberg.org");
                assert_eq!(repository, "example/project");
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("provider: forgejo"));
        assert!(recipe.contains("base_url: https://codeberg.org"));
    }

    #[test]
    fn latest_pattern_rejects_javascript_valid_rust_invalid_lookahead() {
        let draft = latest_draft(r"^(?=app).*\.apk$", false, Some(unique_release_analysis()));
        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_asset_pattern_invalid"
                && diagnostic.message.contains("Rust rejected")
        }));
    }

    #[test]
    fn latest_pattern_distinguishes_missing_empty_and_filtered_release_analysis() {
        let cases = [
            (None, "latest_release_analysis_missing"),
            (
                Some(release_analysis(Vec::new())),
                "latest_release_analysis_empty",
            ),
            (
                Some(release_analysis(vec![trusted_release(
                    "v2-beta",
                    true,
                    &["app-v2-arm64.apk"],
                )])),
                "latest_release_no_eligible_releases",
            ),
        ];
        for (analysis, expected_code) in cases {
            let draft = latest_draft(r"^app-v.*-arm64\.apk$", false, analysis);
            assert!(draft.blocking, "{expected_code}");
            assert!(draft
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected_code));
        }
    }

    #[test]
    fn latest_pattern_blocks_newest_zero_and_multiple_matches() {
        let no_match = latest_draft(r"^other\.apk$", false, Some(unique_release_analysis()));
        assert!(no_match.blocking);
        assert!(no_match
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "latest_release_current_no_match"));

        let multiple = latest_draft(
            r"\.apk$",
            false,
            Some(release_analysis(vec![trusted_release(
                "v2",
                false,
                &["z.apk", "a.apk"],
            )])),
        );
        assert!(multiple.blocking);
        let diagnostic = multiple
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "latest_release_current_multiple_matches")
            .unwrap();
        assert!(diagnostic.message.contains("a.apk, z.apk"));
    }

    #[test]
    fn latest_pattern_keeps_historical_mismatches_as_warnings() {
        let draft = latest_draft(
            r"^app-v\d+-arm64\.apk$",
            false,
            Some(release_analysis(vec![
                trusted_release("current", false, &["app-v3-arm64.apk"]),
                trusted_release("older-zero", false, &["other.apk"]),
                trusted_release(
                    "older-multiple",
                    false,
                    &["app-v1-arm64.apk", "app-v2-arm64.apk"],
                ),
            ])),
        );
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_historical_no_match"
                && diagnostic.severity == DraftSeverity::Warning
        }));
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_historical_multiple_matches"
                && diagnostic.severity == DraftSeverity::Warning
        }));
    }

    #[test]
    fn latest_pattern_uses_trusted_provider_response_order_for_current_release() {
        let draft = latest_draft(
            r"^app-v2-arm64\.apk$",
            false,
            Some(release_analysis(vec![
                trusted_release("first-response", false, &["app-v1-arm64.apk"]),
                trusted_release("second-response", false, &["app-v2-arm64.apk"]),
            ])),
        );
        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_current_no_match"
                && diagnostic.message.contains("first-response")
        }));
    }

    #[test]
    fn latest_pattern_rejects_ambiguous_trusted_release_identity() {
        for analysis in [
            release_analysis(vec![
                trusted_release("duplicate", false, &["one.apk"]),
                trusted_release("duplicate", false, &["two.apk"]),
            ]),
            release_analysis(vec![trusted_release(
                "v1",
                false,
                &["same.apk", "same.apk"],
            )]),
        ] {
            let draft = latest_draft(r"\.apk$", false, Some(analysis));
            assert!(draft.blocking);
            assert!(draft
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "latest_release_analysis_invalid"));
        }
    }

    #[test]
    fn unsafe_download_url_is_blocking() {
        let mut source = source();
        source.download_url = "http://example.com/app.apk".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "remote_download_url_invalid"));
    }

    #[test]
    fn blank_asset_pattern_filters_latest_release_assets_by_kind_only() {
        let draft = latest_draft_with_pattern(None, false, Some(unique_release_analysis()));
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        match &draft.app.artifacts["apk"].source {
            AppArtifactSource::LatestRelease {
                asset_pattern,
                invert_asset_pattern,
                prerelease,
                ..
            } => {
                assert!(asset_pattern.is_none());
                assert!(invert_asset_pattern.is_none());
                assert!(!prerelease);
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        let canonical = draft.app_canonical_yaml.unwrap();
        assert!(!canonical.contains("asset_pattern"));
        assert!(!canonical.contains("invert_asset_pattern"));

        let blank = latest_draft_with_pattern(Some("   "), false, Some(unique_release_analysis()));
        assert!(!blank.blocking, "{:#?}", blank.diagnostics);
        assert!(matches!(
            &blank.app.artifacts["apk"].source,
            AppArtifactSource::LatestRelease {
                asset_pattern: None,
                ..
            }
        ));
    }

    #[test]
    fn kind_only_latest_release_filtering_still_requires_exactly_one_apk() {
        let zero = latest_draft_with_pattern(
            None,
            false,
            Some(release_analysis(vec![trusted_release("v1", false, &[])])),
        );
        assert!(zero.blocking);
        assert!(zero
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "latest_release_current_no_match"));

        let multiple = latest_draft_with_pattern(
            None,
            false,
            Some(release_analysis(vec![trusted_release(
                "v1",
                false,
                &["b.apk", "a.apk"],
            )])),
        );
        assert!(multiple.blocking);
        assert!(multiple.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_current_multiple_matches"
                && diagnostic.message.contains("a.apk, b.apk")
                && diagnostic.message.contains("eligible")
        }));
    }

    #[test]
    fn latest_release_requires_the_runtime_supported_service_origin() {
        let mut self_hosted = source();
        self_hosted.mode = "github_repository".to_string();
        self_hosted.strategy = "latest_compatible_release".to_string();
        self_hosted.base_url = Some("https://github.example.com".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: self_hosted,
            release_analysis: Some(unique_release_analysis()),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_base_url_unsupported"));

        let mut self_hosted_gitlab = source();
        self_hosted_gitlab.mode = "gitlab_repository".to_string();
        self_hosted_gitlab.strategy = "latest_compatible_release".to_string();
        self_hosted_gitlab.provider = Some("gitlab".to_string());
        self_hosted_gitlab.base_url = Some("https://gitlab.example.com".to_string());
        self_hosted_gitlab.repository = Some("group/project".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: self_hosted_gitlab,
            release_analysis: None,
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_base_url_unsupported"));

        let mut canonical = source();
        canonical.mode = "github_repository".to_string();
        canonical.strategy = "latest_compatible_release".to_string();
        canonical.base_url = Some("https://GitHub.com/".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: canonical,
            release_analysis: Some(unique_release_analysis()),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(!draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_base_url_unsupported"));
    }

    #[test]
    fn ambiguous_reviewed_artifact_selection_blocks_generation() {
        let pinned = source();
        let mut app = proposed_app(&facts(), &pinned);
        // The author renamed the generated artifact and added a second
        // artifact with the same strategy, so the reviewed artifact can no
        // longer be identified.
        let renamed = app
            .artifacts
            .shift_remove("apk")
            .expect("proposed artifact");
        app.artifacts.insert("renamed_apk".to_string(), renamed);
        app.artifacts.insert(
            "extra_apk".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::Apk,
                name: Some("Extra APK".to_string()),
                description: None,
                source: AppArtifactSource::DirectUrl {
                    url: "https://github.com/example/project/releases/download/v1/extra.apk"
                        .to_string(),
                    sha256: None,
                },
            },
        );

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: pinned.clone(),
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "remote_source_artifact_ambiguous"));

        // A rename that stays unambiguous still pairs with the reviewed source.
        let mut renamed_only = proposed_app(&facts(), &pinned);
        let renamed = renamed_only
            .artifacts
            .shift_remove("apk")
            .expect("proposed artifact");
        renamed_only
            .artifacts
            .insert("renamed_apk".to_string(), renamed);
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: pinned,
            release_analysis: None,
            app: Some(renamed_only),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft.app.artifacts.contains_key("renamed_apk"));
    }

    #[test]
    fn an_inverted_pattern_on_an_unrelated_artifact_does_not_block_generation() {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some(r"^app-v1.*\.apk$".to_string());
        let mut app = proposed_app(&facts(), &latest);
        app.artifacts.insert(
            "extra_notes".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::File,
                name: Some("Release notes".to_string()),
                description: None,
                source: AppArtifactSource::LatestRelease {
                    provider: ReleaseProvider::Github,
                    base_url: "https://github.com".to_string(),
                    repository: "example/project".to_string(),
                    asset_pattern: Some(r"^app-v1.*\.txt$".to_string()),
                    invert_asset_pattern: Some(true),
                    prerelease: false,
                },
            },
        );

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            release_analysis: Some(unique_release_analysis()),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(!draft
            .diagnostics
            .iter()
            .any(|item| item.code == "latest_release_invert_unsupported"));
    }

    #[test]
    fn a_reviewed_artifact_retyped_as_a_file_blocks_generation() {
        let pinned = source();
        let mut app = proposed_app(&facts(), &pinned);
        app.artifacts
            .get_mut("apk")
            .expect("proposed artifact")
            .kind = AppArtifactKind::File;

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: pinned,
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "remote_source_artifact_missing"));
    }

    #[test]
    fn remote_user_provided_generation_requires_the_user_provided_artifact() {
        let mut user_provided = source();
        user_provided.mode = "direct_apk".to_string();
        user_provided.strategy = "user_provided_apk".to_string();

        let mut app = proposed_app(&facts(), &user_provided);
        app.artifacts.clear();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: user_provided.clone(),
            release_analysis: None,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|item| item.code == "remote_source_artifact_missing"));

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: user_provided,
            release_analysis: None,
            app: None,
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
    }
}

//! Side-effect-free app-definition and recipe draft generation from inspected
//! remote APK sources.
//!
//! Remote generation keeps every intrinsic app fact in the App Definition:
//! package identity, the download policy of the reviewed remote source, the
//! verified launcher activity, and the author's permission automation all live
//! there. The generated recipe references the App Definition and one named APK
//! artifact, so the canonical validation that guards authored catalog content
//! also guards generated documents, and no generated recipe duplicates an app
//! fact the App Definition already owns.
use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::authored_models::{
    emit_app_definition_yaml, is_valid_identifier, validate_app_definition, AppArtifactKind,
    AppArtifactSource, AppArtifactV1, AppDefinitionV1, OrderedValueMap, ReleaseProvider,
    APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use crate::model::{
    AppArtifactInputBinding, AppArtifactReference, InputDeclaration, InputValidation, OrderedMap,
    ParamValue, Recipe, RecipeArtifact, RecipeProvides, Step, StepCondition, StepConstraints,
};
use crate::validation::normalize_expected_sha256;
use indexmap::IndexMap;

use super::apk::ApkInspectionFacts;
use super::app_recipe::{
    app_permission_sets, append_permission_steps, collect_artifacts, collect_targets, error,
    generated_apk_inspection_metadata, merge_additive_permission_sets, pair_reviewed_artifact,
    parse_mapping, verified_launcher_activity, warning, AppArtifactEdit, AppTargetEdit,
    DraftDiagnostic, DraftSeverity, PermissionAutomationSelection, ReviewedArtifactPairing,
};
use super::identifiers::{normalize_identifier_component, recipe_local_token};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteAppRecipeDraftRequest {
    pub facts: ApkInspectionFacts,
    pub source: RemoteSource,
    #[serde(default)]
    release_analysis: Option<TrustedReleaseAnalysis>,
    /// Whether the author enabled permission automation for this request.
    ///
    /// Permission automation is opt-in, so an absent flag means disabled and
    /// the selection below only carries the trusted action identities.
    #[serde(default)]
    pub permission_automation_enabled: bool,
    #[serde(default)]
    pub permission_automation: Option<PermissionAutomationSelection>,
    #[serde(default)]
    pub app: Option<AppDefinitionV1>,
    #[serde(default)]
    pub existing_app: Option<AppDefinitionV1>,
    #[serde(default)]
    pub app_artifact_id: Option<String>,
    #[serde(default)]
    pub recipe_id: Option<String>,
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

/// Proposed authored destination of one generated document.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProposedDestination {
    pub(crate) file_name: Option<String>,
    pub(crate) relative_path: Option<String>,
}

/// One generated remote-source draft.
///
/// Fields consumed when a review candidate is built are visible within the
/// crate; the remaining revision state stays private to this module.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAppRecipeDraft {
    pub(crate) app: AppDefinitionV1,
    /// Transient APK inspection evidence shown during draft review.
    ///
    /// The evidence never becomes part of the saved app definition, so the
    /// canonical YAML an author reviews matches the YAML that is written.
    pub(crate) apk_inspection: Option<Value>,
    pub(crate) recipe: Value,
    recipe_edits: RemoteRecipeEdits,
    app_canonical_yaml: Option<String>,
    pub(crate) recipe_canonical_yaml: Option<String>,
    pub(crate) app_destination: ProposedDestination,
    pub(crate) recipe_destination: ProposedDestination,
    evidence: Vec<FieldEvidence>,
    pub(crate) diagnostics: Vec<DraftDiagnostic>,
    pub(crate) blocking: bool,
}

pub(crate) fn generate_remote_app_recipe_draft(
    request: RemoteAppRecipeDraftRequest,
) -> RemoteAppRecipeDraft {
    let existing_mode = request.existing_app.is_some();
    let permission_automation_enabled = request.permission_automation_enabled;
    let requested_selection = request
        .permission_automation
        .as_ref()
        .filter(|selection| !selection.is_empty());
    let mut diagnostics = Vec::new();
    // Permission automation is opt-in. The request flag states the author's
    // intent and the selection only carries trusted action identities, so a
    // disabled request never adds actions: selected actions with the flag off
    // are contradictory and block instead of being silently dropped.
    if !permission_automation_enabled {
        if requested_selection.is_some() {
            diagnostics.push(error(
                "apk_permission_automation_disabled",
                "Permission automation is disabled, so selected permission actions cannot be applied. Enable permission automation or clear the selected actions.",
                "recipe.permissionAutomation.enabled",
            ));
        }
    } else if let Some(selection) = requested_selection {
        diagnostics.extend(
            selection
                .validation_issues()
                .into_iter()
                .map(|issue| error(issue.code, issue.message, &issue.field)),
        );
        if request.facts.package_name.as_deref() != Some(selection.package_name.as_str()) {
            diagnostics.push(error(
                "apk_permission_automation_package_mismatch",
                "Permission automation must use the package name from the inspected APK manifest.",
                "recipe.permissionAutomation.packageName",
            ));
        }
    }

    let requested_artifact_id = request
        .app_artifact_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let artifact_id = match &requested_artifact_id {
        Some(value) => value.clone(),
        None if existing_mode => proposed_existing_artifact_id(&request.facts),
        None => DEFAULT_APP_ARTIFACT_ID.to_string(),
    };
    if !is_valid_identifier(&artifact_id) {
        diagnostics.push(error(
            "app_artifact_id_invalid",
            "App Artifact ID must be a valid authored identifier.",
            "appArtifactId",
        ));
    }

    let proposed_app = proposed_app(&request.facts, &artifact_id, &request.source);
    let mut app = if existing_mode {
        if request.app.is_some() {
            diagnostics.push(error(
                "existing_app_edits_unsupported",
                "Existing-app generation preserves the selected App Definition and edits only the new artifact and additive permission actions.",
                "app",
            ));
        }
        if request.mappings.is_some() {
            diagnostics.push(error(
                "existing_app_mapping_edits_unsupported",
                "Existing-app generation cannot rewrite the selected App Definition artifact or target maps.",
                "mappings",
            ));
        }
        request
            .existing_app
            .clone()
            .unwrap_or_else(|| proposed_app.clone())
    } else {
        request.app.clone().unwrap_or_else(|| proposed_app.clone())
    };

    // The App Definition package identity must match the inspected APK
    // manifest: generation installs the reviewed APK, so a mismatched package
    // cannot produce an installable pair, whether the App Definition is
    // authored or selected.
    if let Some(inspected_package) = request
        .facts
        .package_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if app.package_id.trim() != inspected_package {
            diagnostics.push(error(
                "app_package_mismatch",
                "The inspected APK package name must match the App Definition package_id; choose the App Definition that owns the inspected package or start over from the correct APK.",
                "packageId",
            ));
        }
    }

    let artifact_id_is_valid = is_valid_identifier(&artifact_id);
    if existing_mode {
        if app.artifacts.contains_key(&artifact_id) {
            diagnostics.push(error(
                "app_artifact_id_collision",
                "The selected App Definition already declares this App Artifact ID; choose a different App Artifact ID.",
                "appArtifactId",
            ));
        } else if artifact_id_is_valid {
            if let Some(policy) = proposed_artifact_source(&request.source) {
                let artifact = proposed_apk_artifact(&app.name, policy);
                app.artifacts.insert(artifact_id.clone(), artifact);
            }
        }
    } else if let Some(mappings) = request.mappings {
        apply_mapping_edits(&mut app, mappings, &mut diagnostics);
    }

    // The generated recipe installs the reviewed APK, so the App Definition
    // must declare exactly one artifact representing the reviewed remote
    // source, and it must be the artifact the author selected. The selected
    // App Artifact ID is authoritative: when artifact edits declare the
    // reviewed source under a different ID, generation blocks instead of
    // emitting a recipe that references a missing artifact.
    let reviewed_source_kind = artifact_strategy_name(&request.source.strategy);
    match pair_reviewed_artifact(&app, &proposed_app, |artifact| {
        // The generated recipe installs an APK, so only an APK artifact can
        // represent the reviewed source: a generic file with the same source
        // strategy must not pair, or it would satisfy the presence check while
        // the App Definition declares no artifact the Recipe installs.
        artifact.kind == AppArtifactKind::Apk
            && reviewed_source_kind
                .is_some_and(|expected| app_artifact_strategy_name(&artifact.source) == expected)
    }) {
        ReviewedArtifactPairing::Paired(paired_id) => {
            if paired_id != artifact_id {
                diagnostics.push(error(
                    "app_artifact_id_mismatch",
                    "The App Definition declares the reviewed remote APK artifact under a different App Artifact ID; set appArtifactId to the declared artifact ID.",
                    "appArtifactId",
                ));
            }
        }
        ReviewedArtifactPairing::Missing => {
            if reviewed_source_kind.is_some() {
                diagnostics.push(error(
                    "remote_source_artifact_missing",
                    "The reviewed app definition no longer declares the remote artifact this source installs. Keep the remote artifact, or start a new draft for a user-provided APK.",
                    "source.strategy",
                ));
            }
        }
        ReviewedArtifactPairing::Ambiguous => {
            if reviewed_source_kind.is_some() {
                diagnostics.push(error(
                    "remote_source_artifact_ambiguous",
                    "The reviewed app definition declares more than one artifact for this installation method, so the reviewed artifact is ambiguous. Keep exactly one artifact that represents this source.",
                    "source.strategy",
                ));
            }
        }
    }

    // The reviewed App Definition artifact owns the download policy, so
    // generation validates the policy the pair will save: the source selection
    // only fills fields the artifact leaves blank, and the reviewed release
    // evidence only confirms the filename policy of the saved artifact.
    let source = reviewed_remote_source(&mut app, &artifact_id, &request.source);
    let inverted_pattern = artifact_inversion(&app, &artifact_id) == Some(true);
    diagnostics.extend(validate_source(
        &source,
        &request.source,
        request.release_analysis.as_ref(),
        inverted_pattern,
    ));

    // Permission automation is reviewed evidence plus canonical App permission
    // sets: both come from an enabled selection that validates and names the
    // inspected package, and the evidence stays transient (never written to
    // persisted App metadata). A disabled request verifies nothing, so every
    // permission set the selected App Definition already declares stays
    // exactly as authored and no grant step is generated.
    let verified_selection = requested_selection
        .filter(|_| permission_automation_enabled)
        .filter(|selection| {
            selection.validation_issues().is_empty()
                && request.facts.package_name.as_deref() == Some(selection.package_name.as_str())
        });
    let apk_inspection = match generated_apk_inspection_metadata(
        &request.facts,
        verified_selection,
        permission_automation_enabled,
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

    if let Some(selection) = verified_selection {
        if existing_mode {
            merge_additive_permission_sets(&mut app, selection, &mut diagnostics);
        } else {
            app.permission_sets = app_permission_sets(selection);
        }
    }

    let mut recipe_edits = request
        .recipe
        .unwrap_or_else(|| proposed_recipe_edits(&app, &artifact_id));
    if recipe_edits.ids.is_none() || request.regenerate_identifiers {
        recipe_edits.ids = Some(generated_ids(&app.id, &artifact_id));
    }
    if let Some(explicit_recipe_id) = request
        .recipe_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !is_valid_identifier(explicit_recipe_id) {
            diagnostics.push(error(
                "recipe_id_invalid",
                "Recipe ID must use authored identifier syntax.",
                "recipeId",
            ));
        } else {
            let mut ids = recipe_edits
                .ids
                .clone()
                .unwrap_or_else(|| generated_ids(&app.id, &artifact_id));
            ids.recipe_id = explicit_recipe_id.to_string();
            recipe_edits.ids = Some(ids);
        }
    }
    fill_empty_recipe_text(&mut recipe_edits, &app);
    if existing_mode {
        if recipe_edits.launch_enabled && app.launcher_activity.is_none() {
            diagnostics.push(error(
                "app_launcher_activity_required",
                "Launch-once generation requires the selected App Definition to declare the verified launcher activity.",
                "recipe.launchEnabled",
            ));
        }
    } else {
        app.launcher_activity = verified_launcher_activity(
            &request.facts,
            recipe_edits.launch_enabled,
            recipe_edits.launcher_activity.as_deref(),
        );
        if recipe_edits.launch_enabled && app.launcher_activity.is_none() {
            diagnostics.push(error(
                "apk_launcher_unverified",
                "Launch-once generation requires a launcher component verified by APK inspection.",
                "recipe.launcherActivity",
            ));
        }
    }

    diagnostics.extend(app_diagnostics(&app));
    let recipe = build_recipe(
        &app,
        &artifact_id,
        &recipe_edits,
        permission_automation_enabled,
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
        evidence: evidence(&request.facts, &request.source, &proposed_app, &app),
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
    inverted_pattern: bool,
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
                    "Latest compatible release requires the release provider's canonical service origin.",
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
                diagnostics.extend(release_pattern_diagnostics(
                    source,
                    release_analysis,
                    inverted_pattern,
                ));
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
    // The download address is the artifact address for a direct source, and
    // optional context for the other strategies, so a blank address is only
    // meaningful outside the direct strategy.
    if source.strategy == "pinned_remote_asset" || !source.download_url.trim().is_empty() {
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
    inverted_pattern: bool,
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
            .filter(|file_name| match expression.as_ref() {
                // An inverted policy installs the assets the positive pattern
                // excludes, so the resolver still has to find exactly one
                // installable asset.
                Some(expression) => expression.is_match(file_name) != inverted_pattern,
                None => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        matching_names.sort();
        let qualifier = if expression.is_none() {
            "eligible"
        } else if inverted_pattern {
            "outside-the-pattern"
        } else {
            "matching the pattern"
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

fn proposed_app(
    facts: &ApkInspectionFacts,
    artifact_id: &str,
    source: &RemoteSource,
) -> AppDefinitionV1 {
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
    if let Some(policy) = proposed_artifact_source(source) {
        let key = if is_valid_identifier(artifact_id) {
            artifact_id.to_string()
        } else {
            DEFAULT_APP_ARTIFACT_ID.to_string()
        };
        artifacts.insert(key, proposed_apk_artifact(&name, policy));
    }
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

/// Proposed APK artifact entry for one app name and download policy.
fn proposed_apk_artifact(app_name: &str, source: AppArtifactSource) -> AppArtifactV1 {
    AppArtifactV1 {
        kind: AppArtifactKind::Apk,
        name: (!app_name.trim().is_empty()).then(|| format!("{app_name} APK")),
        description: None,
        source,
    }
}

/// The download policy the selected remote source declares, or None when the
/// selection does not name one.
///
/// Generation never substitutes a different policy: an unsupported selection
/// blocks with its own diagnostic and leaves the draft without an artifact for
/// that source instead of saving a policy the author never selected.
fn proposed_artifact_source(source: &RemoteSource) -> Option<AppArtifactSource> {
    match source.strategy.as_str() {
        "pinned_remote_asset" => Some(AppArtifactSource::DirectUrl {
            url: source.download_url.trim().to_string(),
            sha256: source
                .trusted_sha256
                .as_deref()
                .and_then(normalize_expected_sha256)
                .map(|value| value.to_ascii_lowercase()),
        }),
        "latest_compatible_release" => {
            let provider = release_provider(source.provider.as_deref())?;
            Some(AppArtifactSource::LatestRelease {
                provider,
                base_url: source.base_url.clone().unwrap_or_default(),
                repository: source.repository.clone().unwrap_or_default(),
                asset_pattern: normalize_asset_pattern(source.asset_pattern.as_deref()),
                invert_asset_pattern: None,
                prerelease: source.include_prereleases,
            })
        }
        "user_provided_apk" => Some(AppArtifactSource::UserProvided),
        _ => None,
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

/// Return the download policy the reviewed App Definition artifact declares.
///
/// The reviewed artifact is authoritative: fields edited there win, and the
/// source selection only fills fields the artifact leaves blank, so the
/// validated policy always describes the App Definition that will be saved.
/// Blank artifact fields are filled from the source selection, so an
/// author-supplied checksum or repository is retained no matter which input
/// produced it and the two generated documents cannot disagree.
fn reviewed_remote_source(
    app: &mut AppDefinitionV1,
    artifact_id: &str,
    selection: &RemoteSource,
) -> RemoteSource {
    let Some(artifact) = app.artifacts.get_mut(artifact_id) else {
        return selection.clone();
    };
    let mut effective = selection.clone();
    match (&mut artifact.source, selection.strategy.as_str()) {
        (AppArtifactSource::DirectUrl { url, sha256 }, "pinned_remote_asset") => {
            if url.trim() == selection.download_url.trim() && sha256.is_none() {
                *sha256 = selection
                    .trusted_sha256
                    .as_deref()
                    .and_then(normalize_expected_sha256)
                    .map(|value| value.to_ascii_lowercase());
            }
            effective.download_url = url.clone();
            if sha256.is_some() {
                effective.trusted_sha256 = sha256.clone();
            } else if url.trim() != selection.download_url.trim() {
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
                if let Some(selected_repository) = selection.repository.as_deref() {
                    *repository = selected_repository.to_string();
                }
            }
            if base_url.trim().is_empty() {
                if let Some(selected_base_url) = selection.base_url.as_deref() {
                    *base_url = selected_base_url.to_string();
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

/// Artifact ID proposed for a brand-new App Definition.
const DEFAULT_APP_ARTIFACT_ID: &str = "apk";

/// Deterministic App Artifact ID proposed for existing-app addition.
///
/// The proposal derives from the inspected application label so identical
/// authored inputs always produce the same artifact identity; the author can
/// replace it with any valid identifier before review.
fn proposed_existing_artifact_id(facts: &ApkInspectionFacts) -> String {
    let label = facts.application_label.as_deref().unwrap_or_default();
    let normalized = normalize_identifier_component(label);
    if normalized.is_empty() {
        DEFAULT_APP_ARTIFACT_ID.to_string()
    } else {
        normalized
    }
}

fn generated_ids(app_id: &str, artifact_id: &str) -> GeneratedRecipeIds {
    let token = recipe_local_token(app_id);
    let artifact_token = recipe_local_token(artifact_id);
    let base = if artifact_id == DEFAULT_APP_ARTIFACT_ID {
        token.clone()
    } else {
        format!("{token}_{artifact_token}")
    };
    GeneratedRecipeIds {
        recipe_id: if artifact_id == DEFAULT_APP_ARTIFACT_ID {
            format!("app.{app_id}.install")
        } else {
            format!("app.{app_id}.{artifact_token}.install")
        },
        input_id: format!("{base}_apk"),
        feature_id: format!("{base}_install"),
        install_step_id: format!("install_{base}"),
        permission_step_id: format!("grant_permissions_{base}"),
        launch_step_id: format!("launch_{base}"),
    }
}

/// Deterministic artifact-resolution step id for one generated recipe.
fn resolve_step_id(app_id: &str, artifact_id: &str) -> String {
    let token = recipe_local_token(app_id);
    if artifact_id == DEFAULT_APP_ARTIFACT_ID {
        format!("resolve_{token}")
    } else {
        format!("resolve_{token}_{}", recipe_local_token(artifact_id))
    }
}

fn proposed_recipe_edits(app: &AppDefinitionV1, artifact_id: &str) -> RemoteRecipeEdits {
    RemoteRecipeEdits {
        ids: Some(generated_ids(&app.id, artifact_id)),
        name: format!("Install {}", app.name),
        description: format!("Install {} from its App Definition APK artifact.", app.name),
        input_label: format!("{} APK", app.name),
        input_description: format!("Local {} APK to install.", app.name),
        ..RemoteRecipeEdits::default()
    }
}

fn fill_empty_recipe_text(edits: &mut RemoteRecipeEdits, app: &AppDefinitionV1) {
    if edits.name.trim().is_empty() {
        edits.name = format!("Install {}", app.name);
    }
    if edits.description.trim().is_empty() {
        edits.description = format!("Install {} from its App Definition APK artifact.", app.name);
    }
    if edits.input_label.trim().is_empty() {
        edits.input_label = format!("{} APK", app.name);
    }
}

/// Build one generated recipe for an App Definition APK artifact.
///
/// Permission grant steps are generated only when the request enabled
/// permission automation: a disabled request installs and launches the App
/// without applying any permission set, even when the App Definition declares
/// sets.
fn build_recipe(
    app: &AppDefinitionV1,
    artifact_id: &str,
    edits: &RemoteRecipeEdits,
    permission_automation_enabled: bool,
) -> Recipe {
    let ids = edits
        .ids
        .clone()
        .unwrap_or_else(|| generated_ids(&app.id, artifact_id));
    let user_provided = app
        .artifacts
        .get(artifact_id)
        .is_some_and(|artifact| matches!(artifact.source, AppArtifactSource::UserProvided));
    let mut inputs = OrderedMap::new();
    if user_provided {
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
    }

    let mut artifacts = OrderedMap::new();
    artifacts.insert(
        artifact_id.to_string(),
        RecipeArtifact::AppArtifact(AppArtifactReference {
            app_ref: app.id.clone(),
            artifact: artifact_id.to_string(),
            input: user_provided.then(|| AppArtifactInputBinding {
                input_id: ids.input_id.clone(),
            }),
            cache: if user_provided {
                "none".to_string()
            } else {
                "default".to_string()
            },
        }),
    );

    let resolve_id = resolve_step_id(&app.id, artifact_id);
    let mut resolve_params = OrderedMap::new();
    resolve_params.insert(
        "artifacts".to_string(),
        ParamValue::Literal(Value::Array(vec![Value::String(artifact_id.to_string())])),
    );
    let mut steps = vec![Step {
        id: resolve_id.clone(),
        type_name: "resolve_artifacts".to_string(),
        name: format!("Resolve {} artifact", app.name),
        description: None,
        progress_note: None,
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
    }];

    let mut install_params = OrderedMap::new();
    install_params.insert(
        "app".to_string(),
        ParamValue::Ref(format!("artifacts.{artifact_id}.local_path")),
    );
    install_params.insert(
        "replace_existing".to_string(),
        ParamValue::Literal(Value::Bool(edits.replace_existing)),
    );
    steps.push(Step {
        id: ids.install_step_id.clone(),
        type_name: "install_apk".to_string(),
        name: format!("Install {} APK", app.name),
        description: None,
        progress_note: Some(format!("Installing {} on the selected device", app.name)),
        user_toggleable: false,
        app_ref: Some(app.id.clone()),
        dependencies: vec![resolve_id],
        constraints: StepConstraints {
            capabilities: vec!["apk_install".to_string()],
            conflicts_with: Vec::new(),
        },
        skip_if: vec![StepCondition {
            type_name: "package_installed".to_string(),
            params: OrderedMap::new(),
        }],
        params: install_params,
        verify: Vec::new(),
    });

    if edits.launch_enabled && app.launcher_activity.is_some() {
        steps.push(Step {
            id: ids.launch_step_id.clone(),
            type_name: "launch_app".to_string(),
            name: format!("Launch {} once", app.name),
            description: Some("Launch the app once after installation.".to_string()),
            progress_note: Some(format!("Launching {} once", app.name)),
            user_toggleable: false,
            app_ref: Some(app.id.clone()),
            dependencies: vec![ids.install_step_id.clone()],
            constraints: StepConstraints {
                capabilities: vec!["app_launch".to_string()],
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: OrderedMap::new(),
            verify: Vec::new(),
        });
    }

    if permission_automation_enabled {
        append_permission_steps(&mut steps, app, &ids.permission_step_id, &ids.install_step_id);
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
    use crate::authored_models::{
    AppOpMode, AppPermissionSet, AppPermissionSets, AppRuntimePermissionAction, AppTargetKind,
    AppTargetLocation, AppTargetV1,
    };
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

    fn latest_source() -> RemoteSource {
        let mut latest = source();
        latest.mode = "github_repository".to_string();
        latest.strategy = "latest_compatible_release".to_string();
        latest.asset_pattern = Some("^app-v.*-arm64\\.apk$".to_string());
        latest
    }

    fn user_provided_source() -> RemoteSource {
        let mut provided = source();
        provided.mode = "direct_apk".to_string();
        provided.strategy = "user_provided_apk".to_string();
        provided.download_url = String::new();
        provided
    }

    fn request(source: RemoteSource) -> RemoteAppRecipeDraftRequest {
        RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
            release_analysis: None,
            permission_automation_enabled: false,
            permission_automation: None,
            app: None,
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            recipe: None,
            mappings: None,
            regenerate_identifiers: false,
        }
    }

    /// SHA-256 hex digest used for deterministic byte comparisons.
    fn permission_sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(bytes);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Canonical documents, diagnostics, and step lines of one generated draft.
    fn permission_artifact_sections(draft: &RemoteAppRecipeDraft) -> [String; 4] {
        [
            draft
                .app_canonical_yaml
                .as_deref()
                .unwrap_or("<none>")
                .to_string(),
            draft
                .recipe_canonical_yaml
                .as_deref()
                .unwrap_or("<none>")
                .to_string(),
            serde_json::to_string(&draft.diagnostics).expect("diagnostics serialize"),
            permission_step_lines(&draft.recipe),
        ]
    }

    /// Deterministic text block pinning one draft document set for comparison.
    fn permission_document_block(draft: &RemoteAppRecipeDraft) -> String {
        use std::fmt::Write as _;
        let [app_yaml, recipe_yaml, diagnostics, steps] = permission_artifact_sections(draft);
        let mut block = String::new();
        writeln!(block, "APP_YAML").expect("block write");
        writeln!(block, "{app_yaml}").expect("block write");
        writeln!(block, "RECIPE_YAML").expect("block write");
        writeln!(block, "{recipe_yaml}").expect("block write");
        writeln!(block, "DIAGNOSTICS").expect("block write");
        writeln!(block, "{diagnostics}").expect("block write");
        writeln!(block, "STEPS").expect("block write");
        writeln!(block, "{steps}").expect("block write");
        block
    }

    /// One line per generated step: identifier, type, and dependencies.
    fn permission_step_lines(recipe: &Value) -> String {
        recipe
            .get("steps")
            .and_then(|steps| steps.as_array())
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| {
                        let id = step
                            .get("id")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<none>");
                        let type_name = step
                            .get("type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<none>");
                        let dependencies = step
                            .get("dependencies")
                            .and_then(|value| value.as_array())
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| item.as_str())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_default();
                        format!("{id}|{type_name}|{dependencies}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    #[test]
    fn permission_step_generation_pins_remote_canonical_documents_and_merge_diagnostics() {
        let mut draft_request = request(source());
        draft_request.app = Some(proposed_draft_app(&source()));
        draft_request.permission_automation_enabled = true;
        draft_request.permission_automation = Some(permission_automation(
            vec![
                runtime_permission("android.permission.RECORD_AUDIO", false),
                runtime_permission("android.permission.CAMERA", true),
            ],
            vec![
                app_op(
                    "android.permission.PACKAGE_USAGE_STATS",
                    "android:get_usage_stats",
                    false,
                ),
                app_op(
                    "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "MANAGE_EXTERNAL_STORAGE",
                    true,
                ),
            ],
        ));
        let generated = generate_remote_app_recipe_draft(draft_request);
        println!("=== CASE remote:new-app ===");
        println!("{}", permission_document_block(&generated));
        println!("=== END CASE remote:new-app ===");
        let [app_yaml, recipe_yaml, diagnostics, steps] = permission_artifact_sections(&generated);
        assert_eq!(permission_sha256_hex(app_yaml.as_bytes()), "24ac7346098eb4edae5b86d99604ecd7f923c3714379737d6f75bd4d0defd632");
        assert_eq!(permission_sha256_hex(recipe_yaml.as_bytes()), "963ce5ca92bf07b0574fab036b0a3549fd5068ea50e0c96101df66c14b573fd0");
        assert_eq!(permission_sha256_hex(diagnostics.as_bytes()), "7cb7f244325c8fd86175ba28c9a645b3b47ac8f689a1a1039c7e4c77f0ef7d04");
        assert_eq!(permission_sha256_hex(steps.as_bytes()), "3e25ea4bd4531cc5cf0b25f05374ab474e99242da53a24fdb6601a52b693de40");
        println!(
            "HASH remote:new-app app={} recipe={} diagnostics={} steps={}",
            permission_sha256_hex(app_yaml.as_bytes()),
            permission_sha256_hex(recipe_yaml.as_bytes()),
            permission_sha256_hex(diagnostics.as_bytes()),
            permission_sha256_hex(steps.as_bytes()),
        );

        let mut bound_conflict = existing_app();
        bound_conflict.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.RECORD_AUDIO".to_string(),
                    android_api_min: Some(29),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let mut conflict_request = request(source());
        conflict_request.existing_app = Some(bound_conflict);
        conflict_request.app_artifact_id = Some("added_apk".to_string());
        conflict_request.permission_automation_enabled = true;
        conflict_request.permission_automation = Some(permission_automation(
            vec![
                runtime_permission("android.permission.RECORD_AUDIO", false),
                runtime_permission("android.permission.CAMERA", true),
            ],
            Vec::new(),
        ));
        let merged = generate_remote_app_recipe_draft(conflict_request);
        println!("=== CASE remote:existing-app-merge ===");
        println!("{}", permission_document_block(&merged));
        println!("=== END CASE remote:existing-app-merge ===");
        let [app_yaml, recipe_yaml, diagnostics, steps] = permission_artifact_sections(&merged);
        assert_eq!(permission_sha256_hex(app_yaml.as_bytes()), "a9f14ed9fedeca7e675456eda91bf4e57995acc258d284789e0a19074d5a0f16");
        assert_eq!(permission_sha256_hex(recipe_yaml.as_bytes()), "6e9c960fd025b2e5b1cbceebc7e7b31d041c4c352987f3b56b07f14ad43abca7");
        assert_eq!(permission_sha256_hex(diagnostics.as_bytes()), "ea0089e46b7e4d0449e5dbb0abd86c8554961caf4a93b7ac1948fdce2ab4021c");
        assert_eq!(permission_sha256_hex(steps.as_bytes()), "b08bc926edfaded77e36818f85d727cb5fee39695dc1381b1f36d77520a87bc9");
        println!(
            "HASH remote:existing-app-merge app={} recipe={} diagnostics={} steps={}",
            permission_sha256_hex(app_yaml.as_bytes()),
            permission_sha256_hex(recipe_yaml.as_bytes()),
            permission_sha256_hex(diagnostics.as_bytes()),
            permission_sha256_hex(steps.as_bytes()),
        );
    }

    fn proposed_draft_app(source: &RemoteSource) -> AppDefinitionV1 {
        let mut app = proposed_app(&facts(), DEFAULT_APP_ARTIFACT_ID, source);
        app.category = Some("emulator".to_string());
        app
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

    fn latest_draft_with_pattern(
        pattern: Option<&str>,
        include_prereleases: bool,
        release_analysis: Option<TrustedReleaseAnalysis>,
    ) -> RemoteAppRecipeDraft {
        let mut latest = latest_source();
        latest.asset_pattern = pattern.map(str::to_string);
        latest.include_prereleases = include_prereleases;
        let mut draft_request = request(latest);
        draft_request.release_analysis = release_analysis;
        generate_remote_app_recipe_draft(draft_request)
    }

    fn latest_draft(
        pattern: &str,
        include_prereleases: bool,
        release_analysis: Option<TrustedReleaseAnalysis>,
    ) -> RemoteAppRecipeDraft {
        latest_draft_with_pattern(Some(pattern), include_prereleases, release_analysis)
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

    fn launch_edits() -> RemoteRecipeEdits {
        RemoteRecipeEdits {
            launch_enabled: true,
            launcher_activity: Some("com.example.remote/.MainActivity".to_string()),
            ..RemoteRecipeEdits::default()
        }
    }

    fn existing_app() -> AppDefinitionV1 {
        let mut app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.to_string(),
            id: "remote_example".to_string(),
            name: "Remote Example".to_string(),
            description: Some("Existing authored app.".to_string()),
            category: Some("emulator".to_string()),
            package_id: "com.example.remote".to_string(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: Some("com.example.remote.MainActivity".to_string()),
            metadata: OrderedValueMap::new(),
        };
        app.artifacts.insert(
            "local_apk".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::Apk,
                name: Some("Existing APK".to_string()),
                description: None,
                source: AppArtifactSource::UserProvided,
            },
        );
        app.targets.insert(
            "save_dir".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::Directory,
                location: AppTargetLocation::AppData,
                path: "files/saves".to_string(),
            },
        );
        app.metadata
            .insert("author".to_string(), Value::String("EmuChef".to_string()));
        app.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.RECORD_AUDIO".to_string(),
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        app
    }

    fn step_types(draft: &RemoteAppRecipeDraft) -> Vec<&str> {
        draft.recipe["steps"]
            .as_array()
            .expect("recipe steps")
            .iter()
            .map(|step| step["type"].as_str().expect("step type"))
            .collect()
    }

    fn diagnostic_codes(draft: &RemoteAppRecipeDraft) -> Vec<&str> {
        draft
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect()
    }

    /// Validate the generated App/Recipe pair with the canonical planner
    /// app-authority checks used for authored catalog content, so generation
    /// tests prove the pair is admissible beyond draft diagnostics.
    fn assert_pair_authority(draft: &RemoteAppRecipeDraft) {
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        let mapping =
            serde_yaml::from_str::<serde_yaml::Mapping>(yaml).expect("recipe yaml mapping");
        let recipe =
            crate::yaml::parse_recipe_mapping(&mapping, std::path::Path::new("recipe.yaml"))
                .expect("generated recipe parses");
        let messages =
            crate::planner::validate_recipe_app_authority(&recipe, std::slice::from_ref(&draft.app));
        assert!(
            messages.is_empty(),
            "app authority diagnostics: {messages:?}"
        );
    }

    #[test]
    fn selected_release_asset_maps_to_direct_url_artifact_referenced_by_the_recipe() {
        let draft = generate_remote_app_recipe_draft(request(source()));

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl { url, sha256 }
                if url == "https://github.com/example/project/releases/download/v1/app.apk"
                    && sha256.is_none()
        ));
        assert_eq!(step_types(&draft), vec!["resolve_artifacts", "install_apk"]);
        assert_eq!(draft.recipe["steps"][0]["id"], "resolve_remote_example");
        assert_eq!(
            draft.recipe["steps"][0]["params"]["artifacts"],
            serde_json::json!(["apk"])
        );
        assert_eq!(draft.recipe["steps"][1]["id"], "install_remote_example");
        assert_eq!(
            draft.recipe["steps"][1]["params"]["app"],
            serde_json::json!({ "ref": "artifacts.apk.local_path" })
        );
        assert_eq!(
            draft.recipe["steps"][1]["dependencies"],
            serde_json::json!(["resolve_remote_example"])
        );
        assert_eq!(draft.recipe["artifacts"]["apk"]["type"], "app_artifact");
        assert_eq!(draft.recipe["artifacts"]["apk"]["appRef"], "remote_example");
        assert_eq!(draft.recipe["artifacts"]["apk"]["artifact"], "apk");
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("strategy: direct_url"));
        assert!(app_yaml.contains("url: https://github.com/example/project/releases/download/v1/app.apk"));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(!recipe_yaml.contains("remote_file"));
        assert!(!recipe_yaml.contains("github.com"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn direct_apk_source_keeps_the_original_https_address_in_new_and_existing_apps() {
        let original_url =
            "https://downloads.example.org/emuchef/app-release.apk?build=1042&channel=stable";
        let mut direct = source();
        direct.mode = "direct_apk".to_string();
        direct.download_url = original_url.to_string();
        // Release context that may accompany a direct address is transient: it
        // must never replace the author's address with a derived or mirrored
        // one, and it is never persisted with the App Definition.
        direct.base_url = Some("https://cdn.example.net".to_string());
        direct.repository = Some("example/project".to_string());
        direct.release_tag = Some("v1".to_string());
        direct.asset_name = Some("app.apk".to_string());

        let new_app = generate_remote_app_recipe_draft(request(direct.clone()));

        assert!(!new_app.blocking, "{:#?}", new_app.diagnostics);
        assert!(matches!(
            &new_app.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl { url, sha256: None } if url == original_url
        ));
        let app_yaml = new_app.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("strategy: direct_url"));
        assert_eq!(app_yaml.matches(original_url).count(), 1);
        assert!(!app_yaml.contains("cdn.example.net"));
        assert!(!app_yaml.contains("releases/download"));
        let recipe_yaml = new_app.recipe_canonical_yaml.as_deref().unwrap();
        assert!(!recipe_yaml.contains("https://"));
        assert!(new_app.apk_inspection.is_some());
        assert!(!new_app.app.metadata.contains_key("apk_inspection"));
        assert_pair_authority(&new_app);

        let original = existing_app();
        let mut existing_request = request(direct);
        existing_request.existing_app = Some(original.clone());
        existing_request.app_artifact_id = Some("direct_apk".to_string());

        let added = generate_remote_app_recipe_draft(existing_request);

        assert!(!added.blocking, "{:#?}", added.diagnostics);
        assert!(matches!(
            &added.app.artifacts["direct_apk"].source,
            AppArtifactSource::DirectUrl { url, sha256: None } if url == original_url
        ));
        assert_eq!(added.app.id, original.id);
        assert_eq!(added.app.package_id, original.package_id);
        assert_eq!(added.app.artifacts.len(), 2);
        assert_eq!(
            added.app.artifacts["local_apk"].source,
            AppArtifactSource::UserProvided
        );
        assert_eq!(added.app.targets, original.targets);
        assert_eq!(added.app.metadata, original.metadata);
        assert_eq!(added.app.launcher_activity, original.launcher_activity);
        assert_eq!(added.app.permission_sets, original.permission_sets);
        assert_eq!(added.recipe["id"], "app.remote_example.direct_apk.install");
        assert_eq!(
            step_types(&added),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert_eq!(
            added.recipe["steps"][0]["id"],
            "resolve_remote_example_direct_apk"
        );
        assert_eq!(
            added.recipe["steps"][1]["id"],
            "install_remote_example_direct_apk"
        );
        let app_yaml = added.app_canonical_yaml.as_deref().unwrap();
        assert_eq!(app_yaml.matches(original_url).count(), 1);
        assert!(!app_yaml.contains("cdn.example.net"));
        assert_pair_authority(&added);
    }

    #[test]
    fn selected_release_asset_checksum_is_recorded_in_the_direct_url_artifact() {
        let mut selection = source();
        selection.trusted_sha256 =
            Some(" \t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n".to_string());

        let draft = generate_remote_app_recipe_draft(request(selection));

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let expected = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl { url, sha256: Some(sha256) }
                if url == "https://github.com/example/project/releases/download/v1/app.apk"
                    && sha256 == expected
        ));
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains(&format!("sha256: {expected}")));
        assert_eq!(
            draft.apk_inspection.as_ref().unwrap()["calculated_sha256"],
            "B".repeat(64)
        );
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
    }

    #[test]
    fn repository_latest_source_maps_to_latest_release_artifact() {
        let mut draft_request = request(latest_source());
        draft_request.release_analysis = Some(unique_release_analysis());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        match &draft.app.artifacts["apk"].source {
            AppArtifactSource::LatestRelease {
                provider,
                base_url,
                repository,
                asset_pattern,
                invert_asset_pattern,
                prerelease,
            } => {
                assert_eq!(*provider, ReleaseProvider::Github);
                assert_eq!(base_url, "https://github.com");
                assert_eq!(repository, "example/project");
                assert_eq!(asset_pattern.as_deref(), Some("^app-v.*-arm64\\.apk$"));
                assert!(invert_asset_pattern.is_none());
                assert!(!prerelease);
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        assert_eq!(step_types(&draft), vec!["resolve_artifacts", "install_apk"]);
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("strategy: latest_release"));
        assert!(app_yaml.contains("provider: github"));
        assert!(app_yaml.contains("repository: example/project"));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("type: app_artifact"));
        assert!(!recipe_yaml.contains("resolve_remote_release"));
        assert!(!recipe_yaml.contains("download_remote_file"));
        assert!(!recipe_yaml.contains("example/project"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn user_provided_source_binds_a_recipe_file_input_to_the_artifact() {
        let draft = generate_remote_app_recipe_draft(request(user_provided_source()));

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.artifacts["apk"].source, AppArtifactSource::UserProvided);
        let input = &draft.recipe["inputs"]["remote_example_apk"];
        assert_eq!(input["type"], "file");
        assert_eq!(input["role"], "apk");
        assert_eq!(input["validation"]["mustExist"], true);
        assert_eq!(input["validation"]["allowedExtensions"], serde_json::json!(["apk"]));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("ref: inputs.remote_example_apk"));
        assert!(recipe_yaml.contains("cache: none"));
        assert!(!recipe_yaml.contains("type: remote_file"));
        assert!(!recipe_yaml.contains("expected_package_name"));
        assert!(!recipe_yaml.contains("expected_sha256"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn an_unsupported_source_never_falls_back_to_another_artifact_policy() {
        let mut draft_request = request(source());
        draft_request.source.strategy = "unlisted_strategy".to_string();

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"remote_strategy_invalid"));
        assert!(!diagnostic_codes(&draft).contains(&"remote_source_artifact_missing"));
        assert!(draft.app.artifacts.is_empty());
        assert!(draft.app_canonical_yaml.is_none());
        assert!(draft.app_destination.relative_path.is_none());
        assert_eq!(draft.recipe["artifacts"]["apk"]["type"], "app_artifact");
        assert_eq!(draft.recipe["artifacts"]["apk"]["artifact"], "apk");
    }

    #[test]
    fn explicit_artifact_and_recipe_ids_are_retained_in_the_generated_pair() {
        let mut draft_request = request(source());
        draft_request.app_artifact_id = Some("release_apk".to_string());
        draft_request.recipe_id = Some("remote_example.install.release".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft.app.artifacts.contains_key("release_apk"));
        assert_eq!(draft.recipe["id"], "remote_example.install.release");
        assert_eq!(draft.recipe["artifacts"]["release_apk"]["artifact"], "release_apk");
        assert_eq!(
            draft.recipe_destination.relative_path.as_deref(),
            Some("recipes/remote_example.install.release.yaml")
        );
        assert_eq!(
            draft.app_destination.relative_path.as_deref(),
            Some("apps/remote_example.yaml")
        );
        assert_eq!(
            draft.recipe["steps"][0]["id"],
            "resolve_remote_example_release_apk"
        );
        assert_eq!(
            draft.recipe["steps"][1]["id"],
            "install_remote_example_release_apk"
        );
        assert_pair_authority(&draft);
    }

    #[test]
    fn invalid_artifact_and_recipe_ids_block_generation() {
        let mut draft_request = request(source());
        draft_request.app_artifact_id = Some("Release Apk".to_string());
        draft_request.recipe_id = Some("Release Apk!".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        let codes = diagnostic_codes(&draft);
        assert!(codes.contains(&"app_artifact_id_invalid"));
        assert!(codes.contains(&"recipe_id_invalid"));
        assert!(draft.app_canonical_yaml.is_none());
    }

    #[test]
    fn new_app_generation_blocks_authored_package_mismatch() {
        let mut app = proposed_draft_app(&source());
        app.package_id = "com.example.other".to_string();
        let mut draft_request = request(source());
        draft_request.app = Some(app);

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "app_package_mismatch" && diagnostic.field == "packageId"
        }));
        assert!(draft.app_canonical_yaml.is_none());
    }

    #[test]
    fn existing_app_generation_appends_one_named_apk_artifact_and_preserves_app_facts() {
        let original = existing_app();
        let mut draft_request = request(source());
        draft_request.existing_app = Some(original.clone());
        draft_request.app_artifact_id = Some("added_apk".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.id, "remote_example");
        assert_eq!(draft.app.name, "Remote Example");
        assert_eq!(draft.app.description, original.description);
        assert_eq!(draft.app.category, original.category);
        assert_eq!(draft.app.package_id, "com.example.remote");
        assert_eq!(draft.app.launcher_activity, original.launcher_activity);
        assert_eq!(draft.app.metadata, original.metadata);
        assert_eq!(draft.app.targets, original.targets);
        assert_eq!(
            draft.app.artifacts["local_apk"].source,
            AppArtifactSource::UserProvided
        );
        assert_eq!(
            draft.app.artifacts["local_apk"].name.as_deref(),
            Some("Existing APK")
        );
        assert_eq!(draft.app.permission_sets, original.permission_sets);
        assert_eq!(draft.app.artifacts.len(), 2);
        let added = &draft.app.artifacts["added_apk"];
        assert_eq!(added.kind, AppArtifactKind::Apk);
        assert_eq!(added.name.as_deref(), Some("Remote Example APK"));
        assert!(matches!(added.source, AppArtifactSource::DirectUrl { .. }));
        assert_eq!(draft.recipe["artifacts"]["added_apk"]["artifact"], "added_apk");
        assert_eq!(
            step_types(&draft),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("grant_permissions"));
        assert_pair_authority(&draft);

        let mut derived_request = request(source());
        derived_request.existing_app = Some(original);
        let derived = generate_remote_app_recipe_draft(derived_request);
        assert!(derived.app.artifacts.contains_key("remote_example"));
        assert!(!derived.blocking, "{:#?}", derived.diagnostics);
    }

    #[test]
    fn existing_app_generation_blocks_an_artifact_id_collision_without_replacing_the_artifact() {
        let mut app = existing_app();
        app.artifacts.insert(
            "added_apk".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::Apk,
                name: Some("Author APK".to_string()),
                description: None,
                source: AppArtifactSource::DirectUrl {
                    url: "https://github.com/example/project/releases/download/v0/author.apk"
                        .to_string(),
                    sha256: None,
                },
            },
        );
        let mut draft_request = request(source());
        draft_request.existing_app = Some(app);
        draft_request.app_artifact_id = Some("added_apk".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"app_artifact_id_collision"));
        assert!(matches!(
            &draft.app.artifacts["added_apk"].source,
            AppArtifactSource::DirectUrl { url, .. }
                if url == "https://github.com/example/project/releases/download/v0/author.apk"
        ));
        assert_eq!(draft.app.artifacts["added_apk"].name.as_deref(), Some("Author APK"));
    }

    #[test]
    fn existing_app_generation_blocks_a_mismatched_inspected_package() {
        let mut app = existing_app();
        app.package_id = "com.example.other".to_string();
        let mut draft_request = request(source());
        draft_request.existing_app = Some(app);
        draft_request.app_artifact_id = Some("added_apk".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"app_package_mismatch"));
    }

    #[test]
    fn existing_app_generation_appends_a_latest_release_artifact_with_exact_ids() {
        let original = existing_app();
        let mut draft_request = request(latest_source());
        draft_request.release_analysis = Some(unique_release_analysis());
        draft_request.existing_app = Some(original.clone());
        draft_request.app_artifact_id = Some("release_apk".to_string());
        draft_request.recipe_id = Some("remote_example.install.release".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.id, original.id);
        assert_eq!(draft.app.package_id, original.package_id);
        assert_eq!(draft.app.artifacts.len(), 2);
        assert_eq!(
            draft.app.artifacts["local_apk"].source,
            AppArtifactSource::UserProvided
        );
        match &draft.app.artifacts["release_apk"].source {
            AppArtifactSource::LatestRelease {
                provider,
                base_url,
                repository,
                asset_pattern,
                invert_asset_pattern,
                prerelease,
            } => {
                assert_eq!(*provider, ReleaseProvider::Github);
                assert_eq!(base_url, "https://github.com");
                assert_eq!(repository, "example/project");
                assert_eq!(asset_pattern.as_deref(), Some("^app-v.*-arm64\\.apk$"));
                assert!(invert_asset_pattern.is_none());
                assert!(!prerelease);
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        assert_eq!(draft.app.targets, original.targets);
        assert_eq!(draft.app.metadata, original.metadata);
        assert_eq!(draft.app.launcher_activity, original.launcher_activity);
        assert_eq!(draft.app.permission_sets, original.permission_sets);
        assert_eq!(draft.recipe["id"], "remote_example.install.release");
        assert_eq!(
            draft.recipe["artifacts"]["release_apk"]["appRef"],
            "remote_example"
        );
        assert_eq!(
            draft.recipe["artifacts"]["release_apk"]["artifact"],
            "release_apk"
        );
        assert_eq!(
            step_types(&draft),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert_eq!(
            draft.recipe["steps"][0]["id"],
            "resolve_remote_example_release_apk"
        );
        assert_eq!(
            draft.recipe["steps"][1]["id"],
            "install_remote_example_release_apk"
        );
        assert_eq!(
            draft.recipe_destination.relative_path.as_deref(),
            Some("recipes/remote_example.install.release.yaml")
        );
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("strategy: latest_release"));
        assert!(app_yaml.contains("repository: example/project"));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("type: app_artifact"));
        assert!(!recipe_yaml.contains("example/project"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn existing_app_generation_appends_a_user_provided_artifact_with_exact_ids() {
        let original = existing_app();
        let mut draft_request = request(user_provided_source());
        draft_request.existing_app = Some(original.clone());
        draft_request.app_artifact_id = Some("user_apk".to_string());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.id, original.id);
        assert_eq!(draft.app.artifacts.len(), 2);
        assert_eq!(
            draft.app.artifacts["user_apk"].source,
            AppArtifactSource::UserProvided
        );
        assert_eq!(draft.app.artifacts["user_apk"].kind, AppArtifactKind::Apk);
        assert_eq!(
            draft.app.artifacts["user_apk"].name.as_deref(),
            Some("Remote Example APK")
        );
        assert_eq!(draft.app.targets, original.targets);
        assert_eq!(draft.app.metadata, original.metadata);
        assert_eq!(draft.app.permission_sets, original.permission_sets);
        assert_eq!(draft.recipe["id"], "app.remote_example.user_apk.install");
        assert_eq!(
            draft.recipe["artifacts"]["user_apk"]["artifact"],
            "user_apk"
        );
        assert_eq!(
            draft.recipe["inputs"]["remote_example_user_apk_apk"]["type"],
            "file"
        );
        assert_eq!(
            draft.recipe["inputs"]["remote_example_user_apk_apk"]["role"],
            "apk"
        );
        assert_eq!(
            step_types(&draft),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert_eq!(
            draft.recipe["steps"][0]["id"],
            "resolve_remote_example_user_apk"
        );
        assert_eq!(
            draft.recipe["steps"][1]["id"],
            "install_remote_example_user_apk"
        );
        assert_eq!(
            draft.recipe["steps"][1]["params"]["app"],
            serde_json::json!({ "ref": "artifacts.user_apk.local_path" })
        );
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("ref: inputs.remote_example_user_apk_apk"));
        assert!(recipe_yaml.contains("cache: none"));
        assert!(!recipe_yaml.contains("type: remote_file"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn existing_app_generation_rejects_app_and_mapping_edits() {
        let mut draft_request = request(source());
        draft_request.existing_app = Some(existing_app());
        draft_request.app = Some(proposed_draft_app(&source()));
        draft_request.mappings = Some(
            serde_json::from_value::<RemoteMappingEdits>(serde_json::json!({
                "artifacts": [],
                "targets": [],
                "metadata": "{}"
            }))
            .expect("mapping edits"),
        );

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        let codes = diagnostic_codes(&draft);
        assert!(codes.contains(&"existing_app_edits_unsupported"));
        assert!(codes.contains(&"existing_app_mapping_edits_unsupported"));
    }

    #[test]
    fn artifact_identity_mismatch_blocks_when_the_reviewed_artifact_is_renamed() {
        let mut renamed_app = proposed_draft_app(&source());
        let renamed = renamed_app
            .artifacts
            .shift_remove("apk")
            .expect("proposed artifact");
        renamed_app
            .artifacts
            .insert("renamed_apk".to_string(), renamed);

        let mut draft_request = request(source());
        draft_request.app = Some(renamed_app.clone());
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"app_artifact_id_mismatch"));

        let mut explicit_request = request(source());
        explicit_request.app = Some(renamed_app);
        explicit_request.app_artifact_id = Some("renamed_apk".to_string());
        let explicit = generate_remote_app_recipe_draft(explicit_request);
        assert!(!explicit.blocking, "{:#?}", explicit.diagnostics);
        assert!(explicit.app.artifacts.contains_key("renamed_apk"));
        assert_eq!(explicit.recipe["artifacts"]["renamed_apk"]["artifact"], "renamed_apk");
        assert_eq!(
            explicit.recipe["id"],
            "app.remote_example.renamed_apk.install"
        );
    }

    #[test]
    fn replaced_or_retyped_reviewed_artifacts_block_generation() {
        let mut replaced_app = proposed_draft_app(&source());
        replaced_app
            .artifacts
            .get_mut("apk")
            .expect("proposed artifact")
            .source = AppArtifactSource::UserProvided;
        let mut replaced_request = request(source());
        replaced_request.app = Some(replaced_app);
        let replaced = generate_remote_app_recipe_draft(replaced_request);
        assert!(replaced.blocking);
        assert!(diagnostic_codes(&replaced).contains(&"remote_source_artifact_missing"));

        let mut retyped_app = proposed_draft_app(&source());
        retyped_app
            .artifacts
            .get_mut("apk")
            .expect("proposed artifact")
            .kind = AppArtifactKind::File;
        let mut retyped_request = request(source());
        retyped_request.app = Some(retyped_app);
        let retyped = generate_remote_app_recipe_draft(retyped_request);
        assert!(retyped.blocking);
        assert!(diagnostic_codes(&retyped).contains(&"remote_source_artifact_missing"));
    }

    #[test]
    fn ambiguous_reviewed_artifact_selection_blocks_generation() {
        let mut app = proposed_draft_app(&source());
        let renamed = app.artifacts.shift_remove("apk").expect("proposed artifact");
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
        let mut draft_request = request(source());
        draft_request.app = Some(app);

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"remote_source_artifact_ambiguous"));
    }

    #[test]
    fn remote_user_provided_generation_requires_the_user_provided_artifact() {
        let mut app = proposed_draft_app(&user_provided_source());
        app.artifacts.clear();
        let mut draft_request = request(user_provided_source());
        draft_request.app = Some(app);

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"remote_source_artifact_missing"));

        let draft = generate_remote_app_recipe_draft(request(user_provided_source()));
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
    }

    #[test]
    fn structured_mapping_rows_replace_artifacts_targets_and_metadata_in_new_apps() {
        let mut draft_request = request(source());
        draft_request.mappings = Some(
            serde_json::from_value::<RemoteMappingEdits>(serde_json::json!({
                "artifacts": [{
                    "id": "apk",
                    "kind": "apk",
                    "name": "Mapped APK",
                    "source": {
                        "strategy": "direct_url",
                        "url": "https://github.com/example/project/releases/download/v1/app.apk"
                    }
                }],
                "targets": [],
                "metadata": "{\"author\": \"EmuChef\"}"
            }))
            .expect("mapping edits"),
        );

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.artifacts["apk"].name.as_deref(), Some("Mapped APK"));
        assert!(draft.app.targets.is_empty());
        assert_eq!(draft.app.metadata["author"], serde_json::json!("EmuChef"));
        assert_eq!(draft.recipe["artifacts"]["apk"]["type"], "app_artifact");
    }

    #[test]
    fn new_app_mapping_rows_keep_authored_targets_and_artifacts() {
        let mut draft_request = request(source());
        draft_request.mappings = Some(
            serde_json::from_value::<RemoteMappingEdits>(serde_json::json!({
                "artifacts": [{
                    "id": "apk",
                    "kind": "apk",
                    "name": "Mapped APK",
                    "source": {
                        "strategy": "direct_url",
                        "url": "https://github.com/example/project/releases/download/v1/app.apk"
                    }
                }],
                "targets": [{
                    "id": "save_dir",
                    "kind": "directory",
                    "location": "app_data",
                    "path": "files/saves"
                }],
                "metadata": "{\"author\": \"EmuChef\"}"
            }))
            .expect("mapping edits"),
        );

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.targets.len(), 1);
        let target = &draft.app.targets["save_dir"];
        assert_eq!(target.kind, AppTargetKind::Directory);
        assert_eq!(target.location, AppTargetLocation::AppData);
        assert_eq!(target.path, "files/saves");
        assert_eq!(
            draft.app.artifacts["apk"].name.as_deref(),
            Some("Mapped APK")
        );
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("save_dir"));
        assert!(app_yaml.contains("files/saves"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn permission_automation_splits_baseline_and_elevated_sets_into_app_aware_steps() {
        let mut draft_request = request(source());
        draft_request.app = Some(proposed_draft_app(&source()));
        draft_request.permission_automation_enabled = true;
        draft_request.permission_automation = Some(permission_automation(
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
        ));

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(
            step_types(&draft),
            vec![
                "resolve_artifacts",
                "install_apk",
                "grant_permissions",
                "grant_permissions"
            ]
        );
        let baseline_step = &draft.recipe["steps"][2];
        assert_eq!(baseline_step["id"], "grant_permissions_remote_example");
        assert_eq!(baseline_step["appRef"], "remote_example");
        assert_eq!(baseline_step["params"]["permission_set"], "baseline");
        assert_eq!(
            baseline_step["params"]["policy"],
            serde_json::json!({ "on_failure": "warn" })
        );
        assert_eq!(
            baseline_step["dependencies"],
            serde_json::json!(["install_remote_example"])
        );
        assert_eq!(baseline_step["constraints"]["capabilities"], serde_json::json!([]));
        let elevated_step = &draft.recipe["steps"][3];
        assert_eq!(
            elevated_step["id"],
            "grant_permissions_remote_example_elevated"
        );
        assert_eq!(elevated_step["params"]["permission_set"], "elevated");
        assert_eq!(
            elevated_step["dependencies"],
            serde_json::json!(["install_remote_example"])
        );

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
        assert_eq!(elevated.app_ops.len(), 1);
        assert_eq!(elevated.app_ops[0].op, "MANAGE_EXTERNAL_STORAGE");
        assert_eq!(elevated.app_ops[0].mode, AppOpMode::Allow);
        assert_eq!(elevated.app_ops[0].android_api_min, Some(30));

        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("permission_set: baseline"));
        assert!(recipe_yaml.contains("permission_set: elevated"));
        assert!(!recipe_yaml.contains("android.permission."));
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("permission_sets:"));
        assert!(!app_yaml.contains("requires_root"));
        assert!(!app_yaml.contains("apk_inspection"));
        assert!(!draft.app.metadata.contains_key("apk_inspection"));

        let evidence = draft
            .apk_inspection
            .as_ref()
            .expect("draft review keeps transient APK inspection evidence");
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
        assert_pair_authority(&draft);
    }

    #[test]
    fn selection_partitions_runtime_and_app_op_actions_by_root_and_omits_empty_sets() {
        let runtime_only = generate_remote_app_recipe_draft({
            let mut draft_request = request(source());
            draft_request.permission_automation_enabled = true;
            draft_request.permission_automation = Some(permission_automation(
                vec![runtime_permission("android.permission.CAMERA", false)],
                Vec::new(),
            ));
            draft_request
        });
        assert!(!runtime_only.blocking, "{:#?}", runtime_only.diagnostics);
        assert_eq!(
            step_types(&runtime_only),
            vec!["resolve_artifacts", "install_apk", "grant_permissions"]
        );
        let sets = runtime_only.app.permission_sets.as_ref().unwrap();
        let baseline = sets.baseline.as_ref().unwrap();
        assert_eq!(baseline.runtime.len(), 1);
        assert_eq!(baseline.runtime[0].permission, "android.permission.CAMERA");
        assert!(baseline.app_ops.is_empty());
        assert!(sets.elevated.is_none());

        let app_op_only = generate_remote_app_recipe_draft({
            let mut draft_request = request(source());
            draft_request.permission_automation_enabled = true;
            draft_request.permission_automation = Some(permission_automation(
                Vec::new(),
                vec![app_op(
                    "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "MANAGE_EXTERNAL_STORAGE",
                    false,
                )],
            ));
            draft_request
        });
        assert!(!app_op_only.blocking, "{:#?}", app_op_only.diagnostics);
        assert_eq!(
            step_types(&app_op_only),
            vec!["resolve_artifacts", "install_apk", "grant_permissions"]
        );
        let sets = app_op_only.app.permission_sets.as_ref().unwrap();
        let baseline = sets.baseline.as_ref().unwrap();
        assert!(baseline.runtime.is_empty());
        assert_eq!(baseline.app_ops.len(), 1);
        assert_eq!(baseline.app_ops[0].op, "MANAGE_EXTERNAL_STORAGE");
        assert!(sets.elevated.is_none());

        let root_only = generate_remote_app_recipe_draft({
            let mut draft_request = request(source());
            draft_request.permission_automation_enabled = true;
            draft_request.permission_automation = Some(permission_automation(
                Vec::new(),
                vec![app_op(
                    "android.permission.MANAGE_EXTERNAL_STORAGE",
                    "MANAGE_EXTERNAL_STORAGE",
                    true,
                )],
            ));
            draft_request
        });
        assert!(!root_only.blocking, "{:#?}", root_only.diagnostics);
        assert_eq!(
            step_types(&root_only),
            vec!["resolve_artifacts", "install_apk", "grant_permissions"]
        );
        let sets = root_only.app.permission_sets.as_ref().unwrap();
        assert!(sets.baseline.is_none());
        let elevated = sets.elevated.as_ref().unwrap();
        assert!(elevated.runtime.is_empty());
        assert_eq!(elevated.app_ops.len(), 1);
        assert_eq!(
            root_only.recipe["steps"][2]["id"],
            "grant_permissions_remote_example_elevated"
        );
        assert_eq!(
            root_only.recipe["steps"][2]["params"]["permission_set"],
            "elevated"
        );
    }

    #[test]
    fn permission_automation_without_selected_actions_produces_no_sets_or_steps() {
        let mut enabled_request = request(source());
        enabled_request.permission_automation_enabled = true;
        enabled_request.permission_automation =
            Some(permission_automation(Vec::new(), Vec::new()));

        let enabled = generate_remote_app_recipe_draft(enabled_request);

        assert!(!enabled.blocking, "{:#?}", enabled.diagnostics);
        assert!(enabled.app.permission_sets.is_none());
        assert_eq!(step_types(&enabled), vec!["resolve_artifacts", "install_apk"]);
        assert!(!enabled
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("permission_sets"));
        assert!(!enabled
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("grant_permissions"));

        let absent = generate_remote_app_recipe_draft(request(source()));

        assert!(!absent.blocking, "{:#?}", absent.diagnostics);
        assert!(absent.app.permission_sets.is_none());
        assert_eq!(step_types(&absent), vec!["resolve_artifacts", "install_apk"]);
    }

    /// A wire request that omits `permissionAutomationEnabled` parses with the
    /// flag off, so the generated pair declares no permission sets and no
    /// grant steps.
    #[test]
    fn absent_permission_automation_flag_deserializes_as_disabled() {
        let parsed: RemoteAppRecipeDraftRequest = serde_json::from_value(serde_json::json!({
            "facts": {
                "packageName": "com.example.remote",
                "applicationLabel": "Remote Example",
                "launcherActivities": ["com.example.remote/.MainActivity"],
                "calculatedSha256": "B".repeat(64),
                "checksumStatus": "not_compared",
                "signatureVerification": "not_performed",
                "split": false,
                "base": true
            },
            "source": {
                "mode": "github_release",
                "strategy": "pinned_remote_asset",
                "downloadUrl": "https://github.com/example/project/releases/download/v1/app.apk",
                "provider": "github",
                "baseUrl": "https://github.com",
                "repository": "example/project",
                "releaseTag": "v1",
                "assetName": "app.apk"
            }
        }))
        .expect("a request without permissionAutomationEnabled parses");

        assert!(!parsed.permission_automation_enabled);

        let draft = generate_remote_app_recipe_draft(parsed);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft.app.permission_sets.is_none());
        assert_eq!(step_types(&draft), vec!["resolve_artifacts", "install_apk"]);
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("grant_permissions"));
    }

    #[test]
    fn disabled_permission_automation_preserves_declared_sets_without_grant_steps() {
        let original = existing_app();
        let mut existing_request = request(source());
        existing_request.existing_app = Some(original.clone());
        existing_request.app_artifact_id = Some("added_apk".to_string());

        let existing = generate_remote_app_recipe_draft(existing_request);

        assert!(!existing.blocking, "{:#?}", existing.diagnostics);
        assert_eq!(existing.app.permission_sets, original.permission_sets);
        assert_eq!(
            step_types(&existing),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert!(existing
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("permission_sets"));
        assert!(!existing
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("grant_permissions"));

        let mut authored = proposed_draft_app(&source());
        authored.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.RECORD_AUDIO".to_string(),
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let mut authored_request = request(source());
        authored_request.app = Some(authored.clone());

        let new_app = generate_remote_app_recipe_draft(authored_request);

        assert!(!new_app.blocking, "{:#?}", new_app.diagnostics);
        assert_eq!(new_app.app.permission_sets, authored.permission_sets);
        assert_eq!(
            step_types(&new_app),
            vec!["resolve_artifacts", "install_apk"]
        );
        assert!(!new_app
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("grant_permissions"));
    }

    #[test]
    fn selected_actions_with_permission_automation_disabled_block_explicitly() {
        let mut disabled_request = request(source());
        disabled_request.permission_automation_enabled = false;
        disabled_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.CAMERA", false)],
            Vec::new(),
        ));

        let disabled = generate_remote_app_recipe_draft(disabled_request);

        assert!(disabled.blocking);
        assert!(diagnostic_codes(&disabled).contains(&"apk_permission_automation_disabled"));
        assert!(disabled.app.permission_sets.is_none());
        assert!(disabled.recipe_canonical_yaml.is_none());
        assert!(!disabled.recipe["steps"]
            .to_string()
            .contains("grant_permissions"));

        let original = existing_app();
        let mut existing_request = request(source());
        existing_request.existing_app = Some(original.clone());
        existing_request.app_artifact_id = Some("added_apk".to_string());
        existing_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.CAMERA", false)],
            Vec::new(),
        ));

        let existing = generate_remote_app_recipe_draft(existing_request);

        assert!(existing.blocking);
        assert!(diagnostic_codes(&existing).contains(&"apk_permission_automation_disabled"));
        assert_eq!(existing.app.permission_sets, original.permission_sets);
        assert!(!existing.recipe["steps"]
            .to_string()
            .contains("grant_permissions"));
    }

    #[test]
    fn enabled_permission_automation_without_selected_actions_grants_the_declared_sets() {
        let original = existing_app();
        let mut draft_request = request(source());
        draft_request.existing_app = Some(original.clone());
        draft_request.app_artifact_id = Some("added_apk".to_string());
        draft_request.permission_automation_enabled = true;

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(draft.app.permission_sets, original.permission_sets);
        assert_eq!(
            step_types(&draft),
            vec![
                "resolve_artifacts",
                "install_apk",
                "grant_permissions"
            ]
        );
        let grant = &draft.recipe["steps"][2];
        assert_eq!(grant["id"], "grant_permissions_remote_example_added_apk");
        assert_eq!(grant["appRef"], "remote_example");
        assert_eq!(grant["params"]["permission_set"], "baseline");
        assert_eq!(
            grant["dependencies"],
            serde_json::json!(["install_remote_example_added_apk"])
        );
        assert!(!grant.to_string().contains("RECORD_AUDIO"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn invalid_permission_automation_selections_block_without_sets_or_steps() {
        let mut duplicate_request = request(source());
        duplicate_request.permission_automation_enabled = true;
        duplicate_request.permission_automation = Some(permission_automation(
            vec![
                runtime_permission("android.permission.CAMERA", false),
                runtime_permission("android.permission.CAMERA", false),
            ],
            Vec::new(),
        ));

        let duplicate = generate_remote_app_recipe_draft(duplicate_request);

        assert!(duplicate.blocking);
        assert!(diagnostic_codes(&duplicate).contains(&"apk_permission_automation_duplicate"));
        assert!(duplicate.app.permission_sets.is_none());
        assert!(duplicate.recipe_canonical_yaml.is_none());
        assert!(!duplicate.recipe["steps"]
            .to_string()
            .contains("grant_permissions"));

        let mut mismatched_request = request(source());
        mismatched_request.permission_automation_enabled = true;
        let mut selection = permission_automation(
            vec![runtime_permission("android.permission.CAMERA", false)],
            Vec::new(),
        );
        selection.package_name = "com.example.other".to_string();
        mismatched_request.permission_automation = Some(selection);

        let mismatched = generate_remote_app_recipe_draft(mismatched_request);

        assert!(mismatched.blocking);
        assert!(diagnostic_codes(&mismatched)
            .contains(&"apk_permission_automation_package_mismatch"));
        assert!(mismatched.app.permission_sets.is_none());
        assert!(mismatched.recipe_canonical_yaml.is_none());
    }

    #[test]
    fn launch_step_never_depends_on_permission_steps() {
        let mut draft_request = request(source());
        draft_request.recipe = Some(launch_edits());
        draft_request.permission_automation_enabled = true;
        draft_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.CAMERA", true)],
            Vec::new(),
        ));

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(
            step_types(&draft),
            vec![
                "resolve_artifacts",
                "install_apk",
                "launch_app",
                "grant_permissions"
            ]
        );
        let launch = &draft.recipe["steps"][2];
        assert_eq!(launch["id"], "launch_remote_example");
        assert_eq!(
            launch["dependencies"],
            serde_json::json!(["install_remote_example"])
        );
        assert_eq!(launch["params"], serde_json::json!({}));
        assert_eq!(
            launch["constraints"]["capabilities"],
            serde_json::json!(["app_launch"])
        );
        assert!(!launch["dependencies"].to_string().contains("grant_permissions"));
        assert_eq!(
            draft.app.launcher_activity.as_deref(),
            Some("com.example.remote.MainActivity")
        );
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(!recipe_yaml.contains("root_shell"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn existing_app_permission_automation_reuses_exact_duplicates_and_blocks_conflicts() {
        let mut reuse_request = request(source());
        reuse_request.existing_app = Some(existing_app());
        reuse_request.app_artifact_id = Some("added_apk".to_string());
        reuse_request.permission_automation_enabled = true;
        reuse_request.permission_automation = Some(permission_automation(
            vec![
                runtime_permission("android.permission.RECORD_AUDIO", false),
                runtime_permission("android.permission.CAMERA", true),
            ],
            Vec::new(),
        ));

        let reuse = generate_remote_app_recipe_draft(reuse_request);
        assert!(!reuse.blocking, "{:#?}", reuse.diagnostics);
        assert!(!diagnostic_codes(&reuse).contains(&"app_permission_action_duplicate"));
        let sets = reuse.app.permission_sets.as_ref().unwrap();
        let baseline = sets.baseline.as_ref().unwrap();
        assert_eq!(baseline.runtime.len(), 1);
        assert_eq!(
            baseline.runtime[0].permission,
            "android.permission.RECORD_AUDIO"
        );
        assert_eq!(baseline.runtime[0].android_api_min, Some(23));
        let elevated = sets.elevated.as_ref().unwrap();
        assert_eq!(elevated.runtime.len(), 1);
        assert_eq!(elevated.runtime[0].permission, "android.permission.CAMERA");

        let mut bound_conflict = existing_app();
        bound_conflict.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.RECORD_AUDIO".to_string(),
                    android_api_min: Some(29),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let mut conflict_request = request(source());
        conflict_request.existing_app = Some(bound_conflict);
        conflict_request.app_artifact_id = Some("added_apk".to_string());
        conflict_request.permission_automation_enabled = true;
        conflict_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.RECORD_AUDIO", false)],
            Vec::new(),
        ));
        let conflict = generate_remote_app_recipe_draft(conflict_request);
        assert!(conflict.blocking);
        assert!(diagnostic_codes(&conflict).contains(&"app_permission_action_conflict"));
        assert_eq!(
            conflict
                .app
                .permission_sets
                .as_ref()
                .unwrap()
                .baseline
                .as_ref()
                .unwrap()
                .runtime[0]
                .android_api_min,
            Some(29)
        );

        let mut set_conflict = existing_app();
        set_conflict.permission_sets = Some(AppPermissionSets {
            baseline: None,
            elevated: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
        });
        let mut set_request = request(source());
        set_request.existing_app = Some(set_conflict);
        set_request.app_artifact_id = Some("added_apk".to_string());
        set_request.permission_automation_enabled = true;
        set_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.CAMERA", false)],
            Vec::new(),
        ));
        let set_result = generate_remote_app_recipe_draft(set_request);
        assert!(set_result.blocking);
        assert!(diagnostic_codes(&set_result).contains(&"app_permission_action_conflict"));
    }

    #[test]
    fn existing_app_launch_requires_the_app_definition_launcher() {
        let mut app = existing_app();
        app.launcher_activity = None;
        let mut draft_request = request(source());
        draft_request.existing_app = Some(app);
        draft_request.app_artifact_id = Some("added_apk".to_string());
        draft_request.recipe = Some(launch_edits());

        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"app_launcher_activity_required"));
        assert!(!step_types(&draft).contains(&"launch_app"));

        let mut authored = existing_app();
        authored.launcher_activity = Some("com.example.remote.SplashActivity".to_string());
        let mut draft_request = request(source());
        draft_request.existing_app = Some(authored);
        draft_request.app_artifact_id = Some("added_apk".to_string());
        draft_request.recipe = Some(launch_edits());

        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(step_types(&draft).contains(&"launch_app"));
        assert_eq!(
            draft.app.launcher_activity.as_deref(),
            Some("com.example.remote.SplashActivity")
        );
    }

    #[test]
    fn new_app_launch_records_only_the_verified_inspected_launcher() {
        let mut draft_request = request(source());
        draft_request.recipe = Some(RemoteRecipeEdits {
            launch_enabled: true,
            launcher_activity: Some("com.other/.MainActivity".to_string()),
            ..RemoteRecipeEdits::default()
        });

        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"apk_launcher_unverified"));
        assert!(draft.app.launcher_activity.is_none());
        assert!(!step_types(&draft).contains(&"launch_app"));

        let mut draft_request = request(source());
        draft_request.recipe = Some(launch_edits());
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert_eq!(
            draft.app.launcher_activity.as_deref(),
            Some("com.example.remote.MainActivity")
        );
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("com.other"));
    }

    #[test]
    fn generated_pair_yaml_roundtrips_through_canonical_app_authority_validation() {
        let mut draft_request = request(latest_source());
        draft_request.release_analysis = Some(unique_release_analysis());
        draft_request.app = Some(proposed_draft_app(&latest_source()));
        draft_request.permission_automation_enabled = true;
        draft_request.permission_automation = Some(permission_automation(
            vec![runtime_permission("android.permission.RECORD_AUDIO", false)],
            vec![app_op(
                "android.permission.MANAGE_EXTERNAL_STORAGE",
                "MANAGE_EXTERNAL_STORAGE",
                true,
            )],
        ));
        draft_request.recipe = Some(launch_edits());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        let parsed_app: AppDefinitionV1 =
            serde_yaml::from_str(app_yaml).expect("generated app yaml parses");
        assert_eq!(parsed_app, draft.app);
        assert!(
            validate_app_definition(&draft.app).is_empty(),
            "{:?}",
            validate_app_definition(&draft.app)
        );
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("type: app_artifact"));
        assert!(!recipe_yaml.contains("apk_inspection"));
        assert!(!recipe_yaml.contains("android.permission."));
        assert!(!recipe_yaml.contains("repository"));
        assert!(!recipe_yaml.contains("root_shell"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn latest_pattern_requires_fresh_trusted_analysis() {
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
            let draft = latest_draft(r"^app-v.*-arm64.apk$", false, analysis);
            assert!(draft.blocking, "{expected_code}");
            assert!(diagnostic_codes(&draft).contains(&expected_code));
        }
    }

    #[test]
    fn latest_pattern_blocks_newest_zero_and_multiple_matches() {
        let no_match = latest_draft(r"^other.apk$", false, Some(unique_release_analysis()));
        assert!(no_match.blocking);
        assert!(diagnostic_codes(&no_match).contains(&"latest_release_current_no_match"));

        let multiple = latest_draft(
            r".apk$",
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
            r"^app-v2-arm64.apk$",
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
    fn latest_pattern_rejects_javascript_valid_rust_invalid_lookahead() {
        let draft = latest_draft(r"^(?=app).*.apk$", false, Some(unique_release_analysis()));
        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "latest_release_asset_pattern_invalid"
                && diagnostic.message.contains("Rust rejected")
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
            let draft = latest_draft(r".apk$", false, Some(analysis));
            assert!(draft.blocking);
            assert!(diagnostic_codes(&draft).contains(&"latest_release_analysis_invalid"));
        }
    }

    #[test]
    fn blank_asset_pattern_resolves_releases_by_kind_alone() {
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
        let canonical = draft.app_canonical_yaml.as_deref().unwrap();
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

        let zero = latest_draft_with_pattern(
            None,
            false,
            Some(release_analysis(vec![trusted_release("v1", false, &[])])),
        );
        assert!(zero.blocking);
        assert!(diagnostic_codes(&zero).contains(&"latest_release_current_no_match"));

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
    fn inverted_artifact_pattern_is_preserved_and_selects_outside_pattern_assets() {
        let mut app = proposed_draft_app(&latest_source());
        if let AppArtifactSource::LatestRelease {
            asset_pattern,
            invert_asset_pattern,
            ..
        } = &mut app.artifacts.get_mut("apk").unwrap().source
        {
            *asset_pattern = Some(r"^app-v1-arm64.apk$".to_string());
            *invert_asset_pattern = Some(true);
        }

        let mut draft_request = request(latest_source());
        draft_request.app = Some(app);
        draft_request.release_analysis = Some(release_analysis(vec![trusted_release(
            "v1",
            false,
            &["app-v1-arm64.apk", "app-v2-arm64.apk"],
        )]));

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(!diagnostic_codes(&draft).contains(&"latest_release_invert_unsupported"));
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("invert_asset_pattern: true"));

        let mut no_remaining_asset_app = proposed_draft_app(&latest_source());
        if let AppArtifactSource::LatestRelease {
            asset_pattern,
            invert_asset_pattern,
            ..
        } = &mut no_remaining_asset_app
            .artifacts
            .get_mut("apk")
            .unwrap()
            .source
        {
            *asset_pattern = Some(r"^app-v1-arm64.apk$".to_string());
            *invert_asset_pattern = Some(true);
        }
        let mut excluded_request = request(latest_source());
        excluded_request.app = Some(no_remaining_asset_app);
        excluded_request.release_analysis =
            Some(release_analysis(vec![trusted_release("v1", false, &["app-v1-arm64.apk"])]));
        let excluded = generate_remote_app_recipe_draft(excluded_request);
        assert!(excluded.blocking);
        assert!(diagnostic_codes(&excluded).contains(&"latest_release_current_no_match"));
    }

    #[test]
    fn latest_release_requires_the_runtime_supported_service_origin() {
        let mut self_hosted = latest_source();
        self_hosted.base_url = Some("https://github.example.com".to_string());
        let mut draft_request = request(self_hosted);
        draft_request.release_analysis = Some(unique_release_analysis());
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"latest_release_base_url_unsupported"));

        let mut self_hosted_gitlab = latest_source();
        self_hosted_gitlab.mode = "gitlab_repository".to_string();
        self_hosted_gitlab.provider = Some("gitlab".to_string());
        self_hosted_gitlab.base_url = Some("https://gitlab.example.com".to_string());
        let draft = generate_remote_app_recipe_draft(request(self_hosted_gitlab));
        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"latest_release_base_url_unsupported"));

        let mut canonical = latest_source();
        canonical.base_url = Some("https://GitHub.com/".to_string());
        let mut draft_request = request(canonical);
        draft_request.release_analysis = Some(unique_release_analysis());
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(!diagnostic_codes(&draft).contains(&"latest_release_base_url_unsupported"));
    }

    #[test]
    fn retargeted_release_repository_warns_instead_of_using_stale_evidence() {
        let mut app = proposed_draft_app(&latest_source());
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::LatestRelease {
            provider: ReleaseProvider::Github,
            base_url: "https://github.com".to_string(),
            repository: "example/other".to_string(),
            asset_pattern: Some(r"^unrelated-.*.apk$".to_string()),
            invert_asset_pattern: None,
            prerelease: false,
        };

        let mut draft_request = request(latest_source());
        draft_request.app = Some(app);
        draft_request.release_analysis = Some(unique_release_analysis());

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(diagnostic_codes(&draft).contains(&"latest_release_analysis_stale"));
        assert!(!draft
            .diagnostics
            .iter()
            .any(|item| item.code.starts_with("latest_release_current_")));
    }

    #[test]
    fn gitlab_and_forgejo_latest_sources_preserve_provider_identity() {
        let mut gitlab = latest_source();
        gitlab.mode = "gitlab_repository".to_string();
        gitlab.provider = Some("gitlab".to_string());
        gitlab.base_url = Some("https://gitlab.com".to_string());
        gitlab.repository = Some("example/group/project".to_string());
        gitlab.download_url = "https://gitlab.com/example/group/project".to_string();
        let draft = generate_remote_app_recipe_draft(request(gitlab));
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
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
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("provider: gitlab"));
        assert!(app_yaml.contains("base_url: https://gitlab.com"));
        assert!(app_yaml.contains("repository: example/group/project"));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(!recipe_yaml.contains("gitlab"));
        assert_pair_authority(&draft);

        let mut forgejo = latest_source();
        forgejo.mode = "forgejo_repository".to_string();
        forgejo.provider = Some("forgejo".to_string());
        forgejo.base_url = Some("https://codeberg.org".to_string());
        forgejo.repository = Some("example/project".to_string());
        forgejo.download_url = "https://codeberg.org/example/project".to_string();
        let draft = generate_remote_app_recipe_draft(request(forgejo));
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        match &draft.app.artifacts["apk"].source {
            AppArtifactSource::LatestRelease { provider, base_url, .. } => {
                assert_eq!(*provider, ReleaseProvider::Forgejo);
                assert_eq!(base_url, "https://codeberg.org");
            }
            other => panic!("expected a latest-release artifact source, got {other:?}"),
        }
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("provider: forgejo"));
        assert!(app_yaml.contains("base_url: https://codeberg.org"));
    }

    #[test]
    fn edited_artifact_policy_drives_validation_and_the_saved_pair() {
        let mut app = proposed_draft_app(&source());
        app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::DirectUrl {
            url: "https://example.test/edited/app.apk".to_string(),
            sha256: Some("c".repeat(64)),
        };
        let mut draft_request = request(source());
        draft_request.app = Some(app);
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let app_yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(app_yaml.contains("url: https://example.test/edited/app.apk"));
        assert!(app_yaml.contains(&format!("sha256: {}", "c".repeat(64))));
        let recipe_yaml = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe_yaml.contains("artifact: apk"));
        assert!(!recipe_yaml.contains("example.test"));

        let mut release_app = proposed_draft_app(&latest_source());
        release_app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::LatestRelease {
            provider: ReleaseProvider::Github,
            base_url: "https://github.com".to_string(),
            repository: "example/edited".to_string(),
            asset_pattern: Some(r"^app-v1.*.apk$".to_string()),
            invert_asset_pattern: None,
            prerelease: true,
        };
        let mut draft_request = request(latest_source());
        draft_request.app = Some(release_app);
        draft_request.release_analysis = Some(unique_release_analysis());
        let draft = generate_remote_app_recipe_draft(draft_request);
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("repository: example/edited"));
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap()
            .contains("example/edited"));
    }

    #[test]
    fn selection_checksum_fills_a_blank_artifact_download_source() {
        let mut selection = source();
        selection.trusted_sha256 = Some("D".repeat(64));
        let app = proposed_app(&facts(), DEFAULT_APP_ARTIFACT_ID, &source());
        let mut draft_request = request(selection);
        draft_request.app = Some(app);

        let draft = generate_remote_app_recipe_draft(draft_request);

        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let expected = "d".repeat(64);
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl { sha256: Some(sha256), .. } if sha256 == &expected
        ));
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .unwrap()
            .contains(&format!("sha256: {expected}")));
    }

    #[test]
    fn trusted_sha256_rules_for_direct_and_other_sources() {
        let mut invalid = source();
        invalid.trusted_sha256 = Some("sha256:not-trusted".to_string());
        let draft = generate_remote_app_recipe_draft(request(invalid));
        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "apk_trusted_sha256_invalid"
                && diagnostic.field == "source.trustedSha256"
        }));
        assert!(matches!(
            &draft.app.artifacts["apk"].source,
            AppArtifactSource::DirectUrl { sha256: None, .. }
        ));
        assert!(!draft
            .recipe_canonical_yaml
            .as_deref()
            .unwrap_or_default()
            .contains("not-trusted"));

        for strategy in ["latest_compatible_release", "user_provided_apk"] {
            let mut selected = if strategy == "latest_compatible_release" {
                latest_source()
            } else {
                user_provided_source()
            };
            selected.trusted_sha256 = Some("A".repeat(64));
            let mut draft_request = request(selected);
            draft_request.release_analysis = (strategy == "latest_compatible_release")
                .then(unique_release_analysis);
            let draft = generate_remote_app_recipe_draft(draft_request);
            assert!(draft.blocking, "{strategy}");
            assert!(draft.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "apk_trusted_sha256_strategy_unsupported"
                    && diagnostic.field == "source.trustedSha256"
            }));
        }

        for strategy in [
            "pinned_remote_asset",
            "latest_compatible_release",
            "user_provided_apk",
        ] {
            let mut selected = match strategy {
                "latest_compatible_release" => latest_source(),
                "user_provided_apk" => user_provided_source(),
                _ => source(),
            };
            selected.trusted_sha256 = Some(" \t\r\n".to_string());
            let mut draft_request = request(selected);
            draft_request.release_analysis = (strategy == "latest_compatible_release")
                .then(unique_release_analysis);
            let draft = generate_remote_app_recipe_draft(draft_request);
            assert!(!draft.blocking, "{strategy}: {:#?}", draft.diagnostics);
        }
    }

    #[test]
    fn unsafe_download_address_blocks_direct_sources() {
        let mut selected = source();
        selected.download_url = "http://example.com/app.apk".to_string();

        let draft = generate_remote_app_recipe_draft(request(selected));

        assert!(draft.blocking);
        assert!(diagnostic_codes(&draft).contains(&"remote_download_url_invalid"));
        assert!(draft.app_canonical_yaml.is_none());
    }
}

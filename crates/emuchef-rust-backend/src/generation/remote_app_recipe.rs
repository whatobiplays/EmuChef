//! Side-effect-free app-definition and recipe generation for inspected remote APK sources.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::authored_models::{
    emit_app_definition_yaml, validate_app_definition, AppArtifactSupport, AppDefinitionV1,
    AppInstallSource, AppPackage, AppProvisioning, AppTrackingSource, ConfigArtifactSupport,
    OrderedValueMap, RequiredArtifactSupport, APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use crate::model::{
    InputDeclaration, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides,
    RemoteFileArtifact, Step, StepCondition, StepConstraints,
};
use crate::validation::normalize_expected_sha256;

use super::apk::ApkInspectionFacts;
use super::app_recipe::{
    build_permission_step, generated_apk_inspection_metadata, PermissionAutomationIssue,
    PermissionAutomationSelection,
};
use super::identifiers::{normalize_identifier_component, recipe_local_token};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RemoteAppRecipeDraftRequest {
    pub facts: ApkInspectionFacts,
    pub source: RemoteSource,
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
    install_source_options: String,
    tracking_source_fields: String,
    metadata: String,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    config_targets: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum DraftSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DraftDiagnostic {
    severity: DraftSeverity,
    code: String,
    message: String,
    field: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceState {
    Verified,
    Derived,
    Suggested,
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
    let mut diagnostics = validate_source(&request.source);
    diagnostics.extend(permission_automation_diagnostics(
        request.permission_automation.as_ref(),
        &request.facts,
        &request.source,
    ));
    if let Some(mappings) = request.mappings {
        apply_mapping_edits(&mut app, mappings, &mut diagnostics);
    }
    app.metadata.shift_remove("apk_inspection");
    let automation_eligible = matches!(
        request.source.strategy.as_str(),
        "pinned_remote_asset" | "latest_compatible_release"
    );
    match generated_apk_inspection_metadata(
        &request.facts,
        request.permission_automation.as_ref(),
        automation_eligible,
    ) {
        Ok(metadata) => {
            app.metadata.insert("apk_inspection".to_string(), metadata);
        }
        Err(issues) => diagnostics.extend(
            issues
                .into_iter()
                .map(|issue| error(issue.code, issue.message, &issue.field)),
        ),
    }
    let mut recipe_edits = request
        .recipe
        .unwrap_or_else(|| proposed_recipe_edits(&app, &request.source));
    if recipe_edits.ids.is_none() || request.regenerate_identifiers {
        recipe_edits.ids = Some(generated_ids(&app.id));
    }
    fill_empty_recipe_text(&mut recipe_edits, &app, &request.source);
    app.provisioning.launch_once_recommended = recipe_edits.launch_enabled;
    diagnostics.extend(app_diagnostics(&app));
    let recipe = build_recipe(
        &app,
        &request.facts,
        &request.source,
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
    }
}

fn validate_source(source: &RemoteSource) -> Vec<DraftDiagnostic> {
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
        match source.asset_pattern.as_deref() {
            Some(pattern) if regex::Regex::new(pattern).is_ok() => {}
            _ => diagnostics.push(error(
                "latest_release_asset_pattern_invalid",
                "Latest compatible release requires a valid APK filename pattern.",
                "source.assetPattern",
            )),
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

fn proposed_app(facts: &ApkInspectionFacts, source: &RemoteSource) -> AppDefinitionV1 {
    let id_source = facts
        .application_label
        .as_deref()
        .or(facts.package_name.as_deref())
        .unwrap_or_default();
    let pinned = source.strategy == "pinned_remote_asset";
    let latest = source.strategy == "latest_compatible_release";
    let mut options = OrderedValueMap::new();
    options.insert(
        "url".to_string(),
        Value::String(source.download_url.clone()),
    );
    if let Some(repository) = &source.repository {
        options.insert("repository".to_string(), Value::String(repository.clone()));
    }
    if let Some(provider) = &source.provider {
        options.insert("provider".to_string(), Value::String(provider.clone()));
    }
    if let Some(base_url) = &source.base_url {
        options.insert("base_url".to_string(), Value::String(base_url.clone()));
    }
    if let Some(tag) = &source.release_tag {
        options.insert("release_tag".to_string(), Value::String(tag.clone()));
    }
    if let Some(asset) = &source.asset_name {
        options.insert("asset_name".to_string(), Value::String(asset.clone()));
    }
    if let Some(pattern) = &source.asset_pattern {
        options.insert("asset_pattern".to_string(), Value::String(pattern.clone()));
    }
    if latest {
        options.insert(
            "include_prereleases".to_string(),
            Value::Bool(source.include_prereleases),
        );
    }
    let mut tracking = OrderedValueMap::new();
    if let Some(repository) = &source.repository {
        tracking.insert("repository".to_string(), Value::String(repository.clone()));
    }
    if let Some(provider) = &source.provider {
        tracking.insert("provider".to_string(), Value::String(provider.clone()));
    }
    if let Some(base_url) = &source.base_url {
        tracking.insert("base_url".to_string(), Value::String(base_url.clone()));
    }
    if let Some(tag) = &source.release_tag {
        tracking.insert("release_tag".to_string(), Value::String(tag.clone()));
    }
    tracking.insert(
        "url".to_string(),
        Value::String(source.download_url.clone()),
    );
    AppDefinitionV1 {
        schema_version: SCHEMA_VERSION_V1,
        kind: APP_DEFINITION_KIND.to_string(),
        id: normalize_identifier_component(id_source),
        name: facts
            .application_label
            .clone()
            .or_else(|| facts.package_name.clone())
            .unwrap_or_default(),
        description: None,
        category: String::new(),
        package: AppPackage {
            primary: facts.package_name.clone().unwrap_or_default(),
            aliases: Vec::new(),
        },
        install_source: AppInstallSource {
            type_name: if pinned {
                "remote_apk"
            } else if latest {
                "remote_release"
            } else {
                "user_provided_apk"
            }
            .to_string(),
            resolver: if pinned {
                "direct_url"
            } else if latest {
                "provider_latest_release"
            } else {
                "none"
            }
            .to_string(),
            options: if pinned || latest {
                options
            } else {
                OrderedValueMap::new()
            },
        },
        tracking_source: AppTrackingSource {
            type_name: if !pinned && !latest {
                "local_apk"
            } else if source.mode.ends_with("_repository") || source.mode.ends_with("_release") {
                "provider_release"
            } else {
                "remote_apk"
            }
            .to_string(),
            fields: if pinned || latest {
                tracking
            } else {
                OrderedValueMap::new()
            },
        },
        artifacts: AppArtifactSupport {
            apk: RequiredArtifactSupport {
                required: pinned || latest,
            },
            shared_storage_config: ConfigArtifactSupport { supported: false },
            app_data_config: ConfigArtifactSupport { supported: false },
            byo_apk: RequiredArtifactSupport {
                required: !pinned && !latest,
            },
        },
        provisioning: AppProvisioning::default(),
        inputs: Vec::new(),
        metadata: OrderedValueMap::new(),
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
            RemoteFileArtifact {
                type_name: "remote_file".to_string(),
                url: source.download_url.clone(),
                cache: "default".to_string(),
            },
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
                source.asset_pattern.clone().unwrap_or_default(),
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
        Value::String(app.package.primary.clone()),
    );
    steps.push(Step {
        id: ids.install_step_id.clone(),
        type_name: "install_apk".to_string(),
        name: format!("Install {}", app.name),
        description: None,
        progress_note: Some(format!("Installing {} on the selected device", app.name)),
        user_toggleable: false,
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
                ParamValue::Literal(Value::String(app.package.primary.clone())),
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
    if let Some(value) = parse_mapping(
        "installSource.options",
        &mappings.install_source_options,
        diagnostics,
    ) {
        app.install_source.options = value;
    }
    if let Some(value) = parse_mapping(
        "trackingSource",
        &mappings.tracking_source_fields,
        diagnostics,
    ) {
        app.tracking_source.fields = value;
    }
    if let Some(value) = parse_mapping("metadata", &mappings.metadata, diagnostics) {
        app.metadata = value;
    }
    app.inputs = parse_mapping_list("inputs", mappings.inputs, diagnostics);
    app.provisioning.config_targets = parse_mapping_list(
        "provisioning.configTargets",
        mappings.config_targets,
        diagnostics,
    );
}

fn parse_mapping(
    field: &str,
    source: &str,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> Option<OrderedValueMap> {
    match serde_json::from_str::<StrictJsonValue>(source) {
        Ok(StrictJsonValue::Object(entries)) => Some(entries.into_iter().collect()),
        Ok(_) => {
            diagnostics.push(error(
                "mapping_json_not_object",
                "Mapping fields must use a JSON object.",
                field,
            ));
            None
        }
        Err(_) => {
            diagnostics.push(error(
                "mapping_json_invalid",
                "Mapping fields must use valid JSON without duplicate keys.",
                field,
            ));
            None
        }
    }
}

fn parse_mapping_list(
    field: &str,
    sources: Vec<String>,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> Vec<OrderedValueMap> {
    sources
        .into_iter()
        .enumerate()
        .filter_map(|(index, source)| {
            parse_mapping(&format!("{field}[{index}]"), &source, diagnostics)
        })
        .collect()
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
            "package.primary",
            if facts.package_name.is_some() {
                EvidenceState::Verified
            } else {
                EvidenceState::Missing
            },
            "apk_manifest",
            current.package.primary != proposed.package.primary,
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
            "install_source",
            EvidenceState::Verified,
            &source.mode,
            current.install_source != proposed.install_source,
        ),
        field_evidence(
            "tracking_source",
            EvidenceState::Verified,
            &source.mode,
            current.tracking_source != proposed.tracking_source,
        ),
        field_evidence(
            "artifacts",
            EvidenceState::Suggested,
            "pinned_remote_asset",
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

fn error(code: &str, message: &str, field: &str) -> DraftDiagnostic {
    DraftDiagnostic {
        severity: DraftSeverity::Error,
        code: code.to_string(),
        message: message.to_string(),
        field: field.to_string(),
    }
}

#[derive(Clone, Debug)]
enum StrictJsonValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<StrictJsonValue>),
    Object(Vec<(String, Value)>),
}
impl StrictJsonValue {
    fn into_value(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => Value::Array(values.into_iter().map(Self::into_value).collect()),
            Self::Object(entries) => Value::Object(entries.into_iter().collect::<Map<_, _>>()),
        }
    }
}
impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictJsonValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Null)
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Null)
            }
            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::Bool(value))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictJsonValue::Number(value.into()))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictJsonValue::Number(value.into()))
            }
            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(StrictJsonValue::Number)
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::String(value.to_string()))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictJsonValue::String(value))
            }
            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictJsonValue>()? {
                    values.push(value);
                }
                Ok(StrictJsonValue::Array(values))
            }
            fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut seen = HashSet::new();
                let mut entries = Vec::new();
                while let Some(key) = mapping.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom(format!("duplicate key {key}")));
                    }
                    let value = mapping.next_value::<StrictJsonValue>()?.into_value();
                    entries.push((key, value));
                }
                Ok(StrictJsonValue::Object(entries))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        }
    }

    fn app_op(permission: &str, operation: &str, requires_root: bool) -> AppOpPermissionSelection {
        AppOpPermissionSelection {
            permission_name: permission.to_string(),
            operation_name: operation.to_string(),
            mode: "allow".to_string(),
            requires_root,
        }
    }

    #[test]
    fn mixed_permission_automation_uses_inspected_package_and_launch_dependency() {
        let source = source();
        let mut app = proposed_app(&facts(), &source);
        app.category = "emulator".to_string();
        app.package.primary = "com.example.edited".to_string();
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
                    "when": { "rooted": true }
                },
                {
                    "package_name": "com.example.remote",
                    "name": "android.permission.RECORD_AUDIO",
                    "required": false
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
                    "when": { "rooted": true }
                },
                {
                    "package_name": "com.example.remote",
                    "op": "ZETA_OP",
                    "mode": "allow",
                    "required": false
                }
            ])
        );
        assert_eq!(
            steps[3]["dependencies"],
            serde_json::json!(["grant_permissions_remote_example"])
        );
        assert!(!permission_step.to_string().contains("com.example.edited"));
        assert!(!permission_step.to_string().contains("root_shell"));
        let metadata = &draft.app.metadata["apk_inspection"];
        assert_eq!(
            metadata["selected_runtime_permissions"],
            serde_json::json!([
                { "permission_name": "android.permission.CAMERA", "requires_root": true },
                { "permission_name": "android.permission.RECORD_AUDIO", "requires_root": false }
            ])
        );
        assert_eq!(
            metadata["selected_app_ops"],
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
                    "required": false
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
                    "when": { "rooted": true }
                }]),
            ),
        ] {
            let source = source();
            let mut app = proposed_app(&facts(), &source);
            app.category = "emulator".to_string();
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
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
        source.asset_pattern = Some("^app\\.apk$".to_string());
        let mut app = proposed_app(&facts(), &source);
        app.category = "emulator".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
        app.category = "emulator".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
            app.category = "emulator".to_string();
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
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
        app.category = "emulator".to_string();
        app.package.primary = "com.example.edited".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: source(),
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
    fn pinned_source_emits_only_explicit_valid_trusted_sha256() {
        let mut source = source();
        source.trusted_sha256 = Some(
            " \t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n".to_string(),
        );
        let mut app = proposed_app(&facts(), &source);
        app.category = "emulator".to_string();

        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
        assert_eq!(
            draft.app.metadata["apk_inspection"]["calculated_sha256"],
            "B".repeat(64)
        );
        assert_eq!(
            draft.app.metadata["apk_inspection"]["checksum_status"],
            "not_compared"
        );
    }

    #[test]
    fn pinned_source_rejects_invalid_trusted_sha256() {
        let mut source = source();
        source.trusted_sha256 = Some("sha256:not-trusted".to_string());
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
                source.asset_pattern = Some("^app\\.apk$".to_string());
            }
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
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
                source.asset_pattern = Some("^app\\.apk$".to_string());
            }
            let mut app = proposed_app(&facts(), &source);
            app.category = "emulator".to_string();
            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: facts(),
                source,
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
    fn user_provided_strategy_preserves_phase_three_source_shape() {
        let mut source = source();
        source.strategy = "user_provided_apk".to_string();
        let app = proposed_app(&facts(), &source);
        assert_eq!(app.install_source.type_name, "user_provided_apk");
        assert_eq!(app.install_source.resolver, "none");
        assert!(app.install_source.options.is_empty());
        assert_eq!(app.tracking_source.type_name, "local_apk");
        assert!(app.tracking_source.fields.is_empty());
        assert!(!app.artifacts.apk.required);
        assert!(app.artifacts.byo_apk.required);

        let mut app = app;
        app.category = "emulator".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
        app.category = "emulator".to_string();
        app.package.primary = "com.example.edited".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
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
            app.category = "emulator".to_string();
            let mut unavailable_facts = facts();
            unavailable_facts.package_name =
                (strategy == "latest_compatible_release").then(|| "   ".to_string());

            let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
                facts: unavailable_facts,
                source,
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
        app.category = "emulator".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        assert_eq!(draft.app.install_source.resolver, "provider_latest_release");
        assert_eq!(
            draft.app.install_source.options["provider"],
            Value::String("gitlab".to_string())
        );
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
        app.category = "emulator".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source: latest,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        assert_eq!(
            draft.app.install_source.options["base_url"],
            Value::String("https://codeberg.org".to_string())
        );
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("provider: forgejo"));
        assert!(recipe.contains("base_url: https://codeberg.org"));
    }

    #[test]
    fn unsafe_download_url_is_blocking() {
        let mut source = source();
        source.download_url = "http://example.com/app.apk".to_string();
        let draft = generate_remote_app_recipe_draft(RemoteAppRecipeDraftRequest {
            facts: facts(),
            source,
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
}

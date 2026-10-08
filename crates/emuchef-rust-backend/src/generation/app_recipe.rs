//! Side-effect-free local-APK app-definition and recipe draft generation.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};

use crate::authored_models::{
    emit_app_definition_yaml, validate_app_definition, AppArtifactKind, AppArtifactSource,
    AppArtifactV1, AppDefinitionV1, AppOpAction, AppOpMode, AppPermissionSet, AppPermissionSets,
    AppRuntimePermissionAction, AppTargetKind, AppTargetLocation, AppTargetV1, OrderedValueMap,
    APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use crate::model::{
    InputDeclaration, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides, Step,
    StepCondition, StepConstraints,
};
use indexmap::IndexMap;

use super::apk::{
    build_apk_inspection_metadata, ApkInspectionFacts, ApkMetadataIssue, SelectedAppOpMetadata,
    SelectedRuntimePermissionMetadata,
};
use super::identifiers::{normalize_identifier_component, recipe_local_token};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppRecipeDraftRequest {
    pub facts: ApkInspectionFacts,
    #[serde(default)]
    pub permission_automation: Option<PermissionAutomationSelection>,
    #[serde(default)]
    pub app: Option<AppDefinitionV1>,
    #[serde(default)]
    pub recipe: Option<RecipeDraftEdits>,
    #[serde(default)]
    pub mappings: Option<AppMappingEdits>,
    #[serde(default)]
    pub regenerate_identifiers: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct GeneratedRecipeIds {
    recipe_id: String,
    input_id: String,
    feature_id: String,
    install_step_id: String,
    permission_step_id: String,
    launch_step_id: String,
}

/// Canonical permission automation resolved by the trusted Tauri boundary.
///
/// This model deliberately contains only literal values. React cannot supply
/// package names, command fragments, policy, or execution conditions directly.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PermissionAutomationSelection {
    pub(crate) package_name: String,
    #[serde(default)]
    pub(crate) runtime_permissions: Vec<RuntimePermissionSelection>,
    #[serde(default)]
    pub(crate) app_ops: Vec<AppOpPermissionSelection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RuntimePermissionSelection {
    pub(crate) permission_name: String,
    pub(crate) requires_root: bool,
    pub(crate) android_api_min: u32,
    pub(crate) android_api_max: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppOpPermissionSelection {
    pub(crate) permission_name: String,
    pub(crate) operation_name: String,
    pub(crate) mode: String,
    pub(crate) requires_root: bool,
    pub(crate) android_api_min: u32,
    pub(crate) android_api_max: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionAutomationIssue {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    pub(crate) field: String,
}

impl PermissionAutomationSelection {
    pub(crate) fn is_empty(&self) -> bool {
        self.runtime_permissions.is_empty() && self.app_ops.is_empty()
    }

    pub(crate) fn validation_issues(&self) -> Vec<PermissionAutomationIssue> {
        let mut issues = Vec::new();
        if self.package_name.trim().is_empty() {
            issues.push(PermissionAutomationIssue {
                code: "apk_permission_automation_invalid",
                message: "Permission automation requires a non-empty inspected package name.",
                field: "recipe.permissionAutomation.packageName".to_string(),
            });
        }

        let mut runtime_permissions = HashSet::new();
        for (index, action) in self.runtime_permissions.iter().enumerate() {
            let field = format!("recipe.permissionAutomation.runtimePermissions[{index}]");
            if action.permission_name.trim().is_empty() {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_invalid",
                    message: "Runtime permission automation requires a non-empty permission name.",
                    field: format!("{field}.permissionName"),
                });
            } else if !runtime_permissions.insert(action.permission_name.as_str()) {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_duplicate",
                    message: "Runtime permission automation contains a duplicate permission.",
                    field: field.clone(),
                });
            }
            validate_permission_api_bounds(
                action.android_api_min,
                action.android_api_max,
                &field,
                &mut issues,
            );
        }

        let mut app_ops = HashSet::new();
        for (index, action) in self.app_ops.iter().enumerate() {
            let field = format!("recipe.permissionAutomation.appOps[{index}]");
            if action.permission_name.trim().is_empty() {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_invalid",
                    message: "App-op automation requires a non-empty permission identity.",
                    field: format!("{field}.permissionName"),
                });
            }
            if action.operation_name.trim().is_empty() {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_invalid",
                    message: "App-op automation requires a non-empty operation name.",
                    field: format!("{field}.operationName"),
                });
            }
            if action.mode != "allow" {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_invalid",
                    message: "App-op automation supports only the reviewed allow mode.",
                    field: format!("{field}.mode"),
                });
            }
            if !app_ops.insert((
                action.permission_name.as_str(),
                action.operation_name.as_str(),
                action.mode.as_str(),
            )) {
                issues.push(PermissionAutomationIssue {
                    code: "apk_permission_automation_duplicate",
                    message: "App-op automation contains a duplicate reviewed action.",
                    field: field.clone(),
                });
            }
            validate_permission_api_bounds(
                action.android_api_min,
                action.android_api_max,
                &field,
                &mut issues,
            );
        }
        issues
    }
}

fn validate_permission_api_bounds(
    minimum: u32,
    maximum: Option<u32>,
    field: &str,
    issues: &mut Vec<PermissionAutomationIssue>,
) {
    if minimum == 0 || maximum.is_some_and(|maximum| maximum == 0 || maximum < minimum) {
        issues.push(PermissionAutomationIssue {
            code: "apk_permission_automation_invalid",
            message: "Permission automation requires a positive, ordered Android API range.",
            field: format!("{field}.androidApiMin"),
        });
    }
}

/// Build the one generated permission step consumed by the existing executor.
pub(crate) fn build_permission_step(
    selection: &PermissionAutomationSelection,
    step_id: String,
    install_step_id: String,
    app_name: &str,
) -> Step {
    let runtime = canonical_runtime_permissions(selection);
    let app_ops = canonical_app_ops(selection);

    let runtime = runtime
        .into_iter()
        .map(|action| {
            let mut value = Map::new();
            value.insert(
                "package_name".to_string(),
                Value::String(selection.package_name.clone()),
            );
            value.insert("name".to_string(), Value::String(action.permission_name));
            value.insert("required".to_string(), Value::Bool(false));
            value.insert(
                "when".to_string(),
                permission_action_condition(
                    action.requires_root,
                    action.android_api_min,
                    action.android_api_max,
                ),
            );
            Value::Object(value)
        })
        .collect();
    let app_ops = app_ops
        .into_iter()
        .map(|action| {
            let mut value = Map::new();
            value.insert(
                "package_name".to_string(),
                Value::String(selection.package_name.clone()),
            );
            value.insert("op".to_string(), Value::String(action.operation_name));
            value.insert("mode".to_string(), Value::String(action.mode));
            value.insert("required".to_string(), Value::Bool(false));
            value.insert(
                "when".to_string(),
                permission_action_condition(
                    action.requires_root,
                    action.android_api_min,
                    action.android_api_max,
                ),
            );
            Value::Object(value)
        })
        .collect();

    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        ParamValue::Literal(Value::Array(runtime)),
    );
    params.insert(
        "appops".to_string(),
        ParamValue::Literal(Value::Array(app_ops)),
    );
    params.insert(
        "policy".to_string(),
        ParamValue::Literal(json!({ "on_failure": "warn", "require_all": false })),
    );
    Step {
        id: step_id,
        type_name: "grant_permissions".to_string(),
        name: format!("Apply optional permissions for {app_name}"),
        description: Some(
            "Attempt selected runtime permission and app-op actions after installation."
                .to_string(),
        ),
        progress_note: Some(format!("Applying selected permissions for {app_name}")),
        user_toggleable: false,
        dependencies: vec![install_step_id],
        constraints: StepConstraints {
            capabilities: vec!["shell_command".to_string()],
            conflicts_with: Vec::new(),
        },
        skip_if: Vec::new(),
        params,
        verify: Vec::new(),
    }
}

fn permission_action_condition(
    requires_root: bool,
    android_api_min: u32,
    android_api_max: Option<u32>,
) -> Value {
    let mut condition = Map::new();
    if requires_root {
        condition.insert("rooted".to_string(), Value::Bool(true));
    }
    condition.insert("android_api_min".to_string(), Value::from(android_api_min));
    if let Some(maximum) = android_api_max {
        condition.insert("android_api_max".to_string(), Value::from(maximum));
    }
    Value::Object(condition)
}

fn canonical_runtime_permissions(
    selection: &PermissionAutomationSelection,
) -> Vec<RuntimePermissionSelection> {
    let mut runtime = selection.runtime_permissions.clone();
    runtime.sort_by(|left, right| {
        left.permission_name
            .cmp(&right.permission_name)
            .then_with(|| left.requires_root.cmp(&right.requires_root))
            .then_with(|| left.android_api_min.cmp(&right.android_api_min))
            .then_with(|| left.android_api_max.cmp(&right.android_api_max))
    });
    runtime.dedup_by(|left, right| left.permission_name == right.permission_name);
    runtime
}

fn canonical_app_ops(selection: &PermissionAutomationSelection) -> Vec<AppOpPermissionSelection> {
    let mut app_ops = selection.app_ops.clone();
    app_ops.sort_by(|left, right| {
        left.operation_name
            .cmp(&right.operation_name)
            .then_with(|| left.mode.cmp(&right.mode))
            .then_with(|| left.permission_name.cmp(&right.permission_name))
            .then_with(|| left.requires_root.cmp(&right.requires_root))
            .then_with(|| left.android_api_min.cmp(&right.android_api_min))
            .then_with(|| left.android_api_max.cmp(&right.android_api_max))
    });
    app_ops.dedup_by(|left, right| {
        left.permission_name == right.permission_name
            && left.operation_name == right.operation_name
            && left.mode == right.mode
    });
    app_ops
}

pub(crate) fn generated_apk_inspection_metadata(
    facts: &ApkInspectionFacts,
    selection: Option<&PermissionAutomationSelection>,
    automation_eligible: bool,
) -> Result<Value, Vec<ApkMetadataIssue>> {
    let selection = selection.filter(|selection| automation_eligible && !selection.is_empty());
    let runtime_permissions = selection
        .map(canonical_runtime_permissions)
        .unwrap_or_default()
        .into_iter()
        .map(|permission| SelectedRuntimePermissionMetadata {
            permission_name: permission.permission_name,
            requires_root: permission.requires_root,
        })
        .collect::<Vec<_>>();
    let app_ops = selection
        .map(canonical_app_ops)
        .unwrap_or_default()
        .into_iter()
        .map(|action| SelectedAppOpMetadata {
            permission_name: action.permission_name,
            operation_name: action.operation_name,
            mode: action.mode,
            requires_root: action.requires_root,
        })
        .collect::<Vec<_>>();
    build_apk_inspection_metadata(facts, &runtime_permissions, &app_ops)
}

/// Build the canonical permission sets implied by verified author selections.
///
/// The native layer partitions every verified action by its root requirement:
/// actions that run without root form the baseline set, and actions that
/// require root form the elevated set. An empty selection produces no sets,
/// and a partition with no actions is omitted.
pub(crate) fn app_permission_sets(
    selection: &PermissionAutomationSelection,
) -> Option<AppPermissionSets> {
    if selection.is_empty() {
        return None;
    }
    let mut baseline = AppPermissionSet::default();
    let mut elevated = AppPermissionSet::default();
    for action in canonical_runtime_permissions(selection) {
        let entry = AppRuntimePermissionAction {
            permission: action.permission_name,
            android_api_min: Some(i64::from(action.android_api_min)),
            android_api_max: action.android_api_max.map(i64::from),
        };
        if action.requires_root {
            elevated.runtime.push(entry);
        } else {
            baseline.runtime.push(entry);
        }
    }
    for action in canonical_app_ops(selection) {
        let entry = AppOpAction {
            op: action.operation_name,
            mode: AppOpMode::Allow,
            android_api_min: Some(i64::from(action.android_api_min)),
            android_api_max: action.android_api_max.map(i64::from),
        };
        if action.requires_root {
            elevated.app_ops.push(entry);
        } else {
            baseline.app_ops.push(entry);
        }
    }
    let present =
        |set: AppPermissionSet| (!set.runtime.is_empty() || !set.app_ops.is_empty()).then_some(set);
    Some(AppPermissionSets {
        baseline: present(baseline),
        elevated: present(elevated),
    })
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RecipeDraftEdits {
    ids: Option<GeneratedRecipeIds>,
    name: String,
    description: String,
    input_label: String,
    input_description: String,
    replace_existing: bool,
    launch_enabled: bool,
    launcher_activity: Option<String>,
}

/// Structured author edits applied to the proposed app definition.
///
/// Artifact and target rows are structured values so the trusted native layer
/// can validate them with the same rules that guard saved documents. Only the
/// opaque metadata map is retained as JSON text, so its key order survives the
/// round trip through React.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppMappingEdits {
    artifacts: Vec<AppArtifactEdit>,
    targets: Vec<AppTargetEdit>,
    metadata: String,
}

/// One authored app artifact row.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppArtifactEdit {
    id: String,
    kind: AppArtifactKind,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    source: AppArtifactSource,
}

/// One authored app target row.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppTargetEdit {
    id: String,
    kind: AppTargetKind,
    location: AppTargetLocation,
    path: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DraftSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DraftDiagnostic {
    pub(crate) severity: DraftSeverity,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) field: String,
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
pub(crate) struct AppRecipeDraft {
    app: AppDefinitionV1,
    /// Transient APK inspection evidence shown during draft review.
    ///
    /// The evidence never becomes part of the saved app definition, so the
    /// canonical YAML an author reviews matches the YAML that is written.
    apk_inspection: Option<Value>,
    recipe: Value,
    recipe_edits: RecipeDraftEdits,
    app_canonical_yaml: Option<String>,
    recipe_canonical_yaml: Option<String>,
    app_destination: ProposedDestination,
    recipe_destination: ProposedDestination,
    evidence: Vec<FieldEvidence>,
    diagnostics: Vec<DraftDiagnostic>,
    blocking: bool,
}

/// Generate or revalidate both authored documents without filesystem writes.
pub(crate) fn generate_app_recipe_draft(request: AppRecipeDraftRequest) -> AppRecipeDraft {
    let proposed_app = proposed_app(&request.facts);
    let mut app = request.app.unwrap_or_else(|| proposed_app.clone());
    let mut diagnostics = request
        .permission_automation
        .as_ref()
        .into_iter()
        .flat_map(PermissionAutomationSelection::validation_issues)
        .map(|issue| error(issue.code, issue.message, &issue.field))
        .collect::<Vec<_>>();
    if request
        .permission_automation
        .as_ref()
        .is_some_and(|selection| !selection.is_empty())
    {
        diagnostics.push(error(
            "apk_permission_automation_strategy_unsupported",
            "Permission automation is supported only for package-enforced remote APK recipes.",
            "recipe.permissionAutomation",
        ));
    }
    if let Some(mappings) = request.mappings {
        apply_mapping_edits(&mut app, mappings, &mut diagnostics);
    }
    // The generated recipe installs the local APK the author inspected, so the
    // app definition must keep declaring that artifact. Structured mapping
    // edits that remove it, retype it as a generic file, or swap in a remote
    // source would otherwise save an app definition and recipe that disagree
    // about where the installed APK comes from.
    match pair_reviewed_artifact(&app, &proposed_app, is_local_apk_artifact) {
        ReviewedArtifactPairing::Paired(_) => {}
        ReviewedArtifactPairing::Missing => diagnostics.push(error(
            "local_artifact_missing",
            "The app definition no longer declares the user-provided APK artifact this recipe installs. Keep the local APK artifact, or start a draft for a repository source.",
            "artifacts",
        )),
        ReviewedArtifactPairing::Ambiguous => diagnostics.push(error(
            "local_artifact_ambiguous",
            "The app definition declares more than one user-provided APK artifact, so the artifact this recipe installs is ambiguous. Keep exactly one local APK artifact.",
            "artifacts",
        )),
    }
    let apk_inspection = match generated_apk_inspection_metadata(&request.facts, None, false) {
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

    let mut recipe_edits = request
        .recipe
        .unwrap_or_else(|| proposed_recipe_edits(&app));
    if recipe_edits.ids.is_none() || request.regenerate_identifiers {
        recipe_edits.ids = Some(generated_ids(&app.id));
    }
    fill_empty_recipe_text(&mut recipe_edits, &app);
    app.launcher_activity = verified_launcher_activity(
        &request.facts,
        recipe_edits.launch_enabled,
        recipe_edits.launcher_activity.as_deref(),
    );
    // Local user-provided APK generation supports no permission automation, so
    // the generated app definition carries no permission sets.
    app.permission_sets = None;

    diagnostics.extend(app_diagnostics(&app));
    let recipe = build_recipe(&app, &request.facts, &recipe_edits, &mut diagnostics);
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
    let app_destination = destination("apps", &app.id, !app_has_error);
    let recipe_destination = destination("recipes", &recipe.id, !recipe_has_error);
    let blocking = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DraftSeverity::Error);

    AppRecipeDraft {
        recipe: crate::dto::recipe_to_dto(&recipe),
        recipe_edits,
        app_canonical_yaml,
        recipe_canonical_yaml,
        app_destination,
        recipe_destination,
        evidence: evidence(&request.facts, &proposed_app, &app),
        diagnostics,
        blocking,
        app,
        apk_inspection,
    }
}

/// Return the inspected launcher activity the author enabled for launch.
///
/// The app definition owns launcher identity, so the activity is recorded only
/// when the author explicitly enabled launch and APK inspection verified that
/// exact component. Inspected components arrive in the Android component form
/// (for example "com.example.app/.MainActivity"), so the recorded value is the
/// equivalent fully qualified activity class name.
pub(crate) fn verified_launcher_activity(
    facts: &ApkInspectionFacts,
    launch_enabled: bool,
    launcher_activity: Option<&str>,
) -> Option<String> {
    if !launch_enabled {
        return None;
    }
    let launcher = launcher_activity
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    facts
        .launcher_activities
        .iter()
        .any(|value| value == launcher)
        .then(|| fully_qualified_activity(launcher, facts.package_name.as_deref()))
}

/// Convert an inspected Android component name into a fully qualified class name.
///
/// The accepted inspection forms are "package/.RelativeActivity",
/// "package/fully.qualified.Activity", and ".RelativeActivity"; a value that is
/// already fully qualified is returned unchanged.
fn fully_qualified_activity(component: &str, package_name: Option<&str>) -> String {
    if let Some((package, class)) = component.split_once('/') {
        if !package.is_empty() && !class.is_empty() {
            return if class.starts_with('.') {
                format!("{package}{class}")
            } else {
                class.to_string()
            };
        }
        return component.to_string();
    }
    match package_name {
        Some(package) if component.starts_with('.') && !package.is_empty() => {
            format!("{package}{component}")
        }
        _ => component.to_string(),
    }
}

fn proposed_app(facts: &ApkInspectionFacts) -> AppDefinitionV1 {
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
            source: AppArtifactSource::UserProvided,
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

/// How the reviewed App Definition artifact of one generation flow relates to
/// the artifact registry after structured mapping edits.
pub(crate) enum ReviewedArtifactPairing {
    /// The reviewed artifact is declared under this id.
    Paired(String),
    /// No artifact represents the reviewed source.
    Missing,
    /// Several artifacts represent the reviewed source, so the reviewed
    /// artifact cannot be identified.
    Ambiguous,
}

/// Locate the App Definition artifact that represents the reviewed source of
/// one generation flow.
///
/// The generated draft proposes exactly one artifact. When the author renamed
/// that artifact, the only artifact whose definition still matches the
/// reviewed source takes its place. Several matching artifacts are ambiguous:
/// generation cannot tell which of them the reviewed source belongs to, so a
/// caller blocks instead of guessing and producing a recipe that belongs to
/// none of the authored artifacts.
pub(crate) fn pair_reviewed_artifact(
    app: &AppDefinitionV1,
    proposed: &AppDefinitionV1,
    matches: impl Fn(&AppArtifactV1) -> bool,
) -> ReviewedArtifactPairing {
    if let Some(id) = proposed.artifacts.keys().find(|id| {
        app.artifacts
            .get(*id)
            .is_some_and(|artifact| matches(artifact))
    }) {
        return ReviewedArtifactPairing::Paired(id.clone());
    }
    let mut candidates = app
        .artifacts
        .iter()
        .filter(|(_, artifact)| matches(artifact))
        .map(|(id, _)| id.clone());
    match (candidates.next(), candidates.next()) {
        (Some(id), None) => ReviewedArtifactPairing::Paired(id),
        (None, _) => ReviewedArtifactPairing::Missing,
        (Some(_), Some(_)) => ReviewedArtifactPairing::Ambiguous,
    }
}

/// Return whether one artifact is a local user-provided APK artifact, the
/// artifact the local generation flow installs.
pub(crate) fn is_local_apk_artifact(artifact: &AppArtifactV1) -> bool {
    artifact.kind == AppArtifactKind::Apk
        && matches!(artifact.source, AppArtifactSource::UserProvided)
}

fn generated_ids(app_id: &str) -> GeneratedRecipeIds {
    let token = recipe_local_token(app_id);
    GeneratedRecipeIds {
        recipe_id: format!("app.{app_id}.install"),
        input_id: format!("{token}_apk"),
        feature_id: format!("{token}_install"),
        install_step_id: format!("install_{token}"),
        permission_step_id: format!("grant_permissions_{token}"),
        launch_step_id: format!("launch_{token}"),
    }
}

fn proposed_recipe_edits(app: &AppDefinitionV1) -> RecipeDraftEdits {
    RecipeDraftEdits {
        ids: Some(generated_ids(&app.id)),
        name: format!("Install {}", app.name),
        description: format!("Install a user-provided {} APK.", app.name),
        input_label: format!("{} APK", app.name),
        input_description: format!("Local {} APK to install.", app.name),
        ..RecipeDraftEdits::default()
    }
}

fn fill_empty_recipe_text(edits: &mut RecipeDraftEdits, app: &AppDefinitionV1) {
    if edits.name.trim().is_empty() {
        edits.name = format!("Install {}", app.name);
    }
    if edits.input_label.trim().is_empty() {
        edits.input_label = format!("{} APK", app.name);
    }
}

fn build_recipe(
    app: &AppDefinitionV1,
    facts: &ApkInspectionFacts,
    edits: &RecipeDraftEdits,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> Recipe {
    let ids = edits.ids.clone().unwrap_or_else(|| generated_ids(&app.id));
    let mut inputs = OrderedMap::new();
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

    let mut install_params = OrderedMap::new();
    install_params.insert(
        "app".to_string(),
        ParamValue::Ref(format!("inputs.{}", ids.input_id)),
    );
    install_params.insert(
        "replace_existing".to_string(),
        ParamValue::Literal(Value::Bool(edits.replace_existing)),
    );
    let mut skip_params = OrderedMap::new();
    skip_params.insert(
        "package_name".to_string(),
        Value::String(app.package_id.clone()),
    );
    let install = Step {
        id: ids.install_step_id.clone(),
        type_name: "install_apk".to_string(),
        name: format!("Install {} APK", app.name),
        description: None,
        progress_note: Some(format!("Installing {} on the selected device", app.name)),
        user_toggleable: false,
        dependencies: Vec::new(),
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
    };
    let mut steps = vec![install];

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
                dependencies: vec![ids.install_step_id.clone()],
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
        artifacts: OrderedMap::new(),
        artifact_groups: OrderedMap::new(),
        steps,
    }
}

fn apply_mapping_edits(
    app: &mut AppDefinitionV1,
    mappings: AppMappingEdits,
    diagnostics: &mut Vec<DraftDiagnostic>,
) {
    app.artifacts = collect_artifacts(mappings.artifacts, diagnostics);
    app.targets = collect_targets(mappings.targets, diagnostics);
    if let Some(value) = parse_mapping("metadata", &mappings.metadata, diagnostics) {
        app.metadata = value;
    }
}

pub(crate) fn collect_artifacts(
    edits: Vec<AppArtifactEdit>,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> IndexMap<String, AppArtifactV1> {
    let mut artifacts = IndexMap::new();
    for (index, edit) in edits.into_iter().enumerate() {
        let id = edit.id.trim().to_string();
        if !claim_entry_id(&id, "artifacts", index, &mut artifacts, diagnostics) {
            continue;
        }
        artifacts.insert(
            id,
            AppArtifactV1 {
                kind: edit.kind,
                name: edit.name,
                description: edit.description,
                source: edit.source,
            },
        );
    }
    artifacts
}

pub(crate) fn collect_targets(
    edits: Vec<AppTargetEdit>,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> IndexMap<String, AppTargetV1> {
    let mut targets = IndexMap::new();
    for (index, edit) in edits.into_iter().enumerate() {
        let id = edit.id.trim().to_string();
        if !claim_entry_id(&id, "targets", index, &mut targets, diagnostics) {
            continue;
        }
        targets.insert(
            id,
            AppTargetV1 {
                kind: edit.kind,
                location: edit.location,
                path: edit.path,
            },
        );
    }
    targets
}

/// Claim one generated row identity, reporting empty and duplicate ids.
fn claim_entry_id<T>(
    id: &str,
    field: &str,
    index: usize,
    entries: &IndexMap<String, T>,
    diagnostics: &mut Vec<DraftDiagnostic>,
) -> bool {
    if id.is_empty() {
        diagnostics.push(error(
            "mapping_entry_id_invalid",
            "Every generated app definition row requires a non-empty id.",
            &format!("{field}[{index}].id"),
        ));
        return false;
    }
    if entries.contains_key(id) {
        diagnostics.push(error(
            "mapping_entry_id_duplicate",
            "Generated app definition row ids must be unique.",
            &format!("{field}[{index}].id"),
        ));
        return false;
    }
    true
}

pub(crate) fn parse_mapping(
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
    proposed: &AppDefinitionV1,
    current: &AppDefinitionV1,
) -> Vec<FieldEvidence> {
    vec![
        field_evidence(
            "package_id",
            state(facts.package_name.is_some(), EvidenceState::Verified),
            "apk_manifest",
            current.package_id != proposed.package_id,
        ),
        field_evidence(
            "name",
            state(facts.application_label.is_some(), EvidenceState::Verified),
            "apk_manifest",
            current.name != proposed.name,
        ),
        field_evidence(
            "id",
            state(!proposed.id.is_empty(), EvidenceState::Derived),
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
            EvidenceState::Suggested,
            "user_provided_apk_strategy",
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

fn state(available: bool, available_state: EvidenceState) -> EvidenceState {
    if available {
        available_state
    } else {
        EvidenceState::Missing
    }
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

pub(crate) fn error(code: &str, message: &str, field: &str) -> DraftDiagnostic {
    DraftDiagnostic {
        severity: DraftSeverity::Error,
        code: code.to_string(),
        message: message.to_string(),
        field: field.to_string(),
    }
}

pub(crate) fn warning(code: &str, message: &str, field: &str) -> DraftDiagnostic {
    DraftDiagnostic {
        severity: DraftSeverity::Warning,
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
    use crate::executor::ExecutorRunner;
    use crate::generation::apk::{ApkPermissionApplicabilityFacts, ApkPermissionDeclarationFacts};
    use crate::planner::{
        DeviceContext, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource, ExecutionStep,
        ExecutionStepConstraints, RuntimeCapabilities,
    };

    fn facts() -> ApkInspectionFacts {
        ApkInspectionFacts {
            package_name: Some("com.example.player".to_string()),
            application_label: Some("Example Player".to_string()),
            launcher_activities: vec!["com.example.player/.MainActivity".to_string()],
            calculated_sha256: "A".repeat(64),
            checksum_status: "not_compared".to_string(),
            signature_verification: "not_performed".to_string(),
            split: Some(false),
            base: Some(true),
            ..ApkInspectionFacts::default()
        }
    }

    fn reviewed_permission(name: &str) -> ApkPermissionDeclarationFacts {
        ApkPermissionDeclarationFacts {
            name: name.to_string(),
            declaration_kind: "uses_permission".to_string(),
            max_sdk_version: None,
            classification: Some("runtime_grantable".to_string()),
            applicability: Some(ApkPermissionApplicabilityFacts {
                status: "applicable".to_string(),
                reason: None,
                maximum_sdk_version: None,
                introduction_api: None,
                minimum_device_api: None,
                minimum_target_sdk: None,
                maximum_target_sdk: None,
                actual_target_sdk: None,
                target_sdk_state: None,
            }),
        }
    }

    #[test]
    fn valid_category_generates_existing_model_recipe_and_no_native_path() {
        let mut app = proposed_app(&facts());
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .is_some_and(|yaml| !yaml.contains('/')));
        let recipe = draft.recipe_canonical_yaml.unwrap();
        assert!(recipe.contains("ref: inputs.example_player_apk"));
        assert!(recipe.contains("package_name: com.example.player"));
        assert!(!recipe.contains("expected_package_name"));
        assert!(!recipe.contains("launch_app"));
    }

    #[test]
    fn launch_requires_verified_component_and_adds_one_step_when_selected() {
        let mut app = proposed_app(&facts());
        app.category = Some("utility".to_string());
        let mut edits = proposed_recipe_edits(&app);
        edits.launch_enabled = true;
        edits.launcher_activity = Some("com.example.player/.MainActivity".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: Some(edits),
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft
            .recipe_canonical_yaml
            .unwrap()
            .contains("type: launch_app"));
        assert_eq!(
            draft.app.launcher_activity.as_deref(),
            Some("com.example.player.MainActivity")
        );
    }

    #[test]
    fn local_permission_automation_is_rejected_without_emitting_a_step() {
        let mut app = proposed_app(&facts());
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![RuntimePermissionSelection {
                    permission_name: "android.permission.CAMERA".to_string(),
                    requires_root: false,
                    android_api_min: 23,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "apk_permission_automation_strategy_unsupported"
        }));
        assert!(!draft.recipe.to_string().contains("grant_permissions"));
        assert!(draft.app.permission_sets.is_none());
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
        assert_eq!(
            draft.apk_inspection.as_ref().unwrap()["selected_runtime_permissions"],
            json!([])
        );
    }

    fn generated_permission_execution_plan(
        action: RuntimePermissionSelection,
        android_api_level: i64,
    ) -> ExecutionPlan {
        let generated = build_permission_step(
            &PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![action],
                app_ops: Vec::new(),
            },
            "grant_permissions_example".to_string(),
            "install_example".to_string(),
            "Example Player",
        );
        let params = generated
            .params
            .into_iter()
            .map(|(name, value)| match value {
                ParamValue::Literal(value) => (name, ExecutionParamValue::Literal { value }),
                ParamValue::Ref(_) => panic!("generated permission params must be literal"),
            })
            .collect();
        let step = ExecutionStep {
            id: generated.id,
            recipe_ref: "example.recipe".to_string(),
            type_name: generated.type_name,
            name: generated.name,
            note: generated.progress_note.unwrap_or_default(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: generated.constraints.capabilities,
                conflicts_with: generated.constraints.conflicts_with,
            },
            params,
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        ExecutionPlan {
            id: "plan.permission-bounds".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "example.device".to_string(),
                device_plan_ref: "example.plan".to_string(),
                selected_recipe_refs: vec!["example.recipe".to_string()],
                expanded_recipe_refs: vec!["example.recipe".to_string()],
                catalog: None,
            },
            recipes: Vec::new(),
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 11,
                android_api_level: Some(android_api_level),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: vec![step],
            schema_version: 1,
            kind: "execution_plan",
        }
    }

    #[test]
    fn generated_api_max_uses_the_real_executor_condition_path() {
        let plan = generated_permission_execution_plan(
            RuntimePermissionSelection {
                permission_name: "android.permission.CAMERA".to_string(),
                requires_root: false,
                android_api_min: 23,
                android_api_max: Some(29),
            },
            30,
        );
        let mut runner = ExecutorRunner::default();
        let result = runner.run(&plan);
        let serialized = serde_json::to_value(result).expect("execution result should serialize");
        let action =
            &serialized["steps"][0]["outputs"]["permission_results"]["value"]["actions"][0];
        assert_eq!(action["status"], "not_applicable");
        assert_eq!(action["reason_code"], "android_api_out_of_range");
        assert!(runner.adapters().device().commands().is_empty());
    }

    #[test]
    fn generated_target_29_write_storage_remains_applicable_on_api_30() {
        let plan = generated_permission_execution_plan(
            RuntimePermissionSelection {
                permission_name: "android.permission.WRITE_EXTERNAL_STORAGE".to_string(),
                requires_root: false,
                android_api_min: 23,
                android_api_max: None,
            },
            30,
        );
        let mut runner = ExecutorRunner::default();
        let result = runner.run(&plan);
        let serialized = serde_json::to_value(result).expect("execution result should serialize");
        let action =
            &serialized["steps"][0]["outputs"]["permission_results"]["value"]["actions"][0];
        assert_ne!(action["status"], "not_applicable");
        assert_eq!(runner.adapters().device().commands().len(), 1);
    }

    #[test]
    fn permission_automation_contract_rejects_authored_commands_and_invalid_literals() {
        assert!(
            serde_json::from_value::<PermissionAutomationSelection>(serde_json::json!({
                "packageName": "com.example.player",
                "runtimePermissions": [{
                    "permissionName": "android.permission.CAMERA",
                    "requiresRoot": false,
                    "command": "pm grant anything"
                }],
                "appOps": []
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PermissionAutomationSelection>(serde_json::json!({
                "packageName": { "ref": "inputs.package" },
                "runtimePermissions": [],
                "appOps": []
            }))
            .is_err()
        );

        let selection = PermissionAutomationSelection {
            package_name: " ".to_string(),
            runtime_permissions: vec![RuntimePermissionSelection {
                permission_name: String::new(),
                requires_root: false,
                android_api_min: 23,
                android_api_max: None,
            }],
            app_ops: vec![AppOpPermissionSelection {
                permission_name: "android.permission.MANAGE_EXTERNAL_STORAGE".to_string(),
                operation_name: String::new(),
                mode: "deny".to_string(),
                requires_root: true,
                android_api_min: 30,
                android_api_max: None,
            }],
        };
        let issues = selection.validation_issues();
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.code == "apk_permission_automation_invalid")
                .count(),
            4
        );
    }

    #[test]
    fn nested_duplicate_json_keys_are_rejected() {
        let mut diagnostics = Vec::new();
        assert!(parse_mapping(
            "metadata",
            r#"{"outer":{"key":1,"key":2}}"#,
            &mut diagnostics
        )
        .is_none());
        assert_eq!(diagnostics[0].code, "mapping_json_invalid");
    }

    #[test]
    fn generated_apk_inspection_evidence_is_transient_and_never_saved() {
        let mut inspection = facts();
        inspection.calculated_sha256 = "ab".repeat(32);
        inspection.version_code = Some("42".to_string());
        inspection.version_name = Some("1.2".to_string());
        inspection.min_sdk = Some(23);
        inspection.target_sdk = Some(35);
        inspection.requested_permissions = vec![
            reviewed_permission("android.permission.INTERNET"),
            reviewed_permission("android.permission.CAMERA"),
            reviewed_permission("android.permission.INTERNET"),
        ];
        let mut app = proposed_app(&inspection);
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        let metadata = draft.apk_inspection.as_ref().unwrap();
        assert_eq!(metadata["package_name"], "com.example.player");
        assert_eq!(metadata["calculated_sha256"], "AB".repeat(32));
        assert_eq!(metadata["checksum_status"], "not_compared");
        assert_eq!(metadata["signature_verification"], "not_performed");
        assert_eq!(metadata["selected_runtime_permissions"], json!([]));
        assert_eq!(metadata["selected_app_ops"], json!([]));
        assert_eq!(
            metadata["requested_permissions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|permission| permission["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["android.permission.CAMERA", "android.permission.INTERNET"]
        );
        assert!(draft.app.metadata.is_empty());
        let yaml = draft.app_canonical_yaml.as_deref().unwrap();
        assert!(!yaml.contains("apk_inspection"));
    }

    #[test]
    fn structured_mapping_rows_replace_artifacts_and_targets_and_preserve_metadata_order() {
        let app = proposed_app(&facts());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![
                    AppArtifactEdit {
                        id: "apk".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: Some("Player APK".to_string()),
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                    AppArtifactEdit {
                        id: "assets".to_string(),
                        kind: AppArtifactKind::File,
                        name: None,
                        description: Some("Optional asset bundle".to_string()),
                        source: AppArtifactSource::DirectUrl {
                            url: "https://downloads.example.com/assets.zip".to_string(),
                            sha256: None,
                        },
                    },
                ],
                targets: vec![AppTargetEdit {
                    id: "config".to_string(),
                    kind: AppTargetKind::File,
                    location: AppTargetLocation::ExternalAppData,
                    path: "files/player.cfg".to_string(),
                }],
                metadata: r#"{"first":1,"last":2}"#.to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        assert_eq!(
            draft.app.artifacts.keys().collect::<Vec<_>>(),
            vec!["apk", "assets"]
        );
        assert_eq!(draft.app.targets.keys().collect::<Vec<_>>(), vec!["config"]);
        assert_eq!(
            draft
                .app
                .metadata
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["first", "last"]
        );
        let yaml = draft.app_canonical_yaml.as_deref().unwrap();
        let round_trip = crate::authored_models::parse_app_definition_yaml(yaml).unwrap();
        assert_eq!(round_trip, draft.app);
    }

    #[test]
    fn mapping_rows_reject_empty_and_duplicate_ids() {
        let app = proposed_app(&facts());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![
                    AppArtifactEdit {
                        id: " ".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: None,
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                    AppArtifactEdit {
                        id: "apk".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: None,
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                    AppArtifactEdit {
                        id: "apk".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: None,
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                ],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "mapping_entry_id_invalid"));
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "mapping_entry_id_duplicate"));
        assert_eq!(draft.app.artifacts.keys().collect::<Vec<_>>(), vec!["apk"]);
    }

    #[test]
    fn authored_metadata_rejects_the_reserved_inspection_key() {
        let app = proposed_app(&facts());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: Vec::new(),
                targets: Vec::new(),
                metadata: r#"{"apk_inspection":{"package_name":"com.example.player"}}"#.to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft.app_canonical_yaml.is_none());
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "metadata_reserved_key"));
    }

    #[test]
    fn malformed_inspection_metadata_blocks_yaml_and_is_not_persisted() {
        let mut inspection = facts();
        inspection.calculated_sha256 = "publisher-value".to_string();
        inspection.checksum_status = "verified".to_string();
        let mut app = proposed_app(&inspection);
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft.app_canonical_yaml.is_none());
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "apk_inspection_metadata_sha256_invalid"));
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "apk_inspection_metadata_checksum_status_invalid"
        }));
    }

    #[test]
    fn unsupported_permission_enums_and_api_bounds_block_metadata() {
        let mut inspection = facts();
        inspection.min_sdk = Some(36);
        inspection.target_sdk = Some(35);
        inspection.requested_permissions = vec![ApkPermissionDeclarationFacts {
            name: "android.permission.CAMERA".to_string(),
            declaration_kind: "future_permission".to_string(),
            max_sdk_version: None,
            classification: Some("future_classification".to_string()),
            applicability: Some(ApkPermissionApplicabilityFacts {
                status: "indeterminate".to_string(),
                reason: Some("target_sdk_unavailable".to_string()),
                maximum_sdk_version: None,
                introduction_api: None,
                minimum_device_api: None,
                minimum_target_sdk: Some(-1),
                maximum_target_sdk: None,
                actual_target_sdk: None,
                target_sdk_state: Some("future_state".to_string()),
            }),
        }];
        let mut app = proposed_app(&inspection);
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        for code in [
            "apk_inspection_metadata_permission_invalid",
            "apk_inspection_metadata_permission_classification_invalid",
            "apk_inspection_metadata_applicability_invalid",
            "apk_inspection_metadata_api_bounds_invalid",
            "apk_inspection_metadata_sdk_bounds_invalid",
        ] {
            assert!(
                draft
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "missing {code}"
            );
        }
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
    }

    #[test]
    fn local_mapping_edits_must_keep_the_user_provided_apk_artifact() {
        // Removing the local APK artifact leaves the generated recipe
        // installing a file the app definition no longer owns.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(proposed_app(&facts())),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_missing"));

        // Retyping the artifact as a generic file contradicts the APK input.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(proposed_app(&facts())),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![AppArtifactEdit {
                    id: "apk".to_string(),
                    kind: AppArtifactKind::File,
                    name: None,
                    description: None,
                    source: AppArtifactSource::UserProvided,
                }],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_missing"));

        // Swapping in a remote source contradicts the user-provided input.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(proposed_app(&facts())),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![AppArtifactEdit {
                    id: "apk".to_string(),
                    kind: AppArtifactKind::Apk,
                    name: None,
                    description: None,
                    source: AppArtifactSource::DirectUrl {
                        url: "https://downloads.example.com/player.apk".to_string(),
                        sha256: None,
                    },
                }],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_missing"));

        // Renaming the artifact keeps one candidate, so generation still pairs
        // the recipe input with it.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(proposed_app(&facts())),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![AppArtifactEdit {
                    id: "player_apk".to_string(),
                    kind: AppArtifactKind::Apk,
                    name: None,
                    description: None,
                    source: AppArtifactSource::UserProvided,
                }],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);

        // Two renamed candidates leave the installed artifact ambiguous.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: facts(),
            app: Some(proposed_app(&facts())),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![
                    AppArtifactEdit {
                        id: "one_apk".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: None,
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                    AppArtifactEdit {
                        id: "two_apk".to_string(),
                        kind: AppArtifactKind::Apk,
                        name: None,
                        description: None,
                        source: AppArtifactSource::UserProvided,
                    },
                ],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_ambiguous"));
    }
}

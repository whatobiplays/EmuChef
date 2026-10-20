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
    AppArtifactInputBinding, AppArtifactReference, InputDeclaration, InputValidation, OrderedMap,
    ParamValue, Recipe, RecipeArtifact, RecipeProvides, Step, StepCondition, StepConstraints,
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
    /// Whether the author enabled permission automation for this request.
    ///
    /// Permission automation is opt-in, so an absent flag means disabled and
    /// the selection below only carries trusted action identities.
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
pub(crate) struct ProposedDestination {
    pub(crate) file_name: Option<String>,
    pub(crate) relative_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppRecipeDraft {
    pub(crate) app: AppDefinitionV1,
    /// Transient APK inspection evidence shown during draft review.
    ///
    /// The evidence never becomes part of the saved app definition, so the
    /// canonical YAML an author reviews matches the YAML that is written.
    pub(crate) apk_inspection: Option<Value>,
    pub(crate) recipe: Value,
    pub(crate) recipe_edits: RecipeDraftEdits,
    pub(crate) app_canonical_yaml: Option<String>,
    pub(crate) recipe_canonical_yaml: Option<String>,
    pub(crate) app_destination: ProposedDestination,
    pub(crate) recipe_destination: ProposedDestination,
    evidence: Vec<FieldEvidence>,
    pub(crate) diagnostics: Vec<DraftDiagnostic>,
    pub(crate) blocking: bool,
}

pub(crate) fn generate_app_recipe_draft(request: AppRecipeDraftRequest) -> AppRecipeDraft {
    let existing_mode = request.existing_app.is_some();
    let permission_automation_enabled = request.permission_automation_enabled;
    let requested_selection = request
        .permission_automation
        .as_ref()
        .filter(|selection| !selection.is_empty());
    let mut diagnostics = Vec::new();
    // Permission automation is opt-in. The request flag states the author's
    // intent and the selection only carries trusted action identities, so a
    // disabled request never adds actions: selected actions submitted with the
    // flag off are contradictory and block instead of being silently dropped.
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
    if !crate::authored_models::is_valid_identifier(&artifact_id) {
        diagnostics.push(error(
            "app_artifact_id_invalid",
            "App Artifact ID must be a valid authored identifier.",
            "appArtifactId",
        ));
    }

    let proposed_app = proposed_app(&request.facts, &artifact_id);
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

    let artifact_id_is_valid = crate::authored_models::is_valid_identifier(&artifact_id);
    if existing_mode {
        if app.artifacts.contains_key(&artifact_id) {
            diagnostics.push(error(
                "app_artifact_id_collision",
                "The selected App Definition already declares this App Artifact ID; choose a different App Artifact ID.",
                "appArtifactId",
            ));
        } else if artifact_id_is_valid {
            app.artifacts
                .insert(artifact_id.clone(), proposed_local_apk_artifact(&app));
        }
    } else if let Some(mappings) = request.mappings {
        apply_mapping_edits(&mut app, mappings, &mut diagnostics);
    }

    // The generated recipe installs the reviewed local APK, so the app
    // definition must declare exactly one artifact representing that source,
    // and it must be the artifact the author selected. The selected App
    // Artifact ID is authoritative: when artifact edits declare the reviewed
    // source under a different ID, generation blocks instead of emitting a
    // recipe that references a missing artifact.
    match pair_reviewed_artifact(&app, &proposed_app, is_local_apk_artifact) {
        ReviewedArtifactPairing::Paired(paired_id) => {
            if paired_id != artifact_id {
                diagnostics.push(error(
                    "app_artifact_id_mismatch",
                    "The App Definition declares the reviewed local APK artifact under a different App Artifact ID; set appArtifactId to the declared artifact ID.",
                    "appArtifactId",
                ));
            }
        }
        ReviewedArtifactPairing::Missing => diagnostics.push(error(
            "local_artifact_missing",
            "The app definition no longer declares the user-provided APK artifact the recipe installs. Keep the local APK artifact, or start a repository-source draft.",
            "artifacts",
        )),
        ReviewedArtifactPairing::Ambiguous => diagnostics.push(error(
            "local_artifact_ambiguous",
            "The app definition declares more than one user-provided APK artifact, so the artifact the recipe installs is ambiguous. Keep exactly one local APK artifact.",
            "artifacts",
        )),
    }

    // Permission automation is reviewed evidence plus canonical App
    // permission sets: both come from an enabled selection that validates and
    // names the inspected package, and the evidence stays transient (never
    // written to persisted App metadata). A disabled request verifies nothing,
    // so every permission set the selected App Definition already declares
    // stays exactly as authored and no grant step is generated.
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
        if !crate::authored_models::is_valid_identifier(explicit_recipe_id) {
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
        &mut diagnostics,
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

fn proposed_app(facts: &ApkInspectionFacts, artifact_id: &str) -> AppDefinitionV1 {
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
    let artifact = AppArtifactV1 {
        kind: AppArtifactKind::Apk,
        name: (!name.trim().is_empty()).then(|| format!("{name} APK")),
        description: None,
        source: AppArtifactSource::UserProvided,
    };
    if crate::authored_models::is_valid_identifier(artifact_id) {
        artifacts.insert(artifact_id.to_string(), artifact);
    } else {
        artifacts.insert(DEFAULT_APP_ARTIFACT_ID.to_string(), artifact);
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
    if let Some(id) = proposed
        .artifacts
        .keys()
        .find(|id| app.artifacts.get(*id).is_some_and(&matches))
    {
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

/// Proposed user-provided APK artifact entry appended by guided generation.
fn proposed_local_apk_artifact(app: &AppDefinitionV1) -> AppArtifactV1 {
    AppArtifactV1 {
        kind: AppArtifactKind::Apk,
        name: (!app.name.trim().is_empty()).then(|| format!("{} APK", app.name)),
        description: None,
        source: AppArtifactSource::UserProvided,
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

/// Append app-aware permission steps for the App Definition permission sets.
///
/// Each present set becomes one granted-permission step that references the
/// App Definition by id; the step never copies permission facts and never
/// blocks launch because permission policy keeps failures advisory. The steps
/// are appended only for requests that enable permission automation, so a
/// disabled request executes none of the declared sets.
pub(crate) fn append_permission_steps(
    steps: &mut Vec<Step>,
    app: &AppDefinitionV1,
    permission_step_id: &str,
    install_step_id: &str,
) {
    let Some(sets) = app.permission_sets.as_ref() else {
        return;
    };
    for (kind, set) in [
        ("baseline", sets.baseline.as_ref()),
        ("elevated", sets.elevated.as_ref()),
    ] {
        let Some(set) = set else {
            continue;
        };
        if set.runtime.is_empty() && set.app_ops.is_empty() {
            continue;
        }
        let step_id = if kind == "baseline" {
            permission_step_id.to_string()
        } else {
            format!("{permission_step_id}_elevated")
        };
        let mut params = OrderedMap::new();
        params.insert(
            "permission_set".to_string(),
            ParamValue::Literal(Value::String(kind.to_string())),
        );
        params.insert(
            "policy".to_string(),
            ParamValue::Literal(json!({ "on_failure": "warn" })),
        );
        steps.push(Step {
            id: step_id,
            type_name: "grant_permissions".to_string(),
            name: format!("Grant {kind} permissions for {}", app.name),
            description: Some(format!(
                "Apply the App Definition {kind} permission set after installation."
            )),
            progress_note: Some(format!("Applying {kind} permissions for {}", app.name)),
            user_toggleable: false,
            app_ref: Some(app.id.clone()),
            dependencies: vec![install_step_id.to_string()],
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params,
            verify: Vec::new(),
        });
    }
}

/// Append author-selected permission actions to existing App Definition sets.
///
/// Existing entries and their declared order are preserved. Actions are
/// identified by their canonical identity (permission for runtime permissions,
/// operation for app-ops): an action already present with the same Android API
/// bounds and mode is reused, an action present with different bounds is
/// reported as an actionable conflict, and a duplicate is never appended.
pub(crate) fn merge_additive_permission_sets(
    app: &mut AppDefinitionV1,
    selection: &PermissionAutomationSelection,
    diagnostics: &mut Vec<DraftDiagnostic>,
) {
    let Some(selected) = app_permission_sets(selection) else {
        return;
    };
    let mut sets = app.permission_sets.take().unwrap_or(AppPermissionSets {
        baseline: None,
        elevated: None,
    });
    let mut baseline = sets.baseline.take().unwrap_or_default();
    let mut elevated = sets.elevated.take().unwrap_or_default();
    if let Some(source) = selected.baseline {
        merge_permission_set(&mut baseline, &elevated, source, "baseline", diagnostics);
    }
    if let Some(source) = selected.elevated {
        merge_permission_set(&mut elevated, &baseline, source, "elevated", diagnostics);
    }
    sets.baseline = present_permission_set(baseline);
    sets.elevated = present_permission_set(elevated);
    app.permission_sets = Some(sets);
}

/// Merge one selected permission set into its App Definition set.
///
/// Equivalence mirrors the persisted App permission identity: runtime
/// permissions match by permission name, app-ops by operation name. A selected
/// action whose bounds or mode differ from the existing declaration leaves the
/// declared entry untouched and records an actionable conflict instead of
/// manufacturing a duplicate the persisted contract rejects.
fn merge_permission_set(
    target: &mut AppPermissionSet,
    other: &AppPermissionSet,
    source: AppPermissionSet,
    set_name: &str,
    diagnostics: &mut Vec<DraftDiagnostic>,
) {
    for action in source.runtime {
        if let Some(existing) = target
            .runtime
            .iter()
            .find(|existing| existing.permission == action.permission)
        {
            if existing.android_api_min == action.android_api_min
                && existing.android_api_max == action.android_api_max
            {
                continue;
            }
            diagnostics.push(error(
                "app_permission_action_conflict",
                &format!(
                    "The selected {} permission is already declared in the App Definition {set_name} set with different Android API bounds.",
                    action.permission
                ),
                &format!("recipe.permissionAutomation.{set_name}.runtime"),
            ));
            continue;
        }
        if other
            .runtime
            .iter()
            .any(|existing| existing.permission == action.permission)
        {
            diagnostics.push(error(
                "app_permission_action_conflict",
                &format!(
                    "The selected {} permission is already declared in the other App Definition permission set.",
                    action.permission
                ),
                &format!("recipe.permissionAutomation.{set_name}.runtime"),
            ));
            continue;
        }
        target.runtime.push(action);
    }
    for action in source.app_ops {
        if let Some(existing) = target
            .app_ops
            .iter()
            .find(|existing| existing.op == action.op)
        {
            if existing.mode == action.mode
                && existing.android_api_min == action.android_api_min
                && existing.android_api_max == action.android_api_max
            {
                continue;
            }
            diagnostics.push(error(
                "app_permission_action_conflict",
                &format!(
                    "The selected {} app-op is already declared in the App Definition {set_name} set with a different mode or Android API bounds.",
                    action.op
                ),
                &format!("recipe.permissionAutomation.{set_name}.appOps"),
            ));
            continue;
        }
        if other
            .app_ops
            .iter()
            .any(|existing| existing.op == action.op)
        {
            diagnostics.push(error(
                "app_permission_action_conflict",
                &format!(
                    "The selected {} app-op is already declared in the other App Definition permission set.",
                    action.op
                ),
                &format!("recipe.permissionAutomation.{set_name}.appOps"),
            ));
            continue;
        }
        target.app_ops.push(action);
    }
}

fn present_permission_set(set: AppPermissionSet) -> Option<AppPermissionSet> {
    (!set.runtime.is_empty() || !set.app_ops.is_empty()).then_some(set)
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

fn proposed_recipe_edits(app: &AppDefinitionV1, artifact_id: &str) -> RecipeDraftEdits {
    RecipeDraftEdits {
        ids: Some(generated_ids(&app.id, artifact_id)),
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
    artifact_id: &str,
    edits: &RecipeDraftEdits,
    _diagnostics: &mut Vec<DraftDiagnostic>,
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

    // Permission automation is opt-in: a disabled request emits no grant
    // steps even when the App Definition declares permission sets, while an
    // enabled request executes the whole set each grant step references.
    if permission_automation_enabled {
        append_permission_steps(
            &mut steps,
            app,
            &ids.permission_step_id,
            &ids.install_step_id,
        );
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
        if !claim_entry_id(&id, "artifacts", index, &artifacts, diagnostics) {
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
        if !claim_entry_id(&id, "targets", index, &targets, diagnostics) {
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
    use crate::generation::apk::{ApkPermissionApplicabilityFacts, ApkPermissionDeclarationFacts};
    use crate::planner::{
        plan_execution_with_app_definitions, DeviceContext, PlannerInput, RuntimeCapabilities,
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

    fn facts_with_package(package_name: &str) -> ApkInspectionFacts {
        let mut facts = facts();
        facts.package_name = Some(package_name.to_string());
        facts
    }

    /// Collect the diagnostic codes of one generated draft for assertions.
    fn diagnostic_codes(draft: &AppRecipeDraft) -> Vec<&str> {
        draft
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect()
    }

    /// Validate the generated App/Recipe pair with the canonical planner
    /// app-authority checks used for authored catalog content, so generation
    /// tests prove the pair is admissible beyond draft diagnostics.
    fn assert_pair_authority(draft: &AppRecipeDraft) {
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        let mapping =
            serde_yaml::from_str::<serde_yaml::Mapping>(yaml).expect("recipe yaml mapping");
        let recipe =
            crate::yaml::parse_recipe_mapping(&mapping, std::path::Path::new("recipe.yaml"))
                .expect("generated recipe parses");
        let messages = crate::planner::validate_recipe_app_authority(
            &recipe,
            std::slice::from_ref(&draft.app),
        );
        assert!(
            messages.is_empty(),
            "app authority diagnostics: {messages:?}"
        );
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
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        assert!(draft
            .app_canonical_yaml
            .as_deref()
            .is_some_and(|yaml| !yaml.contains('/')));
        let recipe = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe.contains("type: app_artifact"));
        assert!(recipe.contains("app_ref: example_player"));
        assert!(recipe.contains("artifact: apk"));
        assert!(recipe.contains("ref: inputs.example_player_apk"));
        assert!(recipe.contains("type: install_apk"));
        assert!(recipe.contains("ref: artifacts.apk.local_path"));
        assert!(!recipe.contains("com.example.player"));
        assert!(!recipe.contains("expected_package_name"));
        assert!(!recipe.contains("launch_app"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn launch_requires_verified_component_and_adds_one_step_when_selected() {
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let mut edits = proposed_recipe_edits(&app, "apk");
        edits.launch_enabled = true;
        edits.launcher_activity = Some("com.example.player/.MainActivity".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: Some(edits),
            mappings: None,
            permission_automation_enabled: false,
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
    fn local_permission_automation_partitions_sets_and_emits_app_aware_steps() {
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![
                    RuntimePermissionSelection {
                        permission_name: "android.permission.CAMERA".to_string(),
                        requires_root: false,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                    RuntimePermissionSelection {
                        permission_name: "android.permission.WRITE_SECURE_SETTINGS".to_string(),
                        requires_root: true,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                ],
                app_ops: vec![AppOpPermissionSelection {
                    permission_name: "android.permission.PACKAGE_USAGE_STATS".to_string(),
                    operation_name: "android:get_usage_stats".to_string(),
                    mode: "allow".to_string(),
                    requires_root: false,
                    android_api_min: 23,
                    android_api_max: None,
                }],
            }),
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        let sets = draft.app.permission_sets.as_ref().expect("permission sets");
        let baseline = sets.baseline.as_ref().expect("baseline set");
        assert_eq!(baseline.runtime.len(), 1);
        assert_eq!(baseline.runtime[0].permission, "android.permission.CAMERA");
        assert_eq!(baseline.app_ops.len(), 1);
        assert_eq!(baseline.app_ops[0].op, "android:get_usage_stats");
        let elevated = sets.elevated.as_ref().expect("elevated set");
        assert_eq!(elevated.runtime.len(), 1);
        assert_eq!(
            elevated.runtime[0].permission,
            "android.permission.WRITE_SECURE_SETTINGS"
        );
        let recipe = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe.contains("type: grant_permissions"));
        assert!(recipe.contains("permission_set: baseline"));
        assert!(recipe.contains("permission_set: elevated"));
        assert!(recipe.contains("on_failure: warn"));
        assert!(recipe.contains("app_ref: example_player"));
        assert!(!recipe.contains("runtime:"));
        assert!(!recipe.contains("com.example.player"));
        assert!(!draft.app.metadata.contains_key("apk_inspection"));
        let metadata = draft
            .apk_inspection
            .as_ref()
            .expect("apk inspection evidence");
        assert_eq!(
            metadata["selected_runtime_permissions"],
            json!([
                {
                    "permission_name": "android.permission.CAMERA",
                    "requires_root": false
                },
                {
                    "permission_name": "android.permission.WRITE_SECURE_SETTINGS",
                    "requires_root": true
                }
            ])
        );
        assert_eq!(
            metadata["selected_app_ops"],
            json!([
                {
                    "permission_name": "android.permission.PACKAGE_USAGE_STATS",
                    "operation_name": "android:get_usage_stats",
                    "mode": "allow",
                    "requires_root": false
                }
            ])
        );
        assert_pair_authority(&draft);
    }

    /// Build one generated new-App draft that opts into launching the app once
    /// and carries both a non-root and a root-required runtime permission.
    fn generated_additive_permission_draft() -> AppRecipeDraft {
        let app = proposed_app(&facts(), "apk");
        let mut recipe = proposed_recipe_edits(&app, "apk");
        recipe.launch_enabled = true;
        recipe.launcher_activity = Some("com.example.player/.MainActivity".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: Some(recipe),
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![
                    RuntimePermissionSelection {
                        permission_name: "android.permission.CAMERA".to_string(),
                        requires_root: false,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                    RuntimePermissionSelection {
                        permission_name: "android.permission.WRITE_SECURE_SETTINGS".to_string(),
                        requires_root: true,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                ],
                app_ops: Vec::new(),
            }),
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        draft
    }

    /// SHA-256 hex digest used for deterministic byte comparisons.
    fn permission_sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(bytes);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Canonical documents, diagnostics, and step lines of one generated draft.
    fn permission_artifact_sections(draft: &AppRecipeDraft) -> [String; 4] {
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
    fn permission_document_block(draft: &AppRecipeDraft) -> String {
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
    fn permission_step_generation_pins_local_canonical_documents_and_merge_diagnostics() {
        let generated = generated_additive_permission_draft();
        println!("=== CASE local:new-app ===");
        println!("{}", permission_document_block(&generated));
        println!("=== END CASE local:new-app ===");
        let [app_yaml, recipe_yaml, diagnostics, steps] = permission_artifact_sections(&generated);
        assert_eq!(
            permission_sha256_hex(app_yaml.as_bytes()),
            "21b39e57cd1d75353241e3e9bfea7c01be3cfa71a6ac322b8995f5fb4e137064"
        );
        assert_eq!(
            permission_sha256_hex(recipe_yaml.as_bytes()),
            "669ffb6c0231d3472e26b7caf7f8588e6ec4f2821f34b794a21c09649bb42660"
        );
        assert_eq!(
            permission_sha256_hex(diagnostics.as_bytes()),
            "7cb7f244325c8fd86175ba28c9a645b3b47ac8f689a1a1039c7e4c77f0ef7d04"
        );
        assert_eq!(
            permission_sha256_hex(steps.as_bytes()),
            "37e96274a12976cf68212b8d46ca220d4c19fc87f99c2d157f5031ff7b282096"
        );
        println!(
            "HASH local:new-app app={} recipe={} diagnostics={} steps={}",
            permission_sha256_hex(app_yaml.as_bytes()),
            permission_sha256_hex(recipe_yaml.as_bytes()),
            permission_sha256_hex(diagnostics.as_bytes()),
            permission_sha256_hex(steps.as_bytes()),
        );

        let mut existing = proposed_app(&facts(), "apk");
        existing.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: None,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let merged = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(existing),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![
                    RuntimePermissionSelection {
                        permission_name: "android.permission.CAMERA".to_string(),
                        requires_root: false,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                    RuntimePermissionSelection {
                        permission_name: "android.permission.ACCESS_FINE_LOCATION".to_string(),
                        requires_root: false,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                    RuntimePermissionSelection {
                        permission_name: "android.permission.WRITE_SECURE_SETTINGS".to_string(),
                        requires_root: true,
                        android_api_min: 23,
                        android_api_max: None,
                    },
                ],
                app_ops: Vec::new(),
            }),
            regenerate_identifiers: false,
        });
        println!("=== CASE local:existing-app-merge ===");
        println!("{}", permission_document_block(&merged));
        println!("=== END CASE local:existing-app-merge ===");
        let [app_yaml, recipe_yaml, diagnostics, steps] = permission_artifact_sections(&merged);
        assert_eq!(
            permission_sha256_hex(app_yaml.as_bytes()),
            "ceaadf3d8ace7df42f90254a75289e3d9c29992cf592bb929f819caa101c5812"
        );
        assert_eq!(
            permission_sha256_hex(recipe_yaml.as_bytes()),
            "6e9c960fd025b2e5b1cbceebc7e7b31d041c4c352987f3b56b07f14ad43abca7"
        );
        assert_eq!(
            permission_sha256_hex(diagnostics.as_bytes()),
            "15b177e49c63f04a12dedc9d197c34e3c31df52bc1ac89bccf68344255b3c689"
        );
        assert_eq!(
            permission_sha256_hex(steps.as_bytes()),
            "03e380f43ce50ebf6f1ef657333cc18fb2e85e9e894ae81ca046c76352870e9a"
        );
        println!(
            "HASH local:existing-app-merge app={} recipe={} diagnostics={} steps={}",
            permission_sha256_hex(app_yaml.as_bytes()),
            permission_sha256_hex(recipe_yaml.as_bytes()),
            permission_sha256_hex(diagnostics.as_bytes()),
            permission_sha256_hex(steps.as_bytes()),
        );
    }

    /// Look up one generated step id from the canonical Recipe YAML.
    fn generated_recipe_step_id(draft: &AppRecipeDraft, type_name: &str, elevated: bool) -> String {
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        let mapping =
            serde_yaml::from_str::<serde_yaml::Mapping>(yaml).expect("recipe yaml mapping");
        let recipe =
            crate::yaml::parse_recipe_mapping(&mapping, std::path::Path::new("recipe.yaml"))
                .expect("generated recipe parses");
        recipe
            .steps
            .iter()
            .find(|step| step.type_name == type_name && step.id.ends_with("_elevated") == elevated)
            .map(|step| step.id.clone())
            .unwrap_or_else(|| {
                panic!("generated recipe declares {type_name} step (elevated: {elevated})")
            })
    }

    /// Plan the generated App/Recipe pair with the canonical planner and a
    /// supplied local APK binding, using the same roadmap a device would.
    fn generated_pair_planner_input(
        draft: &AppRecipeDraft,
        apk_path: &std::path::Path,
        root_shell: bool,
    ) -> PlannerInput {
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        let mapping =
            serde_yaml::from_str::<serde_yaml::Mapping>(yaml).expect("recipe yaml mapping");
        let recipe =
            crate::yaml::parse_recipe_mapping(&mapping, std::path::Path::new("recipe.yaml"))
                .expect("generated recipe parses");
        let input_id = recipe
            .inputs
            .keys()
            .next()
            .expect("generated recipe declares one artifact input")
            .clone();
        let mut explicit_input_bindings = OrderedMap::new();
        explicit_input_bindings.insert(
            format!("{}/{}", recipe.id, input_id),
            json!(apk_path.to_string_lossy().to_string()),
        );
        let recipe_ref = recipe.id.clone();
        PlannerInput {
            recipes: vec![recipe],
            selected_recipe_refs: vec![recipe_ref],
            explicit_input_bindings,
            user_configuration_bindings: OrderedMap::new(),
            device_plan_input_bindings: OrderedMap::new(),
            plan_id: "plan.generated-pair".to_string(),
            device_plan_ref: "example.device_plan".to_string(),
            device_profile_ref: "example.device_profile".to_string(),
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example Handheld".to_string(),
                android_version: 13,
                android_api_level: Some(33),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: true,
                root_shell,
                app_data_write: true,
            },
            catalog_identity: None,
            target_device: None,
            device_plan_presentation: None,
        }
    }

    #[test]
    fn generated_pair_selects_elevated_permissions_only_for_root_capable_targets() {
        let draft = generated_additive_permission_draft();
        assert_pair_authority(&draft);
        let install = generated_recipe_step_id(&draft, "install_apk", false);
        let baseline = generated_recipe_step_id(&draft, "grant_permissions", false);
        let elevated = generated_recipe_step_id(&draft, "grant_permissions", true);
        let launch = generated_recipe_step_id(&draft, "launch_app", false);
        let workspace = tempfile::tempdir().expect("generation test temp root");
        let apk_path = workspace.path().join("example-player.apk");
        std::fs::write(&apk_path, b"test apk").expect("write test apk");

        let rooted = plan_execution_with_app_definitions(
            generated_pair_planner_input(&draft, &apk_path, true),
            std::slice::from_ref(&draft.app),
        );
        assert!(
            rooted.errors.is_empty(),
            "rooted plan diagnostics: {:?}",
            rooted.errors
        );
        let plan = rooted.execution_plan.expect("rooted execution plan");
        let step_ids = plan
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();
        for expected in [&install, &baseline, &elevated, &launch] {
            assert!(
                step_ids.iter().any(|id| id.ends_with(expected.as_str())),
                "rooted plan is missing generated step {expected}: {step_ids:?}"
            );
        }
    }

    #[test]
    fn existing_app_permission_merge_reuses_identical_actions_and_blocks_differing_policies() {
        // The reviewed App already declares CAMERA with no API bounds. The
        // selected action uses the same permission with explicit bounds, so the
        // declaration is preserved and the differing policy is reported instead
        // of appending a duplicate the persisted contract rejects.
        let mut existing = proposed_app(&facts(), "apk");
        existing.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: None,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let request = AppRecipeDraftRequest {
            existing_app: Some(existing),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
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
        };
        let unbounded = generate_app_recipe_draft(request);
        assert!(unbounded.blocking, "{unbounded:#?}");
        assert!(
            diagnostic_codes(&unbounded).contains(&"app_permission_action_conflict"),
            "{unbounded:#?}"
        );
        let kept = unbounded
            .app
            .permission_sets
            .as_ref()
            .unwrap()
            .baseline
            .as_ref()
            .unwrap();
        assert_eq!(kept.runtime.len(), 1, "no duplicate is manufactured");
        assert_eq!(kept.runtime[0].android_api_min, None);
        assert_eq!(kept.runtime[0].permission, "android.permission.CAMERA");

        // An identical declaration is reused without a duplicate or diagnostic.
        let mut identical = proposed_app(&facts(), "apk");
        identical.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let reuse = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(identical),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
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
        assert!(!reuse.blocking, "{reuse:#?}");
        let reused = reuse
            .app
            .permission_sets
            .as_ref()
            .unwrap()
            .baseline
            .as_ref()
            .unwrap();
        assert_eq!(reused.runtime.len(), 1);
        assert!(
            !diagnostic_codes(&reuse).contains(&"app_permission_action_conflict"),
            "{reuse:#?}"
        );

        // App-ops follow the same canonical-identity rule: an existing
        // operation with different Android API bounds is a conflict, never a
        // duplicate.
        let mut app_ops = proposed_app(&facts(), "apk");
        app_ops.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: Vec::new(),
                app_ops: vec![AppOpAction {
                    op: "android:get_usage_stats".to_string(),
                    mode: AppOpMode::Allow,
                    android_api_min: Some(30),
                    android_api_max: None,
                }],
            }),
            elevated: None,
        });
        let op_conflict = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(app_ops),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: Vec::new(),
                app_ops: vec![AppOpPermissionSelection {
                    permission_name: "android.permission.PACKAGE_USAGE_STATS".to_string(),
                    operation_name: "android:get_usage_stats".to_string(),
                    mode: "allow".to_string(),
                    requires_root: false,
                    android_api_min: 31,
                    android_api_max: None,
                }],
            }),
            regenerate_identifiers: false,
        });
        assert!(op_conflict.blocking, "{op_conflict:#?}");
        assert!(
            diagnostic_codes(&op_conflict).contains(&"app_permission_action_conflict"),
            "{op_conflict:#?}"
        );
        let kept_ops = op_conflict
            .app
            .permission_sets
            .as_ref()
            .unwrap()
            .baseline
            .as_ref()
            .unwrap();
        assert_eq!(kept_ops.app_ops.len(), 1);
        assert_eq!(kept_ops.app_ops[0].android_api_min, Some(30));
    }

    #[test]
    fn existing_app_app_op_merge_reuses_identical_actions_and_blocks_mode_conflicts() {
        // App-op merging identifies an action by its operation name and treats
        // mode plus Android API bounds as the declared policy. An identical
        // declaration is reused, while a same-operation action whose selected
        // mode cannot match the allow-only declaration blocks instead of
        // appending a duplicate.

        // An identical declaration (same operation, mode, and bounds) is reused
        // without a duplicate or diagnostic.
        let mut identical = proposed_app(&facts(), "apk");
        identical.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: Vec::new(),
                app_ops: vec![AppOpAction {
                    op: "android:get_usage_stats".to_string(),
                    mode: AppOpMode::Allow,
                    android_api_min: Some(31),
                    android_api_max: None,
                }],
            }),
            elevated: None,
        });
        let reuse = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(identical),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: Vec::new(),
                app_ops: vec![AppOpPermissionSelection {
                    permission_name: "android.permission.PACKAGE_USAGE_STATS".to_string(),
                    operation_name: "android:get_usage_stats".to_string(),
                    mode: "allow".to_string(),
                    requires_root: false,
                    android_api_min: 31,
                    android_api_max: None,
                }],
            }),
            regenerate_identifiers: false,
        });
        assert!(!reuse.blocking, "{reuse:#?}");
        let reused_ops = reuse
            .app
            .permission_sets
            .as_ref()
            .unwrap()
            .baseline
            .as_ref()
            .unwrap();
        assert_eq!(reused_ops.app_ops.len(), 1, "no duplicate is manufactured");
        assert_eq!(reused_ops.app_ops[0].op, "android:get_usage_stats");
        assert_eq!(reused_ops.app_ops[0].mode, AppOpMode::Allow);
        assert_eq!(reused_ops.app_ops[0].android_api_min, Some(31));
        assert!(
            !diagnostic_codes(&reuse).contains(&"app_permission_action_conflict"),
            "{reuse:#?}"
        );

        // Mode identity: the selected operation, its required permission, and
        // its Android API bounds are identical, so only the mode differs. The
        // allow-only schema cannot represent the selection, so generation
        // blocks with a mode diagnostic and preserves the declaration.
        let mut mode_declared = proposed_app(&facts(), "apk");
        mode_declared.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: Vec::new(),
                app_ops: vec![AppOpAction {
                    op: "android:get_usage_stats".to_string(),
                    mode: AppOpMode::Allow,
                    android_api_min: Some(31),
                    android_api_max: None,
                }],
            }),
            elevated: None,
        });
        let mode_conflict = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(mode_declared),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: Vec::new(),
                app_ops: vec![AppOpPermissionSelection {
                    permission_name: "android.permission.PACKAGE_USAGE_STATS".to_string(),
                    operation_name: "android:get_usage_stats".to_string(),
                    mode: "deny".to_string(),
                    requires_root: false,
                    android_api_min: 31,
                    android_api_max: None,
                }],
            }),
            regenerate_identifiers: false,
        });
        assert!(mode_conflict.blocking, "{mode_conflict:#?}");
        assert!(
            mode_conflict.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "apk_permission_automation_invalid"
                    && diagnostic.field == "recipe.permissionAutomation.appOps[0].mode"
            }),
            "{mode_conflict:#?}"
        );
        let kept_mode_ops = mode_conflict
            .app
            .permission_sets
            .as_ref()
            .unwrap()
            .baseline
            .as_ref()
            .unwrap();
        assert_eq!(
            kept_mode_ops.app_ops.len(),
            1,
            "no duplicate is manufactured"
        );
        assert_eq!(kept_mode_ops.app_ops[0].mode, AppOpMode::Allow);
        assert_eq!(kept_mode_ops.app_ops[0].android_api_min, Some(31));
    }

    #[test]
    fn existing_app_permission_merge_rejects_actions_declared_in_the_sibling_set() {
        // One App Definition permission identity cannot appear in both the
        // baseline and elevated sets. When a selected root-required action
        // targets the sibling set of an existing declaration, generation blocks
        // and leaves both sets exactly as authored.

        // App-op declared in the baseline set, selected as a root-required
        // elevated action.
        let mut op_sibling = proposed_app(&facts(), "apk");
        op_sibling.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: Vec::new(),
                app_ops: vec![AppOpAction {
                    op: "android:get_usage_stats".to_string(),
                    mode: AppOpMode::Allow,
                    android_api_min: Some(30),
                    android_api_max: None,
                }],
            }),
            elevated: None,
        });
        let op_cross_set = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(op_sibling),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: Vec::new(),
                app_ops: vec![AppOpPermissionSelection {
                    permission_name: "android.permission.PACKAGE_USAGE_STATS".to_string(),
                    operation_name: "android:get_usage_stats".to_string(),
                    mode: "allow".to_string(),
                    requires_root: true,
                    android_api_min: 30,
                    android_api_max: None,
                }],
            }),
            regenerate_identifiers: false,
        });
        assert!(op_cross_set.blocking, "{op_cross_set:#?}");
        assert!(
            op_cross_set.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "app_permission_action_conflict"
                    && diagnostic.field == "recipe.permissionAutomation.elevated.appOps"
            }),
            "{op_cross_set:#?}"
        );
        let op_sets = op_cross_set.app.permission_sets.as_ref().unwrap();
        let op_baseline = op_sets.baseline.as_ref().unwrap();
        assert_eq!(
            op_baseline.app_ops.len(),
            1,
            "the declared entry is preserved"
        );
        assert_eq!(op_baseline.app_ops[0].android_api_min, Some(30));
        assert!(
            op_sets.elevated.is_none(),
            "the sibling-set action is rejected, not appended"
        );

        // Runtime permission declared in the baseline set, selected as a
        // root-required elevated action.
        let mut runtime_sibling = proposed_app(&facts(), "apk");
        runtime_sibling.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: None,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        let runtime_cross_set = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(runtime_sibling),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.example.player".to_string(),
                runtime_permissions: vec![RuntimePermissionSelection {
                    permission_name: "android.permission.CAMERA".to_string(),
                    requires_root: true,
                    android_api_min: 23,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            regenerate_identifiers: false,
        });
        assert!(runtime_cross_set.blocking, "{runtime_cross_set:#?}");
        assert!(
            runtime_cross_set.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "app_permission_action_conflict"
                    && diagnostic.field == "recipe.permissionAutomation.elevated.runtime"
            }),
            "{runtime_cross_set:#?}"
        );
        let runtime_sets = runtime_cross_set.app.permission_sets.as_ref().unwrap();
        let runtime_baseline = runtime_sets.baseline.as_ref().unwrap();
        assert_eq!(
            runtime_baseline.runtime.len(),
            1,
            "the declared entry is preserved"
        );
        assert_eq!(runtime_baseline.runtime[0].android_api_min, None);
        assert!(
            runtime_sets.elevated.is_none(),
            "the sibling-set action is rejected, not appended"
        );
    }

    #[test]
    fn generated_pair_keeps_install_and_launch_when_elevated_permissions_are_pruned() {
        let draft = generated_additive_permission_draft();
        let install = generated_recipe_step_id(&draft, "install_apk", false);
        let baseline = generated_recipe_step_id(&draft, "grant_permissions", false);
        let elevated = generated_recipe_step_id(&draft, "grant_permissions", true);
        let launch = generated_recipe_step_id(&draft, "launch_app", false);
        let workspace = tempfile::tempdir().expect("generation test temp root");
        let apk_path = workspace.path().join("example-player.apk");
        std::fs::write(&apk_path, b"test apk").expect("write test apk");

        let unrooted = plan_execution_with_app_definitions(
            generated_pair_planner_input(&draft, &apk_path, false),
            std::slice::from_ref(&draft.app),
        );
        assert!(
            unrooted.errors.is_empty(),
            "non-root plan diagnostics: {:?}",
            unrooted.errors
        );
        let plan = unrooted.execution_plan.expect("non-root execution plan");
        let step_ids = plan
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>();
        assert!(
            !step_ids.iter().any(|id| id.ends_with(elevated.as_str())),
            "root-required permission step must not be planned for a non-root target: {step_ids:?}"
        );
        for expected in [&install, &baseline, &launch] {
            assert!(
                step_ids.iter().any(|id| id.ends_with(expected.as_str())),
                "non-root plan is missing generated step {expected}: {step_ids:?}"
            );
        }
        for step in &plan.steps {
            for dependency in &step.dependencies {
                assert!(
                    step_ids.contains(&dependency.as_str()),
                    "step {} depends on unplanned step {dependency}",
                    step.id
                );
            }
        }
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
        let mut app = proposed_app(&inspection, "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
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
        let app = proposed_app(&facts(), "apk");
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
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
            permission_automation_enabled: false,
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
        let app = proposed_app(&facts(), "apk");
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
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
            permission_automation_enabled: false,
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
        let app = proposed_app(&facts(), "apk");
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: Vec::new(),
                targets: Vec::new(),
                metadata: r#"{"apk_inspection":{"package_name":"com.example.player"}}"#.to_string(),
            }),
            permission_automation_enabled: false,
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
        let mut app = proposed_app(&inspection, "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
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
        let mut app = proposed_app(&inspection, "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
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
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
            recipe: None,
            mappings: Some(AppMappingEdits {
                artifacts: vec![],
                targets: Vec::new(),
                metadata: "{}".to_string(),
            }),
            permission_automation_enabled: false,
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
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
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
            permission_automation_enabled: false,
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
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
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
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_missing"));

        // Renaming the artifact registry entry while the draft still selects
        // the default "apk" App Artifact ID blocks: the recipe must reference
        // exactly the selected artifact.
        let renamed_mappings = || AppMappingEdits {
            artifacts: vec![AppArtifactEdit {
                id: "player_apk".to_string(),
                kind: AppArtifactKind::Apk,
                name: None,
                description: None,
                source: AppArtifactSource::UserProvided,
            }],
            targets: Vec::new(),
            metadata: "{}".to_string(),
        };
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
            recipe: None,
            mappings: Some(renamed_mappings()),
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "app_artifact_id_mismatch"));

        // Selecting the renamed artifact ID keeps the generated pair
        // consistent and admissible.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: Some("player_apk".to_string()),
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
            recipe: None,
            mappings: Some(renamed_mappings()),
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:#?}", draft.diagnostics);
        let recipe = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe.contains("artifact: player_apk"));
        assert!(recipe.contains("ref: artifacts.player_apk.local_path"));
        assert!(recipe.contains("ref: inputs.example_player_player_apk_apk"));
        assert!(!recipe.contains("artifacts.apk.local_path"));
        assert_pair_authority(&draft);

        // Two renamed candidates leave the installed artifact ambiguous.
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(proposed_app(&facts(), "apk")),
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
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "local_artifact_ambiguous"));
    }

    #[test]
    fn existing_app_generation_appends_artifact_and_preserves_app_facts() {
        let mut base = proposed_app(&facts(), "apk");
        base.id = "armsx1".to_string();
        base.name = "Armsx1".to_string();
        base.package_id = "com.armsx1".to_string();
        base.launcher_activity = Some("com.armsx1.MainActivity".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(base),
            app_artifact_id: Some("retroarch_apk".to_string()),
            recipe_id: None,
            facts: facts_with_package("com.armsx1"),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        assert_eq!(draft.app.id, "armsx1");
        assert_eq!(draft.app.package_id, "com.armsx1");
        assert_eq!(
            draft.app.launcher_activity.as_deref(),
            Some("com.armsx1.MainActivity")
        );
        assert!(draft.app.artifacts.contains_key("apk"));
        assert_eq!(draft.app.artifacts.len(), 2);
        let added = draft
            .app
            .artifacts
            .get("retroarch_apk")
            .expect("appended artifact");
        assert_eq!(added.kind, AppArtifactKind::Apk);
        assert!(matches!(added.source, AppArtifactSource::UserProvided));
        let recipe = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe.contains("id: app.armsx1.retroarch_apk.install"));
        assert!(recipe.contains("app_ref: armsx1"));
        assert!(recipe.contains("artifact: retroarch_apk"));
        assert!(recipe.contains("ref: artifacts.retroarch_apk.local_path"));
        assert!(recipe.contains("ref: inputs.armsx1_retroarch_apk_apk"));
        assert!(!recipe.contains("com.armsx1"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn explicit_identifiers_are_retained_without_auto_renaming() {
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: Some("armsx1_apk".to_string()),
            recipe_id: Some("app.armsx1.armsx1_apk.install".to_string()),
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: true,
        });
        assert!(!draft.blocking, "{:?}", draft.diagnostics);
        assert!(draft.app.artifacts.contains_key("armsx1_apk"));
        assert!(!draft.app.artifacts.contains_key("apk"));
        let recipe = draft.recipe_canonical_yaml.as_deref().unwrap();
        assert!(recipe.contains("id: app.armsx1.armsx1_apk.install"));
        assert!(recipe.contains("artifact: armsx1_apk"));
        assert!(recipe.contains("ref: artifacts.armsx1_apk.local_path"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn existing_app_artifact_id_collision_blocks_without_replacing_the_artifact() {
        let mut base = proposed_app(&facts(), "apk");
        base.id = "armsx1".to_string();
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(base),
            app_artifact_id: Some("apk".to_string()),
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "app_artifact_id_collision"));
        assert_eq!(draft.app.artifacts.len(), 1);
        assert!(draft.app_canonical_yaml.is_none());
    }

    #[test]
    fn existing_app_generation_blocks_mismatched_inspected_package() {
        let mut base = proposed_app(&facts(), "apk");
        base.id = "armsx1".to_string();
        base.name = "Armsx1".to_string();
        base.package_id = "com.armsx1".to_string();
        base.launcher_activity = Some("com.armsx1.MainActivity".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(base),
            app_artifact_id: Some("retroarch_apk".to_string()),
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "app_package_mismatch"));
        assert!(draft.app_canonical_yaml.is_none());
    }

    #[test]
    fn new_app_generation_blocks_authored_package_mismatch() {
        let mut app = proposed_app(&facts(), "apk");
        app.package_id = "com.other.app".to_string();
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(draft.blocking);
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "app_package_mismatch"));
        assert!(draft.app_canonical_yaml.is_none());
    }

    #[test]
    fn local_permission_automation_requires_the_inspected_package() {
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: Some(PermissionAutomationSelection {
                package_name: "com.other.package".to_string(),
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
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "apk_permission_automation_package_mismatch"));
        assert!(draft.app.permission_sets.is_none());
        assert!(draft
            .apk_inspection
            .as_ref()
            .is_some_and(|metadata| metadata["selected_runtime_permissions"] == json!([])));
    }

    /// Build one existing-App snapshot that declares a baseline runtime
    /// permission and a root-required elevated app-op.
    fn existing_app_with_declared_permission_sets() -> AppDefinitionV1 {
        let mut app = proposed_app(&facts(), "apk");
        app.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: Some(AppPermissionSet {
                runtime: Vec::new(),
                app_ops: vec![AppOpAction {
                    op: "android:write_secure_settings".to_string(),
                    mode: AppOpMode::Allow,
                    android_api_min: Some(23),
                    android_api_max: None,
                }],
            }),
        });
        app
    }

    #[test]
    fn absent_permission_automation_flag_preserves_declared_sets_without_grant_steps() {
        let existing = existing_app_with_declared_permission_sets();
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(existing),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{draft:#?}");
        let sets = draft
            .app
            .permission_sets
            .as_ref()
            .expect("declared permission sets survive generation");
        assert_eq!(
            sets.baseline
                .as_ref()
                .map(|set| set.runtime[0].permission.as_str()),
            Some("android.permission.CAMERA")
        );
        assert_eq!(sets.elevated.as_ref().map(|set| set.app_ops.len()), Some(1));
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        assert!(yaml.contains("type: install_apk"));
        assert!(!yaml.contains("grant_permissions"));
        assert!(!yaml.contains("permission_set"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn disabled_permission_automation_blocks_selected_actions() {
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: false,
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
        assert!(draft.blocking, "{draft:#?}");
        assert!(
            diagnostic_codes(&draft).contains(&"apk_permission_automation_disabled"),
            "{draft:#?}"
        );
        assert!(draft.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "apk_permission_automation_disabled"
                && diagnostic.field == "recipe.permissionAutomation.enabled"
        }));
        assert!(draft.app.permission_sets.is_none());
        assert!(draft.recipe_canonical_yaml.is_none());
        assert!(draft
            .apk_inspection
            .as_ref()
            .is_some_and(|metadata| metadata["selected_runtime_permissions"] == json!([])));
    }

    #[test]
    fn enabled_permission_automation_without_selection_executes_declared_sets() {
        let existing = existing_app_with_declared_permission_sets();
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: Some(existing),
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: None,
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{draft:#?}");
        let sets = draft
            .app
            .permission_sets
            .as_ref()
            .expect("declared permission sets survive generation");
        assert_eq!(sets.baseline.as_ref().map(|set| set.runtime.len()), Some(1));
        assert_eq!(sets.elevated.as_ref().map(|set| set.app_ops.len()), Some(1));
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        assert!(yaml.contains("type: grant_permissions"));
        assert!(yaml.contains("permission_set: baseline"));
        assert!(yaml.contains("permission_set: elevated"));
        assert!(yaml.contains("on_failure: warn"));
        assert_pair_authority(&draft);
    }

    #[test]
    fn enabled_permission_automation_without_declared_sets_emits_no_grant_steps() {
        let mut app = proposed_app(&facts(), "apk");
        app.category = Some("utility".to_string());
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            existing_app: None,
            app_artifact_id: None,
            recipe_id: None,
            facts: facts(),
            app: Some(app),
            recipe: None,
            mappings: None,
            permission_automation_enabled: true,
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking, "{draft:#?}");
        assert!(draft.app.permission_sets.is_none());
        let yaml = draft
            .recipe_canonical_yaml
            .as_deref()
            .expect("generated recipe canonical yaml");
        assert!(!yaml.contains("grant_permissions"));
        assert!(!yaml.contains("permission_set"));
        assert_pair_authority(&draft);
    }
}

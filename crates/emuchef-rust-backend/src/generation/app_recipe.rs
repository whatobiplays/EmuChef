//! Side-effect-free local-APK app-definition and recipe draft generation.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};

use crate::authored_models::{
    emit_app_definition_yaml, validate_app_definition, AppArtifactSupport, AppDefinitionV1,
    AppInstallSource, AppPackage, AppProvisioning, AppTrackingSource, ConfigArtifactSupport,
    OrderedValueMap, RequiredArtifactSupport, APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use crate::model::{
    InputDeclaration, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides, Step,
    StepCondition, StepConstraints,
};

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
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppOpPermissionSelection {
    pub(crate) permission_name: String,
    pub(crate) operation_name: String,
    pub(crate) mode: String,
    pub(crate) requires_root: bool,
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
                    field,
                });
            }
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
                    field,
                });
            }
        }
        issues
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
            if action.requires_root {
                value.insert("when".to_string(), json!({ "rooted": true }));
            }
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
            if action.requires_root {
                value.insert("when".to_string(), json!({ "rooted": true }));
            }
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

fn canonical_runtime_permissions(
    selection: &PermissionAutomationSelection,
) -> Vec<RuntimePermissionSelection> {
    let mut runtime = selection.runtime_permissions.clone();
    runtime.sort_by(|left, right| {
        left.permission_name
            .cmp(&right.permission_name)
            .then_with(|| left.requires_root.cmp(&right.requires_root))
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

/// JSON mapping text is retained as text across React so key order is not lost.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppMappingEdits {
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
pub(crate) struct AppRecipeDraft {
    app: AppDefinitionV1,
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
    app.metadata.shift_remove("apk_inspection");
    match generated_apk_inspection_metadata(&request.facts, None, false) {
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
        .unwrap_or_else(|| proposed_recipe_edits(&app));
    if recipe_edits.ids.is_none() || request.regenerate_identifiers {
        recipe_edits.ids = Some(generated_ids(&app.id));
    }
    fill_empty_recipe_text(&mut recipe_edits, &app);
    app.provisioning.launch_once_recommended = recipe_edits.launch_enabled;

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
    }
}

fn proposed_app(facts: &ApkInspectionFacts) -> AppDefinitionV1 {
    let id_source = facts
        .application_label
        .as_deref()
        .or(facts.package_name.as_deref())
        .unwrap_or_default();
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
            type_name: "user_provided_apk".to_string(),
            resolver: "none".to_string(),
            options: OrderedValueMap::new(),
        },
        tracking_source: AppTrackingSource {
            type_name: "local_apk".to_string(),
            fields: OrderedValueMap::new(),
        },
        artifacts: AppArtifactSupport {
            apk: RequiredArtifactSupport { required: false },
            shared_storage_config: ConfigArtifactSupport { supported: false },
            app_data_config: ConfigArtifactSupport { supported: false },
            byo_apk: RequiredArtifactSupport { required: true },
        },
        provisioning: AppProvisioning::default(),
        inputs: Vec::new(),
        metadata: OrderedValueMap::new(),
    }
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
        Value::String(app.package.primary.clone()),
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
    proposed: &AppDefinitionV1,
    current: &AppDefinitionV1,
) -> Vec<FieldEvidence> {
    vec![
        field_evidence(
            "package.primary",
            state(facts.package_name.is_some(), EvidenceState::Verified),
            "apk_manifest",
            current.package.primary != proposed.package.primary,
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
            "install_source",
            EvidenceState::Suggested,
            "local_apk_strategy",
            current.install_source != proposed.install_source,
        ),
        field_evidence(
            "tracking_source",
            EvidenceState::Suggested,
            "local_apk_strategy",
            current.tracking_source != proposed.tracking_source,
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
    use crate::generation::apk::{ApkPermissionApplicabilityFacts, ApkPermissionDeclarationFacts};

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
                target_sdk_state: None,
            }),
        }
    }

    #[test]
    fn valid_category_generates_existing_model_recipe_and_no_native_path() {
        let mut app = proposed_app(&facts());
        app.category = "utility".to_string();
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
        app.category = "utility".to_string();
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
    }

    #[test]
    fn local_permission_automation_is_rejected_without_emitting_a_step() {
        let mut app = proposed_app(&facts());
        app.category = "utility".to_string();
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
        assert_eq!(
            draft.app.metadata["apk_inspection"]["selected_runtime_permissions"],
            json!([])
        );
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
            }],
            app_ops: vec![AppOpPermissionSelection {
                permission_name: "android.permission.MANAGE_EXTERNAL_STORAGE".to_string(),
                operation_name: String::new(),
                mode: "deny".to_string(),
                requires_root: true,
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
    fn generated_metadata_replaces_reserved_input_and_preserves_unrelated_order() {
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
        app.category = "utility".to_string();
        let draft = generate_app_recipe_draft(AppRecipeDraftRequest {
            facts: inspection,
            app: Some(app),
            recipe: None,
            mappings: Some(AppMappingEdits {
                install_source_options: "{}".to_string(),
                tracking_source_fields: "{}".to_string(),
                metadata: r#"{"first":1,"apk_inspection":"frontend","last":2}"#.to_string(),
                inputs: Vec::new(),
                config_targets: Vec::new(),
            }),
            permission_automation: None,
            regenerate_identifiers: false,
        });
        assert!(!draft.blocking);
        assert_eq!(
            draft
                .app
                .metadata
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["first", "last", "apk_inspection"]
        );
        let metadata = &draft.app.metadata["apk_inspection"];
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
        let yaml = draft.app_canonical_yaml.unwrap();
        let round_trip = crate::authored_models::parse_app_definition_yaml(&yaml).unwrap();
        assert_eq!(round_trip.metadata, draft.app.metadata);
    }

    #[test]
    fn malformed_inspection_metadata_blocks_yaml_and_is_not_persisted() {
        let mut inspection = facts();
        inspection.calculated_sha256 = "publisher-value".to_string();
        inspection.checksum_status = "verified".to_string();
        let mut app = proposed_app(&inspection);
        app.category = "utility".to_string();
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
                target_sdk_state: Some("future_state".to_string()),
            }),
        }];
        let mut app = proposed_app(&inspection);
        app.category = "utility".to_string();
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
}

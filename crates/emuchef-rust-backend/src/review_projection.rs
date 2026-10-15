//! User-facing review projection for one exact generated execution plan.
//!
//! This module is the sole owner of review meaning. It uses typed planner data
//! and authored presentation metadata, never identifiers or free-form error
//! text, to describe the work React may render before execution.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::planner::{
    ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan, ExecutionResolvedInputSnapshot,
    ExecutionStep, PermissionActionApplicability, PermissionFailurePolicy, PermissionPlanAction,
    PermissionSetKind,
};
use crate::runtime_configuration::RuntimeConfigurationDiagnostic;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewProjection {
    pub setup: ReviewSetup,
    pub target: ReviewTarget,
    pub features: Vec<ReviewFeature>,
    pub inputs: Vec<ReviewInput>,
    pub notices: Vec<ReviewNotice>,
    pub work: ReviewWork,
    pub can_execute: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewSetup {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewTarget {
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_api_level: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewFeature {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub automatically_added: bool,
    pub sections: Vec<ReviewSection>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewSection {
    pub kind: &'static str,
    pub label: &'static str,
    pub actions: Vec<ReviewAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewAction {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub requirement: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_location: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewInput {
    pub label: String,
    pub summary: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewNotice {
    pub severity: &'static str,
    pub title: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewWork {
    pub action_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_wait_seconds: Option<u64>,
}

const SECTIONS: [(&str, &str); 7] = [
    ("preparation", "Prerequisites and preparation"),
    ("downloads", "Downloads"),
    ("installs", "Installs"),
    ("copies", "Copies"),
    ("permissions", "Permissions"),
    ("launches", "Launches"),
    ("device_changes", "Device changes"),
];

pub(crate) fn project_review(
    plan: &ExecutionPlan,
    diagnostics: &[RuntimeConfigurationDiagnostic],
) -> ReviewProjection {
    let recipes = plan
        .recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let mut notices = project_warnings(diagnostics);
    let mut unsupported_action = false;
    let mut missing_feature_metadata = plan.recipes.len() != plan.source.expanded_recipe_refs.len();
    let mut missing_required_snapshot = plan.device_plan.is_none();
    let features = plan
        .source
        .expanded_recipe_refs
        .iter()
        .filter_map(|recipe_id| {
            let Some(snapshot) = recipes.get(recipe_id.as_str()).copied() else {
                missing_feature_metadata = true;
                return None;
            };
            let feature_steps = plan
                .steps
                .iter()
                .filter(|step| step.recipe_ref == *recipe_id)
                .collect::<Vec<_>>();
            let mut sections = Vec::new();
            for (kind, label) in SECTIONS {
                let actions = feature_steps
                    .iter()
                    .filter_map(|step| {
                        let action_kind = action_section(step)?;
                        (action_kind == kind).then(|| project_action(plan, step, action_kind))
                    })
                    .collect::<Vec<_>>();
                if !actions.is_empty() {
                    sections.push(ReviewSection {
                        kind,
                        label,
                        actions,
                    });
                }
            }
            if feature_steps
                .iter()
                .any(|step| action_section(step).is_none())
            {
                unsupported_action = true;
            }
            if feature_steps.iter().any(|step| {
                step.type_name == "grant_permissions"
                    && (step.permission_plan.is_some() || step.app_id.is_some())
                    && crate::planner::validated_plan_permission_plan(plan, step).is_err()
            }) {
                unsupported_action = true;
            }
            Some(ReviewFeature {
                name: non_blank(&snapshot.name)
                    .unwrap_or("Selected feature")
                    .to_string(),
                description: snapshot
                    .description
                    .as_deref()
                    .and_then(non_blank)
                    .map(str::to_string),
                automatically_added: !plan.source.selected_recipe_refs.contains(recipe_id),
                sections,
            })
        })
        .collect::<Vec<_>>();
    let inputs = project_inputs(&plan.resolved_inputs);
    if plan
        .resolved_inputs
        .iter()
        .any(|input| input.required && input.value.as_ref().is_none_or(Value::is_null))
    {
        missing_required_snapshot = true;
    }
    if unsupported_action || missing_feature_metadata || missing_required_snapshot {
        notices.push(ReviewNotice {
            severity: "blocker",
            title: "This plan cannot be reviewed safely",
            message: "The plan contains work this version of EmuChef cannot validate for execution. Update the setup catalog or application before continuing.".to_string(),
        });
    }
    let action_count = plan.steps.len();
    let known_wait_milliseconds = plan
        .steps
        .iter()
        .filter(|step| step.type_name == "wait")
        .filter_map(wait_duration_milliseconds)
        .sum::<u64>();
    ReviewProjection {
        setup: ReviewSetup {
            name: plan
                .device_plan
                .as_ref()
                .and_then(|snapshot| snapshot.name.as_deref())
                .and_then(non_blank)
                .unwrap_or("Selected setup")
                .to_string(),
            description: plan
                .device_plan
                .as_ref()
                .and_then(|snapshot| snapshot.description.as_deref())
                .and_then(non_blank)
                .map(str::to_string),
        },
        target: ReviewTarget {
            label: "Connected Android device",
            manufacturer: safe_target_text(&plan.device_context.manufacturer),
            model: safe_target_text(&plan.device_context.model),
            android_version: Some(plan.device_context.android_version),
            android_api_level: plan.device_context.android_api_level,
        },
        features,
        inputs,
        notices,
        work: ReviewWork {
            action_count,
            known_wait_seconds: (known_wait_milliseconds > 0)
                .then_some(known_wait_milliseconds.div_ceil(1_000)),
        },
        can_execute: !unsupported_action && !missing_feature_metadata && !missing_required_snapshot,
    }
}

fn action_section(step: &ExecutionStep) -> Option<&'static str> {
    match step.type_name.as_str() {
        "wait" | "extract_artifacts" | "extract_archive" => Some("preparation"),
        "resolve_artifacts"
        | "resolve_remote_release"
        | "resolve_github_release"
        | "download_remote_file" => Some("downloads"),
        "install_apk" => Some("installs"),
        "copy_files" => Some("copies"),
        "grant_permissions" => Some("permissions"),
        "launch_app" => Some("launches"),
        "force_stop_app" => Some("device_changes"),
        _ => None,
    }
}

fn project_action(plan: &ExecutionPlan, step: &ExecutionStep, section: &str) -> ReviewAction {
    let app = step
        .app_id
        .as_deref()
        .and_then(|app_id| plan.apps.iter().find(|app| app.id == app_id));
    let app_artifact = app.and_then(|app| install_apk_artifact(plan, step, &app.id));
    let late_bound_release = app_artifact.and_then(|artifact| {
        let ExecutionArtifactSource::RemoteRelease {
            provider,
            include_prereleases,
            asset_pattern,
            invert_asset_pattern,
            ..
        } = &artifact.source
        else {
            return None;
        };
        let provider_label = match provider.as_str() {
            "github" => "GitHub",
            "gitlab" => "GitLab",
            "forgejo" => "Forgejo",
            _ => return None,
        };
        Some((
            provider_label,
            *include_prereleases,
            asset_pattern.as_deref(),
            *invert_asset_pattern,
        ))
    });
    let direct_https_apk = app_artifact.is_some_and(|artifact| {
        matches!(&artifact.source, ExecutionArtifactSource::DirectUrl { .. })
    });
    let lifecycle_target = step
        .app_id
        .as_ref()
        .filter(|_| matches!(step.type_name.as_str(), "launch_app" | "force_stop_app"))
        .map(|_| crate::planner::validated_plan_lifecycle_target(plan, step));
    let permission_plan = (step.type_name == "grant_permissions")
        .then(|| crate::planner::validated_plan_permission_plan(plan, step).ok())
        .flatten();
    let title = if let Some((app, permission_plan)) = permission_plan {
        Some(format!(
            "Grant {} permissions to {}",
            match permission_plan.permission_set {
                PermissionSetKind::Baseline => "baseline",
                PermissionSetKind::Elevated => "elevated",
            },
            app.name
        ))
    } else if step.type_name == "install_apk" {
        app.map(|app| format!("Install {}", app.name))
    } else {
        None
    }
    .or_else(|| non_blank(&step.name).map(str::to_string))
    .or_else(|| non_blank(&step.note).map(str::to_string))
    .unwrap_or_else(|| neutral_action_title(section).to_string());
    let description = if let Some((app, permission_plan)) = permission_plan {
        Some(permission_plan_description(app, step, permission_plan))
    } else if step.type_name == "grant_permissions"
        && (step.permission_plan.is_some() || step.app_id.is_some())
    {
        Some("The reviewed permission plan or its App Definition identity is invalid.".to_string())
    } else if let Some(target) = lifecycle_target {
        let target = match target {
            Ok((package, Some(activity))) => format!("Target: {package}/{activity}."),
            Ok((package, None)) => format!("Target package: {package}."),
            Err(_) => "The reviewed app lifecycle target is invalid.".to_string(),
        };
        Some(match non_blank(&step.note).filter(|note| *note != title) {
            Some(note) => format!("{note} {target}"),
            None => target,
        })
    } else if let (
        Some(app),
        Some((provider, include_prereleases, asset_pattern, invert_asset_pattern)),
    ) = (app, late_bound_release)
    {
        let release_scope = if include_prereleases {
            "stable or prerelease"
        } else {
            "stable"
        };
        let pattern = asset_pattern
            .map(|pattern| {
                if invert_asset_pattern {
                    format!(" after excluding filename pattern {pattern}")
                } else {
                    format!(" after matching filename pattern {pattern}")
                }
            })
            .unwrap_or_default();
        let provider_note = if provider == "GitLab" {
            " GitLab prerelease classification uses a best-effort release name and tag heuristic."
        } else {
            ""
        };
        Some(format!(
            "The latest eligible {release_scope} {provider} APK for {} is resolved during execution{pattern}.{provider_note}",
            app.name
        ))
    } else if direct_https_apk {
        Some("The configured public direct HTTPS APK URL is used during execution. It does not pin a release and does not guarantee immutable bytes. The APK SHA-256 is calculated and checked against the authored expected checksum when one is supplied; a calculated checksum alone is not publisher authentication.".to_string())
    } else {
        non_blank(&step.note)
            .filter(|note| *note != title)
            .map(str::to_string)
    };
    ReviewAction {
        title,
        description,
        requirement: if step.skip_if.is_empty() {
            "required"
        } else {
            "conditional"
        },
        device_location: device_destination(step),
    }
}

fn permission_plan_description(
    app: &crate::planner::ExecutionAppSnapshot,
    step: &ExecutionStep,
    permission_plan: &crate::planner::PermissionPlan,
) -> String {
    let set_name = match permission_plan.permission_set {
        PermissionSetKind::Baseline => "baseline",
        PermissionSetKind::Elevated => "elevated",
    };
    let mut lines = vec![format!(
        "Permission set: {set_name}. Target package: {}.",
        app.package_id
    )];
    lines.push(format!(
        "Effective capabilities: {}.",
        step.constraints.capabilities.join(", ")
    ));
    if permission_plan.permission_set == PermissionSetKind::Elevated {
        lines.push("This set requires reviewed root authorization.".to_string());
    }
    lines.push("Planned actions:".to_string());
    for action in &permission_plan.actions {
        let action_name = match action {
            PermissionPlanAction::RuntimePermission { permission, .. } => {
                format!("grant runtime permission {permission}")
            }
            PermissionPlanAction::AppOp { op, .. } => format!("allow app-op {op}"),
        };
        let applicability = match action.applicability() {
            PermissionActionApplicability::Applicable => "applicable".to_string(),
            PermissionActionApplicability::NotApplicable { reason } => {
                format!("not applicable: {}", reason.description())
            }
        };
        lines.push(format!("- {action_name} ({applicability})"));
    }
    if !permission_plan.actions.is_empty()
        && permission_plan.actions.iter().all(|action| {
            matches!(
                action.applicability(),
                PermissionActionApplicability::NotApplicable { .. }
            )
        })
    {
        lines.push(
            "Step disposition: skipped because no planned permission actions apply.".to_string(),
        );
    }
    lines.push(format!(
        "Ordinary action failures {} the step.",
        match permission_plan.on_failure {
            PermissionFailurePolicy::Warn => "produce warnings without failing",
            PermissionFailurePolicy::Fail => "fail",
        }
    ));
    lines.join("\n")
}

fn install_apk_artifact<'a>(
    plan: &'a ExecutionPlan,
    step: &ExecutionStep,
    app_id: &str,
) -> Option<&'a crate::planner::ExecutionArtifact> {
    let ExecutionParamValue::Ref { ref_value } = step.params.get("app")? else {
        return None;
    };
    let artifact_id = ref_value
        .strip_prefix("artifacts.")?
        .strip_suffix(".local_path")?;
    plan.artifacts.iter().find(|artifact| {
        artifact.id == artifact_id
            && artifact
                .app_provenance
                .as_ref()
                .is_some_and(|provenance| provenance.app_id == app_id && provenance.kind == "apk")
    })
}

fn neutral_action_title(section: &str) -> &'static str {
    match section {
        "downloads" => "Prepare required download",
        "installs" => "Install an application",
        "copies" => "Copy required files",
        "permissions" => "Apply required permissions",
        "launches" => "Launch an application",
        "device_changes" => "Prepare the device",
        _ => "Prepare required files",
    }
}

fn device_destination(step: &ExecutionStep) -> Option<String> {
    if step.type_name != "copy_files" {
        return None;
    }
    let ExecutionParamValue::Literal { value } = step.params.get("dest")? else {
        return None;
    };
    value
        .as_str()
        .filter(|value| value.starts_with('/') && !value.contains(".."))
        .map(str::to_string)
}

fn wait_duration_milliseconds(step: &ExecutionStep) -> Option<u64> {
    let ExecutionParamValue::Literal { value } = step.params.get("duration_ms")? else {
        return None;
    };
    value.as_u64()
}

fn project_inputs(inputs: &[ExecutionResolvedInputSnapshot]) -> Vec<ReviewInput> {
    inputs
        .iter()
        .filter_map(|input| {
            let value = input.value.as_ref().filter(|value| !value.is_null())?;
            Some(ReviewInput {
                label: non_blank(&input.label)
                    .unwrap_or("Configured option")
                    .to_string(),
                summary: input_summary(input, value),
                required: input.required,
            })
        })
        .collect()
}

fn input_summary(input: &ExecutionResolvedInputSnapshot, value: &Value) -> String {
    if input.sensitive {
        return "Provided".to_string();
    }
    if input.type_name == "enum" {
        return input
            .selected_enum_label
            .clone()
            .unwrap_or_else(|| "Selected".to_string());
    }
    match input.type_name.as_str() {
        "file" | "directory" | "path" | "path_list" => portable_path_summary(value),
        "device_path" => value
            .as_str()
            .filter(|path| path.starts_with('/'))
            .unwrap_or("Configured on the device")
            .to_string(),
        "boolean" => value
            .as_bool()
            .map(|enabled| if enabled { "Yes" } else { "No" })
            .unwrap_or("Configured")
            .to_string(),
        "integer" => value
            .as_i64()
            .map(|number| number.to_string())
            .unwrap_or_else(|| "Configured".to_string()),
        "string" => "Configured".to_string(),
        _ => "Configured".to_string(),
    }
}

fn portable_path_summary(value: &Value) -> String {
    if let Some(path) = value.as_str() {
        return Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Selected item")
            .to_string();
    }
    let Some(values) = value.as_array() else {
        return "Configured".to_string();
    };
    match values.len() {
        0 => "No items selected".to_string(),
        1 => values[0]
            .as_str()
            .map(|path| portable_path_summary(&Value::String(path.to_string())))
            .unwrap_or_else(|| "1 selected item".to_string()),
        count => format!("{count} selected items"),
    }
}

fn project_warnings(diagnostics: &[RuntimeConfigurationDiagnostic]) -> Vec<ReviewNotice> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "warning")
        .map(|diagnostic| ReviewNotice {
            severity: "warning",
            title: "Review warning",
            message: if diagnostic.code == "binding_path_reused" {
                diagnostic.message.clone()
            } else {
                "This setup includes a warning. Review the selected options before continuing."
                    .to_string()
            },
        })
        .collect()
}

fn safe_target_text(value: &str) -> Option<String> {
    non_blank(value).map(str::to_string)
}

fn non_blank(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OrderedMap;
    use crate::planner::{ExecutionDevicePlanSnapshot, ExecutionRecipeSnapshot};

    #[test]
    fn path_summaries_never_include_parent_directories() {
        assert_eq!(
            portable_path_summary(&Value::String("/Users/private/game.rom".into())),
            "game.rom"
        );
        assert_eq!(
            portable_path_summary(&serde_json::json!(["/one/a", "/two/b"])),
            "2 selected items"
        );
    }

    #[test]
    fn sensitive_and_enum_inputs_use_snapshot_safe_presentation() {
        let mut input = ExecutionResolvedInputSnapshot {
            key: "recipe.review/credential".into(),
            recipe_id: "recipe.review".into(),
            input_id: "credential".into(),
            type_name: "string".into(),
            value: Some(Value::String("secret".into())),
            source: None,
            label: "Credential".into(),
            description: None,
            required: true,
            sensitive: true,
            selected_enum_label: None,
        };
        assert_eq!(
            input_summary(&input, input.value.as_ref().unwrap()),
            "Provided"
        );
        input.type_name = "enum".into();
        input.sensitive = false;
        input.value = Some(Value::String("raw".into()));
        input.selected_enum_label = Some("Friendly choice".into());
        assert_eq!(
            input_summary(&input, input.value.as_ref().unwrap()),
            "Friendly choice"
        );
    }

    fn immutable_review_plan() -> ExecutionPlan {
        ExecutionPlan {
            id: "plan.review-snapshot".into(),
            source: crate::planner::ExecutionPlanSource {
                device_profile_ref: "profile.review".into(),
                device_plan_ref: "plan.review".into(),
                selected_recipe_refs: vec!["recipe.review".into()],
                expanded_recipe_refs: vec!["recipe.review".into()],
                catalog: None,
            },
            device_plan: Some(ExecutionDevicePlanSnapshot {
                id: "plan.review".into(),
                name: Some("Frozen setup name".into()),
                description: Some("Frozen setup description".into()),
            }),
            recipes: vec![ExecutionRecipeSnapshot {
                id: "recipe.review".into(),
                name: "Frozen feature name".into(),
                description: Some("Frozen feature description".into()),
            }],
            apps: Vec::new(),
            resolved_inputs: vec![ExecutionResolvedInputSnapshot {
                key: "recipe.review/mode".into(),
                recipe_id: "recipe.review".into(),
                input_id: "mode".into(),
                type_name: "enum".into(),
                value: Some(Value::String("raw".into())),
                source: None,
                label: "Frozen option label".into(),
                description: Some("Frozen input description".into()),
                required: true,
                sensitive: false,
                selected_enum_label: Some("Frozen selected value".into()),
            }],
            target_device: None,
            device_context: crate::planner::DeviceContext {
                manufacturer: "Example".into(),
                model: "Handheld".into(),
                android_version: 14,
                android_api_level: Some(35),
                device_tags: Vec::new(),
            },
            runtime_capabilities: crate::planner::RuntimeCapabilities {
                adb_available: false,
                apk_install: false,
                shared_storage_write: false,
                app_launch: false,
                shell_command: false,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: Vec::new(),
            schema_version: 1,
            kind: "execution_plan",
        }
    }

    #[test]
    fn review_projection_uses_only_immutable_plan_presentation() {
        let plan = immutable_review_plan();

        let review = project_review(&plan, &[]);

        assert_eq!(review.setup.name, "Frozen setup name");
        assert_eq!(
            review.setup.description.as_deref(),
            Some("Frozen setup description")
        );
        assert_eq!(review.features[0].name, "Frozen feature name");
        assert_eq!(
            review.features[0].description.as_deref(),
            Some("Frozen feature description")
        );
        assert_eq!(review.inputs[0].label, "Frozen option label");
        assert_eq!(review.inputs[0].summary, "Frozen selected value");
        assert!(review.inputs[0].required);
        assert!(review.can_execute);
        assert_eq!(project_review(&plan.clone(), &[]), review);
    }

    #[test]
    fn plan_only_review_uses_frozen_dependency_recipe_and_enum_presentation() {
        let mut plan = immutable_review_plan();
        plan.source
            .expanded_recipe_refs
            .push("recipe.dependency".into());
        plan.recipes.push(ExecutionRecipeSnapshot {
            id: "recipe.dependency".into(),
            name: "Frozen dependency name".into(),
            description: None,
        });
        plan.resolved_inputs.push(ExecutionResolvedInputSnapshot {
            key: "recipe.dependency/mode".into(),
            recipe_id: "recipe.dependency".into(),
            input_id: "mode".into(),
            type_name: "enum".into(),
            value: Some(Value::String("raw-dependency-value".into())),
            source: None,
            label: "Frozen dependency option".into(),
            description: None,
            required: true,
            sensitive: false,
            selected_enum_label: Some("Frozen dependency choice".into()),
        });

        let review = project_review(&plan, &[]);

        assert!(review.can_execute);
        assert_eq!(review.features[0].name, "Frozen feature name");
        assert!(!review.features[0].automatically_added);
        assert_eq!(review.features[1].name, "Frozen dependency name");
        assert_eq!(review.features[1].description, None);
        assert!(review.features[1].automatically_added);
        assert_eq!(review.inputs[0].summary, "Frozen selected value");
        assert_eq!(review.inputs[1].summary, "Frozen dependency choice");
    }

    #[test]
    fn missing_required_review_snapshots_block_safely_and_optional_metadata_is_allowed() {
        let mut complete = immutable_review_plan();
        complete.device_plan.as_mut().unwrap().name = None;
        complete.device_plan.as_mut().unwrap().description = None;
        complete.recipes[0].description = None;
        complete.resolved_inputs[0].description = None;

        let optional_metadata_review = project_review(&complete, &[]);
        assert!(optional_metadata_review.can_execute);
        assert_eq!(optional_metadata_review.setup.name, "Selected setup");
        assert_eq!(optional_metadata_review.setup.description, None);

        let assert_blocked = |plan: &ExecutionPlan| {
            let review = project_review(plan, &[]);
            assert!(!review.can_execute);
            assert!(review.notices.iter().any(|notice| {
                notice.severity == "blocker"
                    && notice.title == "This plan cannot be reviewed safely"
            }));
        };

        let mut missing_device_plan = complete.clone();
        missing_device_plan.device_plan = None;
        assert_blocked(&missing_device_plan);

        let mut missing_recipe = complete.clone();
        missing_recipe.recipes.clear();
        assert_blocked(&missing_recipe);

        let mut missing_required_value = complete;
        missing_required_value.resolved_inputs[0].value = None;
        assert_blocked(&missing_required_value);
    }

    #[test]
    fn app_install_review_uses_only_the_plan_snapshot_and_late_bound_release_policy() {
        use crate::planner::{
            DeviceContext, ExecutionAppSnapshot, ExecutionArtifact, ExecutionArtifactAppProvenance,
            ExecutionArtifactSource, ExecutionParamValue, ExecutionPlanSource,
            ExecutionStepConstraints, RuntimeCapabilities,
        };

        let mut step = ExecutionStep {
            id: "app.armsx1.install/install".to_string(),
            recipe_ref: "app.armsx1.install".to_string(),
            app_id: Some("armsx1".to_string()),
            type_name: "install_apk".to_string(),
            name: "Install or update ARMSX1".to_string(),
            note: "Installing ARMSX1".to_string(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            permission_plan: None,
            params: OrderedMap::new(),
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        let mut plan = ExecutionPlan {
            device_plan: None,
            resolved_inputs: Vec::new(),
            id: "plan.review".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "profile.example".to_string(),
                device_plan_ref: "plan.example".to_string(),
                selected_recipe_refs: vec![step.recipe_ref.clone()],
                expanded_recipe_refs: vec![step.recipe_ref.clone()],
                catalog: None,
            },
            recipes: Vec::new(),
            apps: vec![ExecutionAppSnapshot {
                id: "armsx1".to_string(),
                name: "ARMSX1".to_string(),
                description: Some("Android emulator".to_string()),
                category: None,
                package_id: "com.nanodata.armsx".to_string(),
            }],
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 14,
                android_api_level: Some(35),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: false,
                app_launch: false,
                shell_command: false,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: vec![ExecutionArtifact {
                id: "app.armsx1.install/installer".to_string(),
                type_name: "remote_file".to_string(),
                source: ExecutionArtifactSource::RemoteRelease {
                    provider: "github".to_string(),
                    service_origin: "https://github.com".to_string(),
                    repository: "ARMSX2/ARMSX1".to_string(),
                    include_prereleases: false,
                    asset_pattern: Some("^ARMSX1-release-[0-9]{8}-arm64-v8a\\.apk$".to_string()),
                    invert_asset_pattern: false,
                },
                cache: "default".to_string(),
                app_provenance: Some(ExecutionArtifactAppProvenance {
                    app_id: "armsx1".to_string(),
                    artifact_id: "apk".to_string(),
                    kind: "apk".to_string(),
                }),
            }],
            steps: vec![step.clone()],
            schema_version: 1,
            kind: "execution_plan",
        };
        step.params.insert(
            "app".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "artifacts.app.armsx1.install/installer.local_path".to_string(),
            },
        );

        let action = project_action(&plan, &step, "installs");

        assert_eq!(action.title, "Install ARMSX1");
        let description = action.description.expect("release policy is described");
        assert!(description.contains("GitHub"));
        assert!(description.contains("stable GitHub APK"));

        let mut second_artifact = plan.artifacts[0].clone();
        second_artifact.id = "app.armsx1.install/alternate-apk".to_string();
        second_artifact.source = ExecutionArtifactSource::RemoteRelease {
            provider: "forgejo".to_string(),
            service_origin: "https://forgejo.example".to_string(),
            repository: "maintainer/armsx1".to_string(),
            include_prereleases: true,
            asset_pattern: Some("^selected-armsx1\\.apk$".to_string()),
            invert_asset_pattern: false,
        };
        second_artifact.app_provenance = Some(ExecutionArtifactAppProvenance {
            app_id: "armsx1".to_string(),
            artifact_id: "alternate-apk".to_string(),
            kind: "apk".to_string(),
        });
        plan.artifacts.push(second_artifact);
        step.params.insert(
            "app".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "artifacts.app.armsx1.install/alternate-apk.local_path".to_string(),
            },
        );

        let selected_action = project_action(&plan, &step, "installs");
        let selected_description = selected_action
            .description
            .expect("review should describe the artifact consumed by the install step");
        assert!(selected_description.contains("Forgejo"));
        assert!(selected_description.contains("selected-armsx1\\.apk"));
        assert!(selected_description.contains("stable or prerelease"));

        step.params.insert(
            "app".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "artifacts.app.armsx1.install/installer.local_path".to_string(),
            },
        );
        plan.artifacts[0].source = ExecutionArtifactSource::DirectUrl {
            url: "https://downloads.example.com/app.apk?token=private".to_string(),
            sha256: None,
        };
        let direct_action = project_action(&plan, &step, "installs");
        let description = direct_action
            .description
            .expect("direct URL review should describe its source policy");
        assert!(description.contains("direct HTTPS"));
        assert!(description.contains("does not pin a release"));
        assert!(description.contains("does not guarantee immutable bytes"));
        assert!(description.contains("not publisher authentication"));
        assert!(!description.contains("private"));
    }

    #[test]
    fn permission_review_uses_planned_actions_app_identity_and_applicability() {
        use crate::authored_models::AppOpMode;
        use crate::planner::{
            DeviceContext, ExecutionAppSnapshot, ExecutionPlanSource, ExecutionStepConstraints,
            PermissionActionApplicability, PermissionNotApplicableReason, PermissionPlan,
            PermissionPlanAction, RuntimeCapabilities,
        };

        let step = ExecutionStep {
            id: "app.armsx1.install/grant_elevated".to_string(),
            recipe_ref: "app.armsx1.install".to_string(),
            app_id: Some("armsx1".to_string()),
            type_name: "grant_permissions".to_string(),
            name: "Authored permission note".to_string(),
            note: "Authored note does not define permission actions".to_string(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: vec!["shell_command".to_string(), "root_shell".to_string()],
                conflicts_with: Vec::new(),
            },
            permission_plan: Some(PermissionPlan {
                permission_set: PermissionSetKind::Elevated,
                on_failure: PermissionFailurePolicy::Warn,
                actions: vec![
                    PermissionPlanAction::RuntimePermission {
                        package_name: "com.nanodata.armsx".to_string(),
                        permission: "android.permission.POST_NOTIFICATIONS".to_string(),
                        android_api_min: None,
                        android_api_max: None,
                        applicability: PermissionActionApplicability::Applicable,
                    },
                    PermissionPlanAction::AppOp {
                        package_name: "com.nanodata.armsx".to_string(),
                        op: "MANAGE_EXTERNAL_STORAGE".to_string(),
                        mode: AppOpMode::Allow,
                        android_api_min: None,
                        android_api_max: Some(34),
                        applicability: PermissionActionApplicability::NotApplicable {
                            reason: PermissionNotApplicableReason::AndroidApiAboveMaximum {
                                actual: 35,
                                maximum: 34,
                            },
                        },
                    },
                ],
            }),
            params: OrderedMap::new(),
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        let mut plan = ExecutionPlan {
            device_plan: None,
            resolved_inputs: Vec::new(),
            id: "plan.permission-review".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "profile.example".to_string(),
                device_plan_ref: "plan.example".to_string(),
                selected_recipe_refs: vec![step.recipe_ref.clone()],
                expanded_recipe_refs: vec![step.recipe_ref.clone()],
                catalog: None,
            },
            recipes: Vec::new(),
            apps: vec![ExecutionAppSnapshot {
                id: "armsx1".to_string(),
                name: "ARMSX1".to_string(),
                description: None,
                category: None,
                package_id: "com.nanodata.armsx".to_string(),
            }],
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 15,
                android_api_level: Some(35),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: false,
                app_launch: false,
                shell_command: true,
                package_remove_for_user: false,
                root_shell: true,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: vec![step.clone()],
            schema_version: 1,
            kind: "execution_plan",
        };

        let action = project_action(&plan, &step, "permissions");

        assert_eq!(action.title, "Grant elevated permissions to ARMSX1");
        let description = action.description.expect("permission plan is reviewed");
        assert!(description.contains("Target package: com.nanodata.armsx."));
        assert!(description.contains("Effective capabilities: shell_command, root_shell."));
        assert!(description.contains("requires reviewed root authorization"));
        assert!(
            description.contains("grant runtime permission android.permission.POST_NOTIFICATIONS")
        );
        assert!(description.contains("allow app-op MANAGE_EXTERNAL_STORAGE"));
        assert!(description.contains("Android API level 35 is above the action maximum of 34."));
        assert!(description.contains("Ordinary action failures produce warnings without failing"));
        assert!(!description.contains("Authored note does not define"));

        plan.apps[0].package_id = "com.nanodata.changed".to_string();
        let invalid_action = project_action(&plan, &step, "permissions");
        assert!(invalid_action
            .description
            .as_deref()
            .is_some_and(|description| description
                .contains("permission plan or its App Definition identity is invalid")));
        plan.apps[0].package_id = "com.nanodata.armsx".to_string();

        let baseline_step = ExecutionStep {
            id: "app.armsx1.install/grant_baseline".to_string(),
            recipe_ref: step.recipe_ref.clone(),
            app_id: step.app_id.clone(),
            type_name: step.type_name.clone(),
            name: step.name.clone(),
            note: step.note.clone(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: vec!["shell_command".to_string()],
                conflicts_with: Vec::new(),
            },
            permission_plan: Some(PermissionPlan {
                permission_set: PermissionSetKind::Baseline,
                on_failure: PermissionFailurePolicy::Fail,
                actions: vec![PermissionPlanAction::RuntimePermission {
                    package_name: "com.nanodata.armsx".to_string(),
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: None,
                    android_api_max: None,
                    applicability: PermissionActionApplicability::Applicable,
                }],
            }),
            params: OrderedMap::new(),
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        plan.steps = vec![baseline_step.clone()];
        let baseline_action = project_action(&plan, &baseline_step, "permissions");
        let baseline_description = baseline_action
            .description
            .expect("baseline permission plan is reviewed");
        assert!(baseline_description.contains("Effective capabilities: shell_command."));
        assert!(baseline_description.contains("Target package: com.nanodata.armsx."));
        assert!(baseline_description.contains("Ordinary action failures fail"));

        let mut skipped_step = step.clone();
        skipped_step.id = "app.armsx1.install/grant_all_inapplicable".to_string();
        if let Some(PermissionPlanAction::RuntimePermission { applicability, .. }) = skipped_step
            .permission_plan
            .as_mut()
            .and_then(|permission_plan| permission_plan.actions.first_mut())
        {
            *applicability = PermissionActionApplicability::NotApplicable {
                reason: PermissionNotApplicableReason::AndroidApiBelowMinimum {
                    actual: 28,
                    minimum: 30,
                },
            };
        }
        plan.steps = vec![skipped_step.clone()];
        let skipped_action = project_action(&plan, &skipped_step, "permissions");
        let skipped_description = skipped_action
            .description
            .expect("inapplicable permission plan is reviewed");
        assert!(skipped_description
            .contains("Step disposition: skipped because no planned permission actions apply."));
        assert!(
            skipped_description.contains("Android API level 28 is below the action minimum of 30.")
        );
    }
}

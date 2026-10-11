//! User-facing review projection for one exact generated execution plan.
//!
//! This module is the sole owner of review meaning. It uses typed planner data
//! and authored presentation metadata, never identifiers or free-form error
//! text, to describe the work React may render before execution.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::model::{InputDeclaration, Recipe};
use crate::planner::{
    ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan, ExecutionStep,
    ResolvedInputBinding,
};
use crate::runtime_configuration::{PreparedConfiguration, RuntimeConfigurationDiagnostic};

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
    prepared: &PreparedConfiguration,
    plan: &ExecutionPlan,
    resolved_inputs: &[ResolvedInputBinding],
    diagnostics: &[RuntimeConfigurationDiagnostic],
) -> ReviewProjection {
    let recipes = prepared
        .recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let mut notices = project_warnings(diagnostics);
    let mut unsupported_action = false;
    let mut missing_feature_metadata = false;
    let features = plan
        .source
        .expanded_recipe_refs
        .iter()
        .filter_map(|recipe_id| {
            let Some(authored) = recipes.get(recipe_id.as_str()).copied() else {
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
            Some(ReviewFeature {
                name: non_blank(&authored.name)
                    .unwrap_or("Selected feature")
                    .to_string(),
                description: authored
                    .description
                    .as_deref()
                    .and_then(non_blank)
                    .map(str::to_string),
                automatically_added: !plan.source.selected_recipe_refs.contains(recipe_id),
                sections,
            })
        })
        .collect::<Vec<_>>();
    if unsupported_action || missing_feature_metadata {
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
            name: prepared
                .device_plan_parts
                .as_ref()
                .and_then(|parts| parts.device_plan_name.as_deref())
                .and_then(non_blank)
                .unwrap_or("Selected setup")
                .to_string(),
            description: prepared
                .device_plan_parts
                .as_ref()
                .and_then(|parts| parts.device_plan_description.as_deref())
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
        inputs: project_inputs(resolved_inputs, &recipes),
        notices,
        work: ReviewWork {
            action_count,
            known_wait_seconds: (known_wait_milliseconds > 0)
                .then_some(known_wait_milliseconds.div_ceil(1_000)),
        },
        can_execute: !unsupported_action && !missing_feature_metadata,
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
    let title = if step.type_name == "install_apk" {
        app.map(|app| format!("Install {}", app.name))
    } else {
        None
    }
    .or_else(|| non_blank(&step.name).map(str::to_string))
    .or_else(|| non_blank(&step.note).map(str::to_string))
    .unwrap_or_else(|| neutral_action_title(section).to_string());
    let description = if let Some(target) = lifecycle_target {
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

fn project_inputs(
    inputs: &[ResolvedInputBinding],
    recipes: &HashMap<&str, &Recipe>,
) -> Vec<ReviewInput> {
    inputs
        .iter()
        .filter_map(|input| {
            let declaration = recipes
                .get(input.recipe_id.as_str())?
                .inputs
                .get(&input.input_id)?;
            let value = input.value.as_ref().filter(|value| !value.is_null())?;
            Some(ReviewInput {
                label: non_blank(&declaration.label)
                    .unwrap_or("Configured option")
                    .to_string(),
                summary: input_summary(declaration, value),
                required: declaration.required,
            })
        })
        .collect()
}

fn input_summary(declaration: &InputDeclaration, value: &Value) -> String {
    if declaration.sensitive {
        return "Provided".to_string();
    }
    if declaration.type_name == "enum" {
        if let Some(option) = declaration
            .options
            .iter()
            .find(|option| option.value == *value)
        {
            return option.label.clone();
        }
        return "Selected".to_string();
    }
    match declaration.type_name.as_str() {
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
    use crate::model::{InputOption, InputValidation, OrderedMap};

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
    fn sensitive_and_enum_inputs_use_authored_safe_presentation() {
        let mut declaration = InputDeclaration {
            type_name: "string".into(),
            role: "generic".into(),
            label: "Credential".into(),
            description: None,
            required: true,
            multiple: false,
            validation: InputValidation {
                must_exist: false,
                allowed_extensions: Vec::new(),
                path_kind: None,
                allowed_prefixes: Vec::new(),
            },
            default: Value::Null,
            options: Vec::new(),
            sensitive: true,
            advanced: false,
            metadata: OrderedMap::new(),
        };
        assert_eq!(
            input_summary(&declaration, &Value::String("secret".into())),
            "Provided"
        );
        declaration.type_name = "enum".into();
        declaration.sensitive = false;
        declaration.options = vec![InputOption {
            value: Value::String("raw".into()),
            label: "Friendly choice".into(),
        }];
        assert_eq!(
            input_summary(&declaration, &Value::String("raw".into())),
            "Friendly choice"
        );
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
            params: OrderedMap::new(),
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        let mut plan = ExecutionPlan {
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
}

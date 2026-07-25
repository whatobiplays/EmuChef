//! Side-effect-free preparation and discovery for runtime recipe configuration.
//!
//! This module resolves request overrides, saved configuration, device-plan
//! selection, recipe dependencies, and binding precedence. It performs no ADB,
//! download, extraction, copy, device-write, or persistence operations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::catalog_source::{CatalogIdentity, CatalogSnapshot};
use crate::dto;
use crate::model::{OrderedMap, Recipe};
use crate::planner::{
    expand_recipe_dependencies, resolve_runtime_bindings, BindingDiagnostic, BindingResolution,
    BindingSource, ExecutionPlan, PlannerInput, PlannerLoadError, PlannerMessage,
    TargetDeviceBinding,
};
use crate::planner_device_plan::{self, PlannerInputParts};
use crate::review_projection::{project_review, ReviewProjection};
use crate::user_configuration::{self, UserConfiguration, UserConfigurationLoadError};

/// A persisted configuration reference or an already parsed inline document.
#[derive(Clone, Debug)]
pub(crate) enum UserConfigurationSource {
    Reference(String),
    Inline(Box<UserConfiguration>),
}

#[derive(Clone, Debug)]
pub(crate) struct ConfigurationContextRequest {
    pub catalog: CatalogSnapshot,
    pub configuration_root: Option<PathBuf>,
    pub user_configuration: Option<UserConfigurationSource>,
    pub device_plan: Option<String>,
    pub selected_recipes: Option<Vec<String>>,
    pub explicit_bindings: OrderedMap<Value>,
    pub device_context: Option<DeviceContextOverride>,
    pub target_device: Option<TargetDeviceBinding>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceContextOverride {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub android_version: Option<i64>,
    pub android_api_level: Option<i64>,
    #[serde(default)]
    pub device_tags: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigurationDiagnostic {
    pub severity: &'static str,
    pub code: String,
    pub message: String,
    pub key: Option<String>,
    pub provenance: Option<BindingSource>,
    pub details: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedConfiguration {
    pub catalog: CatalogSnapshot,
    pub effective_device_plan: String,
    pub recipes: Vec<Recipe>,
    pub selected_recipe_refs: Vec<String>,
    pub expanded_recipe_refs: Vec<String>,
    pub explicit_bindings: OrderedMap<Value>,
    pub user_configuration_bindings: OrderedMap<Value>,
    pub device_plan_input_bindings: OrderedMap<Value>,
    pub device_plan_parts: Option<PlannerInputParts>,
    pub device_context: Option<DeviceContextOverride>,
    pub target_device: Option<TargetDeviceBinding>,
    pub binding_resolution: BindingResolution,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogIdentity>,
    /// Exact target binding retained only in the trusted sidecar/Tauri response.
    /// React projections must omit this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_device: Option<ConfigurationTargetDevice>,
    pub device_plan: String,
    pub selected_recipes: Vec<String>,
    pub expanded_recipes: Vec<String>,
    pub recipe_options: Vec<Value>,
    pub inputs: Vec<Value>,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

/// Trusted sidecar DTO for the target retained with a configuration description.
///
/// This type keeps the sidecar's camelCase protocol independent from the
/// snake_case execution-plan serialization of [`TargetDeviceBinding`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationTargetDevice {
    pub serial: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub android_api_level: Option<i64>,
}

impl From<TargetDeviceBinding> for ConfigurationTargetDevice {
    fn from(target: TargetDeviceBinding) -> Self {
        Self {
            serial: target.serial,
            manufacturer: target.manufacturer,
            model: target.model,
            android_api_level: target.android_api_level,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanConfigurationResult {
    pub plan: Option<ExecutionPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) review: Option<ReviewProjection>,
    pub resolved_inputs: Vec<crate::planner::ResolvedInputBinding>,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

#[derive(Debug)]
pub(crate) enum ConfigurationContextError {
    MissingDevicePlan,
    UserConfiguration(UserConfigurationLoadError),
    Catalog(PlannerLoadError),
}

impl std::fmt::Display for ConfigurationContextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDevicePlan => formatter.write_str(
                "Configuration context requires devicePlan or a saved user configuration.",
            ),
            Self::UserConfiguration(error) => error.fmt(formatter),
            Self::Catalog(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ConfigurationContextError {}

pub(crate) fn prepare_configuration(
    request: ConfigurationContextRequest,
) -> Result<PreparedConfiguration, ConfigurationContextError> {
    let recipes = crate::planner::load_top_level_recipes(request.catalog.root())
        .map_err(ConfigurationContextError::Catalog)?;
    let recipe_map = recipes
        .iter()
        .cloned()
        .map(|recipe| (recipe.id.clone(), recipe))
        .collect::<HashMap<_, _>>();

    let saved = match request.user_configuration {
        Some(UserConfigurationSource::Reference(reference)) => Some(
            user_configuration::load_user_configuration_reference(
                request.configuration_root.as_deref(),
                &reference,
            )
            .map(|(_, configuration)| configuration)
            .map_err(ConfigurationContextError::UserConfiguration)?,
        ),
        Some(UserConfigurationSource::Inline(configuration)) => Some(*configuration),
        None => None,
    };
    let effective_device_plan = request
        .device_plan
        .clone()
        .or_else(|| {
            saved
                .as_ref()
                .map(|configuration| configuration.device_plan.clone())
        })
        .ok_or(ConfigurationContextError::MissingDevicePlan)?;

    let mut diagnostics = Vec::new();
    let device_plan_parts = match planner_device_plan::load_planner_input_parts(
        request.catalog.root(),
        &effective_device_plan,
        &recipes,
    ) {
        Ok(parts) => Some(parts),
        Err(error) => {
            diagnostics.push(planner_load_diagnostic(&error, &effective_device_plan));
            None
        }
    };
    let selected_recipe_refs = request
        .selected_recipes
        .clone()
        .or_else(|| {
            saved
                .as_ref()
                .map(|configuration| configuration.selected_recipes.clone())
        })
        .or_else(|| {
            device_plan_parts
                .as_ref()
                .map(|parts| parts.selected_recipe_refs.clone())
        })
        .unwrap_or_default();
    let (expanded_recipe_refs, dependency_errors) =
        expand_recipe_dependencies(&recipe_map, &selected_recipe_refs);
    diagnostics.extend(
        dependency_errors
            .into_iter()
            .map(planner_message_diagnostic),
    );

    let user_configuration_bindings = saved
        .map(|configuration| configuration.bindings)
        .unwrap_or_default();
    let device_plan_input_bindings = device_plan_parts
        .as_ref()
        .map(|parts| parts.device_plan_input_bindings.clone())
        .unwrap_or_default();
    let binding_resolution = resolve_runtime_bindings(
        &recipe_map,
        &expanded_recipe_refs,
        &request.explicit_bindings,
        &user_configuration_bindings,
        &device_plan_input_bindings,
    );
    diagnostics.extend(
        binding_resolution
            .diagnostics
            .iter()
            .map(binding_diagnostic),
    );
    diagnostics.extend(input_reuse_diagnostics(
        &recipe_map,
        &binding_resolution.resolved_inputs,
    ));

    Ok(PreparedConfiguration {
        catalog: request.catalog,
        effective_device_plan,
        recipes,
        selected_recipe_refs,
        expanded_recipe_refs,
        explicit_bindings: request.explicit_bindings,
        user_configuration_bindings,
        device_plan_input_bindings,
        device_plan_parts,
        device_context: request.device_context,
        target_device: request.target_device,
        binding_resolution,
        diagnostics,
    })
}

impl PreparedConfiguration {
    pub(crate) fn planner_input(&self, plan_id: String) -> Option<PlannerInput> {
        let parts = self.device_plan_parts.as_ref()?;
        let mut device_context = parts.device_context.clone();
        if let Some(overrides) = &self.device_context {
            if let Some(manufacturer) = &overrides.manufacturer {
                device_context.manufacturer = manufacturer.clone();
            }
            if let Some(model) = &overrides.model {
                device_context.model = model.clone();
            }
            if let Some(android_version) = overrides.android_version {
                device_context.android_version = android_version;
            }
            if let Some(android_api_level) = overrides.android_api_level {
                device_context.android_api_level = Some(android_api_level);
            }
            if let Some(device_tags) = &overrides.device_tags {
                device_context.device_tags = device_tags.clone();
            }
        }
        Some(PlannerInput {
            recipes: self.recipes.clone(),
            selected_recipe_refs: self.selected_recipe_refs.clone(),
            explicit_input_bindings: self.explicit_bindings.clone(),
            user_configuration_bindings: self.user_configuration_bindings.clone(),
            device_plan_input_bindings: self.device_plan_input_bindings.clone(),
            plan_id,
            device_plan_ref: parts.device_plan_ref.clone(),
            device_profile_ref: parts.device_profile_ref.clone(),
            device_context,
            runtime_capabilities: parts.runtime_capabilities.clone(),
            catalog_identity: self.catalog.identity().cloned(),
            target_device: self.target_device.clone(),
        })
    }
}

pub(crate) fn plan_configuration(
    request: ConfigurationContextRequest,
) -> Result<PlanConfigurationResult, ConfigurationContextError> {
    let prepared = prepare_configuration(request)?;
    let resolved_inputs = prepared.binding_resolution.resolved_inputs.clone();
    if prepared
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == "error")
    {
        return Ok(PlanConfigurationResult {
            plan: None,
            plan_digest: None,
            review: None,
            resolved_inputs,
            diagnostics: prepared.diagnostics,
        });
    }
    let plan_id = format!("plan.{}.001", prepared.effective_device_plan);
    let Some(input) = prepared.planner_input(plan_id) else {
        return Ok(PlanConfigurationResult {
            plan: None,
            plan_digest: None,
            review: None,
            resolved_inputs,
            diagnostics: prepared.diagnostics,
        });
    };
    let result = crate::planner::plan_execution(input);
    let mut diagnostics = prepared.diagnostics.clone();
    diagnostics.extend(
        result
            .warnings
            .into_iter()
            .map(|message| planner_result_diagnostic("warning", message))
            .collect::<Vec<_>>(),
    );
    diagnostics.extend(
        result
            .errors
            .into_iter()
            .map(|message| planner_result_diagnostic("error", message)),
    );
    let plan_digest = result
        .execution_plan
        .as_ref()
        .map(crate::plan_digest::execution_plan_digest)
        .transpose()
        .map_err(|error| {
            ConfigurationContextError::Catalog(PlannerLoadError::new(
                "plan_digest_failed",
                error.to_string(),
            ))
        })?;
    let review = result
        .execution_plan
        .as_ref()
        .map(|plan| project_review(&prepared, plan, &resolved_inputs, &diagnostics));
    Ok(PlanConfigurationResult {
        plan: result.execution_plan,
        plan_digest,
        review,
        resolved_inputs,
        diagnostics,
    })
}

/// Warn when two active, user-supplied file inputs resolve to one filesystem
/// object. Canonical identity is used only for comparison and is never copied
/// into the diagnostic, serialized response, or logs.
pub(crate) fn input_reuse_diagnostics(
    recipes: &HashMap<String, Recipe>,
    resolved_inputs: &[crate::planner::ResolvedInputBinding],
) -> Vec<RuntimeConfigurationDiagnostic> {
    let mut identities = HashMap::<PathBuf, (String, String)>::new();
    let mut diagnostics = Vec::new();
    for input in resolved_inputs {
        if input.type_name != "file"
            || !matches!(
                input.source,
                Some(BindingSource::Explicit | BindingSource::UserConfiguration)
            )
        {
            continue;
        }
        let Some(recipe) = recipes.get(&input.recipe_id) else {
            continue;
        };
        let Some(declaration) = recipe.inputs.get(&input.input_id) else {
            continue;
        };
        let values = input
            .value
            .as_ref()
            .and_then(|value| declaration.binding_items(value))
            .unwrap_or_default();
        for value in values {
            let Some(path) = value.as_str() else {
                continue;
            };
            let Ok(identity) = std::fs::canonicalize(Path::new(path)) else {
                continue;
            };
            if let Some((related_key, related_label)) = identities.get(&identity) {
                if related_key != &input.key {
                    diagnostics.push(RuntimeConfigurationDiagnostic {
                        severity: "warning",
                        code: "binding_path_reused".to_string(),
                        message: format!(
                            "{} uses the same file as {}. Confirm that this reuse is intentional.",
                            declaration.label, related_label
                        ),
                        key: Some(input.key.clone()),
                        provenance: input.source,
                        details: json!({ "related_key": related_key }),
                    });
                }
            } else {
                identities.insert(identity, (input.key.clone(), declaration.label.clone()));
            }
        }
    }
    diagnostics
}

pub(crate) fn describe_configuration(
    request: ConfigurationContextRequest,
) -> Result<ConfigurationDescription, ConfigurationContextError> {
    let prepared = prepare_configuration(request)?;
    let recipes = prepared
        .recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let recipe_options = prepared
        .recipes
        .iter()
        .map(|recipe| {
            let required_capabilities = recipe
                .steps
                .iter()
                .flat_map(|step| step.constraints.capabilities.iter())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut content_requirements = std::collections::BTreeSet::new();
            if !recipe.artifacts.is_empty() {
                content_requirements.insert("network_download");
            }
            for input in recipe.inputs.values() {
                match input.role.as_str() {
                    "apk" => {
                        content_requirements.insert("apk_file");
                    }
                    "bios" => {
                        content_requirements.insert("bios_files");
                    }
                    "rom" | "rom_library" | "content" => {
                        content_requirements.insert("rom_content");
                    }
                    _ => {}
                }
            }
            let unavailable_capabilities = prepared
                .device_plan_parts
                .as_ref()
                .map(|parts| {
                    required_capabilities
                        .iter()
                        .filter(|capability| {
                            !runtime_capability_available(&parts.runtime_capabilities, capability)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let explicitly_selected = prepared.selected_recipe_refs.contains(&recipe.id);
            let recommended = prepared
                .device_plan_parts
                .as_ref()
                .is_some_and(|parts| parts.selected_recipe_refs.contains(&recipe.id));
            let dependency_required =
                prepared.expanded_recipe_refs.contains(&recipe.id) && !explicitly_selected;
            json!({
                "id": recipe.id,
                "name": recipe.name,
                "description": recipe.description,
                "selected": explicitly_selected,
                "recommended": recommended,
                "dependencyRequired": dependency_required,
                "available": unavailable_capabilities.is_empty(),
                "recipeDependencies": recipe.recipe_dependencies,
                "contentRequirements": content_requirements,
                "requiredCapabilities": required_capabilities,
                "unavailableCapabilities": unavailable_capabilities,
            })
        })
        .collect();
    let inputs = prepared
        .binding_resolution
        .resolved_inputs
        .iter()
        .filter_map(|resolved| {
            let declaration = recipes
                .get(resolved.recipe_id.as_str())?
                .inputs
                .get(&resolved.input_id)?;
            let mut dto = dto::input_to_dto(&resolved.recipe_id, &resolved.input_id, declaration);
            let object = dto.as_object_mut()?;
            object.insert(
                "value".to_string(),
                resolved.value.clone().unwrap_or(Value::Null),
            );
            object.insert(
                "valueSource".to_string(),
                resolved.source.map_or(Value::Null, |source| {
                    serde_json::to_value(source).expect("binding source should serialize")
                }),
            );
            object.insert(
                "diagnostics".to_string(),
                json!(prepared
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.key.as_deref() == Some(&resolved.key))
                    .collect::<Vec<_>>()),
            );
            Some(dto)
        })
        .collect();

    Ok(ConfigurationDescription {
        catalog: prepared.catalog.identity().cloned(),
        target_device: prepared.target_device.map(ConfigurationTargetDevice::from),
        device_plan: prepared.effective_device_plan,
        selected_recipes: prepared.selected_recipe_refs,
        expanded_recipes: prepared.expanded_recipe_refs,
        recipe_options,
        inputs,
        diagnostics: prepared.diagnostics,
    })
}

fn runtime_capability_available(
    capabilities: &crate::planner::RuntimeCapabilities,
    capability: &str,
) -> bool {
    match capability {
        "adb_available" => capabilities.adb_available,
        "apk_install" => capabilities.apk_install,
        "shared_storage_write" => capabilities.shared_storage_write,
        "app_launch" => capabilities.app_launch,
        "shell_command" => capabilities.shell_command,
        "package_remove_for_user" => capabilities.package_remove_for_user,
        "root_shell" => capabilities.root_shell,
        "app_data_write" => capabilities.app_data_write,
        _ => false,
    }
}

fn binding_diagnostic(diagnostic: &BindingDiagnostic) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        key: Some(diagnostic.key.clone()),
        provenance: diagnostic.provenance,
        details: diagnostic.details.clone(),
    }
}

fn planner_message_diagnostic(message: PlannerMessage) -> RuntimeConfigurationDiagnostic {
    planner_result_diagnostic("error", message)
}

fn planner_result_diagnostic(
    severity: &'static str,
    message: PlannerMessage,
) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity,
        code: message.code,
        message: message.message,
        key: None,
        provenance: None,
        details: message.details,
    }
}

fn planner_load_diagnostic(
    error: &PlannerLoadError,
    device_plan: &str,
) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: error.code().to_string(),
        message: error.to_string(),
        key: None,
        provenance: Some(BindingSource::DevicePlan),
        details: json!({ "devicePlan": device_plan }),
    }
}

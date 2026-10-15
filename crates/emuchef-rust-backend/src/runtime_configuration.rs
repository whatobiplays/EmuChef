//! Side-effect-free preparation and discovery for runtime recipe configuration.
//!
//! This module resolves request overrides, saved configuration, device-plan
//! selection, recipe dependencies, and binding precedence. It performs no ADB,
//! download, extraction, copy, device-write, or persistence operations.

use std::collections::{BTreeSet, HashMap};
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
    pub runtime_capability_availability: Option<RuntimeCapabilityAvailability>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeCapabilityAvailability {
    pub root_shell: bool,
    pub app_data_write: bool,
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
    pub runtime_capabilities: crate::planner::RuntimeCapabilities,
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
    let runtime_capabilities = device_plan_parts.as_ref().map(|parts| {
        apply_runtime_capability_availability(
            parts.runtime_capabilities.clone(),
            request.runtime_capability_availability.as_ref(),
        )
    });
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
        runtime_capabilities: runtime_capabilities.unwrap_or(crate::planner::RuntimeCapabilities {
            adb_available: false,
            apk_install: false,
            shared_storage_write: false,
            app_launch: false,
            shell_command: false,
            package_remove_for_user: false,
            root_shell: false,
            app_data_write: false,
        }),
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
            runtime_capabilities: self.runtime_capabilities.clone(),
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
    let result = plan_prepared_input(&prepared.catalog, input);
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

/// Plan against the App Definitions owned by the same catalog snapshot as the
/// prepared Recipe input. Planning copies the referenced app facts into the
/// immutable execution plan; review and execution do not reopen the catalog.
pub(crate) fn plan_prepared_input(
    catalog: &CatalogSnapshot,
    input: crate::planner::PlannerInput,
) -> crate::planner::PlanningResult {
    let app_definitions = crate::catalog::load_app_definition_catalog(catalog.root())
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .collect::<Vec<_>>();
    crate::planner::plan_execution_with_app_definitions(input, &app_definitions)
}

#[derive(Debug)]
struct RecipeAvailability {
    available: bool,
    unavailable_capabilities: Vec<String>,
}

/// Classify one Recipe by probing the planner with the same selected device,
/// bindings, App Definitions, and dependency rules used by configuration
/// planning. Counterfactual capability sets are used only to identify which
/// missing capabilities can make a valid retained graph runnable.
fn recipe_availability(
    prepared: &PreparedConfiguration,
    recipe: &Recipe,
    app_definitions: &[crate::authored_models::AppDefinitionV1],
) -> (RecipeAvailability, Vec<RuntimeConfigurationDiagnostic>) {
    let severity = if prepared.expanded_recipe_refs.contains(&recipe.id) {
        "error"
    } else {
        "warning"
    };
    let mut candidate = prepared.clone();
    candidate.selected_recipe_refs = vec![recipe.id.clone()];
    let plan_id = format!("plan.{}.001", candidate.effective_device_plan);
    let Some(input) = candidate.planner_input(plan_id) else {
        return (
            RecipeAvailability {
                available: false,
                unavailable_capabilities: Vec::new(),
            },
            Vec::new(),
        );
    };

    let actual =
        crate::planner::plan_execution_for_recipe_availability(input.clone(), app_definitions);
    let mut diagnostics = recipe_planner_diagnostics(&actual, &recipe.id, severity, false);
    if planning_result_has_steps(&actual) {
        return (
            RecipeAvailability {
                available: true,
                unavailable_capabilities: Vec::new(),
            },
            diagnostics,
        );
    }
    if !planning_result_is_empty_only(&actual) {
        if diagnostics.is_empty() {
            diagnostics.push(recipe_availability_diagnostic(
                &recipe.id,
                severity,
                "recipe_availability_unresolved",
                "The planner could not determine whether this Recipe has a runnable graph.",
            ));
        }
        return (
            RecipeAvailability {
                available: false,
                unavailable_capabilities: Vec::new(),
            },
            diagnostics,
        );
    }

    // An upper-bound planner run distinguishes a genuinely empty or
    // optional-input-only graph from executable work pruned by capabilities.
    let mut upper_bound_input = input.clone();
    upper_bound_input.runtime_capabilities = all_runtime_capabilities();
    let upper_bound = crate::planner::plan_execution_for_recipe_availability_upper_bound(
        upper_bound_input,
        app_definitions,
    );
    if !planning_result_has_steps(&upper_bound) {
        if planning_result_is_empty_only(&upper_bound) {
            diagnostics.extend(recipe_planner_diagnostics(
                &upper_bound,
                &recipe.id,
                severity,
                false,
            ));
            return (
                RecipeAvailability {
                    available: true,
                    unavailable_capabilities: Vec::new(),
                },
                diagnostics,
            );
        }
        diagnostics.extend(recipe_planner_diagnostics(
            &upper_bound,
            &recipe.id,
            severity,
            false,
        ));
        if diagnostics.is_empty() {
            diagnostics.push(recipe_availability_diagnostic(
                &recipe.id,
                severity,
                "recipe_availability_unresolved",
                "The planner could not validate this Recipe graph even with all device capabilities enabled.",
            ));
        }
        return (
            RecipeAvailability {
                available: false,
                unavailable_capabilities: Vec::new(),
            },
            diagnostics,
        );
    }

    let candidate_capabilities = upper_bound
        .execution_plan
        .iter()
        .flat_map(|plan| plan.steps.iter())
        .flat_map(|step| step.constraints.capabilities.iter())
        .filter(|capability| {
            !crate::planner::runtime_capability_available(&input.runtime_capabilities, capability)
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut missing_candidates = Vec::new();
    let mut unsupported_capabilities = Vec::new();
    for capability in candidate_capabilities {
        let mut probe_capabilities = input.runtime_capabilities.clone();
        if crate::planner::set_runtime_capability(&mut probe_capabilities, &capability, true) {
            missing_candidates.push(capability);
        } else {
            unsupported_capabilities.push(capability);
        }
    }
    if missing_candidates.is_empty() {
        diagnostics.extend(recipe_planner_diagnostics(
            &upper_bound,
            &recipe.id,
            severity,
            false,
        ));
        if unsupported_capabilities.is_empty() {
            diagnostics.push(recipe_availability_diagnostic(
                &recipe.id,
                severity,
                "recipe_availability_unresolved",
                "The planner pruned all work without identifying a missing runtime capability.",
            ));
        } else {
            diagnostics.retain(|diagnostic| diagnostic.code != "unsupported_runtime_capability");
            diagnostics.push(unsupported_capability_diagnostic(
                &recipe.id,
                severity,
                &unsupported_capabilities,
            ));
        }
        return (
            RecipeAvailability {
                available: false,
                unavailable_capabilities: Vec::new(),
            },
            diagnostics,
        );
    }

    let mut minimal_solutions = Vec::<Vec<String>>::new();
    for subset in capability_subsets(&missing_candidates) {
        if minimal_solutions.iter().any(|solution| {
            solution
                .iter()
                .all(|capability| subset.contains(capability))
        }) {
            continue;
        }
        let mut probe_input = input.clone();
        let mut unknown_capability = None;
        for capability in &subset {
            if !crate::planner::set_runtime_capability(
                &mut probe_input.runtime_capabilities,
                capability,
                true,
            ) {
                unknown_capability = Some(capability.clone());
                break;
            }
        }
        if let Some(capability) = unknown_capability {
            diagnostics.push(recipe_availability_diagnostic(
                &recipe.id,
                severity,
                "recipe_availability_unknown_capability",
                &format!("The planner selected an unsupported runtime capability: {capability}."),
            ));
            return (
                RecipeAvailability {
                    available: false,
                    unavailable_capabilities: Vec::new(),
                },
                diagnostics,
            );
        }

        let probe =
            crate::planner::plan_execution_for_recipe_availability(probe_input, app_definitions);
        if planning_result_has_steps(&probe) {
            minimal_solutions.push(subset);
        } else if !planning_result_is_empty_only(&probe) {
            diagnostics.extend(recipe_planner_diagnostics(
                &probe, &recipe.id, severity, false,
            ));
            if diagnostics.is_empty() {
                diagnostics.push(recipe_availability_diagnostic(
                    &recipe.id,
                    severity,
                    "recipe_availability_unresolved",
                    "The planner could not validate a capability candidate graph.",
                ));
            }
            return (
                RecipeAvailability {
                    available: false,
                    unavailable_capabilities: Vec::new(),
                },
                diagnostics,
            );
        }
    }

    let blockers = minimal_solutions
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if blockers.is_empty() {
        if unsupported_capabilities.is_empty() {
            diagnostics.push(recipe_availability_diagnostic(
                &recipe.id,
                severity,
                "recipe_availability_unresolved",
                "The planner found executable work but no missing capability set that restores a runnable graph.",
            ));
        } else {
            diagnostics.retain(|diagnostic| diagnostic.code != "unsupported_runtime_capability");
            diagnostics.push(unsupported_capability_diagnostic(
                &recipe.id,
                severity,
                &unsupported_capabilities,
            ));
        }
    }
    (
        RecipeAvailability {
            available: false,
            unavailable_capabilities: blockers,
        },
        diagnostics,
    )
}

fn planning_result_has_steps(result: &crate::planner::PlanningResult) -> bool {
    result
        .execution_plan
        .as_ref()
        .is_some_and(|plan| !plan.steps.is_empty())
}

fn planning_result_is_empty_only(result: &crate::planner::PlanningResult) -> bool {
    result.execution_plan.is_none()
        && !result.errors.is_empty()
        && result
            .errors
            .iter()
            .all(|error| error.code == "empty_execution_plan")
}

fn recipe_planner_diagnostics(
    result: &crate::planner::PlanningResult,
    recipe_id: &str,
    error_severity: &'static str,
    include_empty_plan: bool,
) -> Vec<RuntimeConfigurationDiagnostic> {
    let mut diagnostics = result
        .warnings
        .iter()
        .cloned()
        .map(|message| {
            let mut diagnostic = planner_result_diagnostic("warning", message);
            diagnostic.key = Some(recipe_id.to_string());
            diagnostic
        })
        .collect::<Vec<_>>();
    diagnostics.extend(
        result
            .errors
            .iter()
            .filter(|error| include_empty_plan || error.code != "empty_execution_plan")
            .cloned()
            .map(|message| {
                let mut diagnostic = planner_result_diagnostic(error_severity, message);
                diagnostic.key = Some(recipe_id.to_string());
                diagnostic
            }),
    );
    diagnostics
}

fn recipe_availability_diagnostic(
    recipe_id: &str,
    severity: &'static str,
    code: &str,
    message: &str,
) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity,
        code: code.to_string(),
        message: message.to_string(),
        key: Some(recipe_id.to_string()),
        provenance: None,
        details: json!({}),
    }
}

fn unsupported_capability_diagnostic(
    recipe_id: &str,
    severity: &'static str,
    capabilities: &[String],
) -> RuntimeConfigurationDiagnostic {
    recipe_availability_diagnostic(
        recipe_id,
        severity,
        "recipe_availability_unknown_capability",
        &format!(
            "The Recipe requires unsupported runtime capability name(s): {}. Update the Recipe to use capabilities supported by the planner.",
            capabilities.join(", ")
        ),
    )
}

fn capability_subsets(candidates: &[String]) -> Vec<Vec<String>> {
    fn append_combinations(
        candidates: &[String],
        start: usize,
        remaining: usize,
        current: &mut Vec<String>,
        combinations: &mut Vec<Vec<String>>,
    ) {
        if remaining == 0 {
            combinations.push(current.clone());
            return;
        }
        let last_start = candidates.len() - remaining;
        for index in start..=last_start {
            current.push(candidates[index].clone());
            append_combinations(candidates, index + 1, remaining - 1, current, combinations);
            current.pop();
        }
    }

    let mut combinations = Vec::new();
    for size in 1..=candidates.len() {
        append_combinations(candidates, 0, size, &mut Vec::new(), &mut combinations);
    }
    combinations
}

fn all_runtime_capabilities() -> crate::planner::RuntimeCapabilities {
    crate::planner::RuntimeCapabilities {
        adb_available: true,
        apk_install: true,
        shared_storage_write: true,
        app_launch: true,
        shell_command: true,
        package_remove_for_user: true,
        root_shell: true,
        app_data_write: true,
    }
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
    let app_definitions = crate::catalog::load_app_definition_catalog(prepared.catalog.root())
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .collect::<Vec<_>>();
    let mut target_diagnostics = Vec::new();
    let recipe_options = prepared
        .recipes
        .iter()
        .map(|recipe| {
            let (availability, diagnostics) =
                recipe_availability(&prepared, recipe, &app_definitions);
            target_diagnostics.extend(diagnostics);
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
            let explicitly_selected = prepared.selected_recipe_refs.contains(&recipe.id);
            let recommended = prepared
                .device_plan_parts
                .as_ref()
                .is_some_and(|parts| parts.selected_recipe_refs.contains(&recipe.id));
            let dependency_required =
                prepared.expanded_recipe_refs.contains(&recipe.id) && !explicitly_selected;
            let option = json!({
                "id": recipe.id,
                "name": recipe.name,
                "description": recipe.description,
                "selected": explicitly_selected,
                "recommended": recommended,
                "dependencyRequired": dependency_required,
                "available": availability.available,
                "recipeDependencies": recipe.recipe_dependencies,
                "contentRequirements": content_requirements,
                "unavailableCapabilities": availability.unavailable_capabilities,
            });
            option
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

    let mut diagnostics = prepared.diagnostics;
    diagnostics.extend(target_diagnostics);
    Ok(ConfigurationDescription {
        catalog: prepared.catalog.identity().cloned(),
        target_device: prepared.target_device.map(ConfigurationTargetDevice::from),
        device_plan: prepared.effective_device_plan,
        selected_recipes: prepared.selected_recipe_refs,
        expanded_recipes: prepared.expanded_recipe_refs,
        recipe_options,
        inputs,
        diagnostics,
    })
}

fn apply_runtime_capability_availability(
    mut capabilities: crate::planner::RuntimeCapabilities,
    availability: Option<&RuntimeCapabilityAvailability>,
) -> crate::planner::RuntimeCapabilities {
    if let Some(availability) = availability {
        capabilities.root_shell &= availability.root_shell;
        capabilities.app_data_write &= availability.app_data_write;
    }
    capabilities
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

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> crate::planner::RuntimeCapabilities {
        crate::planner::RuntimeCapabilities {
            adb_available: true,
            apk_install: true,
            shared_storage_write: true,
            app_launch: true,
            shell_command: true,
            package_remove_for_user: true,
            root_shell: true,
            app_data_write: true,
        }
    }

    #[test]
    fn root_capability_mask_is_fail_closed_and_does_not_change_unrelated_capabilities() {
        let masked = apply_runtime_capability_availability(
            capabilities(),
            Some(&RuntimeCapabilityAvailability {
                root_shell: false,
                app_data_write: false,
            }),
        );
        assert!(!masked.root_shell);
        assert!(!masked.app_data_write);
        assert!(masked.adb_available);
        assert!(masked.shared_storage_write);

        let granted = apply_runtime_capability_availability(capabilities(), None);
        assert!(granted.root_shell);
        assert!(granted.app_data_write);
    }

    #[test]
    fn prepared_planner_input_uses_current_root_capability_evidence() {
        let authored_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let catalog = CatalogSnapshot::legacy_local(&authored_root).unwrap();
        let prepared = prepare_configuration(ConfigurationContextRequest {
            catalog,
            configuration_root: None,
            user_configuration: None,
            device_plan: Some("ayaneo.generic.base".to_string()),
            selected_recipes: None,
            explicit_bindings: OrderedMap::new(),
            device_context: None,
            target_device: None,
            runtime_capability_availability: Some(RuntimeCapabilityAvailability {
                root_shell: false,
                app_data_write: false,
            }),
        })
        .unwrap();
        let input = prepared.planner_input("plan.test".to_string()).unwrap();
        assert!(!input.runtime_capabilities.root_shell);
        assert!(!input.runtime_capabilities.app_data_write);
        assert!(input.runtime_capabilities.shared_storage_write);
    }
}

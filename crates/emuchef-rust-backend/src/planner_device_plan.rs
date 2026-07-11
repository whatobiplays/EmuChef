//! Private checked-in device-plan/profile ingestion for Rust planner tests.
//!
//! This module intentionally models only the current authored
//! `device_profiles/*.y*ml` and `device_plans/*.y*ml` shapes needed to build
//! crate-internal `PlannerInput` values. It does not expose protocol, CLI,
//! executor, apply, ADB, or real-device profile-matching behavior.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};

use crate::device_probe::{apply_detected_device_facts_to_context, DetectedDeviceFacts};
use crate::device_profile_match::{
    build_detected_device_profile_mismatch_warning, AndroidVersionRangeCriteria,
    DeviceProfileMatchCriteria,
};
use crate::model::{OrderedMap, Recipe};
use crate::planner::{
    plan_execution, DeviceContext, PlannerInput, PlannerLoadError, PlanningResult, PlanningStatus,
    RuntimeCapabilities,
};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeviceProfileInventoryEntry {
    pub path: PathBuf,
    pub id: String,
    pub runtime_capabilities: RuntimeCapabilities,
    pub device_tags: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevicePlanInventoryEntry {
    pub path: PathBuf,
    pub id: String,
    pub device_profile_ref: String,
    pub selected_recipe_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannerInputParts {
    pub device_plan_ref: String,
    pub device_profile_ref: String,
    pub recipe_refs: Vec<String>,
    pub selected_recipe_refs: Vec<String>,
    pub override_input_bindings: OrderedMap<JsonValue>,
    pub device_context: DeviceContext,
    pub runtime_capabilities: RuntimeCapabilities,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevicePlanProfileMatchCriteria {
    pub device_plan_ref: String,
    pub device_profile_ref: String,
    pub profile_match: DeviceProfileMatchCriteria,
}

#[derive(Clone, Debug)]
struct DeviceProfileRecord {
    #[cfg(test)]
    path: PathBuf,
    profile: DeviceProfileYaml,
}

#[derive(Clone, Debug)]
struct DevicePlanRecord {
    #[cfg(test)]
    path: PathBuf,
    plan: DevicePlanYaml,
}

#[derive(Clone, Debug, Deserialize)]
struct DeviceProfileYaml {
    id: String,
    name: String,
    #[serde(rename = "match", default)]
    match_criteria: DeviceMatchYaml,
    capability_defaults: RuntimeCapabilitiesYaml,
    #[serde(default)]
    device_tags: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct DeviceMatchYaml {
    #[serde(default)]
    manufacturer_contains: Vec<String>,
    #[serde(default)]
    brand_contains: Vec<String>,
    #[serde(default)]
    model_patterns: Vec<String>,
    #[serde(default)]
    android_version: Option<AndroidVersionRangeYaml>,
}

#[derive(Clone, Debug, Deserialize)]
struct AndroidVersionRangeYaml {
    min: Option<i64>,
    max: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeCapabilitiesYaml {
    adb_available: bool,
    apk_install: bool,
    shared_storage_write: bool,
    app_launch: bool,
    shell_command: bool,
    package_remove_for_user: bool,
    root_shell: bool,
    app_data_write: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct DevicePlanYaml {
    id: String,
    device_profile_ref: String,
    #[serde(default)]
    recipes: Vec<DevicePlanRecipeYaml>,
    #[serde(default)]
    defaults: Mapping,
    #[serde(default)]
    overrides: Mapping,
}

#[derive(Clone, Debug, Deserialize)]
struct DevicePlanRecipeYaml {
    recipe_ref: String,
    selected_by_default: bool,
}

#[cfg(test)]
pub(crate) fn discover_device_profile_inventory(
    authored_root: impl AsRef<Path>,
) -> Result<Vec<DeviceProfileInventoryEntry>, PlannerLoadError> {
    load_device_profiles(authored_root.as_ref()).map(|records| {
        records
            .into_iter()
            .map(DeviceProfileInventoryEntry::from)
            .collect()
    })
}

#[cfg(test)]
pub(crate) fn discover_device_plan_inventory(
    authored_root: impl AsRef<Path>,
) -> Result<Vec<DevicePlanInventoryEntry>, PlannerLoadError> {
    load_device_plans(authored_root.as_ref()).map(|records| {
        records
            .into_iter()
            .map(DevicePlanInventoryEntry::from)
            .collect()
    })
}

/// Build planner input from authored device-plan data with detected facts applied.
///
/// This keeps the existing authored device-plan constructor as the source of
/// selected recipes, bindings, profile context, and runtime capabilities. It
/// only layers detected facts over the resulting planner context; route-specific
/// explicit context overrides remain separate and are not applied here.
pub(crate) fn planner_input_from_authored_device_plan_with_detected_facts(
    authored_root: impl AsRef<Path>,
    device_plan_ref: &str,
    plan_id: String,
    input_bindings: OrderedMap<JsonValue>,
    detected_facts: &DetectedDeviceFacts,
) -> Result<PlannerInput, PlannerLoadError> {
    let mut input = PlannerInput::from_authored_device_plan(
        authored_root,
        device_plan_ref,
        plan_id,
        input_bindings,
    )?;
    input.device_context =
        apply_detected_device_facts_to_context(input.device_context, detected_facts);
    Ok(input)
}

/// Build a planner result from authored device-plan data with detected facts.
///
/// This composes the fake/test-backed detected-context path with the pure
/// profile mismatch warning helper. It remains crate-private product behavior:
/// it does not probe devices, call route code, or change normal planner input
/// construction.
pub(crate) fn plan_from_authored_device_plan_with_detected_facts(
    authored_root: impl AsRef<Path>,
    device_plan_ref: &str,
    plan_id: String,
    input_bindings: OrderedMap<JsonValue>,
    detected_facts: &DetectedDeviceFacts,
) -> Result<PlanningResult, PlannerLoadError> {
    let authored_root = authored_root.as_ref();
    let input = planner_input_from_authored_device_plan_with_detected_facts(
        authored_root,
        device_plan_ref,
        plan_id,
        input_bindings,
        detected_facts,
    )?;
    let profile_match = load_device_plan_profile_match_criteria(authored_root, device_plan_ref)?;
    let mut result = plan_execution(input);

    if result.execution_plan.is_some() {
        if let Some(warning) = build_detected_device_profile_mismatch_warning(
            &profile_match.device_plan_ref,
            &profile_match.device_profile_ref,
            detected_facts,
            &profile_match.profile_match,
        ) {
            let already_present = result
                .warnings
                .iter()
                .any(|existing| existing.code == "device_profile_mismatch");
            if !already_present {
                if matches!(result.status, PlanningStatus::Success) {
                    result.status = PlanningStatus::Warning;
                }
                result.warnings.push(warning);
            }
        }
    }

    Ok(result)
}

pub(crate) fn load_device_plan_profile_match_criteria(
    authored_root: impl AsRef<Path>,
    device_plan_ref: &str,
) -> Result<DevicePlanProfileMatchCriteria, PlannerLoadError> {
    let authored_root = authored_root.as_ref();
    let profile_records = load_device_profiles(authored_root)?;
    let plan_records = load_device_plans(authored_root)?;
    let plan = plan_records
        .iter()
        .find(|record| record.plan.id == device_plan_ref)
        .ok_or_else(|| {
            let available = plan_records
                .iter()
                .map(|record| record.plan.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            PlannerLoadError::new(
                "device_plan_not_found",
                format!(
                    "Unknown device plan '{device_plan_ref}'. Available device plans: {available}"
                ),
            )
        })?;
    let profile = profile_records
        .iter()
        .find(|record| record.profile.id == plan.plan.device_profile_ref)
        .ok_or_else(|| {
            PlannerLoadError::new(
                "device_profile_not_found",
                format!(
                    "Device profile '{}' referenced by device plan '{}' was not found.",
                    plan.plan.device_profile_ref, plan.plan.id
                ),
            )
        })?;

    Ok(DevicePlanProfileMatchCriteria {
        device_plan_ref: plan.plan.id.clone(),
        device_profile_ref: profile.profile.id.clone(),
        profile_match: profile_match_criteria(&profile.profile),
    })
}

pub(crate) fn load_planner_input_parts(
    authored_root: &Path,
    device_plan_ref: &str,
    recipes: &[Recipe],
) -> Result<PlannerInputParts, PlannerLoadError> {
    let profile_records = load_device_profiles(authored_root)?;
    let plan_records = load_device_plans(authored_root)?;
    let plan = plan_records
        .iter()
        .find(|record| record.plan.id == device_plan_ref)
        .ok_or_else(|| {
            let available = plan_records
                .iter()
                .map(|record| record.plan.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            PlannerLoadError::new(
                "device_plan_not_found",
                format!(
                    "Unknown device plan '{device_plan_ref}'. Available device plans: {available}"
                ),
            )
        })?;
    let profile = profile_records
        .iter()
        .find(|record| record.profile.id == plan.plan.device_profile_ref)
        .ok_or_else(|| {
            PlannerLoadError::new(
                "device_profile_not_found",
                format!(
                    "Device profile '{}' referenced by device plan '{}' was not found.",
                    plan.plan.device_profile_ref, plan.plan.id
                ),
            )
        })?;

    Ok(PlannerInputParts {
        device_plan_ref: plan.plan.id.clone(),
        device_profile_ref: profile.profile.id.clone(),
        recipe_refs: plan
            .plan
            .recipes
            .iter()
            .map(|selection| selection.recipe_ref.clone())
            .collect(),
        selected_recipe_refs: selected_recipe_refs(&plan.plan),
        override_input_bindings: override_input_bindings(&plan.plan, recipes)?,
        device_context: synthetic_device_context(&profile.profile),
        runtime_capabilities: runtime_capabilities(&profile.profile.capability_defaults),
    })
}

#[cfg(test)]
impl From<DeviceProfileRecord> for DeviceProfileInventoryEntry {
    fn from(record: DeviceProfileRecord) -> Self {
        Self {
            path: record.path,
            id: record.profile.id,
            runtime_capabilities: runtime_capabilities(&record.profile.capability_defaults),
            device_tags: record.profile.device_tags,
        }
    }
}

#[cfg(test)]
impl From<DevicePlanRecord> for DevicePlanInventoryEntry {
    fn from(record: DevicePlanRecord) -> Self {
        Self {
            path: record.path,
            id: record.plan.id.clone(),
            device_profile_ref: record.plan.device_profile_ref.clone(),
            selected_recipe_refs: selected_recipe_refs(&record.plan),
        }
    }
}

fn load_device_profiles(
    authored_root: &Path,
) -> Result<Vec<DeviceProfileRecord>, PlannerLoadError> {
    let mut seen_ids = HashSet::new();
    top_level_yaml_files(&authored_root.join("device_profiles"))?
        .into_iter()
        .map(|path| {
            let profile = parse_authored_yaml::<DeviceProfileYaml>(&path, "device_profile")?;
            if !seen_ids.insert(profile.id.clone()) {
                return Err(PlannerLoadError::new(
                    "device_profile_id_conflict",
                    format!("Duplicate device_profile id '{}'.", profile.id),
                ));
            }
            Ok(DeviceProfileRecord {
                #[cfg(test)]
                path,
                profile,
            })
        })
        .collect()
}

fn load_device_plans(authored_root: &Path) -> Result<Vec<DevicePlanRecord>, PlannerLoadError> {
    let mut seen_ids = HashSet::new();
    top_level_yaml_files(&authored_root.join("device_plans"))?
        .into_iter()
        .map(|path| {
            let plan = parse_authored_yaml::<DevicePlanYaml>(&path, "device_plan")?;
            if !seen_ids.insert(plan.id.clone()) {
                return Err(PlannerLoadError::new(
                    "device_plan_id_conflict",
                    format!("Duplicate device_plan id '{}'.", plan.id),
                ));
            }
            Ok(DevicePlanRecord {
                #[cfg(test)]
                path,
                plan,
            })
        })
        .collect()
}

fn top_level_yaml_files(directory: &Path) -> Result<Vec<PathBuf>, PlannerLoadError> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(directory).map_err(|error| {
        PlannerLoadError::new(
            "io",
            format!("Failed to read directory {}: {error}", directory.display()),
        )
    })?;
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_yaml_extension(path))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn parse_authored_yaml<T>(path: &Path, expected_kind: &str) -> Result<T, PlannerLoadError>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path).map_err(|error| {
        PlannerLoadError::new("io", format!("Failed to read {}: {error}", path.display()))
    })?;
    let value = serde_yaml::from_str::<YamlValue>(&text).map_err(|error| {
        PlannerLoadError::new(
            "authored_data_invalid",
            format!(
                "File '{}' is not valid YAML: {error}.",
                path_file_name(path)
            ),
        )
    })?;
    let empty_mapping = Mapping::new();
    let mapping = match &value {
        YamlValue::Mapping(mapping) => mapping,
        YamlValue::Null => &empty_mapping,
        _ => {
            return Err(PlannerLoadError::new(
                "authored_data_invalid",
                format!(
                    "File '{}' must contain a top-level mapping.",
                    path_file_name(path)
                ),
            ));
        }
    };
    if yaml_i64(mapping, "schema_version") != Some(1) {
        return Err(PlannerLoadError::new(
            "authored_data_invalid",
            format!(
                "File '{}' has unsupported schema_version.",
                path_file_name(path)
            ),
        ));
    }
    if yaml_string(mapping, "kind").as_deref() != Some(expected_kind) {
        return Err(PlannerLoadError::new(
            "authored_data_invalid",
            format!(
                "File '{}' has kind {:?}, expected '{}'.",
                path_file_name(path),
                yaml_string(mapping, "kind"),
                expected_kind
            ),
        ));
    }

    serde_yaml::from_value(value).map_err(|error| {
        PlannerLoadError::new(
            "authored_data_invalid",
            format!(
                "File '{}' has an invalid schema shape: {error}.",
                path_file_name(path)
            ),
        )
    })
}

fn selected_recipe_refs(plan: &DevicePlanYaml) -> Vec<String> {
    plan.recipes
        .iter()
        .filter(|selection| selection.selected_by_default)
        .map(|selection| selection.recipe_ref.clone())
        .collect()
}

fn override_input_bindings(
    plan: &DevicePlanYaml,
    recipes: &[Recipe],
) -> Result<OrderedMap<JsonValue>, PlannerLoadError> {
    // Defaults are parsed so the current authored shape is classified, but
    // device-plan defaults remain inactive private planner metadata.
    let _inactive_defaults = &plan.defaults;
    let mut bindings = OrderedMap::new();
    for (raw_key, raw_value) in &plan.overrides {
        let Some((binding_ref, value)) =
            normalize_override_binding(plan, raw_key, raw_value, recipes)?
        else {
            continue;
        };
        bindings.insert(binding_ref, value);
    }
    Ok(bindings)
}

fn normalize_override_binding(
    plan: &DevicePlanYaml,
    raw_key: &YamlValue,
    raw_value: &YamlValue,
    recipes: &[Recipe],
) -> Result<Option<(String, JsonValue)>, PlannerLoadError> {
    let Some(key) = raw_key.as_str() else {
        return Err(PlannerLoadError::new(
            "device_plan_override_malformed",
            format!(
                "Device plan '{}' override key {} must be a string.",
                plan.id,
                override_key_label(raw_key)
            ),
        ));
    };
    if key == "config_variants" {
        return Ok(None);
    }

    let slash_count = key.matches('/').count();
    if slash_count == 0 {
        return Err(PlannerLoadError::new(
            "device_plan_override_unsupported",
            format!(
                "Device plan '{}' override key '{}' is unsupported. Only 'config_variants' metadata or '<recipe_ref>/<input_id>' binding keys are supported.",
                plan.id, key
            ),
        ));
    }
    if slash_count != 1 {
        return Err(PlannerLoadError::new(
            "device_plan_override_malformed",
            format!(
                "Device plan '{}' override key '{}' must contain exactly one slash.",
                plan.id, key
            ),
        ));
    }

    let (recipe_ref, input_id) = key
        .split_once('/')
        .expect("slash count was checked before split");
    if recipe_ref.is_empty() {
        return Err(PlannerLoadError::new(
            "device_plan_override_malformed",
            format!(
                "Device plan '{}' override key '{}' has an empty recipe segment.",
                plan.id, key
            ),
        ));
    }
    if input_id.is_empty() {
        return Err(PlannerLoadError::new(
            "device_plan_override_malformed",
            format!(
                "Device plan '{}' override key '{}' has an empty input segment.",
                plan.id, key
            ),
        ));
    }

    let Some(recipe) = recipes.iter().find(|recipe| recipe.id == recipe_ref) else {
        return Err(PlannerLoadError::new(
            "device_plan_override_unknown_binding",
            format!(
                "Device plan '{}' override key '{}' references unknown recipe '{}'.",
                plan.id, key, recipe_ref
            ),
        ));
    };
    if !recipe.inputs.contains_key(input_id) {
        return Err(PlannerLoadError::new(
            "device_plan_override_unknown_binding",
            format!(
                "Device plan '{}' override key '{}' references unknown input '{}'.",
                plan.id, key, input_id
            ),
        ));
    }

    let value = serde_json::to_value(raw_value).map_err(|error| {
        PlannerLoadError::new(
            "authored_data_invalid",
            format!(
                "Device plan '{}' override key '{}' value could not be converted to JSON: {error}.",
                plan.id, key
            ),
        )
    })?;
    Ok(Some((key.to_string(), value)))
}

fn override_key_label(key: &YamlValue) -> String {
    key.as_str()
        .map(|value| format!("'{value}'"))
        .unwrap_or_else(|| format!("{key:?}"))
}

fn synthetic_device_context(profile: &DeviceProfileYaml) -> DeviceContext {
    DeviceContext {
        manufacturer: profile
            .match_criteria
            .manufacturer_contains
            .first()
            .cloned()
            .unwrap_or_else(|| format!("profile:{}", profile.id)),
        model: if profile.name.is_empty() {
            format!("profile:{}", profile.id)
        } else {
            profile.name.clone()
        },
        android_version: profile
            .match_criteria
            .android_version
            .as_ref()
            .and_then(|version| version.min)
            .unwrap_or(0),
        android_api_level: None,
        device_tags: profile.device_tags.clone(),
    }
}

fn profile_match_criteria(profile: &DeviceProfileYaml) -> DeviceProfileMatchCriteria {
    DeviceProfileMatchCriteria {
        manufacturer_contains: profile.match_criteria.manufacturer_contains.clone(),
        brand_contains: profile.match_criteria.brand_contains.clone(),
        model_patterns: profile.match_criteria.model_patterns.clone(),
        android_version: profile
            .match_criteria
            .android_version
            .as_ref()
            .map(|version| AndroidVersionRangeCriteria {
                min: version.min,
                max: version.max,
            }),
    }
}

fn runtime_capabilities(capabilities: &RuntimeCapabilitiesYaml) -> RuntimeCapabilities {
    RuntimeCapabilities {
        adb_available: capabilities.adb_available,
        apk_install: capabilities.apk_install,
        shared_storage_write: capabilities.shared_storage_write,
        app_launch: capabilities.app_launch,
        shell_command: capabilities.shell_command,
        package_remove_for_user: capabilities.package_remove_for_user,
        root_shell: capabilities.root_shell,
        app_data_write: capabilities.app_data_write,
    }
}

fn yaml_i64(mapping: &Mapping, key: &str) -> Option<i64> {
    mapping
        .get(YamlValue::String(key.to_string()))
        .and_then(YamlValue::as_i64)
}

fn yaml_string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping
        .get(YamlValue::String(key.to_string()))
        .and_then(YamlValue::as_str)
        .map(ToString::to_string)
}

fn path_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

fn is_yaml_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

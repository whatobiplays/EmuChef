//! Persisted, catalog-independent runtime user-configuration documents.
//!
//! Structural loading validates only the schema needed to preserve and edit a
//! document safely. Recipe, input, device-plan, and value compatibility checks
//! are catalog-aware diagnostics and never prevent a structurally valid
//! document from being loaded or canonically emitted.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};
use sha2::{Digest, Sha256};

use crate::catalog;
use crate::model::{InputDeclaration, Recipe};
use crate::model::{ParamValue, StepCondition};
use crate::planner;
use crate::planner_device_plan;

const KNOWN_FIELDS: &[&str] = &[
    "schema_version",
    "kind",
    "id",
    "name",
    "device_plan",
    "selected_recipes",
    "bindings",
    "compatibility",
];
const VALUE_SOURCE_FIELDS: &[&str] = &["value", "local", "secret"];

const PROHIBITED_AUTHORITY_FIELDS: &[&str] = &[
    "plan",
    "generated_plan",
    "plan_digest",
    "review",
    "review_handle",
    "execution",
    "execution_handle",
    "runtime_generation",
    "session_generation",
    "device_serial",
    "device_identity",
    "diagnostics",
    "launch_authorization",
    "report_identity",
    "credentials",
    "secrets",
];

#[derive(Clone, Debug, PartialEq)]
pub struct SavedContractSnapshot {
    pub id: String,
    pub label: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedInputContractSnapshot {
    pub key: String,
    pub label: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedRecipeContractSnapshot {
    pub id: String,
    pub label: String,
    pub selected: bool,
    pub fingerprint: String,
    pub inputs: Vec<SavedInputContractSnapshot>,
}

/// Durable authored-contract baseline. It contains no generated plan, resolved
/// value, host path, device fact, runtime generation, or execution authority.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedCompatibilityBaseline {
    pub device_plan: SavedContractSnapshot,
    pub recipes: Vec<SavedRecipeContractSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibilityBaselineState {
    /// V1 can be validated now but has no historical authored-contract record.
    PendingFirstV2Save,
    Unchanged,
    MateriallyChanged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserConfiguration {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    pub device_plan: String,
    pub selected_recipes: Vec<String>,
    pub bindings: IndexMap<String, JsonValue>,
    pub compatibility: Option<SavedCompatibilityBaseline>,
    pub extensions: BTreeMap<String, JsonValue>,
    /// Unknown fields are retained in memory during inspection. An explicit V2
    /// write may sanitize these fields only after reporting them to the caller.
    pub unsupported_extensions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserConfigurationLoadErrorKind {
    Io,
    Yaml,
    Structural,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserConfigurationLoadError {
    pub kind: UserConfigurationLoadErrorKind,
    pub code: &'static str,
    pub message: String,
}

impl UserConfigurationLoadError {
    fn io(message: impl Into<String>) -> Self {
        Self {
            kind: UserConfigurationLoadErrorKind::Io,
            code: "user_configuration_io",
            message: message.into(),
        }
    }

    fn yaml(message: impl Into<String>) -> Self {
        Self {
            kind: UserConfigurationLoadErrorKind::Yaml,
            code: "user_configuration_yaml_invalid",
            message: message.into(),
        }
    }

    fn structural(message: impl Into<String>) -> Self {
        Self {
            kind: UserConfigurationLoadErrorKind::Structural,
            code: "user_configuration_structural_invalid",
            message: message.into(),
        }
    }
}

impl fmt::Display for UserConfigurationLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UserConfigurationLoadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserConfigurationReferenceKind {
    Identifier,
    Path,
}

/// Classify an ID-or-path using syntax alone, without probing the filesystem.
pub fn classify_user_configuration_reference(
    value: &str,
) -> Result<UserConfigurationReferenceKind, UserConfigurationLoadError> {
    let lower = value.to_ascii_lowercase();
    if Path::new(value).is_absolute()
        || value.contains('/')
        || value.contains('\\')
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
    {
        return Ok(UserConfigurationReferenceKind::Path);
    }
    let identifier = Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
        .expect("user-configuration identifier regex should compile");
    if identifier.is_match(value) {
        Ok(UserConfigurationReferenceKind::Identifier)
    } else {
        Err(UserConfigurationLoadError::structural(format!(
            "Invalid user-configuration identifier '{value}'."
        )))
    }
}

/// Return the platform-default directory that contains user configurations.
pub fn default_configuration_root() -> Result<PathBuf, UserConfigurationLoadError> {
    #[cfg(target_os = "windows")]
    {
        return env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("EmuChef").join("user-configurations"))
            .ok_or_else(|| UserConfigurationLoadError::io("APPDATA is not set."));
    }
    #[cfg(target_os = "macos")]
    {
        return home_directory().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("EmuChef")
                .join("user-configurations")
        });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(root)
                .join("emuchef")
                .join("user-configurations"));
        }
        return home_directory().map(|home| {
            home.join(".config")
                .join("emuchef")
                .join("user-configurations")
        });
    }
    #[allow(unreachable_code)]
    Err(UserConfigurationLoadError::io(
        "No default configuration directory is available on this platform.",
    ))
}

#[cfg(not(target_os = "windows"))]
fn home_directory() -> Result<PathBuf, UserConfigurationLoadError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| UserConfigurationLoadError::io("HOME is not set."))
}

pub fn resolve_user_configuration_path(
    configuration_root: Option<&Path>,
    id_or_path: &str,
) -> Result<PathBuf, UserConfigurationLoadError> {
    match classify_user_configuration_reference(id_or_path)? {
        UserConfigurationReferenceKind::Path => Ok(PathBuf::from(id_or_path)),
        UserConfigurationReferenceKind::Identifier => Ok(configuration_root
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(default_configuration_root)?
            .join(format!("{id_or_path}.yaml"))),
    }
}

pub fn load_user_configuration(
    path: impl AsRef<Path>,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    let text = fs::read_to_string(path.as_ref()).map_err(|error| {
        UserConfigurationLoadError::io(format!(
            "Failed to read user configuration {}: {error}",
            path.as_ref().display()
        ))
    })?;
    parse_user_configuration(&text)
}

pub fn load_user_configuration_reference(
    configuration_root: Option<&Path>,
    id_or_path: &str,
) -> Result<(PathBuf, UserConfiguration), UserConfigurationLoadError> {
    let path = resolve_user_configuration_path(configuration_root, id_or_path)?;
    let configuration = load_user_configuration(&path)?;
    Ok((path, configuration))
}

pub fn parse_user_configuration(
    text: &str,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    let value = serde_yaml::from_str::<YamlValue>(text)
        .map_err(|error| UserConfigurationLoadError::yaml(error.to_string()))?;
    parse_user_configuration_value(&value)
}

/// Parse an inline JSON document through the canonical user-configuration schema.
///
/// Protocol field names surrounding the document use camelCase, but the
/// document itself retains the persisted schema's snake_case field names.
pub fn parse_inline_user_configuration(
    value: &JsonValue,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    parse_user_configuration_value(&json_to_yaml(value))
}

fn parse_user_configuration_value(
    value: &YamlValue,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    let YamlValue::Mapping(mapping) = value else {
        return Err(UserConfigurationLoadError::structural(
            "User configuration must be a top-level mapping.",
        ));
    };
    parse_user_configuration_mapping(mapping)
}

fn parse_user_configuration_mapping(
    mapping: &Mapping,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    let schema_version = required_i64(mapping, "schema_version")?;
    if !matches!(schema_version, 1 | 2) {
        return Err(UserConfigurationLoadError::structural(format!(
            "Unsupported user-configuration schema_version {schema_version}."
        )));
    }
    let kind = required_non_empty_string(mapping, "kind")?;
    if kind != "user_configuration" {
        return Err(UserConfigurationLoadError::structural(format!(
            "Unsupported user-configuration kind '{kind}'."
        )));
    }
    let id = required_non_empty_string(mapping, "id")?;
    validate_identifier(&id, "user-configuration id")?;
    let name = required_non_empty_string(mapping, "name")?;
    let device_plan = required_non_empty_string(mapping, "device_plan")?;
    let selected_recipes = required_identifier_list(mapping, "selected_recipes")?;
    let bindings = parse_bindings(required_mapping(mapping, "bindings")?)?;
    let compatibility = if schema_version == 2 {
        Some(parse_compatibility(required_mapping(
            mapping,
            "compatibility",
        )?)?)
    } else {
        None
    };

    let mut extensions = BTreeMap::new();
    let mut unsupported_extensions = Vec::new();
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err(UserConfigurationLoadError::structural(
                "User-configuration top-level keys must be strings.",
            ));
        };
        if prohibited_authority_field(key) {
            return Err(UserConfigurationLoadError::structural(format!(
                "User configuration contains prohibited authority field '{key}'."
            )));
        }
        if !KNOWN_FIELDS.contains(&key) {
            extensions.insert(key.to_string(), yaml_to_json(value)?);
            if schema_version == 1 || !key.starts_with("x-") {
                unsupported_extensions.push(key.to_string());
            }
        }
    }

    Ok(UserConfiguration {
        schema_version,
        kind,
        id,
        name,
        device_plan,
        selected_recipes,
        bindings,
        compatibility,
        extensions,
        unsupported_extensions,
    })
}

fn parse_compatibility(
    mapping: &Mapping,
) -> Result<SavedCompatibilityBaseline, UserConfigurationLoadError> {
    reject_unknown_mapping_fields(mapping, &["device_plan", "recipes"], "compatibility")?;
    let device_plan = parse_contract_snapshot(
        required_mapping(mapping, "device_plan")?,
        "compatibility.device_plan",
    )?;
    let recipes_value = mapping
        .get(YamlValue::String("recipes".to_string()))
        .ok_or_else(|| UserConfigurationLoadError::structural("Missing compatibility recipes."))?;
    let YamlValue::Sequence(recipe_values) = recipes_value else {
        return Err(UserConfigurationLoadError::structural(
            "Compatibility recipes must be a list.",
        ));
    };
    let mut recipes = Vec::new();
    for (index, value) in recipe_values.iter().enumerate() {
        let YamlValue::Mapping(recipe) = value else {
            return Err(UserConfigurationLoadError::structural(format!(
                "Compatibility recipe {index} must be a mapping."
            )));
        };
        reject_unknown_mapping_fields(
            recipe,
            &["id", "label", "selected", "fingerprint", "inputs"],
            "compatibility recipe",
        )?;
        let snapshot = parse_contract_snapshot(recipe, "compatibility recipe")?;
        let selected = recipe
            .get(YamlValue::String("selected".to_string()))
            .and_then(YamlValue::as_bool)
            .ok_or_else(|| {
                UserConfigurationLoadError::structural(
                    "Compatibility recipe selected must be a boolean.",
                )
            })?;
        let inputs_value = recipe
            .get(YamlValue::String("inputs".to_string()))
            .ok_or_else(|| {
                UserConfigurationLoadError::structural("Missing compatibility inputs.")
            })?;
        let YamlValue::Sequence(input_values) = inputs_value else {
            return Err(UserConfigurationLoadError::structural(
                "Compatibility inputs must be a list.",
            ));
        };
        let mut inputs = Vec::new();
        for input in input_values {
            let YamlValue::Mapping(input) = input else {
                return Err(UserConfigurationLoadError::structural(
                    "Compatibility input must be a mapping.",
                ));
            };
            reject_unknown_mapping_fields(
                input,
                &["key", "label", "fingerprint"],
                "compatibility input",
            )?;
            inputs.push(SavedInputContractSnapshot {
                key: required_non_empty_string(input, "key")?,
                label: required_non_empty_string(input, "label")?,
                fingerprint: required_fingerprint(input)?,
            });
        }
        recipes.push(SavedRecipeContractSnapshot {
            id: snapshot.id,
            label: snapshot.label,
            selected,
            fingerprint: snapshot.fingerprint,
            inputs,
        });
    }
    Ok(SavedCompatibilityBaseline {
        device_plan,
        recipes,
    })
}

fn parse_contract_snapshot(
    mapping: &Mapping,
    label: &str,
) -> Result<SavedContractSnapshot, UserConfigurationLoadError> {
    let id = required_non_empty_string(mapping, "id")?;
    validate_identifier(&id, label)?;
    Ok(SavedContractSnapshot {
        id,
        label: required_non_empty_string(mapping, "label")?,
        fingerprint: required_fingerprint(mapping)?,
    })
}

fn required_fingerprint(mapping: &Mapping) -> Result<String, UserConfigurationLoadError> {
    let fingerprint = required_non_empty_string(mapping, "fingerprint")?;
    if fingerprint.len() != 64
        || !fingerprint
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(UserConfigurationLoadError::structural(
            "Compatibility fingerprints must be 64 hexadecimal characters.",
        ));
    }
    Ok(fingerprint.to_ascii_lowercase())
}

fn reject_unknown_mapping_fields(
    mapping: &Mapping,
    allowed: &[&str],
    label: &str,
) -> Result<(), UserConfigurationLoadError> {
    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            return Err(UserConfigurationLoadError::structural(format!(
                "{label} keys must be strings."
            )));
        };
        if !allowed.contains(&key) {
            return Err(UserConfigurationLoadError::structural(format!(
                "{label} contains unknown field '{key}'."
            )));
        }
    }
    Ok(())
}

fn prohibited_authority_field(key: &str) -> bool {
    let normalized = key.replace('-', "_").to_ascii_lowercase();
    PROHIBITED_AUTHORITY_FIELDS.contains(&normalized.as_str())
        || normalized.ends_with("_handle")
        || normalized.ends_with("_generation")
}

fn parse_bindings(
    mapping: &Mapping,
) -> Result<IndexMap<String, JsonValue>, UserConfigurationLoadError> {
    let mut bindings = IndexMap::new();
    for (key, entry) in mapping {
        let Some(key) = key.as_str() else {
            return Err(UserConfigurationLoadError::structural(
                "Binding keys must be strings.",
            ));
        };
        validate_binding_key(key)?;
        let YamlValue::Mapping(entry) = entry else {
            return Err(UserConfigurationLoadError::structural(format!(
                "Binding '{key}' must be a mapping."
            )));
        };
        let entry_keys = entry
            .keys()
            .map(|field_key| {
                field_key.as_str().ok_or_else(|| {
                    UserConfigurationLoadError::structural(format!(
                        "Binding '{key}' fields must be strings."
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if entry_keys
            .iter()
            .any(|field| !VALUE_SOURCE_FIELDS.contains(field))
        {
            return Err(UserConfigurationLoadError::structural(format!(
                "Binding '{key}' contains an unknown field."
            )));
        }
        let source_count = VALUE_SOURCE_FIELDS
            .iter()
            .filter(|field| entry_keys.contains(field))
            .count();
        if source_count != 1 {
            return Err(UserConfigurationLoadError::structural(format!(
                "Binding '{key}' must contain exactly one value-source field."
            )));
        }
        if !entry_keys.contains(&"value") {
            return Err(UserConfigurationLoadError::structural(format!(
                "Binding '{key}' uses an unsupported value source."
            )));
        }
        let value = entry
            .get(YamlValue::String("value".to_string()))
            .expect("validated value source should exist");
        bindings.insert(key.to_string(), yaml_to_json(value)?);
    }
    Ok(bindings)
}

pub fn validate_binding_key(key: &str) -> Result<(), UserConfigurationLoadError> {
    let mut parts = key.split('/');
    let recipe_id = parts.next().unwrap_or_default();
    let input_id = parts.next().unwrap_or_default();
    if parts.next().is_some() || recipe_id.is_empty() || input_id.is_empty() {
        return Err(UserConfigurationLoadError::structural(format!(
            "Malformed qualified binding key '{key}'."
        )));
    }
    validate_identifier(recipe_id, "binding recipe id")?;
    validate_identifier(input_id, "binding input id")
}

fn validate_identifier(value: &str, label: &str) -> Result<(), UserConfigurationLoadError> {
    let identifier =
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$").expect("identifier regex should compile");
    if identifier.is_match(value) {
        Ok(())
    } else {
        Err(UserConfigurationLoadError::structural(format!(
            "Invalid {label} '{value}'."
        )))
    }
}

pub fn validate_configuration_identifier(
    value: &str,
    label: &str,
) -> Result<(), UserConfigurationLoadError> {
    validate_identifier(value, label)
}

pub fn emit_user_configuration_yaml(
    configuration: &UserConfiguration,
) -> Result<String, UserConfigurationLoadError> {
    let mut mapping = Mapping::new();
    insert_yaml(
        &mut mapping,
        "schema_version",
        YamlValue::Number(configuration.schema_version.into()),
    );
    insert_yaml(
        &mut mapping,
        "kind",
        YamlValue::String(configuration.kind.clone()),
    );
    insert_yaml(
        &mut mapping,
        "id",
        YamlValue::String(configuration.id.clone()),
    );
    insert_yaml(
        &mut mapping,
        "name",
        YamlValue::String(configuration.name.clone()),
    );
    insert_yaml(
        &mut mapping,
        "device_plan",
        YamlValue::String(configuration.device_plan.clone()),
    );
    insert_yaml(
        &mut mapping,
        "selected_recipes",
        YamlValue::Sequence(
            configuration
                .selected_recipes
                .iter()
                .map(|recipe| YamlValue::String(recipe.clone()))
                .collect(),
        ),
    );
    let mut bindings = Mapping::new();
    let mut binding_entries = configuration.bindings.iter().collect::<Vec<_>>();
    binding_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in binding_entries {
        let mut entry = Mapping::new();
        insert_yaml(&mut entry, "value", json_to_yaml(value));
        insert_yaml(&mut bindings, key, YamlValue::Mapping(entry));
    }
    insert_yaml(&mut mapping, "bindings", YamlValue::Mapping(bindings));

    if let Some(compatibility) = &configuration.compatibility {
        insert_yaml(
            &mut mapping,
            "compatibility",
            emit_compatibility(compatibility),
        );
    }

    for (key, value) in &configuration.extensions {
        if !KNOWN_FIELDS.contains(&key.as_str()) {
            insert_yaml(&mut mapping, key, json_to_yaml(value));
        }
    }

    let output = serde_yaml::to_string(&YamlValue::Mapping(mapping))
        .map_err(|error| UserConfigurationLoadError::yaml(error.to_string()))?;
    Ok(output.strip_prefix("---\n").unwrap_or(&output).to_string())
}

fn emit_compatibility(compatibility: &SavedCompatibilityBaseline) -> YamlValue {
    let mut mapping = Mapping::new();
    insert_yaml(
        &mut mapping,
        "device_plan",
        emit_contract_snapshot(&compatibility.device_plan),
    );
    let recipes = compatibility
        .recipes
        .iter()
        .map(|recipe| {
            let mut mapping = match emit_contract_snapshot(&SavedContractSnapshot {
                id: recipe.id.clone(),
                label: recipe.label.clone(),
                fingerprint: recipe.fingerprint.clone(),
            }) {
                YamlValue::Mapping(mapping) => mapping,
                _ => unreachable!("contract snapshots are mappings"),
            };
            insert_yaml(&mut mapping, "selected", YamlValue::Bool(recipe.selected));
            insert_yaml(
                &mut mapping,
                "inputs",
                YamlValue::Sequence(
                    recipe
                        .inputs
                        .iter()
                        .map(|input| {
                            let mut mapping = Mapping::new();
                            insert_yaml(&mut mapping, "key", YamlValue::String(input.key.clone()));
                            insert_yaml(
                                &mut mapping,
                                "label",
                                YamlValue::String(input.label.clone()),
                            );
                            insert_yaml(
                                &mut mapping,
                                "fingerprint",
                                YamlValue::String(input.fingerprint.clone()),
                            );
                            YamlValue::Mapping(mapping)
                        })
                        .collect(),
                ),
            );
            YamlValue::Mapping(mapping)
        })
        .collect();
    insert_yaml(&mut mapping, "recipes", YamlValue::Sequence(recipes));
    YamlValue::Mapping(mapping)
}

fn emit_contract_snapshot(snapshot: &SavedContractSnapshot) -> YamlValue {
    let mut mapping = Mapping::new();
    insert_yaml(&mut mapping, "id", YamlValue::String(snapshot.id.clone()));
    insert_yaml(
        &mut mapping,
        "label",
        YamlValue::String(snapshot.label.clone()),
    );
    insert_yaml(
        &mut mapping,
        "fingerprint",
        YamlValue::String(snapshot.fingerprint.clone()),
    );
    YamlValue::Mapping(mapping)
}

pub fn validate_user_configuration_with_catalog(
    configuration: &UserConfiguration,
    configuration_path: &Path,
    authored_root: &Path,
) -> Vec<JsonValue> {
    let normalized_root = catalog::normalize_authored_root(
        Some(&authored_root.to_string_lossy()),
        configuration_path,
    )
    .unwrap_or_else(|| authored_root.to_path_buf());
    let recipes = match planner::load_top_level_recipes(&normalized_root) {
        Ok(recipes) => recipes,
        Err(error) => {
            return vec![configuration_diagnostic(
                "authored_catalog_invalid",
                &error.to_string(),
                None,
                "catalog",
                json!({}),
            )];
        }
    };
    validate_with_recipes(configuration, &normalized_root, &recipes)
}

/// Build the durable authored-contract baseline for the current catalog.
///
/// This deliberately hashes authored semantics rather than planner output.
/// Resolved values, paths, device facts, generated plans, and presentation
/// prose never participate in the fingerprint.
pub fn build_compatibility_baseline(
    configuration: &UserConfiguration,
    configuration_path: &Path,
    authored_root: &Path,
) -> Result<SavedCompatibilityBaseline, UserConfigurationLoadError> {
    let normalized_root = catalog::normalize_authored_root(
        Some(&authored_root.to_string_lossy()),
        configuration_path,
    )
    .unwrap_or_else(|| authored_root.to_path_buf());
    let recipes = planner::load_top_level_recipes(&normalized_root).map_err(|error| {
        UserConfigurationLoadError::structural(format!(
            "The authored catalog cannot establish a compatibility baseline: {error}"
        ))
    })?;
    let parts = planner_device_plan::load_planner_input_parts(
        &normalized_root,
        &configuration.device_plan,
        &recipes,
    )
    .map_err(|error| {
        UserConfigurationLoadError::structural(format!(
            "The selected device setup cannot establish a compatibility baseline: {error}"
        ))
    })?;

    let device_plan_value = json!({
        "id": parts.device_plan_ref,
        "profile": parts.device_profile_ref,
        "recipes": sorted_strings(parts.recipe_refs.clone()),
        "selectedRecipes": sorted_strings(parts.selected_recipe_refs.clone()),
        "overrides": parts.device_plan_input_bindings,
        "profileCapabilities": parts.runtime_capabilities,
    });
    let device_plan = SavedContractSnapshot {
        id: configuration.device_plan.clone(),
        label: parts
            .device_plan_name
            .clone()
            .unwrap_or_else(|| "Saved setup".to_string()),
        fingerprint: fingerprint_value(&device_plan_value),
    };

    let recipe_by_id = recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let expanded = expanded_recipe_contracts(&configuration.selected_recipes, &recipe_by_id)?;
    let selected = configuration
        .selected_recipes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut recipe_snapshots = Vec::new();
    for recipe_id in expanded {
        let recipe = recipe_by_id.get(recipe_id.as_str()).ok_or_else(|| {
            UserConfigurationLoadError::structural(format!(
                "Recipe '{recipe_id}' is unavailable for compatibility baseline creation."
            ))
        })?;
        let mut inputs = recipe
            .inputs
            .iter()
            .map(|(input_id, input)| SavedInputContractSnapshot {
                key: format!("{}/{input_id}", recipe.id),
                label: input.label.clone(),
                fingerprint: fingerprint_value(&input_contract_value(input_id, input)),
            })
            .collect::<Vec<_>>();
        inputs.sort_by(|left, right| left.key.cmp(&right.key));
        recipe_snapshots.push(SavedRecipeContractSnapshot {
            id: recipe.id.clone(),
            label: recipe.name.clone(),
            selected: selected.contains(recipe.id.as_str()),
            fingerprint: fingerprint_value(&recipe_contract_value(recipe)),
            inputs,
        });
    }
    recipe_snapshots.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(SavedCompatibilityBaseline {
        device_plan,
        recipes: recipe_snapshots,
    })
}

pub fn compatibility_baseline_state(
    configuration: &UserConfiguration,
    current: &SavedCompatibilityBaseline,
) -> CompatibilityBaselineState {
    match &configuration.compatibility {
        None => CompatibilityBaselineState::PendingFirstV2Save,
        Some(saved) if baseline_semantics_equal(saved, current) => {
            CompatibilityBaselineState::Unchanged
        }
        Some(_) => CompatibilityBaselineState::MateriallyChanged,
    }
}

/// Convert a validated document to V2 and establish the first durable baseline.
/// Unsupported extensions are returned so callers can disclose sanitation.
pub fn prepare_configuration_for_v2_write(
    configuration: &mut UserConfiguration,
    configuration_path: &Path,
    authored_root: &Path,
) -> Result<Vec<String>, UserConfigurationLoadError> {
    let baseline = build_compatibility_baseline(configuration, configuration_path, authored_root)?;
    let sanitized = configuration.unsupported_extensions.clone();
    for key in &sanitized {
        configuration.extensions.remove(key);
    }
    configuration.unsupported_extensions.clear();
    configuration.schema_version = 2;
    configuration.compatibility = Some(baseline);
    Ok(sanitized)
}

fn baseline_semantics_equal(
    left: &SavedCompatibilityBaseline,
    right: &SavedCompatibilityBaseline,
) -> bool {
    left.device_plan.id == right.device_plan.id
        && left.device_plan.fingerprint == right.device_plan.fingerprint
        && left.recipes.len() == right.recipes.len()
        && left
            .recipes
            .iter()
            .zip(&right.recipes)
            .all(|(left, right)| {
                left.id == right.id
                    && left.selected == right.selected
                    && left.fingerprint == right.fingerprint
                    && left.inputs.len() == right.inputs.len()
                    && left.inputs.iter().zip(&right.inputs).all(|(left, right)| {
                        left.key == right.key && left.fingerprint == right.fingerprint
                    })
            })
}

fn expanded_recipe_contracts(
    selected: &[String],
    recipes: &HashMap<&str, &Recipe>,
) -> Result<Vec<String>, UserConfigurationLoadError> {
    fn visit(
        id: &str,
        recipes: &HashMap<&str, &Recipe>,
        visiting: &mut HashSet<String>,
        expanded: &mut HashSet<String>,
    ) -> Result<(), UserConfigurationLoadError> {
        if expanded.contains(id) {
            return Ok(());
        }
        let recipe = recipes.get(id).ok_or_else(|| {
            UserConfigurationLoadError::structural(format!(
                "Recipe '{id}' is unavailable for compatibility baseline creation."
            ))
        })?;
        if !visiting.insert(id.to_string()) {
            return Err(UserConfigurationLoadError::structural(format!(
                "Recipe dependency cycle reaches '{id}'."
            )));
        }
        for dependency in &recipe.recipe_dependencies {
            visit(dependency, recipes, visiting, expanded)?;
        }
        visiting.remove(id);
        expanded.insert(id.to_string());
        Ok(())
    }

    let mut expanded = HashSet::new();
    let mut visiting = HashSet::new();
    for id in selected {
        visit(id, recipes, &mut visiting, &mut expanded)?;
    }
    let mut expanded = expanded.into_iter().collect::<Vec<_>>();
    expanded.sort();
    Ok(expanded)
}

fn recipe_contract_value(recipe: &Recipe) -> JsonValue {
    let inputs = recipe
        .inputs
        .iter()
        .map(|(id, input)| (id.clone(), input_contract_value(id, input)))
        .collect::<JsonMap<_, _>>();
    let artifacts = recipe
        .artifacts
        .iter()
        .map(|(id, artifact)| {
            (
                id.clone(),
                json!({
                    "type": artifact.type_name,
                    "url": artifact.url,
                    "cache": artifact.cache,
                }),
            )
        })
        .collect::<JsonMap<_, _>>();
    let artifact_groups = recipe
        .artifact_groups
        .iter()
        .map(|(id, members)| (id.clone(), json!(sorted_strings(members.clone()))))
        .collect::<JsonMap<_, _>>();
    json!({
        "id": recipe.id,
        "dependencies": sorted_strings(recipe.recipe_dependencies.clone()),
        "provides": sorted_strings(recipe.provides.features.clone()),
        "inputs": inputs,
        "artifacts": artifacts,
        "artifactGroups": artifact_groups,
        "steps": recipe.steps.iter().map(step_contract_value).collect::<Vec<_>>(),
    })
}

fn input_contract_value(id: &str, input: &InputDeclaration) -> JsonValue {
    json!({
        "id": id,
        "type": input.type_name,
        "role": input.role,
        "required": input.required,
        "multiple": input.multiple,
        "validation": {
            "mustExist": input.validation.must_exist,
            "allowedExtensions": sorted_strings(input.validation.allowed_extensions.clone()),
            "pathKind": input.validation.path_kind,
            "allowedPrefixes": sorted_strings(input.validation.allowed_prefixes.clone()),
        },
        "default": input.default,
        "options": input.options.iter().map(|option| option.value.clone()).collect::<Vec<_>>(),
        "sensitive": input.sensitive,
        "metadata": input.metadata,
    })
}

fn step_contract_value(step: &crate::model::Step) -> JsonValue {
    json!({
        "id": step.id,
        "type": step.type_name,
        "userToggleable": step.user_toggleable,
        "dependencies": sorted_strings(step.dependencies.clone()),
        "constraints": {
            "capabilities": sorted_strings(step.constraints.capabilities.clone()),
            "conflictsWith": sorted_strings(step.constraints.conflicts_with.clone()),
        },
        "skipIf": step.skip_if.iter().map(condition_contract_value).collect::<Vec<_>>(),
        "params": step.params.iter().map(|(key, value)| {
            let value = match value {
                ParamValue::Ref(reference) => json!({ "ref": reference }),
                ParamValue::Literal(value) => json!({ "literal": value }),
            };
            (key.clone(), value)
        }).collect::<JsonMap<_, _>>(),
        "verify": step.verify.iter().map(condition_contract_value).collect::<Vec<_>>(),
    })
}

fn condition_contract_value(condition: &StepCondition) -> JsonValue {
    json!({ "type": condition.type_name, "params": condition.params })
}

fn sorted_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn fingerprint_value(value: &JsonValue) -> String {
    let canonical = canonical_json(value);
    let bytes = serde_json::to_vec(&canonical).expect("JSON contract values must serialize");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_json(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(mapping) => {
            let mut entries = mapping.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            JsonValue::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        JsonValue::Array(values) => JsonValue::Array(values.iter().map(canonical_json).collect()),
        value => value.clone(),
    }
}

fn validate_with_recipes(
    configuration: &UserConfiguration,
    authored_root: &Path,
    recipes: &[Recipe],
) -> Vec<JsonValue> {
    let recipe_by_id = recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut effective_recipes = Vec::new();
    let mut visiting = HashSet::new();
    for recipe_id in &configuration.selected_recipes {
        expand_recipe_selection(
            recipe_id,
            &recipe_by_id,
            &mut effective_recipes,
            &mut visiting,
            &mut diagnostics,
        );
    }
    let effective_set = effective_recipes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    let device_overrides = match planner_device_plan::load_planner_input_parts(
        authored_root,
        &configuration.device_plan,
        recipes,
    ) {
        Ok(parts) => parts.device_plan_input_bindings,
        Err(error) => {
            diagnostics.push(configuration_diagnostic(
                error.code(),
                &error.to_string(),
                None,
                "user_configuration",
                json!({ "devicePlan": configuration.device_plan }),
            ));
            IndexMap::new()
        }
    };

    for (key, value) in &configuration.bindings {
        let (recipe_id, input_id) = key
            .split_once('/')
            .expect("structural validation should qualify binding keys");
        let Some(recipe) = recipe_by_id.get(recipe_id) else {
            diagnostics.push(configuration_diagnostic(
                "unknown_recipe",
                &format!("Binding '{key}' references an unknown recipe."),
                Some(key),
                "user_configuration",
                json!({}),
            ));
            continue;
        };
        let Some(input) = recipe.inputs.get(input_id) else {
            diagnostics.push(configuration_diagnostic(
                "unknown_input",
                &format!("Binding '{key}' references an unknown input."),
                Some(key),
                "user_configuration",
                json!({}),
            ));
            continue;
        };
        if !effective_set.contains(recipe_id) {
            diagnostics.push(configuration_diagnostic(
                "binding_recipe_not_selected",
                &format!("Binding '{key}' is outside the selected recipe dependency set."),
                Some(key),
                "user_configuration",
                json!({}),
            ));
        }
        diagnostics.extend(validate_saved_value(key, input, value));
    }

    for recipe_id in effective_recipes {
        let Some(recipe) = recipe_by_id.get(recipe_id.as_str()) else {
            continue;
        };
        for (input_id, input) in &recipe.inputs {
            let key = format!("{recipe_id}/{input_id}");
            if input.required
                && !configuration.bindings.contains_key(&key)
                && !device_overrides.contains_key(&key)
                && input.default.is_null()
            {
                diagnostics.push(configuration_diagnostic(
                    "missing_required_input",
                    &format!("Required input '{key}' has no value."),
                    Some(&key),
                    "missing",
                    json!({ "expectedType": input.type_name }),
                ));
            }
        }
    }
    diagnostics
}

fn expand_recipe_selection(
    recipe_id: &str,
    recipes: &HashMap<&str, &Recipe>,
    expanded: &mut Vec<String>,
    visiting: &mut HashSet<String>,
    diagnostics: &mut Vec<JsonValue>,
) {
    if expanded.iter().any(|existing| existing == recipe_id) {
        return;
    }
    let Some(recipe) = recipes.get(recipe_id) else {
        diagnostics.push(configuration_diagnostic(
            "unknown_recipe",
            &format!("Selected recipe '{recipe_id}' was not found."),
            None,
            "user_configuration",
            json!({ "recipeId": recipe_id }),
        ));
        return;
    };
    if !visiting.insert(recipe_id.to_string()) {
        diagnostics.push(configuration_diagnostic(
            "dependency_cycle",
            &format!("Recipe dependency cycle reaches '{recipe_id}'."),
            None,
            "catalog",
            json!({ "recipeId": recipe_id }),
        ));
        return;
    }
    expanded.push(recipe_id.to_string());
    for dependency in &recipe.recipe_dependencies {
        expand_recipe_selection(dependency, recipes, expanded, visiting, diagnostics);
    }
    visiting.remove(recipe_id);
}

fn validate_saved_value(key: &str, input: &InputDeclaration, value: &JsonValue) -> Vec<JsonValue> {
    if !input.value_matches_type(value) {
        return vec![configuration_diagnostic(
            "incompatible_binding_type",
            &format!("Binding '{key}' is incompatible with its declared input type."),
            Some(key),
            "user_configuration",
            json!({ "expectedType": input.type_name }),
        )];
    }
    let mut diagnostics = Vec::new();
    for item in input.binding_items(value).into_iter().flatten() {
        if input.type_name == "enum" && !input.options.iter().any(|option| option.value == *item) {
            diagnostics.push(configuration_diagnostic(
                "invalid_enum_value",
                &format!("Binding '{key}' is not a declared enum option."),
                Some(key),
                "user_configuration",
                json!({
                    "expected": input.options.iter().map(|option| option.value.clone()).collect::<Vec<_>>(),
                }),
            ));
        }
        if let Some(path) = item.as_str() {
            if !input.validation.allowed_prefixes.is_empty()
                && !input
                    .validation
                    .allowed_prefixes
                    .iter()
                    .any(|prefix| path_has_prefix(path, prefix))
            {
                diagnostics.push(configuration_diagnostic(
                    "invalid_path_prefix",
                    &format!("Binding '{key}' is outside its allowed path prefixes."),
                    Some(key),
                    "user_configuration",
                    json!({ "allowedPrefixes": input.validation.allowed_prefixes }),
                ));
            }
        }
    }
    diagnostics
}

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    prefix == "/" && path.starts_with('/')
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn configuration_diagnostic(
    code: &str,
    message: &str,
    key: Option<&str>,
    provenance: &str,
    details: JsonValue,
) -> JsonValue {
    json!({
        "severity": "error",
        "code": code,
        "message": message,
        "key": key,
        "provenance": provenance,
        "details": details,
    })
}

fn required_mapping<'a>(
    mapping: &'a Mapping,
    key: &str,
) -> Result<&'a Mapping, UserConfigurationLoadError> {
    match mapping.get(YamlValue::String(key.to_string())) {
        Some(YamlValue::Mapping(value)) => Ok(value),
        _ => Err(UserConfigurationLoadError::structural(format!(
            "Required field '{key}' must be a mapping."
        ))),
    }
}

fn required_i64(mapping: &Mapping, key: &str) -> Result<i64, UserConfigurationLoadError> {
    mapping
        .get(YamlValue::String(key.to_string()))
        .and_then(YamlValue::as_i64)
        .ok_or_else(|| {
            UserConfigurationLoadError::structural(format!(
                "Required field '{key}' must be an integer."
            ))
        })
}

fn required_non_empty_string(
    mapping: &Mapping,
    key: &str,
) -> Result<String, UserConfigurationLoadError> {
    match mapping.get(YamlValue::String(key.to_string())) {
        Some(YamlValue::String(value)) if !value.trim().is_empty() => Ok(value.clone()),
        _ => Err(UserConfigurationLoadError::structural(format!(
            "Required field '{key}' must be a non-empty string."
        ))),
    }
}

fn required_identifier_list(
    mapping: &Mapping,
    key: &str,
) -> Result<Vec<String>, UserConfigurationLoadError> {
    let Some(YamlValue::Sequence(values)) = mapping.get(YamlValue::String(key.to_string())) else {
        return Err(UserConfigurationLoadError::structural(format!(
            "Required field '{key}' must be a list."
        )));
    };
    values
        .iter()
        .map(|value| {
            let YamlValue::String(value) = value else {
                return Err(UserConfigurationLoadError::structural(format!(
                    "Field '{key}' entries must be strings."
                )));
            };
            validate_identifier(value, "selected recipe id")?;
            Ok(value.clone())
        })
        .collect()
}

fn yaml_to_json(value: &YamlValue) -> Result<JsonValue, UserConfigurationLoadError> {
    match value {
        YamlValue::Null => Ok(JsonValue::Null),
        YamlValue::Bool(value) => Ok(JsonValue::Bool(*value)),
        YamlValue::Number(value) => serde_json::to_value(value)
            .map_err(|error| UserConfigurationLoadError::structural(error.to_string())),
        YamlValue::String(value) => Ok(JsonValue::String(value.clone())),
        YamlValue::Sequence(values) => values.iter().map(yaml_to_json).collect(),
        YamlValue::Mapping(mapping) => {
            let mut result = JsonMap::new();
            let mut entries = mapping.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| key.as_str().unwrap_or_default());
            for (key, value) in entries {
                let Some(key) = key.as_str() else {
                    return Err(UserConfigurationLoadError::structural(
                        "Extension object keys must be strings.",
                    ));
                };
                result.insert(key.to_string(), yaml_to_json(value)?);
            }
            Ok(JsonValue::Object(result))
        }
        YamlValue::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

fn json_to_yaml(value: &JsonValue) -> YamlValue {
    match value {
        JsonValue::Null => YamlValue::Null,
        JsonValue::Bool(value) => YamlValue::Bool(*value),
        JsonValue::Number(value) => serde_yaml::to_value(value).unwrap_or(YamlValue::Null),
        JsonValue::String(value) => YamlValue::String(value.clone()),
        JsonValue::Array(values) => YamlValue::Sequence(values.iter().map(json_to_yaml).collect()),
        JsonValue::Object(values) => {
            let mut mapping = Mapping::new();
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                insert_yaml(&mut mapping, key, json_to_yaml(value));
            }
            YamlValue::Mapping(mapping)
        }
    }
}

fn insert_yaml(mapping: &mut Mapping, key: &str, value: YamlValue) {
    mapping.insert(YamlValue::String(key.to_string()), value);
}

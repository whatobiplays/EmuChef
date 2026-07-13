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

use crate::catalog;
use crate::model::{InputDeclaration, Recipe};
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
];
const VALUE_SOURCE_FIELDS: &[&str] = &["value", "local", "secret"];

#[derive(Clone, Debug, PartialEq)]
pub struct UserConfiguration {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    pub device_plan: String,
    pub selected_recipes: Vec<String>,
    pub bindings: IndexMap<String, JsonValue>,
    pub extensions: BTreeMap<String, JsonValue>,
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
    let YamlValue::Mapping(mapping) = value else {
        return Err(UserConfigurationLoadError::structural(
            "User configuration must be a top-level mapping.",
        ));
    };
    parse_user_configuration_mapping(&mapping)
}

fn parse_user_configuration_mapping(
    mapping: &Mapping,
) -> Result<UserConfiguration, UserConfigurationLoadError> {
    let schema_version = required_i64(mapping, "schema_version")?;
    if schema_version != 1 {
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

    let mut extensions = BTreeMap::new();
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            return Err(UserConfigurationLoadError::structural(
                "User-configuration top-level keys must be strings.",
            ));
        };
        if !KNOWN_FIELDS.contains(&key) {
            extensions.insert(key.to_string(), yaml_to_json(value)?);
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
        extensions,
    })
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

    for (key, value) in &configuration.extensions {
        if !KNOWN_FIELDS.contains(&key.as_str()) {
            insert_yaml(&mut mapping, key, json_to_yaml(value));
        }
    }

    let output = serde_yaml::to_string(&YamlValue::Mapping(mapping))
        .map_err(|error| UserConfigurationLoadError::yaml(error.to_string()))?;
    Ok(output.strip_prefix("---\n").unwrap_or(&output).to_string())
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
        Ok(parts) => parts.override_input_bindings,
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

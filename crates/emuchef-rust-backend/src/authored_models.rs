//! Typed schema-v1 models for authored app definitions and device profiles.
//!
//! These models define the structural and semantic authority shared by catalog
//! validation, product catalog loading, and future authored-data generators.
//! Fixed schema objects reject unknown fields. Deliberately extensible mappings
//! retain JSON-compatible nested values and insertion order.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const APP_DEFINITION_KIND: &str = "app_definition";
pub const DEVICE_PROFILE_KIND: &str = "device_profile";
pub const SCHEMA_VERSION_V1: i64 = 1;

/// An insertion-ordered, string-keyed mapping of nested JSON-compatible data.
pub type OrderedValueMap = IndexMap<String, Value>;

/// One deterministic semantic-validation diagnostic for an authored model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthoredModelDiagnostic {
    pub code: String,
    pub message: String,
    pub field: String,
}

impl AuthoredModelDiagnostic {
    fn new(code: &str, message: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            field: field.into(),
        }
    }
}

/// A sanitized load, parse, or canonical-emission failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredModelError {
    code: String,
    message: String,
}

impl AuthoredModelError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AuthoredModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AuthoredModelError {}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppDefinitionV1 {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub category: String,
    pub package: AppPackage,
    pub install_source: AppInstallSource,
    pub tracking_source: AppTrackingSource,
    pub artifacts: AppArtifactSupport,
    pub provisioning: AppProvisioning,
    #[serde(default)]
    pub inputs: Vec<OrderedValueMap>,
    #[serde(default)]
    pub metadata: OrderedValueMap,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppPackage {
    pub primary: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppInstallSource {
    #[serde(rename = "type")]
    pub type_name: String,
    pub resolver: String,
    pub options: OrderedValueMap,
}

/// Tracking-source metadata has one required discriminator and ordered extension fields.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AppTrackingSource {
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(flatten)]
    pub fields: OrderedValueMap,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppArtifactSupport {
    pub apk: RequiredArtifactSupport,
    pub shared_storage_config: ConfigArtifactSupport,
    pub app_data_config: ConfigArtifactSupport,
    pub byo_apk: RequiredArtifactSupport,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RequiredArtifactSupport {
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfigArtifactSupport {
    pub supported: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppProvisioning {
    #[serde(default)]
    pub launch_once_recommended: bool,
    #[serde(default)]
    pub shared_storage_paths: Vec<String>,
    #[serde(default)]
    pub app_data_paths: Vec<String>,
    #[serde(default)]
    pub config_targets: Vec<OrderedValueMap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfileV1 {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "match")]
    pub match_criteria: DeviceMatchCriteria,
    pub capability_defaults: DeviceCapabilityDefaults,
    #[serde(default)]
    pub device_tags: Vec<String>,
    #[serde(default)]
    pub metadata: OrderedValueMap,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeviceMatchCriteria {
    #[serde(default)]
    pub manufacturer_contains: Vec<String>,
    #[serde(default)]
    pub brand_contains: Vec<String>,
    #[serde(default)]
    pub model_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_version: Option<AndroidVersionRange>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AndroidVersionRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeviceCapabilityDefaults {
    pub adb_available: bool,
    pub apk_install: bool,
    pub shared_storage_write: bool,
    pub app_launch: bool,
    pub shell_command: bool,
    pub package_remove_for_user: bool,
    pub root_shell: bool,
    pub app_data_write: bool,
}

pub fn load_app_definition(path: impl AsRef<Path>) -> Result<AppDefinitionV1, AuthoredModelError> {
    let source = fs::read_to_string(path.as_ref()).map_err(|_| {
        AuthoredModelError::new(
            "app_definition_io_error",
            "The app definition could not be read.",
        )
    })?;
    parse_app_definition_yaml(&source)
}

pub fn parse_app_definition_yaml(source: &str) -> Result<AppDefinitionV1, AuthoredModelError> {
    serde_yaml::from_str(source).map_err(|_| {
        AuthoredModelError::new(
            "app_definition_yaml_invalid",
            "The app definition is not valid schema-v1 YAML.",
        )
    })
}

pub fn emit_app_definition_yaml(value: &AppDefinitionV1) -> Result<String, AuthoredModelError> {
    require_valid(
        "app_definition_invalid",
        "The app definition failed semantic validation.",
        validate_app_definition(value),
    )?;
    serde_yaml::to_string(value).map_err(|_| {
        AuthoredModelError::new(
            "app_definition_emit_failed",
            "The app definition could not be emitted as canonical YAML.",
        )
    })
}

pub fn load_device_profile(path: impl AsRef<Path>) -> Result<DeviceProfileV1, AuthoredModelError> {
    let source = fs::read_to_string(path.as_ref()).map_err(|_| {
        AuthoredModelError::new(
            "device_profile_io_error",
            "The device profile could not be read.",
        )
    })?;
    parse_device_profile_yaml(&source)
}

pub fn parse_device_profile_yaml(source: &str) -> Result<DeviceProfileV1, AuthoredModelError> {
    serde_yaml::from_str(source).map_err(|_| {
        AuthoredModelError::new(
            "device_profile_yaml_invalid",
            "The device profile is not valid schema-v1 YAML.",
        )
    })
}

pub fn emit_device_profile_yaml(value: &DeviceProfileV1) -> Result<String, AuthoredModelError> {
    require_valid(
        "device_profile_invalid",
        "The device profile failed semantic validation.",
        validate_device_profile(value),
    )?;
    serde_yaml::to_string(value).map_err(|_| {
        AuthoredModelError::new(
            "device_profile_emit_failed",
            "The device profile could not be emitted as canonical YAML.",
        )
    })
}

pub fn validate_app_definition(value: &AppDefinitionV1) -> Vec<AuthoredModelDiagnostic> {
    let mut diagnostics = Vec::new();
    validate_identity(
        &mut diagnostics,
        value.schema_version,
        &value.kind,
        APP_DEFINITION_KIND,
        &value.id,
        &value.name,
    );
    validate_nonblank(
        &mut diagnostics,
        "app_category_invalid",
        "App category must not be empty.",
        "category",
        &value.category,
    );
    validate_package_name(&mut diagnostics, "package.primary", &value.package.primary);
    validate_package_aliases(&mut diagnostics, value);
    validate_nonblank(
        &mut diagnostics,
        "install_source_type_invalid",
        "Install source type must not be empty.",
        "install_source.type",
        &value.install_source.type_name,
    );
    validate_nonblank(
        &mut diagnostics,
        "install_source_resolver_invalid",
        "Install source resolver must not be empty.",
        "install_source.resolver",
        &value.install_source.resolver,
    );
    validate_ordered_map_keys(
        &mut diagnostics,
        "install_source.options",
        &value.install_source.options,
    );
    validate_nonblank(
        &mut diagnostics,
        "tracking_source_type_invalid",
        "Tracking source type must not be empty.",
        "tracking_source.type",
        &value.tracking_source.type_name,
    );
    validate_ordered_map_keys(
        &mut diagnostics,
        "tracking_source",
        &value.tracking_source.fields,
    );
    validate_string_list(
        &mut diagnostics,
        "provisioning.shared_storage_paths",
        &value.provisioning.shared_storage_paths,
        "provisioning_path_invalid",
        false,
    );
    validate_string_list(
        &mut diagnostics,
        "provisioning.app_data_paths",
        &value.provisioning.app_data_paths,
        "provisioning_path_invalid",
        false,
    );
    validate_map_list(
        &mut diagnostics,
        "provisioning.config_targets",
        &value.provisioning.config_targets,
    );
    validate_map_list(&mut diagnostics, "inputs", &value.inputs);
    validate_ordered_map_keys(&mut diagnostics, "metadata", &value.metadata);
    diagnostics
}

pub fn validate_device_profile(value: &DeviceProfileV1) -> Vec<AuthoredModelDiagnostic> {
    let mut diagnostics = Vec::new();
    validate_identity(
        &mut diagnostics,
        value.schema_version,
        &value.kind,
        DEVICE_PROFILE_KIND,
        &value.id,
        &value.name,
    );

    let criteria = &value.match_criteria;
    if criteria.manufacturer_contains.is_empty()
        && criteria.brand_contains.is_empty()
        && criteria.model_patterns.is_empty()
        && criteria.android_version.is_none()
    {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "device_match_empty",
            "Device match criteria must include at least one constraint.",
            "match",
        ));
    }
    validate_string_list(
        &mut diagnostics,
        "match.manufacturer_contains",
        &criteria.manufacturer_contains,
        "device_match_value_invalid",
        false,
    );
    validate_string_list(
        &mut diagnostics,
        "match.brand_contains",
        &criteria.brand_contains,
        "device_match_value_invalid",
        false,
    );
    validate_model_patterns(&mut diagnostics, &criteria.model_patterns);
    validate_android_range(&mut diagnostics, criteria.android_version.as_ref());
    validate_string_list(
        &mut diagnostics,
        "device_tags",
        &value.device_tags,
        "device_tag_invalid",
        true,
    );
    validate_ordered_map_keys(&mut diagnostics, "metadata", &value.metadata);
    diagnostics
}

fn require_valid(
    code: &str,
    message: &str,
    diagnostics: Vec<AuthoredModelDiagnostic>,
) -> Result<(), AuthoredModelError> {
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(AuthoredModelError::new(code, message))
    }
}

fn validate_identity(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    schema_version: i64,
    kind: &str,
    expected_kind: &str,
    id: &str,
    name: &str,
) {
    if schema_version != SCHEMA_VERSION_V1 {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "schema_version_unsupported",
            "schema_version must be 1.",
            "schema_version",
        ));
    }
    if kind != expected_kind {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "authored_kind_invalid",
            format!("kind must be '{expected_kind}'."),
            "kind",
        ));
    }
    if !is_valid_identifier(id) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "authored_id_invalid",
            "ID must contain lowercase alphanumeric segments separated by '.', '_', or '-'.",
            "id",
        ));
    }
    validate_nonblank(
        diagnostics,
        "authored_name_invalid",
        "Name must not be empty.",
        "name",
        name,
    );
}

fn validate_package_aliases(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    value: &AppDefinitionV1,
) {
    let mut seen = HashSet::new();
    for (index, alias) in value.package.aliases.iter().enumerate() {
        validate_package_name(diagnostics, &format!("package.aliases[{index}]"), alias);
        if alias == &value.package.primary || !seen.insert(alias.as_str()) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "package_alias_duplicate",
                "Package aliases must be unique and must differ from package.primary.",
                format!("package.aliases[{index}]"),
            ));
        }
    }
}

fn validate_package_name(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    package: &str,
) {
    let package_pattern = Regex::new(r"^[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+$")
        .expect("the package-name regex is valid");
    if !package_pattern.is_match(package) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "package_name_invalid",
            "Package names must contain at least two valid dot-separated identifier segments.",
            field,
        ));
    }
}

fn validate_model_patterns(diagnostics: &mut Vec<AuthoredModelDiagnostic>, patterns: &[String]) {
    let mut seen = HashSet::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let field = format!("match.model_patterns[{index}]");
        if pattern.trim().is_empty() {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "device_model_pattern_invalid",
                "Device model patterns must not be empty.",
                field,
            ));
        } else if Regex::new(pattern).is_err() {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "device_model_pattern_invalid",
                "Device model pattern is not a valid regular expression.",
                field,
            ));
        }
        if !seen.insert(pattern.as_str()) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "device_match_value_duplicate",
                "Device match values must not be duplicated.",
                format!("match.model_patterns[{index}]"),
            ));
        }
    }
}

fn validate_android_range(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    range: Option<&AndroidVersionRange>,
) {
    let Some(range) = range else {
        return;
    };
    if range.min.is_none() && range.max.is_none() {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "android_version_range_empty",
            "Android version range must define min, max, or both.",
            "match.android_version",
        ));
    }
    if range.min.is_some_and(|value| value < 1) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "android_version_min_invalid",
            "Android minimum version must be greater than zero.",
            "match.android_version.min",
        ));
    }
    if range.max.is_some_and(|value| value < 1) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "android_version_max_invalid",
            "Android maximum version must be greater than zero.",
            "match.android_version.max",
        ));
    }
    if matches!((range.min, range.max), (Some(min), Some(max)) if min > max) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "android_version_range_invalid",
            "Android minimum version must not exceed the maximum version.",
            "match.android_version",
        ));
    }
}

fn validate_nonblank(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    code: &str,
    message: &str,
    field: &str,
    value: &str,
) {
    if value.trim().is_empty() {
        diagnostics.push(AuthoredModelDiagnostic::new(code, message, field));
    }
}

fn validate_string_list(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    values: &[String],
    invalid_code: &str,
    identifiers_only: bool,
) {
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let invalid = if identifiers_only {
            !is_valid_identifier(value)
        } else {
            value.trim().is_empty()
        };
        if invalid {
            diagnostics.push(AuthoredModelDiagnostic::new(
                invalid_code,
                if identifiers_only {
                    "Value must use the authored identifier syntax."
                } else {
                    "Value must not be empty."
                },
                &item_field,
            ));
        }
        if !seen.insert(value.as_str()) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "authored_list_value_duplicate",
                "List values must not be duplicated.",
                item_field,
            ));
        }
    }
}

fn validate_map_list(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    values: &[OrderedValueMap],
) {
    for (index, value) in values.iter().enumerate() {
        validate_ordered_map_keys(diagnostics, &format!("{field}[{index}]"), value);
    }
}

fn validate_ordered_map_keys(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    values: &OrderedValueMap,
) {
    for key in values.keys() {
        if key.trim().is_empty() {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "extension_key_invalid",
                "Extension mapping keys must not be empty.",
                field,
            ));
        }
    }
}

fn is_valid_identifier(value: &str) -> bool {
    Regex::new(r"^[a-z0-9]+(?:[._-][a-z0-9]+)*$")
        .expect("the authored identifier regex is valid")
        .is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_SOURCE: &str = include_str!("../../../authored/apps/retroarch.yaml");
    const PROFILE_SOURCE: &str =
        include_str!("../../../authored/device_profiles/ayaneo.pocket_s2.yaml");

    #[test]
    fn app_definition_canonical_emission_is_idempotent_and_complete() {
        let parsed = parse_app_definition_yaml(APP_SOURCE).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        let reparsed = parse_app_definition_yaml(&emitted).unwrap();
        assert_eq!(reparsed, parsed);
        assert_eq!(emit_app_definition_yaml(&reparsed).unwrap(), emitted);
        assert!(emitted.contains("aliases: []"));
        assert!(emitted.contains("inputs: []"));
    }

    #[test]
    fn device_profile_canonical_emission_is_idempotent_and_complete() {
        let parsed = parse_device_profile_yaml(PROFILE_SOURCE).unwrap();
        assert!(validate_device_profile(&parsed).is_empty());

        let emitted = emit_device_profile_yaml(&parsed).unwrap();
        let reparsed = parse_device_profile_yaml(&emitted).unwrap();
        assert_eq!(reparsed, parsed);
        assert_eq!(emit_device_profile_yaml(&reparsed).unwrap(), emitted);
        assert!(emitted.contains("device_tags:"));
        assert!(emitted.contains("metadata:"));
    }

    #[test]
    fn strict_models_reject_unknown_fixed_fields_and_missing_capabilities() {
        let app = APP_SOURCE.replace("category: emulator", "category: emulator\ninvented: true");
        assert_eq!(
            parse_app_definition_yaml(&app).unwrap_err().code(),
            "app_definition_yaml_invalid"
        );

        let profile = PROFILE_SOURCE.replace("  app_data_write: false\n", "");
        assert_eq!(
            parse_device_profile_yaml(&profile).unwrap_err().code(),
            "device_profile_yaml_invalid"
        );
    }

    #[test]
    fn nested_extension_values_and_order_are_preserved_losslessly() {
        let source = APP_SOURCE
            .replace(
                "    path: sample_artifacts/RetroArch_aarch64.apk",
                "    first:\n      nested: [one, {enabled: true}]\n    second: null",
            )
            .replace(
                "  config_snapshot: vendor/obtainium/apps/retroarch.json\n  app_id: retroarch",
                "  first_tracking:\n    nested: [1, 2]\n  second_tracking: false",
            )
            .replace(
                "  homepage: https://www.retroarch.com/\n  tags:\n    - emulator\n    - frontend",
                "  first_metadata:\n    nested: {enabled: true}\n  second_metadata: [one, two]",
            );
        let parsed = parse_app_definition_yaml(&source).unwrap();

        assert_eq!(
            parsed.install_source.options.keys().collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(
            parsed.tracking_source.fields.keys().collect::<Vec<_>>(),
            vec!["first_tracking", "second_tracking"]
        );
        assert_eq!(
            parsed.metadata.keys().collect::<Vec<_>>(),
            vec!["first_metadata", "second_metadata"]
        );

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), parsed);
        assert!(emitted.find("first:").unwrap() < emitted.find("second:").unwrap());
    }

    #[test]
    fn semantic_validation_reports_invalid_regex_and_android_range_in_order() {
        let mut profile = parse_device_profile_yaml(PROFILE_SOURCE).unwrap();
        profile.match_criteria.model_patterns = vec!["[".to_string()];
        profile.match_criteria.android_version = Some(AndroidVersionRange {
            min: Some(15),
            max: Some(14),
        });

        let diagnostics = validate_device_profile(&profile);
        assert_eq!(diagnostics[0].code, "device_model_pattern_invalid");
        assert_eq!(diagnostics[0].field, "match.model_patterns[0]");
        assert_eq!(diagnostics[1].code, "android_version_range_invalid");
        assert_eq!(
            emit_device_profile_yaml(&profile).unwrap_err().code(),
            "device_profile_invalid"
        );
    }

    #[test]
    fn semantic_validation_enforces_shared_ids_names_packages_and_match_criteria() {
        let mut app = parse_app_definition_yaml(APP_SOURCE).unwrap();
        app.id = "RetroArch".to_string();
        app.name = " ".to_string();
        app.package.primary = "retroarch".to_string();
        let app_codes = validate_app_definition(&app)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert_eq!(
            app_codes,
            [
                "authored_id_invalid",
                "authored_name_invalid",
                "package_name_invalid"
            ]
        );

        let mut profile = parse_device_profile_yaml(PROFILE_SOURCE).unwrap();
        profile.match_criteria = DeviceMatchCriteria::default();
        assert_eq!(
            validate_device_profile(&profile)[0].code,
            "device_match_empty"
        );
    }

    #[test]
    fn strict_app_structures_require_mappings_and_complete_artifact_fields() {
        let non_mapping_options = APP_SOURCE.replace(
            "  options:\n    path: sample_artifacts/RetroArch_aarch64.apk",
            "  options: []",
        );
        assert!(parse_app_definition_yaml(&non_mapping_options).is_err());

        let non_mapping_metadata = APP_SOURCE.replace(
            "metadata:\n  homepage: https://www.retroarch.com/\n  tags:\n    - emulator\n    - frontend",
            "metadata: []",
        );
        assert!(parse_app_definition_yaml(&non_mapping_metadata).is_err());

        let non_mapping_input = APP_SOURCE.replace("inputs: []", "inputs: [invalid]");
        assert!(parse_app_definition_yaml(&non_mapping_input).is_err());

        let incomplete_artifacts = APP_SOURCE.replace("  byo_apk:\n    required: false\n", "");
        assert!(parse_app_definition_yaml(&incomplete_artifacts).is_err());

        let invalid_target =
            APP_SOURCE.replace("  config_targets: []", "  config_targets: [invalid]");
        assert!(parse_app_definition_yaml(&invalid_target).is_err());
    }

    #[test]
    fn app_semantics_validate_sources_aliases_paths_and_extension_keys() {
        let mut app = parse_app_definition_yaml(APP_SOURCE).unwrap();
        app.category = " ".to_string();
        app.package.aliases = vec![app.package.primary.clone(), app.package.primary.clone()];
        app.install_source.type_name = " ".to_string();
        app.install_source.resolver = "".to_string();
        app.install_source
            .options
            .insert(" ".to_string(), Value::Null);
        app.tracking_source.type_name = " ".to_string();
        app.tracking_source
            .fields
            .insert("".to_string(), Value::Null);
        app.provisioning.shared_storage_paths = vec!["".to_string(), "".to_string()];
        app.inputs
            .push(IndexMap::from([("".to_string(), Value::Null)]));
        app.metadata.insert("".to_string(), Value::Null);

        let codes = validate_app_definition(&app)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert!(codes.contains(&"app_category_invalid".to_string()));
        assert!(codes.contains(&"package_alias_duplicate".to_string()));
        assert!(codes.contains(&"install_source_type_invalid".to_string()));
        assert!(codes.contains(&"install_source_resolver_invalid".to_string()));
        assert!(codes.contains(&"tracking_source_type_invalid".to_string()));
        assert!(codes.contains(&"provisioning_path_invalid".to_string()));
        assert!(codes.contains(&"authored_list_value_duplicate".to_string()));
        assert_eq!(
            codes
                .iter()
                .filter(|code| code.as_str() == "extension_key_invalid")
                .count(),
            4
        );
    }

    #[test]
    fn device_semantics_validate_match_lists_tags_and_android_bounds() {
        let mut profile = parse_device_profile_yaml(PROFILE_SOURCE).unwrap();
        profile.match_criteria.manufacturer_contains = vec!["".to_string(), "".to_string()];
        profile.match_criteria.android_version = Some(AndroidVersionRange {
            min: Some(0),
            max: Some(-1),
        });
        profile.device_tags = vec!["Invalid Tag".to_string(), "Invalid Tag".to_string()];
        profile.metadata.insert("".to_string(), Value::Null);

        let codes = validate_device_profile(&profile)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert!(codes.contains(&"device_match_value_invalid".to_string()));
        assert!(codes.contains(&"authored_list_value_duplicate".to_string()));
        assert!(codes.contains(&"android_version_min_invalid".to_string()));
        assert!(codes.contains(&"android_version_max_invalid".to_string()));
        assert!(codes.contains(&"device_tag_invalid".to_string()));
        assert!(codes.contains(&"extension_key_invalid".to_string()));

        profile.match_criteria.android_version = Some(AndroidVersionRange::default());
        assert!(validate_device_profile(&profile)
            .iter()
            .any(|diagnostic| diagnostic.code == "android_version_range_empty"));
    }

    #[test]
    fn load_errors_are_sanitized_and_stable() {
        let missing = Path::new("this-authored-model-does-not-exist.yaml");
        let app_error = load_app_definition(missing).unwrap_err();
        assert_eq!(app_error.code(), "app_definition_io_error");
        assert!(!app_error
            .message()
            .contains(&missing.to_string_lossy().to_string()));

        let profile_error = load_device_profile(missing).unwrap_err();
        assert_eq!(profile_error.code(), "device_profile_io_error");
        assert!(!profile_error
            .message()
            .contains(&missing.to_string_lossy().to_string()));
    }
}

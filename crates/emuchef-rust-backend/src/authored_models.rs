//! Typed schema-v1 models for authored app definitions and device profiles.
//!
//! These models define the structural and semantic authority shared by catalog
//! validation, product catalog loading, and future authored-data generators.
//! Fixed schema objects reject unknown fields. Deliberately extensible mappings
//! retain JSON-compatible nested values and insertion order.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use indexmap::IndexMap;
use regex::Regex;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const APP_DEFINITION_KIND: &str = "app_definition";
pub const DEVICE_PROFILE_KIND: &str = "device_profile";
pub const SCHEMA_VERSION_V1: i64 = 1;
pub const MAX_INLINE_CONFIG_LITERAL_BYTES: usize = 64 * 1024;
pub const MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT: usize = 2 * 1024 * 1024;
pub const MAX_TEMPLATE_VARIABLE_ID_BYTES: usize = 128;

fn is_false(value: &bool) -> bool {
    !*value
}

/// An insertion-ordered, string-keyed mapping of nested JSON-compatible data.
pub type OrderedValueMap = IndexMap<String, Value>;

fn deserialize_non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .map(Some)
        .ok_or_else(|| de::Error::custom("field must be omitted instead of set to null"))
}

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

/// Deserialize one insertion-ordered registry of named App Definition
/// entries.
///
/// A YAML mapping silently keeps the last value of a repeated key, which
/// would let a duplicated artifact or target id replace earlier authored
/// policy before semantic validation can see it and let canonical emission
/// drop that policy without a diagnostic. Rejecting the duplicate preserves
/// the schema rule that one id names exactly one definition.
fn deserialize_named_registry<'de, D, T>(deserializer: D) -> Result<IndexMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct RegistryVisitor<T>(std::marker::PhantomData<T>);

    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for RegistryVisitor<T> {
        type Value = IndexMap<String, T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a mapping of named entries")
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut entries: IndexMap<String, T> = IndexMap::new();
            while let Some((id, entry)) = access.next_entry::<String, T>()? {
                if entries.contains_key(&id) {
                    return Err(de::Error::custom(format!("duplicate entry id {id:?}")));
                }
                entries.insert(id, entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(RegistryVisitor(std::marker::PhantomData))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppDefinitionV1 {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub package_id: String,
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub artifacts: IndexMap<String, AppArtifactV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_sets: Option<AppPermissionSets>,
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub targets: IndexMap<String, AppTargetV1>,
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub configurations: IndexMap<String, AppConfigurationV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_activity: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub metadata: OrderedValueMap,
}

/// One named app-owned resource.
///
/// Independently selectable builds are separate named artifacts; there is no
/// nested variant contract. An `apk` artifact adds APK semantics (the
/// materialized file must be an APK whose manifest package equals the app
/// definition package identity, and it is eligible for installation), while
/// `file` is a generic app-owned resource.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppArtifactV1 {
    pub kind: AppArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: AppArtifactSource,
}

/// The accepted app artifact kinds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppArtifactKind {
    Apk,
    File,
}

/// The public release providers accepted by the latest-release strategy.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseProvider {
    Github,
    Gitlab,
    Forgejo,
}

/// How an app artifact is materialized.
///
/// Exactly three strategies exist. Each strategy accepts only its own fields,
/// so unknown keys and cross-strategy keys are rejected while parsing.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum AppArtifactSource {
    /// The bound recipe artifact input supplies the file at run time.
    UserProvided,
    /// A durable public HTTPS address with an optional trusted static checksum.
    DirectUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    /// A late-bound provider release policy resolved when the artifact is needed.
    ///
    /// `base_url` names the provider service origin, so it carries no web path.
    /// `invert_asset_pattern` is meaningful only together with
    /// `asset_pattern`: `true` is persisted explicitly, while `false` is the
    /// default behavior and is never written.
    LatestRelease {
        provider: ReleaseProvider,
        base_url: String,
        repository: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asset_pattern: Option<String>,
        #[serde(default, skip_serializing_if = "invert_asset_pattern_is_omitted")]
        invert_asset_pattern: Option<bool>,
        prerelease: bool,
    },
}

/// Whether canonical emission omits an authored inversion flag.
///
/// Only an explicit `true` carries information. A `false` flag states the
/// default behavior of forward pattern matching, so it is never written, and an
/// absent flag is omitted as well.
fn invert_asset_pattern_is_omitted(value: &Option<bool>) -> bool {
    !matches!(value, Some(true))
}

/// Strict deserialization for artifact sources.
///
/// Serde's derived implementation for internally tagged enums ignores keys that
/// belong to another strategy, which would let a mixed source document parse as
/// a valid one. Deserializing the flat union of every accepted field first lets
/// each key be checked against the declared strategy before the typed source is
/// built, so unknown keys and cross-strategy keys are rejected deterministically.
/// Deserialize one raw artifact-source field while remembering whether the
/// document supplied it.
///
/// An absent key keeps the outer option empty, while a supplied key, including
/// an explicit `null`, always produces `Some`.
fn deserialize_supplied<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for AppArtifactSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// The declared artifact source strategy.
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Strategy {
            UserProvided,
            DirectUrl,
            LatestRelease,
        }

        /// The flat union of every field any strategy accepts.
        ///
        /// Each field is wrapped in an outer option that records whether the
        /// document supplied the key at all, because a plain option treats an
        /// explicit `null` exactly like an absent key. Presence is what the
        /// per-strategy rejection checks below need: a supplied field of any
        /// value, including `null`, belongs to a different strategy and must
        /// fail parsing rather than be dropped during canonical emission.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawSource {
            strategy: Strategy,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            url: Option<Option<String>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            sha256: Option<Option<String>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            provider: Option<Option<ReleaseProvider>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            base_url: Option<Option<String>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            repository: Option<Option<String>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            asset_pattern: Option<Option<String>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            invert_asset_pattern: Option<Option<bool>>,
            #[serde(default, deserialize_with = "deserialize_supplied")]
            prerelease: Option<Option<bool>>,
        }

        fn required<E: de::Error>(value: Option<String>, name: &str) -> Result<String, E> {
            value.ok_or_else(|| E::custom(format!("the {name} field is required")))
        }

        fn reject<E: de::Error>(present: bool, name: &str, strategy: &str) -> Result<(), E> {
            if present {
                Err(E::custom(format!(
                    "the {strategy} strategy does not accept the {name} field"
                )))
            } else {
                Ok(())
            }
        }

        let raw = RawSource::deserialize(deserializer)?;
        match raw.strategy {
            Strategy::UserProvided => {
                for (present, name) in [
                    (raw.url.is_some(), "url"),
                    (raw.sha256.is_some(), "sha256"),
                    (raw.provider.is_some(), "provider"),
                    (raw.base_url.is_some(), "base_url"),
                    (raw.repository.is_some(), "repository"),
                    (raw.asset_pattern.is_some(), "asset_pattern"),
                    (raw.invert_asset_pattern.is_some(), "invert_asset_pattern"),
                    (raw.prerelease.is_some(), "prerelease"),
                ] {
                    reject::<D::Error>(present, name, "user_provided")?;
                }
                Ok(AppArtifactSource::UserProvided)
            }
            Strategy::DirectUrl => {
                for (present, name) in [
                    (raw.provider.is_some(), "provider"),
                    (raw.base_url.is_some(), "base_url"),
                    (raw.repository.is_some(), "repository"),
                    (raw.asset_pattern.is_some(), "asset_pattern"),
                    (raw.invert_asset_pattern.is_some(), "invert_asset_pattern"),
                    (raw.prerelease.is_some(), "prerelease"),
                ] {
                    reject::<D::Error>(present, name, "direct_url")?;
                }
                Ok(AppArtifactSource::DirectUrl {
                    url: required::<D::Error>(raw.url.flatten(), "url")?,
                    sha256: raw.sha256.flatten(),
                })
            }
            Strategy::LatestRelease => {
                for (present, name) in
                    [(raw.url.is_some(), "url"), (raw.sha256.is_some(), "sha256")]
                {
                    reject::<D::Error>(present, name, "latest_release")?;
                }
                // An authored false flag states the default behavior once a
                // pattern exists, so it is normalized away here and never
                // reaches canonical emission. Without a pattern the authored
                // field is retained so validation can reject the contradiction.
                let asset_pattern = raw.asset_pattern.flatten();
                let invert_asset_pattern = match raw.invert_asset_pattern.flatten() {
                    Some(false) if asset_pattern.is_some() => None,
                    authored => authored,
                };
                Ok(AppArtifactSource::LatestRelease {
                    provider: raw
                        .provider
                        .flatten()
                        .ok_or_else(|| de::Error::custom("the provider field is required"))?,
                    base_url: required::<D::Error>(raw.base_url.flatten(), "base_url")?,
                    repository: required::<D::Error>(raw.repository.flatten(), "repository")?,
                    asset_pattern,
                    invert_asset_pattern,
                    prerelease: raw
                        .prerelease
                        .flatten()
                        .ok_or_else(|| de::Error::custom("the prerelease field is required"))?,
                })
            }
        }
    }
}

/// The standardized optional permission automation sets.
///
/// Exactly two sets exist: `baseline` for normal and non-root automation, and
/// `elevated` for automation that requires root. Either set may be absent, a
/// present set must contain at least one action, and any other set name is
/// rejected as an unknown field.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppPermissionSets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<AppPermissionSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevated: Option<AppPermissionSet>,
}

/// One permission set. A present set must contain at least one action.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppPermissionSet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<AppRuntimePermissionAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_ops: Vec<AppOpAction>,
}

/// One runtime-permission automation action.
///
/// Omitting both API bounds means the automation applies at every API level.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppRuntimePermissionAction {
    pub permission: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_api_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_api_max: Option<i64>,
}

/// One app-op automation action with an explicit mode.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppOpAction {
    pub op: String,
    pub mode: AppOpMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_api_min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_api_max: Option<i64>,
}

/// The only app-op automation mode accepted in schema v1.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppOpMode {
    Allow,
}

/// One named app-owned semantic destination on a target device.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppTargetV1 {
    pub kind: AppTargetKind,
    pub location: AppTargetLocation,
    pub path: String,
}

/// The accepted app target kinds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppTargetKind {
    File,
    Directory,
}

/// The accepted semantic app target locations.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppTargetLocation {
    AppData,
    ExternalAppData,
    SharedStorage,
    AbsoluteDevicePath,
}

/// A supported format for one app-owned configuration document.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum AppConfigurationFormatV1 {
    #[serde(rename = "key_value/v1")]
    KeyValueV1,
    #[serde(rename = "json/v1")]
    JsonV1,
}

impl AppConfigurationFormatV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KeyValueV1 => "key_value/v1",
            Self::JsonV1 => "json/v1",
        }
    }
}

/// Scalar value kinds a configuration template may substitute.
///
/// `DevicePath` belongs to template inputs only. It is not an App Definition
/// managed-property type and does not expand the set of values the app owns.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateVariableTypeV1 {
    String,
    Integer,
    Number,
    Boolean,
    DevicePath,
}

impl TemplateVariableTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::DevicePath => "device_path",
        }
    }

    pub fn matches(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value
                .as_number()
                .is_some_and(|number| number.is_i64() || number.is_u64()),
            Self::Number => value
                .as_number()
                .and_then(serde_json::Number::as_f64)
                .is_some_and(f64::is_finite),
            Self::Boolean => value.is_boolean(),
            Self::DevicePath => value.as_str().is_some_and(is_valid_device_path),
        }
    }

    /// Return whether an existing scalar Recipe input can supply this type.
    pub fn accepts_input_type(self, input_type: &str) -> bool {
        match self {
            Self::String => matches!(input_type, "string" | "enum"),
            Self::Integer => input_type == "integer",
            Self::Number => matches!(input_type, "integer" | "number"),
            Self::Boolean => input_type == "boolean",
            Self::DevicePath => matches!(input_type, "device_path" | "setup_location_ref"),
        }
    }
}

/// Value kinds accepted by managed properties and unmanaged template literals.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValueTypeV1 {
    String,
    Integer,
    Number,
    Boolean,
}

impl ConfigValueTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }

    pub fn matches(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value
                .as_number()
                .is_some_and(|number| number.is_i64() || number.is_u64()),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
        }
    }
}

/// Scalar and structured value types permitted for explicit unmanaged literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmanagedTemplateLiteralTypeV1 {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
}

impl UnmanagedTemplateLiteralTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }

    pub fn matches(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value
                .as_number()
                .is_some_and(|number| number.is_i64() || number.is_u64()),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
        }
    }
}

/// One configuration document owned by an App Definition.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfigurationV1 {
    pub format: AppConfigurationFormatV1,
    pub target: String,
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub templates: IndexMap<String, AppConfigurationTemplateV1>,
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub managed_properties: IndexMap<String, AppManagedPropertyV1>,
}

/// A reusable configuration file scoped to its owning App Definition.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfigurationTemplateV1 {
    /// Path relative to `authored/apps`, under the directory named by the app ID.
    pub path: String,
    /// Template-local typed declarations used only by whole-value placeholders.
    #[serde(
        default,
        skip_serializing_if = "IndexMap::is_empty",
        deserialize_with = "deserialize_named_registry"
    )]
    pub variables: IndexMap<String, TemplateVariableTypeV1>,
}

/// App-owned identity, location, type and constraints for one managed setting.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppManagedPropertyV1 {
    /// Format-specific sequence of literal property-name segments.
    pub path: Vec<String>,
    #[serde(rename = "type")]
    pub type_name: ConfigValueTypeV1,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clearable: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_null_option"
    )]
    pub default: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "AppManagedPropertyValidatorsV1::is_empty"
    )]
    pub validators: AppManagedPropertyValidatorsV1,
}

/// Constraints applied to defaults and Recipe values for a managed property.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppManagedPropertyValidatorsV1 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_null_option"
    )]
    pub minimum: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_null_option"
    )]
    pub maximum: Option<Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_null_option"
    )]
    pub min_length: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_null_option"
    )]
    pub max_length: Option<usize>,
}

impl AppManagedPropertyValidatorsV1 {
    fn is_empty(&self) -> bool {
        self.allowed_values.is_empty()
            && self.minimum.is_none()
            && self.maximum.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
    }
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
    let invalid_yaml = || {
        AuthoredModelError::new(
            "app_definition_yaml_invalid",
            "The app definition is not valid schema-v1 YAML.",
        )
    };
    crate::yaml::validate_unique_yaml_keys(source).map_err(|_| invalid_yaml())?;
    let value: serde_yaml::Value = serde_yaml::from_str(source).map_err(|_| invalid_yaml())?;
    if !app_configuration_paths_have_string_segments(&value) {
        return Err(invalid_yaml());
    }
    serde_yaml::from_value(value).map_err(|_| invalid_yaml())
}

// Check raw YAML node types before serde can coerce scalar values into strings.
fn app_configuration_paths_have_string_segments(value: &serde_yaml::Value) -> bool {
    let Some(configurations) = value
        .get("configurations")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return true;
    };
    configurations.values().all(|configuration| {
        let Some(properties) = configuration
            .get("managed_properties")
            .and_then(serde_yaml::Value::as_mapping)
        else {
            return true;
        };
        properties.values().all(|property| {
            let Some(path) = property.get("path") else {
                return true;
            };
            path.as_sequence()
                .is_some_and(|segments| segments.iter().all(serde_yaml::Value::is_string))
        })
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
    validate_optional_nonblank(
        &mut diagnostics,
        "authored_description_invalid",
        "Description must not be empty when present.",
        "description",
        value.description.as_deref(),
    );
    validate_optional_nonblank(
        &mut diagnostics,
        "app_category_invalid",
        "App category must not be empty when present.",
        "category",
        value.category.as_deref(),
    );
    validate_package_id(&mut diagnostics, &value.package_id);
    validate_artifacts(&mut diagnostics, &value.artifacts);
    validate_permission_sets(&mut diagnostics, value.permission_sets.as_ref());
    validate_targets(&mut diagnostics, &value.targets);
    validate_configurations(
        &mut diagnostics,
        &value.id,
        &value.targets,
        &value.configurations,
    );
    validate_launcher_activity(&mut diagnostics, value.launcher_activity.as_deref());
    validate_metadata(&mut diagnostics, &value.metadata);
    diagnostics
}

fn validate_configurations(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    app_id: &str,
    targets: &IndexMap<String, AppTargetV1>,
    configurations: &IndexMap<String, AppConfigurationV1>,
) {
    for (configuration_id, configuration) in configurations {
        let mut total_literal_bytes = 0usize;
        let field = format!("configurations[{configuration_id}]");
        if !is_valid_identifier(configuration_id) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_configuration_id_invalid",
                "Configuration IDs must contain lowercase alphanumeric segments separated by '.', '_', or '-'.",
                &field,
            ));
        }
        match targets.get(&configuration.target) {
            Some(target) if target.kind == AppTargetKind::File => {}
            _ => diagnostics.push(AuthoredModelDiagnostic::new(
                "app_configuration_target_invalid",
                "A configuration document must reference a declared file App Target.",
                format!("{field}.target"),
            )),
        }

        for (template_id, template) in &configuration.templates {
            let template_field = format!("{field}.templates[{template_id}]");
            if !is_valid_identifier(template_id)
                || !is_valid_app_configuration_template_path(app_id, &template.path)
            {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_configuration_template_path_invalid",
                    "Template paths must be relative slash-separated files under the owning app ID directory in authored/apps.",
                    format!("{template_field}.path"),
                ));
            }
            for variable_id in template.variables.keys() {
                if !is_valid_template_variable_id(variable_id) {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_template_variable_id_invalid",
                        "Template variable IDs must use the authored identifier grammar and contain no more than 128 ASCII characters.",
                        format!("{template_field}.variables[{variable_id}]"),
                    ));
                }
            }
        }

        let mut paths: Vec<(String, Vec<String>)> = Vec::new();
        for (property_id, property) in &configuration.managed_properties {
            let property_field = format!("{field}.managed_properties[{property_id}]");
            if !is_valid_identifier(property_id) {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_config_property_id_invalid",
                    "Managed property IDs must contain lowercase alphanumeric segments separated by '.', '_', or '-'.",
                    &property_field,
                ));
            }
            match configuration_property_path_tokens(configuration.format, &property.path) {
                Some(tokens) => {
                    for (previous_id, previous_tokens) in &paths {
                        if path_tokens_overlap(previous_tokens, &tokens) {
                            diagnostics.push(AuthoredModelDiagnostic::new(
                                "app_config_property_path_overlap",
                                format!("Managed property path overlaps property {previous_id:?}."),
                                format!("{property_field}.path"),
                            ));
                        }
                    }
                    paths.push((property_id.clone(), tokens));
                }
                None => diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_config_property_path_invalid",
                    "Managed property path is invalid for the declared configuration format.",
                    format!("{property_field}.path"),
                )),
            }

            if let Some(default) = &property.default {
                record_config_literal(
                    diagnostics,
                    &format!("{property_field}.default"),
                    default,
                    &mut total_literal_bytes,
                    "app_config_literal_too_large",
                );
                if !managed_property_value_is_valid(property, default) {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_default_invalid",
                        "Managed property default must match its declared type and validators.",
                        format!("{property_field}.default"),
                    ));
                }
            }

            let validators = &property.validators;
            for (index, allowed) in validators.allowed_values.iter().enumerate() {
                record_config_literal(
                    diagnostics,
                    &format!("{property_field}.validators.allowed_values[{index}]"),
                    allowed,
                    &mut total_literal_bytes,
                    "app_config_literal_too_large",
                );
                if !property.type_name.matches(allowed)
                    || validators.allowed_values[..index].iter().any(|previous| {
                        managed_property_values_equal(property.type_name, previous, allowed)
                            .is_none_or(|equal| equal)
                    })
                    || !managed_property_value_is_valid(property, allowed)
                {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_validator_invalid",
                    "Each allowed value must be unique, non-null, match the managed property type, and satisfy its minimum, maximum, min_length, and max_length constraints.",
                        format!("{property_field}.validators.allowed_values[{index}]"),
                    ));
                }
            }
            for (bound_name, bound) in [
                ("minimum", validators.minimum.as_ref()),
                ("maximum", validators.maximum.as_ref()),
            ] {
                if let Some(bound) = bound {
                    record_config_literal(
                        diagnostics,
                        &format!("{property_field}.validators.{bound_name}"),
                        bound,
                        &mut total_literal_bytes,
                        "app_config_literal_too_large",
                    );
                    let bound_matches_type =
                        managed_numeric_bound_matches_type(property.type_name, bound);
                    if !bound_matches_type {
                        diagnostics.push(AuthoredModelDiagnostic::new(
                            "app_config_validator_invalid",
                            "Numeric bounds require a numeric managed property and numeric values.",
                            format!("{property_field}.validators.{bound_name}"),
                        ));
                    }
                }
            }
            if let (Some(minimum), Some(maximum)) =
                (validators.minimum.as_ref(), validators.maximum.as_ref())
            {
                let bounds_match_type =
                    managed_numeric_bound_matches_type(property.type_name, minimum)
                        && managed_numeric_bound_matches_type(property.type_name, maximum);
                if bounds_match_type
                    && compare_managed_numeric_values(property.type_name, minimum, maximum)
                        .is_none_or(|ordering| ordering == Ordering::Greater)
                {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_validator_invalid",
                        "Minimum must not exceed maximum.",
                        format!("{property_field}.validators"),
                    ));
                }
            }
            if validators.min_length.is_some() || validators.max_length.is_some() {
                if property.type_name != ConfigValueTypeV1::String {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_validator_invalid",
                        "min_length and max_length are supported only for string properties.",
                        format!("{property_field}.validators"),
                    ));
                }
                if matches!((validators.min_length, validators.max_length),
                    (Some(minimum), Some(maximum)) if minimum > maximum)
                {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_config_validator_invalid",
                        "min_length must not exceed max_length.",
                        format!("{property_field}.validators"),
                    ));
                }
            }
        }
        if total_literal_bytes > MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_config_literal_budget_exceeded",
                "Inline configuration literals in one configuration document must total no more than 2 MiB.",
                &field,
            ));
        }
    }
}

/// Return path tokens for one supported configuration format.
pub fn configuration_property_path_tokens(
    format: AppConfigurationFormatV1,
    path: &[String],
) -> Option<Vec<String>> {
    if path.is_empty()
        || path
            .iter()
            .any(|segment| segment.is_empty() || segment.chars().any(char::is_control))
    {
        return None;
    }
    match format {
        AppConfigurationFormatV1::KeyValueV1 => {
            let [key] = path else {
                return None;
            };
            if key.trim() != key
                || key
                    .chars()
                    .any(|character| matches!(character, '=' | '/' | '\\'))
            {
                return None;
            }
            Some(path.to_vec())
        }
        // JSON paths identify object members by literal key segments. Array
        // indexing and pointer escape decoding are outside this schema.
        AppConfigurationFormatV1::JsonV1 => Some(path.to_vec()),
    }
}

/// Check whether two format-specific property paths are equal or nested.
pub fn configuration_paths_overlap(
    format: AppConfigurationFormatV1,
    left: &[String],
    right: &[String],
) -> bool {
    configuration_property_path_tokens(format, left)
        .zip(configuration_property_path_tokens(format, right))
        .is_some_and(|(left, right)| path_tokens_overlap(&left, &right))
}

/// Check the lexical ownership boundary for an App Definition template pointer.
pub fn is_valid_app_configuration_template_path(app_id: &str, path: &str) -> bool {
    if path.contains('\\') || path.contains(':') || path.chars().any(char::is_control) {
        return false;
    }
    let components = path.split('/').collect::<Vec<_>>();
    components.len() >= 2
        && components.first() == Some(&app_id)
        && components
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..")
}

/// Compare integer JSON numbers without converting them to floating point.
fn compare_integer_values(left: &Value, right: &Value) -> Option<Ordering> {
    let left = left.as_number()?;
    let right = right.as_number()?;
    match (left.as_i64(), left.as_u64(), right.as_i64(), right.as_u64()) {
        (Some(left), _, Some(right), _) => Some(left.cmp(&right)),
        (Some(left), _, None, Some(right)) if left >= 0 => Some((left as u64).cmp(&right)),
        (Some(_), _, None, Some(_)) => Some(Ordering::Less),
        (None, Some(left), Some(right), _) if right >= 0 => Some(left.cmp(&(right as u64))),
        (None, Some(_), Some(_), _) => Some(Ordering::Greater),
        (None, Some(left), None, Some(right)) => Some(left.cmp(&right)),
        _ => None,
    }
}

/// Compare JSON numbers without converting integers to floating point.
fn compare_number_values(left: &Value, right: &Value) -> Option<Ordering> {
    let left_number = left.as_number()?;
    let right_number = right.as_number()?;
    let left_is_float = left_number.is_f64();
    let right_is_float = right_number.is_f64();

    match (left_is_float, right_is_float) {
        (false, false) => compare_integer_values(left, right),
        (false, true) => compare_integer_and_float(left_number, right_number),
        (true, false) => {
            compare_integer_and_float(right_number, left_number).map(|ordering| ordering.reverse())
        }
        (true, true) => left_number.as_f64()?.partial_cmp(&right_number.as_f64()?),
    }
}

/// Compare an unsigned integer with a finite, nonnegative `f64` exactly.
fn compare_unsigned_integer_to_nonnegative_float(integer: u64, float: f64) -> Ordering {
    let bits = float.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    let (significand, binary_exponent) = if exponent_bits == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1_u64 << 52), exponent_bits - 1023 - 52)
    };

    if significand == 0 {
        return integer.cmp(&0);
    }

    if binary_exponent >= 0 {
        let significand_bits = (u64::BITS - significand.leading_zeros()) as i32;
        if significand_bits + binary_exponent > u64::BITS as i32 {
            return Ordering::Less;
        }
        let float_integer = (significand as u128) << binary_exponent as u32;
        return integer.cmp(&(float_integer as u64));
    }

    let fractional_bits = (-binary_exponent) as u32;
    if fractional_bits >= u64::BITS {
        return if integer == 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    let integral_part = significand >> fractional_bits;
    match integer.cmp(&integral_part) {
        Ordering::Equal if significand & ((1_u64 << fractional_bits) - 1) != 0 => Ordering::Less,
        ordering => ordering,
    }
}

fn compare_signed_integer_to_float(integer: i64, float: f64) -> Ordering {
    if integer < 0 {
        if float >= 0.0 {
            Ordering::Less
        } else {
            compare_unsigned_integer_to_nonnegative_float(integer.unsigned_abs(), -float).reverse()
        }
    } else if float < 0.0 {
        Ordering::Greater
    } else {
        compare_unsigned_integer_to_nonnegative_float(integer as u64, float)
    }
}

fn compare_unsigned_integer_to_float(integer: u64, float: f64) -> Ordering {
    if float < 0.0 {
        Ordering::Greater
    } else {
        compare_unsigned_integer_to_nonnegative_float(integer, float)
    }
}

fn compare_integer_and_float(
    integer: &serde_json::Number,
    float: &serde_json::Number,
) -> Option<Ordering> {
    let float = float.as_f64()?;
    if !float.is_finite() {
        return None;
    }

    if let Some(integer) = integer.as_i64() {
        Some(compare_signed_integer_to_float(integer, float))
    } else {
        Some(compare_unsigned_integer_to_float(integer.as_u64()?, float))
    }
}

fn compare_managed_numeric_values(
    type_name: ConfigValueTypeV1,
    left: &Value,
    right: &Value,
) -> Option<Ordering> {
    match type_name {
        ConfigValueTypeV1::Integer => compare_integer_values(left, right),
        ConfigValueTypeV1::Number => compare_number_values(left, right),
        _ => None,
    }
}

fn managed_numeric_bound_matches_type(type_name: ConfigValueTypeV1, bound: &Value) -> bool {
    match type_name {
        ConfigValueTypeV1::Integer => bound
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64()),
        ConfigValueTypeV1::Number => bound.is_number(),
        _ => false,
    }
}

fn managed_property_values_equal(
    type_name: ConfigValueTypeV1,
    left: &Value,
    right: &Value,
) -> Option<bool> {
    match type_name {
        ConfigValueTypeV1::Integer | ConfigValueTypeV1::Number => {
            compare_managed_numeric_values(type_name, left, right)
                .map(|ordering| ordering == Ordering::Equal)
        }
        _ => Some(left == right),
    }
}

/// Check that a managed value matches its declared type and constraints.
pub fn managed_property_value_is_valid(property: &AppManagedPropertyV1, value: &Value) -> bool {
    if value.is_null() || !property.type_name.matches(value) {
        return false;
    }
    let validators = &property.validators;
    if !validators.allowed_values.is_empty()
        && !validators.allowed_values.iter().any(|allowed| {
            managed_property_values_equal(property.type_name, value, allowed) == Some(true)
        })
    {
        return false;
    }
    if let Some(minimum) = &validators.minimum {
        if compare_managed_numeric_values(property.type_name, value, minimum)
            .is_none_or(|ordering| ordering == Ordering::Less)
        {
            return false;
        }
    }
    if let Some(maximum) = &validators.maximum {
        if compare_managed_numeric_values(property.type_name, value, maximum)
            .is_none_or(|ordering| ordering == Ordering::Greater)
        {
            return false;
        }
    }
    if let Some(min_length) = validators.min_length {
        if value
            .as_str()
            .map_or(true, |value| value.chars().count() < min_length)
        {
            return false;
        }
    }
    if let Some(max_length) = validators.max_length {
        if value
            .as_str()
            .map_or(true, |value| value.chars().count() > max_length)
        {
            return false;
        }
    }
    true
}

pub(crate) fn inline_config_literal_size(value: &Value) -> usize {
    value.as_str().map_or_else(
        || serde_json::to_vec(value).map_or(usize::MAX, |encoded| encoded.len()),
        str::len,
    )
}

fn record_config_literal(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    value: &Value,
    total_bytes: &mut usize,
    code: &str,
) {
    let size = inline_config_literal_size(value);
    *total_bytes = total_bytes.saturating_add(size);
    if size > MAX_INLINE_CONFIG_LITERAL_BYTES {
        diagnostics.push(AuthoredModelDiagnostic::new(
            code,
            "Each inline configuration literal must be no larger than 64 KiB.",
            field,
        ));
    }
}

fn path_tokens_overlap(left: &[String], right: &[String]) -> bool {
    let common = left.len().min(right.len());
    left[..common] == right[..common]
}

/// Metadata keys that may not appear as direct keys because they would shadow
/// the app definition schema or reserved inspection evidence.
const RESERVED_METADATA_KEYS: &[&str] = &[
    "schema_version",
    "kind",
    "id",
    "name",
    "description",
    "category",
    "package_id",
    "artifacts",
    "permission_sets",
    "targets",
    "configurations",
    "launcher_activity",
    "metadata",
    "apk_inspection",
];

fn validate_metadata(diagnostics: &mut Vec<AuthoredModelDiagnostic>, values: &OrderedValueMap) {
    validate_ordered_map_keys(diagnostics, "metadata", values);
    for key in values.keys() {
        if RESERVED_METADATA_KEYS.contains(&key.as_str()) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "metadata_reserved_key",
                format!("Metadata key is reserved by the app definition schema: {key}"),
                "metadata",
            ));
        }
    }
}

fn validate_artifacts(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    artifacts: &IndexMap<String, AppArtifactV1>,
) {
    for (artifact_id, artifact) in artifacts {
        let field = format!("artifacts[{}]", artifact_id);
        if !is_valid_identifier(artifact_id) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_artifact_id_invalid",
                "Artifact IDs must contain lowercase alphanumeric segments separated by '.', '_', or '-'.",
                &field,
            ));
        }
        validate_optional_nonblank(
            diagnostics,
            "app_artifact_text_invalid",
            "Artifact names must not be empty when present.",
            &format!("{}.name", field),
            artifact.name.as_deref(),
        );
        validate_optional_nonblank(
            diagnostics,
            "app_artifact_text_invalid",
            "Artifact descriptions must not be empty when present.",
            &format!("{}.description", field),
            artifact.description.as_deref(),
        );
        validate_artifact_source(diagnostics, &field, &artifact.source);
    }
}

fn validate_artifact_source(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    source: &AppArtifactSource,
) {
    match source {
        AppArtifactSource::UserProvided => {}
        AppArtifactSource::DirectUrl { url, sha256 } => {
            if !is_public_https_url(url) {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_artifact_url_invalid",
                    "Direct URL artifact sources require a public HTTPS address without credentials or a fragment.",
                    format!("{}.source.url", field),
                ));
            }
            if let Some(sha256) = sha256 {
                if !is_lowercase_sha256(sha256) {
                    diagnostics.push(AuthoredModelDiagnostic::new(
                        "app_artifact_sha256_invalid",
                        "The artifact SHA-256 must contain exactly 64 lowercase hexadecimal characters.",
                        format!("{}.source.sha256", field),
                    ));
                }
            }
        }
        AppArtifactSource::LatestRelease {
            provider,
            base_url,
            repository,
            asset_pattern,
            invert_asset_pattern,
            ..
        } => {
            if !is_service_origin(base_url) {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_artifact_base_url_invalid",
                    "Latest-release sources require an HTTPS service origin: no credentials, query, fragment, or path.",
                    format!("{}.source.base_url", field),
                ));
            }
            if !is_valid_repository(*provider, repository) {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_artifact_repository_invalid",
                    "The release repository must name a valid owner and project for the selected provider.",
                    format!("{}.source.repository", field),
                ));
            }
            match asset_pattern {
                Some(pattern) => {
                    if pattern.trim().is_empty() || Regex::new(pattern).is_err() {
                        diagnostics.push(AuthoredModelDiagnostic::new(
                            "app_artifact_asset_pattern_invalid",
                            "The asset pattern must be a non-empty regular expression when present.",
                            format!("{}.source.asset_pattern", field),
                        ));
                    }
                }
                None => {
                    if invert_asset_pattern.is_some() {
                        diagnostics.push(AuthoredModelDiagnostic::new(
                            "app_artifact_invert_without_pattern",
                            "Inverting asset-pattern matches requires an asset pattern.",
                            format!("{}.source.invert_asset_pattern", field),
                        ));
                    }
                }
            }
        }
    }
}

fn validate_permission_sets(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    sets: Option<&AppPermissionSets>,
) {
    let Some(sets) = sets else {
        return;
    };
    if sets.baseline.is_none() && sets.elevated.is_none() {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "app_permission_set_empty",
            "Present permission sets must contain at least one named set.",
            "permission_sets",
        ));
    }
    let mut seen = HashSet::new();
    let named_sets = [
        ("baseline", sets.baseline.as_ref()),
        ("elevated", sets.elevated.as_ref()),
    ];
    for (set_name, set) in named_sets {
        let Some(set) = set else {
            continue;
        };
        let set_field = format!("permission_sets.{}", set_name);
        if set.runtime.is_empty() && set.app_ops.is_empty() {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_permission_set_empty",
                "A present permission set must contain at least one action.",
                &set_field,
            ));
        }
        for (index, action) in set.runtime.iter().enumerate() {
            let field = format!("{}.runtime[{}]", set_field, index);
            validate_nonblank(
                diagnostics,
                "app_permission_action_invalid",
                "Permission names must not be empty.",
                &format!("{}.permission", field),
                &action.permission,
            );
            validate_api_bounds(
                diagnostics,
                &field,
                action.android_api_min,
                action.android_api_max,
            );
            if !action.permission.trim().is_empty()
                && !seen.insert(format!("runtime:{}", action.permission))
            {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_permission_action_duplicate",
                    "Permission automation actions must be unique within and across permission sets.",
                    &field,
                ));
            }
        }
        for (index, action) in set.app_ops.iter().enumerate() {
            let field = format!("{}.app_ops[{}]", set_field, index);
            validate_nonblank(
                diagnostics,
                "app_permission_action_invalid",
                "App-op names must not be empty.",
                &format!("{}.op", field),
                &action.op,
            );
            validate_api_bounds(
                diagnostics,
                &field,
                action.android_api_min,
                action.android_api_max,
            );
            if !action.op.trim().is_empty() && !seen.insert(format!("app_op:{}", action.op)) {
                diagnostics.push(AuthoredModelDiagnostic::new(
                    "app_permission_action_duplicate",
                    "Permission automation actions must be unique within and across permission sets.",
                    &field,
                ));
            }
        }
    }
}

/// Validate one action's optional Android API bounds.
///
/// Bounds must be positive, and a present minimum must not exceed a present
/// maximum. Omitted bounds mean the action applies at every API level.
fn validate_api_bounds(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    field: &str,
    minimum: Option<i64>,
    maximum: Option<i64>,
) {
    let invalid_minimum = minimum.is_some_and(|value| value < 1)
        || matches!((minimum, maximum), (Some(minimum), Some(maximum)) if minimum > maximum);
    if invalid_minimum {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "app_permission_api_bounds_invalid",
            "Android API bounds must be positive, and the minimum must not exceed the maximum.",
            format!("{}.android_api_min", field),
        ));
    }
    if maximum.is_some_and(|value| value < 1) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "app_permission_api_bounds_invalid",
            "Android API bounds must be positive, and the minimum must not exceed the maximum.",
            format!("{}.android_api_max", field),
        ));
    }
}

fn validate_targets(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    targets: &IndexMap<String, AppTargetV1>,
) {
    for (target_id, target) in targets {
        let field = format!("targets[{}]", target_id);
        if !is_valid_identifier(target_id) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_target_id_invalid",
                "Target IDs must contain lowercase alphanumeric segments separated by '.', '_', or '-'.",
                &field,
            ));
        }
        if !is_valid_target_path(target.location, &target.path) {
            diagnostics.push(AuthoredModelDiagnostic::new(
                "app_target_path_invalid",
                "App target paths must be literal slash-separated paths in the form the target location requires.",
                format!("{}.path", field),
            ));
        }
    }
}

fn validate_launcher_activity(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    launcher_activity: Option<&str>,
) {
    let Some(launcher_activity) = launcher_activity else {
        return;
    };
    if !is_valid_concrete_launcher_activity(launcher_activity) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "launcher_activity_invalid",
            "The launcher activity must be one fully-qualified activity class name.",
            "launcher_activity",
        ));
    }
}

/// Check the fully qualified class syntax accepted for an App Definition
/// launcher and for the concrete activity sealed into an execution plan.
pub(crate) fn is_valid_concrete_launcher_activity(activity: &str) -> bool {
    // A nested Android class keeps the `$` separator of its binary name.
    let pattern = Regex::new(r"^[A-Za-z_][A-Za-z0-9_$]*(\.[A-Za-z_][A-Za-z0-9_$]*)+$")
        .expect("the launcher-activity regex is valid");
    pattern.is_match(activity)
}

fn validate_optional_nonblank(
    diagnostics: &mut Vec<AuthoredModelDiagnostic>,
    code: &str,
    message: &str,
    field: &str,
    value: Option<&str>,
) {
    if value.is_some_and(|value| value.trim().is_empty()) {
        diagnostics.push(AuthoredModelDiagnostic::new(code, message, field));
    }
}

/// Return whether a URL is a public HTTPS address accepted by artifact sources.
///
/// Credentials and fragments are always rejected. Query parameters stay legal,
/// because a direct artifact address may legitimately carry them.
fn is_public_https_url(value: &str) -> bool {
    parse_public_https_url(value).is_some()
}

/// Whether a value is a public HTTPS service origin rather than a service path.
///
/// A latest-release `base_url` names the provider service itself, so it accepts
/// only the scheme, the host, and an optional default port. Query parameters,
/// fragments, credentials, and nested web or API paths are all rejected.
pub(crate) fn is_service_origin(value: &str) -> bool {
    parse_public_https_url(value)
        .is_some_and(|parsed| parsed.query().is_none() && matches!(parsed.path(), "" | "/"))
}

/// Parse a public HTTPS address, rejecting credentials, fragments, and hosts
/// that only exist on the local machine.
///
/// An authored App Definition describes a distributable public source, so it
/// rejects the same machine-local hosts the source-selection and download paths
/// already refuse instead of persisting an address those paths cannot use.
pub(crate) fn parse_public_https_url(value: &str) -> Option<url::Url> {
    let Ok(parsed) = url::Url::parse(value) else {
        return None;
    };
    let host = parsed.host()?;
    let is_public = parsed.scheme() == "https"
        && !host_is_non_public(&host)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none();
    is_public.then_some(parsed)
}

/// Return whether a URL host is local or uses a non-public address range.
///
/// A trailing DNS root dot and IPv4-mapped IPv6 address retain the same host
/// policy as their plain forms. Hostname addresses are checked again when each
/// direct HTTPS connection is established.
fn host_is_non_public(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(domain) => {
            let domain = domain.trim().trim_end_matches('.');
            domain.is_empty()
                || domain.eq_ignore_ascii_case("localhost")
                || domain.ends_with(".localhost")
        }
        url::Host::Ipv4(address) => !is_public_ip(std::net::IpAddr::V4(*address)),
        url::Host::Ipv6(address) => !is_public_ip(std::net::IpAddr::V6(*address)),
    }
}

/// Return whether an IP address is globally reachable under the direct-URL
/// source policy. Hostnames are checked against their resolved addresses by the
/// HTTP transport before each connection.
pub(crate) fn is_public_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => !ipv4_is_non_public(address),
        std::net::IpAddr::V6(address) => {
            let segments = address.segments();
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.to_ipv4_mapped().is_some_and(ipv4_is_non_public)
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0))
        }
    }
}

/// Return whether an IPv4 address is private, local, shared, or non-unicast.
fn ipv4_is_non_public(address: std::net::Ipv4Addr) -> bool {
    let [first, second, _, _] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || first == 0
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_documentation()
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0)
        || (first == 198 && (second == 18 || second == 19))
        || first >= 240
}

pub(crate) fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Return whether a release repository matches the provider identity rules.
///
/// GitHub and Forgejo use exactly two path components. GitLab supports nested
/// namespaces, so it accepts the same two-to-twenty component range the release
/// resolver already enforces. Every component is non-empty, at most 100
/// characters, and limited to letters, digits, '-', '_', and '.', so the dot
/// segments that URL resolution treats as path syntax never form a repository.
fn is_valid_repository(provider: ReleaseProvider, repository: &str) -> bool {
    let parts = repository.split('/').collect::<Vec<_>>();
    let accepted_length = match provider {
        ReleaseProvider::Github | ReleaseProvider::Forgejo => parts.len() == 2,
        ReleaseProvider::Gitlab => (2..=20).contains(&parts.len()),
    };
    accepted_length && parts.iter().all(|part| is_valid_repository_component(part))
}

fn is_valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Return whether a target path is literal and uses the form its location requires.
///
/// Semantic locations require a relative path and the absolute device location
/// requires an absolute path. Both forms reject empty segments, repeated
/// separators, trailing separators, '.' and '..' segments, control characters
/// such as NUL, backslashes, home shorthand, environment variables,
/// interpolation, and placeholders.
pub(crate) fn is_valid_target_path(location: AppTargetLocation, path: &str) -> bool {
    if path.is_empty()
        || path.chars().any(char::is_control)
        || path.contains('\\')
        || path.contains('~')
        || path.contains('$')
        || path.contains('{')
        || path.contains('}')
    {
        return false;
    }
    let absolute = path.starts_with('/');
    match location {
        AppTargetLocation::AbsoluteDevicePath if !absolute => return false,
        AppTargetLocation::AbsoluteDevicePath => {}
        _ if absolute => return false,
        _ => {}
    }
    let trimmed = match location {
        AppTargetLocation::AbsoluteDevicePath => path.strip_prefix('/').unwrap_or(path),
        _ => path,
    };
    !trimmed.is_empty()
        && !trimmed.ends_with('/')
        && trimmed
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
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

fn validate_package_id(diagnostics: &mut Vec<AuthoredModelDiagnostic>, package_id: &str) {
    if !is_valid_package_id(package_id) {
        diagnostics.push(AuthoredModelDiagnostic::new(
            "package_id_invalid",
            "Package IDs must contain at least two valid dot-separated identifier segments.",
            "package_id",
        ));
    }
}

/// Check Android package syntax shared by authored definitions and reviewed
/// plan identity checks.
pub(crate) fn is_valid_package_id(package_id: &str) -> bool {
    let package_pattern = Regex::new(r"^[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+$")
        .expect("the package-id regex is valid");
    package_pattern.is_match(package_id)
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

pub(crate) fn is_valid_identifier(value: &str) -> bool {
    Regex::new(r"^[a-z0-9]+(?:[._-][a-z0-9]+)*$")
        .expect("the authored identifier regex is valid")
        .is_match(value)
}

/// Apply the template-local size limit without changing authored IDs elsewhere.
pub fn is_valid_template_variable_id(value: &str) -> bool {
    value.len() <= MAX_TEMPLATE_VARIABLE_ID_BYTES && is_valid_identifier(value)
}

/// Accept a concrete Android device path with no traversal or host-path syntax.
pub fn is_valid_device_path(value: &str) -> bool {
    is_valid_target_path(AppTargetLocation::AbsoluteDevicePath, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_SOURCE: &str = include_str!("../../../authored/apps/retroarch.yaml");
    const PROFILE_SOURCE: &str =
        include_str!("../../../authored/device_profiles/ayaneo.pocket_s2.yaml");
    const APP_CONFIG_SOURCE: &str =
        include_str!("../fixtures/configuration_contracts/app-key-value.yaml");
    const JSON_CONFIG_SOURCE: &str =
        include_str!("../fixtures/configuration_contracts/app-nested-json.yaml");
    const APP_HEADER: &str = "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\n";

    fn app_source_with_literal_defaults(count: usize, value: &str) -> String {
        let mut source = String::from(
            "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\ntargets:\n  config_file:\n    kind: file\n    location: app_data\n    path: files/config.cfg\nconfigurations:\n  main:\n    format: key_value/v1\n    target: config_file\n    managed_properties:\n",
        );
        let serialized = serde_json::to_string(value).expect("test literal should serialize");
        for index in 0..count {
            source.push_str(&format!(
                "      property_{index}:\n        path: [property_{index}]\n        type: string\n        default: {serialized}\n"
            ));
        }
        source
    }

    fn managed_property_mut<'a>(
        app: &'a mut AppDefinitionV1,
        property_id: &str,
    ) -> &'a mut AppManagedPropertyV1 {
        app.configurations
            .get_mut("main")
            .expect("main configuration exists")
            .managed_properties
            .get_mut(property_id)
            .expect("managed property exists")
    }

    fn allowed_value_field(property_id: &str, index: usize) -> String {
        format!("configurations[main].managed_properties[{property_id}].validators.allowed_values[{index}]")
    }

    #[test]
    fn app_definition_yaml_rejects_duplicate_configuration_and_property_keys() {
        let unique_configuration = app_source_with_literal_defaults(1, "value");
        assert!(parse_app_definition_yaml(&unique_configuration).is_ok());

        let duplicate_configuration = unique_configuration.replacen(
            "  main:\n    format: key_value/v1\n    target: config_file\n",
            "  main:\n    format: key_value/v1\n    target: config_file\n  main:\n    format: key_value/v1\n    target: config_file\n",
            1,
        );
        assert_ne!(duplicate_configuration, unique_configuration);
        assert!(parse_app_definition_yaml(&duplicate_configuration).is_err());

        let unique_properties = app_source_with_literal_defaults(2, "value");
        let duplicate_property =
            unique_properties.replacen("      property_1:", "      property_0:", 1);
        assert_ne!(duplicate_property, unique_properties);
        assert!(parse_app_definition_yaml(&duplicate_property).is_err());
    }

    #[test]
    fn configuration_documents_for_key_value_and_nested_json_round_trip() {
        for source in [APP_CONFIG_SOURCE, JSON_CONFIG_SOURCE] {
            let parsed = parse_app_definition_yaml(source).expect("typed configuration schema");
            assert!(validate_app_definition(&parsed).is_empty());

            let canonical = emit_app_definition_yaml(&parsed).expect("canonical App Definition");
            let reloaded = parse_app_definition_yaml(&canonical).expect("canonical schema reloads");
            assert_eq!(reloaded, parsed);
        }
    }

    #[test]
    fn nested_json_managed_paths_must_not_overlap() {
        let source = JSON_CONFIG_SOURCE.replace(
            "      overlay_path:\n",
            "      overlay_parent:\n        path: [ui, overlays]\n        type: string\n      overlay_path:\n",
        );
        let parsed = parse_app_definition_yaml(&source).expect("schema shape parses");
        let diagnostics = validate_app_definition(&parsed);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "app_config_property_path_overlap"),
            "overlapping JSON segment paths need an actionable diagnostic: {diagnostics:?}"
        );
    }

    #[test]
    fn json_v1_property_paths_use_nonempty_object_member_segments() {
        let literal_segments = vec!["ui".to_string(), "a/b".to_string(), "~name".to_string()];
        assert_eq!(
            configuration_property_path_tokens(AppConfigurationFormatV1::JsonV1, &literal_segments),
            Some(literal_segments.clone())
        );
        assert!(
            configuration_property_path_tokens(AppConfigurationFormatV1::JsonV1, &[]).is_none()
        );
        assert!(configuration_property_path_tokens(
            AppConfigurationFormatV1::JsonV1,
            &["ui".to_string(), "".to_string()]
        )
        .is_none());
        assert!(configuration_property_path_tokens(
            AppConfigurationFormatV1::JsonV1,
            &["ui".to_string(), "bad\nkey".to_string()]
        )
        .is_none());
        assert!(configuration_paths_overlap(
            AppConfigurationFormatV1::JsonV1,
            &["ui".to_string(), "a/b".to_string()],
            &["ui".to_string(), "a/b".to_string(), "child".to_string()]
        ));
        assert!(!configuration_paths_overlap(
            AppConfigurationFormatV1::JsonV1,
            &["ui".to_string(), "a/b".to_string()],
            &["ui".to_string(), "a".to_string(), "b".to_string()]
        ));
    }

    #[test]
    fn app_configuration_paths_are_segment_lists() {
        let parsed = parse_app_definition_yaml(JSON_CONFIG_SOURCE)
            .expect("JSON paths should accept explicit member-name segments");
        assert!(validate_app_definition(&parsed).is_empty());

        let scalar_pointer_paths = JSON_CONFIG_SOURCE
            .replace(
                "path: [audio, output, buffer_frames]",
                "path: /audio/output/buffer_frames",
            )
            .replace(
                "path: [ui, overlays, primary, path]",
                "path: /ui/overlays/primary/path",
            );
        assert_ne!(scalar_pointer_paths, JSON_CONFIG_SOURCE);
        assert!(scalar_pointer_paths.contains("path: /audio/output/buffer_frames"));
        assert!(scalar_pointer_paths.contains("path: /ui/overlays/primary/path"));
        assert!(
            parse_app_definition_yaml(&scalar_pointer_paths).is_err(),
            "scalar JSON Pointer paths must be rejected"
        );
    }

    #[test]
    fn app_configuration_path_segments_must_be_yaml_strings() {
        for path in ["[1]", "[true]", "[[nested]]", "[{nested: value}]"] {
            let source =
                APP_CONFIG_SOURCE.replace("path: [audio_driver]", &format!("path: {path}"));
            assert_ne!(
                source, APP_CONFIG_SOURCE,
                "test path must alter the fixture"
            );
            assert!(
                parse_app_definition_yaml(&source).is_err(),
                "non-string managed path segments must be rejected: {path}"
            );
        }
    }

    #[test]
    fn app_managed_clearability_is_authored_and_object_types_are_rejected() {
        let parsed = parse_app_definition_yaml(APP_CONFIG_SOURCE)
            .expect("App Definition clearability should parse");
        let properties = &parsed.configurations["main"].managed_properties;
        assert!(properties["network_enabled"].clearable);
        assert!(!properties["audio_driver"].clearable);

        let object_managed_property = APP_CONFIG_SOURCE.replacen("type: string", "type: object", 1);
        assert!(parse_app_definition_yaml(&object_managed_property).is_err());
    }

    #[test]
    fn string_min_length_is_validated_and_applies_to_defaults() {
        let source = APP_CONFIG_SOURCE.replacen("min_length: 1", "min_length: 6", 1);
        let app = parse_app_definition_yaml(&source).expect("min_length should parse");
        assert!(validate_app_definition(&app)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_default_invalid"));

        let incompatible =
            APP_CONFIG_SOURCE.replace("maximum: 240", "maximum: 240\n          min_length: 1");
        let app = parse_app_definition_yaml(&incompatible).expect("validator should parse");
        assert!(validate_app_definition(&app)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_validator_invalid"));
    }

    #[test]
    fn allowed_string_values_shorter_than_min_length_are_rejected() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "shader_path");
        property.validators.min_length = Some(3);
        property.validators.allowed_values = vec![serde_json::json!("xy")];

        let diagnostics = validate_app_definition(&app);
        let field = allowed_value_field("shader_path", 0);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.field == field)
            .expect("an allowed value shorter than min_length must be rejected");
        assert_eq!(diagnostic.code, "app_config_validator_invalid");
        assert!(diagnostic.message.contains("min_length"));
    }

    #[test]
    fn allowed_string_values_longer_than_max_length_are_rejected() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "shader_path");
        property.default = Some(serde_json::json!("ok"));
        property.validators.max_length = Some(4);
        property.validators.allowed_values = vec![serde_json::json!("longer")];

        let diagnostics = validate_app_definition(&app);
        let field = allowed_value_field("shader_path", 0);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.field == field)
            .expect("an allowed value longer than max_length must be rejected");
        assert_eq!(diagnostic.code, "app_config_validator_invalid");
        assert!(diagnostic.message.contains("max_length"));
    }

    #[test]
    fn allowed_integer_values_outside_minimum_and_maximum_are_rejected() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.validators.allowed_values = vec![
            serde_json::json!(29),
            serde_json::json!(30),
            serde_json::json!(240),
            serde_json::json!(241),
        ];

        let diagnostics = validate_app_definition(&app);
        for (index, bound) in [(0, "minimum"), (3, "maximum")] {
            let field = allowed_value_field("frame_limit", index);
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| diagnostic.field == field)
                .unwrap_or_else(|| panic!("allowed value outside {bound} must be rejected"));
            assert_eq!(diagnostic.code, "app_config_validator_invalid");
            assert!(diagnostic.message.contains(bound));
        }
        for index in [1, 2] {
            let field = allowed_value_field("frame_limit", index);
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.field != field),
                "an allowed value on a numeric bound must remain valid"
            );
        }
    }

    #[test]
    fn mixed_allowed_values_validate_each_entry_and_preserve_valid_values() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "shader_path");
        property.validators.min_length = Some(3);
        property.validators.max_length = Some(5);
        property.validators.allowed_values =
            vec![serde_json::json!("valid"), serde_json::json!("no")];

        let diagnostics = validate_app_definition(&app);
        let allowed_value_fields: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.field.contains(".validators.allowed_values["))
            .map(|diagnostic| diagnostic.field.as_str())
            .collect();
        assert_eq!(
            allowed_value_fields,
            vec![
                "configurations[main].managed_properties[shader_path].validators.allowed_values[1]"
            ]
        );
        assert!(diagnostics
            .iter()
            .find(|diagnostic| diagnostic.field == allowed_value_field("shader_path", 1))
            .is_some_and(|diagnostic| diagnostic.message.contains("min_length")));
    }

    #[test]
    fn integer_bounds_compare_exact_values_above_f64_integer_precision() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = app
            .configurations
            .get_mut("main")
            .expect("configuration exists")
            .managed_properties
            .get_mut("frame_limit")
            .expect("integer property exists");
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_993_i64));
        property.validators.maximum = Some(serde_json::json!(9_007_199_254_740_994_i64));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_993_i64)
        ));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_994_i64)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_992_i64)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_995_i64)
        ));
    }

    #[test]
    fn number_bounds_compare_integer_values_above_f64_precision_exactly() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.validators.allowed_values.clear();
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_993_i64));
        property.validators.maximum = Some(serde_json::json!(9_007_199_254_740_994_i64));

        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_993_i64)
        ));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_994_i64)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_992_i64)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_995_i64)
        ));
    }

    #[test]
    fn number_bounds_compare_decimal_and_exact_mixed_representations() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.validators.allowed_values.clear();
        property.validators.minimum = Some(serde_json::json!(1.5));
        property.validators.maximum = Some(serde_json::json!(2.5));

        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(2)
        ));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(1.5)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(1)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(2.75)
        ));

        property.validators.minimum = Some(serde_json::json!(1));
        property.validators.maximum = Some(serde_json::json!(2));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(1.5)
        ));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(2.0)
        ));

        property.validators.minimum = Some(serde_json::json!(2.0));
        property.validators.maximum = Some(serde_json::json!(2));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(2)
        ));
    }

    #[test]
    fn number_bounds_compare_mixed_integer_and_float_values_exactly() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.validators.allowed_values.clear();
        let value = serde_json::json!(9_007_199_254_740_993_i64);
        let lower_bound = serde_json::json!(9_007_199_254_740_992.0);

        property.validators.minimum = Some(lower_bound.clone());
        property.validators.maximum = None;
        assert!(managed_property_value_is_valid(property, &value));

        property.validators.minimum = None;
        property.validators.maximum = Some(lower_bound);
        assert!(!managed_property_value_is_valid(property, &value));

        property.validators.maximum = None;
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_994.0));
        assert!(!managed_property_value_is_valid(property, &value));
    }

    #[test]
    fn number_mixed_comparisons_cover_signed_unsigned_and_fractional_boundaries() {
        let cases = [
            (
                serde_json::json!(9_007_199_254_740_993_i64),
                serde_json::json!(9_007_199_254_740_992.0),
                Ordering::Greater,
            ),
            (
                serde_json::json!(u64::MAX),
                serde_json::json!(18_446_744_073_709_549_568.0),
                Ordering::Greater,
            ),
            (
                serde_json::json!(u64::MAX),
                serde_json::json!(u64::MAX as f64),
                Ordering::Less,
            ),
            (
                serde_json::json!(i64::MIN),
                serde_json::json!(i64::MIN as f64),
                Ordering::Equal,
            ),
            (
                serde_json::json!(i64::MAX),
                serde_json::json!(i64::MAX as f64),
                Ordering::Less,
            ),
            (serde_json::json!(2), serde_json::json!(2.5), Ordering::Less),
            (
                serde_json::json!(-2),
                serde_json::json!(-2.5),
                Ordering::Greater,
            ),
            (
                serde_json::json!(0),
                serde_json::json!(-0.5),
                Ordering::Greater,
            ),
            (
                serde_json::json!(0),
                serde_json::json!(f64::from_bits(1)),
                Ordering::Less,
            ),
            (
                serde_json::json!(0),
                serde_json::json!(-f64::from_bits(1)),
                Ordering::Greater,
            ),
        ];

        for (left, right, expected) in cases {
            assert_eq!(compare_number_values(&left, &right), Some(expected));
            assert_eq!(
                compare_number_values(&right, &left),
                Some(expected.reverse())
            );
        }
    }

    #[test]
    fn number_contradictory_integer_bounds_are_rejected_exactly() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.default = None;
        property.validators.allowed_values.clear();
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_993_i64));
        property.validators.maximum = Some(serde_json::json!(9_007_199_254_740_992_i64));

        let diagnostics = validate_app_definition(&app);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "app_config_validator_invalid"
                && diagnostic.field
                    == "configurations[main].managed_properties[frame_limit].validators"
                && diagnostic
                    .message
                    .contains("Minimum must not exceed maximum")
        }));
    }

    #[test]
    fn number_mixed_contradictory_bounds_are_rejected_exactly() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.default = None;
        property.validators.allowed_values.clear();
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_993_i64));
        property.validators.maximum = Some(serde_json::json!(9_007_199_254_740_992.0));

        let diagnostics = validate_app_definition(&app);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "app_config_validator_invalid"
                && diagnostic.field
                    == "configurations[main].managed_properties[frame_limit].validators"
                && diagnostic
                    .message
                    .contains("Minimum must not exceed maximum")
        }));
    }

    #[test]
    fn number_allowed_values_obey_exact_integer_bounds_above_f64_precision() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.default = None;
        property.validators.minimum = Some(serde_json::json!(9_007_199_254_740_992_i64));
        property.validators.maximum = Some(serde_json::json!(9_007_199_254_740_992_i64));
        property.validators.allowed_values = vec![
            serde_json::json!(9_007_199_254_740_992_i64),
            serde_json::json!(9_007_199_254_740_993_i64),
        ];

        let diagnostics = validate_app_definition(&app);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.field == allowed_value_field("frame_limit", 1)
                && diagnostic.code == "app_config_validator_invalid"
                && diagnostic.message.contains("maximum")
        }));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.field != allowed_value_field("frame_limit", 0) }));
    }

    #[test]
    fn number_allowed_values_detect_exact_duplicates_without_rounding() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.default = None;
        property.validators.minimum = None;
        property.validators.maximum = None;
        property.validators.allowed_values = vec![
            serde_json::json!(9_007_199_254_740_992_i64),
            serde_json::json!(9_007_199_254_740_993_i64),
            serde_json::json!(2),
            serde_json::json!(2.0),
        ];

        let diagnostics = validate_app_definition(&app);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.field == allowed_value_field("frame_limit", 3)
                && diagnostic.code == "app_config_validator_invalid"
        }));
        for index in [0, 1, 2] {
            assert!(diagnostics.iter().all(|diagnostic| {
                diagnostic.field != allowed_value_field("frame_limit", index)
            }));
        }
    }

    #[test]
    fn number_allowed_values_use_exact_numeric_equality() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.type_name = ConfigValueTypeV1::Number;
        property.validators.minimum = None;
        property.validators.maximum = None;
        property.validators.allowed_values = vec![serde_json::json!(9_007_199_254_740_992_i64)];

        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_992_i64)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_993_i64)
        ));

        property.validators.allowed_values = vec![serde_json::json!(2)];
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(2.0)
        ));
        property.validators.allowed_values = vec![serde_json::json!(2.0)];
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(2)
        ));
    }

    #[test]
    fn number_allowed_values_keep_distinct_mixed_large_numbers_distinct() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        {
            let property = managed_property_mut(&mut app, "frame_limit");
            property.type_name = ConfigValueTypeV1::Number;
            property.default = None;
            property.validators.minimum = None;
            property.validators.maximum = None;
            property.validators.allowed_values = vec![
                serde_json::json!(9_007_199_254_740_992.0),
                serde_json::json!(9_007_199_254_740_993_i64),
            ];
        }

        let diagnostics = validate_app_definition(&app);
        for index in [0, 1] {
            assert!(diagnostics.iter().all(|diagnostic| {
                diagnostic.field != allowed_value_field("frame_limit", index)
            }));
        }
        let property = managed_property_mut(&mut app, "frame_limit");
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_992.0)
        ));
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(9_007_199_254_740_993_i64)
        ));
    }

    #[test]
    fn number_type_validation_keeps_integer_and_number_distinct() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let property = managed_property_mut(&mut app, "frame_limit");
        property.validators.allowed_values.clear();
        property.type_name = ConfigValueTypeV1::Number;
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(30.5)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!("30")
        ));

        property.type_name = ConfigValueTypeV1::Integer;
        assert!(managed_property_value_is_valid(
            property,
            &serde_json::json!(30)
        ));
        assert!(!managed_property_value_is_valid(
            property,
            &serde_json::json!(30.5)
        ));
    }

    #[test]
    fn aggregate_inline_literal_budget_is_per_configuration_document() {
        let mut app = parse_app_definition_yaml(APP_CONFIG_SOURCE).expect("fixture should parse");
        let large_default = serde_json::json!("x".repeat(1_100_000));
        let mut second = app.configurations["main"].clone();
        app.configurations
            .get_mut("main")
            .expect("main configuration exists")
            .managed_properties
            .get_mut("shader_path")
            .expect("string property exists")
            .default = Some(large_default.clone());
        second
            .managed_properties
            .get_mut("shader_path")
            .expect("string property exists")
            .default = Some(large_default);
        app.configurations.insert("alternate".to_string(), second);

        assert!(!validate_app_definition(&app)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_literal_budget_exceeded"));
    }
    #[test]
    fn app_configuration_rejects_duplicate_ids_paths_and_incompatible_validators() {
        let duplicate_id = APP_CONFIG_SOURCE.replace("      shader_path:", "      audio_driver:");
        assert!(parse_app_definition_yaml(&duplicate_id).is_err());
        let duplicate_path =
            APP_CONFIG_SOURCE.replace("path: [shader_path]", "path: [audio_driver]");
        let parsed = parse_app_definition_yaml(&duplicate_path)
            .expect("duplicate paths are semantic errors");
        assert!(validate_app_definition(&parsed)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_property_path_overlap"));
        let incompatible_default = APP_CONFIG_SOURCE.replace("default: alsa", "default: 42");
        let parsed = parse_app_definition_yaml(&incompatible_default)
            .expect("typed default parses structurally");
        assert!(validate_app_definition(&parsed)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_default_invalid"));
        let incompatible_allowed_value =
            APP_CONFIG_SOURCE.replace("allowed_values: [alsa, \"\"]", "allowed_values: [7]");
        let parsed = parse_app_definition_yaml(&incompatible_allowed_value)
            .expect("typed validator parses structurally");
        assert!(validate_app_definition(&parsed)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_validator_invalid"));
    }
    #[test]
    fn app_configuration_rejects_null_values_and_unsafe_template_paths() {
        let null_default = APP_CONFIG_SOURCE.replace("default: alsa", "default: null");
        assert!(parse_app_definition_yaml(&null_default).is_err());
        let unsafe_template = APP_CONFIG_SOURCE.replace("retroarch/default.cfg", "../outside.cfg");
        let parsed = parse_app_definition_yaml(&unsafe_template)
            .expect("template path is validated semantically");
        assert!(validate_app_definition(&parsed)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_configuration_template_path_invalid"));
    }
    #[test]
    fn configuration_schema_unknown_fields_are_rejected() {
        let source = APP_CONFIG_SOURCE.replace(
            "    target: config_file\n",
            "    target: config_file\n    formatter_plugin: custom\n",
        );
        assert!(parse_app_definition_yaml(&source).is_err());
    }

    #[test]
    fn inline_configuration_literals_obey_inclusive_byte_limits() {
        let exact_value = "x".repeat(64 * 1024);
        let parsed = parse_app_definition_yaml(&app_source_with_literal_defaults(1, &exact_value))
            .expect("64 KiB inline literal should load");
        assert!(validate_app_definition(&parsed).is_empty());

        let oversized = "x".repeat(64 * 1024 + 1);
        let parsed = parse_app_definition_yaml(&app_source_with_literal_defaults(1, &oversized))
            .expect("oversized values remain structurally parseable");
        assert!(validate_app_definition(&parsed)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_literal_too_large"));

        let aggregate_exact =
            parse_app_definition_yaml(&app_source_with_literal_defaults(32, &exact_value))
                .expect("exact aggregate fixture should load");
        assert!(validate_app_definition(&aggregate_exact).is_empty());

        let aggregate_over =
            parse_app_definition_yaml(&app_source_with_literal_defaults(33, &exact_value))
                .expect("aggregate fixture should load structurally");
        assert!(validate_app_definition(&aggregate_over)
            .iter()
            .any(|diagnostic| diagnostic.code == "app_config_literal_budget_exceeded"));
    }

    fn base_app() -> AppDefinitionV1 {
        AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.to_string(),
            id: "example".to_string(),
            name: "Example".to_string(),
            description: None,
            category: None,
            package_id: "com.example.app".to_string(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            configurations: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        }
    }

    fn app_with_artifact(source: AppArtifactSource) -> AppDefinitionV1 {
        let mut app = base_app();
        app.artifacts.insert(
            "apk".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::Apk,
                name: None,
                description: None,
                source,
            },
        );
        app
    }

    fn app_with_target(location: AppTargetLocation, path: &str) -> AppDefinitionV1 {
        let mut app = base_app();
        app.targets.insert(
            "target".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::Directory,
                location,
                path: path.to_string(),
            },
        );
        app
    }

    fn codes(app: &AppDefinitionV1) -> Vec<String> {
        validate_app_definition(app)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    fn latest_release_source(provider: ReleaseProvider, repository: &str) -> AppArtifactSource {
        AppArtifactSource::LatestRelease {
            provider,
            base_url: "https://github.com".to_string(),
            repository: repository.to_string(),
            asset_pattern: Some("^app-release\\.apk$".to_string()),
            invert_asset_pattern: None,
            prerelease: false,
        }
    }

    fn runtime_action(
        permission: &str,
        android_api_min: Option<i64>,
        android_api_max: Option<i64>,
    ) -> AppRuntimePermissionAction {
        AppRuntimePermissionAction {
            permission: permission.to_string(),
            android_api_min,
            android_api_max,
        }
    }

    #[test]
    fn app_definition_canonical_emission_is_idempotent_and_complete() {
        let parsed = parse_app_definition_yaml(APP_SOURCE).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        let reparsed = parse_app_definition_yaml(&emitted).unwrap();
        assert_eq!(reparsed, parsed);
        assert_eq!(emit_app_definition_yaml(&reparsed).unwrap(), emitted);
        assert!(emitted.contains("package_id: com.retroarch.aarch64"));
        assert!(emitted.contains("permission_sets:"));
        assert!(emitted.contains("targets:"));
        assert!(!emitted.contains("aliases"));
        assert!(!emitted.contains("inputs"));
    }

    #[test]
    fn app_definition_omits_absent_optional_sections() {
        let source = "schema_version: 1\nkind: app_definition\nid: minimal\nname: Minimal\npackage_id: com.example.minimal\nartifacts:\n  apk:\n    kind: apk\n    source:\n      strategy: user_provided\n";
        let parsed = parse_app_definition_yaml(source).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        for absent in [
            "description:",
            "category:",
            "permission_sets:",
            "targets:",
            "launcher_activity:",
            "metadata:",
        ] {
            assert!(
                !emitted.contains(absent),
                "{absent} should be omitted: {emitted}"
            );
        }
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), parsed);
    }

    #[test]
    fn legacy_app_definition_fields_are_rejected_without_compatibility() {
        let legacy = "schema_version: 1\nkind: app_definition\nid: legacy\nname: Legacy\npackage:\n  primary: com.example.legacy\n  aliases: []\ninstall_source:\n  type: local_placeholder\ntracking_source:\n  type: local\nartifacts:\n  apk:\n    required: true\n  shared_storage_config:\n    supported: false\n  app_data_config:\n    supported: false\n  byo_apk:\n    required: false\nprovisioning:\n  launch_once_recommended: false\ninputs: []\n";
        assert_eq!(
            parse_app_definition_yaml(legacy).unwrap_err().code(),
            "app_definition_yaml_invalid"
        );

        for legacy_fragment in [
            "package:\n  primary: com.example.legacy\n",
            "install_source:\n  type: local_placeholder\n",
            "tracking_source:\n  type: local\n",
            "provisioning:\n  launch_once_recommended: true\n",
            "inputs: []\n",
            "required: true\n",
        ] {
            let source = format!("{APP_SOURCE}{legacy_fragment}");
            assert_eq!(
                parse_app_definition_yaml(&source).unwrap_err().code(),
                "app_definition_yaml_invalid",
                "{legacy_fragment} should be rejected"
            );
        }
    }

    #[test]
    fn nested_artifact_and_metadata_order_are_preserved_losslessly() {
        let source = "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\nartifacts:\n  first:\n    kind: file\n    source:\n      strategy: user_provided\n  second:\n    kind: apk\n    source:\n      strategy: user_provided\nmetadata:\n  first_metadata:\n    nested:\n      - one\n      - enabled: true\n  second_metadata:\n    - null\n";
        let parsed = parse_app_definition_yaml(source).unwrap();
        assert_eq!(
            parsed.artifacts.keys().collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(
            parsed.metadata.keys().collect::<Vec<_>>(),
            vec!["first_metadata", "second_metadata"]
        );

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), parsed);
        assert!(emitted.find("first:").unwrap() < emitted.find("second:").unwrap());
        assert!(
            emitted.find("first_metadata:").unwrap() < emitted.find("second_metadata:").unwrap()
        );
    }

    #[test]
    fn artifact_sources_round_trip_every_accepted_strategy() {
        let source = "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\nartifacts:\n  local_apk:\n    kind: apk\n    name: Local APK\n    description: Built by the author.\n    source:\n      strategy: user_provided\n  direct:\n    kind: apk\n    source:\n      strategy: direct_url\n      url: https://downloads.example.com/app.apk?version=2\n      sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n  release:\n    kind: apk\n    source:\n      strategy: latest_release\n      provider: gitlab\n      base_url: https://gitlab.com\n      repository: group/subgroup/project\n      asset_pattern: '^app-v[0-9]+\\.apk$'\n      invert_asset_pattern: true\n      prerelease: true\n";
        let parsed = parse_app_definition_yaml(source).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), parsed);
        assert_eq!(emit_app_definition_yaml(&parsed).unwrap(), emitted);

        let without_prerelease = source.replace("      prerelease: true\n", "");
        assert_eq!(
            parse_app_definition_yaml(&without_prerelease)
                .unwrap_err()
                .code(),
            "app_definition_yaml_invalid"
        );
    }

    #[test]
    fn artifact_source_strategies_reject_cross_strategy_fields() {
        for fragment in [
            "artifacts:\n  direct:\n    kind: apk\n    source:\n      strategy: user_provided\n      url: https://downloads.example.com/app.apk\n",
            "artifacts:\n  direct:\n    kind: apk\n    source:\n      strategy: direct_url\n      url: https://downloads.example.com/app.apk\n      prerelease: false\n",
            "artifacts:\n  direct:\n    kind: apk\n    source:\n      strategy: latest_release\n      provider: github\n      base_url: https://github.com\n      repository: owner/repository\n      prerelease: false\n      url: https://downloads.example.com/app.apk\n",
            "artifacts:\n  direct:\n    kind: apk\n    source:\n      strategy: direct_url\n      url: https://downloads.example.com/app.apk\n      invert_asset_pattern: true\n",
        ] {
            let source = format!("{APP_HEADER}{fragment}");
            assert_eq!(
                parse_app_definition_yaml(&source).unwrap_err().code(),
                "app_definition_yaml_invalid",
                "{fragment}"
            );
        }
    }

    #[test]
    fn direct_url_artifact_validation_enforces_public_https_and_lowercase_checksums() {
        let lowercase_sha256 = "a".repeat(64);
        let uppercase_sha256 = "B".repeat(64);
        for (url, sha256, expected) in [
            (
                "http://downloads.example.com/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://user:secret@downloads.example.com/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://downloads.example.com/app.apk#section",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://localhost/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://downloads.localhost/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://localhost./app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://[::ffff:127.0.0.1]/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://127.0.0.1/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://10.0.0.1/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://172.16.0.1/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://192.168.1.1/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://169.254.10.20/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://0.0.0.0/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://0.1.2.3/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://[::1]/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://[fc00::1]/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://[fe80::1]/app.apk",
                None,
                Some("app_artifact_url_invalid"),
            ),
            (
                "https://downloads.example.com/app.apk?version=2",
                None,
                None,
            ),
            ("https://8.8.8.8/app.apk", None, None),
            (
                "https://downloads.example.com/app.apk?version=2",
                Some(lowercase_sha256.as_str()),
                None,
            ),
            (
                "https://downloads.example.com/app.apk",
                Some(uppercase_sha256.as_str()),
                Some("app_artifact_sha256_invalid"),
            ),
            (
                "https://downloads.example.com/app.apk",
                Some("abc"),
                Some("app_artifact_sha256_invalid"),
            ),
        ] {
            let app = app_with_artifact(AppArtifactSource::DirectUrl {
                url: url.to_string(),
                sha256: sha256.map(str::to_string),
            });
            let codes = codes(&app);
            match expected {
                Some(expected) => {
                    assert!(codes.contains(&expected.to_string()), "{url}: {codes:?}")
                }
                None => assert!(codes.is_empty(), "{url}: {codes:?}"),
            }
        }
    }

    #[test]
    fn latest_release_repository_validation_is_provider_aware() {
        let deep_gitlab = (1..=20)
            .map(|index| format!("namespace{index}"))
            .collect::<Vec<_>>()
            .join("/");
        let too_deep_gitlab = (1..=21)
            .map(|index| format!("namespace{index}"))
            .collect::<Vec<_>>()
            .join("/");
        let long_component = "a".repeat(101);
        let long_repository = format!("owner/{long_component}");
        for (provider, repository, valid) in [
            (ReleaseProvider::Github, "owner/repository", true),
            (ReleaseProvider::Github, "owner", false),
            (ReleaseProvider::Github, "owner/repository/extra", false),
            (ReleaseProvider::Github, "owner/repo name", false),
            (ReleaseProvider::Forgejo, "owner/repository", true),
            (ReleaseProvider::Forgejo, "owner/repository/extra", false),
            (ReleaseProvider::Forgejo, "owner", false),
            (ReleaseProvider::Gitlab, "group/project", true),
            (ReleaseProvider::Gitlab, "group/subgroup/project", true),
            (ReleaseProvider::Gitlab, deep_gitlab.as_str(), true),
            (ReleaseProvider::Gitlab, too_deep_gitlab.as_str(), false),
            (ReleaseProvider::Gitlab, "group", false),
            (ReleaseProvider::Gitlab, "group//project", false),
            (ReleaseProvider::Gitlab, long_repository.as_str(), false),
            (ReleaseProvider::Github, "owner/..", false),
            (ReleaseProvider::Gitlab, "group/./project", false),
        ] {
            let app = app_with_artifact(latest_release_source(provider, repository));
            let codes = codes(&app);
            if valid {
                assert!(codes.is_empty(), "{provider:?} {repository}: {codes:?}");
            } else {
                assert!(
                    codes.contains(&"app_artifact_repository_invalid".to_string()),
                    "{provider:?} {repository}: {codes:?}"
                );
            }
        }
    }

    #[test]
    fn latest_release_validation_enforces_service_origin_pattern_and_invert_rules() {
        for valid_base_url in [
            "https://github.com",
            "https://github.com/",
            "https://gitlab.example.com",
        ] {
            let mut app = app_with_artifact(latest_release_source(
                ReleaseProvider::Github,
                "owner/repository",
            ));
            if let Some(AppArtifactSource::LatestRelease { base_url, .. }) = app
                .artifacts
                .get_mut("apk")
                .map(|artifact| &mut artifact.source)
            {
                *base_url = valid_base_url.to_string();
            }
            assert!(
                codes(&app).is_empty(),
                "{valid_base_url}: {:?}",
                codes(&app)
            );
        }

        for invalid_base_url in [
            "http://github.com",
            "https://github.com?owner=1",
            "https://user:secret@github.com",
            "https://github.com#releases",
            "https://github.com/api/v3",
            "https://gitlab.com/group",
            "https://github.com/releases/",
            "https://localhost",
            "https://127.0.0.1",
            "https://[::1]",
        ] {
            let mut app = app_with_artifact(latest_release_source(
                ReleaseProvider::Github,
                "owner/repository",
            ));
            if let Some(AppArtifactSource::LatestRelease { base_url, .. }) = app
                .artifacts
                .get_mut("apk")
                .map(|artifact| &mut artifact.source)
            {
                *base_url = invalid_base_url.to_string();
            }
            assert!(
                codes(&app).contains(&"app_artifact_base_url_invalid".to_string()),
                "{invalid_base_url}: {:?}",
                codes(&app)
            );
        }

        for invalid_pattern in ["[", "  "] {
            let mut app = app_with_artifact(latest_release_source(
                ReleaseProvider::Github,
                "owner/repository",
            ));
            if let Some(AppArtifactSource::LatestRelease { asset_pattern, .. }) = app
                .artifacts
                .get_mut("apk")
                .map(|artifact| &mut artifact.source)
            {
                *asset_pattern = Some(invalid_pattern.to_string());
            }
            assert!(codes(&app).contains(&"app_artifact_asset_pattern_invalid".to_string()));
        }

        for invert in [Some(true), Some(false)] {
            let mut app = app_with_artifact(latest_release_source(
                ReleaseProvider::Github,
                "owner/repository",
            ));
            if let Some(AppArtifactSource::LatestRelease {
                asset_pattern,
                invert_asset_pattern,
                ..
            }) = app
                .artifacts
                .get_mut("apk")
                .map(|artifact| &mut artifact.source)
            {
                *asset_pattern = None;
                *invert_asset_pattern = invert;
            }
            assert!(codes(&app).contains(&"app_artifact_invert_without_pattern".to_string()));
        }

        let mut app = app_with_artifact(latest_release_source(
            ReleaseProvider::Github,
            "owner/repository",
        ));
        if let Some(AppArtifactSource::LatestRelease {
            invert_asset_pattern,
            ..
        }) = app
            .artifacts
            .get_mut("apk")
            .map(|artifact| &mut artifact.source)
        {
            *invert_asset_pattern = Some(true);
        }
        assert!(codes(&app).is_empty());
        let emitted = emit_app_definition_yaml(&app).unwrap();
        assert!(emitted.contains("invert_asset_pattern: true"));

        if let Some(AppArtifactSource::LatestRelease {
            invert_asset_pattern,
            ..
        }) = app
            .artifacts
            .get_mut("apk")
            .map(|artifact| &mut artifact.source)
        {
            *invert_asset_pattern = None;
        }
        let emitted = emit_app_definition_yaml(&app).unwrap();
        assert!(!emitted.contains("invert_asset_pattern"));
    }

    #[test]
    fn latest_release_canonicalizes_an_explicit_default_inversion_flag() {
        let source = format!(
            "{APP_HEADER}artifacts:\n  release:\n    kind: apk\n    source:\n      strategy: latest_release\n      provider: github\n      base_url: https://github.com\n      repository: owner/repository\n      asset_pattern: '^app[.]apk$'\n      invert_asset_pattern: false\n      prerelease: false\n"
        );
        let parsed = parse_app_definition_yaml(&source).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());

        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        assert!(!emitted.contains("invert_asset_pattern"), "{emitted}");

        let reparsed = parse_app_definition_yaml(&emitted).unwrap();
        assert_eq!(reparsed, parsed);
        assert_eq!(emit_app_definition_yaml(&reparsed).unwrap(), emitted);
    }

    #[test]
    fn latest_release_rejects_an_inversion_flag_without_a_pattern() {
        for authored in ["true", "false"] {
            let source = format!(
                "{APP_HEADER}artifacts:\n  release:\n    kind: apk\n    source:\n      strategy: latest_release\n      provider: github\n      base_url: https://github.com\n      repository: owner/repository\n      invert_asset_pattern: {authored}\n      prerelease: false\n"
            );
            let parsed = parse_app_definition_yaml(&source).unwrap();
            assert!(
                codes(&parsed).contains(&"app_artifact_invert_without_pattern".to_string()),
                "invert_asset_pattern: {authored}: {:?}",
                codes(&parsed)
            );
        }
    }

    #[test]
    fn permission_set_validation_enforces_sets_actions_bounds_and_duplicates() {
        let valid = "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\npermission_sets:\n  baseline:\n    runtime:\n      - permission: android.permission.POST_NOTIFICATIONS\n        android_api_min: 33\n    app_ops:\n      - op: MANAGE_EXTERNAL_STORAGE\n        mode: allow\n  elevated:\n    runtime:\n      - permission: android.permission.WRITE_SECURE_SETTINGS\n";
        let parsed = parse_app_definition_yaml(valid).unwrap();
        assert!(validate_app_definition(&parsed).is_empty());
        let emitted = emit_app_definition_yaml(&parsed).unwrap();
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), parsed);

        let mut app = base_app();
        app.permission_sets = Some(AppPermissionSets::default());
        assert!(codes(&app).contains(&"app_permission_set_empty".to_string()));

        let mut app = base_app();
        app.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet::default()),
            elevated: None,
        });
        assert!(codes(&app).contains(&"app_permission_set_empty".to_string()));

        let mut app = base_app();
        app.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![
                    runtime_action("android.permission.POST_NOTIFICATIONS", Some(33), None),
                    runtime_action("android.permission.POST_NOTIFICATIONS", Some(34), None),
                ],
                app_ops: Vec::new(),
            }),
            elevated: None,
        });
        assert!(codes(&app).contains(&"app_permission_action_duplicate".to_string()));

        let mut app = base_app();
        app.permission_sets = Some(AppPermissionSets {
            baseline: Some(AppPermissionSet {
                runtime: vec![runtime_action(
                    "android.permission.WRITE_SECURE_SETTINGS",
                    None,
                    None,
                )],
                app_ops: Vec::new(),
            }),
            elevated: Some(AppPermissionSet {
                runtime: vec![runtime_action(
                    "android.permission.WRITE_SECURE_SETTINGS",
                    None,
                    None,
                )],
                app_ops: Vec::new(),
            }),
        });
        assert!(codes(&app).contains(&"app_permission_action_duplicate".to_string()));

        for (minimum, maximum) in [
            (Some(0), None),
            (Some(35), Some(33)),
            (Some(33), Some(0)),
            (None, Some(0)),
        ] {
            let mut app = base_app();
            app.permission_sets = Some(AppPermissionSets {
                baseline: Some(AppPermissionSet {
                    runtime: vec![runtime_action(
                        "android.permission.POST_NOTIFICATIONS",
                        minimum,
                        maximum,
                    )],
                    app_ops: Vec::new(),
                }),
                elevated: None,
            });
            assert!(
                codes(&app).contains(&"app_permission_api_bounds_invalid".to_string()),
                "{minimum:?} {maximum:?}"
            );
        }
    }

    #[test]
    fn permission_set_structures_reject_unknown_fields_and_actions() {
        for fragment in [
            "permission_sets:\n  administrator: {}\n",
            "permission_sets:\n  baseline:\n    required: true\n",
            "permission_sets:\n  baseline:\n    runtime:\n      - permission: android.permission.POST_NOTIFICATIONS\n        requires_root: true\n",
            "permission_sets:\n  baseline:\n    app_ops:\n      - op: MANAGE_EXTERNAL_STORAGE\n        mode: deny\n",
            "permission_sets:\n  baseline:\n    app_ops:\n      - op: MANAGE_EXTERNAL_STORAGE\n",
        ] {
            let source = format!("{APP_HEADER}{fragment}");
            assert_eq!(
                parse_app_definition_yaml(&source).unwrap_err().code(),
                "app_definition_yaml_invalid",
                "{fragment}"
            );
        }
    }

    #[test]
    fn app_target_validation_enforces_kinds_locations_and_literal_paths() {
        let mut app = base_app();
        app.targets.insert(
            "config".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::File,
                location: AppTargetLocation::AppData,
                path: "config/settings.cfg".to_string(),
            },
        );
        app.targets.insert(
            "system".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::Directory,
                location: AppTargetLocation::SharedStorage,
                path: "RetroArch/system".to_string(),
            },
        );
        app.targets.insert(
            "files".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::File,
                location: AppTargetLocation::ExternalAppData,
                path: "files/retroarch.cfg".to_string(),
            },
        );
        app.targets.insert(
            "staging".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::Directory,
                location: AppTargetLocation::AbsoluteDevicePath,
                path: "/data/local/tmp/example".to_string(),
            },
        );
        assert!(validate_app_definition(&app).is_empty());
        let emitted = emit_app_definition_yaml(&app).unwrap();
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), app);

        for (location, path) in [
            (AppTargetLocation::AppData, "/data/config.cfg"),
            (AppTargetLocation::AppData, "config//settings.cfg"),
            (AppTargetLocation::AppData, "config/"),
            (AppTargetLocation::AppData, "."),
            (AppTargetLocation::AppData, ".."),
            (AppTargetLocation::AppData, "config/../settings.cfg"),
            (AppTargetLocation::AppData, "config/settings.cfg/"),
            (AppTargetLocation::AppData, "config/settings\u{0}.cfg"),
            (AppTargetLocation::SharedStorage, "RetroArch\\system"),
            (AppTargetLocation::SharedStorage, "~/RetroArch"),
            (AppTargetLocation::SharedStorage, "$HOME/RetroArch"),
            (AppTargetLocation::SharedStorage, "{root}/RetroArch"),
            (
                AppTargetLocation::AbsoluteDevicePath,
                "data/local/tmp/example",
            ),
            (
                AppTargetLocation::AbsoluteDevicePath,
                "//data/local/tmp/example",
            ),
        ] {
            let app = app_with_target(location, path);
            assert!(
                codes(&app).contains(&"app_target_path_invalid".to_string()),
                "{location:?} {path}: {:?}",
                codes(&app)
            );
        }

        let mut app = base_app();
        app.targets.insert(
            "Invalid Target".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::Directory,
                location: AppTargetLocation::AppData,
                path: "config".to_string(),
            },
        );
        assert!(codes(&app).contains(&"app_target_id_invalid".to_string()));

        for fragment in [
            "targets:\n  config:\n    kind: file\n    location: app_data\n    role: config\n    path: config/settings.cfg\n",
            "targets:\n  config:\n    kind: symlink\n    location: app_data\n    path: config/settings.cfg\n",
            "targets:\n  config:\n    kind: file\n    location: sdcard\n    path: config/settings.cfg\n",
        ] {
            let source = format!("{APP_HEADER}{fragment}");
            assert_eq!(
                parse_app_definition_yaml(&source).unwrap_err().code(),
                "app_definition_yaml_invalid",
                "{fragment}"
            );
        }
    }

    #[test]
    fn launcher_activity_validation_accepts_only_fully_qualified_classes() {
        for valid in [
            "com.example.app.MainActivity",
            "com.example.app.Main_Activity2",
            "com.example.app.MainActivity$Nested",
            "com.example.app.Outer$Inner$MainActivity",
        ] {
            let mut app = base_app();
            app.launcher_activity = Some(valid.to_string());
            assert!(codes(&app).is_empty(), "{valid}: {:?}", codes(&app));
        }

        for invalid in [
            "MainActivity",
            ".MainActivity",
            "com.example.app/.MainActivity",
            "com/example/app.MainActivity",
            "com.example.app.Main Activity",
            "com..example.MainActivity",
            "com.example.9App.MainActivity",
        ] {
            let mut app = base_app();
            app.launcher_activity = Some(invalid.to_string());
            assert!(
                codes(&app).contains(&"launcher_activity_invalid".to_string()),
                "{invalid}"
            );
        }
    }

    #[test]
    fn metadata_rejects_reserved_keys_and_preserves_order() {
        let mut app = base_app();
        for reserved in ["package_id", "apk_inspection", "artifacts", "metadata"] {
            app.metadata.insert(reserved.to_string(), Value::Null);
        }
        let reserved_count = codes(&app)
            .iter()
            .filter(|code| code.as_str() == "metadata_reserved_key")
            .count();
        assert_eq!(reserved_count, 4);

        let mut app = base_app();
        app.metadata.insert("first".to_string(), Value::Null);
        app.metadata.insert("second".to_string(), Value::Bool(true));
        let emitted = emit_app_definition_yaml(&app).unwrap();
        assert!(emitted.find("first:").unwrap() < emitted.find("second:").unwrap());
        assert_eq!(parse_app_definition_yaml(&emitted).unwrap(), app);
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
    fn strict_app_structures_require_mappings_and_complete_artifact_fields() {
        for fragment in [
            "metadata: []\n",
            "artifacts: []\n",
            "artifacts:\n  apk:\n    kind: bundle\n    source:\n      strategy: user_provided\n",
            "artifacts:\n  apk:\n    kind: apk\n",
            "permission_sets: []\n",
            "targets: []\n",
        ] {
            let source = format!("{APP_HEADER}{fragment}");
            assert_eq!(
                parse_app_definition_yaml(&source).unwrap_err().code(),
                "app_definition_yaml_invalid",
                "{fragment}"
            );
        }
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
        app.package_id = "retroarch".to_string();
        app.configurations
            .get_mut("main")
            .unwrap()
            .templates
            .get_mut("default")
            .unwrap()
            .path = "RetroArch/default.cfg".to_string();
        let app_codes = codes(&app);
        assert_eq!(
            app_codes,
            [
                "authored_id_invalid",
                "authored_name_invalid",
                "package_id_invalid"
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

    #[test]
    fn duplicate_registry_ids_are_rejected_while_parsing() {
        let duplicate_artifact = format!(
            "{APP_HEADER}artifacts:
  apk:
    kind: apk
    source:
      strategy: user_provided
  apk:
    kind: file
    source:
      strategy: user_provided
"
        );
        assert_eq!(
            parse_app_definition_yaml(&duplicate_artifact)
                .unwrap_err()
                .code(),
            "app_definition_yaml_invalid"
        );

        let duplicate_target = format!(
            "{APP_HEADER}artifacts:
  apk:
    kind: apk
    source:
      strategy: user_provided
targets:
  system:
    kind: directory
    location: shared_storage
    path: App/system
  system:
    kind: directory
    location: shared_storage
    path: App/system2
"
        );
        assert_eq!(
            parse_app_definition_yaml(&duplicate_target)
                .unwrap_err()
                .code(),
            "app_definition_yaml_invalid"
        );
    }

    #[test]
    fn null_valued_cross_strategy_fields_are_rejected() {
        // An explicit null still supplies the key, so it cannot smuggle a
        // forbidden field past the per-strategy rejection.
        let user_provided = format!(
            "{APP_HEADER}artifacts:
  apk:
    kind: apk
    source:
      strategy: user_provided
      url: null
"
        );
        assert_eq!(
            parse_app_definition_yaml(&user_provided)
                .unwrap_err()
                .code(),
            "app_definition_yaml_invalid"
        );

        let direct_url = format!(
            "{APP_HEADER}artifacts:
  apk:
    kind: apk
    source:
      strategy: direct_url
      url: https://downloads.example.com/app.apk
      repository: null
"
        );
        assert_eq!(
            parse_app_definition_yaml(&direct_url).unwrap_err().code(),
            "app_definition_yaml_invalid"
        );

        // A null value for an optional field of the declared strategy still
        // means the same as an absent key.
        let optional_null = format!(
            "{APP_HEADER}artifacts:
  apk:
    kind: apk
    source:
      strategy: latest_release
      provider: github
      base_url: https://github.com
      repository: example/app
      asset_pattern: null
      prerelease: false
"
        );
        let parsed =
            parse_app_definition_yaml(&optional_null).expect("equivalent to an absent pattern");
        assert!(matches!(
            &parsed.artifacts["apk"].source,
            AppArtifactSource::LatestRelease {
                asset_pattern: None,
                ..
            }
        ));
        let emitted = emit_app_definition_yaml(&parsed).expect("canonical emission");
        assert!(!emitted.contains("asset_pattern"));
    }
}

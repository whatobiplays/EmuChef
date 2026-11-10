//! Deterministic, non-publishing rendering for one App Configuration.
//!
//! The renderer consumes an already-resolved set of Recipe inputs. It has no
//! device-write or publication operation: callers receive exact artifact bytes,
//! their SHA-256, and sanitized provenance for a later review boundary.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::authored_models::{
    self, AppConfigurationFormatV1, AppConfigurationV1, AppDefinitionV1, AppManagedPropertyV1,
    AppTargetKind, AppTargetV1, TemplateVariableTypeV1,
    MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT,
};
use crate::model::{InputDeclaration, Recipe, RecipeConfigOverrideV1, RecipeConfigurationBaseV1};
pub use crate::planner::{BindingSource, ResolvedInputBinding};
use crate::storage_targets::StorageTargetBinding;

const OUTPUT_VERSION: u8 = 3;
const AUTHORITY_FINGERPRINT_VERSION: u8 = 2;
const CATALOG_AUTHORITY_FINGERPRINT_VERSION: u8 = 2;

/// Current trusted device/session identity supplied by the Tauri authority.
///
/// Obtain this context for the current render attempt. A concrete device path or
/// Setup Location reference is rejected unless its session epoch matches.
#[derive(Clone, PartialEq, Eq)]
pub struct RenderAuthority {
    pub device_handle: String,
    pub device_plan: String,
    pub session_epoch: u64,
}

/// A Tauri-resolved Setup Location paired with the device session that issued it.
///
/// Build this value immediately from `device_storage::setup_location_bindings`.
/// The renderer checks both the device handle and session epoch before using its
/// device path.
#[derive(Clone, PartialEq, Eq)]
pub struct CurrentSetupLocationBinding {
    pub device_handle: String,
    pub binding: StorageTargetBinding,
}

impl fmt::Debug for CurrentSetupLocationBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentSetupLocationBinding")
            .field("device_handle", &"[redacted]")
            .field("binding_id", &self.binding.id)
            .field("session_epoch", &self.binding.session_epoch)
            .finish()
    }
}

impl fmt::Debug for RenderAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderAuthority")
            .field("device_handle", &"[redacted]")
            .field("device_plan", &self.device_plan)
            .field("session_epoch", &self.session_epoch)
            .finish()
    }
}

/// Trusted value and T11 provenance already resolved for a Recipe reset.
#[derive(Clone, PartialEq)]
pub struct ResolvedResetValue {
    pub value: Value,
    pub source: ResetValueSource,
}

impl fmt::Debug for ResolvedResetValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedResetValue")
            .field("value", &"[redacted]")
            .field("source", &self.source)
            .finish()
    }
}

/// Permitted origin labels for a pre-resolved reset value.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetValueSource {
    ExactModelRecommendation,
    BrandRecommendation,
    DeviceIndependentDefault,
}

impl ResetValueSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExactModelRecommendation => "exact_model_recommendation",
            Self::BrandRecommendation => "brand_recommendation",
            Self::DeviceIndependentDefault => "device_independent_default",
        }
    }
}

/// Inputs for one render of a Recipe's selected App Configuration.
pub struct ConfigurationRenderRequest<'a> {
    pub app: &'a AppDefinitionV1,
    pub recipe: &'a Recipe,
    /// Root containing the authored `apps/` directory.
    pub authored_root: &'a Path,
    /// Effective Recipe values returned by the existing input resolver.
    pub inputs: &'a [ResolvedInputBinding],
    /// Current Tauri device/session authority when a device path is involved.
    pub authority: Option<&'a RenderAuthority>,
    /// Current Tauri-resolved Setup Locations available to this render attempt.
    pub setup_locations: &'a [CurrentSetupLocationBinding],
    /// T11 values supplied for explicit Recipe `reset` operations.
    pub resolved_resets: &'a IndexMap<String, ResolvedResetValue>,
}

/// A sanitized failure that never includes an input value or source path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationRenderError {
    code: &'static str,
    field: String,
}

impl ConfigurationRenderError {
    fn new(code: &'static str, field: impl Into<String>) -> Self {
        Self {
            code,
            field: field.into(),
        }
    }

    /// Stable machine-readable failure code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Sanitized authored field or logical ID associated with the failure.
    pub fn field(&self) -> &str {
        &self.field
    }
}

impl fmt::Display for ConfigurationRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.code, self.field)
    }
}

impl std::error::Error for ConfigurationRenderError {}

/// Provenance for the selected template source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TemplateSourceProvenance {
    pub kind: String,
    pub id: String,
    pub input_source: Option<String>,
}

/// App-owned target identity included in the review record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenderTargetProvenance {
    pub id: String,
    pub kind: String,
    pub location: String,
    pub path: String,
}

/// A non-sensitive record of one template variable binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VariableBindingProvenance {
    pub variable_id: String,
    pub input_id: String,
    pub variable_type: String,
    pub input_source: String,
    pub sensitive: bool,
}

/// A non-sensitive record of one direct Recipe property operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ManagedPropertyOperationProvenance {
    pub property_id: String,
    pub operation: String,
    pub winner: String,
    pub reset_source: Option<String>,
}

/// A recorded collision between a template value and a direct property edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ManagedPropertyCollisionProvenance {
    pub property_id: String,
    pub winner: String,
}

/// Versioned identity for the validated App Definition authority used by a render.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CatalogAuthorityProvenance {
    pub fingerprint_version: u8,
    pub authority_sha256: String,
}

impl fmt::Debug for CatalogAuthorityProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogAuthorityProvenance")
            .field("fingerprint_version", &self.fingerprint_version)
            .field("authority_sha256", &"[redacted]")
            .finish()
    }
}

/// Current device binding identity without device paths or per-input values.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DeviceAuthorityProvenance {
    pub fingerprint_version: u8,
    pub session_epoch: u64,
    pub authority_sha256: String,
}

impl fmt::Debug for DeviceAuthorityProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceAuthorityProvenance")
            .field("fingerprint_version", &self.fingerprint_version)
            .field("session_epoch", &self.session_epoch)
            .field("authority_sha256", &"[redacted]")
            .finish()
    }
}

/// Stable source, target, binding, and edit information for review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigurationRenderProvenance {
    pub schema_version: u8,
    pub app_id: String,
    pub configuration_id: String,
    pub template_source: TemplateSourceProvenance,
    pub format: String,
    pub target: RenderTargetProvenance,
    pub catalog_authority: CatalogAuthorityProvenance,
    pub variables: Vec<VariableBindingProvenance>,
    pub managed_property_operations: Vec<ManagedPropertyOperationProvenance>,
    pub managed_property_collisions: Vec<ManagedPropertyCollisionProvenance>,
    pub device_authority: Option<DeviceAuthorityProvenance>,
}

/// Exact rendered artifact and its review identity.
///
/// `Debug` intentionally omits bytes because they may contain sensitive values.
pub struct RenderedConfiguration {
    bytes: Vec<u8>,
    sha256: String,
    provenance: ConfigurationRenderProvenance,
}

impl RenderedConfiguration {
    /// Exact deterministic output bytes. The caller must retain them in trusted storage.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Lowercase SHA-256 of the exact output bytes.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Sanitized, versioned source and target provenance.
    pub fn provenance(&self) -> &ConfigurationRenderProvenance {
        &self.provenance
    }
}

impl fmt::Debug for RenderedConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedConfiguration")
            .field("bytes", &"[redacted]")
            .field("sha256", &"[redacted]")
            .field("provenance", &"[redacted]")
            .finish()
    }
}

/// Render one selected template and return immutable review-bound bytes.
pub fn render_recipe_configuration(
    request: ConfigurationRenderRequest<'_>,
) -> Result<RenderedConfiguration, ConfigurationRenderError> {
    let recipe_config =
        request.recipe.config.as_ref().ok_or_else(|| {
            ConfigurationRenderError::new("render_configuration_missing", "config")
        })?;
    if request.app.id != recipe_config.app_ref {
        return Err(ConfigurationRenderError::new(
            "render_app_reference_mismatch",
            "config.app_ref",
        ));
    }
    if !authored_models::validate_app_definition(request.app).is_empty() {
        return Err(ConfigurationRenderError::new(
            "render_app_definition_invalid",
            "app_definition",
        ));
    }
    let catalog_errors = crate::catalog::validate_recipe_configuration(
        "recipe",
        request.recipe,
        std::slice::from_ref(request.app),
    );
    if !catalog_errors.is_empty() {
        return Err(ConfigurationRenderError::new(
            "render_recipe_configuration_invalid",
            "config",
        ));
    }
    let configuration = request
        .app
        .configurations
        .get(&recipe_config.configuration)
        .ok_or_else(|| {
            ConfigurationRenderError::new("render_configuration_unknown", "config.configuration")
        })?;
    if configuration.target != recipe_config.target {
        return Err(ConfigurationRenderError::new(
            "render_target_mismatch",
            "config.target",
        ));
    }
    let target = request
        .app
        .targets
        .get(&configuration.target)
        .filter(|target| target.kind == AppTargetKind::File)
        .ok_or_else(|| ConfigurationRenderError::new("render_target_invalid", "config.target"))?;

    let (template_path, template_source, declarations, catalog_relative_path) = match &recipe_config
        .base
    {
        RecipeConfigurationBaseV1::Template(template_id) => {
            if !recipe_config.input_template_variables.is_empty() {
                return Err(ConfigurationRenderError::new(
                    "render_variable_declaration_owner_invalid",
                    "config.base.variables",
                ));
            }
            let template = configuration.templates.get(template_id).ok_or_else(|| {
                ConfigurationRenderError::new("render_template_unknown", "config.base.template")
            })?;
            if !authored_models::is_valid_app_configuration_template_path(
                &request.app.id,
                &template.path,
            ) {
                return Err(ConfigurationRenderError::new(
                    "render_template_path_invalid",
                    "config.base.template",
                ));
            }
            let path = resolve_catalog_template_path(
                request.authored_root,
                &request.app.id,
                &template.path,
            )?;
            (
                path,
                TemplateSourceProvenance {
                    kind: "app_definition".to_string(),
                    id: template_id.clone(),
                    input_source: None,
                },
                &template.variables,
                Some(PathBuf::from("apps").join(&template.path)),
            )
        }
        RecipeConfigurationBaseV1::Input(input_id) => {
            let (declaration, resolved) = resolved_input(
                request.recipe,
                request.inputs,
                input_id,
                "config.base.input",
            )?;
            if declaration.multiple || !matches!(declaration.type_name.as_str(), "file" | "path") {
                return Err(ConfigurationRenderError::new(
                    "render_base_input_invalid",
                    "config.base.input",
                ));
            }
            let resolved_value = resolved.value.as_ref().ok_or_else(|| {
                ConfigurationRenderError::new("render_input_missing", "config.base.input")
            })?;
            let input_source = resolved.source.ok_or_else(|| {
                ConfigurationRenderError::new("render_input_source_missing", "config.base.input")
            })?;
            validate_input_value(input_id, declaration, resolved_value, "config.base.input")?;
            let path = resolved_value.as_str().ok_or_else(|| {
                ConfigurationRenderError::new("render_base_input_invalid", "config.base.input")
            })?;
            (
                PathBuf::from(path),
                TemplateSourceProvenance {
                    kind: "recipe_input".to_string(),
                    id: input_id.clone(),
                    input_source: Some(binding_source_name(input_source).to_string()),
                },
                &recipe_config.input_template_variables,
                None,
            )
        }
    };
    let capture_result = match &catalog_relative_path {
        Some(relative_path) => {
            crate::app_definition_persistence::capture_opened_app_definition_file_bounded_under_root(
                request.authored_root,
                relative_path,
                MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT,
            )
        }
        None => crate::app_definition_persistence::capture_opened_app_definition_file_bounded(
            &template_path,
            MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT,
        ),
    };
    let captured = capture_result.map_err(|error| {
        if error.code == "app_definition_file_too_large" {
            ConfigurationRenderError::new("render_template_too_large", "config.base")
        } else {
            ConfigurationRenderError::new("render_template_unavailable", "config.base")
        }
    })?;

    let mut document = parse_document(configuration.format, captured.raw_bytes())?;
    let references = document.variable_references()?;
    for variable_id in &references {
        if !declarations.contains_key(variable_id) {
            return Err(ConfigurationRenderError::new(
                "render_variable_undeclared",
                format!("config.variables[{variable_id}]"),
            ));
        }
        if !recipe_config.variables.contains_key(variable_id) {
            return Err(ConfigurationRenderError::new(
                "render_variable_binding_missing",
                format!("config.variables[{variable_id}]"),
            ));
        }
    }

    let mut resolved_variables: IndexMap<String, Cow<'_, Value>> = IndexMap::new();
    let mut variable_provenance = Vec::new();
    let mut used_locations = Vec::new();
    let mut has_device_path_variable = false;
    for (variable_id, input_id) in &recipe_config.variables {
        let variable_type = declarations.get(variable_id).ok_or_else(|| {
            ConfigurationRenderError::new(
                "render_variable_undeclared",
                format!("config.variables[{variable_id}]"),
            )
        })?;
        let (input_declaration, resolved) = resolved_input(
            request.recipe,
            request.inputs,
            input_id,
            &format!("config.variables[{variable_id}]"),
        )?;
        if input_declaration.multiple
            || !variable_type.accepts_input_type(&input_declaration.type_name)
        {
            return Err(ConfigurationRenderError::new(
                "render_variable_input_type_invalid",
                format!("config.variables[{variable_id}]"),
            ));
        }
        let resolved_value = resolved.value.as_ref().ok_or_else(|| {
            ConfigurationRenderError::new(
                "render_input_missing",
                format!("config.variables[{variable_id}]"),
            )
        })?;
        validate_input_value(
            input_id,
            input_declaration,
            resolved_value,
            &format!("config.variables[{variable_id}]"),
        )?;
        let value = if input_declaration.type_name == "setup_location_ref" {
            let location_id = resolved_value.as_str().ok_or_else(|| {
                ConfigurationRenderError::new(
                    "render_setup_location_invalid",
                    format!("config.variables[{variable_id}]"),
                )
            })?;
            let binding = resolve_setup_location(
                location_id,
                request.authority,
                request.setup_locations,
                &format!("config.variables[{variable_id}]"),
            )?;
            let path = binding.binding.destination_path().map_err(|_| {
                ConfigurationRenderError::new(
                    "render_setup_location_invalid",
                    format!("config.variables[{variable_id}]"),
                )
            })?;
            let path = path.to_str().ok_or_else(|| {
                ConfigurationRenderError::new(
                    "render_setup_location_invalid",
                    format!("config.variables[{variable_id}]"),
                )
            })?;
            if !authored_models::is_valid_device_path(path) {
                return Err(ConfigurationRenderError::new(
                    "render_device_path_invalid",
                    format!("config.variables[{variable_id}]"),
                ));
            }
            used_locations.push(binding.clone());
            Cow::Owned(Value::String(path.to_string()))
        } else {
            Cow::Borrowed(resolved_value)
        };
        if !variable_type.matches(value.as_ref()) {
            return Err(ConfigurationRenderError::new(
                "render_variable_value_invalid",
                format!("config.variables[{variable_id}]"),
            ));
        }
        if variable_type == &TemplateVariableTypeV1::DevicePath {
            has_device_path_variable = true;
        }
        resolved_variables.insert(variable_id.clone(), value);
        variable_provenance.push(VariableBindingProvenance {
            variable_id: variable_id.clone(),
            input_id: input_id.clone(),
            variable_type: variable_type.as_str().to_string(),
            input_source: binding_source_name(resolved.source.ok_or_else(|| {
                ConfigurationRenderError::new(
                    "render_input_source_missing",
                    format!("config.variables[{variable_id}]"),
                )
            })?)
            .to_string(),
            sensitive: input_declaration.sensitive,
        });
    }
    if has_device_path_variable {
        let authority = request.authority.ok_or_else(|| {
            ConfigurationRenderError::new("render_device_authority_missing", "config.variables")
        })?;
        validate_authority(authority)?;
    }

    document.substitute_bounded(
        configuration.format,
        &resolved_variables,
        MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT,
    )?;
    for literal in &recipe_config.unmanaged_literals {
        document.set_path(configuration.format, &literal.path, literal.value.clone())?;
    }
    validate_managed_values(&document, configuration)?;

    let mut operations = Vec::new();
    let mut collisions = Vec::new();
    for (property_id, exposure) in &recipe_config.properties {
        let Some(operation) = &exposure.override_operation else {
            continue;
        };
        let property = configuration
            .managed_properties
            .get(property_id)
            .ok_or_else(|| {
                ConfigurationRenderError::new(
                    "render_managed_property_unknown",
                    format!("config.properties[{property_id}]"),
                )
            })?;
        let collision = document.path_exists(configuration.format, &property.path)?;
        let (operation_name, reset_source) = match operation {
            RecipeConfigOverrideV1::Value(value) => {
                if !authored_models::managed_property_value_is_valid(property, value) {
                    return Err(ConfigurationRenderError::new(
                        "render_managed_value_invalid",
                        format!("config.properties[{property_id}].override.value"),
                    ));
                }
                document.set_path(configuration.format, &property.path, value.clone())?;
                ("value", None)
            }
            RecipeConfigOverrideV1::Clear => {
                if !property.clearable {
                    return Err(ConfigurationRenderError::new(
                        "render_clear_not_allowed",
                        format!("config.properties[{property_id}].override.clear"),
                    ));
                }
                document.remove_path(configuration.format, &property.path)?;
                ("clear", None)
            }
            RecipeConfigOverrideV1::Reset => {
                let reset = request.resolved_resets.get(property_id).ok_or_else(|| {
                    ConfigurationRenderError::new(
                        "render_reset_unresolved",
                        format!("config.properties[{property_id}].override.reset"),
                    )
                })?;
                if !authored_models::managed_property_value_is_valid(property, &reset.value) {
                    return Err(ConfigurationRenderError::new(
                        "render_reset_value_invalid",
                        format!("config.properties[{property_id}].override.reset"),
                    ));
                }
                document.set_path(configuration.format, &property.path, reset.value.clone())?;
                ("reset", Some(reset.source.as_str().to_string()))
            }
        };
        if collision {
            collisions.push(ManagedPropertyCollisionProvenance {
                property_id: property_id.clone(),
                winner: "recipe_operation".to_string(),
            });
        }
        operations.push(ManagedPropertyOperationProvenance {
            property_id: property_id.clone(),
            operation: operation_name.to_string(),
            winner: "recipe_operation".to_string(),
            reset_source,
        });
    }
    validate_managed_values(&document, configuration)?;
    for (property_id, exposure) in &recipe_config.properties {
        if exposure.required {
            let property = configuration
                .managed_properties
                .get(property_id)
                .ok_or_else(|| {
                    ConfigurationRenderError::new(
                        "render_managed_property_unknown",
                        format!("config.properties[{property_id}]"),
                    )
                })?;
            if !document.path_exists(configuration.format, &property.path)? {
                return Err(ConfigurationRenderError::new(
                    "render_required_managed_value_missing",
                    format!("config.properties[{property_id}]"),
                ));
            }
        }
    }

    variable_provenance.sort_by(|left, right| left.variable_id.cmp(&right.variable_id));
    operations.sort_by(|left, right| left.property_id.cmp(&right.property_id));
    collisions.sort_by(|left, right| left.property_id.cmp(&right.property_id));
    let device_authority = if has_device_path_variable {
        let authority = request.authority.ok_or_else(|| {
            ConfigurationRenderError::new("render_device_authority_missing", "config.variables")
        })?;
        validate_authority(authority)?;
        Some(DeviceAuthorityProvenance {
            fingerprint_version: AUTHORITY_FINGERPRINT_VERSION,
            session_epoch: authority.session_epoch,
            authority_sha256: authority_fingerprint(authority, &used_locations),
        })
    } else {
        None
    };
    let bytes = document.serialize_bounded(
        configuration.format,
        MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT,
    )?;
    let sha256 = digest_hex(Sha256::digest(&bytes).as_ref());
    let catalog_authority_sha256 =
        catalog_authority_fingerprint(CatalogAuthorityFingerprintInput {
            app_id: &request.app.id,
            package_id: &request.app.package_id,
            configuration_id: &recipe_config.configuration,
            configuration,
            target,
            template_source: &template_source,
            catalog_template_path: catalog_relative_path.as_deref(),
            declarations,
            exact_template_source: captured.raw_bytes(),
        })?;
    let provenance = ConfigurationRenderProvenance {
        schema_version: OUTPUT_VERSION,
        app_id: request.app.id.clone(),
        configuration_id: recipe_config.configuration.clone(),
        template_source,
        format: configuration.format.as_str().to_string(),
        target: RenderTargetProvenance {
            id: configuration.target.clone(),
            kind: target_kind_name(target.kind).to_string(),
            location: target_location_name(target.location).to_string(),
            path: target.path.clone(),
        },
        catalog_authority: CatalogAuthorityProvenance {
            fingerprint_version: CATALOG_AUTHORITY_FINGERPRINT_VERSION,
            authority_sha256: catalog_authority_sha256,
        },
        variables: variable_provenance,
        managed_property_operations: operations,
        managed_property_collisions: collisions,
        device_authority,
    };
    Ok(RenderedConfiguration {
        bytes,
        sha256,
        provenance,
    })
}

fn resolve_catalog_template_path(
    authored_root: &Path,
    app_id: &str,
    template_path: &str,
) -> Result<PathBuf, ConfigurationRenderError> {
    let unavailable =
        || ConfigurationRenderError::new("render_template_unavailable", "config.base.template");
    let root = authored_root.canonicalize().map_err(|_| unavailable())?;
    let apps_directory = root.join("apps");
    let apps_metadata = fs::symlink_metadata(&apps_directory).map_err(|_| unavailable())?;
    if apps_metadata.file_type().is_symlink() || !apps_metadata.is_dir() {
        return Err(unavailable());
    }

    let app_directory = apps_directory.join(app_id);
    let app_metadata = fs::symlink_metadata(&app_directory).map_err(|_| unavailable())?;
    if app_metadata.file_type().is_symlink() || !app_metadata.is_dir() {
        return Err(unavailable());
    }

    let mut components = Path::new(template_path).components();
    match components.next() {
        Some(Component::Normal(component)) if component == std::ffi::OsStr::new(app_id) => {}
        _ => return Err(unavailable()),
    }

    let mut candidate = app_directory.clone();
    let mut remaining = components.peekable();
    if remaining.peek().is_none() {
        return Err(unavailable());
    }
    while let Some(component) = remaining.next() {
        let Component::Normal(name) = component else {
            return Err(unavailable());
        };
        candidate.push(name);
        let metadata = fs::symlink_metadata(&candidate).map_err(|_| unavailable())?;
        if metadata.file_type().is_symlink()
            || if remaining.peek().is_some() {
                !metadata.is_dir()
            } else {
                !metadata.is_file()
            }
        {
            return Err(unavailable());
        }
        if remaining.peek().is_none()
            && metadata.len() > MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT as u64
        {
            return Err(ConfigurationRenderError::new(
                "render_template_too_large",
                "config.base",
            ));
        }
    }

    let canonical_app_directory = app_directory.canonicalize().map_err(|_| unavailable())?;
    let canonical_template = candidate.canonicalize().map_err(|_| unavailable())?;
    if !canonical_template.starts_with(&canonical_app_directory)
        || !fs::metadata(&canonical_template)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
    {
        return Err(unavailable());
    }
    Ok(canonical_template)
}

fn target_kind_name(kind: AppTargetKind) -> &'static str {
    match kind {
        AppTargetKind::File => "file",
        AppTargetKind::Directory => "directory",
    }
}

fn target_location_name(location: crate::authored_models::AppTargetLocation) -> &'static str {
    use crate::authored_models::AppTargetLocation;
    match location {
        AppTargetLocation::AppData => "app_data",
        AppTargetLocation::ExternalAppData => "external_app_data",
        AppTargetLocation::SharedStorage => "shared_storage",
        AppTargetLocation::AbsoluteDevicePath => "absolute_device_path",
    }
}

fn resolved_input<'a>(
    recipe: &'a Recipe,
    inputs: &'a [ResolvedInputBinding],
    input_id: &str,
    field: &str,
) -> Result<(&'a InputDeclaration, &'a ResolvedInputBinding), ConfigurationRenderError> {
    let declaration = recipe
        .inputs
        .get(input_id)
        .ok_or_else(|| ConfigurationRenderError::new("render_input_unknown", field.to_string()))?;
    let mut matches = inputs
        .iter()
        .filter(|input| input.recipe_id == recipe.id && input.input_id == input_id);
    let resolved = matches
        .next()
        .ok_or_else(|| ConfigurationRenderError::new("render_input_missing", field.to_string()))?;
    if matches.next().is_some() {
        return Err(ConfigurationRenderError::new(
            "render_input_duplicate",
            field.to_string(),
        ));
    }
    if resolved.key != format!("{}/{}", recipe.id, input_id)
        || resolved.type_name != declaration.type_name
    {
        return Err(ConfigurationRenderError::new(
            "render_input_declaration_mismatch",
            field.to_string(),
        ));
    }
    Ok((declaration, resolved))
}

fn binding_source_name(source: BindingSource) -> &'static str {
    match source {
        BindingSource::Explicit => "explicit",
        BindingSource::UserConfiguration => "user_configuration",
        BindingSource::DevicePlan => "device_plan",
        BindingSource::RecipeDefault => "recipe_default",
    }
}

fn validate_input_value(
    input_id: &str,
    declaration: &InputDeclaration,
    value: &Value,
    field: &str,
) -> Result<(), ConfigurationRenderError> {
    if declaration.multiple
        || !declaration.value_matches_type(value)
        || !crate::planner::validate_binding_value(input_id, declaration, value).is_empty()
    {
        return Err(ConfigurationRenderError::new(
            "render_input_value_invalid",
            field.to_string(),
        ));
    }
    Ok(())
}

fn validate_authority(authority: &RenderAuthority) -> Result<(), ConfigurationRenderError> {
    if authority.device_handle.trim().is_empty()
        || authority.device_handle.len() > 256
        || authority.device_handle.chars().any(char::is_control)
        || !authored_models::is_valid_identifier(&authority.device_plan)
        || authority.session_epoch == 0
    {
        return Err(ConfigurationRenderError::new(
            "render_device_authority_invalid",
            "device_authority",
        ));
    }
    Ok(())
}

fn resolve_setup_location<'a>(
    location_id: &str,
    authority: Option<&RenderAuthority>,
    locations: &'a [CurrentSetupLocationBinding],
    field: &str,
) -> Result<&'a CurrentSetupLocationBinding, ConfigurationRenderError> {
    let authority = authority.ok_or_else(|| {
        ConfigurationRenderError::new("render_device_authority_missing", field.to_string())
    })?;
    validate_authority(authority)?;
    let mut matches = locations
        .iter()
        .filter(|location| location.binding.id == location_id);
    let location = matches.next().ok_or_else(|| {
        ConfigurationRenderError::new("render_setup_location_unknown", field.to_string())
    })?;
    if matches.next().is_some() {
        return Err(ConfigurationRenderError::new(
            "render_setup_location_duplicate",
            field.to_string(),
        ));
    }
    location.binding.validate().map_err(|_| {
        ConfigurationRenderError::new("render_setup_location_invalid", field.to_string())
    })?;
    if location.device_handle != authority.device_handle
        || location.binding.session_epoch != authority.session_epoch
        || location.binding.device_plan != authority.device_plan
    {
        return Err(ConfigurationRenderError::new(
            "render_setup_location_stale",
            field.to_string(),
        ));
    }
    Ok(location)
}

fn authority_fingerprint(
    authority: &RenderAuthority,
    locations: &[CurrentSetupLocationBinding],
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"emuchef-render-authority-v2\0");
    hash_authority_field(&mut hash, authority.device_handle.as_bytes());
    hash_authority_field(&mut hash, authority.device_plan.as_bytes());
    hash_authority_field(&mut hash, &authority.session_epoch.to_be_bytes());
    let mut locations = locations.iter().collect::<Vec<_>>();
    locations.sort_by(|left, right| left.binding.id.cmp(&right.binding.id));
    hash_authority_field(&mut hash, &(locations.len() as u64).to_be_bytes());
    for location in locations {
        let binding = &location.binding;
        hash_authority_field(&mut hash, binding.id.as_bytes());
        hash_authority_field(&mut hash, binding.device_plan.as_bytes());
        hash_authority_field(&mut hash, &binding.session_epoch.to_be_bytes());
        hash_authority_field(&mut hash, binding.name.as_bytes());
        hash_authority_field(&mut hash, binding.relative_path.as_bytes());
        hash_authority_field(&mut hash, binding.volume_handle.as_bytes());
        hash_authority_field(&mut hash, volume_kind_name(binding.volume_kind).as_bytes());
        hash_authority_field(&mut hash, binding.mount_root.as_bytes());
        hash_authority_field(&mut hash, binding.volume_identity.as_bytes());
        hash_authority_field(&mut hash, binding.volume_continuity.as_bytes());
        hash_authority_field(&mut hash, binding.observation_token.as_bytes());
    }
    digest_hex(hash.finalize().as_ref())
}

#[derive(Serialize)]
struct CatalogAuthorityMaterial<'a> {
    fingerprint_version: u8,
    app_id: &'a str,
    package_id: &'a str,
    configuration_id: &'a str,
    format: &'static str,
    target_id: &'a str,
    target: &'a AppTargetV1,
    managed_properties: BTreeMap<&'a str, &'a AppManagedPropertyV1>,
    template_source_kind: &'a str,
    template_source_id: &'a str,
    catalog_template_path: Option<&'a str>,
    template_variables: BTreeMap<&'a str, TemplateVariableTypeV1>,
}

struct CatalogAuthorityFingerprintInput<'a> {
    app_id: &'a str,
    package_id: &'a str,
    configuration_id: &'a str,
    configuration: &'a AppConfigurationV1,
    target: &'a AppTargetV1,
    template_source: &'a TemplateSourceProvenance,
    catalog_template_path: Option<&'a Path>,
    declarations: &'a IndexMap<String, TemplateVariableTypeV1>,
    exact_template_source: &'a [u8],
}

fn catalog_authority_fingerprint(
    input: CatalogAuthorityFingerprintInput<'_>,
) -> Result<String, ConfigurationRenderError> {
    let CatalogAuthorityFingerprintInput {
        app_id,
        package_id,
        configuration_id,
        configuration,
        target,
        template_source,
        catalog_template_path,
        declarations,
        exact_template_source,
    } = input;
    let managed_properties = configuration
        .managed_properties
        .iter()
        .map(|(property_id, property)| (property_id.as_str(), property))
        .collect();
    let template_variables = declarations
        .iter()
        .map(|(variable_id, variable_type)| (variable_id.as_str(), *variable_type))
        .collect();
    let material = CatalogAuthorityMaterial {
        fingerprint_version: CATALOG_AUTHORITY_FINGERPRINT_VERSION,
        app_id,
        package_id,
        configuration_id,
        format: configuration.format.as_str(),
        target_id: &configuration.target,
        target,
        managed_properties,
        template_source_kind: &template_source.kind,
        template_source_id: &template_source.id,
        catalog_template_path: catalog_template_path.and_then(Path::to_str),
        template_variables,
    };
    let encoded_material = serde_json::to_vec(&material).map_err(|_| {
        ConfigurationRenderError::new("render_authority_serialization_failed", "app_definition")
    })?;
    let mut hash = Sha256::new();
    hash.update(b"emuchef-catalog-render-authority-v2\0");
    hash_authority_field(&mut hash, &encoded_material);
    hash_authority_field(&mut hash, exact_template_source);
    Ok(digest_hex(hash.finalize().as_ref()))
}

fn hash_authority_field(hash: &mut Sha256, field: &[u8]) {
    hash.update((field.len() as u64).to_be_bytes());
    hash.update(field);
}

fn volume_kind_name(kind: crate::storage_targets::StorageTargetVolumeKind) -> &'static str {
    match kind {
        crate::storage_targets::StorageTargetVolumeKind::InternalShared => "internal_shared",
        crate::storage_targets::StorageTargetVolumeKind::RemovableSd => "removable_sd",
    }
}

fn digest_hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn parse_document(
    format: AppConfigurationFormatV1,
    bytes: &[u8],
) -> Result<ConfigurationDocument, ConfigurationRenderError> {
    if bytes.len() > MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT {
        return Err(ConfigurationRenderError::new(
            "render_template_too_large",
            "config.base",
        ));
    }
    match format {
        AppConfigurationFormatV1::KeyValueV1 => parse_key_value_document(bytes),
        AppConfigurationFormatV1::JsonV1 => parse_json_document(bytes),
    }
}

#[derive(Clone, Debug)]
enum ConfigurationDocument {
    KeyValue(KeyValueDocument),
    Json(Value),
}

#[derive(Clone, Debug, Default)]
struct KeyValueDocument {
    comments: Vec<String>,
    entries: BTreeMap<String, Value>,
}

impl ConfigurationDocument {
    fn variable_references(&self) -> Result<HashSet<String>, ConfigurationRenderError> {
        let mut references = HashSet::new();
        match self {
            Self::KeyValue(document) => {
                for comment in &document.comments {
                    reject_placeholder_position(comment)?;
                }
                for (key, value) in &document.entries {
                    reject_placeholder_position(key)?;
                    collect_value_references(value, &mut references)?;
                }
            }
            Self::Json(value) => collect_json_references(value, &mut references)?,
        }
        Ok(references)
    }

    fn substitute(
        &mut self,
        values: &IndexMap<String, Cow<'_, Value>>,
    ) -> Result<(), ConfigurationRenderError> {
        match self {
            Self::KeyValue(document) => {
                for value in document.entries.values_mut() {
                    substitute_value(value, values)?;
                }
            }
            Self::Json(value) => substitute_json(value, values)?,
        }
        Ok(())
    }

    fn substitute_bounded(
        &mut self,
        format: AppConfigurationFormatV1,
        values: &IndexMap<String, Cow<'_, Value>>,
        maximum_bytes: usize,
    ) -> Result<(), ConfigurationRenderError> {
        self.measure_serialized_size(format, Some(values), maximum_bytes)?;
        self.substitute(values)
    }

    fn serialize_bounded(
        &self,
        format: AppConfigurationFormatV1,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, ConfigurationRenderError> {
        self.measure_serialized_size(format, None, maximum_bytes)?;
        self.serialize(format)
    }

    fn measure_serialized_size(
        &self,
        format: AppConfigurationFormatV1,
        variables: Option<&IndexMap<String, Cow<'_, Value>>>,
        maximum_bytes: usize,
    ) -> Result<usize, ConfigurationRenderError> {
        let mut output = SerializedSizeCounter::new(maximum_bytes);
        match (self, format) {
            (Self::KeyValue(document), AppConfigurationFormatV1::KeyValueV1) => {
                for comment in &document.comments {
                    output.add_bytes(comment.as_bytes())?;
                    output.add_bytes(b"\n")?;
                }
                for (key, value) in &document.entries {
                    output.add_bytes(key.as_bytes())?;
                    output.add_bytes(b"=")?;
                    let value = if let (Some(variables), Value::String(text)) = (variables, value) {
                        match placeholder(text)? {
                            Some(variable_id) => variables
                                .get(&variable_id)
                                .map(Cow::as_ref)
                                .ok_or_else(|| {
                                    ConfigurationRenderError::new(
                                        "render_variable_binding_missing",
                                        format!("config.variables[{variable_id}]"),
                                    )
                                })?,
                            None => value,
                        }
                    } else {
                        value
                    };
                    if !is_key_value_scalar(value) {
                        return Err(ConfigurationRenderError::new(
                            "render_key_value_value_invalid",
                            "config.base",
                        ));
                    }
                    output.add_json(value)?;
                    output.add_bytes(b"\n")?;
                }
            }
            (Self::Json(value), AppConfigurationFormatV1::JsonV1) => {
                count_json_value(value, variables, &mut output)?;
                output.add_bytes(b"\n")?;
            }
            _ => {
                return Err(ConfigurationRenderError::new(
                    "render_format_mismatch",
                    "config.format",
                ));
            }
        }
        Ok(output.bytes)
    }

    fn path_exists(
        &self,
        format: AppConfigurationFormatV1,
        path: &[String],
    ) -> Result<bool, ConfigurationRenderError> {
        match (self, format) {
            (Self::KeyValue(document), AppConfigurationFormatV1::KeyValueV1) => {
                let [key] = path else {
                    return Err(ConfigurationRenderError::new(
                        "render_managed_path_invalid",
                        "managed_property.path",
                    ));
                };
                Ok(document.entries.contains_key(key))
            }
            (Self::Json(value), AppConfigurationFormatV1::JsonV1) => {
                Ok(json_path_get(value, path).is_some())
            }
            _ => Err(ConfigurationRenderError::new(
                "render_format_mismatch",
                "config.format",
            )),
        }
    }

    fn set_path(
        &mut self,
        format: AppConfigurationFormatV1,
        path: &[String],
        value: Value,
    ) -> Result<(), ConfigurationRenderError> {
        match (self, format) {
            (Self::KeyValue(document), AppConfigurationFormatV1::KeyValueV1) => {
                let [key] = path else {
                    return Err(ConfigurationRenderError::new(
                        "render_managed_path_invalid",
                        "managed_property.path",
                    ));
                };
                if !is_key_value_key_safe(key) {
                    return Err(ConfigurationRenderError::new(
                        "render_key_value_path_invalid",
                        "managed_property.path",
                    ));
                }
                if !is_key_value_scalar(&value) {
                    return Err(ConfigurationRenderError::new(
                        "render_key_value_value_invalid",
                        "managed_property.path",
                    ));
                }
                document.entries.insert(key.clone(), value);
            }
            (Self::Json(document), AppConfigurationFormatV1::JsonV1) => {
                json_path_set(document, path, value)?;
            }
            _ => {
                return Err(ConfigurationRenderError::new(
                    "render_format_mismatch",
                    "config.format",
                ))
            }
        }
        Ok(())
    }

    fn remove_path(
        &mut self,
        format: AppConfigurationFormatV1,
        path: &[String],
    ) -> Result<(), ConfigurationRenderError> {
        match (self, format) {
            (Self::KeyValue(document), AppConfigurationFormatV1::KeyValueV1) => {
                let [key] = path else {
                    return Err(ConfigurationRenderError::new(
                        "render_managed_path_invalid",
                        "managed_property.path",
                    ));
                };
                document.entries.remove(key);
            }
            (Self::Json(document), AppConfigurationFormatV1::JsonV1) => {
                json_path_remove(document, path)?;
            }
            _ => {
                return Err(ConfigurationRenderError::new(
                    "render_format_mismatch",
                    "config.format",
                ))
            }
        }
        Ok(())
    }

    fn serialize(
        &self,
        format: AppConfigurationFormatV1,
    ) -> Result<Vec<u8>, ConfigurationRenderError> {
        match (self, format) {
            (Self::KeyValue(document), AppConfigurationFormatV1::KeyValueV1) => {
                let mut output = String::new();
                for comment in &document.comments {
                    output.push_str(comment);
                    output.push('\n');
                }
                for (key, value) in &document.entries {
                    output.push_str(key);
                    output.push('=');
                    output.push_str(&encode_key_value_scalar(value)?);
                    output.push('\n');
                }
                Ok(output.into_bytes())
            }
            (Self::Json(value), AppConfigurationFormatV1::JsonV1) => {
                let mut output = Vec::new();
                write_canonical_json(value, &mut output)?;
                output.push(b'\n');
                Ok(output)
            }
            _ => Err(ConfigurationRenderError::new(
                "render_format_mismatch",
                "config.format",
            )),
        }
    }
}

struct SerializedSizeCounter {
    bytes: usize,
    maximum_bytes: usize,
    exceeded: bool,
}

impl SerializedSizeCounter {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            bytes: 0,
            maximum_bytes,
            exceeded: false,
        }
    }

    fn add_bytes(&mut self, bytes: &[u8]) -> Result<(), ConfigurationRenderError> {
        if self.write_all(bytes).is_err() {
            return Err(self.error());
        }
        Ok(())
    }

    fn add_json<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), ConfigurationRenderError> {
        if serde_json::to_writer(&mut *self, value).is_err() {
            return Err(self.error());
        }
        Ok(())
    }

    fn error(&self) -> ConfigurationRenderError {
        if self.exceeded {
            ConfigurationRenderError::new("render_expanded_configuration_too_large", "config.base")
        } else {
            ConfigurationRenderError::new("render_serialization_failed", "config")
        }
    }
}

impl Write for SerializedSizeCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next_size) = self.bytes.checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("serialized size overflow"));
        };
        if next_size > self.maximum_bytes {
            self.exceeded = true;
            return Err(io::Error::other("serialized size limit exceeded"));
        }
        self.bytes = next_size;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn count_json_value(
    value: &Value,
    variables: Option<&IndexMap<String, Cow<'_, Value>>>,
    output: &mut SerializedSizeCounter,
) -> Result<(), ConfigurationRenderError> {
    if let (Some(variables), Value::String(text)) = (variables, value) {
        if let Some(variable_id) = placeholder(text)? {
            let replacement = variables.get(&variable_id).ok_or_else(|| {
                ConfigurationRenderError::new(
                    "render_variable_binding_missing",
                    format!("config.variables[{variable_id}]"),
                )
            })?;
            return count_json_value(replacement.as_ref(), None, output);
        }
    }

    match value {
        Value::Null => output.add_bytes(b"null"),
        Value::Bool(false) => output.add_bytes(b"false"),
        Value::Bool(true) => output.add_bytes(b"true"),
        Value::Number(number) => output.add_bytes(number.to_string().as_bytes()),
        Value::String(value) => output.add_json(value),
        Value::Array(items) => {
            output.add_bytes(b"[")?;
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.add_bytes(b",")?;
                }
                count_json_value(item, variables, output)?;
            }
            output.add_bytes(b"]")
        }
        Value::Object(object) => {
            output.add_bytes(b"{")?;
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.add_bytes(b",")?;
                }
                reject_placeholder_position(key)?;
                output.add_json(key)?;
                output.add_bytes(b":")?;
                count_json_value(&object[*key], variables, output)?;
            }
            output.add_bytes(b"}")
        }
    }
}

fn parse_key_value_document(
    bytes: &[u8],
) -> Result<ConfigurationDocument, ConfigurationRenderError> {
    let source = std::str::from_utf8(bytes).map_err(|_| {
        ConfigurationRenderError::new("render_template_encoding_unsupported", "config.base")
    })?;
    let mut document = KeyValueDocument::default();
    for line in source.lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if line.chars().any(char::is_control) {
                return Err(ConfigurationRenderError::new(
                    "render_template_malformed",
                    "config.base",
                ));
            }
            document.comments.push(line.to_string());
            continue;
        }
        if line.chars().any(char::is_control) {
            return Err(ConfigurationRenderError::new(
                "render_template_malformed",
                "config.base",
            ));
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(ConfigurationRenderError::new(
                "render_template_malformed",
                "config.base",
            ));
        };
        if !is_key_value_key_safe(key) {
            return Err(ConfigurationRenderError::new(
                "render_template_malformed",
                "config.base",
            ));
        }
        let value = parse_key_value_scalar(raw_value)?;
        if document.entries.insert(key.to_string(), value).is_some() {
            return Err(ConfigurationRenderError::new(
                "render_template_duplicate_key",
                "config.base",
            ));
        }
    }
    Ok(ConfigurationDocument::KeyValue(document))
}

fn is_key_value_key_safe(key: &str) -> bool {
    !key.starts_with('#')
        && authored_models::configuration_property_path_tokens(
            AppConfigurationFormatV1::KeyValueV1,
            &[key.to_string()],
        )
        .is_some()
}

fn parse_key_value_scalar(raw: &str) -> Result<Value, ConfigurationRenderError> {
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw)
            .map(Value::String)
            .map_err(|_| {
                ConfigurationRenderError::new("render_template_malformed", "config.base")
            });
    }
    if matches!(raw, "true" | "false" | "null")
        || raw.as_bytes().first().is_some_and(u8::is_ascii_digit)
        || raw.starts_with('-')
    {
        return serde_json::from_str::<Value>(raw).map_err(|_| {
            ConfigurationRenderError::new("render_template_malformed", "config.base")
        });
    }
    if raw.chars().any(char::is_control) || raw.contains(['#', '=']) {
        return Err(ConfigurationRenderError::new(
            "render_template_malformed",
            "config.base",
        ));
    }
    Ok(Value::String(raw.to_string()))
}

fn is_key_value_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
    )
}

fn encode_key_value_scalar(value: &Value) -> Result<String, ConfigurationRenderError> {
    if !is_key_value_scalar(value) {
        return Err(ConfigurationRenderError::new(
            "render_key_value_value_invalid",
            "config.base",
        ));
    }
    serde_json::to_string(value)
        .map_err(|_| ConfigurationRenderError::new("render_key_value_value_invalid", "config.base"))
}

fn parse_json_document(bytes: &[u8]) -> Result<ConfigurationDocument, ConfigurationRenderError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let StrictJson(value) = StrictJson::deserialize(&mut deserializer)
        .map_err(|_| ConfigurationRenderError::new("render_template_malformed", "config.base"))?;
    deserializer
        .end()
        .map_err(|_| ConfigurationRenderError::new("render_template_malformed", "config.base"))?;
    if !value.is_object() {
        return Err(ConfigurationRenderError::new(
            "render_template_root_invalid",
            "config.base",
        ));
    }
    Ok(ConfigurationDocument::Json(value))
}

struct StrictJson(Value);

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_unit()
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictJson)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictJson(Value::String(value)))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(StrictJson(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictJson(Value::Array(values)))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = mapping.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            let StrictJson(value) = mapping.next_value()?;
            values.insert(key, value);
        }
        Ok(StrictJson(Value::Object(values)))
    }
}

fn collect_value_references(
    value: &Value,
    references: &mut HashSet<String>,
) -> Result<(), ConfigurationRenderError> {
    if let Value::String(value) = value {
        if let Some(variable_id) = placeholder(value)? {
            references.insert(variable_id);
        }
    }
    Ok(())
}

fn collect_json_references(
    value: &Value,
    references: &mut HashSet<String>,
) -> Result<(), ConfigurationRenderError> {
    match value {
        Value::String(value) => collect_value_references(&Value::String(value.clone()), references),
        Value::Array(items) => {
            for item in items {
                collect_json_references(item, references)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            for (key, value) in object {
                reject_placeholder_position(key)?;
                collect_json_references(value, references)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn placeholder(value: &str) -> Result<Option<String>, ConfigurationRenderError> {
    if !contains_template_delimiter(value) {
        return Ok(None);
    }
    let Some(body) = value
        .strip_prefix("{{")
        .and_then(|text| text.strip_suffix("}}"))
    else {
        if has_complete_template_expression(value) {
            return Err(ConfigurationRenderError::new(
                "render_placeholder_position_invalid",
                "config.base",
            ));
        }
        return Err(ConfigurationRenderError::new(
            "render_placeholder_malformed",
            "config.base",
        ));
    };
    if body.contains("{{") || body.contains("}}") {
        return Err(ConfigurationRenderError::new(
            "render_placeholder_malformed",
            "config.base",
        ));
    }
    let Some((namespace, variable_id)) = body.split_once(':') else {
        return Err(ConfigurationRenderError::new(
            "render_placeholder_malformed",
            "config.base",
        ));
    };
    if namespace != "emuchef" {
        return Err(ConfigurationRenderError::new(
            "render_placeholder_namespace_unsupported",
            "config.base",
        ));
    }
    if variable_id.is_empty() || !authored_models::is_valid_template_variable_id(variable_id) {
        return Err(ConfigurationRenderError::new(
            "render_placeholder_malformed",
            "config.base",
        ));
    }
    Ok(Some(variable_id.to_string()))
}

fn contains_template_delimiter(value: &str) -> bool {
    value.contains("{{") || value.contains("}}")
}

fn has_complete_template_expression(value: &str) -> bool {
    value
        .find("{{")
        .is_some_and(|start| value[start + 2..].find("}}").is_some())
}

fn reject_placeholder_position(value: &str) -> Result<(), ConfigurationRenderError> {
    if contains_template_delimiter(value) {
        return Err(ConfigurationRenderError::new(
            "render_placeholder_position_invalid",
            "config.base",
        ));
    }
    Ok(())
}

fn substitute_value(
    value: &mut Value,
    variables: &IndexMap<String, Cow<'_, Value>>,
) -> Result<(), ConfigurationRenderError> {
    if let Value::String(text) = value {
        if let Some(variable_id) = placeholder(text)? {
            *value = variables
                .get(&variable_id)
                .map(|value| value.as_ref().clone())
                .ok_or_else(|| {
                    ConfigurationRenderError::new(
                        "render_variable_binding_missing",
                        format!("config.variables[{variable_id}]"),
                    )
                })?;
        }
    }
    Ok(())
}

fn substitute_json(
    value: &mut Value,
    variables: &IndexMap<String, Cow<'_, Value>>,
) -> Result<(), ConfigurationRenderError> {
    match value {
        Value::String(_) => substitute_value(value, variables),
        Value::Array(items) => {
            for item in items {
                substitute_json(item, variables)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                reject_placeholder_position(key)?;
                substitute_json(value, variables)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_managed_values(
    document: &ConfigurationDocument,
    configuration: &AppConfigurationV1,
) -> Result<(), ConfigurationRenderError> {
    for (property_id, property) in &configuration.managed_properties {
        let value = match document {
            ConfigurationDocument::KeyValue(document) => {
                let [key] = property.path.as_slice() else {
                    return Err(ConfigurationRenderError::new(
                        "render_managed_path_invalid",
                        format!("managed_properties[{property_id}].path"),
                    ));
                };
                document.entries.get(key)
            }
            ConfigurationDocument::Json(document) => json_path_get(document, &property.path),
        };
        if value
            .is_some_and(|value| !authored_models::managed_property_value_is_valid(property, value))
        {
            return Err(ConfigurationRenderError::new(
                "render_managed_value_invalid",
                format!("managed_properties[{property_id}]"),
            ));
        }
    }
    Ok(())
}

fn json_path_get<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    let child = value.as_object()?.get(first)?;
    if rest.is_empty() {
        Some(child)
    } else {
        json_path_get(child, rest)
    }
}

fn json_path_set(
    value: &mut Value,
    path: &[String],
    replacement: Value,
) -> Result<(), ConfigurationRenderError> {
    let (key, rest) = path.split_first().ok_or_else(|| {
        ConfigurationRenderError::new("render_managed_path_invalid", "managed_property.path")
    })?;
    let object = value.as_object_mut().ok_or_else(|| {
        ConfigurationRenderError::new("render_managed_path_conflict", "managed_property.path")
    })?;
    if rest.is_empty() {
        object.insert(key.clone(), replacement);
        return Ok(());
    }
    let child = object
        .entry(key.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    json_path_set(child, rest, replacement)
}

fn json_path_remove(value: &mut Value, path: &[String]) -> Result<(), ConfigurationRenderError> {
    let (key, rest) = path.split_first().ok_or_else(|| {
        ConfigurationRenderError::new("render_managed_path_invalid", "managed_property.path")
    })?;
    let Some(object) = value.as_object_mut() else {
        return Err(ConfigurationRenderError::new(
            "render_managed_path_conflict",
            "managed_property.path",
        ));
    };
    if rest.is_empty() {
        object.remove(key);
        return Ok(());
    }
    match object.get_mut(key) {
        None => Ok(()),
        Some(child) if child.is_object() => json_path_remove(child, rest),
        Some(_) => Err(ConfigurationRenderError::new(
            "render_managed_path_conflict",
            "managed_property.path",
        )),
    }
}

fn write_canonical_json(
    value: &Value,
    output: &mut Vec<u8>,
) -> Result<(), ConfigurationRenderError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => output.extend_from_slice(
            serde_json::to_string(value)
                .map_err(|_| {
                    ConfigurationRenderError::new("render_serialization_failed", "config")
                })?
                .as_bytes(),
        ),
        Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(object) => {
            output.push(b'{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|_| {
                            ConfigurationRenderError::new("render_serialization_failed", "config")
                        })?
                        .as_bytes(),
                );
                output.push(b':');
                write_canonical_json(&object[*key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod expansion_tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn json_expansion_accepts_the_limit_and_rejects_aggregate_overflow_before_substitution() {
        let value = Value::String("repeat".to_string());
        let variables = IndexMap::from([("value".to_string(), Cow::Borrowed(&value))]);
        let expected = b"{\"left\":\"repeat\",\"right\":\"repeat\"}\n";
        let document_value = || {
            ConfigurationDocument::Json(serde_json::json!({
                "left": "{{emuchef:value}}",
                "right": "{{emuchef:value}}",
            }))
        };

        let mut exact = document_value();
        exact
            .substitute_bounded(AppConfigurationFormatV1::JsonV1, &variables, expected.len())
            .expect("exact serialized output limit should be accepted");
        assert_eq!(
            exact.serialize(AppConfigurationFormatV1::JsonV1).unwrap(),
            expected
        );

        let mut oversized = document_value();
        let error = oversized
            .substitute_bounded(
                AppConfigurationFormatV1::JsonV1,
                &variables,
                expected.len() - 1,
            )
            .expect_err("aggregate repeated expansion must exceed the smaller limit");
        assert_eq!(error.code(), "render_expanded_configuration_too_large");
        assert!(oversized.variable_references().unwrap().contains("value"));
    }

    #[test]
    fn key_value_expansion_counts_json_scalar_escaping_at_the_limit() {
        let value = Value::String("quote\"\nline".to_string());
        let encoded = serde_json::to_string(&value).unwrap();
        let expected = format!("first={encoded}\nsecond={encoded}\n");
        let variables = IndexMap::from([("value".to_string(), Cow::Borrowed(&value))]);
        let document_value = || {
            ConfigurationDocument::KeyValue(KeyValueDocument {
                comments: Vec::new(),
                entries: BTreeMap::from([
                    (
                        "first".to_string(),
                        Value::String("{{emuchef:value}}".to_string()),
                    ),
                    (
                        "second".to_string(),
                        Value::String("{{emuchef:value}}".to_string()),
                    ),
                ]),
            })
        };

        let mut exact = document_value();
        exact
            .substitute_bounded(
                AppConfigurationFormatV1::KeyValueV1,
                &variables,
                expected.len(),
            )
            .expect("escaped key-value scalars should fit at the exact output limit");
        assert_eq!(
            exact
                .serialize(AppConfigurationFormatV1::KeyValueV1)
                .unwrap(),
            expected.as_bytes()
        );

        let mut oversized = document_value();
        let error = oversized
            .substitute_bounded(
                AppConfigurationFormatV1::KeyValueV1,
                &variables,
                expected.len() - 1,
            )
            .expect_err("aggregate escaped expansion must exceed the smaller limit");
        assert_eq!(error.code(), "render_expanded_configuration_too_large");
        assert!(oversized.variable_references().unwrap().contains("value"));
    }
}

//! Side-effect-free preparation and discovery for runtime recipe configuration.
//!
//! This module resolves request overrides, saved configuration, device-plan
//! selection, recipe dependencies, and binding precedence. It performs no ADB,
//! download, extraction, copy, device-write, or persistence operations.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::catalog_source::{CatalogIdentity, CatalogSnapshot};
use crate::dto;
use crate::model::{OrderedMap, Recipe};
use crate::planner::{
    expand_recipe_dependencies, resolve_runtime_bindings, BindingDiagnostic, BindingResolution,
    BindingSource, ExecutionPlan, PlannerInput, PlannerLoadError, PlannerMessage,
    ResolvedInputBinding, TargetDeviceBinding,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RuntimeCapabilityAvailability {
    pub root_shell: bool,
    pub app_data_write: bool,
    pub root_shell_route: Option<crate::planner::RootShellRoute>,
    pub(crate) config_create_write_evidence: Option<ConfigCreateWriteEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConfigCreateWriteEvidence {
    target_identity: Value,
}

impl ConfigCreateWriteEvidence {
    fn authorizes_target(&self, target_identity: &Value) -> bool {
        self.target_identity == *target_identity
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeCapabilityAvailabilityRequest {
    pub root_shell: bool,
    pub app_data_write: bool,
    #[serde(default)]
    pub root_shell_route: Option<crate::planner::RootShellRoute>,
}

impl From<RuntimeCapabilityAvailabilityRequest> for RuntimeCapabilityAvailability {
    fn from(request: RuntimeCapabilityAvailabilityRequest) -> Self {
        Self {
            root_shell: request.root_shell,
            app_data_write: request.app_data_write,
            root_shell_route: request.root_shell_route,
            config_create_write_evidence: None,
        }
    }
}

#[cfg(test)]
pub(crate) fn test_config_create_write_evidence() -> ConfigCreateWriteEvidence {
    ConfigCreateWriteEvidence {
        target_identity: json!({
            "appId": "retroarch",
            "packageId": "com.retroarch.aarch64",
            "targetId": "config",
            "target": {
                "kind": "file",
                "location": "external_app_data",
                "path": "files/retroarch.cfg"
            }
        }),
    }
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
    pub runtime_capability_availability: Option<RuntimeCapabilityAvailability>,
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
    pub copy_commit_preparation_required: bool,
    pub resolved_inputs: Vec<crate::planner::ResolvedInputBinding>,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

/// One source selected for the optional RetroArch configuration write.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum RetroArchConfigurationSource {
    Generated {
        recipe_id: String,
        video_threaded: bool,
        binding_source: BindingSource,
    },
    Uploaded {
        recipe_id: String,
        path: String,
        binding_source: BindingSource,
    },
}

const RETROARCH_CONFIGURATION_RECIPE_IDS: [&str; 2] =
    ["app.retroarch.provision", "app.retroarch.configure"];

impl RetroArchConfigurationSource {
    fn recipe_id(&self) -> &str {
        match self {
            Self::Generated { recipe_id, .. } | Self::Uploaded { recipe_id, .. } => recipe_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectiveRetroArchConfigurationMode {
    None,
    Managed,
    Uploaded,
}

impl EffectiveRetroArchConfigurationMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Managed => "managed",
            Self::Uploaded => "uploaded",
        }
    }
}

struct EffectiveRetroArchConfigurationSelection {
    recipe_id: String,
    mode: EffectiveRetroArchConfigurationMode,
    video_threaded: Option<(bool, BindingSource)>,
    uploaded: Option<(String, BindingSource)>,
}

pub(crate) const MAX_RETROARCH_CONFIGURATION_BYTES: usize =
    crate::authored_models::MAX_INLINE_CONFIG_LITERAL_BYTES_PER_DOCUMENT;

/// Exact bytes and source identity retained for planning and execution.
/// Review preparation adds the resolved target identity and confirmed-absence
/// precondition before the canonical plan digest is finalized.
struct FrozenRetroArchConfiguration {
    bytes: Vec<u8>,
    sha256: String,
    source_identity: Value,
}

impl std::fmt::Debug for FrozenRetroArchConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenRetroArchConfiguration")
            .field("bytes", &"[redacted]")
            .field("sha256", &self.sha256)
            .field("source_identity", &"[redacted]")
            .finish()
    }
}

impl std::fmt::Debug for RetroArchConfigurationSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generated {
                video_threaded,
                binding_source,
                ..
            } => formatter
                .debug_struct("GeneratedRetroArchConfiguration")
                .field("video_threaded", video_threaded)
                .field("binding_source", binding_source)
                .finish(),
            Self::Uploaded { binding_source, .. } => formatter
                .debug_struct("UploadedRetroArchConfiguration")
                .field("path", &"[redacted]")
                .field("binding_source", binding_source)
                .finish(),
        }
    }
}

/// Resolves one source choice for planning and presentation. A file-only saved
/// setup remains an upload selection for compatibility.
fn resolve_retroarch_configuration_selection(
    inputs: &[ResolvedInputBinding],
) -> Result<Option<EffectiveRetroArchConfigurationSelection>, &'static str> {
    fn selected<'a>(
        inputs: &'a [ResolvedInputBinding],
        recipe_id: &str,
        input_id: &str,
    ) -> Result<Option<&'a ResolvedInputBinding>, &'static str> {
        let mut matches = inputs
            .iter()
            .filter(|input| input.recipe_id == recipe_id && input.input_id == input_id);
        let Some(input) = matches.next() else {
            return Ok(None);
        };
        if matches.next().is_some() || input.key != format!("{recipe_id}/{input_id}") {
            return Err("configuration_input_ambiguous");
        }
        Ok(Some(input))
    }

    fn is_user_binding(source: Option<BindingSource>) -> bool {
        matches!(
            source,
            Some(
                BindingSource::Explicit
                    | BindingSource::UserConfiguration
                    | BindingSource::DevicePlan
            )
        )
    }

    let recipe_ids = RETROARCH_CONFIGURATION_RECIPE_IDS
        .iter()
        .copied()
        .filter(|recipe_id| {
            inputs.iter().any(|input| {
                input.recipe_id == *recipe_id
                    && matches!(
                        input.input_id.as_str(),
                        "retroarch_cfg_source" | "retroarch_video_threaded" | "retroarch_cfg"
                    )
            })
        })
        .collect::<Vec<_>>();
    let [recipe_id] = recipe_ids.as_slice() else {
        return if recipe_ids.is_empty() {
            Ok(None)
        } else {
            Err("configuration_input_ambiguous")
        };
    };

    let mode = selected(inputs, recipe_id, "retroarch_cfg_source")?;
    let setting = selected(inputs, recipe_id, "retroarch_video_threaded")?;
    let uploaded = selected(inputs, recipe_id, "retroarch_cfg")?;

    let selected_mode = mode
        .and_then(|input| input.value.as_ref())
        .filter(|value| !value.is_null())
        .map(|value| value.as_str().ok_or("configuration_source_invalid"))
        .transpose()?;
    if selected_mode.is_some_and(|value| !matches!(value, "none" | "managed" | "uploaded")) {
        return Err("configuration_source_invalid");
    }

    let video_threaded = setting
        .and_then(|input| input.value.as_ref())
        .filter(|value| !value.is_null())
        .map(|value| value.as_bool().ok_or("configuration_managed_value_invalid"))
        .transpose()?;
    let setting_source = setting.and_then(|input| input.source);
    let active_setting = video_threaded.is_some()
        && setting_source.is_some_and(|source| source != BindingSource::RecipeDefault);

    let upload_path = uploaded
        .and_then(|input| input.value.as_ref())
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .filter(|path| !path.trim().is_empty())
                .map(str::to_string)
                .ok_or("configuration_upload_invalid")
        })
        .transpose()?;
    let upload_source = uploaded.and_then(|input| input.source);
    let active_upload = upload_path.is_some() && upload_source.is_some();

    let mode_source = mode.and_then(|input| input.source);
    let managed_selected = match selected_mode {
        Some("managed") => true,
        Some("uploaded") => false,
        Some("none") if is_user_binding(mode_source) => false,
        Some("none") => active_setting,
        None => active_setting,
        _ => unreachable!("mode values are validated above"),
    };
    let uploaded_selected = match selected_mode {
        Some("uploaded") => true,
        Some("managed") => false,
        Some("none") if is_user_binding(mode_source) => false,
        Some("none") => active_upload,
        None => active_upload,
        _ => unreachable!("mode values are validated above"),
    };

    if selected_mode == Some("none")
        && is_user_binding(mode_source)
        && (active_setting || active_upload)
    {
        return Err("configuration_source_conflict");
    }
    if (managed_selected && active_upload) || (uploaded_selected && active_setting) {
        return Err("configuration_source_conflict");
    }

    let selected_mode = match (managed_selected, uploaded_selected) {
        (true, false) => EffectiveRetroArchConfigurationMode::Managed,
        (false, true) => EffectiveRetroArchConfigurationMode::Uploaded,
        (false, false) => EffectiveRetroArchConfigurationMode::None,
        (true, true) => return Err("configuration_source_conflict"),
    };
    let video_threaded = video_threaded.zip(setting_source);
    let uploaded = upload_path.zip(upload_source);

    Ok(Some(EffectiveRetroArchConfigurationSelection {
        recipe_id: (*recipe_id).to_string(),
        mode: selected_mode,
        video_threaded,
        uploaded,
    }))
}

/// The user-facing mode is derived from the same Rust source decision that
/// planning consumes. Conflicts stay visible so the UI cannot hide stale data.
fn effective_retroarch_configuration_source(
    inputs: &[ResolvedInputBinding],
    recipe_id: &str,
) -> &'static str {
    match resolve_retroarch_configuration_selection(inputs) {
        Ok(Some(selection)) if selection.recipe_id == recipe_id => selection.mode.as_str(),
        Ok(Some(_)) => "conflict",
        Ok(None) => "none",
        Err(_) => "conflict",
    }
}

/// Selects one executable source and rejects stale bindings from the other.
fn select_retroarch_configuration_source(
    inputs: &[ResolvedInputBinding],
) -> Result<Option<RetroArchConfigurationSource>, &'static str> {
    let Some(selection) = resolve_retroarch_configuration_selection(inputs)? else {
        return Ok(None);
    };
    match selection.mode {
        EffectiveRetroArchConfigurationMode::None => Ok(None),
        EffectiveRetroArchConfigurationMode::Managed => {
            let (video_threaded, binding_source) = selection
                .video_threaded
                .ok_or("configuration_managed_value_missing")?;
            Ok(Some(RetroArchConfigurationSource::Generated {
                recipe_id: selection.recipe_id,
                video_threaded,
                binding_source,
            }))
        }
        EffectiveRetroArchConfigurationMode::Uploaded => {
            let (path, binding_source) =
                selection.uploaded.ok_or("configuration_upload_missing")?;
            Ok(Some(RetroArchConfigurationSource::Uploaded {
                recipe_id: selection.recipe_id,
                path,
                binding_source,
            }))
        }
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(unix)]
fn host_file_object_identity(metadata: &std::fs::Metadata) -> Value {
    use std::os::unix::fs::MetadataExt;

    json!({
        "platform": "unix",
        "device": metadata.dev(),
        "inode": metadata.ino(),
        "length": metadata.len(),
        "modifiedSeconds": metadata.mtime(),
        "modifiedNanoseconds": metadata.mtime_nsec(),
        "changedSeconds": metadata.ctime(),
        "changedNanoseconds": metadata.ctime_nsec(),
    })
}

#[cfg(not(unix))]
fn host_file_object_identity(metadata: &std::fs::Metadata) -> Value {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string());
    json!({
        "platform": std::env::consts::OS,
        "length": metadata.len(),
        "modifiedNanosecondsSinceEpoch": modified,
    })
}

fn freeze_uploaded_retroarch_configuration(
    recipe_id: &str,
    path: &str,
    binding_source: BindingSource,
) -> Result<FrozenRetroArchConfiguration, &'static str> {
    let source_path = Path::new(path);
    let mut file =
        std::fs::File::open(source_path).map_err(|_| "configuration_upload_unavailable")?;
    let before = file
        .metadata()
        .map_err(|_| "configuration_upload_unavailable")?;
    if !before.is_file() {
        return Err("configuration_upload_not_file");
    }
    if before.len() > MAX_RETROARCH_CONFIGURATION_BYTES as u64 {
        return Err("configuration_upload_too_large");
    }

    let source_identity_before = host_file_object_identity(&before);
    let mut bytes = Vec::with_capacity(before.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_RETROARCH_CONFIGURATION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "configuration_upload_unavailable")?;
    if bytes.len() > MAX_RETROARCH_CONFIGURATION_BYTES {
        return Err("configuration_upload_too_large");
    }

    let after = file
        .metadata()
        .map_err(|_| "configuration_upload_unavailable")?;
    if source_identity_before != host_file_object_identity(&after)
        || after.len() != bytes.len() as u64
    {
        return Err("configuration_upload_changed_during_capture");
    }

    let sha256 = sha256_hex(&bytes);
    let canonical_path = std::fs::canonicalize(source_path).unwrap_or_else(|_| source_path.into());
    let path_identity_sha256 = sha256_hex(canonical_path.to_string_lossy().as_bytes());
    Ok(FrozenRetroArchConfiguration {
        source_identity: json!({
            "kind": "uploaded",
            "recipeId": recipe_id,
            "inputId": "retroarch_cfg",
            "bindingSource": binding_source,
            "pathIdentitySha256": path_identity_sha256,
            "fileObject": source_identity_before,
            "byteLength": bytes.len(),
            "sha256": sha256,
        }),
        bytes,
        sha256,
    })
}

fn retroarch_app_definition(
    prepared: &PreparedConfiguration,
) -> Result<crate::authored_models::AppDefinitionV1, &'static str> {
    crate::catalog::load_app_definition_catalog(prepared.catalog.root())
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .find(|app| app.id == "retroarch")
        .ok_or("configuration_app_definition_missing")
}

fn retroarch_target_identity(
    app: &crate::authored_models::AppDefinitionV1,
) -> Result<Value, &'static str> {
    let target = app
        .targets
        .get("config")
        .ok_or("configuration_target_missing")?;
    Ok(json!({
        "appId": app.id,
        "packageId": app.package_id,
        "targetId": "config",
        "target": target,
    }))
}

fn freeze_generated_retroarch_configuration(
    prepared: &PreparedConfiguration,
    recipe_id: &str,
    video_threaded: bool,
    binding_source: BindingSource,
) -> Result<FrozenRetroArchConfiguration, RuntimeConfigurationDiagnostic> {
    let app = retroarch_app_definition(prepared).map_err(|code| {
        configuration_diagnostic(
            code,
            "The RetroArch app configuration target is unavailable.",
        )
    })?;
    let recipe = prepared
        .recipes
        .iter()
        .find(|recipe| recipe.id == recipe_id)
        .ok_or_else(|| {
            configuration_diagnostic(
                "configuration_recipe_missing",
                "The selected RetroArch configuration recipe is unavailable.",
            )
        })?;
    let resolved_resets = indexmap::IndexMap::new();
    let rendered = crate::configuration_renderer::render_recipe_configuration(
        crate::configuration_renderer::ConfigurationRenderRequest {
            app: &app,
            recipe,
            authored_root: prepared.catalog.root(),
            inputs: &prepared.binding_resolution.resolved_inputs,
            authority: None,
            setup_locations: &[],
            resolved_resets: &resolved_resets,
        },
    )
    .map_err(|error| configuration_diagnostic(error.code(), &error.to_string()))?;
    if rendered.sha256() != sha256_hex(rendered.bytes()) {
        return Err(configuration_diagnostic(
            "configuration_render_hash_mismatch",
            "The generated configuration hash did not match its bytes.",
        ));
    }
    let provenance = serde_json::to_value(rendered.provenance()).map_err(|_| {
        configuration_diagnostic(
            "configuration_provenance_invalid",
            "The generated configuration provenance could not be captured.",
        )
    })?;
    let bytes = rendered.bytes().to_vec();
    let sha256 = rendered.sha256().to_string();
    Ok(FrozenRetroArchConfiguration {
        source_identity: json!({
            "kind": "generated",
            "recipeId": recipe_id,
            "appId": app.id,
            "configurationId": "main",
            "bindingSource": binding_source,
            "videoThreaded": video_threaded,
            "renderProvenance": provenance,
            "sha256": sha256,
            "byteLength": bytes.len(),
        }),
        bytes,
        sha256,
    })
}

fn configuration_diagnostic(
    code: impl Into<String>,
    message: &str,
) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: code.into(),
        message: message.to_string(),
        key: None,
        provenance: None,
        details: json!({}),
    }
}

fn create_configuration_planner_input_file(
    bytes: &[u8],
) -> Result<tempfile::NamedTempFile, &'static str> {
    let mut file = tempfile::Builder::new()
        .suffix(".cfg")
        .tempfile()
        .map_err(|_| "configuration_staging_unavailable")?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| "configuration_staging_unavailable")?;
    Ok(file)
}

fn frozen_configuration_value(
    frozen: &FrozenRetroArchConfiguration,
    target_identity: Value,
) -> Value {
    let byte_length = frozen.bytes.len();
    json!({
        "type": "configuration_artifact",
        "schemaVersion": 1,
        "bytes": frozen.bytes,
        "byteLength": byte_length,
        "sha256": frozen.sha256,
        "sourceIdentity": frozen.source_identity,
        "targetIdentity": target_identity,
        "absencePrecondition": Value::Null,
    })
}

fn bind_frozen_configuration_to_plan(
    plan: &mut ExecutionPlan,
    recipe_id: &str,
    artifact: Value,
) -> Result<(), &'static str> {
    let seed_step_ids = plan
        .steps
        .iter()
        .filter(|step| step.recipe_ref == recipe_id && step.id.ends_with("seed_retroarch_cfg"))
        .map(|step| step.id.clone())
        .collect::<Vec<_>>();
    let [seed_step_id] = seed_step_ids.as_slice() else {
        return Err("configuration_create_step_missing");
    };
    let seed_step = plan
        .steps
        .iter_mut()
        .find(|step| step.id == *seed_step_id)
        .ok_or("configuration_create_step_missing")?;
    if seed_step.type_name != "copy_files" {
        return Err("configuration_create_step_invalid");
    }
    seed_step.params.insert(
        "source".to_string(),
        crate::planner::ExecutionParamValue::Literal { value: artifact },
    );
    seed_step.params.insert(
        "copy_policy".to_string(),
        crate::planner::ExecutionParamValue::Literal {
            value: Value::String("create_only".to_string()),
        },
    );

    for input in &mut plan.inputs {
        if input.id == format!("{recipe_id}/retroarch_cfg") {
            input.value.value = Value::Null;
            input.value.location = None;
        }
    }
    for input in &mut plan.resolved_inputs {
        if input.key == format!("{recipe_id}/retroarch_cfg") {
            input.value = None;
        }
    }

    if recipe_id == "app.retroarch.provision" {
        if let Some(launch_step) = plan
            .steps
            .iter_mut()
            .find(|step| step.recipe_ref == recipe_id && step.id.ends_with("launch_retroarch"))
        {
            if !launch_step.dependencies.contains(seed_step_id) {
                launch_step.dependencies.push(seed_step_id.clone());
            }
        }
    }

    Ok(())
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
            root_shell_route: None,
        }),
        runtime_capability_availability: request.runtime_capability_availability,
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
        let input = PlannerInput {
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
            config_create_destination: None,
            catalog_identity: self.catalog.identity().cloned(),
            target_device: self.target_device.clone(),
            device_plan_presentation: Some(crate::planner::ExecutionDevicePlanSnapshot {
                id: parts.device_plan_ref.clone(),
                name: parts.device_plan_name.clone(),
                description: parts.device_plan_description.clone(),
            }),
        };
        Some(input)
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
            copy_commit_preparation_required: false,
            resolved_inputs,
            diagnostics: prepared.diagnostics,
        });
    }
    let selected_configuration = match select_retroarch_configuration_source(&resolved_inputs) {
        Ok(source) => source,
        Err(code) => {
            let mut diagnostics = prepared.diagnostics.clone();
            diagnostics.push(configuration_diagnostic(
                code,
                "The selected RetroArch configuration inputs conflict or are invalid.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        }
    };
    let frozen_configuration = match selected_configuration.as_ref() {
        Some(RetroArchConfigurationSource::Generated {
            recipe_id,
            video_threaded,
            binding_source,
        }) => match freeze_generated_retroarch_configuration(
            &prepared,
            recipe_id,
            *video_threaded,
            *binding_source,
        ) {
            Ok(frozen) => Some(frozen),
            Err(diagnostic) => {
                let mut diagnostics = prepared.diagnostics.clone();
                diagnostics.push(diagnostic);
                return Ok(PlanConfigurationResult {
                    plan: None,
                    plan_digest: None,
                    review: None,
                    copy_commit_preparation_required: false,
                    resolved_inputs,
                    diagnostics,
                });
            }
        },
        Some(RetroArchConfigurationSource::Uploaded {
            recipe_id,
            path,
            binding_source,
        }) => match freeze_uploaded_retroarch_configuration(recipe_id, path, *binding_source) {
            Ok(frozen) => Some(frozen),
            Err(code) => {
                let mut diagnostics = prepared.diagnostics.clone();
                diagnostics.push(configuration_diagnostic(
                    code,
                    "The supplied RetroArch configuration could not be frozen safely.",
                ));
                return Ok(PlanConfigurationResult {
                    plan: None,
                    plan_digest: None,
                    review: None,
                    copy_commit_preparation_required: false,
                    resolved_inputs,
                    diagnostics,
                });
            }
        },
        None => None,
    };

    let retroarch_config_create_target = match retroarch_config_create_target_identity(
        &prepared,
        selected_configuration.as_ref(),
        frozen_configuration.as_ref(),
    ) {
        Ok(target) => target,
        Err(code) => {
            let mut diagnostics = prepared.diagnostics.clone();
            diagnostics.push(configuration_diagnostic(
                code,
                "The selected configuration artifact does not resolve to the authorized RetroArch App Target.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        }
    };
    let retroarch_config_create_selected = retroarch_config_create_target.is_some();
    if retroarch_config_create_selected
        && !config_create_write_evidence_available(
            prepared.runtime_capability_availability.as_ref(),
            retroarch_config_create_target.as_ref().unwrap(),
        )
    {
        let mut diagnostics = prepared.diagnostics.clone();
        diagnostics.push(configuration_diagnostic(
            "config_create_write_unverified",
            "The selected RetroArch configuration target has no current write-probe evidence.",
        ));
        return Ok(PlanConfigurationResult {
            plan: None,
            plan_digest: None,
            review: None,
            copy_commit_preparation_required: false,
            resolved_inputs,
            diagnostics,
        });
    }

    let plan_id = format!("plan.{}.001", prepared.effective_device_plan);
    let Some(mut input) = prepared.planner_input(plan_id) else {
        return Ok(PlanConfigurationResult {
            plan: None,
            plan_digest: None,
            review: None,
            copy_commit_preparation_required: false,
            resolved_inputs,
            diagnostics: prepared.diagnostics,
        });
    };
    if let Some(target_identity) = retroarch_config_create_target.as_ref() {
        let Some(package_id) = target_identity
            .get("packageId")
            .and_then(Value::as_str)
            .filter(|package_id| !package_id.trim().is_empty())
        else {
            let mut diagnostics = prepared.diagnostics.clone();
            diagnostics.push(configuration_diagnostic(
                "retroarch_config_create_target_invalid",
                "The selected configuration target has no resolved package identity.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        };
        let Some(target_path) = target_identity
            .pointer("/target/path")
            .and_then(Value::as_str)
            .filter(|target_path| !target_path.trim().is_empty())
        else {
            let mut diagnostics = prepared.diagnostics.clone();
            diagnostics.push(configuration_diagnostic(
                "retroarch_config_create_target_invalid",
                "The selected configuration target has no resolved path.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        };
        input.config_create_destination = Some(format!(
            "/storage/emulated/0/Android/data/{package_id}/{target_path}"
        ));
    }
    let mut generated_input_file = None;
    if let (Some(RetroArchConfigurationSource::Generated { recipe_id, .. }), Some(frozen)) =
        (&selected_configuration, &frozen_configuration)
    {
        let temporary = create_configuration_planner_input_file(&frozen.bytes).map_err(|code| {
            ConfigurationContextError::Catalog(PlannerLoadError::new(
                code,
                "Generated RetroArch configuration could not be staged for planning.".to_string(),
            ))
        })?;
        input.explicit_input_bindings.insert(
            format!("{recipe_id}/retroarch_cfg"),
            json!(temporary.path().to_string_lossy()),
        );
        generated_input_file = Some(temporary);
    }

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
    let mut execution_plan = result.execution_plan;
    if let (Some(selected), Some(frozen)) = (&selected_configuration, &frozen_configuration) {
        let Some(target_identity) = retroarch_config_create_target else {
            diagnostics.push(configuration_diagnostic(
                "retroarch_config_create_target_invalid",
                "The selected configuration is missing its validated RetroArch App Target.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        };
        let Some(plan) = execution_plan.as_mut() else {
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        };
        if let Err(code) = bind_frozen_configuration_to_plan(
            plan,
            selected.recipe_id(),
            frozen_configuration_value(frozen, target_identity),
        ) {
            diagnostics.push(configuration_diagnostic(
                code,
                "The selected RetroArch configuration action is not present in the plan.",
            ));
            return Ok(PlanConfigurationResult {
                plan: None,
                plan_digest: None,
                review: None,
                copy_commit_preparation_required: false,
                resolved_inputs,
                diagnostics,
            });
        }
    }
    drop(generated_input_file);

    let copy_commit_preparation_required = execution_plan
        .as_ref()
        .is_some_and(crate::executor::plan_requires_copy_commit_preparation);
    let plan_digest = if copy_commit_preparation_required {
        None
    } else {
        execution_plan
            .as_ref()
            .map(crate::plan_digest::execution_plan_digest)
            .transpose()
            .map_err(|error| {
                ConfigurationContextError::Catalog(PlannerLoadError::new(
                    "plan_digest_failed",
                    error.to_string(),
                ))
            })?
    };
    let review = if copy_commit_preparation_required {
        None
    } else {
        execution_plan
            .as_ref()
            .map(|plan| project_review(plan, &diagnostics))
    };
    Ok(PlanConfigurationResult {
        plan: execution_plan,
        plan_digest,
        review,
        copy_commit_preparation_required,
        resolved_inputs,
        diagnostics,
    })
}

const RETROARCH_CONFIG_CREATE_RECIPES: [&str; 2] =
    ["app.retroarch.configure", "app.retroarch.provision"];

#[cfg(test)]
fn retroarch_config_create_selected(resolved_inputs: &[ResolvedInputBinding]) -> bool {
    matches!(
        select_retroarch_configuration_source(resolved_inputs),
        Ok(Some(selected)) if RETROARCH_CONFIG_CREATE_RECIPES.contains(&selected.recipe_id())
    )
}

fn config_create_write_evidence_available(
    availability: Option<&RuntimeCapabilityAvailability>,
    target_identity: &Value,
) -> bool {
    availability.is_some_and(|availability| {
        availability.root_shell
            && availability.app_data_write
            && availability.root_shell_route.is_some()
            && availability
                .config_create_write_evidence
                .as_ref()
                .is_some_and(|evidence| evidence.authorizes_target(target_identity))
    })
}

fn retroarch_config_create_target_identity(
    prepared: &PreparedConfiguration,
    selected: Option<&RetroArchConfigurationSource>,
    frozen: Option<&FrozenRetroArchConfiguration>,
) -> Result<Option<Value>, &'static str> {
    let Some(selected) = selected else {
        return if frozen.is_none() {
            Ok(None)
        } else {
            Err("retroarch_config_create_artifact_invalid")
        };
    };
    let recipe_id = selected.recipe_id();
    if !RETROARCH_CONFIG_CREATE_RECIPES.contains(&recipe_id) {
        return Ok(None);
    }
    let expected_source_kind = match selected {
        RetroArchConfigurationSource::Generated { .. } => "generated",
        RetroArchConfigurationSource::Uploaded { .. } => "uploaded",
    };
    let frozen = frozen.ok_or("retroarch_config_create_artifact_invalid")?;
    if frozen
        .source_identity
        .get("recipeId")
        .and_then(Value::as_str)
        != Some(recipe_id)
        || frozen.source_identity.get("kind").and_then(Value::as_str) != Some(expected_source_kind)
    {
        return Err("retroarch_config_create_artifact_invalid");
    }

    let target_identity = retroarch_target_identity(&retroarch_app_definition(prepared)?)?;
    let target = target_identity.get("target");
    let package_id = target_identity
        .get("packageId")
        .and_then(Value::as_str)
        .filter(|package_id| !package_id.trim().is_empty());
    if target_identity.get("appId").and_then(Value::as_str) != Some("retroarch")
        || target_identity.get("targetId").and_then(Value::as_str) != Some("config")
        || package_id.is_none()
        || target
            .and_then(|target| target.get("kind"))
            .and_then(Value::as_str)
            != Some("file")
        || target
            .and_then(|target| target.get("location"))
            .and_then(Value::as_str)
            != Some("external_app_data")
        || target
            .and_then(|target| target.get("path"))
            .and_then(Value::as_str)
            != Some("files/retroarch.cfg")
    {
        return Err("retroarch_config_create_target_invalid");
    }
    Ok(Some(target_identity))
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
        root_shell_route: None,
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
            if resolved.input_id == "retroarch_cfg_source"
                && RETROARCH_CONFIGURATION_RECIPE_IDS.contains(&resolved.recipe_id.as_str())
            {
                object.insert(
                    "effectiveConfigurationSource".to_string(),
                    json!(effective_retroarch_configuration_source(
                        &prepared.binding_resolution.resolved_inputs,
                        &resolved.recipe_id,
                    )),
                );
            }
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
        let route_is_verified = availability.root_shell_route.is_some();
        capabilities.root_shell &= availability.root_shell && route_is_verified;
        capabilities.app_data_write &= availability.app_data_write && route_is_verified;
        capabilities.root_shell_route = (availability.root_shell && route_is_verified)
            .then_some(availability.root_shell_route)
            .flatten();
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
            root_shell_route: None,
        }
    }

    #[test]
    fn root_capability_mask_is_fail_closed_and_does_not_change_unrelated_capabilities() {
        let masked = apply_runtime_capability_availability(
            capabilities(),
            Some(&RuntimeCapabilityAvailability {
                root_shell: false,
                app_data_write: false,
                root_shell_route: None,
                config_create_write_evidence: None,
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
                root_shell_route: None,
                config_create_write_evidence: None,
            }),
        })
        .unwrap();
        let input = prepared.planner_input("plan.test".to_string()).unwrap();
        assert!(!input.runtime_capabilities.root_shell);
        assert!(!input.runtime_capabilities.app_data_write);
        assert!(input.runtime_capabilities.shared_storage_write);
    }

    #[test]
    fn runtime_root_route_requires_and_preserves_verified_route() {
        let masked = apply_runtime_capability_availability(
            capabilities(),
            Some(&RuntimeCapabilityAvailability {
                root_shell: true,
                app_data_write: true,
                root_shell_route: None,
                config_create_write_evidence: None,
            }),
        );
        assert!(!masked.root_shell);
        assert!(!masked.app_data_write);
        assert_eq!(masked.root_shell_route, None);

        let selected = apply_runtime_capability_availability(
            capabilities(),
            Some(&RuntimeCapabilityAvailability {
                root_shell: true,
                app_data_write: true,
                root_shell_route: Some(crate::planner::RootShellRoute::SuZero),
                config_create_write_evidence: None,
            }),
        );
        assert!(selected.root_shell);
        assert!(selected.app_data_write);
        assert_eq!(
            selected.root_shell_route,
            Some(crate::planner::RootShellRoute::SuZero)
        );
        assert_eq!(
            serde_json::to_value(selected).unwrap()["root_shell_route"],
            "suZero"
        );
    }
}

#[cfg(test)]
#[path = "runtime_configuration_retroarch_config_create_tests.rs"]
mod config_create_tests;

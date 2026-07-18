use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::catalog_source::{CatalogIdentity, CatalogSnapshot, CatalogSource, LocalCatalogSource};
use crate::envelope;
use crate::errors::ApiError;
use crate::planner::TargetDeviceBinding;
use crate::protocol;
use crate::runtime_configuration::{self, ConfigurationContextRequest, UserConfigurationSource};
use crate::session::DocumentSessionManager;
use crate::step_specs;
use crate::user_configuration;
use crate::validation;
use crate::yaml;

/// Validate and dispatch a one-shot request object.
pub fn handle_one_shot_value(request: Value) -> Value {
    match handle_request(request) {
        Ok(response) => response,
        Err(error) => envelope::failure(error),
    }
}

/// Validate and dispatch a sidecar request object, including sidecar id rules.
pub fn handle_sidecar_value(request: Value, sessions: &mut DocumentSessionManager) -> Value {
    let mut request_id = None;
    let response = match validate_request_object(request) {
        Ok(object) => match validate_sidecar_id(&object) {
            Ok(id) => {
                request_id = Some(id);
                handle_validated_sidecar_object(&object, sessions).unwrap_or_else(envelope::failure)
            }
            Err(error) => envelope::failure(error),
        },
        Err(error) => envelope::failure(error),
    };

    envelope::with_id(response, request_id)
}

fn handle_request(request: Value) -> Result<Value, ApiError> {
    let object = validate_request_object(request)?;
    handle_validated_object(&object)
}

fn handle_validated_object(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let request_type = validate_request_type(object)?;
    validate_payload(object)?;

    match request_type {
        "hello" => Ok(envelope::success(protocol::hello_result())),
        "ping" => Ok(envelope::success(protocol::ping_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        "emitUserConfigurationYamlFromPath" => {
            handle_emit_user_configuration_yaml_from_path(object)
        }
        "validateUserConfigurationPath" => handle_validate_user_configuration_path(object),
        "describeCatalog" => handle_describe_catalog(object),
        "listAdbDevices" => handle_list_adb_devices(object),
        "probeDevice" => handle_probe_device(object),
        "inspectApk" => handle_inspect_apk(object),
        "generateAppRecipeDraft" => handle_generate_app_recipe_draft(object),
        "generateRemoteAppRecipeDraft" => handle_generate_remote_app_recipe_draft(object),
        "generateDeviceProfileDraft" => handle_generate_device_profile_draft(object),
        "checkGeneratedCatalogCollisions" => handle_check_generated_catalog_collisions(object),
        "matchDevice" => handle_match_device(object),
        "describeConfiguration" => handle_describe_configuration(object),
        "planConfiguration" => handle_plan_configuration(object),
        unknown => Err(ApiError::invalid_request(format!(
            "Unknown request type: {unknown}"
        ))),
    }
}

fn handle_validated_sidecar_object(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let request_type = validate_request_type(object)?;
    validate_payload(object)?;

    match request_type {
        "hello" => Ok(envelope::success(protocol::hello_result())),
        "negotiateCapabilities" => handle_negotiate_capabilities(object),
        "ping" => Ok(envelope::success(protocol::ping_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        "emitUserConfigurationYamlFromPath" => {
            handle_emit_user_configuration_yaml_from_path(object)
        }
        "validateUserConfigurationPath" => handle_validate_user_configuration_path(object),
        "describeCatalog" => handle_describe_catalog(object),
        "listAdbDevices" => handle_list_adb_devices(object),
        "probeDevice" => handle_probe_device(object),
        "inspectApk" => handle_inspect_apk(object),
        "generateAppRecipeDraft" => handle_generate_app_recipe_draft(object),
        "generateRemoteAppRecipeDraft" => handle_generate_remote_app_recipe_draft(object),
        "generateDeviceProfileDraft" => handle_generate_device_profile_draft(object),
        "checkGeneratedCatalogCollisions" => handle_check_generated_catalog_collisions(object),
        "matchDevice" => handle_match_device(object),
        "openUserConfiguration" => handle_open_user_configuration(object, sessions),
        "createUserConfiguration" => handle_create_user_configuration(object, sessions),
        "getUserConfigurationDocument" => handle_get_user_configuration_document(object, sessions),
        "saveUserConfiguration" => handle_save_user_configuration(object, sessions),
        "saveUserConfigurationAs" => handle_save_user_configuration_as(object, sessions),
        "setUserConfigurationBinding" => handle_set_user_configuration_binding(object, sessions),
        "removeUserConfigurationBinding" => {
            handle_remove_user_configuration_binding(object, sessions)
        }
        "setUserConfigurationSelectedRecipes" => {
            handle_set_user_configuration_selected_recipes(object, sessions)
        }
        "setUserConfigurationDevicePlan" => {
            handle_set_user_configuration_device_plan(object, sessions)
        }
        "validateUserConfiguration" => handle_validate_user_configuration(object, sessions),
        "emitUserConfigurationYaml" => handle_emit_user_configuration_yaml(object, sessions),
        "setUserConfigurationAuthoredRoot" => {
            handle_set_user_configuration_authored_root(object, sessions)
        }
        "closeUserConfiguration" => handle_close_user_configuration(object, sessions),
        "describeConfiguration" => handle_describe_configuration(object),
        "planConfiguration" => handle_plan_configuration(object),
        "startExecution" => handle_start_execution(object, sessions),
        "getExecution" => handle_get_execution(object, sessions),
        "getExecutionEvents" => handle_get_execution_events(object, sessions),
        "cancelExecution" => handle_cancel_execution(object, sessions),
        "launchExecutionApp" => handle_launch_execution_app(object, sessions),
        "openRecipe" => handle_open_recipe(object, sessions),
        "createRecipeFromTemplate" => handle_create_recipe_from_template(object, sessions),
        "getDocument" => handle_get_document(object, sessions),
        "saveRecipe" => handle_save_recipe(object, sessions),
        "saveRecipeAs" => handle_save_recipe_as(object, sessions),
        "applyRecipeCommand" => handle_apply_recipe_command(object, sessions),
        "undo" => handle_undo(object, sessions),
        "redo" => handle_redo(object, sessions),
        "emitYaml" => handle_emit_yaml(object, sessions),
        "validate" => handle_validate(object, sessions),
        "getRefIndex" => handle_get_ref_index(object, sessions),
        "setDocumentAuthoredRoot" => handle_set_document_authored_root(object, sessions),
        "closeDocument" => handle_close_document(object, sessions),
        unknown => Err(ApiError::invalid_request(format!(
            "Unknown request type: {unknown}"
        ))),
    }
}

fn handle_negotiate_capabilities(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let required = optional_string_array(payload, "requiredCapabilities")?.unwrap_or_default();
    let optional = optional_string_array(payload, "optionalCapabilities")?.unwrap_or_default();
    Ok(envelope::success(protocol::negotiate_capabilities(
        &required, &optional,
    )))
}

fn handle_start_execution(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    for forbidden in ["runtimeRoot", "cacheRoot", "adbPath"] {
        if payload.contains_key(forbidden) {
            return Err(ApiError::invalid_request_with_details(
                format!("Execution runtime policy is configured only at sidecar startup; '{forbidden}' is not accepted."),
                json!({ "field": forbidden }),
            ));
        }
    }
    let plan = payload.get("plan").ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: plan",
            json!({ "field": "plan" }),
        )
    })?;
    let plan = crate::cli::parse_execution_plan_json(plan).map_err(|message| {
        ApiError::new(
            crate::errors::ApiErrorCode::InvalidExecutionPlan,
            message,
            json!({ "field": "plan" }),
        )
    })?;
    let plan_digest = required_string(payload, "planDigest")?.to_string();
    if plan_digest.len() != 64 || !plan_digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::invalid_request_with_details(
            "Request field 'planDigest' must be a 64-character SHA-256 hexadecimal value.",
            json!({ "field": "planDigest" }),
        ));
    }
    let mode_value = required_string(payload, "mode")?;
    let mode = crate::execution_session::ExecutionMode::parse(mode_value).ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request field 'mode' must be 'real' or 'dry_run'.",
            json!({ "field": "mode" }),
        )
    })?;
    let target = optional_target_device(payload, "targetDevice")?;
    Ok(envelope::success(sessions.executions().start(
        plan,
        plan_digest,
        mode,
        target,
    )?))
}

fn handle_get_execution(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions
            .executions()
            .get(required_string(payload, "executionId")?)?,
    ))
}

fn handle_get_execution_events(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let after_sequence = match payload.get("afterSequence") {
        None | Some(Value::Null) => 0,
        Some(Value::Number(number)) => number.as_u64().ok_or_else(|| {
            ApiError::invalid_request_with_details(
                "Request field 'afterSequence' must be a non-negative integer.",
                json!({ "field": "afterSequence" }),
            )
        })?,
        Some(_) => {
            return Err(ApiError::invalid_request_with_details(
                "Request field 'afterSequence' must be a non-negative integer.",
                json!({ "field": "afterSequence" }),
            ));
        }
    };
    Ok(envelope::success(sessions.executions().events(
        required_string(payload, "executionId")?,
        after_sequence,
    )?))
}

fn handle_cancel_execution(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions
            .executions()
            .cancel(required_string(payload, "executionId")?)?,
    ))
}

fn handle_launch_execution_app(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions
            .executions()
            .launch_app(required_string(payload, "executionId")?)?,
    ))
}

fn validate_request_object(request: Value) -> Result<Map<String, Value>, ApiError> {
    match request {
        Value::Object(object) => Ok(object),
        _ => Err(ApiError::invalid_request("Request must be a JSON object.")),
    }
}

fn validate_sidecar_id(object: &Map<String, Value>) -> Result<String, ApiError> {
    match object.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Ok(id.clone()),
        _ => Err(ApiError::invalid_request(
            "Sidecar request must include a non-empty string id.",
        )),
    }
}

fn validate_request_type(object: &Map<String, Value>) -> Result<&str, ApiError> {
    match object.get("type") {
        Some(Value::String(request_type)) if !request_type.is_empty() => Ok(request_type),
        _ => Err(ApiError::invalid_request(
            "Request must include a string type.",
        )),
    }
}

fn validate_payload(object: &Map<String, Value>) -> Result<(), ApiError> {
    match object.get("payload") {
        None | Some(Value::Null) | Some(Value::Object(_)) => Ok(()),
        _ => Err(ApiError::invalid_request(
            "Request payload must be an object.",
        )),
    }
}

fn handle_emit_recipe_yaml_from_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    match yaml::emit_recipe_yaml_from_path(Path::new(path)) {
        Ok(yaml) => Ok(envelope::success(json!({ "yaml": yaml }))),
        Err(error) => Err(ApiError::load_failed(
            format!("Failed to emit recipe YAML: {error}"),
            json!({ "path": path }),
        )),
    }
}

fn handle_validate_recipe_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(validation::validate_recipe_path_result(
        Path::new(path),
        authored_root.map(Path::new),
    )))
}

fn handle_emit_user_configuration_yaml_from_path(
    object: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let configuration = user_configuration::load_user_configuration(&path).map_err(|error| {
        ApiError::load_failed(
            format!("Failed to load user configuration: {error}"),
            json!({ "path": path }),
        )
    })?;
    let yaml =
        user_configuration::emit_user_configuration_yaml(&configuration).map_err(|error| {
            ApiError::load_failed(
                format!("Failed to emit user configuration: {error}"),
                json!({ "path": path }),
            )
        })?;
    Ok(envelope::success(json!({ "yaml": yaml })))
}

fn handle_validate_user_configuration_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    let configuration = user_configuration::load_user_configuration(&path).map_err(|error| {
        ApiError::load_failed(
            format!("Failed to load user configuration: {error}"),
            json!({ "path": path }),
        )
    })?;
    let diagnostics = authored_root.map_or_else(Vec::new, |root| {
        user_configuration::validate_user_configuration_with_catalog(
            &configuration,
            &path,
            Path::new(root),
        )
    });
    Ok(envelope::success(json!({ "diagnostics": diagnostics })))
}

fn handle_describe_configuration(object: &Map<String, Value>) -> Result<Value, ApiError> {
    handle_runtime_configuration_request(object, false)
}

fn handle_describe_catalog(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let snapshot = resolved_catalog(payload_object(object)?, false)?;
    Ok(envelope::success(crate::product_catalog::describe(
        &snapshot,
    )?))
}

fn handle_list_adb_devices(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let adb_path = required_string(payload, "adbPath")?;
    Ok(envelope::success(
        crate::end_user_runtime::list_adb_devices(adb_path)?,
    ))
}

fn handle_probe_device(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let adb_path = required_string(payload, "adbPath")?;
    let serial = required_string(payload, "serial")?;
    Ok(envelope::success(crate::end_user_runtime::probe_device(
        adb_path, serial,
    )?))
}

fn handle_inspect_apk(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let request = serde_json::from_value::<
        crate::apk_authoring_inspection::ApkAuthoringInspectionRequest,
    >(Value::Object(payload.clone()))
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "APK inspection input is invalid.",
            json!({ "field": "payload" }),
        )
    })?;
    if !request.is_valid() {
        return Err(ApiError::invalid_request_with_details(
            "APK inspection input is invalid.",
            json!({ "field": "payload" }),
        ));
    }
    let result = crate::apk_authoring_inspection::inspect_apk_for_authoring(
        Path::new(&request.apk_path),
        request.connected_device_api,
    )
    .map_err(|error| {
        ApiError::command_failed(error.message(), json!({ "reason": error.code() }))
    })?;
    let result = serde_json::to_value(result).map_err(|_| {
        ApiError::command_failed(
            "APK inspection result could not be represented.",
            json!({ "reason": "apk_inspection_serialization_failed" }),
        )
    })?;
    Ok(envelope::success(result))
}

fn handle_generate_app_recipe_draft(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let request = serde_json::from_value::<crate::generation::AppRecipeDraftRequest>(
        Value::Object(payload.clone()),
    )
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "App-and-recipe draft input is invalid.",
            json!({ "field": "payload" }),
        )
    })?;
    let result = serde_json::to_value(crate::generation::generate_app_recipe_draft(request))
        .map_err(|_| {
            ApiError::command_failed(
                "App-and-recipe draft could not be represented.",
                json!({ "reason": "app_recipe_draft_serialization_failed" }),
            )
        })?;
    Ok(envelope::success(result))
}

fn handle_generate_remote_app_recipe_draft(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let request = serde_json::from_value::<crate::generation::RemoteAppRecipeDraftRequest>(
        Value::Object(payload.clone()),
    )
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "Remote app-and-recipe draft input is invalid.",
            json!({ "field": "payload" }),
        )
    })?;
    let result = serde_json::to_value(crate::generation::generate_remote_app_recipe_draft(request))
        .map_err(|_| {
            ApiError::command_failed(
                "Remote app-and-recipe draft could not be represented.",
                json!({ "reason": "remote_app_recipe_draft_serialization_failed" }),
            )
        })?;
    Ok(envelope::success(result))
}

fn handle_generate_device_profile_draft(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let request = serde_json::from_value::<crate::generation::DeviceProfileDraftRequest>(
        Value::Object(payload.clone()),
    )
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "Device-profile draft input is invalid.",
            json!({ "field": "payload" }),
        )
    })?;
    let draft = crate::generation::generate_device_profile_draft(request);
    let result = serde_json::to_value(draft).map_err(|_| {
        ApiError::command_failed(
            "Device-profile draft could not be represented.",
            json!({ "reason": "device_profile_draft_serialization_failed" }),
        )
    })?;
    Ok(envelope::success(result))
}

fn handle_check_generated_catalog_collisions(
    object: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let authored_root = required_string(payload, "authoredRoot")?;
    if payload.contains_key("app")
        || payload.contains_key("recipeId")
        || payload.contains_key("recipe")
    {
        let request = parse_app_recipe_collision_request(payload)?;
        let collisions =
            crate::generation::check_app_recipe_collisions(Path::new(authored_root), &request);
        let result = serde_json::to_value(collisions).map_err(|_| {
            ApiError::command_failed(
                "App-and-recipe collision results could not be represented.",
                json!({ "reason": "app_recipe_collision_serialization_failed" }),
            )
        })?;
        return Ok(envelope::success(result));
    }

    let facts = payload.get("facts").cloned().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: facts",
            json!({ "field": "facts" }),
        )
    })?;
    let facts = serde_json::from_value::<crate::generation::SafeDetectedDeviceFacts>(facts)
        .map_err(|_| {
            ApiError::invalid_request_with_details(
                "Request field 'facts' is invalid.",
                json!({ "field": "facts" }),
            )
        })?;
    let profile = payload.get("profile").cloned().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: profile",
            json!({ "field": "profile" }),
        )
    })?;
    let profile = serde_json::from_value::<crate::authored_models::DeviceProfileV1>(profile)
        .map_err(|_| {
            ApiError::invalid_request_with_details(
                "Request field 'profile' is invalid.",
                json!({ "field": "profile" }),
            )
        })?;
    let collisions = crate::generation::check_device_profile_collisions(
        Path::new(authored_root),
        &facts,
        &profile,
    );
    let result = serde_json::to_value(collisions).map_err(|_| {
        ApiError::command_failed(
            "Device-profile collision results could not be represented.",
            json!({ "reason": "device_profile_collision_serialization_failed" }),
        )
    })?;
    Ok(envelope::success(result))
}

fn parse_app_recipe_collision_request(
    payload: &Map<String, Value>,
) -> Result<crate::generation::AppRecipeCollisionRequest, ApiError> {
    let invalid = || {
        ApiError::invalid_request_with_details(
            "App-and-recipe collision input is invalid.",
            json!({ "field": "payload" }),
        )
    };
    if payload
        .keys()
        .any(|key| !matches!(key.as_str(), "authoredRoot" | "app" | "recipe" | "recipeId"))
    {
        return Err(invalid());
    }
    let app = payload
        .get("app")
        .cloned()
        .ok_or_else(&invalid)
        .and_then(|value| serde_json::from_value(value).map_err(|_| invalid()))?;
    let legacy_recipe_id = match payload.get("recipeId") {
        None => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => return Err(invalid()),
    };
    let recipe = match payload.get("recipe") {
        None => None,
        Some(value) => Some(parse_collision_recipe_dto(value.clone()).ok_or_else(&invalid)?),
    };
    let recipe_id = match (&recipe, legacy_recipe_id) {
        (Some(recipe), Some(recipe_id)) if recipe.id == recipe_id => recipe_id,
        (Some(_), Some(_)) => return Err(invalid()),
        (Some(recipe), None) => recipe.id.clone(),
        (None, Some(recipe_id)) => recipe_id,
        (None, None) => return Err(invalid()),
    };
    Ok(crate::generation::AppRecipeCollisionRequest {
        app,
        recipe_id,
        recipe,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CollisionRecipeDto {
    schema_version: i64,
    kind: String,
    id: String,
    name: String,
    description: String,
    recipe_dependencies: Vec<String>,
    provides: CollisionRecipeProvidesDto,
    inputs: crate::model::OrderedMap<CollisionInputDto>,
    artifacts: crate::model::OrderedMap<CollisionArtifactDto>,
    artifact_groups: crate::model::OrderedMap<Vec<String>>,
    steps: Vec<CollisionStepDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionRecipeProvidesDto {
    features: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CollisionInputDto {
    id: String,
    recipe_id: String,
    input_id: String,
    key: String,
    #[serde(rename = "type")]
    type_name: String,
    role: String,
    label: String,
    description: String,
    required: bool,
    multiple: bool,
    validation: CollisionInputValidationDto,
    default: Value,
    options: Vec<CollisionInputOptionDto>,
    sensitive: bool,
    advanced: bool,
    metadata: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CollisionInputValidationDto {
    must_exist: bool,
    allowed_extensions: Vec<String>,
    path_kind: Option<String>,
    allowed_prefixes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionInputOptionDto {
    value: Value,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionArtifactDto {
    id: String,
    #[serde(rename = "type")]
    type_name: String,
    url: String,
    cache: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CollisionStepDto {
    id: String,
    #[serde(rename = "type")]
    type_name: String,
    name: String,
    description: String,
    #[serde(default)]
    progress_note: Option<String>,
    user_toggleable: bool,
    dependencies: Vec<String>,
    constraints: CollisionStepConstraintsDto,
    skip_if: Vec<CollisionStepConditionDto>,
    params: Map<String, Value>,
    verify: Vec<CollisionStepConditionDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CollisionStepConstraintsDto {
    capabilities: Vec<String>,
    conflicts_with: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionStepConditionDto {
    #[serde(rename = "type")]
    type_name: String,
    params: Map<String, Value>,
}

fn parse_collision_recipe_dto(value: Value) -> Option<crate::model::Recipe> {
    let dto = serde_json::from_value::<CollisionRecipeDto>(value).ok()?;
    let authored = dto.into_authored_value();
    let serde_yaml::Value::Mapping(mapping) = serde_yaml::to_value(authored).ok()? else {
        return None;
    };
    crate::yaml::parse_recipe_mapping(&mapping, Path::new("collision-request.yaml")).ok()
}

impl CollisionRecipeDto {
    fn into_authored_value(self) -> Value {
        json!({
            "schema_version": self.schema_version,
            "kind": self.kind,
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "recipe_dependencies": self.recipe_dependencies,
            "provides": { "features": self.provides.features },
            "inputs": object_value(self.inputs.into_iter().map(|(id, input)| (id, input.into_authored_value()))),
            "artifacts": object_value(self.artifacts.into_iter().map(|(id, artifact)| (id, artifact.into_authored_value()))),
            "artifact_groups": object_value(self.artifact_groups.into_iter().map(|(id, members)| (id, json!(members)))),
            "steps": self.steps.into_iter().map(CollisionStepDto::into_authored_value).collect::<Vec<_>>(),
        })
    }
}

impl CollisionInputDto {
    fn into_authored_value(self) -> Value {
        let _projection_identity = (self.id, self.recipe_id, self.input_id, self.key);
        json!({
            "type": self.type_name,
            "role": self.role,
            "label": self.label,
            "description": self.description,
            "required": self.required,
            "multiple": self.multiple,
            "validation": {
                "must_exist": self.validation.must_exist,
                "allowed_extensions": self.validation.allowed_extensions,
                "path_kind": self.validation.path_kind,
                "allowed_prefixes": self.validation.allowed_prefixes,
            },
            "default": self.default,
            "options": self.options.into_iter().map(|option| json!({
                "value": option.value,
                "label": option.label,
            })).collect::<Vec<_>>(),
            "sensitive": self.sensitive,
            "advanced": self.advanced,
            "metadata": Value::Object(self.metadata),
        })
    }
}

impl CollisionArtifactDto {
    fn into_authored_value(self) -> Value {
        let _projection_id = self.id;
        json!({
            "type": self.type_name,
            "url": self.url,
            "cache": self.cache,
        })
    }
}

impl CollisionStepDto {
    fn into_authored_value(self) -> Value {
        let mut value = json!({
            "id": self.id,
            "type": self.type_name,
            "name": self.name,
            "description": self.description,
            "user_toggleable": self.user_toggleable,
            "dependencies": self.dependencies,
            "constraints": {
                "capabilities": self.constraints.capabilities,
                "conflicts_with": self.constraints.conflicts_with,
            },
            "skip_if": self.skip_if.into_iter().map(CollisionStepConditionDto::into_authored_value).collect::<Vec<_>>(),
            "params": Value::Object(self.params),
            "verify": self.verify.into_iter().map(CollisionStepConditionDto::into_authored_value).collect::<Vec<_>>(),
        });
        if let (Some(progress_note), Some(object)) = (self.progress_note, value.as_object_mut()) {
            object.insert("progress_note".to_string(), Value::String(progress_note));
        }
        value
    }
}

impl CollisionStepConditionDto {
    fn into_authored_value(self) -> Value {
        json!({
            "type": self.type_name,
            "params": Value::Object(self.params),
        })
    }
}

fn object_value(values: impl IntoIterator<Item = (String, Value)>) -> Value {
    Value::Object(values.into_iter().collect())
}

fn handle_match_device(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let snapshot = resolved_catalog(payload, false)?;
    let facts = payload.get("facts").cloned().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request field 'facts' is required.",
            json!({ "field": "facts" }),
        )
    })?;
    let facts = serde_json::from_value(facts).map_err(|error| {
        ApiError::invalid_request_with_details(
            format!("Request field 'facts' is invalid: {error}"),
            json!({ "field": "facts" }),
        )
    })?;
    Ok(envelope::success(crate::end_user_runtime::match_device(
        &snapshot, &facts,
    )?))
}

fn handle_plan_configuration(object: &Map<String, Value>) -> Result<Value, ApiError> {
    handle_runtime_configuration_request(object, true)
}

fn handle_runtime_configuration_request(
    object: &Map<String, Value>,
    plan: bool,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let catalog = resolved_catalog(payload, true)?;
    let configuration_root = optional_string(payload, "configurationRoot")?;
    let user_configuration = optional_user_configuration_source(payload)?;
    let device_plan = optional_string(payload, "devicePlan")?;
    if user_configuration.is_none() && device_plan.is_none() {
        return Err(ApiError::invalid_request_with_details(
            "Runtime configuration requires 'devicePlan' or 'userConfiguration'.",
            json!({ "fields": ["devicePlan", "userConfiguration"] }),
        ));
    }
    let selected_recipes = optional_string_array(payload, "selectedRecipes")?;
    let explicit_bindings = optional_binding_map(payload, "bindings")?;
    let device_context = optional_device_context(payload, "deviceContext")?;
    let target_device = optional_target_device(payload, "targetDevice")?;
    let request = ConfigurationContextRequest {
        catalog,
        configuration_root: configuration_root.map(Path::new).map(Path::to_path_buf),
        user_configuration,
        device_plan: device_plan.map(ToString::to_string),
        selected_recipes,
        explicit_bindings,
        device_context,
        target_device,
    };
    let result = if plan {
        runtime_configuration::plan_configuration(request).and_then(|result| {
            serde_json::to_value(result).map_err(|error| {
                runtime_configuration::ConfigurationContextError::Catalog(
                    crate::planner::PlannerLoadError::new(
                        "serialization_failed",
                        error.to_string(),
                    ),
                )
            })
        })
    } else {
        runtime_configuration::describe_configuration(request).and_then(|result| {
            serde_json::to_value(result).map_err(|error| {
                runtime_configuration::ConfigurationContextError::Catalog(
                    crate::planner::PlannerLoadError::new(
                        "serialization_failed",
                        error.to_string(),
                    ),
                )
            })
        })
    }
    .map_err(|error| {
        ApiError::load_failed(
            format!("Failed to prepare runtime configuration: {error}"),
            json!({ "operation": if plan { "planConfiguration" } else { "describeConfiguration" } }),
        )
    })?;
    Ok(envelope::success(result))
}

fn resolved_catalog(
    payload: &Map<String, Value>,
    allow_legacy_authored_root: bool,
) -> Result<CatalogSnapshot, ApiError> {
    let catalog = payload.get("catalog").filter(|value| !value.is_null());
    let authored_root = optional_string(payload, "authoredRoot")?;
    if catalog.is_some() && authored_root.is_some() {
        return Err(ApiError::invalid_request_with_details(
            "Request must provide only one of 'catalog' or legacy 'authoredRoot'.",
            json!({ "fields": ["catalog", "authoredRoot"] }),
        ));
    }
    if let Some(Value::Object(catalog)) = catalog {
        let root = required_string(catalog, "root")?;
        let mut identity = catalog.clone();
        identity.remove("root");
        let identity: CatalogIdentity =
            serde_json::from_value(Value::Object(identity)).map_err(|error| {
                ApiError::invalid_request_with_details(
                    format!("Request field 'catalog' is invalid: {error}"),
                    json!({ "field": "catalog" }),
                )
            })?;
        return LocalCatalogSource::new(root, identity)
            .resolve()
            .map_err(|error| {
                ApiError::load_failed(
                    format!("Failed to resolve catalog snapshot: {error}"),
                    json!({ "code": error.code() }),
                )
            });
    }
    if catalog.is_some() {
        return Err(ApiError::invalid_request_with_details(
            "Request field 'catalog' must be an object.",
            json!({ "field": "catalog" }),
        ));
    }
    if allow_legacy_authored_root {
        if let Some(root) = authored_root {
            return CatalogSnapshot::legacy_local(root).map_err(|error| {
                ApiError::load_failed(
                    format!("Failed to resolve legacy authored catalog: {error}"),
                    json!({ "code": error.code() }),
                )
            });
        }
    }
    Err(ApiError::invalid_request_with_details(
        "Product request requires a resolved 'catalog' snapshot.",
        json!({ "field": "catalog" }),
    ))
}

fn optional_target_device(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Option<TargetDeviceBinding>, ApiError> {
    let Some(value) = payload.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = value.as_object().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be an object."),
            json!({ "field": field }),
        )
    })?;
    let serial = required_string(object, "serial")?.trim().to_string();
    if serial.is_empty() {
        return Err(ApiError::invalid_request_with_details(
            "Target device serial must be non-empty.",
            json!({ "field": format!("{field}.serial") }),
        ));
    }
    Ok(Some(TargetDeviceBinding {
        serial,
        manufacturer: optional_string(object, "manufacturer")?.map(ToString::to_string),
        model: optional_string(object, "model")?.map(ToString::to_string),
        android_api_level: match object.get("androidApiLevel") {
            None | Some(Value::Null) => None,
            Some(Value::Number(value)) => Some(value.as_i64().ok_or_else(|| {
                ApiError::invalid_request_with_details(
                    "Target Android API level must be an integer.",
                    json!({ "field": format!("{field}.androidApiLevel") }),
                )
            })?),
            Some(_) => {
                return Err(ApiError::invalid_request_with_details(
                    "Target Android API level must be an integer.",
                    json!({ "field": format!("{field}.androidApiLevel") }),
                ));
            }
        },
    }))
}

fn handle_open_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.open_user_configuration(
        &path.to_string_lossy(),
        authored_root,
    )?))
}

fn handle_create_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_string(payload, "path")?;
    let configuration_id = required_string(payload, "configurationId")?;
    let name = required_string(payload, "name")?;
    let device_plan = required_string(payload, "devicePlan")?;
    let selected_recipes = required_string_array(payload, "selectedRecipes")?;
    let bindings = optional_binding_map(payload, "bindings")?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.create_user_configuration(
        path,
        configuration_id,
        name,
        device_plan,
        selected_recipes,
        bindings,
        authored_root,
    )?))
}

fn handle_get_user_configuration_document(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.get_user_configuration_document(required_document_id(payload)?)?,
    ))
}

fn handle_save_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.save_user_configuration(required_document_id(payload)?)?,
    ))
}

fn handle_save_user_configuration_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    let configuration_id = optional_string(payload, "configurationId")?;
    let name = optional_string(payload, "name")?;
    Ok(envelope::success(sessions.save_user_configuration_as(
        document_id,
        path,
        configuration_id,
        name,
    )?))
}

fn handle_set_user_configuration_binding(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let key = required_string(payload, "key")?;
    let value = payload.get("value").cloned().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: value",
            json!({ "field": "value" }),
        )
    })?;
    Ok(envelope::success(sessions.set_user_configuration_binding(
        document_id,
        key,
        Some(value),
    )?))
}

fn handle_remove_user_configuration_binding(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.set_user_configuration_binding(
        required_document_id(payload)?,
        required_string(payload, "key")?,
        None,
    )?))
}

fn handle_set_user_configuration_selected_recipes(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_selected_recipes(
            required_document_id(payload)?,
            required_string_array(payload, "selectedRecipes")?,
        )?,
    ))
}

fn handle_set_user_configuration_device_plan(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_device_plan(
            required_document_id(payload)?,
            required_string(payload, "devicePlan")?.to_string(),
        )?,
    ))
}

fn handle_validate_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.validate_user_configuration(
        required_document_id(payload)?,
    )?))
}

fn handle_emit_user_configuration_yaml(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.emit_user_configuration_yaml(
        required_document_id(payload)?,
    )?))
}

fn handle_set_user_configuration_authored_root(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_authored_root(
            required_document_id(payload)?,
            required_nullable_string(payload, "authoredRoot")?,
        )?,
    ))
}

fn handle_close_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.close_user_configuration(
        required_document_id(payload)?,
    )?))
}

fn handle_open_recipe(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(
        sessions.open_recipe(path, authored_root)?,
    ))
}

fn handle_create_recipe_from_template(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let template_path = required_string(payload, "templatePath")?;
    let destination_path = required_string(payload, "destinationPath")?;
    let recipe_id = required_string(payload, "recipeId")?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.create_recipe_from_template(
        template_path,
        destination_path,
        recipe_id,
        authored_root,
    )?))
}

fn handle_get_document(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.get_document(document_id)?))
}

fn handle_save_recipe(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.save_recipe(document_id)?))
}

fn handle_save_recipe_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    Ok(envelope::success(
        sessions.save_recipe_as(document_id, path)?,
    ))
}

fn handle_apply_recipe_command(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let command = payload.get("command").unwrap_or(&Value::Null);
    Ok(envelope::success(
        sessions.apply_recipe_command(document_id, command)?,
    ))
}

fn handle_undo(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.undo(document_id)?))
}

fn handle_redo(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.redo(document_id)?))
}

fn handle_emit_yaml(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.emit_yaml(document_id)?))
}

fn handle_validate(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.validate(document_id)?))
}

fn handle_get_ref_index(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.get_ref_index(document_id)?))
}

fn handle_set_document_authored_root(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let authored_root = required_nullable_string(payload, "authoredRoot")?;
    Ok(envelope::success(
        sessions.set_document_authored_root(document_id, authored_root)?,
    ))
}

fn handle_close_document(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.close_document(document_id)?))
}

fn payload_object(object: &Map<String, Value>) -> Result<&Map<String, Value>, ApiError> {
    match object.get("payload") {
        Some(Value::Object(payload)) => Ok(payload),
        None | Some(Value::Null) => {
            static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
            Ok(EMPTY.get_or_init(Map::new))
        }
        _ => Err(ApiError::invalid_request(
            "Request payload must be an object.",
        )),
    }
}

fn required_path(payload: &Map<String, Value>) -> Result<&str, ApiError> {
    required_string(payload, "path")
}

fn required_string<'a>(payload: &'a Map<String, Value>, field: &str) -> Result<&'a str, ApiError> {
    match payload.get(field) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request payload is missing required field: {field}"),
            json!({ "field": field }),
        )),
    }
}

fn required_document_id(payload: &Map<String, Value>) -> Result<&str, ApiError> {
    required_string(payload, "documentId")
}

fn optional_string<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ApiError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be a non-empty string when provided."),
            json!({ "field": field }),
        )),
    }
}

fn required_nullable_string<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ApiError> {
    match payload.get(field) {
        None => Err(ApiError::invalid_request_with_details(
            format!("Request payload is missing required field: {field}"),
            json!({ "field": field }),
        )),
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be a non-empty string when provided."),
            json!({ "field": field }),
        )),
    }
}

fn required_string_array(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, ApiError> {
    let Some(Value::Array(values)) = payload.get(field) else {
        return Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be an array of strings."),
            json!({ "field": field }),
        ));
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(value) if !value.is_empty() => Ok(value.clone()),
            _ => Err(ApiError::invalid_request_with_details(
                format!("Request field '{field}' must be an array of non-empty strings."),
                json!({ "field": field }),
            )),
        })
        .collect()
}

fn optional_string_array(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_string_array(payload, field).map(Some),
    }
}

fn optional_binding_map(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<crate::model::OrderedMap<Value>, ApiError> {
    let Some(value) = payload.get(field) else {
        return Ok(crate::model::OrderedMap::new());
    };
    let Value::Object(bindings) = value else {
        return Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be an object."),
            json!({ "field": field }),
        ));
    };
    let mut result = crate::model::OrderedMap::new();
    for (key, value) in bindings {
        user_configuration::validate_binding_key(key).map_err(|error| {
            ApiError::invalid_request_with_details(
                format!("Request binding key is invalid: {error}"),
                json!({ "field": field, "key": key }),
            )
        })?;
        result.insert(key.clone(), value.clone());
    }
    Ok(result)
}

fn optional_user_configuration_source(
    payload: &Map<String, Value>,
) -> Result<Option<UserConfigurationSource>, ApiError> {
    match payload.get("userConfiguration") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(reference)) if !reference.is_empty() => {
            Ok(Some(UserConfigurationSource::Reference(reference.clone())))
        }
        Some(value @ Value::Object(_)) => {
            user_configuration::parse_inline_user_configuration(value)
                .map(UserConfigurationSource::Inline)
                .map(Some)
                .map_err(|error| {
                    ApiError::load_failed(
                        format!("Failed to load inline user configuration: {error}"),
                        json!({ "field": "userConfiguration" }),
                    )
                })
        }
        Some(_) => Err(ApiError::invalid_request_with_details(
            "Request field 'userConfiguration' must be a non-empty string or an object.",
            json!({ "field": "userConfiguration" }),
        )),
    }
}

fn optional_device_context(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Option<runtime_configuration::DeviceContextOverride>, ApiError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => serde_json::from_value(Value::Object(value.clone()))
            .map(Some)
            .map_err(|error| {
                ApiError::invalid_request_with_details(
                    format!("Request field '{field}' is invalid: {error}"),
                    json!({ "field": field }),
                )
            }),
        Some(_) => Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be an object."),
            json!({ "field": field }),
        )),
    }
}

fn user_configuration_path(payload: &Map<String, Value>) -> Result<std::path::PathBuf, ApiError> {
    let path = optional_string(payload, "path")?;
    let reference = optional_string(payload, "userConfiguration")?;
    let value = match (path, reference) {
        (Some(path), None) | (None, Some(path)) => path,
        (Some(_), Some(_)) => {
            return Err(ApiError::invalid_request_with_details(
                "Request must provide only one of 'path' or 'userConfiguration'.",
                json!({ "fields": ["path", "userConfiguration"] }),
            ));
        }
        (None, None) => {
            return Err(ApiError::invalid_request_with_details(
                "Request must provide 'path' or 'userConfiguration'.",
                json!({ "fields": ["path", "userConfiguration"] }),
            ));
        }
    };
    let configuration_root = optional_string(payload, "configurationRoot")?;
    user_configuration::resolve_user_configuration_path(configuration_root.map(Path::new), value)
        .map_err(|error| {
            ApiError::invalid_request_with_details(
                format!("Invalid user-configuration reference: {error}"),
                json!({ "userConfiguration": value }),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ParamValue;

    fn collision_app() -> Value {
        json!({
            "schema_version": 1,
            "kind": "app_definition",
            "id": "example",
            "name": "Example",
            "category": "utility",
            "package": { "primary": "com.example.app", "aliases": [] },
            "install_source": { "type": "remote_apk", "resolver": "direct_url", "options": {} },
            "tracking_source": { "type": "remote_apk" },
            "artifacts": {
                "apk": { "required": true },
                "shared_storage_config": { "supported": false },
                "app_data_config": { "supported": false },
                "byo_apk": { "required": false }
            },
            "provisioning": {
                "launch_once_recommended": false,
                "shared_storage_paths": [],
                "app_data_paths": [],
                "config_targets": []
            },
            "inputs": [],
            "metadata": {}
        })
    }

    fn collision_recipe() -> crate::model::Recipe {
        let raw = r#"schema_version: 1
kind: recipe
id: app.example.install
name: Install Example
provides:
  features: []
steps:
- id: install
  type: install_apk
  name: Install
  user_toggleable: false
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  skip_if: []
  params:
    app: {ref: inputs.apk}
    expected_package_name: com.example.app
    replace_existing: false
  verify: []
"#;
        let serde_yaml::Value::Mapping(mapping) = serde_yaml::from_str(raw).unwrap() else {
            panic!("recipe fixture must be a mapping");
        };
        crate::yaml::parse_recipe_mapping(&mapping, Path::new("request-test.yaml")).unwrap()
    }

    #[test]
    fn collision_recipe_dto_preserves_references_and_literals() {
        let recipe = collision_recipe();
        let decoded = parse_collision_recipe_dto(crate::dto::recipe_to_dto(&recipe)).unwrap();
        assert_eq!(
            decoded.steps[0].params["app"],
            ParamValue::Ref("inputs.apk".to_string())
        );
        assert_eq!(
            decoded.steps[0].params["expected_package_name"],
            ParamValue::Literal(Value::String("com.example.app".to_string()))
        );
        assert_eq!(
            decoded.steps[0].params["replace_existing"],
            ParamValue::Literal(Value::Bool(false))
        );
    }

    #[test]
    fn collision_request_supports_new_and_legacy_forms_strictly() {
        let recipe = collision_recipe();
        let recipe_dto = crate::dto::recipe_to_dto(&recipe);
        let new_payload = json!({
            "authoredRoot": "/tmp/authored",
            "app": collision_app(),
            "recipe": recipe_dto,
        });
        let parsed = parse_app_recipe_collision_request(new_payload.as_object().unwrap()).unwrap();
        assert_eq!(parsed.recipe_id, "app.example.install");
        assert!(parsed.recipe.is_some());

        let legacy_payload = json!({
            "authoredRoot": "/tmp/authored",
            "app": collision_app(),
            "recipeId": "app.example.install",
        });
        let parsed =
            parse_app_recipe_collision_request(legacy_payload.as_object().unwrap()).unwrap();
        assert_eq!(parsed.recipe_id, "app.example.install");
        assert!(parsed.recipe.is_none());

        for invalid_payload in [
            json!({ "authoredRoot": "/tmp/authored", "app": collision_app() }),
            json!({
                "authoredRoot": "/tmp/authored",
                "app": collision_app(),
                "recipe": crate::dto::recipe_to_dto(&recipe),
                "recipeId": "app.other.install"
            }),
            json!({
                "authoredRoot": "/tmp/authored",
                "app": collision_app(),
                "recipeId": "app.example.install",
                "fingerprint": "frontend-authored"
            }),
        ] {
            assert!(
                parse_app_recipe_collision_request(invalid_payload.as_object().unwrap()).is_err()
            );
        }
    }
}

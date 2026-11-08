use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app_generator_review::{self, SelectedExistingApp};
use crate::catalog_refactor::CatalogChangeIntent;
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
        "qualifyDevice" => handle_qualify_device(object),
        "listDeviceVolumes" => handle_list_device_volumes(object),
        "checkRoot" => handle_check_root(object),
        "inspectApk" => handle_inspect_apk(object),
        "generateAppRecipeDraft" => handle_generate_app_recipe_draft(object, None),
        "generateRemoteAppRecipeDraft" => handle_generate_remote_app_recipe_draft(object, None),
        "generateDeviceProfileDraft" => handle_generate_device_profile_draft(object),
        "checkGeneratedCatalogCollisions" => handle_check_generated_catalog_collisions(object),
        "matchDevice" => handle_match_device(object),
        "listAppGeneratorExistingApps" => handle_list_app_generator_existing_apps(object),
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
    if request_type == "prepareExecutionReview" {
        validate_prepare_execution_review_payload(object)?;
    } else {
        validate_payload(object)?;
    }

    match request_type {
        "hello" => handle_sidecar_hello(object, sessions),
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
        "qualifyDevice" => handle_qualify_device(object),
        "listDeviceVolumes" => handle_list_device_volumes(object),
        "checkRoot" => handle_check_root(object),
        "inspectApk" => handle_inspect_apk(object),
        "generateAppRecipeDraft" => handle_generate_app_recipe_draft(object, Some(sessions)),
        "generateRemoteAppRecipeDraft" => {
            handle_generate_remote_app_recipe_draft(object, Some(sessions))
        }
        "generateDeviceProfileDraft" => handle_generate_device_profile_draft(object),
        "checkGeneratedCatalogCollisions" => handle_check_generated_catalog_collisions(object),
        "listAppGeneratorExistingApps" => handle_list_app_generator_existing_apps(object),
        "selectAppGeneratorExistingApp" => {
            handle_select_app_generator_existing_app(object, sessions)
        }
        "reviewAppGeneratorDraft" => handle_review_app_generator_draft(object, sessions),
        "reviewRemoteAppGeneratorDraft" => {
            handle_review_remote_app_generator_draft(object, sessions)
        }
        "saveAppGeneratorReview" => handle_save_app_generator_review(object, sessions),
        "appGeneratorPublicationStatus" => {
            handle_app_generator_publication_status(object, sessions)
        }
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
        "prepareExecutionReview" => handle_prepare_execution_review(object, sessions),
        "startExecution" => handle_start_execution(object, sessions),
        "getExecution" => handle_get_execution(object, sessions),
        "getExecutionEvents" => handle_get_execution_events(object, sessions),
        "cancelExecution" => handle_cancel_execution(object, sessions),
        "revokeSetupLocationLease" => handle_revoke_setup_location_lease(object, sessions),
        "retireSetupLocationLeaseAttempt" => {
            handle_retire_setup_location_lease_attempt(object, sessions)
        }
        "launchExecutionApp" => handle_launch_execution_app(object, sessions),
        "openRecipe" => handle_open_recipe(object, sessions),
        "openAppDefinition" => handle_open_app_definition(object, sessions),
        "createRecipeFromTemplate" => handle_create_recipe_from_template(object, sessions),
        "getDocument" => handle_get_document(object, sessions),
        "saveRecipe" => handle_save_recipe(object, sessions),
        "saveRecipeAs" => handle_save_recipe_as(object, sessions),
        "saveAppDefinition" => handle_save_app_definition(object, sessions),
        "saveAppDefinitionAs" => handle_save_app_definition_as(object, sessions),
        "saveDocument" => handle_save_document(object, sessions),
        "saveDocumentAs" => handle_save_document_as(object, sessions),
        "applyRecipeCommand" => handle_apply_recipe_command(object, sessions),
        "applyAppDefinitionCommand" => handle_apply_app_definition_command(object, sessions),
        "previewCatalogChange" => handle_preview_catalog_change(object, sessions),
        "applyCatalogChange" => handle_apply_catalog_change(object, sessions),
        "catalogChangeStatus" => handle_catalog_change_status(object, sessions),
        "listCatalogChangeRecoveries" => handle_list_catalog_change_recoveries(object, sessions),
        "retryCatalogChangeRecovery" => handle_retry_catalog_change_recovery(object, sessions),
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
    let setup_location_lease = payload
        .get("setupLocationLease")
        .cloned()
        .map(serde_json::from_value::<crate::storage_targets::SetupLocationExecutionLease>)
        .transpose()
        .map_err(|_| {
            ApiError::invalid_request_with_details(
                "Request field 'setupLocationLease' is invalid.",
                json!({ "field": "setupLocationLease" }),
            )
        })?;
    Ok(envelope::success(
        sessions.executions().start_with_setup_location_lease(
            plan,
            plan_digest,
            mode,
            target,
            setup_location_lease,
        )?,
    ))
}

fn handle_sidecar_hello(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    if let Some(value) = payload.get("runtimeGeneration") {
        let runtime_generation = value.as_u64().ok_or_else(|| {
            ApiError::invalid_request_with_details(
                "Request field 'runtimeGeneration' must be a positive integer.",
                json!({ "field": "runtimeGeneration" }),
            )
        })?;
        sessions
            .executions()
            .set_runtime_generation(runtime_generation)?;
    }
    Ok(envelope::success(protocol::hello_result()))
}

fn handle_prepare_execution_review(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    for forbidden in ["runtimeRoot", "cacheRoot", "adbPath"] {
        if payload.contains_key(forbidden) {
            return Err(ApiError::invalid_request_with_details(
                format!("Request field '{forbidden}' is controlled by the sidecar."),
                json!({ "field": forbidden }),
            ));
        }
    }
    let plan_value = payload.get("plan").ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request field 'plan' is required.",
            json!({ "field": "plan" }),
        )
    })?;
    let plan = crate::cli::parse_execution_plan_json(plan_value).map_err(|message| {
        ApiError::new(
            crate::errors::ApiErrorCode::InvalidExecutionPlan,
            message,
            json!({ "field": "plan" }),
        )
    })?;
    let mode = match required_string(payload, "mode")? {
        "real" => crate::execution_session::ExecutionMode::Real,
        "dry_run" => crate::execution_session::ExecutionMode::DryRun,
        _ => {
            return Err(ApiError::invalid_request_with_details(
                "Request field 'mode' must be 'real' or 'dry_run'.",
                json!({ "field": "mode" }),
            ));
        }
    };
    let target = optional_target_device(payload, "targetDevice")?;
    let storage_targets = match payload.get("storageTargets") {
        Some(value) => serde_json::from_value::<Vec<crate::storage_targets::StorageTargetBinding>>(
            value.clone(),
        )
        .map_err(|_| {
            ApiError::invalid_request_with_details(
                "Request field 'storageTargets' is invalid.",
                json!({ "field": "storageTargets" }),
            )
        })?,
        None => Vec::new(),
    };
    let plan = sessions
        .executions()
        .prepare_review_plan_with_storage_targets(plan, mode, target, storage_targets)?;
    let plan_digest = crate::plan_digest::execution_plan_digest(&plan).map_err(|error| {
        ApiError::new(
            crate::errors::ApiErrorCode::InvalidExecutionPlan,
            format!(
                "Execution plan could not be canonically hashed after review preparation: {error}"
            ),
            json!({ "field": "plan" }),
        )
    })?;
    let review = crate::review_projection::project_review(&plan, &[]);
    let review =
        serde_json::to_value(review).map_err(|_| setup_location_review_projection_error())?;
    let review = sanitize_copy_destination_review(&plan, review)?;
    let plan = serde_json::to_value(plan).map_err(|error| {
        ApiError::new(
            crate::errors::ApiErrorCode::InternalError,
            format!("Prepared execution plan could not be serialized: {error}"),
            json!({}),
        )
    })?;
    Ok(envelope::success(json!({
        "plan": plan,
        "planDigest": plan_digest,
        "review": review,
    })))
}

/// Replace the physical Setup Location boundary in each copy action with the
/// logical choice and any reviewed directory-creation effect.
fn sanitize_copy_destination_review(
    plan: &crate::planner::ExecutionPlan,
    mut review: Value,
) -> Result<Value, ApiError> {
    use crate::executor::copy_commit::CopyCommitDestinationKind;

    let protected_snapshot_count = plan
        .copy_commit_snapshots
        .iter()
        .filter(|snapshot| {
            matches!(
                snapshot.destination_kind,
                CopyCommitDestinationKind::SetupLocation
                    | CopyCommitDestinationKind::SharedDevicePath
            )
        })
        .count();
    if protected_snapshot_count == 0 {
        return sanitize_unprotected_shared_copy_actions(review);
    }

    let features = review
        .get_mut("features")
        .and_then(Value::as_array_mut)
        .ok_or_else(setup_location_review_projection_error)?;
    if features.len() != plan.source.expanded_recipe_refs.len() {
        return Err(setup_location_review_projection_error());
    }

    let mut sanitized_count = 0;
    for (recipe_id, feature) in plan.source.expanded_recipe_refs.iter().zip(features) {
        let snapshots = plan
            .steps
            .iter()
            .filter(|step| step.recipe_ref == *recipe_id)
            .filter_map(|step| {
                plan.copy_commit_snapshots
                    .iter()
                    .find(|snapshot| snapshot.step_id == step.id)
            })
            .collect::<Vec<_>>();
        let sections = feature
            .get_mut("sections")
            .and_then(Value::as_array_mut)
            .ok_or_else(setup_location_review_projection_error)?;
        let mut actions = Vec::new();
        for section in sections {
            if section.get("kind").and_then(Value::as_str) != Some("copies") {
                continue;
            }
            let Some(section_actions) = section.get_mut("actions").and_then(Value::as_array_mut)
            else {
                return Err(setup_location_review_projection_error());
            };
            actions.extend(
                section_actions
                    .iter_mut()
                    .filter(|action| action.get("copyChanges").is_some_and(Value::is_object)),
            );
        }
        if actions.len() != snapshots.len() {
            return Err(setup_location_review_projection_error());
        }

        for (action, snapshot) in actions.into_iter().zip(snapshots) {
            match snapshot.destination_kind {
                CopyCommitDestinationKind::SetupLocation => {
                    let binding = snapshot
                        .setup_location_binding
                        .as_ref()
                        .ok_or_else(setup_location_review_projection_error)?;
                    let precondition = snapshot
                        .setup_destination_precondition
                        .as_ref()
                        .ok_or_else(setup_location_review_projection_error)?;
                    let destination = binding
                        .destination_path()
                        .map_err(|_| setup_location_review_projection_error())?;
                    if snapshot.destination_identity != destination.to_string_lossy() {
                        return Err(setup_location_review_projection_error());
                    }
                    sanitize_setup_location_copy_action(action, binding, precondition)?;
                    sanitized_count += 1;
                }
                CopyCommitDestinationKind::SharedDevicePath => {
                    sanitize_shared_device_copy_action(
                        action,
                        &snapshot.destination_identity,
                        snapshot.destination_binding.as_deref(),
                        true,
                    )?;
                    sanitized_count += 1;
                }
                CopyCommitDestinationKind::AppTarget => {}
            }
        }
    }

    if sanitized_count != protected_snapshot_count {
        return Err(setup_location_review_projection_error());
    }
    sanitize_unprotected_shared_copy_actions(review)
}

/// Replaces physical storage paths in legacy copy actions that have no trusted snapshot.
fn sanitize_unprotected_shared_copy_actions(mut review: Value) -> Result<Value, ApiError> {
    let features = review
        .get_mut("features")
        .and_then(Value::as_array_mut)
        .ok_or_else(setup_location_review_projection_error)?;
    for feature in features {
        let sections = feature
            .get_mut("sections")
            .and_then(Value::as_array_mut)
            .ok_or_else(setup_location_review_projection_error)?;
        for section in sections {
            if section.get("kind").and_then(Value::as_str) != Some("copies") {
                continue;
            }
            let actions = section
                .get_mut("actions")
                .and_then(Value::as_array_mut)
                .ok_or_else(setup_location_review_projection_error)?;
            for action in actions {
                let selected_boundary = action
                    .pointer("/copyChanges/selectedBoundary")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let device_location = action
                    .get("deviceLocation")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let boundary_is_shared = selected_boundary
                    .as_deref()
                    .is_some_and(is_shared_device_destination);
                let location_is_shared = device_location
                    .as_deref()
                    .is_some_and(is_shared_device_destination);
                if location_is_shared
                    && selected_boundary
                        .as_deref()
                        .is_some_and(|boundary| device_location.as_deref() != Some(boundary))
                {
                    return Err(setup_location_review_projection_error());
                }
                if boundary_is_shared {
                    sanitize_shared_device_copy_action(
                        action,
                        selected_boundary
                            .as_deref()
                            .ok_or_else(setup_location_review_projection_error)?,
                        None,
                        false,
                    )?;
                } else if location_is_shared {
                    sanitize_shared_device_copy_action(
                        action,
                        device_location
                            .as_deref()
                            .ok_or_else(setup_location_review_projection_error)?,
                        None,
                        false,
                    )?;
                }
            }
        }
    }
    Ok(review)
}

fn is_shared_device_destination(destination: &str) -> bool {
    ["/storage", "/sdcard", "/mnt/media_rw", "/mnt/media"]
        .iter()
        .any(|root| {
            destination == *root
                || destination
                    .strip_prefix(root)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
}

fn sanitize_setup_location_copy_action(
    action: &mut Value,
    binding: &crate::storage_targets::StorageTargetBinding,
    precondition: &crate::storage_targets::SetupDestinationPrecondition,
) -> Result<(), ApiError> {
    use crate::storage_targets::StorageTargetVolumeKind;

    binding
        .validate()
        .map_err(|_| setup_location_review_projection_error())?;
    let components = binding.relative_path.split('/').collect::<Vec<_>>();
    if precondition.missing_components.len() > components.len() {
        return Err(setup_location_review_projection_error());
    }
    let existing_count = components.len() - precondition.missing_components.len();
    let expected_anchor = components[..existing_count].iter().fold(
        std::path::PathBuf::from(&binding.mount_root),
        |mut path, component| {
            path.push(component);
            path
        },
    );
    if expected_anchor.to_string_lossy() != precondition.existing_anchor_path
        || components[existing_count..]
            != precondition
                .missing_components
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
    {
        return Err(setup_location_review_projection_error());
    }

    let destination = binding
        .destination_path()
        .map_err(|_| setup_location_review_projection_error())?;
    let selected_boundary = action
        .pointer("/copyChanges/selectedBoundary")
        .and_then(Value::as_str)
        .ok_or_else(setup_location_review_projection_error)?;
    if selected_boundary != destination.to_string_lossy() {
        return Err(setup_location_review_projection_error());
    }

    let volume_label = match binding.volume_kind {
        StorageTargetVolumeKind::InternalShared => "Internal storage",
        StorageTargetVolumeKind::RemovableSd => "SD card",
    };
    let creation_effect = if precondition.missing_components.is_empty() {
        String::new()
    } else {
        format!(
            "; create missing directories: {}",
            precondition.missing_components.join("/")
        )
    };
    let public_boundary = format!(
        "{} ({volume_label}) /{}{creation_effect}",
        binding.name, binding.relative_path
    );
    let copy_changes = action
        .get_mut("copyChanges")
        .and_then(Value::as_object_mut)
        .ok_or_else(setup_location_review_projection_error)?;
    copy_changes.insert(
        "selectedBoundary".to_string(),
        Value::String(public_boundary),
    );
    if let Some(object) = action.as_object_mut() {
        object.remove("deviceLocation");
    }

    let serialized =
        serde_json::to_string(action).map_err(|_| setup_location_review_projection_error())?;
    let private_values = [
        binding.id.as_str(),
        binding.device_plan.as_str(),
        binding.volume_handle.as_str(),
        binding.mount_root.as_str(),
        binding.volume_identity.as_str(),
        binding.volume_continuity.as_str(),
        binding.observation_token.as_str(),
        precondition.canonical_scope.as_str(),
        precondition.existing_anchor_path.as_str(),
        precondition.existing_anchor_binding.as_str(),
    ];
    if private_values
        .iter()
        .any(|value| !value.is_empty() && serialized.contains(value))
    {
        return Err(setup_location_review_projection_error());
    }
    Ok(())
}

fn sanitize_shared_device_copy_action(
    action: &mut Value,
    destination_identity: &str,
    destination_binding: Option<&str>,
    manifest_backed: bool,
) -> Result<(), ApiError> {
    let selected_boundary = action
        .pointer("/copyChanges/selectedBoundary")
        .and_then(Value::as_str)
        .map(str::to_string);
    if selected_boundary
        .as_deref()
        .is_some_and(|selected| selected != destination_identity)
        || (selected_boundary.is_none() && action.get("copyChanges").is_some())
    {
        return Err(setup_location_review_projection_error());
    }

    let (public_boundary, mut scope_private_roots) =
        sanitized_shared_destination_boundary(destination_identity);
    let canonical_scope = match destination_binding {
        Some(binding) => crate::executor::canonical_path_from_binding(binding)
            .map_err(|_| setup_location_review_projection_error())?,
        None => crate::executor::canonical_shared_destination(destination_identity)
            .map_err(|_| setup_location_review_projection_error())?,
    };
    let canonical_scope_text = canonical_scope.to_string_lossy().into_owned();
    scope_private_roots.extend(sanitized_shared_destination_boundary(&canonical_scope_text).1);
    let object = action
        .as_object_mut()
        .ok_or_else(setup_location_review_projection_error)?;
    if manifest_backed {
        let copy_changes = object
            .get_mut("copyChanges")
            .and_then(Value::as_object_mut)
            .ok_or_else(setup_location_review_projection_error)?;
        copy_changes.insert(
            "selectedBoundary".to_string(),
            Value::String(public_boundary),
        );
        object.remove("deviceLocation");
    } else {
        object.remove("copyChanges");
        object.insert("deviceLocation".to_string(), Value::String(public_boundary));
    }

    let serialized =
        serde_json::to_string(action).map_err(|_| setup_location_review_projection_error())?;
    let mut private_values = vec![destination_identity.to_string(), canonical_scope_text];
    private_values.extend(scope_private_roots);
    if let Some(binding) = destination_binding {
        private_values.extend(crate::executor::path_binding_private_values(binding));
    }
    if private_values
        .iter()
        .any(|value| !value.is_empty() && serialized.contains(value))
    {
        return Err(setup_location_review_projection_error());
    }
    Ok(())
}

fn sanitized_shared_destination_boundary(destination: &str) -> (String, Vec<String>) {
    for (root, label) in [
        ("/storage/emulated/0", "Internal shared storage"),
        ("/storage/self/primary", "Internal shared storage"),
        ("/sdcard", "Internal shared storage"),
    ] {
        if destination == root {
            return (label.to_string(), vec![root.to_string()]);
        }
        if let Some(suffix) = destination
            .strip_prefix(root)
            .filter(|suffix| suffix.starts_with('/'))
        {
            return (
                format!("{label} /{}", suffix.trim_start_matches('/')),
                vec![root.to_string()],
            );
        }
    }

    if let Some(remainder) = destination.strip_prefix("/storage/") {
        let (volume_component, suffix) = remainder.split_once('/').unwrap_or((remainder, ""));
        if safe_shared_path_component(volume_component)
            && !matches!(volume_component, "emulated" | "self")
        {
            let root = format!("/storage/{volume_component}");
            return (
                append_shared_destination_suffix("Shared device storage", suffix),
                vec![root],
            );
        }
    }

    if let Some(remainder) = destination.strip_prefix("/mnt/media_rw/") {
        let (volume_component, suffix) = remainder.split_once('/').unwrap_or((remainder, ""));
        if safe_shared_path_component(volume_component) {
            let root = format!("/mnt/media_rw/{volume_component}");
            return (
                append_shared_destination_suffix("Shared device storage", suffix),
                vec![root],
            );
        }
    }

    if let Some(suffix) = destination.strip_prefix("/mnt/media/") {
        return (
            append_shared_destination_suffix("Shared device storage", suffix),
            vec!["/mnt/media".to_string()],
        );
    }

    ("Shared device storage destination".to_string(), Vec::new())
}

fn safe_shared_path_component(component: &str) -> bool {
    !component.is_empty()
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn append_shared_destination_suffix(label: &str, suffix: &str) -> String {
    let suffix = suffix.trim_start_matches('/');
    if suffix.is_empty() {
        label.to_string()
    } else {
        format!("{label} /{suffix}")
    }
}

fn setup_location_review_projection_error() -> ApiError {
    ApiError::new(
        crate::errors::ApiErrorCode::InvalidExecutionPlan,
        "Setup Location review could not be safely projected.",
        json!({ "field": "review" }),
    )
}

fn validate_prepare_execution_review_payload(object: &Map<String, Value>) -> Result<(), ApiError> {
    let payload = payload_object(object)?;
    for field in payload.keys() {
        if !matches!(
            field.as_str(),
            "plan" | "mode" | "targetDevice" | "storageTargets"
        ) {
            return Err(ApiError::invalid_request_with_details(
                format!("Request field '{field}' is not supported for review preparation."),
                json!({ "field": field }),
            ));
        }
    }
    Ok(())
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

fn handle_revoke_setup_location_lease(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let context = serde_json::from_value::<crate::storage_targets::SetupLocationExecutionLease>(
        Value::Object(payload.clone()),
    )
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "Setup Location lease revocation is invalid.",
            json!({}),
        )
    })?;
    Ok(envelope::success(
        sessions
            .executions()
            .revoke_setup_location_lease(&context)?,
    ))
}

fn handle_retire_setup_location_lease_attempt(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let context = serde_json::from_value::<crate::storage_targets::SetupLocationExecutionLease>(
        Value::Object(payload.clone()),
    )
    .map_err(|_| {
        ApiError::invalid_request_with_details(
            "Setup Location attempt retirement is invalid.",
            json!({}),
        )
    })?;
    Ok(envelope::success(
        sessions
            .executions()
            .retire_setup_location_lease_attempt(&context)?,
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

fn handle_qualify_device(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let adb_path = required_string(payload, "adbPath")?;
    let serial = required_string(payload, "serial")?;
    Ok(envelope::success(crate::end_user_runtime::qualify_device(
        adb_path, serial,
    )?))
}

fn handle_list_device_volumes(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let adb_path = required_string(payload, "adbPath")?;
    let serial = required_string(payload, "serial")?;
    Ok(envelope::success(
        crate::end_user_runtime::list_device_volumes(adb_path, serial)?,
    ))
}
fn handle_check_root(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let adb_path = required_string(payload, "adbPath")?;
    let serial = required_string(payload, "serial")?;
    Ok(envelope::success(crate::end_user_runtime::check_root(
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
    let result =
        crate::apk_authoring_inspection::inspect_apk_for_authoring(Path::new(&request.apk_path))
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

/// Build one non-committable local App/Recipe draft preview.
///
/// The preview shares the review request shape: the native layer supplies the
/// authored root and, for an existing-App target, the opaque snapshot handle
/// captured by `selectAppGeneratorExistingApp`. This boundary resolves the
/// retained App Definition exactly like a review request, so a preview can never
/// be built from App facts the caller invented.
fn handle_generate_app_recipe_draft(
    object: &Map<String, Value>,
    sessions: Option<&DocumentSessionManager>,
) -> Result<Value, ApiError> {
    let payload = resolve_draft_generator_payload(object, sessions)?;
    let draft_request =
        serde_json::from_value::<crate::generation::AppRecipeDraftRequest>(Value::Object(payload))
            .map_err(|error| {
                ApiError::invalid_request_with_details(
                    format!("App-and-recipe draft input is invalid: {error}"),
                    json!({ "field": "payload" }),
                )
            })?;
    let result = serde_json::to_value(crate::generation::generate_app_recipe_draft(draft_request))
        .map_err(|_| {
            ApiError::command_failed(
                "App-and-recipe draft could not be represented.",
                json!({ "reason": "app_recipe_draft_serialization_failed" }),
            )
        })?;
    Ok(envelope::success(result))
}

/// Resolve one App Generator draft payload for preview consumption.
///
/// The returned payload matches the strict generation request schema: routing
/// fields are removed and, for an existing-App target, the retained App
/// Definition replaces the caller's snapshot handle.
fn resolve_draft_generator_payload(
    object: &Map<String, Value>,
    sessions: Option<&DocumentSessionManager>,
) -> Result<Map<String, Value>, ApiError> {
    let payload = payload_object(object)?;
    let (_, stripped, _) = resolve_app_generator_payload(payload, sessions, false)?;
    Ok(stripped)
}

/// Build one non-committable remote-source App/Recipe draft preview.
///
/// The preview resolves the retained existing-App snapshot through the same
/// trust boundary as its local counterpart.
fn handle_generate_remote_app_recipe_draft(
    object: &Map<String, Value>,
    sessions: Option<&DocumentSessionManager>,
) -> Result<Value, ApiError> {
    let payload = resolve_draft_generator_payload(object, sessions)?;
    let draft_request = serde_json::from_value::<crate::generation::RemoteAppRecipeDraftRequest>(
        Value::Object(payload),
    )
    .map_err(|error| {
        ApiError::invalid_request_with_details(
            format!("Remote app-and-recipe draft input is invalid: {error}"),
            json!({ "field": "payload" }),
        )
    })?;
    let result = serde_json::to_value(crate::generation::generate_remote_app_recipe_draft(
        draft_request,
    ))
    .map_err(|_| {
        ApiError::command_failed(
            "Remote app-and-recipe draft could not be represented.",
            json!({ "reason": "remote_app_recipe_draft_serialization_failed" }),
        )
    })?;
    Ok(envelope::success(result))
}

/// List the catalog Apps one authored root offers for extension.
///
/// The inventory result reports whether every App Definition under the root
/// could be read. An incomplete inventory blocks existing-App selection,
/// because the generator cannot rule out duplicate identities when part of the
/// catalog is unreadable.
fn handle_list_app_generator_existing_apps(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let authored_root = required_string(payload, "authoredRoot")?;
    Ok(envelope::success(app_generator_review::list_existing_apps(
        Path::new(authored_root),
    )?))
}

/// Capture one existing App Definition as a root-bound snapshot handle.
///
/// The returned handle retains the exact file bytes and filesystem identity
/// captured at selection time, so a later review or publication validates
/// against the file the author actually selected.
fn handle_select_app_generator_existing_app(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let authored_root = required_string(payload, "authoredRoot")?;
    let app_path = required_string(payload, "appPath")?;
    Ok(envelope::success(
        app_generator_review::select_existing_app(
            sessions.app_generator_mut(),
            Path::new(authored_root),
            app_path,
        )?,
    ))
}

/// One parsed App Generator request.
///
/// The struct separates backend-only routing fields from the generation input
/// so the generation payload keeps its strict schema.
struct AppGeneratorRequest {
    authored_root: PathBuf,
    request_id: i64,
    artifact_name: Option<String>,
    existing: Option<SelectedExistingApp>,
    payload: Map<String, Value>,
}

/// Parse an App Generator payload and resolve its trusted existing-App snapshot.
///
/// A review always names its authored root and request identity, because the
/// retained review handle binds to that root and orders the author's revisions.
/// The native layer never supplies App Definition facts for an existing-App
/// draft: it provides only the opaque snapshot handle captured by
/// `selectAppGeneratorExistingApp`, and this boundary substitutes the retained
/// App Definition before generation, for previews and reviews alike. A handle
/// captured against a different authored root is treated as invalid rather than
/// re-bound.
fn parse_app_generator_request(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<AppGeneratorRequest, ApiError> {
    let payload = payload_object(object)?;
    let (canonical_root, stripped, existing) =
        resolve_app_generator_payload(payload, Some(sessions), true)?;
    let request_id = required_i64(payload, "requestId")?;
    let artifact_name = optional_string(payload, "artifactName")?.map(str::to_string);
    Ok(AppGeneratorRequest {
        authored_root: canonical_root,
        request_id,
        artifact_name,
        existing,
        payload: stripped,
    })
}

/// Holds the normalized root, forwarded payload, and selected app definition.
type ResolvedAppGeneratorPayload = (PathBuf, Map<String, Value>, Option<SelectedExistingApp>);

fn resolve_app_generator_payload(
    payload: &Map<String, Value>,
    sessions: Option<&DocumentSessionManager>,
    require_root: bool,
) -> Result<ResolvedAppGeneratorPayload, ApiError> {
    if payload.contains_key("existingApp") {
        return Err(ApiError::invalid_request_with_details(
            "Request field 'existingApp' is not accepted; select an existing App and provide 'existingAppSnapshotId'.",
            json!({ "field": "existingApp" }),
        ));
    }
    let snapshot_id = optional_string(payload, "existingAppSnapshotId")?;
    let authored_root = match optional_string(payload, "authoredRoot")? {
        Some(root) => Some(root),
        None if require_root || snapshot_id.is_some() => {
            return Err(ApiError::invalid_request_with_details(
                "Request payload is missing required field: authoredRoot",
                json!({ "field": "authoredRoot" }),
            ));
        }
        None => None,
    };
    let canonical_root = match (authored_root, snapshot_id) {
        (Some(root), Some(handle)) => {
            let canonical = app_generator_review::canonical_authored_root(Path::new(root))?;
            let snapshot = sessions
                .ok_or_else(|| {
                    ApiError::invalid_request_with_details(
                        "An existing-App draft preview requires a stateful session.",
                        json!({ "field": "existingAppSnapshotId" }),
                    )
                })?
                .app_generator()
                .select(handle)
                .cloned()
                .ok_or_else(app_generator_review::review_invalidated_error)?;
            if snapshot.authored_root != canonical {
                return Err(app_generator_review::review_invalidated_error());
            }
            let app = serde_json::to_value(&snapshot.app).map_err(|_| {
                ApiError::command_failed(
                    "The selected App Definition could not be represented.",
                    json!({ "reason": "app_definition_serialization_failed" }),
                )
            })?;
            let mut stripped = payload.clone();
            for field in [
                "authoredRoot",
                "requestId",
                "artifactName",
                "existingAppSnapshotId",
            ] {
                stripped.remove(field);
            }
            stripped.insert("existingApp".to_string(), app);
            return Ok((canonical, stripped, Some(snapshot)));
        }
        (Some(root), None) => app_generator_review::canonical_authored_root(Path::new(root))?,
        (None, _) => PathBuf::new(),
    };
    let mut stripped = payload.clone();
    for field in [
        "authoredRoot",
        "requestId",
        "artifactName",
        "existingAppSnapshotId",
    ] {
        stripped.remove(field);
    }
    Ok((canonical_root, stripped, None))
}

/// Build one immutable review handle from a local App Generator draft.
fn handle_review_app_generator_draft(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let request = parse_app_generator_request(object, sessions)?;
    let draft_request = serde_json::from_value::<crate::generation::AppRecipeDraftRequest>(
        Value::Object(request.payload),
    )
    .map_err(|error| {
        ApiError::invalid_request_with_details(
            format!("App-and-recipe review input is invalid: {error}"),
            json!({ "field": "payload" }),
        )
    })?;
    let permission_automation_enabled = draft_request.permission_automation_enabled;
    let draft = crate::generation::generate_app_recipe_draft(draft_request);
    let intent = app_generator_review::ReviewIntent {
        authored_root: request.authored_root,
        request_id: request.request_id,
        artifact_name: request.artifact_name,
        existing: request.existing,
        permission_automation_enabled,
    };
    Ok(envelope::success(app_generator_review::review_from_draft(
        sessions.app_generator_mut(),
        draft,
        intent,
    )?))
}

/// Build one immutable review handle from a remote-source App Generator draft.
fn handle_review_remote_app_generator_draft(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let request = parse_app_generator_request(object, sessions)?;
    let draft_request = serde_json::from_value::<crate::generation::RemoteAppRecipeDraftRequest>(
        Value::Object(request.payload),
    )
    .map_err(|error| {
        ApiError::invalid_request_with_details(
            format!("Remote app-and-recipe review input is invalid: {error}"),
            json!({ "field": "payload" }),
        )
    })?;
    let permission_automation_enabled = draft_request.permission_automation_enabled;
    let draft = crate::generation::generate_remote_app_recipe_draft(draft_request);
    let intent = app_generator_review::ReviewIntent {
        authored_root: request.authored_root,
        request_id: request.request_id,
        artifact_name: request.artifact_name,
        existing: request.existing,
        permission_automation_enabled,
    };
    Ok(envelope::success(
        app_generator_review::review_from_remote_draft(
            sessions.app_generator_mut(),
            draft,
            intent,
        )?,
    ))
}

/// Publish one reviewed generated App/Recipe pair.
///
/// The native layer passes only the operation identity it recorded and the
/// immutable review handle; the retained candidate pair is published exactly
/// as reviewed, and a repeated operation id replays its recorded result.
fn handle_save_app_generator_review(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let operation_id = required_string(payload, "operationId")?;
    let review_id = required_string(payload, "reviewId")?;
    let request_id = required_i64(payload, "requestId")?;
    Ok(envelope::success(sessions.save_generated_app_review(
        operation_id,
        review_id,
        request_id,
    )?))
}

/// Report the recorded publication outcome for one native operation.
///
/// A missing record reports `unknown` with `detail.reason` set to
/// `no_record`: the native layer interprets that as an unconfirmed outcome and
/// never retries automatically.
fn handle_app_generator_publication_status(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let operation_id = required_string(payload, "operationId")?;
    Ok(envelope::success(app_generator_review::publication_status(
        sessions.app_generator(),
        operation_id,
    )?))
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
        .ok_or_else(invalid)
        .and_then(|value| serde_json::from_value(value).map_err(|_| invalid()))?;
    let legacy_recipe_id = match payload.get("recipeId") {
        None => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => return Err(invalid()),
    };
    let recipe = match payload.get("recipe") {
        None => None,
        Some(value) => Some(parse_collision_recipe_dto(value.clone()).ok_or_else(invalid)?),
    };
    let recipe_id = match (&recipe, legacy_recipe_id) {
        (Some(recipe), Some(recipe_id)) if recipe.id == recipe_id => recipe_id,
        (Some(_), Some(_)) => return Err(invalid()),
        (Some(recipe), None) => recipe.id.clone(),
        (None, Some(recipe_id)) => recipe_id,
        (None, None) => return Err(invalid()),
    };
    Ok(crate::generation::AppRecipeCollisionRequest { app, recipe_id })
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
    app_ref: Option<String>,
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
        if let (Some(app_ref), Some(object)) = (self.app_ref, value.as_object_mut()) {
            object.insert("app_ref".to_string(), Value::String(app_ref));
        }
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
    let runtime_capability_availability =
        optional_runtime_capability_availability(payload, "runtimeCapabilityAvailability")?;
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
        runtime_capability_availability,
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

fn handle_open_app_definition(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(
        sessions.open_app_definition(path, authored_root)?,
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

fn handle_save_app_definition(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(
        sessions.save_app_definition(document_id)?,
    ))
}

fn handle_save_app_definition_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    Ok(envelope::success(
        sessions.save_app_definition_as(document_id, path)?,
    ))
}

fn handle_save_document(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.save_document(document_id)?))
}

fn handle_save_document_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    Ok(envelope::success(
        sessions.save_document_as(document_id, path)?,
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

fn handle_apply_app_definition_command(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let command = payload.get("command").unwrap_or(&Value::Null);
    Ok(envelope::success(
        sessions.apply_app_definition_command(document_id, command)?,
    ))
}

fn handle_preview_catalog_change(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let intent = payload.get("intent").ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: intent",
            json!({ "field": "intent" }),
        )
    })?;
    let intent: CatalogChangeIntent = serde_json::from_value(intent.clone()).map_err(|error| {
        ApiError::invalid_request_with_details(
            format!("Request field 'intent' is invalid: {error}"),
            json!({ "field": "intent" }),
        )
    })?;
    Ok(envelope::success(
        sessions.preview_catalog_change(document_id, intent)?,
    ))
}

fn handle_apply_catalog_change(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.apply_catalog_change(
        required_string(payload, "operationId")?,
        required_string(payload, "previewToken")?,
    )?))
}

fn handle_catalog_change_status(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.catalog_change_status(
        required_string(payload, "authoredRoot")?,
        required_string(payload, "operationId")?,
    )?))
}

fn handle_list_catalog_change_recoveries(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.list_catalog_change_recoveries(
        required_string(payload, "authoredRoot")?,
    )?))
}

fn handle_retry_catalog_change_recovery(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.retry_catalog_change_recovery(
        required_string(payload, "authoredRoot")?,
        required_string(payload, "operationId")?,
    )?))
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

fn required_i64(payload: &Map<String, Value>, field: &str) -> Result<i64, ApiError> {
    match payload.get(field) {
        Some(Value::Number(number)) => number.as_i64().ok_or_else(|| {
            ApiError::invalid_request_with_details(
                format!("Request field '{field}' must be a signed integer."),
                json!({ "field": field }),
            )
        }),
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
                .map(Box::new)
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

fn optional_runtime_capability_availability(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Option<runtime_configuration::RuntimeCapabilityAvailability>, ApiError> {
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
            "package_id": "com.example.app",
            "artifacts": {
                "apk": { "kind": "apk", "source": { "strategy": "user_provided" } }
            },
            "metadata": {}
        })
    }

    /// The retired pre-v1 App Definition shape.
    ///
    /// Kept only so the request boundary can prove that the strict v1 parser
    /// never accepts legacy App Definition payloads through a compatibility
    /// path.
    fn legacy_collision_app() -> Value {
        json!({
            "schema_version": 1,
            "kind": "app_definition",
            "id": "example",
            "name": "Example",
            "category": "utility",
            "package": { "primary": "com.example.app", "aliases": [] },
            "artifacts": { "apk": { "required": true } },
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
    fn qualify_device_dispatch_requires_the_serial_field() {
        let mut sessions = crate::session::DocumentSessionManager::default();
        let response = handle_sidecar_value(
            json!({
                "id": "qualification-test",
                "type": "qualifyDevice",
                "payload": { "adbPath": "/managed/adb" },
            }),
            &mut sessions,
        );

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["details"]["field"], "serial");
        assert_ne!(
            response["error"]["message"],
            "Unknown request type: qualifyDevice"
        );
    }

    #[test]
    fn check_root_dispatch_requires_the_serial_field() {
        let mut sessions = crate::session::DocumentSessionManager::default();
        let response = handle_sidecar_value(
            json!({
                "id": "root-check-test",
                "type": "checkRoot",
                "payload": { "adbPath": "/managed/adb" },
            }),
            &mut sessions,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["details"]["field"], "serial");
        assert_ne!(
            response["error"]["message"],
            "Unknown request type: checkRoot"
        );
    }

    #[test]
    fn retire_setup_location_attempt_dispatches_through_the_trusted_session_manager() {
        let mut sessions = crate::session::DocumentSessionManager::default();
        sessions.executions().set_runtime_generation(7).unwrap();
        let context = crate::storage_targets::SetupLocationExecutionLease {
            attempt_sequence: 1,
            device_handle: "device-test".to_string(),
            session_epoch: 4,
            runtime_generation: 7,
            plan_id: "plan-test".to_string(),
            plan_digest: "a".repeat(64),
        };
        let response = handle_sidecar_value(
            json!({
            "id": "retire-setup-location-attempt",
            "type": "retireSetupLocationLeaseAttempt",
            "payload": context,
            }),
            &mut sessions,
        );
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"]["retired"], true);
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

        let legacy_payload = json!({
            "authoredRoot": "/tmp/authored",
            "app": collision_app(),
            "recipeId": "app.example.install",
        });
        let parsed =
            parse_app_recipe_collision_request(legacy_payload.as_object().unwrap()).unwrap();
        assert_eq!(parsed.recipe_id, "app.example.install");

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
            json!({
                "authoredRoot": "/tmp/authored",
                "app": legacy_collision_app(),
                "recipeId": "app.example.install",
            }),
        ] {
            assert!(
                parse_app_recipe_collision_request(invalid_payload.as_object().unwrap()).is_err()
            );
        }
    }

    #[test]
    fn setup_location_review_action_hides_physical_binding_and_shows_creation_effect() {
        let binding = crate::storage_targets::StorageTargetBinding {
            id: "location-id".to_string(),
            device_plan: "device-plan".to_string(),
            session_epoch: 7,
            name: "ROM Library".to_string(),
            relative_path: "roms/library".to_string(),
            volume_handle: "1".repeat(32),
            volume_kind: crate::storage_targets::StorageTargetVolumeKind::RemovableSd,
            mount_root: "/storage/AB12-CD34".to_string(),
            volume_identity: "2".repeat(32),
            volume_continuity: "3".repeat(32),
            observation_token: "4".repeat(32),
        };
        let precondition = crate::storage_targets::SetupDestinationPrecondition {
            canonical_scope: "/mnt/media_rw/AB12-CD34/roms/library".to_string(),
            existing_anchor_path: "/storage/AB12-CD34/roms".to_string(),
            existing_anchor_binding: "/mnt/media_rw/AB12-CD34/roms|parent:1|selected:2".to_string(),
            missing_components: vec!["library".to_string()],
        };
        let mut action = json!({
            "deviceLocation": "/storage/AB12-CD34/roms/library",
            "copyChanges": {
                "selectedBoundary": "/storage/AB12-CD34/roms/library",
                "additions": ["game.rom"],
                "replacements": [],
                "deletions": []
            }
        });

        sanitize_setup_location_copy_action(&mut action, &binding, &precondition).unwrap();

        assert_eq!(
            action
                .pointer("/copyChanges/selectedBoundary")
                .and_then(Value::as_str),
            Some("ROM Library (SD card) /roms/library; create missing directories: library")
        );
        assert!(action.get("deviceLocation").is_none());
        let serialized = serde_json::to_string(&action).unwrap();
        for private in [
            binding.id.as_str(),
            binding.device_plan.as_str(),
            binding.volume_handle.as_str(),
            binding.mount_root.as_str(),
            binding.volume_identity.as_str(),
            binding.volume_continuity.as_str(),
            binding.observation_token.as_str(),
            precondition.canonical_scope.as_str(),
            precondition.existing_anchor_path.as_str(),
            precondition.existing_anchor_binding.as_str(),
        ] {
            assert!(!serialized.contains(private), "review exposed {private}");
        }
    }

    #[test]
    fn setup_location_review_action_rejects_an_unmatched_destination() {
        let binding = crate::storage_targets::StorageTargetBinding {
            id: "location-id".to_string(),
            device_plan: "device-plan".to_string(),
            session_epoch: 1,
            name: "Internal ROMs".to_string(),
            relative_path: "roms".to_string(),
            volume_handle: "1".repeat(32),
            volume_kind: crate::storage_targets::StorageTargetVolumeKind::InternalShared,
            mount_root: "/storage/emulated/0".to_string(),
            volume_identity: "2".repeat(32),
            volume_continuity: "3".repeat(32),
            observation_token: "4".repeat(32),
        };
        let precondition = crate::storage_targets::SetupDestinationPrecondition {
            canonical_scope: "/mnt/user/0/primary/roms".to_string(),
            existing_anchor_path: "/storage/emulated/0".to_string(),
            existing_anchor_binding: "/mnt/user/0/primary|parent:1|selected:2".to_string(),
            missing_components: vec!["roms".to_string()],
        };
        let mut action = json!({
            "copyChanges": { "selectedBoundary": "/storage/other/roms" }
        });

        assert!(sanitize_setup_location_copy_action(&mut action, &binding, &precondition).is_err());
        assert_eq!(
            action
                .pointer("/copyChanges/selectedBoundary")
                .and_then(Value::as_str),
            Some("/storage/other/roms")
        );
    }

    #[test]
    fn shared_recipe_setup_location_refs_bind_distinct_volumes_into_reviewed_effects() {
        use crate::execution_session::{
            ExecutionMode, ExecutionSessionManager, SidecarRuntimeConfig,
        };
        use crate::model::OrderedMap;
        use crate::planner::{
            DeviceContext, ExecutionDevicePlanSnapshot, PlannerInput, RuntimeCapabilities,
        };
        use crate::storage_targets::{StorageTargetBinding, StorageTargetVolumeKind};

        const RECIPE_YAML: &str = r#"
schema_version: 1
kind: recipe
id: test.setup_location_copy
name: Setup Location copy
recipe_dependencies: []
provides:
  features: []
inputs:
  source:
    type: directory
    role: source
    label: Copy source
    required: true
  destination:
    type: setup_location_ref
    role: destination
    label: Setup Location
    required: true
artifacts: {}
steps:
  - id: copy
    type: copy_files
    name: Copy selected files
    user_toggleable: false
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params:
      source: { ref: inputs.source }
      dest: { ref: inputs.destination }
      copy_policy: merge
    verify: []
"#;

        let temporary = tempfile::tempdir().expect("temporary review root");
        let rom_source = temporary.path().join("rom-source");
        let save_source = temporary.path().join("save-source");
        std::fs::create_dir_all(&rom_source).expect("ROM source directory");
        std::fs::create_dir_all(&save_source).expect("save source directory");
        std::fs::write(rom_source.join("game.rom"), b"reviewed ROM").expect("ROM source file");
        std::fs::write(save_source.join("profile.dat"), b"reviewed save")
            .expect("save source file");

        let recipe = |id: &str, name: &str| {
            let mut recipe = crate::yaml::load_recipe_from_bytes(
                RECIPE_YAML.as_bytes(),
                std::path::Path::new("recipes/setup-location-copy.yaml"),
            )
            .expect("shared Setup Location Recipe parses");
            recipe.id = id.to_string();
            recipe.name = name.to_string();
            recipe
        };
        let recipe_ids = ["test.copy_roms", "test.copy_saves"];
        let mut input_bindings = OrderedMap::new();
        input_bindings.insert(
            format!("{}/source", recipe_ids[0]),
            serde_json::json!(rom_source.to_string_lossy()),
        );
        input_bindings.insert(
            format!("{}/destination", recipe_ids[0]),
            serde_json::json!("location-roms"),
        );
        input_bindings.insert(
            format!("{}/source", recipe_ids[1]),
            serde_json::json!(save_source.to_string_lossy()),
        );
        input_bindings.insert(
            format!("{}/destination", recipe_ids[1]),
            serde_json::json!("location-saves"),
        );

        let input = PlannerInput {
            recipes: vec![
                recipe(recipe_ids[0], "Copy ROM library"),
                recipe(recipe_ids[1], "Copy save library"),
            ],
            selected_recipe_refs: recipe_ids.iter().map(|id| (*id).to_string()).collect(),
            explicit_input_bindings: input_bindings,
            user_configuration_bindings: OrderedMap::new(),
            device_plan_input_bindings: OrderedMap::new(),
            plan_id: "plan.setup_locations.integration".to_string(),
            device_plan_ref: "example.setup_locations".to_string(),
            device_profile_ref: "example.device".to_string(),
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Device".to_string(),
                android_version: 13,
                android_api_level: Some(33),
                device_tags: Vec::new(),
                root_shell_mode: None,
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: false,
                shared_storage_write: true,
                app_launch: false,
                shell_command: false,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
                root_shell_route: None,
            },
            catalog_identity: None,
            target_device: None,
            device_plan_presentation: Some(ExecutionDevicePlanSnapshot {
                id: "example.setup_locations".to_string(),
                name: Some("Example Setup Locations".to_string()),
                description: None,
            }),
        };
        let planning = crate::planner::plan_execution(input);
        assert_eq!(
            planning.status,
            crate::planner::PlanningStatus::Success,
            "both Recipes should accept their shared typed destination contract: {:?}",
            planning.errors
        );
        let plan = planning.execution_plan.expect("planned Recipe graph");
        let setup_location_inputs = plan
            .inputs
            .iter()
            .filter(|input| {
                input
                    .value
                    .value
                    .as_str()
                    .is_some_and(|value| value.starts_with("location-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(setup_location_inputs.len(), 2);
        assert!(
        setup_location_inputs
            .iter()
            .all(|input| input.value.type_name == "setup_location_ref"),
        "PlannerInput bindings must retain the shared Setup Location type: {setup_location_inputs:#?}"
    );

        let binding = |id: &str,
                       name: &str,
                       volume_kind: StorageTargetVolumeKind,
                       mount_root: &str,
                       relative_path: &str,
                       evidence: [char; 4]| StorageTargetBinding {
            id: id.to_string(),
            device_plan: "example.setup_locations".to_string(),
            session_epoch: 1,
            name: name.to_string(),
            relative_path: relative_path.to_string(),
            volume_handle: evidence[0].to_string().repeat(32),
            volume_kind,
            mount_root: mount_root.to_string(),
            volume_identity: evidence[1].to_string().repeat(32),
            volume_continuity: evidence[2].to_string().repeat(32),
            observation_token: evidence[3].to_string().repeat(32),
        };
        let storage_targets = vec![
            binding(
                "location-roms",
                "ROM Library",
                StorageTargetVolumeKind::InternalShared,
                "/storage/emulated/0",
                "roms/library",
                ['a', 'b', 'c', 'd'],
            ),
            binding(
                "location-saves",
                "Save Library",
                StorageTargetVolumeKind::RemovableSd,
                "/storage/ABCD-1234",
                "saves/library",
                ['5', '6', '7', '8'],
            ),
        ];

        let manager = ExecutionSessionManager::new(SidecarRuntimeConfig {
            runtime_root: temporary.path().join("runtime"),
            cache_root: temporary.path().join("cache"),
            adb_path: "adb".to_string(),
        });
        let reviewed_plan = manager
            .prepare_review_plan_with_storage_targets(
                plan,
                ExecutionMode::DryRun,
                None,
                storage_targets,
            )
            .expect("dry-run review preparation uses only the fake device adapter");

        assert_eq!(
            reviewed_plan.copy_commit_snapshots.len(),
            2,
            "reviewed plan: {reviewed_plan:#?}"
        );
        let mut bound_locations = reviewed_plan
            .copy_commit_snapshots
            .iter()
            .map(|snapshot| {
                let binding = snapshot
                    .setup_location_binding
                    .as_ref()
                    .expect("copy snapshot retains trusted Setup Location binding");
                (binding.id.as_str(), binding.volume_kind)
            })
            .collect::<Vec<_>>();
        bound_locations.sort_by_key(|(id, _)| *id);
        assert_eq!(
            bound_locations,
            vec![
                ("location-roms", StorageTargetVolumeKind::InternalShared),
                ("location-saves", StorageTargetVolumeKind::RemovableSd),
            ]
        );

        let review = crate::review_projection::project_review(&reviewed_plan, &[]);
        let review = serde_json::to_value(review).expect("review projection serializes");
        let features = review["features"]
            .as_array()
            .expect("review contains Recipe features");
        assert_eq!(
            features.len(),
            2,
            "both selected Recipe references are reviewed"
        );

        for (feature_name, location_name, boundary, addition, missing_components) in [
            (
                "Copy ROM library",
                "ROM Library",
                "/storage/emulated/0/roms/library",
                "game.rom",
                ["roms", "library"],
            ),
            (
                "Copy save library",
                "Save Library",
                "/storage/ABCD-1234/saves/library",
                "profile.dat",
                ["saves", "library"],
            ),
        ] {
            let feature = features
                .iter()
                .find(|feature| feature["name"] == feature_name)
                .unwrap_or_else(|| panic!("review is missing feature {feature_name}"));
            let action = feature["sections"]
                .as_array()
                .expect("review sections")
                .iter()
                .find(|section| section["kind"] == "copies")
                .and_then(|section| section["actions"].as_array())
                .and_then(|actions| actions.first())
                .expect("Recipe review contains a copy action");
            assert_eq!(
                action["copyChanges"]["selectedBoundary"], boundary,
                "the copy effect must use the corresponding trusted destination"
            );
            assert_eq!(
                action["copyChanges"]["additions"],
                serde_json::json!([addition]),
                "reviewed additions must come from this Recipe's source manifest"
            );

            let snapshot = reviewed_plan
                .copy_commit_snapshots
                .iter()
                .find(|snapshot| {
                    snapshot
                        .setup_location_binding
                        .as_ref()
                        .is_some_and(|binding| binding.name == location_name)
                })
                .expect("the review snapshot retains the named location binding");
            assert_eq!(
                snapshot
                    .setup_destination_precondition
                    .as_ref()
                    .expect("review snapshot retains destination preconditions")
                    .missing_components,
                missing_components,
                "directory creation is part of the immutable reviewed effect"
            );
        }
    }

    #[test]
    fn copy_review_sanitizes_mixed_setup_and_legacy_shared_destinations() {
        use crate::executor::copy_commit::{
            CopyCommitDestinationKind, CopyCommitOperation, CopyCommitRequest,
            ExecutionCopyCommitSnapshot,
        };
        use crate::planner::{
            DeviceContext, ExecutionDevicePlanSnapshot, ExecutionParamValue, ExecutionPlan,
            ExecutionPlanSource, ExecutionRecipeSnapshot, ExecutionStep, ExecutionStepConstraints,
            RuntimeCapabilities,
        };

        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let fake_root = temporary.path().join("fake-device");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&fake_root).unwrap();
        let binding = crate::storage_targets::StorageTargetBinding {
            id: "location-id".to_string(),
            device_plan: "device-plan".to_string(),
            session_epoch: 7,
            name: "ROM Library".to_string(),
            relative_path: "roms/library".to_string(),
            volume_handle: "1".repeat(32),
            volume_kind: crate::storage_targets::StorageTargetVolumeKind::RemovableSd,
            mount_root: "/storage/AB12-CD34".to_string(),
            volume_identity: "2".repeat(32),
            volume_continuity: "3".repeat(32),
            observation_token: "4".repeat(32),
        };
        let precondition = crate::storage_targets::SetupDestinationPrecondition {
            canonical_scope: "/mnt/media_rw/AB12-CD34/roms/library".to_string(),
            existing_anchor_path: "/storage/AB12-CD34/roms".to_string(),
            existing_anchor_binding: "/mnt/media_rw/AB12-CD34/roms|parent:1|selected:2".to_string(),
            missing_components: vec!["library".to_string()],
        };
        let request = CopyCommitRequest::reviewed_setup_directory(
            &source,
            &fake_root,
            binding,
            precondition,
            CopyCommitOperation::AddUpdate,
            Vec::new(),
        )
        .unwrap();
        let snapshot = ExecutionCopyCommitSnapshot::from_request("copy", &source, &request);
        let step = ExecutionStep {
            id: "copy".to_string(),
            recipe_ref: "recipe".to_string(),
            app_id: None,
            type_name: "copy_files".to_string(),
            name: "Copy ROMs".to_string(),
            note: String::new(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            permission_plan: None,
            params: crate::model::OrderedMap::new(),
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        let legacy_destination = "/storage/EF56-789A/roms/legacy/settings.cfg";
        let mut legacy_step = step.clone();
        legacy_step.id = "legacy-copy".to_string();
        legacy_step.name = "Copy legacy settings".to_string();
        legacy_step.params.insert(
            "source".to_string(),
            ExecutionParamValue::Literal {
                value: json!({
                    "type": "file_path",
                    "value": "/tmp/legacy-settings.cfg",
                    "location": "host"
                }),
            },
        );
        legacy_step.params.insert(
            "dest".to_string(),
            ExecutionParamValue::Literal {
                value: json!(legacy_destination),
            },
        );
        let mut plan = ExecutionPlan {
            id: "plan".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "profile".to_string(),
                device_plan_ref: "device-plan".to_string(),
                selected_recipe_refs: vec!["recipe".to_string()],
                expanded_recipe_refs: vec!["recipe".to_string()],
                catalog: None,
            },
            device_plan: Some(ExecutionDevicePlanSnapshot {
                id: "device-plan".to_string(),
                name: None,
                description: None,
            }),
            recipes: vec![ExecutionRecipeSnapshot {
                id: "recipe".to_string(),
                name: "ROM setup".to_string(),
                description: None,
            }],
            apps: Vec::new(),
            resolved_inputs: Vec::new(),
            copy_commit_snapshots: vec![snapshot],
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Device".to_string(),
                android_version: 1,
                android_api_level: None,
                device_tags: Vec::new(),
                root_shell_mode: None,
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: false,
                shared_storage_write: true,
                app_launch: false,
                shell_command: false,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
                root_shell_route: None,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: vec![step, legacy_step],
            schema_version: 1,
            kind: "execution_plan",
        };
        let mixed_projection =
            serde_json::to_value(crate::review_projection::project_review(&plan, &[])).unwrap();
        let projected_actions = &mixed_projection["features"][0]["sections"][0]["actions"];
        assert_eq!(projected_actions.as_array().unwrap().len(), 2);
        assert_eq!(projected_actions[1]["deviceLocation"], legacy_destination);
        assert!(projected_actions[1].get("copyChanges").is_none());

        let sanitized_mixed = sanitize_copy_destination_review(&plan, mixed_projection).unwrap();
        let mixed_actions = &sanitized_mixed["features"][0]["sections"][0]["actions"];
        assert_eq!(
            mixed_actions[1]["deviceLocation"],
            "Shared device storage /roms/legacy/settings.cfg"
        );
        assert!(mixed_actions[1].get("copyChanges").is_none());
        let serialized_mixed = serde_json::to_string(&sanitized_mixed).unwrap();
        assert!(!serialized_mixed.contains("/storage/EF56-789A"));
        assert!(!serialized_mixed.contains("EF56-789A"));

        let review = json!({
            "features": [{
                "sections": [{
                    "kind": "copies",
                    "actions": [{
                        "deviceLocation": "/storage/AB12-CD34/roms/library",
                        "copyChanges": {
                            "selectedBoundary": "/storage/AB12-CD34/roms/library",
                            "additions": [],
                            "replacements": [],
                            "deletions": []
                        }
                    }]
                }]
            }]
        });

        let sanitized = sanitize_copy_destination_review(&plan, review).unwrap();

        let action = &sanitized["features"][0]["sections"][0]["actions"][0];
        assert_eq!(
            action
                .pointer("/copyChanges/selectedBoundary")
                .and_then(Value::as_str),
            Some("ROM Library (SD card) /roms/library; create missing directories: library")
        );
        assert!(action.get("deviceLocation").is_none());
        let serialized = serde_json::to_string(action).unwrap();
        assert!(!serialized.contains("/storage/AB12-CD34"));
        assert!(!serialized.contains("volume_identity"));
        assert!(!serialized.contains("continuity"));

        let legacy_snapshot = &mut plan.copy_commit_snapshots[0];
        legacy_snapshot.destination_kind = CopyCommitDestinationKind::SharedDevicePath;
        legacy_snapshot.destination_identity = "/storage/AB12-CD34/roms/library".to_string();
        legacy_snapshot.destination_binding = Some(
            json!({
                "canonicalPath": "/mnt/media_rw/AB12-CD34/roms/library",
                "parentIdentity": "parent:1",
                "selectedIdentity": "selected:2",
            })
            .to_string(),
        );
        legacy_snapshot.setup_location_binding = None;
        legacy_snapshot.setup_destination_precondition = None;
        let legacy_review = json!({
            "features": [{
                "sections": [{
                    "kind": "copies",
                    "actions": [{
                        "deviceLocation": "/storage/AB12-CD34/roms/library",
                        "copyChanges": {
                            "selectedBoundary": "/storage/AB12-CD34/roms/library",
                            "additions": [],
                            "replacements": [],
                            "deletions": []
                        }
                    }]
                }]
            }]
        });
        let mut leaking_legacy_review = legacy_review.clone();
        leaking_legacy_review["features"][0]["sections"][0]["actions"][0]["privateIdentity"] =
            Value::String("selected:2".to_string());
        assert!(sanitize_copy_destination_review(&plan, leaking_legacy_review).is_err());

        let sanitized_legacy = sanitize_copy_destination_review(&plan, legacy_review).unwrap();
        let legacy_action = &sanitized_legacy["features"][0]["sections"][0]["actions"][0];
        assert_eq!(
            legacy_action
                .pointer("/copyChanges/selectedBoundary")
                .and_then(Value::as_str),
            Some("Shared device storage /roms/library")
        );
        assert!(legacy_action.get("deviceLocation").is_none());
        let serialized_legacy = serde_json::to_string(legacy_action).unwrap();
        for private_value in [
            "/storage/AB12-CD34",
            "/mnt/media_rw/AB12-CD34",
            "AB12-CD34",
            "parent:1",
        ] {
            assert!(!serialized_legacy.contains(private_value));
        }

        let destination = "/storage/AB12-CD34/roms/library/settings.cfg";
        plan.copy_commit_snapshots.clear();
        plan.steps[0].params.insert(
            "source".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!({
                    "type": "file_path",
                    "value": "/tmp/settings.cfg",
                    "location": "host"
                }),
            },
        );
        plan.steps[0].params.insert(
            "dest".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!(destination),
            },
        );
        let unsnapshotted_legacy_review = json!({
            "features": [{
                "sections": [{
                    "kind": "copies",
                    "actions": [{
                        "deviceLocation": destination,
                        "copyChanges": {
                            "selectedBoundary": destination,
                            "additions": [],
                            "replacements": [],
                            "deletions": []
                        }
                    }]
                }]
            }]
        });
        let sanitized_unsnapshotted =
            sanitize_copy_destination_review(&plan, unsnapshotted_legacy_review).unwrap();
        let unsnapshotted_action =
            &sanitized_unsnapshotted["features"][0]["sections"][0]["actions"][0];
        assert_eq!(
            unsnapshotted_action["deviceLocation"],
            "Shared device storage /roms/library/settings.cfg"
        );
        assert!(unsnapshotted_action.get("copyChanges").is_none());
        let serialized_unsnapshotted = serde_json::to_string(unsnapshotted_action).unwrap();
        assert!(!serialized_unsnapshotted.contains("/storage/AB12-CD34"));
        assert!(!serialized_unsnapshotted.contains("AB12-CD34"));

        let mut actual_request_plan = plan.clone();
        actual_request_plan.steps = vec![plan.steps[1].clone()];
        actual_request_plan.copy_commit_snapshots.clear();
        actual_request_plan.steps[0].params.insert(
            "dest_kind".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!("file"),
            },
        );
        let request = json!({
            "payload": {
                "plan": serde_json::to_value(&actual_request_plan).unwrap(),
                "mode": "dry_run"
            }
        });
        let response = super::handle_prepare_execution_review(
            request.as_object().unwrap(),
            &crate::session::DocumentSessionManager::default(),
        )
        .unwrap();
        assert_eq!(response["ok"], true);
        let actual_action =
            &response["result"]["review"]["features"][0]["sections"][0]["actions"][0];
        assert_eq!(
            actual_action["deviceLocation"],
            "Shared device storage /roms/legacy/settings.cfg"
        );
        assert!(actual_action.get("copyChanges").is_none());
        let actual_public_review = serde_json::to_string(actual_action).unwrap();
        assert!(!actual_public_review.contains("/storage/EF56-789A"));
        assert!(!actual_public_review.contains("EF56-789A"));

        let mut mismatched_projection = serde_json::to_value(
            crate::review_projection::project_review(&actual_request_plan, &[]),
        )
        .unwrap();
        mismatched_projection["features"][0]["sections"][0]["actions"][0]["copyChanges"] =
            json!({"selectedBoundary": "/storage/EF56-789A/roms/other/settings.cfg"});
        assert!(
            sanitize_copy_destination_review(&actual_request_plan, mismatched_projection).is_err()
        );

        let mut mixed_request_plan = plan.clone();
        mixed_request_plan.copy_commit_snapshots.clear();
        mixed_request_plan.inputs.clear();
        let mixed_runtime_root = temporary.path().join("request-runtime");
        let mixed_cache_root = temporary.path().join("request-cache");
        let mixed_source = mixed_cache_root.join("mixed-source");
        std::fs::create_dir_all(&mixed_source).unwrap();
        mixed_request_plan.steps[0].params.insert(
            "source".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!({
                    "type": "directory_path",
                    "value": mixed_source.to_string_lossy(),
                    "location": "host"
                }),
            },
        );
        mixed_request_plan.steps[0].params.insert(
            "dest".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!("/storage/AB12-CD34/roms/library"),
            },
        );
        mixed_request_plan.steps[0].params.insert(
            "dest_kind".to_string(),
            crate::planner::ExecutionParamValue::Literal {
                value: json!("directory"),
            },
        );
        let mixed_request = json!({
            "payload": {
                "plan": serde_json::to_value(&mixed_request_plan).unwrap(),
                "mode": "dry_run"
            }
        });
        let mixed_sessions = crate::session::DocumentSessionManager::with_runtime_config(
            crate::execution_session::SidecarRuntimeConfig {
                runtime_root: mixed_runtime_root,
                cache_root: mixed_cache_root,
                adb_path: "adb".to_string(),
            },
        );
        let mixed_response = super::handle_prepare_execution_review(
            mixed_request.as_object().unwrap(),
            &mixed_sessions,
        );
        assert!(
            mixed_response.is_ok(),
            "public mixed review preparation failed at request handling: {mixed_response:?}"
        );
        let mixed_response = mixed_response.unwrap();
        assert_eq!(mixed_response["ok"], true);
        let mixed_public_review = &mixed_response["result"]["review"];
        let mixed_public_actions = &mixed_public_review["features"][0]["sections"][0]["actions"];
        assert_eq!(mixed_public_actions.as_array().unwrap().len(), 2);
        assert_eq!(
            mixed_public_actions[1]["deviceLocation"],
            "Shared device storage /roms/legacy/settings.cfg"
        );
        assert!(mixed_public_actions[1].get("copyChanges").is_none());
        let mixed_public_text = serde_json::to_string(mixed_public_review).unwrap();
        let private_values = [
            "/storage/AB12-CD34".to_string(),
            "/storage/EF56-789A".to_string(),
            "AB12-CD34".to_string(),
            "EF56-789A".to_string(),
            "1".repeat(32),
            "2".repeat(32),
            "3".repeat(32),
            "4".repeat(32),
        ];
        for private_value in private_values {
            assert!(!mixed_public_text.contains(&private_value));
        }

        let app_target_path = "/data/user/0/com.example.app/files/settings.cfg";
        let app_target_review = json!({
            "features": [{
                "sections": [{
                    "kind": "copies",
                    "actions": [{
                        "deviceLocation": app_target_path,
                        "copyChanges": {
                            "selectedBoundary": app_target_path,
                            "additions": [],
                            "replacements": [],
                            "deletions": []
                        }
                    }]
                }]
            }]
        });
        let app_target_projection =
            sanitize_copy_destination_review(&plan, app_target_review).unwrap();
        assert_eq!(
            app_target_projection["features"][0]["sections"][0]["actions"][0]
                .pointer("/copyChanges/selectedBoundary")
                .and_then(Value::as_str),
            Some(app_target_path)
        );
    }
}

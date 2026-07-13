//! Trusted adaptation of retained reviews to simulated Phase 0 executions.
//!
//! React supplies only opaque, session-scoped handles. This module owns target
//! and digest revalidation, forces dry-run mode, and projects sidecar reports
//! into serial-free, path-safe DTOs.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::commands::{
    catalog, current_adb_path, redact_absolute_paths, redact_exact_serial, safe_error, AppState,
};
use crate::handles::ReviewedPlanSnapshot;
use crate::sidecar::SidecarState;

/// One opaque app handle mapped to the sidecar execution that implements it.
#[derive(Clone, Debug)]
struct ExecutionMapping {
    public_handle: String,
    sidecar_id: String,
    review_handle: String,
    review: ReviewedPlanSnapshot,
}

/// Bounded, restart-volatile execution handle state.
///
/// A start reservation prevents concurrent preflight races. At most one active
/// mapping and the latest terminal mapping are retained; terminal replacement
/// drops the older handle permanently.
#[derive(Default)]
pub struct ExecutionHandleStore {
    start_reserved: bool,
    active: Option<ExecutionMapping>,
    latest_terminal: Option<ExecutionMapping>,
}

impl ExecutionHandleStore {
    fn reserve_start(&mut self) -> Result<(), String> {
        if self.start_reserved || self.active.is_some() {
            return Err(safe_error(
                "execution_in_progress",
                "A simulated run is already starting or active.",
            ));
        }
        self.start_reserved = true;
        Ok(())
    }

    fn release_start(&mut self) {
        self.start_reserved = false;
    }

    fn bind_started(
        &mut self,
        sidecar_id: String,
        review_handle: String,
        review: ReviewedPlanSnapshot,
    ) -> ExecutionMapping {
        debug_assert!(self.start_reserved);
        let mapping = ExecutionMapping {
            public_handle: format!("execution_{}", Uuid::new_v4().simple()),
            sidecar_id,
            review_handle,
            review,
        };
        self.start_reserved = false;
        self.active = Some(mapping.clone());
        mapping
    }

    fn mapping(&self, public_handle: &str) -> Result<ExecutionMapping, String> {
        self.active
            .as_ref()
            .filter(|mapping| mapping.public_handle == public_handle)
            .or_else(|| {
                self.latest_terminal
                    .as_ref()
                    .filter(|mapping| mapping.public_handle == public_handle)
            })
            .cloned()
            .ok_or_else(|| {
                safe_error(
                    "execution_unavailable",
                    "This simulated run is unavailable. Return to Review or generate a new review.",
                )
            })
    }

    fn mark_terminal(&mut self, public_handle: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|mapping| mapping.public_handle == public_handle)
        {
            self.latest_terminal = self.active.take();
        }
    }

    fn forget_active(&mut self, public_handle: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|mapping| mapping.public_handle == public_handle)
        {
            self.active = None;
        }
    }
}

trait RuntimeRequester {
    fn request(&self, request_type: &str, payload: Value) -> Result<Value, String>;
}

impl RuntimeRequester for SidecarState {
    fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
        SidecarState::request(self, request_type, payload)
    }
}

fn runtime_request(
    runtime: &impl RuntimeRequester,
    request_type: &str,
    payload: Value,
) -> Result<Value, String> {
    runtime.request(request_type, payload)
}

#[tauri::command]
pub fn start_simulated_execution(
    review_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut executions = state.executions.lock().map_err(|_| {
        safe_error(
            "execution_state_unavailable",
            "Simulated execution state is unavailable.",
        )
    })?;
    executions.reserve_start()?;

    let outcome = start_simulated_execution_inner(&review_handle, &state, &mut executions);
    if outcome.is_err() {
        executions.release_start();
    }
    outcome
}

fn start_simulated_execution_inner(
    review_handle: &str,
    state: &AppState,
    executions: &mut ExecutionHandleStore,
) -> Result<Value, String> {
    let review = state
        .handles
        .lock()
        .map_err(|_| session_error())?
        .review(review_handle)?
        .clone();

    validate_catalog(&review, state)?;
    let adb_path = current_adb_path(state)?;
    let inventory = runtime_request(
        &state.sidecar,
        "listAdbDevices",
        json!({ "adbPath": &adb_path }),
    )
    .map_err(|_| stale_review("The reviewed device could not be found."))?;

    let (serial, refreshed_review) = {
        let mut handles = state.handles.lock().map_err(|_| session_error())?;
        handles
            .update_devices(&inventory)
            .map_err(|_| stale_review("The reviewed device inventory changed."))?;
        let refreshed = handles.review(review_handle)?.clone();
        let device = handles
            .device(&refreshed.device_handle)
            .map_err(|_| stale_review("The reviewed device disconnected."))?;
        if device.state != "available" {
            return Err(stale_review(
                "The reviewed device is not currently available.",
            ));
        }
        (device.serial.clone(), refreshed)
    };

    let facts = runtime_request(
        &state.sidecar,
        "probeDevice",
        json!({ "adbPath": adb_path, "serial": &serial }),
    )
    .map_err(|_| stale_review("The reviewed device facts could not be refreshed."))?;
    validate_target(&refreshed_review.target, &serial, &facts)?;
    validate_plan_digest(&refreshed_review)?;

    let start_result = request_dry_run_start(&state.sidecar, &refreshed_review)?;
    bind_start_result(executions, review_handle, refreshed_review, &start_result)
}

fn request_dry_run_start(
    runtime: &impl RuntimeRequester,
    review: &ReviewedPlanSnapshot,
) -> Result<Value, String> {
    runtime_request(
        runtime,
        "startExecution",
        json!({
            "plan": review.response.get("plan"),
            "planDigest": review.plan_digest,
            "mode": "dry_run",
            "targetDevice": review.target,
        }),
    )
    .map_err(|error| execution_start_error(&error))
}

fn bind_start_result(
    executions: &mut ExecutionHandleStore,
    review_handle: &str,
    review: ReviewedPlanSnapshot,
    start_result: &Value,
) -> Result<Value, String> {
    let report = start_result.get("execution").ok_or_else(|| {
        safe_error(
            "simulation_start_failed",
            "The simulated run returned an invalid initial report.",
        )
    })?;
    let sidecar_id = report
        .get("executionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            safe_error(
                "simulation_start_failed",
                "The simulated run did not provide an execution identifier.",
            )
        })?
        .to_string();
    let mapping = executions.bind_started(sidecar_id, review_handle.to_string(), review);
    Ok(project_snapshot(&mapping, report))
}

#[tauri::command]
pub fn get_simulated_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?
        .mapping(&execution_handle)?;
    let response = match runtime_request(
        &state.sidecar,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    ) {
        Ok(response) => response,
        Err(error) => {
            if sidecar_error_code(&error).as_deref() == Some("unknown_execution") {
                state
                    .executions
                    .lock()
                    .map_err(|_| execution_state_error())?
                    .forget_active(&execution_handle);
                return Err(safe_error(
                    "execution_unavailable",
                    "The in-memory simulated run was lost. Return to Review or generate a new review.",
                ));
            }
            return Err(safe_error(
                "execution_status_failed",
                "The simulated run status could not be refreshed.",
            ));
        }
    };
    let report = response.get("execution").ok_or_else(|| {
        safe_error(
            "execution_status_failed",
            "The simulated run returned an invalid status report.",
        )
    })?;
    let public = project_snapshot(&mapping, report);
    if is_terminal_status(report.get("status").and_then(Value::as_str)) {
        state
            .executions
            .lock()
            .map_err(|_| execution_state_error())?
            .mark_terminal(&execution_handle);
    }
    Ok(public)
}

#[tauri::command]
pub fn get_simulated_execution_events(
    execution_handle: String,
    after_sequence: u64,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut executions = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?;
    request_simulated_execution_events(
        &state.sidecar,
        &mut executions,
        &execution_handle,
        after_sequence,
    )
}

fn request_simulated_execution_events(
    runtime: &impl RuntimeRequester,
    executions: &mut ExecutionHandleStore,
    execution_handle: &str,
    after_sequence: u64,
) -> Result<Value, String> {
    let mapping = executions.mapping(execution_handle)?;
    let response = runtime_request(
        runtime,
        "getExecutionEvents",
        json!({
            "executionId": mapping.sidecar_id,
            "afterSequence": after_sequence,
        }),
    );
    let response = match response {
        Ok(response) => response,
        Err(error) if sidecar_error_code(&error).as_deref() == Some("unknown_execution") => {
            executions.forget_active(execution_handle);
            return Err(safe_error(
                "execution_unavailable",
                "The in-memory simulated run was lost. Return to Review or generate a new review.",
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_status_failed",
                "Incremental simulated progress could not be refreshed.",
            ));
        }
    };
    Ok(project_event_batch(&mapping, &response))
}

#[tauri::command]
pub fn cancel_simulated_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?
        .mapping(&execution_handle)?;
    let response = runtime_request(
        &state.sidecar,
        "cancelExecution",
        json!({ "executionId": mapping.sidecar_id }),
    )
    .map_err(|_| {
        safe_error(
            "execution_cancel_failed",
            "Cancellation could not be requested for this simulated run.",
        )
    })?;
    Ok(json!({
        "executionHandle": execution_handle,
        "accepted": response.get("accepted").and_then(Value::as_bool).unwrap_or(false),
        "status": response.get("status").and_then(Value::as_str).unwrap_or("running"),
    }))
}

fn validate_catalog(review: &ReviewedPlanSnapshot, state: &AppState) -> Result<(), String> {
    let current = catalog(state)?;
    let identity = serde_json::to_value(current.public_identity()).map_err(|_| {
        safe_error(
            "catalog_resource_invalid",
            "The packaged setup catalog identity could not be verified.",
        )
    })?;
    let fields_match = ["sourceKind", "sourceId", "version", "contentDigest"]
        .iter()
        .all(|field| review.catalog_identity.get(field) == identity.get(field));
    if review.catalog_digest != current.digest() || !fields_match {
        return Err(stale_review("The setup catalog changed after review."));
    }
    Ok(())
}

fn validate_target(target: &Value, serial: &str, facts: &Value) -> Result<(), String> {
    let reviewed_serial = target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reviewed_serial.trim() != serial.trim() {
        return Err(stale_review("The target device changed after review."));
    }
    for (reviewed_field, actual_field) in [("manufacturer", "manufacturer"), ("model", "model")] {
        if let Some(expected) = target.get(reviewed_field).and_then(Value::as_str) {
            let actual = facts.get(actual_field).and_then(Value::as_str);
            if actual
                .is_none_or(|value| normalize_target_text(expected) != normalize_target_text(value))
            {
                return Err(stale_review(
                    "The target device facts changed after review.",
                ));
            }
        }
    }
    if let Some(expected) = target.get("androidApiLevel").and_then(Value::as_i64) {
        if facts.get("android_api_level").and_then(Value::as_i64) != Some(expected) {
            return Err(stale_review(
                "The target Android API level changed after review.",
            ));
        }
    }
    Ok(())
}

fn normalize_target_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn validate_plan_digest(review: &ReviewedPlanSnapshot) -> Result<(), String> {
    let plan = review
        .response
        .get("plan")
        .ok_or_else(|| stale_review("The retained reviewed plan is no longer available."))?;
    let actual = canonical_json_digest(plan)
        .map_err(|_| stale_review("The retained reviewed plan could not be verified."))?;
    if actual != review.plan_digest.to_lowercase() {
        return Err(stale_review("The reviewed plan changed after review."));
    }
    Ok(())
}

fn canonical_json_digest(value: &Value) -> Result<String, serde_json::Error> {
    let canonical = canonicalize(value.clone());
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        scalar => scalar,
    }
}

fn project_snapshot(mapping: &ExecutionMapping, report: &Value) -> Value {
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let report_recipes = report
        .get("recipes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let retained_recipes = mapping
        .review
        .response
        .pointer("/plan/recipes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut by_id = report_recipes
        .iter()
        .filter_map(|recipe| Some((recipe.get("recipeId")?.as_str()?.to_string(), recipe)))
        .collect::<HashMap<_, _>>();
    let mut recipes = Vec::new();
    for retained in retained_recipes {
        let Some(recipe_id) = retained.get("id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(report_recipe) = by_id.remove(recipe_id) {
            recipes.push(project_recipe(report_recipe, Some(&retained), exact_serial));
        }
    }
    for report_recipe in &report_recipes {
        let recipe_id = report_recipe
            .get("recipeId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if by_id.remove(recipe_id).is_some() {
            recipes.push(project_recipe(report_recipe, None, exact_serial));
        }
    }
    let warnings = project_issues(report.get("warnings"), exact_serial);
    let errors = project_issues(report.get("errors"), exact_serial);
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let mut public = json!({
        "executionHandle": mapping.public_handle,
        "reviewHandle": mapping.review_handle,
        "simulated": true,
        "status": status,
        "startedAt": report.get("startedAt"),
        "finishedAt": report.get("finishedAt"),
        "latestSequence": report.get("latestSequence").and_then(Value::as_u64).unwrap_or(0),
        "terminal": is_terminal_status(Some(status)),
        "recipes": recipes,
        "warnings": warnings,
        "errors": errors,
    });
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn project_recipe(recipe: &Value, retained: Option<&Value>, exact_serial: &str) -> Value {
    let recipe_id = recipe
        .get("recipeId")
        .and_then(Value::as_str)
        .unwrap_or("unknown_recipe");
    let name = recipe
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            retained
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| humanize_identifier(recipe_id));
    let steps = recipe
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|step| project_step(step, exact_serial))
        .collect::<Vec<_>>();
    json!({
        "recipeId": recipe_id,
        "name": sanitize_text(&name, exact_serial),
        "description": retained
            .and_then(|value| value.get("description"))
            .and_then(Value::as_str)
            .map(|value| sanitize_text(value, exact_serial)),
        "status": recipe.get("status").and_then(Value::as_str).unwrap_or("pending"),
        "steps": steps,
    })
}

fn project_step(step: &Value, exact_serial: &str) -> Value {
    let step_id = step
        .get("stepId")
        .and_then(Value::as_str)
        .unwrap_or("unknown_step");
    let name = step
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| humanize_identifier(step_id));
    json!({
        "stepId": step_id,
        "name": sanitize_text(&name, exact_serial),
        "note": step.get("note").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
        "status": step.get("status").and_then(Value::as_str).unwrap_or("pending"),
        "message": step.get("message").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
    })
}

fn project_issues(value: Option<&Value>, exact_serial: &str) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|issue| {
            json!({
                "code": issue.get("code").and_then(Value::as_str).unwrap_or("execution_issue"),
                "message": issue.get("message").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)).unwrap_or_else(|| "Simulated work reported an issue.".to_string()),
                "recipeId": issue.get("recipeId"),
                "stepId": issue.get("stepId"),
            })
        })
        .collect()
}

fn project_event_batch(mapping: &ExecutionMapping, response: &Value) -> Value {
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut events = response
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let sequence = event.get("sequence").and_then(Value::as_u64)?;
            if !seen.insert(sequence) {
                return None;
            }
            Some(json!({
                "sequence": sequence,
                "timestamp": event.get("timestamp"),
                "eventType": event.get("eventType"),
                "recipeId": event.get("recipeId"),
                "stepId": event.get("stepId"),
                "phase": event.get("phase"),
                "status": event.get("status"),
                "note": event.get("note").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
                "message": event.get("message").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
                "issue": event.get("issue").map(|issue| project_issues(Some(&Value::Array(vec![issue.clone()])), exact_serial).into_iter().next().unwrap_or(Value::Null)),
            }))
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.get("sequence").and_then(Value::as_u64).unwrap_or(0));
    let mut public = json!({
        "executionHandle": mapping.public_handle,
        "events": events,
        "latestSequence": response.get("latestSequence").and_then(Value::as_u64).unwrap_or(0),
        "terminal": response.get("terminal").and_then(Value::as_bool).unwrap_or(false),
    });
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn sanitize_text(value: &str, exact_serial: &str) -> String {
    let without_serial = if exact_serial.is_empty() {
        value.to_string()
    } else {
        value.replace(exact_serial, "[device]")
    };
    redact_absolute_paths(&without_serial)
}

fn humanize_identifier(value: &str) -> String {
    value
        .split(['.', '_', '-', '/'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_terminal_status(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("succeeded" | "succeeded_with_warnings" | "failed" | "cancelled")
    )
}

fn sidecar_error_code(error: &str) -> Option<String> {
    serde_json::from_str::<Value>(error)
        .ok()?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

fn execution_start_error(error: &str) -> String {
    match sidecar_error_code(error).as_deref() {
        Some("execution_in_progress") => safe_error(
            "execution_in_progress",
            "Another simulated run is already active.",
        ),
        Some("plan_digest_mismatch" | "target_device_mismatch") => {
            stale_review("The reviewed plan or target changed before simulation.")
        }
        _ => safe_error(
            "simulation_start_failed",
            "The simulated run could not be started.",
        ),
    }
}

fn stale_review(message: &str) -> String {
    safe_error("review_stale", message)
}

fn session_error() -> String {
    safe_error(
        "session_state_unavailable",
        "Review session state is unavailable.",
    )
}

fn execution_state_error() -> String {
    safe_error(
        "execution_state_unavailable",
        "Simulated execution state is unavailable.",
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use super::*;

    fn review() -> ReviewedPlanSnapshot {
        let plan = json!({
            "kind": "execution_plan",
            "id": "plan.one",
            "recipes": [{ "id": "recipe.one", "name": "Recipe One", "description": "Safe description" }],
            "steps": [],
            "target_device": { "serial": "sensitive-serial", "manufacturer": "AYANEO", "model": "Pocket S", "android_api_level": 33 },
        });
        ReviewedPlanSnapshot {
            response: json!({ "plan": plan }),
            target: json!({ "serial": "sensitive-serial", "manufacturer": "AYANEO", "model": "Pocket S", "androidApiLevel": 33 }),
            catalog_identity: json!({
                "sourceKind": "bundled", "sourceId": "catalog", "version": "1",
                "contentDigest": { "algorithm": "sha256", "value": "catalog" }
            }),
            catalog_digest: "catalog".to_string(),
            plan_digest: canonical_json_digest(&plan).unwrap(),
            device_handle: "device_one".to_string(),
            created: Instant::now(),
            last_access: Instant::now(),
        }
    }

    #[test]
    fn store_is_bounded_and_handles_are_never_reused() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start().unwrap();
        let first = store.bind_started("sidecar-1".into(), "review-1".into(), review());
        assert!(store
            .reserve_start()
            .unwrap_err()
            .contains("execution_in_progress"));
        store.mark_terminal(&first.public_handle);
        store.reserve_start().unwrap();
        let second = store.bind_started("sidecar-2".into(), "review-2".into(), review());
        assert_ne!(first.public_handle, second.public_handle);
        store.mark_terminal(&second.public_handle);
        assert!(store
            .mapping(&first.public_handle)
            .unwrap_err()
            .contains("execution_unavailable"));
        assert_eq!(
            store.mapping(&second.public_handle).unwrap().sidecar_id,
            "sidecar-2"
        );
    }

    #[test]
    fn failed_start_reservation_can_always_be_released() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start().unwrap();
        store.release_start();
        store.reserve_start().unwrap();
    }

    #[test]
    fn target_comparison_matches_phase_zero_normalization_only() {
        let target = json!({
            "serial": " serial ", "manufacturer": "AyaNeo", "model": "Pocket   S", "androidApiLevel": 33
        });
        let facts = json!({
            "manufacturer": " ayaneo ", "model": "pocket s", "android_api_level": 33,
            "brand": "irrelevant changed brand"
        });
        validate_target(&target, "serial", &facts).unwrap();
        assert!(validate_target(&target, "different", &facts)
            .unwrap_err()
            .contains("review_stale"));
        assert!(validate_target(
            &target,
            "serial",
            &json!({ "manufacturer": "AYANEO", "model": "Other", "android_api_level": 33 })
        )
        .unwrap_err()
        .contains("review_stale"));
    }

    #[test]
    fn canonical_digest_detects_retained_plan_mutation() {
        let mut retained = review();
        validate_plan_digest(&retained).unwrap();
        retained.response["plan"]["id"] = json!("changed");
        assert!(validate_plan_digest(&retained)
            .unwrap_err()
            .contains("review_stale"));
    }

    #[test]
    fn projections_are_ordered_and_remove_sensitive_runtime_fields() {
        let retained = review();
        let mapping = ExecutionMapping {
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: retained,
        };
        let report = json!({
            "executionId": "execution-private",
            "reviewedPlan": { "secret": true },
            "targetDevice": { "serial": "sensitive-serial" },
            "status": "failed", "startedAt": "2026-01-01T00:00:00Z", "finishedAt": "2026-01-01T00:00:01Z", "latestSequence": 4,
            "recipes": [{
                "recipeId": "recipe.one", "name": "Recipe One", "status": "blocked",
                "steps": [{ "stepId": "step.one", "name": "", "note": "Read /Users/private/file", "status": "blocked", "message": "sensitive-serial failed", "outputs": { "path": "/secret" } }]
            }],
            "warnings": [], "errors": [{ "code": "blocked", "message": "At /private/path", "recipeId": "recipe.one", "stepId": "step.one" }]
        });
        let public = project_snapshot(&mapping, &report);
        let serialized = public.to_string();
        assert_eq!(public["recipes"][0]["name"], "Recipe One");
        assert_eq!(public["recipes"][0]["steps"][0]["name"], "Step One");
        assert!(!serialized.contains("execution-private"));
        assert!(!serialized.contains("reviewedPlan"));
        assert!(!serialized.contains("targetDevice"));
        assert!(!serialized.contains("outputs"));
        assert!(!serialized.contains("sensitive-serial"));
        assert!(!serialized.contains("/Users/private"));
        assert!(!serialized.contains("/private/path"));
    }

    struct FakeRuntime {
        requests: Mutex<Vec<(String, Value)>>,
        result: Result<Value, String>,
    }

    impl RuntimeRequester for FakeRuntime {
        fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
            self.requests
                .lock()
                .unwrap()
                .push((request_type.into(), payload));
            self.result.clone()
        }
    }

    #[test]
    fn deterministic_runtime_records_existing_phase_zero_request_shape() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({})),
        };
        runtime
            .request(
                "getExecutionEvents",
                json!({ "executionId": "private", "afterSequence": 7 }),
            )
            .unwrap();
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests[0].0, "getExecutionEvents");
        assert_eq!(requests[0].1["afterSequence"], 7);
    }

    #[test]
    fn deterministic_runtime_proves_start_is_forced_dry_run_with_retained_data() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({ "execution": { "executionId": "sidecar-private" } })),
        };
        let retained = review();
        request_dry_run_start(&runtime, &retained).unwrap();
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "startExecution");
        assert_eq!(requests[0].1["mode"], "dry_run");
        assert_eq!(requests[0].1["plan"], retained.response["plan"]);
        assert_eq!(requests[0].1["planDigest"], retained.plan_digest);
        assert_eq!(requests[0].1["targetDevice"], retained.target);
        assert!(requests[0].1.get("adbPath").is_none());
        assert!(requests[0].1.get("runtimeRoot").is_none());
        assert!(requests[0].1.get("cacheRoot").is_none());
    }

    #[test]
    fn unknown_event_session_releases_only_the_matching_active_mapping() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "unknown_execution", "message": "private" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start().unwrap();
        let active = store.bind_started("sidecar-active".into(), "review".into(), review());

        let error =
            request_simulated_execution_events(&runtime, &mut store, &active.public_handle, 3)
                .unwrap_err();
        assert!(error.contains("execution_unavailable"));
        assert!(!error.contains("sidecar-active"));
        assert!(!error.contains("private"));
        assert!(store
            .mapping(&active.public_handle)
            .unwrap_err()
            .contains("execution_unavailable"));
        store
            .reserve_start()
            .expect("the lost active slot should be reusable");
    }

    #[test]
    fn ordinary_event_failure_keeps_the_active_mapping_reserved() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "runtime_request_failed" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start().unwrap();
        let active = store.bind_started("sidecar-active".into(), "review".into(), review());

        let error =
            request_simulated_execution_events(&runtime, &mut store, &active.public_handle, 0)
                .unwrap_err();
        assert!(error.contains("execution_status_failed"));
        assert_eq!(
            store.mapping(&active.public_handle).unwrap().sidecar_id,
            "sidecar-active"
        );
        assert!(store
            .reserve_start()
            .unwrap_err()
            .contains("execution_in_progress"));
    }

    #[test]
    fn unknown_terminal_event_session_does_not_remove_another_active_mapping() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "unknown_execution" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start().unwrap();
        let terminal = store.bind_started("sidecar-terminal".into(), "review-old".into(), review());
        store.mark_terminal(&terminal.public_handle);
        store.reserve_start().unwrap();
        let active = store.bind_started("sidecar-active".into(), "review-new".into(), review());

        let error =
            request_simulated_execution_events(&runtime, &mut store, &terminal.public_handle, 0)
                .unwrap_err();
        assert!(error.contains("execution_unavailable"));
        assert_eq!(
            store.mapping(&terminal.public_handle).unwrap().sidecar_id,
            "sidecar-terminal"
        );
        assert_eq!(
            store.mapping(&active.public_handle).unwrap().sidecar_id,
            "sidecar-active"
        );
        assert!(store
            .reserve_start()
            .unwrap_err()
            .contains("execution_in_progress"));
    }
}

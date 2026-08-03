//! Shared root-work classification for reviewed execution plans.
//!
//! The backend evaluates typed plans while Tauri retains a serialized plan for
//! authority invalidation. This module keeps the root-work semantics in one
//! source without requiring the Tauri package to depend on the backend crate or
//! exposing authority metadata in a public review response.

use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RootRequirements {
    pub(crate) root_shell: bool,
    pub(crate) app_data_write: bool,
}

impl RootRequirements {
    pub(crate) fn any(self) -> bool {
        self.root_shell || self.app_data_write
    }

    #[allow(dead_code)] // The backend aggregates typed steps; Tauri classifies one JSON step at a time.
    pub(crate) fn merge(&mut self, other: Self) {
        self.root_shell |= other.root_shell;
        self.app_data_write |= other.app_data_write;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootWorkOperation {
    CopyFiles,
    ExtractArchive,
    GrantPermissions,
    Other,
}

pub(crate) fn root_work_operation(step_type: &str) -> RootWorkOperation {
    match step_type {
        "copy_files" => RootWorkOperation::CopyFiles,
        "extract_archive" => RootWorkOperation::ExtractArchive,
        "grant_permissions" => RootWorkOperation::GrantPermissions,
        _ => RootWorkOperation::Other,
    }
}

/// Normalize the two execution capabilities that can require root authority.
pub(crate) fn root_requirement_constraints<'a>(
    capabilities: impl IntoIterator<Item = &'a str>,
) -> (bool, bool) {
    capabilities.into_iter().fold(
        (false, false),
        |(root_shell, app_data_write), capability| {
            (
                root_shell || capability == "root_shell",
                app_data_write || capability == "app_data_write",
            )
        },
    )
}

/// Return whether a reviewed predicate can reach an app-private shell boundary.
pub(crate) fn condition_requires_root(condition_type: &str, path: Option<&str>) -> bool {
    matches!(condition_type, "path_exists" | "file_exists") && path.is_some_and(is_app_private_path)
}

/// Normalized root-relevant facts supplied by either a typed or serialized plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RootRequirementStep {
    pub(crate) operation: RootWorkOperation,
    pub(crate) root_shell_constraint: bool,
    pub(crate) app_data_write_constraint: bool,
    pub(crate) condition_requires_root: bool,
    pub(crate) source_requires_root: bool,
    pub(crate) destination_requires_root: bool,
    pub(crate) extracts_to_device: bool,
}

/// Classify all reviewed root-dependent work from normalized step facts.
pub(crate) fn classify_root_requirement_step(step: RootRequirementStep) -> RootRequirements {
    let mut requirements = RootRequirements::default();
    if step.operation != RootWorkOperation::GrantPermissions {
        if step.root_shell_constraint || step.app_data_write_constraint {
            requirements.root_shell = true;
        }
        if step.app_data_write_constraint {
            requirements.app_data_write = true;
        }
    }
    if step.condition_requires_root {
        requirements.root_shell = true;
    }
    match step.operation {
        RootWorkOperation::CopyFiles => {
            if step.source_requires_root {
                requirements.root_shell = true;
            }
            if step.destination_requires_root {
                requirements.root_shell = true;
                requirements.app_data_write = true;
            }
        }
        RootWorkOperation::ExtractArchive
            if step.extracts_to_device && step.destination_requires_root =>
        {
            requirements.root_shell = true;
            requirements.app_data_write = true;
        }
        RootWorkOperation::ExtractArchive
        | RootWorkOperation::GrantPermissions
        | RootWorkOperation::Other => {}
    }
    requirements
}

/// Classify a serialized backend-authored reviewed plan for Tauri retention.
///
/// This is compiled directly into Tauri from this source file so both process
/// owners use the same operation and app-private-path semantics.
#[allow(dead_code)] // The backend calls the typed entry point; Tauri calls this one.
pub(crate) fn reviewed_plan_requires_root_json(plan: &Value) -> bool {
    let inputs = plan
        .get("inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    plan.get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps
                .iter()
                .any(|step| json_step_root_requirements(step, &inputs).any())
        })
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_step_root_requirements(step: &Value, inputs: &[Value]) -> RootRequirements {
    let operation = step
        .get("type")
        .or_else(|| step.get("type_name"))
        .and_then(Value::as_str)
        .map(root_work_operation)
        .unwrap_or(RootWorkOperation::Other);
    let (root_shell_constraint, app_data_write_constraint) = json_constraint_requirements(step);
    let condition_requires_root = step
        .get("skip_if")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            step.get("verify")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .any(json_condition_requires_root);
    let param = |name| step.get("params").and_then(|params| params.get(name));
    let source_requires_root = param("source")
        .and_then(|value| json_param_runtime_value(value, inputs))
        .is_some_and(json_runtime_value_requires_root);
    let destination_requires_root =
        param("dest").is_some_and(|value| json_destination_requires_root(value, inputs));
    let extracts_to_device =
        param("extract_on").and_then(|value| json_param_string(value, inputs)) == Some("device");
    classify_root_requirement_step(RootRequirementStep {
        operation,
        root_shell_constraint,
        app_data_write_constraint,
        condition_requires_root,
        source_requires_root,
        destination_requires_root,
        extracts_to_device,
    })
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_constraint_requirements(step: &Value) -> (bool, bool) {
    root_requirement_constraints(
        step.get("constraints")
            .and_then(|constraints| constraints.get("capabilities"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    )
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_condition_requires_root(condition: &Value) -> bool {
    condition_requires_root(
        condition
            .get("type_name")
            .or_else(|| condition.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        condition
            .get("params")
            .and_then(|params| params.get("path"))
            .and_then(Value::as_str),
    )
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_param_runtime_value<'a>(param: &'a Value, inputs: &'a [Value]) -> Option<&'a Value> {
    if let Some(ref_value) = param
        .get("ref")
        .or_else(|| param.get("ref_value"))
        .and_then(Value::as_str)
    {
        return json_input_runtime_value(inputs, ref_value);
    }
    Some(json_param_literal(param))
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_destination_requires_root(param: &Value, inputs: &[Value]) -> bool {
    json_param_string(param, inputs).is_some_and(is_app_private_path)
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_param_string<'a>(param: &'a Value, inputs: &'a [Value]) -> Option<&'a str> {
    if let Some(ref_value) = param
        .get("ref")
        .or_else(|| param.get("ref_value"))
        .and_then(Value::as_str)
    {
        return json_input_runtime_value(inputs, ref_value)
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str);
    }
    json_param_literal(param).as_str()
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_param_literal(param: &Value) -> &Value {
    param.get("value").unwrap_or(param)
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_input_runtime_value<'a>(inputs: &'a [Value], ref_value: &str) -> Option<&'a Value> {
    let input_id = ref_value.strip_prefix("inputs.")?;
    if input_id.is_empty() {
        return None;
    }
    inputs
        .iter()
        .find(|input| input.get("id").and_then(Value::as_str) == Some(input_id))
        .and_then(|input| input.get("value"))
}

#[allow(dead_code)] // Reached by the retained-JSON entry point in Tauri.
fn json_runtime_value_requires_root(value: &Value) -> bool {
    value.get("location").and_then(Value::as_str) == Some("device")
        && match value.get("value") {
            Some(Value::String(path)) => is_app_private_path(path),
            Some(Value::Array(paths)) => paths
                .iter()
                .filter_map(Value::as_str)
                .any(is_app_private_path),
            _ => false,
        }
}

/// Return whether a device path crosses the application-private storage boundary.
///
/// The execution adapter uses this same predicate when choosing whether to wrap
/// a shell command in `su`, so review-time classification cannot drift from the
/// privileged command boundary.
pub(crate) fn is_app_private_path(path: &str) -> bool {
    path.starts_with("/data/user/") || path.starts_with("/data/data/")
}

#[cfg(test)]
mod tests {
    use super::condition_requires_root;

    #[test]
    fn shared_condition_classifier_requires_root_only_for_private_device_predicates() {
        assert!(condition_requires_root(
            "path_exists",
            Some("/data/data/com.example.app/files")
        ));
        assert!(condition_requires_root(
            "file_exists",
            Some("/data/user/0/com.example.app/files")
        ));
        assert!(!condition_requires_root(
            "path_exists",
            Some("/sdcard/EmuChef/files")
        ));
        assert!(!condition_requires_root(
            "package_installed",
            Some("/data/data/com.example.app/files")
        ));
    }
}

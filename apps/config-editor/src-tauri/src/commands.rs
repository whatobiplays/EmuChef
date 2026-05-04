use serde_json::{json, Value};

use crate::python_bridge::{build_request, run_request};

#[tauri::command]
pub fn list_step_specs() -> Result<Value, String> {
    run_request(build_request("listStepSpecs", None))
}

#[tauri::command]
pub fn open_recipe(path: String, authored_root: Option<String>) -> Result<Value, String> {
    run_request(build_request(
        "openRecipe",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

#[tauri::command]
pub fn validate_recipe_path(path: String, authored_root: Option<String>) -> Result<Value, String> {
    run_request(build_request(
        "validateRecipePath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

#[tauri::command]
pub fn emit_recipe_yaml_from_path(
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    run_request(build_request(
        "emitRecipeYamlFromPath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

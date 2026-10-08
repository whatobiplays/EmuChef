//! JSON DTO projection for recipe documents.
//!
//! These projections preserve the editor protocol shape for the authored recipe
//! fields modeled by the authored recipe schema. Fields outside that model are intentionally left
//! out of scope instead of adding new semantics.

use serde_json::{json, Map, Value};

use crate::document::RecipeDocument;
use crate::model::{InputDeclaration, ParamValue, Recipe, RecipeArtifact, Step, StepCondition};
use crate::ref_index;

pub fn document_to_dto(document: &RecipeDocument, document_id: &str) -> Value {
    json!({
        "documentId": document_id,
        "path": document.path().to_string_lossy(),
        "authoredRoot": document
            .authored_root()
            .map(|path| path.to_string_lossy().to_string()),
        "dirty": document.is_dirty(),
        "canUndo": document.can_undo(),
        "canRedo": document.can_redo(),
        "recipe": recipe_to_dto(document.recipe()),
        "yaml": document.yaml(),
        "diagnostics": document.diagnostics(),
        "refIndex": ref_index::ref_index_to_dto(document.recipe()),
    })
}

pub(crate) fn recipe_to_dto(recipe: &Recipe) -> Value {
    json!({
        "schemaVersion": recipe.schema_version,
        "kind": recipe.kind,
        "id": recipe.id,
        "name": recipe.name,
        "description": recipe.description.clone().unwrap_or_default(),
        "recipeDependencies": recipe.recipe_dependencies,
        "provides": {"features": recipe.provides.features},
        "inputs": map_values(recipe.inputs.iter().map(|(id, input)| (id, input_to_dto(&recipe.id, id, input)))),
        "artifacts": map_values(recipe.artifacts.iter().map(|(id, artifact)| (id, artifact_to_dto(id, artifact)))),
        "artifactGroups": map_values(recipe.artifact_groups.iter().map(|(id, members)| (id, json!(members)))),
        "steps": recipe.steps.iter().map(step_to_dto).collect::<Vec<_>>(),
    })
}

pub(crate) fn input_to_dto(recipe_id: &str, input_id: &str, input: &InputDeclaration) -> Value {
    json!({
        "id": input_id,
        "recipeId": recipe_id,
        "inputId": input_id,
        "key": format!("{recipe_id}/{input_id}"),
        "type": input.type_name,
        "role": input.role,
        "label": input.label,
        "description": input.description.clone().unwrap_or_default(),
        "required": input.required,
        "multiple": input.multiple,
        "validation": {
            "mustExist": input.validation.must_exist,
            "allowedExtensions": input.validation.allowed_extensions,
            "pathKind": input.validation.path_kind,
            "allowedPrefixes": input.validation.allowed_prefixes,
        },
        "default": input.default,
        "options": input.options.iter().map(|option| json!({
            "value": option.value,
            "label": option.label,
        })).collect::<Vec<_>>(),
        "sensitive": input.sensitive,
        "advanced": input.advanced,
        "metadata": map_values(input.metadata.iter().map(|(key, value)| (key, value.clone()))),
    })
}

fn artifact_to_dto(artifact_id: &str, artifact: &RecipeArtifact) -> Value {
    match artifact {
        RecipeArtifact::RemoteFile(artifact) => json!({
            "id": artifact_id,
            "type": "remote_file",
            "url": artifact.url,
            "cache": artifact.cache,
        }),
        RecipeArtifact::AppArtifact(artifact) => json!({
            "id": artifact_id,
            "type": "app_artifact",
            "appRef": artifact.app_ref,
            "artifact": artifact.artifact,
            "cache": artifact.cache,
        }),
    }
}

fn step_to_dto(step: &Step) -> Value {
    let mut dto = json!({
        "id": step.id,
        "type": step.type_name,
        "name": step.name,
        "description": step.description.clone().unwrap_or_default(),
        "userToggleable": step.user_toggleable,
        "dependencies": step.dependencies,
        "constraints": {
            "capabilities": step.constraints.capabilities,
            "conflictsWith": step.constraints.conflicts_with,
        },
        "skipIf": step.skip_if.iter().map(condition_to_dto).collect::<Vec<_>>(),
        "params": map_values(step.params.iter().map(|(key, value)| (key, param_to_dto(value)))),
        "verify": step.verify.iter().map(condition_to_dto).collect::<Vec<_>>(),
    });
    if let (Some(app_ref), Some(object)) = (&step.app_ref, dto.as_object_mut()) {
        object.insert("appRef".to_string(), json!(app_ref));
    }
    if let (Some(progress_note), Some(object)) = (&step.progress_note, dto.as_object_mut()) {
        object.insert("progressNote".to_string(), json!(progress_note));
    }
    dto
}

fn condition_to_dto(condition: &StepCondition) -> Value {
    json!({
        "type": condition.type_name,
        "params": map_values(condition.params.iter().map(|(key, value)| (key, value.clone()))),
    })
}

fn param_to_dto(value: &ParamValue) -> Value {
    match value {
        ParamValue::Ref(ref_value) => json!({ "ref": ref_value }),
        ParamValue::Literal(value) => value.clone(),
    }
}

fn map_values<'a>(values: impl IntoIterator<Item = (&'a String, Value)>) -> Value {
    let mut object = Map::new();
    for (key, value) in values {
        object.insert(key.clone(), value);
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::step_to_dto;
    use crate::model::{OrderedMap, Step, StepConstraints};

    fn step(app_ref: Option<&str>) -> Step {
        Step {
            id: "install".to_string(),
            type_name: "install_apk".to_string(),
            name: "Install app".to_string(),
            description: None,
            progress_note: None,
            user_toggleable: false,
            app_ref: app_ref.map(str::to_string),
            dependencies: Vec::new(),
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: OrderedMap::new(),
            verify: Vec::new(),
        }
    }

    #[test]
    fn step_dto_preserves_legacy_shape_and_includes_app_context_when_present() {
        let legacy = step_to_dto(&step(None));
        assert!(legacy.get("appRef").is_none());

        let migrated = step_to_dto(&step(Some("armsx1")));
        assert_eq!(migrated["appRef"], "armsx1");
    }
}

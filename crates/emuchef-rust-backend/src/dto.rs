//! JSON DTO projections for Recipe and App Definition editor documents.
//!
//! Recipe projections preserve the editor protocol shape for authored Recipe
//! fields. App Definition diagnostics use camel-case object locations expected
//! by the Config Editor without changing Recipe diagnostic compatibility.

use serde_json::{json, Map, Value};

use crate::app_definition_document::AppDefinitionDocument;
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

/// Project App Definition diagnostic locations into the editor's camel-case DTO.
/// Recipe diagnostics retain their existing projection for protocol compatibility.
pub(crate) fn app_diagnostics_to_dto(diagnostics: &[Value]) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let Some(object) = diagnostic.as_object() else {
                return diagnostic.clone();
            };
            let mut projected = object.clone();
            for (source, destination) in [("object_kind", "objectKind"), ("object_id", "objectId")]
            {
                if let Some(value) = projected.remove(source) {
                    projected.entry(destination.to_string()).or_insert(value);
                }
            }
            Value::Object(projected)
        })
        .collect()
}

/// Convert an App Definition editor session into its stable sidecar DTO.
pub fn app_definition_document_to_dto(
    document: &AppDefinitionDocument,
    document_id: &str,
) -> Value {
    let identity_locked = document.catalog_identity().is_some();
    json!({
        "documentKind": "app_definition",
        "owningAuthoredRoot": document
            .owning_authored_root()
            .map(|root| root.to_string_lossy().to_string()),
        "documentId": document_id,
        "path": document.path().to_string_lossy(),
        "authoredRoot": document
            .authored_root()
            .map(|path| path.to_string_lossy().to_string()),
        "dirty": document.is_dirty(),
        "canUndo": document.can_undo(),
        "canRedo": document.can_redo(),
        "appDefinition": document.app_definition(),
        "yaml": document.yaml(),
        "diagnostics": app_diagnostics_to_dto(document.diagnostics()),
        "editRestrictions": {
            "identityLocked": identity_locked,
            "canEditIdentity": !identity_locked,
            "canSaveAs": document.can_save_as(),
        },
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
        ParamValue::AppTarget(target_id) => json!({ "app_target": target_id }),
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
    use super::{app_diagnostics_to_dto, step_to_dto};
    use crate::model::{OrderedMap, Step, StepConstraints};
    use serde_json::json;

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

    #[test]
    fn app_diagnostic_projection_camelizes_only_app_location_keys() {
        let raw = json!({
            "severity": "error",
            "code": "app_target_unknown",
            "object_kind": "step",
            "object_id": "install",
        });
        let projected = app_diagnostics_to_dto(std::slice::from_ref(&raw));

        assert_eq!(projected[0]["objectKind"], "step");
        assert_eq!(projected[0]["objectId"], "install");
        assert!(projected[0].get("object_kind").is_none());
        assert!(projected[0].get("object_id").is_none());
        assert_eq!(raw["object_kind"], "step");
        assert_eq!(raw["object_id"], "install");
    }
}

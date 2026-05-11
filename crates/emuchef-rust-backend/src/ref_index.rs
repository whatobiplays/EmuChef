//! RefIndex generation for the Phase 6H modeled authored recipe surface.
//!
//! The Python backend remains the reference implementation. This module mirrors
//! its current `build_ref_index` behavior for recipe inputs, runtime artifact
//! fields, authored step ids, and declared StepSpec outputs. It intentionally
//! does not inspect authored param refs, artifact groups, planner state,
//! catalog context, or executor/device data.

use serde_json::{json, Value};

use crate::model::Recipe;
use crate::step_specs;

const RUNTIME_ARTIFACT_FIELDS: &[&str] = &[
    "cache_hit",
    "error",
    "filename",
    "local_path",
    "resolved_url",
    "status",
];

pub fn ref_index_to_dto(recipe: &Recipe) -> Value {
    let input_refs = input_refs(recipe);
    let artifact_refs = artifact_refs(recipe);
    let step_refs = step_refs(recipe);
    let step_output_refs = step_output_refs(recipe);
    let all_refs = input_refs
        .iter()
        .chain(artifact_refs.iter())
        .chain(step_refs.iter())
        .chain(step_output_refs.iter())
        .cloned()
        .collect::<Vec<_>>();
    let candidates = input_candidates(recipe)
        .into_iter()
        .chain(artifact_candidates(recipe))
        .chain(step_output_candidates(recipe))
        .collect::<Vec<_>>();

    json!({
        "inputRefs": input_refs,
        "artifactRefs": artifact_refs,
        "stepRefs": step_refs,
        "stepOutputRefs": step_output_refs,
        "allRefs": all_refs,
        "candidates": candidates,
    })
}

fn input_refs(recipe: &Recipe) -> Vec<String> {
    let mut refs = recipe
        .inputs
        .keys()
        .map(|input_id| format!("inputs.{input_id}"))
        .collect::<Vec<_>>();
    refs.sort();
    refs
}

fn artifact_refs(recipe: &Recipe) -> Vec<String> {
    let mut artifact_ids = recipe.artifacts.keys().collect::<Vec<_>>();
    artifact_ids.sort();
    artifact_ids
        .into_iter()
        .flat_map(|artifact_id| {
            RUNTIME_ARTIFACT_FIELDS
                .iter()
                .map(move |field| format!("artifacts.{artifact_id}.{field}"))
        })
        .collect()
}

fn step_refs(recipe: &Recipe) -> Vec<String> {
    recipe
        .steps
        .iter()
        .map(|step| format!("steps.{}", step.id))
        .collect()
}

fn step_output_refs(recipe: &Recipe) -> Vec<String> {
    recipe
        .steps
        .iter()
        .flat_map(|step| {
            step_specs::step_spec_for(&step.type_name)
                .into_iter()
                .flat_map(move |spec| {
                    spec.outputs
                        .into_iter()
                        .map(move |output| format!("steps.{}.outputs.{}", step.id, output.name))
                })
        })
        .collect()
}

fn input_candidates(recipe: &Recipe) -> Vec<Value> {
    recipe
        .inputs
        .iter()
        .map(|(input_id, input)| {
            json!({
                "ref": format!("inputs.{input_id}"),
                "label": format!("Input \u{00b7} {input_id}"),
                "valueType": input_value_type(&input.type_name, input.multiple),
                "sourceKind": "input",
                "sourceId": input_id,
            })
        })
        .collect()
}

fn artifact_candidates(recipe: &Recipe) -> Vec<Value> {
    recipe
        .artifacts
        .keys()
        .flat_map(|artifact_id| {
            RUNTIME_ARTIFACT_FIELDS.iter().map(move |field| {
                json!({
                    "ref": format!("artifacts.{artifact_id}.{field}"),
                    "label": format!("Artifact \u{00b7} {artifact_id}.{field}"),
                    "valueType": artifact_field_value_type(field),
                    "sourceKind": "artifact",
                    "sourceId": artifact_id,
                })
            })
        })
        .collect()
}

fn step_output_candidates(recipe: &Recipe) -> Vec<Value> {
    recipe
        .steps
        .iter()
        .flat_map(|step| {
            step_specs::step_spec_for(&step.type_name)
                .into_iter()
                .flat_map(move |spec| {
                    spec.outputs.into_iter().map(move |output| {
                        json!({
                            "ref": format!("steps.{}.outputs.{}", step.id, output.name),
                            "label": format!("Step Output \u{00b7} {}.{}", step.id, output.name),
                            "valueType": output.value_type,
                            "sourceKind": "step_output",
                            "sourceId": step.id,
                        })
                    })
                })
        })
        .collect()
}

fn input_value_type(type_name: &str, multiple: bool) -> &'static str {
    if multiple {
        "path_list"
    } else if type_name == "directory" {
        "directory_path"
    } else {
        "file_path"
    }
}

fn artifact_field_value_type(field: &str) -> &'static str {
    match field {
        "cache_hit" => "boolean",
        "local_path" => "file_path",
        "error" | "filename" | "resolved_url" | "status" => "string",
        _ => unreachable!("runtime artifact fields are fixed by RUNTIME_ARTIFACT_FIELDS"),
    }
}

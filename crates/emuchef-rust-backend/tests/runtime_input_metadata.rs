use std::fs;

use emuchef_rust_backend::{document::RecipeDocument, dto, yaml};
use serde_json::json;
use tempfile::TempDir;

fn write_recipe(inputs: &str) -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().expect("temp directory should be created");
    let path = temp.path().join("recipe.yaml");
    let text = format!(
        "schema_version: 1\nkind: recipe\nid: test.runtime_inputs\nname: Runtime inputs\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n{inputs}artifacts: {{}}\nartifact_groups: {{}}\nsteps: []\n"
    );
    fs::write(&path, text).expect("recipe should be written");
    (temp, path)
}

#[test]
fn every_runtime_input_type_round_trips_with_deterministic_metadata() {
    let (_temp, path) = write_recipe(
        "  text:\n    type: string\n    default: hello\n  count:\n    type: integer\n    default: 3\n  enabled:\n    type: boolean\n    default: true\n  policy:\n    type: enum\n    default: merge\n    options:\n      - value: merge\n      - value: sync\n        label: Mirror files\n  file_value:\n    type: file\n    default: /tmp/file.bin\n  directory_value:\n    type: directory\n    default: /tmp/files\n  path_value:\n    type: path\n    default: relative/path\n  device_value:\n    type: device_path\n    default: /sdcard/ROMs\n    validation:\n      path_kind: directory\n      allowed_prefixes:\n        - /sdcard\n        - /storage/emulated/0\n  strings:\n    type: string_list\n    default: [zip, 7z]\n  paths:\n    type: path_list\n    default: [/tmp/a, /tmp/b]\n  settings:\n    type: object\n    default:\n      retries: 2\n  aliases:\n    type: string\n    multiple: true\n    default: [one, two]\n    sensitive: true\n    advanced: true\n",
    );

    let first = yaml::load_recipe_from_path(&path).expect("all supported input types should load");
    assert_eq!(first.inputs.len(), 12);
    assert_eq!(first.inputs["policy"].options[0].label, "merge");
    assert_eq!(
        first.inputs["device_value"]
            .validation
            .allowed_prefixes
            .len(),
        2
    );
    assert!(first.inputs["aliases"].sensitive);
    assert!(first.inputs["aliases"].advanced);

    let emitted = yaml::emit_recipe_yaml(&first).expect("input metadata should emit");
    let first_position = emitted.find("  text:").expect("first input should emit");
    let last_position = emitted.find("  aliases:").expect("last input should emit");
    assert!(
        first_position < last_position,
        "input ordering should be preserved"
    );
    assert!(emitted.contains("allowed_prefixes:\n      - /sdcard"));
    assert!(emitted.contains("sensitive: true\n    advanced: true"));

    fs::write(&path, &emitted).expect("canonical YAML should be writable");
    let second = yaml::load_recipe_from_path(&path).expect("canonical YAML should reload");
    assert_eq!(first, second);
}

#[test]
fn invalid_runtime_input_declarations_are_rejected_deterministically() {
    let cases = [
        (
            "  value:\n    type: unknown\n",
            "unsupported type 'unknown'",
        ),
        (
            "  value:\n    type: boolean\n    default: yes\n",
            "default is incompatible with type 'boolean'",
        ),
        (
            "  value:\n    type: enum\n    default: other\n    options:\n      - value: merge\n",
            "default \"other\" is not an enum option",
        ),
        (
            "  value:\n    type: enum\n    options:\n      - value: merge\n      - value: merge\n",
            "duplicate option value \"merge\"",
        ),
        ("  value:\n    type: enum\n", "requires at least one option"),
        (
            "  value:\n    type: device_path\n    validation:\n      allowed_prefixes: [sdcard]\n",
            "allowed_prefixes entries must be absolute paths",
        ),
        (
            "  value:\n    type: path\n    validation:\n      path_kind: socket\n",
            "path_kind must be 'file' or 'directory'",
        ),
        (
            "  value:\n    type: string\n    sensitive: secret\n",
            "'sensitive' must be a boolean",
        ),
    ];

    for (inputs, expected) in cases {
        let (_temp, path) = write_recipe(inputs);
        let error = yaml::load_recipe_from_path(&path)
            .expect_err("invalid input declaration should be rejected");
        let message = error
            .issue
            .as_ref()
            .map(|issue| issue.message.as_str())
            .unwrap_or(error.message.as_str());
        assert!(
            message.contains(expected),
            "expected {expected:?} in {:?}",
            message
        );
    }
}

#[test]
fn recipe_document_dto_exposes_runtime_configuration_schema() {
    let (_temp, path) = write_recipe(
        "  destination:\n    type: device_path\n    role: rom_destination\n    label: Device ROM folder\n    required: true\n    validation:\n      path_kind: directory\n      allowed_prefixes: [/sdcard]\n    default: /sdcard/ROMs\n    advanced: true\n",
    );
    let document = RecipeDocument::open(&path, None).expect("document should open");
    let value = dto::document_to_dto(&document, "doc-1");

    assert_eq!(
        value["recipe"]["inputs"]["destination"],
        json!({
            "id": "destination",
            "recipeId": "test.runtime_inputs",
            "inputId": "destination",
            "key": "test.runtime_inputs/destination",
            "type": "device_path",
            "role": "rom_destination",
            "label": "Device ROM folder",
            "description": "",
            "required": true,
            "multiple": false,
            "validation": {
                "mustExist": false,
                "allowedExtensions": [],
                "pathKind": "directory",
                "allowedPrefixes": ["/sdcard"],
            },
            "default": "/sdcard/ROMs",
            "options": [],
            "sensitive": false,
            "advanced": true,
            "metadata": {},
        })
    );
}

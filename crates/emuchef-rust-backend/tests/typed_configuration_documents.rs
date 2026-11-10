use std::fs;

use emuchef_rust_backend::authored_models::{
    parse_app_definition_yaml, AppDefinitionV1, TemplateVariableTypeV1,
};
use emuchef_rust_backend::request::handle_sidecar_value;
use emuchef_rust_backend::session::DocumentSessionManager;
use serde_json::{json, Value};

mod common;

fn request(sessions: &mut DocumentSessionManager, request_type: &str, payload: Value) -> Value {
    handle_sidecar_value(
        json!({
            "id": request_type,
            "type": request_type,
            "payload": payload,
        }),
        sessions,
    )
}

fn app_with_template_variable() -> String {
    include_str!("../fixtures/configuration_contracts/app-key-value.yaml").replace(
        "        path: retroarch/default.cfg",
        "        path: retroarch/default.cfg\n        variables:\n          install_path: device_path",
    )
}

fn write_catalog(root: &std::path::Path) -> std::path::PathBuf {
    let apps = root.join("apps");
    let recipes = root.join("recipes");
    for name in ["apps", "recipes", "device_plans", "device_profiles"] {
        fs::create_dir_all(root.join(name)).expect("authored catalog directory");
    }
    fs::create_dir_all(apps.join("retroarch")).expect("App Definition directory");
    fs::write(
        apps.join("retroarch.yaml"),
        include_bytes!("../fixtures/configuration_contracts/app-key-value.yaml"),
    )
    .expect("App Definition fixture");
    fs::write(apps.join("retroarch/default.cfg"), "audio_driver=alsa\n").expect("catalog template");
    recipes.join("recipe.retroarch.config.yaml")
}

#[test]
fn app_definition_document_load_and_save_preserve_template_declarations() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("authored");
    let apps = root.join("apps");
    for name in ["apps", "recipes", "device_plans", "device_profiles"] {
        fs::create_dir_all(root.join(name)).expect("authored catalog directory");
    }
    fs::create_dir_all(apps.join("retroarch")).expect("App Definition directory");
    fs::write(apps.join("retroarch.yaml"), app_with_template_variable())
        .expect("App Definition with template variables");
    fs::write(apps.join("retroarch/default.cfg"), "audio_driver=alsa\n").expect("template file");
    let mut sessions = common::test_sessions();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({"path": apps.join("retroarch.yaml"), "authoredRoot": root}),
    );
    assert_eq!(opened["ok"], true, "App Definition should open: {opened}");
    assert_eq!(
        opened["result"]["document"]["appDefinition"]["configurations"]["main"]["templates"]
            ["default"]["variables"]["install_path"],
        "device_path"
    );

    let saved = request(
        &mut sessions,
        "saveAppDefinition",
        json!({"documentId": "doc-1"}),
    );
    assert_eq!(saved["ok"], true, "App Definition should save: {saved}");
    let source = fs::read_to_string(apps.join("retroarch.yaml")).expect("saved YAML");
    let parsed: AppDefinitionV1 = parse_app_definition_yaml(&source).expect("saved model");
    assert_eq!(
        parsed.configurations["main"].templates["default"].variables["install_path"],
        TemplateVariableTypeV1::DevicePath
    );
}

#[test]
fn recipe_document_load_and_save_preserve_user_declarations_bindings_and_number_inputs() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("authored");
    let recipe_path = write_catalog(&root);
    let source = include_str!("../fixtures/configuration_contracts/recipe-key-value.yaml")
        .replace(
            "inputs:\n  user_template:",
            "inputs:\n  numeric_value:\n    type: number\n    role: generic\n    label: Numeric value\n    required: false\n    default: 1.25\n  user_template:",
        )
        .replace(
            "    input: user_template\n",
            "    input: user_template\n    variables:\n      frame_limit: number\n",
        )
        .replace(
            "  properties:\n",
            "  variables:\n    frame_limit: numeric_value\n  properties:\n",
        );
    fs::write(&recipe_path, source).expect("Recipe with user template declarations");
    let mut sessions = common::test_sessions();

    let opened = request(
        &mut sessions,
        "openRecipe",
        json!({"path": recipe_path, "authoredRoot": root}),
    );
    assert_eq!(opened["ok"], true, "Recipe should open: {opened}");
    let document = &opened["result"]["document"];
    assert_eq!(
        document["recipe"]["inputs"]["numeric_value"]["type"],
        "number"
    );
    assert_eq!(
        document["recipe"]["config"]["base"]["variables"]["frame_limit"],
        "number"
    );
    assert_eq!(
        document["recipe"]["config"]["variables"]["frame_limit"],
        "numeric_value"
    );

    let saved = request(&mut sessions, "saveRecipe", json!({"documentId": "doc-1"}));
    assert_eq!(saved["ok"], true, "Recipe should save: {saved}");
    let canonical = fs::read_to_string(recipe_path).expect("saved Recipe YAML");
    let reparsed = emuchef_rust_backend::yaml::load_recipe_from_bytes(
        canonical.as_bytes(),
        std::path::Path::new("recipes/recipe.retroarch.config.yaml"),
    )
    .expect("saved Recipe should parse");
    assert_eq!(
        reparsed.config.as_ref().unwrap().variables["frame_limit"],
        "numeric_value"
    );
    assert_eq!(
        reparsed.config.as_ref().unwrap().input_template_variables["frame_limit"],
        TemplateVariableTypeV1::Number
    );
}

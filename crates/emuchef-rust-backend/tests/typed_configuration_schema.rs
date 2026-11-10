use std::path::Path;

use emuchef_rust_backend::authored_models::{
    emit_app_definition_yaml, parse_app_definition_yaml, validate_app_definition,
    TemplateVariableTypeV1,
};
use emuchef_rust_backend::yaml::{emit_recipe_yaml, load_recipe_from_bytes};

const APP_SOURCE: &str = include_str!("../fixtures/configuration_contracts/app-key-value.yaml");
const RECIPE_SOURCE: &str =
    include_str!("../fixtures/configuration_contracts/recipe-key-value.yaml");

#[test]
fn app_template_variable_declarations_round_trip_without_changing_managed_contracts() {
    let source = APP_SOURCE.replace(
        "        path: retroarch/default.cfg",
        "        path: retroarch/default.cfg\n        variables:\n          root_path: device_path\n          frame_limit: number",
    );

    let app = parse_app_definition_yaml(&source).expect("App Definition should parse");
    assert!(validate_app_definition(&app).is_empty());
    let template = &app.configurations["main"].templates["default"];
    assert_eq!(template.path, "retroarch/default.cfg");
    assert_eq!(
        template.variables["root_path"],
        TemplateVariableTypeV1::DevicePath
    );
    assert_eq!(
        template.variables["frame_limit"],
        TemplateVariableTypeV1::Number
    );
    assert_eq!(
        app.configurations["main"].managed_properties["frame_limit"].path,
        ["frame_limit"]
    );

    let canonical = emit_app_definition_yaml(&app).expect("valid App Definition should emit");
    let reparsed = parse_app_definition_yaml(&canonical).expect("canonical YAML should parse");
    assert_eq!(reparsed, app);
    assert_eq!(
        reparsed.configurations["main"].templates["default"].variables,
        template.variables
    );
}

#[test]
fn only_template_variable_identifiers_receive_the_128_byte_cap() {
    let valid_id = format!("v{}", "a".repeat(127));
    let invalid_id = format!("v{}", "a".repeat(128));
    let source = APP_SOURCE.replace(
        "        path: retroarch/default.cfg",
        &format!(
            "        path: retroarch/default.cfg\n        variables:\n          {valid_id}: string\n          {invalid_id}: string"
        ),
    );

    let app = parse_app_definition_yaml(&source).expect("declarations should parse structurally");
    let diagnostics = validate_app_definition(&app);
    let variable_id_errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "app_config_template_variable_id_invalid")
        .collect::<Vec<_>>();
    assert_eq!(variable_id_errors.len(), 1);
    assert!(variable_id_errors[0].field.contains(&invalid_id));

    let long_property_id = format!("p{}", "a".repeat(128));
    let source = APP_SOURCE.replace(
        "      audio_driver:\n",
        &format!(
            "      {long_property_id}:\n        path: [custom_property]\n        type: string\n      audio_driver:\n"
        ),
    );
    let app = parse_app_definition_yaml(&source).expect("long existing authored ID should parse");
    assert!(!validate_app_definition(&app)
        .iter()
        .any(|diagnostic| diagnostic.code == "app_config_property_id_invalid"));
}

#[test]
fn recipe_user_template_declarations_and_input_bindings_round_trip() {
    let source = RECIPE_SOURCE
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

    let recipe = load_recipe_from_bytes(source.as_bytes(), Path::new("recipes/configuration.yaml"))
        .expect("user-template declarations should parse");
    let config = recipe.config.as_ref().expect("configuration should exist");
    assert_eq!(config.variables["frame_limit"], "numeric_value");
    let output = emit_recipe_yaml(&recipe).expect("Recipe should emit");
    let reparsed =
        load_recipe_from_bytes(output.as_bytes(), Path::new("recipes/configuration.yaml"))
            .expect("emitted Recipe should parse");
    assert_eq!(reparsed, recipe);
    assert!(output.contains("frame_limit: number"));
    assert!(output.contains("frame_limit: numeric_value"));
}

#[test]
fn duplicate_variable_declarations_and_bindings_are_rejected_at_parse_time() {
    let app_source = APP_SOURCE.replace(
        "        path: retroarch/default.cfg",
        "        path: retroarch/default.cfg\n        variables:\n          frame_limit: integer\n          frame_limit: number",
    );
    assert!(parse_app_definition_yaml(&app_source).is_err());

    let recipe_source = RECIPE_SOURCE.replace(
        "  properties:\n",
        "  variables:\n    frame_limit: input_a\n    frame_limit: input_b\n  properties:\n",
    );
    assert!(load_recipe_from_bytes(
        recipe_source.as_bytes(),
        Path::new("recipes/configuration.yaml")
    )
    .is_err());
}

#[test]
fn multiple_number_inputs_are_rejected_without_a_numeric_list_runtime_type() {
    let recipe_source = RECIPE_SOURCE.replace(
        "inputs:\n  user_template:",
        "inputs:\n  numeric_values:\n    type: number\n    role: generic\n    label: Numeric values\n    required: false\n    multiple: true\n  user_template:",
    );

    let error = load_recipe_from_bytes(
        recipe_source.as_bytes(),
        Path::new("recipes/configuration.yaml"),
    )
    .expect_err("multiple numeric inputs have no declared runtime list type");

    assert!(error
        .issue
        .expect("unsupported input shape should include an authored-data issue")
        .message
        .contains("numeric-list runtime values are unsupported"));
}

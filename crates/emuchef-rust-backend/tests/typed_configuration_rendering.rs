use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::authored_models::{
    parse_app_definition_yaml, AppDefinitionV1, TemplateVariableTypeV1,
};
use emuchef_rust_backend::configuration_renderer::{
    render_recipe_configuration, BindingSource, ConfigurationRenderRequest,
    CurrentSetupLocationBinding, RenderAuthority, ResetValueSource, ResolvedInputBinding,
    ResolvedResetValue,
};
use emuchef_rust_backend::model::{
    InputDeclaration, InputValidation, OrderedMap, Recipe, RecipeConfigOverrideV1,
    RecipeConfigPropertyV1, RecipeConfigurationBaseV1,
};
use emuchef_rust_backend::storage_targets::{StorageTargetBinding, StorageTargetVolumeKind};
use serde_json::{json, Value};
use sha2::Digest;

struct RenderWorkspace {
    _directory: tempfile::TempDir,
    authored_root: PathBuf,
}

impl RenderWorkspace {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary render directory");
        let authored_root = directory.path().join("authored");
        fs::create_dir_all(authored_root.join("apps/example"))
            .expect("App Definition template directory");
        Self {
            _directory: directory,
            authored_root,
        }
    }

    fn write_catalog_template(&self, bytes: &[u8]) {
        fs::write(self.authored_root.join("apps/example/default.cfg"), bytes)
            .expect("catalog template");
    }

    fn write_user_template(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self._directory.path().join(name);
        fs::write(&path, bytes).expect("user template");
        path
    }
}

fn app(
    format: &str,
    template_variables: Option<&str>,
    managed_properties: &str,
) -> AppDefinitionV1 {
    let mut source = format!(
        "schema_version: 1\nkind: app_definition\nid: example\nname: Example\npackage_id: com.example.app\ntargets:\n  settings:\n    kind: file\n    location: app_data\n    path: files/settings.cfg\nconfigurations:\n  main:\n    format: {format}\n    target: settings\n"
    );
    source.push_str("    templates:\n      default:\n        path: example/default.cfg\n");
    if let Some(variables) = template_variables {
        source.push_str("        variables:\n");
        source.push_str(variables);
    }
    if !managed_properties.is_empty() {
        source.push_str("    managed_properties:\n");
        source.push_str(managed_properties);
    }
    parse_app_definition_yaml(&source).expect("App Definition should parse")
}

fn recipe(app: &AppDefinitionV1) -> Recipe {
    let mut recipe = emuchef_rust_backend::yaml::load_recipe_from_bytes(
        include_bytes!("../fixtures/configuration_contracts/recipe-key-value.yaml"),
        Path::new("recipes/configuration.yaml"),
    )
    .expect("Recipe fixture should parse");
    let config = recipe.config.as_mut().expect("configuration fixture");
    config.app_ref = app.id.clone();
    config.configuration = "main".to_string();
    config.target = "settings".to_string();
    config.base = RecipeConfigurationBaseV1::Template("default".to_string());
    config.input_template_variables.clear();
    config.variables.clear();
    config.properties.clear();
    config.unmanaged_literals.clear();
    recipe.inputs.clear();
    recipe
}

fn input(type_name: &str, required: bool, sensitive: bool) -> InputDeclaration {
    InputDeclaration {
        type_name: type_name.to_string(),
        role: "generic".to_string(),
        label: "Render input".to_string(),
        description: None,
        required,
        multiple: false,
        validation: InputValidation {
            must_exist: false,
            allowed_extensions: Vec::new(),
            path_kind: None,
            allowed_prefixes: Vec::new(),
        },
        default: Value::Null,
        options: Vec::new(),
        sensitive,
        advanced: false,
        metadata: OrderedMap::new(),
    }
}

fn binding(
    recipe: &Recipe,
    input_id: &str,
    type_name: &str,
    value: Value,
    source: BindingSource,
) -> ResolvedInputBinding {
    ResolvedInputBinding {
        key: format!("{}/{}", recipe.id, input_id),
        recipe_id: recipe.id.clone(),
        input_id: input_id.to_string(),
        type_name: type_name.to_string(),
        value: Some(value),
        source: Some(source),
    }
}

fn render<'a>(
    workspace: &'a RenderWorkspace,
    app: &'a AppDefinitionV1,
    recipe: &'a Recipe,
    inputs: &'a [ResolvedInputBinding],
    authority: Option<&'a RenderAuthority>,
    locations: &'a [CurrentSetupLocationBinding],
    resets: &'a OrderedMap<ResolvedResetValue>,
) -> Result<
    emuchef_rust_backend::configuration_renderer::RenderedConfiguration,
    emuchef_rust_backend::configuration_renderer::ConfigurationRenderError,
> {
    render_recipe_configuration(ConfigurationRenderRequest {
        app,
        recipe,
        authored_root: &workspace.authored_root,
        inputs,
        authority,
        setup_locations: locations,
        resolved_resets: resets,
    })
}

#[test]
fn key_value_rendering_is_deterministic_typed_and_direct_operations_win_collisions() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(
        b"audio_driver={{emuchef:driver}}\nnetwork_enabled={{emuchef:enabled}}\nframe_limit={{emuchef:limit}}\ncustom_extension=retained\n",
    );
    let app = app(
        "key_value/v1",
        Some("          driver: string\n          enabled: boolean\n          limit: integer\n"),
        "      audio_driver:\n        path: [audio_driver]\n        type: string\n      network_enabled:\n        path: [network_enabled]\n        type: boolean\n      frame_limit:\n        path: [frame_limit]\n        type: integer\n",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("driver_input".into(), input("string", true, false));
    recipe
        .inputs
        .insert("enabled_input".into(), input("boolean", true, false));
    recipe
        .inputs
        .insert("limit_input".into(), input("integer", true, false));
    let config = recipe.config.as_mut().unwrap();
    config.variables = [
        ("driver".into(), "driver_input".into()),
        ("enabled".into(), "enabled_input".into()),
        ("limit".into(), "limit_input".into()),
    ]
    .into_iter()
    .collect();
    config.properties.insert(
        "network_enabled".into(),
        emuchef_rust_backend::model::RecipeConfigPropertyV1 {
            label: None,
            description: None,
            required: false,
            advanced: false,
            override_operation: Some(RecipeConfigOverrideV1::Value(json!(false))),
        },
    );
    let inputs = vec![
        binding(
            &recipe,
            "driver_input",
            "string",
            json!("alsa"),
            BindingSource::Explicit,
        ),
        binding(
            &recipe,
            "enabled_input",
            "boolean",
            json!(true),
            BindingSource::RecipeDefault,
        ),
        binding(
            &recipe,
            "limit_input",
            "integer",
            json!(120),
            BindingSource::UserConfiguration,
        ),
    ];

    let first = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("valid key-value template should render");
    let variable_bindings = recipe.config.as_ref().unwrap().variables.clone();
    let mut reordered_recipe = recipe.clone();
    reordered_recipe.config.as_mut().unwrap().variables = variable_bindings
        .iter()
        .rev()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let second = render(
        &workspace,
        &app,
        &reordered_recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("binding order must not affect output");

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.sha256(), second.sha256());
    assert_eq!(first.provenance(), second.provenance());
    let expected_hash = sha2::Sha256::digest(first.bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(first.sha256(), expected_hash);
    assert_eq!(
        first.bytes(),
        b"audio_driver=\"alsa\"\ncustom_extension=\"retained\"\nframe_limit=120\nnetwork_enabled=false\n"
    );
    assert_eq!(
        first.provenance().managed_property_collisions[0].property_id,
        "network_enabled"
    );
    assert_eq!(
        first.provenance().managed_property_collisions[0].winner,
        "recipe_operation"
    );
}

#[test]
fn json_rendering_encodes_typed_scalars_and_preserves_unknown_literals() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(
        br#"{"ui":{"name":"{{emuchef:name}}","scale":"{{emuchef:scale}}"},"flags":{"enabled":"{{emuchef:enabled}}"},"vendor":{"literal":true}}"#,
    );
    let app = app(
        "json/v1",
        Some("          name: string\n          scale: number\n          enabled: boolean\n"),
        "      name:\n        path: [ui, name]\n        type: string\n      scale:\n        path: [ui, scale]\n        type: number\n      enabled:\n        path: [flags, enabled]\n        type: boolean\n",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("name_input".into(), input("string", true, false));
    recipe
        .inputs
        .insert("scale_input".into(), input("number", true, false));
    recipe
        .inputs
        .insert("enabled_input".into(), input("boolean", true, false));
    recipe.config.as_mut().unwrap().variables = [
        ("name".into(), "name_input".into()),
        ("scale".into(), "scale_input".into()),
        ("enabled".into(), "enabled_input".into()),
    ]
    .into_iter()
    .collect();
    let inputs = vec![
        binding(
            &recipe,
            "name_input",
            "string",
            json!("A\nB"),
            BindingSource::Explicit,
        ),
        binding(
            &recipe,
            "scale_input",
            "number",
            json!(1.25),
            BindingSource::Explicit,
        ),
        binding(
            &recipe,
            "enabled_input",
            "boolean",
            json!(true),
            BindingSource::Explicit,
        ),
    ];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("JSON strings and typed values should render safely");

    assert_eq!(
        rendered.bytes(),
        br#"{"flags":{"enabled":true},"ui":{"name":"A\nB","scale":1.25},"vendor":{"literal":true}}"#
            .iter()
            .copied()
        .chain(b"\n".iter().copied())
            .collect::<Vec<_>>()
    );
    assert_eq!(rendered.provenance().format, "json/v1");
}

#[test]
fn user_template_declarations_render_without_changing_app_property_authority() {
    let workspace = RenderWorkspace::new();
    let user_path = workspace.write_user_template(
        "user.cfg",
        b"audio_driver={{emuchef:driver}}\ncustom_extension=preserved\n",
    );
    let app = app(
        "key_value/v1",
        None,
        "      audio_driver:\n        path: [audio_driver]\n        type: string\n        validators:\n          allowed_values: [alsa, pulse]\n",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("user_template".into(), input("file", true, false));
    recipe
        .inputs
        .insert("driver_input".into(), input("string", true, false));
    let config = recipe.config.as_mut().unwrap();
    config.base = RecipeConfigurationBaseV1::Input("user_template".into());
    config.input_template_variables = [("driver".into(), TemplateVariableTypeV1::String)]
        .into_iter()
        .collect();
    config
        .variables
        .insert("driver".into(), "driver_input".into());
    let inputs = vec![
        binding(
            &recipe,
            "user_template",
            "file",
            json!(user_path.to_string_lossy().to_string()),
            BindingSource::Explicit,
        ),
        binding(
            &recipe,
            "driver_input",
            "string",
            json!("alsa"),
            BindingSource::Explicit,
        ),
    ];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("admitted user template should render");

    assert_eq!(
        rendered.bytes(),
        b"audio_driver=\"alsa\"\ncustom_extension=\"preserved\"\n"
    );
    assert_eq!(rendered.provenance().template_source.kind, "recipe_input");
    assert_eq!(rendered.provenance().template_source.id, "user_template");
    assert_eq!(rendered.provenance().target.id, "settings");

    let mut invalid_inputs = inputs.clone();
    invalid_inputs[1].value = Some(json!("not-an-app-approved-driver"));
    let invalid = render(
        &workspace,
        &app,
        &recipe,
        &invalid_inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("user declarations cannot replace App Definition validators");
    assert_eq!(invalid.code(), "render_managed_value_invalid");
}

#[test]
fn invalid_substituted_managed_value_fails_before_a_valid_direct_override() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"frame_limit={{emuchef:limit}}\n");
    let app = app(
        "key_value/v1",
        Some("          limit: number\n"),
        "      frame_limit:\n        path: [frame_limit]\n        type: integer\n",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("limit_input".into(), input("number", true, false));
    let config = recipe.config.as_mut().unwrap();
    config
        .variables
        .insert("limit".into(), "limit_input".into());
    config.properties.insert(
        "frame_limit".into(),
        emuchef_rust_backend::model::RecipeConfigPropertyV1 {
            label: None,
            description: None,
            required: false,
            advanced: false,
            override_operation: Some(RecipeConfigOverrideV1::Value(json!(60))),
        },
    );
    let inputs = vec![binding(
        &recipe,
        "limit_input",
        "number",
        json!(20.5),
        BindingSource::Explicit,
    )];

    let error = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("invalid substituted managed value must block before override");

    assert_eq!(error.code(), "render_managed_value_invalid");
}

#[test]
fn duplicate_keys_malformed_placeholders_and_numeric_coercion_fail_closed() {
    let workspace = RenderWorkspace::new();
    let app = app("key_value/v1", Some("          amount: number\n"), "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("amount_input".into(), input("number", false, false));
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("amount".into(), "amount_input".into());

    workspace.write_catalog_template(b"amount={{emuchef:amount}}\namount=1\n");
    let duplicate = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("duplicate key-value statements must be rejected");
    assert_eq!(duplicate.code(), "render_template_duplicate_key");

    workspace.write_catalog_template(b"amount=prefix{{emuchef:amount}}\n");
    let malformed = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("partial placeholder must be rejected");
    assert_eq!(malformed.code(), "render_placeholder_position_invalid");

    workspace.write_catalog_template(b"amount={{emuchef:amount}}\n");
    let coerced_inputs = vec![binding(
        &recipe,
        "amount_input",
        "number",
        json!("1.25"),
        BindingSource::Explicit,
    )];
    let coercion = render(
        &workspace,
        &app,
        &recipe,
        &coerced_inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("numeric-looking strings must not be coerced");
    assert_eq!(coercion.code(), "render_input_value_invalid");
}

#[test]
fn duplicate_json_object_keys_are_rejected() {
    let workspace = RenderWorkspace::new();
    let json_app = app("json/v1", None, "");
    let json_recipe = recipe(&json_app);
    workspace.write_catalog_template(br#"{"settings":{"first":1,"first":2}}"#);
    let duplicate_json_key = render(
        &workspace,
        &json_app,
        &json_recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("duplicate JSON keys must not be silently overwritten");
    assert_eq!(duplicate_json_key.code(), "render_template_malformed");
}

#[test]
fn empty_clear_absent_and_t11_reset_remain_distinct() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"audio_driver=template_value\n");
    let app = app(
        "key_value/v1",
        None,
        "      audio_driver:\n        path: [audio_driver]\n        type: string\n        default: app_default\n        clearable: true\n",
    );
    let mut recipe = recipe(&app);
    let exposure = RecipeConfigPropertyV1 {
        label: None,
        description: None,
        required: false,
        advanced: false,
        override_operation: None,
    };
    recipe
        .config
        .as_mut()
        .unwrap()
        .properties
        .insert("audio_driver".into(), exposure);

    let absent = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("an absent operation preserves the template value");
    assert_eq!(absent.bytes(), b"audio_driver=\"template_value\"\n");

    recipe.config.as_mut().unwrap().properties["audio_driver"].override_operation =
        Some(RecipeConfigOverrideV1::Value(json!("")));
    let empty = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("explicit empty string is a value operation");
    assert_eq!(empty.bytes(), b"audio_driver=\"\"\n");

    recipe.config.as_mut().unwrap().properties["audio_driver"].override_operation =
        Some(RecipeConfigOverrideV1::Clear);
    let clear = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("clear removes a clearable managed property");
    assert!(clear.bytes().is_empty());

    recipe.config.as_mut().unwrap().properties["audio_driver"].override_operation =
        Some(RecipeConfigOverrideV1::Reset);
    let resets = [(
        "audio_driver".into(),
        ResolvedResetValue {
            value: json!("t11_value"),
            source: ResetValueSource::DeviceIndependentDefault,
        },
    )]
    .into_iter()
    .collect();
    let reset = render(&workspace, &app, &recipe, &[], None, &[], &resets)
        .expect("reset uses only the trusted pre-resolved T11 value");
    assert_eq!(reset.bytes(), b"audio_driver=\"t11_value\"\n");
    assert_eq!(
        reset.provenance().managed_property_operations[0]
            .reset_source
            .as_deref(),
        Some("device_independent_default")
    );
}

#[test]
fn key_value_string_newlines_cannot_inject_assignments_or_comments() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"vendor_note={{emuchef:note}}\n");
    let app = app("key_value/v1", Some("          note: string\n"), "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("note_input".into(), input("string", true, false));
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("note".into(), "note_input".into());
    let inputs = vec![binding(
        &recipe,
        "note_input",
        "string",
        json!("first line\n#not-a-comment\ninjected=true\""),
        BindingSource::Explicit,
    )];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("newlines and quotes are escaped inside a single key-value scalar");

    assert_eq!(
        rendered.bytes(),
        b"vendor_note=\"first line\\n#not-a-comment\\ninjected=true\\\"\"\n"
    );
}

#[test]
fn missing_undeclared_and_duplicate_variable_bindings_fail_closed() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"audio_driver={{emuchef:driver}}\n");
    let app = app("key_value/v1", Some("          driver: string\n"), "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("driver_input".into(), input("string", true, false));

    let missing = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("a referenced variable without a config.variables binding must block");
    assert_eq!(missing.code(), "render_variable_binding_missing");

    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("undeclared".into(), "driver_input".into());
    let undeclared = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("config.variables cannot declare a new variable");
    assert_eq!(undeclared.code(), "render_recipe_configuration_invalid");

    recipe.config.as_mut().unwrap().variables.clear();
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("driver".into(), "driver_input".into());
    let resolved = binding(
        &recipe,
        "driver_input",
        "string",
        json!("alsa"),
        BindingSource::Explicit,
    );
    let duplicate = render(
        &workspace,
        &app,
        &recipe,
        &[resolved.clone(), resolved],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("duplicate resolved values for one Recipe input must block");
    assert_eq!(duplicate.code(), "render_input_duplicate");
}

#[test]
fn device_path_variables_reject_host_path_inputs_and_traversal() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"install_path={{emuchef:destination}}\n");
    let app = app(
        "key_value/v1",
        Some("          destination: device_path\n"),
        "",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("destination_input".into(), input("path", true, false));
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("destination".into(), "destination_input".into());
    let host_path = binding(
        &recipe,
        "destination_input",
        "path",
        json!("/tmp/settings.cfg"),
        BindingSource::Explicit,
    );
    let error = render(
        &workspace,
        &app,
        &recipe,
        &[host_path],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("host path inputs cannot populate a device_path variable");
    assert_eq!(error.code(), "render_recipe_configuration_invalid");

    recipe.inputs.insert(
        "destination_input".into(),
        input("device_path", true, false),
    );
    let traversal = binding(
        &recipe,
        "destination_input",
        "device_path",
        json!("/storage/emulated/0/../private/settings.cfg"),
        BindingSource::Explicit,
    );
    let error = render(
        &workspace,
        &app,
        &recipe,
        &[traversal],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("device paths with traversal components must block");
    assert_eq!(error.code(), "render_variable_value_invalid");
}

#[cfg(unix)]
#[test]
fn user_template_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let workspace = RenderWorkspace::new();
    let target = workspace.write_user_template("real.cfg", b"audio_driver=alsa\n");
    let link = workspace._directory.path().join("linked.cfg");
    symlink(target, &link).expect("user template symlink");
    let app = app("key_value/v1", None, "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("user_template".into(), input("file", true, false));
    recipe.config.as_mut().unwrap().base = RecipeConfigurationBaseV1::Input("user_template".into());
    let inputs = vec![binding(
        &recipe,
        "user_template",
        "file",
        json!(link.to_string_lossy().to_string()),
        BindingSource::Explicit,
    )];

    let error = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("symbolic links must not become template sources");

    assert_eq!(error.code(), "render_template_unavailable");
}

#[cfg(unix)]
#[test]
fn user_template_parent_symlinks_remain_supported() {
    use std::os::unix::fs::symlink;

    let workspace = RenderWorkspace::new();
    let external_directory = workspace._directory.path().join("external-templates");
    fs::create_dir(&external_directory).expect("external template directory");
    fs::write(
        external_directory.join("selected.cfg"),
        b"audio_driver=alsa\n",
    )
    .expect("external user template");
    let linked_directory = workspace._directory.path().join("selected-templates");
    symlink(&external_directory, &linked_directory).expect("user template directory link");
    let user_template = linked_directory.join("selected.cfg");
    let app = app("key_value/v1", None, "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("user_template".into(), input("file", true, false));
    recipe.config.as_mut().unwrap().base = RecipeConfigurationBaseV1::Input("user_template".into());
    let inputs = vec![binding(
        &recipe,
        "user_template",
        "file",
        json!(user_template.to_string_lossy().to_string()),
        BindingSource::Explicit,
    )];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("a user template may remain under its selected symlinked directory");

    assert_eq!(rendered.bytes(), b"audio_driver=\"alsa\"\n");
}

#[cfg(unix)]
#[test]
fn catalog_template_parent_symlink_cannot_escape_the_app_tree() {
    use std::os::unix::fs::symlink;

    let workspace = RenderWorkspace::new();
    let outside = workspace._directory.path().join("outside-app");
    fs::create_dir_all(&outside).expect("outside directory");
    fs::write(outside.join("default.cfg"), b"audio_driver=alsa\n").expect("outside template");
    let app_directory = workspace.authored_root.join("apps/example");
    fs::remove_dir(&app_directory).expect("remove original App template directory");
    symlink(&outside, &app_directory).expect("symlink App template directory outside root");
    let app = app("key_value/v1", None, "");
    let recipe = recipe(&app);

    let error = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("a parent symlink must not redirect an App Definition template");

    assert_eq!(error.code(), "render_template_unavailable");
}

#[cfg(unix)]
#[test]
fn catalog_template_final_component_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let workspace = RenderWorkspace::new();
    let outside = workspace.write_user_template("outside.cfg", b"audio_driver=alsa\n");
    let catalog_template = workspace.authored_root.join("apps/example/default.cfg");
    symlink(outside, &catalog_template).expect("catalog template symlink");
    let app = app("key_value/v1", None, "");
    let recipe = recipe(&app);

    let error = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("a final-component symlink must not become a catalog template");

    assert_eq!(error.code(), "render_template_unavailable");
}

#[test]
fn regular_catalog_template_files_remain_renderable() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"audio_driver=alsa\n");
    let app = app("key_value/v1", None, "");
    let recipe = recipe(&app);

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("a regular file in the App template tree remains valid");

    assert_eq!(rendered.bytes(), b"audio_driver=\"alsa\"\n");
}

#[cfg(unix)]
#[test]
fn oversized_template_is_rejected_before_opening_unreadable_contents() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = RenderWorkspace::new();
    let template_path = workspace.authored_root.join("apps/example/default.cfg");
    let file = fs::File::create(&template_path).expect("sparse template");
    file.set_len((2 * 1024 * 1024 + 1) as u64)
        .expect("make oversized sparse template");
    let mut permissions = fs::metadata(&template_path)
        .expect("template metadata")
        .permissions();
    permissions.set_mode(0o0);
    fs::set_permissions(&template_path, permissions).expect("make template unreadable");
    let app = app("key_value/v1", None, "");
    let recipe = recipe(&app);

    let error = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("the size cap must reject the file before opening its contents");

    assert_eq!(error.code(), "render_template_too_large");
}

#[test]
fn unsupported_and_malformed_template_tokens_fail_in_both_formats() {
    let cases: [(&str, &[u8], &str); 10] = [
        (
            "key_value/v1",
            b"value={{other:unknown}}\n",
            "render_placeholder_namespace_unsupported",
        ),
        (
            "json/v1",
            br#"{"value":"{{other:unknown}}"}"#,
            "render_placeholder_namespace_unsupported",
        ),
        (
            "key_value/v1",
            b"value={{emuchef:broken}\n",
            "render_placeholder_malformed",
        ),
        (
            "json/v1",
            br#"{"value":"{{emuchef:broken}"}"#,
            "render_placeholder_malformed",
        ),
        (
            "key_value/v1",
            b"value=prefix {{emuchef:setting}} suffix\n",
            "render_placeholder_position_invalid",
        ),
        (
            "json/v1",
            br#"{"value":"prefix {{emuchef:setting}} suffix"}"#,
            "render_placeholder_position_invalid",
        ),
        (
            "key_value/v1",
            b"{{other:key}}=value\n",
            "render_placeholder_position_invalid",
        ),
        (
            "json/v1",
            br#"{"{{other:key}}":"value"}"#,
            "render_placeholder_position_invalid",
        ),
        (
            "key_value/v1",
            b"# {{other:comment}}\nvalue=static\n",
            "render_placeholder_position_invalid",
        ),
        (
            "json/v1",
            br#"{"value":"{{emuchef:setting}} and {{other:setting}}"}"#,
            "render_placeholder_malformed",
        ),
    ];

    for (format, template, expected_code) in cases {
        let workspace = RenderWorkspace::new();
        workspace.write_catalog_template(template);
        let app = app(format, None, "");
        let recipe = recipe(&app);

        let error = render(
            &workspace,
            &app,
            &recipe,
            &[],
            None,
            &[],
            &OrderedMap::new(),
        )
        .expect_err("unsupported template expressions must not remain literal");

        assert_eq!(
            error.code(),
            expected_code,
            "format={format}, template={template:?}"
        );
    }
}

#[test]
fn ordinary_single_brace_literals_remain_static_in_both_formats() {
    for (format, template) in [
        (
            "key_value/v1",
            b"value=literal {single} } brace\n".as_slice(),
        ),
        (
            "json/v1",
            br#"{"value":"literal {single} } brace"}"#.as_slice(),
        ),
    ] {
        let workspace = RenderWorkspace::new();
        workspace.write_catalog_template(template);
        let app = app(format, None, "");
        let recipe = recipe(&app);

        let rendered = render(
            &workspace,
            &app,
            &recipe,
            &[],
            None,
            &[],
            &OrderedMap::new(),
        )
        .expect("ordinary brace text is not a template expression");

        assert!(rendered
            .bytes()
            .windows(8)
            .any(|window| window == b"{single}"));
    }
}

#[test]
fn key_value_hash_prefixed_managed_property_cannot_become_a_comment() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"ordinary=value\n");
    let app = app(
        "key_value/v1",
        None,
        "      comment_key:\n        path: [\"#comment\"]\n        type: string\n",
    );
    let mut recipe = recipe(&app);
    recipe.config.as_mut().unwrap().properties.insert(
        "comment_key".into(),
        RecipeConfigPropertyV1 {
            label: None,
            description: None,
            required: false,
            advanced: false,
            override_operation: Some(RecipeConfigOverrideV1::Value(json!("active value"))),
        },
    );

    let error = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("a key beginning with the comment marker cannot be serialized as a property");

    assert_eq!(error.code(), "render_key_value_path_invalid");
}

#[test]
fn oversized_and_unsupported_encoding_templates_fail_closed() {
    let workspace = RenderWorkspace::new();
    let app = app("key_value/v1", None, "");
    let recipe = recipe(&app);

    let oversized_path = workspace.authored_root.join("apps/example/default.cfg");
    fs::File::create(&oversized_path)
        .expect("sparse oversized template")
        .set_len((2 * 1024 * 1024 + 1) as u64)
        .expect("set oversized template length");
    let oversized = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("oversized template must be rejected before parsing");
    assert_eq!(oversized.code(), "render_template_too_large");

    workspace.write_catalog_template(&[0xff, 0xfe]);
    let unsupported_encoding = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("unsupported template encoding must be rejected");
    assert_eq!(
        unsupported_encoding.code(),
        "render_template_encoding_unsupported"
    );
}

#[test]
fn number_variables_keep_large_integer_precision_and_finite_float_values() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(br#"{"amount":"{{emuchef:amount}}"}"#);
    let app = app(
        "json/v1",
        Some("          amount: number\n"),
        "      amount:\n        path: [amount]\n        type: number\n",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("amount_input".into(), input("number", true, false));
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("amount".into(), "amount_input".into());
    let inputs = vec![binding(
        &recipe,
        "amount_input",
        "number",
        json!(u64::MAX),
        BindingSource::Explicit,
    )];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("largest unsigned integer remains a valid JSON number");
    let output: Value = serde_json::from_slice(rendered.bytes()).expect("rendered JSON");
    assert_eq!(output["amount"].as_u64(), Some(u64::MAX));
}

#[test]
fn sensitive_values_are_absent_from_errors_and_provenance() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"vendor_token={{emuchef:secret}}\n");
    let app = app("key_value/v1", Some("          secret: string\n"), "");
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("secret_input".into(), input("string", true, true));
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("secret".into(), "secret_input".into());
    let secret = "Sensitive-Value-42";
    let inputs = vec![binding(
        &recipe,
        "secret_input",
        "string",
        json!(secret),
        BindingSource::Explicit,
    )];

    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("unknown literal can receive an explicitly declared variable");

    assert!(std::str::from_utf8(rendered.bytes())
        .unwrap()
        .contains(secret));
    let encoded_provenance = serde_json::to_string(rendered.provenance()).unwrap_or_default();
    assert!(!encoded_provenance.contains(secret));
    assert!(!format!("{rendered:?}").contains(secret));
    assert!(rendered.provenance().variables[0].sensitive);

    let invalid_secret = binding(
        &recipe,
        "secret_input",
        "string",
        json!({"token": secret}),
        BindingSource::Explicit,
    );
    let error = render(
        &workspace,
        &app,
        &recipe,
        &[invalid_secret],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("invalid sensitive values must produce sanitized errors");
    assert!(!format!("{error:?}").contains(secret));
}

#[test]
fn setup_location_resolution_requires_current_device_session_authority() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"install_path={{emuchef:destination}}\n");
    let app = app(
        "key_value/v1",
        Some("          destination: device_path\n"),
        "",
    );
    let mut recipe = recipe(&app);
    recipe.inputs.insert(
        "location_input".into(),
        input("setup_location_ref", true, false),
    );
    recipe
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("destination".into(), "location_input".into());
    let inputs = vec![binding(
        &recipe,
        "location_input",
        "setup_location_ref",
        json!("roms"),
        BindingSource::DevicePlan,
    )];
    let authority = RenderAuthority {
        device_handle: "device-session-handle".into(),
        device_plan: "plan.example".into(),
        session_epoch: 12,
    };
    let binding = StorageTargetBinding {
        id: "roms".into(),
        device_plan: "plan.example".into(),
        session_epoch: 11,
        name: "ROMs".into(),
        relative_path: "EmuChef/ROMs".into(),
        volume_handle: "11111111111111111111111111111111".into(),
        volume_kind: StorageTargetVolumeKind::InternalShared,
        mount_root: "/storage/emulated/0".into(),
        volume_identity: "22222222222222222222222222222222".into(),
        volume_continuity: "33333333333333333333333333333333".into(),
        observation_token: "44444444444444444444444444444444".into(),
    };

    let stale_location = CurrentSetupLocationBinding {
        device_handle: authority.device_handle.clone(),
        binding: binding.clone(),
    };
    let error = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&authority),
        &[stale_location],
        &OrderedMap::new(),
    )
    .expect_err("stale Setup Location session must block rendering");

    assert_eq!(error.code(), "render_setup_location_stale");

    let mut current_binding = binding;
    current_binding.session_epoch = 12;
    let current_location = CurrentSetupLocationBinding {
        device_handle: authority.device_handle.clone(),
        binding: current_binding.clone(),
    };
    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&authority),
        std::slice::from_ref(&current_location),
        &OrderedMap::new(),
    )
    .expect("current Setup Location binding should resolve to its device path");
    assert_eq!(
        rendered.bytes(),
        b"install_path=\"/storage/emulated/0/EmuChef/ROMs\"\n"
    );
    let provenance = rendered.provenance().device_authority.as_ref().unwrap();
    let provenance_json = serde_json::to_string(rendered.provenance()).unwrap();
    assert!(!provenance_json.contains("/storage/emulated/0"));
    let original_authority_hash = provenance.authority_sha256.clone();

    let mut changed_identity_binding = current_binding.clone();
    changed_identity_binding.volume_identity = "55555555555555555555555555555555".into();
    let changed_identity = CurrentSetupLocationBinding {
        device_handle: authority.device_handle.clone(),
        binding: changed_identity_binding,
    };
    let identity_render = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&authority),
        std::slice::from_ref(&changed_identity),
        &OrderedMap::new(),
    )
    .expect("a current volume identity change still has a valid device path");
    assert_eq!(identity_render.bytes(), rendered.bytes());
    assert_ne!(
        identity_render
            .provenance()
            .device_authority
            .as_ref()
            .unwrap()
            .authority_sha256,
        original_authority_hash
    );

    let mut changed_continuity_binding = current_binding.clone();
    changed_continuity_binding.volume_continuity = "66666666666666666666666666666666".into();
    let changed_continuity = CurrentSetupLocationBinding {
        device_handle: authority.device_handle.clone(),
        binding: changed_continuity_binding,
    };
    let continuity_render = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&authority),
        std::slice::from_ref(&changed_continuity),
        &OrderedMap::new(),
    )
    .expect("a current volume continuity change still has a valid device path");
    assert_eq!(continuity_render.bytes(), rendered.bytes());
    assert_ne!(
        continuity_render
            .provenance()
            .device_authority
            .as_ref()
            .unwrap()
            .authority_sha256,
        original_authority_hash
    );

    let mut changed_authority = authority;
    changed_authority.device_handle = "different-device-session".into();
    let changed_device_error = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&changed_authority),
        &[current_location],
        &OrderedMap::new(),
    )
    .expect_err("a device binding from another session must be rejected");
    assert_eq!(changed_device_error.code(), "render_setup_location_stale");

    let changed_location = CurrentSetupLocationBinding {
        device_handle: changed_authority.device_handle.clone(),
        binding: StorageTargetBinding {
            volume_handle: "55555555555555555555555555555555".into(),
            ..current_binding
        },
    };
    let changed = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&changed_authority),
        &[changed_location],
        &OrderedMap::new(),
    )
    .expect("fresh current device binding should render");
    assert_ne!(
        changed
            .provenance()
            .device_authority
            .as_ref()
            .unwrap()
            .authority_sha256,
        original_authority_hash
    );
}

#[test]
fn unresolved_reset_blocks_and_trusted_pre_resolved_reset_is_used_without_recomputation() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"frame_limit=30\n");
    let app = app(
        "key_value/v1",
        None,
        "      frame_limit:\n        path: [frame_limit]\n        type: integer\n        default: 60\n        validators:\n          minimum: 30\n          maximum: 240\n",
    );
    let mut recipe = recipe(&app);
    recipe.config.as_mut().unwrap().properties.insert(
        "frame_limit".into(),
        emuchef_rust_backend::model::RecipeConfigPropertyV1 {
            label: None,
            description: None,
            required: false,
            advanced: false,
            override_operation: Some(RecipeConfigOverrideV1::Reset),
        },
    );

    let unresolved = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect_err("reset without T11 value and provenance must block");
    assert_eq!(unresolved.code(), "render_reset_unresolved");

    let resets = [(
        "frame_limit".into(),
        ResolvedResetValue {
            value: json!(90),
            source: ResetValueSource::ExactModelRecommendation,
        },
    )]
    .into_iter()
    .collect();
    let rendered = render(&workspace, &app, &recipe, &[], None, &[], &resets)
        .expect("trusted pre-resolved reset should render");
    assert_eq!(rendered.bytes(), b"frame_limit=90\n");
}

#[test]
fn catalog_authority_fingerprint_tracks_managed_validator_changes() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"frame_limit=12\n");
    let app_before = app(
        "key_value/v1",
        None,
        "      frame_limit:\n        path: [frame_limit]\n        type: integer\n",
    );
    let mut app_after = app_before.clone();
    app_after
        .configurations
        .get_mut("main")
        .unwrap()
        .managed_properties
        .get_mut("frame_limit")
        .unwrap()
        .validators
        .maximum = Some(json!(20));
    let recipe_before = recipe(&app_before);
    let recipe_after = recipe(&app_after);

    let before = render(
        &workspace,
        &app_before,
        &recipe_before,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the original managed-property authority should render");
    let after = render(
        &workspace,
        &app_after,
        &recipe_after,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the changed validator still accepts the template value");

    assert_eq!(before.bytes(), after.bytes());
    assert_ne!(
        before.provenance().catalog_authority.authority_sha256,
        after.provenance().catalog_authority.authority_sha256,
    );
}

#[test]
fn catalog_authority_fingerprint_tracks_template_declaration_changes() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"display_name={{emuchef:name}}\n");
    let app_before = app("key_value/v1", Some("          name: string\n"), "");
    let app_after = app(
        "key_value/v1",
        Some("          name: string\n          unused: boolean\n"),
        "",
    );
    let mut recipe_before = recipe(&app_before);
    recipe_before
        .inputs
        .insert("name_input".into(), input("string", true, false));
    recipe_before
        .config
        .as_mut()
        .unwrap()
        .variables
        .insert("name".into(), "name_input".into());
    let mut recipe_after = recipe(&app_after);
    recipe_after.inputs = recipe_before.inputs.clone();
    recipe_after.config.as_mut().unwrap().variables =
        recipe_before.config.as_ref().unwrap().variables.clone();
    let inputs = vec![binding(
        &recipe_before,
        "name_input",
        "string",
        json!("same"),
        BindingSource::Explicit,
    )];

    let before = render(
        &workspace,
        &app_before,
        &recipe_before,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the original declaration should render");
    let after = render(
        &workspace,
        &app_after,
        &recipe_after,
        &inputs,
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("an unused declaration does not affect rendered bytes");

    assert_eq!(before.bytes(), after.bytes());
    assert_ne!(
        before.provenance().catalog_authority.authority_sha256,
        after.provenance().catalog_authority.authority_sha256,
    );
}

#[test]
fn catalog_authority_fingerprint_tracks_target_changes_without_changing_bytes() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"mode=stable\n");
    let app_before = app("key_value/v1", None, "");
    let mut app_after = app_before.clone();
    app_after.targets.get_mut("settings").unwrap().path = "files/other.cfg".into();
    let recipe_before = recipe(&app_before);
    let recipe_after = recipe(&app_after);

    let before = render(
        &workspace,
        &app_before,
        &recipe_before,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the original target should render");
    let after = render(
        &workspace,
        &app_after,
        &recipe_after,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the changed target remains valid");

    assert_eq!(before.bytes(), after.bytes());
    assert_ne!(
        before.provenance().catalog_authority.authority_sha256,
        after.provenance().catalog_authority.authority_sha256,
    );
}

#[test]
fn catalog_authority_fingerprint_tracks_package_identity_without_changing_bytes() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"mode=stable\n");
    let app_before = app("key_value/v1", None, "");
    let mut app_after = app_before.clone();
    app_after.package_id = "com.changed.app".into();
    let recipe_before = recipe(&app_before);
    let recipe_after = recipe(&app_after);
    let before = render(
        &workspace,
        &app_before,
        &recipe_before,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the original package identity should render");
    let after = render(
        &workspace,
        &app_after,
        &recipe_after,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the changed package identity should render");

    assert_eq!(before.bytes(), after.bytes());
    assert_ne!(
        before.provenance().catalog_authority.authority_sha256,
        after.provenance().catalog_authority.authority_sha256,
    );
}

#[test]
fn catalog_authority_fingerprint_tracks_exact_template_source_bytes() {
    let workspace = RenderWorkspace::new();
    let app = app("json/v1", None, "");
    let recipe = recipe(&app);
    workspace.write_catalog_template(b"{\"mode\":\"stable\"}");
    let before = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the compact JSON template should render");

    workspace.write_catalog_template(b" { \"mode\" : \"stable\" } \n");
    let after = render(
        &workspace,
        &app,
        &recipe,
        &[],
        None,
        &[],
        &OrderedMap::new(),
    )
    .expect("the reformatted JSON template should render");

    assert_eq!(before.bytes(), after.bytes());
    assert_ne!(
        before.provenance().catalog_authority.authority_sha256,
        after.provenance().catalog_authority.authority_sha256,
    );
}

#[test]
fn debug_redacts_artifact_sensitive_and_device_authority_fingerprints() {
    let workspace = RenderWorkspace::new();
    workspace.write_catalog_template(b"token={{emuchef:secret}}\ninstall_path={{emuchef:path}}\n");
    let app = app(
        "key_value/v1",
        Some("          secret: string\n          path: device_path\n"),
        "",
    );
    let mut recipe = recipe(&app);
    recipe
        .inputs
        .insert("secret_input".into(), input("string", true, true));
    recipe
        .inputs
        .insert("path_input".into(), input("device_path", true, false));
    recipe.config.as_mut().unwrap().variables = [
        ("secret".into(), "secret_input".into()),
        ("path".into(), "path_input".into()),
    ]
    .into_iter()
    .collect();
    let secret = "x7";
    let inputs = vec![
        binding(
            &recipe,
            "secret_input",
            "string",
            json!(secret),
            BindingSource::Explicit,
        ),
        binding(
            &recipe,
            "path_input",
            "device_path",
            json!("/data/local/tmp"),
            BindingSource::Explicit,
        ),
    ];
    let authority = RenderAuthority {
        device_handle: "trusted-device".into(),
        device_plan: "plan".into(),
        session_epoch: 4,
    };
    let rendered = render(
        &workspace,
        &app,
        &recipe,
        &inputs,
        Some(&authority),
        &[],
        &OrderedMap::new(),
    )
    .expect("valid sensitive and device-path bindings should render");
    let device_fingerprint = rendered
        .provenance()
        .device_authority
        .as_ref()
        .expect("device authority should be recorded")
        .authority_sha256
        .clone();
    let catalog_fingerprint = rendered
        .provenance()
        .catalog_authority
        .authority_sha256
        .clone();
    let debug = format!("{rendered:?}");
    let provenance_debug = format!("{:?}", rendered.provenance());

    assert!(std::str::from_utf8(rendered.bytes())
        .unwrap()
        .contains(secret));
    assert!(!debug.contains(secret));
    assert!(!debug.contains(rendered.sha256()));
    assert!(!debug.contains(&device_fingerprint));
    assert!(!debug.contains(&catalog_fingerprint));
    assert!(!provenance_debug.contains(&device_fingerprint));
    assert!(!provenance_debug.contains(&catalog_fingerprint));
}

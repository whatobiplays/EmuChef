use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::{
    jsonl,
    user_configuration::{
        classify_user_configuration_reference, emit_user_configuration_yaml,
        load_user_configuration, parse_user_configuration, resolve_user_configuration_path,
        validate_user_configuration_with_catalog, UserConfigurationReferenceKind,
    },
};
use serde_json::{json, Value};
use tempfile::TempDir;

fn valid_yaml() -> &'static str {
    "schema_version: 1\nkind: user_configuration\nid: test.default\nname: Test Default\ndevice_plan: test.plan\nselected_recipes:\n  - feature.test\nbindings:\n  feature.test/value:\n    value: saved\n"
}

fn write_configuration(temp: &TempDir, text: &str) -> PathBuf {
    let path = temp.path().join("configuration.yaml");
    fs::write(&path, text).expect("configuration should be written");
    path
}

fn write_authored_root(temp: &TempDir, sensitive: bool) -> PathBuf {
    let root = temp.path().join("authored");
    for directory in ["recipes", "device_plans", "device_profiles", "apps"] {
        fs::create_dir_all(root.join(directory)).expect("authored directory should be created");
    }
    fs::write(
        root.join("recipes/feature.test.yaml"),
        format!(
            "schema_version: 1\nkind: recipe\nid: feature.test\nname: Feature test\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n  value:\n    type: enum\n    required: true\n    sensitive: {sensitive}\n    options:\n      - value: saved\n      - value: explicit\n  destination:\n    type: device_path\n    required: true\n    validation:\n      allowed_prefixes: [/sdcard]\nartifacts: {{}}\nartifact_groups: {{}}\nsteps: []\n"
        ),
    )
    .expect("recipe should be written");
    fs::write(
        root.join("device_profiles/test.profile.yaml"),
        "schema_version: 1\nkind: device_profile\nid: test.profile\nname: Test profile\nmatch: {}\ncapability_defaults:\n  adb_available: true\n  apk_install: true\n  shared_storage_write: true\n  app_launch: true\n  shell_command: true\n  package_remove_for_user: false\n  root_shell: false\n  app_data_write: false\ndevice_tags: []\n",
    )
    .expect("device profile should be written");
    fs::write(
        root.join("device_plans/test.plan.yaml"),
        "schema_version: 1\nkind: device_plan\nid: test.plan\nname: Test plan\ndevice_profile_ref: test.profile\nrecipes:\n  - recipe_ref: feature.test\n    selected_by_default: true\ndefaults: {}\noverrides: {}\n",
    )
    .expect("device plan should be written");
    root
}

#[test]
fn structural_loading_requires_device_plan_and_strict_binding_entries() {
    let cases = [
        (
            valid_yaml().replace("device_plan: test.plan\n", ""),
            "device_plan",
        ),
        (
            valid_yaml().replace("device_plan: test.plan", "device_plan: null"),
            "device_plan",
        ),
        (
            valid_yaml().replace("device_plan: test.plan", "device_plan: ''"),
            "device_plan",
        ),
        (
            valid_yaml().replace("device_plan: test.plan", "device_plan: []"),
            "device_plan",
        ),
        (
            valid_yaml().replace("feature.test/value", "feature.test.value"),
            "Malformed qualified binding key",
        ),
        (
            valid_yaml().replace("    value: saved", "    local: saved"),
            "unsupported value source",
        ),
        (
            valid_yaml().replace("    value: saved", "    value: saved\n    secret: saved"),
            "exactly one value-source field",
        ),
        (
            valid_yaml().replace("    value: saved", "    metadata: saved"),
            "unknown field",
        ),
        (
            valid_yaml().replace("name: Test Default", "name: First\nname: Second"),
            "duplicate",
        ),
    ];

    for (text, expected) in cases {
        let error = parse_user_configuration(&text)
            .expect_err("structurally invalid configuration should fail");
        assert!(
            error
                .message
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase()),
            "expected {expected:?} in {:?}",
            error.message
        );
    }
}

#[test]
fn canonical_emission_preserves_extensions_without_overriding_known_fields() {
    let text = format!(
        "{}alpha_extension:\n  z: 2\n  a: 1\nzeta_extension: true\n",
        valid_yaml()
    );
    let mut configuration = parse_user_configuration(&text).expect("configuration should load");
    configuration
        .extensions
        .insert("name".to_string(), json!("extension replacement"));
    let emitted = emit_user_configuration_yaml(&configuration).expect("configuration should emit");

    assert!(emitted.contains("name: Test Default"));
    assert!(!emitted.contains("extension replacement"));
    assert!(
        emitted.find("alpha_extension:").unwrap() < emitted.find("zeta_extension:").unwrap(),
        "extension fields should be sorted"
    );
    assert!(
        emitted.find("feature.test/value:").unwrap() < emitted.find("alpha_extension:").unwrap(),
        "known fields should precede extensions"
    );
    let reparsed = parse_user_configuration(&emitted).expect("canonical YAML should reload");
    assert_eq!(reparsed.name, "Test Default");
    assert_eq!(
        reparsed.extensions["alpha_extension"],
        json!({"a": 1, "z": 2})
    );
}

#[test]
fn id_or_path_classification_is_syntax_only_and_rooted_ids_do_not_fallback() {
    assert_eq!(
        classify_user_configuration_reference("my.odin2.default").unwrap(),
        UserConfigurationReferenceKind::Identifier
    );
    for path in [
        "/tmp/missing",
        "nested/missing",
        r"nested\missing",
        "missing.YAML",
        "missing.yml",
    ] {
        assert_eq!(
            classify_user_configuration_reference(path).unwrap(),
            UserConfigurationReferenceKind::Path,
            "{path}"
        );
    }
    assert!(classify_user_configuration_reference("bad id").is_err());

    let root = Path::new("/tmp/configurations");
    assert_eq!(
        resolve_user_configuration_path(Some(root), "my.odin2.default").unwrap(),
        root.join("my.odin2.default.yaml")
    );
    assert_eq!(
        resolve_user_configuration_path(Some(root), "missing.yaml").unwrap(),
        PathBuf::from("missing.yaml")
    );
}

#[test]
fn catalog_semantics_are_diagnostics_and_sensitive_values_are_redacted() {
    let temp = TempDir::new().expect("temp root should be created");
    let authored_root = write_authored_root(&temp, true);
    let path = write_configuration(
        &temp,
        "schema_version: 1\nkind: user_configuration\nid: test.default\nname: Test Default\ndevice_plan: test.plan\nselected_recipes:\n  - feature.test\nbindings:\n  feature.test/value:\n    value: DO_NOT_LEAK\n  feature.test/destination:\n    value: /data/private\n  feature.test/missing:\n    value: ignored\n  other.recipe/value:\n    value: ignored\n",
    );
    let configuration = load_user_configuration(&path)
        .expect("semantic errors must not prevent structural loading");
    let emitted = emit_user_configuration_yaml(&configuration)
        .expect("semantic errors must not prevent canonical emission");
    assert!(emitted.contains("DO_NOT_LEAK"));

    let diagnostics =
        validate_user_configuration_with_catalog(&configuration, &path, &authored_root);
    let serialized = serde_json::to_string(&diagnostics).unwrap();
    assert!(!serialized.contains("DO_NOT_LEAK"));
    for code in [
        "invalid_enum_value",
        "invalid_path_prefix",
        "unknown_input",
        "unknown_recipe",
    ] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == code),
            "expected {code} in {diagnostics:#?}"
        );
    }

    let missing = parse_user_configuration(
        "schema_version: 1\nkind: user_configuration\nid: missing.default\nname: Missing\ndevice_plan: test.plan\nselected_recipes: [feature.test]\nbindings: {}\n",
    )
    .unwrap();
    let missing_diagnostics =
        validate_user_configuration_with_catalog(&missing, &path, &authored_root);
    assert!(missing_diagnostics
        .iter()
        .any(|diagnostic| diagnostic["code"] == "missing_required_input"));
}

#[test]
fn sidecar_user_configuration_document_remains_editable_with_semantic_errors() {
    let temp = TempDir::new().expect("temp root should be created");
    let authored_root = write_authored_root(&temp, false);
    let path = write_configuration(
        &temp,
        "schema_version: 1\nkind: user_configuration\nid: test.default\nname: Test Default\ndevice_plan: test.plan\nselected_recipes: [feature.test]\nbindings:\n  feature.test/value:\n    value: invalid\n",
    );
    let requests = [
        json!({
            "id": "open",
            "type": "openUserConfiguration",
            "payload": {
                "path": path,
                "authoredRoot": authored_root,
            }
        }),
        json!({
            "id": "set",
            "type": "setUserConfigurationBinding",
            "payload": {
                "documentId": "doc-1",
                "key": "feature.test/value",
                "value": "saved",
            }
        }),
        json!({
            "id": "clear-recipes",
            "type": "setUserConfigurationSelectedRecipes",
            "payload": {
                "documentId": "doc-1",
                "selectedRecipes": [],
            }
        }),
        json!({
            "id": "emit",
            "type": "emitUserConfigurationYaml",
            "payload": { "documentId": "doc-1" }
        }),
    ];
    let input = format!(
        "{}\n",
        requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let responses = jsonl::process_jsonl(&input)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["ok"], true);
    assert!(responses[0]["result"]["document"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "invalid_enum_value"));
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["configuration"]["bindings"]["feature.test/value"],
        "saved"
    );
    assert_eq!(responses[2]["ok"], true);
    assert_eq!(
        responses[2]["result"]["document"]["configuration"]["selectedRecipes"],
        json!([])
    );
    assert_eq!(responses[3]["ok"], true);
    assert!(responses[3]["result"]["yaml"]
        .as_str()
        .unwrap()
        .contains("selected_recipes: []"));
}

#[test]
fn create_with_bindings_and_save_as_identity_preserve_portable_intent() {
    let temp = TempDir::new().expect("temp root should be created");
    let authored_root = write_authored_root(&temp, false);
    let original = temp.path().join("original.yaml");
    let copy = temp.path().join("copy.yaml");
    let requests = [
        json!({
            "id": "create",
            "type": "createUserConfiguration",
            "payload": {
                "path": original,
                "configurationId": "saved.original",
                "name": "Original",
                "devicePlan": "test.plan",
                "selectedRecipes": ["feature.test"],
                "bindings": { "feature.test/value": "saved" },
                "authoredRoot": authored_root,
            }
        }),
        json!({
            "id": "save-as",
            "type": "saveUserConfigurationAs",
            "payload": {
                "documentId": "doc-1",
                "path": copy,
                "configurationId": "saved.copy",
                "name": "Copy",
            }
        }),
    ];
    let input = format!(
        "{}\n",
        requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let responses = jsonl::process_jsonl(&input)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(responses[0]["ok"], true, "{:#}", responses[0]);
    assert_eq!(
        responses[0]["result"]["document"]["configuration"]["bindings"]["feature.test/value"],
        "saved"
    );
    assert_eq!(responses[1]["ok"], true, "{:#}", responses[1]);
    assert_eq!(
        responses[1]["result"]["document"]["configuration"]["id"],
        "saved.copy"
    );
    assert_eq!(
        responses[1]["result"]["document"]["configuration"]["name"],
        "Copy"
    );
    assert!(fs::read_to_string(&original)
        .unwrap()
        .contains("id: saved.original"));
    let copy_text = fs::read_to_string(&copy).unwrap();
    assert!(copy_text.contains("id: saved.copy"));
    assert!(copy_text.contains("feature.test/value"));
}

#[test]
fn failed_save_as_keeps_the_current_document_identity_and_path() {
    let temp = TempDir::new().expect("temp root should be created");
    let path = write_configuration(&temp, valid_yaml());
    let missing = temp.path().join("missing").join("copy.yaml");
    let requests = [
        json!({
            "id": "open",
            "type": "openUserConfiguration",
            "payload": { "path": path }
        }),
        json!({
            "id": "save-as",
            "type": "saveUserConfigurationAs",
            "payload": {
                "documentId": "doc-1",
                "path": missing,
                "configurationId": "saved.failed-copy",
                "name": "Failed Copy",
            }
        }),
        json!({
            "id": "get",
            "type": "getUserConfigurationDocument",
            "payload": { "documentId": "doc-1" }
        }),
    ];
    let input = format!(
        "{}\n",
        requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let responses = jsonl::process_jsonl(&input)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(responses[1]["ok"], false);
    assert_eq!(
        responses[2]["result"]["document"]["configuration"]["id"],
        "test.default"
    );
    assert_eq!(
        responses[2]["result"]["document"]["configuration"]["name"],
        "Test Default"
    );
    assert_eq!(
        responses[2]["result"]["document"]["path"],
        responses[0]["result"]["document"]["path"]
    );
}

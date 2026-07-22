use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::{
    jsonl,
    user_configuration::{
        build_compatibility_baseline, classify_user_configuration_reference,
        compatibility_baseline_state, emit_user_configuration_yaml, load_user_configuration,
        parse_user_configuration, resolve_user_configuration_path,
        validate_user_configuration_with_catalog, CompatibilityBaselineState,
        UserConfigurationReferenceKind,
    },
    user_configuration_document::{document_to_dto, UserConfigurationDocument},
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
    assert_eq!(
        responses[1]["result"]["document"]["configuration"]["compatibility"],
        responses[0]["result"]["document"]["configuration"]["compatibility"],
        "persistence identity changes must not alter semantic fingerprints"
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

#[test]
fn save_as_collision_preserves_source_destination_and_live_identity() {
    let temp = TempDir::new().expect("temp root should be created");
    let source = write_configuration(&temp, valid_yaml());
    let source_before = fs::read(&source).unwrap();
    let destination = temp.path().join("existing.yaml");
    fs::write(&destination, b"existing destination\n").unwrap();
    let mut document = UserConfigurationDocument::open(&source, None).unwrap();

    document
        .save_as_with_identity(&destination, Some("saved.copy"), Some("Copy"))
        .expect_err("an existing destination must not be overwritten");

    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(fs::read(&destination).unwrap(), b"existing destination\n");
    assert_eq!(document.path(), source.canonicalize().unwrap());
    assert_eq!(document.configuration().id, "test.default");
    assert_eq!(document.configuration().name, "Test Default");
}

#[test]
fn v1_open_has_no_historical_baseline_and_first_explicit_save_establishes_v2() {
    let temp = TempDir::new().expect("temp root should be created");
    let authored_root = write_authored_root(&temp, false);
    let path = write_configuration(&temp, valid_yaml());
    let original = fs::read(&path).unwrap();
    let mut document =
        UserConfigurationDocument::open(&path, Some(authored_root.to_string_lossy().as_ref()))
            .unwrap();
    let opened = document_to_dto(&document, "doc-1");
    assert_eq!(
        opened["compatibilityStatus"]["baselineState"],
        "pending_first_v2_save"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "inspection must not rewrite V1"
    );

    document.save().unwrap();
    let saved = document_to_dto(&document, "doc-1");
    assert_eq!(saved["compatibilityStatus"]["baselineState"], "unchanged");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("schema_version: 2\n"));
    assert!(text.contains("compatibility:"));
    for prohibited in [
        "plan_digest",
        "review_handle",
        "execution_handle",
        "device_serial",
    ] {
        assert!(!text.contains(prohibited));
    }
}

#[test]
fn authored_contract_fingerprints_ignore_labels_and_resolved_values() {
    let temp = TempDir::new().expect("temp root should be created");
    let authored_root = write_authored_root(&temp, false);
    let path = write_configuration(&temp, valid_yaml());
    let mut first_configuration = load_user_configuration(&path).unwrap();
    let first = build_compatibility_baseline(&first_configuration, &path, &authored_root).unwrap();

    first_configuration
        .bindings
        .insert("feature.test/value".to_string(), json!("explicit"));
    fs::write(
        authored_root.join("recipes/feature.test.yaml"),
        fs::read_to_string(authored_root.join("recipes/feature.test.yaml"))
            .unwrap()
            .replace("name: Feature test", "name: Renamed presentation only"),
    )
    .unwrap();
    fs::write(
        authored_root.join("device_plans/test.plan.yaml"),
        fs::read_to_string(authored_root.join("device_plans/test.plan.yaml"))
            .unwrap()
            .replace("name: Test plan", "name: New display label"),
    )
    .unwrap();
    let second = build_compatibility_baseline(&first_configuration, &path, &authored_root).unwrap();

    assert_eq!(
        first.device_plan.fingerprint,
        second.device_plan.fingerprint
    );
    assert_eq!(first.recipes[0].fingerprint, second.recipes[0].fingerprint);
    assert_eq!(
        first.recipes[0].inputs[0].fingerprint,
        second.recipes[0].inputs[0].fingerprint
    );
    first_configuration.compatibility = Some(first);
    assert_eq!(
        compatibility_baseline_state(&first_configuration, &second),
        CompatibilityBaselineState::Unchanged
    );
}

#[test]
fn v2_additive_extension_policy_preserves_namespaced_fields_and_reports_unsupported_fields() {
    let fingerprint = "a".repeat(64);
    let text = format!(
        "schema_version: 2\nkind: user_configuration\nid: test.default\nname: Test\ndevice_plan: test.plan\nselected_recipes: []\nbindings: {{}}\ncompatibility:\n  device_plan:\n    id: test.plan\n    label: Test plan\n    fingerprint: {fingerprint}\n  recipes: []\nx-vendor-note: safe additive metadata\nlegacy_extra: pending sanitation\n"
    );
    let parsed = parse_user_configuration(&text).unwrap();
    assert_eq!(parsed.extensions["x-vendor-note"], "safe additive metadata");
    assert_eq!(parsed.extensions["legacy_extra"], "pending sanitation");
    assert_eq!(parsed.unsupported_extensions, vec!["legacy_extra"]);

    let prohibited = text.replace(
        "legacy_extra: pending sanitation",
        "execution_handle: forbidden",
    );
    assert!(parse_user_configuration(&prohibited)
        .unwrap_err()
        .message
        .contains("prohibited authority field"));
}

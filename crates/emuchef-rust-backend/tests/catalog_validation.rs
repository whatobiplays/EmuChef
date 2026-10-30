use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::{protocol, request, run_with_args_and_input};
use serde_json::{json, Value};

mod common;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
}

fn backend_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn plain_recipe_path(name: &str) -> PathBuf {
    backend_fixture_root().join("recipes").join(name)
}

fn workspace_root(name: &str) -> PathBuf {
    fixture_root().join(name)
}

fn authored_root(name: &str) -> PathBuf {
    workspace_root(name).join("authored")
}

fn recipe_path(workspace: &str, name: &str) -> PathBuf {
    authored_root(workspace).join("recipes").join(name)
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("compatibility_goldens_v1")
        .join(name)
}

fn read_diagnostic_golden(name: &str) -> Vec<Value> {
    let text = fs::read_to_string(golden_path(name))
        .expect("Compatibility diagnostic fixture should exist");
    serde_json::from_str(&text).expect("Compatibility diagnostic fixture should be valid JSON")
}

fn parse_stdout_json(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly one JSON response line");
    serde_json::from_str(lines[0]).expect("response should be valid JSON")
}

fn one_shot_response(request: Value) -> Value {
    let output = run_with_args_and_input(&[request.to_string()], "");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    parse_stdout_json(&output.stdout)
}

fn sidecar_responses(requests: Vec<Value>) -> Vec<Value> {
    let mut sessions = common::test_sessions();
    requests
        .into_iter()
        .map(|request| request::handle_sidecar_value(request, &mut sessions))
        .collect()
}

struct TempWorkspace {
    dir: PathBuf,
}

impl TempWorkspace {
    fn copy_fixture(name: &str) -> Self {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emuchef-rust-backend-phase6l-{}-{unique}-{sequence}",
            std::process::id()
        ));
        copy_dir_all(&workspace_root(name), &dir);
        Self { dir }
    }

    fn authored_root(&self) -> PathBuf {
        self.dir.join("authored")
    }

    fn recipe_path(&self, name: &str) -> PathBuf {
        self.authored_root().join("recipes").join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn copy_dir_all(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("destination directory should be created");
    for entry in fs::read_dir(source).expect("source directory should be readable") {
        let entry = entry.expect("source entry should be readable");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("fixture file should copy");
        }
    }
}

fn validate_path(path: &Path, authored_root: Option<&Path>) -> Value {
    let mut payload = json!({ "path": path });
    if let Some(root) = authored_root {
        payload["authoredRoot"] = json!(root);
    }
    one_shot_response(json!({
        "type": "validateRecipePath",
        "payload": payload
    }))
}

fn validate_path_with_null_root(path: &Path) -> Value {
    one_shot_response(json!({
        "type": "validateRecipePath",
        "payload": {"path": path, "authoredRoot": null}
    }))
}

fn diagnostic_fields(diagnostic: &Value) -> Value {
    json!({
        "severity": diagnostic["severity"],
        "code": diagnostic["code"],
        "objectKind": diagnostic["objectKind"],
        "objectId": diagnostic["objectId"],
        "field": diagnostic["field"],
    })
}

fn diagnostic_set(response: &Value) -> Vec<Value> {
    response["result"]["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .map(diagnostic_fields)
        .collect()
}

fn document_diagnostic_set(response: &Value) -> Vec<Value> {
    response["result"]["document"]["diagnostics"]
        .as_array()
        .expect("document diagnostics should be an array")
        .iter()
        .map(diagnostic_fields)
        .collect()
}

fn normalized_path(path: &Path) -> Value {
    json!(path.canonicalize().unwrap().to_string_lossy().to_string())
}

#[test]
fn capabilities_include_catalog_context_session_update() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
            "listAdbDevices",
            "probeDevice",
            "qualifyDevice",
            "checkRoot",
            "inspectApk",
            "generateAppRecipeDraft",
            "generateRemoteAppRecipeDraft",
            "generateDeviceProfileDraft",
            "checkGeneratedCatalogCollisions",
            "listAppGeneratorExistingApps",
            "selectAppGeneratorExistingApp",
            "reviewAppGeneratorDraft",
            "reviewRemoteAppGeneratorDraft",
            "saveAppGeneratorReview",
            "appGeneratorPublicationStatus",
            "matchDevice",
            "negotiateCapabilities",
            "openUserConfiguration",
            "createUserConfiguration",
            "getUserConfigurationDocument",
            "saveUserConfiguration",
            "saveUserConfigurationAs",
            "setUserConfigurationBinding",
            "removeUserConfigurationBinding",
            "setUserConfigurationSelectedRecipes",
            "setUserConfigurationDevicePlan",
            "validateUserConfiguration",
            "emitUserConfigurationYaml",
            "setUserConfigurationAuthoredRoot",
            "closeUserConfiguration",
            "describeConfiguration",
            "planConfiguration",
            "prepareExecutionReview",
            "startExecution",
            "getExecution",
            "getExecutionEvents",
            "cancelExecution",
            "launchExecutionApp",
            "openRecipe",
            "openAppDefinition",
            "createRecipeFromTemplate",
            "getDocument",
            "saveRecipe",
            "saveRecipeAs",
            "saveAppDefinition",
            "saveAppDefinitionAs",
            "saveDocument",
            "saveDocumentAs",
            "closeDocument",
            "applyRecipeCommand",
            "applyAppDefinitionCommand",
            "previewCatalogChange",
            "applyCatalogChange",
            "catalogChangeStatus",
            "listCatalogChangeRecoveries",
            "retryCatalogChangeRecovery",
            "undo",
            "redo",
            "emitYaml",
            "validate",
            "getRefIndex",
            "setDocumentAuthoredRoot",
            "ping",
        ]
    );
}

#[test]
fn recipe_admission_ignores_unrelated_malformed_catalog_entries_but_root_validation_reports_them() {
    let workspace = TempWorkspace::copy_fixture("nested_ignored");
    let authored_root = workspace.authored_root();
    let recipe = workspace.recipe_path("main.yaml");
    fs::copy(plain_recipe_path("minimal_recipe.yaml"), &recipe)
        .expect("generic valid Recipe fixture should be copied");
    let unrelated_app = authored_root.join("apps/unrelated.yaml");
    let unrelated_profile = authored_root.join("device_profiles/unrelated.yaml");
    fs::create_dir_all(unrelated_app.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated_profile.parent().unwrap()).unwrap();
    fs::write(&unrelated_app, "schema_version: [\n").unwrap();
    fs::write(&unrelated_profile, "schema_version: [\n").unwrap();

    let recipe_diagnostics = diagnostic_set(&validate_path(&recipe, Some(&authored_root)));
    assert!(
        !recipe_diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error"),
        "unrelated App Definition and Device Profile errors must not block a generic Recipe: {recipe_diagnostics:?}"
    );

    let (_, root_diagnostics) =
        emuchef_rust_backend::catalog::validate_authored_root(&authored_root);
    let root_diagnostics = serde_json::to_string(&root_diagnostics).unwrap();
    assert!(root_diagnostics.contains("apps/unrelated.yaml"));
    assert!(root_diagnostics.contains("device_profiles/unrelated.yaml"));
}

#[test]
fn recipe_admission_still_rejects_an_invalid_referenced_app_definition() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let app_definition = authored_root.join("apps/demo_app.yaml");
    let contents = fs::read_to_string(&app_definition).unwrap();
    fs::write(
        app_definition,
        contents.replace(
            "package_id: com.example.demo",
            "package_id: invalid package",
        ),
    )
    .unwrap();

    let diagnostics = diagnostic_set(&validate_path(
        &workspace.recipe_path("demo.app.install.yaml"),
        Some(&authored_root),
    ));

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["severity"] == "error" && diagnostic["code"] == "app_definition_invalid"
    }));
}

#[test]
fn recipe_admission_rejects_duplicate_reachable_dependency_id() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let recipes_dir = authored_root.join("recipes");
    let minimal = fs::read_to_string(plain_recipe_path("minimal_recipe.yaml"))
        .expect("minimal Recipe fixture should be readable");

    let parent = minimal
        .replace("phase6e.minimal", "dependency.parent")
        .replace(
            "recipe_dependencies: []",
            "recipe_dependencies:\n  - dependency.duplicate",
        );
    let parent_path = recipes_dir.join("dependency_parent.yaml");
    fs::write(&parent_path, parent).expect("parent Recipe should be written");

    let valid_dependency = minimal.replace("phase6e.minimal", "dependency.duplicate");
    fs::write(
        recipes_dir.join("a_dependency_duplicate.yaml"),
        valid_dependency,
    )
    .expect("valid dependency Recipe should be written");

    let invalid_dependency =
        fs::read_to_string(plain_recipe_path("phase6k_unknown_input_ref.yaml"))
            .expect("invalid Recipe fixture should be readable")
            .replace("phase6k.unknown_input_ref", "dependency.duplicate");
    fs::write(
        recipes_dir.join("b_dependency_duplicate.yaml"),
        invalid_dependency,
    )
    .expect("invalid duplicate dependency Recipe should be written");

    let diagnostics = diagnostic_set(&validate_path(&parent_path, Some(&authored_root)));
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "recipe_id_conflict"
                && diagnostic["objectId"] == "dependency.duplicate"
        }),
        "a duplicate reachable dependency id must fail admission: {diagnostics:?}"
    );
}

#[test]
fn recipe_admission_rejects_duplicate_reachable_dependency_id_when_one_file_fails_schema_validation(
) {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let recipes_dir = authored_root.join("recipes");
    let minimal = fs::read_to_string(plain_recipe_path("minimal_recipe.yaml"))
        .expect("minimal Recipe fixture should be readable");
    let parent = minimal
        .replace("phase6e.minimal", "dependency.parent")
        .replace(
            "recipe_dependencies: []",
            "recipe_dependencies:\n  - dependency.duplicate",
        );
    let parent_path = recipes_dir.join("dependency_parent_invalid_schema.yaml");
    fs::write(&parent_path, parent).expect("parent Recipe should be written");

    let valid_dependency = minimal.replace("phase6e.minimal", "dependency.duplicate");
    fs::write(
        recipes_dir.join("a_dependency_duplicate_valid.yaml"),
        valid_dependency,
    )
    .expect("valid dependency Recipe should be written");

    let invalid_schema_dependency = minimal
        .replace("phase6e.minimal", "dependency.duplicate")
        .replacen("schema_version: 1", "schema_version: 2", 1);
    fs::write(
        recipes_dir.join("b_dependency_duplicate_invalid_schema.yaml"),
        invalid_schema_dependency,
    )
    .expect("schema-invalid dependency Recipe should be written");

    let diagnostics = diagnostic_set(&validate_path(&parent_path, Some(&authored_root)));
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "recipe_id_conflict"
                && diagnostic["objectId"] == "dependency.duplicate"
        }),
        "duplicate reachable dependency id must fail admission even when one file fails schema validation: {diagnostics:?}"
    );
}

#[test]
fn recipe_admission_deduplicates_relevant_app_definition_diagnostics() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let recipes_dir = authored_root.join("recipes");
    let app_contents = fs::read_to_string(authored_root.join("apps/demo_app.yaml"))
        .expect("App Definition fixture should be readable")
        .replace(
            "package_id: com.example.demo",
            "package_id: com.example.demo.duplicate",
        );
    fs::write(
        authored_root.join("apps/duplicate_demo_app.yaml"),
        app_contents,
    )
    .expect("duplicate App Definition should be written");

    let minimal = fs::read_to_string(plain_recipe_path("minimal_recipe.yaml"))
        .expect("minimal Recipe fixture should be readable");
    let parent = minimal
        .replace("phase6e.minimal", "dependency.app_parent")
        .replace(
            "recipe_dependencies: []",
            "recipe_dependencies:\n  - demo.app.install",
        )
        .replace(
            "artifacts: {}",
            "artifacts:\n  parent_apk:\n    type: app_artifact\n    app_ref: demo_app\n    artifact: apk",
        );
    let parent_path = recipes_dir.join("app_parent.yaml");
    fs::write(&parent_path, parent).expect("parent Recipe should be written");

    let diagnostics = diagnostic_set(&validate_path(&parent_path, Some(&authored_root)));
    let app_definition_conflicts = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic["severity"] == "error" && diagnostic["code"] == "app_definition_id_conflict"
        })
        .count();
    assert_eq!(
        app_definition_conflicts, 1,
        "the same relevant App Definition diagnostic must appear once: {diagnostics:?}"
    );
}

#[test]
fn recipe_admission_reports_package_installed_parameter_violation_once() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let recipe_path = workspace.recipe_path("demo.app.install.yaml");
    let contents = fs::read_to_string(&recipe_path)
        .expect("App-aware Recipe fixture should be readable")
        .replace(
            "      - type: package_installed\n        params: {}\n",
            "      - type: package_installed\n        params:\n          package: com.example.demo\n",
        );
    fs::write(&recipe_path, contents).expect("App-aware Recipe fixture should be written");

    let diagnostics = diagnostic_set(&validate_path(&recipe_path, Some(&authored_root)));
    let violations = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "app_condition_package_parameter_forbidden"
        })
        .count();
    assert_eq!(
        violations, 1,
        "a retired package condition parameter must be reported once: {diagnostics:?}"
    );
}

#[test]
fn standalone_validation_rejects_package_installed_params_on_app_context_step() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let recipe_path = workspace.recipe_path("demo.app.install.yaml");
    let contents = fs::read_to_string(&recipe_path)
        .expect("App-aware Recipe fixture should be readable")
        .replace(
            "      - type: package_installed\n        params: {}\n",
            "      - type: package_installed\n        params:\n          package: com.example.demo\n",
        );
    fs::write(&recipe_path, contents).expect("App-aware Recipe fixture should be written");

    let diagnostics = diagnostic_set(&validate_path_with_null_root(&recipe_path));
    let violations = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "app_condition_package_parameter_forbidden"
        })
        .count();
    assert_eq!(
        violations, 1,
        "standalone validation must still reject retired package condition params: {diagnostics:?}"
    );
}

#[test]
fn unreadable_recipe_catalog_fails_admission_and_root_validation() {
    let workspace = TempWorkspace::copy_fixture("nested_ignored");
    let authored_root = workspace.authored_root();
    let recipes_path = authored_root.join("recipes");
    fs::remove_dir_all(&recipes_path).expect("Recipe fixture directory should be removable");
    fs::write(&recipes_path, "not a directory")
        .expect("Recipe catalog obstruction should be written");

    let candidate = workspace.dir.join("candidate.yaml");
    fs::copy(plain_recipe_path("minimal_recipe.yaml"), &candidate)
        .expect("candidate Recipe should be copied");
    let admission_diagnostics = diagnostic_set(&validate_path(&candidate, Some(&authored_root)));
    assert!(
        admission_diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "recipe_catalog_unreadable"
        }),
        "Recipe admission must fail closed when the Recipe catalog cannot be read: {admission_diagnostics:?}"
    );

    let (_, root_diagnostics) =
        emuchef_rust_backend::catalog::validate_authored_root(&authored_root);
    assert!(
        root_diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error" && diagnostic["code"] == "recipe_catalog_unreadable"
        }),
        "authored-root validation must report an unreadable Recipe catalog: {root_diagnostics:?}"
    );
}

#[cfg(unix)]
#[test]
fn authored_root_rejects_a_dangling_device_plan_yaml_symlink() {
    let workspace = TempWorkspace::copy_fixture("nested_ignored");
    let authored_root = workspace.authored_root();
    let device_plans = authored_root.join("device_plans");
    fs::create_dir_all(&device_plans).expect("create Device Plan directory");
    std::os::unix::fs::symlink("missing-target.yaml", device_plans.join("dangling.yaml"))
        .expect("create dangling Device Plan symlink");

    let (validated_paths, diagnostics) =
        emuchef_rust_backend::catalog::validate_authored_root(&authored_root);

    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "device_plan_catalog_unreadable"
        }),
        "a dangling Device Plan symlink must fail authored-root validation: {diagnostics:?}"
    );
    assert!(
        !validated_paths
            .iter()
            .any(|path| path.ends_with("dangling.yaml")),
        "an unreadable Device Plan must not be reported as validated: {validated_paths:?}"
    );
}

#[cfg(unix)]
#[test]
fn dangling_catalog_directory_symlinks_fail_both_catalog_validations() {
    for directory_name in ["recipes", "apps", "device_profiles", "device_plans"] {
        let workspace = TempWorkspace::copy_fixture("nested_ignored");
        let authored_root = workspace.authored_root();
        let directory = authored_root.join(directory_name);
        if directory.exists() {
            fs::remove_dir_all(&directory).expect("remove optional catalog directory");
        }
        std::os::unix::fs::symlink(
            authored_root.join(format!("missing-{directory_name}")),
            &directory,
        )
        .expect("create dangling catalog directory symlink");

        let (_, root_diagnostics) =
            emuchef_rust_backend::catalog::validate_authored_root(&authored_root);
        assert!(
            root_diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && diagnostic["code"]
                        .as_str()
                        .is_some_and(|code| code.ends_with("_catalog_unreadable"))
            }),
            "authored-root validation must reject a dangling {directory_name} symlink: {root_diagnostics:?}"
        );

        let model_diagnostics =
            emuchef_rust_backend::catalog::validate_authored_catalog_models(&authored_root);
        assert!(
            model_diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && diagnostic["code"]
                        .as_str()
                        .is_some_and(|code| code.ends_with("_catalog_unreadable"))
            }),
            "catalog model validation must reject a dangling {directory_name} symlink: {model_diagnostics:?}"
        );
    }
}

#[test]
fn authored_catalog_models_rejects_an_unreadable_recipe_catalog_for_a_zero_recipe_plan() {
    let workspace = TempWorkspace::copy_fixture("nested_ignored");
    let authored_root = workspace.authored_root();
    fs::create_dir_all(authored_root.join("device_profiles"))
        .expect("create Device Profile directory");
    fs::create_dir_all(authored_root.join("device_plans")).expect("create Device Plan directory");
    fs::write(
        authored_root.join("device_profiles/ayaneo.generic.yaml"),
        include_str!("../../../authored/device_profiles/ayaneo.generic.yaml"),
    )
    .expect("write valid Device Profile");
    fs::write(
        authored_root.join("device_plans/no-recipes.yaml"),
        r#"schema_version: 1
kind: device_plan
id: test.no_recipes
name: No Recipes
description: Zero-recipe plan used to validate catalog readability.
device_profile_ref: ayaneo.generic
recipes: []
"#,
    )
    .expect("write valid zero-recipe Device Plan");

    let recipes = authored_root.join("recipes");
    fs::remove_dir_all(&recipes).expect("remove Recipe directory");
    fs::write(&recipes, "not a directory").expect("obstruct Recipe directory");

    let diagnostics =
        emuchef_rust_backend::catalog::validate_authored_catalog_models(&authored_root);

    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["severity"] == "error"
                && diagnostic["code"] == "recipe_catalog_unreadable"
        }),
        "public catalog model validation must not accept an unreadable Recipe catalog: {diagnostics:?}"
    );
}

#[test]
fn recipe_admission_validates_sibling_dependencies_in_authored_order() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let authored_root = workspace.authored_root();
    let recipes_dir = authored_root.join("recipes");
    let minimal = fs::read_to_string(plain_recipe_path("minimal_recipe.yaml"))
        .expect("minimal Recipe fixture should be readable");

    let parent = minimal
        .replace("phase6e.minimal", "dependency.parent")
        .replace(
            "recipe_dependencies: []",
            "recipe_dependencies:\n  - dependency.first\n  - dependency.second",
        );
    let parent_path = recipes_dir.join("dependency_order_parent.yaml");
    fs::write(&parent_path, parent).expect("parent Recipe should be written");

    for (file, id, missing_id) in [
        (
            "a_dependency_first.yaml",
            "dependency.first",
            "missing.first",
        ),
        (
            "b_dependency_second.yaml",
            "dependency.second",
            "missing.second",
        ),
    ] {
        let dependency = minimal.replace("phase6e.minimal", id).replace(
            "recipe_dependencies: []",
            &format!("recipe_dependencies:\n  - {missing_id}"),
        );
        fs::write(recipes_dir.join(file), dependency).expect("dependency Recipe should be written");
    }

    let diagnostics = diagnostic_set(&validate_path(&parent_path, Some(&authored_root)));
    let observed = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "recipe_not_found")
        .filter_map(|diagnostic| diagnostic["objectId"].as_str())
        .filter(|object_id| object_id.starts_with("dependency."))
        .collect::<Vec<_>>();
    assert_eq!(
        observed,
        vec!["dependency.first", "dependency.second"],
        "siblings must validate in authored dependency order: {diagnostics:?}"
    );
}

#[test]
fn validate_recipe_path_uses_explicit_authored_root_but_does_not_infer_it() {
    let path = recipe_path("complete", "main.yaml");
    let inferred = validate_path(&path, None);
    assert_eq!(inferred["ok"], true);
    let inferred_diagnostics = diagnostic_set(&inferred);
    assert_eq!(
        inferred_diagnostics,
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );

    let null_root = validate_path_with_null_root(&path);
    assert_eq!(null_root["ok"], true);
    let null_diagnostics = diagnostic_set(&null_root);
    assert_eq!(
        null_diagnostics,
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );

    let explicit = validate_path(&path, Some(&authored_root("complete")));
    assert_eq!(explicit["ok"], true);
    assert_eq!(
        diagnostic_set(&explicit),
        read_diagnostic_golden("phase6l_complete_explicit_root.diagnostics.json")
    );
}

#[test]
fn nonexistent_non_null_authored_root_is_empty_catalog_context_not_request_failure() {
    let path = recipe_path("complete", "main.yaml");
    let missing_root = workspace_root("complete").join("missing-authored-root");
    let response = validate_path(&path, Some(&missing_root));

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_missing_authored_root.diagnostics.json")
    );
}

#[test]
fn valid_authored_root_reports_recipe_dependency_diagnostics() {
    let response = validate_path(
        &recipe_path("missing_dependency", "missing_dependency.yaml"),
        Some(&authored_root("missing_dependency")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_missing_dependency.diagnostics.json")
    );
}

#[test]
fn recipe_dependency_cycles_use_validation_local_graph_checks() {
    let response = validate_path(
        &recipe_path("dependency_cycle", "cycle_a.yaml"),
        Some(&authored_root("dependency_cycle")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_dependency_cycle.diagnostics.json")
    );
}

#[test]
fn catalog_scan_uses_only_compatibility_verified_top_level_globs() {
    // Catalog validation scans only these top-level authored globs:
    // apps/*.y*ml, recipes/*.y*ml, device_profiles/*.y*ml, device_plans/*.y*ml.
    // This nested recipe is intentionally ignored, so the dependency is missing.
    let response = validate_path(
        &recipe_path("nested_ignored", "main.yaml"),
        Some(&authored_root("nested_ignored")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_nested_ignored.diagnostics.json")
    );
}

#[test]
fn open_recipe_infers_and_normalizes_authored_root_like_compatibility() {
    let inferred_path = recipe_path("complete", "main.yaml");
    let repo_root = workspace_root("complete");
    let responses = sidecar_responses(vec![
        json!({
            "id": "open-inferred",
            "type": "openRecipe",
            "payload": {"path": inferred_path}
        }),
        json!({
            "id": "open-null-inferred",
            "type": "openRecipe",
            "payload": {"path": inferred_path, "authoredRoot": null}
        }),
        json!({
            "id": "open-normalized",
            "type": "openRecipe",
            "payload": {"path": inferred_path, "authoredRoot": repo_root}
        }),
    ]);

    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["document"]["authoredRoot"],
            authored_root("complete")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            document_diagnostic_set(response),
            read_diagnostic_golden("phase6l_complete_explicit_root.diagnostics.json")
        );
    }
}

#[test]
fn set_document_authored_root_updates_null_session_context_without_reopening() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": plain_recipe_path("minimal_recipe.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": backend_fixture_root()}
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        responses[0]["result"]["document"]["authoredRoot"],
        Value::Null
    );
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        normalized_path(&backend_fixture_root())
    );
    assert_eq!(document_diagnostic_set(&responses[1]), Vec::<Value>::new());
    assert_eq!(
        responses[1]["result"]["document"]["refIndex"]["allRefs"],
        json!([])
    );
}

#[test]
fn set_document_authored_root_switches_catalog_context() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": recipe_path("complete", "main.yaml"),
                "authoredRoot": authored_root("complete")
            }
        }),
        json!({
            "id": "switch-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {
                "documentId": "doc-1",
                "authoredRoot": authored_root("missing_dependency")
            }
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        normalized_path(&authored_root("missing_dependency"))
    );
    assert_eq!(
        document_diagnostic_set(&responses[1]),
        vec![json!({
            "severity": "error",
            "code": "recipe_not_found",
            "objectKind": "recipe",
            "objectId": "phase6l.main",
            "field": "recipe_dependencies[0]",
        })]
    );
}

#[test]
fn set_document_authored_root_null_clears_context_without_inference() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": recipe_path("complete", "main.yaml"),
                "authoredRoot": authored_root("complete")
            }
        }),
        json!({
            "id": "clear-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": null}
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        Value::Null
    );
    assert_eq!(
        document_diagnostic_set(&responses[1]),
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );
}

#[test]
fn set_document_authored_root_preserves_clean_dirty_and_history_state() {
    let clean_responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path("complete", "main.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": authored_root("complete")}
        }),
    ]);
    let clean_document = &clean_responses[1]["result"]["document"];
    assert_eq!(clean_document["dirty"], false);
    assert_eq!(clean_document["canUndo"], false);
    assert_eq!(clean_document["canRedo"], false);

    let dirty_responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path("complete", "main.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Unsaved Name"}
            }
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": authored_root("complete")}
        }),
    ]);
    let dirty_document = &dirty_responses[2]["result"]["document"];
    assert_eq!(dirty_document["dirty"], true);
    assert_eq!(dirty_document["canUndo"], true);
    assert_eq!(dirty_document["canRedo"], false);
    assert_eq!(dirty_document["recipe"]["name"], "Unsaved Name");
}

#[test]
fn set_document_authored_root_preserves_in_memory_content_when_disk_changes() {
    let workspace = TempWorkspace::copy_fixture("complete");
    let recipe_path = workspace.recipe_path("main.yaml");
    let original_disk_yaml =
        fs::read_to_string(&recipe_path).expect("copied recipe should be readable");
    let mut sessions = common::test_sessions();

    let opened = request::handle_sidecar_value(
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path, "authoredRoot": null}
        }),
        &mut sessions,
    );
    assert_eq!(opened["ok"], true);

    let edited = request::handle_sidecar_value(
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "AddInput", "inputId": "unsaved_input"}
            }
        }),
        &mut sessions,
    );
    assert_eq!(edited["ok"], true);

    fs::write(
        &recipe_path,
        original_disk_yaml.replace("Phase 6L Main", "Disk Reloaded Name"),
    )
    .expect("backing file should be mutable");

    let response = request::handle_sidecar_value(
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": workspace.authored_root()}
        }),
        &mut sessions,
    );

    assert_eq!(response["ok"], true);
    let document = &response["result"]["document"];
    assert_eq!(document["dirty"], true);
    assert_eq!(document["recipe"]["name"], "Phase 6L Main");
    assert!(document["yaml"]
        .as_str()
        .unwrap()
        .contains("unsaved_input:"));
    assert!(!document["yaml"]
        .as_str()
        .unwrap()
        .contains("Disk Reloaded Name"));
    assert!(document["refIndex"]["inputRefs"]
        .as_array()
        .unwrap()
        .contains(&json!("inputs.unsaved_input")));
}

#[test]
fn open_recipe_reports_duplicate_recipe_id_conflict_against_catalog() {
    let responses = sidecar_responses(vec![json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {
            "path": recipe_path("duplicate", "target_duplicate.yaml"),
            "authoredRoot": authored_root("duplicate")
        }
    })]);

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        document_diagnostic_set(&responses[0]),
        read_diagnostic_golden("phase6l_duplicate_open.diagnostics.json")
    );
}

#[test]
fn duplicate_recipe_id_conflict_matches_compatibility_first_file_replacement_semantics() {
    let responses = sidecar_responses(vec![json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {
            "path": recipe_path("duplicate_reverse", "a_target_duplicate.yaml"),
            "authoredRoot": authored_root("duplicate_reverse")
        }
    })]);

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        document_diagnostic_set(&responses[0]),
        read_diagnostic_golden("phase6l_duplicate_reverse_open.diagnostics.json")
    );
}

#[test]
fn commands_undo_redo_save_and_session_validate_reuse_stored_authored_root() {
    let workspace = TempWorkspace::copy_fixture("missing_dependency");
    let path = workspace.recipe_path("main.yaml");
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": path}
        }),
        json!({
            "id": "command",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Renamed Main"}
            }
        }),
        json!({
            "id": "validate-after-command",
            "type": "validate",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "undo",
            "type": "undo",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "redo",
            "type": "redo",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "save",
            "type": "saveRecipe",
            "payload": {"documentId": "doc-1"}
        }),
    ]);

    assert_eq!(responses.len(), 6);
    let unrelated_recipe = validate_path(
        &workspace.recipe_path("missing_dependency.yaml"),
        Some(&workspace.authored_root()),
    );
    assert_eq!(unrelated_recipe["ok"], true);
    assert_eq!(
        diagnostic_set(&unrelated_recipe),
        read_diagnostic_golden("phase6l_missing_dependency_open.diagnostics.json")
    );
    for response in &responses {
        assert_eq!(response["ok"], true, "{response:#?}");
        let diagnostics = if response["result"].get("document").is_some() {
            document_diagnostic_set(response)
        } else {
            diagnostic_set(response)
        };
        assert!(diagnostics.is_empty(), "{response:#?}");
    }

    let expected_root = workspace.authored_root().canonicalize().unwrap();
    assert_eq!(
        responses[0]["result"]["document"]["authoredRoot"],
        expected_root.to_string_lossy().to_string()
    );
    for response in [&responses[1], &responses[3], &responses[4], &responses[5]] {
        assert_eq!(
            response["result"]["document"]["authoredRoot"],
            expected_root.to_string_lossy().to_string()
        );
    }
}

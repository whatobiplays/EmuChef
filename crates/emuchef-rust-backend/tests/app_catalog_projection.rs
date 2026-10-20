use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::{request, session::DocumentSessionManager};
use serde_json::{json, Value};

mod common;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
}

fn workspace_root(name: &str) -> PathBuf {
    fixture_root().join(name)
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
            "emuchef-rust-backend-app-catalog-{}-{unique}-{sequence}",
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

    fn app_path(&self, name: &str) -> PathBuf {
        self.authored_root().join("apps").join(name)
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
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("file type should be readable")
            .is_dir()
        {
            copy_dir_all(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("fixture file should copy");
        }
    }
}

fn open_recipe(
    sessions: &mut DocumentSessionManager,
    path: &Path,
    authored_root: Option<&Path>,
) -> Value {
    let response = request::handle_sidecar_value(
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": path,
                "authoredRoot": authored_root.map(|root| root.to_string_lossy().to_string()),
            }
        }),
        sessions,
    );
    assert_eq!(
        response["ok"], true,
        "openRecipe should succeed: {response}"
    );
    response["result"]["document"].clone()
}

fn request_document(
    sessions: &mut DocumentSessionManager,
    request_type: &str,
    document_id: &str,
) -> Value {
    let response = request::handle_sidecar_value(
        json!({
            "id": request_type,
            "type": request_type,
            "payload": {"documentId": document_id}
        }),
        sessions,
    );
    assert_eq!(
        response["ok"], true,
        "{request_type} should succeed: {response}"
    );
    response["result"]["document"].clone()
}

fn app_catalog(document: &Value) -> &Value {
    &document["appCatalog"]
}

fn app_ids(catalog: &Value) -> Vec<String> {
    catalog["apps"]
        .as_array()
        .expect("appCatalog.apps should be an array")
        .iter()
        .map(|app| {
            app["id"]
                .as_str()
                .expect("app id should be a string")
                .to_string()
        })
        .collect()
}

fn app<'a>(catalog: &'a Value, app_id: &str) -> &'a Value {
    catalog["apps"]
        .as_array()
        .expect("appCatalog.apps should be an array")
        .iter()
        .find(|candidate| candidate["id"] == app_id)
        .unwrap_or_else(|| panic!("app {app_id} should be projected"))
}

#[test]
fn ready_catalog_projects_app_selections_deterministically() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &workspace.recipe_path("demo_recipe.yaml"),
        Some(&workspace.authored_root()),
    );

    let catalog = app_catalog(&document);
    assert_eq!(catalog["status"], "ready");
    assert_eq!(app_ids(catalog), vec!["demo_app", "second_app"]);
    assert_eq!(catalog["diagnostics"], json!([]));

    let demo = app(catalog, "demo_app");
    assert_eq!(demo["name"], "Demo App");
    assert_eq!(demo["launcherActivity"], "com.example.demo.MainActivity");
    assert_eq!(
        demo["artifacts"],
        json!([
            {"id": "apk", "kind": "apk", "requiresInput": false},
            {"id": "config", "kind": "file", "requiresInput": true},
        ])
    );
    assert_eq!(demo["permissionSets"], json!(["baseline"]));
    assert_eq!(
        demo["targets"],
        json!([
            {"id": "system", "kind": "directory", "location": "shared_storage", "path": "Demo/system"},
            {"id": "config_file", "kind": "file", "location": "external_app_data", "path": "files/demo.cfg"},
        ])
    );

    let second = app(catalog, "second_app");
    assert_eq!(second["launcherActivity"], Value::Null);
    assert_eq!(second["permissionSets"], json!([]));
    assert_eq!(second["targets"], json!([]));
    assert_eq!(
        second["artifacts"],
        json!([{"id": "apk", "kind": "apk", "requiresInput": false}])
    );
}

#[test]
fn unbound_document_projects_unbound_catalog() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let loose_recipe = workspace.dir.join("loose_recipe.yaml");
    fs::copy(workspace.recipe_path("demo_recipe.yaml"), &loose_recipe)
        .expect("recipe should copy outside the authored tree");
    let mut sessions = common::test_sessions();
    let document = open_recipe(&mut sessions, &loose_recipe, None);

    let catalog = app_catalog(&document);
    assert_eq!(catalog["status"], "unbound");
    assert_eq!(catalog["apps"], json!([]));
    assert_eq!(catalog["diagnostics"], json!([]));
}

#[test]
fn invalid_catalog_fails_closed_with_relative_diagnostics() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    fs::write(
        workspace.app_path("zz_broken.yaml"),
        "schema_version: 1\nkind: app_definition\nid: broken\n",
    )
    .expect("broken app definition should be written");

    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &workspace.recipe_path("demo_recipe.yaml"),
        Some(&workspace.authored_root()),
    );

    let catalog = app_catalog(&document);
    assert_eq!(catalog["status"], "invalid");
    assert_eq!(catalog["apps"], json!([]));
    let diagnostics = catalog["diagnostics"]
        .as_array()
        .expect("invalid catalog diagnostics should be an array");
    assert!(!diagnostics.is_empty());
    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["file"] == "apps/zz_broken.yaml"));
}

#[test]
fn get_document_does_not_rescan_catalog_and_validate_refreshes_deterministically() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &workspace.recipe_path("demo_recipe.yaml"),
        Some(&workspace.authored_root()),
    );
    assert_eq!(app_catalog(&document)["status"], "ready");
    let document_id = document["documentId"].as_str().unwrap().to_string();

    fs::write(
        workspace.app_path("zz_broken.yaml"),
        "schema_version: 1\nkind: app_definition\nid: broken\n",
    )
    .expect("broken app definition should be written");

    let reserialized = request_document(&mut sessions, "getDocument", &document_id);
    assert_eq!(app_catalog(&reserialized)["status"], "ready");
    assert_eq!(
        app_ids(app_catalog(&reserialized)),
        vec!["demo_app", "second_app"]
    );

    let refreshed = request::handle_sidecar_value(
        json!({
            "id": "refresh",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": document_id, "authoredRoot": workspace.authored_root()}
        }),
        &mut sessions,
    );
    assert_eq!(refreshed["ok"], true, "refresh should succeed: {refreshed}");
    let refreshed = &refreshed["result"]["document"];
    assert_eq!(app_catalog(refreshed)["status"], "invalid");
    assert_eq!(app_catalog(refreshed)["apps"], json!([]));
}

#[test]
fn authored_root_change_retains_authored_values_and_refreshes_catalog() {
    let workspace_a = TempWorkspace::copy_fixture("editor_app_catalog");
    let workspace_b = TempWorkspace::copy_fixture("editor_app_catalog");
    fs::remove_file(workspace_b.app_path("second_app.yaml"))
        .expect("second app fixture should be removable");

    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &workspace_a.recipe_path("demo.app.install.yaml"),
        Some(&workspace_a.authored_root()),
    );
    let document_id = document["documentId"].as_str().unwrap().to_string();

    let edited = request::handle_sidecar_value(
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": document_id,
                "command": {"type": "SetStepAppRef", "stepId": "launch_demo", "appRef": "second_app"}
            }
        }),
        &mut sessions,
    );
    assert_eq!(edited["ok"], true, "edit should succeed: {edited}");
    let edited_document = &edited["result"]["document"];
    assert_eq!(edited_document["canUndo"], true);
    assert!(edited_document["yaml"]
        .as_str()
        .unwrap()
        .contains("app_ref: second_app"));

    let response = request::handle_sidecar_value(
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": document_id, "authoredRoot": workspace_b.authored_root()}
        }),
        &mut sessions,
    );
    assert_eq!(
        response["ok"], true,
        "setDocumentAuthoredRoot should succeed: {response}"
    );
    let rebound = &response["result"]["document"];

    assert!(rebound["yaml"]
        .as_str()
        .unwrap()
        .contains("app_ref: second_app"));
    assert_eq!(rebound["canUndo"], true);
    assert_eq!(rebound["canRedo"], false);
    assert_eq!(rebound["dirty"], true);
    assert_eq!(app_ids(app_catalog(rebound)), vec!["demo_app"]);

    let diagnostics = rebound["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_definition_unknown"),
        "retained unknown app reference should stay visible with a diagnostic: {diagnostics:?}"
    );

    let undone = request_document(&mut sessions, "undo", &document_id);
    assert!(undone["yaml"]
        .as_str()
        .unwrap()
        .contains("app_ref: demo_app"));
    assert_eq!(app_ids(app_catalog(&undone)), vec!["demo_app"]);
    assert_eq!(undone["canRedo"], true);

    let redone = request_document(&mut sessions, "redo", &document_id);
    assert!(redone["yaml"]
        .as_str()
        .unwrap()
        .contains("app_ref: second_app"));
    assert_eq!(redone["canRedo"], false);
}

fn assert_duplicate_package_conflict_blocks_demo_recipe(competitor_id: &str) {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let source = fs::read_to_string(workspace.app_path("demo_app.yaml"))
        .expect("fixture App Definition should be readable");
    let mut competitor: serde_yaml::Value =
        serde_yaml::from_str(&source).expect("fixture App Definition should be valid YAML");
    let artifact_id = competitor["artifacts"]
        .as_mapping()
        .and_then(|artifacts| artifacts.keys().next())
        .and_then(serde_yaml::Value::as_str)
        .expect("fixture App Definition should have an artifact")
        .to_string();
    competitor["id"] = serde_yaml::Value::String(competitor_id.to_string());
    let recipe_path = workspace.recipe_path("duplicate_package_probe.yaml");
    fs::write(
        &recipe_path,
        format!(
            "schema_version: 1\nkind: recipe\nid: duplicate_package_probe\nname: Duplicate package probe\nartifacts:\n  installer:\n    type: app_artifact\n    app_ref: demo_app\n    artifact: {artifact_id}\nsteps: []\n"
        ),
    )
    .expect("Recipe referencing demo_app should be written");

    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &recipe_path,
        Some(&workspace.authored_root()),
    );
    let document_id = document["documentId"].as_str().unwrap().to_string();
    fs::write(
        workspace.app_path(&format!("{competitor_id}.yaml")),
        serde_yaml::to_string(&competitor).unwrap(),
    )
    .expect("duplicate package App Definition should be written");
    let validated = request_document(&mut sessions, "validate", &document_id);
    let diagnostics = validated["diagnostics"]
        .as_array()
        .expect("Recipe validation diagnostics should be an array");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_package_id_conflict"),
        "duplicate package conflict should block a Recipe referencing demo_app: {diagnostics:?}"
    );
}

#[test]
fn duplicate_package_conflict_blocks_recipe_referencing_earlier_loaded_app() {
    assert_duplicate_package_conflict_blocks_demo_recipe("zzz_competing");
}

#[test]
fn duplicate_package_conflict_blocks_recipe_referencing_later_loaded_app() {
    assert_duplicate_package_conflict_blocks_demo_recipe("aaa_competing");
}

#[test]
fn validate_returns_the_full_refreshed_recipe_document_after_external_app_definition_change() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = common::test_sessions();
    let document = open_recipe(
        &mut sessions,
        &workspace.recipe_path("demo_recipe.yaml"),
        Some(&workspace.authored_root()),
    );
    let document_id = document["documentId"].as_str().unwrap().to_string();
    assert_eq!(app_catalog(&document)["status"], "ready");
    assert_eq!(
        app_ids(app_catalog(&document)),
        vec!["demo_app", "second_app"]
    );

    fs::write(
        workspace.app_path("third_app.yaml"),
        "schema_version: 1\nkind: app_definition\nid: third_app\nname: Third App\ndescription: Third catalog fixture app.\ncategory: emulator\npackage_id: com.example.third\nartifacts:\n  apk:\n    kind: apk\n    name: Third APK\n    source:\n      strategy: direct_url\n      url: https://example.com/third.apk\n",
    )
    .expect("third app definition should be written");

    let response = request::handle_sidecar_value(
        json!({
            "id": "validate",
            "type": "validate",
            "payload": {"documentId": document_id}
        }),
        &mut sessions,
    );
    assert_eq!(response["ok"], true, "validate should succeed: {response}");
    let result = &response["result"];

    // Validate returns the complete refreshed Recipe projection, not
    // diagnostics alone, so the editor can re-render from authoritative state.
    let refreshed = &result["document"];
    assert_eq!(refreshed["documentId"], document["documentId"]);
    assert_eq!(refreshed["path"], document["path"]);
    assert_eq!(refreshed["authoredRoot"], document["authoredRoot"]);
    assert_eq!(refreshed["dirty"], false);
    assert_eq!(refreshed["canUndo"], false);
    assert_eq!(refreshed["canRedo"], false);
    assert_eq!(
        app_ids(app_catalog(refreshed)),
        vec!["demo_app", "second_app", "third_app"]
    );
    // Diagnostics stay available at the top level for existing consumers and
    // match the projection embedded in the returned document.
    assert_eq!(refreshed["diagnostics"], result["diagnostics"]);
    assert!(refreshed["diagnostics"].as_array().is_some());
}

#[test]
fn catalog_change_refreshes_an_unrelated_dirty_recipe_without_replacing_its_model() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let recipes = workspace.authored_root().join("recipes");
    for entry in fs::read_dir(&recipes).expect("recipes should be readable") {
        let path = entry.expect("recipe entry").path();
        let keep = matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("demo.launcher_unavailable.yaml" | "demo.target_usage.yaml")
        );
        if !keep {
            fs::remove_file(path).expect("unused fixture recipe should be removable");
        }
    }

    let mut sessions = common::test_sessions();
    let unrelated = open_recipe(
        &mut sessions,
        &workspace.recipe_path("demo.launcher_unavailable.yaml"),
        Some(&workspace.authored_root()),
    );
    let unrelated_id = unrelated["documentId"].as_str().unwrap().to_string();
    let edited = request::handle_sidecar_value(
        json!({
            "id": "edit-unrelated",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": unrelated_id,
                "command": {
                    "type": "SetOverviewField",
                    "field": "name",
                    "value": "Unsaved launcher name"
                }
            }
        }),
        &mut sessions,
    );
    assert_eq!(edited["ok"], true, "edit should succeed: {edited}");
    let edited_document = &edited["result"]["document"];
    assert_eq!(edited_document["dirty"], true);
    assert_eq!(edited_document["canUndo"], true);
    let edited_yaml = edited_document["yaml"].as_str().unwrap().to_string();
    assert!(edited_yaml.contains("name: Unsaved launcher name"));
    assert!(edited_yaml.contains("app_ref: second_app"));

    let opened_app = request::handle_sidecar_value(
        json!({
            "id": "open-app",
            "type": "openAppDefinition",
            "payload": {
                "path": workspace.app_path("demo_app.yaml"),
                "authoredRoot": workspace.authored_root().to_string_lossy()
            }
        }),
        &mut sessions,
    );
    assert_eq!(opened_app["ok"], true, "App open failed: {opened_app}");
    let app_document_id = opened_app["result"]["document"]["documentId"]
        .as_str()
        .unwrap()
        .to_string();

    let preview = request::handle_sidecar_value(
        json!({
            "id": "preview-app-rename",
            "type": "previewCatalogChange",
            "payload": {
                "documentId": app_document_id,
                "intent": {
                    "kind": "renameApp",
                    "appId": "demo_app",
                    "newId": "renamed_demo_app"
                }
            }
        }),
        &mut sessions,
    );
    assert_eq!(preview["ok"], true, "preview failed: {preview}");
    let preview_result = &preview["result"];
    assert_eq!(preview_result["blockers"], json!([]));
    assert!(preview_result["changedFiles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|path| !path.as_str().unwrap().starts_with('/')));
    assert!(preview_result["references"]
        .as_array()
        .unwrap()
        .iter()
        .all(|reference| !reference["path"].as_str().unwrap().starts_with('/')));
    let app_operation = preview_result["operationId"].as_str().unwrap().to_string();
    let app_token = preview_result["previewToken"].as_str().unwrap().to_string();
    let applied = request::handle_sidecar_value(
        json!({
            "id": "apply-app-rename",
            "type": "applyCatalogChange",
            "payload": {"operationId": app_operation, "previewToken": app_token}
        }),
        &mut sessions,
    );
    assert_eq!(applied["ok"], true, "apply failed: {applied}");
    assert_eq!(applied["result"]["state"], "completed");

    let refreshed = request_document(&mut sessions, "getDocument", &unrelated_id);
    assert_eq!(refreshed["yaml"], edited_yaml);
    assert_eq!(refreshed["dirty"], true);
    assert_eq!(refreshed["canUndo"], true);
    assert_eq!(
        app_ids(app_catalog(&refreshed)),
        vec!["renamed_demo_app", "second_app"]
    );
    let renamed = app(app_catalog(&refreshed), "renamed_demo_app");
    assert!(renamed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|target| target["id"] == "config_file"));

    let target_preview = request::handle_sidecar_value(
        json!({
            "id": "preview-target-rename",
            "type": "previewCatalogChange",
            "payload": {
                "documentId": app_document_id,
                "intent": {
                    "kind": "renameTarget",
                    "appId": "renamed_demo_app",
                    "targetId": "config_file",
                    "newId": "config_path"
                }
            }
        }),
        &mut sessions,
    );
    assert_eq!(
        target_preview["ok"], true,
        "target preview failed: {target_preview}"
    );
    let target_preview_result = &target_preview["result"];
    assert_eq!(target_preview_result["blockers"], json!([]));
    let target_operation = target_preview_result["operationId"]
        .as_str()
        .unwrap()
        .to_string();
    let target_token = target_preview_result["previewToken"]
        .as_str()
        .unwrap()
        .to_string();
    let target_applied = request::handle_sidecar_value(
        json!({
            "id": "apply-target-rename",
            "type": "applyCatalogChange",
            "payload": {"operationId": target_operation, "previewToken": target_token}
        }),
        &mut sessions,
    );
    assert_eq!(
        target_applied["ok"], true,
        "target apply failed: {target_applied}"
    );
    assert_eq!(target_applied["result"]["state"], "completed");

    let refreshed = request_document(&mut sessions, "getDocument", &unrelated_id);
    assert_eq!(refreshed["yaml"], edited_yaml);
    assert_eq!(refreshed["dirty"], true);
    assert_eq!(refreshed["canUndo"], true);
    let renamed = app(app_catalog(&refreshed), "renamed_demo_app");
    let target_ids = renamed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(target_ids.contains(&"config_path"));
    assert!(!target_ids.contains(&"config_file"));
}

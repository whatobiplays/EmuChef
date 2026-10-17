//! App Generator review and publication sidecar tests.
//!
//! The suite drives the stateful sidecar service through its JSON surface:
//! catalog App listing and selection, immutable review handles, combined
//! App/Recipe publication, and publication-status reporting. Each test builds
//! its own temporary authored catalog so publication state never leaks between
//! cases.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use emuchef_rust_backend::request::handle_sidecar_value;
use emuchef_rust_backend::session::DocumentSessionManager;
use serde_json::{json, Value};

fn call(sessions: &mut DocumentSessionManager, request_type: &str, payload: Value) -> Value {
    handle_sidecar_value(
        json!({
            "id": request_type,
            "type": request_type,
            "payload": payload,
        }),
        sessions,
    )
}

fn success(sessions: &mut DocumentSessionManager, request_type: &str, payload: Value) -> Value {
    let response = call(sessions, request_type, payload);
    assert_eq!(response["ok"], true, "{response:#}");
    response["result"].clone()
}

/// Inspected APK facts for one package, with reviewed permission declarations.
fn facts(package: &str, permissions: Value) -> Value {
    json!({
        "packageName": package,
        "applicationLabel": "Example Player",
        "launcherActivities": [format!("{package}/.MainActivity")],
        "calculatedSha256": "b".repeat(64),
        "checksumStatus": "not_compared",
        "signatureVerification": "not_performed",
        "requestedPermissions": permissions,
        "split": false,
        "base": true,
    })
}

fn catalog() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary catalog root");
    fs::create_dir(root.path().join("apps")).unwrap();
    fs::create_dir(root.path().join("recipes")).unwrap();
    root
}

fn authored_root(catalog: &tempfile::TempDir) -> String {
    catalog
        .path()
        .canonicalize()
        .expect("catalog root canonicalizes")
        .to_string_lossy()
        .into_owned()
}

fn sessions_for(catalog: &tempfile::TempDir) -> DocumentSessionManager {
    DocumentSessionManager::with_catalog_storage(catalog.path().join(".test-application-state"))
}

fn review_new_app(
    sessions: &mut DocumentSessionManager,
    catalog: &tempfile::TempDir,
    request_id: i64,
) -> Value {
    success(
        sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(catalog),
            "requestId": request_id,
            "facts": facts("com.example.player", json!([])),
            "appArtifactId": "apk",
        }),
    )
}

/// Write the canonical App produced by a review into the catalog as the
/// pre-existing App that a later existing-App review extends.
fn write_existing_app(catalog: &tempfile::TempDir, review: &Value) -> String {
    let app_id = review["app"]["id"].as_str().expect("review app id");
    let relative = format!("apps/{app_id}.yaml");
    let yaml = review["appCanonicalYaml"]
        .as_str()
        .expect("canonical App YAML");
    let path = catalog.path().join(&relative);
    fs::write(&path, yaml).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o640);
    fs::set_permissions(&path, permissions).unwrap();
    relative
}

fn select_existing_app(
    sessions: &mut DocumentSessionManager,
    catalog: &tempfile::TempDir,
    app_path: &str,
) -> Value {
    success(
        sessions,
        "selectAppGeneratorExistingApp",
        json!({
            "authoredRoot": authored_root(catalog),
            "appPath": app_path,
        }),
    )
}

fn diagnostic_codes(review: &Value) -> Vec<String> {
    review["diagnostics"]
        .as_array()
        .expect("review diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .map(str::to_string)
        .collect()
}

#[test]
fn generated_pair_is_rejected_while_catalog_recovery_is_unresolved() {
    use emuchef_rust_backend::catalog_change::{CatalogChangeCoordinator, OperationRecord};
    let catalog = catalog();
    let storage_base = catalog.path().join(".test-application-state");
    let mut sessions = DocumentSessionManager::with_catalog_storage(&storage_base);
    let review = review_new_app(&mut sessions, &catalog, 1);
    assert_eq!(review["blocking"], false, "{review:#}");
    let coordinator =
        CatalogChangeCoordinator::open_with_storage(catalog.path(), &storage_base).unwrap();
    let guard = coordinator.acquire_publication().unwrap();
    coordinator
        .begin_operation(&guard, OperationRecord::new("pending", "test", "digest"))
        .unwrap();
    drop(guard);
    let saved = call(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "blocked-pair", "reviewId": review["reviewId"], "requestId": 1,
        }),
    );
    assert!(
        saved["ok"] == false || saved["result"]["ok"] == false,
        "{saved:#}"
    );
    assert_eq!(
        fs::read_dir(catalog.path().join("apps")).unwrap().count(),
        0
    );
    assert_eq!(
        fs::read_dir(catalog.path().join("recipes"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn reviewed_new_app_pair_publishes_once_and_reports_its_outcome() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let review = review_new_app(&mut sessions, &catalog, 1);
    assert_eq!(review["blocking"], false, "{review:#}");
    assert_eq!(review["target"]["mode"], "new_app");
    assert_eq!(review["target"]["packageLocked"], false);
    assert_eq!(review["artifact"]["appArtifactId"], "apk");
    assert_eq!(review["recipe"]["id"], "app.example_player.install");
    let app_path = review["destinations"]["app"]["relativePath"]
        .as_str()
        .expect("App destination")
        .to_string();
    let recipe_path = review["destinations"]["recipe"]["relativePath"]
        .as_str()
        .expect("Recipe destination")
        .to_string();
    assert!(app_path.starts_with("apps/"), "{app_path}");
    assert!(recipe_path.starts_with("recipes/"), "{recipe_path}");

    let save_request = json!({
        "operationId": "publish-new-app",
        "reviewId": review["reviewId"],
        "requestId": 1,
    });
    let saved = success(
        &mut sessions,
        "saveAppGeneratorReview",
        save_request.clone(),
    );
    assert_eq!(saved["ok"], true);
    assert_eq!(saved["outcome"], "saved");
    assert_eq!(saved["app"]["appPath"], json!(app_path));
    assert_eq!(saved["recipe"]["recipePath"], json!(recipe_path));
    let created = saved["createdFiles"].as_array().unwrap();
    assert!(created.iter().any(|value| value == &json!(app_path)));
    assert!(created.iter().any(|value| value == &json!(recipe_path)));
    assert!(saved["replacedFiles"].as_array().unwrap().is_empty());

    let app_text = fs::read_to_string(catalog.path().join(&app_path)).unwrap();
    assert!(app_text.contains("com.example.player"), "{app_text}");
    assert!(app_text.contains("user_provided"), "{app_text}");
    let recipe_text = fs::read_to_string(catalog.path().join(&recipe_path)).unwrap();
    assert!(
        recipe_text.contains("app_ref: example_player"),
        "{recipe_text}"
    );
    assert!(recipe_text.contains("app_artifact"), "{recipe_text}");

    // A repeated attempt replays the recorded outcome instead of publishing
    // the pair a second time.
    let replay = success(&mut sessions, "saveAppGeneratorReview", save_request);
    assert_eq!(replay, saved);

    let status = success(
        &mut sessions,
        "appGeneratorPublicationStatus",
        json!({ "operationId": "publish-new-app" }),
    );
    assert_eq!(status["outcome"], "saved");
    assert_eq!(status["confirmed"], true);
    let missing = success(
        &mut sessions,
        "appGeneratorPublicationStatus",
        json!({ "operationId": "never-attempted" }),
    );
    assert_eq!(missing["outcome"], "unknown");
    assert_eq!(missing["confirmed"], false);
    assert_eq!(missing["detail"]["reason"], "no_record");
}

#[test]
fn existing_app_publication_appends_the_reviewed_artifact_and_preserves_the_file() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 11);
    let app_relative = write_existing_app(&catalog, &baseline);

    let listing = success(
        &mut sessions,
        "listAppGeneratorExistingApps",
        json!({ "authoredRoot": authored_root(&catalog) }),
    );
    assert_eq!(listing["inventory"]["state"], "complete", "{listing:#}");
    assert!(
        listing["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|app| app["appPath"] == json!(app_relative)),
        "{listing:#}"
    );

    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);
    assert_eq!(selection["packageId"], "com.example.player");
    assert_eq!(selection["artifactIds"], json!(["apk"]));

    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 12,
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "recipeId": "app.example_player.extra_install",
            "artifactName": "Extra APK",
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");
    assert_eq!(review["target"]["mode"], "existing_app");
    assert_eq!(review["target"]["packageLocked"], true);
    assert_eq!(review["artifact"]["appArtifactId"], "extra");
    assert_eq!(review["artifact"]["change"], "added");
    assert_eq!(review["recipe"]["id"], "app.example_player.extra_install");
    assert_eq!(review["changeImpact"]["addedArtifactIds"], json!(["extra"]));
    assert!(
        review["changeImpact"]["preservedArtifactIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "apk"),
        "{review:#}"
    );

    let saved = success(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-existing-app",
            "reviewId": review["reviewId"],
            "requestId": 12,
        }),
    );
    assert_eq!(saved["outcome"], "saved");
    assert_eq!(saved["app"]["appPath"], json!(app_relative));
    assert!(
        saved["replacedFiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &json!(app_relative)),
        "{saved:#}"
    );

    let app_text = fs::read_to_string(catalog.path().join(&app_relative)).unwrap();
    assert!(app_text.contains("extra"), "{app_text}");
    assert!(app_text.contains("apk"), "{app_text}");
    let mode = fs::metadata(catalog.path().join(&app_relative))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o640,
        "the published App must keep its original file mode"
    );
}

#[test]
fn existing_app_publication_reconciles_a_clean_open_document_before_success() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 13);
    let app_relative = write_existing_app(&catalog, &baseline);
    let app_path = catalog.path().join(&app_relative);
    let opened = success(
        &mut sessions,
        "openAppDefinition",
        json!({
            "path": app_path.to_string_lossy(),
            "authoredRoot": authored_root(&catalog),
        }),
    );
    let document_id = opened["document"]["documentId"]
        .as_str()
        .expect("document id")
        .to_string();
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);
    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 14,
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "recipeId": "app.example_player.extra_install",
            "artifactName": "Extra APK",
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");

    let published = success(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-existing-app-with-open-session",
            "reviewId": review["reviewId"],
            "requestId": 14,
        }),
    );
    assert_eq!(published["outcome"], "saved");

    let refreshed = success(
        &mut sessions,
        "getDocument",
        json!({ "documentId": document_id }),
    );
    assert_eq!(refreshed["document"]["dirty"], false);
    assert!(
        serde_json::to_string(&refreshed["document"]["appDefinition"])
            .unwrap()
            .contains("extra"),
        "the open document must include the generated App artifact: {refreshed:#}"
    );
}

#[test]
fn dirty_open_app_document_blocks_publication_until_the_draft_is_discarded() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 21);
    let app_relative = write_existing_app(&catalog, &baseline);
    let app_path = catalog.path().join(&app_relative);
    let before = fs::read_to_string(&app_path).unwrap();

    let opened = success(
        &mut sessions,
        "openAppDefinition",
        json!({
            "path": app_path.to_string_lossy(),
            "authoredRoot": authored_root(&catalog),
        }),
    );
    let document_id = opened["document"]["documentId"]
        .as_str()
        .expect("document id")
        .to_string();
    let renamed = success(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": document_id,
            "command": { "type": "SetName", "name": "Renamed Player" },
        }),
    );
    assert_eq!(renamed["document"]["dirty"], true, "{renamed:#}");

    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);
    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 22,
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "recipeId": "app.example_player.extra_install",
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");
    let save_request = json!({
        "operationId": "publish-with-dirty-draft",
        "reviewId": review["reviewId"],
        "requestId": 22,
    });

    let refused = call(
        &mut sessions,
        "saveAppGeneratorReview",
        save_request.clone(),
    );
    assert_eq!(refused["ok"], false, "{refused:#}");
    assert_eq!(refused["error"]["code"], "dirty_app_document");

    let after_refusal = success(
        &mut sessions,
        "getDocument",
        json!({ "documentId": document_id }),
    );
    assert_eq!(
        after_refusal["document"]["dirty"], true,
        "{after_refusal:#}"
    );
    assert_eq!(
        after_refusal["document"]["appDefinition"]["name"],
        "Renamed Player"
    );
    assert_eq!(fs::read_to_string(&app_path).unwrap(), before);
    assert!(
        fs::read_dir(catalog.path().join("recipes"))
            .unwrap()
            .next()
            .is_none(),
        "a refused publication must not create catalog files"
    );

    // Discarding the unsaved draft leaves the reviewed candidate publishable.
    success(
        &mut sessions,
        "closeDocument",
        json!({ "documentId": document_id }),
    );
    let saved = success(&mut sessions, "saveAppGeneratorReview", save_request);
    assert_eq!(saved["outcome"], "saved");
    let published = fs::read_to_string(&app_path).unwrap();
    assert!(!published.contains("Renamed Player"), "{published}");
    assert!(published.contains("extra"), "{published}");
}

#[test]
fn save_requires_the_review_request_identity_and_keeps_the_handle_after_refusal() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let review = review_new_app(&mut sessions, &catalog, 31);

    let refusal = call(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-request-mismatch",
            "reviewId": review["reviewId"],
            "requestId": 32,
        }),
    );
    assert_eq!(refusal["ok"], false, "{refusal:#}");
    assert_eq!(refusal["error"]["code"], "review_invalidated");

    let saved = success(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-request-match",
            "reviewId": review["reviewId"],
            "requestId": 31,
        }),
    );
    assert_eq!(saved["outcome"], "saved");

    let refused_status = success(
        &mut sessions,
        "appGeneratorPublicationStatus",
        json!({ "operationId": "publish-request-mismatch" }),
    );
    assert_eq!(refused_status["outcome"], "unknown");
    assert_eq!(refused_status["detail"]["reason"], "no_record");
}

#[test]
fn colliding_recipe_identity_blocks_without_renaming_the_generated_recipe() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    fs::write(
        catalog.path().join("recipes/app.example_player.install.yaml"),
        "schema_version: 1\nkind: recipe\nid: app.example_player.install\nname: Existing Install\nsteps: []\n",
    )
    .unwrap();

    let review = review_new_app(&mut sessions, &catalog, 41);
    assert_eq!(review["blocking"], true, "{review:#}");
    assert_eq!(
        review["recipe"]["id"], "app.example_player.install",
        "the chosen Recipe identity is never renamed to dodge a collision"
    );
    assert!(!diagnostic_codes(&review).is_empty(), "{review:#}");
}

#[test]
fn colliding_artifact_identity_blocks_without_renaming_the_reviewed_artifact() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 51);
    let app_relative = write_existing_app(&catalog, &baseline);
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);

    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 52,
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "apk",
            "recipeId": "app.example_player.extra_install",
        }),
    );
    assert_eq!(review["blocking"], true, "{review:#}");
    assert!(
        review["app"]["artifacts"]["apk"].is_object(),
        "the colliding identity is never reassigned: {review:#}"
    );
    assert!(!diagnostic_codes(&review).is_empty(), "{review:#}");
}

#[test]
fn existing_app_review_blocks_when_the_inspected_package_differs() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 61);
    let app_relative = write_existing_app(&catalog, &baseline);
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);

    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 62,
            "facts": facts("com.other.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
        }),
    );
    assert_eq!(review["blocking"], true, "{review:#}");
    assert!(!diagnostic_codes(&review).is_empty(), "{review:#}");
}

#[test]
fn permission_automation_review_separates_baseline_and_elevated_actions() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let declarations = json!([
        { "name": "android.permission.CAMERA", "declarationKind": "uses_permission" },
        { "name": "android.permission.WRITE_SECURE_SETTINGS", "declarationKind": "uses_permission" },
    ]);
    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 71,
            "facts": facts("com.example.player", declarations),
            "appArtifactId": "apk",
            "permissionAutomationEnabled": true,
            "permissionAutomation": {
                "packageName": "com.example.player",
                "runtimePermissions": [
                    {
                        "permissionName": "android.permission.CAMERA",
                        "requiresRoot": false,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    },
                ],
                "appOps": [
                    {
                        "permissionName": "android.permission.WRITE_SECURE_SETTINGS",
                        "operationName": "android:write_secure_settings",
                        "mode": "allow",
                        "requiresRoot": true,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    },
                ],
            },
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");
    let automation = &review["permissionAutomation"];
    assert_eq!(automation["enabled"], true);
    assert!(
        automation["baseline"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["permission"] == "android.permission.CAMERA"),
        "{review:#}"
    );
    assert!(
        automation["elevated"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["op"] == "android:write_secure_settings"),
        "{review:#}"
    );
    assert_eq!(automation["elevated"]["rootRequired"], true);
    assert_eq!(automation["elevated"]["optional"], true);
    assert!(review["apkInspection"].is_object(), "{review:#}");

    let saved = success(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-permission-automation",
            "reviewId": review["reviewId"],
            "requestId": 71,
        }),
    );
    assert_eq!(saved["outcome"], "saved");
    let app_text = fs::read_to_string(
        catalog
            .path()
            .join(saved["app"]["appPath"].as_str().unwrap()),
    )
    .unwrap();
    assert!(app_text.contains("android.permission.CAMERA"), "{app_text}");
    assert!(
        !app_text.contains("apk_inspection"),
        "transient inspection evidence is never persisted: {app_text}"
    );
}

/// Permission automation is opt-in on the wire: an absent
/// `permissionAutomationEnabled` flag reports disabled intent, preserves the
/// declared App permission sets without generating grant steps, and blocks a
/// selection instead of applying it. An enabled request still executes the
/// whole declared set when it selects no actions.
#[test]
fn permission_automation_review_reports_the_request_intent_flag() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let declarations = json!([
        { "name": "android.permission.CAMERA", "declarationKind": "uses_permission" },
    ]);
    let baseline = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 91,
            "facts": facts("com.example.player", declarations.clone()),
            "appArtifactId": "apk",
            "permissionAutomationEnabled": true,
            "permissionAutomation": {
                "packageName": "com.example.player",
                "runtimePermissions": [
                    {
                        "permissionName": "android.permission.CAMERA",
                        "requiresRoot": false,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    }
                ],
                "appOps": [],
            },
        }),
    );
    assert_eq!(baseline["blocking"], false, "{baseline:#}");
    let app_relative = write_existing_app(&catalog, &baseline);
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);

    let disabled = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 92,
            "facts": facts("com.example.player", declarations.clone()),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
        }),
    );
    assert_eq!(disabled["blocking"], false, "{disabled:#}");
    assert_eq!(disabled["permissionAutomation"]["enabled"], false);
    assert!(
        disabled["permissionAutomation"]["baseline"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["permission"] == "android.permission.CAMERA"),
        "{disabled:#}"
    );
    assert_eq!(
        disabled["changeImpact"]["addedPermissionActions"],
        json!({ "baseline": [], "elevated": [] })
    );
    let disabled_yaml = disabled["recipeCanonicalYaml"]
        .as_str()
        .expect("canonical Recipe YAML");
    assert!(
        !disabled_yaml.contains("grant_permissions") && !disabled_yaml.contains("permission_set"),
        "{disabled_yaml}"
    );

    let enabled = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 93,
            "facts": facts("com.example.player", declarations.clone()),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "permissionAutomationEnabled": true,
        }),
    );
    assert_eq!(enabled["blocking"], false, "{enabled:#}");
    assert_eq!(enabled["permissionAutomation"]["enabled"], true);
    let enabled_yaml = enabled["recipeCanonicalYaml"]
        .as_str()
        .expect("canonical Recipe YAML");
    assert!(
        enabled_yaml.contains("permission_set: baseline"),
        "{enabled_yaml}"
    );

    let contradicted = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 94,
            "facts": facts("com.example.player", declarations),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "permissionAutomation": {
                "packageName": "com.example.player",
                "runtimePermissions": [
                    {
                        "permissionName": "android.permission.CAMERA",
                        "requiresRoot": false,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    }
                ],
                "appOps": [],
            },
        }),
    );
    assert_eq!(contradicted["blocking"], true, "{contradicted:#}");
    assert_eq!(contradicted["permissionAutomation"]["enabled"], false);
    assert!(
        diagnostic_codes(&contradicted).contains(&"apk_permission_automation_disabled".to_string()),
        "{contradicted:#}"
    );
}

#[test]
fn unreadable_catalog_entry_blocks_existing_app_selection() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let unreadable = catalog.path().join("apps/blocked.yaml");
    fs::write(&unreadable, "schema_version: 1\nkind: app_definition\n").unwrap();
    let mut permissions = fs::metadata(&unreadable).unwrap().permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&unreadable, permissions).unwrap();

    let listing = success(
        &mut sessions,
        "listAppGeneratorExistingApps",
        json!({ "authoredRoot": authored_root(&catalog) }),
    );
    assert_eq!(listing["inventory"]["state"], "incomplete", "{listing:#}");

    let refused = call(
        &mut sessions,
        "selectAppGeneratorExistingApp",
        json!({
            "authoredRoot": authored_root(&catalog),
            "appPath": "apps/example_player.yaml",
        }),
    );
    assert_eq!(refused["ok"], false, "{refused:#}");
    assert_eq!(
        refused["error"]["code"], "app_catalog_inventory_incomplete",
        "{refused:#}"
    );
}

/// One pinned public HTTPS release used by the remote-source review flow.
fn remote_source() -> Value {
    json!({
        "mode": "github_release",
        "strategy": "pinned_remote_asset",
        "downloadUrl": "https://github.com/example/project/releases/download/v1/app.apk",
        "provider": "github",
        "baseUrl": "https://github.com",
        "repository": "example/project",
        "releaseTag": "v1",
        "assetName": "app.apk",
    })
}

#[test]
fn remote_review_uses_the_same_stateful_handles_and_publishes_the_pair() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let review = success(
        &mut sessions,
        "reviewRemoteAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 81,
            "facts": facts("com.example.remote", json!([])),
            "source": remote_source(),
            "appArtifactId": "apk",
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");
    assert_eq!(review["target"]["mode"], "new_app");
    assert_eq!(review["artifact"]["source"]["strategy"], "direct_url");
    assert_eq!(review["permissionAutomation"]["enabled"], false);

    let saved = success(
        &mut sessions,
        "saveAppGeneratorReview",
        json!({
            "operationId": "publish-remote-app",
            "reviewId": review["reviewId"],
            "requestId": 81,
        }),
    );
    assert_eq!(saved["outcome"], "saved");
    let app_path = saved["app"]["appPath"].as_str().unwrap().to_string();
    let recipe_path = saved["recipe"]["recipePath"].as_str().unwrap().to_string();
    let app_text = fs::read_to_string(catalog.path().join(&app_path)).unwrap();
    assert!(app_text.contains("direct_url"), "{app_text}");
    let recipe_text = fs::read_to_string(catalog.path().join(&recipe_path)).unwrap();
    assert!(recipe_text.contains("app_ref"), "{recipe_text}");

    // The published App can be extended through the same remote review flow.
    let selection = select_existing_app(&mut sessions, &catalog, &app_path);
    let extension = success(
        &mut sessions,
        "reviewRemoteAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 82,
            "facts": facts("com.example.remote", json!([])),
            "source": remote_source(),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "permissionAutomationEnabled": true,
        }),
    );
    assert_eq!(extension["blocking"], false, "{extension:#}");
    assert_eq!(extension["target"]["mode"], "existing_app");
    assert_eq!(extension["permissionAutomation"]["enabled"], true);
    assert_eq!(
        extension["changeImpact"]["addedArtifactIds"],
        json!(["extra"])
    );
}

/// Write one reviewed App into the catalog and capture the snapshot handle a
/// later existing-App preview or review resolves.
fn existing_app_snapshot(
    sessions: &mut DocumentSessionManager,
    catalog: &tempfile::TempDir,
    request_id: i64,
) -> String {
    let baseline = review_new_app(sessions, catalog, request_id);
    let app_relative = write_existing_app(catalog, &baseline);
    let selection = select_existing_app(sessions, catalog, &app_relative);
    selection["appSnapshotId"].as_str().unwrap().to_string()
}

/// Build the payload the native preview boundary sends for a remote source.
fn remote_preview_payload(
    catalog: &tempfile::TempDir,
    snapshot_id: Option<&str>,
    app_artifact_id: &str,
) -> Value {
    let mut payload = json!({
        "facts": facts("com.example.player", json!([])),
        "source": remote_source(),
        "appArtifactId": app_artifact_id,
        "regenerateIdentifiers": false,
    });
    if let Some(snapshot_id) = snapshot_id {
        payload["authoredRoot"] = json!(authored_root(catalog));
        payload["existingAppSnapshotId"] = json!(snapshot_id);
    }
    payload
}

#[test]
fn previews_resolve_the_retained_existing_app_snapshot_for_local_and_remote_sources() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let snapshot_id = existing_app_snapshot(&mut sessions, &catalog, 91);

    let local = success(
        &mut sessions,
        "generateAppRecipeDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": snapshot_id,
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(local["blocking"], false, "{local:#}");
    assert_eq!(
        local["app"]["package_id"], "com.example.player",
        "{local:#}"
    );
    assert_eq!(local["app"]["id"], "example_player", "{local:#}");
    assert!(
        local["app"]["artifacts"]["extra"].is_object(),
        "the preview extends the selected App under the chosen artifact identity: {local:#}"
    );
    assert!(local["recipe"]["steps"].is_array(), "{local:#}");

    let remote = success(
        &mut sessions,
        "generateRemoteAppRecipeDraft",
        remote_preview_payload(&catalog, Some(&snapshot_id), "extra"),
    );
    assert_eq!(remote["blocking"], false, "{remote:#}");
    assert_eq!(
        remote["app"]["package_id"], "com.example.player",
        "{remote:#}"
    );
    assert_eq!(
        remote["app"]["artifacts"]["extra"]["source"]["strategy"], "direct_url",
        "{remote:#}"
    );
}

#[test]
fn previews_block_when_the_inspected_package_differs_from_the_selected_app() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let snapshot_id = existing_app_snapshot(&mut sessions, &catalog, 92);

    let local = success(
        &mut sessions,
        "generateAppRecipeDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "facts": facts("com.other.player", json!([])),
            "existingAppSnapshotId": snapshot_id,
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(local["blocking"], true, "{local:#}");
    assert!(!diagnostic_codes(&local).is_empty(), "{local:#}");

    let remote = success(
        &mut sessions,
        "generateRemoteAppRecipeDraft",
        json!({
            "facts": facts("com.other.player", json!([])),
            "source": remote_source(),
            "authoredRoot": authored_root(&catalog),
            "existingAppSnapshotId": snapshot_id,
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(remote["blocking"], true, "{remote:#}");
    assert!(!diagnostic_codes(&remote).is_empty(), "{remote:#}");
}

#[test]
fn previews_reject_an_unknown_or_mismatched_existing_app_snapshot_handle() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    existing_app_snapshot(&mut sessions, &catalog, 93);

    let unknown = call(
        &mut sessions,
        "generateAppRecipeDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": "missing-app-snapshot",
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(unknown["ok"], false, "{unknown:#}");
    assert_eq!(
        unknown["error"]["code"], "review_invalidated",
        "{unknown:#}"
    );

    let mismatched = call(
        &mut sessions,
        "generateRemoteAppRecipeDraft",
        json!({
            "facts": facts("com.example.player", json!([])),
            "source": remote_source(),
            "authoredRoot": authored_root(&catalog),
            "existingAppSnapshotId": "any-handle",
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(mismatched["ok"], false, "{mismatched:#}");
    assert_eq!(
        mismatched["error"]["code"], "review_invalidated",
        "{mismatched:#}"
    );
}

/// Write one persisted Recipe that references the App under review so the
/// change-impact summary has an affected dependent Recipe to classify.
///
/// The persisted Recipe is derived from the canonical generated Recipe so it
/// stays valid under the same authored contract, with only its identity and
/// display name changed to keep the generated destination free.
fn write_dependent_recipe(catalog: &tempfile::TempDir, baseline: &Value) -> String {
    let canonical = baseline["recipeCanonicalYaml"]
        .as_str()
        .expect("baseline canonical Recipe YAML");
    let yaml = canonical
        .replace(
            "app.example_player.install",
            "app.example_player.prepared_install",
        )
        .replace("Install Example Player", "Prepared Example Player");
    let relative = "recipes/app.example_player.prepared_install.yaml".to_string();
    fs::write(catalog.path().join(&relative), yaml).unwrap();
    relative
}

#[test]
fn existing_app_review_reports_affected_recipes_and_added_action_sets() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 81);
    let app_relative = write_existing_app(&catalog, &baseline);
    let dependent = write_dependent_recipe(&catalog, &baseline);
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);

    let declarations = json!([
        { "name": "android.permission.CAMERA", "declarationKind": "uses_permission" },
        { "name": "android.permission.WRITE_SECURE_SETTINGS", "declarationKind": "uses_permission" },
    ]);
    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 82,
            "facts": facts("com.example.player", declarations),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
            "permissionAutomationEnabled": true,
            "permissionAutomation": {
                "packageName": "com.example.player",
                "runtimePermissions": [
                    {
                        "permissionName": "android.permission.CAMERA",
                        "requiresRoot": false,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    },
                ],
                "appOps": [
                    {
                        "permissionName": "android.permission.WRITE_SECURE_SETTINGS",
                        "operationName": "android:write_secure_settings",
                        "mode": "allow",
                        "requiresRoot": true,
                        "androidApiMin": 23,
                        "androidApiMax": null,
                    },
                ],
            },
        }),
    );
    assert_eq!(review["blocking"], false, "{review:#}");

    // The change-impact summary reports the persisted Recipe that shares the
    // App under review, classified through the same dependent-Recipe
    // validation path used for ordinary App saves.
    let affected = review["changeImpact"]["affectedRecipes"]
        .as_array()
        .expect("affected recipes");
    assert_eq!(affected.len(), 1, "{review:#}");
    assert_eq!(
        affected[0]["recipeId"], "app.example_player.prepared_install",
        "{review:#}"
    );
    assert_eq!(affected[0]["recipePath"], json!(dependent), "{review:#}");
    assert_eq!(affected[0]["status"], "revalidated_ok", "{review:#}");
    assert!(
        affected[0]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{review:#}"
    );

    // The summary reports the whole reviewed permission set as added actions,
    // split by baseline and elevated set just like the persisted App.
    assert_eq!(
        review["changeImpact"]["addedPermissionActions"]["baseline"],
        json!([{
            "kind": "runtime_permission",
            "permission": "android.permission.CAMERA",
            "androidApiMin": 23,
            "androidApiMax": null,
        }]),
        "{review:#}"
    );
    assert_eq!(
        review["changeImpact"]["addedPermissionActions"]["elevated"],
        json!([{
            "kind": "app_op",
            "permission": "android:write_secure_settings",
            "op": "android:write_secure_settings",
            "mode": "allow",
            "androidApiMin": 23,
            "androidApiMax": null,
        }]),
        "{review:#}"
    );
}

#[test]
fn previews_reject_a_valid_snapshot_handle_from_another_root_or_session() {
    let catalog_a = catalog();
    let mut sessions = sessions_for(&catalog_a);
    let snapshot_id = existing_app_snapshot(&mut sessions, &catalog_a, 94);

    // A real, canonical catalog that even holds an identical App Definition:
    // only the selected root differs, so the retained handle must still refuse.
    let catalog_b = catalog();
    let mut catalog_b_sessions = sessions_for(&catalog_b);
    let baseline_b = review_new_app(&mut catalog_b_sessions, &catalog_b, 95);
    write_existing_app(&catalog_b, &baseline_b);

    let wrong_root = call(
        &mut sessions,
        "generateAppRecipeDraft",
        json!({
            "authoredRoot": authored_root(&catalog_b),
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": snapshot_id,
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(wrong_root["ok"], false, "{wrong_root:#}");
    assert_eq!(
        wrong_root["error"]["code"], "review_invalidated",
        "{wrong_root:#}"
    );

    // The handle is also bound to the stateful session that captured it.
    let mut other_sessions = sessions_for(&catalog_a);
    let other_session = call(
        &mut other_sessions,
        "generateAppRecipeDraft",
        json!({
            "authoredRoot": authored_root(&catalog_a),
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": snapshot_id,
            "appArtifactId": "extra",
            "regenerateIdentifiers": false,
        }),
    );
    assert_eq!(other_session["ok"], false, "{other_session:#}");
    assert_eq!(
        other_session["error"]["code"], "review_invalidated",
        "{other_session:#}"
    );
}

#[test]
fn existing_app_review_blocks_when_a_dependent_recipe_cannot_be_classified() {
    let catalog = catalog();
    let mut sessions = sessions_for(&catalog);
    let baseline = review_new_app(&mut sessions, &catalog, 83);
    let app_relative = write_existing_app(&catalog, &baseline);
    fs::write(
        catalog.path().join("recipes/unclassified.yaml"),
        "steps: [not a recipe]",
    )
    .unwrap();
    let selection = select_existing_app(&mut sessions, &catalog, &app_relative);

    let review = success(
        &mut sessions,
        "reviewAppGeneratorDraft",
        json!({
            "authoredRoot": authored_root(&catalog),
            "requestId": 84,
            "facts": facts("com.example.player", json!([])),
            "existingAppSnapshotId": selection["appSnapshotId"],
            "appArtifactId": "extra",
        }),
    );
    assert_eq!(review["blocking"], true, "{review:#}");
    assert!(
        diagnostic_codes(&review)
            .iter()
            .any(|code| code == "app_dependency_classification_incomplete"),
        "{review:#}"
    );
    assert!(
        review["reviewId"].is_null(),
        "a blocked review never yields a publishable handle: {review:#}"
    );
}

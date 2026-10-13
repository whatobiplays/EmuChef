use std::fs;

use emuchef_rust_backend::request::handle_sidecar_value;
use emuchef_rust_backend::session::DocumentSessionManager;
use serde_json::{json, Value};

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

#[test]
fn semantically_invalid_app_documents_open_and_can_be_repaired_incrementally() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("invalid-app.yaml");
    let original = "schema_version: 1\nkind: app_definition\nid: Invalid_ID\nname: ' '\npackage_id: invalid-package\n";
    fs::write(&path, original).expect("invalid app definition should be written");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": path, "authoredRoot": null }),
    );
    assert_eq!(
        opened["ok"], true,
        "typed semantic errors must not block open"
    );
    let document = &opened["result"]["document"];
    assert_eq!(document["documentKind"], "app_definition");
    assert!(document
        .as_object()
        .unwrap()
        .contains_key("owningAuthoredRoot"));
    assert_eq!(document["owningAuthoredRoot"], Value::Null);
    assert_eq!(document["documentId"], "doc-1");
    assert!(document["path"].is_string());
    assert_eq!(document["authoredRoot"], Value::Null);
    assert_eq!(document["dirty"], false);
    assert_eq!(document["canUndo"], false);
    assert_eq!(document["canRedo"], false);
    assert_eq!(document["appDefinition"]["id"], "Invalid_ID");
    assert_eq!(document["appDefinition"]["package_id"], "invalid-package");
    assert!(document["yaml"].is_string());
    assert!(document["diagnostics"].is_array());
    assert_eq!(document["editRestrictions"]["canSaveAs"], false);
    assert_eq!(document["diagnostics"].as_array().unwrap().len(), 3);
    assert_eq!(document["editRestrictions"]["identityLocked"], false);
    assert_eq!(document["editRestrictions"]["canEditIdentity"], true);

    let malformed = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": 7 }
        }),
    );
    assert_eq!(malformed["ok"], false);
    let after_malformed = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(
        after_malformed["result"]["document"]["yaml"], document["yaml"],
        "malformed command payloads must not change draft content"
    );
    assert_eq!(after_malformed["result"]["document"]["canUndo"], false);

    let set_name = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": "RetroArch" }
        }),
    );
    assert_eq!(set_name["ok"], true);
    let partially_repaired = &set_name["result"]["document"];
    assert_eq!(partially_repaired["appDefinition"]["name"], "RetroArch");
    assert_eq!(
        partially_repaired["diagnostics"].as_array().unwrap().len(),
        2,
        "repairing one field must not require unrelated errors to be fixed first"
    );

    let rejected_save = request(
        &mut sessions,
        "saveAppDefinition",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(
        rejected_save["ok"], false,
        "invalid App Definitions cannot be saved"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let after_rejected_save = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(after_rejected_save["result"]["document"]["dirty"], true);
    assert_eq!(
        after_rejected_save["result"]["document"]["appDefinition"]["name"],
        "RetroArch"
    );

    for command in [
        json!({ "type": "SetId", "id": "retroarch" }),
        json!({ "type": "SetPackageId", "packageId": "com.retroarch.aarch64" }),
    ] {
        let response = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({ "documentId": "doc-1", "command": command }),
        );
        assert_eq!(response["ok"], true);
    }

    let saved = request(
        &mut sessions,
        "saveAppDefinition",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(saved["ok"], true);
    let saved_document = &saved["result"]["document"];
    assert_eq!(saved_document["diagnostics"], json!([]));
    assert_eq!(saved_document["dirty"], false);
    let saved_yaml = fs::read_to_string(&path).unwrap();
    assert_eq!(saved_yaml, saved_document["yaml"]);
    assert!(!saved_yaml.contains("artifacts:"));
    assert!(!saved_yaml.contains("targets:"));

    let undone = request(&mut sessions, "undo", json!({ "documentId": "doc-1" }));
    assert_eq!(undone["ok"], true);
    assert_eq!(undone["result"]["document"]["dirty"], true);
    assert_eq!(
        undone["result"]["document"]["appDefinition"]["id"],
        "retroarch"
    );

    let redone = request(&mut sessions, "redo", json!({ "documentId": "doc-1" }));
    assert_eq!(redone["ok"], true);
    assert_eq!(redone["result"]["document"]["dirty"], false);
}

#[test]
fn malformed_commands_and_invalid_changed_fields_are_atomic() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("invalid-app.yaml");
    let original = "schema_version: 1\nkind: app_definition\nid: retroarch\nname: ' '\npackage_id: com.retroarch.aarch64\n";
    fs::write(&path, original).expect("invalid app definition should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], true);
    let baseline_yaml = opened["result"]["document"]["yaml"].clone();

    for command in [
        json!({ "type": "SetName", "name": 7 }),
        json!({ "type": "SetName", "name": "   " }),
        json!({ "type": "SetName", "name": "Valid", "extra": true }),
        json!({ "type": "SetDescription" }),
        json!({ "type": "SetPermissionSet", "permissionSet": "baseline" }),
    ] {
        let response = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({ "documentId": "doc-1", "command": command }),
        );
        assert_eq!(response["ok"], false);
        let current = request(
            &mut sessions,
            "getDocument",
            json!({ "documentId": "doc-1" }),
        );
        assert_eq!(current["result"]["document"]["yaml"], baseline_yaml);
        assert_eq!(current["result"]["document"]["canUndo"], false);
    }
}

#[test]
fn app_history_noops_preserve_redo_and_shared_lifecycle_rejects_recipe_only_calls() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("app.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: retroarch\nname: RetroArch\npackage_id: com.retroarch.aarch64\n",
    )
    .expect("app definition should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], true);

    let changed = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": "RetroArch Emulator" }
        }),
    );
    assert_eq!(changed["result"]["commandResult"]["changed"], true);
    let undone = request(&mut sessions, "undo", json!({ "documentId": "doc-1" }));
    assert_eq!(undone["result"]["commandResult"]["changed"], true);
    assert_eq!(undone["result"]["document"]["canRedo"], true);

    let noop = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": "RetroArch" }
        }),
    );
    assert_eq!(noop["result"]["commandResult"]["changed"], false);
    assert_eq!(noop["result"]["document"]["canRedo"], true);

    let wrong_kind = request(
        &mut sessions,
        "getRefIndex",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(wrong_kind["ok"], false);
    assert_eq!(wrong_kind["error"]["details"]["expectedKind"], "recipe");
}

#[test]
fn catalog_app_identity_remains_locked_when_session_root_changes() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let root = directory.path().join("authored");
    let apps = root.join("apps");
    fs::create_dir_all(apps.join("../recipes")).expect("catalog directories should be created");
    let path = apps.join("retroarch.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: Invalid_ID\nname: RetroArch\npackage_id: invalid-package\n",
    )
    .expect("catalog app definition should be written");
    let other_root = directory.path().join("other-authored");
    fs::create_dir_all(other_root.join("recipes")).expect("other root should be created");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": path, "authoredRoot": root }),
    );
    assert_eq!(opened["ok"], true);
    assert_eq!(
        opened["result"]["document"]["editRestrictions"]["identityLocked"],
        true
    );

    let changed_root = request(
        &mut sessions,
        "setDocumentAuthoredRoot",
        json!({ "documentId": "doc-1", "authoredRoot": other_root }),
    );
    assert_eq!(changed_root["ok"], true);
    assert_eq!(
        changed_root["result"]["document"]["editRestrictions"]["identityLocked"],
        true
    );

    let rename = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({ "documentId": "doc-1", "command": { "type": "SetId", "id": "retroarch" } }),
    );
    assert_eq!(rename["ok"], false);
    let current = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(
        current["result"]["document"]["appDefinition"]["id"],
        "Invalid_ID"
    );
}

#[test]
fn physical_catalog_owner_keeps_identity_and_dependency_admission_when_another_root_is_selected() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let owner = directory.path().join("owner");
    let selected = directory.path().join("selected");
    let next_selected = directory.path().join("next-selected");
    for root in [&owner, &selected, &next_selected] {
        fs::create_dir_all(root.join("apps")).expect("apps directory should be created");
        fs::create_dir_all(root.join("recipes")).expect("recipes directory should be created");
    }
    let owner = owner.canonicalize().unwrap();
    let selected = selected.canonicalize().unwrap();
    let next_selected = next_selected.canonicalize().unwrap();
    let path = owner.join("apps/sample.yaml");
    let original = "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\nartifacts:\n  installer:\n    kind: apk\n    source: {strategy: direct_url, url: 'https://example.com/sample.apk'}\n";
    fs::write(&path, original).expect("catalog App Definition should be written");
    fs::write(
        owner.join("recipes/dependent.yaml"),
        "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  installer:\n    type: app_artifact\n    app_ref: sample\n    artifact: missing\n",
    )
    .expect("dependent Recipe should be written");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": path, "authoredRoot": selected }),
    );
    assert_eq!(
        opened["ok"], true,
        "the draft should open with its selected root"
    );
    assert_eq!(
        opened["result"]["document"]["editRestrictions"]["identityLocked"], true,
        "the physical catalog owner must lock identity even when another root is selected"
    );

    let edited = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": "Edited Sample" }
        }),
    );
    assert_eq!(edited["ok"], true);
    let saved = request(
        &mut sessions,
        "saveAppDefinition",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(
        saved["ok"], false,
        "owner Recipe admission must still block Save"
    );
    assert!(saved["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "app_artifact_unknown"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(
        opened["result"]["document"]["owningAuthoredRoot"],
        owner.to_string_lossy().as_ref()
    );
    assert_eq!(
        opened["result"]["document"]["authoredRoot"],
        selected.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    for root in [&selected, &next_selected] {
        fs::write(root.join("recipes/unrelated.yaml"), "schema_version: 1\nkind: recipe\nid: unrelated\nname: Unrelated\nartifacts:\n  invalid:\n    type: app_artifact\n    app_ref: unrelated\n    artifact: missing\n").unwrap();
    }
    let changed_root = request(
        &mut sessions,
        "setDocumentAuthoredRoot",
        json!({"documentId": "doc-1", "authoredRoot": next_selected}),
    );
    assert_eq!(changed_root["ok"], true);
    let changed = &changed_root["result"]["document"];
    assert_eq!(
        changed["owningAuthoredRoot"],
        owner.to_string_lossy().as_ref()
    );
    assert_eq!(
        changed["authoredRoot"],
        next_selected.to_string_lossy().as_ref()
    );
    assert_eq!(changed["editRestrictions"]["identityLocked"], true);
    for command in [
        json!({"type": "SetId", "id": "renamed"}),
        json!({"type": "SetPackageId", "packageId": "com.example.renamed"}),
    ] {
        let rejected = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({"documentId": "doc-1", "command": command}),
        );
        assert_eq!(rejected["ok"], false);
    }
    let rejected_save = request(
        &mut sessions,
        "saveAppDefinition",
        json!({"documentId": "doc-1"}),
    );
    assert_eq!(rejected_save["ok"], false);
    assert!(
        rejected_save["error"]["details"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_artifact_unknown"
                && diagnostic["file"] == "recipes/dependent.yaml"
                && diagnostic["objectId"] == "installer"),
        "{rejected_save}"
    );
    fs::write(owner.join("recipes/dependent.yaml"), "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  installer:\n    type: app_artifact\n    app_ref: sample\n    artifact: installer\n").unwrap();
    let saved = request(
        &mut sessions,
        "saveAppDefinition",
        json!({"documentId": "doc-1"}),
    );
    assert_eq!(
        saved["ok"], true,
        "selected catalog errors must not block the owner's valid save: {saved}"
    );
    assert_eq!(
        saved["result"]["document"]["owningAuthoredRoot"],
        owner.to_string_lossy().as_ref()
    );
    assert_eq!(
        saved["result"]["document"]["authoredRoot"],
        next_selected.to_string_lossy().as_ref()
    );
    assert_eq!(
        saved["result"]["document"]["path"],
        path.to_string_lossy().as_ref()
    );
    assert!(fs::read_to_string(&path).unwrap().contains("Edited Sample"));
    assert!(!selected.join("apps/sample.yaml").exists());
    assert!(!next_selected.join("apps/sample.yaml").exists());
}

#[cfg(unix)]
#[test]
fn app_open_rejects_a_symlinked_selected_root_apps_directory() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let selected_root = directory.path().join("selected");
    let redirected_apps = directory.path().join("redirected-apps");
    fs::create_dir_all(&selected_root).expect("selected root should be created");
    fs::create_dir_all(selected_root.join("recipes")).expect("recipes should be created");
    fs::create_dir_all(&redirected_apps).expect("redirected directory should be created");
    symlink(&redirected_apps, selected_root.join("apps"))
        .expect("apps directory should link to external location");
    fs::write(
        redirected_apps.join("sample.yaml"),
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("App Definition should be written");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({
            "path": selected_root.join("apps/sample.yaml"),
            "authoredRoot": selected_root,
        }),
    );

    assert_eq!(opened["ok"], false);
    assert!(opened["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "app_catalog_path_invalid"));
    assert!(opened["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("symbolic link"))));
    let no_session = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(no_session["ok"], false);
}

#[cfg(unix)]
#[test]
fn app_open_rejects_a_symlinked_physical_apps_directory_despite_another_selected_root() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let physical_root = directory.path().join("physical-owner");
    let redirected_apps = directory.path().join("redirected-apps");
    let selected_root = directory.path().join("selected");
    fs::create_dir_all(physical_root.join("recipes")).expect("physical root should be created");
    fs::create_dir_all(&redirected_apps).expect("redirected directory should be created");
    fs::create_dir_all(selected_root.join("apps")).expect("selected apps should be created");
    fs::create_dir_all(selected_root.join("recipes")).expect("selected recipes should be created");
    symlink(&redirected_apps, physical_root.join("apps"))
        .expect("physical apps directory should link to external location");
    let app_path = redirected_apps.join("sample.yaml");
    fs::write(
        &app_path,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("App Definition should be written");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({
            "path": physical_root.join("apps/sample.yaml"),
            "authoredRoot": selected_root,
        }),
    );

    assert_eq!(opened["ok"], false);
    assert!(opened["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "app_catalog_path_invalid"));
    assert!(opened["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("symbolic link"))));
    let no_session = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(no_session["ok"], false);
}

#[test]
fn app_save_as_requires_selected_root_and_rejects_a_different_destination_root() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let source = directory.path().join("standalone.yaml");
    fs::write(
        &source,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("standalone App Definition should be written");
    let selected = directory.path().join("selected");
    let other = directory.path().join("other");
    for root in [&selected, &other] {
        fs::create_dir_all(root.join("apps")).expect("apps directory should be created");
        fs::create_dir_all(root.join("recipes")).expect("recipes directory should be created");
    }
    let selected_destination = selected.join("apps/sample.yaml");
    let other_destination = other.join("apps/sample.yaml");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": source }),
    );
    assert_eq!(opened["ok"], true);

    let missing_root = request(
        &mut sessions,
        "saveAppDefinitionAs",
        json!({ "documentId": "doc-1", "path": other_destination }),
    );
    assert_eq!(missing_root["ok"], false);
    assert!(missing_root["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Choose an authored root"));
    assert!(!other.join("apps/sample.yaml").exists());
    assert_eq!(
        request(
            &mut sessions,
            "getDocument",
            json!({ "documentId": "doc-1" })
        )["result"]["document"]["authoredRoot"],
        Value::Null,
        "the destination picker must not implicitly select its parent root"
    );

    let selected_root = request(
        &mut sessions,
        "setDocumentAuthoredRoot",
        json!({ "documentId": "doc-1", "authoredRoot": selected }),
    );
    assert_eq!(selected_root["ok"], true);
    let mismatched = request(
        &mut sessions,
        "saveAppDefinitionAs",
        json!({ "documentId": "doc-1", "path": other_destination }),
    );
    assert_eq!(mismatched["ok"], false);
    assert!(mismatched["error"]["message"]
        .as_str()
        .unwrap()
        .contains("currently selected authored root"));
    assert!(!other.join("apps/sample.yaml").exists());

    let traversal = request(
        &mut sessions,
        "saveAppDefinitionAs",
        json!({
            "documentId": "doc-1",
            "path": selected.join("apps/../apps/sample.yaml")
        }),
    );
    assert_eq!(traversal["ok"], false);
    assert!(!selected.join("apps/sample.yaml").exists());

    let saved = request(
        &mut sessions,
        "saveAppDefinitionAs",
        json!({ "documentId": "doc-1", "path": selected_destination }),
    );
    assert_eq!(
        saved["ok"], true,
        "Save As to the selected root should succeed: {saved}"
    );
    assert!(selected.join("apps/sample.yaml").exists());
    assert_eq!(
        saved["result"]["document"]["owningAuthoredRoot"],
        selected.canonicalize().unwrap().to_string_lossy().as_ref()
    );
}

#[test]
fn app_save_as_availability_requires_an_existing_directory_root() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("standalone.yaml");
    fs::write(&source, "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n").unwrap();
    let disk_before = fs::read(&source).unwrap();
    let valid_root = directory.path().join("valid");
    fs::create_dir_all(valid_root.join("apps")).unwrap();
    fs::create_dir_all(valid_root.join("recipes")).unwrap();
    let invalid_file = directory.path().join("file");
    fs::write(&invalid_file, "not a directory").unwrap();
    let absent_root = directory.path().join("absent");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({"path": source, "authoredRoot": valid_root}),
    );
    assert_eq!(opened["ok"], true);
    let baseline = &opened["result"]["document"];
    assert_eq!(baseline["owningAuthoredRoot"], Value::Null);
    assert_eq!(baseline["editRestrictions"]["canSaveAs"], true);
    for root in [&invalid_file, &absent_root] {
        let changed = request(
            &mut sessions,
            "setDocumentAuthoredRoot",
            json!({"documentId": "doc-1", "authoredRoot": root}),
        );
        assert_eq!(changed["ok"], true);
        let document = &changed["result"]["document"];
        assert_eq!(document["editRestrictions"]["canSaveAs"], false);
        assert_eq!(document["owningAuthoredRoot"], Value::Null);
        for field in [
            "appDefinition",
            "yaml",
            "path",
            "dirty",
            "canUndo",
            "canRedo",
        ] {
            assert_eq!(
                document[field], baseline[field],
                "invalid root changed {field}"
            );
        }
        let rejected = request(
            &mut sessions,
            "saveAppDefinitionAs",
            json!({"documentId": "doc-1", "path": root.join("apps/sample.yaml")}),
        );
        assert_eq!(rejected["ok"], false);
        assert!(!root.join("apps/sample.yaml").exists());
        assert_eq!(fs::read(&source).unwrap(), disk_before);
    }
    let restored = request(
        &mut sessions,
        "setDocumentAuthoredRoot",
        json!({"documentId": "doc-1", "authoredRoot": valid_root}),
    );
    assert_eq!(
        restored["result"]["document"]["editRestrictions"]["canSaveAs"],
        true
    );
    assert_eq!(
        restored["result"]["document"]["owningAuthoredRoot"],
        Value::Null
    );
}

#[cfg(unix)]
#[test]
fn app_save_as_availability_rejects_symlinked_catalog_directories() {
    for linked_directory in ["apps", "recipes"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("standalone.yaml");
        fs::write(&source, "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n").unwrap();
        let root = directory.path().join("selected");
        let outside = directory.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let real_directory = if linked_directory == "apps" {
            "recipes"
        } else {
            "apps"
        };
        fs::create_dir_all(root.join(real_directory)).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(linked_directory)).unwrap();
        let mut sessions = DocumentSessionManager::default();
        let opened = request(&mut sessions, "openAppDefinition", json!({"path": source}));
        assert_eq!(opened["ok"], true);
        let selected = request(
            &mut sessions,
            "setDocumentAuthoredRoot",
            json!({"documentId": "doc-1", "authoredRoot": root}),
        );
        assert_eq!(
            selected["result"]["document"]["editRestrictions"]["canSaveAs"],
            false
        );
        assert_eq!(
            selected["result"]["document"]["owningAuthoredRoot"],
            Value::Null
        );
        assert_eq!(
            selected["result"]["document"]["appDefinition"],
            opened["result"]["document"]["appDefinition"]
        );
        assert!(!outside.join("sample.yaml").exists());
        fs::remove_file(root.join(linked_directory)).unwrap();
        fs::create_dir_all(root.join(linked_directory)).unwrap();
        let corrected = request(
            &mut sessions,
            "setDocumentAuthoredRoot",
            json!({"documentId": "doc-1", "authoredRoot": root}),
        );
        assert_eq!(
            corrected["result"]["document"]["editRestrictions"]["canSaveAs"],
            true
        );
        let refreshed = request(&mut sessions, "getDocument", json!({"documentId": "doc-1"}));
        assert_eq!(
            refreshed["result"]["document"]["editRestrictions"]["canSaveAs"],
            true
        );
        assert_eq!(
            refreshed["result"]["document"]["appDefinition"],
            opened["result"]["document"]["appDefinition"]
        );
    }
}

#[test]
fn app_save_as_availability_requires_both_catalog_directories_and_refreshes_after_correction() {
    for missing_directory in ["apps", "recipes"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("standalone.yaml");
        fs::write(&source, "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n").unwrap();
        let disk_before = fs::read(&source).unwrap();
        let root = directory.path().join("selected");
        let existing_directory = if missing_directory == "apps" {
            "recipes"
        } else {
            "apps"
        };
        fs::create_dir_all(root.join(existing_directory)).unwrap();
        let mut sessions = DocumentSessionManager::default();
        let opened = request(
            &mut sessions,
            "openAppDefinition",
            json!({"path": source, "authoredRoot": root}),
        );
        assert_eq!(opened["ok"], true);
        let baseline = &opened["result"]["document"];
        assert_eq!(baseline["editRestrictions"]["canSaveAs"], false);
        let repeated = request(
            &mut sessions,
            "setDocumentAuthoredRoot",
            json!({"documentId": "doc-1", "authoredRoot": root}),
        );
        assert_eq!(
            repeated["result"]["document"]["editRestrictions"]["canSaveAs"],
            false
        );
        let refreshed = request(&mut sessions, "getDocument", json!({"documentId": "doc-1"}));
        assert_eq!(
            refreshed["result"]["document"]["editRestrictions"]["canSaveAs"],
            false
        );
        fs::create_dir_all(root.join(missing_directory)).unwrap();
        let corrected = request(
            &mut sessions,
            "setDocumentAuthoredRoot",
            json!({"documentId": "doc-1", "authoredRoot": root}),
        );
        assert_eq!(
            corrected["result"]["document"]["editRestrictions"]["canSaveAs"],
            true
        );
        let refreshed = request(&mut sessions, "getDocument", json!({"documentId": "doc-1"}));
        let document = &refreshed["result"]["document"];
        assert_eq!(document["editRestrictions"]["canSaveAs"], true);
        assert_eq!(document["owningAuthoredRoot"], Value::Null);
        for field in [
            "appDefinition",
            "yaml",
            "path",
            "dirty",
            "canUndo",
            "canRedo",
        ] {
            assert_eq!(
                document[field], baseline[field],
                "corrected directories changed {field}"
            );
        }
        assert_eq!(fs::read(source).unwrap(), disk_before);
        assert!(!root.join("apps/sample.yaml").exists());
    }
}

#[test]
fn rejected_referenced_artifact_removal_keeps_document_diagnostics_unchanged() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let root = directory.path().join("authored");
    fs::create_dir_all(root.join("apps")).expect("apps directory should be created");
    fs::create_dir_all(root.join("recipes")).expect("recipes directory should be created");
    let path = root.join("apps/sample.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\nartifacts:\n  installer:\n    kind: apk\n    source:\n      strategy: user_provided\n",
    )
    .expect("App Definition should be written");
    fs::write(
        root.join("recipes/dependent.yaml"),
        "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  installer:\n    type: app_artifact\n    app_ref: sample\n    artifact: installer\n",
    )
    .expect("dependent Recipe should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": path, "authoredRoot": root }),
    );
    assert_eq!(opened["ok"], true);
    let before = opened["result"]["document"].clone();

    let rejected = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "RemoveArtifact", "artifactId": "installer" }
        }),
    );
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"]["details"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "app_artifact_unknown"));

    let after = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    let after = &after["result"]["document"];
    assert_eq!(after["yaml"], before["yaml"]);
    assert_eq!(after["diagnostics"], before["diagnostics"]);
    assert_eq!(after["dirty"], before["dirty"]);
    assert_eq!(after["canUndo"], before["canUndo"]);
    assert_eq!(after["canRedo"], before["canRedo"]);
    assert_eq!(after["appDefinition"], before["appDefinition"]);
}

#[test]
fn referenced_target_removal_in_conditions_is_atomic() {
    for condition_section in ["skip_if", "verify"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("authored");
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("recipes")).unwrap();
        let path = root.join("apps/sample.yaml");
        fs::write(&path, "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\ntargets:\n  save:\n    kind: file\n    location: app_data\n    path: save.dat\n").unwrap();
        fs::write(root.join("recipes/dependent.yaml"), format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nsteps:\n  - id: launch\n    name: Launch\n    type: launch_app\n    user_toggleable: false\n    app_ref: sample\n    {condition_section}:\n      - type: path_exists\n        params:\n          path: {{app_target: save}}\n")).unwrap();
        let mut sessions = DocumentSessionManager::default();
        let opened = request(
            &mut sessions,
            "openAppDefinition",
            json!({"path": path, "authoredRoot": root}),
        );
        assert_eq!(opened["ok"], true);
        let before = opened["result"]["document"].clone();
        let rejected = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({"documentId": "doc-1", "command": {"type": "RemoveTarget", "targetId": "save"}}),
        );
        assert_eq!(rejected["ok"], false, "{condition_section}: {rejected}");
        assert!(rejected["error"]["details"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_target_unknown"));
        let after = request(&mut sessions, "getDocument", json!({"documentId": "doc-1"}));
        assert_eq!(after["result"]["document"], before);
        assert!(path.exists());
    }
}

#[test]
fn unreferenced_entries_can_be_removed_despite_other_invalid_references() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("authored");
    fs::create_dir_all(root.join("apps")).unwrap();
    fs::create_dir_all(root.join("recipes")).unwrap();
    let path = root.join("apps/sample.yaml");
    fs::write(&path, "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\nartifacts:\n  unused:\n    kind: apk\n    source: {strategy: user_provided}\ntargets:\n  unused:\n    kind: file\n    location: app_data\n    path: unused.dat\n").unwrap();
    let disk_before = fs::read(&path).unwrap();
    fs::write(root.join("recipes/dependent.yaml"), "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  missing:\n    type: app_artifact\n    app_ref: sample\n    artifact: missing\nsteps:\n  - id: launch\n    name: Launch\n    type: launch_app\n    user_toggleable: false\n    app_ref: sample\n    skip_if:\n      - type: path_exists\n        params:\n          path: {app_target: missing}\n").unwrap();
    let mut sessions = DocumentSessionManager::default();
    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({"path": path, "authoredRoot": root}),
    );
    assert_eq!(opened["ok"], true);
    for command in [
        json!({"type": "RemoveArtifact", "artifactId": "unused"}),
        json!({"type": "RemoveTarget", "targetId": "unused"}),
    ] {
        let edited = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({"documentId": "doc-1", "command": command}),
        );
        assert_eq!(edited["ok"], true, "{edited}");
        assert_eq!(edited["result"]["document"]["dirty"], true);
        assert_eq!(edited["result"]["document"]["canUndo"], true);
    }
    assert_eq!(fs::read(path).unwrap(), disk_before);
}

#[test]
fn permission_actions_cannot_be_duplicated_across_baseline_and_elevated_sets() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("app.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("App Definition should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], true);

    let baseline = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": {
                "type": "SetPermissionSet",
                "permissionSet": "baseline",
                "value": { "runtime": [{ "permission": "android.permission.CAMERA" }] }
            }
        }),
    );
    assert_eq!(baseline["ok"], true);
    let before = baseline["result"]["document"].clone();

    let duplicate = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": {
                "type": "SetPermissionSet",
                "permissionSet": "elevated",
                "value": { "runtime": [{ "permission": "android.permission.CAMERA" }] }
            }
        }),
    );
    assert_eq!(duplicate["ok"], false);
    let after = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    let after = &after["result"]["document"];
    assert_eq!(after["yaml"], before["yaml"]);
    assert_eq!(after["canUndo"], before["canUndo"]);
    assert_eq!(
        after["appDefinition"]["permission_sets"]["elevated"],
        Value::Null
    );
}

#[test]
fn malformed_app_definition_cannot_open_into_a_session() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("malformed.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nunknown: true\n",
    )
    .expect("malformed app definition should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], false);
    let missing = request(
        &mut sessions,
        "getDocument",
        json!({ "documentId": "doc-1" }),
    );
    assert_eq!(missing["ok"], false);
}

#[test]
fn incompatible_app_definition_headers_fail_open_with_field_diagnostics() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    for (file_name, source, expected_code, expected_field, message_fragment) in [
        (
            "wrong-kind.yaml",
            "schema_version: 1\nkind: recipe\nid: sample\nname: Sample\n",
            "authored_kind_invalid",
            "kind",
            "Expected kind 'app_definition'",
        ),
        (
            "unsupported-version.yaml",
            "schema_version: 2\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
            "schema_version_unsupported",
            "schema_version",
            "Unsupported App Definition schema_version",
        ),
    ] {
        let path = directory.path().join(file_name);
        fs::write(&path, source).expect("incompatible App Definition should be written");
        let mut sessions = DocumentSessionManager::default();
        let opened = request(
            &mut sessions,
            "openAppDefinition",
            json!({ "path": path }),
        );

        assert_eq!(opened["ok"], false, "incompatible header opened: {opened}");
        assert!(opened["error"]["message"]
            .as_str()
            .unwrap()
            .contains("opens only App Definitions with kind 'app_definition' and schema_version 1"));
        let diagnostic = opened["error"]["details"]["diagnostics"][0].clone();
        assert_eq!(diagnostic["code"], expected_code);
        assert!(diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(message_fragment));
        assert_eq!(diagnostic["field"], expected_field);
        assert_eq!(diagnostic["objectKind"], "app_definition");
        assert_eq!(diagnostic["objectId"], "sample");
        assert!(diagnostic["file"].as_str().unwrap().ends_with(file_name));

        let no_session = request(
            &mut sessions,
            "getDocument",
            json!({ "documentId": "doc-1" }),
        );
        assert_eq!(no_session["ok"], false);
    }
}

#[test]
fn draft_history_never_replaces_the_opened_raw_byte_baseline() {
    use emuchef_rust_backend::app_definition_document::AppDefinitionDocument;

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("invalid-app.yaml");
    let original = b"schema_version: 1\r\nkind: app_definition\r\nid: retroarch\r\nname: ' '\r\npackage_id: com.retroarch.aarch64\r\n";
    fs::write(&path, original).expect("invalid app definition should be written");
    let mut document = AppDefinitionDocument::open(&path, None).unwrap();
    assert_eq!(document.opened_bytes(), original);

    document
        .apply_command(&json!({ "type": "SetName", "name": "RetroArch" }))
        .unwrap();
    assert_eq!(document.opened_bytes(), original);
    assert!(document.can_undo());
    assert!(document.undo());
    assert_eq!(document.opened_bytes(), original);
    assert!(document.redo());
    assert_eq!(document.opened_bytes(), original);
}

#[test]
fn app_document_operations_are_advertised_by_the_sidecar_protocol() {
    let mut sessions = DocumentSessionManager::default();
    let hello = handle_sidecar_value(json!({ "id": "hello", "type": "hello" }), &mut sessions);
    assert_eq!(hello["ok"], true);
    let capabilities = hello["result"]["capabilities"].as_array().unwrap();
    for capability in ["openAppDefinition", "applyAppDefinitionCommand"] {
        assert!(capabilities.iter().any(|value| value == capability));
    }
}

#[test]
fn entry_commands_repair_independently_of_invalid_siblings() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("invalid-siblings.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: retroarch\nname: RetroArch\npackage_id: com.retroarch.aarch64\nartifacts:\n  bad_source:\n    kind: apk\n    source: {strategy: direct_url, url: 'http://invalid'}\ntargets:\n  bad_target:\n    kind: file\n    location: app_data\n    path: ../invalid\npermission_sets:\n  baseline: {}\nmetadata:\n  id: reserved\n  name: reserved\n",
    )
    .expect("semantically invalid app should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], true);

    let add_artifact = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": {
                "type": "AddArtifact",
                "artifactId": "user_apk",
                "artifact": { "kind": "apk", "source": { "strategy": "user_provided" } }
            }
        }),
    );
    assert_eq!(
        add_artifact["ok"], true,
        "an invalid sibling must not block a valid addition"
    );

    let set_elevated = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": {
                "type": "SetPermissionSet",
                "permissionSet": "elevated",
                "value": { "runtime": [{ "permission": "android.permission.POST_NOTIFICATIONS" }] }
            }
        }),
    );
    assert_eq!(
        set_elevated["ok"], true,
        "an invalid baseline set must not block an elevated-set edit"
    );

    let remove_first_reserved = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "RemoveMetadataEntry", "key": "id" }
        }),
    );
    assert_eq!(remove_first_reserved["ok"], true);
    assert_eq!(
        remove_first_reserved["result"]["document"]["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    let remove_second_reserved = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "RemoveMetadataEntry", "key": "name" }
        }),
    );
    assert_eq!(remove_second_reserved["ok"], true);
    assert_eq!(
        remove_second_reserved["result"]["document"]["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn same_code_invalid_artifact_and_target_replacements_are_rejected_atomically() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("invalid-entries.yaml");
    fs::write(
        &path,
        "schema_version: 1\nkind: app_definition\nid: retroarch\nname: RetroArch\npackage_id: com.retroarch.aarch64\nartifacts:\n  bad_source:\n    kind: apk\n    source: {strategy: direct_url, url: 'http://invalid'}\ntargets:\n  bad_target:\n    kind: file\n    location: app_data\n    path: ../invalid\n",
    )
    .expect("semantically invalid app should be written");
    let mut sessions = DocumentSessionManager::default();
    let opened = request(&mut sessions, "openAppDefinition", json!({ "path": path }));
    assert_eq!(opened["ok"], true);
    let before_yaml = opened["result"]["document"]["yaml"].clone();

    for command in [
        json!({
            "type": "ReplaceArtifact",
            "artifactId": "bad_source",
            "artifact": { "kind": "apk", "source": { "strategy": "direct_url", "url": "ftp://still-invalid" } }
        }),
        json!({
            "type": "ReplaceTarget",
            "targetId": "bad_target",
            "target": { "kind": "file", "location": "app_data", "path": "../still-invalid" }
        }),
    ] {
        let response = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({ "documentId": "doc-1", "command": command }),
        );
        assert_eq!(response["ok"], false);
        let current = request(
            &mut sessions,
            "getDocument",
            json!({ "documentId": "doc-1" }),
        );
        assert_eq!(current["result"]["document"]["yaml"], before_yaml);
        assert_eq!(current["result"]["document"]["canUndo"], false);
    }
}

#[test]
fn save_as_discards_history_that_could_restore_a_different_catalog_identity() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let source = directory.path().join("standalone.yaml");
    fs::write(
        &source,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("standalone App Definition should be written");
    let root = directory.path().join("authored");
    fs::create_dir_all(root.join("apps")).expect("apps directory should be created");
    fs::create_dir_all(root.join("recipes")).expect("recipes directory should be created");
    let destination = root.join("apps/sample.yaml");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": source }),
    );
    assert_eq!(opened["ok"], true);
    let selected_root = request(
        &mut sessions,
        "setDocumentAuthoredRoot",
        json!({ "documentId": "doc-1", "authoredRoot": root }),
    );
    assert_eq!(selected_root["ok"], true);

    for command in [
        json!({ "type": "SetName", "name": "Published Sample" }),
        json!({ "type": "SetId", "id": "temporary" }),
        json!({ "type": "SetId", "id": "sample" }),
        json!({ "type": "SetPackageId", "packageId": "com.example.temporary" }),
    ] {
        let changed = request(
            &mut sessions,
            "applyAppDefinitionCommand",
            json!({ "documentId": "doc-1", "command": command }),
        );
        assert_eq!(changed["ok"], true, "command failed: {changed}");
    }
    let undone_package = request(&mut sessions, "undo", json!({ "documentId": "doc-1" }));
    assert_eq!(undone_package["ok"], true);
    assert_eq!(
        undone_package["result"]["document"]["appDefinition"]["package_id"],
        "com.example.sample"
    );

    let saved = request(
        &mut sessions,
        "saveAppDefinitionAs",
        json!({ "documentId": "doc-1", "path": destination }),
    );
    assert_eq!(saved["ok"], true, "Save As failed: {saved}");
    assert_eq!(
        saved["result"]["document"]["editRestrictions"]["identityLocked"],
        true
    );
    assert_eq!(
        saved["result"]["document"]["canRedo"], false,
        "Redo must not restore the pre-publication package identity"
    );

    for operation in ["undo", "undo", "redo", "redo"] {
        let response = request(&mut sessions, operation, json!({ "documentId": "doc-1" }));
        assert_eq!(response["ok"], true);
        let app = &response["result"]["document"]["appDefinition"];
        assert_eq!(app["id"], "sample");
        assert_eq!(app["package_id"], "com.example.sample");
    }
}

#[test]
fn app_diagnostic_locations_are_camel_case_on_open_edit_and_validate() {
    fn assert_app_target_location(diagnostics: &Value) {
        let diagnostics = diagnostics
            .as_array()
            .expect("diagnostics should be an array");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == "app_target_unknown"
                && diagnostic["objectKind"] == "step"
                && diagnostic["objectId"] == "launch"
                && diagnostic.get("object_kind").is_none()
                && diagnostic.get("object_id").is_none()
        }));
        for diagnostic in diagnostics {
            assert!(diagnostic.get("object_kind").is_none());
            assert!(diagnostic.get("object_id").is_none());
        }
    }

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let root = directory.path().join("authored");
    fs::create_dir_all(root.join("apps")).expect("apps directory should be created");
    fs::create_dir_all(root.join("recipes")).expect("recipes directory should be created");
    let app_path = root.join("apps/sample.yaml");
    fs::write(
        &app_path,
        "schema_version: 1\nkind: app_definition\nid: sample\nname: Sample\npackage_id: com.example.sample\n",
    )
    .expect("App Definition should be written");
    fs::write(
        root.join("recipes/dependent.yaml"),
        "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nsteps:\n  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: sample\n    skip_if:\n      - type: path_exists\n        params:\n          path:\n            app_target: missing_dest\n",
    )
    .expect("dependent Recipe should be written");
    let mut sessions = DocumentSessionManager::default();

    let opened = request(
        &mut sessions,
        "openAppDefinition",
        json!({ "path": app_path, "authoredRoot": root }),
    );
    assert_eq!(opened["ok"], true);
    assert_app_target_location(&opened["result"]["document"]["diagnostics"]);

    let edited = request(
        &mut sessions,
        "applyAppDefinitionCommand",
        json!({
            "documentId": "doc-1",
            "command": { "type": "SetName", "name": "Edited Sample" }
        }),
    );
    assert_eq!(edited["ok"], true);
    assert_app_target_location(&edited["result"]["document"]["diagnostics"]);

    let validated = request(&mut sessions, "validate", json!({ "documentId": "doc-1" }));
    assert_eq!(validated["ok"], true);
    assert_app_target_location(&validated["result"]["diagnostics"]);
}

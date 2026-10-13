//! Save admission and persistence tests for typed App Definition documents.
//!
//! The suite exercises the registered backend modules and the same scoped
//! Recipe validators used by execution planning.

use emuchef_rust_backend::{app_definition_admission, app_definition_persistence, authored_models};

use std::fs;
use std::path::{Path, PathBuf};

use app_definition_admission::{
    admit_app_definition_save, AppDefinitionCatalogIdentity, AppDefinitionSaveContext,
    AppDefinitionSaveDestination,
};
use app_definition_persistence::{capture_opened_app_definition_file, persist_app_definition};
use authored_models::{
    AppArtifactKind, AppArtifactSource, AppArtifactV1, AppDefinitionV1, AppOpAction, AppOpMode,
    AppPermissionSet, AppPermissionSets, AppRuntimePermissionAction, AppTargetKind,
    AppTargetLocation, AppTargetV1, OrderedValueMap, APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
};
use indexmap::IndexMap;
use tempfile::TempDir;

fn app(id: &str, package_id: &str) -> AppDefinitionV1 {
    AppDefinitionV1 {
        schema_version: SCHEMA_VERSION_V1,
        kind: APP_DEFINITION_KIND.to_string(),
        id: id.to_string(),
        name: "Example App".to_string(),
        description: None,
        category: None,
        package_id: package_id.to_string(),
        artifacts: IndexMap::new(),
        permission_sets: None,
        targets: IndexMap::new(),
        launcher_activity: None,
        metadata: OrderedValueMap::new(),
    }
}

fn opened_context(path: &Path) -> AppDefinitionSaveContext {
    AppDefinitionSaveContext {
        opened_file: capture_opened_app_definition_file(path).unwrap(),
        authored_root: None,
        opened_catalog_identity: None,
        destination: AppDefinitionSaveDestination::OpenedFile,
    }
}

fn catalog_context(root: &Path, path: &Path, opened: &AppDefinitionV1) -> AppDefinitionSaveContext {
    AppDefinitionSaveContext {
        opened_file: capture_opened_app_definition_file(path).unwrap(),
        authored_root: Some(root.to_path_buf()),
        opened_catalog_identity: Some(AppDefinitionCatalogIdentity {
            id: opened.id.clone(),
            package_id: opened.package_id.clone(),
        }),
        destination: AppDefinitionSaveDestination::OpenedFile,
    }
}

fn save_as_context(root: &Path, opened_path: &Path) -> AppDefinitionSaveContext {
    AppDefinitionSaveContext {
        opened_file: capture_opened_app_definition_file(opened_path).unwrap(),
        authored_root: Some(root.to_path_buf()),
        opened_catalog_identity: None,
        destination: AppDefinitionSaveDestination::NewCatalogApp,
    }
}

fn catalog() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("apps")).unwrap();
    fs::create_dir(root.path().join("recipes")).unwrap();
    root
}

fn write_app(path: &Path, value: &AppDefinitionV1) {
    fs::write(
        path,
        authored_models::emit_app_definition_yaml(value).unwrap(),
    )
    .unwrap();
}

fn write_recipe(root: &Path, name: &str, source: &str) -> PathBuf {
    let path = root.join("recipes").join(name);
    fs::write(&path, source).unwrap();
    path
}

fn recipe_with_step(step: &str) -> String {
    format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nsteps:\n{step}")
}

fn has_diagnostic(
    admission: &app_definition_admission::AppDefinitionSaveAdmission,
    code: &str,
) -> bool {
    admission
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn admission_for_recipe(
    source: &str,
    target_kind: AppTargetKind,
) -> app_definition_admission::AppDefinitionSaveAdmission {
    let root = catalog();
    let mut opened = app("sample", "com.example.sample");
    opened.targets.insert(
        "save".to_string(),
        AppTargetV1 {
            kind: target_kind,
            location: AppTargetLocation::AppData,
            path: "save".to_string(),
        },
    );
    opened.artifacts.insert(
        "apk".to_string(),
        AppArtifactV1 {
            kind: AppArtifactKind::Apk,
            name: None,
            description: None,
            source: AppArtifactSource::DirectUrl {
                url: "https://example.com/app.apk".to_string(),
                sha256: None,
            },
        },
    );
    opened.permission_sets = Some(AppPermissionSets {
        baseline: Some(AppPermissionSet {
            runtime: vec![AppRuntimePermissionAction {
                permission: "android.permission.POST_NOTIFICATIONS".to_string(),
                android_api_min: Some(33),
                android_api_max: None,
            }],
            app_ops: Vec::new(),
        }),
        elevated: None,
    });
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(root.path(), "dependent.yaml", source);
    admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened))
}

#[test]
fn named_and_primary_file_outputs_use_the_runtime_reference_contract() {
    for source in ["steps.download.outputs.local_path", "steps.download"] {
        let recipe = recipe_with_step(&format!("  - id: download\n    type: download_remote_file\n    name: Download\n    user_toggleable: false\n    params:\n      url: https://example.com/file\n  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: sample\n    params:\n      source: {{ref: {source}}}\n      dest: {{app_target: save}}\n"));
        let admission = admission_for_recipe(&recipe, AppTargetKind::File);
        assert!(
            admission.can_save(),
            "{source}: {:?}",
            admission.diagnostics
        );
    }
}

#[test]
fn multiple_file_inputs_require_a_directory_target() {
    for multiple in [false, true] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\ninputs:\n  source:\n    type: file\n    role: source\n    label: Source\n    multiple: {multiple}\nsteps:\n  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: sample\n    params:\n      source: {{ref: inputs.source}}\n      dest: {{app_target: save}}\n");
        let admission = admission_for_recipe(&recipe, AppTargetKind::File);
        assert_eq!(
            admission.can_save(),
            !multiple,
            "{:?}",
            admission.diagnostics
        );
        if multiple {
            assert!(has_diagnostic(&admission, "app_target_kind_invalid"));
            let directory = admission_for_recipe(&recipe, AppTargetKind::Directory);
            assert!(directory.can_save(), "{:?}", directory.diagnostics);
        }
    }
}

#[test]
fn copy_provenance_is_checked_in_both_app_context_directions() {
    for (artifact_app, context_app) in [("other", "sample"), ("sample", "other")] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: {artifact_app}\n    artifact: apk\nsteps:\n  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: {context_app}\n    params:\n      source: {{ref: artifacts.apk.local_path}}\n      dest: {{app_target: save}}\n");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(!admission.can_save());
        assert!(
            has_diagnostic(&admission, "app_source_provenance_mismatch"),
            "{:?}",
            admission.diagnostics
        );
    }
}

#[test]
fn install_requires_a_resolver_dependency_ancestor_selecting_its_artifact() {
    for dependencies in ["[]", "[resolve]"] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\nartifact_groups:\n  installers: [apk]\nsteps:\n  - id: resolve\n    type: resolve_artifacts\n    name: Resolve\n    user_toggleable: false\n    params:\n      artifact_groups: [installers]\n  - id: install\n    type: install_apk\n    name: Install\n    user_toggleable: false\n    app_ref: sample\n    dependencies: {dependencies}\n    params:\n      app: {{ref: artifacts.apk.local_path}}\n");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert_eq!(
            admission.can_save(),
            dependencies != "[]",
            "{:?}",
            admission.diagnostics
        );
        if dependencies == "[]" {
            assert!(has_diagnostic(
                &admission,
                "app_install_resolution_dependency_missing"
            ));
        }
    }
}

#[test]
fn app_permission_policy_is_required_and_strict_without_device_applicability() {
    for (policy, valid) in [
        ("", false),
        ("      policy: []\n", false),
        ("      policy: {}\n", false),
        ("      policy: {on_failure: ignore}\n", false),
        (
            "      policy: {on_failure: warn, require_all: true}\n",
            false,
        ),
        ("      policy: {on_failure: fail}\n", true),
    ] {
        let recipe = recipe_with_step(&format!("  - id: permissions\n    type: grant_permissions\n    name: Permissions\n    user_toggleable: false\n    app_ref: sample\n    params:\n      permission_set: baseline\n{policy}"));
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert_eq!(
            admission.can_save(),
            valid,
            "{policy}: {:?}",
            admission.diagnostics
        );
        if !valid {
            assert!(
                admission.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.code.as_str(),
                    "missing_required_param" | "invalid_param_value" | "invalid_enum_value"
                )),
                "{:?}",
                admission.diagnostics
            );
            assert!(!has_diagnostic(
                &admission,
                "app_dependency_classification_incomplete"
            ));
        }
    }
}

#[test]
fn app_permission_parameters_exclude_legacy_actions_using_the_step_spec() {
    for parameter in ["runtime", "appops"] {
        let recipe = recipe_with_step(&format!("  - id: permissions\n    type: grant_permissions\n    name: Permissions\n    user_toggleable: false\n    app_ref: sample\n    params:\n      permission_set: baseline\n      policy: {{on_failure: warn}}\n      {parameter}: {{}}\n"));
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(!admission.can_save());
        assert!(
            admission
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unknown_param"
                    && diagnostic.object_id.as_deref() == Some("permissions")
                    && diagnostic.field.as_deref() == Some(parameter)),
            "{:?}",
            admission.diagnostics
        );
        assert!(!has_diagnostic(
            &admission,
            "app_dependency_classification_incomplete"
        ));
    }

    let recipe = recipe_with_step("  - id: permissions\n    type: grant_permissions\n    name: Permissions\n    user_toggleable: false\n    app_ref: sample\n    params:\n      permission_set: baseline\n      policy: {on_failure: warn}\n  - id: unrelated\n    type: grant_permissions\n    name: Unrelated\n    user_toggleable: false\n    app_ref: other_app\n    params:\n      runtime: {}\n      appops: {}\n");
    let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
    assert!(!admission.can_save(), "{:?}", admission.diagnostics);
}

#[test]
#[cfg(unix)]
fn replacement_save_preserves_opened_unix_permission_bits() {
    use std::os::unix::fs::PermissionsExt;

    for mode in [0o640, 0o644, 0o750] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.yaml");
        let opened = app("sample", "com.example.sample");
        write_app(&path, &opened);
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        let original_bytes = fs::read(&path).unwrap();
        let mut edited = opened.clone();
        edited.description = Some("Saved with original permissions".to_string());
        let admission = admit_app_definition_save(&edited, &opened_context(&path));
        assert!(admission.can_save(), "{:?}", admission.diagnostics);

        let saved = persist_app_definition(&edited, &admission).unwrap();

        assert_ne!(fs::read(&path).unwrap(), original_bytes);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            mode
        );
        assert_eq!(saved.raw_bytes(), fs::read(&path).unwrap());
        let context = AppDefinitionSaveContext {
            opened_file: saved,
            authored_root: None,
            opened_catalog_identity: None,
            destination: AppDefinitionSaveDestination::OpenedFile,
        };
        assert!(admit_app_definition_save(&edited, &context).can_save());
    }
}

#[test]
fn install_app_artifact_context_cannot_be_omitted_or_switched_to_another_app() {
    for (context, code) in [
        ("", "app_context_required"),
        ("    app_ref: other_app\n", "app_definition_unknown"),
    ] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\nsteps:\n  - id: resolve\n    type: resolve_artifacts\n    name: Resolve\n    user_toggleable: false\n    params:\n      artifacts: [apk]\n  - id: install\n    type: install_apk\n    name: Install\n    user_toggleable: false\n{context}    dependencies: [resolve]\n    params:\n      app: {{ref: artifacts.apk.local_path}}\n    verify:\n      - type: package_installed\n        params: {{}}\n");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(!admission.can_save());
        assert!(
            has_diagnostic(&admission, code),
            "{:?}",
            admission.diagnostics
        );
        assert!(!has_diagnostic(
            &admission,
            "app_dependency_classification_incomplete"
        ));
    }
}

#[test]
fn missing_install_context_blocks_owned_artifact_install_before_prerequisite_checks() {
    for extra in ["", "      expected_package_name: com.example.override\n"] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\nsteps:\n  - id: install\n    type: install_apk\n    name: Install\n    user_toggleable: false\n    params:\n      app: {{ref: artifacts.apk.local_path}}\n{extra}");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(!admission.can_save());
        assert!(
            has_diagnostic(&admission, "app_context_required"),
            "{:?}",
            admission.diagnostics
        );
        assert!(!has_diagnostic(
            &admission,
            "app_dependency_classification_incomplete"
        ));
    }
}

#[test]
fn app_condition_context_parameters_are_forbidden_in_both_condition_sections() {
    for section in ["skip_if", "verify"] {
        for path in ["{app_target: save}", "/data/local/tmp/example"] {
            let recipe = recipe_with_step(&format!("  - id: stop\n    type: force_stop_app\n    name: Stop\n    user_toggleable: false\n    app_ref: sample\n    {section}:\n      - type: path_exists\n        params:\n          path: {path}\n          app_ref: sample\n"));
            let admission = admission_for_recipe(&recipe, AppTargetKind::File);
            assert!(!admission.can_save());
            assert!(
                admission
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "app_condition_context_inapplicable"),
                "{:?}",
                admission.diagnostics
            );
            assert!(!has_diagnostic(
                &admission,
                "app_dependency_classification_incomplete"
            ));
        }
    }
}

#[test]
fn invalid_other_app_contexts_inside_an_affected_recipe_block_save() {
    let recipe = "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  other_apk:\n    type: app_artifact\n    app_ref: other_app\n    artifact: apk\nsteps:\n  - id: stop\n    type: force_stop_app\n    name: Stop\n    user_toggleable: false\n    app_ref: sample\n  - id: unrelated_install\n    type: install_apk\n    name: Unrelated install\n    user_toggleable: false\n    params:\n      app: {ref: artifacts.other_apk.local_path}\n    verify:\n      - type: package_installed\n        params: {}\n  - id: unrelated_condition\n    type: force_stop_app\n    name: Unrelated condition\n    user_toggleable: false\n    app_ref: other_app\n    verify:\n      - type: path_exists\n        params:\n          path: /data/local/tmp/example\n          app_ref: other_app\n";
    let admission = admission_for_recipe(recipe, AppTargetKind::Directory);
    assert!(!admission.can_save(), "{:?}", admission.diagnostics);
}

#[test]
fn directory_target_copy_requires_a_valid_source_and_policy_contract() {
    for source in [
        "",
        "      source: /data/local/tmp/file\n",
        "      source: {ref: inputs.missing}\n",
        "      source: {ref: artifacts.apk.filename}\n",
        "      source: {ref: malformed}\n",
    ] {
        let recipe = recipe_with_step(&format!("  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: sample\n    params:\n{source}      dest: {{app_target: save}}\n"));
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(
            !admission.can_save(),
            "{source}: {:?}",
            admission.diagnostics
        );
        if source.contains("malformed") {
            assert!(has_diagnostic(
                &admission,
                "app_dependency_classification_incomplete"
            ));
        } else {
            assert!(
                admission.diagnostics.iter().any(|diagnostic| diagnostic
                    .field
                    .as_deref()
                    .is_some_and(|field| field.ends_with("source"))),
                "{:?}",
                admission.diagnostics
            );
        }
    }
    for (policy, valid) in [
        ("", true),
        ("      copy_policy: merge\n", true),
        ("      copy_policy: sync\n", true),
        ("      copy_policy: delete\n", false),
        ("      copy_policy: 42\n", false),
        ("      copy_policy: {ref: inputs.missing}\n", false),
    ] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\nsteps:\n  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: sample\n    params:\n      source: {{ref: artifacts.apk.local_path}}\n      dest: {{app_target: save}}\n{policy}");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert_eq!(
            admission.can_save(),
            valid,
            "{policy}: {:?}",
            admission.diagnostics
        );
    }
}

#[test]
fn invalid_resolver_selections_cannot_satisfy_owned_install_prerequisites() {
    for selector in [
        "      artifacts: [apk, 42]\n",
        "      artifacts: [apk, missing]\n",
        "      artifact_groups: [installers, missing]\n",
        "      artifacts: [apk]\n      artifact_groups: [installers]\n",
        "      artifacts: [apk]\n      artifact_groups: installers\n",
        "      artifacts: [apk]\n      unexpected: true\n",
    ] {
        let recipe = format!("schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  apk:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\nartifact_groups:\n  installers: [apk]\nsteps:\n  - id: resolve\n    type: resolve_artifacts\n    name: Resolve\n    user_toggleable: false\n    params:\n{selector}  - id: install\n    type: install_apk\n    name: Install\n    user_toggleable: false\n    app_ref: sample\n    dependencies: [resolve]\n    params:\n      app: {{ref: artifacts.apk.local_path}}\n");
        let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
        assert!(
            !admission.can_save(),
            "{selector}: {:?}",
            admission.diagnostics
        );
        assert!(
            admission
                .diagnostics
                .iter()
                .any(
                    |diagnostic| diagnostic.object_id.as_deref() == Some("resolve")
                        || diagnostic
                            .field
                            .as_deref()
                            .is_some_and(|field| field.starts_with("steps[0].params"))
                ),
            "{:?}",
            admission.diagnostics
        );
        assert!(!has_diagnostic(
            &admission,
            "app_dependency_classification_incomplete"
        ));
    }
}

#[test]
fn invalid_app_is_rejected_before_any_file_is_published() {
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("app.yaml");
    let original = app("sample", "com.example.sample");
    write_app(&path, &original);
    let original_bytes = fs::read(&path).unwrap();

    let mut invalid = original.clone();
    invalid.package_id = "not a package".to_string();
    let admission = admit_app_definition_save(&invalid, &opened_context(&path));

    assert!(!admission.can_save());
    assert!(!admission.diagnostics.is_empty());
    assert_eq!(fs::read(path).unwrap(), original_bytes);
}

#[test]
fn dependent_reference_queries_use_exact_app_entries_despite_existing_recipe_errors() {
    let root = catalog();
    let mut opened = app("sample", "com.example.sample");
    opened.artifacts.insert(
        "owned_installer".into(),
        AppArtifactV1 {
            kind: AppArtifactKind::Apk,
            name: None,
            description: None,
            source: AppArtifactSource::UserProvided,
        },
    );
    for (id, kind) in [
        ("direct_target", AppTargetKind::Directory),
        ("skip_target", AppTargetKind::Directory),
        ("verify_target", AppTargetKind::File),
    ] {
        opened.targets.insert(
            id.into(),
            AppTargetV1 {
                kind,
                location: AppTargetLocation::AppData,
                path: id.into(),
            },
        );
    }
    let mut edited = opened.clone();
    edited.artifacts.clear();
    edited.targets.clear();
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let recipe_path = write_recipe(root.path(), "dependent.yaml", "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  recipe_binding:\n    type: app_artifact\n    app_ref: sample\n    artifact: owned_installer\n  other_binding:\n    type: app_artifact\n    app_ref: other\n    artifact: other_only\nsteps:\n  - id: copy\n    type: copy_files\n    name: Copy\n    user_toggleable: false\n    app_ref: sample\n    params:\n      source: {ref: inputs.missing}\n      dest: {app_target: direct_target}\n    skip_if:\n      - type: path_exists\n        params: {path: {app_target: skip_target}}\n    verify:\n      - type: file_exists\n        params: {path: {app_target: verify_target}}\n  - id: other\n    type: force_stop_app\n    name: Other\n    user_toggleable: false\n    app_ref: other\n    verify:\n      - type: path_exists\n        params: {path: {app_target: other_target}}\n");
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));
    assert!(admission.snapshot.is_some());
    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "app_artifact_unknown"));
    assert!(admission.has_dependent_artifact_reference("owned_installer"));
    for id in [
        "recipe_binding",
        "other_binding",
        "other_only",
        "unused_installer",
    ] {
        assert!(!admission.has_dependent_artifact_reference(id), "{id}");
    }
    for id in ["direct_target", "skip_target", "verify_target"] {
        assert!(admission.has_dependent_target_reference(id), "{id}");
    }
    for id in ["other_target", "unused_target", "skip"] {
        assert!(!admission.has_dependent_target_reference(id), "{id}");
    }
    // Queries retain precisely the admitted bytes even if an external writer
    // changes the Recipe later. Publication independently rechecks that input.
    fs::write(
        &recipe_path,
        recipe_with_step("  - id: stop\n    type: force_stop_app\n    app_ref: sample\n"),
    )
    .unwrap();
    assert!(admission.has_dependent_artifact_reference("owned_installer"));
    assert!(admission.has_dependent_target_reference("verify_target"));
    let error = persist_app_definition(&edited, &admission).unwrap_err();
    assert_eq!(error.code, "app_definition_save_rejected");
    assert_eq!(
        fs::read(&path).unwrap(),
        authored_models::emit_app_definition_yaml(&opened)
            .unwrap()
            .as_bytes()
    );
}

#[test]
fn all_app_contexts_in_an_affected_recipe_use_captured_definitions() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let mut other = app("other", "com.example.other");
    let other_path = root.path().join("apps/other.yaml");
    write_app(&other_path, &other);
    write_recipe(root.path(), "dependent.yaml", &recipe_with_step("  - id: stop\n    type: force_stop_app\n    name: Stop\n    user_toggleable: false\n    app_ref: sample\n  - id: launch_other\n    type: launch_app\n    name: Launch other\n    user_toggleable: false\n    app_ref: other\n"));
    let context = catalog_context(root.path(), &path, &opened);
    let rejected = admit_app_definition_save(&opened, &context);
    assert!(!rejected.can_save());
    assert!(
        has_diagnostic(&rejected, "app_launcher_activity_required"),
        "{:?}",
        rejected.diagnostics
    );
    other.launcher_activity = Some("com.example.other.MainActivity".into());
    write_app(&other_path, &other);
    let admitted = admit_app_definition_save(&opened, &context);
    assert!(admitted.can_save(), "{:?}", admitted.diagnostics);
    write_app(
        &root.path().join("apps/duplicate.yaml"),
        &app("other", "com.example.duplicate"),
    );
    let ambiguous = admit_app_definition_save(&opened, &context);
    assert!(!ambiguous.can_save());
    assert!(
        has_diagnostic(&ambiguous, "app_id_conflict"),
        "{:?}",
        ambiguous.diagnostics
    );
    assert_eq!(fs::read(&path).unwrap(), context.opened_file.raw_bytes());
}

#[test]
fn local_errors_anywhere_in_a_directly_affected_recipe_block_save() {
    let recipe = recipe_with_step("  - id: stop\n    type: force_stop_app\n    name: Stop\n    user_toggleable: false\n    app_ref: sample\n  - id: other_step\n    type: wait\n    name: Wait\n    user_toggleable: false\n    dependencies: [absent]\n    params: {duration_ms: 1}\n");
    let admission = admission_for_recipe(&recipe, AppTargetKind::Directory);
    assert!(!admission.can_save(), "{:?}", admission.diagnostics);
    assert!(
        has_diagnostic(&admission, "step_not_found"),
        "{:?}",
        admission.diagnostics
    );
    assert!(!has_diagnostic(
        &admission,
        "app_dependency_classification_incomplete"
    ));
}

#[test]
fn unrelated_semantic_errors_do_not_block_a_valid_catalog_save() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    fs::write(
        root.path().join("apps/unrelated.yaml"),
        "schema_version: 1\nkind: app_definition\nid: unrelated\nname: ''\npackage_id: com.example.other\n",
    )
    .unwrap();
    write_recipe(
        root.path(),
        "unrelated.yaml",
        "schema_version: 1\nkind: recipe\nid: unrelated\nname: ''\nsteps:\n  - id: wait\n    type: wait\n    name: Invalid wait\n    user_toggleable: false\n    dependencies: [missing]\n    params: {duration_ms: -1}\n",
    );

    let mut edited = opened.clone();
    edited.description = Some("Updated description".to_string());
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));

    assert!(admission.can_save(), "{:?}", admission.diagnostics);
}

#[test]
fn duplicate_competing_id_or_package_blocks_save() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_app(
        &root.path().join("apps/competitor.yaml"),
        &app("competitor", "com.example.sample"),
    );

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "app_package_id_conflict"));
}

#[test]
fn competing_app_id_with_a_distinct_package_rejects_save_without_changing_disk() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    let competitor_path = root.path().join("apps/competitor.yaml");
    write_app(&path, &opened);
    write_app(&competitor_path, &app("sample", "com.example.competitor"));
    let opened_bytes = fs::read(&path).unwrap();
    let competitor_bytes = fs::read(&competitor_path).unwrap();
    let mut edited = opened.clone();
    edited.description = Some("Proposed update".to_string());

    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(
        admission.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "app_definition_id_conflict"
                && diagnostic.object_id.as_deref() == Some("sample")
                && diagnostic.field.as_deref() == Some("id")
                && diagnostic
                    .file
                    .as_ref()
                    .is_some_and(|file| Path::new(file).ends_with("apps/competitor.yaml"))
        }),
        "{:?}",
        admission.diagnostics
    );
    assert!(!has_diagnostic(&admission, "app_package_id_conflict"));
    assert!(!has_diagnostic(&admission, "app_identity_immutable"));
    let error = persist_app_definition(&edited, &admission).unwrap_err();
    assert_eq!(error.code, "app_definition_save_rejected");
    assert_eq!(fs::read(&path).unwrap(), opened_bytes);
    assert_eq!(fs::read(&competitor_path).unwrap(), competitor_bytes);
}

#[test]
fn direct_app_artifact_reference_must_still_resolve() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(
        root.path(),
        "dependent.yaml",
        "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nartifacts:\n  installer:\n    type: app_artifact\n    app_ref: sample\n    artifact: apk\n",
    );

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "app_artifact_unknown"));
}

#[test]
fn permission_set_existence_is_checked_without_device_api_context() {
    let root = catalog();
    let mut opened = app("sample", "com.example.sample");
    opened.permission_sets = Some(AppPermissionSets {
        baseline: Some(AppPermissionSet {
            runtime: vec![AppRuntimePermissionAction {
                permission: "android.permission.CAMERA".to_string(),
                android_api_min: Some(34),
                android_api_max: Some(36),
            }],
            app_ops: vec![AppOpAction {
                op: "CAMERA".to_string(),
                mode: AppOpMode::Allow,
                android_api_min: None,
                android_api_max: None,
            }],
        }),
        elevated: None,
    });
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(
        root.path(),
        "permissions.yaml",
        &recipe_with_step(
            "  - id: permissions\n    type: grant_permissions\n    name: Grant permissions\n    user_toggleable: false\n    app_ref: sample\n    params:\n      permission_set: elevated\n      policy:\n        on_failure: warn\n",
        ),
    );

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "permission_set_missing"));
}

#[test]
fn existing_static_permission_set_does_not_require_device_api_context() {
    let root = catalog();
    let mut opened = app("sample", "com.example.sample");
    opened.permission_sets = Some(AppPermissionSets {
        baseline: Some(AppPermissionSet {
            runtime: vec![AppRuntimePermissionAction {
                permission: "android.permission.POST_NOTIFICATIONS".to_string(),
                android_api_min: Some(33),
                android_api_max: None,
            }],
            app_ops: Vec::new(),
        }),
        elevated: None,
    });
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(
        root.path(),
        "permissions.yaml",
        &recipe_with_step(
            "  - id: permissions\n    type: grant_permissions\n    name: Grant permissions\n    user_toggleable: false\n    app_ref: sample\n    params:\n      permission_set: baseline\n      policy:\n        on_failure: warn\n",
        ),
    );

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(admission.can_save(), "{:?}", admission.diagnostics);
}

#[test]
fn launcher_and_app_target_condition_references_are_admitted_together() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(
        root.path(),
        "lifecycle-and-target.yaml",
        &recipe_with_step(
            "  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: sample\n    skip_if:\n      - type: path_exists\n        params:\n          path:\n            app_target: save_dir\n",
        ),
    );

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "app_launcher_activity_required"));
    assert!(has_diagnostic(&admission, "app_target_unknown"));
}

#[test]
fn malformed_recipe_prevents_claiming_a_complete_dependency_closure() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    write_recipe(root.path(), "unclassifiable.yaml", "steps: [not a recipe]");

    let admission =
        admit_app_definition_save(&opened, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(admission.snapshot.is_none());
    assert!(has_diagnostic(
        &admission,
        "app_dependency_classification_incomplete"
    ));
}

#[test]
fn changed_recipe_after_admission_rejects_save_without_changing_the_app_file() {
    let root = catalog();
    let mut opened = app("sample", "com.example.sample");
    opened.launcher_activity = Some("com.example.sample.MainActivity".to_string());
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let recipe_path = write_recipe(
        root.path(),
        "dependent.yaml",
        &recipe_with_step(
            "  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: sample\n",
        ),
    );
    let mut edited = opened.clone();
    edited.description = Some("Changed".to_string());
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));
    assert!(admission.can_save(), "{:?}", admission.diagnostics);
    let original_bytes = fs::read(&path).unwrap();
    fs::write(
        recipe_path,
        "schema_version: 1\nkind: recipe\nid: changed\nname: Changed\n",
    )
    .unwrap();

    let error = persist_app_definition(&edited, &admission).unwrap_err();

    assert_eq!(error.code, "app_definition_stale_state");
    assert_eq!(fs::read(path).unwrap(), original_bytes);
}

#[test]
fn changed_competing_app_after_admission_rejects_save_without_overwriting_it() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let competitor = root.path().join("apps/competitor.yaml");
    write_app(&competitor, &app("competitor", "com.example.other"));
    let mut edited = opened.clone();
    edited.description = Some("Changed".to_string());
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));
    assert!(admission.can_save(), "{:?}", admission.diagnostics);
    let original_bytes = fs::read(&path).unwrap();
    write_app(&competitor, &app("competitor", "com.example.sample"));
    let competitor_bytes = fs::read(&competitor).unwrap();

    let error = persist_app_definition(&edited, &admission).unwrap_err();

    assert_eq!(error.code, "app_definition_stale_state");
    assert_eq!(fs::read(path).unwrap(), original_bytes);
    assert_eq!(fs::read(competitor).unwrap(), competitor_bytes);
}

#[test]
fn opened_app_bytes_and_identity_are_rechecked_before_publication() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let mut edited = opened.clone();
    edited.description = Some("Changed".to_string());
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));
    assert!(admission.can_save(), "{:?}", admission.diagnostics);
    let changed_bytes = authored_models::emit_app_definition_yaml(&opened)
        .unwrap()
        .replace("Example App", "Externally changed")
        .into_bytes();
    fs::write(&path, &changed_bytes).unwrap();

    let error = persist_app_definition(&edited, &admission).unwrap_err();

    assert_eq!(error.code, "app_definition_stale_state");
    assert_eq!(fs::read(path).unwrap(), changed_bytes);
}

#[test]
fn new_recipe_after_admission_invalidates_the_inventory_snapshot() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let mut edited = opened.clone();
    edited.description = Some("Changed".to_string());
    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));
    assert!(admission.can_save(), "{:?}", admission.diagnostics);
    let original_bytes = fs::read(&path).unwrap();
    write_recipe(
        root.path(),
        "new-dependent.yaml",
        &recipe_with_step(
            "  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: sample\n",
        ),
    );

    let error = persist_app_definition(&edited, &admission).unwrap_err();

    assert_eq!(error.code, "app_definition_stale_state");
    assert_eq!(fs::read(path).unwrap(), original_bytes);
}

#[test]
fn existing_catalog_identity_cannot_be_changed_through_save() {
    let root = catalog();
    let opened = app("sample", "com.example.sample");
    let path = root.path().join("apps/sample.yaml");
    write_app(&path, &opened);
    let mut edited = opened.clone();
    edited.id = "renamed".to_string();

    let admission =
        admit_app_definition_save(&edited, &catalog_context(root.path(), &path, &opened));

    assert!(!admission.can_save());
    assert!(has_diagnostic(&admission, "app_identity_immutable"));
}

#[test]
fn symlinked_catalog_directory_is_rejected() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = catalog();
        symlink(outside.path().join("apps"), root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let opened = app("sample", "com.example.sample");
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("app.yaml");
        write_app(&path, &opened);

        let admission = admit_app_definition_save(&opened, &save_as_context(root.path(), &path));

        assert!(!admission.can_save());
        assert!(has_diagnostic(&admission, "app_catalog_path_invalid"));
    }
}

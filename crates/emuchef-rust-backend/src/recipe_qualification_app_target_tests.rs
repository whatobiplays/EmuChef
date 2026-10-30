//! Generic protected App Target qualification for T03 copy semantics.
//!
//! The fixture uses a disposable app definition and source-built Android test
//! application. It exercises the authored catalog resolver and normal planner
//! before sending the resulting plan through the trusted executor.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::catalog_source::CatalogSnapshot;
use crate::execution_session::SidecarRuntimeConfig;
use crate::executor::adb::{
    probe_root_with_route, AdbCommandError, AdbCommandExecutor, AdbCommandResult,
    ProcessAdbCommandExecutor, RealAdbDevice, RootProbeOutcome,
};
use crate::executor::{ExecutionRunResult, ExecutorAdapters, ExecutorRunner};
use crate::model::OrderedMap;
use crate::owned_process::ProcessOperation;
use crate::planner::{
    ExecutionParamValue, ExecutionPlan, ExecutionStep, RootShellRoute, TargetDeviceBinding,
};
use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

const APP_ID: &str = "t03.copy_fixture";
const APP_PACKAGE: &str = "com.emuchef.fixture";
const RECIPE_ID: &str = "t03.copy_app_target";
const DEVICE_PLAN_ID: &str = "t03.copy_fixture.base";
const SOURCE_KEY: &str = "t03.copy_app_target/source";
const POLICY_KEY: &str = "t03.copy_app_target/policy";
const APP_TARGET_RELATIVE_PATH: &str = "files/assets";

/// Isolates the selected source tree from preexisting destination fixture data.
///
/// The source contains only the paths selected for the App Target. Existing
/// destination files and the sibling tree live under the separate fake device
/// root so they cannot be staged as source content.
struct QualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    source_root: PathBuf,
}

impl QualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("T03 workspace should be created");
        let source_root = temp.path().join("source");
        fs::create_dir_all(source_root.join("empty-source")).unwrap();
        fs::create_dir_all(source_root.join("nested")).unwrap();
        fs::write(source_root.join("A.rom"), b"A source\n").unwrap();
        fs::write(source_root.join("B.rom"), b"updated B\n").unwrap();
        fs::write(source_root.join("nested/inside.bin"), b"nested payload\n").unwrap();

        let fake_device_root = temp.path().join("fake-device");
        let destination_root = fake_app_target_root(&fake_device_root);
        fs::create_dir_all(destination_root.join("stale-dir")).unwrap();
        fs::create_dir_all(destination_root.join("old-empty")).unwrap();
        fs::create_dir_all(
            fake_device_root
                .join("data/user/0")
                .join(APP_PACKAGE)
                .join("files/sibling"),
        )
        .unwrap();
        fs::write(destination_root.join("B.rom"), b"old B at the same path\n").unwrap();
        fs::write(destination_root.join("C.rom"), b"unrelated C\n").unwrap();
        fs::write(destination_root.join("stale-dir/stale.txt"), b"stale\n").unwrap();
        fs::write(
            fake_device_root
                .join("data/user/0")
                .join(APP_PACKAGE)
                .join("files/sibling/keep.txt"),
            b"outside the selected App Target\n",
        )
        .unwrap();

        Self {
            runtime_root: temp.path().join("runtime"),
            cache_root: temp.path().join("cache"),
            fake_device_root,
            source_root,
            _temp: temp,
        }
    }

    fn destination_root(&self) -> PathBuf {
        fake_app_target_root(&self.fake_device_root)
    }

    fn catalog_root(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("backend crate should be inside the repository")
            .join("tests/fixtures/t03_app_target/authored")
    }
}

fn fake_app_target_root(fake_device_root: &Path) -> PathBuf {
    fake_device_root
        .join("data/user/0")
        .join(APP_PACKAGE)
        .join(APP_TARGET_RELATIVE_PATH)
}

fn resolved_app_target_path(plan: &ExecutionPlan) -> &str {
    let Some(ExecutionParamValue::Literal { value }) =
        plan.steps.first().and_then(|step| step.params.get("dest"))
    else {
        panic!("prepared App Target plan should contain a literal resolved destination");
    };
    value
        .as_str()
        .expect("resolved App Target destination should be a string")
}

fn build_synthetic_fixture(repo_root: &Path) -> (tempfile::TempDir, PathBuf) {
    let output_root = tempfile::tempdir().expect("T03 APK output directory should be created");
    let canonical_repo_root = repo_root
        .canonicalize()
        .expect("repository root should be canonicalized");
    let canonical_output_root = output_root
        .path()
        .canonicalize()
        .expect("T03 APK output directory should be canonicalized");
    assert!(
        !canonical_output_root.starts_with(&canonical_repo_root),
        "generated T03 APK and build temporary files must remain outside Git"
    );
    let apk_path = canonical_output_root.join("t03-app-target-fixture.apk");
    let build_script = canonical_repo_root.join("scripts/build-t03-app-target-fixture.sh");
    let output = Command::new("bash")
        .arg(&build_script)
        .arg("--output")
        .arg(&apk_path)
        .env("TMPDIR", &canonical_output_root)
        .output()
        .unwrap_or_else(|error| panic!("failed to start synthetic T03 APK source build: {error}"));
    assert!(
        output.status.success(),
        "synthetic T03 APK source build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(apk_path.is_file(), "source build should create the APK");
    assert!(
        !apk_path.starts_with(&canonical_repo_root),
        "generated T03 APK must remain outside Git"
    );
    (output_root, apk_path)
}

fn plan_app_target_copy(workspace: &QualificationWorkspace, policy: &str) -> ExecutionPlan {
    plan_app_target_copy_with_target_device(workspace, policy, None)
}

fn plan_app_target_copy_with_target_device(
    workspace: &QualificationWorkspace,
    policy: &str,
    target_device: Option<TargetDeviceBinding>,
) -> ExecutionPlan {
    let mut explicit_bindings = OrderedMap::new();
    explicit_bindings.insert(
        SOURCE_KEY.to_string(),
        Value::String(workspace.source_root.to_string_lossy().into_owned()),
    );
    explicit_bindings.insert(POLICY_KEY.to_string(), Value::String(policy.to_string()));

    let prepared = plan_configuration(ConfigurationContextRequest {
        catalog: CatalogSnapshot::legacy_local(workspace.catalog_root())
            .expect("synthetic T03 catalog should be admitted"),
        configuration_root: None,
        user_configuration: None,
        device_plan: Some(DEVICE_PLAN_ID.to_string()),
        selected_recipes: Some(vec![RECIPE_ID.to_string()]),
        explicit_bindings,
        device_context: None,
        target_device,
        runtime_capability_availability: None,
    })
    .expect("synthetic App Target plan should be prepared");
    assert!(
        prepared.copy_commit_preparation_required,
        "copy plans must announce their required pre-review device inspection"
    );
    assert!(
        prepared.plan_digest.is_none() && prepared.review.is_none(),
        "copy plans must not expose a final digest or review before device snapshots are prepared"
    );

    assert!(
        prepared
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "synthetic App Target planning diagnostics: {:#?}",
        prepared.diagnostics
    );
    let plan = prepared
        .plan
        .expect("synthetic App Target plan should exist");
    let mut plan = plan;
    let adapters = ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
    );
    ExecutorRunner::new(adapters)
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("synthetic App Target copy snapshot should be prepared before review");
    let review = crate::review_projection::project_review(&plan, &prepared.diagnostics);
    assert!(
        review.can_execute,
        "synthetic App Target review should be executable"
    );
    assert_eq!(plan.source.selected_recipe_refs, vec![RECIPE_ID]);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].app_id.as_deref(), Some(APP_ID));
    let destination = resolved_app_target_path(&plan);
    assert!(
        Path::new(destination).is_absolute(),
        "the catalog's app_data target must resolve to a device path"
    );
    assert!(
        destination.ends_with(&format!("{APP_PACKAGE}/{APP_TARGET_RELATIVE_PATH}")),
        "the catalog's App Target should resolve to the package-private assets directory: {destination}"
    );
    assert_eq!(
        plan.steps[0].params.get("dest_kind"),
        Some(&ExecutionParamValue::Literal {
            value: json!("directory")
        })
    );
    assert!(plan.runtime_capabilities.app_data_write);
    assert!(plan.runtime_capabilities.root_shell);
    plan
}

#[test]
fn app_target_planning_retains_the_selected_target_device_binding() {
    let workspace = QualificationWorkspace::new();
    let target_device = TargetDeviceBinding {
        serial: "host-test-device".to_string(),
        manufacturer: Some("Test Manufacturer".to_string()),
        model: Some("Test Model".to_string()),
        android_api_level: Some(36),
    };

    let plan =
        plan_app_target_copy_with_target_device(&workspace, "merge", Some(target_device.clone()));

    assert_eq!(
        plan.target_device.as_ref().map(|selected| (
            selected.serial.as_str(),
            selected.manufacturer.as_deref(),
            selected.model.as_deref(),
            selected.android_api_level,
        )),
        Some((
            target_device.serial.as_str(),
            target_device.manufacturer.as_deref(),
            target_device.model.as_deref(),
            target_device.android_api_level,
        )),
        "the planner must retain all selected device facts on the execution plan"
    );
}

fn run_fake_copy(workspace: &QualificationWorkspace, plan: &ExecutionPlan) -> ExecutionRunResult {
    let result = execute_fake_copy(workspace, plan);
    assert!(result.success, "trusted fake executor result: {result:#?}");
    result
}
fn execute_fake_copy(
    workspace: &QualificationWorkspace,
    plan: &ExecutionPlan,
) -> ExecutionRunResult {
    let adapters = ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
    );
    ExecutorRunner::new(adapters).run(plan)
}

fn entries(root: &Path) -> Vec<String> {
    fn visit(root: &Path, current: &Path, output: &mut Vec<String>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            output.push(relative);
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, output);
            }
        }
    }
    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort();
    output
}

#[test]
fn add_update_fixture_starts_with_the_same_path_b_file() {
    let workspace = QualificationWorkspace::new();

    assert_eq!(
        fs::read(workspace.destination_root().join("B.rom")).unwrap(),
        b"old B at the same path\n",
        "Add/Update must exercise replacing a preexisting file with the same reviewed path"
    );
}

#[test]
fn synthetic_app_target_copy_policies_preserve_the_reviewed_tree_boundary() {
    let expected_source = [
        "A.rom",
        "B.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
    ];
    let expected_merged = [
        "A.rom",
        "B.rom",
        "C.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
        "old-empty",
        "stale-dir",
        "stale-dir/stale.txt",
    ];
    let expected_replaced = [
        "A.rom",
        "B.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
    ];
    let expected_before = [
        "B.rom",
        "C.rom",
        "old-empty",
        "stale-dir",
        "stale-dir/stale.txt",
    ];

    for policy in ["merge", "replace", "sync"] {
        let workspace = QualificationWorkspace::new();
        assert_eq!(
            entries(&workspace.source_root),
            expected_source
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>(),
            "selected source tree for policy={policy}"
        );
        assert!(
            !workspace
                .destination_root()
                .starts_with(&workspace.source_root),
            "destination fixture must be outside the selected source tree"
        );
        let plan = plan_app_target_copy(&workspace, policy);
        let sibling = workspace
            .fake_device_root
            .join("data/user/0")
            .join(APP_PACKAGE)
            .join("files/sibling/keep.txt");

        if policy == "merge" {
            assert_eq!(
                fs::read(workspace.destination_root().join("B.rom")).unwrap(),
                b"old B at the same path\n",
                "Add/Update starts with old bytes at the exact source-relative path"
            );
        }
        let result = run_fake_copy(&workspace, &plan);

        let expected: &[&str] = if policy == "merge" {
            &expected_merged
        } else {
            &expected_replaced
        };
        assert_eq!(
            entries(&workspace.destination_root()),
            expected
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>(),
            "policy={policy}"
        );
        let commit_report = &result.steps[0].outputs["copy_commit_report"].value;
        assert_eq!(
            commit_report["beforePaths"],
            json!(expected_before),
            "reviewed destination manifest for policy={policy}"
        );
        assert_eq!(
            commit_report["afterPaths"],
            json!(expected),
            "verified destination manifest for policy={policy}"
        );
        let expected_deleted: &[&str] = match policy {
            "merge" => &[],
            "replace" => &[
                "B.rom",
                "C.rom",
                "old-empty",
                "stale-dir",
                "stale-dir/stale.txt",
            ],
            "sync" => &["C.rom", "old-empty", "stale-dir", "stale-dir/stale.txt"],
            _ => unreachable!(),
        };
        assert_eq!(
            commit_report["deletedPaths"],
            json!(expected_deleted),
            "reviewed deletion set for policy={policy}"
        );
        assert_eq!(
            fs::read(workspace.destination_root().join("A.rom")).unwrap(),
            b"A source\n"
        );
        assert_eq!(
            fs::read(workspace.destination_root().join("B.rom")).unwrap(),
            b"updated B\n"
        );
        assert_eq!(
            fs::read(workspace.destination_root().join("nested/inside.bin")).unwrap(),
            b"nested payload\n"
        );
        assert_eq!(
            fs::read(workspace.destination_root().join("C.rom")).ok(),
            (policy == "merge").then_some(b"unrelated C\n".to_vec()),
            "unrelated C content for policy={policy}"
        );
        assert!(workspace.destination_root().join("empty-source").is_dir());
        assert_eq!(
            workspace.destination_root().join("old-empty").is_dir(),
            policy == "merge",
            "preexisting empty directory preservation for policy={policy}"
        );
        assert_eq!(
            fs::read(sibling).unwrap(),
            b"outside the selected App Target\n",
            "copy policy {policy} must preserve app-data siblings outside assets"
        );
    }
}

#[test]
fn review_discloses_exact_replace_and_mirror_deletion_paths() {
    for policy in ["replace", "sync"] {
        let workspace = QualificationWorkspace::new();
        fs::write(workspace.destination_root().join("A.rom"), b"old A\n").unwrap();
        fs::create_dir_all(workspace.destination_root().join("nested")).unwrap();
        fs::write(
            workspace.destination_root().join("nested/inside.bin"),
            b"old nested payload\n",
        )
        .unwrap();
        let plan = plan_app_target_copy(&workspace, policy);
        let review =
            serde_json::to_value(crate::review_projection::project_review(&plan, &[])).unwrap();
        let disclosed = &review["features"][0]["sections"][0]["actions"][0]["reviewedDeletionSet"];
        let expected = if policy == "replace" {
            json!([
                "A.rom",
                "B.rom",
                "C.rom",
                "nested",
                "nested/inside.bin",
                "old-empty",
                "stale-dir",
                "stale-dir/stale.txt"
            ])
        } else {
            json!(["C.rom", "old-empty", "stale-dir", "stale-dir/stale.txt"])
        };
        assert_eq!(disclosed, &expected, "reviewed deletion set for {policy}");
    }
}

#[test]
fn changing_reviewed_copy_snapshot_changes_plan_digest() {
    let workspace = QualificationWorkspace::new();
    let plan = plan_app_target_copy(&workspace, "replace");
    let original = crate::plan_digest::execution_plan_digest(&plan).unwrap();

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0].operation =
        crate::executor::copy_commit::CopyCommitOperation::Mirror;
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0]
        .destination_identity
        .push_str("/changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0]
        .selected_root
        .push("changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0].destination_binding = Some("changed-binding".to_string());
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0].source_manifest[0]
        .relative_path
        .push_str(".changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0]
        .source_digest_sha256
        .push_str("changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0].destination_before_manifest[0]
        .relative_path
        .push_str(".changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0]
        .destination_before_digest_sha256
        .push_str("changed");
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );

    let mut changed = plan.clone();
    changed.copy_commit_snapshots[0]
        .reviewed_deletion_set
        .push("different-reviewed-entry".to_string());
    assert_ne!(
        original,
        crate::plan_digest::execution_plan_digest(&changed).unwrap()
    );
}

#[test]
fn execution_uses_the_reviewed_copy_snapshot() {
    let workspace = QualificationWorkspace::new();
    let plan = plan_app_target_copy(&workspace, "replace");
    let snapshot = &plan.copy_commit_snapshots[0];
    let expected_source_digest = snapshot.source_digest_sha256.clone();
    let expected_before_manifest =
        serde_json::to_value(&snapshot.destination_before_manifest).unwrap();
    let expected_deletions = snapshot.reviewed_deletion_set.clone();

    let result = execute_fake_copy(&workspace, &plan);

    assert!(result.success, "trusted fake executor result: {result:#?}");
    let report = &result.steps[0].outputs["copy_commit_report"].value;
    assert_eq!(report["sourceDigestSha256"], json!(expected_source_digest));
    assert_eq!(report["beforeManifest"], expected_before_manifest);
    assert_eq!(report["reviewedDeletionSet"], json!(expected_deletions));
    assert_eq!(report["deletedPaths"], json!(expected_deletions));
}

#[test]
fn new_destination_entry_after_review_is_rejected_before_deletion() {
    let workspace = QualificationWorkspace::new();
    let plan = plan_app_target_copy(&workspace, "replace");
    let new_entry = workspace.destination_root().join("added-after-review.txt");
    fs::write(&new_entry, b"unreviewed\n").unwrap();

    let result = execute_fake_copy(&workspace, &plan);

    assert!(!result.success, "destination drift must reject the copy");
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Failed,
        "destination drift must be reported as a failed execution step"
    );
    assert!(new_entry.exists(), "unreviewed entry must not be deleted");
    assert!(
        workspace.destination_root().join("C.rom").exists(),
        "reviewed destination entries must remain when pre-mutation validation fails"
    );
    assert!(
        workspace
            .destination_root()
            .join("stale-dir/stale.txt")
            .exists(),
        "the reviewed deletion set must not be applied after destination drift"
    );
}

#[test]
fn changed_source_after_review_is_rejected_before_destination_mutation() {
    let workspace = QualificationWorkspace::new();
    let plan = plan_app_target_copy(&workspace, "replace");
    fs::write(
        workspace.source_root.join("A.rom"),
        b"changed after review\n",
    )
    .unwrap();

    let result = execute_fake_copy(&workspace, &plan);

    assert!(!result.success, "source drift must reject the copy");
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Failed,
        "source drift must be reported as a failed execution step"
    );
    assert!(
        !workspace.destination_root().join("A.rom").exists(),
        "source files must not be published after source drift"
    );
    assert!(
        workspace.destination_root().join("C.rom").exists(),
        "reviewed destination entries must remain when source validation fails"
    );
    assert!(
        workspace
            .destination_root()
            .join("stale-dir/stale.txt")
            .exists(),
        "the reviewed deletion set must not be applied after source drift"
    );
}

#[test]
fn verification_failure_after_copy_commit_reports_failed_partial_result() {
    let workspace = QualificationWorkspace::new();
    let mut plan = plan_app_target_copy(&workspace, "merge");
    let missing_path = format!(
        "{}/missing-verification-marker",
        resolved_app_target_path(&plan)
    );
    let mut params = OrderedMap::new();
    params.insert("path".to_string(), json!(missing_path));
    plan.steps[0]
        .verify
        .push(crate::planner::ExecutionStepCondition {
            type_name: "path_exists".to_string(),
            app_id: None,
            params,
        });

    let result = execute_fake_copy(&workspace, &plan);

    assert!(
        !result.success,
        "failed post-copy verification must fail the run"
    );
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Failed,
        "a committed copy with failed verification must report a failed step"
    );
    assert_eq!(
        fs::read(workspace.destination_root().join("A.rom")).unwrap(),
        b"A source\n",
        "failure reporting must preserve the already committed copy result"
    );
    assert!(
        workspace.destination_root().join("C.rom").exists(),
        "Add/Update must preserve unrelated destination data after verification failure"
    );
}

#[test]
fn add_update_snapshot_has_no_deletion_set() {
    let workspace = QualificationWorkspace::new();
    let plan = plan_app_target_copy(&workspace, "merge");
    assert_eq!(
        plan.copy_commit_snapshots[0].reviewed_deletion_set,
        Vec::<String>::new()
    );
}

#[test]
fn review_payload_classifies_copy_changes_for_each_directory_operation() {
    for policy in ["merge", "replace", "sync"] {
        let workspace = QualificationWorkspace::new();
        let plan = plan_app_target_copy(&workspace, policy);
        let snapshot = &plan.copy_commit_snapshots[0];
        let review =
            serde_json::to_value(crate::review_projection::project_review(&plan, &[])).unwrap();
        let action = &review["features"][0]["sections"][0]["actions"][0];
        let copy_changes = &action["copyChanges"];
        let additions = snapshot
            .source_manifest
            .iter()
            .filter(|source| {
                !snapshot
                    .destination_before_manifest
                    .iter()
                    .any(|destination| destination.relative_path == source.relative_path)
            })
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();
        let replaces_selected_tree = matches!(
            snapshot.operation,
            crate::executor::copy_commit::CopyCommitOperation::Replace
        );
        let replacements = snapshot
            .source_manifest
            .iter()
            .filter(|source| {
                snapshot
                    .destination_before_manifest
                    .iter()
                    .any(|destination| {
                        destination.relative_path == source.relative_path
                            && (replaces_selected_tree || destination != *source)
                    })
            })
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            copy_changes["selectedBoundary"],
            json!(resolved_app_target_path(&plan))
        );
        assert_eq!(copy_changes["additions"], json!(additions));
        assert_eq!(copy_changes["replacements"], json!(replacements));
        assert_eq!(
            copy_changes["deletions"],
            json!(snapshot.reviewed_deletion_set)
        );
        assert_eq!(
            action["reviewedDeletionSet"], copy_changes["deletions"],
            "the compatibility deletion list must match the exact copy change set for {policy}"
        );
        assert!(copy_changes.get("selected_boundary").is_none());
    }
}

#[test]
fn identical_source_and_destination_entries_are_not_projected_as_replacements() {
    for policy in ["merge", "sync"] {
        let workspace = QualificationWorkspace::new();
        let destination_root = workspace.destination_root();
        fs::create_dir_all(destination_root.join("nested")).unwrap();
        fs::write(
            destination_root.join("nested/inside.bin"),
            b"nested payload\n",
        )
        .unwrap();

        let plan = plan_app_target_copy(&workspace, policy);
        let snapshot = &plan.copy_commit_snapshots[0];
        for path in ["nested", "nested/inside.bin"] {
            let source = snapshot
                .source_manifest
                .iter()
                .find(|entry| entry.relative_path == path)
                .expect("source manifest should include the selected entry");
            let destination = snapshot
                .destination_before_manifest
                .iter()
                .find(|entry| entry.relative_path == path)
                .expect("destination manifest should include the matching entry");
            assert_eq!(source.kind, destination.kind);
            assert_eq!(source.sha256, destination.sha256);
            assert_ne!(
                source.entry_identity, destination.entry_identity,
                "the independent trees should have distinct filesystem identities for {path}"
            );
        }

        let review =
            serde_json::to_value(crate::review_projection::project_review(&plan, &[])).unwrap();
        let copy_changes = &review["features"][0]["sections"][0]["actions"][0]["copyChanges"];
        assert_eq!(
            copy_changes["replacements"],
            json!(["B.rom"]),
            "only the same-path file with different content is a replacement for {policy}"
        );
    }
}

#[test]
fn sidecar_pre_review_preparation_binds_review_digest_and_executor_snapshot() {
    let workspace = QualificationWorkspace::new();
    let destination_root = workspace.destination_root();
    for entry in fs::read_dir(&destination_root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fs::remove_dir_all(path).unwrap();
        } else {
            fs::remove_file(path).unwrap();
        }
    }

    let mut plan = plan_app_target_copy(&workspace, "merge");
    plan.copy_commit_snapshots.clear();
    let mut sessions = crate::session::DocumentSessionManager::with_runtime_config(
        crate::execution_session::SidecarRuntimeConfig {
            runtime_root: workspace.runtime_root.clone(),
            cache_root: workspace.cache_root.clone(),
            adb_path: "adb".to_string(),
        },
    );
    let unprepared_digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    assert!(sessions
        .executions()
        .start(
            plan.clone(),
            unprepared_digest,
            crate::execution_session::ExecutionMode::DryRun,
            None,
        )
        .is_err());
    let response = crate::request::handle_sidecar_value(
        json!({
            "id": "t03-copy-review",
            "type": "prepareExecutionReview",
            "payload": {
                "plan": plan,
                "mode": "dry_run",
                "targetDevice": null
            }
        }),
        &mut sessions,
    );
    let result = response.get("result").unwrap_or(&response);
    assert!(
        result["plan"].is_object(),
        "preparation endpoint error: code={:?}, message={:?}",
        response["error"]["code"],
        response["error"]["message"]
    );

    let prepared_plan = crate::cli::parse_execution_plan_json(&result["plan"]).unwrap();
    assert_eq!(prepared_plan.copy_commit_snapshots.len(), 1);
    assert_eq!(
        result["planDigest"],
        json!(crate::plan_digest::execution_plan_digest(&prepared_plan).unwrap())
    );
    let expected_review = crate::review_projection::project_review(&prepared_plan, &[]);
    assert_eq!(
        result["review"],
        serde_json::to_value(expected_review).unwrap()
    );

    let execution = execute_fake_copy(&workspace, &prepared_plan);
    assert!(
        execution.success,
        "prepared snapshot was rejected: {execution:#?}"
    );
    assert_eq!(
        execution.steps[0].outputs["copy_commit_report"].value["sourceDigestSha256"],
        json!(prepared_plan.copy_commit_snapshots[0].source_digest_sha256)
    );
}

#[test]
fn app_target_runtime_step_output_source_is_rejected_before_mutation() {
    let workspace = QualificationWorkspace::new();
    let before = entries(&workspace.destination_root());
    let mut plan = plan_app_target_copy(&workspace, "merge");
    plan.copy_commit_snapshots.clear();
    plan.steps[0].params.insert(
        "source".to_string(),
        ExecutionParamValue::Ref {
            ref_value: "steps.extract.outputs.directory".to_string(),
        },
    );
    let adapters = ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
    );

    let result = ExecutorRunner::new(adapters).prepare_copy_commit_snapshots(&mut plan);

    assert!(
        result.is_err(),
        "an App Target source produced by a runtime step must fail before review"
    );
    assert!(plan.copy_commit_snapshots.is_empty());
    assert_eq!(
        entries(&workspace.destination_root()),
        before,
        "rejecting the runtime source must not mutate the destination"
    );
}

#[test]
fn app_target_runtime_step_output_source_is_rejected_for_shared_storage_before_review() {
    let workspace = QualificationWorkspace::new();
    let before = entries(&workspace.destination_root());
    let mut plan = plan_app_target_copy(&workspace, "merge");
    plan.copy_commit_snapshots.clear();
    plan.steps[0].params.insert(
        "source".to_string(),
        ExecutionParamValue::Ref {
            ref_value: "steps.extract.outputs.directory".to_string(),
        },
    );
    plan.steps[0].params.insert(
        "dest".to_string(),
        ExecutionParamValue::Literal {
            value: json!("/sdcard/ROMs"),
        },
    );
    let adapters = ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
    );

    let error = ExecutorRunner::new(adapters)
        .prepare_copy_commit_snapshots(&mut plan)
        .expect_err("shared-storage review must reject an unbound runtime source");

    assert!(
        error.contains("App Target sources from runtime step outputs cannot be reviewed"),
        "the rejection should identify the unsupported review source: {error}"
    );
    assert!(plan.copy_commit_snapshots.is_empty());
    assert_eq!(
        entries(&workspace.destination_root()),
        before,
        "review preparation must reject the source before any destination mutation"
    );
}

#[test]
fn preparation_rejects_legacy_copy_operands_and_generic_steps() {
    let workspace = QualificationWorkspace::new();
    let source_value = |type_name: &str, location: &str| {
        serde_json::to_value(crate::planner::RuntimeValue {
            type_name: type_name.to_string(),
            value: json!(workspace.source_root),
            location: Some(location.to_string()),
        })
        .unwrap()
    };
    let mut plans = Vec::new();

    let safe_file_path = workspace.source_root.join("safe-file.txt");
    fs::write(&safe_file_path, b"safe file").unwrap();
    let mut safe_file = plan_app_target_copy(&workspace, "merge");
    safe_file.copy_commit_snapshots.clear();
    safe_file.steps[0].params.insert(
        "source".to_string(),
        ExecutionParamValue::Literal {
            value: serde_json::to_value(crate::planner::RuntimeValue {
                type_name: "file_path".to_string(),
                value: json!(safe_file_path),
                location: Some("host".to_string()),
            })
            .unwrap(),
        },
    );
    safe_file.steps[0].params.insert(
        "dest".to_string(),
        ExecutionParamValue::Literal {
            value: json!("/sdcard/EmuChef/safe-file.txt"),
        },
    );
    safe_file.steps[0].params.insert(
        "dest_kind".to_string(),
        ExecutionParamValue::Literal {
            value: json!("file"),
        },
    );
    let file_adapters = ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
    );
    ExecutorRunner::new(file_adapters)
        .prepare_copy_commit_snapshots(&mut safe_file)
        .expect("safe single-file copies do not require directory snapshots");
    assert!(safe_file.copy_commit_snapshots.is_empty());

    let mut generic = plan_app_target_copy(&workspace, "sync");
    generic.copy_commit_snapshots.clear();
    generic.steps[0].app_id = None;
    plans.push(("generic copy", generic));

    let mut file_destination = plan_app_target_copy(&workspace, "merge");
    file_destination.copy_commit_snapshots.clear();
    file_destination.steps[0].params.insert(
        "dest_kind".to_string(),
        ExecutionParamValue::Literal {
            value: json!("file"),
        },
    );
    plans.push(("file destination", file_destination));

    for type_name in ["path", "path_list"] {
        let mut plan = plan_app_target_copy(&workspace, "merge");
        plan.copy_commit_snapshots.clear();
        plan.steps[0].params.insert(
            "source".to_string(),
            ExecutionParamValue::Literal {
                value: source_value(type_name, "host"),
            },
        );
        plans.push((type_name, plan));
    }

    let mut device_source = plan_app_target_copy(&workspace, "merge");
    device_source.copy_commit_snapshots.clear();
    device_source.steps[0].params.insert(
        "source".to_string(),
        ExecutionParamValue::Literal {
            value: source_value("directory_path", "device"),
        },
    );
    plans.push(("device source", device_source));

    for (description, mut plan) in plans {
        let adapters = ExecutorAdapters::with_sandbox_roots(
            workspace.runtime_root.clone(),
            workspace.cache_root.clone(),
            workspace.fake_device_root.clone(),
            vec![workspace.source_root.parent().unwrap().to_path_buf()],
        );
        let result = ExecutorRunner::new(adapters).prepare_copy_commit_snapshots(&mut plan);
        assert!(
            result.is_err(),
            "unsupported {description} copy must fail before review preparation"
        );
    }
}

#[test]
fn unreviewed_generic_mirror_does_not_mutate_destination() {
    let workspace = QualificationWorkspace::new();
    let marker = workspace.destination_root().join("must-survive.txt");
    fs::write(&marker, b"review required\n").unwrap();
    let mut plan = plan_app_target_copy(&workspace, "sync");
    plan.copy_commit_snapshots.clear();
    plan.steps[0].app_id = None;

    let result = execute_fake_copy(&workspace, &plan);

    assert!(
        !result.success,
        "an unreviewed generic mirror must be rejected"
    );
    assert!(
        marker.exists(),
        "a rejected copy must not mutate the destination"
    );
}

fn adb_output(serial: &str, args: &[&str]) -> String {
    let output = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .args(args)
        .output()
        .expect("ADB must be available for controlled T03 qualification");
    assert!(
        output.status.success(),
        "controlled ADB command failed: {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("controlled ADB output should be UTF-8")
}

struct SyntheticFixtureCleanup {
    serial: String,
    installed_or_installing: bool,
}

impl SyntheticFixtureCleanup {
    fn new(serial: &str) -> Self {
        Self {
            serial: serial.to_string(),
            installed_or_installing: false,
        }
    }

    fn arm(&mut self) {
        self.installed_or_installing = true;
    }

    fn uninstall(&mut self) {
        if !self.installed_or_installing {
            return;
        }
        verify_t03_avd(&self.serial);
        let output = Command::new("adb")
            .args([
                "-s",
                self.serial.as_str(),
                "shell",
                "pm",
                "uninstall",
                "--user",
                "0",
                APP_PACKAGE,
            ])
            .output()
            .expect("ADB must be available to uninstall the synthetic T03 fixture");
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "Success",
            "synthetic T03 fixture uninstall failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        self.installed_or_installing = false;
        println!("T03 synthetic app removed from controlled AVD: package={APP_PACKAGE}");
    }
}

impl Drop for SyntheticFixtureCleanup {
    fn drop(&mut self) {
        if !self.installed_or_installing {
            return;
        }
        match Command::new("adb")
            .args([
                "-s",
                self.serial.as_str(),
                "shell",
                "pm",
                "uninstall",
                "--user",
                "0",
                APP_PACKAGE,
            ])
            .output()
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => eprintln!(
                "failed to remove synthetic T03 fixture after test exit: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => {
                eprintln!("failed to remove synthetic T03 fixture after test exit: {error}")
            }
        }
    }
}

fn verify_t03_avd_identity(serial: &str) -> (String, String) {
    assert_eq!(
        serial, "emulator-5554",
        "T03 controlled qualification requires the dedicated emulator-5554 serial"
    );
    let devices = Command::new("adb")
        .args(["devices", "-l"])
        .output()
        .expect("ADB must be available for controlled T03 qualification");
    assert!(devices.status.success());
    let devices = String::from_utf8_lossy(&devices.stdout);
    let online_devices = devices
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some("device"))
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    assert_eq!(
        online_devices,
        vec![serial],
        "no physical or other Android device may be attached during T03 qualification"
    );
    assert!(
        devices.lines().any(|line| {
            line.split_whitespace().next() == Some(serial)
                && line.split_whitespace().nth(1) == Some("device")
                && line.contains("device:emu64a")
        }),
        "selected serial must be an attached Android emulator: {devices}"
    );
    let avd_name = adb_output(serial, &["emu", "avd", "name"]);
    assert_eq!(
        avd_name.lines().next().map(str::trim),
        Some("EmuChef-Android16-Root"),
        "T03 controlled qualification must use the dedicated AVD"
    );
    let api_level = adb_output(serial, &["shell", "getprop", "ro.build.version.sdk"]);
    assert_eq!(api_level.trim(), "36");
    assert_eq!(
        adb_output(serial, &["shell", "getprop", "ro.kernel.qemu"]).trim(),
        "1"
    );
    (avd_name, api_level)
}

fn verify_t03_avd(serial: &str) -> RootShellRoute {
    let (avd_name, api_level) = verify_t03_avd_identity(serial);
    let shell_uid = adb_output(serial, &["shell", "id", "-u"]);
    assert_eq!(shell_uid.trim(), "2000", "ADB shell must remain UID 2000");
    let shell_context = adb_output(serial, &["shell", "id", "-Z"]);
    assert_eq!(shell_context.trim(), "u:r:shell:s0");
    let root_uid = adb_output(serial, &["shell", "su", "0", "id", "-u"]);
    assert_eq!(
        root_uid.trim(),
        "0",
        "the fixed su 0 route must prove UID 0"
    );
    let root_context = adb_output(serial, &["shell", "su", "0", "id", "-Z"]);
    assert!(
        root_context.trim() == "u:r:su:s0",
        "unexpected su 0 SELinux context: {root_context}"
    );
    let selinux = adb_output(serial, &["shell", "getenforce"]);
    assert_eq!(selinux.trim(), "Enforcing");
    let mut root_probe_executor = ProcessAdbCommandExecutor::default();
    let root_probe = probe_root_with_route(&mut root_probe_executor, "adb", serial);
    assert_eq!(root_probe.outcome, RootProbeOutcome::Granted);
    let root_shell_route = root_probe
        .route
        .expect("UID 0 and SELinux context must select a verified root route");
    assert_eq!(root_shell_route, RootShellRoute::SuZero);
    println!(
        "T03 controlled AVD verified before package mutation: serial={serial}, avd={}, api={}, shell_uid={}, shell_context={}, su_uid={}, su_context={}, selinux={}, root_route={root_shell_route:?}",
        avd_name.trim(),
        api_level.trim(),
        shell_uid.trim(),
        shell_context.trim(),
        root_uid.trim(),
        root_context.trim(),
        selinux.trim()
    );
    root_shell_route
}

/// Captures a controlled destination tree through the AVD's verified root route.
struct QualifiedRootTreeArchive<'a> {
    serial: &'a str,
    remote: &'a str,
}

impl QualifiedRootTreeArchive<'_> {
    fn capture(&self) -> Vec<u8> {
        let script = format!(
            "cd {} && exec tar -cf - .",
            quote_posix_shell_argument(self.remote)
        );
        let command = format!("su 0 sh -c {}", quote_posix_shell_argument(&script));
        let output = Command::new("adb")
            .args(["-s", self.serial, "exec-out"])
            .arg(command)
            .output()
            .expect("ADB must be available to inspect the controlled destination tree");
        assert!(
            output.status.success(),
            "controlled root destination archive failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
}

fn quote_posix_shell_argument(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\"'\"'"))
}

fn pull_tree_as_qualified_root(serial: &str, remote: &str, local: &Path) {
    let archive = QualifiedRootTreeArchive { serial, remote }.capture();
    if local.exists() {
        fs::remove_dir_all(local)
            .expect("previous controlled destination snapshot must be removable");
    }
    fs::create_dir_all(local).expect("controlled destination snapshot directory must be creatable");

    let mut extractor = Command::new("tar")
        .args(["-xf", "-", "-C"])
        .arg(local)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("host tar must be available to inspect the controlled destination tree");
    let mut archive_stdin = extractor
        .stdin
        .take()
        .expect("host tar archive input must be available");
    let write_result = archive_stdin.write_all(&archive);
    drop(archive_stdin);
    let extraction = extractor
        .wait_with_output()
        .expect("host tar must finish extracting the controlled destination tree");
    assert!(
        write_result.is_ok(),
        "controlled destination archive input failed: {}",
        write_result.unwrap_err()
    );
    assert!(
        extraction.status.success(),
        "controlled destination archive extraction failed: {}",
        String::from_utf8_lossy(&extraction.stderr)
    );
}

#[test]
fn qualified_root_tree_archive_quotes_remote_paths() {
    assert_eq!(
        quote_posix_shell_argument("/data path/'quoted'"),
        r#"'/data path/'"'"'quoted'"'"''"#
    );
}

#[derive(Debug, PartialEq, Eq)]
enum FixtureInstallAction {
    InstallOnly,
    UninstallThenInstall,
}

#[derive(Debug, PartialEq, Eq)]
enum FixtureUninstallOutcome {
    Removed,
    AlreadyAbsent,
}

fn fixture_package_is_present(package_listing: &str) -> bool {
    let expected = format!("package:{APP_PACKAGE}");
    package_listing.lines().any(|line| line.trim() == expected)
}

fn fixture_package_listing_presence(package_listing: &str) -> Result<bool, String> {
    for line in package_listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some(package) = line.strip_prefix("package:") else {
            return Err(format!("unrecognized package listing output: {line:?}"));
        };
        if package.is_empty() || package.chars().any(char::is_whitespace) {
            return Err(format!("invalid package listing entry: {line:?}"));
        }
    }
    Ok(fixture_package_is_present(package_listing))
}

fn fixture_install_action(package_listing: &str) -> Result<FixtureInstallAction, String> {
    if fixture_package_listing_presence(package_listing)? {
        Ok(FixtureInstallAction::UninstallThenInstall)
    } else {
        Ok(FixtureInstallAction::InstallOnly)
    }
}

fn classify_fixture_uninstall_result(
    command_succeeded: bool,
    stdout: &str,
    stderr: &str,
) -> Result<FixtureUninstallOutcome, String> {
    if !command_succeeded {
        return Err(format!("ADB package uninstall failed: {stderr}"));
    }
    if !stderr.trim().is_empty() {
        return Err(format!("ADB package uninstall wrote to stderr: {stderr}"));
    }
    match stdout.trim() {
        "Success" => Ok(FixtureUninstallOutcome::Removed),
        "Failure [not installed for 0]" => Ok(FixtureUninstallOutcome::AlreadyAbsent),
        other => Err(format!("unrecognized package uninstall result: {other:?}")),
    }
}

fn install_synthetic_fixture(serial: &str, apk: &Path) {
    verify_t03_avd(serial);
    let package_listing = adb_output(serial, &["shell", "pm", "list", "packages", "--user", "0"]);
    let install_action = fixture_install_action(&package_listing)
        .expect("package manager output must be recognized before fixture mutation");
    if install_action == FixtureInstallAction::UninstallThenInstall {
        let output = Command::new("adb")
            .args([
                "-s",
                serial,
                "shell",
                "pm",
                "uninstall",
                "--user",
                "0",
                APP_PACKAGE,
            ])
            .output()
            .expect("ADB must be available to uninstall the synthetic T03 fixture");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        classify_fixture_uninstall_result(output.status.success(), &stdout, &stderr)
            .unwrap_or_else(|error| panic!("failed to uninstall only {APP_PACKAGE}: {error}"));
    }

    let mut setup_device =
        RealAdbDevice::<ProcessAdbCommandExecutor>::new("adb", Some(serial.to_string()));
    setup_device
        .install_apk(apk, false)
        .expect("synthetic App Target application should install on the dedicated AVD");
}

fn run_controlled_real_copy(
    workspace: &QualificationWorkspace,
    plan: &ExecutionPlan,
    serial: &str,
) -> (ExecutionRunResult, Arc<Mutex<Vec<String>>>) {
    let command_trace = Arc::new(Mutex::new(Vec::new()));
    let device = RealAdbDevice::with_executor(
        "adb",
        Some(serial),
        ReportingAdbCommandExecutor {
            inner: ProcessAdbCommandExecutor::default(),
            command_trace: command_trace.clone(),
        },
    );
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.source_root.parent().unwrap().to_path_buf()],
        false,
    );
    (ExecutorRunner::new(adapters).run(plan), command_trace)
}

#[derive(Debug)]
struct ReportingAdbCommandExecutor {
    inner: ProcessAdbCommandExecutor,
    command_trace: Arc<Mutex<Vec<String>>>,
}

fn controlled_copy_commit_stage(operation: ProcessOperation) -> &'static str {
    match operation {
        ProcessOperation::Probe | ProcessOperation::Predicate => {
            "CopyCommitDevice::inspect_tree_or_path_binding"
        }
        ProcessOperation::RootPreflight => "CopyCommitDevice::root_preflight",
        ProcessOperation::ShellMutation => {
            "CopyCommitDevice::mkdir_remove_or_selected_root_mutation"
        }
        ProcessOperation::Push => "CopyCommitDevice::push_file_or_prepare_mutation_helper",
        ProcessOperation::DeviceCopy => "CopyCommitDevice::copy_file",
        ProcessOperation::Install => "CopyCommitDevice::prepare_mutation_helper",
        ProcessOperation::Launch => "controlled_copy_setup::launch",
        ProcessOperation::ForceStop => "controlled_copy_setup::force_stop",
        ProcessOperation::GenericFallback => "CopyCommitDevice::generic_fallback",
    }
}

impl AdbCommandExecutor for ReportingAdbCommandExecutor {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let result = self.inner.run_for(args, operation);
        let stage = controlled_copy_commit_stage(operation);
        let diagnostic = match &result {
            Ok(output) => format!(
                "stage={stage} operation={operation:?} args={args:?} exit={} stdout={:?} stderr={:?}",
                output.returncode, output.stdout, output.stderr
            ),
            Err(error) => {
                let stderr = match error {
                    AdbCommandError::CommandFailed { stderr, .. } => stderr.as_str(),
                    _ => "unavailable from this typed ADB error",
                };
                format!(
                    "stage={stage} operation={operation:?} args={args:?} typed_error={error:?} stdout=<unavailable from AdbCommandError> stderr={stderr:?}"
                )
            }
        };
        if let Ok(mut trace) = self.command_trace.lock() {
            trace.push(diagnostic.clone());
        }
        match &result {
            Ok(output) if output.returncode != 0 => {
                eprintln!("T03 controlled CopyCommit command failed: {diagnostic}")
            }
            Err(_) => eprintln!("T03 controlled CopyCommit command error: {diagnostic}"),
            _ => {}
        }
        result
    }
}

#[test]
#[ignore = "controlled Android qualification requires the dedicated AVD and local Android SDK/JDK"]
fn controlled_android_synthetic_app_target_matches_fake_for_all_copy_policies() {
    let serial = std::env::var("EMUChef_T03_CONTROLLED_SERIAL")
        .expect("set EMUChef_T03_CONTROLLED_SERIAL to the dedicated emulator serial");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should be inside the repository");
    let (_apk_output_root, apk) = build_synthetic_fixture(repo_root);
    let root_shell_route = verify_t03_avd(&serial);
    let mut fixture_cleanup = SyntheticFixtureCleanup::new(&serial);

    let expected_merged = [
        "A.rom",
        "B.rom",
        "C.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
        "old-empty",
        "stale-dir",
        "stale-dir/stale.txt",
    ];
    let expected_replaced = [
        "A.rom",
        "B.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
    ];
    let expected_before = [
        "B.rom",
        "C.rom",
        "old-empty",
        "stale-dir",
        "stale-dir/stale.txt",
    ];
    let expected_source_paths = [
        "A.rom",
        "B.rom",
        "empty-source",
        "nested",
        "nested/inside.bin",
    ];
    let expected_sibling = b"outside the selected App Target\n";

    for policy in ["merge", "replace", "sync"] {
        fixture_cleanup.arm();
        install_synthetic_fixture(&serial, &apk);
        let mut setup_device =
            RealAdbDevice::<ProcessAdbCommandExecutor>::new("adb", Some(serial.clone()));
        setup_device
            .launch_app(
                APP_PACKAGE,
                Some("org.emuchef.t03.copyfixture.FixtureActivity"),
            )
            .expect("synthetic app should create its private assets directory");
        let workspace = QualificationWorkspace::new();
        assert_eq!(
            entries(&workspace.source_root),
            expected_source_paths
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>(),
            "controlled source tree for policy={policy}"
        );
        let destination_fixture = workspace.destination_root();
        assert!(
            !destination_fixture.starts_with(&workspace.source_root),
            "controlled destination fixture must be outside the selected source tree"
        );
        let old_b = destination_fixture.join("B.rom");
        let unrelated_c = destination_fixture.join("C.rom");
        let setup_plan = plan_app_target_copy(&workspace, policy);
        let dest = resolved_app_target_path(&setup_plan).to_string();
        let selected_parent = Path::new(&dest)
            .parent()
            .expect("fixture destination should have an app-data parent")
            .to_string_lossy()
            .into_owned();
        assert_eq!(verify_t03_avd(&serial), root_shell_route);
        adb_output(
            &serial,
            &[
                "shell",
                "su",
                "0",
                "mkdir",
                "-p",
                &format!("{dest}/old-empty"),
                &format!("{dest}/stale-dir"),
                &format!("{selected_parent}/sibling"),
            ],
        );
        let device_staging_root = format!(
            "/data/local/tmp/{}",
            workspace
                ._temp
                .path()
                .file_name()
                .expect("T03 workspace should have a unique directory name")
                .to_string_lossy()
        );
        adb_output(&serial, &["shell", "mkdir", "-p", &device_staging_root]);
        let stale = destination_fixture.join("stale-dir/stale.txt");
        let sibling = workspace
            .fake_device_root
            .join("data/user/0")
            .join(APP_PACKAGE)
            .join("files/sibling/keep.txt");
        for (index, (source, target)) in [
            (&old_b, format!("{dest}/B.rom")),
            (&unrelated_c, format!("{dest}/C.rom")),
            (&stale, format!("{dest}/stale-dir/stale.txt")),
            (&sibling, format!("{selected_parent}/sibling/keep.txt")),
        ]
        .into_iter()
        .enumerate()
        {
            let staged_source = format!("{device_staging_root}/{index}.fixture");
            let output = Command::new("adb")
                .args(["-s", serial.as_str(), "push"])
                .arg(source)
                .arg(&staged_source)
                .output()
                .expect("ADB must be available to seed the disposable App Target fixture");
            assert!(
                output.status.success(),
                "seed push failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            adb_output(
                &serial,
                &["shell", "su", "0", "cp", &staged_source, &target],
            );
            adb_output(&serial, &["shell", "rm", "-f", &staged_source]);
        }
        adb_output(&serial, &["shell", "rmdir", &device_staging_root]);
        let api_level = adb_output(&serial, &["shell", "getprop", "ro.build.version.sdk"])
            .trim()
            .parse::<u32>()
            .expect("controlled Android preflight should report its API level");
        assert_eq!(
            api_level, 36,
            "controlled Android preflight should report fixture API 36 for policy={policy}"
        );
        let manufacturer = adb_output(&serial, &["shell", "getprop", "ro.product.manufacturer"])
            .trim()
            .to_string();
        assert!(
            !manufacturer.is_empty(),
            "controlled Android preflight should report its manufacturer"
        );
        let model = adb_output(&serial, &["shell", "getprop", "ro.product.model"])
            .trim()
            .to_string();
        assert!(
            !model.is_empty(),
            "controlled Android preflight should report its model"
        );
        let target_device = TargetDeviceBinding {
            serial: serial.clone(),
            manufacturer: Some(manufacturer.clone()),
            model: Some(model.clone()),
            android_api_level: Some(i64::from(api_level)),
        };
        let mut fake_plan =
            plan_app_target_copy_with_target_device(&workspace, policy, Some(target_device));
        fake_plan.runtime_capabilities.root_shell_route = Some(root_shell_route);
        fake_plan
            .device_context
            .bind_verified_root_shell_mode(Some(root_shell_route));
        assert_eq!(
            fake_plan.runtime_capabilities.root_shell_route,
            Some(RootShellRoute::SuZero),
            "the controlled plan must retain the route verified by live AVD preflight"
        );
        let fake_result = run_fake_copy(&workspace, &fake_plan);
        let mut review_manager =
            crate::session::DocumentSessionManager::with_runtime_config(SidecarRuntimeConfig {
                runtime_root: workspace.runtime_root.clone(),
                cache_root: workspace.cache_root.clone(),
                adb_path: "adb".to_string(),
            });
        let plan = fake_plan.clone();
        assert_eq!(
            plan.target_device.as_ref().map(|binding| (
                binding.serial.as_str(),
                binding.manufacturer.as_deref(),
                binding.model.as_deref(),
                binding.android_api_level,
            )),
            Some((
                serial.as_str(),
                Some(manufacturer.as_str()),
                Some(model.as_str()),
                Some(i64::from(api_level)),
            )),
            "controlled review plan must carry only verified target facts for policy={policy}"
        );
        let supplied = plan
            .target_device
            .clone()
            .expect("controlled review plan must include its target binding");
        let review_response = crate::request::handle_sidecar_value(
            json!({
                "id": "t03-copy-review-apply",
                "type": "prepareExecutionReview",
                "payload": {
                    "plan": plan,
                    "mode": "real",
                    "targetDevice": {
                        "serial": supplied.serial,
                        "manufacturer": supplied.manufacturer,
                        "model": supplied.model,
                        "androidApiLevel": supplied.android_api_level
                    }
                }
            }),
            &mut review_manager,
        );
        let reviewed = review_response.get("result").unwrap_or(&review_response);
        assert!(
            reviewed["plan"].is_object(),
            "production real review must return the plan used for controlled apply policy={policy}; response={review_response:#?}"
        );
        let real_reviewed_plan = crate::cli::parse_execution_plan_json(&reviewed["plan"])
            .expect("production review plan should parse for controlled apply");
        assert_eq!(
            reviewed["planDigest"],
            json!(crate::plan_digest::execution_plan_digest(&real_reviewed_plan).unwrap()),
            "controlled apply must consume the exact plan digest returned by production review policy={policy}"
        );
        assert!(
            real_reviewed_plan.copy_commit_snapshots[0]
                .destination_binding
                .is_some(),
            "controlled real review must capture the Android destination binding policy={policy}"
        );
        assert_eq!(
            real_reviewed_plan.copy_commit_snapshots[0].operation,
            fake_plan.copy_commit_snapshots[0].operation,
            "real review preparation must preserve the reviewed copy operation policy={policy}"
        );
        assert_eq!(
real_reviewed_plan.copy_commit_snapshots[0].reviewed_deletion_set,
fake_plan.copy_commit_snapshots[0].reviewed_deletion_set,
"real review preparation must preserve the exact reviewed deletion manifest policy={policy}"
);
        assert_eq!(
            real_reviewed_plan.runtime_capabilities.root_shell_route,
            Some(root_shell_route),
            "the reviewed plan must retain the verified route for mutation and rechecks"
        );
        assert_eq!(
            real_reviewed_plan.device_context.root_shell_mode,
            Some(root_shell_route),
            "the reviewed device context must bind the selected root-shell mode"
        );
        let drift_marker = format!("{dest}/unreviewed-drift.rom");
        adb_output(
            &serial,
            &["shell", "su", "0", "touch", drift_marker.as_str()],
        );
        let (drift_result, drift_trace) =
            run_controlled_real_copy(&workspace, &real_reviewed_plan, &serial);
        let drift_trace = drift_trace
            .lock()
            .map(|commands| commands.join("\n"))
            .unwrap_or_else(|poisoned| poisoned.into_inner().join("\n"));
        adb_output(
            &serial,
            &["shell", "su", "0", "rm", "-f", drift_marker.as_str()],
        );
        assert!(
            !drift_result.success
                && format!("{drift_result:#?}")
                    .contains("selected destination changed after review"),
            "real Apply must reject selected-tree drift before mutation policy={policy}: {drift_result:#?}; command trace:\n{drift_trace}"
        );
        assert!(
            !drift_trace.contains("operation=Push")
                && !drift_trace.contains("operation=DeviceCopy")
                && !drift_trace.contains("operation=ShellMutation"),
            "drift rejection must occur before helper staging or root mutation: {drift_trace}"
        );
        let (real_result, command_trace) =
            run_controlled_real_copy(&workspace, &real_reviewed_plan, &serial);
        let command_trace = command_trace
            .lock()
            .map(|commands| commands.join("\n"))
            .unwrap_or_else(|poisoned| poisoned.into_inner().join("\n"));
        assert!(
            real_result.success,
            "controlled-real CopyCommit stage=AndroidCopyCommitAdapter::commit policy={policy}; executor result: {real_result:#?}; command trace:\n{command_trace}"
        );
        assert!(
            command_trace.contains("\"su\", \"0\", \"id\""),
            "the selected root route must be used for UID checks: {command_trace}"
        );
        assert!(
            command_trace.contains("su 0 sh -c '/data/local/tmp/emuchef/t03-copy-"),
            "the exact su 0 helper command must reach the adapter: {command_trace}"
        );

        let expected: &[&str] = if policy == "merge" {
            &expected_merged
        } else {
            &expected_replaced
        };
        let pulled = workspace._temp.path().join("controlled-app-files");
        pull_tree_as_qualified_root(&serial, &selected_parent, &pulled);
        let pulled_destination = pulled.join("assets");
        assert_eq!(
            entries(&pulled_destination),
            expected
                .iter()
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>(),
            "controlled destination for policy={policy}"
        );
        assert_eq!(
            fs::read(pulled_destination.join("A.rom")).unwrap(),
            b"A source\n"
        );
        assert_eq!(
            fs::read(pulled_destination.join("B.rom")).unwrap(),
            b"updated B\n"
        );
        assert_eq!(
            fs::read(pulled_destination.join("C.rom")).ok(),
            (policy == "merge").then_some(b"unrelated C\n".to_vec()),
            "controlled C content for policy={policy}"
        );
        assert_eq!(
            fs::read(pulled.join("sibling/keep.txt")).unwrap(),
            expected_sibling
        );
        assert!(pulled_destination.join("empty-source").is_dir());
        assert_eq!(
            pulled_destination.join("old-empty").is_dir(),
            policy == "merge",
            "controlled preexisting empty directory for policy={policy}"
        );

        let fake_report = &fake_result.steps[0].outputs["copy_commit_report"].value;
        let real_report = &real_result.steps[0].outputs["copy_commit_report"].value;
        assert_eq!(
            real_report["stagedPaths"],
            json!(expected_source_paths),
            "controlled Android must stage the reviewed source manifest, including empty directories, through CopyCommitAdapter for policy={policy}"
        );
        assert_eq!(real_report["stagingVerified"], json!(true));
        assert_eq!(
            real_report["destinationBindingVerified"],
            json!(true),
            "controlled Android must verify the reviewed destination binding policy={policy}"
        );
        assert_eq!(real_report["beforePaths"], json!(expected_before));
        assert_eq!(real_report["afterPaths"], json!(expected));
        assert_eq!(real_report["beforePaths"], fake_report["beforePaths"]);
        assert_eq!(
            real_report["reviewedDeletionSet"],
            fake_report["reviewedDeletionSet"]
        );
        assert_eq!(real_report["deletedPaths"], fake_report["deletedPaths"]);
        assert_eq!(real_report["afterPaths"], fake_report["afterPaths"]);
        let expected_deleted: &[&str] = match policy {
            "merge" => &[],
            "replace" => &[
                "B.rom",
                "C.rom",
                "old-empty",
                "stale-dir",
                "stale-dir/stale.txt",
            ],
            "sync" => &["C.rom", "old-empty", "stale-dir", "stale-dir/stale.txt"],
            _ => unreachable!(),
        };
        assert_eq!(real_report["reviewedDeletionSet"], json!(expected_deleted));
        assert_eq!(real_report["deletedPaths"], json!(expected_deleted));
        println!(
    "T03 controlled copy policy={} ({policy}) beforePaths={} afterPaths={} reviewedDeletionSet={} deletedPaths={}",
    match policy {
        "merge" => "Add/Update",
        "replace" => "Replace",
        "sync" => "Mirror",
        _ => unreachable!(),
    },
    real_report["beforePaths"],
    real_report["afterPaths"],
    real_report["reviewedDeletionSet"],
    real_report["deletedPaths"],
);
    }
    fixture_cleanup.uninstall();
}

#[allow(dead_code)]
fn _step_type_is_kept_visible_for_qualification_docs(_step: &ExecutionStep) {}

#[test]
fn fixture_package_presence_uses_an_exact_package_match() {
    assert!(!fixture_package_is_present(""));
    assert!(fixture_package_is_present("package:com.emuchef.fixture\n"));
    assert!(!fixture_package_is_present(
        "package:com.emuchef.fixture.debug\n"
    ));
    assert!(!fixture_package_is_present("package:org.emuchef.fixture\n"));
}

#[test]
fn package_listing_parser_rejects_unrecognized_output_before_install_mutation() {
    assert_eq!(fixture_package_listing_presence(""), Ok(false));
    assert_eq!(
        fixture_package_listing_presence("package:org.example.other\n"),
        Ok(false)
    );
    assert_eq!(
        fixture_package_listing_presence("package:com.emuchef.fixture\n"),
        Ok(true)
    );
    assert!(fixture_package_listing_presence("Error: package manager unavailable\n").is_err());
}

#[test]
fn uninstall_classifier_accepts_only_success_or_the_known_absent_status() {
    assert_eq!(
        classify_fixture_uninstall_result(true, "Success\n", ""),
        Ok(FixtureUninstallOutcome::Removed)
    );
    assert_eq!(
        classify_fixture_uninstall_result(true, "Failure [not installed for 0]\n", ""),
        Ok(FixtureUninstallOutcome::AlreadyAbsent)
    );
    assert!(classify_fixture_uninstall_result(false, "", "transport failure").is_err());
    assert!(classify_fixture_uninstall_result(
        true,
        "Failure [DELETE_FAILED_INTERNAL_ERROR]\n",
        ""
    )
    .is_err());
    assert!(classify_fixture_uninstall_result(true, "Success\n", "warning").is_err());
}

#[test]
fn absent_fixture_package_is_installed_without_uninstalling() {
    assert_eq!(
        fixture_install_action(""),
        Ok(FixtureInstallAction::InstallOnly)
    );
    assert_eq!(
        fixture_install_action("package:org.example.other\\n"),
        Ok(FixtureInstallAction::InstallOnly)
    );
    assert_eq!(
        fixture_install_action("package:com.emuchef.fixture\n"),
        Ok(FixtureInstallAction::UninstallThenInstall)
    );
}

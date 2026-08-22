//! Phase 6C.2 root executor qualification safety and physical-device harness.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{condition, executor_step, fixture_install_step, fixture_root, literal, runtime_value};
use crate::executor::adb::{AdbCommandExecutor, RealAdbDevice};
use crate::executor::{ExecutorAdapters, ExecutorRunner};
use crate::planner::{
    DeviceContext, ExecutionPlan, ExecutionPlanSource, ExecutionStep, RuntimeCapabilities,
};

const FIXTURE_PACKAGE: &str = "com.emuchef.fixture";
const DATA_DATA_PREFIX: &str = "/data/data/com.emuchef.fixture/emuchef-qualification-data/";
const DATA_USER_PREFIX: &str = "/data/user/0/com.emuchef.fixture/emuchef-qualification-user/";
const EXPECTED_PREFIX_ALLOWLIST: &str = concat!(
    "/data/data/com.emuchef.fixture/emuchef-qualification-data/",
    ",",
    "/data/user/0/com.emuchef.fixture/emuchef-qualification-user/"
);
const GLOBAL_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_TESTS";
const ROOT_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_ROOT_TESTS";
const DEVICE_SERIAL_ENV: &str = "EMUCHEF_TEST_DEVICE_SERIAL";
const PACKAGE_ALLOWLIST_ENV: &str = "EMUCHEF_TEST_PACKAGE_ALLOWLIST";
const PREFIX_ALLOWLIST_ENV: &str = "EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST";
const DESTRUCTIVE_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS";

/// One deliberately selected root-qualification operation group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootQualificationGroup {
    Preflight,
    Filesystem,
    Copy,
    Combined,
    CleanupFailure,
}

impl RootQualificationGroup {
    const ALL: [Self; 5] = [
        Self::Preflight,
        Self::Filesystem,
        Self::Copy,
        Self::Combined,
        Self::CleanupFailure,
    ];

    const fn env_name(self) -> &'static str {
        match self {
            Self::Preflight => "EMUCHEF_RUN_REAL_ADB_ROOT_PREFLIGHT_TESTS",
            Self::Filesystem => "EMUCHEF_RUN_REAL_ADB_ROOT_FILESYSTEM_TESTS",
            Self::Copy => "EMUCHEF_RUN_REAL_ADB_ROOT_COPY_TESTS",
            Self::Combined => "EMUCHEF_RUN_REAL_ADB_ROOT_COMBINED_TESTS",
            Self::CleanupFailure => "EMUCHEF_RUN_REAL_ADB_ROOT_CLEANUP_FAILURE_TESTS",
        }
    }

    const fn requires_destructive_opt_in(self) -> bool {
        !matches!(self, Self::Preflight)
    }
}

/// Committed lexical authority for Phase 6C.2 device mutations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootQualificationContract {
    schema_version: u32,
    package_name: String,
    data_data_prefix: String,
    data_user_prefix: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RootQualificationInvocation {
    serial: String,
    group: RootQualificationGroup,
}

/// Fixed operation classification emitted by the qualification harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RootOperationOutcome {
    Succeeded,
    PreflightFailed,
    OperationFailed,
}

/// Cleanup is reported independently so it cannot overwrite the operation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RootCleanupOutcome {
    NotAttempted,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RootQualificationReport {
    operation: RootOperationOutcome,
    cleanup: RootCleanupOutcome,
    residual_paths: Vec<String>,
    #[serde(skip_serializing)]
    operation_message: Option<String>,
    #[serde(skip_serializing)]
    cleanup_message: Option<String>,
}

impl RootQualificationReport {
    fn preflight_failed(message: impl Into<String>) -> Self {
        Self {
            operation: RootOperationOutcome::PreflightFailed,
            cleanup: RootCleanupOutcome::NotAttempted,
            residual_paths: Vec::new(),
            operation_message: Some(message.into()),
            cleanup_message: None,
        }
    }

    fn from_operation_and_cleanup(
        operation: Result<(), String>,
        cleanup: Option<Result<(), String>>,
        residual_paths: Vec<String>,
    ) -> Self {
        let (operation_outcome, operation_message) = match operation {
            Ok(()) => (RootOperationOutcome::Succeeded, None),
            Err(message) => (RootOperationOutcome::OperationFailed, Some(message)),
        };
        let (cleanup_outcome, cleanup_message) = match cleanup {
            None => (RootCleanupOutcome::NotAttempted, None),
            Some(Ok(())) => (RootCleanupOutcome::Succeeded, None),
            Some(Err(message)) => (RootCleanupOutcome::Failed, Some(message)),
        };
        Self {
            operation: operation_outcome,
            cleanup: cleanup_outcome,
            residual_paths,
            operation_message,
            cleanup_message,
        }
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .to_path_buf()
}

fn root_contract_path() -> PathBuf {
    repository_root().join("fixtures/android/phase-6c2-root/qualification-contract.json")
}

fn load_root_contract() -> Result<RootQualificationContract, String> {
    let bytes = std::fs::read(root_contract_path())
        .map_err(|_| "root qualification contract is unavailable".to_string())?;
    let contract = serde_json::from_slice::<RootQualificationContract>(&bytes)
        .map_err(|_| "root qualification contract is invalid".to_string())?;
    validate_contract(&contract)?;
    Ok(contract)
}

fn exact_contract() -> RootQualificationContract {
    RootQualificationContract {
        schema_version: 1,
        package_name: FIXTURE_PACKAGE.to_string(),
        data_data_prefix: DATA_DATA_PREFIX.to_string(),
        data_user_prefix: DATA_USER_PREFIX.to_string(),
    }
}

fn validate_contract(contract: &RootQualificationContract) -> Result<(), String> {
    if contract != &exact_contract() {
        return Err(
            "root qualification contract does not match the fixed Phase 6C.2 authority".to_string(),
        );
    }
    Ok(())
}

/// Validate every environment authority before the physical harness may query ADB.
fn validate_invocation(
    required: RootQualificationGroup,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<RootQualificationInvocation, String> {
    require_exact(&lookup, GLOBAL_OPT_IN, "1")?;
    require_exact(&lookup, ROOT_OPT_IN, "1")?;
    let serial = lookup(DEVICE_SERIAL_ENV)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{DEVICE_SERIAL_ENV} must select one exact device"))?;
    require_exact(&lookup, PACKAGE_ALLOWLIST_ENV, FIXTURE_PACKAGE)?;
    require_exact(&lookup, PREFIX_ALLOWLIST_ENV, EXPECTED_PREFIX_ALLOWLIST).map_err(|_| {
        format!(
            "{PREFIX_ALLOWLIST_ENV} must exactly match the committed root qualification contract"
        )
    })?;
    let enabled = RootQualificationGroup::ALL
        .into_iter()
        .filter(|group| lookup(group.env_name()).as_deref() == Some("1"))
        .collect::<Vec<_>>();
    if enabled != [required] {
        return Err("exactly one matching root qualification group must be enabled".to_string());
    }
    if required.requires_destructive_opt_in() {
        require_exact(&lookup, DESTRUCTIVE_OPT_IN, "1")?;
    }
    Ok(RootQualificationInvocation {
        serial,
        group: required,
    })
}

fn require_exact(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &str,
    expected: &str,
) -> Result<(), String> {
    if lookup(name).as_deref() != Some(expected) {
        return Err(format!("{name} must equal {expected}"));
    }
    Ok(())
}

/// Accept only a non-root child beneath one fixed app-private prefix.
fn validate_owned_path(
    contract: &RootQualificationContract,
    candidate: &str,
) -> Result<String, String> {
    validate_contract(contract)?;
    let suffix = [&contract.data_data_prefix, &contract.data_user_prefix]
        .into_iter()
        .find_map(|prefix| candidate.strip_prefix(prefix))
        .ok_or_else(|| "root qualification path is outside the approved prefixes".to_string())?;
    let normalized = suffix.trim_end_matches('/');
    if normalized.is_empty()
        || candidate.contains("//")
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err("root qualification path must be a normalized child path".to_string());
    }
    Ok(candidate.trim_end_matches('/').to_string())
}

fn cleanup_failure_path(
    contract: &RootQualificationContract,
    run_id: &str,
) -> Result<String, String> {
    if run_id.is_empty()
        || run_id.len() > 64
        || !run_id
            .chars()
            .all(|character| character.is_ascii_digit() || character == '-')
    {
        return Err(
            "root cleanup-failure run id must contain only bounded digits and hyphens".to_string(),
        );
    }
    validate_owned_path(
        contract,
        &format!("{}cleanup-failure-{run_id}", contract.data_user_prefix),
    )
}

fn validate_inventory(inventory: &str, selected_serial: &str) -> Result<(), String> {
    let online = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            (fields.next() == Some("device")).then_some(serial)
        })
        .collect::<Vec<_>>();
    if online == [selected_serial] {
        Ok(())
    } else {
        Err(
            "root qualification requires exactly one online device matching the selected serial"
                .to_string(),
        )
    }
}

#[derive(Debug)]
struct ManualRootQualification {
    invocation: RootQualificationInvocation,
    contract: RootQualificationContract,
}

/// Apply the reviewed-root authority that production `ExecutorRunner` derives
/// from a root-capable plan to a supplied adapter. The Phase 6C.2 direct
/// helpers use this exact configuration so their privileged commands run under
/// the same root-authority boundary as the reviewed production path.
fn with_reviewed_root_authority<E: AdbCommandExecutor>(
    mut device: RealAdbDevice<E>,
) -> RealAdbDevice<E> {
    device.configure_root_authority(true);
    device
}

impl ManualRootQualification {
    /// Fresh, unconfigured adapter used only at the reviewed production-plan
    /// boundary. `ExecutorRunner` derives and applies reviewed-root authority
    /// from the plan itself; this method must never pre-authorize the device.
    fn device(&self) -> RealAdbDevice {
        RealAdbDevice::new("adb", Some(self.invocation.serial.clone()))
    }

    /// Qualification-only direct adapter for the post-plan helpers. The Phase
    /// 6C.2 invocation guards already authorized the manual harness, so the
    /// adapter is granted the same reviewed-root authority that the production
    /// runner applies to root-capable plans.
    fn authorized_direct_device(&self) -> RealAdbDevice {
        with_reviewed_root_authority(self.device())
    }

    fn validate_paths(&self, paths: &[String]) {
        for path in paths {
            validate_owned_path(&self.contract, path)
                .expect("physical root qualification path must remain contract-owned");
        }
    }

    fn qualify_filesystem_operations<E: AdbCommandExecutor>(
        &self,
        device: &mut RealAdbDevice<E>,
        copied_file: &str,
        created_tree: &str,
    ) -> Result<(), String> {
        validate_owned_path(&self.contract, copied_file)?;
        validate_owned_path(&self.contract, created_tree)?;
        let tree_root = created_tree
            .strip_suffix("/nested")
            .ok_or_else(|| "root filesystem tree must use the fixed nested child".to_string())?;
        validate_owned_path(&self.contract, tree_root)?;
        let copied_parent = copied_file
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .ok_or_else(|| "root filesystem file must have an owned parent".to_string())?;
        validate_owned_path(&self.contract, copied_parent)?;

        device.check_root()?;
        if !device.path_exists(copied_file)? || !device.path_is_dir(copied_parent)? {
            return Err("root filesystem predicates did not observe prepared state".to_string());
        }
        device.mkdir_p(created_tree)?;
        if !device.path_is_dir(created_tree)? {
            return Err(
                "root mkdir qualification did not create the expected directory".to_string(),
            );
        }
        device.remove_file(copied_file)?;
        if device.path_exists(copied_file)? {
            return Err("root file removal left the expected file behind".to_string());
        }
        device.remove_tree(tree_root)?;
        if device.path_exists(tree_root)? {
            return Err("root recursive removal left the expected tree behind".to_string());
        }
        Ok(())
    }

    fn create_owned_directory<E: AdbCommandExecutor>(
        &self,
        device: &mut RealAdbDevice<E>,
        path: &str,
    ) -> Result<(), String> {
        validate_owned_path(&self.contract, path)?;
        device.check_root()?;
        device.mkdir_p(path)?;
        if device.path_is_dir(path)? {
            Ok(())
        } else {
            Err("root cleanup-failure setup did not create the owned child".to_string())
        }
    }

    /// Remove only exact children that have already passed the committed contract guard.
    fn cleanup_paths<E: AdbCommandExecutor>(
        &self,
        device: &mut RealAdbDevice<E>,
        paths: &[String],
        injected_residual: Option<&str>,
    ) -> (Result<(), String>, Vec<String>) {
        self.validate_paths(paths);
        if let Some(residual) = injected_residual {
            validate_owned_path(&self.contract, residual)
                .expect("injected residual must remain contract-owned");
        }
        if device.check_root().is_err() {
            return (
                Err("root qualification cleanup preflight failed".to_string()),
                paths.to_vec(),
            );
        }
        let mut failed = false;
        let mut residual_paths = Vec::new();
        for path in paths {
            if injected_residual == Some(path.as_str()) {
                residual_paths.push(path.clone());
                failed = true;
                continue;
            }
            if device.remove_tree(path).is_err() {
                failed = true;
            }
            match device.path_exists(path) {
                Ok(false) => {}
                Ok(true) | Err(_) => {
                    failed = true;
                    residual_paths.push(path.clone());
                }
            }
        }
        if failed {
            (
                Err("root qualification cleanup left contract-owned residual state".to_string()),
                residual_paths,
            )
        } else {
            (Ok(()), residual_paths)
        }
    }
}

fn manual_root_qualification(required: RootQualificationGroup) -> Option<ManualRootQualification> {
    if std::env::var(GLOBAL_OPT_IN).ok().as_deref() != Some("1") {
        eprintln!("Skipping: set {GLOBAL_OPT_IN}=1 to run manual real-ADB root tests.");
        return None;
    }
    let invocation = validate_invocation(required, |name| std::env::var(name).ok())
        .expect("every exact Phase 6C.2 root qualification guard must be set before ADB");
    let contract = load_root_contract().expect("root qualification contract must be valid");

    let output = Command::new("adb")
        .args(["devices", "-l"])
        .output()
        .expect("ADB inventory must be available after qualification guards pass");
    assert!(
        output.status.success(),
        "ADB inventory command must succeed"
    );
    let inventory = String::from_utf8_lossy(&output.stdout);
    validate_inventory(&inventory, &invocation.serial)
        .expect("one online device must exactly match the selected serial");

    Some(ManualRootQualification {
        invocation,
        contract,
    })
}

fn root_preflight_step() -> ExecutionStep {
    let mut step = executor_step("fixture.root/preflight", "wait");
    step.recipe_ref = "fixture.root".to_string();
    step.constraints.capabilities = vec!["root_shell".to_string()];
    step.params
        .insert("duration_ms".to_string(), literal(json!(1)));
    step
}

fn root_host_copy_step(
    id: &str,
    source_type: &str,
    source: PathBuf,
    destination: &str,
) -> ExecutionStep {
    let mut step = executor_step(id, "copy_files");
    step.recipe_ref = "fixture.root".to_string();
    step.params.insert(
        "source".to_string(),
        literal(runtime_value(source_type, json!(source), Some("host"))),
    );
    step.params
        .insert("dest".to_string(), literal(json!(destination)));
    step.params
        .insert("copy_policy".to_string(), literal(json!("replace")));
    step
}

fn root_device_copy_step(
    id: &str,
    source_type: &str,
    source: &str,
    destination: &str,
) -> ExecutionStep {
    let mut step = executor_step(id, "copy_files");
    step.recipe_ref = "fixture.root".to_string();
    step.params.insert(
        "source".to_string(),
        literal(runtime_value(source_type, json!(source), Some("device"))),
    );
    step.params
        .insert("dest".to_string(), literal(json!(destination)));
    step.params
        .insert("copy_policy".to_string(), literal(json!("replace")));
    step
}

/// Reviewed copy steps and exact contract-owned roots removed after the group finishes.
struct RootCopyGroupPlan {
    steps: Vec<ExecutionStep>,
    cleanup_paths: Vec<String>,
}

/// Build the dedicated copy qualification as a deterministic reviewed-plan fixture.
///
/// Host staging and device-to-device placement use separate source and destination
/// children across both approved private-path aliases. A linear dependency chain
/// ensures that a failed prerequisite blocks every subsequent copy operation.
fn root_copy_group_plan(contract: &RootQualificationContract) -> RootCopyGroupPlan {
    let data_data_root = format!("{}copy", contract.data_data_prefix);
    let data_user_root = format!("{}copy", contract.data_user_prefix);
    let source_file = format!("{data_data_root}/source-file.txt");
    let source_directory = format!("{data_user_root}/source-directory");
    let copied_file = format!("{data_user_root}/copied-file.txt");
    let copied_directory = format!("{data_data_root}/copied-directory");

    let preflight = root_preflight_step();
    let mut install = fixture_install_step();
    install.dependencies = vec![preflight.id.clone()];
    let mut stage_file = root_host_copy_step(
        "fixture.root/copy-stage-file",
        "file_path",
        fixture_root().join("corpus/source/single-file.txt"),
        &source_file,
    );
    stage_file.dependencies = vec![install.id.clone()];
    stage_file.verify = vec![condition("path_exists", json!({ "path": source_file }))];
    let mut stage_directory = root_host_copy_step(
        "fixture.root/copy-stage-directory",
        "directory_path",
        fixture_root().join("corpus/source/nested"),
        &source_directory,
    );
    stage_directory.dependencies = vec![stage_file.id.clone()];
    stage_directory.verify = vec![condition(
        "path_exists",
        json!({ "path": format!("{source_directory}/alpha/beta/two.txt") }),
    )];
    let mut copy_file = root_device_copy_step(
        "fixture.root/copy-device-file",
        "file_path",
        &source_file,
        &copied_file,
    );
    copy_file.dependencies = vec![stage_directory.id.clone()];
    copy_file.verify = vec![condition("path_exists", json!({ "path": copied_file }))];
    let mut copy_directory = root_device_copy_step(
        "fixture.root/copy-device-directory",
        "directory_path",
        &source_directory,
        &copied_directory,
    );
    copy_directory.dependencies = vec![copy_file.id.clone()];
    copy_directory.verify = vec![condition(
        "path_exists",
        json!({ "path": format!("{copied_directory}/alpha/one.txt") }),
    )];

    RootCopyGroupPlan {
        steps: vec![
            preflight,
            install,
            stage_file,
            stage_directory,
            copy_file,
            copy_directory,
        ],
        cleanup_paths: vec![data_data_root, data_user_root],
    }
}

fn run_root_executor_plan<E: AdbCommandExecutor>(
    device: RealAdbDevice<E>,
    steps: Vec<ExecutionStep>,
) -> crate::executor::ExecutionRunResult {
    let sandbox = tempfile::tempdir().expect("root qualification sandbox should be created");
    let plan = ExecutionPlan {
        id: "plan.phase6c2.root.001".to_string(),
        source: ExecutionPlanSource {
            device_profile_ref: "fixture.root".to_string(),
            device_plan_ref: "fixture.root".to_string(),
            selected_recipe_refs: vec!["fixture.root".to_string()],
            expanded_recipe_refs: vec!["fixture.root".to_string()],
            catalog: None,
        },
        recipes: Vec::new(),
        target_device: None,
        device_context: DeviceContext {
            manufacturer: "Qualification fixture".to_string(),
            model: "Rooted Android device".to_string(),
            android_version: 11,
            android_api_level: Some(30),
            device_tags: Vec::new(),
        },
        runtime_capabilities: RuntimeCapabilities {
            adb_available: true,
            apk_install: true,
            shared_storage_write: true,
            app_launch: true,
            shell_command: true,
            package_remove_for_user: false,
            root_shell: true,
            app_data_write: true,
        },
        inputs: Vec::new(),
        artifacts: Vec::new(),
        steps,
        schema_version: 1,
        kind: "execution_plan",
    };
    let mut runner = ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        sandbox.path().join("runtime"),
        sandbox.path().join("cache"),
        sandbox.path().join("fake-device"),
        vec![fixture_root()],
        false,
    ));
    runner.run(&plan)
}

fn unique_run_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    format!("{}-{timestamp}", std::process::id())
}

fn emit_report(report: &RootQualificationReport) {
    eprintln!(
        "{}",
        serde_json::to_string(report).expect("root qualification report should serialize")
    );
}

fn preflight_failure_report(
    result: &crate::executor::ExecutionRunResult,
) -> Option<RootQualificationReport> {
    result
        .steps
        .first()
        .filter(|step| step.status == crate::executor::StepRunStatus::Failed)
        .map(|_| RootQualificationReport::preflight_failed("root preflight failed"))
}

#[test]
#[ignore = "manual root preflight qualification; requires one deliberately prepared rooted device and every root preflight guard"]
fn manual_real_adb_root_preflight_group() {
    let Some(qualification) = manual_root_qualification(RootQualificationGroup::Preflight) else {
        return;
    };
    let operation = qualification.device().check_root();
    let report = match operation {
        Ok(()) => RootQualificationReport::from_operation_and_cleanup(Ok(()), None, Vec::new()),
        Err(message) => RootQualificationReport::preflight_failed(message),
    };
    emit_report(&report);
    assert_eq!(report.operation, RootOperationOutcome::Succeeded);
    assert_eq!(report.cleanup, RootCleanupOutcome::NotAttempted);
}

#[test]
#[ignore = "manual root filesystem qualification; mutates only approved fixture-private children and requires every destructive guard"]
fn manual_real_adb_root_filesystem_group() {
    let Some(qualification) = manual_root_qualification(RootQualificationGroup::Filesystem) else {
        return;
    };
    let data_data_root = format!("{}filesystem", qualification.contract.data_data_prefix);
    let data_user_root = format!("{}filesystem", qualification.contract.data_user_prefix);
    let copied_file = format!("{data_data_root}/copied.txt");
    let created_tree = format!("{data_user_root}/tree/nested");
    let owned_paths = vec![data_data_root.clone(), data_user_root.clone()];
    qualification.validate_paths(&owned_paths);

    let mut install = fixture_install_step();
    let preflight = root_preflight_step();
    install.dependencies = vec![preflight.id.clone()];
    let mut copy = root_host_copy_step(
        "fixture.root/filesystem-copy",
        "file_path",
        fixture_root().join("corpus/source/single-file.txt"),
        &copied_file,
    );
    copy.dependencies = vec![install.id.clone()];
    copy.verify = vec![condition("path_exists", json!({ "path": copied_file }))];
    let plan_result =
        run_root_executor_plan(qualification.device(), vec![preflight, install, copy]);

    if let Some(report) = preflight_failure_report(&plan_result) {
        emit_report(&report);
        panic!("root filesystem qualification preflight failed");
    }

    let operation = if !plan_result.success {
        Err("root filesystem preparation plan failed".to_string())
    } else {
        let mut operation_device = qualification.authorized_direct_device();
        qualification.qualify_filesystem_operations(
            &mut operation_device,
            &copied_file,
            &created_tree,
        )
    };
    let mut cleanup_device = qualification.authorized_direct_device();
    let (cleanup, residual_paths) =
        qualification.cleanup_paths(&mut cleanup_device, &owned_paths, None);
    let report = RootQualificationReport::from_operation_and_cleanup(
        operation,
        Some(cleanup),
        residual_paths,
    );
    emit_report(&report);
    assert_eq!(report.operation, RootOperationOutcome::Succeeded);
    assert_eq!(report.cleanup, RootCleanupOutcome::Succeeded);
    assert!(report.residual_paths.is_empty());
}

#[test]
#[ignore = "manual root copy qualification; mutates only approved fixture-private children and requires every destructive guard"]
fn manual_real_adb_root_copy_group() {
    let Some(qualification) = manual_root_qualification(RootQualificationGroup::Copy) else {
        return;
    };
    let plan = root_copy_group_plan(&qualification.contract);
    qualification.validate_paths(&plan.cleanup_paths);
    let result = run_root_executor_plan(qualification.device(), plan.steps);
    if let Some(report) = preflight_failure_report(&result) {
        emit_report(&report);
        panic!("root copy qualification preflight failed");
    }
    let operation = result
        .success
        .then_some(())
        .ok_or_else(|| "root copy qualification plan failed".to_string());
    let mut cleanup_device = qualification.authorized_direct_device();
    let (cleanup, residual_paths) =
        qualification.cleanup_paths(&mut cleanup_device, &plan.cleanup_paths, None);
    let report = RootQualificationReport::from_operation_and_cleanup(
        operation,
        Some(cleanup),
        residual_paths,
    );
    emit_report(&report);
    assert_eq!(report.operation, RootOperationOutcome::Succeeded);
    assert_eq!(report.cleanup, RootCleanupOutcome::Succeeded);
}

#[test]
#[ignore = "manual combined root executor qualification; requires one deliberately prepared rooted device and every destructive guard"]
fn manual_real_adb_root_combined_group() {
    let Some(qualification) = manual_root_qualification(RootQualificationGroup::Combined) else {
        return;
    };
    let data_data_root = format!("{}combined", qualification.contract.data_data_prefix);
    let data_user_root = format!("{}combined", qualification.contract.data_user_prefix);
    let source_file = format!("{data_data_root}/source.txt");
    let source_directory = format!("{data_user_root}/source-directory");
    let copied_file = format!("{data_user_root}/copied.txt");
    let copied_directory = format!("{data_data_root}/copied-directory");
    let owned_paths = vec![data_data_root.clone(), data_user_root.clone()];
    qualification.validate_paths(&owned_paths);

    let preflight = root_preflight_step();
    let mut install = fixture_install_step();
    install.dependencies = vec![preflight.id.clone()];
    let mut stage_file = root_host_copy_step(
        "fixture.root/combined-stage-file",
        "file_path",
        fixture_root().join("corpus/source/single-file.txt"),
        &source_file,
    );
    stage_file.dependencies = vec![install.id.clone()];
    let mut stage_directory = root_host_copy_step(
        "fixture.root/combined-stage-directory",
        "directory_path",
        fixture_root().join("corpus/source/nested"),
        &source_directory,
    );
    stage_directory.dependencies = vec![stage_file.id.clone()];
    let mut copy_file = root_device_copy_step(
        "fixture.root/combined-device-file",
        "file_path",
        &source_file,
        &copied_file,
    );
    copy_file.dependencies = vec![stage_directory.id.clone()];
    copy_file.verify = vec![condition("path_exists", json!({ "path": copied_file }))];
    let mut copy_directory = root_device_copy_step(
        "fixture.root/combined-device-directory",
        "directory_path",
        &source_directory,
        &copied_directory,
    );
    copy_directory.dependencies = vec![copy_file.id.clone()];
    copy_directory.verify = vec![condition(
        "path_exists",
        json!({ "path": format!("{copied_directory}/alpha/one.txt") }),
    )];
    let result = run_root_executor_plan(
        qualification.device(),
        vec![
            preflight,
            install,
            stage_file,
            stage_directory,
            copy_file,
            copy_directory,
        ],
    );
    if let Some(report) = preflight_failure_report(&result) {
        emit_report(&report);
        panic!("combined root qualification preflight failed");
    }
    let operation = result
        .success
        .then_some(())
        .ok_or_else(|| "combined root qualification plan failed".to_string());
    let mut cleanup_device = qualification.authorized_direct_device();
    let (cleanup, residual_paths) =
        qualification.cleanup_paths(&mut cleanup_device, &owned_paths, None);
    let report = RootQualificationReport::from_operation_and_cleanup(
        operation,
        Some(cleanup),
        residual_paths,
    );
    emit_report(&report);
    assert_eq!(report.operation, RootOperationOutcome::Succeeded);
    assert_eq!(report.cleanup, RootCleanupOutcome::Succeeded);
}

#[test]
#[ignore = "manual controlled root cleanup-failure qualification; leaves one uniquely named approved child and requires every destructive guard"]
fn manual_real_adb_root_cleanup_failure_group() {
    let Some(qualification) = manual_root_qualification(RootQualificationGroup::CleanupFailure)
    else {
        return;
    };
    let residual = cleanup_failure_path(&qualification.contract, &unique_run_id())
        .expect("generated cleanup-failure path should be bounded");
    qualification.validate_paths(std::slice::from_ref(&residual));
    let preflight = root_preflight_step();
    let mut install = fixture_install_step();
    install.dependencies = vec![preflight.id.clone()];
    let preparation = run_root_executor_plan(qualification.device(), vec![preflight, install]);
    if let Some(report) = preflight_failure_report(&preparation) {
        emit_report(&report);
        panic!("root cleanup-failure qualification preflight failed");
    }
    let operation = if preparation.success {
        let mut operation_device = qualification.authorized_direct_device();
        qualification.create_owned_directory(&mut operation_device, &residual)
    } else {
        Err("root cleanup-failure preparation plan failed".to_string())
    };
    let mut cleanup_device = qualification.authorized_direct_device();
    let (cleanup, residual_paths) = qualification.cleanup_paths(
        &mut cleanup_device,
        std::slice::from_ref(&residual),
        Some(residual.as_str()),
    );
    let report = RootQualificationReport::from_operation_and_cleanup(
        operation,
        Some(cleanup),
        residual_paths,
    );
    emit_report(&report);
    assert_eq!(report.operation, RootOperationOutcome::Succeeded);
    assert_eq!(report.cleanup, RootCleanupOutcome::Failed);
    assert_eq!(report.residual_paths, vec![residual]);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::executor::adb::FakeAdbCommandExecutor;

    fn qualification_for(group: RootQualificationGroup) -> ManualRootQualification {
        ManualRootQualification {
            invocation: RootQualificationInvocation {
                serial: "prepared-device".to_string(),
                group,
            },
            contract: exact_contract(),
        }
    }

    fn granted_root_probe() -> (i32, &'static str, &'static str) {
        (0, "uid=0(root) gid=0(root)\n", "")
    }

    fn plain_result(returncode: i32) -> (i32, &'static str, &'static str) {
        (returncode, "", "")
    }

    fn fake_executor(responses: &[(i32, &'static str, &'static str)]) -> FakeAdbCommandExecutor {
        let mut executor = FakeAdbCommandExecutor::default();
        for (returncode, stdout, stderr) in responses {
            executor.push_completed(*returncode, stdout, stderr);
        }
        executor
    }

    fn filesystem_paths() -> (String, String) {
        (
            format!("{DATA_DATA_PREFIX}filesystem/copied.txt"),
            format!("{DATA_USER_PREFIX}filesystem/tree/nested"),
        )
    }

    fn exact_environment(group: RootQualificationGroup) -> Vec<(&'static str, String)> {
        vec![
            (GLOBAL_OPT_IN, "1".to_string()),
            (ROOT_OPT_IN, "1".to_string()),
            (DEVICE_SERIAL_ENV, "prepared-device".to_string()),
            (PACKAGE_ALLOWLIST_ENV, FIXTURE_PACKAGE.to_string()),
            (PREFIX_ALLOWLIST_ENV, EXPECTED_PREFIX_ALLOWLIST.to_string()),
            (group.env_name(), "1".to_string()),
        ]
    }

    #[test]
    fn committed_contract_has_exact_fixture_authority() {
        let contract = load_root_contract().expect("root qualification contract should load");

        assert_eq!(contract.package_name, FIXTURE_PACKAGE);
        assert_eq!(contract.data_data_prefix, DATA_DATA_PREFIX);
        assert_eq!(contract.data_user_prefix, DATA_USER_PREFIX);
        validate_contract(&contract).expect("committed root contract should be valid");
    }

    #[test]
    fn invocation_requires_every_non_destructive_guard_before_adb() {
        let environment = exact_environment(RootQualificationGroup::Preflight);
        let invocation = validate_invocation(RootQualificationGroup::Preflight, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect("exact preflight guards should be accepted");

        assert_eq!(invocation.serial, "prepared-device");
        assert_eq!(invocation.group, RootQualificationGroup::Preflight);
    }

    #[test]
    fn invocation_rejects_missing_common_authority() {
        let mut environment = exact_environment(RootQualificationGroup::Preflight);
        for (missing, expected) in [
            (ROOT_OPT_IN, format!("{ROOT_OPT_IN} must equal 1")),
            (
                DEVICE_SERIAL_ENV,
                format!("{DEVICE_SERIAL_ENV} must select one exact device"),
            ),
            (
                PACKAGE_ALLOWLIST_ENV,
                format!("{PACKAGE_ALLOWLIST_ENV} must equal {FIXTURE_PACKAGE}"),
            ),
        ] {
            let removed = environment
                .iter()
                .position(|(name, _)| *name == missing)
                .expect("test environment should contain each required value");
            let entry = environment.remove(removed);
            let error = validate_invocation(RootQualificationGroup::Preflight, |name| {
                environment
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.clone())
            })
            .expect_err("missing common authority must fail closed");
            assert_eq!(error, expected);
            environment.insert(removed, entry);
        }
    }

    #[test]
    fn mutating_group_requires_destructive_opt_in() {
        let environment = exact_environment(RootQualificationGroup::Filesystem);
        let error = validate_invocation(RootQualificationGroup::Filesystem, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect_err("filesystem qualification must require destructive authority");

        assert_eq!(
            error,
            "EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS must equal 1"
        );
    }

    #[test]
    fn mutating_group_accepts_exact_destructive_authority() {
        let mut environment = exact_environment(RootQualificationGroup::Filesystem);
        environment.push((DESTRUCTIVE_OPT_IN, "1".to_string()));

        let invocation = validate_invocation(RootQualificationGroup::Filesystem, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect("complete destructive authority should be accepted");

        assert_eq!(invocation.group, RootQualificationGroup::Filesystem);
    }

    #[test]
    fn invocation_rejects_multiple_groups_and_non_exact_allowlists() {
        let mut environment = exact_environment(RootQualificationGroup::Preflight);
        environment.push((RootQualificationGroup::Copy.env_name(), "1".to_string()));
        let multiple = validate_invocation(RootQualificationGroup::Preflight, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect_err("multiple groups must fail closed");
        assert_eq!(
            multiple,
            "exactly one matching root qualification group must be enabled"
        );

        let mut environment = exact_environment(RootQualificationGroup::Preflight);
        environment.retain(|(name, _)| *name != PREFIX_ALLOWLIST_ENV);
        environment.push((PREFIX_ALLOWLIST_ENV, DATA_DATA_PREFIX.to_string()));
        let allowlist = validate_invocation(RootQualificationGroup::Preflight, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .expect_err("partial prefix authority must fail closed");
        assert_eq!(allowlist, "EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST must exactly match the committed root qualification contract");
    }

    #[test]
    fn owned_paths_reject_prefix_equality_traversal_and_unrelated_children() {
        let contract = exact_contract();
        assert!(validate_owned_path(
            &contract,
            "/data/data/com.emuchef.fixture/emuchef-qualification-data/output/file.txt",
        )
        .is_ok());
        assert!(validate_owned_path(&contract, DATA_DATA_PREFIX).is_err());
        assert!(validate_owned_path(
            &contract,
            "/data/data/com.emuchef.fixture/emuchef-qualification-data/../escape",
        )
        .is_err());
        assert!(validate_owned_path(
            &contract,
            "/data/data/com.emuchef.fixture/not-owned/file.txt",
        )
        .is_err());
    }

    #[test]
    fn cleanup_failure_path_is_unique_bounded_child_of_data_user_prefix() {
        let contract = exact_contract();
        let first = cleanup_failure_path(&contract, "123-456").expect("run id should be valid");
        let second = cleanup_failure_path(&contract, "123-457").expect("run id should be valid");

        assert_eq!(
            first,
            "/data/user/0/com.emuchef.fixture/emuchef-qualification-user/cleanup-failure-123-456"
        );
        assert_ne!(first, second);
        assert!(validate_owned_path(&contract, &first).is_ok());
        assert!(cleanup_failure_path(&contract, "../escape").is_err());
    }

    #[test]
    fn copy_group_plan_stages_and_copies_distinct_file_and_directory_paths() {
        let contract = exact_contract();
        let plan = root_copy_group_plan(&contract);
        let steps = serde_json::to_value(&plan.steps).expect("copy plan should serialize");
        let steps = steps
            .as_array()
            .expect("copy plan steps should be an array");

        assert_eq!(
            steps
                .iter()
                .map(|step| step["id"].as_str().expect("step id should be text"))
                .collect::<Vec<_>>(),
            vec![
                "fixture.root/preflight",
                "fixture/install",
                "fixture.root/copy-stage-file",
                "fixture.root/copy-stage-directory",
                "fixture.root/copy-device-file",
                "fixture.root/copy-device-directory",
            ]
        );
        assert_eq!(steps[1]["dependencies"], json!(["fixture.root/preflight"]));
        assert_eq!(steps[2]["dependencies"], json!(["fixture/install"]));
        assert_eq!(
            steps[3]["dependencies"],
            json!(["fixture.root/copy-stage-file"])
        );
        assert_eq!(
            steps[4]["dependencies"],
            json!(["fixture.root/copy-stage-directory"])
        );
        assert_eq!(
            steps[5]["dependencies"],
            json!(["fixture.root/copy-device-file"])
        );
        for step in &steps[2..=5] {
            assert_eq!(step["type"], "copy_files");
            assert_eq!(step["recipe_ref"], "fixture.root");
        }

        let source_file = format!("{DATA_DATA_PREFIX}copy/source-file.txt");
        let source_directory = format!("{DATA_USER_PREFIX}copy/source-directory");
        let copied_file = format!("{DATA_USER_PREFIX}copy/copied-file.txt");
        let copied_directory = format!("{DATA_DATA_PREFIX}copy/copied-directory");
        assert_eq!(steps[2]["params"]["source"]["value"]["location"], "host");
        assert_eq!(steps[2]["params"]["source"]["value"]["type"], "file_path");
        assert_eq!(
            steps[2]["params"]["source"]["value"]["value"],
            json!(fixture_root().join("corpus/source/single-file.txt"))
        );
        assert_eq!(steps[2]["params"]["dest"]["value"], source_file);
        assert_eq!(steps[3]["params"]["source"]["value"]["location"], "host");
        assert_eq!(
            steps[3]["params"]["source"]["value"]["type"],
            "directory_path"
        );
        assert_eq!(
            steps[3]["params"]["source"]["value"]["value"],
            json!(fixture_root().join("corpus/source/nested"))
        );
        assert_eq!(steps[3]["params"]["dest"]["value"], source_directory);
        assert_eq!(steps[4]["params"]["source"]["value"]["location"], "device");
        assert_eq!(steps[4]["params"]["source"]["value"]["type"], "file_path");
        assert_eq!(steps[4]["params"]["source"]["value"]["value"], source_file);
        assert_eq!(steps[4]["params"]["dest"]["value"], copied_file);
        assert_eq!(steps[5]["params"]["source"]["value"]["location"], "device");
        assert_eq!(
            steps[5]["params"]["source"]["value"]["type"],
            "directory_path"
        );
        assert_eq!(
            steps[5]["params"]["source"]["value"]["value"],
            source_directory
        );
        assert_eq!(steps[5]["params"]["dest"]["value"], copied_directory);
        assert_ne!(source_file, copied_file);
        assert_ne!(source_directory, copied_directory);

        assert_eq!(
            steps[2]["verify"][0]["params"]["path"],
            format!("{DATA_DATA_PREFIX}copy/source-file.txt")
        );
        assert_eq!(
            steps[3]["verify"][0]["params"]["path"],
            format!("{DATA_USER_PREFIX}copy/source-directory/alpha/beta/two.txt")
        );
        assert_eq!(
            steps[4]["verify"][0]["params"]["path"],
            format!("{DATA_USER_PREFIX}copy/copied-file.txt")
        );
        assert_eq!(
            steps[5]["verify"][0]["params"]["path"],
            format!("{DATA_DATA_PREFIX}copy/copied-directory/alpha/one.txt")
        );

        assert_eq!(
            plan.cleanup_paths,
            vec![
                format!("{DATA_DATA_PREFIX}copy"),
                format!("{DATA_USER_PREFIX}copy"),
            ]
        );
        for path in &plan.cleanup_paths {
            validate_owned_path(&contract, path).expect("cleanup path should be contract-owned");
        }
    }

    #[test]
    fn inventory_requires_one_online_device_matching_the_selected_serial() {
        let one = "List of devices attached\nprepared-device device product:test model:Test\n";
        assert!(validate_inventory(one, "prepared-device").is_ok());

        let multiple = concat!(
            "List of devices attached\n",
            "prepared-device device product:test model:Test\n",
            "other-device device product:test model:Other\n"
        );
        assert_eq!(
            validate_inventory(multiple, "prepared-device"),
            Err("root qualification requires exactly one online device matching the selected serial".to_string())
        );
        assert!(validate_inventory(
            "List of devices attached\nprepared-device unauthorized\n",
            "prepared-device"
        )
        .is_err());
    }

    #[test]
    fn reports_keep_preflight_operation_and_cleanup_failures_distinct() {
        let preflight = RootQualificationReport::preflight_failed("root unavailable");
        assert_eq!(preflight.operation, RootOperationOutcome::PreflightFailed);
        assert_eq!(preflight.cleanup, RootCleanupOutcome::NotAttempted);

        let combined = RootQualificationReport::from_operation_and_cleanup(
            Err("privileged copy denied".to_string()),
            Some(Err("owned child remains".to_string())),
            vec![format!("{}cleanup-failure-123", DATA_USER_PREFIX)],
        );
        assert_eq!(combined.operation, RootOperationOutcome::OperationFailed);
        assert_eq!(combined.cleanup, RootCleanupOutcome::Failed);
        assert_eq!(combined.residual_paths.len(), 1);
        assert_eq!(
            combined.operation_message.as_deref(),
            Some("privileged copy denied")
        );
        assert_eq!(
            combined.cleanup_message.as_deref(),
            Some("owned child remains")
        );
        let serialized = serde_json::to_string(&combined).expect("report should serialize");
        assert!(!serialized.contains("privileged copy denied"));
        assert!(!serialized.contains("owned child remains"));
    }

    #[test]
    fn direct_helper_rejects_fresh_device_even_after_successful_check_root() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let (copied_file, created_tree) = filesystem_paths();
        let mut device = RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&[granted_root_probe()]),
        );

        let error = qualification
            .qualify_filesystem_operations(&mut device, &copied_file, &created_tree)
            .expect_err("a fresh direct device must not gain authority from check_root");

        assert_eq!(
            error,
            "Continued root authority could not be confirmed safely."
        );
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ends_with(&[
            "shell".to_string(),
            "su".to_string(),
            "-c".to_string(),
            "id".to_string(),
        ]));
    }

    #[test]
    fn authorized_direct_helper_runs_privileged_commands_with_live_revalidation() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let (copied_file, created_tree) = filesystem_paths();
        let tree_root = format!("{DATA_USER_PREFIX}filesystem/tree");
        let responses = [
            granted_root_probe(), // check_root
            granted_root_probe(), // path_exists(copied_file) revalidation
            plain_result(0),      // test -e copied_file
            granted_root_probe(), // path_is_dir(copied_parent) revalidation
            plain_result(0),      // test -d copied_parent
            granted_root_probe(), // mkdir_p revalidation
            plain_result(0),      // mkdir -p created_tree
            granted_root_probe(), // path_is_dir(created_tree) revalidation
            plain_result(0),      // test -d created_tree
            granted_root_probe(), // remove_file revalidation
            plain_result(0),      // rm -f copied_file
            granted_root_probe(), // path_exists(copied_file) revalidation
            plain_result(1),      // test -e copied_file
            granted_root_probe(), // remove_tree revalidation
            plain_result(0),      // rm -rf tree_root
            granted_root_probe(), // path_exists(tree_root) revalidation
            plain_result(1),      // test -e tree_root
        ];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&responses),
        ));

        qualification
            .qualify_filesystem_operations(&mut device, &copied_file, &created_tree)
            .expect("reviewed qualification authority should permit direct filesystem work");

        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 17);
        assert!(calls[0].ends_with(&[
            "shell".to_string(),
            "su".to_string(),
            "-c".to_string(),
            "id".to_string(),
        ]));
        for command_index in (2..calls.len()).step_by(2) {
            assert!(
                calls[command_index - 1].ends_with(&[
                    "shell".to_string(),
                    "su".to_string(),
                    "-c".to_string(),
                    "id".to_string(),
                ]),
                "every privileged command must be preceded by a live root probe"
            );
            let command = calls[command_index]
                .last()
                .expect("command call should include a shell payload");
            assert!(
                command.starts_with("su -c '") && command != "su -c id",
                "privileged command must be su-wrapped"
            );
        }
        assert!(calls[2]
            .last()
            .unwrap()
            .contains(format!("test -e {copied_file}").as_str()));
        assert!(calls[6]
            .last()
            .unwrap()
            .contains(format!("mkdir -p {created_tree}").as_str()));
        assert!(calls[10]
            .last()
            .unwrap()
            .contains(format!("rm -f {copied_file}").as_str()));
        assert!(calls[14]
            .last()
            .unwrap()
            .contains(format!("rm -rf {tree_root}").as_str()));
    }

    #[test]
    fn create_owned_directory_uses_reviewed_authority_and_revalidation() {
        let qualification = qualification_for(RootQualificationGroup::CleanupFailure);
        let path = format!("{DATA_USER_PREFIX}cleanup-failure-123");
        let responses = [
            granted_root_probe(), // check_root
            granted_root_probe(), // mkdir_p revalidation
            plain_result(0),      // mkdir -p path
            granted_root_probe(), // path_is_dir revalidation
            plain_result(0),      // test -d path
        ];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&responses),
        ));

        qualification
            .create_owned_directory(&mut device, &path)
            .expect("reviewed qualification authority should permit owned directory setup");

        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 5);
        assert!(calls[0].ends_with(&[
            "shell".to_string(),
            "su".to_string(),
            "-c".to_string(),
            "id".to_string(),
        ]));
        assert!(calls[2]
            .last()
            .unwrap()
            .contains(format!("mkdir -p {path}").as_str()));
        assert!(calls[4]
            .last()
            .unwrap()
            .contains(format!("test -d {path}").as_str()));
    }

    #[test]
    fn root_denial_after_authorization_fails_before_mutation() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let (copied_file, created_tree) = filesystem_paths();
        let responses = [
            granted_root_probe(),             // check_root
            (1, "", "su: permission denied"), // revalidation denial
        ];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&responses),
        ));

        let error = qualification
            .qualify_filesystem_operations(&mut device, &copied_file, &created_tree)
            .expect_err("live root denial must fail the direct operation");

        assert_eq!(error, "Root authority was revoked during execution.");
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 2);
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument.contains("test -e"))));
    }

    #[test]
    fn root_timeout_after_authorization_fails_before_mutation() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let (copied_file, created_tree) = filesystem_paths();
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "uid=0(root) gid=0(root)\n", "");
        executor.push_timed_out();
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            executor,
        ));

        let error = qualification
            .qualify_filesystem_operations(&mut device, &copied_file, &created_tree)
            .expect_err("live root timeout must fail the direct operation");

        assert_eq!(error, "The ADB operation timed out.");
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 2);
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument.contains("test -e"))));
    }

    #[test]
    fn cleanup_removes_only_contract_owned_children_with_revalidation() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let owned_paths = vec![
            format!("{DATA_DATA_PREFIX}filesystem"),
            format!("{DATA_USER_PREFIX}filesystem"),
        ];
        let responses = [
            granted_root_probe(), // check_root
            granted_root_probe(), // remove_tree(data_data) revalidation
            plain_result(0),      // rm -rf data_data
            granted_root_probe(), // path_exists(data_data) revalidation
            plain_result(1),      // test -e data_data
            granted_root_probe(), // remove_tree(data_user) revalidation
            plain_result(0),      // rm -rf data_user
            granted_root_probe(), // path_exists(data_user) revalidation
            plain_result(1),      // test -e data_user
        ];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&responses),
        ));

        let (cleanup, residual_paths) =
            qualification.cleanup_paths(&mut device, &owned_paths, None);

        assert_eq!(cleanup, Ok(()));
        assert!(residual_paths.is_empty());
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 9);
        for command_index in [2, 4, 6, 8] {
            assert!(calls[command_index - 1].ends_with(&[
                "shell".to_string(),
                "su".to_string(),
                "-c".to_string(),
                "id".to_string(),
            ]));
        }
        assert!(calls[2]
            .last()
            .unwrap()
            .contains(format!("rm -rf {}", owned_paths[0]).as_str()));
        assert!(calls[6]
            .last()
            .unwrap()
            .contains(format!("rm -rf {}", owned_paths[1]).as_str()));
    }

    #[test]
    fn cleanup_root_revocation_reports_residual_without_mutation() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let owned_paths = vec![format!("{DATA_DATA_PREFIX}filesystem")];
        let responses = [
            granted_root_probe(),             // check_root
            (1, "", "su: permission denied"), // remove_tree revalidation denial
            (1, "", "su: permission denied"), // path_exists revalidation denial
        ];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&responses),
        ));

        let (cleanup, residual_paths) =
            qualification.cleanup_paths(&mut device, &owned_paths, None);

        assert_eq!(
            cleanup,
            Err("root qualification cleanup left contract-owned residual state".to_string())
        );
        assert_eq!(residual_paths, owned_paths);
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 3);
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument.contains("rm -rf"))));
    }

    #[test]
    fn cleanup_skips_injected_residual_and_reports_it() {
        let qualification = qualification_for(RootQualificationGroup::CleanupFailure);
        let residual = format!("{DATA_USER_PREFIX}cleanup-failure-123");
        let owned_paths = vec![residual.clone()];
        let mut device = with_reviewed_root_authority(RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&[granted_root_probe()]),
        ));

        let (cleanup, residual_paths) =
            qualification.cleanup_paths(&mut device, &owned_paths, Some(&residual));

        assert_eq!(
            cleanup,
            Err("root qualification cleanup left contract-owned residual state".to_string())
        );
        assert_eq!(residual_paths, vec![residual]);
        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 1);
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument.contains("rm -rf"))));
    }

    #[test]
    #[should_panic(expected = "physical root qualification path must remain contract-owned")]
    fn cleanup_rejects_outside_prefix_before_any_device_command() {
        let qualification = qualification_for(RootQualificationGroup::Filesystem);
        let mut device = RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            FakeAdbCommandExecutor::default(),
        );
        let _ = qualification.cleanup_paths(
            &mut device,
            &["/data/data/com.other.app/escape".to_string()],
            None,
        );
    }

    #[test]
    fn plan_runner_derives_reviewed_root_authority_for_fresh_direct_device() {
        let copied_file = format!("{DATA_DATA_PREFIX}filesystem/copied.txt");
        let device = RealAdbDevice::with_executor(
            "adb",
            Some("prepared-device"),
            fake_executor(&[granted_root_probe(), plain_result(0)]),
        );
        let mut step = executor_step("fixture.root/verify", "wait");
        step.recipe_ref = "fixture.root".to_string();
        step.constraints.capabilities = vec!["root_shell".to_string()];
        step.params
            .insert("duration_ms".to_string(), literal(json!(1)));
        step.verify = vec![condition("path_exists", json!({ "path": copied_file }))];

        let result = run_root_executor_plan(device, vec![step]);

        assert!(
            result.success,
            "the reviewed root-capable plan should authorize the fresh device"
        );
    }
}

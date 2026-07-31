use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::executor::adb::RealAdbDevice;
use crate::executor::{ExecutorAdapters, ExecutorRunner};
use crate::model::OrderedMap;
use crate::planner::{
    DeviceContext, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource, ExecutionStep,
    ExecutionStepCondition, ExecutionStepConstraints, RuntimeCapabilities,
};

mod qualification;
mod root_qualification;

use qualification::{
    classify_permission_appop_support, classify_preflight, load_contract, validate_group_opt_ins,
    validate_owned_destination, validate_package, QualificationClassification,
    QualificationContract, QualificationGroup, FIXTURE_PACKAGE, GLOBAL_OPT_IN,
};

#[test]
#[ignore = "manual mutating executor qualification; requires the install/package group and one prepared non-root device"]
fn manual_real_adb_install_package_group() {
    let Some(mut qualification) = manual_qualification(QualificationGroup::InstallPackage) else {
        return;
    };

    let install = fixture_install_step();
    let preparation = qualification.cleanup(false);
    let package_absent_before = qualification.package_installed();
    let result = run_executor_plan(qualification.device(), vec![install.clone()]);
    let package_present_after_install = qualification.package_installed();
    let idempotent = run_executor_plan(qualification.device(), vec![install]);
    let cleanup = qualification.cleanup(false);
    assert!(
        preparation.success,
        "fixture preparation cleanup result: {preparation:#?}"
    );
    assert_eq!(package_absent_before, Ok(false));
    assert!(result.success, "fixture install result: {result:#?}");
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(package_present_after_install, Ok(true));
    assert!(
        idempotent.success,
        "fixture idempotency result: {idempotent:#?}"
    );
    assert_eq!(
        idempotent.steps[0].status,
        crate::executor::StepRunStatus::Skipped
    );
    assert!(cleanup.success, "fixture cleanup result: {cleanup:#?}");
    assert!(!cleanup.package_remaining);
    assert!(!cleanup.process_remaining);
    assert_ne!(cleanup.appop_remaining, Some(true));
    assert!(cleanup.residual_roots.is_empty());
}

#[test]
#[ignore = "manual mutating executor qualification; requires the copy/extraction group and one prepared non-root device"]
fn manual_real_adb_copy_extraction_group() {
    let Some(mut qualification) = manual_qualification(QualificationGroup::CopyExtraction) else {
        return;
    };
    let fixture_root = fixture_root();
    let destination_root = qualification
        .contract
        .destination_root
        .trim_end_matches('/')
        .to_string();
    let single_destination = format!("{destination_root}/single");
    let directory_destination = format!("{destination_root}/nested");
    let archive_destination = format!("{destination_root}/archive");
    for destination in [
        &single_destination,
        &directory_destination,
        &archive_destination,
    ] {
        validate_owned_destination(&qualification.contract, destination, false)
            .expect("qualification destinations must remain manifest-owned");
    }

    let mut single = copy_step(
        "fixture/copy-single",
        "file_path",
        fixture_root.join("corpus/source/single-file.txt"),
        &single_destination,
    );
    single.verify = vec![condition(
        "path_exists",
        json!({ "path": single_destination }),
    )];
    let mut directory = copy_step(
        "fixture/copy-directory",
        "directory_path",
        fixture_root.join("corpus/source/nested"),
        &directory_destination,
    );
    directory.dependencies = vec![single.id.clone()];
    directory.verify = vec![condition(
        "path_exists",
        json!({ "path": format!("{directory_destination}/alpha/one.txt") }),
    )];
    let mut archive = executor_step("fixture/extract-archive", "extract_archive");
    archive.dependencies = vec![directory.id.clone()];
    archive.params.insert(
        "archive".to_string(),
        literal(runtime_value(
            "file_path",
            json!(fixture_root.join("corpus/archive.zip")),
            Some("host"),
        )),
    );
    archive
        .params
        .insert("extract_on".to_string(), literal(json!("device")));
    archive
        .params
        .insert("dest".to_string(), literal(json!(archive_destination)));
    archive.verify = vec![condition(
        "path_exists",
        json!({ "path": format!("{archive_destination}/alpha") }),
    )];
    let mut idempotent = copy_step(
        "fixture/copy-single-idempotent",
        "file_path",
        fixture_root.join("corpus/source/single-file.txt"),
        &single_destination,
    );
    idempotent.dependencies = vec![archive.id.clone()];
    idempotent.skip_if = vec![condition(
        "path_exists",
        json!({ "path": single_destination }),
    )];

    let result = run_executor_plan(
        qualification.device(),
        vec![single, directory, archive, idempotent],
    );
    let cleanup = qualification.cleanup(false);
    assert!(result.success, "copy/extraction result: {result:#?}");
    assert_eq!(
        result.steps.last().unwrap().status,
        crate::executor::StepRunStatus::Skipped
    );
    assert!(cleanup.success, "fixture cleanup result: {cleanup:#?}");
    assert!(!cleanup.process_remaining);
    assert!(cleanup.residual_roots.is_empty());

    let verification_destination = format!("{destination_root}/verification-failure.txt");
    let missing_verification = format!("{destination_root}/verification-missing.txt");
    validate_owned_destination(&qualification.contract, &verification_destination, false)
        .expect("verification destination must remain manifest-owned");
    validate_owned_destination(&qualification.contract, &missing_verification, false)
        .expect("verification predicate must remain manifest-owned");
    let mut verification_failure = copy_step(
        "fixture/verification-failure",
        "file_path",
        fixture_root.join("corpus/source/single-file.txt"),
        &verification_destination,
    );
    verification_failure.verify = vec![condition(
        "path_exists",
        json!({ "path": missing_verification }),
    )];
    let verification_result = run_executor_plan(qualification.device(), vec![verification_failure]);
    let verification_cleanup = qualification.cleanup(false);
    assert!(!verification_result.success);
    assert_eq!(
        verification_result.steps[0].status,
        crate::executor::StepRunStatus::Failed
    );
    assert!(
        verification_cleanup.success,
        "verification-failure cleanup result: {verification_cleanup:#?}"
    );

    let partial_destination = format!("{destination_root}/partial-setup.txt");
    let failing_destination = format!("{destination_root}/execution-failure.txt");
    for destination in [&partial_destination, &failing_destination] {
        validate_owned_destination(&qualification.contract, destination, false)
            .expect("execution-failure destinations must remain manifest-owned");
    }
    let partial_setup = copy_step(
        "fixture/partial-setup",
        "file_path",
        fixture_root.join("corpus/source/single-file.txt"),
        &partial_destination,
    );
    let mut execution_failure = copy_step(
        "fixture/execution-failure",
        "file_path",
        fixture_root.join("corpus/source/missing-file.txt"),
        &failing_destination,
    );
    execution_failure.dependencies = vec![partial_setup.id.clone()];
    let execution_result = run_executor_plan(
        qualification.device(),
        vec![partial_setup, execution_failure],
    );
    let execution_cleanup = qualification.cleanup(false);
    assert!(!execution_result.success);
    assert_eq!(
        execution_result.steps[0].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(
        execution_result.steps[1].status,
        crate::executor::StepRunStatus::Failed
    );
    assert!(
        execution_cleanup.success,
        "execution-failure cleanup result: {execution_cleanup:#?}"
    );
}

#[test]
#[ignore = "manual mutating executor qualification; requires the permission/app-op group and one prepared non-root device"]
fn manual_real_adb_permission_appop_group() {
    let Some(mut qualification) = manual_qualification(QualificationGroup::PermissionAppop) else {
        return;
    };
    let preparation = qualification.cleanup(false);
    let mut permissions = executor_step("fixture/permissions", "grant_permissions");
    permissions.dependencies = vec!["fixture/install".to_string()];
    permissions.params.insert(
        "runtime".to_string(),
        literal(json!([{
            "package_name": FIXTURE_PACKAGE,
            "name": "android.permission.CAMERA",
            "required": true
        }])),
    );
    permissions.params.insert(
        "appops".to_string(),
        literal(json!([{
            "package_name": FIXTURE_PACKAGE,
            "op": "CAMERA",
            "mode": "allow",
            "required": false
        }])),
    );
    permissions.params.insert(
        "policy".to_string(),
        literal(json!({ "on_failure": "warn", "require_all": false })),
    );
    let mut already_granted = permissions.clone();
    already_granted.id = "fixture/permissions-already-granted".to_string();
    already_granted.dependencies = vec![permissions.id.clone()];

    let result = run_executor_plan(
        qualification.device(),
        vec![fixture_install_step(), permissions, already_granted],
    );
    let camera_granted = qualification.camera_permission_granted();
    let appop_allowed = qualification
        .appop_available
        .then(|| qualification.camera_appop_allowed());
    let appop_outcome = classify_permission_appop_support(true, qualification.appop_available);
    let cleanup = qualification.cleanup(false);
    assert!(
        preparation.success,
        "fixture preparation cleanup result: {preparation:#?}"
    );
    assert!(result.success, "permission/app-op result: {result:#?}");
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(
        result.steps[1].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(
        result.steps[2].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(camera_granted, Ok(true));
    if let Some(appop_allowed) = appop_allowed {
        assert_eq!(appop_allowed, Ok(true));
        assert_eq!(
            appop_outcome.classification,
            QualificationClassification::Supported
        );
    } else {
        assert_eq!(
            appop_outcome.classification,
            QualificationClassification::Unsupported
        );
        eprintln!(
            "{}",
            serde_json::to_string(&appop_outcome)
                .expect("unsupported app-op outcome should be serializable")
        );
    }
    assert!(cleanup.success, "fixture cleanup result: {cleanup:#?}");
    assert_ne!(cleanup.camera_permission_remaining, Some(true));
    assert_ne!(cleanup.appop_remaining, Some(true));
    assert!(!cleanup.process_remaining);
    eprintln!(
        "Qualification cleanup app-op reset supported: {}",
        cleanup.appop_reset_supported
    );
}

#[test]
#[ignore = "manual mutating executor qualification; requires the launch/force-stop group and one prepared non-root device"]
fn manual_real_adb_launch_force_stop_group() {
    let Some(mut qualification) = manual_qualification(QualificationGroup::LaunchForceStop) else {
        return;
    };
    let preparation = qualification.cleanup(false);
    let mut launch = executor_step("fixture/launch", "launch_app");
    launch.dependencies = vec!["fixture/install".to_string()];
    launch
        .params
        .insert("package_name".to_string(), literal(json!(FIXTURE_PACKAGE)));
    launch.params.insert(
        "activity".to_string(),
        literal(json!("com.emuchef.fixture.MainActivity")),
    );
    let mut stop = executor_step("fixture/force-stop", "force_stop_app");
    stop.params
        .insert("package_name".to_string(), literal(json!(FIXTURE_PACKAGE)));
    let mut stop_again = stop.clone();
    stop_again.id = "fixture/force-stop-again".to_string();
    stop_again.dependencies = vec![stop.id.clone()];

    let launch_result =
        run_executor_plan(qualification.device(), vec![fixture_install_step(), launch]);
    let process_running_after_launch = qualification.process_running();
    let stop_result = run_executor_plan(qualification.device(), vec![stop, stop_again]);
    let cleanup = qualification.cleanup(false);
    assert!(
        preparation.success,
        "fixture preparation cleanup result: {preparation:#?}"
    );
    assert!(
        launch_result.success,
        "fixture launch result: {launch_result:#?}"
    );
    assert_eq!(
        launch_result.steps[1].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(process_running_after_launch, Ok(true));
    assert!(
        stop_result.success,
        "fixture force-stop result: {stop_result:#?}"
    );
    assert_eq!(
        stop_result.steps[0].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(
        stop_result.steps[1].status,
        crate::executor::StepRunStatus::Executed
    );
    assert!(cleanup.success, "fixture cleanup result: {cleanup:#?}");
    assert!(!cleanup.package_remaining);
    assert!(!cleanup.process_remaining);
}

#[test]
#[ignore = "manual controlled cleanup-report qualification; requires the cleanup-failure group and one prepared non-root device"]
fn manual_real_adb_cleanup_failure_group() {
    let Some(mut qualification) = manual_qualification(QualificationGroup::CleanupFailure) else {
        return;
    };
    let destination = format!(
        "{}/cleanup-failure-marker",
        qualification
            .contract
            .destination_root
            .trim_end_matches('/')
    );
    let copy = copy_step(
        "fixture/cleanup-failure-setup",
        "file_path",
        fixture_root().join("corpus/source/single-file.txt"),
        &destination,
    );
    let result = run_executor_plan(qualification.device(), vec![copy]);
    let cleanup = qualification.cleanup(true);
    assert!(result.success, "cleanup failure setup result: {result:#?}");
    assert!(!cleanup.success);
    assert_eq!(cleanup.classification, "cleanup_failed");
    assert!(!cleanup.residual_roots.is_empty());
    assert!(!cleanup.package_remaining);
    assert!(!cleanup.process_remaining);
    assert_ne!(cleanup.camera_permission_remaining, Some(true));
    assert_ne!(cleanup.appop_remaining, Some(true));
}

fn fixture_apk_path() -> PathBuf {
    fixture_root().join("android-fixture/fixture.apk")
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .join("tests/fixtures/phase-6c/non-root")
}

fn fixture_apk_checksum() -> String {
    std::fs::read_to_string(
        fixture_apk_path()
            .parent()
            .expect("fixture APK should have a parent directory")
            .join("fixture.apk.sha256"),
    )
    .expect("fixture checksum should be readable")
    .trim()
    .to_string()
}

fn fixture_install_step() -> ExecutionStep {
    let mut install = executor_step("fixture/install", "install_apk");
    install.params.insert(
        "app".to_string(),
        literal(runtime_value(
            "file_path",
            json!(fixture_apk_path()),
            Some("host"),
        )),
    );
    install.params.insert(
        "expected_package_name".to_string(),
        literal(json!(FIXTURE_PACKAGE)),
    );
    install.params.insert(
        "expected_sha256".to_string(),
        literal(json!(fixture_apk_checksum())),
    );
    install
        .params
        .insert("replace_existing".to_string(), literal(json!(true)));
    install.skip_if = vec![condition(
        "package_installed",
        json!({ "package_name": FIXTURE_PACKAGE }),
    )];
    install.verify = vec![condition(
        "package_installed",
        json!({ "package_name": FIXTURE_PACKAGE }),
    )];
    install
}

fn run_executor_plan(
    device: RealAdbDevice,
    steps: Vec<ExecutionStep>,
) -> crate::executor::ExecutionRunResult {
    let sandbox = tempfile::tempdir().expect("qualification sandbox should be created");
    let plan = ExecutionPlan {
        id: "plan.phase6c.fixture.001".to_string(),
        source: ExecutionPlanSource {
            device_profile_ref: "fixture.non_root".to_string(),
            device_plan_ref: "fixture.non_root".to_string(),
            selected_recipe_refs: vec!["fixture.non_root".to_string()],
            expanded_recipe_refs: vec!["fixture.non_root".to_string()],
            catalog: None,
        },
        recipes: Vec::new(),
        target_device: None,
        device_context: DeviceContext {
            manufacturer: "Qualification fixture".to_string(),
            model: "Non-root Android device".to_string(),
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
            root_shell: false,
            app_data_write: false,
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

fn executor_step(id: &str, type_name: &str) -> ExecutionStep {
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "fixture.non_root".to_string(),
        type_name: type_name.to_string(),
        name: id.to_string(),
        note: id.to_string(),
        dependencies: Vec::new(),
        constraints: ExecutionStepConstraints {
            capabilities: Vec::new(),
            conflicts_with: Vec::new(),
        },
        params: OrderedMap::new(),
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

fn copy_step(id: &str, source_type: &str, source: PathBuf, destination: &str) -> ExecutionStep {
    let mut step = executor_step(id, "copy_files");
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

fn literal(value: Value) -> ExecutionParamValue {
    ExecutionParamValue::Literal { value }
}

fn runtime_value(type_name: &str, value: Value, location: Option<&str>) -> Value {
    json!({ "type": type_name, "value": value, "location": location })
}

fn condition(type_name: &str, params: Value) -> ExecutionStepCondition {
    let mut condition_params = OrderedMap::new();
    for (key, value) in params
        .as_object()
        .expect("qualification condition parameters should be an object")
    {
        condition_params.insert(key.clone(), value.clone());
    }
    ExecutionStepCondition {
        type_name: type_name.to_string(),
        params: condition_params,
    }
}

#[derive(Debug)]
struct ManualQualification {
    serial: String,
    contract: QualificationContract,
    appop_available: bool,
}

#[derive(Debug)]
struct CleanupReport {
    success: bool,
    classification: &'static str,
    residual_roots: Vec<String>,
    package_remaining: bool,
    camera_permission_remaining: Option<bool>,
    appop_reset_supported: bool,
    appop_remaining: Option<bool>,
    process_remaining: bool,
}

impl ManualQualification {
    fn device(&self) -> RealAdbDevice {
        RealAdbDevice::new("adb", Some(self.serial.clone()))
    }

    fn package_installed(&self) -> Result<bool, String> {
        adb_query(
            &self.serial,
            &["shell", "pm", "list", "packages", FIXTURE_PACKAGE],
        )
        .map(|output| {
            output
                .lines()
                .any(|line| line.trim() == format!("package:{FIXTURE_PACKAGE}"))
        })
    }

    fn camera_permission_granted(&self) -> Result<bool, String> {
        adb_query(
            &self.serial,
            &["shell", "dumpsys", "package", FIXTURE_PACKAGE],
        )
        .map(|output| {
            output.contains("android.permission.CAMERA: granted=true")
                || output.contains("android.permission.CAMERA granted=true")
        })
    }

    fn camera_appop_allowed(&self) -> Result<bool, String> {
        adb_query(
            &self.serial,
            &["shell", "appops", "get", FIXTURE_PACKAGE, "CAMERA"],
        )
        .map(|output| output.contains("allow"))
    }

    fn process_running(&self) -> Result<bool, String> {
        adb_query(&self.serial, &["shell", "pidof", FIXTURE_PACKAGE])
            .map(|output| !output.trim().is_empty())
    }

    fn cleanup(&mut self, inject_owned_root_failure: bool) -> CleanupReport {
        validate_package(
            FIXTURE_PACKAGE,
            optional_env("EMUCHEF_TEST_PACKAGE_ALLOWLIST").as_deref(),
        )
        .expect("cleanup package authority must remain valid");
        let _ = adb_command(
            &self.serial,
            &["shell", "am", "force-stop", FIXTURE_PACKAGE],
        );
        let permission_reset = adb_command(
            &self.serial,
            &[
                "shell",
                "pm",
                "revoke",
                FIXTURE_PACKAGE,
                "android.permission.CAMERA",
            ],
        );
        let appop_reset_supported =
            adb_command(&self.serial, &["shell", "appops", "reset", FIXTURE_PACKAGE]).is_ok();
        let _ = adb_command(&self.serial, &["uninstall", FIXTURE_PACKAGE]);

        let mut cleanup_failed = false;
        let mut residual_roots = Vec::new();
        for root in [
            &self.contract.shared_storage_root,
            &self.contract.app_specific_external_storage_root,
        ] {
            let root = validate_owned_destination(&self.contract, root, true)
                .expect("cleanup root must be declared by the qualification contract");
            let removal_failed = if inject_owned_root_failure
                && root == self.contract.shared_storage_root.trim_end_matches('/')
            {
                true
            } else {
                adb_command(&self.serial, &["shell", "rm", "-rf", &root]).is_err()
            };
            let root_remaining = match adb_path_exists(&self.serial, &root) {
                Ok(remaining) => remaining,
                Err(_) => true,
            };
            if removal_failed || root_remaining {
                cleanup_failed = true;
                residual_roots.push(root);
            }
        }

        let package_remaining = adb_query(&self.serial, &["shell", "pm", "path", FIXTURE_PACKAGE])
            .is_ok_and(|output| output.contains("package:"));
        let camera_permission_remaining = adb_query(
            &self.serial,
            &["shell", "dumpsys", "package", FIXTURE_PACKAGE],
        )
        .ok()
        .map(|output| {
            output.contains("android.permission.CAMERA: granted=true")
                || output.contains("android.permission.CAMERA granted=true")
        });
        let appop_remaining = adb_query(
            &self.serial,
            &["shell", "appops", "get", FIXTURE_PACKAGE, "CAMERA"],
        )
        .ok()
        .map(|output| output.contains("allow"));
        let process_remaining = adb_query(&self.serial, &["shell", "pidof", FIXTURE_PACKAGE])
            .is_ok_and(|output| !output.trim().is_empty());
        if package_remaining
            || camera_permission_remaining == Some(true)
            || appop_remaining == Some(true)
            || process_remaining
        {
            cleanup_failed = true;
        }

        CleanupReport {
            success: !cleanup_failed,
            classification: if cleanup_failed {
                "cleanup_failed"
            } else {
                "cleanup_succeeded"
            },
            residual_roots,
            package_remaining,
            camera_permission_remaining: permission_reset
                .ok()
                .map(|_| camera_permission_remaining.unwrap_or(false)),
            appop_reset_supported,
            appop_remaining,
            process_remaining,
        }
    }
}

fn manual_qualification(required: QualificationGroup) -> Option<ManualQualification> {
    if env::var(GLOBAL_OPT_IN).ok().as_deref() != Some("1") {
        eprintln!("Skipping: set {GLOBAL_OPT_IN}=1 to run manual real-ADB tests.");
        return None;
    }
    let enabled = QualificationGroup::ALL
        .into_iter()
        .filter(|group| env::var(group.env_name()).ok().as_deref() == Some("1"))
        .collect::<Vec<_>>();
    validate_group_opt_ins(Some("1"), &enabled, required)
        .expect("exactly one matching qualification group must be enabled");
    let serial = optional_env("EMUCHEF_TEST_DEVICE_SERIAL")
        .expect("EMUCHEF_TEST_DEVICE_SERIAL must select the prepared device");
    validate_package(
        FIXTURE_PACKAGE,
        optional_env("EMUCHEF_TEST_PACKAGE_ALLOWLIST").as_deref(),
    )
    .expect("the exact fixture package must be allowlisted");
    let contract = load_contract().expect("qualification contract must be valid");

    let inventory = adb_inventory().expect("ADB inventory must be available");
    let online = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let candidate = fields.next()?;
            (fields.next() == Some("device")).then_some(candidate)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        online,
        vec![serial.as_str()],
        "Qualification requires exactly one online device matching the selected serial."
    );
    let api = adb_query(&serial, &["shell", "getprop", "ro.build.version.sdk"])
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    let uid = adb_query(&serial, &["shell", "id", "-u"])
        .expect("non-root qualification must be readable");
    assert_ne!(
        uid.trim(),
        "0",
        "Phase 6C.1 refuses a device whose shell is root."
    );
    let package_manager_available = !matches!(
        required,
        QualificationGroup::InstallPackage | QualificationGroup::PermissionAppop
    ) || adb_query(&serial, &["shell", "pm", "path", "android"])
        .is_ok();
    let activity_manager_available = required != QualificationGroup::LaunchForceStop
        || adb_query(&serial, &["shell", "am", "help"]).is_ok();
    let shared_storage_available = !matches!(
        required,
        QualificationGroup::CopyExtraction | QualificationGroup::CleanupFailure
    ) || adb_query(&serial, &["shell", "test", "-d", "/sdcard"])
        .is_ok();
    let outcome = classify_preflight(
        required,
        api,
        package_manager_available,
        activity_manager_available,
        shared_storage_available,
    );
    if outcome.classification == QualificationClassification::Unsupported {
        eprintln!(
            "{}",
            serde_json::to_string(&outcome).expect("qualification outcome should be serializable")
        );
        return None;
    }
    let appop_available = required != QualificationGroup::PermissionAppop
        || adb_query(&serial, &["shell", "appops", "help"]).is_ok();
    let camera_permission_available = required != QualificationGroup::PermissionAppop
        || adb_query(&serial, &["shell", "pm", "list", "permissions"])
            .is_ok_and(|output| output.contains("android.permission.CAMERA"));
    if required == QualificationGroup::PermissionAppop {
        let outcome =
            classify_permission_appop_support(camera_permission_available, appop_available);
        if outcome.classification == QualificationClassification::Unsupported {
            eprintln!(
                "{}",
                serde_json::to_string(&outcome)
                    .expect("unsupported permission/app-op outcome should be serializable")
            );
            return None;
        }
    }
    Some(ManualQualification {
        serial,
        contract,
        appop_available,
    })
}

fn adb_inventory() -> Result<String, String> {
    let output = Command::new("adb")
        .args(["devices", "-l"])
        .output()
        .map_err(|_| "ADB inventory command is unavailable".to_string())?;
    if !output.status.success() {
        return Err("ADB inventory command failed".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn adb_query(serial: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("adb")
        .args(["-s", serial])
        .args(args)
        .output()
        .map_err(|_| "ADB qualification command is unavailable".to_string())?;
    if !output.status.success() {
        return Err("ADB qualification command failed".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn adb_command(serial: &str, args: &[&str]) -> Result<(), String> {
    adb_query(serial, args).map(|_| ())
}

fn adb_path_exists(serial: &str, path: &str) -> Result<bool, String> {
    let output = Command::new("adb")
        .args(["-s", serial, "shell", "test", "-e", path])
        .output()
        .map_err(|_| "ADB residual-path query is unavailable".to_string())?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
        return Ok(false);
    }
    Err("ADB residual-path query failed".to_string())
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

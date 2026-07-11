use std::env;
use std::path::PathBuf;

use crate::executor::adb::RealAdbDevice;

const GLOBAL_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_TESTS";

#[test]
#[ignore = "manual real-ADB test; requires EMUCHEF_RUN_REAL_ADB_TESTS=1 and an attached test device"]
fn manual_real_adb_package_installed_check() {
    let Some(mut device) = manual_device() else {
        return;
    };
    let Some(package) = optional_env("EMUCHEF_TEST_PACKAGE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_PACKAGE to check package installation state.");
        return;
    };

    let installed = device
        .package_installed(&package)
        .expect("manual package check should complete through real ADB");
    eprintln!("Package {package} installed: {installed}");
}

#[test]
#[ignore = "manual real-ADB test; requires EMUCHEF_RUN_REAL_ADB_TESTS=1 and an attached test device"]
fn manual_real_adb_path_exists_check() {
    let Some(mut device) = manual_device() else {
        return;
    };
    let path = optional_env("EMUCHEF_TEST_DEVICE_PATH").unwrap_or_else(|| "/sdcard".to_string());

    let exists = device
        .path_exists(&path)
        .expect("manual path check should complete through real ADB");
    eprintln!("Device path {path} exists: {exists}");
}

#[test]
#[ignore = "manual mutating real-ADB test; requires explicit install opt-in and test APK"]
fn manual_real_adb_install_apk_requires_explicit_opt_in() {
    let Some(mut device) = manual_device() else {
        return;
    };
    if !per_test_opt_in("EMUCHEF_RUN_REAL_ADB_INSTALL_TEST") {
        return;
    }
    let Some(apk) = optional_env("EMUCHEF_TEST_APK").map(PathBuf::from) else {
        eprintln!("Skipping: set EMUCHEF_TEST_APK to a test-owned APK path.");
        return;
    };

    device
        .install_apk(&apk, true)
        .expect("manual APK install should complete through real ADB");
}

#[test]
#[ignore = "manual mutating real-ADB test; requires explicit launch opt-in and test package"]
fn manual_real_adb_launch_app_requires_explicit_opt_in() {
    let Some(mut device) = manual_device() else {
        return;
    };
    if !per_test_opt_in("EMUCHEF_RUN_REAL_ADB_LAUNCH_TEST") {
        return;
    }
    let Some(package) = optional_env("EMUCHEF_TEST_PACKAGE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_PACKAGE to a test-owned package.");
        return;
    };

    device
        .launch_app(&package, optional_env("EMUCHEF_TEST_ACTIVITY").as_deref())
        .expect("manual app launch should complete through real ADB");
}

#[test]
#[ignore = "manual mutating real-ADB test; requires explicit force-stop opt-in and test package"]
fn manual_real_adb_force_stop_app_requires_explicit_opt_in() {
    let Some(mut device) = manual_device() else {
        return;
    };
    if !per_test_opt_in("EMUCHEF_RUN_REAL_ADB_FORCE_STOP_TEST") {
        return;
    }
    let Some(package) = optional_env("EMUCHEF_TEST_PACKAGE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_PACKAGE to a test-owned package.");
        return;
    };
    guard_test_package(&package);

    device
        .force_stop_app(&package)
        .expect("manual force-stop should complete through real ADB");
}

#[test]
#[ignore = "manual mutating real-ADB test; requires explicit permission opt-in and package allowlist"]
fn manual_real_adb_runtime_permission_requires_allowlist() {
    let Some(mut device) = manual_device() else {
        return;
    };
    if !per_test_opt_in("EMUCHEF_RUN_REAL_ADB_PERMISSION_TEST") {
        return;
    }
    let Some(package) = optional_env("EMUCHEF_TEST_PACKAGE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_PACKAGE to a test-owned package.");
        return;
    };
    guard_test_package(&package);
    let Some(permission) = optional_env("EMUCHEF_TEST_RUNTIME_PERMISSION") else {
        eprintln!("Skipping: set EMUCHEF_TEST_RUNTIME_PERMISSION for the test-owned package.");
        return;
    };

    device
        .run_plan_command(vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            package,
            permission,
        ])
        .expect("manual permission grant should complete through real ADB");
}

#[test]
#[ignore = "manual mutating real-ADB test; requires explicit appops opt-in and package allowlist"]
fn manual_real_adb_appops_requires_allowlist() {
    let Some(mut device) = manual_device() else {
        return;
    };
    if !per_test_opt_in("EMUCHEF_RUN_REAL_ADB_APPOPS_TEST") {
        return;
    }
    let Some(package) = optional_env("EMUCHEF_TEST_PACKAGE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_PACKAGE to a test-owned package.");
        return;
    };
    guard_test_package(&package);
    let Some(op) = optional_env("EMUCHEF_TEST_APPOP") else {
        eprintln!("Skipping: set EMUCHEF_TEST_APPOP for the test-owned package.");
        return;
    };
    let Some(mode) = optional_env("EMUCHEF_TEST_APPOP_MODE") else {
        eprintln!("Skipping: set EMUCHEF_TEST_APPOP_MODE for the test-owned package.");
        return;
    };

    device
        .run_plan_command(vec![
            "adb".to_string(),
            "shell".to_string(),
            "appops".to_string(),
            "set".to_string(),
            package,
            op,
            mode,
        ])
        .expect("manual appops command should complete through real ADB");
}

fn manual_device() -> Option<RealAdbDevice> {
    if env::var(GLOBAL_OPT_IN).ok().as_deref() != Some("1") {
        eprintln!("Skipping: set {GLOBAL_OPT_IN}=1 to run manual real-ADB tests.");
        return None;
    }
    Some(RealAdbDevice::new(
        "adb",
        optional_env("EMUCHEF_TEST_DEVICE_SERIAL"),
    ))
}

fn per_test_opt_in(name: &str) -> bool {
    if env::var(name).ok().as_deref() == Some("1") {
        true
    } else {
        eprintln!("Skipping: set {name}=1 to run this device-affecting manual test.");
        false
    }
}

fn guard_test_package(package: &str) {
    if is_system_like_package(package) {
        panic!("Refusing to run a device-affecting manual test against system-like package {package:?}.");
    }
    let allowlist = optional_env("EMUCHEF_TEST_PACKAGE_ALLOWLIST")
        .expect("Set EMUCHEF_TEST_PACKAGE_ALLOWLIST to the exact test-owned package before running this manual test.");
    let allowed = allowlist
        .split(',')
        .map(str::trim)
        .any(|allowed| allowed == package);
    assert!(
        allowed,
        "Package {package:?} is not listed in EMUCHEF_TEST_PACKAGE_ALLOWLIST."
    );
}

fn is_system_like_package(package: &str) -> bool {
    package == "android"
        || package.starts_with("android.")
        || package.starts_with("com.android.")
        || package.starts_with("com.google.android.")
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

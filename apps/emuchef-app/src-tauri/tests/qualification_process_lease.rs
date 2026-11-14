#[path = "../src/qualification_process_lease.rs"]
mod qualification_process_lease;

use std::{fs, path::Path, process::Command};

use qualification_process_lease::{LaunchConfig, QualificationProcessLease};
use serde_json::{json, Value};
use tempfile::tempdir;

const TARGET_ID: &str = "konkr-pocket-advance-retroarch-config-create";
const TOKEN: &str = "launch-token-fixture";
const HOST: &str = "host-fixture";

fn verified_record(launch_pid: u32) -> Value {
    json!({
        "schemaVersion": 1,
        "qualificationTargetId": TARGET_ID,
        "token": TOKEN,
        "status": "verified",
        "launchOwner": { "pid": launch_pid, "host": HOST },
        "appProcess": null,
        "verifiedArtifact": {
            "manifestSha256": format!("sha256:{}", "a".repeat(64)),
            "gitCommit": "b".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "c".repeat(64)),
            "materialInputsDigest": format!("sha256:{}", "d".repeat(64)),
            "backendFeatures": ["real-execution", "android-copy-commit-helper"],
            "arm64HelperSha256": format!("sha256:{}", "e".repeat(64)),
            "sidecarSha256": format!("sha256:{}", "f".repeat(64)),
            "runtimeSidecarSha256": format!("sha256:{}", "f".repeat(64))
        }
    })
}

fn fixture(root: &Path, status: &str, launch_pid: u32) -> (LaunchConfig, Value) {
    let session_path = root.join("retroarch-config-create-qualification-session.json");
    let lock_path = root.join("retroarch-config-create-sidecar-build.lock");
    let mut record = verified_record(launch_pid);
    record["status"] = json!(status);
    fs::write(&session_path, serde_json::to_vec(&record).unwrap()).unwrap();
    (
        LaunchConfig {
            session_path,
            lock_path,
            token: TOKEN.to_owned(),
            target_id: TARGET_ID.to_owned(),
            host: HOST.to_owned(),
            launch_pid,
        },
        record,
    )
}

#[test]
fn native_session_claim_records_the_actual_app_pid_and_clean_exit() {
    let root = tempdir().unwrap();
    let (config, _) = fixture(root.path(), "verified", std::process::id());

    let session = QualificationProcessLease::start(config.clone()).unwrap();
    let active: Value = serde_json::from_slice(&fs::read(&config.session_path).unwrap()).unwrap();
    assert_eq!(active["status"], "active");
    assert_eq!(active["appProcess"]["pid"], std::process::id());
    assert_eq!(active["appProcess"]["host"], HOST);

    session.finish().unwrap();
    let ended: Value = serde_json::from_slice(&fs::read(&config.session_path).unwrap()).unwrap();
    assert_eq!(ended["status"], "ended-cleanly");
    assert_eq!(ended["appProcess"]["pid"], std::process::id());
}

#[cfg(all(unix, feature = "real-execution"))]
#[test]
fn environment_launch_claims_and_finishes_the_verified_lease() {
    const CHILD_MARKER: &str = "EMUCHEF_CONFIG_CREATE_QUALIFICATION_LEASE_CHILD";

    if std::env::var_os(CHILD_MARKER).is_some() {
        let lease = QualificationProcessLease::start_from_environment()
            .unwrap()
            .expect("qualification environment must start a lease");
        lease.finish().unwrap();
        return;
    }

    let root = tempdir().unwrap();
    let (config, _) = fixture(root.path(), "verified", std::process::id());
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "environment_launch_claims_and_finishes_the_verified_lease",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("EMUCHEF_CONFIG_CREATE_QUALIFICATION", "1")
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE",
            &config.session_path,
        )
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_LOCK_FILE",
            &config.lock_path,
        )
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_TOKEN",
            &config.token,
        )
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID",
            &config.target_id,
        )
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_HOST",
            &config.host,
        )
        .env(
            "EMUCHEF_CONFIG_CREATE_QUALIFICATION_LAUNCH_PID",
            std::process::id().to_string(),
        )
        .status()
        .unwrap();

    assert!(child.success());
    let ended: Value = serde_json::from_slice(&fs::read(&config.session_path).unwrap()).unwrap();
    assert_eq!(ended["status"], "ended-cleanly");
}

#[test]
fn native_claim_rejects_wrong_token_target_and_launch_owner_without_rewriting() {
    let root = tempdir().unwrap();
    let (config, _) = fixture(root.path(), "verified", std::process::id());
    let original = fs::read(&config.session_path).unwrap();

    let mut wrong_token = config.clone();
    wrong_token.token = "wrong-token".to_owned();
    assert!(QualificationProcessLease::start(wrong_token).is_err());

    let mut wrong_target = config.clone();
    wrong_target.target_id = "another-target".to_owned();
    assert!(QualificationProcessLease::start(wrong_target).is_err());

    let mut wrong_owner = config.clone();
    wrong_owner.launch_pid += 1;
    assert!(QualificationProcessLease::start(wrong_owner).is_err());

    assert_eq!(fs::read(&config.session_path).unwrap(), original);
}

#[test]
fn native_claim_requires_verified_startup_and_the_shared_lock() {
    let root = tempdir().unwrap();
    let (config, _) = fixture(root.path(), "launching", std::process::id());
    let original = fs::read(&config.session_path).unwrap();
    assert!(QualificationProcessLease::start(config.clone()).is_err());
    assert_eq!(fs::read(&config.session_path).unwrap(), original);

    fs::write(&config.lock_path, b"another process owns the lock").unwrap();
    let (verified_config, _) = fixture(root.path(), "verified", std::process::id());
    let original = fs::read(&verified_config.session_path).unwrap();
    assert!(QualificationProcessLease::start(verified_config.clone()).is_err());
    assert_eq!(fs::read(&verified_config.session_path).unwrap(), original);
    assert!(config.lock_path.exists());
}

#[test]
fn native_claim_rejects_a_verified_lease_after_its_launcher_exits() {
    let root = tempdir().unwrap();
    let mut child = Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap();
    let launch_pid = child.id();
    child.wait().unwrap();
    let (config, _) = fixture(root.path(), "verified", launch_pid);
    let original = fs::read(&config.session_path).unwrap();

    let error = QualificationProcessLease::start(config.clone())
        .err()
        .unwrap();
    assert!(error.contains("launch owner process has ended"));
    assert_eq!(fs::read(&config.session_path).unwrap(), original);
}

#[test]
fn finish_refuses_a_session_record_replaced_by_another_owner() {
    let root = tempdir().unwrap();
    let (config, _) = fixture(root.path(), "verified", std::process::id());
    let session = QualificationProcessLease::start(config.clone()).unwrap();
    let mut replacement = verified_record(std::process::id());
    replacement["token"] = json!("replacement-token");
    replacement["status"] = json!("active");
    replacement["appProcess"] = json!({ "pid": std::process::id(), "host": HOST });
    fs::write(
        &config.session_path,
        serde_json::to_vec(&replacement).unwrap(),
    )
    .unwrap();

    assert!(session.finish().is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&config.session_path).unwrap()).unwrap()["token"],
        "replacement-token"
    );
}

#[cfg(unix)]
#[test]
fn failed_native_lock_write_does_not_publish_a_partial_lock() {
    const CHILD_ROOT: &str = "EMUCHEF_CONFIG_CREATE_QUALIFICATION_LOCK_WRITE_FAILURE_ROOT";

    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = Path::new(&root);
        let (config, _) = fixture(root, "verified", std::process::id());
        unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let limit = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
        }

        assert!(QualificationProcessLease::start(config).is_err());
        return;
    }

    let root = tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("failed_native_lock_write_does_not_publish_a_partial_lock")
        .env(CHILD_ROOT, root.path())
        .status()
        .unwrap();

    assert!(status.success());
    assert!(!root
        .path()
        .join("retroarch-config-create-sidecar-build.lock")
        .exists());
}

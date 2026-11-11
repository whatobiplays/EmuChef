#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const FROZEN_BYTES: &[u8] = b"video_threaded=false\ncustom=kept\n";

struct HelperHarness {
    _directory: TempDir,
    executable: PathBuf,
    selected_root: PathBuf,
    stage_root: PathBuf,
}

impl HelperHarness {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("helper test directory should be created");
        let executable = directory.path().join("copy-commit-helper");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/executor/copy_commit_helper.c");
        let status = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg("-DEMUCHEF_COPY_COMMIT_HELPER_TEST")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .status()
            .expect("host C compiler should be available for helper contract tests");
        assert!(status.success(), "the selected-root helper should compile");

        let selected_root = directory.path().join("selected-root");
        let stage_root = directory.path().join("stage");
        fs::create_dir(&selected_root).expect("selected root should be created");
        fs::create_dir(&stage_root).expect("staging directory should be created");
        fs::write(stage_root.join("retroarch.cfg"), FROZEN_BYTES)
            .expect("staged configuration should be written");

        Self {
            _directory: directory,
            executable,
            selected_root: fs::canonicalize(selected_root).unwrap(),
            stage_root: fs::canonicalize(stage_root).unwrap(),
        }
    }

    fn digest(&self) -> String {
        crate::runtime_configuration::sha256_hex(FROZEN_BYTES)
    }

    fn invoke(
        &self,
        race: Option<&str>,
        partial_write: bool,
        digest: Option<&str>,
        identity: Option<&str>,
    ) -> Output {
        let metadata = fs::metadata(&self.selected_root).unwrap();
        let expected_identity = format!("{}:{}", metadata.dev(), metadata.ino());
        let expected_digest = digest.map(str::to_string).unwrap_or_else(|| self.digest());
        let mut command = Command::new(&self.executable);
        command
            .arg(&self.selected_root)
            .arg(&self.selected_root)
            .arg(identity.unwrap_or(&expected_identity))
            .arg(&self.stage_root)
            .arg("create-file")
            .arg("retroarch.cfg")
            .arg("absent")
            .arg(expected_digest);
        if let Some(race) = race {
            command.env("EMU_CHEF_TEST_FINAL_OPEN_RACE", race);
        }
        if partial_write {
            command.env("EMU_CHEF_TEST_PARTIAL_FINAL_WRITE", "1");
        }
        command.output().expect("helper process should complete")
    }

    fn invoke_final_readback_fifo_race(&self) -> Output {
        let metadata = fs::metadata(&self.selected_root).unwrap();
        let expected_identity = format!("{}:{}", metadata.dev(), metadata.ino());
        Command::new(&self.executable)
            .arg(&self.selected_root)
            .arg(&self.selected_root)
            .arg(expected_identity)
            .arg(&self.stage_root)
            .arg("create-file")
            .arg("retroarch.cfg")
            .arg("absent")
            .arg(self.digest())
            .env("EMU_CHEF_TEST_FINAL_READBACK_RACE", "fifo")
            .output()
            .expect("helper process should complete")
    }

    fn target(&self) -> PathBuf {
        self.selected_root.join("retroarch.cfg")
    }
}

#[test]
fn selected_root_helper_creates_and_reads_back_exact_frozen_bytes() {
    let harness = HelperHarness::new();
    let output = harness.invoke(None, false, None, None);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(harness.target()).unwrap(), FROZEN_BYTES);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(&harness.digest()));
    assert!(stdout.contains(&FROZEN_BYTES.len().to_string()));
}

#[test]
fn selected_root_helper_preserves_initial_and_racing_targets() {
    let initially_present = HelperHarness::new();
    let existing_bytes = b"owner configuration";
    fs::write(initially_present.target(), existing_bytes).unwrap();
    let output = initially_present.invoke(None, false, None, None);
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(
        fs::read(initially_present.target()).unwrap(),
        existing_bytes
    );

    let appears_during_final_open = HelperHarness::new();
    let output = appears_during_final_open.invoke(Some("file"), false, None, None);
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(
        fs::read(appears_during_final_open.target()).unwrap(),
        b"concurrent target"
    );

    let symlink_appears_during_final_open = HelperHarness::new();
    let output = symlink_appears_during_final_open.invoke(Some("symlink"), false, None, None);
    assert_eq!(output.status.code(), Some(10));
    assert!(
        fs::symlink_metadata(symlink_appears_during_final_open.target())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn selected_root_helper_rejects_stale_identity_and_staged_hash() {
    let stale_identity = HelperHarness::new();
    let output = stale_identity.invoke(None, false, None, Some("1:1"));
    assert!(!output.status.success());
    assert!(!stale_identity.target().exists());

    let wrong_digest = HelperHarness::new();
    let output = wrong_digest.invoke(None, false, Some(&"0".repeat(64)), None);
    assert_eq!(output.status.code(), Some(11));
    assert!(!wrong_digest.target().exists());
}

#[test]
fn selected_root_helper_reports_partial_bytes_without_removing_them() {
    let harness = HelperHarness::new();
    let output = harness.invoke(None, true, None, None);

    assert_eq!(output.status.code(), Some(12));
    assert_eq!(fs::read(harness.target()).unwrap(), b"partial");
}

#[test]
fn selected_root_helper_rejects_fifo_replacement_during_final_readback() {
    let harness = HelperHarness::new();
    let output = harness.invoke_final_readback_fifo_race();

    assert_eq!(output.status.code(), Some(12));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("validate final configuration length"),
        "unexpected helper failure: {stderr}"
    );
    assert!(fs::symlink_metadata(harness.target())
        .unwrap()
        .file_type()
        .is_fifo());
}

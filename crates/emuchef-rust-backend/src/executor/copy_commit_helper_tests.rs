#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
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

    fn read_configuration(&self) -> Output {
        let metadata = fs::metadata(&self.selected_root).unwrap();
        let expected_identity = format!("{}:{}", metadata.dev(), metadata.ino());
        Command::new(&self.executable)
            .arg(&self.selected_root)
            .arg(&self.selected_root)
            .arg(expected_identity)
            .arg(&self.stage_root)
            .arg("read-file")
            .arg("retroarch.cfg")
            .arg("regular")
            .output()
            .unwrap()
    }

    fn invoke_replace(&self, operation_id: &str, original: &[u8], replacement: &[u8]) -> Output {
        fs::write(self.selected_root.join("retroarch.cfg"), original).unwrap();
        fs::write(self.stage_root.join("retroarch.cfg"), replacement).unwrap();
        let metadata = fs::metadata(&self.selected_root).unwrap();
        let expected_identity = format!("{}:{}", metadata.dev(), metadata.ino());
        let mut child = Command::new(&self.executable)
            .arg(&self.selected_root)
            .arg(&self.selected_root)
            .arg(expected_identity)
            .arg(&self.stage_root)
            .arg("replace-file")
            .arg("retroarch.cfg")
            .arg(crate::runtime_configuration::sha256_hex(original))
            .arg(original.len().to_string())
            .arg(crate::runtime_configuration::sha256_hex(replacement))
            .arg(operation_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let mut stdout_reader = BufReader::new(child.stdout.take().unwrap());
        let mut ready_frame = String::new();
        stdout_reader.read_line(&mut ready_frame).unwrap();
        if ready_frame == format!("READY {operation_id}\n") {
            let mut stdin = child.stdin.take().unwrap();
            write!(stdin, "COMMIT {operation_id}\n").unwrap();
            stdin.flush().unwrap();
            drop(stdin);
        }

        let mut stdout = ready_frame.into_bytes();
        stdout_reader.read_to_end(&mut stdout).unwrap();
        let status = child.wait().unwrap();
        let mut stderr = Vec::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        Output {
            status,
            stdout,
            stderr,
        }
    }

    fn invoke(
        &self,
        race: Option<&str>,
        partial_write: bool,
        digest: Option<&str>,
        identity: Option<&str>,
    ) -> Output {
        self.invoke_create(
            "create-test-operation",
            "create-test-operation",
            race,
            partial_write,
            digest,
            identity,
            None,
        )
        .0
    }

    fn invoke_create(
        &self,
        operation_id: &str,
        commit_id: &str,
        race: Option<&str>,
        partial_write: bool,
        digest: Option<&str>,
        identity: Option<&str>,
        final_readback_race: Option<&str>,
    ) -> (Output, bool) {
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
            .arg(expected_digest)
            .arg(operation_id);
        if let Some(race) = race {
            command.env("EMU_CHEF_TEST_FINAL_OPEN_RACE", race);
        }
        if partial_write {
            command.env("EMU_CHEF_TEST_PARTIAL_FINAL_WRITE", "1");
        }
        if let Some(race) = final_readback_race {
            command.env("EMU_CHEF_TEST_FINAL_READBACK_RACE", race);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("helper process should start");
        let mut stdout_reader = BufReader::new(child.stdout.take().unwrap());
        let mut ready_frame = String::new();
        stdout_reader.read_line(&mut ready_frame).unwrap();
        let target_exists_at_ready = self.target().exists();
        if ready_frame == format!("READY {operation_id}\n") {
            let mut stdin = child.stdin.take().unwrap();
            write!(stdin, "COMMIT {commit_id}\n").unwrap();
            stdin.flush().unwrap();
        } else {
            drop(child.stdin.take());
        }
        let mut stdout = ready_frame.into_bytes();
        stdout_reader.read_to_end(&mut stdout).unwrap();
        let status = child.wait().unwrap();
        let mut stderr = Vec::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        (
            Output {
                status,
                stdout,
                stderr,
            },
            target_exists_at_ready,
        )
    }

    fn invoke_final_readback_fifo_race(&self) -> Output {
        self.invoke_create(
            "create-readback-race",
            "create-readback-race",
            None,
            false,
            None,
            None,
            Some("fifo"),
        )
        .0
    }

    fn target(&self) -> PathBuf {
        self.selected_root.join("retroarch.cfg")
    }

    fn operation_status(&self, operation_id: &str) -> Output {
        let metadata = fs::metadata(&self.selected_root).unwrap();
        let expected_identity = format!("{}:{}", metadata.dev(), metadata.ino());
        Command::new(&self.executable)
            .arg(&self.selected_root)
            .arg(&self.selected_root)
            .arg(expected_identity)
            .arg(&self.stage_root)
            .arg("operation-status")
            .arg(operation_id)
            .arg("v1")
            .output()
            .expect("operation status query should complete")
    }
}

#[test]
fn selected_root_helper_reads_only_the_existing_regular_retroarch_config() {
    let harness = HelperHarness::new();
    let original = b"video_threaded=true\ncustom=original\n";
    fs::write(harness.selected_root.join("retroarch.cfg"), original).unwrap();

    let output = harness.read_configuration();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (header, encoded) = stdout.split_once('\n').unwrap();
    let mut fields = header.split_whitespace();
    assert_eq!(fields.next(), Some("READ"));
    assert_eq!(
        fields.next().and_then(|value| value.parse::<usize>().ok()),
        Some(original.len())
    );
    assert_eq!(
        fields.next(),
        Some(crate::runtime_configuration::sha256_hex(original).as_str())
    );
    assert_eq!(fields.next(), None);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .unwrap(),
        original
    );
}

#[test]
fn selected_root_helper_replaces_the_existing_config_after_operation_bound_commit() {
    let harness = HelperHarness::new();
    let original = b"video_threaded=true\ncustom=original\n";
    let replacement = b"video_threaded=false\ncustom=replaced\n";
    let destination = harness.selected_root.join("retroarch.cfg");
    fs::write(&destination, original).unwrap();
    let original_inode = fs::metadata(&destination).unwrap().ino();

    let output = harness.invoke_replace("replace-operation", original, replacement);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&destination).unwrap(), replacement);
    assert_eq!(fs::metadata(&destination).unwrap().ino(), original_inode);
    assert!(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.starts_with("REPLACED ")));
}

#[test]
fn selected_root_helper_rejects_a_symlinked_retroarch_config_read() {
    let harness = HelperHarness::new();
    let outside = harness._directory.path().join("outside-config");
    fs::write(&outside, b"private data").unwrap();
    std::os::unix::fs::symlink(&outside, harness.selected_root.join("retroarch.cfg")).unwrap();

    let output = harness.read_configuration();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
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
fn selected_root_helper_waits_for_matching_create_commit_before_mutation() {
    let harness = HelperHarness::new();
    let (output, target_exists_at_ready) = harness.invoke_create(
        "create-operation",
        "create-operation",
        None,
        false,
        None,
        None,
        None,
    );

    assert!(!target_exists_at_ready, "target changed before COMMIT");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("READY create-operation\n"));
    assert!(stdout.contains("CREATED "));
    let status = harness.operation_status("create-operation");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        format!(
            "STATUS 1 create-operation create complete retroarch.cfg {} {} absent 0 {} {} complete complete\n",
            harness.digest(),
            FROZEN_BYTES.len(),
            harness.digest(),
            FROZEN_BYTES.len(),
        )
    );
}

#[test]
fn selected_root_helper_rejects_a_mismatched_create_commit() {
    let harness = HelperHarness::new();
    let (output, target_exists_at_ready) = harness.invoke_create(
        "create-operation",
        "another-operation",
        None,
        false,
        None,
        None,
        None,
    );

    assert!(!target_exists_at_ready, "target changed before COMMIT");
    assert!(!output.status.success());
    assert!(!harness.target().exists());
    let status = harness.operation_status("create-operation");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(String::from_utf8(status.stdout)
        .unwrap()
        .contains(" create aborted retroarch.cfg "));
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

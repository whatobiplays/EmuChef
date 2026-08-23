use std::path::{Path, PathBuf};
use std::process::Command;

const QUALIFICATION_TOOL: &str = "../../../tools/device-qualification.mjs";

// These paths mirror the canonical tool's material inputs. The build script
// only uses them to invalidate Cargo's build-script cache; the Node tool still
// computes the build digest and identity.
const MATERIAL_ROOTS: &[&str] = &[
    "../../../authored",
    "../../../crates/emuchef-rust-backend",
    "../src",
    "src",
];
const MATERIAL_FILES: &[&str] = &[
    "../package.json",
    "../package-lock.json",
    "Cargo.toml",
    "Cargo.lock",
    "tauri.conf.json",
];

// A recordable build must be reevaluated when Git identity or tracked-file
// state changes. Watching every tracked path catches unstaged edits, while
// these Git paths catch reference, index, and worktree state changes.
const GIT_STATE_PATHS: &[&str] = &["HEAD", "index", "packed-refs", "refs", "logs/HEAD"];

fn main() {
    println!("cargo:rerun-if-env-changed=EMUCHEF_DEVICE_QUALIFICATION");
    if qualification_requested() {
        let manifest_dir = PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"),
        );
        configure_qualification_reruns(&manifest_dir);
        embed_build_identity(&manifest_dir);
    }
    tauri_build::build();
}

fn qualification_requested() -> bool {
    std::env::var("EMUCHEF_DEVICE_QUALIFICATION")
        .ok()
        .as_deref()
        == Some("1")
}

fn configure_qualification_reruns(manifest_dir: &Path) {
    let repo_root = manifest_dir.join("../../..");
    emit_rerun_if_changed(&qualification_tool(manifest_dir));

    for relative_path in MATERIAL_ROOTS.iter().chain(MATERIAL_FILES) {
        emit_rerun_if_changed(&manifest_dir.join(relative_path));
    }

    for tracked_path in tracked_paths(&repo_root) {
        emit_rerun_if_changed(&repo_root.join(tracked_path));
    }

    for git_state_path in GIT_STATE_PATHS {
        emit_rerun_if_changed(&git_path(&repo_root, git_state_path));
    }
}

fn embed_build_identity(manifest_dir: &Path) {
    let tool = qualification_tool(manifest_dir);
    let output = Command::new("node")
        .arg(tool)
        .arg("--build-identity")
        .arg("--require-clean")
        .output()
        .expect("device qualification build identity command must start");
    assert!(
        output.status.success(),
        "device qualification requires a clean committed source state: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("device qualification build identity must be valid JSON");
    let encoded = serde_json::to_string(&value).expect("build identity must serialize");
    println!("cargo:rustc-env=EMUCHEF_QUALIFICATION_BUILD_IDENTITY={encoded}");
}

fn qualification_tool(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .join(QUALIFICATION_TOOL)
        .canonicalize()
        .expect("device qualification Node tool must exist")
}

fn emit_rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

fn tracked_paths(repo_root: &Path) -> impl Iterator<Item = PathBuf> {
    git_output(repo_root, &["ls-files", "-z"])
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>()
        .into_iter()
}

fn git_path(repo_root: &Path, path: &str) -> PathBuf {
    let resolved = PathBuf::from(git_output(repo_root, &["rev-parse", "--git-path", path]).trim());
    if resolved.is_absolute() {
        resolved
    } else {
        repo_root.join(resolved)
    }
}

fn git_output(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("Git must be available for qualification build invalidation");
    assert!(
        output.status.success(),
        "Git command `{}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout).expect("Git output must be UTF-8")
}

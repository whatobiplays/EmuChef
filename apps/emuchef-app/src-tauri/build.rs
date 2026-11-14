use std::path::{Path, PathBuf};
use std::process::Command;

const QUALIFICATION_TOOL: &str = "../../../tools/device-qualification.mjs";
const QUALIFICATION_BUILD_IDENTITY_ENV: &str = "EMUCHEF_QUALIFICATION_BUILD_IDENTITY";

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
const CONFIG_CREATE_MATERIAL_ROOTS: &[&str] = &[
    "authored/",
    "apps/emuchef-app/",
    "crates/emuchef-rust-backend/",
    "scripts/sidecar-target-contract.mjs",
    "tools/device-qualification.mjs",
    "docs/qualification/retroarch-config-create/target.json",
];
const CONFIG_CREATE_GENERATED_PATHS: &[&str] = &[
    "apps/emuchef-app/node_modules/",
    "apps/emuchef-app/dist/",
    "apps/emuchef-app/src-tauri/target/",
    "apps/emuchef-app/src-tauri/binaries/",
    "crates/emuchef-rust-backend/target/",
];

fn main() {
    println!("cargo:rerun-if-env-changed=EMUCHEF_DEVICE_QUALIFICATION");
    println!("cargo:rerun-if-env-changed=EMUCHEF_CONFIG_CREATE_QUALIFICATION");
    println!("cargo:rerun-if-env-changed=EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    if config_create_qualification_requested() {
        configure_qualified_sidecar_reruns(&manifest_dir);
        verify_qualified_sidecar(&manifest_dir);
    }
    if qualification_requested() {
        configure_qualification_reruns(&manifest_dir);
        embed_build_identity(&manifest_dir);
    } else {
        clear_embedded_build_identity();
    }
    tauri_build::build();
}

fn qualification_requested() -> bool {
    std::env::var("EMUCHEF_DEVICE_QUALIFICATION")
        .ok()
        .as_deref()
        == Some("1")
}

fn config_create_qualification_requested() -> bool {
    std::env::var("EMUCHEF_CONFIG_CREATE_QUALIFICATION")
        .ok()
        .as_deref()
        == Some("1")
}

fn configure_qualified_sidecar_reruns(manifest_dir: &Path) {
    let repo_root = manifest_dir.join("../../..");
    emit_rerun_if_changed(&manifest_dir.join("../scripts/verify-qualification-sidecar.mjs"));
    emit_rerun_if_changed(&manifest_dir.join("binaries"));
    emit_rerun_if_changed(
        &manifest_dir.join(
            "../../../crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
        ),
    );
    for material_path in retroarch_config_create_material_paths(&repo_root) {
        emit_rerun_if_changed(&repo_root.join(material_path));
    }
    for git_state_path in GIT_STATE_PATHS {
        emit_rerun_if_changed(&git_path(&repo_root, git_state_path));
    }
}

fn retroarch_config_create_material_paths(repo_root: &Path) -> Vec<PathBuf> {
    git_output(
        repo_root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
    .split('\0')
    .filter(|path| !path.is_empty())
    .filter(|path| {
        CONFIG_CREATE_MATERIAL_ROOTS
            .iter()
            .any(|root| path.starts_with(root))
    })
    .filter(|path| {
        !CONFIG_CREATE_GENERATED_PATHS
            .iter()
            .any(|root| path.starts_with(root))
    })
    .map(PathBuf::from)
    .collect()
}

fn verify_qualified_sidecar(manifest_dir: &Path) {
    let script = manifest_dir.join("../scripts/verify-qualification-sidecar.mjs");
    let output = Command::new("node")
        .arg(script)
        .output()
        .expect("configuration-create sidecar verification command must start");
    assert!(
        output.status.success(),
        "configuration-create qualification requires the verified qualification sidecar: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
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
    println!("cargo:rustc-env={QUALIFICATION_BUILD_IDENTITY_ENV}={encoded}");
}

/// Masks any inherited identity so ordinary builds cannot consume qualification metadata.
fn clear_embedded_build_identity() {
    println!("cargo:rustc-env={QUALIFICATION_BUILD_IDENTITY_ENV}=");
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

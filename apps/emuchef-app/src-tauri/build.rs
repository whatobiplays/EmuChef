fn main() {
    println!("cargo:rerun-if-env-changed=EMUCHEF_DEVICE_QUALIFICATION");
    if std::env::var("EMUCHEF_DEVICE_QUALIFICATION")
        .ok()
        .as_deref()
        == Some("1")
    {
        let manifest_dir = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
        );
        let tool = manifest_dir.join("../../../tools/device-qualification.mjs");
        let output = std::process::Command::new("node")
            .arg(tool)
            .arg("--build-identity")
            .arg("--require-clean")
            .output()
            .expect("device qualification build identity command must start");
        assert!(
            output.status.success(),
            "device qualification requires a clean committed source state"
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("device qualification build identity must be valid JSON");
        let encoded = serde_json::to_string(&value).expect("build identity must serialize");
        println!("cargo:rustc-env=EMUCHEF_QUALIFICATION_BUILD_IDENTITY={encoded}");
    }
    tauri_build::build();
}

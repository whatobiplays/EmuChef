//! Read-only build identity emitted by the qualification-only sidecar.

#[cfg(all(feature = "real-execution", feature = "android-copy-commit-helper"))]
use serde_json::json;
use serde_json::Value;

#[cfg(any(
    test,
    all(feature = "real-execution", feature = "android-copy-commit-helper")
))]
fn source_identity<'a>(
    git_commit: Option<&'a str>,
    material_build_digest: Option<&'a str>,
    material_inputs_digest: Option<&'a str>,
) -> Option<(&'a str, &'a str, &'a str)> {
    Some((git_commit?, material_build_digest?, material_inputs_digest?))
}

/// Reports the compiled backend features and the helper bytes linked into this binary.
pub fn sidecar_provenance() -> Option<Value> {
    #[cfg(all(feature = "real-execution", feature = "android-copy-commit-helper"))]
    {
        let (git_commit, material_build_digest, material_inputs_digest) = source_identity(
            option_env!("EMU_CHEF_CONFIG_CREATE_QUALIFICATION_BUILD_GIT_COMMIT"),
            option_env!("EMU_CHEF_CONFIG_CREATE_QUALIFICATION_MATERIAL_DIGEST"),
            option_env!("EMU_CHEF_CONFIG_CREATE_QUALIFICATION_MATERIAL_INPUTS_DIGEST"),
        )?;
        let helper = include_bytes!(env!("EMU_CHEF_COPY_COMMIT_HELPER_AARCH64"));
        let digest = format!(
            "sha256:{}",
            crate::runtime_configuration::sha256_hex(helper)
        );
        Some(json!({
            "schemaVersion": 1,
            "backendFeatures": ["real-execution", "android-copy-commit-helper"],
            "arm64HelperSha256": digest,
            "gitCommit": git_commit,
            "materialBuildDigest": material_build_digest,
            "materialInputsDigest": material_inputs_digest,
        }))
    }

    #[cfg(not(all(feature = "real-execution", feature = "android-copy-commit-helper")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::source_identity;

    #[test]
    fn source_identity_requires_the_complete_compile_time_binding() {
        assert_eq!(
            source_identity(Some("commit"), Some("build"), Some("inputs")),
            Some(("commit", "build", "inputs")),
        );
        assert_eq!(source_identity(Some("a"), None, Some("sha256:cccc")), None,);
    }
}

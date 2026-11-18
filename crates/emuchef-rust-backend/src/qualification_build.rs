//! Read-only build identity emitted by the qualification-only sidecar.

use serde::{Deserialize, Serialize};
#[cfg(all(feature = "real-execution", feature = "android-copy-commit-helper"))]
use serde_json::json;
use serde_json::Value;

const RESTRICT_PRIVILEGED_EXECUTION_ARGUMENT: &str = "--restrict-privileged-execution";
const QUALIFICATION_SOURCE_IDENTITY_ARGUMENT: &str = "--qualification-source-identity";
const RUNTIME_GENERATION_ARGUMENT: &str = "--runtime-generation";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationSourceIdentity {
    app_version: String,
    git_commit: String,
    material_build_digest: String,
    real_execution_enabled: bool,
    qualification_contract: u32,
}

/// Validates the private sidecar launch arguments for restricted execution.
///
/// The requested identity must match the identity embedded by a clean,
/// real-execution debug build. The returned JSON is canonicalized from that
/// embedded identity for the sidecar's startup handshake.
pub fn restricted_execution_policy(
    arguments: &[String],
    debug_build: bool,
    real_execution_enabled: bool,
    runtime_opt_in: bool,
    embedded_identity: Option<&str>,
) -> Result<Option<String>, String> {
    let restriction_count = arguments
        .iter()
        .filter(|argument| argument.as_str() == RESTRICT_PRIVILEGED_EXECUTION_ARGUMENT)
        .count();
    let identity_positions = arguments
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            (argument == QUALIFICATION_SOURCE_IDENTITY_ARGUMENT).then_some(index)
        })
        .collect::<Vec<_>>();

    if restriction_count == 0 {
        return if identity_positions.is_empty() {
            Ok(None)
        } else {
            Err("qualification identity requires restricted execution".to_string())
        };
    }
    if restriction_count != 1 || identity_positions.len() != 1 {
        return Err("restricted execution startup arguments are invalid".to_string());
    }
    if !(debug_build && real_execution_enabled && runtime_opt_in) {
        return Err("restricted execution requires an enabled qualification build".to_string());
    }

    let requested_identity = arguments
        .get(identity_positions[0] + 1)
        .ok_or_else(|| "restricted execution build identity is missing".to_string())?;
    let embedded_identity = embedded_identity
        .ok_or_else(|| "restricted execution build identity is unavailable".to_string())?;
    let requested_identity = parse_qualification_source_identity(requested_identity)?;
    let embedded_identity = parse_qualification_source_identity(embedded_identity)?;
    if !valid_qualification_source_identity(&embedded_identity)
        || requested_identity != embedded_identity
    {
        return Err("restricted execution build identity does not match".to_string());
    }

    serde_json::to_string(&embedded_identity)
        .map(Some)
        .map_err(|_| "restricted execution build identity is invalid".to_string())
}

/// Returns the identity embedded for the restricted sidecar build, if present.
pub fn embedded_restricted_execution_build_identity() -> Option<&'static str> {
    option_env!("EMU_CHEF_RESTRICTED_EXECUTION_BUILD_IDENTITY")
        .filter(|identity| !identity.is_empty())
}

/// Reads the positive runtime generation trusted from the native process launcher.
pub fn restricted_execution_runtime_generation(
    arguments: &[String],
    restriction_enabled: bool,
) -> Result<Option<u64>, String> {
    let positions = arguments
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == RUNTIME_GENERATION_ARGUMENT).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() > 1 || (restriction_enabled && positions.len() != 1) {
        return Err("restricted execution runtime generation is invalid".to_string());
    }
    let Some(position) = positions.first().copied() else {
        return Ok(None);
    };
    let generation = arguments
        .get(position + 1)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|generation| *generation > 0)
        .ok_or_else(|| "restricted execution runtime generation is invalid".to_string())?;
    Ok(Some(generation))
}

fn parse_qualification_source_identity(value: &str) -> Result<QualificationSourceIdentity, String> {
    serde_json::from_str(value)
        .map_err(|_| "restricted execution build identity is invalid".to_string())
}

fn valid_qualification_source_identity(identity: &QualificationSourceIdentity) -> bool {
    identity.real_execution_enabled
        && !identity.app_version.is_empty()
        && identity.qualification_contract > 0
        && is_lowercase_hex(&identity.git_commit, 40)
        && identity
            .material_build_digest
            .strip_prefix("sha256:")
            .is_some_and(|digest| is_lowercase_hex(digest, 64))
}

fn is_lowercase_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

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
    use super::{restricted_execution_policy, source_identity};
    use serde_json::json;

    #[test]
    fn source_identity_requires_the_complete_compile_time_binding() {
        assert_eq!(
            source_identity(Some("commit"), Some("build"), Some("inputs")),
            Some(("commit", "build", "inputs")),
        );
        assert_eq!(source_identity(Some("a"), None, Some("sha256:cccc")), None,);
    }

    #[test]
    fn restricted_execution_startup_requires_all_gates_and_matching_identity() {
        let identity = json!({
            "appVersion": "0.1.0",
            "gitCommit": "1".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1,
        });
        let encoded = serde_json::to_string(&identity).unwrap();
        let arguments = vec![
            "--restrict-privileged-execution".to_string(),
            "--qualification-source-identity".to_string(),
            encoded.clone(),
        ];

        assert_eq!(
            restricted_execution_policy(&arguments, true, true, true, Some(&encoded)).unwrap(),
            Some(encoded.clone())
        );
        for (debug_build, real_execution, runtime_opt_in) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(restricted_execution_policy(
                &arguments,
                debug_build,
                real_execution,
                runtime_opt_in,
                Some(&encoded),
            )
            .is_err());
        }
    }

    #[test]
    fn restricted_execution_startup_rejects_missing_or_mismatched_identity() {
        let identity = json!({
            "appVersion": "0.1.0",
            "gitCommit": "1".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1,
        });
        let encoded = serde_json::to_string(&identity).unwrap();
        let arguments = vec![
            "--restrict-privileged-execution".to_string(),
            "--qualification-source-identity".to_string(),
            encoded.clone(),
        ];
        let no_restriction = vec!["--cache-root".to_string(), "/tmp/cache".to_string()];

        assert_eq!(
            restricted_execution_policy(&no_restriction, true, true, true, None).unwrap(),
            None
        );
        assert!(restricted_execution_policy(&arguments, true, true, true, None).is_err());
        assert!(restricted_execution_policy(
            &arguments,
            true,
            true,
            true,
            Some(r#"{"unexpected":true}"#),
        )
        .is_err());
    }

    #[test]
    fn restricted_execution_generation_requires_one_positive_native_value() {
        use super::restricted_execution_runtime_generation;

        assert!(restricted_execution_runtime_generation(&[], true).is_err());
        assert!(restricted_execution_runtime_generation(
            &["--runtime-generation".to_string(), "0".to_string()],
            true,
        )
        .is_err());
        assert!(restricted_execution_runtime_generation(
            &[
                "--runtime-generation".to_string(),
                "7".to_string(),
                "--runtime-generation".to_string(),
                "8".to_string(),
            ],
            true,
        )
        .is_err());
        assert_eq!(
            restricted_execution_runtime_generation(
                &["--runtime-generation".to_string(), "7".to_string()],
                true,
            )
            .unwrap(),
            Some(7)
        );
    }
}

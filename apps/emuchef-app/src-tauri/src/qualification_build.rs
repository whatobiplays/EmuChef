//! Trusted build metadata and the runtime gates for recordable qualification.
//!
//! The build script obtains the embedded identity from the repository-owned
//! Node qualification tool. This module only deserializes that identity and
//! evaluates whether the application was compiled and explicitly launched in
//! a configuration that may expose qualification behavior.

use serde::{Deserialize, Serialize};

/// Immutable build metadata embedded by `build.rs` for qualification builds.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationBuildIdentity {
    pub(crate) app_version: String,
    pub(crate) git_commit: String,
    pub(crate) material_build_digest: String,
    pub(crate) real_execution_enabled: bool,
    pub(crate) qualification_contract: u32,
}

/// Inputs to the four independent qualification-mode enablement gates.
#[derive(Clone, Debug)]
pub(crate) struct QualificationGateInputs {
    pub(crate) debug_build: bool,
    pub(crate) real_execution_enabled: bool,
    pub(crate) runtime_opt_in: bool,
    pub(crate) embedded_identity: Option<QualificationBuildIdentity>,
}

/// Returns whether every qualification-mode gate is satisfied.
pub(crate) fn qualification_mode_enabled(inputs: &QualificationGateInputs) -> bool {
    inputs.debug_build
        && inputs.real_execution_enabled
        && inputs.runtime_opt_in
        && inputs
            .embedded_identity
            .as_ref()
            .is_some_and(|identity| identity.real_execution_enabled)
}

/// Reads the trusted build identity embedded by the qualification build script.
pub(crate) fn embedded_build_identity() -> Option<QualificationBuildIdentity> {
    embedded_build_identity_from(option_env!("EMUCHEF_QUALIFICATION_BUILD_IDENTITY"))
}

/// Collects the compile-time and runtime inputs used by the mode gate.
pub(crate) fn qualification_gate_inputs() -> QualificationGateInputs {
    QualificationGateInputs {
        debug_build: cfg!(debug_assertions),
        real_execution_enabled: cfg!(feature = "real-execution"),
        runtime_opt_in: std::env::var("EMUCHEF_DEVICE_QUALIFICATION")
            .ok()
            .as_deref()
            == Some("1"),
        embedded_identity: embedded_build_identity(),
    }
}

/// Evaluates qualification mode using the current application environment.
pub(crate) fn qualification_mode_enabled_at_runtime() -> bool {
    qualification_mode_enabled(&qualification_gate_inputs())
}

/// Returns the clean qualification identity for an explicitly restricted app launch.
pub(crate) fn restricted_execution_identity(
    arguments: &[String],
    qualification_enabled: bool,
    embedded_identity: Option<QualificationBuildIdentity>,
) -> Result<Option<String>, &'static str> {
    let requested = arguments
        .iter()
        .filter(|argument| argument.as_str() == "--restrict-privileged-execution")
        .count();
    if requested == 0 {
        return Ok(None);
    }
    if requested != 1 || !qualification_enabled {
        return Err("restricted execution requires the enabled qualification build");
    }
    let identity = embedded_identity
        .filter(valid_restricted_execution_identity)
        .ok_or("restricted execution build identity is unavailable")?;
    serde_json::to_string(&identity)
        .map(Some)
        .map_err(|_| "restricted execution build identity is invalid")
}

fn valid_restricted_execution_identity(identity: &QualificationBuildIdentity) -> bool {
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

fn embedded_build_identity_from(value: Option<&str>) -> Option<QualificationBuildIdentity> {
    value.and_then(parse_embedded_build_identity)
}

fn parse_embedded_build_identity(value: &str) -> Option<QualificationBuildIdentity> {
    serde_json::from_str(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualification_mode_requires_every_gate() {
        let valid = QualificationGateInputs {
            debug_build: true,
            real_execution_enabled: true,
            runtime_opt_in: true,
            embedded_identity: Some(test_identity()),
        };
        assert!(qualification_mode_enabled(&valid));

        for invalid in [
            QualificationGateInputs {
                debug_build: false,
                ..valid.clone()
            },
            QualificationGateInputs {
                real_execution_enabled: false,
                ..valid.clone()
            },
            QualificationGateInputs {
                runtime_opt_in: false,
                ..valid.clone()
            },
            QualificationGateInputs {
                embedded_identity: None,
                ..valid.clone()
            },
        ] {
            assert!(!qualification_mode_enabled(&invalid));
        }

        let identity_without_real_execution = QualificationGateInputs {
            embedded_identity: Some(QualificationBuildIdentity {
                real_execution_enabled: false,
                ..test_identity()
            }),
            ..valid
        };
        assert!(!qualification_mode_enabled(
            &identity_without_real_execution
        ));
    }

    #[test]
    fn build_identity_uses_strict_camel_case_json() {
        let identity = test_identity();
        let encoded = serde_json::to_string(&identity).expect("test identity should serialize");
        assert!(encoded.contains("\"appVersion\":"));
        assert!(encoded.contains("\"gitCommit\":"));

        let decoded: QualificationBuildIdentity =
            serde_json::from_str(&encoded).expect("serialized identity should deserialize");
        assert_eq!(decoded, identity);

        let with_unknown_field = format!(
            "{}\n",
            encoded.trim_end_matches('}').to_owned() + ",\"unexpected\":true}"
        );
        assert!(serde_json::from_str::<QualificationBuildIdentity>(&with_unknown_field).is_err());
    }

    #[test]
    fn malformed_embedded_identity_is_not_usable() {
        assert!(embedded_build_identity_from(Some("not-json")).is_none());
        assert!(embedded_build_identity_from(Some(
            r#"{"appVersion":"0.1.0","gitCommit":"1111111111111111111111111111111111111111","materialBuildDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","realExecutionEnabled":true,"qualificationContract":1,"unexpected":true}"#
        ))
        .is_none());
    }

    #[test]
    fn embedded_build_identity_is_absent_for_an_ordinary_build() {
        assert!(embedded_build_identity().is_none());
    }

    #[test]
    fn qualification_gate_inputs_capture_default_runtime_state() {
        let inputs = qualification_gate_inputs();

        assert_eq!(inputs.debug_build, cfg!(debug_assertions));
        assert_eq!(
            inputs.real_execution_enabled,
            cfg!(feature = "real-execution")
        );
        assert!(!inputs.runtime_opt_in);
        assert_eq!(inputs.embedded_identity, embedded_build_identity());
        assert!(!qualification_mode_enabled_at_runtime());
    }

    #[test]
    fn qualification_gate_inputs_remain_disabled_without_all_runtime_inputs() {
        let inputs = qualification_gate_inputs();
        assert!(!qualification_mode_enabled(&inputs));
    }

    #[test]
    fn restricted_execution_requires_explicit_flag_all_gates_and_valid_identity() {
        let flag = "--restrict-privileged-execution".to_string();
        assert_eq!(
            restricted_execution_identity(&[], false, None).unwrap(),
            None
        );
        assert!(restricted_execution_identity(&[flag.clone()], false, None).is_err());
        assert!(restricted_execution_identity(&[flag.clone()], true, None).is_err());

        let encoded = restricted_execution_identity(&[flag.clone()], true, Some(test_identity()))
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<QualificationBuildIdentity>(&encoded).unwrap(),
            test_identity()
        );
        assert!(
            restricted_execution_identity(&[flag.clone(), flag], true, Some(test_identity()))
                .is_err()
        );
        assert!(restricted_execution_identity(
            &["--restrict-privileged-execution".to_string()],
            true,
            Some(QualificationBuildIdentity {
                material_build_digest: "sha256:bad".into(),
                ..test_identity()
            }),
        )
        .is_err());
    }

    fn test_identity() -> QualificationBuildIdentity {
        QualificationBuildIdentity {
            app_version: "0.1.0".into(),
            git_commit: "1".repeat(40),
            material_build_digest: format!("sha256:{}", "a".repeat(64)),
            real_execution_enabled: true,
            qualification_contract: 1,
        }
    }
}

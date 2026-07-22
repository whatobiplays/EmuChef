//! Stable public support-code registry.
//!
//! These codes are a user-facing compatibility contract. Internal errors map
//! into this closed registry; their names and dynamic text are never projected.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SupportCode {
    ServiceStartFailed,
    ServiceUnsupported,
    PlatformToolsInvalid,
    PlatformToolsMissing,
    PlatformToolsLimited,
    DeviceUnavailable,
    CatalogUnavailable,
    CacheInspectionFailed,
    CacheCleanupFailed,
    CacheCleanupPartial,
    UpdateCheckFailed,
    RecoveryDataInvalid,
    ExecutionStateUnavailable,
    SavedConfigurationReferenceMissing,
    SupportUnavailable,
}

/// The bounded subsystem classes accepted by the fallback mapper. Dynamic
/// internal error names and messages are deliberately not part of this enum.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportSubsystem {
    Service,
    PlatformTools,
    Device,
    Catalog,
    Cache,
    Updates,
    Recovery,
    Execution,
    Support,
}

pub struct SupportCodeEntry {
    pub code: &'static str,
    #[allow(dead_code)]
    pub subsystem: &'static str,
    pub message: &'static str,
}

impl SupportCode {
    #[cfg(test)]
    pub const ALL: [Self; 15] = [
        Self::ServiceStartFailed,
        Self::ServiceUnsupported,
        Self::PlatformToolsInvalid,
        Self::PlatformToolsMissing,
        Self::PlatformToolsLimited,
        Self::DeviceUnavailable,
        Self::CatalogUnavailable,
        Self::CacheInspectionFailed,
        Self::CacheCleanupFailed,
        Self::CacheCleanupPartial,
        Self::UpdateCheckFailed,
        Self::RecoveryDataInvalid,
        Self::ExecutionStateUnavailable,
        Self::SavedConfigurationReferenceMissing,
        Self::SupportUnavailable,
    ];

    pub fn entry(self) -> SupportCodeEntry {
        match self {
            Self::ServiceStartFailed => SupportCodeEntry {
                code: "EMUCHEF-SERVICE-START-FAILED",
                subsystem: "service",
                message: "The local app service could not start.",
            },
            Self::ServiceUnsupported => SupportCodeEntry {
                code: "EMUCHEF-SERVICE-UNSUPPORTED",
                subsystem: "service",
                message: "The local app service is not supported by this build.",
            },
            Self::PlatformToolsInvalid => SupportCodeEntry {
                code: "EMUCHEF-PLATFORM-TOOLS-INVALID",
                subsystem: "platform_tools",
                message: "The managed Platform-Tools installation did not pass validation.",
            },
            Self::PlatformToolsMissing => SupportCodeEntry {
                code: "EMUCHEF-PLATFORM-TOOLS-MISSING",
                subsystem: "platform_tools",
                message: "Platform-Tools are not installed for EmuChef.",
            },
            Self::PlatformToolsLimited => SupportCodeEntry {
                code: "EMUCHEF-PLATFORM-TOOLS-LIMITED",
                subsystem: "platform_tools",
                message: "Platform-Tools are available with a compatibility warning.",
            },
            Self::DeviceUnavailable => SupportCodeEntry {
                code: "EMUCHEF-DEVICE-UNAVAILABLE",
                subsystem: "device",
                message: "A previously selected device is not currently available.",
            },
            Self::CatalogUnavailable => SupportCodeEntry {
                code: "EMUCHEF-CATALOG-UNAVAILABLE",
                subsystem: "catalog",
                message: "The bundled setup catalog is unavailable.",
            },
            Self::CacheInspectionFailed => SupportCodeEntry {
                code: "EMUCHEF-CACHE-INSPECTION-FAILED",
                subsystem: "cache",
                message: "App-owned storage could not be inspected safely.",
            },
            Self::CacheCleanupFailed => SupportCodeEntry {
                code: "EMUCHEF-CACHE-CLEANUP-FAILED",
                subsystem: "cache",
                message: "One or more approved cache entries could not be removed.",
            },
            Self::CacheCleanupPartial => SupportCodeEntry {
                code: "EMUCHEF-CACHE-CLEANUP-PARTIAL",
                subsystem: "cache",
                message: "Only part of an approved cache entry was removed.",
            },
            Self::UpdateCheckFailed => SupportCodeEntry {
                code: "EMUCHEF-UPDATES-CHECK-FAILED",
                subsystem: "updates",
                message: "Update information could not be validated.",
            },
            Self::RecoveryDataInvalid => SupportCodeEntry {
                code: "EMUCHEF-RECOVERY-DATA-INVALID",
                subsystem: "recovery",
                message: "Saved recovery data did not pass its safety checks.",
            },
            Self::ExecutionStateUnavailable => SupportCodeEntry {
                code: "EMUCHEF-EXECUTION-STATE-UNAVAILABLE",
                subsystem: "execution",
                message: "Retained execution status is temporarily unavailable.",
            },
            Self::SavedConfigurationReferenceMissing => SupportCodeEntry {
                code: "EMUCHEF-SAVED-REFERENCE-MISSING",
                subsystem: "saved_configuration",
                message: "One or more Recent setup references cannot be opened.",
            },
            Self::SupportUnavailable => SupportCodeEntry {
                code: "EMUCHEF-SUPPORT-UNAVAILABLE",
                subsystem: "support",
                message: "Troubleshooting status is temporarily unavailable.",
            },
        }
    }

    pub fn code(self) -> &'static str {
        self.entry().code
    }

    /// Map an unknown internal failure to a stable public failure class. The
    /// internal value is accepted only to make accidental serialization easy
    /// to regression-test; it never influences or enters the public result.
    #[cfg(test)]
    pub fn fallback_for(subsystem: SupportSubsystem, _internal: &str) -> Self {
        match subsystem {
            SupportSubsystem::Service => Self::ServiceStartFailed,
            SupportSubsystem::PlatformTools => Self::PlatformToolsInvalid,
            SupportSubsystem::Device => Self::DeviceUnavailable,
            SupportSubsystem::Catalog => Self::CatalogUnavailable,
            SupportSubsystem::Cache => Self::CacheInspectionFailed,
            SupportSubsystem::Updates => Self::UpdateCheckFailed,
            SupportSubsystem::Recovery => Self::RecoveryDataInvalid,
            SupportSubsystem::Execution => Self::ExecutionStateUnavailable,
            SupportSubsystem::Support => Self::SupportUnavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn public_registry_is_unique_static_and_uppercase_ascii() {
        let mut codes = HashSet::new();
        for value in SupportCode::ALL {
            let entry = value.entry();
            assert!(codes.insert(entry.code));
            assert!(entry.code.starts_with("EMUCHEF-"));
            assert!(entry
                .code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-'));
            assert!(!entry.message.contains(['/', '\\', '\n', '\r']));
            assert!(!entry.message.to_ascii_lowercase().contains("code"));
        }
    }

    #[test]
    fn mappings_are_deterministic_and_contain_no_dynamic_values() {
        for value in SupportCode::ALL {
            assert_eq!(value.code(), value.entry().code);
            assert!(!value.code().contains("1234"));
            assert!(!value.entry().message.contains("internal_"));
        }
    }

    #[test]
    fn unknown_internal_failures_map_to_bounded_fallbacks_without_leaking_text() {
        let hostile = "/Users/alice/private adb_serial=secret internal_result_9271";
        for subsystem in [
            SupportSubsystem::Service,
            SupportSubsystem::PlatformTools,
            SupportSubsystem::Device,
            SupportSubsystem::Catalog,
            SupportSubsystem::Cache,
            SupportSubsystem::Updates,
            SupportSubsystem::Recovery,
            SupportSubsystem::Execution,
            SupportSubsystem::Support,
        ] {
            let entry = SupportCode::fallback_for(subsystem, hostile).entry();
            assert!(!entry.code.contains(hostile));
            assert!(!entry.message.contains(hostile));
            assert_eq!(
                entry.subsystem,
                SupportCode::fallback_for(subsystem, "different")
                    .entry()
                    .subsystem
            );
        }
    }
}

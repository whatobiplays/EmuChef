#[cfg(test)]
mod tests {
    use super::super::adb::{AdbCommandError, RootProbeFailureReason, RootProbeOutcome};

    #[test]
    fn completed_root_probe_classifies_denied_and_unavailable_as_revoked() {
        assert_eq!(
            super::classify_probe(RootProbeOutcome::Denied),
            Err(AdbCommandError::RootAuthorityRevoked)
        );
        assert_eq!(
            super::classify_probe(RootProbeOutcome::Unavailable),
            Err(AdbCommandError::RootAuthorityRevoked)
        );
    }

    #[test]
    fn completed_unexpected_root_probe_is_unverified() {
        assert_eq!(
            super::classify_probe(RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::UnexpectedResponse,
                message: "unexpected",
            }),
            Err(AdbCommandError::RootAuthorityUnverified)
        );
    }

    #[test]
    fn guard_records_only_trustworthy_completed_mutation_results() {
        let mut guard = super::RootAuthorityGuard::default();
        guard.configure(true);
        assert!(guard.is_authorized());
        assert!(!guard.has_trustworthy_mutation());

        guard.record_result(
            super::DeviceCommandEffect::Mutating,
            &Ok::<(), AdbCommandError>(()),
        );
        assert!(guard.has_trustworthy_mutation());

        let mut fresh = super::RootAuthorityGuard::default();
        fresh.configure(true);
        fresh.record_result(
            super::DeviceCommandEffect::Mutating,
            &Err::<(), _>(AdbCommandError::TimedOut {
                cleanup: crate::owned_process::ProcessCleanup::Confirmed,
            }),
        );
        assert!(!fresh.has_trustworthy_mutation());
        fresh.record_result(
            super::DeviceCommandEffect::Mutating,
            &Err::<(), _>(AdbCommandError::CommandFailed),
        );
        assert!(fresh.has_trustworthy_mutation());
    }

    #[test]
    fn read_only_results_never_create_mutation_evidence() {
        let mut guard = super::RootAuthorityGuard::default();
        guard.configure(true);
        guard.record_result(
            super::DeviceCommandEffect::ReadOnly,
            &Ok::<(), AdbCommandError>(()),
        );
        guard.record_result(
            super::DeviceCommandEffect::ReadOnly,
            &Err::<(), _>(AdbCommandError::CommandFailed),
        );
        assert!(!guard.has_trustworthy_mutation());
    }
}

use super::adb::{AdbCommandError, RootProbeFailureReason, RootProbeOutcome};

pub(crate) const ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER: &str =
    "Root authority could not be confirmed after earlier device changes may have occurred.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeviceCommandEffect {
    ReadOnly,
    Mutating,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RootAuthorityGuard {
    reviewed_root_authorized: bool,
    trustworthy_mutation_completed: bool,
}

impl RootAuthorityGuard {
    pub(super) fn configure(&mut self, reviewed_root_authorized: bool) {
        self.reviewed_root_authorized = reviewed_root_authorized;
        self.trustworthy_mutation_completed = false;
    }

    pub(super) fn is_authorized(&self) -> bool {
        self.reviewed_root_authorized
    }

    pub(super) fn has_trustworthy_mutation(&self) -> bool {
        self.trustworthy_mutation_completed
    }

    pub(super) fn record_result<T>(
        &mut self,
        effect: DeviceCommandEffect,
        result: &Result<T, AdbCommandError>,
    ) {
        if effect == DeviceCommandEffect::Mutating
            && matches!(result, Ok(_) | Err(AdbCommandError::CommandFailed))
        {
            self.trustworthy_mutation_completed = true;
        }
    }
}

pub(super) fn classify_probe(outcome: RootProbeOutcome) -> Result<(), AdbCommandError> {
    match outcome {
        RootProbeOutcome::Granted => Ok(()),
        RootProbeOutcome::Denied | RootProbeOutcome::Unavailable => {
            Err(AdbCommandError::RootAuthorityRevoked)
        }
        RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::UnexpectedResponse,
            ..
        } => Err(AdbCommandError::RootAuthorityUnverified),
        RootProbeOutcome::CheckFailed { .. } => Err(AdbCommandError::RootCheckFailed),
    }
}

//! Phase 6D.6 physical interruption qualification.
//!
//! This module is deliberately ignored by default.  It is a narrow harness
//! around the reviewed `ExecutionPlan` and `ExecutorRunner<RealAdbDevice>`
//! boundary, not a second ADB implementation.  Every invocation selects one
//! scenario, one exact serial, one repetition, and one empty host sentinel
//! directory before it is allowed to query or mutate a device.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{
    adb_inventory, adb_path_exists, adb_query, condition, copy_step, fixture_apk_checksum,
    fixture_root, load_contract, optional_env, validate_owned_destination, validate_package,
    FIXTURE_PACKAGE,
};
use crate::execution_session::ExecutionSessionManager;
use crate::execution_session::ExecutionSlotObservation;
use crate::executor::adb::RealAdbDevice;
use crate::executor::{
    ExecutionProgressEvent, ExecutionRunResult, ExecutorAdapters, ExecutorRunner, FakeDryRunDevice,
    OperationLifecycle, ProgressPhase, StepFailureKind, StepRunRecord, StepRunStatus,
};
use crate::owned_process::{
    arm_test_process_deadline, OwnedProcessLifecycleEvent, OwnedProcessObservationHandle,
    OwnedProcessOperationId, ProcessCleanup, ProcessOperation,
};
use crate::planner::{
    DeviceContext, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource, RuntimeCapabilities,
    TargetDeviceBinding,
};

const PHASE_OPT_IN: &str = "EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS";
const SCENARIO_ENV: &str = "EMUCHEF_PHASE_6D6_SCENARIO";
const REPETITION_ENV: &str = "EMUCHEF_PHASE_6D6_REPETITION";
const SERIAL_ENV: &str = "EMUCHEF_TEST_DEVICE_SERIAL";
const PACKAGE_ALLOWLIST_ENV: &str = "EMUCHEF_TEST_PACKAGE_ALLOWLIST";
const SENTINEL_DIR_ENV: &str = "EMUCHEF_PHASE_6D6_SENTINEL_DIR";
const ROOT_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_ROOT_TESTS";
const ROOT_DESTRUCTIVE_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS";
const ROOT_PREFIX_ALLOWLIST_ENV: &str = "EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST";
const ROOT_PREFIX_ALLOWLIST: &str = concat!(
    "/data/data/com.emuchef.fixture/emuchef-qualification-data/",
    ",",
    "/data/user/0/com.emuchef.fixture/emuchef-qualification-user/"
);
const ROOT_DATA_PREFIX: &str = "/data/data/com.emuchef.fixture/emuchef-qualification-data/";
const ROOT_USER_PREFIX: &str = "/data/user/0/com.emuchef.fixture/emuchef-qualification-user/";
const STORAGE_DESTRUCTIVE_OPT_IN: &str = "EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE";
const AUTHORIZATION_RESET_OPT_IN: &str = "EMUCHEF_PHASE_6D6_AUTHORIZATION_RESET";
const IDENTITY_REPLACEMENT_OPT_IN: &str = "EMUCHEF_PHASE_6D6_IDENTITY_REPLACEMENT";
const HOST_SLEEP_OPT_IN: &str = "EMUCHEF_PHASE_6D6_HOST_SLEEP";
const SENTINEL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SENTINEL_CONTENT_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);
const MIN_STORAGE_INITIAL_FREE_KIB: u64 = 4 * 1024 * 1024;
const RECOVERY_RESERVE_KIB: u64 = 1024 * 1024;
// Keep shell-visible headroom while constructing the bounded reserve/filler.
// The reviewed ADB copy is sized separately against total free blocks so
// filesystem-reserved free space cannot make the mutation unexpectedly fit.
const STORAGE_CLEANUP_HEADROOM_KIB: u64 = 64 * 1024;
const MAX_STORAGE_FILLER_KIB: u64 = 4 * 1024 * 1024;
const MAX_STORAGE_INITIAL_FREE_KIB: u64 =
    RECOVERY_RESERVE_KIB + MAX_STORAGE_FILLER_KIB + STORAGE_CLEANUP_HEADROOM_KIB;
const MAX_SERIAL_BYTES: usize = 256;
const STORAGE_FILL_FILE: &str = "phase6d6-storage-fill.bin";
const STORAGE_RESERVE_FILE: &str = "phase6d6-storage-recovery-reserve.bin";
const LOW_STORAGE_FAILURE_MARGIN_KIB: u64 = 64 * 1024;
const MAX_LOW_STORAGE_HOST_PAYLOAD_KIB: u64 = 512 * 1024;
const ACTIVE_CALIBRATION_KIB: u64 = 256 * 1024;
const ACTIVE_TARGET_MS: u64 = 30_000;
const ACTIVE_MIN_PREDICTED_MS: u64 = 15_000;
const ACTIVE_MAX_PREDICTED_MS: u64 = 240_000;
const ACTIVE_MIN_KIB: u64 = 512 * 1024;
const ACTIVE_MAX_KIB: u64 = 8 * 1024 * 1024;
const ACTIVE_CLEANUP_HEADROOM_KIB: u64 = 1024 * 1024;
const ACTIVE_SAMPLE_FRESHNESS: Duration = Duration::from_secs(5);
const ACTIVE_CALIBRATION_SOURCE_FILE: &str = "phase6d6-active-calibration-source.bin";
const ACTIVE_CALIBRATION_DEST_FILE: &str = "phase6d6-active-calibration-dest.bin";
const ACTIVE_SOURCE_FILE: &str = "phase6d6-active-source.bin";
const ACTIVE_HOST_CHUNK_BYTES: usize = 1024 * 1024;
const TIMEOUT_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(15);
const TIMEOUT_LIVENESS_SAMPLE_OFFSET: Duration = Duration::from_secs(12);
const TIMEOUT_SOURCE_PATH: &str = "/dev/zero";
const TIMEOUT_DESTINATION_PATH: &str = "/dev/null";
/// The scoped, one-shot host-sleep qualification deadline for the private
/// `DeviceCopy` stimulus. 120 seconds gives the operator a practical window to
/// create the pre-suspend markers, physically sleep and wake the host, and (for
/// `host_sleep_before_deadline`) wake before the deadline, while keeping the
/// after-deadline repetition bounded. It is deliberately longer than the
/// 15-second operation-timeout value, which is too short for reliable manual
/// sleep qualification. Production `ProcessOperation::DeviceCopy` remains
/// 300 seconds.
const HOST_SLEEP_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(120);
/// The documented host-sleep measurement tolerance in milliseconds. It covers
/// the enforced 4-second `sleep-entered` handoff window after `sleep-ready`,
/// the 1-second sample-to-`sleep-ready` latency budget, canonical-second
/// sentinel quantization on both the sleep and wake boundaries, and scheduler
/// margin. The enforced handoff plus latency budget and the tolerance are
/// asserted compatible by tests.
const HOST_SLEEP_MEASUREMENT_TOLERANCE_MS: u64 = 8_000;
/// The operator must create `sleep-entered` within this window after the
/// harness creates `sleep-ready`; combined with
/// `HOST_SLEEP_READY_LATENCY_BUDGET` it fits inside the exact-child liveness
/// freshness bound.
const HOST_SLEEP_HANDOFF_WINDOW: Duration = Duration::from_secs(4);
/// The harness must create `sleep-ready` within this budget after the
/// authoritative pre-suspend liveness sample.
const HOST_SLEEP_READY_LATENCY_BUDGET: Duration = Duration::from_secs(1);
const HOST_SLEEP_TIMER_IMPLEMENTATION: &str = "async_io::Timer";
const HOST_SLEEP_MEASUREMENT_BASIS: &str =
    "owned_process_monotonic_deadline_clock_samples_and_sentinel_timestamps";
const SCENARIO_MANIFEST: &str =
    include_str!("../../../../docs/testing/phase-6d6/scenario-manifest.json");

/// Every supported physical scenario allows two independent clean repetitions.
pub const SCENARIOS: [&str; 13] = [
    "cancellation_active",
    "cancellation_boundary",
    "usb_disconnect_active",
    "usb_disconnect_boundary",
    "device_offline",
    "device_unauthorized",
    "identity_stability",
    "identity_replacement",
    "root_revocation",
    "operation_timeout",
    "low_storage",
    "host_sleep_before_deadline",
    "host_sleep_after_deadline",
];

const MANDATORY_SCENARIOS: [&str; 12] = [
    "cancellation_active",
    "cancellation_boundary",
    "usb_disconnect_active",
    "usb_disconnect_boundary",
    "device_unauthorized",
    "identity_stability",
    "identity_replacement",
    "root_revocation",
    "operation_timeout",
    "low_storage",
    "host_sleep_before_deadline",
    "host_sleep_after_deadline",
];
const CONDITIONAL_SCENARIOS: [&str; 1] = ["device_offline"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    CancellationActive,
    CancellationBoundary,
    UsbDisconnectActive,
    UsbDisconnectBoundary,
    DeviceOffline,
    DeviceUnauthorized,
    IdentityStability,
    IdentityReplacement,
    RootRevocation,
    OperationTimeout,
    LowStorage,
    HostSleepBeforeDeadline,
    HostSleepAfterDeadline,
}

impl Scenario {
    const ALL: [Self; 13] = [
        Self::CancellationActive,
        Self::CancellationBoundary,
        Self::UsbDisconnectActive,
        Self::UsbDisconnectBoundary,
        Self::DeviceOffline,
        Self::DeviceUnauthorized,
        Self::IdentityStability,
        Self::IdentityReplacement,
        Self::RootRevocation,
        Self::OperationTimeout,
        Self::LowStorage,
        Self::HostSleepBeforeDeadline,
        Self::HostSleepAfterDeadline,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::CancellationActive => "cancellation_active",
            Self::CancellationBoundary => "cancellation_boundary",
            Self::UsbDisconnectActive => "usb_disconnect_active",
            Self::UsbDisconnectBoundary => "usb_disconnect_boundary",
            Self::DeviceOffline => "device_offline",
            Self::DeviceUnauthorized => "device_unauthorized",
            Self::IdentityStability => "identity_stability",
            Self::IdentityReplacement => "identity_replacement",
            Self::RootRevocation => "root_revocation",
            Self::OperationTimeout => "operation_timeout",
            Self::LowStorage => "low_storage",
            Self::HostSleepBeforeDeadline => "host_sleep_before_deadline",
            Self::HostSleepAfterDeadline => "host_sleep_after_deadline",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.as_str() == value)
            .ok_or_else(|| format!("{SCENARIO_ENV} must name exactly one supported scenario"))
    }

    const fn requires_root(self) -> bool {
        matches!(self, Self::RootRevocation)
    }

    const fn requires_destructive_opt_in(self) -> Option<&'static str> {
        match self {
            Self::LowStorage => Some(STORAGE_DESTRUCTIVE_OPT_IN),
            Self::DeviceUnauthorized => Some(AUTHORIZATION_RESET_OPT_IN),
            Self::IdentityReplacement => Some(IDENTITY_REPLACEMENT_OPT_IN),
            Self::HostSleepBeforeDeadline | Self::HostSleepAfterDeadline => Some(HOST_SLEEP_OPT_IN),
            _ => None,
        }
    }

    const fn is_active_checkpoint(self) -> bool {
        matches!(
            self,
            Self::CancellationActive
                | Self::UsbDisconnectActive
                | Self::DeviceOffline
                | Self::HostSleepBeforeDeadline
                | Self::HostSleepAfterDeadline
        )
    }

    const fn is_boundary_checkpoint(self) -> bool {
        matches!(
            self,
            Self::CancellationBoundary
                | Self::UsbDisconnectBoundary
                | Self::DeviceUnauthorized
                | Self::IdentityStability
                | Self::IdentityReplacement
                | Self::RootRevocation
        )
    }

    const fn supports_host_push_active_stimulus(self) -> bool {
        matches!(
            self,
            Self::CancellationActive | Self::UsbDisconnectActive | Self::DeviceOffline
        )
    }

    const fn requires_process_observer(self) -> bool {
        self.supports_host_push_active_stimulus() || self.requires_device_copy_stimulus()
    }

    const fn requires_device_copy_stimulus(self) -> bool {
        matches!(
            self,
            Self::OperationTimeout | Self::HostSleepBeforeDeadline | Self::HostSleepAfterDeadline
        )
    }

    const fn requires_terminal_recovery(self) -> bool {
        matches!(
            self,
            Self::UsbDisconnectActive
                | Self::UsbDisconnectBoundary
                | Self::DeviceOffline
                | Self::DeviceUnauthorized
        )
    }

    const fn is_root(self) -> bool {
        matches!(self, Self::RootRevocation)
    }
}

const fn active_process_operation(scenario: Scenario) -> ProcessOperation {
    if scenario.supports_host_push_active_stimulus() {
        ProcessOperation::Push
    } else {
        ProcessOperation::DeviceCopy
    }
}

const fn process_operation_class(operation: ProcessOperation) -> Option<&'static str> {
    match operation {
        ProcessOperation::Push => Some("host_push"),
        ProcessOperation::DeviceCopy => Some("device_copy"),
        _ => None,
    }
}

const fn active_operation_class(scenario: Scenario) -> &'static str {
    match process_operation_class(active_process_operation(scenario)) {
        Some(value) => value,
        None => "device_copy",
    }
}

/// Load the single checked-in scenario contract shared with the host
/// validator.  A physical record is never evaluated from the observed
/// `ExecutionRunResult::success` bit alone: this contract defines which
/// expected failures are qualifying and which facts must be present.
fn scenario_contract(scenario: Scenario) -> ScenarioContract {
    let manifest = serde_json::from_str::<ScenarioManifest>(SCENARIO_MANIFEST)
        .expect("the Phase 6D.6 scenario manifest must be valid JSON");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.required_repetitions, 2);
    assert_eq!(manifest.scenarios, SCENARIOS);
    assert_eq!(manifest.mandatory_scenarios, MANDATORY_SCENARIOS);
    assert_eq!(manifest.conditional_scenarios, CONDITIONAL_SCENARIOS);
    assert_eq!(manifest.ui_smoke_scenario, "ui_smoke_composite");
    assert_eq!(manifest.ui_smoke_required_repetitions, 2);
    assert_eq!(
        manifest.ui_smoke_subcases,
        ["cancellation", "transport", "root", "storage", "host_sleep"]
    );
    assert_eq!(manifest.ui_smoke_contracts.len(), 5);
    for name in &manifest.ui_smoke_subcases {
        let contract = manifest
            .ui_smoke_contracts
            .get(name)
            .expect("each UI smoke subcase must have one authoritative contract");
        assert!(!contract.allowed_issue_codes.is_empty());
        assert!(matches!(
            contract.terminal_step_projection.as_str(),
            "failed" | "cancelled"
        ));
        assert!(contract.not_attempted_required);
        assert!(matches!(
            contract.partial_change_presentation.as_str(),
            "possible_partial_change" | "indeterminate"
        ));
        assert!(!contract.recovery_state.is_empty());
        assert!(!contract.authored_title.is_empty());
        assert!(!contract.authored_issue_text.is_empty());
        assert!(!contract.authored_remediation.is_empty());
        assert_eq!(contract.required_artifact_kind, "ui_state_capture");
        assert_eq!(
            contract.forbidden_controls,
            ["resume", "replay", "checkpoint", "ownership_transfer"]
        );
        if name == "cancellation" {
            assert!(!contract.authority_invalidated);
        } else {
            assert!(contract.authority_invalidated);
        }
    }
    assert!(manifest.gates.is_object());
    assert_eq!(
        manifest.outcomes,
        ["passed", "failed", "skipped", "blocked"]
    );
    manifest
        .scenario_contracts
        .get(scenario.as_str())
        .cloned()
        .expect("every physical scenario must have one machine-readable contract")
}

fn qualification_test_result(record: &Value) -> Result<(), String> {
    if record.get("outcome").and_then(Value::as_str) == Some("passed") {
        Ok(())
    } else {
        Err(format!(
            "Phase 6D.6 qualification did not pass: {}",
            record
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("missing outcome")
        ))
    }
}

fn parse_canonical_unix(value: &str) -> Option<u64> {
    let digits = value.strip_prefix("unix:")?;
    if digits.is_empty()
        || digits.len() > 12
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits
        .parse::<u64>()
        .ok()
        .filter(|value| *value <= 253_402_300_799)
}

fn evaluate_scenario_contract(contract: &ScenarioContract, observed: &Value) -> Result<(), String> {
    let success = observed
        .get("success")
        .and_then(Value::as_bool)
        .ok_or_else(|| "scenario result is missing success".to_string())?;
    match contract.expected_execution.as_str() {
        "success" if !success => return Err("expected successful execution".to_string()),
        "failure" if success => return Err("expected execution failure".to_string()),
        "interruption" if success => return Err("expected interrupted execution".to_string()),
        _ => {}
    }
    let issue = observed
        .get("issue")
        .and_then(|value| value.as_str().map(ToString::to_string));
    if !contract.allowed_issue_codes.contains(&issue) {
        return Err(format!("unexpected terminal issue code: {issue:?}"));
    }
    let states = observed
        .get("stepStates")
        .ok_or_else(|| "scenario result is missing step states".to_string())?;
    let matches_state = contract.allowed_step_states.iter().any(|candidate| {
        [
            ("executed", candidate.executed),
            ("skipped", candidate.skipped),
            ("failed", candidate.failed),
            ("cancelled", candidate.cancelled),
            ("blocked", candidate.blocked),
            ("notAttempted", candidate.not_attempted),
        ]
        .into_iter()
        .all(|(field, expected)| states.get(field).and_then(Value::as_u64) == Some(expected as u64))
    });
    if !matches_state {
        return Err("step-state accounting does not match the scenario contract".to_string());
    }
    let partial = observed
        .get("partialChangesPossible")
        .and_then(Value::as_bool)
        .ok_or_else(|| "scenario result is missing partial-change disposition".to_string())?;
    if (contract.partial_changes == "required" && !partial)
        || (contract.partial_changes == "forbidden" && partial)
    {
        return Err("partial-change disposition does not match the scenario contract".to_string());
    }
    let authority = observed
        .get("authorityInvalidated")
        .and_then(Value::as_bool)
        .ok_or_else(|| "scenario result is missing authority disposition".to_string())?;
    if (contract.authority_invalidation == "required" && !authority)
        || (contract.authority_invalidation == "forbidden" && authority)
    {
        return Err("authority invalidation does not match the scenario contract".to_string());
    }
    if contract.requires_active_slot_released
        && observed.get("activeSlotReleased").and_then(Value::as_bool) != Some(true)
    {
        return Err("active execution slot was not observed released".to_string());
    }
    let slot = observed
        .get("activeSlotObservation")
        .and_then(Value::as_object)
        .ok_or_else(|| "production execution slot lifecycle is missing".to_string())?;
    let slot_time = |field: &str| {
        slot.get(field)
            .and_then(Value::as_str)
            .and_then(parse_canonical_unix)
            .ok_or_else(|| format!("execution slot {field} is missing or noncanonical"))
    };
    let acquired_at = slot_time("acquiredAt")?;
    let terminal_cleanup_at = slot_time("terminalCleanupAt")?;
    let released_at = slot_time("releasedAt")?;
    if slot.get("acquired").and_then(Value::as_bool) != Some(true)
        || slot.get("released").and_then(Value::as_bool) != Some(true)
        || slot.get("sourceKind").and_then(Value::as_str) != Some("production_owned_slot")
        || slot.get("evidence").and_then(Value::as_str)
            != Some(contract.slot_observation.source.as_str())
        || (contract.slot_observation.exact_run_scope
            && slot.get("runId").and_then(Value::as_str)
                != observed.get("runScope").and_then(Value::as_str))
        || !(acquired_at <= terminal_cleanup_at && terminal_cleanup_at <= released_at)
    {
        return Err(
            "production execution slot lifecycle does not match the qualifying run".to_string(),
        );
    }
    if let Some(active) = &contract.active_process {
        let evidence = observed
            .get("activeProcess")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                "exact target child liveness and mutation timing evidence is missing".to_string()
            })?;
        let parse_time = |field: &str| {
            evidence
                .get(field)
                .and_then(Value::as_str)
                .and_then(parse_canonical_unix)
                .ok_or_else(|| format!("active process {field} is missing or noncanonical"))
        };
        let spawned = parse_time("spawnedAt")?;
        let mutation = parse_time("mutationStartedAt")?;
        let checked = parse_time("checkedAliveAt")?;
        let action = parse_time("actionAt")?;
        let terminal = parse_time("terminalAt")?;
        let run_bound = evidence.get("runId").and_then(Value::as_str)
            == observed.get("runScope").and_then(Value::as_str);
        let expected_action_kind = if contract.timeout.is_some() {
            "deadline_reached"
        } else {
            "operator_action"
        };
        let action_kind = evidence.get("actionKind").and_then(Value::as_str);
        let chronology_valid = if contract.timeout.is_some() {
            spawned <= mutation && mutation <= checked && checked <= action && action <= terminal
        } else {
            spawned <= mutation && mutation <= checked && checked <= action && action < terminal
        };
        if !active.required
            || evidence.get("operationClass").and_then(Value::as_str)
                != Some(active.operation_class.as_str())
            || (active.exact_run_binding && !run_bound)
            || evidence
                .get("aliveImmediatelyBeforeAction")
                .and_then(Value::as_bool)
                != Some(true)
            || evidence
                .get("terminalReportedBeforeAction")
                .and_then(Value::as_bool)
                != Some(false)
            || !chronology_valid
            || (contract.timeout.is_some() && action_kind != Some(expected_action_kind))
            || (contract.timeout.is_none()
                && action_kind.is_some()
                && action_kind != Some(expected_action_kind))
            || (contract.timeout.is_none()
                && active.action_must_precede_terminal
                && action >= terminal)
        {
            return Err("lifecycle action was not bound to the live target mutation".to_string());
        }
    }
    if let Some(timeout) = &contract.timeout {
        let observed_timeout = observed
            .get("timeout")
            .and_then(Value::as_object)
            .ok_or_else(|| "operation-timeout metadata is missing".to_string())?;
        let expected_timeout_keys = [
            "productionDeadlineMs",
            "qualificationDeadlineMs",
            "deadlineSource",
            "processCleanup",
        ];
        if observed_timeout.len() != expected_timeout_keys.len()
            || expected_timeout_keys
                .iter()
                .any(|key| !observed_timeout.contains_key(*key))
        {
            return Err(
                "operation-timeout metadata must contain exactly the four contract fields"
                    .to_string(),
            );
        }
        let process_cleanup = observed_timeout
            .get("processCleanup")
            .and_then(Value::as_str);
        if !matches!(
            process_cleanup,
            Some("confirmed" | "uncertain" | "not_observed")
        ) {
            return Err("operation-timeout process cleanup state is invalid".to_string());
        }
        if observed_timeout
            .get("productionDeadlineMs")
            .and_then(Value::as_u64)
            != Some(timeout.production_deadline_ms)
            || observed_timeout
                .get("qualificationDeadlineMs")
                .and_then(Value::as_u64)
                != Some(timeout.qualification_deadline_ms)
            || observed_timeout
                .get("deadlineSource")
                .and_then(Value::as_str)
                != Some(timeout.deadline_source.as_str())
            || process_cleanup != Some(timeout.process_cleanup.as_str())
        {
            return Err(
                "operation-timeout metadata does not match the authoritative contract".to_string(),
            );
        }
    } else if observed
        .get("timeout")
        .is_some_and(|timeout| !timeout.is_null())
    {
        return Err("non-timeout scenario contains timeout metadata".to_string());
    }
    if let Some(active) = &contract.active_cancellation {
        let evidence = observed
            .get("activeCancellation")
            .and_then(Value::as_object)
            .ok_or_else(|| "active cancellation evidence is missing".to_string())?;
        if active.required
            && (evidence.get("requestPhase").and_then(Value::as_str)
                != Some(active.request_phase.as_str())
                || evidence
                    .get("requestBeforeFinished")
                    .and_then(Value::as_bool)
                    != Some(active.request_before_finished)
                || evidence
                    .get("laterWorkNotAttempted")
                    .and_then(Value::as_bool)
                    != Some(true))
        {
            return Err(
                "active cancellation was not observed at the required lifecycle phase".to_string(),
            );
        }
        if active.required && issue.is_some() {
            return Err(
                "active cancellation cannot be qualified from an unrelated failure".to_string(),
            );
        }
    }
    if let Some(host_sleep) = &contract.host_sleep {
        let evidence = observed
            .get("hostSleep")
            .and_then(Value::as_object)
            .ok_or_else(|| "host-sleep timing evidence is missing".to_string())?;
        let classification = evidence
            .get("timerClassification")
            .and_then(Value::as_str)
            .ok_or_else(|| "host-sleep timer classification is missing".to_string())?;
        if matches!(classification, "indeterminate" | "contradictory") {
            return Err("indeterminate host-sleep timing cannot qualify".to_string());
        }
        if !host_sleep
            .allowed_timer_classifications
            .iter()
            .any(|allowed| allowed == classification)
        {
            return Err(
                "host-sleep timer classification is outside the scenario contract".to_string(),
            );
        }
        let terminal = evidence
            .get("terminalOutcome")
            .and_then(Value::as_str)
            .ok_or_else(|| "host-sleep terminal outcome is missing".to_string())?;
        if terminal == "transport_loss" && host_sleep.transport_loss_blocks_measurement {
            return Err("transport loss is not timer evidence".to_string());
        }
        if !host_sleep
            .allowed_terminal_outcomes
            .iter()
            .any(|allowed| allowed == terminal)
        {
            return Err("host-sleep terminal outcome is outside the scenario contract".to_string());
        }
        if evidence.get("deadlineClockSource").and_then(Value::as_str)
            != Some(host_sleep.deadline_clock.as_str())
            || host_sleep.classification_basis != "clock_advancement_and_remaining_budget"
        {
            return Err(
                "host-sleep deadline clock does not match the scenario contract".to_string(),
            );
        }
        if host_sleep.measurement_tolerance_required
            && evidence
                .get("measurementToleranceMs")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                == 0
        {
            return Err("host-sleep measurement tolerance is missing".to_string());
        }
        let parse_unix = |field: &str| -> Result<u64, String> {
            let value = evidence
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| value.strip_prefix("unix:"))
                .ok_or_else(|| format!("host-sleep {field} is missing"))?;
            if value.len() > 12
                || (value.len() > 1 && value.starts_with('0'))
                || !value.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(format!("host-sleep {field} is not canonical"));
            }
            value
                .parse::<u64>()
                .map_err(|_| format!("host-sleep {field} is not canonical"))
        };
        let start = parse_unix("operationStartedAt")?;
        let wake = parse_unix("wakeAt")?;
        let deadline_ms = evidence
            .get("deadlineMs")
            .and_then(Value::as_u64)
            .ok_or_else(|| "host-sleep deadline is missing".to_string())?;
        let wake_elapsed_ms = wake.saturating_sub(start).saturating_mul(1000);
        let expected_phase = evidence.get("operatorActionPhase").and_then(Value::as_str);
        let phase_valid = match host_sleep.phase_rule.as_str() {
            "wake_before_deadline_threshold" => wake_elapsed_ms < deadline_ms,
            "wake_at_or_after_deadline_threshold" => wake_elapsed_ms >= deadline_ms,
            _ => false,
        };
        if expected_phase != Some(host_sleep.phase.as_str()) || !phase_valid {
            return Err("host-sleep evidence is from the wrong deadline phase".to_string());
        }
        let measured = HostSleepClockMeasurement {
            suspended_wall_ms: evidence
                .get("suspendedWallMs")
                .and_then(Value::as_u64)
                .ok_or_else(|| "host-sleep suspended wall duration is missing".to_string())?,
            deadline_clock_advance_ms: evidence
                .get("deadlineClockAdvanceDuringSuspensionMs")
                .and_then(Value::as_u64)
                .ok_or_else(|| "host-sleep deadline-clock advancement is missing".to_string())?,
            remaining_before_sleep_ms: evidence
                .get("remainingBeforeSleepMs")
                .and_then(Value::as_u64)
                .ok_or_else(|| "host-sleep pre-sleep budget is missing".to_string())?,
            remaining_after_wake_ms: evidence
                .get("remainingAfterWakeMs")
                .and_then(Value::as_u64)
                .ok_or_else(|| "host-sleep post-wake budget is missing".to_string())?,
            tolerance_ms: evidence
                .get("measurementToleranceMs")
                .and_then(Value::as_u64)
                .ok_or_else(|| "host-sleep tolerance is missing".to_string())?,
        };
        let derived = match classify_host_sleep_clock(measured) {
            HostSleepClockClassification::SuspendedTimeIncluded => "suspended_time_included",
            HostSleepClockClassification::SuspendedTimeExcluded => "suspended_time_excluded",
            HostSleepClockClassification::Indeterminate => "indeterminate",
            HostSleepClockClassification::Contradictory => "contradictory",
        };
        if derived != classification {
            return Err(
                "host-sleep classification is not derived from measured clock advancement"
                    .to_string(),
            );
        }
    }
    if let Some(identity) = &contract.identity_transition {
        let evidence = observed
            .get("identityTransition")
            .and_then(Value::as_object)
            .ok_or_else(|| "identity disconnect and reconnect evidence is missing".to_string())?;
        let time = |field: &str| {
            evidence
                .get(field)
                .and_then(Value::as_str)
                .and_then(parse_canonical_unix)
                .ok_or_else(|| format!("identity {field} is missing or noncanonical"))
        };
        let disconnected = time("originalDisconnectedAt")?;
        let absent_from = time("serialAbsentFrom")?;
        let absent_until = time("serialAbsentUntil")?;
        let attached = time("replacementAttachedAt")?;
        let same_serial = evidence.get("initialSerial") == evidence.get("replacementSerial");
        let same_fingerprint =
            evidence.get("initialFingerprint") == evidence.get("replacementFingerprint");
        let common_valid = identity.required
            && (!identity.same_serial || same_serial)
            && (!identity.requires_serial_absent_interval || absent_until > absent_from)
            && disconnected <= absent_from
            && absent_until <= attached
            && evidence.get("neverSimultaneous").and_then(Value::as_bool)
                == Some(identity.requires_never_simultaneous)
            && evidence
                .get("authorityInvalidated")
                .and_then(Value::as_bool)
                == Some(identity.authority_invalidated)
            && evidence.get("runId").and_then(Value::as_str)
                == observed.get("runScope").and_then(Value::as_str);
        let mode_valid = match identity.mode.as_str() {
            "stable_reconnect" => {
                identity.same_fingerprint
                    && identity.requires_original_disconnect_before_reconnect
                    && same_fingerprint
                    && evidence.get("expectedIssueCode") == Some(&Value::Null)
            }
            "same_serial_replacement" => {
                identity.different_fingerprint
                    && identity.requires_original_disconnect_before_replacement_attach
                    && !same_fingerprint
                    && evidence
                        .get("expectedIssueCode")
                        .and_then(Value::as_str)
                        .is_some_and(|issue| {
                            identity
                                .terminal_issue_codes
                                .iter()
                                .any(|allowed| allowed == issue)
                        })
            }
            _ => false,
        };
        if !common_valid || !mode_valid {
            return Err(
                "identity evidence does not prove the contracted reconnect transition".to_string(),
            );
        }
    }
    if let Some(authorization) = &contract.authorization_transition {
        let evidence = observed
            .get("authorizationTransition")
            .and_then(Value::as_object)
            .ok_or_else(|| "authorization transition evidence is missing".to_string())?;
        let time = |field: &str| {
            evidence
                .get(field)
                .and_then(Value::as_str)
                .and_then(parse_canonical_unix)
                .ok_or_else(|| format!("authorization {field} is missing or noncanonical"))
        };
        let initial = time("initialObservedAt")?;
        let operation = time("operationStartedAt")?;
        let first_completed = time("firstOperationCompletedAt")?;
        let revoked = time("revocationCheckpointAt")?;
        let disconnected = time("originalDisconnectedAt")?;
        let absent_from = time("serialAbsentFrom")?;
        let absent_until = time("serialAbsentUntil")?;
        let reconnected = time("reconnectedAt")?;
        let unauthorized = time("observedAt")?;
        let terminal = time("terminalDetectedAt")?;
        let cleanup_started = time("cleanupStartedAt")?;
        let cleanup_completed = time("cleanupCompletedAt")?;
        let final_observed = time("finalStateObservedAt")?;
        let boundary_ready = observed
            .get("boundaryReadyAt")
            .and_then(Value::as_str)
            .and_then(parse_canonical_unix)
            .ok_or_else(|| {
                "authorization boundaryReadyAt is missing or noncanonical".to_string()
            })?;
        let operator_action = observed
            .get("operatorActionAt")
            .and_then(Value::as_str)
            .and_then(parse_canonical_unix)
            .ok_or_else(|| {
                "authorization operatorActionAt is missing or noncanonical".to_string()
            })?;
        let chronology = initial < operation
            && operation < first_completed
            && first_completed <= boundary_ready
            && boundary_ready < revoked
            && revoked <= disconnected
            && disconnected <= absent_from
            && absent_from < absent_until
            && absent_until <= reconnected
            && reconnected <= unauthorized
            && unauthorized <= operator_action
            && operator_action <= terminal
            && terminal < cleanup_started
            && cleanup_started <= cleanup_completed
            && cleanup_completed < final_observed;
        let contract_chronology = authorization.initial_observation_before_operation
            && authorization.first_operation_completed_before_revocation
            && authorization.revocation_before_disconnect
            && authorization.requires_serial_absent_interval
            && authorization.reconnect_before_unauthorized
            && authorization.unauthorized_before_or_at_terminal
            && authorization.terminal_before_cleanup
            && authorization.final_state_after_cleanup;
        let issue_valid = evidence
            .get("issueCode")
            .and_then(Value::as_str)
            .is_some_and(|transition_issue| {
                issue.as_deref() == Some(transition_issue)
                    && authorization
                        .terminal_issue_codes
                        .iter()
                        .any(|allowed| allowed == transition_issue)
            });
        let scope_valid = evidence.get("runId").and_then(Value::as_str)
            == observed.get("runScope").and_then(Value::as_str)
            && evidence.get("deviceScope").and_then(Value::as_str)
                == observed.get("deviceScope").and_then(Value::as_str);
        if !authorization.required
            || evidence.get("initialState").and_then(Value::as_str)
                != Some(authorization.initial_state.as_str())
            || evidence.get("observedState").and_then(Value::as_str)
                != Some(authorization.revoked_state.as_str())
            || !chronology
            || !contract_chronology
            || !issue_valid
            || (authorization.exact_run_and_device_scope && !scope_valid)
            || evidence
                .get("authorityInvalidated")
                .and_then(Value::as_bool)
                != Some(authorization.authority_invalidated)
            || evidence.get("automaticResume").and_then(Value::as_bool)
                != Some(authorization.automatic_resume)
        {
            return Err(
                "authorization evidence does not prove the contracted chronology".to_string(),
            );
        }
    }
    if observed.get("cleanup").and_then(Value::as_str) != Some(contract.cleanup_outcome.as_str()) {
        return Err("cleanup outcome does not match the scenario contract".to_string());
    }
    if observed.get("residual").and_then(Value::as_str) != Some(contract.residual_outcome.as_str())
    {
        return Err("residual outcome does not match the scenario contract".to_string());
    }
    Ok(())
}

fn validate_low_storage_preflight(input: LowStoragePreflight) -> Result<(), String> {
    if input.initial_free_kib < MIN_STORAGE_INITIAL_FREE_KIB {
        return Err(
            "low-storage qualification requires at least four GiB free before mutation".to_string(),
        );
    }
    if input.recovery_reserve_kib < RECOVERY_RESERVE_KIB {
        return Err("low-storage qualification requires a one-GiB recovery reserve".to_string());
    }
    if input.filler_kib == 0 || input.filler_kib > input.max_filler_kib {
        return Err("low-storage filler allocation is outside its explicit bound".to_string());
    }
    if input.initial_free_kib
        > input
            .recovery_reserve_kib
            .saturating_add(input.max_filler_kib)
            .saturating_add(STORAGE_CLEANUP_HEADROOM_KIB)
    {
        return Err("low-storage preflight would exceed the bounded filler allocation".to_string());
    }
    if input.filler_kib
        > input
            .initial_free_kib
            .saturating_sub(input.recovery_reserve_kib)
            .saturating_sub(STORAGE_CLEANUP_HEADROOM_KIB)
    {
        return Err("low-storage filler would consume cleanup headroom".to_string());
    }
    if !input.reserve_owned || !input.filler_owned {
        return Err("low-storage filler and recovery reserve must be fixture-owned".to_string());
    }
    Ok(())
}

fn bounded_storage_filler_kib(after_reserve_kib: u64) -> u64 {
    after_reserve_kib
        .saturating_sub(STORAGE_CLEANUP_HEADROOM_KIB)
        .min(MAX_STORAGE_FILLER_KIB)
}

fn bounded_storage_filler_kib_from_observations(
    initial_free_kib: u64,
    after_reserve_kib: u64,
) -> u64 {
    let initial_budget_kib = initial_free_kib
        .saturating_sub(RECOVERY_RESERVE_KIB)
        .saturating_sub(STORAGE_CLEANUP_HEADROOM_KIB)
        .min(MAX_STORAGE_FILLER_KIB);

    bounded_storage_filler_kib(after_reserve_kib).min(initial_budget_kib)
}

fn storage_cleanup_order() -> [&'static str; 4] {
    ["payload", "filler", "sentinel", "recovery-reserve"]
}

fn fixture_owned_run_path(path: &str) -> bool {
    let root = "/sdcard/EmuChefQualification/com.emuchef.fixture/output/";
    path.starts_with(root) && !path.contains("..") && path[root.len()..].starts_with("phase6d6-")
}

fn run_scope_paths(invocation: &Invocation) -> (String, String) {
    let scope_suffix = invocation
        .run_scope
        .rsplit('/')
        .next()
        .unwrap_or("phase6d6-invalid-scope");
    if invocation.scenario.is_root() {
        (
            format!("{ROOT_DATA_PREFIX}{scope_suffix}/phase6d6-first.txt"),
            format!("{ROOT_USER_PREFIX}{scope_suffix}/phase6d6-second.txt"),
        )
    } else {
        (
            format!("{}/phase6d6-first.txt", invocation.run_scope),
            format!("{}/phase6d6-second.txt", invocation.run_scope),
        )
    }
}

fn run_scope_roots(invocation: &Invocation) -> Vec<String> {
    if invocation.scenario.is_root() {
        let scope_suffix = invocation
            .run_scope
            .rsplit('/')
            .next()
            .unwrap_or("phase6d6-invalid-scope");
        vec![
            format!("{ROOT_DATA_PREFIX}{scope_suffix}"),
            format!("{ROOT_USER_PREFIX}{scope_suffix}"),
        ]
    } else {
        vec![invocation.run_scope.clone()]
    }
}

#[derive(Clone, Debug)]
struct Invocation {
    scenario: Scenario,
    repetition: u8,
    serial: String,
    sentinel: Sentinel,
    contract: super::QualificationContract,
    run_scope: String,
    run_id: String,
    sentinel_id: String,
    sentinel_nonce: String,
    evidence_path: String,
    trace_path: String,
    scenario_contract: ScenarioContract,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ScenarioContract {
    expected_outcome: String,
    expected_execution: String,
    allowed_issue_codes: Vec<Option<String>>,
    allowed_step_states: Vec<StepStateContract>,
    partial_changes: String,
    authority_invalidation: String,
    requires_active_slot_released: bool,
    slot_observation: SlotObservationContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_cancellation: Option<ActiveCancellationContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_process: Option<ActiveProcessContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<TimeoutContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_sleep: Option<HostSleepContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_transition: Option<IdentityTransitionContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_transition: Option<AuthorizationTransitionContract>,
    cleanup_outcome: String,
    residual_outcome: String,
    facts: ScenarioFactsContract,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SlotObservationContract {
    source: String,
    exact_run_scope: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ActiveCancellationContract {
    required: bool,
    request_phase: String,
    request_before_finished: bool,
    terminal_behavior: String,
    disallowed_issue_codes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ActiveProcessContract {
    required: bool,
    operation_class: String,
    action_must_precede_terminal: bool,
    exact_run_binding: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TimeoutContract {
    production_deadline_ms: u64,
    qualification_deadline_ms: u64,
    deadline_source: String,
    process_cleanup: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct IdentityTransitionContract {
    required: bool,
    mode: String,
    same_serial: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    same_fingerprint: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    different_fingerprint: bool,
    requires_serial_absent_interval: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    requires_original_disconnect_before_reconnect: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    requires_original_disconnect_before_replacement_attach: bool,
    requires_never_simultaneous: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    terminal_issue_codes: Vec<String>,
    authority_invalidated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AuthorizationTransitionContract {
    required: bool,
    initial_state: String,
    revoked_state: String,
    terminal_issue_codes: Vec<String>,
    authority_invalidated: bool,
    automatic_resume: bool,
    initial_observation_before_operation: bool,
    first_operation_completed_before_revocation: bool,
    revocation_before_disconnect: bool,
    requires_serial_absent_interval: bool,
    reconnect_before_unauthorized: bool,
    unauthorized_before_or_at_terminal: bool,
    terminal_before_cleanup: bool,
    final_state_after_cleanup: bool,
    exact_run_and_device_scope: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct HostSleepContract {
    phase: String,
    allowed_timer_classifications: Vec<String>,
    allowed_terminal_outcomes: Vec<String>,
    transport_loss_blocks_measurement: bool,
    deadline_clock: String,
    classification_basis: String,
    measurement_tolerance_required: bool,
    phase_rule: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct StepStateContract {
    executed: usize,
    skipped: usize,
    failed: usize,
    cancelled: usize,
    blocked: usize,
    not_attempted: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ScenarioFactsContract {
    root_shell: bool,
    active_checkpoint: bool,
    boundary_checkpoint: bool,
    requires_sentinel_action: bool,
    storage: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScenarioManifest {
    schema_version: u32,
    scenarios: Vec<String>,
    mandatory_scenarios: Vec<String>,
    conditional_scenarios: Vec<String>,
    required_repetitions: u8,
    ui_smoke_scenario: String,
    ui_smoke_required_repetitions: u8,
    ui_smoke_subcases: Vec<String>,
    ui_smoke_contracts: std::collections::BTreeMap<String, UiSmokeContract>,
    scenario_contracts: std::collections::BTreeMap<String, ScenarioContract>,
    gates: serde_json::Value,
    outcomes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiSmokeContract {
    allowed_issue_codes: Vec<Option<String>>,
    terminal_step_projection: String,
    not_attempted_required: bool,
    partial_change_presentation: String,
    authority_invalidated: bool,
    recovery_state: String,
    authored_title: String,
    authored_issue_text: String,
    authored_remediation: String,
    required_artifact_kind: String,
    forbidden_controls: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
struct LowStoragePreflight {
    initial_free_kib: u64,
    recovery_reserve_kib: u64,
    filler_kib: u64,
    max_filler_kib: u64,
    reserve_owned: bool,
    filler_owned: bool,
}

#[derive(Clone, Debug)]
struct StorageObservation {
    initial_free_kib: u64,
    recovery_reserve_kib: u64,
    filler_kib: u64,
    payload_kib: u64,
    final_free_kib: Option<u64>,
    restored_recovery_reserve_kib: Option<u64>,
    reserve_created: bool,
    reserve_removed: bool,
    ownership_verified: bool,
    bounded_allocation: bool,
    cleanup_verified: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveStimulusDerivation {
    payload_kib: u64,
    predicted_ms: u64,
}

#[derive(Debug)]
struct ActiveStimulus {
    host_workspace: tempfile::TempDir,
    host_source_path: PathBuf,
    device_destination_path: String,
    payload_kib: u64,
    predicted_ms: u64,
}

/// Exact private pseudo-device copy used only by the timeout qualification.
/// `cp /dev/zero /dev/null` is a genuinely live device-side copy with no
/// persistent payload; it requires no special-file creation and stays active
/// until the scoped qualification deadline wins through the ordinary
/// owned-process timeout path.
#[derive(Clone, Debug)]
struct TimeoutStimulus {
    device_source_path: String,
    device_destination_path: String,
}

fn derive_active_stimulus(
    calibration_kib: u64,
    elapsed_ms: u64,
) -> Result<ActiveStimulusDerivation, String> {
    if calibration_kib == 0 || elapsed_ms == 0 {
        return Err(
            "active host-push calibration did not produce measurable throughput".to_string(),
        );
    }
    let target = (calibration_kib as u128)
        .saturating_mul(ACTIVE_TARGET_MS as u128)
        .checked_div(elapsed_ms as u128)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            "active host-push calibration overflowed its bounded calculation".to_string()
        })?;
    let payload_kib = target.clamp(ACTIVE_MIN_KIB, ACTIVE_MAX_KIB);
    let predicted_ms = (payload_kib as u128)
        .saturating_mul(elapsed_ms as u128)
        .checked_div(calibration_kib as u128)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "active host-push predicted duration was not representable".to_string())?;
    if !(ACTIVE_MIN_PREDICTED_MS..=ACTIVE_MAX_PREDICTED_MS).contains(&predicted_ms) {
        return Err(
            "active host-push calibration cannot provide the required bounded operator window"
                .to_string(),
        );
    }
    Ok(ActiveStimulusDerivation {
        payload_kib,
        predicted_ms,
    })
}

fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn write_active_host_bytes(file: &mut fs::File, byte_len: u64, seed: u64) -> io::Result<()> {
    let mut state = seed;
    let mut remaining = byte_len;
    let mut buffer = vec![0_u8; ACTIVE_HOST_CHUNK_BYTES];
    while remaining > 0 {
        let count = usize::try_from(remaining.min(ACTIVE_HOST_CHUNK_BYTES as u64))
            .expect("bounded active host chunk length should fit usize");
        let mut offset = 0;
        while offset < count {
            let word = splitmix64_next(&mut state).to_le_bytes();
            let available = (count - offset).min(word.len());
            buffer[offset..offset + available].copy_from_slice(&word[..available]);
            offset += available;
        }
        file.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    Ok(())
}

fn write_active_host_fixture_with(
    path: &Path,
    byte_len: u64,
    seed: u64,
    writer: impl FnOnce(&mut fs::File, u64, u64) -> io::Result<()>,
) -> Result<(), String> {
    if byte_len == 0 {
        return Err("active host fixture length must be non-zero".to_string());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "active host fixture could not be created".to_string())?;
    let result = writer(&mut file, byte_len, seed)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all());
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
        return Err("active host fixture could not be written".to_string());
    }
    drop(file);
    if fs::metadata(path).map(|metadata| metadata.len()).ok() != Some(byte_len) {
        let _ = fs::remove_file(path);
        return Err("active host fixture size could not be verified".to_string());
    }
    Ok(())
}

fn write_active_host_fixture(path: &Path, byte_len: u64, seed: u64) -> Result<(), String> {
    write_active_host_fixture_with(path, byte_len, seed, write_active_host_bytes)
}

fn active_host_seed(run_scope: &str) -> u64 {
    let encoded = digest(&format!("phase6d6-active-host:{run_scope}"));
    u64::from_str_radix(&encoded[..16], 16)
        .expect("the internal active-host digest prefix should be hexadecimal")
}

fn active_stimulus_device_paths(invocation: &Invocation) -> (String, String) {
    (
        format!("{}/{}", invocation.run_scope, ACTIVE_CALIBRATION_DEST_FILE),
        run_scope_paths(invocation).0,
    )
}

fn is_timeout_source_path(path: &str) -> bool {
    path == TIMEOUT_SOURCE_PATH
}

fn is_timeout_destination_path(path: &str) -> bool {
    path == TIMEOUT_DESTINATION_PATH
}

fn reviewed_plan_source_location(scenario: Scenario) -> &'static str {
    if scenario.requires_device_copy_stimulus() {
        "device"
    } else {
        "host"
    }
}

fn cleanup_device_paths_for_scenario(
    scenario: Scenario,
    first: &str,
    second: &str,
    run_scope: &str,
) -> Vec<String> {
    let mut paths = vec![first.to_string(), second.to_string()];
    if scenario.supports_host_push_active_stimulus() {
        paths.push(format!("{run_scope}/{ACTIVE_CALIBRATION_DEST_FILE}"));
    }
    if scenario == Scenario::LowStorage {
        paths.push(format!("{run_scope}/{STORAGE_FILL_FILE}"));
        paths.push(format!("{run_scope}/{STORAGE_RESERVE_FILE}"));
    }
    paths
}

fn canonical_unix_seconds(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn active_process_evidence(
    events: &[OwnedProcessLifecycleEvent],
    action_time: Option<SystemTime>,
    run_scope: &str,
    expected_operation: ProcessOperation,
) -> Option<Value> {
    let action = action_time?;
    let operation_class = process_operation_class(expected_operation)?;
    let (operation_id, mutation_started) = events.iter().find_map(|event| match event {
        OwnedProcessLifecycleEvent::MutationStarted {
            operation_id,
            operation,
            at,
        } if *operation == expected_operation => Some((*operation_id, *at)),
        _ => None,
    })?;
    let spawned = events.iter().find_map(|event| match event {
        OwnedProcessLifecycleEvent::Spawned {
            operation_id: observed,
            operation,
            at,
        } if *observed == operation_id && *operation == expected_operation => Some(*at),
        _ => None,
    })?;
    let (checked_alive, alive, terminal_reported) =
        events.iter().find_map(|event| match event {
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: observed,
                operation,
                at,
                alive,
                terminal_reported,
            } if *observed == operation_id && *operation == expected_operation => {
                Some((*at, *alive, *terminal_reported))
            }
            _ => None,
        })?;
    let terminal = events.iter().find_map(|event| match event {
        OwnedProcessLifecycleEvent::Terminal {
            operation_id: observed,
            operation,
            at,
        } if *observed == operation_id && *operation == expected_operation => Some(*at),
        _ => None,
    })?;
    let freshness = action.duration_since(checked_alive).ok()?;
    let action_second = canonical_unix_seconds(action)?;
    let terminal_second = canonical_unix_seconds(terminal)?;
    if alive != Some(true)
        || terminal_reported
        || !(spawned <= mutation_started
            && mutation_started <= checked_alive
            && checked_alive <= action
            && action < terminal)
        || freshness > ACTIVE_SAMPLE_FRESHNESS
        || action_second >= terminal_second
    {
        return None;
    }
    let raw_identity = operation_id.as_u64();
    Some(json!({
        "runId": run_scope,
        "operationId": format!(
            "operation-sha256:{}",
            digest(&format!(
                "phase6d6-operation:{run_scope}:{raw_identity}"
            ))
        ),
        "operationClass": operation_class,
        "childIdentity": format!(
            "child-sha256:{}",
            digest(&format!("phase6d6-child:{run_scope}:{raw_identity}"))
        ),
        "spawnedAt": system_time_value(Some(spawned)),
        "mutationStartedAt": system_time_value(Some(mutation_started)),
        "checkedAliveAt": system_time_value(Some(checked_alive)),
        "actionAt": system_time_value(Some(action)),
        "actionKind": "operator_action",
        "terminalAt": system_time_value(Some(terminal)),
        "aliveImmediatelyBeforeAction": true,
        "terminalReportedBeforeAction": false,
    }))
}

fn timeout_process_evidence(
    events: &[OwnedProcessLifecycleEvent],
    run_scope: &str,
) -> Option<Value> {
    // The shared observer records lifecycle events for many operations. Select the
    // timeout target from the unique DeviceCopy deadline at the qualification
    // duration, then validate only that operation's complete lifecycle.
    let deadline_targets: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline,
                at,
            } if *deadline == TIMEOUT_QUALIFICATION_DEADLINE => Some((*operation_id, *at)),
            _ => None,
        })
        .collect();
    if deadline_targets.len() != 1 {
        return None;
    }
    let (operation_id, _) = deadline_targets[0];

    // An event for the selected operation ID with a different operation class
    // makes the identity correlation ambiguous, even when the DeviceCopy events
    // otherwise form a valid sequence.
    if events.iter().any(|event| {
        let (observed_id, operation) = match event {
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::DeadlineClockStarted {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id,
                operation,
                ..
            }
            | OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation,
                ..
            } => (*operation_id, *operation),
        };
        observed_id == operation_id && operation != ProcessOperation::DeviceCopy
    }) {
        return None;
    }

    let spawned: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::Spawned {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
            } if *observed == operation_id => Some(*at),
            _ => None,
        })
        .collect();
    let mutation_started: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
            } if *observed == operation_id => Some(*at),
            _ => None,
        })
        .collect();
    let liveness: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
                alive,
                terminal_reported,
            } if *observed == operation_id => Some((*at, *alive, *terminal_reported)),
            _ => None,
        })
        .collect();
    let deadlines: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                deadline,
                at,
            } if *observed == operation_id => Some((*at, *deadline)),
            _ => None,
        })
        .collect();
    let terminals: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            OwnedProcessLifecycleEvent::Terminal {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
            } if *observed == operation_id => Some(*at),
            _ => None,
        })
        .collect();
    if spawned.len() != 1
        || mutation_started.len() != 1
        || liveness.len() != 1
        || deadlines.len() != 1
        || terminals.len() != 1
    {
        return None;
    }
    let spawned = spawned[0];
    let mutation_started = mutation_started[0];
    let (checked_alive, alive, terminal_reported) = liveness[0];
    let (deadline_at, deadline) = deadlines[0];
    let terminal = terminals[0];
    if alive != Some(true)
        || terminal_reported
        || deadline != TIMEOUT_QUALIFICATION_DEADLINE
        || !(spawned <= mutation_started
            && mutation_started < checked_alive
            && checked_alive < deadline_at
            && deadline_at < terminal)
    {
        return None;
    }
    let raw_identity = operation_id.as_u64();
    Some(json!({
        "runId": run_scope,
        "operationId": format!(
            "operation-sha256:{}",
            digest(&format!("phase6d6-operation:{run_scope}:{raw_identity}"))
        ),
        "operationClass": "device_copy",
        "childIdentity": format!(
            "child-sha256:{}",
            digest(&format!("phase6d6-child:{run_scope}:{raw_identity}"))
        ),
        "spawnedAt": system_time_value(Some(spawned)),
        "mutationStartedAt": system_time_value(Some(mutation_started)),
        "checkedAliveAt": system_time_value(Some(checked_alive)),
        "actionAt": system_time_value(Some(deadline_at)),
        "actionKind": "deadline_reached",
        "terminalAt": system_time_value(Some(terminal)),
        "aliveImmediatelyBeforeAction": true,
        "terminalReportedBeforeAction": false,
    }))
}

fn timeout_process_cleanup(result: Result<&ExecutionRunResult, &String>) -> &'static str {
    match result
        .ok()
        .and_then(|run| run.steps.first())
        .and_then(|step| step.cleanup)
    {
        Some(ProcessCleanup::Confirmed) => "confirmed",
        Some(ProcessCleanup::Uncertain) => "uncertain",
        Some(ProcessCleanup::NotRequired) | None => "not_observed",
    }
}

fn timeout_metadata(result: Result<&ExecutionRunResult, &String>) -> Value {
    json!({
        "productionDeadlineMs": ProcessOperation::DeviceCopy.deadline().as_millis(),
        "qualificationDeadlineMs": TIMEOUT_QUALIFICATION_DEADLINE.as_millis(),
        "deadlineSource": "test_only_scoped_override",
        "processCleanup": timeout_process_cleanup(result),
    })
}

#[derive(Clone, Debug)]
struct Sentinel {
    directory: PathBuf,
}

impl Sentinel {
    fn from_environment() -> Result<Self, String> {
        let raw = env::var(SENTINEL_DIR_ENV).map_err(|_| {
            format!("{SENTINEL_DIR_ENV} must point to an empty test-owned directory")
        })?;
        let directory = PathBuf::from(raw);
        if !directory.is_absolute()
            || directory
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "{SENTINEL_DIR_ENV} must be an absolute path without '..'"
            ));
        }
        if !directory.is_dir() {
            return Err(format!(
                "{SENTINEL_DIR_ENV} must name an existing directory"
            ));
        }
        if fs::read_dir(&directory)
            .map_err(|_| "the sentinel directory could not be inspected".to_string())?
            .next()
            .is_some()
        {
            return Err("the sentinel directory must be empty before qualification".to_string());
        }
        Ok(Self { directory })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn mark(&self, name: &str, contents: &str) -> Result<SystemTime, String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path(name))
            .map_err(|_| format!("sentinel {name} could not be created"))?;
        file.write_all(contents.as_bytes())
            .map_err(|_| format!("sentinel {name} could not be written"))?;
        self.marker_time(name)
    }

    fn marker_time(&self, name: &str) -> Result<SystemTime, String> {
        fs::metadata(self.path(name))
            .map_err(|_| format!("sentinel {name} timestamp is unavailable"))?
            .modified()
            .map_err(|_| format!("sentinel {name} timestamp is unavailable"))
    }

    fn named_action_now(&self, name: &str) -> Result<Option<SystemTime>, String> {
        let action = self.path(name);
        if !action.exists() {
            if self.path("abort").exists() {
                return Err("operator aborted the bounded checkpoint".to_string());
            }
            return Ok(None);
        }
        let settle_started = Instant::now();
        loop {
            let content = fs::read_to_string(&action)
                .map_err(|_| format!("{name} sentinel could not be read"))?;
            if content == "ack\n" {
                return self.marker_time(name).map(Some);
            }
            if settle_started.elapsed() >= SENTINEL_CONTENT_SETTLE_TIMEOUT {
                return Err(format!("{name} sentinel must contain exactly 'ack'"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn action_now(&self) -> Result<Option<SystemTime>, String> {
        self.named_action_now("operator-action")
    }

    fn wait_for_named_action_after(
        &self,
        name: &str,
        not_before: SystemTime,
    ) -> Result<SystemTime, String> {
        let started = Instant::now();
        loop {
            if let Some(timestamp) = self.named_action_now(name)? {
                if timestamp < not_before {
                    return Err(format!("{name} checkpoint marker was stale"));
                }
                return Ok(timestamp);
            }
            if started.elapsed() >= SENTINEL_TIMEOUT {
                return Err(format!("{name} checkpoint timed out after ten minutes"));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn wait_for_action_after(&self, not_before: SystemTime) -> Result<SystemTime, String> {
        self.wait_for_named_action_after("operator-action", not_before)
    }

    fn cleanup(&self) -> Result<(), String> {
        for name in [
            "armed",
            "operation-started",
            "active-ready",
            "sleep-ready",
            "boundary-ready",
            "operation-finished",
            "terminal-ready",
            "authorization-revoked",
            "unauthorized-observed",
            "cleanup-ready",
            "sleep-requested",
            "sleep-entered",
            "wake",
        ] {
            let path = self.path(name);
            if path.exists() {
                fs::remove_file(path)
                    .map_err(|_| "sentinel cleanup could not remove a marker".to_string())?;
            }
        }
        if self.path("operator-action").exists() {
            fs::remove_file(self.path("operator-action"))
                .map_err(|_| "sentinel cleanup could not remove operator action".to_string())?;
        }
        if self.path("abort").exists() {
            fs::remove_file(self.path("abort"))
                .map_err(|_| "sentinel cleanup could not remove abort marker".to_string())?;
        }
        if fs::read_dir(&self.directory)
            .map_err(|_| "sentinel residual state could not be inspected".to_string())?
            .next()
            .is_some()
        {
            return Err("sentinel directory retained unexpected residual state".to_string());
        }
        Ok(())
    }
}

fn wait_for_sentinel_marker(
    sentinel: &Sentinel,
    name: &str,
    timeout: Duration,
) -> Result<SystemTime, String> {
    let started = Instant::now();
    loop {
        if let Ok(time) = sentinel.marker_time(name) {
            return Ok(time);
        }
        if sentinel.path("abort").exists() {
            return Err("operator aborted the bounded checkpoint".to_string());
        }
        if started.elapsed() >= timeout {
            return Err(format!("{name} checkpoint timed out after ten minutes"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_sentinel_action(
    sentinel: &Sentinel,
    name: &str,
    timeout: Duration,
) -> Result<SystemTime, String> {
    let started = Instant::now();
    loop {
        if let Some(time) = sentinel.named_action_now(name)? {
            return Ok(time);
        }
        if started.elapsed() >= timeout {
            return Err(format!("{name} checkpoint timed out after ten minutes"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Run the deterministic host-sleep pre-suspend handshake.
///
/// The operator creates `sleep-requested` while the host is awake; the watcher
/// proves the exact `DeviceCopy` child is alive, samples the exact
/// owned-process deadline clock, creates `sleep-ready`, and only then allows
/// the operator to create `sleep-entered` (the final awake handoff, within a
/// bounded window) and physically suspend the host. After resume the operator
/// creates `wake`, and the watcher samples the deadline clock again from the
/// retained exact basis. The returned time is the `sleep-entered` handoff,
/// which is the activeProcess action boundary for host sleep.
fn run_host_sleep_watcher(
    observer: &OwnedProcessObservationHandle,
    sentinel: &Sentinel,
) -> Result<SystemTime, String> {
    let operation_started =
        wait_for_sentinel_marker(sentinel, "operation-started", SENTINEL_TIMEOUT)?;
    let sleep_requested = wait_for_sentinel_action(sentinel, "sleep-requested", SENTINEL_TIMEOUT)?;
    if sleep_requested < operation_started {
        return Err("sleep-requested predates operation start".to_string());
    }
    let operation_id =
        observer.wait_for_mutation(ProcessOperation::DeviceCopy, SENTINEL_TIMEOUT)?;
    observer.request_liveness_sample(operation_id)?;
    let liveness = observer.wait_for_liveness(operation_id, SENTINEL_TIMEOUT)?;
    if liveness.alive != Some(true) || liveness.terminal_reported {
        return Err(
            "the exact target DeviceCopy child was not alive at the pre-suspend handoff"
                .to_string(),
        );
    }
    observer.request_deadline_clock_sample(operation_id)?;
    let sleep_ready = sentinel.mark("sleep-ready", "ready\n")?;
    let ready_latency = sleep_ready
        .duration_since(liveness.at)
        .map_err(|_| "sleep-ready preceded the exact-child liveness sample".to_string())?;
    if ready_latency > HOST_SLEEP_READY_LATENCY_BUDGET {
        return Err("sleep-ready exceeded the pre-suspend sample latency budget".to_string());
    }
    let sleep_entered = wait_for_sentinel_action(sentinel, "sleep-entered", SENTINEL_TIMEOUT)?;
    if sleep_entered < sleep_ready {
        return Err("sleep-entered predates the sleep-ready handshake".to_string());
    }
    let handoff = sleep_entered
        .duration_since(sleep_ready)
        .map_err(|_| "sleep-entered predates the sleep-ready handshake".to_string())?;
    if handoff > HOST_SLEEP_HANDOFF_WINDOW {
        return Err("sleep-entered exceeded the bounded handoff window".to_string());
    }
    let freshness = sleep_entered
        .duration_since(liveness.at)
        .map_err(|_| "sleep-entered preceded the exact-child liveness sample".to_string())?;
    if freshness > ACTIVE_SAMPLE_FRESHNESS {
        return Err("sleep-entered exceeded the exact-child liveness freshness window".to_string());
    }
    let wake = wait_for_sentinel_action(sentinel, "wake", SENTINEL_TIMEOUT)?;
    if wake < sleep_entered {
        return Err("wake predates the pre-suspend handoff".to_string());
    }
    observer.request_deadline_clock_sample(operation_id)?;
    Ok(sleep_entered)
}

/// Final host-sleep lifecycle snapshot.
///
/// This must be taken only after the bounded host-sleep watcher has finished,
/// so the retained-basis post-wake sample is included even when the owner
/// reached terminal immediately after resume and the operator created `wake`
/// afterward.
fn host_sleep_events_snapshot(
    observer: &Option<OwnedProcessObservationHandle>,
) -> Option<Vec<OwnedProcessLifecycleEvent>> {
    observer.as_ref().map(|observer| observer.events())
}

fn wait_for_terminal_cleanup_authority(
    sentinel: &Sentinel,
    checkpoint_kind: &str,
) -> Result<(), String> {
    let terminal_ready = sentinel
        .mark("terminal-ready", "ready\n")
        .map_err(|error| format!("{checkpoint_kind} terminal checkpoint failed: {error}"))?;
    sentinel
        .wait_for_named_action_after("cleanup-ready", terminal_ready)
        .map(|_| ())
        .map_err(|error| format!("{checkpoint_kind} cleanup authority checkpoint failed: {error}"))
}

fn wait_for_root_cleanup_authority(sentinel: &Sentinel) -> Result<(), String> {
    wait_for_terminal_cleanup_authority(sentinel, "root")
}

fn wait_for_device_cleanup_authority(sentinel: &Sentinel) -> Result<(), String> {
    wait_for_terminal_cleanup_authority(sentinel, "device recovery")
}

#[derive(Clone, Debug)]
struct DeviceFacts {
    serial: String,
    manufacturer: String,
    model: String,
    android_version: String,
    api_level: i64,
    abi: String,
    build_fingerprint: String,
    root_shell: bool,
    root_version: Option<String>,
}

/// Run the one ignored physical qualification entry point.  A blocked gate or
/// failed expected-failure contract is returned as a test error; libtest must
/// not turn a printed blocker into a successful ignored test.
#[test]
#[ignore = "manual Phase 6D.6 physical interruption qualification; requires exact opt-ins, one prepared device, and an active operator"]
fn manual_phase_6d6_physical_interruption_qualification() -> Result<(), String> {
    let record = run_invocation().map_err(|blocker| {
        eprintln!("Phase 6D.6 blocked before qualification: {blocker}");
        blocker
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&record).expect("qualification record is JSON")
    );
    qualification_test_result(&record)
}

fn run_invocation() -> Result<Value, String> {
    let invocation = validate_invocation()?;
    let armed_at = invocation
        .sentinel
        .mark("armed", "phase-6d6\n")
        .map_err(|error| format!("sentinel gate failed: {error}"))?;

    let facts = match preflight_device(&invocation) {
        Ok(facts) => facts,
        Err(error) => {
            let _ = invocation.sentinel.cleanup();
            return Err(error);
        }
    };
    if let Err(error) = ensure_clean_fixture(&invocation, &facts) {
        let _ = invocation.sentinel.cleanup();
        return Err(error);
    }
    let mut storage = if invocation.scenario == Scenario::LowStorage {
        let result = invocation
            .sentinel
            .wait_for_action_after(armed_at)
            .and_then(|_| prepare_low_storage(&invocation, &facts));
        match result {
            Ok(observation) => Some(observation),
            Err(error) => {
                let cleanup = cleanup_fixture(&invocation, &facts, None, None);
                return Err(format!(
                    "{error}; low-storage preparation cleanup outcome={}",
                    cleanup.0
                ));
            }
        }
    } else {
        None
    };
    let low_storage_host_payload = if invocation.scenario == Scenario::LowStorage {
        match storage
            .as_ref()
            .map(|observation| observation.payload_kib)
            .ok_or_else(|| "low-storage preparation did not produce a payload size".to_string())
            .and_then(|payload_kib| create_low_storage_host_payload(&invocation, payload_kib))
        {
            Ok(path) => Some(path),
            Err(error) => {
                let cleanup = cleanup_fixture(&invocation, &facts, None, None);
                return Err(format!(
                    "{error}; low-storage host payload cleanup outcome={}",
                    cleanup.0
                ));
            }
        }
    } else {
        None
    };
    let active_stimulus = if invocation.scenario.supports_host_push_active_stimulus() {
        match prepare_active_stimulus(&invocation, &facts) {
            Ok(stimulus) => Some(stimulus),
            Err(error) => {
                let cleanup = cleanup_fixture(
                    &invocation,
                    &facts,
                    low_storage_host_payload.as_deref(),
                    None,
                );
                return Err(format!(
                    "{error}; active host-push preparation cleanup outcome={}",
                    cleanup.0
                ));
            }
        }
    } else {
        None
    };
    let timeout_stimulus = if invocation.scenario.requires_device_copy_stimulus() {
        match prepare_timeout_stimulus(&invocation, &facts) {
            Ok(stimulus) => Some(stimulus),
            Err(error) => {
                let cleanup = cleanup_fixture(
                    &invocation,
                    &facts,
                    low_storage_host_payload.as_deref(),
                    active_stimulus.as_ref(),
                );
                return Err(format!(
                    "{error}; timeout stimulus preparation cleanup outcome={}",
                    cleanup.0
                ));
            }
        }
    } else {
        None
    };
    let plan = reviewed_plan(
        &invocation,
        &facts,
        low_storage_host_payload.as_deref(),
        active_stimulus.as_ref(),
        timeout_stimulus.as_ref(),
    );
    let reviewed = run_reviewed_plan(
        &invocation,
        plan,
        &facts,
        active_stimulus.as_ref(),
        timeout_stimulus.as_ref(),
    );
    let ReviewedPlanObservation {
        result,
        action_time,
        checkpoint_error,
        slot_observation,
        active_process,
        active_cancellation,
        identity_capture,
        mut authorization_capture,
        terminal_detected_at,
        host_sleep_events,
        timeout,
    } = reviewed;

    let mut checkpoint_error = checkpoint_error;
    if invocation.scenario.is_root() {
        let root_cleanup_error = wait_for_root_cleanup_authority(&invocation.sentinel).err();
        if checkpoint_error.is_none() {
            checkpoint_error = root_cleanup_error;
        }
    }
    if invocation.scenario.requires_terminal_recovery() {
        let recovery_error = wait_for_device_cleanup_authority(&invocation.sentinel).err();
        if checkpoint_error.is_none() {
            checkpoint_error = recovery_error;
        }
    }
    if invocation.scenario == Scenario::DeviceUnauthorized {
        if let Ok(terminal_ready) = invocation.sentinel.marker_time("terminal-ready") {
            wait_until_later_canonical_second(terminal_ready);
        }
    }
    let sentinel = sentinel_evidence(&invocation);
    let cleanup_started_at = SystemTime::now();
    let cleanup = cleanup_fixture(
        &invocation,
        &facts,
        low_storage_host_payload.as_deref(),
        active_stimulus.as_ref(),
    );
    let cleanup_completed_at = SystemTime::now();
    if invocation.scenario == Scenario::DeviceUnauthorized {
        wait_until_later_canonical_second(cleanup_completed_at);
        let final_authorized = matches!(
            selected_serial_observation(&invocation.serial),
            Ok(SelectedSerialObservation::Attached(_))
        );
        let final_state_observed_at = final_authorized.then(SystemTime::now);
        if let Some(capture) = authorization_capture.as_mut() {
            capture.final_authorized = final_authorized;
            capture.final_state_observed_at = final_state_observed_at;
        }
    }
    if let Some(observation) = storage.as_mut() {
        let root = invocation.contract.destination_root.trim_end_matches('/');
        observation.final_free_kib = free_space_kib(&invocation.serial, root).ok();
        observation.reserve_removed = !fixture_path_exists(
            &invocation,
            &facts,
            &format!("{}/{}", invocation.run_scope, STORAGE_RESERVE_FILE),
        )
        .unwrap_or(true);
        observation.restored_recovery_reserve_kib = observation
            .final_free_kib
            .filter(|free| *free >= observation.initial_free_kib.saturating_sub(64 * 1024))
            .map(|_| observation.recovery_reserve_kib);
        observation.cleanup_verified = cleanup.0 == "succeeded"
            && observation.reserve_removed
            && observation.restored_recovery_reserve_kib.is_some();
    }
    let record = evidence_record(EvidenceInputs {
        invocation: &invocation,
        facts: &facts,
        result: result.as_ref(),
        action_time,
        checkpoint_error,
        cleanup,
        slot_observation,
        active_process,
        active_cancellation,
        identity_capture,
        authorization_capture,
        terminal_detected_at,
        host_sleep_events,
        timeout,
        storage: storage.as_ref(),
        host_payload: low_storage_host_payload.as_deref(),
        active_stimulus: active_stimulus.as_ref(),
        sentinel,
        cleanup_started_at,
        cleanup_completed_at,
    });
    let _evidence_path = write_evidence(&invocation, &record)?;
    Ok(record)
}

fn validate_invocation() -> Result<Invocation, String> {
    require_exact("EMUCHEF_RUN_REAL_ADB_TESTS", "1")?;
    require_exact(PHASE_OPT_IN, "1")?;
    let scenario = Scenario::parse(
        &env::var(SCENARIO_ENV).map_err(|_| format!("{SCENARIO_ENV} is required"))?,
    )?;
    let repetition = env::var(REPETITION_ENV)
        .map_err(|_| format!("{REPETITION_ENV} must be 1 or 2"))?
        .parse::<u8>()
        .map_err(|_| format!("{REPETITION_ENV} must be 1 or 2"))?;
    if !matches!(repetition, 1 | 2) {
        return Err(format!("{REPETITION_ENV} must be 1 or 2"));
    }
    let serial =
        env::var(SERIAL_ENV).map_err(|_| format!("{SERIAL_ENV} must select one exact device"))?;
    if serial.trim().is_empty()
        || serial.len() > MAX_SERIAL_BYTES
        || serial.contains(char::is_whitespace)
    {
        return Err(format!("{SERIAL_ENV} is not a valid exact serial"));
    }
    validate_package(
        FIXTURE_PACKAGE,
        optional_env(PACKAGE_ALLOWLIST_ENV).as_deref(),
    )?;
    let contract = load_contract()?;
    validate_owned_destination(
        &contract,
        &format!(
            "{}/phase6d6-probe",
            contract.destination_root.trim_end_matches('/')
        ),
        false,
    )?;
    if scenario.requires_root() {
        require_exact(ROOT_OPT_IN, "1")?;
        require_exact(ROOT_DESTRUCTIVE_OPT_IN, "1")?;
        require_exact(ROOT_PREFIX_ALLOWLIST_ENV, ROOT_PREFIX_ALLOWLIST)?;
    }
    if let Some(opt_in) = scenario.requires_destructive_opt_in() {
        require_exact(opt_in, "1")?;
    }
    let sentinel = Sentinel::from_environment()?;
    let run_scope = unique_run_scope(&contract, scenario, repetition, &serial)?;
    let run_digest = digest(&format!("phase6d6-run:{run_scope}"));
    let path_suffix = &run_digest[..16];
    Ok(Invocation {
        scenario,
        repetition,
        serial,
        sentinel,
        contract,
        run_scope,
        run_id: format!("physical-run-sha256:{run_digest}"),
        sentinel_id: format!(
            "sentinel-sha256:{}",
            digest(&format!("phase6d6-sentinel:{path_suffix}"))
        ),
        sentinel_nonce: format!(
            "nonce-sha256:{}",
            digest(&format!("phase6d6-nonce:{path_suffix}"))
        ),
        evidence_path: format!(
            "docs/testing/phase-6d6/evidence/{}-rep{repetition}-{path_suffix}.json",
            scenario.as_str()
        ),
        trace_path: format!(
            "docs/testing/phase-6d6/evidence/traces/{}-rep{repetition}-{path_suffix}.json",
            scenario.as_str()
        ),
        scenario_contract: scenario_contract(scenario),
    })
}

fn unique_run_scope(
    contract: &super::QualificationContract,
    scenario: Scenario,
    repetition: u8,
    serial: &str,
) -> Result<String, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| format!("{}-{}", duration.as_secs(), duration.subsec_nanos()))
        .unwrap_or_else(|_| "clock-unavailable".to_string());
    let scope_digest = digest(&format!("{serial}:{nonce}"));
    let root = if scenario.is_root() {
        ROOT_DATA_PREFIX.trim_end_matches('/')
    } else {
        contract.destination_root.trim_end_matches('/')
    };
    let scope = format!(
        "{root}/phase6d6-{}-rep{repetition}-{}",
        scenario.as_str(),
        &scope_digest[..16]
    );
    if scenario.is_root() {
        if scope.contains("..") || !scope.starts_with(ROOT_DATA_PREFIX) {
            return Err("root run scope escaped the committed prefix".to_string());
        }
        Ok(scope)
    } else {
        validate_owned_destination(contract, &scope, false).map(|_| scope)
    }
}

fn require_exact(name: &str, expected: &str) -> Result<(), String> {
    if env::var(name).ok().as_deref() != Some(expected) {
        return Err(format!("{name} must equal {expected}"));
    }
    Ok(())
}

const PRODUCTION_ROOT_PROBE_ARGS: [&str; 4] = ["shell", "su", "-c", "id"];

fn production_root_probe_granted(stdout: &str) -> bool {
    let normalized = stdout.trim().to_ascii_lowercase();
    normalized.starts_with("uid=0(") || normalized.starts_with("uid=0 ")
}

fn preflight_device(invocation: &Invocation) -> Result<DeviceFacts, String> {
    let inventory = adb_inventory()?;
    let inventory_rows = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.to_string(), fields.next()?.to_string()))
        })
        .collect::<Vec<_>>();
    if inventory_rows.len() != 1
        || inventory_rows[0].0 != invocation.serial
        || inventory_rows[0].1 != "device"
    {
        return Err("ADB inventory must contain only the selected online serial".to_string());
    }
    let online = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            (fields.next() == Some("device")).then_some(serial.to_string())
        })
        .collect::<Vec<_>>();
    if online != [invocation.serial.clone()] {
        return Err("ADB inventory must contain exactly the selected online serial".to_string());
    }
    let manufacturer = query_property(&invocation.serial, "ro.product.manufacturer")?;
    let model = query_property(&invocation.serial, "ro.product.model")?;
    let android_version = query_property(&invocation.serial, "ro.build.version.release")?;
    let api_level = query_property(&invocation.serial, "ro.build.version.sdk")?
        .parse::<i64>()
        .map_err(|_| "device API level is not a number".to_string())?;
    let abi = query_property(&invocation.serial, "ro.product.cpu.abilist")?;
    let build_fingerprint = query_property(&invocation.serial, "ro.build.fingerprint")?;
    let adb_shell_uid = adb_query(&invocation.serial, &["shell", "id", "-u"])?;
    let adb_shell_is_root = adb_shell_uid.trim() == "0";
    let root_shell = if invocation.scenario.is_root() {
        let root_identity = adb_query(&invocation.serial, &PRODUCTION_ROOT_PROBE_ARGS)?;
        if !production_root_probe_granted(&root_identity) {
            return Err("root qualification requires granted su authority".to_string());
        }
        true
    } else {
        if adb_shell_is_root {
            return Err("non-root qualification refuses a shell with uid 0".to_string());
        }
        false
    };
    let root_version = if root_shell {
        adb_query(&invocation.serial, &["shell", "su", "--version"])
            .ok()
            .map(|value| sanitize_fact(value.lines().next().unwrap_or("unreported")))
    } else {
        None
    };
    Ok(DeviceFacts {
        serial: invocation.serial.clone(),
        manufacturer: sanitize_fact(&manufacturer),
        model: sanitize_fact(&model),
        android_version: sanitize_fact(&android_version),
        api_level,
        abi: sanitize_fact(&abi),
        build_fingerprint: sanitize_fact(&build_fingerprint),
        root_shell,
        root_version,
    })
}

fn query_property(serial: &str, property: &str) -> Result<String, String> {
    let value = adb_query(serial, &["shell", "getprop", property])?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("required device property {property} was empty"));
    }
    Ok(value)
}

fn ensure_clean_fixture(invocation: &Invocation, facts: &DeviceFacts) -> Result<(), String> {
    let (first, second) = run_scope_paths(invocation);
    for scope in run_scope_roots(invocation) {
        if fixture_path_exists(invocation, facts, &scope)? {
            return Err("the unique fixture run scope already exists".to_string());
        }
    }
    for path in [first, second] {
        if fixture_path_exists(invocation, facts, &path)? {
            return Err("fixture destination was not clean before qualification".to_string());
        }
    }
    if invocation.scenario == Scenario::LowStorage {
        let filler = format!("{}/{STORAGE_FILL_FILE}", invocation.run_scope);
        let reserve = format!("{}/{STORAGE_RESERVE_FILE}", invocation.run_scope);
        if fixture_path_exists(invocation, facts, &filler)?
            || fixture_path_exists(invocation, facts, &reserve)?
        {
            return Err("low-storage run-scoped filler or reserve was not clean".to_string());
        }
    }
    Ok(())
}

fn fixture_path_exists(
    invocation: &Invocation,
    facts: &DeviceFacts,
    path: &str,
) -> Result<bool, String> {
    if invocation.scenario.is_root() {
        adb_root_path_exists(&facts.serial, path)
    } else {
        adb_path_exists(&facts.serial, path)
    }
}

fn adb_root_path_exists(serial: &str, path: &str) -> Result<bool, String> {
    let output = Command::new("adb")
        .args(["-s", serial, "shell", "su", "-c", "test", "-e", path])
        .output()
        .map_err(|_| "ADB root-path query is unavailable".to_string())?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty() {
        return Ok(false);
    }
    Err("ADB root-path query failed".to_string())
}

fn reviewed_plan(
    invocation: &Invocation,
    facts: &DeviceFacts,
    low_storage_host_payload: Option<&Path>,
    active_stimulus: Option<&ActiveStimulus>,
    timeout_stimulus: Option<&TimeoutStimulus>,
) -> ExecutionPlan {
    if timeout_stimulus.is_some() && !invocation.scenario.requires_device_copy_stimulus() {
        panic!("pseudo-device stimulus is only valid for timeout or host-sleep scenarios");
    }
    let root = invocation.contract.destination_root.trim_end_matches('/');
    let mut steps = Vec::new();
    let (run_scope_first_destination, second_destination) = run_scope_paths(invocation);
    let first_destination = timeout_stimulus
        .map(|stimulus| stimulus.device_destination_path.clone())
        .unwrap_or(run_scope_first_destination);
    let first_source = timeout_stimulus
        .map(|stimulus| PathBuf::from(&stimulus.device_source_path))
        .or_else(|| active_stimulus.map(|stimulus| stimulus.host_source_path.clone()))
        .or_else(|| low_storage_host_payload.map(Path::to_path_buf))
        .unwrap_or_else(|| fixture_root().join("corpus/source/single-file.txt"));
    let second_source = fixture_root().join("corpus/source/nested/alpha/one.txt");
    if !invocation.scenario.is_root() {
        let non_root_destination = format!("{root}/phase6d6-first.txt");
        validate_owned_destination(&invocation.contract, &non_root_destination, false)
            .expect("reviewed destination must remain fixture-owned");
        if invocation.scenario == Scenario::LowStorage {
            let fill_destination = format!("{}/{}", invocation.run_scope, STORAGE_FILL_FILE);
            validate_owned_destination(&invocation.contract, &fill_destination, false)
                .expect("low-storage filler must remain fixture-owned");
        }
    }
    if invocation.scenario.is_root() {
        // Root paths are checked against the existing root contract before the
        // plan is built.  This is an assertion for the fixed contract, not a
        // new authority surface.
        assert!(first_destination.starts_with(ROOT_DATA_PREFIX));
        assert!(second_destination.starts_with(ROOT_USER_PREFIX));
    }
    let mut first = copy_step(
        &format!("phase6d6/{}/first", invocation.scenario.as_str()),
        "file_path",
        first_source,
        &first_destination,
    );
    if let Some(active_stimulus) = active_stimulus {
        assert_eq!(active_stimulus.device_destination_path, first_destination);
        assert!(active_stimulus
            .host_source_path
            .starts_with(active_stimulus.host_workspace.path()));
    }
    if let Some(timeout_stimulus) = timeout_stimulus {
        assert_eq!(timeout_stimulus.device_destination_path, first_destination);
        assert!(is_timeout_source_path(&timeout_stimulus.device_source_path));
        assert!(is_timeout_destination_path(
            &timeout_stimulus.device_destination_path
        ));
        match first.params.get_mut("source") {
            Some(ExecutionParamValue::Literal { value }) => {
                value["location"] =
                    Value::String(reviewed_plan_source_location(invocation.scenario).to_string());
            }
            _ => panic!("reviewed timeout copy source must remain a literal runtime value"),
        }
        // The ordinary file-copy path removes the destination first under
        // "replace"; "merge" preserves the existing /dev/null node so the
        // exact production `cp /dev/zero /dev/null` operation remains the only
        // DeviceCopy and never unlinks or recreates either pseudo-device.
        if let Some(ExecutionParamValue::Literal { value }) = first.params.get_mut("copy_policy") {
            *value = Value::String("merge".to_string());
        }
    }
    first.verify = vec![condition(
        "path_exists",
        json!({ "path": first_destination }),
    )];
    let mut second = copy_step(
        &format!("phase6d6/{}/second", invocation.scenario.as_str()),
        "file_path",
        second_source,
        &second_destination,
    );
    second.dependencies = vec![first.id.clone()];
    second.verify = vec![condition(
        "path_exists",
        json!({ "path": second_destination }),
    )];
    steps.push(first);
    steps.push(second);
    ExecutionPlan {
        id: format!(
            "plan.phase6d6.{}.{}",
            invocation.scenario.as_str(),
            invocation.repetition
        ),
        source: ExecutionPlanSource {
            device_profile_ref: "fixture.phase6d6".to_string(),
            device_plan_ref: "fixture.phase6d6".to_string(),
            selected_recipe_refs: vec!["fixture.phase6d6".to_string()],
            expanded_recipe_refs: vec!["fixture.phase6d6".to_string()],
            catalog: None,
        },
        recipes: Vec::new(),
        target_device: Some(TargetDeviceBinding {
            serial: facts.serial.clone(),
            manufacturer: Some(facts.manufacturer.clone()),
            model: Some(facts.model.clone()),
            android_api_level: Some(facts.api_level),
        }),
        device_context: DeviceContext {
            manufacturer: facts.manufacturer.clone(),
            model: facts.model.clone(),
            android_version: facts.android_version.parse().unwrap_or(0),
            android_api_level: Some(facts.api_level),
            device_tags: vec!["phase6d6".to_string()],
        },
        runtime_capabilities: RuntimeCapabilities {
            adb_available: true,
            apk_install: false,
            shared_storage_write: !invocation.scenario.is_root(),
            app_launch: false,
            shell_command: true,
            package_remove_for_user: false,
            root_shell: facts.root_shell,
            app_data_write: facts.root_shell,
        },
        inputs: Vec::new(),
        artifacts: Vec::new(),
        steps,
        schema_version: 1,
        kind: "execution_plan",
    }
}

struct ReviewedPlanObservation {
    result: Result<ExecutionRunResult, String>,
    action_time: Option<SystemTime>,
    checkpoint_error: Option<String>,
    slot_observation: ExecutionSlotObservation,
    active_process: Option<Value>,
    active_cancellation: Option<Value>,
    identity_capture: Option<IdentityTransitionCapture>,
    authorization_capture: Option<AuthorizationTransitionCapture>,
    terminal_detected_at: Option<SystemTime>,
    host_sleep_events: Option<Vec<OwnedProcessLifecycleEvent>>,
    timeout: Option<Value>,
}

fn is_boundary_checkpoint_event(
    scenario: Scenario,
    first_step_id: &str,
    event: &ExecutionProgressEvent,
) -> bool {
    scenario.is_boundary_checkpoint()
        && event.step_id == first_step_id
        && event.phase == ProgressPhase::Finished
}

fn checkpoint_failure_message(scenario: Scenario) -> &'static str {
    if matches!(
        scenario,
        Scenario::HostSleepBeforeDeadline | Scenario::HostSleepAfterDeadline
    ) {
        "host-sleep pre-suspend handshake or deadline-clock observation was missing or invalid"
    } else if scenario == Scenario::OperationTimeout {
        "timeout liveness/deadline observation was missing or invalid"
    } else {
        "bounded operator checkpoint was missing or aborted"
    }
}

fn wait_until_later_canonical_second(after: SystemTime) {
    let after_second = canonical_unix_seconds(after).unwrap_or(0);
    while canonical_unix_seconds(SystemTime::now()).unwrap_or(0) <= after_second {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn timeout_liveness_checkpoint(
    observer: OwnedProcessObservationHandle,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let started = Instant::now();
    let operation_id = loop {
        let events = observer.events();
        if let Some((operation_id, _)) = events.iter().find_map(|event| match event {
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at,
            } => Some((*operation_id, *at)),
            _ => None,
        }) {
            break operation_id;
        }
        if events.iter().any(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::Terminal {
                    operation: ProcessOperation::DeviceCopy,
                    ..
                }
            )
        }) {
            return Err(
                "timeout child reached terminal cleanup before the liveness sample".to_string(),
            );
        }
        if stop.load(Ordering::Acquire) || started.elapsed() >= SENTINEL_TIMEOUT {
            return Err(
                "timeout mutation was not observed before the bounded deadline".to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mutation_started = observer
        .events()
        .iter()
        .find_map(|event| match event {
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
            } if *observed == operation_id => Some(*at),
            _ => None,
        })
        .ok_or_else(|| "matching timeout mutation timestamp was not retained".to_string())?;
    let sample_target = mutation_started
        .checked_add(TIMEOUT_LIVENESS_SAMPLE_OFFSET)
        .ok_or_else(|| "timeout liveness sample target overflowed".to_string())?;
    while SystemTime::now() < sample_target {
        let events = observer.events();
        if events.iter().any(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineReached {
                    operation_id: observed,
                    operation: ProcessOperation::DeviceCopy,
                    ..
                } if *observed == operation_id
            ) || matches!(
                event,
                OwnedProcessLifecycleEvent::Terminal {
                    operation_id: observed,
                    operation: ProcessOperation::DeviceCopy,
                    ..
                } if *observed == operation_id
            )
        }) {
            return Err(
                "timeout child reached its deadline before the 12-second liveness sample"
                    .to_string(),
            );
        }
        if stop.load(Ordering::Acquire) {
            return Err(
                "timeout qualification stopped before the 12-second liveness sample".to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    observer.request_liveness_sample(operation_id)?;
    loop {
        let events = observer.events();
        if let Some(sample) = events.iter().find_map(|event| match event {
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: observed,
                operation: ProcessOperation::DeviceCopy,
                at,
                alive,
                terminal_reported,
            } if *observed == operation_id => Some((*at, *alive, *terminal_reported)),
            _ => None,
        }) {
            if sample.1 == Some(true) && !sample.2 {
                return Ok(());
            }
            return Err("timeout child was not alive immediately before the deadline".to_string());
        }
        if events.iter().any(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineReached {
                    operation_id: observed,
                    operation: ProcessOperation::DeviceCopy,
                    ..
                } if *observed == operation_id
            ) || matches!(
                event,
                OwnedProcessLifecycleEvent::Terminal {
                    operation_id: observed,
                    operation: ProcessOperation::DeviceCopy,
                    ..
                } if *observed == operation_id
            )
        }) {
            return Err(
                "timeout child reached its deadline before the liveness sample was recorded"
                    .to_string(),
            );
        }
        if stop.load(Ordering::Acquire) {
            return Err(
                "timeout qualification stopped before the liveness sample was recorded".to_string(),
            );
        }
        if started.elapsed() >= SENTINEL_TIMEOUT {
            return Err("timeout liveness sample was not recorded before the deadline".to_string());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run_reviewed_plan(
    invocation: &Invocation,
    plan: ExecutionPlan,
    facts: &DeviceFacts,
    active_stimulus: Option<&ActiveStimulus>,
    timeout_stimulus: Option<&TimeoutStimulus>,
) -> ReviewedPlanObservation {
    let action_seen = Arc::new(AtomicBool::new(false));
    let checkpoint_failed = Arc::new(AtomicBool::new(false));
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let action_time = Arc::new(std::sync::Mutex::new(None));
    let in_flight = Arc::new(AtomicBool::new(false));
    let active_capture = Arc::new(std::sync::Mutex::new(ActiveCancellationCapture::default()));
    let scenario = invocation.scenario;
    let identity_capture = matches!(
        scenario,
        Scenario::IdentityStability | Scenario::IdentityReplacement
    )
    .then(|| {
        Arc::new(std::sync::Mutex::new(IdentityTransitionCapture {
            never_simultaneous: true,
            ..IdentityTransitionCapture::default()
        }))
    });
    let authorization_capture = (scenario == Scenario::DeviceUnauthorized).then(|| {
        let initial_authorized = matches!(
            selected_serial_observation(&facts.serial),
            Ok(SelectedSerialObservation::Attached(_))
        );
        let initial_observed_at = initial_authorized.then(SystemTime::now);
        Arc::new(std::sync::Mutex::new(AuthorizationTransitionCapture {
            initial_authorized,
            initial_observed_at,
            ..AuthorizationTransitionCapture::default()
        }))
    });
    if let Some(capture) = authorization_capture.as_ref() {
        let initial = capture
            .lock()
            .ok()
            .and_then(|value| value.initial_observed_at);
        if let Some(initial) = initial {
            wait_until_later_canonical_second(initial);
        }
    }
    let process_observer = scenario
        .requires_process_observer()
        .then(OwnedProcessObservationHandle::default);
    if scenario == Scenario::OperationTimeout && timeout_stimulus.is_none() {
        checkpoint_failed.store(true, Ordering::Release);
    }
    let transition_stop = Arc::new(AtomicBool::new(false));
    let sentinel = invocation.sentinel.clone();
    let callback_sentinel = sentinel.clone();
    let callback_action_time = Arc::clone(&action_time);
    let callback_action_seen = Arc::clone(&action_seen);
    let callback_checkpoint_failed = Arc::clone(&checkpoint_failed);
    let callback_cancel_requested = Arc::clone(&cancel_requested);
    let callback_active_capture = Arc::clone(&active_capture);
    let observer_sentinel = sentinel.clone();
    let observer_in_flight = Arc::clone(&in_flight);
    let observer_capture = Arc::clone(&active_capture);
    let first_step_id = plan
        .steps
        .first()
        .map(|step| step.id.clone())
        .expect("reviewed qualification plan has a first atomic step");
    let boundary_step_id = first_step_id.clone();
    let identity_observer = identity_capture.as_ref().map(|capture| {
        spawn_identity_transition_observer(
            facts.serial.clone(),
            facts.build_fingerprint.clone(),
            Arc::clone(capture),
            Arc::clone(&transition_stop),
        )
    });
    let authorization_observer = authorization_capture.as_ref().map(|capture| {
        spawn_authorization_transition_observer(
            facts.serial.clone(),
            sentinel.clone(),
            Arc::clone(capture),
            Arc::clone(&transition_stop),
        )
    });
    let sandbox = tempfile::tempdir().expect("qualification sandbox should be created");
    let device = match process_observer.as_ref() {
        Some(observer) => RealAdbDevice::new_with_process_observer(
            "adb",
            Some(facts.serial.clone()),
            observer.clone(),
        ),
        None => RealAdbDevice::new("adb", Some(facts.serial.clone())),
    };
    let mut read_only_roots = vec![fixture_root()];
    if let Some(stimulus) = active_stimulus {
        read_only_roots.push(stimulus.host_workspace.path().to_path_buf());
    }
    let runner = ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        sandbox.path().join("runtime"),
        sandbox.path().join("cache"),
        sandbox.path().join("device"),
        read_only_roots,
        false,
    ));
    let mut runner = runner;
    let watcher = if scenario.is_active_checkpoint() {
        let watcher_sentinel = sentinel.clone();
        let watcher_in_flight = Arc::clone(&in_flight);
        let watcher_capture = Arc::clone(&active_capture);
        let watcher_action_seen = Arc::clone(&action_seen);
        let watcher_action_time = Arc::clone(&action_time);
        let watcher_checkpoint_failed = Arc::clone(&checkpoint_failed);
        let watcher_cancel_requested = Arc::clone(&cancel_requested);
        let watcher_process_observer = process_observer.clone();
        Some(std::thread::spawn(move || {
            if scenario.supports_host_push_active_stimulus() {
                let result = (|| -> Result<(), String> {
                    let observer = watcher_process_observer.ok_or_else(|| {
                        "active scenario did not install the exact-child observer".to_string()
                    })?;
                    let operation_id = observer
                        .wait_for_mutation(active_process_operation(scenario), SENTINEL_TIMEOUT)?;
                    let operation_started = watcher_sentinel
                        .marker_time("operation-started")
                        .map_err(|_| {
                            "active operation-started marker was unavailable".to_string()
                        })?;
                    wait_until_later_canonical_second(operation_started);
                    observer.request_liveness_sample(operation_id)?;
                    let sample = observer.wait_for_liveness(operation_id, SENTINEL_TIMEOUT)?;
                    if sample.alive != Some(true) || sample.terminal_reported {
                        return Err(
                            "exact target child was not alive before the operator action"
                                .to_string(),
                        );
                    }
                    let active_ready = watcher_sentinel.mark("active-ready", "ready\n")?;
                    let timestamp = watcher_sentinel.wait_for_action_after(active_ready)?;
                    let freshness = timestamp.duration_since(sample.at).map_err(|_| {
                        "operator action preceded the exact-child liveness sample".to_string()
                    })?;
                    if freshness > ACTIVE_SAMPLE_FRESHNESS {
                        return Err(
                            "operator action exceeded the exact-child liveness freshness window"
                                .to_string(),
                        );
                    }
                    watcher_action_seen.store(true, Ordering::Release);
                    if let Ok(mut slot) = watcher_action_time.lock() {
                        *slot = Some(timestamp);
                    }
                    if let Ok(mut capture) = watcher_capture.lock() {
                        capture.requested_at = Some(timestamp);
                    }
                    if scenario == Scenario::CancellationActive {
                        watcher_cancel_requested.store(true, Ordering::Release);
                    }
                    Ok(())
                })();
                if result.is_err() {
                    watcher_checkpoint_failed.store(true, Ordering::Release);
                    if scenario == Scenario::CancellationActive {
                        watcher_cancel_requested.store(true, Ordering::Release);
                    }
                }
                return;
            }
            if matches!(
                scenario,
                Scenario::HostSleepBeforeDeadline | Scenario::HostSleepAfterDeadline
            ) {
                let result = (|| -> Result<SystemTime, String> {
                    let observer = watcher_process_observer.ok_or_else(|| {
                        "host-sleep scenario did not install the exact-child observer".to_string()
                    })?;
                    run_host_sleep_watcher(&observer, &watcher_sentinel)
                })();
                match result {
                    Ok(action_time) => {
                        watcher_action_seen.store(true, Ordering::Release);
                        if let Ok(mut slot) = watcher_action_time.lock() {
                            *slot = Some(action_time);
                        }
                        if let Ok(mut capture) = watcher_capture.lock() {
                            capture.requested_at = Some(action_time);
                        }
                    }
                    Err(_) => {
                        watcher_checkpoint_failed.store(true, Ordering::Release);
                    }
                }
                return;
            }
            let started = Instant::now();
            while !watcher_in_flight.load(Ordering::Acquire) && started.elapsed() < SENTINEL_TIMEOUT
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut action_observed = false;
            while watcher_in_flight.load(Ordering::Acquire) && started.elapsed() < SENTINEL_TIMEOUT
            {
                // Host sleep uses its own pre-suspend handshake watcher above;
                // this branch handles the ordinary in-flight interruption
                // cases through the explicit operator-action marker.
                let action = watcher_sentinel.action_now();
                match action {
                    Ok(Some(timestamp)) => {
                        let operation_started =
                            watcher_sentinel.marker_time("operation-started").ok();
                        if operation_started.is_none() || timestamp < operation_started.unwrap() {
                            watcher_checkpoint_failed.store(true, Ordering::Release);
                            watcher_cancel_requested.store(true, Ordering::Release);
                            return;
                        }
                        action_observed = true;
                        watcher_action_seen.store(true, Ordering::Release);
                        if let Ok(mut slot) = watcher_action_time.lock() {
                            *slot = Some(timestamp);
                        }
                        if let Ok(mut capture) = watcher_capture.lock() {
                            capture.requested_at = Some(timestamp);
                        }
                        if scenario == Scenario::CancellationActive {
                            watcher_cancel_requested.store(true, Ordering::Release);
                        }
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => {
                        watcher_checkpoint_failed.store(true, Ordering::Release);
                        watcher_cancel_requested.store(true, Ordering::Release);
                        return;
                    }
                }
            }
            if !action_observed {
                watcher_checkpoint_failed.store(true, Ordering::Release);
                if scenario == Scenario::CancellationActive {
                    watcher_cancel_requested.store(true, Ordering::Release);
                }
            }
        }))
    } else {
        None
    };
    let run_id = format!("run-scope-sha256:{}", digest(&invocation.run_scope));
    let session_manager = ExecutionSessionManager::default();
    let observed_run = session_manager.test_run_under_observed_slot(&run_id, || {
        let _deadline_guard = match scenario {
            Scenario::OperationTimeout => Some(arm_test_process_deadline(
                ProcessOperation::DeviceCopy,
                TIMEOUT_QUALIFICATION_DEADLINE,
            )),
            Scenario::HostSleepBeforeDeadline | Scenario::HostSleepAfterDeadline => {
                Some(arm_test_process_deadline(
                    ProcessOperation::DeviceCopy,
                    HOST_SLEEP_QUALIFICATION_DEADLINE,
                ))
            }
            _ => None,
        };
        let timeout_stop = Arc::new(AtomicBool::new(false));
        let timeout_watcher = if scenario == Scenario::OperationTimeout {
            let observer = process_observer
                .clone()
                .expect("operation-timeout qualification must install the exact-child observer");
            let watcher_stop = Arc::clone(&timeout_stop);
            Some(std::thread::spawn(move || {
                timeout_liveness_checkpoint(observer, watcher_stop)
            }))
        } else {
            None
        };
        let result = runner.run_with_progress_and_cancel_observed(
            &plan,
            move |event: ExecutionProgressEvent| {
                if is_boundary_checkpoint_event(scenario, &boundary_step_id, &event) {
                    let boundary_ready = callback_sentinel.mark("boundary-ready", "ready\n");
                    match boundary_ready
                        .and_then(|timestamp| callback_sentinel.wait_for_action_after(timestamp))
                    {
                        Ok(timestamp) => {
                            callback_action_seen.store(true, Ordering::SeqCst);
                            if let Ok(mut slot) = callback_action_time.lock() {
                                *slot = Some(timestamp);
                            }
                            if let Ok(mut capture) = callback_active_capture.lock() {
                                capture.requested_at = Some(timestamp);
                            }
                            if scenario == Scenario::CancellationBoundary {
                                callback_cancel_requested.store(true, Ordering::SeqCst);
                            }
                        }
                        Err(_) => {
                            callback_checkpoint_failed.store(true, Ordering::SeqCst);
                            callback_cancel_requested.store(true, Ordering::SeqCst);
                        }
                    }
                }
            },
            move || cancel_requested.load(Ordering::SeqCst),
            move |lifecycle| match lifecycle {
                OperationLifecycle::Started { step_id } if step_id == first_step_id => {
                    observer_in_flight.store(true, Ordering::Release);
                    let started_at = observer_sentinel
                        .mark("operation-started", "started\n")
                        .ok();
                    if let Ok(mut capture) = observer_capture.lock() {
                        capture.in_flight_observed_at = started_at;
                    }
                }
                OperationLifecycle::Finished { step_id } if step_id == first_step_id => {
                    let finished_at = observer_sentinel
                        .mark("operation-finished", "finished\n")
                        .ok();
                    if let Ok(mut capture) = observer_capture.lock() {
                        capture.operation_finished_at = finished_at;
                    }
                    observer_in_flight.store(false, Ordering::Release);
                }
                _ => {}
            },
        );
        if let Some(watcher) = timeout_watcher {
            timeout_stop.store(true, Ordering::Release);
            match watcher.join() {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    checkpoint_failed.store(true, Ordering::Release);
                }
            }
        }
        result
    });
    let (result, session_slot_observation) = match observed_run {
        Ok(value) => value,
        Err(error) => {
            let timeout =
                (scenario == Scenario::OperationTimeout).then(|| timeout_metadata(Err(&error)));
            return ReviewedPlanObservation {
                result: Err(error),
                action_time: None,
                checkpoint_error: Some(
                    "production execution slot could not be acquired".to_string(),
                ),
                slot_observation: ExecutionSlotObservation {
                    run_id: run_id.clone(),
                    execution_id: run_id,
                    acquired: false,
                    released: false,
                    acquired_at_unix: 0,
                    terminal_cleanup_at_unix: None,
                    released_at_unix: None,
                },
                active_cancellation: None,
                active_process: None,
                identity_capture: None,
                authorization_capture: None,
                terminal_detected_at: None,
                host_sleep_events: None,
                timeout,
            };
        }
    };
    let terminal_detected_at = Some(SystemTime::now());
    if scenario == Scenario::DeviceUnauthorized {
        let grace_started = Instant::now();
        while grace_started.elapsed() < Duration::from_secs(3) {
            let observed = authorization_capture.as_ref().is_some_and(|capture| {
                capture
                    .lock()
                    .ok()
                    .and_then(|value| value.observed_at)
                    .is_some()
            });
            if observed {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    transition_stop.store(true, Ordering::Release);
    if let Some(observer) = identity_observer {
        let _ = observer.join();
    }
    if let Some(observer) = authorization_observer {
        let _ = observer.join();
    }
    if let Some(watcher) = watcher {
        if watcher.join().is_err() {
            checkpoint_failed.store(true, Ordering::Release);
        }
    }
    // The final host-sleep lifecycle snapshot must happen only after the
    // bounded host-sleep watcher has finished: the watcher publishes the
    // retained-basis post-wake sample after observing the wake marker, which
    // may be after the owner reached terminal immediately following resume.
    let host_sleep_events = host_sleep_events_snapshot(&process_observer);
    let slot_observation = session_slot_observation
        .lock()
        .map(|value| value.clone())
        .unwrap_or(ExecutionSlotObservation {
            run_id: run_id.clone(),
            execution_id: run_id.clone(),
            acquired: false,
            released: false,
            acquired_at_unix: 0,
            terminal_cleanup_at_unix: None,
            released_at_unix: None,
        });
    let action_time = action_time.lock().ok().and_then(|slot| *slot);
    let active_process = process_observer.as_ref().and_then(|observer| {
        if scenario == Scenario::OperationTimeout {
            timeout_process_evidence(&observer.events(), &run_id)
        } else {
            active_process_evidence(
                &observer.events(),
                action_time,
                &run_id,
                active_process_operation(scenario),
            )
        }
    });
    let checkpoint_error = if checkpoint_failed.load(Ordering::SeqCst) {
        Some(checkpoint_failure_message(scenario).to_string())
    } else if invocation.scenario.is_active_checkpoint() && !action_seen.load(Ordering::SeqCst) {
        Some("operator action was not observed before the first operation completed".to_string())
    } else {
        None
    };
    let active_cancellation = if scenario == Scenario::CancellationActive
        || scenario == Scenario::CancellationBoundary
    {
        let capture = active_capture.lock().ok().map(|value| value.clone());
        let later_work_not_attempted =
            result.steps.iter().skip(1).all(|step| {
                !matches!(step.status, StepRunStatus::Executed | StepRunStatus::Failed)
            });
        capture.map(|capture| {
            json!({
                "requestPhase": if scenario == Scenario::CancellationActive { "in_flight" } else { "safe_boundary" },
                "inFlightObservedAt": system_time_value(capture.in_flight_observed_at),
                "requestedAt": system_time_value(capture.requested_at),
                "operationFinishedAt": system_time_value(capture.operation_finished_at),
                "requestBeforeFinished": capture.requested_at.zip(capture.operation_finished_at).is_some_and(|(requested, finished)| requested < finished),
                "laterWorkNotAttempted": later_work_not_attempted,
                "operatorEvidence": "bounded sentinel action observed at the production runner lifecycle checkpoint",
            })
        })
    } else {
        None
    };
    let identity_capture =
        identity_capture.and_then(|capture| capture.lock().ok().map(|value| value.clone()));
    let authorization_capture =
        authorization_capture.and_then(|capture| capture.lock().ok().map(|value| value.clone()));
    let timeout = (scenario == Scenario::OperationTimeout).then(|| timeout_metadata(Ok(&result)));
    ReviewedPlanObservation {
        result: Ok(result),
        action_time,
        checkpoint_error,
        slot_observation,
        active_process,
        active_cancellation,
        identity_capture,
        authorization_capture,
        terminal_detected_at,
        host_sleep_events,
        timeout,
    }
}

#[derive(Clone, Debug, Default)]
struct ActiveCancellationCapture {
    in_flight_observed_at: Option<SystemTime>,
    requested_at: Option<SystemTime>,
    operation_finished_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Default)]
struct IdentityTransitionCapture {
    original_attached: bool,
    original_disconnected_at: Option<SystemTime>,
    serial_absent_from: Option<SystemTime>,
    serial_absent_until: Option<SystemTime>,
    replacement_attached_at: Option<SystemTime>,
    replacement_fingerprint: Option<String>,
    never_simultaneous: bool,
}

#[derive(Clone, Debug, Default)]
struct AuthorizationTransitionCapture {
    initial_authorized: bool,
    initial_observed_at: Option<SystemTime>,
    revocation_checkpoint_at: Option<SystemTime>,
    original_disconnected_at: Option<SystemTime>,
    serial_absent_from: Option<SystemTime>,
    serial_absent_until: Option<SystemTime>,
    reconnected_at: Option<SystemTime>,
    observed_at: Option<SystemTime>,
    final_authorized: bool,
    final_state_observed_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SelectedSerialObservation {
    Absent,
    Attached(String),
    Unauthorized,
    Other,
}

/// Poll the host ADB inventory without treating an ADB command error as proof
/// that the selected serial was absent.  Identity qualification requires a
/// successful inventory sample with no selected row, followed by a successful
/// sample for the replacement.
fn selected_serial_observation(serial: &str) -> Result<SelectedSerialObservation, String> {
    let inventory = adb_inventory()?;
    let rows = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.to_string(), fields.next()?.to_string()))
        })
        .filter(|(candidate, _)| candidate == serial)
        .collect::<Vec<_>>();
    if rows.len() > 1 {
        return Ok(SelectedSerialObservation::Other);
    }
    let Some((_, state)) = rows.into_iter().next() else {
        return Ok(SelectedSerialObservation::Absent);
    };
    match state.as_str() {
        "device" => query_property(serial, "ro.build.fingerprint")
            .map(|fingerprint| SelectedSerialObservation::Attached(sanitize_fact(&fingerprint))),
        "unauthorized" => Ok(SelectedSerialObservation::Unauthorized),
        _ => Ok(SelectedSerialObservation::Other),
    }
}

fn spawn_identity_transition_observer(
    serial: String,
    original_fingerprint: String,
    capture: Arc<std::sync::Mutex<IdentityTransitionCapture>>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut absence_started = None;
        while !stop.load(Ordering::Acquire) {
            if let Ok(observation) = selected_serial_observation(&serial) {
                let now = SystemTime::now();
                match observation {
                    SelectedSerialObservation::Attached(fingerprint) => {
                        if absence_started.is_some() {
                            if let Ok(mut value) = capture.lock() {
                                value.serial_absent_until = Some(now);
                                value.replacement_attached_at = Some(now);
                                value.replacement_fingerprint = Some(fingerprint);
                                value.never_simultaneous = true;
                            }
                            break;
                        }
                        if let Ok(mut value) = capture.lock() {
                            if !value.original_attached && fingerprint == original_fingerprint {
                                value.original_attached = true;
                                value.never_simultaneous = true;
                            } else if value.original_attached && fingerprint != original_fingerprint
                            {
                                // A different fingerprint without a confirmed
                                // serial-absent interval is simultaneous/ambiguous
                                // evidence and must never qualify replacement.
                                value.never_simultaneous = false;
                            }
                        }
                    }
                    SelectedSerialObservation::Absent => {
                        if capture
                            .lock()
                            .map(|value| value.original_attached)
                            .unwrap_or(false)
                            && absence_started.is_none()
                        {
                            absence_started = Some(now);
                            if let Ok(mut value) = capture.lock() {
                                value.original_disconnected_at = Some(now);
                                value.serial_absent_from = Some(now);
                            }
                        }
                    }
                    SelectedSerialObservation::Unauthorized | SelectedSerialObservation::Other => {}
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    })
}

fn spawn_authorization_transition_observer(
    serial: String,
    sentinel: Sentinel,
    capture: Arc<std::sync::Mutex<AuthorizationTransitionCapture>>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let started = Instant::now();
        let revocation_checkpoint = loop {
            if stop.load(Ordering::Acquire) || started.elapsed() >= SENTINEL_TIMEOUT {
                return;
            }
            match sentinel.named_action_now("authorization-revoked") {
                Ok(Some(checkpoint)) => break checkpoint,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return,
            }
        };
        if let Ok(mut value) = capture.lock() {
            value.revocation_checkpoint_at = Some(revocation_checkpoint);
        }

        let mut absence_started = None;
        while !stop.load(Ordering::Acquire) {
            if let Ok(observation) = selected_serial_observation(&serial) {
                let now = SystemTime::now();
                match observation {
                    SelectedSerialObservation::Absent if absence_started.is_none() => {
                        absence_started = Some(now);
                        if let Ok(mut value) = capture.lock() {
                            value.original_disconnected_at = Some(now);
                            value.serial_absent_from = Some(now);
                        }
                    }
                    SelectedSerialObservation::Unauthorized if absence_started.is_some() => {
                        if let Ok(mut value) = capture.lock() {
                            value.serial_absent_until = Some(now);
                            value.reconnected_at = Some(now);
                            value.observed_at = Some(now);
                        }
                        let _ = sentinel.mark("unauthorized-observed", "ready\n");
                        break;
                    }
                    SelectedSerialObservation::Absent
                    | SelectedSerialObservation::Attached(_)
                    | SelectedSerialObservation::Unauthorized
                    | SelectedSerialObservation::Other => {}
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })
}

fn cleanup_active_host_stimulus(stimulus: &ActiveStimulus) -> Vec<String> {
    let mut residual = Vec::new();
    if fs::remove_file(&stimulus.host_source_path).is_err() && stimulus.host_source_path.exists() {
        residual.push("fixture-owned active host payload remained".to_string());
    }
    if fs::remove_dir(stimulus.host_workspace.path()).is_err()
        && stimulus.host_workspace.path().exists()
    {
        residual.push("fixture-owned active host workspace remained".to_string());
    }
    residual
}

fn cleanup_fixture(
    invocation: &Invocation,
    facts: &DeviceFacts,
    low_storage_host_payload: Option<&Path>,
    active_stimulus: Option<&ActiveStimulus>,
) -> (String, Vec<String>) {
    let (first, second) = run_scope_paths(invocation);
    let mut residual = Vec::new();
    let remove_owned_path = |path: &str, residual: &mut Vec<String>| {
        let result = if invocation.scenario.is_root() {
            adb_query(&facts.serial, &["shell", "su", "-c", "rm", "-f", path])
        } else {
            adb_query(&facts.serial, &["shell", "rm", "-f", path])
        };
        let exists = fixture_path_exists(invocation, facts, path).unwrap_or(true);
        if result.is_err() || exists {
            residual.push("fixture-owned residual path".to_string());
        }
    };

    // Keep cleanup order explicit: payloads, low-storage filler, sentinel,
    // then the recovery reserve.  Removing the reserve last preserves the
    // recovery budget for the operator if an earlier cleanup operation fails.
    for path in [first, second] {
        remove_owned_path(&path, &mut residual);
    }
    if invocation.scenario.supports_host_push_active_stimulus() {
        let (calibration_destination, _) = active_stimulus_device_paths(invocation);
        remove_owned_path(&calibration_destination, &mut residual);
    }
    if invocation.scenario == Scenario::LowStorage {
        remove_owned_path(
            &format!("{}/{}", invocation.run_scope, STORAGE_FILL_FILE),
            &mut residual,
        );
    }
    if invocation.sentinel.cleanup().is_err() {
        residual.push("sentinel residual state remained".to_string());
    }
    if invocation.scenario == Scenario::LowStorage {
        remove_owned_path(
            &format!("{}/{}", invocation.run_scope, STORAGE_RESERVE_FILE),
            &mut residual,
        );
    }
    for scope in run_scope_roots(invocation) {
        let scope_existed = fixture_path_exists(invocation, facts, &scope).unwrap_or(true);
        if scope_existed {
            let removed = if invocation.scenario.is_root() {
                adb_query(&facts.serial, &["shell", "su", "-c", "rmdir", &scope]).is_ok()
            } else {
                adb_query(&facts.serial, &["shell", "rmdir", &scope]).is_ok()
            };
            if !removed || fixture_path_exists(invocation, facts, &scope).unwrap_or(true) {
                residual.push("fixture-owned run scope remained".to_string());
            }
        }
    }
    if let Some(path) = low_storage_host_payload {
        if fs::remove_file(path).is_err() && path.exists() {
            residual.push("fixture-owned host payload remained".to_string());
        }
    }
    if let Some(stimulus) = active_stimulus {
        residual.extend(cleanup_active_host_stimulus(stimulus));
    }
    let outcome = if residual.is_empty() {
        "succeeded"
    } else {
        "failed"
    };
    (outcome.to_string(), residual)
}

struct EvidenceInputs<'a> {
    invocation: &'a Invocation,
    facts: &'a DeviceFacts,
    result: Result<&'a ExecutionRunResult, &'a String>,
    action_time: Option<SystemTime>,
    checkpoint_error: Option<String>,
    cleanup: (String, Vec<String>),
    slot_observation: ExecutionSlotObservation,
    active_process: Option<Value>,
    active_cancellation: Option<Value>,
    identity_capture: Option<IdentityTransitionCapture>,
    authorization_capture: Option<AuthorizationTransitionCapture>,
    terminal_detected_at: Option<SystemTime>,
    host_sleep_events: Option<Vec<OwnedProcessLifecycleEvent>>,
    timeout: Option<Value>,
    storage: Option<&'a StorageObservation>,
    host_payload: Option<&'a Path>,
    active_stimulus: Option<&'a ActiveStimulus>,
    sentinel: Value,
    cleanup_started_at: SystemTime,
    cleanup_completed_at: SystemTime,
}

fn evidence_record(inputs: EvidenceInputs<'_>) -> Value {
    let EvidenceInputs {
        invocation,
        facts,
        result,
        action_time,
        checkpoint_error,
        cleanup,
        slot_observation,
        active_process,
        active_cancellation,
        identity_capture,
        authorization_capture,
        terminal_detected_at,
        host_sleep_events,
        timeout,
        storage,
        host_payload,
        active_stimulus,
        sentinel,
        cleanup_started_at,
        cleanup_completed_at,
    } = inputs;
    let (cleanup_outcome, residuals) = cleanup;
    let host_sleep = host_sleep_evidence(
        &invocation.scenario,
        &sentinel,
        result,
        host_sleep_events.as_deref().unwrap_or(&[]),
    );
    let identity_transition = identity_transition_evidence(
        &invocation.scenario,
        facts,
        &invocation.run_scope,
        result,
        identity_capture.as_ref(),
    );
    let authorization_transition = authorization_transition_evidence(
        invocation,
        result,
        authorization_capture.as_ref(),
        terminal_detected_at,
        &sentinel,
        cleanup_started_at,
        cleanup_completed_at,
    );
    let (outcome, issue_code, steps, partial_changes, authority_invalidated, contract_error) =
        match result {
            Ok(run) => {
                let issue = run
                    .steps
                    .iter()
                    .find_map(|step| step.failure_kind.map(issue_code));
                let partial = partial_changes_possible(run);
                let authority = issue.is_some_and(|code| {
                    matches!(
                        code,
                        "operation_timed_out"
                            | "device_transport_lost"
                            | "device_offline"
                            | "device_unauthorized"
                            | "device_disconnected"
                            | "device_identity_changed"
                            | "device_identity_unverified"
                            | "root_authority_revoked"
                            | "root_authority_unverified"
                            | "device_storage_exhausted"
                    )
                });
                let steps = step_summary(run);
                let observed = json!({
                    "success": run.success,
                    "issue": issue,
                    "stepStates": steps,
                    "partialChangesPossible": partial,
                    "authorityInvalidated": authority,
                    "activeSlotReleased": slot_observation.released,
                    "runScope": format!("run-scope-sha256:{}", digest(&invocation.run_scope)),
                    "deviceScope": format!("serial-sha256:{}", digest(&invocation.serial)),
                    "boundaryReadyAt": sentinel.get("boundaryReadyAt").cloned().unwrap_or(Value::Null),
                    "operatorActionAt": sentinel.get("operatorActionAt").cloned().unwrap_or(Value::Null),
                    "activeSlotObservation": {
                        "acquired": slot_observation.acquired,
                        "released": slot_observation.released,
                        "runId": slot_observation.run_id,
                        "executionId": format!("execution-sha256:{}", digest(&slot_observation.execution_id)),
                        "acquiredAt": format!("unix:{}", slot_observation.acquired_at_unix),
                        "terminalCleanupAt": slot_observation.terminal_cleanup_at_unix.map(|value| format!("unix:{value}")),
                        "releasedAt": slot_observation.released_at_unix.map(|value| format!("unix:{value}")),
                        "sourceKind": "production_owned_slot",
                        "evidence": "production-execution-session-slot",
                    },
                    "activeProcess": active_process.clone().unwrap_or(Value::Null),
                    "timeout": timeout.clone().unwrap_or(Value::Null),
                    "activeCancellation": active_cancellation.clone().unwrap_or(Value::Null),
                    "hostSleep": host_sleep.clone(),
                    "identityTransition": identity_transition.clone(),
                    "authorizationTransition": authorization_transition.clone(),
                    "cleanup": cleanup_outcome,
                    "residual": if residuals.is_empty() { "clean" } else { "residual" },
                });
                let contract_error =
                    evaluate_scenario_contract(&invocation.scenario_contract, &observed).err();
                let missing_physical_measurement =
                    invocation.scenario_contract.host_sleep.is_some()
                        && (host_sleep.is_null()
                            || host_sleep
                                .get("timerClassification")
                                .and_then(Value::as_str)
                                .is_some_and(|classification| {
                                    matches!(classification, "indeterminate" | "contradictory")
                                })
                            || host_sleep.get("terminalOutcome").and_then(Value::as_str)
                                == Some("transport_loss"))
                        || (invocation.scenario_contract.identity_transition.is_some()
                            && observed.get("identityTransition") == Some(&Value::Null))
                        || (invocation
                            .scenario_contract
                            .authorization_transition
                            .is_some()
                            && observed.get("authorizationTransition") == Some(&Value::Null))
                        || (invocation.scenario_contract.active_process.is_some()
                            && observed.get("activeProcess") == Some(&Value::Null));
                let outcome = if checkpoint_error.is_some() || missing_physical_measurement {
                    "blocked"
                } else if contract_error.is_none() {
                    "passed"
                } else {
                    "failed"
                };
                (outcome, issue, steps, partial, authority, contract_error)
            }
            Err(error) => (
                "blocked",
                None,
                json!({ "executed": 0, "skipped": 0, "failed": 0, "cancelled": 0, "blocked": 0, "notAttempted": 0, "error": sanitize_fact(error) }),
                false,
                false,
                Some(sanitize_fact(error)),
            ),
        };
    let mut notes = vec![
        "Physical qualification evidence is sanitized; raw ADB output is intentionally omitted.".to_string(),
        "The operation used the production reviewed plan and ExecutorRunner<RealAdbDevice> boundary.".to_string(),
    ];
    if let Some(error) = checkpoint_error {
        notes.push(error);
    }
    if let Some(error) = contract_error {
        notes.push(format!("scenarioContractError={error}"));
    }
    if let Some(action_time) = action_time {
        if let Ok(seconds) = action_time.duration_since(UNIX_EPOCH) {
            notes.push(format!(
                "operatorActionTimestampUnixSeconds={}",
                seconds.as_secs()
            ));
        }
    }
    if let Some(stimulus) = active_stimulus {
        notes.push(format!(
            "activeStimulusPayloadKib={} predictedDurationMs={}",
            stimulus.payload_kib, stimulus.predicted_ms
        ));
    }
    let storage_value = storage
        .map(|observation| {
            json!({
                "initialFreeKib": observation.initial_free_kib,
                "recoveryReserveKib": observation.recovery_reserve_kib,
                "fillerKib": observation.filler_kib,
                "finalFreeKib": observation.final_free_kib,
                "restoredRecoveryReserveKib": observation.restored_recovery_reserve_kib,
                "reserveCreated": observation.reserve_created,
                "reserveRemoved": observation.reserve_removed,
                "ownershipVerified": observation.ownership_verified,
                "boundedAllocation": observation.bounded_allocation,
                "cleanupVerified": observation.cleanup_verified,
            })
        })
        .unwrap_or(Value::Null);
    let (first_path, second_path) = run_scope_paths(invocation);
    let mut owned_paths = cleanup_device_paths_for_scenario(
        invocation.scenario,
        &first_path,
        &second_path,
        &invocation.run_scope,
    );
    if let Some(stimulus) = active_stimulus {
        owned_paths.push(
            stimulus
                .host_workspace
                .path()
                .to_string_lossy()
                .into_owned(),
        );
        owned_paths.push(
            stimulus
                .host_workspace
                .path()
                .join(ACTIVE_CALIBRATION_SOURCE_FILE)
                .to_string_lossy()
                .into_owned(),
        );
        owned_paths.push(stimulus.host_source_path.to_string_lossy().into_owned());
    }
    if let Some(path) = host_payload {
        owned_paths.push(path.to_string_lossy().into_owned());
    }
    let contract_value = serde_json::to_value(&invocation.scenario_contract)
        .expect("scenario contract must serialize");
    let trace = json!({
        "runId": invocation.run_id,
        "scenario": invocation.scenario.as_str(),
        "repetition": invocation.repetition,
        "sentinel": sentinel,
        "slot": {
            "runId": slot_observation.run_id,
            "executionId": format!("execution-sha256:{}", digest(&slot_observation.execution_id)),
            "acquired": slot_observation.acquired,
            "released": slot_observation.released,
            "acquiredAt": format!("unix:{}", slot_observation.acquired_at_unix),
            "terminalCleanupAt": slot_observation.terminal_cleanup_at_unix.map(|value| format!("unix:{value}")),
            "releasedAt": slot_observation.released_at_unix.map(|value| format!("unix:{value}")),
        },
        "terminal": {
            "outcome": outcome,
            "issueCode": issue_code,
            "stepStates": steps,
        },
    });
    let trace_digest = format!("sha256:{}", canonical_value_digest(&trace));
    let mut record = json!({
        "schemaVersion": 1,
        "scenario": invocation.scenario.as_str(),
        "repetition": invocation.repetition,
        "timestamp": timestamp(),
        "commit": git_head(),
        "host": host_facts(),
        "platformToolsRevision": platform_tools_revision(),
        "device": {
            "identity": format!("serial-sha256:{}", digest(&facts.serial)),
            "model": facts.model,
            "androidVersion": facts.android_version,
            "apiLevel": facts.api_level,
            "abi": facts.abi,
            "buildFingerprint": facts.build_fingerprint,
        },
        "root": if facts.root_shell {
            json!({
                "implementationVersion": facts
                    .root_version
                    .as_deref()
                    .unwrap_or("unreported")
            })
        } else {
            Value::Null
        },
        "fixtureApkSha256": fixture_apk_checksum(),
        "optIns": opt_ins(invocation),
        "command": "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution physical_interruption_qualification::manual_phase_6d6_physical_interruption_qualification -- --ignored --exact",
        "preparation": "Phase 6C fixture contract, exact serial inventory, package allowlist, destination ownership, and empty sentinel directory validated.",
        "operatorAction": if invocation.scenario.is_active_checkpoint() || invocation.scenario.is_boundary_checkpoint() || invocation.scenario == Scenario::LowStorage { "Required bounded sentinel action was requested from the operator." } else if invocation.scenario == Scenario::OperationTimeout { "No operator transition or action marker was used; the fixed test-only qualification deadline exercised the production owned-process timeout path." } else { "No operator transition required; controlled test seam or identity observation used." },
        "executionSuccess": result.as_ref().ok().is_some_and(|run| run.success),
        "observedIssueCode": issue_code,
        "stepStates": steps,
        "partialChangesPossible": partial_changes,
        "authorityInvalidated": authority_invalidated,
        "activeSlotReleased": slot_observation.released,
        "activeSlotObservation": {
            "observed": slot_observation.acquired,
            "acquired": slot_observation.acquired,
            "released": slot_observation.released,
            "runId": slot_observation.run_id,
            "executionId": format!("execution-sha256:{}", digest(&slot_observation.execution_id)),
            "acquiredAt": format!("unix:{}", slot_observation.acquired_at_unix),
            "terminalCleanupAt": slot_observation.terminal_cleanup_at_unix.map(|value| format!("unix:{value}")),
            "releasedAt": slot_observation.released_at_unix.map(|value| format!("unix:{value}")),
            "sourceKind": "production_owned_slot",
            "evidence": "production-execution-session-slot",
        },
        "activeProcess": active_process.clone().unwrap_or(Value::Null),
        "timeout": timeout.unwrap_or(Value::Null),
        "activeCancellation": active_cancellation.unwrap_or(Value::Null),
        "hostSleep": host_sleep,
        "identityTransition": identity_transition,
        "authorizationTransition": authorization_transition,
        "scenarioContract": contract_value,
        "scenarioFacts": {
            "rootShell": facts.root_shell,
            "activeCheckpoint": invocation.scenario.is_active_checkpoint(),
            "boundaryCheckpoint": invocation.scenario.is_boundary_checkpoint(),
            "requiresSentinelAction": invocation.scenario.is_active_checkpoint() || invocation.scenario.is_boundary_checkpoint() || invocation.scenario == Scenario::LowStorage,
            "storage": invocation.scenario == Scenario::LowStorage,
            "runScope": format!("run-scope-sha256:{}", digest(&invocation.run_scope)),
            "operationClass": active_operation_class(invocation.scenario),
        },
        "sentinel": sentinel,
        "storage": storage_value,
        "cleanup": {
            "command": "adb -s <selected-serial> shell rm -f <fixture-owned-path>",
            "outcome": cleanup_outcome,
            "ownedPathDigests": owned_paths
                .iter()
                .map(|path| format!("path-sha256:{}", digest(path)))
                .collect::<Vec<_>>(),
            "verified": cleanup_outcome == "succeeded",
            "nonFixtureDeletion": false,
        },
        "residualStateCheck": {
            "outcome": if residuals.is_empty() { "clean" } else { "residual" },
            "residuals": residuals,
        },
        "outcome": outcome,
        "notes": notes,
    });
    let fields = record
        .as_object_mut()
        .expect("qualification record must be an object");
    fields.insert(
        "runId".to_string(),
        Value::String(invocation.run_id.clone()),
    );
    fields.insert(
        "evidencePath".to_string(),
        Value::String(invocation.evidence_path.clone()),
    );
    fields.insert(
        "tracePath".to_string(),
        Value::String(invocation.trace_path.clone()),
    );
    fields.insert(
        "recordDigest".to_string(),
        Value::String(format!("sha256:{}", "0".repeat(64))),
    );
    fields.insert("traceDigest".to_string(), Value::String(trace_digest));
    fields.insert("trace".to_string(), trace);
    let record_digest = format!("sha256:{}", canonical_record_digest(&record));
    record["recordDigest"] = Value::String(record_digest);
    record
}

fn step_summary(result: &ExecutionRunResult) -> Value {
    let count = |status: StepRunStatus| {
        result
            .steps
            .iter()
            .filter(|step| step.status == status)
            .count()
    };
    json!({
        "executed": count(StepRunStatus::Executed),
        "skipped": count(StepRunStatus::Skipped),
        "failed": count(StepRunStatus::Failed),
        "cancelled": count(StepRunStatus::Cancelled),
        "blocked": count(StepRunStatus::Blocked),
        "notAttempted": result.total_steps.saturating_sub(result.steps.len()),
    })
}

fn partial_changes_possible(result: &ExecutionRunResult) -> bool {
    !result.success
        && result
            .steps
            .iter()
            .any(|step| matches!(step.status, StepRunStatus::Executed | StepRunStatus::Failed))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostSleepClockClassification {
    SuspendedTimeIncluded,
    SuspendedTimeExcluded,
    Indeterminate,
    Contradictory,
}

#[derive(Clone, Copy, Debug)]
struct HostSleepClockMeasurement {
    suspended_wall_ms: u64,
    deadline_clock_advance_ms: u64,
    remaining_before_sleep_ms: u64,
    remaining_after_wake_ms: u64,
    tolerance_ms: u64,
}

fn within_tolerance(left: u64, right: u64, tolerance: u64) -> bool {
    left.abs_diff(right) <= tolerance
}

/// Classify suspension from the deadline clock and remaining budget only.
/// Terminal success or timeout is deliberately not an input.
fn classify_host_sleep_clock(
    measurement: HostSleepClockMeasurement,
) -> HostSleepClockClassification {
    if measurement.remaining_after_wake_ms > measurement.remaining_before_sleep_ms {
        return HostSleepClockClassification::Contradictory;
    }
    let consumed_budget = measurement
        .remaining_before_sleep_ms
        .saturating_sub(measurement.remaining_after_wake_ms);
    let expected_consumption = measurement
        .deadline_clock_advance_ms
        .min(measurement.remaining_before_sleep_ms);
    let budget_matches_clock = within_tolerance(
        consumed_budget,
        expected_consumption,
        measurement.tolerance_ms,
    );
    let clock_excluded = measurement.deadline_clock_advance_ms <= measurement.tolerance_ms;
    let clock_included = within_tolerance(
        measurement.deadline_clock_advance_ms,
        measurement.suspended_wall_ms,
        measurement.tolerance_ms,
    );
    match (clock_excluded, clock_included, budget_matches_clock) {
        (true, false, true) => HostSleepClockClassification::SuspendedTimeExcluded,
        (false, true, true) => HostSleepClockClassification::SuspendedTimeIncluded,
        (true, false, false) | (false, true, false) => HostSleepClockClassification::Contradictory,
        _ => HostSleepClockClassification::Indeterminate,
    }
}

fn host_sleep_unix_seconds(value: &Value, field: &str) -> Option<u64> {
    value
        .get(field)?
        .as_str()?
        .strip_prefix("unix:")?
        .parse()
        .ok()
}

fn toolchain() -> String {
    let output = Command::new("rustc").arg("--version").output();
    let Ok(output) = output else {
        return "unreported".to_string();
    };
    output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(sanitize_fact)
        })
        .flatten()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unreported".to_string())
}

/// Build the complete sanitized host-sleep evidence object from the exact
/// owned-process deadline-clock samples, the fresh sentinel markers, and the
/// terminal execution result.
///
/// Fail-closed behavior: any missing, mismatched, out-of-order, or
/// contradictory observation returns `Value::Null` so the repetition is
/// blocked. The pre-suspend and post-wake samples are the two watcher-requested
/// samples on the exact selected operation; the terminal sample is
/// owner-recorded and authoritative for terminal chronology. The post-wake
/// sample may legitimately follow the owner terminal sample when the deadline
/// became ready immediately after host resume.
fn host_sleep_evidence(
    scenario: &Scenario,
    sentinel: &Value,
    result: Result<&ExecutionRunResult, &String>,
    events: &[OwnedProcessLifecycleEvent],
) -> Value {
    if !matches!(
        scenario,
        Scenario::HostSleepBeforeDeadline | Scenario::HostSleepAfterDeadline
    ) {
        return Value::Null;
    }
    let run = match result {
        Ok(run) => run,
        Err(_) => return Value::Null,
    };
    let issue = run
        .steps
        .iter()
        .find_map(|step| step.failure_kind.map(issue_code));
    // Only truthful terminal branches may serialize host-sleep evidence.
    // Identity, root, storage, authorization, generic step, unknown, and
    // contradictory result states block the repetition rather than being
    // relabeled as transport loss. Runtime loss is not produced by this
    // harness because it has no authoritative runtime/session-loss evidence.
    let terminal_outcome = match (run.success, issue) {
        (true, None) => "completed",
        (false, Some("operation_timed_out")) => "timed_out",
        (
            false,
            Some(
                "device_offline"
                | "device_disconnected"
                | "adb_server_unavailable"
                | "device_transport_lost",
            ),
        ) => "transport_loss",
        _ => return Value::Null,
    };
    let deadline_ns = u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos())
        .expect("the host-sleep qualification deadline must fit u64 nanoseconds");
    let mut candidates = events.iter().filter_map(|event| match event {
        OwnedProcessLifecycleEvent::DeadlineClockStarted {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            deadline_ns: observed,
            ..
        } if *observed == deadline_ns => Some(*operation_id),
        _ => None,
    });
    let Some(operation_id) = candidates.next() else {
        return Value::Null;
    };
    if candidates.next().is_some() {
        return Value::Null;
    }
    if events.iter().any(|event| {
        event.operation_id() == operation_id && event.operation() != ProcessOperation::DeviceCopy
    }) {
        return Value::Null;
    }
    let clock_starts = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineClockStarted {
                    operation_id: observed,
                    ..
                } if *observed == operation_id
            )
        })
        .count();
    if clock_starts != 1 {
        return Value::Null;
    }
    let (deadline_clock_start_ns, deadline_start_at) = events
        .iter()
        .find_map(|event| match event {
            OwnedProcessLifecycleEvent::DeadlineClockStarted {
                operation_id: observed,
                deadline_clock_start_ns,
                at,
                ..
            } if *observed == operation_id => Some((*deadline_clock_start_ns, *at)),
            _ => None,
        })
        .expect("the single clock start must be present");
    if deadline_clock_start_ns != 0 {
        return Value::Null;
    }
    let deadline_start_sec = canonical_unix_seconds(deadline_start_at).unwrap_or(0);
    let Some((sentinel_started_sec, requested_sec, entered_sec, wake_sec)) = (|| {
        Some((
            host_sleep_unix_seconds(sentinel, "operationStartedAt")?,
            host_sleep_unix_seconds(sentinel, "sleepRequestedAt")?,
            host_sleep_unix_seconds(sentinel, "sleepEnteredAt")?,
            host_sleep_unix_seconds(sentinel, "wakeAt")?,
        ))
    })() else {
        return Value::Null;
    };
    // The sentinel progress marker is an independent observation and must
    // precede the exact owned-process deadline-clock start, which is the
    // authority for the 120-second host-sleep deadline phase.
    if !(sentinel_started_sec <= deadline_start_sec
        && deadline_start_sec <= requested_sec
        && requested_sec <= entered_sec
        && entered_sec <= wake_sec)
    {
        return Value::Null;
    }
    let requested_samples = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    operation_id: observed,
                    owner_reported: false,
                    ..
                } if *observed == operation_id
            )
        })
        .collect::<Vec<_>>();
    if requested_samples.len() != 2 {
        return Value::Null;
    }
    let before = match requested_samples[0] {
        OwnedProcessLifecycleEvent::DeadlineClockSampled {
            deadline_clock_ns,
            remaining_ns,
            ..
        } => (*deadline_clock_ns, *remaining_ns),
        _ => unreachable!("requested sample must be a sampled event"),
    };
    let after = match requested_samples[1] {
        OwnedProcessLifecycleEvent::DeadlineClockSampled {
            deadline_clock_ns,
            remaining_ns,
            ..
        } => (*deadline_clock_ns, *remaining_ns),
        _ => unreachable!("requested sample must be a sampled event"),
    };
    let terminal_samples = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    operation_id: observed,
                    owner_reported: true,
                    ..
                } if *observed == operation_id
            )
        })
        .count();
    if terminal_samples != 1 {
        return Value::Null;
    }
    let terminal_sample = events
        .iter()
        .find_map(|event| match event {
            OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id: observed,
                owner_reported: true,
                deadline_clock_ns,
                remaining_ns,
                deadline_reached,
                ..
            } if *observed == operation_id => {
                Some((*deadline_clock_ns, *remaining_ns, *deadline_reached))
            }
            _ => None,
        })
        .expect("the single owner terminal sample must be present");
    let (terminal_ns, _terminal_remaining_ns, terminal_reached) = terminal_sample;
    let terminal_events = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::Terminal {
                    operation_id: observed,
                    ..
                } if *observed == operation_id
            )
        })
        .count();
    if terminal_events != 1 {
        return Value::Null;
    }
    let terminal_at = events
        .iter()
        .find_map(|event| match event {
            OwnedProcessLifecycleEvent::Terminal {
                operation_id: observed,
                at,
                ..
            } if *observed == operation_id => Some(*at),
            _ => None,
        })
        .expect("the single terminal event must be present");
    let before_seconds = canonical_unix_seconds(requested_samples[0].at()).unwrap_or(0);
    let after_seconds = canonical_unix_seconds(requested_samples[1].at()).unwrap_or(0);
    let terminal_seconds = canonical_unix_seconds(terminal_at).unwrap_or(0);
    if before_seconds > entered_sec
        || after_seconds < wake_sec
        || before.0 > after.0
        || before.0 > terminal_ns
    {
        return Value::Null;
    }
    let deadline_events = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineReached {
                    operation_id: observed,
                    ..
                } if *observed == operation_id
            )
        })
        .count();
    let matching_deadline_events = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                OwnedProcessLifecycleEvent::DeadlineReached {
                    operation_id: observed,
                    deadline,
                    ..
                } if *observed == operation_id
                    && *deadline == HOST_SLEEP_QUALIFICATION_DEADLINE
            )
        })
        .count();
    // The owner-emitted DeadlineReached event is the authority for whether the
    // timeout branch won. timed_out requires exactly one matching event;
    // completed and transport_loss both require zero. A monotonic clock sample
    // at or beyond the nominal deadline is only a clock observation and does
    // not select the timeout branch.
    match terminal_outcome {
        "timed_out"
            if deadline_events != 1 || matching_deadline_events != 1 || !terminal_reached =>
        {
            return Value::Null;
        }
        "completed" | "transport_loss" if deadline_events != 0 => {
            return Value::Null;
        }
        _ => {}
    }
    let remaining_before_ms = before.1 / 1_000_000;
    let remaining_after_ms = after.1 / 1_000_000;
    if remaining_before_ms == 0 || remaining_after_ms > remaining_before_ms {
        return Value::Null;
    }
    let deadline_ms = HOST_SLEEP_QUALIFICATION_DEADLINE.as_millis() as u64;
    let wake_elapsed_ms = wake_sec
        .saturating_sub(deadline_start_sec)
        .saturating_mul(1000);
    let phase_valid = if *scenario == Scenario::HostSleepBeforeDeadline {
        wake_elapsed_ms < deadline_ms
    } else {
        wake_elapsed_ms >= deadline_ms
    };
    if !phase_valid {
        return Value::Null;
    }
    let advance_ms = after.0.saturating_sub(before.0) / 1_000_000;
    let classification = match classify_host_sleep_clock(HostSleepClockMeasurement {
        suspended_wall_ms: wake_sec.saturating_sub(entered_sec).saturating_mul(1000),
        deadline_clock_advance_ms: advance_ms,
        remaining_before_sleep_ms: remaining_before_ms,
        remaining_after_wake_ms: remaining_after_ms,
        tolerance_ms: HOST_SLEEP_MEASUREMENT_TOLERANCE_MS,
    }) {
        HostSleepClockClassification::SuspendedTimeIncluded => "suspended_time_included",
        HostSleepClockClassification::SuspendedTimeExcluded => "suspended_time_excluded",
        HostSleepClockClassification::Indeterminate
        | HostSleepClockClassification::Contradictory => return Value::Null,
    };
    let terminal_clock_elapsed_ms = terminal_ns / 1_000_000;
    if terminal_outcome == "timed_out" && terminal_clock_elapsed_ms < deadline_ms {
        return Value::Null;
    }
    let host = host_facts();
    let host_os = host
        .get("os")
        .and_then(Value::as_str)
        .unwrap_or("unreported")
        .to_string();
    let host_version = host
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("unreported")
        .to_string();
    json!({
        "sleepRequestedAt": format!("unix:{requested_sec}"),
        "sleepEnteredAt": format!("unix:{entered_sec}"),
        "wakeAt": format!("unix:{wake_sec}"),
        "wallElapsedMs": terminal_seconds.saturating_sub(deadline_start_sec).saturating_mul(1000),
        "executorElapsedMs": terminal_clock_elapsed_ms,
        "deadlineMs": deadline_ms,
        "operationStartedAt": format!("unix:{deadline_start_sec}"),
        "terminalAt": format!("unix:{terminal_seconds}"),
        "terminalOutcome": terminal_outcome,
        "hostOs": host_os,
        "hostVersion": host_version,
        "timerImplementation": HOST_SLEEP_TIMER_IMPLEMENTATION,
        "toolchain": toolchain(),
        "timerClassification": classification,
        "measurementBasis": HOST_SLEEP_MEASUREMENT_BASIS,
        "transportLossBlockedMeasurement": matches!(terminal_outcome, "transport_loss" | "runtime_loss"),
        "elapsedBeforeSleepMs": requested_sec.saturating_sub(deadline_start_sec).saturating_mul(1000),
        "deadlineClockStartNs": format!("monotonic-ns:{deadline_clock_start_ns}"),
        "deadlineClockBeforeSleepNs": format!("monotonic-ns:{}", before.0),
        "deadlineClockAfterWakeNs": format!("monotonic-ns:{}", after.0),
        "deadlineClockTerminalNs": format!("monotonic-ns:{terminal_ns}"),
        "suspendedWallMs": wake_sec.saturating_sub(entered_sec).saturating_mul(1000),
        "deadlineClockAdvanceDuringSuspensionMs": advance_ms,
        "remainingBeforeSleepMs": remaining_before_ms,
        "remainingAfterWakeMs": remaining_after_ms,
        "measurementToleranceMs": HOST_SLEEP_MEASUREMENT_TOLERANCE_MS,
        "toleranceRationale": "8000 ms covers the enforced 4-second sleep-entered handoff window after sleep-ready, the 1-second sample-to-sleep-ready latency budget, canonical-second sentinel quantization on both sleep/wake boundaries, and scheduler margin.",
        "operatorActionPhase": if *scenario == Scenario::HostSleepBeforeDeadline {
            "before_deadline"
        } else {
            "after_deadline"
        },
        "deadlineClockSource": "owned_process_monotonic_deadline_clock",
    })
}

fn identity_transition_evidence(
    scenario: &Scenario,
    facts: &DeviceFacts,
    run_scope: &str,
    result: Result<&ExecutionRunResult, &String>,
    capture: Option<&IdentityTransitionCapture>,
) -> Value {
    if !matches!(
        scenario,
        Scenario::IdentityStability | Scenario::IdentityReplacement
    ) {
        return Value::Null;
    }
    let Some(capture) = capture else {
        return Value::Null;
    };
    let (
        Some(original_disconnected_at),
        Some(serial_absent_from),
        Some(serial_absent_until),
        Some(replacement_attached_at),
        Some(replacement_fingerprint),
    ) = (
        capture.original_disconnected_at,
        capture.serial_absent_from,
        capture.serial_absent_until,
        capture.replacement_attached_at,
        capture.replacement_fingerprint.as_deref(),
    )
    else {
        return Value::Null;
    };
    let expected_issue = result.ok().and_then(|run| {
        run.steps.iter().find_map(|step| {
            step.failure_kind.map(issue_code).filter(|code| {
                matches!(
                    *code,
                    "device_identity_changed" | "device_identity_unverified"
                )
            })
        })
    });
    if *scenario == Scenario::IdentityReplacement && expected_issue.is_none() {
        return Value::Null;
    }
    if *scenario == Scenario::IdentityStability
        && (expected_issue.is_some() || replacement_fingerprint != facts.build_fingerprint)
    {
        return Value::Null;
    }
    let cleanup_final_attached = matches!(
        selected_serial_observation(&facts.serial),
        Ok(SelectedSerialObservation::Attached(_))
    );
    if !cleanup_final_attached {
        return Value::Null;
    }
    json!({
        "initialSerial": format!("serial-sha256:{}", digest(&facts.serial)),
        "initialFingerprint": format!("fingerprint-sha256:{}", digest(&facts.build_fingerprint)),
        "originalAttached": capture.original_attached,
        "originalDisconnectedAt": system_time_value(Some(original_disconnected_at)),
        "serialAbsentFrom": system_time_value(Some(serial_absent_from)),
        "serialAbsentUntil": system_time_value(Some(serial_absent_until)),
        "replacementAttachedAt": system_time_value(Some(replacement_attached_at)),
        "replacementSerial": format!("serial-sha256:{}", digest(&facts.serial)),
        "replacementFingerprint": format!("fingerprint-sha256:{}", digest(replacement_fingerprint)),
        "neverSimultaneous": capture.never_simultaneous,
        "expectedIssueCode": expected_issue,
        "authorityInvalidated": *scenario == Scenario::IdentityReplacement,
        "cleanupFinalAttached": cleanup_final_attached,
        "runId": format!("run-scope-sha256:{}", digest(run_scope)),
    })
}

fn is_authorization_terminal_issue(code: &str) -> bool {
    matches!(code, "device_unauthorized" | "device_identity_unverified")
}

fn authorization_transition_evidence(
    invocation: &Invocation,
    result: Result<&ExecutionRunResult, &String>,
    capture: Option<&AuthorizationTransitionCapture>,
    terminal_detected_at: Option<SystemTime>,
    sentinel: &Value,
    cleanup_started_at: SystemTime,
    cleanup_completed_at: SystemTime,
) -> Value {
    if invocation.scenario != Scenario::DeviceUnauthorized {
        return Value::Null;
    }
    let Some(capture) = capture else {
        return Value::Null;
    };
    let (
        Some(initial_observed_at),
        Some(revocation_checkpoint_at),
        Some(original_disconnected_at),
        Some(serial_absent_from),
        Some(serial_absent_until),
        Some(reconnected_at),
        Some(observed_at),
        Some(final_state_observed_at),
        Some(terminal_detected_at),
    ) = (
        capture.initial_observed_at,
        capture.revocation_checkpoint_at,
        capture.original_disconnected_at,
        capture.serial_absent_from,
        capture.serial_absent_until,
        capture.reconnected_at,
        capture.observed_at,
        capture.final_state_observed_at,
        terminal_detected_at,
    )
    else {
        return Value::Null;
    };
    if !capture.initial_authorized || !capture.final_authorized {
        return Value::Null;
    }
    let issue = result.ok().and_then(|run| {
        run.steps
            .iter()
            .find_map(|step| step.failure_kind.map(issue_code))
            .filter(|code| is_authorization_terminal_issue(code))
    });
    let Some(issue) = issue else {
        return Value::Null;
    };
    let operation_started_at = sentinel
        .get("operationStartedAt")
        .cloned()
        .unwrap_or(Value::Null);
    let first_operation_completed_at = sentinel
        .get("operationFinishedAt")
        .cloned()
        .unwrap_or(Value::Null);
    if operation_started_at.is_null() || first_operation_completed_at.is_null() {
        return Value::Null;
    }
    json!({
        "initialState": "authorized",
        "initialObservedAt": system_time_value(Some(initial_observed_at)),
        "operationStartedAt": operation_started_at,
        "firstOperationCompletedAt": first_operation_completed_at,
        "revocationCheckpointAt": system_time_value(Some(revocation_checkpoint_at)),
        "originalDisconnectedAt": system_time_value(Some(original_disconnected_at)),
        "serialAbsentFrom": system_time_value(Some(serial_absent_from)),
        "serialAbsentUntil": system_time_value(Some(serial_absent_until)),
        "reconnectedAt": system_time_value(Some(reconnected_at)),
        "observedState": "unauthorized",
        "observedAt": system_time_value(Some(observed_at)),
        "terminalDetectedAt": system_time_value(Some(terminal_detected_at)),
        "cleanupStartedAt": system_time_value(Some(cleanup_started_at)),
        "cleanupCompletedAt": system_time_value(Some(cleanup_completed_at)),
        "issueCode": issue,
        "authorityInvalidated": true,
        "automaticResume": false,
        "cleanupFinalState": "authorized",
        "finalStateObservedAt": system_time_value(Some(final_state_observed_at)),
        "runId": format!("run-scope-sha256:{}", digest(&invocation.run_scope)),
        "deviceScope": format!("serial-sha256:{}", digest(&invocation.serial)),
    })
}

fn system_time_value(value: Option<SystemTime>) -> Value {
    value
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| Value::String(format!("unix:{}", duration.as_secs())))
        .unwrap_or(Value::Null)
}

fn sentinel_evidence(invocation: &Invocation) -> Value {
    let sentinel = &invocation.sentinel;
    let armed = sentinel.marker_time("armed").ok();
    let operation_started = sentinel.marker_time("operation-started").ok();
    let boundary_ready = sentinel.marker_time("boundary-ready").ok();
    let operator_action = sentinel.marker_time("operator-action").ok();
    let operation_finished = sentinel.marker_time("operation-finished").ok();
    let authorization_revoked = sentinel.marker_time("authorization-revoked").ok();
    let unauthorized_observed = sentinel.marker_time("unauthorized-observed").ok();
    let cleanup_ready = sentinel.marker_time("cleanup-ready").ok();
    let sleep_requested = sentinel.marker_time("sleep-requested").ok();
    let sleep_entered = sentinel.marker_time("sleep-entered").ok();
    let wake = sentinel.marker_time("wake").ok();
    let markers = [
        armed,
        operation_started,
        boundary_ready,
        operator_action,
        operation_finished,
        authorization_revoked,
        unauthorized_observed,
        cleanup_ready,
        sleep_requested,
        sleep_entered,
        wake,
    ];
    let unique_markers = markers.iter().enumerate().all(|(index, value)| {
        value.is_none_or(|value| {
            markers
                .iter()
                .skip(index + 1)
                .all(|other| other != &Some(value))
        })
    });
    let chronology_valid = armed
        .zip(operation_started)
        .zip(operation_finished)
        .is_some_and(|((armed, started), finished)| armed <= started && started <= finished)
        && boundary_ready
            .zip(operator_action)
            .is_none_or(|(boundary, action)| boundary <= action)
        && operation_finished
            .zip(cleanup_ready)
            .is_none_or(|(finished, cleanup)| finished <= cleanup);
    let authorization_chronology_valid = boundary_ready
        .zip(authorization_revoked)
        .zip(unauthorized_observed)
        .zip(operator_action)
        .is_none_or(|(((boundary, revoked), unauthorized), action)| {
            boundary <= revoked && revoked <= unauthorized && unauthorized <= action
        });
    let host_sleep_chronology_valid = operation_started
        .zip(sleep_requested)
        .zip(sleep_entered)
        .zip(wake)
        .is_none_or(|(((started, requested), entered), wake)| {
            // wake is the operator's first post-resume acknowledgement. The
            // owned process may reach terminal immediately after resume, so
            // the wake marker may follow the operation-finished marker.
            started <= requested && requested <= entered && entered <= wake
        });
    json!({
        "sentinelId": invocation.sentinel_id,
        "nonce": invocation.sentinel_nonce,
        "runId": invocation.run_id,
        "scenario": invocation.scenario.as_str(),
        "repetition": invocation.repetition,
        "armedAt": system_time_value(armed),
        "operationStartedAt": system_time_value(operation_started),
        "boundaryReadyAt": system_time_value(boundary_ready),
        "operatorActionAt": system_time_value(operator_action),
        "operationFinishedAt": system_time_value(operation_finished),
        "cleanupReadyAt": system_time_value(cleanup_ready),
        "sleepRequestedAt": system_time_value(sleep_requested),
        "sleepEnteredAt": system_time_value(sleep_entered),
        "wakeAt": system_time_value(wake),
        "chronologyValid": chronology_valid
            && host_sleep_chronology_valid
            && authorization_chronology_valid,
        "uniqueMarkers": unique_markers,
    })
}

fn issue_code(kind: StepFailureKind) -> &'static str {
    match kind {
        StepFailureKind::OperationTimedOut => "operation_timed_out",
        StepFailureKind::DeviceOffline => "device_offline",
        StepFailureKind::DeviceUnauthorized => "device_unauthorized",
        StepFailureKind::DeviceDisconnected => "device_disconnected",
        StepFailureKind::DeviceStorageExhausted => "device_storage_exhausted",
        StepFailureKind::AdbServerUnavailable => "adb_server_unavailable",
        StepFailureKind::TransportReset | StepFailureKind::TransportFailure => {
            "device_transport_lost"
        }
        StepFailureKind::DeviceIdentityChanged(_) => "device_identity_changed",
        StepFailureKind::DeviceIdentityUnverified(_) => "device_identity_unverified",
        StepFailureKind::RootAuthorityRevoked => "root_authority_revoked",
        StepFailureKind::RootAuthorityUnverified => "root_authority_unverified",
        _ => "step_execution_failed",
    }
}

fn write_evidence(invocation: &Invocation, record: &Value) -> Result<String, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live below the repository root");
    let evidence_file = root.join(&invocation.evidence_path);
    let trace_file = root.join(&invocation.trace_path);
    for file in [&evidence_file, &trace_file] {
        fs::create_dir_all(
            file.parent()
                .ok_or_else(|| "evidence path has no parent directory".to_string())?,
        )
        .map_err(|_| "evidence directory could not be created".to_string())?;
    }
    let trace = record
        .get("trace")
        .ok_or_else(|| "evidence record is missing its trace".to_string())?;
    let trace_text = serde_json::to_string_pretty(trace)
        .map_err(|_| "supporting trace is not JSON".to_string())?;
    let mut trace_output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&trace_file)
        .map_err(|_| {
            "unique supporting trace already exists or could not be created".to_string()
        })?;
    if trace_output
        .write_all(format!("{trace_text}\n").as_bytes())
        .is_err()
    {
        let _ = fs::remove_file(&trace_file);
        return Err("supporting trace could not be written".to_string());
    }
    let text = serde_json::to_string_pretty(record)
        .map_err(|_| "evidence record is not JSON".to_string())?;
    let mut evidence_output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&evidence_file)
    {
        Ok(file) => file,
        Err(_) => {
            let _ = fs::remove_file(&trace_file);
            return Err(
                "unique evidence record already exists or could not be created".to_string(),
            );
        }
    };
    if evidence_output
        .write_all(format!("{text}\n").as_bytes())
        .is_err()
    {
        let _ = fs::remove_file(&evidence_file);
        let _ = fs::remove_file(&trace_file);
        return Err("sanitized evidence could not be written".to_string());
    }
    Ok(invocation.evidence_path.clone())
}

fn parse_available_kib(output: &str) -> Result<u64, String> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| "device free-space output was missing its header".to_string())?;
    let header_fields = header.split_whitespace().collect::<Vec<_>>();
    let available_index = header_fields
        .iter()
        .position(|field| matches!(*field, "Available" | "Avail"))
        .ok_or_else(|| "device free-space output did not identify available blocks".to_string())?;

    for line in lines {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if let Some(value) = fields
            .get(available_index)
            .and_then(|value| value.parse::<u64>().ok())
        {
            return Ok(value);
        }
        if available_index > 0
            && fields
                .first()
                .is_some_and(|field| field.parse::<u64>().is_ok())
        {
            if let Some(value) = fields
                .get(available_index - 1)
                .and_then(|value| value.parse::<u64>().ok())
            {
                return Ok(value);
            }
        }
    }

    Err("device free-space available blocks were not parseable".to_string())
}

fn free_space_kib(serial: &str, path: &str) -> Result<u64, String> {
    let output = adb_query(serial, &["shell", "df", "-k", path])?;
    parse_available_kib(&output)
}

fn total_free_space_kib(serial: &str, path: &str) -> Result<u64, String> {
    let output = adb_query(serial, &["shell", "df", "-k", path])?;
    parse_total_free_kib(&output)
}

fn parse_total_free_kib(output: &str) -> Result<u64, String> {
    let available_kib = parse_available_kib(output)?;
    let fields = output
        .lines()
        .skip(1)
        .flat_map(|line| line.split_whitespace())
        .collect::<Vec<_>>();

    for window in fields.windows(5) {
        let Ok(blocks_kib) = window[0].parse::<u64>() else {
            continue;
        };
        let Ok(used_kib) = window[1].parse::<u64>() else {
            continue;
        };
        let Ok(row_available_kib) = window[2].parse::<u64>() else {
            continue;
        };
        let Some(percent) = window[3].strip_suffix('%') else {
            continue;
        };
        let Ok(percent) = percent.parse::<u8>() else {
            continue;
        };
        if percent > 100 || !window[4].starts_with('/') || row_available_kib != available_kib {
            continue;
        }

        let total_free_kib = blocks_kib
            .checked_sub(used_kib)
            .ok_or_else(|| "df reported used blocks above total blocks".to_string())?;
        if available_kib > total_free_kib {
            return Err("df reported available blocks above total free blocks".to_string());
        }
        return Ok(total_free_kib);
    }

    Err("df output did not contain a complete storage row".to_string())
}

fn bounded_low_storage_payload_kib(total_free_kib: u64) -> Result<u64, String> {
    let payload_kib = total_free_kib
        .checked_add(LOW_STORAGE_FAILURE_MARGIN_KIB)
        .ok_or_else(|| "low-storage payload size overflowed".to_string())?;
    if payload_kib > MAX_LOW_STORAGE_HOST_PAYLOAD_KIB {
        return Err(
            "low-storage total free space exceeds the bounded host payload window".to_string(),
        );
    }
    Ok(payload_kib)
}

fn device_file_size_bytes(serial: &str, path: &str) -> Result<u64, String> {
    let output = adb_query(serial, &["shell", "stat", "-c", "%s", path])?;
    output
        .trim()
        .parse::<u64>()
        .map_err(|_| "device file size was not parseable".to_string())
}

fn device_mutation(
    invocation: &Invocation,
    facts: &DeviceFacts,
    command: &str,
) -> Result<(), String> {
    if command.contains('\0') || command.contains("..") {
        return Err("device mutation command contained unsafe path text".to_string());
    }
    if invocation.scenario.is_root() {
        adb_query(&facts.serial, &["shell", "su", "-c", command]).map(|_| ())
    } else {
        adb_query(&facts.serial, &["shell", command]).map(|_| ())
    }
}

fn create_run_scope(invocation: &Invocation, facts: &DeviceFacts) -> Result<(), String> {
    for scope in run_scope_roots(invocation) {
        device_mutation(invocation, facts, &format!("mkdir -p {scope}"))?;
    }
    Ok(())
}

fn create_bounded_device_file(
    invocation: &Invocation,
    facts: &DeviceFacts,
    path: &str,
    kib: u64,
) -> Result<(), String> {
    if kib == 0 || kib > MAX_STORAGE_FILLER_KIB && path.ends_with(STORAGE_FILL_FILE) {
        return Err("device file allocation exceeded the explicit bound".to_string());
    }
    device_mutation(
        invocation,
        facts,
        &format!("dd if=/dev/zero of={path} bs=1024 count={kib} conv=fsync"),
    )
}

fn prepare_active_stimulus(
    invocation: &Invocation,
    facts: &DeviceFacts,
) -> Result<ActiveStimulus, String> {
    if !invocation.scenario.supports_host_push_active_stimulus() {
        return Err(
            "active host-push stimulus was requested for an unsupported scenario".to_string(),
        );
    }
    let root = invocation.contract.destination_root.trim_end_matches('/');
    let (calibration_destination, device_destination_path) =
        active_stimulus_device_paths(invocation);
    create_run_scope(invocation, facts)?;
    if fixture_path_exists(invocation, facts, &calibration_destination)?
        || fixture_path_exists(invocation, facts, &device_destination_path)?
    {
        return Err("active host-push stimulus found pre-existing fixture state".to_string());
    }

    let host_workspace = tempfile::Builder::new()
        .prefix("emuchef-phase6d6-active-")
        .tempdir()
        .map_err(|_| "active host workspace could not be created".to_string())?;
    let calibration_source = host_workspace.path().join(ACTIVE_CALIBRATION_SOURCE_FILE);
    let host_source_path = host_workspace.path().join(ACTIVE_SOURCE_FILE);
    let seed = active_host_seed(&invocation.run_scope);
    write_active_host_fixture(
        &calibration_source,
        ACTIVE_CALIBRATION_KIB.saturating_mul(1024),
        seed,
    )?;

    let mut calibration_device = RealAdbDevice::new("adb", Some(facts.serial.clone()));
    let calibration_started = Instant::now();
    let push_result = calibration_device.push(&calibration_source, &calibration_destination, false);
    let elapsed_ms = u64::try_from(calibration_started.elapsed().as_millis())
        .map_err(|_| "active host-push calibration duration was not representable".to_string())?;
    let destination_verified = push_result.is_ok()
        && device_file_size_bytes(&facts.serial, &calibration_destination).ok()
            == Some(ACTIVE_CALIBRATION_KIB.saturating_mul(1024));
    let device_cleanup = calibration_device.remove_file(&calibration_destination);
    let host_cleanup = fs::remove_file(&calibration_source);
    if push_result.is_err() {
        return Err("active host-push calibration failed".to_string());
    }
    if !destination_verified {
        return Err("active host-push calibration destination could not be verified".to_string());
    }
    if device_cleanup.is_err()
        || fixture_path_exists(invocation, facts, &calibration_destination).unwrap_or(true)
        || (host_cleanup.is_err() && calibration_source.exists())
    {
        return Err("active host-push calibration cleanup failed".to_string());
    }

    let derived = derive_active_stimulus(ACTIVE_CALIBRATION_KIB, elapsed_ms)?;
    let free_kib = free_space_kib(&facts.serial, root)?;
    let required_kib = derived
        .payload_kib
        .checked_add(ACTIVE_CLEANUP_HEADROOM_KIB)
        .ok_or_else(|| "active host-push free-space requirement overflowed".to_string())?;
    if free_kib < required_kib {
        return Err(format!(
            "active host-push qualification requires at least {required_kib} KiB free after calibration"
        ));
    }
    write_active_host_fixture(
        &host_source_path,
        derived.payload_kib.saturating_mul(1024),
        seed ^ 0xA5A5_5A5A_D3C4_B2E1,
    )?;

    Ok(ActiveStimulus {
        host_workspace,
        host_source_path,
        device_destination_path,
        payload_kib: derived.payload_kib,
        predicted_ms: derived.predicted_ms,
    })
}

fn prepare_timeout_stimulus(
    invocation: &Invocation,
    facts: &DeviceFacts,
) -> Result<TimeoutStimulus, String> {
    let stimulus = timeout_stimulus_for_scenario(invocation.scenario)?;
    create_run_scope(invocation, facts)?;
    Ok(stimulus)
}

/// Validate that a scenario may use the private pseudo-device copy and build
/// the stimulus without touching ADB.
fn timeout_stimulus_for_scenario(scenario: Scenario) -> Result<TimeoutStimulus, String> {
    if !scenario.requires_device_copy_stimulus() {
        return Err(
            "timeout pseudo-device stimulus was requested for an unsupported scenario".to_string(),
        );
    }
    if !is_timeout_source_path(TIMEOUT_SOURCE_PATH)
        || !is_timeout_destination_path(TIMEOUT_DESTINATION_PATH)
    {
        return Err(
            "timeout pseudo-device stimulus path escaped its private constants".to_string(),
        );
    }
    Ok(TimeoutStimulus {
        device_source_path: TIMEOUT_SOURCE_PATH.to_string(),
        device_destination_path: TIMEOUT_DESTINATION_PATH.to_string(),
    })
}

fn write_low_storage_host_payload(
    path: &Path,
    payload_bytes: u64,
    run_scope: &str,
) -> Result<(), String> {
    // A zero-filled set_len payload can consume substantially fewer device
    // blocks than its logical length on /storage/emulated. Reuse the
    // deterministic nontrivial fixture writer so the qualification forces
    // real data allocation before the post-push fsync durability boundary.
    let seed = active_host_seed(run_scope) ^ 0x4C4F_575F_5354_4F52;
    write_active_host_fixture(path, payload_bytes, seed)
        .map_err(|_| "low-storage host payload could not be written".to_string())
}

fn create_low_storage_host_payload(
    invocation: &Invocation,
    payload_kib: u64,
) -> Result<PathBuf, String> {
    if payload_kib == 0 || payload_kib > MAX_LOW_STORAGE_HOST_PAYLOAD_KIB {
        return Err("low-storage host payload exceeded its explicit bound".to_string());
    }
    let payload_bytes = payload_kib
        .checked_mul(1024)
        .ok_or_else(|| "low-storage host payload size overflowed".to_string())?;
    let scope_suffix = invocation
        .run_scope
        .rsplit('/')
        .next()
        .ok_or_else(|| "low-storage run scope was missing its unique suffix".to_string())?;
    let path = fixture_root().join(format!(".phase6d6-low-storage-payload-{scope_suffix}.bin"));
    write_low_storage_host_payload(&path, payload_bytes, &invocation.run_scope)?;
    Ok(path)
}

fn prepare_low_storage(
    invocation: &Invocation,
    facts: &DeviceFacts,
) -> Result<StorageObservation, String> {
    let root = invocation.contract.destination_root.trim_end_matches('/');
    let initial_free_kib = free_space_kib(&facts.serial, root)?;
    let reserve_path = format!("{}/{}", invocation.run_scope, STORAGE_RESERVE_FILE);
    let filler_path = format!("{}/{}", invocation.run_scope, STORAGE_FILL_FILE);
    validate_low_storage_preflight(LowStoragePreflight {
        initial_free_kib,
        recovery_reserve_kib: RECOVERY_RESERVE_KIB,
        filler_kib: bounded_storage_filler_kib(
            initial_free_kib.saturating_sub(RECOVERY_RESERVE_KIB),
        ),
        max_filler_kib: MAX_STORAGE_FILLER_KIB,
        reserve_owned: fixture_owned_run_path(&reserve_path),
        filler_owned: fixture_owned_run_path(&filler_path),
    })?;
    create_run_scope(invocation, facts)?;
    create_bounded_device_file(invocation, facts, &reserve_path, RECOVERY_RESERVE_KIB)?;
    if !fixture_path_exists(invocation, facts, &reserve_path)? {
        return Err("one-GiB recovery reserve could not be verified".to_string());
    }
    let after_reserve = free_space_kib(&facts.serial, root)?;
    let filler_kib = bounded_storage_filler_kib_from_observations(initial_free_kib, after_reserve);
    if filler_kib == 0 {
        return Err(
            "bounded filler could not be allocated while retaining recovery reserve".to_string(),
        );
    }
    validate_low_storage_preflight(LowStoragePreflight {
        initial_free_kib,
        recovery_reserve_kib: RECOVERY_RESERVE_KIB,
        filler_kib,
        max_filler_kib: MAX_STORAGE_FILLER_KIB,
        reserve_owned: true,
        filler_owned: true,
    })?;
    create_bounded_device_file(invocation, facts, &filler_path, filler_kib)?;
    if !fixture_path_exists(invocation, facts, &filler_path)? {
        return Err("bounded low-storage filler could not be verified".to_string());
    }
    let post_filler_total_free_kib = total_free_space_kib(&facts.serial, root)?;
    let payload_kib = bounded_low_storage_payload_kib(post_filler_total_free_kib)?;
    Ok(StorageObservation {
        initial_free_kib,
        recovery_reserve_kib: RECOVERY_RESERVE_KIB,
        filler_kib,
        payload_kib,
        final_free_kib: None,
        restored_recovery_reserve_kib: None,
        reserve_created: true,
        reserve_removed: false,
        ownership_verified: true,
        bounded_allocation: filler_kib <= MAX_STORAGE_FILLER_KIB,
        cleanup_verified: false,
    })
}

fn platform_tools_revision() -> String {
    let output = Command::new("adb").arg("version").output();
    let Ok(output) = output else {
        return "unavailable".to_string();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next() == Some("Android") && parts.next() == Some("Debug"))
                .then(|| sanitize_fact(line))
        })
        .unwrap_or_else(|| "unreported".to_string())
}

fn host_facts() -> Value {
    let version = if cfg!(target_os = "macos") {
        Command::new("sw_vers")
            .args(["-productVersion"])
            .output()
            .ok()
            .map(|output| sanitize_fact(&String::from_utf8_lossy(&output.stdout)))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unreported".to_string())
    } else {
        "unreported".to_string()
    };
    json!({
        "os": std::env::consts::OS,
        "version": version,
        "architecture": std::env::consts::ARCH,
    })
}

fn opt_ins(invocation: &Invocation) -> Vec<String> {
    let mut values = vec![
        "EMUCHEF_RUN_REAL_ADB_TESTS=1".to_string(),
        format!("{PHASE_OPT_IN}=1"),
        format!("{SCENARIO_ENV}={}", invocation.scenario.as_str()),
        format!("{REPETITION_ENV}={}", invocation.repetition),
        format!("{SERIAL_ENV}=selected"),
        format!("{PACKAGE_ALLOWLIST_ENV}={FIXTURE_PACKAGE}"),
        format!("{SENTINEL_DIR_ENV}=test-owned-directory"),
    ];
    if invocation.scenario.is_root() {
        values.extend([
            format!("{ROOT_OPT_IN}=1"),
            format!("{ROOT_DESTRUCTIVE_OPT_IN}=1"),
            format!("{ROOT_PREFIX_ALLOWLIST_ENV}=committed-root-prefixes"),
        ]);
    }
    if let Some(opt_in) = invocation.scenario.requires_destructive_opt_in() {
        values.push(format!("{opt_in}=1"));
    }
    values
}

fn timestamp() -> String {
    format!(
        "unix:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    )
}

fn git_head() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| sanitize_fact(&String::from_utf8_lossy(&output.stdout)))
        .filter(|value| value.len() == 40)
        .unwrap_or_else(|| "unreported".to_string())
}

fn digest(value: &str) -> String {
    digest_bytes(value.as_bytes())
}

fn digest_bytes(value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Recursively sort object keys before hashing so Rust evidence digests match
/// the dependency-free Node validator's canonical JSON algorithm.
fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_value(&values[key]));
            }
            Value::Object(canonical)
        }
        _ => value.clone(),
    }
}

fn canonical_value_digest(value: &Value) -> String {
    let encoded = serde_json::to_vec(&canonical_value(value))
        .expect("qualification evidence values must serialize");
    digest_bytes(&encoded)
}

fn canonical_record_digest(record: &Value) -> String {
    let mut content = record.clone();
    content
        .as_object_mut()
        .expect("qualification record must be an object")
        .remove("recordDigest");
    canonical_value_digest(&content)
}

fn sanitize_fact(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | '-' | ':' | '/' | '+' | ',' | ' ')
        })
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_inventory_is_exact_and_requires_two_repetitions() {
        assert_eq!(SCENARIOS.len(), 13);
        assert_eq!(Scenario::ALL.len(), 13);
        assert!(Scenario::parse("low_storage").is_ok());
        assert!(Scenario::parse("not-a-scenario").is_err());
        assert_eq!(SENTINEL_TIMEOUT, Duration::from_secs(600));
    }

    #[test]
    fn serialized_identity_contracts_match_the_authoritative_manifest_shape() {
        let manifest = serde_json::from_str::<Value>(SCENARIO_MANIFEST)
            .expect("the Phase 6D.6 scenario manifest must be valid JSON");
        for scenario in [Scenario::IdentityStability, Scenario::IdentityReplacement] {
            let expected = &manifest["scenarioContracts"][scenario.as_str()];
            let actual = serde_json::to_value(scenario_contract(scenario))
                .expect("the scenario contract must serialize");
            assert_eq!(actual, *expected);
        }
    }

    #[test]
    fn serial_digest_and_sanitized_facts_never_return_raw_identity() {
        let serial = "physical-serial-42";
        assert_ne!(digest(serial), serial);
        assert_eq!(
            sanitize_fact(" model\nwith\tunsafe\0bytes "),
            "modelwithunsafebytes"
        );
    }

    #[test]
    fn root_preflight_matches_the_production_magisk_su_probe() {
        assert_eq!(PRODUCTION_ROOT_PROBE_ARGS, ["shell", "su", "-c", "id"]);
        assert!(production_root_probe_granted(
            "uid=0(root) gid=0(root) groups=0(root)\n"
        ));
        assert!(production_root_probe_granted("UID=0(ROOT) GID=0(ROOT)\n"));
        assert!(!production_root_probe_granted("0\n"));
        assert!(!production_root_probe_granted(
            "uid=2000(shell) gid=2000(shell)\n"
        ));
    }

    #[test]
    fn issue_codes_preserve_storage_and_terminal_precedence() {
        assert_eq!(
            issue_code(StepFailureKind::DeviceStorageExhausted),
            "device_storage_exhausted"
        );
        assert_eq!(
            issue_code(StepFailureKind::OperationTimedOut),
            "operation_timed_out"
        );
        assert_eq!(
            issue_code(StepFailureKind::TransportFailure),
            "device_transport_lost"
        );
    }

    #[test]
    fn blocked_or_failed_invocations_are_non_successful_test_results() {
        for outcome in ["blocked", "failed", "skipped"] {
            assert!(qualification_test_result(&json!({ "outcome": outcome })).is_err());
        }
        assert!(qualification_test_result(&json!({ "outcome": "passed" })).is_ok());
    }

    #[test]
    fn explicitly_invoked_ignored_harness_fails_when_a_gate_is_missing() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = Command::new(executable)
            .args([
                "--exact",
                "executor_real_adb_tests::physical_interruption_qualification::manual_phase_6d6_physical_interruption_qualification",
                "--ignored",
                "--nocapture",
            ])
            .env_remove("EMUCHEF_RUN_REAL_ADB_TESTS")
            .env_remove(PHASE_OPT_IN)
            .env_remove(SCENARIO_ENV)
            .env_remove(REPETITION_ENV)
            .env_remove(SERIAL_ENV)
            .env_remove(PACKAGE_ALLOWLIST_ENV)
            .env_remove(SENTINEL_DIR_ENV)
            .output()
            .expect("the ignored harness subprocess should start");
        assert!(
            !output.status.success(),
            "a blocked explicit invocation must be a non-successful test"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("blocked") || stderr.contains("must equal"));
    }

    #[test]
    fn boundary_checkpoint_matches_the_first_step_id_with_one_based_progress_index() {
        let event = ExecutionProgressEvent {
            step_index: 1,
            total_steps: 2,
            step_id: "phase6d6/cancellation_boundary/first".to_string(),
            recipe_ref: "fixture.phase6d6".to_string(),
            step_name: "first".to_string(),
            note: "first".to_string(),
            phase: ProgressPhase::Finished,
            status: None,
            message: None,
        };

        assert!(is_boundary_checkpoint_event(
            Scenario::CancellationBoundary,
            "phase6d6/cancellation_boundary/first",
            &event,
        ));
        assert!(!is_boundary_checkpoint_event(
            Scenario::CancellationBoundary,
            "phase6d6/cancellation_boundary/second",
            &event,
        ));
        assert!(!is_boundary_checkpoint_event(
            Scenario::CancellationActive,
            "phase6d6/cancellation_boundary/first",
            &event,
        ));
    }

    #[test]
    fn usb_disconnect_boundary_waits_for_terminal_recovery_before_cleanup() {
        assert!(Scenario::UsbDisconnectActive.requires_terminal_recovery());
        assert!(Scenario::UsbDisconnectBoundary.requires_terminal_recovery());
        assert!(!Scenario::CancellationBoundary.requires_terminal_recovery());
    }

    #[test]
    fn sentinel_action_allows_an_in_progress_marker_write_to_settle() {
        let directory = tempfile::tempdir().expect("sentinel directory should be available");
        let sentinel = Sentinel {
            directory: directory.path().to_path_buf(),
        };
        let marker = sentinel.path("cleanup-ready");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .expect("the operator marker should become visible before its write completes");
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            fs::write(marker, "ack\n").expect("the operator marker write should complete");
        });

        assert!(sentinel
            .named_action_now("cleanup-ready")
            .expect("a completing marker write should be accepted")
            .is_some());
        writer.join().expect("operator marker writer should finish");
        sentinel
            .cleanup()
            .expect("sentinel markers should clean up");
    }

    #[test]
    fn root_cleanup_checkpoint_exposes_terminal_result_before_accepting_cleanup_ack() {
        let directory = tempfile::tempdir().expect("sentinel directory should be available");
        let sentinel = Sentinel {
            directory: directory.path().to_path_buf(),
        };
        let operator = sentinel.clone();
        let operator_thread = std::thread::spawn(move || {
            let started = Instant::now();
            while !operator.path("terminal-ready").exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "terminal-ready should be exposed before cleanup authorization"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            operator
                .mark("cleanup-ready", "ack\n")
                .expect("operator cleanup acknowledgement should be recorded");
        });

        wait_for_root_cleanup_authority(&sentinel)
            .expect("fresh cleanup acknowledgement should release the checkpoint");
        operator_thread
            .join()
            .expect("operator checkpoint thread should finish");
        assert!(
            sentinel.marker_time("terminal-ready").unwrap()
                <= sentinel.marker_time("cleanup-ready").unwrap()
        );
        sentinel
            .cleanup()
            .expect("sentinel markers should clean up");
    }

    #[test]
    fn scenario_contract_accepts_expected_failure_but_rejects_mismatch() {
        let contract = scenario_contract(Scenario::OperationTimeout);
        let run_scope = format!("run-scope-sha256:{}", "a".repeat(64));
        let slot = json!({
            "acquired": true,
            "released": true,
            "runId": run_scope,
            "executionId": format!("execution-sha256:{}", "b".repeat(64)),
            "acquiredAt": "unix:1",
            "terminalCleanupAt": "unix:3",
            "releasedAt": "unix:3",
            "sourceKind": "production_owned_slot",
            "evidence": "production-execution-session-slot",
        });
        let mut expected = json!({
            "success": false,
            "issue": "operation_timed_out",
            "stepStates": {"executed": 0, "skipped": 0, "failed": 1, "cancelled": 0, "blocked": 0, "notAttempted": 1},
            "partialChangesPossible": false,
            "authorityInvalidated": true,
            "activeSlotReleased": true,
            "runScope": run_scope,
            "activeSlotObservation": slot,
            "activeProcess": {
                "runId": run_scope,
                "operationId": format!("operation-sha256:{}", "e".repeat(64)),
                "operationClass": "device_copy",
                "childIdentity": format!("child-sha256:{}", "f".repeat(64)),
                "spawnedAt": "unix:1",
                "mutationStartedAt": "unix:1",
                "checkedAliveAt": "unix:2",
                "actionAt": "unix:2",
                "actionKind": "deadline_reached",
                "terminalAt": "unix:3",
                "aliveImmediatelyBeforeAction": true,
                "terminalReportedBeforeAction": false,
            },
            "timeout": {
                "productionDeadlineMs": 300000,
                "qualificationDeadlineMs": 15000,
                "deadlineSource": "test_only_scoped_override",
                "processCleanup": "confirmed"
            },
            "cleanup": "succeeded",
            "residual": "clean"
        });
        assert!(evaluate_scenario_contract(&contract, &expected).is_ok());
        expected["timeout"]["extra"] = json!(true);
        assert!(evaluate_scenario_contract(&contract, &expected).is_err());
        expected["timeout"] = json!({
            "productionDeadlineMs": 300000,
            "qualificationDeadlineMs": 15000,
            "deadlineSource": "test_only_scoped_override",
            "processCleanup": "uncertain"
        });
        assert!(evaluate_scenario_contract(&contract, &expected).is_err());
        expected["timeout"] = json!({
            "productionDeadlineMs": 300000,
            "qualificationDeadlineMs": 15000,
            "deadlineSource": "test_only_scoped_override",
            "processCleanup": "confirmed"
        });
        expected["activeProcess"]
            .as_object_mut()
            .unwrap()
            .remove("actionKind");
        assert!(evaluate_scenario_contract(&contract, &expected).is_err());
        assert!(evaluate_scenario_contract(
            &contract,
            &json!({
                "success": true,
                "issue": null,
                "stepStates": {"executed": 2, "skipped": 0, "failed": 0, "cancelled": 0, "blocked": 0, "notAttempted": 0},
                "partialChangesPossible": false,
                "authorityInvalidated": false,
                "activeSlotReleased": true,
                "runScope": run_scope,
                "activeSlotObservation": slot,
                "cleanup": "succeeded",
                "residual": "clean"
            })
        )
        .is_err());
        assert!(evaluate_scenario_contract(
            &contract,
            &json!({
                "success": false,
                "issue": "device_storage_exhausted",
                "stepStates": {"executed": 0, "skipped": 0, "failed": 1, "cancelled": 0, "blocked": 0, "notAttempted": 1},
                "partialChangesPossible": false,
                "authorityInvalidated": true,
                "activeSlotReleased": true,
                "runScope": run_scope,
                "activeSlotObservation": slot,
                "cleanup": "succeeded",
                "residual": "clean"
            })
        )
        .is_err());

        let stable_contract = scenario_contract(Scenario::IdentityStability);
        assert!(evaluate_scenario_contract(
            &stable_contract,
            &json!({
                "success": true,
                "issue": null,
                "stepStates": {"executed": 2, "skipped": 0, "failed": 0, "cancelled": 0, "blocked": 0, "notAttempted": 0},
                "partialChangesPossible": false,
                "authorityInvalidated": false,
                "activeSlotReleased": true,
                "runScope": run_scope,
                "activeSlotObservation": slot,
                "identityTransition": {
                    "initialSerial": format!("serial-sha256:{}", "c".repeat(64)),
                    "replacementSerial": format!("serial-sha256:{}", "c".repeat(64)),
                    "initialFingerprint": format!("fingerprint-sha256:{}", "d".repeat(64)),
                    "replacementFingerprint": format!("fingerprint-sha256:{}", "d".repeat(64)),
                    "originalDisconnectedAt": "unix:2",
                    "serialAbsentFrom": "unix:2",
                    "serialAbsentUntil": "unix:3",
                    "replacementAttachedAt": "unix:4",
                    "neverSimultaneous": true,
                    "authorityInvalidated": false,
                    "expectedIssueCode": null,
                    "runId": run_scope,
                },
                "cleanup": "succeeded",
                "residual": "clean"
            })
        )
        .is_ok());
    }

    #[test]
    fn host_sleep_classification_uses_deadline_clock_advancement_not_terminal_outcome() {
        assert_eq!(
            classify_host_sleep_clock(HostSleepClockMeasurement {
                suspended_wall_ms: 10_000,
                deadline_clock_advance_ms: 25,
                remaining_before_sleep_ms: 20_000,
                remaining_after_wake_ms: 19_975,
                tolerance_ms: 100,
            }),
            HostSleepClockClassification::SuspendedTimeExcluded
        );
        assert_eq!(
            classify_host_sleep_clock(HostSleepClockMeasurement {
                suspended_wall_ms: 10_000,
                deadline_clock_advance_ms: 9_990,
                remaining_before_sleep_ms: 20_000,
                remaining_after_wake_ms: 10_010,
                tolerance_ms: 100,
            }),
            HostSleepClockClassification::SuspendedTimeIncluded
        );
        assert_eq!(
            classify_host_sleep_clock(HostSleepClockMeasurement {
                suspended_wall_ms: 10_000,
                deadline_clock_advance_ms: 9_990,
                remaining_before_sleep_ms: 20_000,
                remaining_after_wake_ms: 19_975,
                tolerance_ms: 100,
            }),
            HostSleepClockClassification::Contradictory
        );
    }

    #[test]
    fn active_contract_requires_the_exact_live_child_before_operator_action() {
        let contract = scenario_contract(Scenario::CancellationActive);
        let run_scope = format!("run-scope-sha256:{}", "a".repeat(64));
        let mut observed = json!({
            "success": false,
            "issue": null,
            "stepStates": {"executed": 1, "skipped": 0, "failed": 0, "cancelled": 1, "blocked": 0, "notAttempted": 0},
            "partialChangesPossible": false,
            "authorityInvalidated": false,
            "activeSlotReleased": true,
            "runScope": run_scope,
            "deviceScope": format!("serial-sha256:{}", "b".repeat(64)),
            "activeSlotObservation": {
                "acquired": true,
                "released": true,
                "runId": run_scope,
                "executionId": format!("execution-sha256:{}", "c".repeat(64)),
                "acquiredAt": "unix:1",
                "terminalCleanupAt": "unix:6",
                "releasedAt": "unix:6",
                "sourceKind": "production_owned_slot",
                "evidence": "production-execution-session-slot",
            },
            "activeProcess": {
                "runId": run_scope,
                "operationId": format!("operation-sha256:{}", "d".repeat(64)),
                "operationClass": "host_push",
                "childIdentity": format!("child-sha256:{}", "e".repeat(64)),
                "spawnedAt": "unix:1",
                "mutationStartedAt": "unix:2",
                "checkedAliveAt": "unix:3",
                "actionAt": "unix:4",
                "terminalAt": "unix:5",
                "aliveImmediatelyBeforeAction": true,
                "terminalReportedBeforeAction": false,
            },
            "activeCancellation": {
                "requestPhase": "in_flight",
                "requestBeforeFinished": true,
                "laterWorkNotAttempted": true,
            },
            "cleanup": "succeeded",
            "residual": "clean",
        });
        assert!(evaluate_scenario_contract(&contract, &observed).is_ok());

        observed["activeProcess"]["terminalAt"] = json!("unix:4");
        assert!(evaluate_scenario_contract(&contract, &observed).is_err());
        observed["activeProcess"]["terminalAt"] = json!("unix:5");
        observed["activeProcess"]["runId"] = json!(format!("run-scope-sha256:{}", "f".repeat(64)));
        assert!(evaluate_scenario_contract(&contract, &observed).is_err());
        observed["activeProcess"] = Value::Null;
        assert!(evaluate_scenario_contract(&contract, &observed).is_err());
    }

    #[test]
    fn active_host_fixture_is_exact_deterministic_and_nontrivial() {
        let temp = tempfile::tempdir().expect("fixture tempdir should be created");
        let first = temp.path().join("first.bin");
        let second = temp.path().join("second.bin");
        write_active_host_fixture(&first, 131_089, 7).expect("first fixture should be generated");
        write_active_host_fixture(&second, 131_089, 7).expect("second fixture should be generated");
        let first_bytes = fs::read(&first).expect("first fixture should be readable");
        let second_bytes = fs::read(&second).expect("second fixture should be readable");
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(first_bytes.len(), 131_089);
        assert!(first_bytes.iter().any(|byte| *byte != 0));
        assert_ne!(&first_bytes[..64], &first_bytes[64..128]);
    }

    #[test]
    fn active_host_fixture_removes_partial_file_after_write_failure() {
        let temp = tempfile::tempdir().expect("fixture tempdir should be created");
        let path = temp.path().join("partial.bin");
        let result = write_active_host_fixture_with(&path, 4096, 9, |file, _, _| {
            file.write_all(&[1_u8; 32])?;
            Err(io::Error::other("injected fixture write failure"))
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn active_host_fixture_changes_with_run_seed() {
        let temp = tempfile::tempdir().expect("fixture tempdir should be created");
        let first = temp.path().join("first.bin");
        let second = temp.path().join("second.bin");
        write_active_host_fixture(&first, 4096, 1).expect("first fixture should be generated");
        write_active_host_fixture(&second, 4096, 2).expect("second fixture should be generated");
        assert_ne!(
            fs::read(first).expect("first fixture should be readable"),
            fs::read(second).expect("second fixture should be readable")
        );
    }

    #[test]
    fn active_host_cleanup_removes_only_the_run_owned_workspace() {
        let parent = tempfile::tempdir().expect("parent tempdir should be created");
        let sibling = parent.path().join("sibling.txt");
        fs::write(&sibling, b"preserve").expect("sibling should be created");
        let workspace = tempfile::Builder::new()
            .prefix("active-workspace-")
            .tempdir_in(parent.path())
            .expect("active workspace should be created");
        let source = workspace.path().join(ACTIVE_SOURCE_FILE);
        fs::write(&source, b"payload").expect("active source should be created");
        let workspace_path = workspace.path().to_path_buf();
        let stimulus = ActiveStimulus {
            host_workspace: workspace,
            host_source_path: source,
            device_destination_path: "/fixture/owned/destination".to_string(),
            payload_kib: 1,
            predicted_ms: 1,
        };

        assert!(cleanup_active_host_stimulus(&stimulus).is_empty());
        assert!(!workspace_path.exists());
        assert_eq!(fs::read(&sibling).unwrap(), b"preserve");
    }

    #[test]
    fn active_scenarios_use_the_reviewed_host_push_operation() {
        for scenario in [
            Scenario::CancellationActive,
            Scenario::UsbDisconnectActive,
            Scenario::DeviceOffline,
        ] {
            assert_eq!(active_process_operation(scenario), ProcessOperation::Push);
            assert_eq!(active_operation_class(scenario), "host_push");
        }
        assert_eq!(
            active_process_operation(Scenario::OperationTimeout),
            ProcessOperation::DeviceCopy
        );
        assert_eq!(
            active_operation_class(Scenario::OperationTimeout),
            "device_copy"
        );
    }

    #[test]
    fn timeout_observer_is_separate_from_host_push_stimulus_support() {
        assert!(!Scenario::OperationTimeout.supports_host_push_active_stimulus());
        assert!(Scenario::OperationTimeout.requires_process_observer());
        assert!(Scenario::CancellationActive.supports_host_push_active_stimulus());
        assert!(Scenario::CancellationActive.requires_process_observer());
        assert!(!Scenario::DeviceUnauthorized.requires_process_observer());
    }

    fn timeout_test_invocation(scenario: Scenario) -> (Invocation, DeviceFacts) {
        let contract = load_contract().expect("the committed qualification contract should load");
        let run_scope = format!(
            "{}/phase6d6-operation-timeout-test",
            contract.destination_root.trim_end_matches('/')
        );
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let invocation = Invocation {
            scenario,
            repetition: 1,
            serial: "selected-serial".to_string(),
            sentinel: Sentinel {
                directory: sentinel_directory.path().to_path_buf(),
            },
            contract,
            run_scope: run_scope.clone(),
            run_id: "physical-run-sha256:test".to_string(),
            sentinel_id: "sentinel-sha256:test".to_string(),
            sentinel_nonce: "nonce-sha256:test".to_string(),
            evidence_path: "evidence.json".to_string(),
            trace_path: "trace.json".to_string(),
            scenario_contract: scenario_contract(scenario),
        };
        let facts = DeviceFacts {
            serial: invocation.serial.clone(),
            manufacturer: "fixture".to_string(),
            model: "fixture".to_string(),
            android_version: "35".to_string(),
            api_level: 35,
            abi: "arm64-v8a".to_string(),
            build_fingerprint: "fixture/fingerprint".to_string(),
            root_shell: false,
            root_version: None,
        };
        (invocation, facts)
    }

    #[test]
    fn timeout_stimulus_paths_are_exact_private_pseudo_devices_and_ignore_environment() {
        assert_eq!(TIMEOUT_SOURCE_PATH, "/dev/zero");
        assert_eq!(TIMEOUT_DESTINATION_PATH, "/dev/null");
        assert_eq!(TIMEOUT_QUALIFICATION_DEADLINE, Duration::from_secs(15));
        assert_eq!(TIMEOUT_LIVENESS_SAMPLE_OFFSET, Duration::from_secs(12));
        assert_eq!(
            ProcessOperation::DeviceCopy.deadline(),
            Duration::from_secs(300)
        );
        assert!(is_timeout_source_path(TIMEOUT_SOURCE_PATH));
        assert!(!is_timeout_source_path("/dev/random"));
        assert!(is_timeout_destination_path(TIMEOUT_DESTINATION_PATH));
        assert!(!is_timeout_destination_path("/dev/null/"));

        env::set_var("EMUCHEF_PHASE_6D6_TIMEOUT_SOURCE_PATH", "/dev/random");
        env::set_var("EMUCHEF_PHASE_6D6_TIMEOUT_DESTINATION_PATH", "/dev/zero");
        assert_eq!(TIMEOUT_SOURCE_PATH, "/dev/zero");
        assert_eq!(TIMEOUT_DESTINATION_PATH, "/dev/null");
        env::remove_var("EMUCHEF_PHASE_6D6_TIMEOUT_SOURCE_PATH");
        env::remove_var("EMUCHEF_PHASE_6D6_TIMEOUT_DESTINATION_PATH");
    }

    #[test]
    fn timeout_pseudo_device_paths_are_not_fixture_owned_and_absent_from_cleanup_inventory() {
        let contract = load_contract().expect("the committed qualification contract should load");
        assert!(validate_owned_destination(&contract, TIMEOUT_SOURCE_PATH, false,).is_err());
        assert!(validate_owned_destination(&contract, TIMEOUT_DESTINATION_PATH, false,).is_err());

        let first = "/sdcard/EmuChefQualification/com.emuchef.fixture/output/phase6d6-run-a/phase6d6-first.txt";
        let second = "/sdcard/EmuChefQualification/com.emuchef.fixture/output/phase6d6-run-a/phase6d6-second.txt";
        let run_scope = "/sdcard/EmuChefQualification/com.emuchef.fixture/output/phase6d6-run-a";
        let timeout =
            cleanup_device_paths_for_scenario(Scenario::OperationTimeout, first, second, run_scope);
        assert_eq!(timeout, vec![first.to_string(), second.to_string()]);
        let ordinary = cleanup_device_paths_for_scenario(
            Scenario::CancellationBoundary,
            first,
            second,
            run_scope,
        );
        assert_eq!(ordinary, vec![first.to_string(), second.to_string()]);
    }

    #[test]
    fn timeout_plan_source_location_changes_only_the_timeout_scenario() {
        assert_eq!(
            reviewed_plan_source_location(Scenario::OperationTimeout),
            "device"
        );
        assert_eq!(
            reviewed_plan_source_location(Scenario::HostSleepBeforeDeadline),
            "device"
        );
        assert_eq!(
            reviewed_plan_source_location(Scenario::HostSleepAfterDeadline),
            "device"
        );
        for scenario in [
            Scenario::CancellationActive,
            Scenario::CancellationBoundary,
            Scenario::LowStorage,
        ] {
            assert_eq!(reviewed_plan_source_location(scenario), "host");
        }
    }

    #[test]
    fn timeout_reviewed_plan_uses_exact_pseudo_device_copy_and_keeps_second_step_dependent() {
        let (invocation, facts) = timeout_test_invocation(Scenario::OperationTimeout);
        let timeout = TimeoutStimulus {
            device_source_path: TIMEOUT_SOURCE_PATH.to_string(),
            device_destination_path: TIMEOUT_DESTINATION_PATH.to_string(),
        };
        let plan = reviewed_plan(&invocation, &facts, None, None, Some(&timeout));

        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[1].dependencies, vec![plan.steps[0].id.clone()]);
        match plan.steps[0].params.get("source") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value["value"], json!(TIMEOUT_SOURCE_PATH));
                assert_eq!(value["location"], Value::String("device".to_string()));
            }
            _ => panic!("timeout first source should remain a literal runtime value"),
        }
        match plan.steps[0].params.get("dest") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value, &json!(TIMEOUT_DESTINATION_PATH));
            }
            _ => panic!("timeout first destination should remain a literal runtime value"),
        }
        match plan.steps[0].params.get("copy_policy") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value, &json!("merge"));
            }
            _ => panic!("timeout copy policy should remain a literal runtime value"),
        }
        assert_eq!(
            plan.steps[0].verify[0].params.get("path"),
            Some(&json!(TIMEOUT_DESTINATION_PATH))
        );
        match plan.steps[1].params.get("dest") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value, &json!(run_scope_paths(&invocation).1));
            }
            _ => panic!("timeout second destination should remain run-scoped"),
        }
    }

    #[test]
    fn timeout_plan_production_device_copy_path_never_unlinks_or_recurses_on_pseudo_devices() {
        let (invocation, facts) = timeout_test_invocation(Scenario::OperationTimeout);
        let timeout = TimeoutStimulus {
            device_source_path: TIMEOUT_SOURCE_PATH.to_string(),
            device_destination_path: TIMEOUT_DESTINATION_PATH.to_string(),
        };
        let plan = reviewed_plan(&invocation, &facts, None, None, Some(&timeout));
        let sandbox = tempfile::tempdir().expect("qualification sandbox should be available");
        let runner = ExecutorRunner::new(ExecutorAdapters::with_device_and_sandbox_roots(
            FakeDryRunDevice::default(),
            sandbox.path().join("runtime"),
            sandbox.path().join("cache"),
            sandbox.path().join("fake-device"),
            vec![fixture_root()],
            false,
        ));
        let mut runner = runner;
        let _result = runner.run(&plan);
        let commands = runner.adapters().device().commands().to_vec();

        // The exact production copy is a non-recursive, non-privileged
        // `copy_on_device("/dev/zero", "/dev/null")`; no replace/remove or
        // special-file creation path can precede it.
        assert!(commands.iter().any(|command| command
            == &vec![
                "copy_on_device".to_string(),
                TIMEOUT_SOURCE_PATH.to_string(),
                TIMEOUT_DESTINATION_PATH.to_string(),
                "False".to_string(),
                "False".to_string(),
            ]));
        assert!(!commands.iter().any(|command| matches!(
            command.first().map(String::as_str),
            Some("remove_file" | "remove_tree" | "mknod")
        )));
        for pseudo_device in [TIMEOUT_SOURCE_PATH, TIMEOUT_DESTINATION_PATH] {
            assert!(!commands.iter().any(|command| {
                command.first().map(String::as_str) == Some("remove_file")
                    && command.get(1).map(String::as_str) == Some(pseudo_device)
            }));
            assert!(!commands.iter().any(|command| {
                command.first().map(String::as_str) == Some("mkdir_p")
                    && command.get(1).map(String::as_str) == Some(pseudo_device)
            }));
        }
        // The only non-read-only prerequisite is the ordinary file-copy
        // no-op `mkdir_p("/dev")`; the pseudo-device nodes themselves are only
        // probed read-only and then used as the exact copy source/destination.
        assert!(commands
            .iter()
            .any(|command| command == &vec!["mkdir_p".to_string(), "/dev".to_string(),]));
        assert!(commands.iter().any(|command| command
            == &vec![
                "path_is_dir".to_string(),
                TIMEOUT_DESTINATION_PATH.to_string(),
                "False".to_string(),
            ]));
    }

    #[test]
    fn prepare_timeout_stimulus_rejects_non_timeout_scenarios() {
        let (invocation, facts) = timeout_test_invocation(Scenario::CancellationBoundary);
        let error = prepare_timeout_stimulus(&invocation, &facts)
            .expect_err("non-timeout scenarios must not obtain the pseudo-device stimulus");
        assert!(
            error.contains("unsupported scenario"),
            "unexpected rejection message: {error}"
        );
    }

    #[test]
    #[should_panic(
        expected = "pseudo-device stimulus is only valid for timeout or host-sleep scenarios"
    )]
    fn reviewed_plan_rejects_timeout_stimulus_for_ordinary_scenarios() {
        let (invocation, facts) = timeout_test_invocation(Scenario::CancellationBoundary);
        let timeout = TimeoutStimulus {
            device_source_path: TIMEOUT_SOURCE_PATH.to_string(),
            device_destination_path: TIMEOUT_DESTINATION_PATH.to_string(),
        };
        let _ = reviewed_plan(&invocation, &facts, None, None, Some(&timeout));
    }

    #[test]
    fn timeout_checkpoint_failure_uses_a_timeout_specific_reason() {
        assert_eq!(
            checkpoint_failure_message(Scenario::OperationTimeout),
            "timeout liveness/deadline observation was missing or invalid"
        );
        assert_eq!(
            checkpoint_failure_message(Scenario::HostSleepBeforeDeadline),
            "host-sleep pre-suspend handshake or deadline-clock observation was missing or invalid"
        );
        assert_eq!(
            checkpoint_failure_message(Scenario::HostSleepAfterDeadline),
            "host-sleep pre-suspend handshake or deadline-clock observation was missing or invalid"
        );
        assert_eq!(
            checkpoint_failure_message(Scenario::CancellationActive),
            "bounded operator checkpoint was missing or aborted"
        );
    }

    #[test]
    fn timeout_process_evidence_requires_one_live_child_until_deadline_then_terminal() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(700);
        let base = UNIX_EPOCH + Duration::from_secs(10);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(100),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(12_000),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline: TIMEOUT_QUALIFICATION_DEADLINE,
                at: base + Duration::from_millis(15_000),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(15_100),
            },
        ];
        let evidence = timeout_process_evidence(&events, "run-scope-test")
            .expect("the exact timeout lifecycle should serialize");
        assert_eq!(evidence["operationClass"], "device_copy");
        assert_eq!(evidence["actionKind"], "deadline_reached");
        assert_eq!(evidence["aliveImmediatelyBeforeAction"], true);
        assert_eq!(evidence["terminalReportedBeforeAction"], false);
        assert!(timeout_process_evidence(&events[..3], "run-scope-test").is_none());
    }

    #[test]
    fn timeout_process_evidence_ignores_unrelated_observer_lifecycle_events() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(703);
        let unrelated_id = OwnedProcessOperationId::from_raw_for_test(704);
        let base = UNIX_EPOCH + Duration::from_secs(30);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id: unrelated_id,
                operation: ProcessOperation::Probe,
                at: base,
            },
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(1),
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id: unrelated_id,
                operation: ProcessOperation::Probe,
                at: base + Duration::from_millis(2),
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(3),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: unrelated_id,
                operation: ProcessOperation::Probe,
                at: base + Duration::from_millis(4),
                alive: Some(false),
                terminal_reported: true,
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(5),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id: unrelated_id,
                operation: ProcessOperation::Probe,
                deadline: Duration::from_secs(30),
                at: base + Duration::from_millis(6),
            },
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline: TIMEOUT_QUALIFICATION_DEADLINE,
                at: base + Duration::from_millis(7),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id: unrelated_id,
                operation: ProcessOperation::Probe,
                at: base + Duration::from_millis(8),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(9),
            },
        ];

        assert!(timeout_process_evidence(&events, "run-scope-test").is_some());

        let mut mismatched_target = events.clone();
        mismatched_target.push(OwnedProcessLifecycleEvent::Spawned {
            operation_id,
            operation: ProcessOperation::Push,
            at: base + Duration::from_millis(10),
        });
        assert!(timeout_process_evidence(&mismatched_target, "run-scope-test").is_none());
    }

    #[test]
    fn timeout_process_evidence_rejects_duplicate_target_deadlines() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(705);
        let base = UNIX_EPOCH + Duration::from_secs(40);
        let valid_events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(1),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(2),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline: TIMEOUT_QUALIFICATION_DEADLINE,
                at: base + Duration::from_millis(3),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(4),
            },
        ];
        let mut duplicate_target = valid_events.clone();
        duplicate_target.push(OwnedProcessLifecycleEvent::DeadlineReached {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            deadline: TIMEOUT_QUALIFICATION_DEADLINE,
            at: base + Duration::from_millis(5),
        });
        assert!(timeout_process_evidence(&duplicate_target, "run-scope-test").is_none());

        let mut contradictory_deadline = valid_events.clone();
        contradictory_deadline.push(OwnedProcessLifecycleEvent::DeadlineReached {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            deadline: Duration::from_secs(14),
            at: base + Duration::from_millis(5),
        });
        assert!(timeout_process_evidence(&contradictory_deadline, "run-scope-test").is_none());

        let mut wrong_deadline = valid_events.clone();
        for event in &mut wrong_deadline {
            if let OwnedProcessLifecycleEvent::DeadlineReached { deadline, .. } = event {
                *deadline = Duration::from_secs(14);
            }
        }
        assert!(timeout_process_evidence(&wrong_deadline, "run-scope-test").is_none());

        let mut dead_liveness = valid_events.clone();
        if let Some(OwnedProcessLifecycleEvent::LivenessSampled { alive, .. }) =
            dead_liveness.iter_mut().find(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::LivenessSampled {
                        operation_id: observed,
                        operation: ProcessOperation::DeviceCopy,
                        ..
                    } if *observed == operation_id
                )
            })
        {
            *alive = Some(false);
        }
        assert!(timeout_process_evidence(&dead_liveness, "run-scope-test").is_none());

        let mut terminal_before_deadline = valid_events.clone();
        if let Some(OwnedProcessLifecycleEvent::Terminal { at, .. }) =
            terminal_before_deadline.iter_mut().find(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::Terminal {
                        operation_id: observed,
                        operation: ProcessOperation::DeviceCopy,
                        ..
                    } if *observed == operation_id
                )
            })
        {
            *at = base + Duration::from_millis(2);
        }
        assert!(timeout_process_evidence(&terminal_before_deadline, "run-scope-test").is_none());

        let other_id = OwnedProcessOperationId::from_raw_for_test(706);
        let mut ambiguous_targets = valid_events;
        ambiguous_targets.push(OwnedProcessLifecycleEvent::DeadlineReached {
            operation_id: other_id,
            operation: ProcessOperation::DeviceCopy,
            deadline: TIMEOUT_QUALIFICATION_DEADLINE,
            at: base + Duration::from_millis(6),
        });
        assert!(timeout_process_evidence(&ambiguous_targets, "run-scope-test").is_none());
    }

    #[test]
    fn timeout_process_evidence_rejects_mixed_operation_ids_and_classes() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(701);
        let other_id = OwnedProcessOperationId::from_raw_for_test(702);
        let base = UNIX_EPOCH + Duration::from_secs(20);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(1),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: other_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(2),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline: TIMEOUT_QUALIFICATION_DEADLINE,
                at: base + Duration::from_millis(3),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(4),
            },
        ];
        assert!(timeout_process_evidence(&events, "run-scope-test").is_none());

        let mut wrong_class = events;
        wrong_class.insert(
            0,
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::Push,
                at: base,
            },
        );
        assert!(timeout_process_evidence(&wrong_class, "run-scope-test").is_none());
    }

    #[test]
    fn device_unauthorized_uses_a_safe_boundary_without_active_process_capture() {
        let scenario = Scenario::DeviceUnauthorized;
        let contract = scenario_contract(scenario);

        assert!(!scenario.is_active_checkpoint());
        assert!(scenario.is_boundary_checkpoint());
        assert!(!scenario.supports_host_push_active_stimulus());
        assert_eq!(
            active_process_operation(scenario),
            ProcessOperation::DeviceCopy
        );
        assert_eq!(active_operation_class(scenario), "device_copy");
        assert!(contract.active_process.is_none());
        assert_eq!(
            contract.allowed_issue_codes,
            vec![
                Some("device_unauthorized".to_string()),
                Some("device_identity_unverified".to_string()),
            ]
        );
        assert_eq!(
            contract
                .authorization_transition
                .as_ref()
                .expect("authorization transition contract is required")
                .terminal_issue_codes,
            vec![
                "device_unauthorized".to_string(),
                "device_identity_unverified".to_string(),
            ]
        );
        assert_eq!(
            contract.allowed_step_states,
            vec![StepStateContract {
                executed: 1,
                skipped: 0,
                failed: 1,
                cancelled: 0,
                blocked: 0,
                not_attempted: 0,
            }]
        );
        assert!(contract.facts.boundary_checkpoint);
        assert!(!contract.facts.active_checkpoint);
    }

    #[test]
    fn authorization_terminal_issue_accepts_transport_or_identity_precedence_only() {
        assert!(is_authorization_terminal_issue("device_unauthorized"));
        assert!(is_authorization_terminal_issue(
            "device_identity_unverified"
        ));
        assert!(!is_authorization_terminal_issue("device_identity_changed"));
        assert!(!is_authorization_terminal_issue("device_disconnected"));
        assert!(!is_authorization_terminal_issue("device_offline"));
    }

    #[test]
    fn active_stimulus_derivation_targets_the_bounded_operator_window() {
        let derived = derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 6_880)
            .expect("measured host-push calibration should derive a payload");
        assert!((1_100 * 1024..=1_130 * 1024).contains(&derived.payload_kib));
        assert!((29_000..=31_000).contains(&derived.predicted_ms));
        assert!((ACTIVE_MIN_KIB..=ACTIVE_MAX_KIB).contains(&derived.payload_kib));
    }

    #[test]
    fn active_stimulus_derivation_rejects_zero_or_unusable_throughput() {
        assert!(derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 0).is_err());
        assert!(derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 1).is_err());
    }

    #[test]
    fn exact_push_child_capture_serializes_host_push_and_rejects_relabeling() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(7);
        let base = UNIX_EPOCH + Duration::from_secs(10);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::Push,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::Push,
                at: base,
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::Push,
                at: base + Duration::from_secs(1),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::Push,
                at: base + Duration::from_secs(3),
            },
        ];
        let evidence = active_process_evidence(
            &events,
            Some(base + Duration::from_secs(2)),
            "run-scope-test",
            ProcessOperation::Push,
        )
        .expect("complete exact push-child evidence should serialize");

        assert_eq!(evidence["runId"], "run-scope-test");
        assert_eq!(evidence["operationClass"], "host_push");
        assert_eq!(evidence["aliveImmediatelyBeforeAction"], true);
        assert_eq!(evidence["terminalReportedBeforeAction"], false);
        assert!(active_process_evidence(
            &events,
            Some(base + Duration::from_secs(2)),
            "run-scope-test",
            ProcessOperation::DeviceCopy,
        )
        .is_none());
    }

    #[test]
    fn same_second_action_and_terminal_remain_blocked() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(8);
        let base = UNIX_EPOCH + Duration::from_secs(20);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(900),
            },
        ];
        assert!(active_process_evidence(
            &events,
            Some(base + Duration::from_millis(500)),
            "run-scope-test",
            ProcessOperation::DeviceCopy,
        )
        .is_none());
    }

    #[test]
    fn rust_contract_adapter_enforces_host_clock_measurement_and_phase() {
        let before_contract = scenario_contract(Scenario::HostSleepBeforeDeadline);
        let run_scope = format!("run-scope-sha256:{}", "1".repeat(64));
        let observed = json!({
            "success": false,
            "issue": "operation_timed_out",
            "stepStates": {"executed": 0, "skipped": 0, "failed": 1, "cancelled": 0, "blocked": 0, "notAttempted": 1},
            "partialChangesPossible": false,
            "authorityInvalidated": true,
            "activeSlotReleased": true,
            "runScope": run_scope,
            "deviceScope": format!("serial-sha256:{}", "2".repeat(64)),
            "activeSlotObservation": {
                "acquired": true, "released": true, "runId": run_scope,
                "executionId": format!("execution-sha256:{}", "3".repeat(64)),
                "acquiredAt": "unix:1", "terminalCleanupAt": "unix:8", "releasedAt": "unix:8",
                "sourceKind": "production_owned_slot", "evidence": "production-execution-session-slot"
            },
            "activeProcess": {
                "runId": run_scope,
                "operationId": format!("operation-sha256:{}", "4".repeat(64)),
                "operationClass": "device_copy",
                "childIdentity": format!("child-sha256:{}", "5".repeat(64)),
                "spawnedAt": "unix:1", "mutationStartedAt": "unix:2", "checkedAliveAt": "unix:3",
                "actionAt": "unix:4", "terminalAt": "unix:7",
                "aliveImmediatelyBeforeAction": true, "terminalReportedBeforeAction": false
            },
            "hostSleep": {
                "timerClassification": "suspended_time_included",
                "terminalOutcome": "timed_out",
                "deadlineClockSource": "owned_process_monotonic_deadline_clock",
                "measurementToleranceMs": 100,
                "operationStartedAt": "unix:1",
                "wakeAt": "unix:4",
                "deadlineMs": 5000,
                "operatorActionPhase": "before_deadline",
                "suspendedWallMs": 2000,
                "deadlineClockAdvanceDuringSuspensionMs": 2000,
                "remainingBeforeSleepMs": 4000,
                "remainingAfterWakeMs": 2000
            },
            "cleanup": "succeeded",
            "residual": "clean",
        });
        assert!(evaluate_scenario_contract(&before_contract, &observed).is_ok());
        let after_contract = scenario_contract(Scenario::HostSleepAfterDeadline);
        assert!(evaluate_scenario_contract(&after_contract, &observed).is_err());
    }

    #[test]
    fn successful_execution_does_not_claim_possible_partial_changes() {
        let step = |status| StepRunRecord {
            step_id: "fixture/step".to_string(),
            status,
            message: None,
            outputs: Default::default(),
            failure_kind: None,
            cleanup: None,
        };
        let successful = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 2,
            steps: vec![step(StepRunStatus::Executed), step(StepRunStatus::Executed)],
        };
        let failed = ExecutionRunResult {
            success: false,
            cancelled: false,
            total_steps: 2,
            steps: vec![step(StepRunStatus::Executed), step(StepRunStatus::Failed)],
        };
        assert!(!partial_changes_possible(&successful));
        assert!(partial_changes_possible(&failed));
    }

    #[test]
    fn df_parser_uses_available_column_instead_of_used_column() {
        let output = "Filesystem 1K-blocks Used Available Use% Mounted on\nvolume 10000000 7000000 3000000 70% mountpoint\n";
        assert_eq!(parse_available_kib(output), Ok(3_000_000));
    }

    #[test]
    fn df_parser_accepts_toybox_avail_header() {
        let output = "Filesystem 1K-blocks Used Avail Use% Mounted on\nvolume 8000000 6000000 2000000 75% mountpoint\n";
        assert_eq!(parse_available_kib(output), Ok(2_000_000));
    }

    #[test]
    fn df_parser_accepts_a_wrapped_filesystem_name() {
        let output = "Filesystem 1K-blocks Used Available Use% Mounted on\nvery-long-filesystem-name\n8000000 6000000 2000000 75% mountpoint\n";
        assert_eq!(parse_available_kib(output), Ok(2_000_000));
    }

    #[test]
    fn df_parser_fails_closed_without_an_available_column() {
        let output =
            "Filesystem 1K-blocks Used Use% Mounted on\nvolume 8000000 6000000 75% mountpoint\n";
        assert!(parse_available_kib(output).is_err());
    }

    #[test]
    fn operation_timeout_requires_exact_live_child_evidence() {
        let contract = scenario_contract(Scenario::OperationTimeout);
        assert!(contract
            .active_process
            .as_ref()
            .is_some_and(|rule| rule.required && rule.exact_run_binding));
    }

    #[test]
    fn low_storage_preflight_requires_four_gib_and_bounded_fixture_owned_reserve() {
        assert!(validate_low_storage_preflight(LowStoragePreflight {
            initial_free_kib: 4 * 1024 * 1024,
            recovery_reserve_kib: 1024 * 1024,
            filler_kib: bounded_storage_filler_kib(3 * 1024 * 1024),
            max_filler_kib: MAX_STORAGE_FILLER_KIB,
            reserve_owned: true,
            filler_owned: true,
        })
        .is_ok());
        for invalid in [
            LowStoragePreflight {
                initial_free_kib: 4 * 1024 * 1024 - 1,
                recovery_reserve_kib: 1024 * 1024,
                filler_kib: bounded_storage_filler_kib(3 * 1024 * 1024),
                max_filler_kib: MAX_STORAGE_FILLER_KIB,
                reserve_owned: true,
                filler_owned: true,
            },
            LowStoragePreflight {
                initial_free_kib: 4 * 1024 * 1024,
                recovery_reserve_kib: 1024 * 1024 - 1,
                filler_kib: bounded_storage_filler_kib(3 * 1024 * 1024),
                max_filler_kib: MAX_STORAGE_FILLER_KIB,
                reserve_owned: true,
                filler_owned: true,
            },
            LowStoragePreflight {
                initial_free_kib: 4 * 1024 * 1024,
                recovery_reserve_kib: 1024 * 1024,
                filler_kib: MAX_STORAGE_FILLER_KIB + 1,
                max_filler_kib: MAX_STORAGE_FILLER_KIB,
                reserve_owned: true,
                filler_owned: true,
            },
            LowStoragePreflight {
                initial_free_kib: 4 * 1024 * 1024,
                recovery_reserve_kib: 1024 * 1024,
                filler_kib: bounded_storage_filler_kib(3 * 1024 * 1024),
                max_filler_kib: MAX_STORAGE_FILLER_KIB,
                reserve_owned: false,
                filler_owned: true,
            },
        ] {
            assert!(validate_low_storage_preflight(invalid).is_err());
        }
    }

    #[test]
    fn low_storage_cleanup_is_ordered_and_fixture_scoped() {
        assert_eq!(
            storage_cleanup_order(),
            ["payload", "filler", "sentinel", "recovery-reserve"]
        );
        assert!(fixture_owned_run_path(
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/phase6d6-run/payload"
        ));
        assert!(!fixture_owned_run_path(
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/../other"
        ));
        assert!(!fixture_owned_run_path("/sdcard/other/payload"));
    }

    #[test]
    fn low_storage_filler_preserves_cleanup_headroom_and_stays_bounded() {
        assert_eq!(
            bounded_storage_filler_kib(3 * 1024 * 1024),
            3 * 1024 * 1024 - STORAGE_CLEANUP_HEADROOM_KIB
        );
        assert!(validate_low_storage_preflight(LowStoragePreflight {
            initial_free_kib: MAX_STORAGE_INITIAL_FREE_KIB + 1,
            recovery_reserve_kib: RECOVERY_RESERVE_KIB,
            filler_kib: MAX_STORAGE_FILLER_KIB,
            max_filler_kib: MAX_STORAGE_FILLER_KIB,
            reserve_owned: true,
            filler_owned: true,
        })
        .is_err());
    }

    #[test]
    fn low_storage_filler_uses_conservative_budget_when_df_reserve_accounting_is_optimistic() {
        let initial_free_kib = 4_920_980;
        let nominal_after_reserve = initial_free_kib - RECOVERY_RESERVE_KIB;
        let after_reserve_kib = nominal_after_reserve + 128;

        let filler_kib =
            bounded_storage_filler_kib_from_observations(initial_free_kib, after_reserve_kib);
        let initial_budget_kib =
            initial_free_kib - RECOVERY_RESERVE_KIB - STORAGE_CLEANUP_HEADROOM_KIB;

        assert_eq!(filler_kib, initial_budget_kib);
        assert!(validate_low_storage_preflight(LowStoragePreflight {
            initial_free_kib,
            recovery_reserve_kib: RECOVERY_RESERVE_KIB,
            filler_kib,
            max_filler_kib: MAX_STORAGE_FILLER_KIB,
            reserve_owned: true,
            filler_owned: true,
        })
        .is_ok());
    }

    #[test]
    fn low_storage_payload_exceeds_total_free_blocks_including_reserved_space() {
        // Physical GT78-VN evidence exposed 147,456 KiB of free blocks that were
        // absent from df's Available column.  Leaving 64 MiB Available therefore
        // still let the old static 128 MiB payload fit.  Size against Blocks-Used.
        let output = concat!(
            "Filesystem 1K-blocks Used Available Use% Mounted on\n",
            "/dev/fuse 52232748 52019752 65540 100% /storage/emulated\n",
        );
        let available_kib = parse_available_kib(output).expect("Available KiB should parse");
        let total_free_kib = parse_total_free_kib(output).expect("total free KiB should parse");
        assert_eq!(available_kib, 65_540);
        assert_eq!(total_free_kib, 212_996);
        assert_eq!(total_free_kib - available_kib, 147_456);

        let payload_kib = bounded_low_storage_payload_kib(total_free_kib)
            .expect("observed reserve gap should fit inside the bounded payload window");
        assert_eq!(payload_kib, total_free_kib + LOW_STORAGE_FAILURE_MARGIN_KIB);
        assert!(payload_kib > total_free_kib);
        assert!(payload_kib <= MAX_LOW_STORAGE_HOST_PAYLOAD_KIB);
    }

    #[test]
    fn low_storage_payload_fails_closed_when_total_free_exceeds_the_bound() {
        assert!(bounded_low_storage_payload_kib(MAX_LOW_STORAGE_HOST_PAYLOAD_KIB).is_err());
    }

    #[test]
    fn low_storage_host_payload_contains_deterministic_nontrivial_bytes() {
        let workspace = tempfile::tempdir().expect("temporary workspace should be available");
        let first = workspace.path().join("first.bin");
        let second = workspace.path().join("second.bin");
        let byte_len = 32 * 1024 + 7;

        write_low_storage_host_payload(&first, byte_len, "phase6d6-low-storage-test")
            .expect("first low-storage fixture should be written");
        write_low_storage_host_payload(&second, byte_len, "phase6d6-low-storage-test")
            .expect("second low-storage fixture should be written");
        let first_bytes = fs::read(&first).expect("first low-storage fixture should be readable");
        let second_bytes =
            fs::read(&second).expect("second low-storage fixture should be readable");

        assert_eq!(first_bytes.len(), byte_len as usize);
        assert!(first_bytes.iter().any(|byte| *byte != 0));
        assert_eq!(first_bytes, second_bytes);
    }

    #[test]
    fn host_sleep_deadline_and_handshake_constants_are_bounded_and_compatible() {
        assert_eq!(HOST_SLEEP_QUALIFICATION_DEADLINE, Duration::from_secs(120));
        assert_eq!(HOST_SLEEP_MEASUREMENT_TOLERANCE_MS, 8_000);
        assert_eq!(HOST_SLEEP_HANDOFF_WINDOW, Duration::from_secs(4));
        assert_eq!(HOST_SLEEP_READY_LATENCY_BUDGET, Duration::from_secs(1));
        assert_eq!(
            ProcessOperation::DeviceCopy.deadline(),
            Duration::from_secs(300)
        );
        assert!(
            HOST_SLEEP_HANDOFF_WINDOW + HOST_SLEEP_READY_LATENCY_BUDGET <= ACTIVE_SAMPLE_FRESHNESS,
            "the enforced handoff window plus ready latency must fit the liveness freshness bound"
        );
        assert!(
            HOST_SLEEP_HANDOFF_WINDOW.as_millis() as u64
                + HOST_SLEEP_READY_LATENCY_BUDGET.as_millis() as u64
                + 2_000
                + 1_000
                <= HOST_SLEEP_MEASUREMENT_TOLERANCE_MS,
            "the tolerance must cover handoff latency, marker quantization, and scheduler margin"
        );
    }

    #[test]
    fn host_sleep_scenarios_require_the_exact_child_observer_and_device_copy_stimulus() {
        for scenario in [
            Scenario::HostSleepBeforeDeadline,
            Scenario::HostSleepAfterDeadline,
        ] {
            assert!(scenario.requires_process_observer());
            assert!(scenario.requires_device_copy_stimulus());
            assert_eq!(
                active_process_operation(scenario),
                ProcessOperation::DeviceCopy
            );
            assert_eq!(
                process_operation_class(ProcessOperation::DeviceCopy),
                Some("device_copy")
            );
            assert!(scenario.is_active_checkpoint());
        }
    }

    #[test]
    fn timeout_stimulus_for_scenario_accepts_only_timeout_and_host_sleep() {
        for scenario in [
            Scenario::OperationTimeout,
            Scenario::HostSleepBeforeDeadline,
            Scenario::HostSleepAfterDeadline,
        ] {
            let stimulus = timeout_stimulus_for_scenario(scenario).expect(
                "timeout and host-sleep scenarios must obtain the private pseudo-device copy",
            );
            assert_eq!(stimulus.device_source_path, TIMEOUT_SOURCE_PATH);
            assert_eq!(stimulus.device_destination_path, TIMEOUT_DESTINATION_PATH);
        }
        assert!(timeout_stimulus_for_scenario(Scenario::CancellationBoundary).is_err());
    }

    #[test]
    fn reviewed_plan_accepts_host_sleep_pseudo_device_stimulus() {
        let (invocation, facts) = timeout_test_invocation(Scenario::HostSleepBeforeDeadline);
        let stimulus = timeout_stimulus_for_scenario(Scenario::HostSleepBeforeDeadline)
            .expect("host sleep must obtain the private pseudo-device copy");
        let plan = reviewed_plan(&invocation, &facts, None, None, Some(&stimulus));
        assert_eq!(plan.steps.len(), 2);
        match plan.steps[0].params.get("source") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value["value"], json!(TIMEOUT_SOURCE_PATH));
                assert_eq!(value["location"], Value::String("device".to_string()));
            }
            _ => panic!("host-sleep first source should remain a literal runtime value"),
        }
        match plan.steps[0].params.get("dest") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value, &json!(TIMEOUT_DESTINATION_PATH));
            }
            _ => panic!("host-sleep first destination should remain a literal runtime value"),
        }
        match plan.steps[0].params.get("copy_policy") {
            Some(ExecutionParamValue::Literal { value }) => {
                assert_eq!(value, &json!("merge"));
            }
            _ => panic!("host-sleep copy policy should remain a literal runtime value"),
        }
    }

    fn wait_for_test_marker(sentinel: &Sentinel, name: &str) -> SystemTime {
        let started = Instant::now();
        loop {
            if let Ok(time) = sentinel.marker_time(name) {
                return time;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "{name} marker did not appear within the test window"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn host_sleep_watcher_process(
        observer: OwnedProcessObservationHandle,
    ) -> std::thread::JoinHandle<
        Result<
            crate::owned_process::CapturedProcessOutput,
            crate::owned_process::OwnedProcessError,
        >,
    > {
        let executable = std::env::current_exe().expect("test executable should be available");
        let executable = executable
            .to_str()
            .expect("test executable should be utf-8")
            .to_string();
        std::thread::spawn(move || {
            let _deadline = arm_test_process_deadline(
                ProcessOperation::DeviceCopy,
                HOST_SLEEP_QUALIFICATION_DEADLINE,
            );
            crate::owned_process::run_owned_process_observed(
                &executable,
                &[
                    "--exact".to_string(),
                    "executor_real_adb_tests::physical_interruption_qualification::tests::watcher_helper"
                        .to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::DeviceCopy,
                observer,
            )
        })
    }

    #[test]
    fn host_sleep_watcher_completes_the_pre_suspend_handshake_and_returns_the_handoff_action() {
        let observer = OwnedProcessObservationHandle::default();
        let process = host_sleep_watcher_process(observer.clone());
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");

        let watcher_observer = observer.clone();
        let watcher_sentinel = sentinel.clone();
        let watcher = std::thread::spawn(move || {
            run_host_sleep_watcher(&watcher_observer, &watcher_sentinel)
        });

        let sleep_ready = wait_for_test_marker(&sentinel, "sleep-ready");
        assert!(
            sleep_ready
                >= sentinel
                    .marker_time("sleep-requested")
                    .expect("requested time")
        );
        sentinel
            .mark("sleep-entered", "ack\n")
            .expect("sleep-entered marker should be created");
        sentinel
            .mark("wake", "ack\n")
            .expect("wake marker should be created");

        let action_at = watcher
            .join()
            .expect("host-sleep watcher should finish")
            .expect("the pre-suspend handshake should complete");
        let entered = sentinel.marker_time("sleep-entered").expect("entered time");
        let wake = sentinel.marker_time("wake").expect("wake time");
        assert_eq!(
            action_at, entered,
            "the action boundary must be the sleep-entered handoff"
        );
        assert!(entered < wake, "wake must follow the pre-suspend handoff");

        let events = observer.events();
        let requested = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        owner_reported: false,
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requested.len(),
            2,
            "exactly one pre-suspend and one post-wake sample"
        );
        let after_at = match requested[1] {
            OwnedProcessLifecycleEvent::DeadlineClockSampled { at, .. } => *at,
            _ => unreachable!("requested sample must carry a timestamp"),
        };
        assert!(
            after_at >= wake,
            "the post-wake sample must be requested only after the wake marker is observed"
        );
        process
            .join()
            .expect("host-sleep helper process should finish")
            .expect("the observed helper should complete");
    }

    #[test]
    fn host_sleep_watcher_blocks_when_the_exact_child_is_not_alive() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let executable = executable
            .to_str()
            .expect("test executable should be utf-8")
            .to_string();
        let process_observer = observer.clone();
        let process = std::thread::spawn(move || {
            crate::owned_process::run_owned_process_observed(
                &executable,
                &[
                    "--exact".to_string(),
                    "owned_process::tests::normal_helper".to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::DeviceCopy,
                process_observer,
            )
        });
        process
            .join()
            .expect("helper process should finish")
            .expect("the completed child should remain completed");

        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");
        let error = run_host_sleep_watcher(&observer, &sentinel)
            .expect_err("a terminal exact child cannot pass the pre-suspend handshake");
        assert!(
            error.contains("not alive"),
            "the rejection must name the failed liveness proof: {error}"
        );
    }

    #[test]
    fn host_sleep_watcher_rejects_markers_out_of_order() {
        let observer = OwnedProcessObservationHandle::default();
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        let error = run_host_sleep_watcher(&observer, &sentinel)
            .expect_err("a handoff marker before operation start must be rejected");
        assert!(
            error.contains("predates operation start"),
            "the rejection must name the stale marker: {error}"
        );
    }

    #[test]
    fn host_sleep_watcher_rejects_non_ack_markers() {
        let observer = OwnedProcessObservationHandle::default();
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "not-ack\n")
            .expect("sleep-requested marker should be created");
        let error = run_host_sleep_watcher(&observer, &sentinel)
            .expect_err("a marker without the exact ack content must be rejected");
        assert!(
            error.contains("ack"),
            "the rejection must name the ack requirement: {error}"
        );
    }

    #[test]
    fn host_sleep_watcher_enforces_the_bounded_handoff_window() {
        let observer = OwnedProcessObservationHandle::default();
        let process = host_sleep_watcher_process(observer.clone());
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");

        let watcher_observer = observer.clone();
        let watcher_sentinel = sentinel.clone();
        let watcher = std::thread::spawn(move || {
            run_host_sleep_watcher(&watcher_observer, &watcher_sentinel)
        });
        let _ = wait_for_test_marker(&sentinel, "sleep-ready");
        std::thread::sleep(HOST_SLEEP_HANDOFF_WINDOW + Duration::from_millis(300));
        sentinel
            .mark("sleep-entered", "ack\n")
            .expect("sleep-entered marker should be created");
        let error = watcher
            .join()
            .expect("host-sleep watcher should finish")
            .expect_err("a late sleep-entered marker must exceed the bounded handoff window");
        assert!(
            error.contains("handoff window"),
            "the rejection must name the bounded handoff window: {error}"
        );
        process
            .join()
            .expect("host-sleep helper process should finish")
            .expect("the observed helper should complete");
    }

    #[derive(Clone, Copy, Debug)]
    struct HostSleepSampleFixture {
        deadline_start_wall_ms: u64,
        before_wall_ms: u64,
        before_ns: u64,
        before_remaining_ns: u64,
        after_wall_ms: u64,
        after_ns: u64,
        after_remaining_ns: u64,
        terminal_wall_ms: u64,
        terminal_ns: u64,
        terminal_remaining_ns: u64,
    }

    const HOST_SLEEP_EXCLUDED_SAMPLES: HostSleepSampleFixture = HostSleepSampleFixture {
        deadline_start_wall_ms: 1_000,
        before_wall_ms: 3_050,
        before_ns: 2_050_000_000,
        before_remaining_ns: 117_950_000_000,
        after_wall_ms: 30_200,
        after_ns: 2_250_000_000,
        after_remaining_ns: 117_750_000_000,
        terminal_wall_ms: 31_000,
        terminal_ns: 3_050_000_000,
        terminal_remaining_ns: 116_950_000_000,
    };

    const HOST_SLEEP_INCLUDED_SAMPLES: HostSleepSampleFixture = HostSleepSampleFixture {
        deadline_start_wall_ms: 1_000,
        before_wall_ms: 3_100,
        before_ns: 2_100_000_000,
        before_remaining_ns: 117_900_000_000,
        after_wall_ms: 130_500,
        after_ns: 129_500_000_000,
        after_remaining_ns: 0,
        terminal_wall_ms: 129_000,
        terminal_ns: 128_000_000_000,
        terminal_remaining_ns: 0,
    };

    fn host_sleep_events(
        operation_id: OwnedProcessOperationId,
        base: SystemTime,
        deadline_ns: u64,
        samples: HostSleepSampleFixture,
        deadline_reached_event: bool,
    ) -> Vec<OwnedProcessLifecycleEvent> {
        let mut events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(100),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(2),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::DeadlineClockStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline_clock_start_ns: 0,
                deadline_ns,
                at: base + Duration::from_millis(samples.deadline_start_wall_ms),
            },
            OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(samples.before_wall_ms),
                deadline_clock_ns: samples.before_ns,
                remaining_ns: samples.before_remaining_ns,
                deadline_reached: false,
                owner_reported: false,
            },
        ];
        if deadline_reached_event {
            events.push(OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline: HOST_SLEEP_QUALIFICATION_DEADLINE,
                at: base + Duration::from_secs(120),
            });
        }
        events.push(OwnedProcessLifecycleEvent::Terminal {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: base + Duration::from_millis(samples.terminal_wall_ms),
        });
        events.push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: base + Duration::from_millis(samples.terminal_wall_ms),
            deadline_clock_ns: samples.terminal_ns,
            remaining_ns: samples.terminal_remaining_ns,
            deadline_reached: samples.terminal_ns >= deadline_ns,
            owner_reported: true,
        });
        events.push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: base + Duration::from_millis(samples.after_wall_ms),
            deadline_clock_ns: samples.after_ns,
            remaining_ns: samples.after_remaining_ns,
            deadline_reached: samples.after_ns >= deadline_ns,
            owner_reported: false,
        });
        events
    }

    fn host_sleep_sentinel(started: u64, requested: u64, entered: u64, wake: u64) -> Value {
        json!({
            "operationStartedAt": format!("unix:{started}"),
            "sleepRequestedAt": format!("unix:{requested}"),
            "sleepEnteredAt": format!("unix:{entered}"),
            "wakeAt": format!("unix:{wake}"),
        })
    }

    fn host_sleep_step(failure: Option<StepFailureKind>) -> ExecutionRunResult {
        ExecutionRunResult {
            success: failure.is_none(),
            cancelled: false,
            total_steps: 2,
            steps: vec![StepRunRecord {
                step_id: "phase6d6/host-sleep/first".to_string(),
                status: if failure.is_some() {
                    StepRunStatus::Failed
                } else {
                    StepRunStatus::Executed
                },
                message: None,
                outputs: Default::default(),
                failure_kind: failure,
                cleanup: Some(ProcessCleanup::Confirmed),
            }],
        }
    }

    #[test]
    fn host_sleep_evidence_builds_a_complete_excluded_before_deadline_completion() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(900);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(None);
        let evidence = host_sleep_evidence(
            &Scenario::HostSleepBeforeDeadline,
            &host_sleep_sentinel(1, 3, 4, 30),
            Ok(&run),
            &events,
        );
        let value = evidence
            .as_object()
            .expect("a complete excluded before-deadline record must serialize");
        assert_eq!(value["terminalOutcome"], "completed");
        assert_eq!(value["timerClassification"], "suspended_time_excluded");
        assert_eq!(value["deadlineClockStartNs"], "monotonic-ns:0");
        assert_eq!(
            value["deadlineClockBeforeSleepNs"],
            "monotonic-ns:2050000000"
        );
        assert_eq!(value["deadlineClockAfterWakeNs"], "monotonic-ns:2250000000");
        assert_eq!(value["deadlineClockTerminalNs"], "monotonic-ns:3050000000");
        assert_eq!(value["deadlineClockAdvanceDuringSuspensionMs"], 200);
        assert_eq!(value["suspendedWallMs"], 26000);
        assert_eq!(value["remainingBeforeSleepMs"], 117950);
        assert_eq!(value["remainingAfterWakeMs"], 117750);
        assert_eq!(value["measurementToleranceMs"], 8000);
        assert_eq!(value["deadlineMs"], 120000);
        assert_eq!(value["executorElapsedMs"], 3050);
        assert_eq!(value["wallElapsedMs"], 30000);
        assert_eq!(value["elapsedBeforeSleepMs"], 2000);
        assert_eq!(value["operationStartedAt"], "unix:1");
        assert_eq!(value["operatorActionPhase"], "before_deadline");
        assert_eq!(
            value["deadlineClockSource"],
            "owned_process_monotonic_deadline_clock"
        );
        assert_eq!(value["transportLossBlockedMeasurement"], false);
        assert_eq!(value["terminalAt"], "unix:31");
    }

    #[test]
    fn host_sleep_evidence_builds_an_included_after_deadline_timeout_with_terminal_before_post_wake(
    ) {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(901);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_INCLUDED_SAMPLES,
            true,
        );
        let run = host_sleep_step(Some(StepFailureKind::OperationTimedOut));
        let evidence = host_sleep_evidence(
            &Scenario::HostSleepAfterDeadline,
            &host_sleep_sentinel(1, 3, 4, 130),
            Ok(&run),
            &events,
        );
        let value = evidence
            .as_object()
            .expect("a complete included after-deadline record must serialize");
        assert_eq!(value["terminalOutcome"], "timed_out");
        assert_eq!(value["timerClassification"], "suspended_time_included");
        assert_eq!(
            value["deadlineClockBeforeSleepNs"],
            "monotonic-ns:2100000000"
        );
        assert_eq!(
            value["deadlineClockTerminalNs"],
            "monotonic-ns:128000000000"
        );
        assert_eq!(
            value["deadlineClockAfterWakeNs"],
            "monotonic-ns:129500000000"
        );
        assert_eq!(value["deadlineClockAdvanceDuringSuspensionMs"], 127400);
        assert_eq!(value["remainingBeforeSleepMs"], 117900);
        assert_eq!(value["remainingAfterWakeMs"], 0);
        assert_eq!(value["operationStartedAt"], "unix:1");
        assert_eq!(value["operatorActionPhase"], "after_deadline");
        assert_eq!(value["terminalAt"], "unix:129");
    }

    #[test]
    fn host_sleep_evidence_blocks_missing_clock_samples() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(902);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::DeadlineClockStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline_clock_start_ns: 0,
                deadline_ns,
                at: base + Duration::from_millis(1),
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(5),
            },
        ];
        let run = host_sleep_step(Some(StepFailureKind::OperationTimedOut));
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 3, 4, 30),
                Ok(&run),
                &events,
            )
            .is_null(),
            "missing requested deadline-clock samples must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_a_different_operation_identity() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(903);
        let base = UNIX_EPOCH;
        let events = vec![OwnedProcessLifecycleEvent::DeadlineClockStarted {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            deadline_clock_start_ns: 0,
            deadline_ns: u64::try_from(TIMEOUT_QUALIFICATION_DEADLINE.as_nanos())
                .expect("timeout deadline fits u64"),
            at: base,
        }];
        let run = host_sleep_step(Some(StepFailureKind::OperationTimedOut));
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 3, 4, 30),
                Ok(&run),
                &events,
            )
            .is_null(),
            "a deadline-clock basis from a different operation identity must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_timeout_without_the_owner_deadline_event() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(904);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(Some(StepFailureKind::OperationTimedOut));
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 2, 3, 4),
                Ok(&run),
                &events,
            )
            .is_null(),
            "a timeout without the owner-recorded DeadlineReached event must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_completion_with_an_owner_deadline_event() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(905);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            true,
        );
        let run = host_sleep_step(None);
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 2, 3, 4),
                Ok(&run),
                &events,
            )
            .is_null(),
            "completion with an owner-recorded DeadlineReached event must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_an_increased_remaining_budget() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(906);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HostSleepSampleFixture {
                deadline_start_wall_ms: 1_000,
                before_wall_ms: 3_100,
                before_ns: 2_100_000_000,
                before_remaining_ns: 100_000_000_000,
                after_wall_ms: 30_200,
                after_ns: 2_250_000_000,
                after_remaining_ns: 110_000_000_000,
                terminal_wall_ms: 31_000,
                terminal_ns: 3_050_000_000,
                terminal_remaining_ns: 116_950_000_000,
            },
            false,
        );
        let run = host_sleep_step(None);
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 3, 4, 30),
                Ok(&run),
                &events,
            )
            .is_null(),
            "an increased remaining budget across suspension must block"
        );
    }

    #[test]
    fn host_sleep_evidence_marks_transport_loss_as_a_measurement_blocker() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(907);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(Some(StepFailureKind::DeviceDisconnected));
        let evidence = host_sleep_evidence(
            &Scenario::HostSleepBeforeDeadline,
            &host_sleep_sentinel(1, 3, 4, 30),
            Ok(&run),
            &events,
        );
        let value = evidence
            .as_object()
            .expect("a transport-loss record must serialize with its blocker flag");
        assert_eq!(value["terminalOutcome"], "transport_loss");
        assert_eq!(value["transportLossBlockedMeasurement"], true);
    }

    #[test]
    fn host_sleep_evidence_accepts_transport_loss_with_deadline_crossed_and_no_deadline_reached() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(914);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HostSleepSampleFixture {
                deadline_start_wall_ms: 1_000,
                before_wall_ms: 3_050,
                before_ns: 2_050_000_000,
                before_remaining_ns: 117_950_000_000,
                after_wall_ms: 30_200,
                after_ns: 2_250_000_000,
                after_remaining_ns: 117_750_000_000,
                terminal_wall_ms: 130_000,
                terminal_ns: 121_000_000_000,
                terminal_remaining_ns: 0,
            },
            false,
        );
        let run = host_sleep_step(Some(StepFailureKind::DeviceDisconnected));
        let evidence = host_sleep_evidence(
            &Scenario::HostSleepBeforeDeadline,
            &host_sleep_sentinel(1, 3, 4, 30),
            Ok(&run),
            &events,
        );
        let value = evidence
            .as_object()
            .expect("transport loss with a crossed monotonic deadline and zero DeadlineReached must serialize");
        assert_eq!(value["terminalOutcome"], "transport_loss");
        assert_eq!(value["transportLossBlockedMeasurement"], true);
        assert_eq!(
            value["deadlineClockTerminalNs"],
            "monotonic-ns:121000000000"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_transport_loss_with_an_owner_deadline_reached() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(915);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HostSleepSampleFixture {
                deadline_start_wall_ms: 1_000,
                before_wall_ms: 3_050,
                before_ns: 2_050_000_000,
                before_remaining_ns: 117_950_000_000,
                after_wall_ms: 30_200,
                after_ns: 2_250_000_000,
                after_remaining_ns: 117_750_000_000,
                terminal_wall_ms: 130_000,
                terminal_ns: 121_000_000_000,
                terminal_remaining_ns: 0,
            },
            true,
        );
        let run = host_sleep_step(Some(StepFailureKind::DeviceDisconnected));
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 3, 4, 30),
                Ok(&run),
                &events,
            )
            .is_null(),
            "transport loss with an owner DeadlineReached event must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_wrong_phase_chronology() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(908);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(None);
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepAfterDeadline,
                &host_sleep_sentinel(1, 3, 4, 4),
                Ok(&run),
                &events,
            )
            .is_null(),
            "a wake before the after-deadline threshold must not qualify as after-deadline"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_missing_sentinel_markers() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(909);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(None);
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &json!({ "operationStartedAt": "unix:1" }),
                Ok(&run),
                &events,
            )
            .is_null(),
            "missing sentinel sleep markers must block"
        );
    }

    #[test]
    fn host_sleep_evidence_anchors_phase_to_the_exact_deadline_clock_start() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(910);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HostSleepSampleFixture {
                deadline_start_wall_ms: 5_000,
                before_wall_ms: 6_100,
                before_ns: 1_100_000_000,
                before_remaining_ns: 118_900_000_000,
                after_wall_ms: 122_200,
                after_ns: 1_300_000_000,
                after_remaining_ns: 118_700_000_000,
                terminal_wall_ms: 123_000,
                terminal_ns: 2_100_000_000,
                terminal_remaining_ns: 117_900_000_000,
            },
            false,
        );
        let run = host_sleep_step(None);
        let sentinel = host_sleep_sentinel(1, 6, 7, 122);
        let before_value = host_sleep_evidence(
            &Scenario::HostSleepBeforeDeadline,
            &sentinel,
            Ok(&run),
            &events,
        );
        let before = before_value
            .as_object()
            .expect("the exact deadline-clock start must qualify the before-deadline phase");
        assert_eq!(before["operationStartedAt"], "unix:5");
        assert_eq!(before["elapsedBeforeSleepMs"], 1000);
        assert_eq!(before["wallElapsedMs"], 118000);
        assert_eq!(before["operatorActionPhase"], "before_deadline");
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepAfterDeadline,
                &sentinel,
                Ok(&run),
                &events
            )
            .is_null(),
            "the sentinel progress marker alone must not qualify the after-deadline phase"
        );
    }

    #[test]
    fn host_sleep_evidence_only_relabels_genuine_transport_failures_as_transport_loss() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(911);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let sentinel = host_sleep_sentinel(1, 3, 4, 30);
        let outcome = |kind| {
            let run = host_sleep_step(Some(kind));
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&run),
                &events,
            )
        };
        let timed_out_events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_INCLUDED_SAMPLES,
            true,
        );
        let timed_out = host_sleep_evidence(
            &Scenario::HostSleepAfterDeadline,
            &host_sleep_sentinel(1, 3, 4, 130),
            Ok(&host_sleep_step(Some(StepFailureKind::OperationTimedOut))),
            &timed_out_events,
        );
        assert_eq!(timed_out["terminalOutcome"], "timed_out");
        for kind in [
            StepFailureKind::DeviceDisconnected,
            StepFailureKind::DeviceOffline,
            StepFailureKind::AdbServerUnavailable,
            StepFailureKind::TransportReset,
            StepFailureKind::TransportFailure,
        ] {
            assert_eq!(
                outcome(kind)["terminalOutcome"],
                "transport_loss",
                "a genuine transport failure must serialize transport_loss"
            );
        }
        for kind in [
            StepFailureKind::DeviceUnauthorized,
            StepFailureKind::DeviceStorageExhausted,
            StepFailureKind::DeviceIdentityChanged(
                crate::executor::identity::IdentityCheckPhase::PreOperation,
            ),
            StepFailureKind::DeviceIdentityUnverified(
                crate::executor::identity::IdentityCheckPhase::PreOperation,
            ),
            StepFailureKind::RootAuthorityRevoked,
            StepFailureKind::RootAuthorityUnverified,
            StepFailureKind::OperationCommandFailed,
            StepFailureKind::OperationProcessFailed,
            StepFailureKind::OperationFailed,
            StepFailureKind::RootDenied,
            StepFailureKind::RootUnavailable,
        ] {
            assert!(
                outcome(kind).is_null(),
                "an unrelated failure must block instead of being relabeled as transport loss"
            );
        }
        let contradictory_success = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 2,
            steps: vec![StepRunRecord {
                step_id: "phase6d6/host-sleep/first".to_string(),
                status: StepRunStatus::Failed,
                message: None,
                outputs: Default::default(),
                failure_kind: Some(StepFailureKind::OperationTimedOut),
                cleanup: Some(ProcessCleanup::Confirmed),
            }],
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&contradictory_success),
                &events,
            )
            .is_null(),
            "success with a failure issue is contradictory and must block"
        );
        let contradictory_failure = ExecutionRunResult {
            success: false,
            cancelled: false,
            total_steps: 2,
            steps: vec![StepRunRecord {
                step_id: "phase6d6/host-sleep/first".to_string(),
                status: StepRunStatus::Failed,
                message: None,
                outputs: Default::default(),
                failure_kind: None,
                cleanup: Some(ProcessCleanup::Confirmed),
            }],
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&contradictory_failure),
                &events,
            )
            .is_null(),
            "failure without an issue is contradictory and must block"
        );
    }

    #[test]
    fn host_sleep_evidence_blocks_ambiguous_duplicate_observations() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(912);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        let run = host_sleep_step(None);
        let sentinel = host_sleep_sentinel(1, 3, 4, 30);
        let extra_owner_sample = {
            let mut events = events.clone();
            events.push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(31),
                deadline_clock_ns: 3_100_000_000,
                remaining_ns: 116_900_000_000,
                deadline_reached: false,
                owner_reported: true,
            });
            events
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&run),
                &extra_owner_sample,
            )
            .is_null(),
            "a duplicate owner terminal sample must block"
        );
        let extra_terminal = {
            let mut events = events.clone();
            events.push(OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(32),
            });
            events
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&run),
                &extra_terminal,
            )
            .is_null(),
            "a duplicate Terminal event must block"
        );
        let extra_requested_sample = {
            let mut events = events.clone();
            events.push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_millis(30_300),
                deadline_clock_ns: 2_300_000_000,
                remaining_ns: 117_700_000_000,
                deadline_reached: false,
                owner_reported: false,
            });
            events
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&run),
                &extra_requested_sample,
            )
            .is_null(),
            "a third watcher-requested sample must block"
        );
        let extra_clock_start = {
            let mut events = events.clone();
            events.push(OwnedProcessLifecycleEvent::DeadlineClockStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                deadline_clock_start_ns: 0,
                deadline_ns: u64::try_from(TIMEOUT_QUALIFICATION_DEADLINE.as_nanos())
                    .expect("timeout deadline fits u64"),
                at: base + Duration::from_millis(1_100),
            });
            events
        };
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &sentinel,
                Ok(&run),
                &extra_clock_start,
            )
            .is_null(),
            "a second DeadlineClockStarted for the selected operation must block"
        );
    }

    #[test]
    fn host_sleep_evidence_requires_the_run_local_zero_origin() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(913);
        let base = UNIX_EPOCH;
        let deadline_ns =
            u64::try_from(HOST_SLEEP_QUALIFICATION_DEADLINE.as_nanos()).expect("deadline fits u64");
        let mut events = host_sleep_events(
            operation_id,
            base,
            deadline_ns,
            HOST_SLEEP_EXCLUDED_SAMPLES,
            false,
        );
        for event in &mut events {
            if let OwnedProcessLifecycleEvent::DeadlineClockStarted {
                deadline_clock_start_ns,
                ..
            } = event
            {
                *deadline_clock_start_ns = 1;
            }
        }
        let run = host_sleep_step(None);
        assert!(
            host_sleep_evidence(
                &Scenario::HostSleepBeforeDeadline,
                &host_sleep_sentinel(1, 3, 4, 30),
                Ok(&run),
                &events,
            )
            .is_null(),
            "a non-zero run-local deadline-clock origin must block"
        );
    }

    #[test]
    fn host_sleep_watcher_abort_stops_each_operator_checkpoint_promptly() {
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("abort", "abort\n")
            .expect("abort marker should be created");
        let observer = OwnedProcessObservationHandle::default();
        let started = Instant::now();
        let error = run_host_sleep_watcher(&observer, &sentinel)
            .expect_err("abort while waiting for sleep-requested must stop the checkpoint");
        assert!(
            error.contains("abort"),
            "the rejection must name the abort: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "abort must stop the wait promptly instead of waiting for the ten-minute timeout"
        );
        assert!(
            !sentinel.path("sleep-ready").exists(),
            "abort must prevent the sleep-ready handshake marker"
        );
    }

    #[test]
    fn host_sleep_watcher_abort_stops_while_waiting_for_sleep_entered() {
        let observer = OwnedProcessObservationHandle::default();
        let process = host_sleep_watcher_process(observer.clone());
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");
        let watcher_observer = observer.clone();
        let watcher_sentinel = sentinel.clone();
        let watcher = std::thread::spawn(move || {
            run_host_sleep_watcher(&watcher_observer, &watcher_sentinel)
        });
        let _ = wait_for_test_marker(&sentinel, "sleep-ready");
        sentinel
            .mark("abort", "abort\n")
            .expect("abort marker should be created");
        let started = Instant::now();
        let error = watcher
            .join()
            .expect("host-sleep watcher should finish")
            .expect_err("abort while waiting for sleep-entered must stop the checkpoint");
        assert!(
            error.contains("abort"),
            "the rejection must name the abort: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "abort must stop the wait promptly"
        );
        process
            .join()
            .expect("host-sleep helper process should finish")
            .expect("the observed helper should complete");
    }

    #[test]
    fn host_sleep_watcher_abort_stops_while_waiting_for_wake() {
        let observer = OwnedProcessObservationHandle::default();
        let process = host_sleep_watcher_process(observer.clone());
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");
        let watcher_observer = observer.clone();
        let watcher_sentinel = sentinel.clone();
        let watcher = std::thread::spawn(move || {
            run_host_sleep_watcher(&watcher_observer, &watcher_sentinel)
        });
        let _ = wait_for_test_marker(&sentinel, "sleep-ready");
        sentinel
            .mark("sleep-entered", "ack\n")
            .expect("sleep-entered marker should be created");
        sentinel
            .mark("abort", "abort\n")
            .expect("abort marker should be created");
        let started = Instant::now();
        let error = watcher
            .join()
            .expect("host-sleep watcher should finish")
            .expect_err("abort while waiting for wake must stop the checkpoint");
        assert!(
            error.contains("abort"),
            "the rejection must name the abort: {error}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "abort must stop the wait promptly"
        );
        process
            .join()
            .expect("host-sleep helper process should finish")
            .expect("the observed helper should complete");
    }

    #[test]
    fn host_sleep_final_snapshot_includes_the_post_wake_sample_after_owner_terminal() {
        let observer = OwnedProcessObservationHandle::default();
        let process = host_sleep_watcher_process(observer.clone());
        let sentinel_directory = tempfile::tempdir().expect("sentinel directory should exist");
        let sentinel = Sentinel {
            directory: sentinel_directory.path().to_path_buf(),
        };
        sentinel
            .mark("operation-started", "started\n")
            .expect("operation-started marker should be created");
        sentinel
            .mark("sleep-requested", "ack\n")
            .expect("sleep-requested marker should be created");
        let watcher_observer = observer.clone();
        let watcher_sentinel = sentinel.clone();
        let watcher = std::thread::spawn(move || {
            run_host_sleep_watcher(&watcher_observer, &watcher_sentinel)
        });
        let _ = wait_for_test_marker(&sentinel, "sleep-ready");
        sentinel
            .mark("sleep-entered", "ack\n")
            .expect("sleep-entered marker should be created");

        // Model the supported terminal-before-wake chronology: the exact child
        // reaches terminal first, and only then does the operator create wake.
        let terminal_deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if observer
                .events()
                .iter()
                .any(|event| matches!(event, OwnedProcessLifecycleEvent::Terminal { .. }))
            {
                break;
            }
            assert!(
                Instant::now() < terminal_deadline,
                "the owner terminal event must appear before the wake marker"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let before_at = observer
            .events()
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    owner_reported: false,
                    at,
                    ..
                } => Some(*at),
                _ => None,
            })
            .expect("the pre-suspend deadline-clock sample must exist");
        while SystemTime::now()
            .duration_since(before_at)
            .expect("the pre-suspend sample must precede now")
            < Duration::from_millis(8_500)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        sentinel
            .mark("wake", "ack\n")
            .expect("wake marker should be created");
        let action_at = watcher
            .join()
            .expect("host-sleep watcher should finish")
            .expect("the handshake should complete");
        process
            .join()
            .expect("host-sleep helper process should finish")
            .expect("the observed helper should complete");

        // The final snapshot uses the same path as run_reviewed_plan and must
        // therefore be taken only after the watcher finished.
        let snapshot = host_sleep_events_snapshot(&Some(observer.clone()))
            .expect("the final observer snapshot must be available");
        let operation_id = snapshot
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockStarted { operation_id, .. } => {
                    Some(*operation_id)
                }
                _ => None,
            })
            .expect("the selected operation must have a clock start");
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::DeadlineClockStarted {
                            operation_id: observed,
                            ..
                        } if *observed == operation_id
                    )
                })
                .count(),
            1,
            "the final snapshot must contain exactly one DeadlineClockStarted"
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::DeadlineClockSampled {
                            operation_id: observed,
                            owner_reported: false,
                            ..
                        } if *observed == operation_id
                    )
                })
                .count(),
            2,
            "the final snapshot must contain exactly two watcher-requested samples"
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::DeadlineClockSampled {
                            operation_id: observed,
                            owner_reported: true,
                            ..
                        } if *observed == operation_id
                    )
                })
                .count(),
            1,
            "the final snapshot must contain exactly one owner-reported terminal sample"
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::Terminal {
                            operation_id: observed,
                            ..
                        } if *observed == operation_id
                    )
                })
                .count(),
            1,
            "the final snapshot must contain exactly one Terminal event"
        );
        let clock_start_at = snapshot
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockStarted {
                    operation_id: observed,
                    at,
                    ..
                } if *observed == operation_id => Some(*at),
                _ => None,
            })
            .expect("the clock start wall observation must exist");
        let after_at = snapshot
            .iter()
            .filter_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    operation_id: observed,
                    owner_reported: false,
                    at,
                    ..
                } if *observed == operation_id => Some(*at),
                _ => None,
            })
            .next_back()
            .expect("the post-wake sample must exist");
        assert!(
            action_at < sentinel.marker_time("wake").expect("wake time"),
            "the sleep-entered handoff must precede the wake acknowledgement"
        );
        let sentinel_value = json!({
            "operationStartedAt": format!("unix:{}", canonical_unix_seconds(clock_start_at).unwrap_or(0)),
            "sleepRequestedAt": format!("unix:{}", canonical_unix_seconds(before_at).unwrap_or(0)),
            "sleepEnteredAt": format!("unix:{}", canonical_unix_seconds(before_at).unwrap_or(0)),
            "wakeAt": format!("unix:{}", canonical_unix_seconds(after_at).unwrap_or(0)),
        });
        let evidence = host_sleep_evidence(
            &Scenario::HostSleepBeforeDeadline,
            &sentinel_value,
            Ok(&host_sleep_step(None)),
            &snapshot,
        );
        assert!(
            evidence.as_object().is_some(),
            "the complete host-sleep evidence must be non-null from the final snapshot"
        );
    }

    #[test]
    fn active_process_evidence_uses_the_sleep_entered_handoff_not_wake() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(920);
        let base = UNIX_EPOCH + Duration::from_secs(100);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(1),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(2),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(10),
            },
        ];
        let sleep_entered = base + Duration::from_secs(4);
        let wake = base + Duration::from_secs(30);
        let evidence = active_process_evidence(
            &events,
            Some(sleep_entered),
            "run-scope-host-sleep",
            ProcessOperation::DeviceCopy,
        )
        .expect("a complete host-sleep lifecycle capture must serialize activeProcess");
        assert_eq!(evidence["operationClass"], "device_copy");
        assert_eq!(evidence["actionKind"], "operator_action");
        assert_eq!(evidence["checkedAliveAt"], "unix:102");
        assert_eq!(evidence["actionAt"], "unix:104");
        assert_ne!(
            evidence["actionAt"],
            system_time_value(Some(wake)),
            "the wake timestamp must not be substituted for the sleep-entered action"
        );
        assert_eq!(evidence["terminalAt"], "unix:110");
        assert_eq!(evidence["aliveImmediatelyBeforeAction"], true);
        assert_eq!(evidence["terminalReportedBeforeAction"], false);
        assert!(
            sleep_entered
                .duration_since(base + Duration::from_secs(2))
                .expect("the handoff must follow the liveness sample")
                <= ACTIVE_SAMPLE_FRESHNESS,
            "the pre-suspend liveness freshness bound must hold"
        );
    }

    #[test]
    fn active_process_evidence_rejects_a_stale_pre_suspend_liveness_sample() {
        let operation_id = OwnedProcessOperationId::from_raw_for_test(921);
        let base = UNIX_EPOCH + Duration::from_secs(200);
        let events = vec![
            OwnedProcessLifecycleEvent::Spawned {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base,
            },
            OwnedProcessLifecycleEvent::MutationStarted {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(1),
            },
            OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(2),
                alive: Some(true),
                terminal_reported: false,
            },
            OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation: ProcessOperation::DeviceCopy,
                at: base + Duration::from_secs(20),
            },
        ];
        let stale_handoff = base + Duration::from_secs(9);
        assert!(
            active_process_evidence(
                &events,
                Some(stale_handoff),
                "run-scope-host-sleep",
                ProcessOperation::DeviceCopy,
            )
            .is_none(),
            "a pre-suspend liveness sample older than the freshness bound must not qualify"
        );
    }

    #[test]
    #[ignore]
    fn watcher_helper() {
        std::thread::sleep(Duration::from_secs(5));
    }
}

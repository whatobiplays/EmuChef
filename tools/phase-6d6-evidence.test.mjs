import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  CONDITIONAL_SCENARIOS,
  LEGACY_AUDIT_CONTRACTS,
  MANDATORY_SCENARIOS,
  REQUIRED_REPETITIONS,
  SCENARIOS,
  UI_SMOKE_SCENARIO,
  UI_SMOKE_SUBCASES,
  UI_SMOKE_CONTRACTS,
  canonicalDigest,
  evidenceRecordDigest,
  scenarioContractFor,
  validateEvidenceManifest,
  validateEvidenceRecord,
  validateGateEnvironment,
  validateRunbookCommands,
  validateUiSmokeRecord,
} from "./phase-6d6-evidence.mjs";

const BASE_RECORD = {
  schemaVersion: 1,
  scenario: "cancellation_active",
  repetition: 1,
  timestamp: "unix:1785790000",
  commit: "a".repeat(40),
  runId: "physical-run-sha256:" + "d".repeat(64),
  evidencePath: "docs/testing/phase-6d6/evidence/cancellation_active-rep1.json",
  tracePath: "docs/testing/phase-6d6/evidence/traces/cancellation_active-rep1.json",
  recordDigest: "sha256:" + "0".repeat(64),
  traceDigest: "sha256:" + "0".repeat(64),
  trace: { events: ["fixture-run-started", "fixture-run-terminal"] },
  host: { os: "macos", version: "15.6", architecture: "arm64" },
  platformToolsRevision: "Android Debug Bridge version 1.0.41",
  device: {
    identity: `serial-sha256:${"b".repeat(64)}`,
    model: "qualification-model",
    androidVersion: "14",
    apiLevel: 34,
    abi: "arm64-v8a",
    buildFingerprint: "vendor/device/build:14/ABC/123:user/release-keys",
  },
  root: null,
  fixtureApkSha256: "c".repeat(64),
  optIns: [
    "EMUCHEF_RUN_REAL_ADB_TESTS=1",
    "EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1",
    "EMUCHEF_PHASE_6D6_SCENARIO=cancellation_active",
    "EMUCHEF_PHASE_6D6_REPETITION=1",
    "EMUCHEF_TEST_DEVICE_SERIAL=selected",
    "EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture",
    "EMUCHEF_PHASE_6D6_SENTINEL_DIR=test-owned-directory",
  ],
  command:
    "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution physical_interruption_qualification::manual_phase_6d6_physical_interruption_qualification -- --ignored --exact",
  preparation: "safe fixture preparation",
  operatorAction: "Required bounded sentinel action was requested from the operator.",
  observedIssueCode: null,
  stepStates: {
    executed: 2,
    skipped: 0,
    failed: 0,
    cancelled: 0,
    blocked: 0,
    notAttempted: 0,
  },
  partialChangesPossible: false,
  authorityInvalidated: false,
  activeSlotReleased: true,
  activeSlotObservation: {
    observed: true,
    acquired: true,
    released: true,
    runId: "run-scope-sha256:" + "d".repeat(64),
    executionId: "execution-sha256:" + "d".repeat(64),
    acquiredAt: "unix:1785790000",
    terminalCleanupAt: "unix:1785790004",
    releasedAt: "unix:1785790004",
    sourceKind: "production_owned_slot",
    evidence: "production-execution-session-slot",
  },
  activeProcess: null,
  activeCancellation: null,
  hostSleep: null,
  identityTransition: null,
  authorizationTransition: null,
  cleanup: {
    command: "adb -s <selected-serial> shell rm -f <fixture-owned-path>",
    outcome: "succeeded",
  },
  residualStateCheck: { outcome: "clean", residuals: [] },
  outcome: "passed",
  notes: ["Sanitized evidence only."],
};

function sealRecord(record) {
  record.traceDigest = `sha256:${canonicalDigest(record.trace)}`;
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}

function measuredHostClock({
  phase,
  classification = "suspended_time_excluded",
  beforeNs = 1_000_000_000,
  afterNs = 1_050_000_000,
  terminalNs = 3_000_000_000,
  suspendedWallMs = 2_000,
  remainingBeforeSleepMs = 4_000,
  remainingAfterWakeMs = 3_950,
} = {}) {
  return {
    deadlineClockStartNs: "monotonic-ns:0",
    deadlineClockBeforeSleepNs: `monotonic-ns:${beforeNs}`,
    deadlineClockAfterWakeNs: `monotonic-ns:${afterNs}`,
    deadlineClockTerminalNs: `monotonic-ns:${terminalNs}`,
    suspendedWallMs,
    deadlineClockAdvanceDuringSuspensionMs: (afterNs - beforeNs) / 1_000_000,
    remainingBeforeSleepMs,
    remainingAfterWakeMs,
    measurementToleranceMs: 100,
    toleranceRationale: "One hundred milliseconds bounds marker and scheduler jitter.",
    operatorActionPhase: phase,
    deadlineClockSource: "owned_process_monotonic_deadline_clock",
    timerClassification: classification,
  };
}

function physicalIdentity(scenario, repetition) {
  return Buffer.from(`${scenario}:${repetition}`).toString("hex").padEnd(64, "0").slice(0, 64);
}

function physicalTrace(scenario, repetition) {
  const unique = physicalIdentity(scenario, repetition);
  return {
    runId: "physical-run-sha256:" + unique,
    events: [`${scenario}:${repetition}:started`, `${scenario}:${repetition}:terminal`],
  };
}

function recordForScenario(scenario, repetition) {
  const contract = scenarioContractFor(scenario);
  const unique = physicalIdentity(scenario, repetition);
  const issue = scenario === "usb_disconnect_active"
    ? "device_transport_lost"
    : contract.allowedIssueCodes.find((code) => code !== null) ?? null;
  const state = contract.allowedStepStates[0];
  const scenarioOptIns = {
    root_revocation: [
      "EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1",
      "EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1",
      "EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST=committed-root-prefixes",
    ],
    device_unauthorized: ["EMUCHEF_PHASE_6D6_AUTHORIZATION_RESET=1"],
    identity_replacement: ["EMUCHEF_PHASE_6D6_IDENTITY_REPLACEMENT=1"],
    low_storage: ["EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE=1"],
    host_sleep_before_deadline: ["EMUCHEF_PHASE_6D6_HOST_SLEEP=1"],
    host_sleep_after_deadline: ["EMUCHEF_PHASE_6D6_HOST_SLEEP=1"],
  }[scenario] ?? [];
  const record = {
    ...BASE_RECORD,
    scenario,
    repetition,
    runId: "physical-run-sha256:" + unique,
    evidencePath: `docs/testing/phase-6d6/evidence/${scenario}-rep${repetition}-${unique.slice(0, 8)}.json`,
    tracePath: `docs/testing/phase-6d6/evidence/traces/${scenario}-rep${repetition}-${unique.slice(0, 8)}.json`,
    trace: physicalTrace(scenario, repetition),
    root: contract.facts.rootShell ? { implementationVersion: "su 1.0" } : null,
    optIns: [
      ...BASE_RECORD.optIns.map((value) =>
        value.startsWith("EMUCHEF_PHASE_6D6_SCENARIO=")
          ? "EMUCHEF_PHASE_6D6_SCENARIO=" + scenario
          : value.startsWith("EMUCHEF_PHASE_6D6_REPETITION=")
            ? "EMUCHEF_PHASE_6D6_REPETITION=" + repetition
            : value,
      ),
      ...scenarioOptIns,
    ],
    executionSuccess: contract.expectedExecution === "success",
    observedIssueCode: issue,
    stepStates: state,
    partialChangesPossible: contract.partialChanges === "required" || (contract.partialChanges === "allowed" && issue !== null),
    authorityInvalidated: contract.authorityInvalidation === "required" || (contract.authorityInvalidation === "allowed" && issue !== null),
    scenarioContract: contract,
    scenarioFacts: {
      ...contract.facts,
      runScope: "run-scope-sha256:" + unique,
      operationClass: contract.activeProcess?.operationClass ?? "device_copy",
    },
    sentinel: {
      sentinelId: "sentinel-sha256:" + unique,
      nonce: "nonce-sha256:" + unique.split("").reverse().join(""),
      runId: "physical-run-sha256:" + unique,
      scenario,
      repetition,
      armedAt: "unix:1785790000",
      operationStartedAt: contract.facts.requiresSentinelAction ? "unix:1785790001" : null,
      boundaryReadyAt: contract.facts.boundaryCheckpoint ? "unix:1785790002" : null,
      operatorActionAt: contract.facts.requiresSentinelAction ? "unix:1785790003" : null,
      operationFinishedAt: "unix:1785790004",
      cleanupReadyAt: contract.facts.rootShell ? "unix:1785790005" : null,
      sleepRequestedAt: scenario.startsWith("host_sleep") ? "unix:1785790001" : null,
      sleepEnteredAt: scenario.startsWith("host_sleep") ? "unix:1785790002" : null,
      wakeAt: scenario === "host_sleep_before_deadline" ? "unix:1785790004" : scenario === "host_sleep_after_deadline" ? "unix:1785790008" : null,
      chronologyValid: true,
      uniqueMarkers: true,
    },
    activeSlotObservation: {
      observed: true,
      acquired: true,
      released: true,
      runId: "run-scope-sha256:" + unique,
      executionId: "execution-sha256:" + unique,
      acquiredAt: "unix:1785790000",
      terminalCleanupAt: "unix:1785790004",
      releasedAt: "unix:1785790004",
      sourceKind: "production_owned_slot",
      evidence: "production-execution-session-slot",
    },
    activeProcess: contract.activeProcess ? {
      runId: "run-scope-sha256:" + unique,
      operationId: "operation-sha256:" + unique,
      operationClass: contract.activeProcess.operationClass,
      childIdentity: "child-sha256:" + unique,
      spawnedAt: "unix:1785790001",
      mutationStartedAt: "unix:1785790001",
      checkedAliveAt: "unix:1785790002",
      actionAt: "unix:1785790002",
      actionKind: contract.timeout ? "deadline_reached" : "operator_action",
      terminalAt: "unix:1785790004",
      aliveImmediatelyBeforeAction: true,
      terminalReportedBeforeAction: false,
    } : null,
    timeout: contract.timeout ? structuredClone(contract.timeout) : null,
    storage: contract.facts.storage ? {
      initialFreeKib: 4 * 1024 * 1024,
      recoveryReserveKib: 1024 * 1024,
      fillerKib: 3 * 1024 * 1024 - 64 * 1024,
      finalFreeKib: 4 * 1024 * 1024,
      restoredRecoveryReserveKib: 1024 * 1024,
      reserveCreated: true,
      reserveRemoved: true,
      ownershipVerified: true,
      boundedAllocation: true,
      cleanupVerified: true,
    } : null,
    cleanup: {
      ...BASE_RECORD.cleanup,
      ownedPathDigests: [
        "path-sha256:" + unique,
        "path-sha256:" + unique.split("").reverse().join(""),
      ],
      verified: true,
      nonFixtureDeletion: false,
    },
  };
  if (scenario === "cancellation_active") {
    record.activeCancellation = {
      requestPhase: "in_flight",
      inFlightObservedAt: "unix:1785790001",
      requestedAt: "unix:1785790002",
      operationFinishedAt: "unix:1785790003",
      requestBeforeFinished: true,
      laterWorkNotAttempted: true,
      operatorEvidence: "operator acknowledged the in-flight checkpoint",
    };
  } else if (scenario === "cancellation_boundary") {
    record.activeCancellation = {
      requestPhase: "safe_boundary",
      inFlightObservedAt: "unix:1785790001",
      requestedAt: "unix:1785790003",
      operationFinishedAt: "unix:1785790002",
      requestBeforeFinished: false,
      laterWorkNotAttempted: true,
      operatorEvidence: "operator acknowledged the safe boundary checkpoint",
    };
  } else if (scenario === "host_sleep_before_deadline" || scenario === "host_sleep_after_deadline") {
    const afterDeadline = scenario === "host_sleep_after_deadline";
    if (afterDeadline) {
      record.executionSuccess = false;
      record.observedIssueCode = "operation_timed_out";
      record.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
      record.partialChangesPossible = true;
      record.authorityInvalidated = true;
      record.sentinel = {
        ...record.sentinel,
        operationFinishedAt: "unix:1785790010",
        operatorActionAt: null,
        sleepRequestedAt: "unix:1785790006",
        sleepEnteredAt: "unix:1785790006",
        wakeAt: "unix:1785790010",
      };
      record.hostSleep = {
        sleepRequestedAt: "unix:1785790006",
        sleepEnteredAt: "unix:1785790006",
        wakeAt: "unix:1785790010",
        wallElapsedMs: 9000,
        executorElapsedMs: 9000,
        deadlineMs: 5000,
        operationStartedAt: "unix:1785790001",
        terminalAt: "unix:1785790010",
        terminalOutcome: "timed_out",
        hostOs: "macOS",
        hostVersion: "15.6",
        timerImplementation: "async_io::Timer",
        toolchain: "rustc 1.85.0",
        timerClassification: "suspended_time_included",
        measurementBasis: "runner_monotonic_elapsed_and_sentinel_timestamps",
        transportLossBlockedMeasurement: false,
        elapsedBeforeSleepMs: 5000,
        ...measuredHostClock({
          phase: "after_deadline",
          classification: "suspended_time_included",
          beforeNs: 5_000_000_000,
          afterNs: 9_000_000_000,
          terminalNs: 9_000_000_000,
          suspendedWallMs: 4_000,
          remainingBeforeSleepMs: 0,
          remainingAfterWakeMs: 0,
        }),
      };
    } else {
      record.executionSuccess = true;
      record.observedIssueCode = null;
      record.stepStates = { executed: 2, skipped: 0, failed: 0, cancelled: 0, blocked: 0, notAttempted: 0 };
      record.hostSleep = {
        sleepRequestedAt: "unix:1785790001",
        sleepEnteredAt: "unix:1785790002",
        wakeAt: "unix:1785790004",
        wallElapsedMs: 5000,
        executorElapsedMs: 3000,
        deadlineMs: 5000,
        operationStartedAt: "unix:1785790000",
        terminalAt: "unix:1785790005",
        terminalOutcome: "completed",
        hostOs: "macOS",
        hostVersion: "15.6",
        timerImplementation: "async_io::Timer",
        toolchain: "rustc 1.85.0",
        timerClassification: "suspended_time_excluded",
        measurementBasis: "operator_observed",
        transportLossBlockedMeasurement: false,
        elapsedBeforeSleepMs: 1000,
        ...measuredHostClock({
          phase: "before_deadline",
          suspendedWallMs: 2_000,
        }),
      };
    }
  } else if (scenario === "identity_stability" || scenario === "identity_replacement") {
    const stable = scenario === "identity_stability";
    record.identityTransition = {
      initialSerial: "serial-sha256:" + "1".repeat(64),
      initialFingerprint: "fingerprint-sha256:" + "2".repeat(64),
      originalAttached: true,
      originalDisconnectedAt: "unix:1785790002",
      serialAbsentFrom: "unix:1785790002",
      serialAbsentUntil: "unix:1785790003",
      replacementAttachedAt: "unix:1785790004",
      replacementSerial: "serial-sha256:" + "1".repeat(64),
      replacementFingerprint: "fingerprint-sha256:" + (stable ? "2" : "3").repeat(64),
      neverSimultaneous: true,
      expectedIssueCode: stable ? null : issue,
      authorityInvalidated: !stable,
      cleanupFinalAttached: true,
      runId: "run-scope-sha256:" + unique,
    };
  } else if (scenario === "device_unauthorized") {
    record.sentinel.operationStartedAt = "unix:1785790001";
    record.sentinel.operationFinishedAt = "unix:1785790002";
    record.sentinel.boundaryReadyAt = "unix:1785790002";
    record.sentinel.operatorActionAt = "unix:1785790007";
    record.sentinel.cleanupReadyAt = "unix:1785790009";
    record.activeSlotObservation.terminalCleanupAt = "unix:1785790010";
    record.activeSlotObservation.releasedAt = "unix:1785790010";
    record.authorizationTransition = {
      initialState: "authorized",
      initialObservedAt: "unix:1785790000",
      operationStartedAt: "unix:1785790001",
      firstOperationCompletedAt: "unix:1785790002",
      revocationCheckpointAt: "unix:1785790003",
      originalDisconnectedAt: "unix:1785790004",
      serialAbsentFrom: "unix:1785790004",
      serialAbsentUntil: "unix:1785790005",
      reconnectedAt: "unix:1785790005",
      observedState: "unauthorized",
      observedAt: "unix:1785790006",
      terminalDetectedAt: "unix:1785790008",
      cleanupStartedAt: "unix:1785790009",
      cleanupCompletedAt: "unix:1785790010",
      issueCode: "device_unauthorized",
      authorityInvalidated: true,
      automaticResume: false,
      cleanupFinalState: "authorized",
      finalStateObservedAt: "unix:1785790011",
      runId: "run-scope-sha256:" + unique,
      deviceScope: record.device.identity,
    };
  }
  return sealRecord(record);
}

function uiSmokeRecord(repetition, suffix = String(repetition)) {
  const identity = (label) => Buffer.from(`${suffix}:${label}`).toString("hex").padEnd(64, "0").slice(0, 64);
  const issueByName = {
    cancellation: null,
    transport: "device_transport_lost",
    root: "root_authority_revoked",
    storage: "device_storage_exhausted",
    host_sleep: "operation_timed_out",
  };
  const physicalScenarioByName = {
    cancellation: "cancellation_active",
    transport: "usb_disconnect_active",
    root: "root_revocation",
    storage: "low_storage",
    host_sleep: "host_sleep_after_deadline",
  };
  const record = {
    schemaVersion: 1,
    scenario: UI_SMOKE_SCENARIO,
    repetition,
    timestamp: "unix:1785790100",
    commit: "a".repeat(40),
    runId: "ui-smoke-run-sha256:" + identity("composite"),
    recordDigest: "sha256:" + "0".repeat(64),
    developmentBuild: {
      identity: "emuchef-dev-build",
      version: "0.1.0-qualification",
      digest: "sha256:" + identity("development-build"),
    },
    subcases: UI_SMOKE_SUBCASES.map((name) => {
      const contract = UI_SMOKE_CONTRACTS[name];
      const physicalScenario = physicalScenarioByName[name];
      const backendRunId = "physical-run-sha256:" + physicalIdentity(physicalScenario, repetition);
      const uiState = {
        backendRunId,
        authoredTitle: contract.authoredTitle,
        authoredIssueText: contract.authoredIssueText,
        authoredRemediation: contract.authoredRemediation,
        terminalStepProjection: contract.terminalStepProjection,
        notAttempted: 1,
        partialChangePresentation: contract.partialChangePresentation,
        authorityInvalidated: contract.authorityInvalidated,
        recoveryState: contract.recoveryState,
        availableControls: [],
      };
      const artifactDigest = `sha256:${canonicalDigest(uiState)}`;
      return {
        name,
        subRunId: "ui-subrun-sha256:" + identity(`${name}:subrun`),
        backendRunId,
        backendTraceDigest: `sha256:${canonicalDigest(physicalTrace(physicalScenario, repetition))}`,
        backendIssueCode: issueByName[name],
        uiState,
        uiArtifact: {
          kind: contract.requiredArtifactKind,
          path: `docs/testing/phase-6d6/evidence/ui/${name}-rep${repetition}-${identity(name).slice(0, 8)}.json`,
          content: structuredClone(uiState),
          digest: artifactDigest,
        },
        operatorObservation: {
          artifactDigest,
          observedAt: "unix:1785790101",
          statement: `Operator verified the sanitized ${name} terminal presentation.`,
        },
      };
    }),
    outcome: "passed",
    notes: ["Sanitized development-build smoke evidence."],
  };
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}

test("the checked-in evidence template has a current canonical digest", () => {
  const templatePath = fileURLToPath(new URL(
    "../docs/testing/phase-6d6/evidence-template.json",
    import.meta.url,
  ));
  const template = JSON.parse(readFileSync(templatePath, "utf8"));
  assert.equal(template.recordDigest, evidenceRecordDigest(template));
  assert.doesNotThrow(() => validateEvidenceRecord(template));
});

test("the supported matrix partitions twelve mandatory scenarios and one conditional scenario", () => {
  assert.equal(SCENARIOS.length, 13);
  assert.equal(MANDATORY_SCENARIOS.length, 12);
  assert.deepEqual(CONDITIONAL_SCENARIOS, ["device_offline"]);
  assert.equal(REQUIRED_REPETITIONS, 2);
  assert.equal(new Set(SCENARIOS).size, SCENARIOS.length);
  assert.equal(new Set(MANDATORY_SCENARIOS).size, MANDATORY_SCENARIOS.length);
  assert.equal(new Set(CONDITIONAL_SCENARIOS).size, CONDITIONAL_SCENARIOS.length);
  assert.deepEqual(
    new Set([...MANDATORY_SCENARIOS, ...CONDITIONAL_SCENARIOS]),
    new Set(SCENARIOS),
  );
  assert.ok(!MANDATORY_SCENARIOS.includes("device_offline"));
});

test("gate validation requires one exact scenario and all safety bindings", () => {
  const environment = {
    EMUCHEF_RUN_REAL_ADB_TESTS: "1",
    EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS: "1",
    EMUCHEF_PHASE_6D6_SCENARIO: "cancellation_active",
    EMUCHEF_PHASE_6D6_REPETITION: "1",
    EMUCHEF_TEST_DEVICE_SERIAL: "selected-device",
    EMUCHEF_TEST_PACKAGE_ALLOWLIST: "com.emuchef.fixture",
    EMUCHEF_PHASE_6D6_SENTINEL_DIR: "/tmp/phase-6d6",
  };
  assert.doesNotThrow(() => validateGateEnvironment(environment));
  assert.doesNotThrow(() => validateGateEnvironment({
    ...environment,
    EMUCHEF_PHASE_6D6_SCENARIO: "device_offline",
  }));
  assert.throws(
    () => validateGateEnvironment({ ...environment, EMUCHEF_PHASE_6D6_SCENARIO: "all" }),
    /exactly one supported scenario/,
  );
  assert.throws(
    () => validateGateEnvironment({ ...environment, EMUCHEF_TEST_DEVICE_SERIAL: "" }),
    /exact serial/,
  );
  assert.throws(
    () => validateGateEnvironment({ ...environment, EMUCHEF_PHASE_6D6_SCENARIO: "low_storage" }),
    /EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE/,
  );
});

test("valid evidence is strict, sanitized, and schema-shaped", () => {
  const valid = recordForScenario("cancellation_active", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(valid));
  assert.throws(
    () => validateEvidenceRecord({ ...valid, serial: "raw-device-serial" }),
    /unknown field/,
  );
  assert.throws(
    () => validateEvidenceRecord({ ...valid, command: "adb -s raw-device-serial shell" }),
    /raw serial|private payload/,
  );
});

test("low-storage evidence enforces the bounded filler and cleanup headroom", () => {
  const valid = recordForScenario("low_storage", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(valid));
  assert.throws(
    () => validateEvidenceRecord({
      ...valid,
      storage: { ...valid.storage, fillerKib: 4 * 1024 * 1024 },
    }),
    /storage safety bounds/,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...valid,
      storage: { ...valid.storage, initialFreeKib: 5_308_417 },
    }),
    /storage safety bounds/,
  );
});

test("manifest completeness requires every mandatory scenario but not conditional offline evidence", () => {
  const incomplete = validateEvidenceManifest([recordForScenario("cancellation_active", 1)]);
  assert.equal(incomplete.complete, false);
  assert.equal(
    incomplete.missing.length,
    MANDATORY_SCENARIOS.length * REQUIRED_REPETITIONS - 1,
  );
  assert.ok(!incomplete.missing.includes("device_offline:1"));
  assert.ok(!incomplete.missing.includes("device_offline:2"));

  const mandatory = MANDATORY_SCENARIOS.flatMap((scenario) =>
    [1, 2].map((repetition) => ({
      ...recordForScenario(scenario, repetition),
      optIns: recordForScenario(scenario, repetition).optIns.map((value) =>
        value.startsWith("EMUCHEF_PHASE_6D6_SCENARIO=")
          ? `EMUCHEF_PHASE_6D6_SCENARIO=${scenario}`
          : value.startsWith("EMUCHEF_PHASE_6D6_REPETITION=")
            ? `EMUCHEF_PHASE_6D6_REPETITION=${repetition}`
            : value,
      ),
    })),
  );
  const uiSmoke = [uiSmokeRecord(1, "1"), uiSmokeRecord(2, "2")];
  assert.equal(validateEvidenceManifest([...mandatory, ...uiSmoke]).complete, true);

  const conditionalOffline = [1, 2].map((repetition) =>
    recordForScenario("device_offline", repetition),
  );
  const withConditional = validateEvidenceManifest([
    ...mandatory,
    ...conditionalOffline,
    ...uiSmoke,
  ]);
  assert.equal(withConditional.complete, true);
  assert.deepEqual(withConditional.missing, []);
});

test("legacy active authorization evidence remains auditable but cannot qualify", () => {
  const legacyContract = LEGACY_AUDIT_CONTRACTS.device_unauthorized[0];
  const blocked = recordForScenario("device_unauthorized", 1);
  blocked.outcome = "blocked";
  blocked.executionSuccess = true;
  blocked.observedIssueCode = null;
  blocked.stepStates = {
    executed: 2,
    skipped: 0,
    failed: 0,
    cancelled: 0,
    blocked: 0,
    notAttempted: 0,
  };
  blocked.partialChangesPossible = false;
  blocked.authorityInvalidated = false;
  blocked.scenarioContract = structuredClone(legacyContract);
  blocked.scenarioFacts = {
    ...legacyContract.facts,
    runScope: blocked.scenarioFacts.runScope,
    operationClass: "host_push",
  };
  blocked.authorizationTransition = null;
  blocked.sentinel.boundaryReadyAt = null;
  blocked.sentinel.operatorActionAt = "unix:1785790002";
  blocked.sentinel.operationFinishedAt = "unix:1785790004";
  blocked.sentinel.cleanupReadyAt = "unix:1785790005";
  blocked.activeSlotObservation.terminalCleanupAt = "unix:1785790004";
  blocked.activeSlotObservation.releasedAt = "unix:1785790004";
  blocked.activeProcess = structuredClone(recordForScenario("cancellation_active", 1).activeProcess);
  blocked.activeProcess.runId = blocked.scenarioFacts.runScope;
  blocked.activeProcess.spawnedAt = "unix:1785790001";
  blocked.activeProcess.mutationStartedAt = "unix:1785790001";
  blocked.activeProcess.checkedAliveAt = "unix:1785790002";
  blocked.activeProcess.actionAt = "unix:1785790002";
  blocked.activeProcess.terminalAt = "unix:1785790004";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(blocked)));

  const promoted = structuredClone(blocked);
  promoted.outcome = "passed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(promoted)),
    /approved non-passing audit snapshot|scenario contract/i,
  );
});

test("legacy safe-boundary authorization evidence remains auditable but cannot qualify", () => {
  const legacyContract = LEGACY_AUDIT_CONTRACTS.device_unauthorized[1];
  const blocked = recordForScenario("device_unauthorized", 1);
  blocked.outcome = "blocked";
  blocked.observedIssueCode = "device_identity_unverified";
  blocked.scenarioContract = structuredClone(legacyContract);
  blocked.authorizationTransition = null;
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(blocked)));

  const promoted = structuredClone(blocked);
  promoted.outcome = "passed";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(promoted)),
    /approved non-passing audit snapshot|scenario contract/i,
  );
});

test("semantic scenario contracts accept expected failures and reject relabelled evidence", () => {
  const timeout = recordForScenario("operation_timeout", 1);
  timeout.outcome = "passed";
  timeout.observedIssueCode = "operation_timed_out";
  timeout.partialChangesPossible = false;
  timeout.authorityInvalidated = true;
  timeout.stepStates = { executed: 0, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 1 };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(timeout)));

  assert.throws(
    () => validateEvidenceRecord({ ...timeout, observedIssueCode: "device_storage_exhausted" }),
    /scenario contract|issue code|expected/i,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...timeout,
      scenario: "low_storage",
      optIns: timeout.optIns.map((value) =>
        value.startsWith("EMUCHEF_PHASE_6D6_SCENARIO=")
          ? "EMUCHEF_PHASE_6D6_SCENARIO=low_storage"
          : value,
      ).concat("EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE=1"),
      scenarioContract: scenarioContractFor("operation_timeout"),
    }),
    /scenario contract|scenario identity/i,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...timeout,
      activeSlotObservation: { observed: false, acquired: false, released: true, runId: "run-scope-sha256:" + "d".repeat(64), evidence: "hard-coded" },
    }),
    /active slot/i,
  );
  const interruption = recordForScenario("cancellation_active", 1);
  interruption.executionSuccess = true;
  assert.throws(() => validateEvidenceRecord(interruption), /interrupted execution/);
  const root = recordForScenario("root_revocation", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(root));
  assert.throws(
    () => validateEvidenceRecord({
      ...root,
      sentinel: { ...root.sentinel, cleanupReadyAt: null },
    }),
    /cleanup authority/,
  );
});

test("failed physical attempts retain non-clean evidence without qualifying", () => {
  const failed = recordForScenario("operation_timeout", 1);
  failed.outcome = "failed";
  failed.observedIssueCode = "device_transport_lost";
  failed.activeSlotReleased = false;
  failed.activeSlotObservation = {
    observed: true,
    acquired: true,
    released: false,
    runId: failed.scenarioFacts.runScope,
    executionId: failed.activeSlotObservation.executionId,
    acquiredAt: failed.activeSlotObservation.acquiredAt,
    terminalCleanupAt: null,
    releasedAt: null,
    sourceKind: "production_owned_slot",
    evidence: "production-execution-session-slot",
  };
  failed.cleanup = { ...failed.cleanup, outcome: "failed", verified: false };
  failed.residualStateCheck = {
    outcome: "residual",
    residuals: ["fixture-owned residual path"],
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(failed)));
  assert.equal(validateEvidenceManifest([failed]).complete, false);
});

test("timeout metadata stays strict on failed records while historical omissions remain compatible", () => {
  const failed = recordForScenario("operation_timeout", 1);
  failed.outcome = "failed";
  failed.observedIssueCode = "device_transport_lost";
  failed.activeSlotReleased = false;
  failed.activeSlotObservation = {
    observed: true,
    acquired: true,
    released: false,
    runId: failed.scenarioFacts.runScope,
    executionId: failed.activeSlotObservation.executionId,
    acquiredAt: failed.activeSlotObservation.acquiredAt,
    terminalCleanupAt: null,
    releasedAt: null,
    sourceKind: "production_owned_slot",
    evidence: "production-execution-session-slot",
  };
  failed.cleanup = { ...failed.cleanup, outcome: "failed", verified: false };
  failed.residualStateCheck = { outcome: "residual", residuals: ["fixture-owned residual path"] };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(failed)));

  for (const [field, value] of [
    ["productionDeadlineMs", 1],
    ["qualificationDeadlineMs", 1],
    ["deadlineSource", "unscoped_override"],
  ]) {
    const invalid = structuredClone(failed);
    invalid.timeout[field] = value;
    assert.throws(() => validateEvidenceRecord(sealRecord(invalid)), /timeout|deadline/i);
  }

  const missingTimeout = structuredClone(failed);
  delete missingTimeout.timeout;
  assert.throws(() => validateEvidenceRecord(sealRecord(missingTimeout)), /timeout metadata/i);

  for (const cleanup of ["uncertain", "not_observed"]) {
    const incompleteCleanup = structuredClone(failed);
    incompleteCleanup.timeout.processCleanup = cleanup;
    assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(incompleteCleanup)));
    assert.equal(validateEvidenceManifest([incompleteCleanup]).complete, false);
  }

  const missingActionKind = structuredClone(failed);
  delete missingActionKind.activeProcess.actionKind;
  assert.throws(() => validateEvidenceRecord(sealRecord(missingActionKind)), /actionKind/i);

  const failedWithoutProcess = structuredClone(failed);
  failedWithoutProcess.activeProcess = null;
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(failedWithoutProcess)));

  const passingWithoutProcess = recordForScenario("operation_timeout", 1);
  passingWithoutProcess.activeProcess = null;
  assert.throws(() => validateEvidenceRecord(sealRecord(passingWithoutProcess)), /target process|active mutation/i);

  const historical = recordForScenario("cancellation_active", 1);
  delete historical.timeout;
  delete historical.activeProcess.actionKind;
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(historical)));
});

test("runbook validation requires ignored execution, sentinel timeout, and explicit gates", () => {
  const runbook = [
    "EMUCHEF_RUN_REAL_ADB_TESTS=1 EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1",
    "EMUCHEF_PHASE_6D6_SCENARIO=cancellation_active",
    "EMUCHEF_PHASE_6D6_REPETITION=1 EMUCHEF_TEST_DEVICE_SERIAL=<selected>",
    "EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture",
    "EMUCHEF_PHASE_6D6_SENTINEL_DIR=/tmp/phase-6d6",
    "root cleanup-ready marker uses ack",
    "authorization-revoked uses ack; unauthorized-observed precedes operator-action",
    "sleep-requested sleep-entered wake",
    "production runner lifecycle reports in_flight; serial absence is observed",
    "device_offline is conditional diagnostic evidence",
    "UI smoke is mandatory closure evidence",
    "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution -- --ignored --exact",
    "bounded sentinel timeout: 600 seconds",
  ].join("\n");
  assert.doesNotThrow(() => validateRunbookCommands(runbook));
  assert.throws(() => validateRunbookCommands(runbook.replace("--ignored", "")), /--ignored/);
  assert.throws(() => validateRunbookCommands(runbook.replace("600 seconds", "forever")), /600/);
});

test("host-sleep qualification requires measured timer evidence and accepts both legitimate branches", () => {
  const base = recordForScenario("host_sleep_before_deadline", 1);
  base.hostSleep = null;
  base.outcome = "passed";
  base.executionSuccess = true;
  base.observedIssueCode = null;
  base.stepStates = { executed: 2, skipped: 0, failed: 0, cancelled: 0, blocked: 0, notAttempted: 0 };
  assert.throws(() => validateEvidenceRecord(base), /issue|expected|host sleep|timer/i);

  const completed = {
    ...base,
    hostSleep: {
      sleepRequestedAt: "unix:1785790001",
      sleepEnteredAt: "unix:1785790002",
      wakeAt: "unix:1785790004",
      wallElapsedMs: 4000,
      executorElapsedMs: 3000,
      deadlineMs: 5000,
      operationStartedAt: "unix:1785790001",
      terminalAt: "unix:1785790005",
      terminalOutcome: "completed",
      hostOs: "macOS",
      hostVersion: "15.6",
      timerImplementation: "async_io::Timer",
      toolchain: "rustc 1.85.0",
      timerClassification: "suspended_time_excluded",
      measurementBasis: "operator_observed",
      transportLossBlockedMeasurement: false,
      elapsedBeforeSleepMs: 0,
      ...measuredHostClock({
        phase: "before_deadline",
        beforeNs: 0,
        afterNs: 50_000_000,
        terminalNs: 3_000_000_000,
        remainingBeforeSleepMs: 5_000,
        remainingAfterWakeMs: 4_950,
      }),
    },
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(completed)));

  const timedOut = {
    ...completed,
    executionSuccess: false,
    observedIssueCode: "operation_timed_out",
    stepStates: { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 },
    hostSleep: {
      ...completed.hostSleep,
      terminalAt: "unix:1785790007",
      wallElapsedMs: 6_000,
      terminalOutcome: "timed_out",
      executorElapsedMs: 6_000,
      ...measuredHostClock({
        phase: "before_deadline",
        classification: "suspended_time_included",
        beforeNs: 0,
        afterNs: 2_000_000_000,
        terminalNs: 6_000_000_000,
        remainingBeforeSleepMs: 5_000,
        remainingAfterWakeMs: 3_000,
      }),
    },
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(timedOut)));
});

test("host-sleep negatives cannot qualify generic or contradictory timing", () => {
  const base = recordForScenario("host_sleep_after_deadline", 1);
  base.hostSleep = null;
  base.outcome = "passed";
  base.executionSuccess = false;
  base.observedIssueCode = "operation_timed_out";
  base.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
  const timing = {
    sleepRequestedAt: "unix:1785790007", sleepEnteredAt: "unix:1785790007", wakeAt: "unix:1785790008",
    wallElapsedMs: 8000, executorElapsedMs: 8000, deadlineMs: 5000,
    operationStartedAt: "unix:1785790001", terminalAt: "unix:1785790009", terminalOutcome: "timed_out",
    hostOs: "macOS", hostVersion: "15.6", timerImplementation: "async_io::Timer",
    toolchain: "rustc 1.85.0", timerClassification: "suspended_time_included",
    measurementBasis: "operator_observed", transportLossBlockedMeasurement: false,
    elapsedBeforeSleepMs: 6000,
    ...measuredHostClock({
      phase: "after_deadline",
      classification: "suspended_time_included",
      beforeNs: 6_000_000_000,
      afterNs: 7_000_000_000,
      terminalNs: 8_000_000_000,
      suspendedWallMs: 1_000,
      remainingBeforeSleepMs: 0,
      remainingAfterWakeMs: 0,
    }),
  };
  assert.throws(() => validateEvidenceRecord(base), /host sleep|timer/i);
  assert.throws(() => validateEvidenceRecord({ ...base, hostSleep: { ...timing, wakeAt: "unix:1001" } }), /chronology|sleep|wake/i);
  assert.throws(() => validateEvidenceRecord({ ...base, hostSleep: { ...timing, wallElapsedMs: 1 } }), /duration|elapsed|timer/i);
  assert.throws(() => validateEvidenceRecord({ ...base, hostSleep: { ...timing, timerClassification: "indeterminate" } }), /indeterminate|timer/i);
  assert.throws(() => validateEvidenceRecord({ ...base, hostSleep: { ...timing, timerClassification: "contradictory" } }), /contradictory|timer/i);
  assert.throws(() => validateEvidenceRecord({ ...base, hostSleep: { ...timing, terminalOutcome: "transport_loss", timerClassification: "suspended_time_included", transportLossBlockedMeasurement: false } }), /transport|timer/i);
});

test("post-threshold host sleep accepts a measured suspended-time-included timeout branch", () => {
  const after = recordForScenario("host_sleep_after_deadline", 1);
  after.executionSuccess = false;
  after.observedIssueCode = "operation_timed_out";
  after.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
  after.partialChangesPossible = true;
  after.authorityInvalidated = true;
  after.sentinel = {
    ...after.sentinel,
    operationFinishedAt: "unix:1785790010",
    operatorActionAt: null,
    sleepRequestedAt: "unix:1785790006",
    sleepEnteredAt: "unix:1785790006",
    wakeAt: "unix:1785790010",
  };
  after.hostSleep = {
    sleepRequestedAt: "unix:1785790006",
    sleepEnteredAt: "unix:1785790006",
    wakeAt: "unix:1785790010",
    wallElapsedMs: 9000,
    executorElapsedMs: 9000,
    deadlineMs: 5000,
    operationStartedAt: "unix:1785790001",
    terminalAt: "unix:1785790010",
    terminalOutcome: "timed_out",
    hostOs: "macOS",
    hostVersion: "15.6",
    timerImplementation: "async_io::Timer",
    toolchain: "rustc 1.85.0",
    timerClassification: "suspended_time_included",
    measurementBasis: "runner_monotonic_elapsed_and_sentinel_timestamps",
    transportLossBlockedMeasurement: false,
    elapsedBeforeSleepMs: 5000,
    ...measuredHostClock({
      phase: "after_deadline",
      classification: "suspended_time_included",
      beforeNs: 5_000_000_000,
      afterNs: 9_000_000_000,
      terminalNs: 9_000_000_000,
      suspendedWallMs: 4_000,
      remainingBeforeSleepMs: 0,
      remainingAfterWakeMs: 0,
    }),
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(after)));
});

test("host sleep accepts an owner terminal that precedes the retained-basis post-wake sample", () => {
  const after = recordForScenario("host_sleep_after_deadline", 1);
  after.executionSuccess = false;
  after.observedIssueCode = "operation_timed_out";
  after.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
  after.partialChangesPossible = true;
  after.authorityInvalidated = true;
  after.sentinel = {
    ...after.sentinel,
    operationFinishedAt: "unix:1785790010",
    operatorActionAt: null,
    sleepRequestedAt: "unix:1785790006",
    sleepEnteredAt: "unix:1785790006",
    wakeAt: "unix:1785790010",
  };
  after.hostSleep = {
    sleepRequestedAt: "unix:1785790006",
    sleepEnteredAt: "unix:1785790006",
    wakeAt: "unix:1785790010",
    wallElapsedMs: 8000,
    executorElapsedMs: 9000,
    deadlineMs: 5000,
    operationStartedAt: "unix:1785790001",
    terminalAt: "unix:1785790009",
    terminalOutcome: "timed_out",
    hostOs: "macOS",
    hostVersion: "15.6",
    timerImplementation: "async_io::Timer",
    toolchain: "rustc 1.85.0",
    timerClassification: "suspended_time_included",
    measurementBasis: "owned_process_monotonic_deadline_clock_samples_and_sentinel_timestamps",
    transportLossBlockedMeasurement: false,
    elapsedBeforeSleepMs: 5000,
    ...measuredHostClock({
      phase: "after_deadline",
      classification: "suspended_time_included",
      beforeNs: 5_000_000_000,
      afterNs: 9_050_000_000,
      terminalNs: 9_000_000_000,
      suspendedWallMs: 4_000,
      remainingBeforeSleepMs: 0,
      remainingAfterWakeMs: 0,
    }),
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(after)));
});

test("host sleep accepts completion when the owner won before the timer at the deadline boundary", () => {
  const before = recordForScenario("host_sleep_before_deadline", 1);
  before.executionSuccess = true;
  before.observedIssueCode = null;
  before.stepStates = { executed: 2, skipped: 0, failed: 0, cancelled: 0, blocked: 0, notAttempted: 0 };
  before.sentinel.operationFinishedAt = "unix:1785790006";
  before.hostSleep = {
    ...before.hostSleep,
    terminalAt: "unix:1785790006",
    wallElapsedMs: 6000,
    executorElapsedMs: 6000,
    terminalOutcome: "completed",
    ...measuredHostClock({
      phase: "before_deadline",
      classification: "suspended_time_excluded",
      beforeNs: 1_000_000_000,
      afterNs: 1_050_000_000,
      terminalNs: 6_000_000_000,
    }),
  };
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(before)));
});

test("host sleep deadline phase derives from the exact deadline-clock start, not the sentinel progress marker", () => {
  const before = recordForScenario("host_sleep_before_deadline", 1);
  before.sentinel = {
    ...before.sentinel,
    operationStartedAt: "unix:1785790000",
    operationFinishedAt: "unix:1785790006",
    sleepRequestedAt: "unix:1785790004",
    sleepEnteredAt: "unix:1785790004",
    wakeAt: "unix:1785790005",
  };
  before.hostSleep = {
    ...before.hostSleep,
    operationStartedAt: "unix:1785790004",
    sleepRequestedAt: "unix:1785790004",
    sleepEnteredAt: "unix:1785790004",
    wakeAt: "unix:1785790005",
    terminalAt: "unix:1785790006",
    wallElapsedMs: 2000,
    executorElapsedMs: 2000,
    elapsedBeforeSleepMs: 0,
    terminalOutcome: "completed",
    ...measuredHostClock({
      phase: "before_deadline",
      classification: "suspended_time_excluded",
      beforeNs: 1_000_000_000,
      afterNs: 1_050_000_000,
      terminalNs: 2_000_000_000,
      suspendedWallMs: 1_000,
      remainingBeforeSleepMs: 4_000,
      remainingAfterWakeMs: 3_950,
    }),
  };
  // The sentinel progress marker alone would place wake at 5 seconds, at the
  // 5-second deadline; the exact deadline-clock start at 4 seconds leaves
  // wake before the deadline, so the before-deadline branch must qualify.
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(before)));
  const after = {
    ...before,
    scenario: "host_sleep_after_deadline",
    optIns: before.optIns.map((value) =>
      value === "EMUCHEF_PHASE_6D6_SCENARIO=host_sleep_before_deadline"
        ? "EMUCHEF_PHASE_6D6_SCENARIO=host_sleep_after_deadline"
        : value,
    ),
    scenarioContract: scenarioContractFor("host_sleep_after_deadline"),
    hostSleep: {
      ...before.hostSleep,
      operatorActionPhase: "after_deadline",
    },
  };
  assert.throws(
    () => validateEvidenceRecord(after),
    /phase|threshold/i,
    "the earlier sentinel progress marker must not qualify the after-deadline branch",
  );
});

test("active cancellation evidence is exact and slot evidence names the observed run lifecycle", () => {
  const active = recordForScenario("cancellation_active", 1);
  active.outcome = "passed";
  active.executionSuccess = false;
  active.observedIssueCode = "device_transport_lost";
  active.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
  assert.throws(() => validateEvidenceRecord(active), /cancellation|scenario contract|transport/i);
  const valid = recordForScenario("cancellation_active", 1);
  valid.activeSlotObservation = {
    observed: true,
    acquired: true,
    released: true,
    runId: "run-scope-sha256:" + "d".repeat(64),
    evidence: "private-session-terminal-observation",
  };
  assert.throws(() => validateEvidenceRecord(valid), /active slot|run lifecycle/i);
  const releaseWithoutAcquisition = recordForScenario("cancellation_active", 1);
  releaseWithoutAcquisition.activeSlotObservation = {
    observed: true,
    acquired: false,
    released: true,
    runId: "run-scope-sha256:" + "d".repeat(64),
    evidence: "production-execution-session-slot",
  };
  assert.throws(() => validateEvidenceRecord(releaseWithoutAcquisition), /release|acquisition|active slot/i);
  const crossRun = recordForScenario("cancellation_active", 1);
  crossRun.activeSlotObservation.runId = "run-scope-sha256:" + "e".repeat(64);
  assert.throws(() => validateEvidenceRecord(crossRun), /another run|active slot/i);
  const missingRelease = recordForScenario("cancellation_active", 1);
  missingRelease.activeSlotReleased = false;
  missingRelease.activeSlotObservation.released = false;
  assert.throws(() => validateEvidenceRecord(missingRelease), /release|slot/i);
});

test("UI smoke is two distinct composite repetitions covering every required subcase", () => {
  const first = uiSmokeRecord(1, "1");
  const second = uiSmokeRecord(2, "2");
  assert.doesNotThrow(() => validateUiSmokeRecord(first));
  assert.doesNotThrow(() => validateUiSmokeRecord(second));
  const incomplete = validateEvidenceManifest([first, ...SCENARIOS.flatMap((scenario) => [1, 2].map((repetition) => recordForScenario(scenario, repetition)))]);
  assert.equal(incomplete.complete, false);
  assert.deepEqual(incomplete.missingUiSmoke, ["ui_smoke_composite:2"]);
  assert.throws(() => validateUiSmokeRecord({ ...first, subcases: first.subcases.slice(0, 4) }), /five subcases/);
  assert.throws(() => validateUiSmokeRecord({ ...first, subcases: [first.subcases[0], ...first.subcases] }), /five|distinct|required/);
  const reused = uiSmokeRecord(2, "2");
  reused.subcases[1] = { ...reused.subcases[1], subRunId: first.subcases[0].subRunId };
  reused.recordDigest = evidenceRecordDigest(reused);
  assert.throws(
    () => validateEvidenceManifest([
      ...SCENARIOS.flatMap((scenario) => [1, 2].map((repetition) => recordForScenario(scenario, repetition))),
      first,
      reused,
    ]),
    /reused|sub-run|copied/i,
  );
  assert.equal(validateEvidenceManifest([
    ...SCENARIOS.flatMap((scenario) => [1, 2].map((repetition) => recordForScenario(scenario, repetition))),
    first,
    second,
  ]).complete, true);
});

test("identity and authorization transitions require ordered, genuine observations", () => {
  const identity = recordForScenario("identity_replacement", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(identity));
  assert.throws(
    () => validateEvidenceRecord({
      ...identity,
      identityTransition: {
        ...identity.identityTransition,
        replacementAttachedAt: "unix:1785790002",
      },
    }),
    /ordering|replacement/,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...identity,
      identityTransition: {
        ...identity.identityTransition,
        replacementFingerprint: identity.identityTransition.initialFingerprint,
      },
    }),
    /serial|fingerprint|identity/,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...identity,
      identityTransition: {
        ...identity.identityTransition,
        runId: "run-scope-sha256:" + "e".repeat(64),
      },
    }),
    /another run|identity/,
  );

  const authorization = recordForScenario("device_unauthorized", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(authorization)));
  const identityPrecedence = structuredClone(authorization);
  identityPrecedence.observedIssueCode = "device_identity_unverified";
  identityPrecedence.authorizationTransition.issueCode = "device_identity_unverified";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(identityPrecedence)));

  const missingTransition = structuredClone(identityPrecedence);
  missingTransition.authorizationTransition = null;
  assert.throws(
    () => validateEvidenceRecord(sealRecord(missingTransition)),
    /authorization revocation requires measured transition evidence/i,
  );

  const mismatchedIdentityBranch = structuredClone(identityPrecedence);
  mismatchedIdentityBranch.authorizationTransition.issueCode = "device_unauthorized";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(mismatchedIdentityBranch)),
    /authorization|terminal branch|issue/i,
  );
  for (const invalidIssue of [
    "device_transport_lost",
    "device_offline",
    "device_disconnected",
    "device_identity_changed",
  ]) {
    assert.throws(
      () => validateEvidenceRecord(sealRecord({
        ...authorization,
        observedIssueCode: invalidIssue,
        authorizationTransition: {
          ...authorization.authorizationTransition,
          issueCode: invalidIssue,
        },
      })),
      /authorization|terminal branch|issue|contract/,
    );
  }
  assert.throws(
    () => validateEvidenceRecord({
      ...authorization,
      authorizationTransition: {
        ...authorization.authorizationTransition,
        observedAt: "unix:1785790001",
      },
    }),
    /before|authorization/,
  );
  assert.throws(
    () => validateEvidenceRecord({
      ...authorization,
      authorizationTransition: {
        ...authorization.authorizationTransition,
        automaticResume: true,
      },
    }),
    /automatic resume|authorization/,
  );
});

test("active interruption cannot pass without exact live child evidence", () => {
  const active = recordForScenario("cancellation_active", 1);
  active.activeProcess = null;
  assert.throws(() => validateEvidenceRecord(active), /target process|child|active mutation/i);

  const relabelledProcess = recordForScenario("cancellation_active", 1);
  relabelledProcess.activeProcess.operationClass = "device_copy";
  assert.throws(() => validateEvidenceRecord(relabelledProcess), /another run or operation|operation class/i);

  const relabelledFacts = recordForScenario("cancellation_active", 1);
  relabelledFacts.scenarioFacts.operationClass = "device_copy";
  assert.throws(() => validateEvidenceRecord(relabelledFacts), /operation class|scenario contract/i);
});

test("operation timeout cannot qualify with altered deadline, liveness, chronology, or operator evidence", () => {
  const valid = recordForScenario("operation_timeout", 1);
  assert.doesNotThrow(() => validateEvidenceRecord(valid));
  const cases = [
    ["production deadline", (record) => { record.timeout.productionDeadlineMs = 15_000; }],
    ["qualification deadline", (record) => { record.timeout.qualificationDeadlineMs = 16_000; }],
    ["deadline source", (record) => { record.timeout.deadlineSource = "process_delay"; }],
    ["uncertain cleanup", (record) => { record.timeout.processCleanup = "uncertain"; }],
    ["missing timeout", (record) => { delete record.timeout; }],
    ["wrong operation class", (record) => { record.activeProcess.operationClass = "host_push"; }],
    ["unknown liveness", (record) => { record.activeProcess.aliveImmediatelyBeforeAction = false; }],
    ["terminal reported", (record) => { record.activeProcess.terminalReportedBeforeAction = true; }],
    ["deadline before liveness", (record) => { record.activeProcess.actionAt = "unix:1"; }],
    ["terminal before deadline", (record) => { record.activeProcess.terminalAt = "unix:1"; }],
    ["operator action kind", (record) => { record.activeProcess.actionKind = "operator_action"; }],
    ["operator marker", (record) => { record.sentinel.operatorActionAt = "unix:2"; }],
  ];
  for (const [label, mutate] of cases) {
    const candidate = recordForScenario("operation_timeout", 1);
    mutate(candidate);
    sealRecord(candidate);
    assert.throws(
      () => validateEvidenceRecord(candidate),
      undefined,
      `${label} must not qualify`,
    );
  }
});

test("host sleep enforces the manifest phase instead of reusing before-threshold evidence", () => {
  const before = recordForScenario("host_sleep_before_deadline", 1);
  const relabelled = {
    ...before,
    scenario: "host_sleep_after_deadline",
    optIns: before.optIns.map((value) =>
      value === "EMUCHEF_PHASE_6D6_SCENARIO=host_sleep_before_deadline"
        ? "EMUCHEF_PHASE_6D6_SCENARIO=host_sleep_after_deadline"
        : value,
    ),
    scenarioContract: scenarioContractFor("host_sleep_after_deadline"),
  };
  assert.throws(() => validateEvidenceRecord(relabelled), /phase|threshold/i);
});

test("excluded suspension may time out later from active time without changing classification", () => {
  const record = recordForScenario("host_sleep_before_deadline", 1);
  record.executionSuccess = false;
  record.observedIssueCode = "operation_timed_out";
  record.stepStates = { executed: 1, skipped: 0, failed: 1, cancelled: 0, blocked: 0, notAttempted: 0 };
  record.hostSleep = {
    ...record.hostSleep,
    terminalAt: "unix:1785790010",
    wallElapsedMs: 10_000,
    terminalOutcome: "timed_out",
    executorElapsedMs: 6_000,
    timerClassification: "suspended_time_excluded",
    deadlineClockStartNs: "monotonic-ns:0",
    deadlineClockBeforeSleepNs: "monotonic-ns:1000000000",
    deadlineClockAfterWakeNs: "monotonic-ns:1050000000",
    deadlineClockTerminalNs: "monotonic-ns:6000000000",
    suspendedWallMs: 2_000,
    deadlineClockAdvanceDuringSuspensionMs: 50,
    remainingBeforeSleepMs: 4_000,
    remainingAfterWakeMs: 3_950,
    measurementToleranceMs: 100,
    toleranceRationale: "One hundred milliseconds bounds marker and scheduler jitter.",
    operatorActionPhase: "before_deadline",
  };
  record.sentinel.operationFinishedAt = "unix:1785790010";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(record)));
});

test("matrix validation rejects reused run scope and supporting evidence", () => {
  const first = recordForScenario("operation_timeout", 1);
  const second = recordForScenario("low_storage", 1);
  second.scenarioFacts.runScope = first.scenarioFacts.runScope;
  second.activeSlotObservation.runId = first.scenarioFacts.runScope;
  sealRecord(second);
  assert.throws(() => validateEvidenceManifest([first, second]), /duplicate|reused|run scope|trace/i);
});

test("identity stability requires a real absent interval and same-fingerprint reconnect", () => {
  const stable = recordForScenario("identity_stability", 1);
  stable.identityTransition = null;
  assert.throws(() => validateEvidenceRecord(stable), /identity|disconnect|reconnect|absent/i);
});

test("UI smoke rejects arbitrary authored prose and self-attested artifacts", () => {
  const smoke = uiSmokeRecord(1, "1");
  smoke.subcases[3] = {
    ...smoke.subcases[3],
    uiState: {
      ...smoke.subcases[3].uiState,
      authoredIssueText: "Everything looked fine to me.",
      authoredRemediation: "Try whatever seems useful.",
      terminalStepProjection: "succeeded",
      partialChangePresentation: "none",
      authorityInvalidated: false,
      recoveryState: "none",
    },
  };
  assert.throws(() => validateUiSmokeRecord(smoke), /authored|storage|projection|authority|artifact/i);
});

test("authorization chronology uses parsed numeric timestamps", () => {
  const authorization = recordForScenario("device_unauthorized", 1);
  authorization.sentinel.armedAt = "unix:0";
  authorization.sentinel.operationStartedAt = "unix:1";
  authorization.sentinel.operationFinishedAt = "unix:2";
  authorization.sentinel.boundaryReadyAt = "unix:2";
  authorization.sentinel.operatorActionAt = "unix:10";
  authorization.sentinel.cleanupReadyAt = "unix:12";
  authorization.authorizationTransition.initialObservedAt = "unix:0";
  authorization.authorizationTransition.operationStartedAt = "unix:1";
  authorization.authorizationTransition.firstOperationCompletedAt = "unix:2";
  authorization.authorizationTransition.revocationCheckpointAt = "unix:3";
  authorization.authorizationTransition.originalDisconnectedAt = "unix:4";
  authorization.authorizationTransition.serialAbsentFrom = "unix:4";
  authorization.authorizationTransition.serialAbsentUntil = "unix:5";
  authorization.authorizationTransition.reconnectedAt = "unix:5";
  authorization.authorizationTransition.observedAt = "unix:9";
  authorization.authorizationTransition.terminalDetectedAt = "unix:11";
  authorization.authorizationTransition.cleanupStartedAt = "unix:12";
  authorization.authorizationTransition.cleanupCompletedAt = "unix:13";
  authorization.authorizationTransition.finalStateObservedAt = "unix:14";
  assert.doesNotThrow(() => validateEvidenceRecord(sealRecord(authorization)));
});

test("canonical timestamps reject leading zeros, mixed units, fractions, and timezone text", () => {
  for (const invalid of ["unix:01", "unix:1785790000000", "unix:1.5", "2026-08-03T21:33:21Z", 1785790000]) {
    const record = recordForScenario("operation_timeout", 1);
    record.timestamp = invalid;
    assert.throws(() => validateEvidenceRecord(sealRecord(record)), /timestamp|format|range/i);
  }
  const nested = recordForScenario("cancellation_active", 1);
  nested.activeProcess.actionAt = "unix:0002";
  assert.throws(() => validateEvidenceRecord(sealRecord(nested)), /activeProcess\.actionAt|format/i);
});

test("authorization rejects every invalid lifecycle boundary, reconnect chronology, and cross-device evidence", () => {
  const mutations = [
    ["initialObservedAt", "unix:1785790002"],
    ["firstOperationCompletedAt", "unix:1785790001"],
    ["revocationCheckpointAt", "unix:1785790001"],
    ["originalDisconnectedAt", "unix:1785790002"],
    ["serialAbsentFrom", "unix:1785790002"],
    ["serialAbsentUntil", "unix:1785790004"],
    ["reconnectedAt", "unix:1785790004"],
    ["observedAt", "unix:1785790004"],
    ["terminalDetectedAt", "unix:1785790006"],
    ["cleanupStartedAt", "unix:1785790008"],
    ["finalStateObservedAt", "unix:1785790010"],
  ];
  for (const [field, value] of mutations) {
    const record = recordForScenario("device_unauthorized", 1);
    record.authorizationTransition[field] = value;
    assert.throws(() => validateEvidenceRecord(sealRecord(record)), /authorization|chronology|terminal|cleanup/i);
  }
  for (const field of ["originalDisconnectedAt", "serialAbsentFrom", "serialAbsentUntil", "reconnectedAt"]) {
    const record = recordForScenario("device_unauthorized", 1);
    delete record.authorizationTransition[field];
    assert.throws(() => validateEvidenceRecord(sealRecord(record)), /authorizationTransition|authorization transition/i);
  }
  const earlyRevocation = recordForScenario("device_unauthorized", 1);
  earlyRevocation.sentinel.boundaryReadyAt = earlyRevocation.authorizationTransition.revocationCheckpointAt;
  assert.throws(
    () => validateEvidenceRecord(sealRecord(earlyRevocation)),
    /completed safe boundary|authorization revocation/i,
  );
  const earlyRelease = recordForScenario("device_unauthorized", 1);
  earlyRelease.sentinel.operatorActionAt = "unix:1785790005";
  assert.throws(
    () => validateEvidenceRecord(sealRecord(earlyRelease)),
    /boundary release|unauthorized observation/i,
  );
  const activeRelabel = recordForScenario("device_unauthorized", 1);
  activeRelabel.activeProcess = recordForScenario("cancellation_active", 1).activeProcess;
  assert.throws(
    () => validateEvidenceRecord(sealRecord(activeRelabel)),
    /active target-process|active process|authorization/i,
  );
  const crossDevice = recordForScenario("device_unauthorized", 1);
  crossDevice.authorizationTransition.deviceScope = "serial-sha256:" + "f".repeat(64);
  assert.throws(() => validateEvidenceRecord(sealRecord(crossDevice)), /device scope/i);
});

test("record and trace digests reject tampering and relabelled supporting evidence", () => {
  const tampered = recordForScenario("operation_timeout", 1);
  tampered.notes.push("content changed after sealing");
  assert.throws(() => validateEvidenceRecord(tampered), /canonical record content digest/i);

  const first = recordForScenario("operation_timeout", 1);
  const relabelled = recordForScenario("operation_timeout", 2);
  relabelled.trace = structuredClone(first.trace);
  sealRecord(relabelled);
  assert.throws(() => validateEvidenceManifest([first, relabelled]), /traceDigest|reused|duplicate/i);
});

test("every physical identity category is globally unique", () => {
  const fields = [
    ["runId", (first, second) => { second.runId = first.runId; second.sentinel.runId = first.runId; }],
    ["sentinelId", (first, second) => { second.sentinel.sentinelId = first.sentinel.sentinelId; }],
    ["nonce", (first, second) => { second.sentinel.nonce = first.sentinel.nonce; }],
    ["evidencePath", (first, second) => { second.evidencePath = first.evidencePath; }],
    ["tracePath", (first, second) => { second.tracePath = first.tracePath; }],
    ["slotExecutionId", (first, second) => { second.activeSlotObservation.executionId = first.activeSlotObservation.executionId; }],
  ];
  for (const [label, reuse] of fields) {
    const first = recordForScenario("operation_timeout", 1);
    const second = recordForScenario("operation_timeout", 2);
    reuse(first, second);
    sealRecord(second);
    assert.throws(() => validateEvidenceManifest([first, second]), new RegExp(`${label}|duplicate|reused`, "i"));
  }
});

test("UI smoke requires matching artifacts, backend bindings, safe controls, and recursive sanitization", () => {
  const mismatchedArtifact = uiSmokeRecord(1, "1");
  mismatchedArtifact.subcases[0].uiArtifact.digest = "sha256:" + "f".repeat(64);
  mismatchedArtifact.recordDigest = evidenceRecordDigest(mismatchedArtifact);
  assert.throws(() => validateUiSmokeRecord(mismatchedArtifact), /artifact content or digest/i);

  const wrongBackend = uiSmokeRecord(1, "1");
  wrongBackend.subcases[0].uiState.backendRunId = wrongBackend.subcases[1].backendRunId;
  wrongBackend.recordDigest = evidenceRecordDigest(wrongBackend);
  assert.throws(() => validateUiSmokeRecord(wrongBackend), /another backend run/i);

  const unsafeControl = uiSmokeRecord(1, "1");
  unsafeControl.subcases[0].uiState.availableControls = ["resume"];
  unsafeControl.subcases[0].uiArtifact.content = structuredClone(unsafeControl.subcases[0].uiState);
  unsafeControl.subcases[0].uiArtifact.digest = `sha256:${canonicalDigest(unsafeControl.subcases[0].uiState)}`;
  unsafeControl.subcases[0].operatorObservation.artifactDigest = unsafeControl.subcases[0].uiArtifact.digest;
  unsafeControl.recordDigest = evidenceRecordDigest(unsafeControl);
  assert.throws(() => validateUiSmokeRecord(unsafeControl), /forbidden.*control/i);

  for (const mutate of [
    (record) => { record.developmentBuild.identity = "/Users/private/build"; },
    (record) => { record.subcases[0].operatorObservation.statement = "pass" + "word=do-not-store"; },
    (record) => {
      record.subcases[0].uiState.availableControls = ["serial=ABCDEF123456"];
      record.subcases[0].uiArtifact.content = structuredClone(record.subcases[0].uiState);
      record.subcases[0].uiArtifact.digest = `sha256:${canonicalDigest(record.subcases[0].uiState)}`;
      record.subcases[0].operatorObservation.artifactDigest = record.subcases[0].uiArtifact.digest;
    },
  ]) {
    const record = uiSmokeRecord(1, "1");
    mutate(record);
    record.recordDigest = evidenceRecordDigest(record);
    assert.throws(() => validateUiSmokeRecord(record), /unsafe|unsanitized/i);
  }
});

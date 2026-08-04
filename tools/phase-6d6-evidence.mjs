#!/usr/bin/env node

/**
 * Dependency-free validator for the Phase 6D.6 physical evidence contract.
 *
 * The validator deliberately treats missing physical records as an incomplete
 * qualification rather than as an error.  CI can therefore verify the gates,
 * schema, sanitization, and runbook without touching ADB; only a complete
 * matrix of two passing repetitions is allowed to report `complete: true`.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const CHECKED_IN_MANIFEST = JSON.parse(readFileSync(
  fileURLToPath(new URL("../docs/testing/phase-6d6/scenario-manifest.json", import.meta.url)),
  "utf8",
));
export const SCENARIOS = CHECKED_IN_MANIFEST.scenarios;
export const REQUIRED_REPETITIONS = CHECKED_IN_MANIFEST.requiredRepetitions;
export const SCENARIO_CONTRACTS = CHECKED_IN_MANIFEST.scenarioContracts;
export const UI_SMOKE_SCENARIO = CHECKED_IN_MANIFEST.uiSmokeScenario;
export const UI_SMOKE_REQUIRED_REPETITIONS = CHECKED_IN_MANIFEST.uiSmokeRequiredRepetitions;
export const UI_SMOKE_SUBCASES = CHECKED_IN_MANIFEST.uiSmokeSubcases;
export const UI_SMOKE_CONTRACTS = CHECKED_IN_MANIFEST.uiSmokeContracts;
const ROOT_PREFIX_ALLOWLIST = "/data/data/com.emuchef.fixture/emuchef-qualification-data/,/data/user/0/com.emuchef.fixture/emuchef-qualification-user/";
const MIN_STORAGE_INITIAL_FREE_KIB = 4 * 1024 * 1024;
const RECOVERY_RESERVE_KIB = 1024 * 1024;
const STORAGE_CLEANUP_HEADROOM_KIB = 64 * 1024;
const MAX_STORAGE_FILLER_KIB = 4 * 1024 * 1024;
const MAX_STORAGE_INITIAL_FREE_KIB = RECOVERY_RESERVE_KIB + MAX_STORAGE_FILLER_KIB + STORAGE_CLEANUP_HEADROOM_KIB;
const SCENARIO_OPT_INS = {
  root_revocation: [
    ["EMUCHEF_RUN_REAL_ADB_ROOT_TESTS", "1"],
    ["EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS", "1"],
    ["EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST", ROOT_PREFIX_ALLOWLIST],
  ],
  device_unauthorized: [["EMUCHEF_PHASE_6D6_AUTHORIZATION_RESET", "1"]],
  identity_replacement: [["EMUCHEF_PHASE_6D6_IDENTITY_REPLACEMENT", "1"]],
  low_storage: [["EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE", "1"]],
  host_sleep_before_deadline: [["EMUCHEF_PHASE_6D6_HOST_SLEEP", "1"]],
  host_sleep_after_deadline: [["EMUCHEF_PHASE_6D6_HOST_SLEEP", "1"]],
};

const REQUIRED_TOP_LEVEL_FIELDS = [
  "schemaVersion",
  "scenario",
  "repetition",
  "timestamp",
  "commit",
  "runId",
  "evidencePath",
  "tracePath",
  "recordDigest",
  "traceDigest",
  "trace",
  "host",
  "platformToolsRevision",
  "device",
  "root",
  "fixtureApkSha256",
  "optIns",
  "command",
  "preparation",
  "operatorAction",
  "executionSuccess",
  "observedIssueCode",
  "stepStates",
  "partialChangesPossible",
  "authorityInvalidated",
  "activeSlotReleased",
  "activeSlotObservation",
  "activeProcess",
  "activeCancellation",
  "hostSleep",
  "identityTransition",
  "authorizationTransition",
  "scenarioContract",
  "scenarioFacts",
  "sentinel",
  "storage",
  "cleanup",
  "residualStateCheck",
  "outcome",
  "notes",
];
const ISSUE_CODES = new Set([
  "operation_timed_out",
  "device_storage_exhausted",
  "device_offline",
  "device_unauthorized",
  "device_disconnected",
  "adb_server_unavailable",
  "device_transport_lost",
  "device_identity_changed",
  "device_identity_unverified",
  "root_authority_revoked",
  "root_authority_unverified",
  "step_execution_failed",
]);
const OUTCOMES = new Set(["passed", "failed", "skipped", "blocked"]);

function fail(message) {
  throw new Error(message);
}

function assertObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
}

function assertKeys(value, expected, label) {
  assertObject(value, label);
  const expectedSet = new Set(expected);
  for (const key of Object.keys(value)) {
    if (!expectedSet.has(key)) fail(`${label} contains unknown field ${key}`);
  }
  for (const key of expected) {
    if (!(key in value)) fail(`${label} is missing field ${key}`);
  }
}

function assertString(value, label, pattern = null) {
  if (typeof value !== "string" || value.length === 0) fail(`${label} must be a non-empty string`);
  if (pattern && !pattern.test(value)) fail(`${label} has an invalid format`);
}

function equalJson(left, right) {
  return JSON.stringify(canonicalize(left)) === JSON.stringify(canonicalize(right));
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]));
  }
  return value;
}

export function canonicalDigest(value) {
  return createHash("sha256").update(JSON.stringify(canonicalize(value))).digest("hex");
}

export function evidenceRecordDigest(record) {
  const canonical = structuredClone(record);
  delete canonical.recordDigest;
  return `sha256:${canonicalDigest(canonical)}`;
}

/** Return the immutable contract for one exact scenario identity. */
export function scenarioContractFor(scenario) {
  const contract = SCENARIO_CONTRACTS[scenario];
  if (!contract) fail("scenario contract is missing for the selected scenario");
  return JSON.parse(JSON.stringify(contract));
}

function requiredScenarioOptIns(scenario) {
  return (SCENARIO_OPT_INS[scenario] ?? []).map(([name, value]) =>
    name === "EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST"
      ? [name, "committed-root-prefixes"]
      : [name, value],
  );
}

const UI_SMOKE_ISSUES = {
  cancellation: new Set([null, "step_execution_failed"]),
  transport: new Set(["device_transport_lost", "device_disconnected", "device_offline"]),
  root: new Set(["root_authority_revoked", "root_authority_unverified"]),
  storage: new Set(["device_storage_exhausted"]),
  host_sleep: new Set(["operation_timed_out", "runtime_session_lost"]),
};

const UI_SMOKE_PHYSICAL_SCENARIOS = {
  cancellation: new Set(["cancellation_active", "cancellation_boundary"]),
  transport: new Set(["usb_disconnect_active", "usb_disconnect_boundary", "device_offline"]),
  root: new Set(["root_revocation"]),
  storage: new Set(["low_storage"]),
  host_sleep: new Set(["host_sleep_before_deadline", "host_sleep_after_deadline"]),
};

export function validateUiBackendBinding(subcase, physicalRecord) {
  if (!physicalRecord || physicalRecord.runId !== subcase.backendRunId || physicalRecord.traceDigest !== subcase.backendTraceDigest) {
    fail("UI smoke subcase is not bound to its physical backend run and trace");
  }
  if (physicalRecord.outcome !== "passed") fail("UI smoke subcase must reference a passing physical backend record");
  const allowedScenarios = UI_SMOKE_PHYSICAL_SCENARIOS[subcase.name];
  if (!allowedScenarios || !allowedScenarios.has(physicalRecord.scenario)) fail("UI smoke subcase references the wrong physical scenario category");
  if (physicalRecord.observedIssueCode !== subcase.backendIssueCode) fail("UI smoke backend issue code does not match the physical backend record");
  return true;
}

function assertRecursivelySanitized(value, label) {
  if (typeof value === "string") {
    const unsafe = /(?:\/Users\/|\/home\/|[A-Za-z]:\\Users\\|\/data\/|\/sdcard\/|\/storage\/emulated\/|-----BEGIN|(?:access[_-]?token|password|secret|credential|private[_-]?key)\s*[:=]|(?:thread '[^']+' panicked|stack trace)|(?:serial|device(?:Id| identifier)?)\s*[:=]\s*(?!sha256:|<)[A-Za-z0-9._:-]{6,})/i;
    if (unsafe.test(value)) fail(`${label} contains unsafe or unsanitized text`);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertRecursivelySanitized(item, `${label}[${index}]`));
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) assertRecursivelySanitized(item, `${label}.${key}`);
  }
}

/** Validate one composite development-build UI smoke record. */
export function validateUiSmokeRecord(record) {
  assertKeys(record, ["schemaVersion", "scenario", "repetition", "timestamp", "commit", "runId", "recordDigest", "developmentBuild", "subcases", "outcome", "notes"], "UI smoke record");
  if (record.schemaVersion !== 1 || record.scenario !== UI_SMOKE_SCENARIO) fail("UI smoke scenario identity is invalid");
  if (!Number.isInteger(record.repetition) || record.repetition < 1 || record.repetition > UI_SMOKE_REQUIRED_REPETITIONS) fail("UI smoke repetition is invalid");
  unixSeconds(record.timestamp, "UI smoke timestamp");
  assertString(record.commit, "UI smoke commit", /^(?:[0-9a-f]{40}|unreported)$/);
  assertString(record.runId, "UI smoke runId", /^ui-smoke-run-sha256:[0-9a-f]{64}$/);
  assertString(record.recordDigest, "UI smoke recordDigest", /^sha256:[0-9a-f]{64}$/);
  assertKeys(record.developmentBuild, ["identity", "version", "digest"], "UI smoke developmentBuild");
  assertString(record.developmentBuild.identity, "UI smoke build identity");
  assertString(record.developmentBuild.version, "UI smoke build version");
  assertString(record.developmentBuild.digest, "UI smoke build digest", /^sha256:[0-9a-f]{64}$/);
  if (!Array.isArray(record.subcases) || record.subcases.length !== UI_SMOKE_SUBCASES.length) fail("UI smoke must contain exactly five subcases");
  const seen = new Set();
  for (const subcase of record.subcases) {
    assertKeys(subcase, ["name", "subRunId", "backendRunId", "backendTraceDigest", "backendIssueCode", "uiState", "uiArtifact", "operatorObservation"], "UI smoke subcase");
    if (!UI_SMOKE_SUBCASES.includes(subcase.name) || seen.has(subcase.name)) fail("UI smoke subcases must be the five distinct required cases");
    seen.add(subcase.name);
    if (subcase.backendIssueCode !== null && typeof subcase.backendIssueCode !== "string") fail("UI smoke backend issue code must be null or a string");
    if (!UI_SMOKE_ISSUES[subcase.name].has(subcase.backendIssueCode)) fail("UI smoke backend issue code is not allowed for the subcase");
    const contract = UI_SMOKE_CONTRACTS[subcase.name];
    if (!contract || !contract.allowedIssueCodes.some((code) => code === subcase.backendIssueCode)) fail("UI smoke backend issue code does not match the authoritative subcase contract");
    assertString(subcase.subRunId, `UI smoke ${subcase.name} subRunId`, /^ui-subrun-sha256:[0-9a-f]{64}$/);
    assertString(subcase.backendRunId, `UI smoke ${subcase.name} backendRunId`, /^physical-run-sha256:[0-9a-f]{64}$/);
    assertString(subcase.backendTraceDigest, `UI smoke ${subcase.name} backendTraceDigest`, /^sha256:[0-9a-f]{64}$/);
    assertKeys(subcase.uiState, ["backendRunId", "authoredTitle", "authoredIssueText", "authoredRemediation", "terminalStepProjection", "notAttempted", "partialChangePresentation", "authorityInvalidated", "recoveryState", "availableControls"], `UI smoke ${subcase.name} uiState`);
    if (subcase.uiState.backendRunId !== subcase.backendRunId) fail("UI state is bound to another backend run");
    for (const field of ["authoredTitle", "authoredIssueText", "authoredRemediation", "terminalStepProjection", "partialChangePresentation", "recoveryState"]) assertString(subcase.uiState[field], `UI smoke ${subcase.name} uiState.${field}`);
    if (!Number.isSafeInteger(subcase.uiState.notAttempted) || subcase.uiState.notAttempted < 0) fail("UI smoke not-attempted work must be a non-negative integer");
    if (typeof subcase.uiState.authorityInvalidated !== "boolean" || !Array.isArray(subcase.uiState.availableControls) || subcase.uiState.availableControls.some((control) => typeof control !== "string")) fail("UI smoke authority and control state is incomplete");
    if (subcase.uiState.authoredTitle !== contract.authoredTitle || subcase.uiState.authoredIssueText !== contract.authoredIssueText || subcase.uiState.authoredRemediation !== contract.authoredRemediation) fail("UI smoke authored title, issue, and remediation must match the authoritative catalog");
    if (subcase.uiState.terminalStepProjection !== contract.terminalStepProjection || subcase.uiState.partialChangePresentation !== contract.partialChangePresentation) fail("UI smoke terminal projection does not match the subcase contract");
    if (contract.notAttemptedRequired && subcase.uiState.notAttempted < 1) fail("UI smoke must project not-attempted work");
    if (subcase.uiState.authorityInvalidated !== contract.authorityInvalidated || subcase.uiState.recoveryState !== contract.recoveryState) fail("UI smoke authority recovery does not match the subcase contract");
    if (contract.forbiddenControls.some((control) => subcase.uiState.availableControls.includes(control))) fail("UI smoke exposes a forbidden resume, replay, checkpoint, or ownership-transfer control");
    assertKeys(subcase.uiArtifact, ["kind", "path", "content", "digest"], `UI smoke ${subcase.name} uiArtifact`);
    if (subcase.uiArtifact.kind !== contract.requiredArtifactKind) fail("UI smoke artifact kind does not match the contract");
    assertString(subcase.uiArtifact.path, `UI smoke ${subcase.name} artifact path`, /^docs\/testing\/phase-6d6\/evidence\/ui\/[a-z0-9_-]+\.json$/);
    assertObject(subcase.uiArtifact.content, `UI smoke ${subcase.name} artifact content`);
    assertString(subcase.uiArtifact.digest, `UI smoke ${subcase.name} artifact digest`, /^sha256:[0-9a-f]{64}$/);
    if (!equalJson(subcase.uiArtifact.content, subcase.uiState) || subcase.uiArtifact.digest !== `sha256:${canonicalDigest(subcase.uiArtifact.content)}`) fail("UI smoke artifact content or digest does not match captured UI state");
    assertKeys(subcase.operatorObservation, ["artifactDigest", "observedAt", "statement"], `UI smoke ${subcase.name} operatorObservation`);
    if (subcase.operatorObservation.artifactDigest !== subcase.uiArtifact.digest) fail("UI smoke operator observation is bound to another artifact");
    unixSeconds(subcase.operatorObservation.observedAt, `UI smoke ${subcase.name} operator observedAt`);
    assertString(subcase.operatorObservation.statement, `UI smoke ${subcase.name} operator statement`);
  }
  if (seen.size !== UI_SMOKE_SUBCASES.length) fail("UI smoke is missing a required subcase");
  if (!new Set(OUTCOMES).has(record.outcome)) fail("UI smoke outcome is invalid");
  if (record.outcome === "passed" && record.subcases.some((subcase) => subcase.uiState.notAttempted < 1)) fail("a passing UI smoke repetition cannot omit not-attempted projection");
  if (!Array.isArray(record.notes) || record.notes.some((value) => typeof value !== "string")) fail("UI smoke notes must be strings");
  assertRecursivelySanitized(record, "UI smoke record");
  if (evidenceRecordDigest(record) !== record.recordDigest) fail("canonical UI smoke record content digest does not match the evidence record");
  return true;
}

/** Validate the exact environment gate family without invoking ADB. */
export function validateGateEnvironment(environment) {
  const required = [
    ["EMUCHEF_RUN_REAL_ADB_TESTS", "1"],
    ["EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS", "1"],
    ["EMUCHEF_TEST_PACKAGE_ALLOWLIST", "com.emuchef.fixture"],
  ];
  for (const [name, expected] of required) {
    if (environment[name] !== expected) fail(`${name} must equal ${expected}`);
  }
  if (!SCENARIOS.includes(environment.EMUCHEF_PHASE_6D6_SCENARIO)) {
    fail("EMUCHEF_PHASE_6D6_SCENARIO must name exactly one supported scenario");
  }
  for (const [name, expected] of SCENARIO_OPT_INS[environment.EMUCHEF_PHASE_6D6_SCENARIO] ?? []) {
    if (environment[name] !== expected) fail(`${name} must equal ${expected}`);
  }
  if (!/^[12]$/.test(environment.EMUCHEF_PHASE_6D6_REPETITION ?? "")) {
    fail("EMUCHEF_PHASE_6D6_REPETITION must be 1 or 2");
  }
  if (typeof environment.EMUCHEF_TEST_DEVICE_SERIAL !== "string" || !/^[^\s]{1,256}$/.test(environment.EMUCHEF_TEST_DEVICE_SERIAL)) {
    fail("EMUCHEF_TEST_DEVICE_SERIAL must select one exact serial");
  }
  assertString(environment.EMUCHEF_PHASE_6D6_SENTINEL_DIR, "EMUCHEF_PHASE_6D6_SENTINEL_DIR", /^\/(?!.*\.\.).+$/);
  return true;
}

/** Validate one sanitized evidence record against the strict schema contract. */
export function validateEvidenceRecord(record) {
  assertKeys(record, REQUIRED_TOP_LEVEL_FIELDS, "evidence record");
  if (record.schemaVersion !== 1) fail("schemaVersion must be 1");
  if (!SCENARIOS.includes(record.scenario)) fail("evidence scenario is not in the mandatory matrix");
  if (!Number.isInteger(record.repetition) || ![1, 2].includes(record.repetition)) {
    fail("evidence repetition must be 1 or 2");
  }
  unixSeconds(record.timestamp, "timestamp");
  assertString(record.commit, "commit", /^(?:[0-9a-f]{40}|unreported)$/);
  assertString(record.runId, "runId", /^physical-run-sha256:[0-9a-f]{64}$/);
  assertString(record.evidencePath, "evidencePath", /^docs\/testing\/phase-6d6\/evidence\/[a-z0-9_-]+\.json$/);
  assertString(record.tracePath, "tracePath", /^docs\/testing\/phase-6d6\/evidence\/traces\/[a-z0-9_-]+\.json$/);
  assertString(record.recordDigest, "recordDigest", /^sha256:[0-9a-f]{64}$/);
  assertString(record.traceDigest, "traceDigest", /^sha256:[0-9a-f]{64}$/);
  assertObject(record.trace, "trace");
  assertKeys(record.host, ["os", "version", "architecture"], "host");
  for (const field of ["os", "version", "architecture"]) assertString(record.host[field], `host.${field}`);
  assertString(record.platformToolsRevision, "platformToolsRevision");
  assertKeys(record.device, ["identity", "model", "androidVersion", "apiLevel", "abi", "buildFingerprint"], "device");
  assertString(record.device.identity, "device.identity", /^serial-sha256:[0-9a-f]{64}$/);
  for (const field of ["model", "androidVersion", "abi", "buildFingerprint"]) {
    assertString(record.device[field], `device.${field}`);
  }
  if (!Number.isInteger(record.device.apiLevel) || record.device.apiLevel < 1) fail("device.apiLevel must be positive");
  if (record.root !== null) {
    assertKeys(record.root, ["implementationVersion"], "root");
    assertString(record.root.implementationVersion, "root.implementationVersion");
  }
  assertString(record.fixtureApkSha256, "fixtureApkSha256", /^[0-9a-f]{64}$/);
  if (!Array.isArray(record.optIns) || record.optIns.some((value) => typeof value !== "string")) fail("optIns must be an array of strings");
  for (const required of [
    "EMUCHEF_RUN_REAL_ADB_TESTS=1",
    "EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1",
    `EMUCHEF_PHASE_6D6_SCENARIO=${record.scenario}`,
    `EMUCHEF_PHASE_6D6_REPETITION=${record.repetition}`,
    "EMUCHEF_TEST_DEVICE_SERIAL=selected",
    "EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture",
    ...requiredScenarioOptIns(record.scenario).map(([name, value]) => name + "=" + value),
  ]) {
    if (!record.optIns.includes(required)) fail(`optIns is missing ${required}`);
  }
  const serialized = JSON.stringify(record);
  if (/adb\s+-s\s+(?!<selected-serial>|selected\b)[^\s]+/i.test(serialized)) fail("evidence contains a raw serial or private payload");
  if (/(?:password|credential|access[_-]?token|private[_-]?key|\/data\/data\/|\/sdcard\/|\/Users\/|\/home\/|stack trace|panicked at|-----BEGIN)/i.test(serialized)) fail("evidence contains a raw private payload");
  assertString(record.command, "command");
  if (!record.command.includes("--ignored") || !record.command.includes("--exact")) fail("command must use the ignored exact physical harness");
  assertString(record.preparation, "preparation");
  assertString(record.operatorAction, "operatorAction");
  if (typeof record.executionSuccess !== "boolean") fail("executionSuccess must be boolean");
  if (record.observedIssueCode !== null && !ISSUE_CODES.has(record.observedIssueCode)) fail("observedIssueCode is not allowlisted");
  if (!OUTCOMES.has(record.outcome)) fail("outcome is invalid");
  const passingRecord = record.outcome === "passed";
  assertKeys(record.stepStates, ["executed", "skipped", "failed", "cancelled", "blocked", "notAttempted"], "stepStates");
  for (const field of Object.keys(record.stepStates)) {
    if (!Number.isInteger(record.stepStates[field]) || record.stepStates[field] < 0) fail(`stepStates.${field} must be non-negative`);
  }
  for (const field of ["partialChangesPossible", "authorityInvalidated", "activeSlotReleased"]) {
    if (typeof record[field] !== "boolean") fail(`${field} must be boolean`);
  }
  const contract = scenarioContractFor(record.scenario);
  if (!equalJson(record.scenarioContract, contract)) fail("scenario contract does not match scenario identity");
  assertKeys(record.scenarioFacts, [
    "rootShell", "activeCheckpoint", "boundaryCheckpoint", "requiresSentinelAction",
    "storage", "runScope", "operationClass",
  ], "scenarioFacts");
  const facts = { ...record.scenarioFacts };
  delete facts.runScope;
  delete facts.operationClass;
  if (!equalJson(facts, contract.facts)) fail("scenario facts do not match scenario contract");
  assertString(record.scenarioFacts.runScope, "scenarioFacts.runScope", /^run-scope-sha256:[0-9a-f]{64}$/);
  if (record.scenarioFacts.operationClass !== "device_copy") fail("scenario operation class is not the reviewed device-copy operation");
  if (record.scenarioFacts.rootShell !== (record.root !== null)) fail("root fact does not match root evidence");
  if (passingRecord) {
    if (!contract.allowedIssueCodes.some((code) => code === record.observedIssueCode)) fail("observed issue code is not allowed by the scenario contract");
    if (!contract.allowedStepStates.some((candidate) => equalJson(candidate, record.stepStates))) fail("step-state accounting does not match the scenario contract");
    if (contract.expectedExecution === "success" && (!record.executionSuccess || record.observedIssueCode !== null)) fail("expected successful execution is not represented");
    if (contract.expectedExecution === "failure" && record.executionSuccess) fail("expected execution failure was recorded as success");
    if (contract.expectedExecution === "interruption" && record.executionSuccess) fail("expected interrupted execution was recorded as success");
    if (contract.partialChanges === "required" && !record.partialChangesPossible) fail("partial changes were required by the scenario contract");
    if (contract.partialChanges === "forbidden" && record.partialChangesPossible) fail("partial changes are forbidden by the scenario contract");
    if (contract.authorityInvalidation === "required" && !record.authorityInvalidated) fail("authority invalidation was required by the scenario contract");
    if (contract.authorityInvalidation === "forbidden" && record.authorityInvalidated) fail("authority invalidation is forbidden by the scenario contract");
  }
  assertKeys(record.activeSlotObservation, ["observed", "acquired", "released", "runId", "executionId", "acquiredAt", "terminalCleanupAt", "releasedAt", "sourceKind", "evidence"], "active slot observation");
  for (const field of ["observed", "acquired", "released"]) {
    if (typeof record.activeSlotObservation[field] !== "boolean") fail("active slot observation booleans are required");
  }
  assertString(record.activeSlotObservation.runId, "activeSlotObservation.runId", /^run-scope-sha256:[0-9a-f]{64}$/);
  assertString(record.activeSlotObservation.executionId, "activeSlotObservation.executionId", /^execution-sha256:[0-9a-f]{64}$/);
  if (record.activeSlotObservation.runId !== record.scenarioFacts.runScope) fail("active slot observation belongs to another run");
  if (record.activeSlotObservation.released !== record.activeSlotReleased) fail("active slot release observation disagrees with the terminal result");
  if (record.activeSlotObservation.acquired && !record.activeSlotObservation.observed) fail("active slot acquisition cannot be unobserved");
  if (passingRecord && (record.activeSlotObservation.observed !== true || record.activeSlotObservation.acquired !== true || record.activeSlotObservation.released !== true)) fail("active slot acquisition and release must be observed by the production lifecycle seam");
  const expectedSlotSource = contract.slotObservation?.source;
  if (!expectedSlotSource || record.activeSlotObservation.evidence !== expectedSlotSource) fail("active slot evidence is not an observable production lifecycle result");
  if (record.activeSlotObservation.sourceKind !== "production_owned_slot") fail("active slot evidence came from a harness-created auxiliary lease or shadow flag");
  if (record.activeSlotObservation.released && !record.activeSlotObservation.acquired) fail("active slot release without prior acquisition is invalid");
  const slotAcquired = unixSeconds(record.activeSlotObservation.acquiredAt, "activeSlotObservation.acquiredAt");
  const slotTerminal = record.activeSlotObservation.terminalCleanupAt === null ? null : unixSeconds(record.activeSlotObservation.terminalCleanupAt, "activeSlotObservation.terminalCleanupAt");
  const slotReleased = record.activeSlotObservation.releasedAt === null ? null : unixSeconds(record.activeSlotObservation.releasedAt, "activeSlotObservation.releasedAt");
  if (record.activeSlotObservation.released && (slotTerminal === null || slotReleased === null || slotTerminal < slotAcquired || slotReleased < slotTerminal)) fail("active slot release must follow terminal cleanup for the same execution");

  validateActiveProcess(record, contract, passingRecord);

  validateActiveCancellation(record, contract, passingRecord);
  validateHostSleep(record, contract, passingRecord);
  validateIdentityTransition(record, contract, passingRecord);
  validateAuthorizationTransition(record, contract, passingRecord);
  assertKeys(record.sentinel, [
    "sentinelId", "nonce", "runId", "scenario", "repetition",
    "armedAt", "operationStartedAt", "boundaryReadyAt", "operatorActionAt",
    "operationFinishedAt", "cleanupReadyAt", "sleepRequestedAt", "sleepEnteredAt", "wakeAt",
    "chronologyValid", "uniqueMarkers",
  ], "sentinel");
  assertString(record.sentinel.sentinelId, "sentinel.sentinelId", /^sentinel-sha256:[0-9a-f]{64}$/);
  assertString(record.sentinel.nonce, "sentinel.nonce", /^nonce-sha256:[0-9a-f]{64}$/);
  if (record.sentinel.runId !== record.runId || record.sentinel.scenario !== record.scenario || record.sentinel.repetition !== record.repetition) fail("sentinel identity is not bound to the exact run, scenario, and repetition");
  for (const field of ["armedAt", "operationStartedAt", "boundaryReadyAt", "operatorActionAt", "operationFinishedAt", "cleanupReadyAt", "sleepRequestedAt", "sleepEnteredAt", "wakeAt"]) {
    if (record.sentinel[field] !== null) assertString(record.sentinel[field], "sentinel." + field, /^unix:\d+$/);
  }
  const sentinelSeconds = Object.fromEntries(
    ["armedAt", "operationStartedAt", "boundaryReadyAt", "operatorActionAt", "operationFinishedAt", "cleanupReadyAt", "sleepRequestedAt", "sleepEnteredAt", "wakeAt"]
      .filter((field) => record.sentinel[field] !== null)
      .map((field) => [field, unixSeconds(record.sentinel[field], `sentinel.${field}`)]),
  );
  if (typeof record.sentinel.chronologyValid !== "boolean" || typeof record.sentinel.uniqueMarkers !== "boolean") fail("sentinel chronology flags must be boolean");
  if (sentinelSeconds.armedAt !== undefined && sentinelSeconds.operationStartedAt !== undefined && sentinelSeconds.operationStartedAt < sentinelSeconds.armedAt) fail("sentinel operation started before arming");
  if (sentinelSeconds.operationFinishedAt !== undefined && sentinelSeconds.operationFinishedAt < (sentinelSeconds.operationStartedAt ?? sentinelSeconds.armedAt ?? sentinelSeconds.operationFinishedAt)) fail("sentinel operation finished before start");
  if (sentinelSeconds.armedAt !== undefined && sentinelSeconds.boundaryReadyAt !== undefined && sentinelSeconds.boundaryReadyAt < sentinelSeconds.armedAt) fail("sentinel boundary marker predates arming");
  if (sentinelSeconds.armedAt !== undefined && sentinelSeconds.operatorActionAt !== undefined && sentinelSeconds.operatorActionAt < (sentinelSeconds.boundaryReadyAt ?? sentinelSeconds.operationStartedAt ?? sentinelSeconds.armedAt)) fail("sentinel operator action predates its checkpoint");
  if (sentinelSeconds.operationFinishedAt !== undefined && sentinelSeconds.cleanupReadyAt !== undefined && sentinelSeconds.cleanupReadyAt < sentinelSeconds.operationFinishedAt) fail("sentinel cleanup-ready marker predates terminal operation");
  if (passingRecord && (sentinelSeconds.armedAt === undefined || sentinelSeconds.operationFinishedAt === undefined)) fail("sentinel chronology must include armed and finished markers");
  if (passingRecord && contract.facts.requiresSentinelAction && record.hostSleep === null && sentinelSeconds.operatorActionAt === undefined) fail("scenario requires a fresh operator sentinel action");
  if (passingRecord && record.hostSleep !== null && (sentinelSeconds.sleepRequestedAt === undefined || sentinelSeconds.sleepEnteredAt === undefined || sentinelSeconds.wakeAt === undefined)) fail("host sleep scenario requires fresh sleep transition markers");
  if (passingRecord && contract.facts.boundaryCheckpoint && sentinelSeconds.boundaryReadyAt === undefined) fail("boundary scenario is missing boundary-ready chronology");
  if (passingRecord && record.scenario === "root_revocation" && sentinelSeconds.cleanupReadyAt === undefined) fail("root cleanup authority was not restored at the bounded checkpoint");
  if (passingRecord && (record.sentinel.chronologyValid !== true || record.sentinel.uniqueMarkers !== true)) fail("sentinel chronology or uniqueness was not verified");
  if (passingRecord && record.hostSleep !== null) {
    if (record.sentinel.sleepRequestedAt !== record.hostSleep.sleepRequestedAt
      || record.sentinel.sleepEnteredAt !== record.hostSleep.sleepEnteredAt
      || record.sentinel.wakeAt !== record.hostSleep.wakeAt) fail("host sleep evidence must use the fresh sentinel sleep markers");
  }
  if (contract.facts.storage) {
    assertObject(record.storage, "storage");
    assertKeys(record.storage, [
      "initialFreeKib", "recoveryReserveKib", "fillerKib", "finalFreeKib",
      "restoredRecoveryReserveKib", "reserveCreated", "reserveRemoved",
      "ownershipVerified", "boundedAllocation", "cleanupVerified",
    ], "storage");
    for (const field of ["initialFreeKib", "recoveryReserveKib", "fillerKib"]) {
      if (!Number.isInteger(record.storage[field]) || record.storage[field] < 0) fail("storage." + field + " must be a non-negative integer");
    }
    for (const field of ["finalFreeKib", "restoredRecoveryReserveKib"]) {
      if (record.storage[field] !== null && (!Number.isInteger(record.storage[field]) || record.storage[field] < 0)) fail("storage." + field + " must be a non-negative integer or null");
      if (passingRecord && record.storage[field] === null) fail("storage." + field + " must be observed for a passing attempt");
    }
    if (record.storage.initialFreeKib < MIN_STORAGE_INITIAL_FREE_KIB
      || record.storage.initialFreeKib > MAX_STORAGE_INITIAL_FREE_KIB
      || record.storage.recoveryReserveKib < RECOVERY_RESERVE_KIB
      || record.storage.fillerKib <= 0
      || record.storage.fillerKib > MAX_STORAGE_FILLER_KIB
      || record.storage.fillerKib > record.storage.initialFreeKib - record.storage.recoveryReserveKib - STORAGE_CLEANUP_HEADROOM_KIB) fail("storage safety bounds are not proven");
    for (const field of ["reserveCreated", "reserveRemoved", "ownershipVerified", "boundedAllocation", "cleanupVerified"]) {
      if (typeof record.storage[field] !== "boolean") fail("storage." + field + " must be boolean");
      if (passingRecord && record.storage[field] !== true) fail("storage." + field + " must be true for qualifying evidence");
    }
  } else if (record.storage !== null) {
    fail("non-storage scenarios must not contain storage mutation evidence");
  }
  assertKeys(record.cleanup, ["command", "outcome", "ownedPathDigests", "verified", "nonFixtureDeletion"], "cleanup");
  assertString(record.cleanup.command, "cleanup.command");
  if (!new Set(["succeeded", "failed", "not_attempted"]).has(record.cleanup.outcome)) fail("cleanup.outcome is invalid");
  if (!Array.isArray(record.cleanup.ownedPathDigests) || record.cleanup.ownedPathDigests.length < 2 || record.cleanup.ownedPathDigests.some((value) => typeof value !== "string" || !/^path-sha256:[0-9a-f]{64}$/.test(value))) fail("cleanup owned paths must be hashed fixture paths");
  if (record.cleanup.nonFixtureDeletion !== false) fail("cleanup must prove that no non-fixture path was deleted");
  if (typeof record.cleanup.verified !== "boolean") fail("cleanup.verified must be boolean");
  assertKeys(record.residualStateCheck, ["outcome", "residuals"], "residualStateCheck");
  if (!new Set(["clean", "residual", "unknown"]).has(record.residualStateCheck.outcome)) fail("residualStateCheck.outcome is invalid");
  if (!Array.isArray(record.residualStateCheck.residuals) || record.residualStateCheck.residuals.some((value) => typeof value !== "string")) fail("residualStateCheck.residuals must be strings");
  if (!Array.isArray(record.notes) || record.notes.some((value) => typeof value !== "string")) fail("notes must be strings");

  if (passingRecord && (record.cleanup.outcome !== contract.cleanupOutcome || record.residualStateCheck.outcome !== contract.residualOutcome)) fail("cleanup or residual outcome does not match the scenario contract");
  if (passingRecord && (record.cleanup.outcome !== "succeeded" || record.residualStateCheck.outcome !== "clean" || !record.cleanup.verified)) fail("a passed evidence record requires clean cleanup and residual state");
  if (`sha256:${canonicalDigest(record.trace)}` !== record.traceDigest) fail("trace content digest does not match the supporting evidence");
  if (evidenceRecordDigest(record) !== record.recordDigest) fail("canonical record content digest does not match the evidence record");
  return true;
}

function unixSeconds(value, label) {
  assertString(value, label, /^unix:(?:0|[1-9]\d{0,11})$/);
  const seconds = BigInt(value.slice(5));
  if (seconds > 253402300799n) fail(`${label} is outside the canonical timestamp range`);
  return seconds;
}

function monotonicNanos(value, label) {
  assertString(value, label, /^monotonic-ns:(?:0|[1-9]\d{0,19})$/);
  return BigInt(value.slice("monotonic-ns:".length));
}

function validateActiveProcess(record, contract, passingRecord) {
  const rule = contract.activeProcess ?? null;
  if (!rule) {
    if (record.activeProcess !== null) fail("non-active scenarios must not contain active target-process evidence");
    return;
  }
  if (record.activeProcess === null) {
    if (passingRecord) fail("active mutation qualification requires exact target process evidence");
    return;
  }
  assertKeys(record.activeProcess, [
    "runId", "operationId", "operationClass", "childIdentity", "spawnedAt",
    "mutationStartedAt", "checkedAliveAt", "actionAt", "terminalAt",
    "aliveImmediatelyBeforeAction", "terminalReportedBeforeAction",
  ], "activeProcess");
  assertString(record.activeProcess.runId, "activeProcess.runId", /^run-scope-sha256:[0-9a-f]{64}$/);
  assertString(record.activeProcess.operationId, "activeProcess.operationId", /^operation-sha256:[0-9a-f]{64}$/);
  assertString(record.activeProcess.childIdentity, "activeProcess.childIdentity", /^child-sha256:[0-9a-f]{64}$/);
  if (record.activeProcess.runId !== record.scenarioFacts.runScope || record.activeProcess.operationClass !== rule.operationClass) fail("active target process belongs to another run or operation");
  const spawned = unixSeconds(record.activeProcess.spawnedAt, "activeProcess.spawnedAt");
  const mutationStarted = unixSeconds(record.activeProcess.mutationStartedAt, "activeProcess.mutationStartedAt");
  const checkedAlive = unixSeconds(record.activeProcess.checkedAliveAt, "activeProcess.checkedAliveAt");
  const action = unixSeconds(record.activeProcess.actionAt, "activeProcess.actionAt");
  const terminal = unixSeconds(record.activeProcess.terminalAt, "activeProcess.terminalAt");
  if (!(spawned <= mutationStarted && mutationStarted <= checkedAlive && checkedAlive <= action && action < terminal)) fail("operator action must precede the exact target process terminal event");
  if (record.activeProcess.aliveImmediatelyBeforeAction !== true || record.activeProcess.terminalReportedBeforeAction !== false) fail("exact target child was not proven alive immediately before the action");
}

function validateActiveCancellation(record, contract, passingRecord) {
  const rule = contract.activeCancellation ?? null;
  if (!rule) {
    if (record.activeCancellation !== null) fail("non-cancellation scenarios must not contain active cancellation evidence");
    return;
  }
  if (record.activeCancellation === null) {
    if (passingRecord && rule.required) fail("active cancellation evidence is required");
    return;
  }
  assertKeys(record.activeCancellation, [
    "requestPhase", "inFlightObservedAt", "requestedAt", "operationFinishedAt",
    "requestBeforeFinished", "laterWorkNotAttempted", "operatorEvidence",
  ], "activeCancellation");
  if (record.activeCancellation.requestPhase !== rule.requestPhase) fail("active cancellation checkpoint phase does not match the scenario contract");
  if (typeof record.activeCancellation.requestBeforeFinished !== "boolean" || typeof record.activeCancellation.laterWorkNotAttempted !== "boolean") fail("active cancellation timing booleans are required");
  if (typeof record.activeCancellation.operatorEvidence !== "string" || record.activeCancellation.operatorEvidence.length === 0) fail("active cancellation operator evidence is required");
  const inFlight = unixSeconds(record.activeCancellation.inFlightObservedAt, "activeCancellation.inFlightObservedAt");
  const requested = unixSeconds(record.activeCancellation.requestedAt, "activeCancellation.requestedAt");
  const finished = unixSeconds(record.activeCancellation.operationFinishedAt, "activeCancellation.operationFinishedAt");
  if (rule.requestPhase === "in_flight") {
    if (requested < inFlight || finished < requested) fail("active cancellation chronology is contradictory");
  } else if (rule.requestPhase === "safe_boundary") {
    if (finished < inFlight || requested < finished) fail("safe-boundary cancellation chronology is contradictory");
  } else {
    fail("active cancellation request phase is not supported");
  }
  if (record.activeCancellation.requestBeforeFinished !== (requested < finished)) fail("active cancellation request timing flag is incorrect");
  if (passingRecord && rule.required) {
    if (record.activeCancellation.requestBeforeFinished !== true || record.activeCancellation.laterWorkNotAttempted !== true) fail("active cancellation must be requested in flight before terminal finish and leave later work unattempted");
    if (record.observedIssueCode !== null || record.executionSuccess) fail("active cancellation cannot be relabeled as transport or another failure");
    const expected = rule.terminalBehavior;
    if (expected === "active_operation_completes_then_later_work_not_attempted" && !equalJson(record.stepStates, { executed: 1, skipped: 0, failed: 0, cancelled: 1, blocked: 0, notAttempted: 0 })) fail("active cancellation step accounting is not the established terminal behavior");
  }
}

function validateHostSleep(record, contract, passingRecord) {
  const rule = contract.hostSleep ?? null;
  if (!rule) {
    if (record.hostSleep !== null) fail("non-host-sleep scenarios must not contain host-sleep timing evidence");
    return;
  }
  if (record.hostSleep === null) {
    if (passingRecord) fail("host sleep qualification requires measured timing evidence");
    return;
  }
  assertKeys(record.hostSleep, [
    "sleepRequestedAt", "sleepEnteredAt", "wakeAt", "wallElapsedMs", "executorElapsedMs",
    "deadlineMs", "operationStartedAt", "terminalAt", "terminalOutcome", "hostOs",
    "hostVersion", "timerImplementation", "toolchain", "timerClassification",
    "measurementBasis", "transportLossBlockedMeasurement", "elapsedBeforeSleepMs",
    "deadlineClockStartNs", "deadlineClockBeforeSleepNs", "deadlineClockAfterWakeNs",
    "deadlineClockTerminalNs", "suspendedWallMs", "deadlineClockAdvanceDuringSuspensionMs",
    "remainingBeforeSleepMs", "remainingAfterWakeMs", "measurementToleranceMs",
    "toleranceRationale", "operatorActionPhase", "deadlineClockSource",
  ], "hostSleep");
  const requested = unixSeconds(record.hostSleep.sleepRequestedAt, "hostSleep.sleepRequestedAt");
  const entered = unixSeconds(record.hostSleep.sleepEnteredAt, "hostSleep.sleepEnteredAt");
  const wake = unixSeconds(record.hostSleep.wakeAt, "hostSleep.wakeAt");
  const started = unixSeconds(record.hostSleep.operationStartedAt, "hostSleep.operationStartedAt");
  const terminal = unixSeconds(record.hostSleep.terminalAt, "hostSleep.terminalAt");
  if (!(started <= requested && requested <= entered && entered <= wake && wake <= terminal)) fail("host sleep timestamps are not ordered");
  for (const field of ["wallElapsedMs", "executorElapsedMs", "deadlineMs", "elapsedBeforeSleepMs"]) {
    if (!Number.isSafeInteger(record.hostSleep[field]) || record.hostSleep[field] < 0) fail(`hostSleep.${field} must be a non-negative integer`);
  }
  if (BigInt(record.hostSleep.wallElapsedMs) !== (terminal - started) * 1000n) fail("host sleep wall duration is inconsistent with timestamps");
  if (BigInt(record.hostSleep.elapsedBeforeSleepMs) !== (requested - started) * 1000n) fail("host sleep pre-sleep duration is inconsistent with timestamps");
  const wakeElapsedMs = Number((wake - started) * 1000n);
  const expectedPhaseRule = rule.phase === "before_deadline"
    ? "wake_before_deadline_threshold"
    : "wake_at_or_after_deadline_threshold";
  if (rule.phaseRule !== expectedPhaseRule) fail("host-sleep phase rule is unsupported or contradictory");
  if (rule.phase === "before_deadline" && wakeElapsedMs >= record.hostSleep.deadlineMs) fail("host sleep evidence woke after the required before-threshold phase");
  if (rule.phase === "after_deadline" && wakeElapsedMs < record.hostSleep.deadlineMs) fail("host sleep evidence woke before the required after-threshold phase");
  for (const field of ["hostOs", "hostVersion", "timerImplementation", "toolchain", "measurementBasis"]) assertString(record.hostSleep[field], `hostSleep.${field}`);
  if (record.hostSleep.deadlineClockSource !== rule.deadlineClock || rule.classificationBasis !== "clock_advancement_and_remaining_budget") fail("host-sleep deadline clock does not match the authoritative contract");
  if (!["suspended_time_included", "suspended_time_excluded", "indeterminate", "contradictory"].includes(record.hostSleep.timerClassification)) fail("host sleep timer classification is invalid");
  if (!["completed", "timed_out", "transport_loss", "runtime_loss"].includes(record.hostSleep.terminalOutcome)) fail("host sleep terminal outcome is invalid");
  if (typeof record.hostSleep.transportLossBlockedMeasurement !== "boolean") fail("host sleep transport measurement flag is required");
  const clockStart = monotonicNanos(record.hostSleep.deadlineClockStartNs, "hostSleep.deadlineClockStartNs");
  const clockBefore = monotonicNanos(record.hostSleep.deadlineClockBeforeSleepNs, "hostSleep.deadlineClockBeforeSleepNs");
  const clockAfter = monotonicNanos(record.hostSleep.deadlineClockAfterWakeNs, "hostSleep.deadlineClockAfterWakeNs");
  const clockTerminal = monotonicNanos(record.hostSleep.deadlineClockTerminalNs, "hostSleep.deadlineClockTerminalNs");
  if (!(clockStart <= clockBefore && clockBefore <= clockAfter && clockAfter <= clockTerminal)) fail("deadline-clock samples are not ordered");
  for (const field of ["suspendedWallMs", "deadlineClockAdvanceDuringSuspensionMs", "remainingBeforeSleepMs", "remainingAfterWakeMs", "measurementToleranceMs"]) {
    if (!Number.isSafeInteger(record.hostSleep[field]) || record.hostSleep[field] < 0) fail(`hostSleep.${field} must be a non-negative safe integer`);
  }
  assertString(record.hostSleep.toleranceRationale, "hostSleep.toleranceRationale");
  if (rule.measurementToleranceRequired && record.hostSleep.measurementToleranceMs === 0) fail("host-sleep measurement tolerance and rationale are required");
  if (record.hostSleep.operatorActionPhase !== rule.phase) fail("host sleep operator action phase does not match the scenario contract");
  const suspendedWallMs = Number((wake - entered) * 1000n);
  if (record.hostSleep.suspendedWallMs !== suspendedWallMs) fail("suspended wall duration is inconsistent with sleep and wake timestamps");
  const advanceMs = Number((clockAfter - clockBefore) / 1_000_000n);
  if (record.hostSleep.deadlineClockAdvanceDuringSuspensionMs !== advanceMs) fail("deadline-clock advancement does not match the recorded clock samples");
  if (record.hostSleep.remainingAfterWakeMs > record.hostSleep.remainingBeforeSleepMs) fail("deadline budget increased during host suspension");
  const consumedBudget = record.hostSleep.remainingBeforeSleepMs - record.hostSleep.remainingAfterWakeMs;
  const tolerance = record.hostSleep.measurementToleranceMs;
  const near = (left, right) => Math.abs(left - right) <= tolerance;
  const expectedBudgetConsumption = Math.min(advanceMs, record.hostSleep.remainingBeforeSleepMs);
  const budgetMatches = near(consumedBudget, expectedBudgetConsumption);
  const derivedClassification = advanceMs <= tolerance && budgetMatches
    ? "suspended_time_excluded"
    : near(advanceMs, suspendedWallMs) && budgetMatches
      ? "suspended_time_included"
      : (advanceMs <= tolerance || near(advanceMs, suspendedWallMs)) && !budgetMatches
        ? "contradictory"
        : "indeterminate";
  if (record.hostSleep.timerClassification !== derivedClassification) fail("host timer classification is inconsistent with measured deadline-clock advancement");
  const terminalClockElapsedMs = Number((clockTerminal - clockStart) / 1_000_000n);
  if (!near(record.hostSleep.executorElapsedMs, terminalClockElapsedMs)) fail("executor elapsed time is inconsistent with the deadline clock");
  if (record.hostSleep.terminalOutcome === "timed_out" && terminalClockElapsedMs < record.hostSleep.deadlineMs) fail("timeout occurred before the measured deadline clock exhausted its budget");
  if (record.hostSleep.terminalOutcome === "completed" && terminalClockElapsedMs >= record.hostSleep.deadlineMs) fail("completion contradicts the measured exhausted deadline budget");
  if (passingRecord) {
    if (record.hostSleep.timerClassification === "indeterminate" || record.hostSleep.timerClassification === "contradictory") fail("indeterminate or contradictory host timer evidence cannot pass");
    if (!rule.allowedTimerClassifications.includes(record.hostSleep.timerClassification)) fail("host timer classification is not allowed by the scenario contract");
    if (record.hostSleep.terminalOutcome === "transport_loss") {
      if (!record.hostSleep.transportLossBlockedMeasurement) fail("transport loss cannot qualify host timer behavior without an explicit measurement blocker");
      fail("transport loss leaves host timer qualification blocked");
    }
    if (!rule.allowedTerminalOutcomes.includes(record.hostSleep.terminalOutcome)) fail("host terminal outcome is not allowed by the scenario contract");
    if (record.hostSleep.terminalOutcome === "completed" && (!record.executionSuccess || record.observedIssueCode !== null)) fail("completed host-sleep evidence disagrees with the terminal execution result");
    if (record.hostSleep.terminalOutcome === "timed_out" && (record.executionSuccess || record.observedIssueCode !== "operation_timed_out")) fail("timed-out host-sleep evidence disagrees with the terminal execution result");
  }
}

function validateIdentityTransition(record, contract, passingRecord) {
  const rule = contract.identityTransition ?? null;
  if (!rule) {
    if (record.identityTransition !== null) fail("non-identity scenarios must not contain identity-transition evidence");
    return;
  }
  if (record.identityTransition === null) {
    if (passingRecord) fail("identity qualification requires measured disconnect and reconnect evidence");
    return;
  }
  assertKeys(record.identityTransition, [
    "initialSerial", "initialFingerprint", "originalAttached", "originalDisconnectedAt",
    "serialAbsentFrom", "serialAbsentUntil", "replacementAttachedAt", "replacementSerial",
    "replacementFingerprint", "neverSimultaneous", "expectedIssueCode", "authorityInvalidated",
    "cleanupFinalAttached", "runId",
  ], "identityTransition");
  for (const field of ["initialSerial", "replacementSerial"]) assertString(record.identityTransition[field], `identityTransition.${field}`, /^serial-sha256:[0-9a-f]{64}$/);
  for (const field of ["initialFingerprint", "replacementFingerprint"]) assertString(record.identityTransition[field], `identityTransition.${field}`, /^fingerprint-sha256:[0-9a-f]{64}$/);
  for (const field of ["originalDisconnectedAt", "serialAbsentFrom", "serialAbsentUntil", "replacementAttachedAt"]) unixSeconds(record.identityTransition[field], `identityTransition.${field}`);
  assertString(record.identityTransition.runId, "identityTransition.runId", /^run-scope-sha256:[0-9a-f]{64}$/);
  if (record.identityTransition.runId !== record.scenarioFacts.runScope) fail("identity evidence belongs to another run");
  if (typeof record.identityTransition.originalAttached !== "boolean" || typeof record.identityTransition.neverSimultaneous !== "boolean" || typeof record.identityTransition.authorityInvalidated !== "boolean" || typeof record.identityTransition.cleanupFinalAttached !== "boolean") fail("identity transition booleans are required");
  const disconnected = unixSeconds(record.identityTransition.originalDisconnectedAt, "identityTransition.originalDisconnectedAt");
  const absentFrom = unixSeconds(record.identityTransition.serialAbsentFrom, "identityTransition.serialAbsentFrom");
  const absentUntil = unixSeconds(record.identityTransition.serialAbsentUntil, "identityTransition.serialAbsentUntil");
  const attached = unixSeconds(record.identityTransition.replacementAttachedAt, "identityTransition.replacementAttachedAt");
  if (!record.identityTransition.originalAttached || absentFrom < disconnected || absentUntil < absentFrom || attached < absentUntil) fail("identity replacement ordering is not proven");
  if (record.identityTransition.initialSerial !== record.identityTransition.replacementSerial) fail("identity reconnect must retain the selected serial");
  if (rule.mode === "stable_reconnect") {
    if (record.identityTransition.initialFingerprint !== record.identityTransition.replacementFingerprint || record.identityTransition.authorityInvalidated) fail("identity stability must reconnect the same fingerprint without invalidating authority");
  } else if (rule.mode === "same_serial_replacement") {
    if (record.identityTransition.initialFingerprint === record.identityTransition.replacementFingerprint || !record.identityTransition.authorityInvalidated) fail("identity replacement must change the stable fingerprint and invalidate authority");
  } else {
    fail("identity transition mode is unsupported");
  }
  if (!record.identityTransition.neverSimultaneous || !record.identityTransition.cleanupFinalAttached) fail("identity reconnect must prove one-at-a-time attachment and clean final attachment");
  const operationStarted = record.sentinel.operationStartedAt === null ? null : unixSeconds(record.sentinel.operationStartedAt, "sentinel.operationStartedAt");
  const boundaryReady = record.sentinel.boundaryReadyAt === null ? null : unixSeconds(record.sentinel.boundaryReadyAt, "sentinel.boundaryReadyAt");
  if (operationStarted !== null && disconnected < operationStarted) fail("identity replacement disconnected before the operation checkpoint");
  if (boundaryReady !== null && disconnected < boundaryReady) fail("identity replacement disconnected before the safe boundary checkpoint");
  if (passingRecord && rule.mode === "same_serial_replacement" && (!rule.terminalIssueCodes.includes(record.identityTransition.expectedIssueCode) || record.observedIssueCode !== record.identityTransition.expectedIssueCode)) fail("identity replacement issue evidence does not match the contract");
  if (passingRecord && rule.mode === "stable_reconnect" && (record.identityTransition.expectedIssueCode !== null || record.observedIssueCode !== null)) fail("stable identity reconnect cannot report a replacement issue");
}

function validateAuthorizationTransition(record, contract, passingRecord) {
  const rule = contract.authorizationTransition ?? null;
  if (!rule) {
    if (record.authorizationTransition !== null) fail("non-authorization scenarios must not contain authorization-transition evidence");
    return;
  }
  if (record.authorizationTransition === null) {
    if (passingRecord) fail("authorization revocation requires measured transition evidence");
    return;
  }
  assertKeys(record.authorizationTransition, [
    "initialState", "initialObservedAt", "operationStartedAt", "revocationCheckpointAt",
    "observedState", "observedAt", "terminalDetectedAt", "cleanupStartedAt",
    "cleanupCompletedAt", "issueCode", "authorityInvalidated", "automaticResume",
    "cleanupFinalState", "finalStateObservedAt", "runId", "deviceScope",
  ], "authorizationTransition");
  if (record.authorizationTransition.initialState !== rule.initialState || record.authorizationTransition.observedState !== rule.revokedState) fail("authorization transition states do not match the contract");
  const initial = unixSeconds(record.authorizationTransition.initialObservedAt, "authorizationTransition.initialObservedAt");
  const operationStarted = unixSeconds(record.authorizationTransition.operationStartedAt, "authorizationTransition.operationStartedAt");
  const revoked = unixSeconds(record.authorizationTransition.revocationCheckpointAt, "authorizationTransition.revocationCheckpointAt");
  const observed = unixSeconds(record.authorizationTransition.observedAt, "authorizationTransition.observedAt");
  const terminal = unixSeconds(record.authorizationTransition.terminalDetectedAt, "authorizationTransition.terminalDetectedAt");
  const cleanupStarted = unixSeconds(record.authorizationTransition.cleanupStartedAt, "authorizationTransition.cleanupStartedAt");
  const cleanupCompleted = unixSeconds(record.authorizationTransition.cleanupCompletedAt, "authorizationTransition.cleanupCompletedAt");
  const finalObserved = unixSeconds(record.authorizationTransition.finalStateObservedAt, "authorizationTransition.finalStateObservedAt");
  assertString(record.authorizationTransition.runId, "authorizationTransition.runId", /^run-scope-sha256:[0-9a-f]{64}$/);
  if (record.authorizationTransition.runId !== record.scenarioFacts.runScope) fail("authorization evidence belongs to another run");
  assertString(record.authorizationTransition.deviceScope, "authorizationTransition.deviceScope", /^serial-sha256:[0-9a-f]{64}$/);
  if (record.authorizationTransition.deviceScope !== record.device.identity) fail("authorization evidence belongs to another device scope");
  if (record.authorizationTransition.issueCode !== "device_unauthorized" || record.observedIssueCode !== "device_unauthorized") fail("generic disconnect/offline evidence cannot qualify authorization revocation");
  if (!(initial < operationStarted && operationStarted < revoked && revoked < observed && observed <= terminal && terminal < cleanupStarted && cleanupStarted <= cleanupCompleted && cleanupCompleted < finalObserved)) fail("authorization transition chronology is invalid");
  if (record.sentinel.operationStartedAt === null || operationStarted !== unixSeconds(record.sentinel.operationStartedAt, "sentinel.operationStartedAt")) fail("authorization operation start is not bound to the qualifying run");
  if (record.activeProcess === null || terminal !== unixSeconds(record.activeProcess.terminalAt, "activeProcess.terminalAt")) fail("authorization terminal detection is not bound to the target process");
  if (typeof record.authorizationTransition.authorityInvalidated !== "boolean" || typeof record.authorizationTransition.automaticResume !== "boolean" || typeof record.authorizationTransition.cleanupFinalState !== "string") fail("authorization transition evidence is incomplete");
  if (passingRecord && (!record.authorizationTransition.authorityInvalidated || record.authorizationTransition.automaticResume || record.authorizationTransition.cleanupFinalState !== "authorized")) fail("authorization revocation must invalidate authority without automatic resume and finish clean");
}

/** Validate all records and report the precise missing scenario/repetition pairs. */
export function validateEvidenceManifest(records) {
  if (!Array.isArray(records)) fail("evidence manifest records must be an array");
  const passingRepetitions = new Set();
  const runScopes = new Set();
  const uniqueFields = new Map([
    ["runId", new Set()], ["evidencePath", new Set()], ["tracePath", new Set()],
    ["traceDigest", new Set()], ["sentinelId", new Set()], ["nonce", new Set()],
    ["slotExecutionId", new Set()],
  ]);
  const physicalByRun = new Map();
  const uiSmoke = [];
  for (const record of records) {
    if (record?.scenario === UI_SMOKE_SCENARIO) {
      validateUiSmokeRecord(record);
      uiSmoke.push(record);
      continue;
    }
    validateEvidenceRecord(record);
    const key = `${record.scenario}:${record.repetition}`;
    if (record.outcome === "passed") {
      if (passingRepetitions.has(key)) fail(`duplicate passing evidence repetition ${key}`);
      passingRepetitions.add(key);
    }
    if (runScopes.has(record.scenarioFacts.runScope)) fail("duplicate or reused run scope");
    runScopes.add(record.scenarioFacts.runScope);
    const identities = {
      runId: record.runId,
      evidencePath: record.evidencePath,
      tracePath: record.tracePath,
      traceDigest: record.traceDigest,
      sentinelId: record.sentinel.sentinelId,
      nonce: record.sentinel.nonce,
      slotExecutionId: record.activeSlotObservation.executionId,
    };
    for (const [name, value] of Object.entries(identities)) {
      const values = uniqueFields.get(name);
      if (values.has(value)) fail(`duplicate or reused ${name}`);
      values.add(value);
    }
    physicalByRun.set(record.runId, record);
  }
  const missing = [];
  for (const scenario of SCENARIOS) {
    for (let repetition = 1; repetition <= REQUIRED_REPETITIONS; repetition += 1) {
      const key = `${scenario}:${repetition}`;
      if (!passingRepetitions.has(key)) missing.push(key);
    }
  }
  const uiSmokeSeen = new Set();
  const uiIdentities = new Map([
    ["composite run ID", new Set()], ["sub-run ID", new Set()],
    ["backend run ID", new Set()], ["backend trace digest", new Set()],
    ["UI artifact path", new Set()], ["UI artifact digest", new Set()],
  ]);
  const missingUiSmoke = [];
  for (const record of uiSmoke) {
    if (uiSmokeSeen.has(record.repetition)) fail(`duplicate UI smoke repetition ${record.repetition}`);
    uiSmokeSeen.add(record.repetition);
    if (uiIdentities.get("composite run ID").has(record.runId)) fail("UI smoke composite run ID was reused");
    uiIdentities.get("composite run ID").add(record.runId);
    for (const subcase of record.subcases) {
      const identities = {
        "sub-run ID": subcase.subRunId,
        "backend run ID": subcase.backendRunId,
        "backend trace digest": subcase.backendTraceDigest,
        "UI artifact path": subcase.uiArtifact.path,
        "UI artifact digest": subcase.uiArtifact.digest,
      };
      for (const [name, value] of Object.entries(identities)) {
        const values = uiIdentities.get(name);
        if (values.has(value)) fail(`UI smoke ${name} was reused across subcases or repetitions`);
        values.add(value);
      }
      validateUiBackendBinding(subcase, physicalByRun.get(subcase.backendRunId));
    }
  }
  for (let repetition = 1; repetition <= UI_SMOKE_REQUIRED_REPETITIONS; repetition += 1) {
    const record = uiSmoke.find((candidate) => candidate.repetition === repetition);
    if (!record || record.outcome !== "passed") missingUiSmoke.push(`${UI_SMOKE_SCENARIO}:${repetition}`);
  }
  return { complete: missing.length === 0 && missingUiSmoke.length === 0, missing, missingUiSmoke };
}

/** Check the checked-in manual runbook without requiring ADB or an operator. */
export function validateRunbookCommands(text) {
  assertString(text, "runbook");
  for (const marker of [
    "EMUCHEF_RUN_REAL_ADB_TESTS=1",
    "EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1",
    "EMUCHEF_PHASE_6D6_SCENARIO=",
    "EMUCHEF_PHASE_6D6_REPETITION=",
    "EMUCHEF_TEST_DEVICE_SERIAL=",
    "EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture",
    "EMUCHEF_PHASE_6D6_SENTINEL_DIR=",
    "cleanup-ready",
    "sleep-requested",
    "sleep-entered",
    "wake",
    "production runner lifecycle",
    "serial absence",
    "UI smoke is mandatory",
    "--ignored",
    "--exact",
    "600",
  ]) {
    if (!text.includes(marker)) fail(`runbook is missing ${marker}`);
  }
  if (/adb\s+(?:start-server|kill-server|reconnect)/i.test(text)) fail("runbook contains an automatic reconnect, retry, or resume path");
  if (!/one exact serial|exact selected serial|EMUCHEF_TEST_DEVICE_SERIAL=/i.test(text)) fail("runbook must require one exact serial");
  return true;
}

function repositoryRoot() {
  return fileURLToPath(new URL("../", import.meta.url));
}

function commandOption(args, name, fallback) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
}

function validateSchemaContract(schema) {
  assertObject(schema, "evidence schema");
  if (!equalJson(schema.required, REQUIRED_TOP_LEVEL_FIELDS)) fail("evidence schema top-level fields drifted from the validator");
  const hostRequired = schema.properties?.hostSleep?.required;
  for (const field of ["deadlineClockStartNs", "deadlineClockBeforeSleepNs", "deadlineClockAfterWakeNs", "deadlineClockTerminalNs", "measurementToleranceMs", "operatorActionPhase", "deadlineClockSource"]) {
    if (!hostRequired?.includes(field)) fail(`evidence schema host-sleep contract is missing ${field}`);
  }
  const authorizationRequired = schema.properties?.authorizationTransition?.required;
  for (const field of ["initialObservedAt", "operationStartedAt", "terminalDetectedAt", "cleanupStartedAt", "cleanupCompletedAt", "finalStateObservedAt", "deviceScope"]) {
    if (!authorizationRequired?.includes(field)) fail(`evidence schema authorization contract is missing ${field}`);
  }
  const uiRequired = schema.$defs?.uiSmokeRecord?.properties?.subcases?.items?.required;
  for (const field of ["subRunId", "backendRunId", "backendTraceDigest", "uiState", "uiArtifact", "operatorObservation"]) {
    if (!uiRequired?.includes(field)) fail(`evidence schema UI-smoke contract is missing ${field}`);
  }
  const canonicalTimestampPattern = "^unix:(0|[1-9][0-9]{0,11})$";
  const inspectPatterns = (value) => {
    if (Array.isArray(value)) return value.forEach(inspectPatterns);
    if (value === null || typeof value !== "object") return;
    if (typeof value.pattern === "string" && value.pattern.startsWith("^unix:") && value.pattern !== canonicalTimestampPattern) fail("evidence schema contains a noncanonical timestamp pattern");
    Object.values(value).forEach(inspectPatterns);
  };
  inspectPatterns(schema);
}

/** Run the host-only validator used by CI and local qualification preparation. */
export function validateRepositoryContract(args = process.argv.slice(2)) {
  const root = repositoryRoot();
  const manifestPath = commandOption(args, "--manifest", path.join(root, "docs/testing/phase-6d6/scenario-manifest.json"));
  const schemaPath = commandOption(args, "--schema", path.join(root, "docs/testing/phase-6d6/evidence-schema.json"));
  const templatePath = commandOption(args, "--template", path.join(root, "docs/testing/phase-6d6/evidence-template.json"));
  const runbookPath = commandOption(args, "--runbook", path.join(root, "docs/manual/phase-6d6-physical-interruption-qualification.md"));
  const evidenceDirectory = commandOption(args, "--evidence-dir", path.join(root, "docs/testing/phase-6d6/evidence"));
  const projectionSourcePath = path.join(root, "apps/emuchef-app/src-tauri/src/execution.rs");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  assertKeys(manifest, ["schemaVersion", "scenarios", "requiredRepetitions", "uiSmokeScenario", "uiSmokeRequiredRepetitions", "uiSmokeSubcases", "uiSmokeContracts", "scenarioContracts", "gates", "outcomes"], "scenario manifest");
  if (manifest.schemaVersion !== 1 || JSON.stringify(manifest.scenarios) !== JSON.stringify(SCENARIOS) || manifest.requiredRepetitions !== REQUIRED_REPETITIONS || manifest.uiSmokeScenario !== UI_SMOKE_SCENARIO || manifest.uiSmokeRequiredRepetitions !== UI_SMOKE_REQUIRED_REPETITIONS || !equalJson(manifest.uiSmokeSubcases, UI_SMOKE_SUBCASES) || !equalJson(manifest.uiSmokeContracts, UI_SMOKE_CONTRACTS) || !equalJson(manifest.scenarioContracts, SCENARIO_CONTRACTS)) fail("scenario manifest does not match the mandatory matrix or scenario contracts");
  const projectionSource = readFileSync(projectionSourcePath, "utf8");
  for (const [name, contract] of Object.entries(UI_SMOKE_CONTRACTS)) {
    const authoredFields = name === "cancellation"
      ? [contract.authoredIssueText]
      : [contract.authoredTitle, contract.authoredIssueText, contract.authoredRemediation];
    if (authoredFields.some((text) => !projectionSource.includes(text))) fail(`UI smoke ${name} authored content drifted from the production projection catalog`);
  }
  validateSchemaContract(JSON.parse(readFileSync(schemaPath, "utf8")));
  validateEvidenceRecord(JSON.parse(readFileSync(templatePath, "utf8")));
  validateRunbookCommands(readFileSync(runbookPath, "utf8"));
  const records = [];
  if (existsSync(evidenceDirectory)) {
    for (const file of readdirSync(evidenceDirectory).filter((name) => name.endsWith(".json")).sort()) {
      records.push(JSON.parse(readFileSync(path.join(evidenceDirectory, file), "utf8")));
    }
  }
  return { ...validateEvidenceManifest(records), recordCount: records.length };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const result = validateRepositoryContract();
    if (result.complete) {
      process.stdout.write(`Phase 6D.6 evidence contract complete (${result.recordCount} records).\n`);
    } else {
      process.stdout.write(`Phase 6D.6 evidence contract valid but incomplete (${result.missing.length} physical repetitions and ${result.missingUiSmoke.length} UI-smoke repetitions missing).\n`);
    }
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}

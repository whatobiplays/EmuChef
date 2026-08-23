#!/usr/bin/env node

/**
 * Dependency-free device qualification evidence foundation.
 *
 * This module owns the repository contracts for physical-device
 * qualification: canonical workflow definitions, device-target identity,
 * immutable evidence records, compatibility projection, derived workflow
 * state, derived support tiers, and the generated qualification matrix.
 * It is pure tooling and never replaces production planner or executor
 * authority. No function in this module performs a physical-device run.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REPO_ROOT = fileURLToPath(new URL("../", import.meta.url));
const QUALIFICATION_ROOT = path.join(REPO_ROOT, "docs/testing/device-qualification");
const EVIDENCE_ROOT = path.join(QUALIFICATION_ROOT, "evidence");

const WORKFLOW_FIELDS = [
  "id",
  "version",
  "purpose",
  "productionRecipes",
  "requiredCapabilities",
  "prerequisites",
  "compatibilityDimensions",
  "automatedObservations",
  "humanCheckpoints",
];
const AUTOMATED_OBSERVATION_FIELDS = ["id", "required"];
const HUMAN_CHECKPOINT_FIELDS = [
  "id",
  "instruction",
  "fact",
  "allowedOutcomes",
  "required",
];
const TARGET_FIELDS = [
  "id",
  "profileId",
  "manufacturer",
  "model",
  "androidVersion",
  "androidApi",
  "abiSocClass",
  "rootState",
  "connectionType",
  "firmwareBuild",
  "capabilities",
  "deferredWorkflows",
];
const FACT_FIELDS = ["value", "source"];
const FACT_SOURCES = new Set([
  "production_observation",
  "explicit_root_check",
  "operator_attestation",
]);
const TARGET_FACT_FIELDS = [
  "profileId",
  "manufacturer",
  "model",
  "androidVersion",
  "androidApi",
  "abiSocClass",
  "rootState",
  "connectionType",
  "firmwareBuild",
];
const FACT_SOURCE_BY_FIELD = new Map([
  ["profileId", new Set(["production_observation"])],
  ["manufacturer", new Set(["production_observation"])],
  ["model", new Set(["production_observation"])],
  ["androidVersion", new Set(["production_observation"])],
  ["androidApi", new Set(["production_observation"])],
  ["abiSocClass", new Set(["production_observation"])],
  ["rootState", new Set(["explicit_root_check"])],
  ["connectionType", new Set(["operator_attestation", "production_observation"])],
  ["firmwareBuild", new Set(["production_observation"])],
]);
const COMPATIBILITY_DIMENSIONS = new Set([
  "emuchef_build",
  "workflow_version",
  "authored_content",
  "runtime_contract",
  "device_profile",
  "android_api",
  "firmware_build",
  "abi_soc_class",
  "root_state",
]);
const ROOT_STATES = new Set(["non_root", "rooted"]);
const CONNECTION_TYPES = new Set(["usb2", "usb3"]);
const CHECKPOINT_OUTCOME_SET = new Set(["pass", "fail", "unable_to_verify"]);
const BUILD_IDENTITY_FIELDS = [
  "appVersion",
  "gitCommit",
  "materialBuildDigest",
  "realExecutionEnabled",
  "qualificationContract",
];
const MATERIAL_ROOTS = [
  "authored/",
  "crates/emuchef-rust-backend/",
  "apps/emuchef-app/src/",
  "apps/emuchef-app/src-tauri/src/",
];
const MATERIAL_EXACT_FILES = new Set([
  "apps/emuchef-app/package.json",
  "apps/emuchef-app/package-lock.json",
  "apps/emuchef-app/src-tauri/Cargo.toml",
  "apps/emuchef-app/src-tauri/Cargo.lock",
  "apps/emuchef-app/src-tauri/tauri.conf.json",
]);
const MATERIAL_EXCLUDED = [
  "apps/emuchef-app/src/DeviceQualificationOverlay.tsx",
  "apps/emuchef-app/src/useDeviceQualificationMode.ts",
  "apps/emuchef-app/src-tauri/src/qualification_build.rs",
  "apps/emuchef-app/src-tauri/src/qualification_repository.rs",
  "apps/emuchef-app/src-tauri/src/qualification_mode.rs",
  "apps/emuchef-app/src/Phase6d6UiSmoke.tsx",
  "apps/emuchef-app/src-tauri/src/phase6d6_ui_smoke.rs",
];

export const QUALIFICATION_CONTRACT_VERSION = 1;
export const RUNTIME_CONTRACT = "real-execution-v1";

export const EVIDENCE_RECORD_FIELDS = [
  "schemaVersion",
  "runId",
  "recordDigest",
  "capturedAt",
  "workflowId",
  "workflowVersion",
  "deviceTarget",
  "fingerprint",
  "fingerprintDigest",
  "runValidity",
  "qualificationOutcome",
  "automatedObservations",
  "humanCheckpoints",
  "targetWideFailure",
  "limitations",
  "artifacts",
];
export const FINGERPRINT_FIELDS = [
  "schemaVersion",
  "emuchefBuild",
  "workflowVersion",
  "authoredContent",
  "runtimeContract",
  "deviceProfile",
  "androidApi",
  "firmwareBuild",
  "abiSocClass",
  "rootState",
  "connectionType",
];
export const RUN_VALIDITIES = ["valid", "invalid"];
export const QUALIFICATION_OUTCOMES = ["passed", "failed", "not_observed"];
export const CHECKPOINT_OUTCOMES = ["pass", "fail", "unable_to_verify"];
export const TARGET_WIDE_FAILURES = [
  null,
  "device_identity_unverified",
  "device_identity_changed",
  "required_device_prerequisite_unavailable",
  "safety_invariant_failed",
];
const AUTHORED_CONTENT_ENTRY_FIELDS = ["id", "sha256"];
const EVIDENCE_ARTIFACT_FIELDS = ["id", "kind", "path", "sha256"];
const EVIDENCE_DEVICE_TARGET_FIELDS = [
  "id",
  ...TARGET_FACT_FIELDS,
];
const AUTOMATED_OBSERVATION_RECORD_FIELDS = ["id", "outcome", "observedAt"];
const HUMAN_CHECKPOINT_RECORD_FIELDS = ["checkpointId", "outcome", "observedAt"];
const TARGET_REGISTRATION_CANDIDATE_FIELDS = [
  "candidateSchemaVersion",
  "candidateId",
  "kind",
  "capturedAt",
  "build",
  "target",
];
const QUALIFICATION_RUN_CANDIDATE_FIELDS = [
  "candidateSchemaVersion",
  "candidateId",
  "kind",
  "capturedAt",
  "build",
  "workflowId",
  "workflowVersion",
  "deviceTargetId",
  "fingerprint",
  "runValidity",
  "qualificationOutcome",
  "automatedObservations",
  "humanCheckpoints",
  "targetWideFailure",
  "limitations",
  "artifacts",
];
const DIMENSION_FIELDS = {
  emuchef_build: "emuchefBuild",
  workflow_version: "workflowVersion",
  authored_content: "authoredContent",
  runtime_contract: "runtimeContract",
  device_profile: "deviceProfile",
  android_api: "androidApi",
  firmware_build: "firmwareBuild",
  abi_soc_class: "abiSocClass",
  root_state: "rootState",
};
const RUN_ID_PATTERN = /^qualification-run-sha256:[0-9a-f]{64}$/;
const CANDIDATE_ID_PATTERN = /^qualification-candidate-[0-9a-f]{32}$/;
const DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const TARGET_ID_PATTERN = /^device-target-sha256:[0-9a-f]{64}$/;
const RFC3339_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const GIT_COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const EXECUTION_REPORT_ARTIFACT = {
  id: "execution-report",
  kind: "production_execution_report",
  path: "execution-report.json",
};
const PRE_EXECUTION_PREREQUISITE_CHECKPOINTS = new Set([
  "clean_or_deliberately_reset_device",
]);
const defaultFsOps = {
  mkdirSync,
  renameSync,
  rmSync,
  writeFileSync,
};

function fail(message) {
  throw new Error(message);
}

function assertObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
}

function assertExactKeys(value, expected, label) {
  assertObject(value, label);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${label} fields must be exactly ${wanted.join(", ")}`);
  }
}

function assertString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} must be a non-empty string`);
  }
}

function assertBoolean(value, label) {
  if (typeof value !== "boolean") {
    fail(`${label} must be a boolean`);
  }
}

function assertStringArray(value, label) {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string" || item.length === 0)) {
    fail(`${label} must be an array of non-empty strings`);
  }
  if (new Set(value).size !== value.length) {
    fail(`${label} must not contain duplicates`);
  }
}

function assertPositiveInteger(value, label) {
  if (!Number.isInteger(value) || value < 1) {
    fail(`${label} must be a positive integer`);
  }
}

function assertFactWrapper(target, field, validator) {
  const fact = target[field];
  assertExactKeys(fact, FACT_FIELDS, `device target ${field}`);
  if (!FACT_SOURCES.has(fact.source)) {
    fail(`device target ${field} source ${fact.source} is not supported`);
  }
  const legalSources = FACT_SOURCE_BY_FIELD.get(field);
  if (!legalSources?.has(fact.source)) {
    fail(`device target ${field} source must be one of ${[...legalSources].join(", ")}`);
  }
  validator(fact.value, `device target ${field} value`);
  return fact;
}

export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]),
    );
  }
  return value;
}

export function canonicalDigest(value) {
  return `sha256:${createHash("sha256")
    .update(JSON.stringify(canonicalize(value)))
    .digest("hex")}`;
}

function isMaterialBuildPath(relativePath) {
  if (MATERIAL_EXCLUDED.includes(relativePath)) return false;
  return MATERIAL_EXACT_FILES.has(relativePath)
    || MATERIAL_ROOTS.some((prefix) => relativePath.startsWith(prefix));
}

export function materialBuildDigestFromEntries(entries) {
  const material = entries
    .filter((entry) => isMaterialBuildPath(entry.path))
    .sort((left, right) => left.path.localeCompare(right.path));
  return canonicalDigest(material);
}

function trackedFiles(repoRoot) {
  const output = execFileSync("git", ["ls-files", "-z"], { cwd: repoRoot, encoding: "utf8" });
  return output.split("\0").filter(Boolean);
}

function trackedWorktreeStatus(repoRoot) {
  return execFileSync(
    "git",
    ["status", "--porcelain", "--untracked-files=no"],
    { cwd: repoRoot, encoding: "utf8" },
  );
}

function materialBuildDigest(repoRoot) {
  const entries = trackedFiles(repoRoot)
    .filter((relativePath) => existsSync(path.join(repoRoot, relativePath)))
    .map((relativePath) => ({
      path: relativePath,
      sha256: createHash("sha256")
        .update(readFileSync(path.join(repoRoot, relativePath)))
        .digest("hex"),
    }));
  return materialBuildDigestFromEntries(entries);
}

function validateBuildIdentity(buildIdentity, label) {
  assertExactKeys(buildIdentity, BUILD_IDENTITY_FIELDS, label);
  assertString(buildIdentity.appVersion, `${label} appVersion`);
  assertString(buildIdentity.gitCommit, `${label} gitCommit`);
  if (!GIT_COMMIT_PATTERN.test(buildIdentity.gitCommit)) {
    fail(`${label} gitCommit must be a 40-character lowercase hex commit`);
  }
  assertString(buildIdentity.materialBuildDigest, `${label} materialBuildDigest`);
  if (!DIGEST_PATTERN.test(buildIdentity.materialBuildDigest)) {
    fail(`${label} materialBuildDigest must be a sha256 digest`);
  }
  assertBoolean(buildIdentity.realExecutionEnabled, `${label} realExecutionEnabled`);
  assertPositiveInteger(buildIdentity.qualificationContract, `${label} qualificationContract`);
  return buildIdentity;
}

export function compareBuildIdentity(current, evidence) {
  validateBuildIdentity(current, "current build identity");
  validateBuildIdentity(evidence, "evidence build identity");
  return current.appVersion === evidence.appVersion
    && current.materialBuildDigest === evidence.materialBuildDigest
    && current.realExecutionEnabled === evidence.realExecutionEnabled
    && current.qualificationContract === evidence.qualificationContract
    ? "compatible"
    : "invalidating";
}

export function buildMaterialIdentity({ repoRoot, requireClean = false }) {
  if (requireClean && trackedWorktreeStatus(repoRoot) !== "") {
    fail("device qualification requires a clean tracked worktree");
  }
  const packageJson = JSON.parse(readFileSync(path.join(repoRoot, "apps/emuchef-app/package.json"), "utf8"));
  return {
    appVersion: packageJson.version,
    gitCommit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim(),
    materialBuildDigest: materialBuildDigest(repoRoot),
    realExecutionEnabled: true,
    qualificationContract: QUALIFICATION_CONTRACT_VERSION,
  };
}

export function targetFactValue(target, field) {
  return target[field].value;
}

export function deviceTargetIdentityPayload(target) {
  return Object.fromEntries(
    TARGET_FACT_FIELDS.map((field) => [field, targetFactValue(target, field)]),
  );
}

export function deviceTargetId(target) {
  return `device-target-sha256:${canonicalDigest(deviceTargetIdentityPayload(target)).slice("sha256:".length)}`;
}

function equalJson(left, right) {
  return JSON.stringify(canonicalize(left)) === JSON.stringify(canonicalize(right));
}

export function validateWorkflowCatalog(value) {
  assertExactKeys(value, ["schemaVersion", "workflows"], "workflow catalog");
  if (value.schemaVersion !== 1) fail("workflow catalog schemaVersion must be 1");
  if (!Array.isArray(value.workflows) || value.workflows.length === 0) {
    fail("workflow catalog must contain at least one workflow");
  }
  const seenIds = new Set();
  for (const workflow of value.workflows) {
    assertExactKeys(workflow, WORKFLOW_FIELDS, "workflow");
    assertString(workflow.id, "workflow id");
    if (seenIds.has(workflow.id)) fail(`duplicate workflow id ${workflow.id}`);
    seenIds.add(workflow.id);
    assertPositiveInteger(workflow.version, "workflow version");
    assertString(workflow.purpose, "workflow purpose");
    assertStringArray(workflow.productionRecipes, "workflow productionRecipes");
    assertStringArray(workflow.requiredCapabilities, "workflow requiredCapabilities");
    assertStringArray(workflow.prerequisites, "workflow prerequisites");
    assertStringArray(workflow.compatibilityDimensions, "workflow compatibilityDimensions");
    if (workflow.compatibilityDimensions.length === 0) {
      fail("workflow compatibilityDimensions must not be empty");
    }
    for (const dimension of workflow.compatibilityDimensions) {
      if (!COMPATIBILITY_DIMENSIONS.has(dimension)) {
        fail(`workflow compatibility dimension ${dimension} is not supported`);
      }
    }
    if (!Array.isArray(workflow.automatedObservations) || workflow.automatedObservations.length === 0) {
      fail("workflow automatedObservations must contain at least one entry");
    }
    const observationIds = new Set();
    for (const observation of workflow.automatedObservations) {
      assertExactKeys(observation, AUTOMATED_OBSERVATION_FIELDS, "automated observation");
      assertString(observation.id, "automated observation id");
      if (observationIds.has(observation.id)) fail(`duplicate automated observation id ${observation.id}`);
      observationIds.add(observation.id);
      if (typeof observation.required !== "boolean") {
        fail("automated observation required must be a boolean");
      }
    }
    if (!Array.isArray(workflow.humanCheckpoints)) {
      fail("workflow humanCheckpoints must be an array");
    }
    const checkpointIds = new Set();
    for (const checkpoint of workflow.humanCheckpoints) {
      assertExactKeys(checkpoint, HUMAN_CHECKPOINT_FIELDS, "human checkpoint");
      assertString(checkpoint.id, "human checkpoint id");
      if (checkpointIds.has(checkpoint.id)) fail(`duplicate human checkpoint id ${checkpoint.id}`);
      checkpointIds.add(checkpoint.id);
      assertString(checkpoint.instruction, "human checkpoint instruction");
      assertString(checkpoint.fact, "human checkpoint fact");
      if (!Array.isArray(checkpoint.allowedOutcomes) || checkpoint.allowedOutcomes.length === 0) {
        fail("human checkpoint allowedOutcomes must contain at least one outcome");
      }
      for (const outcome of checkpoint.allowedOutcomes) {
        if (!CHECKPOINT_OUTCOME_SET.has(outcome)) fail(`human checkpoint outcome ${outcome} is not supported`);
      }
      if (new Set(checkpoint.allowedOutcomes).size !== checkpoint.allowedOutcomes.length) {
        fail("human checkpoint allowedOutcomes must not contain duplicates");
      }
      if (typeof checkpoint.required !== "boolean") {
        fail("human checkpoint required must be a boolean");
      }
    }
  }
  return value;
}

export function validateDeviceTargets(value, { authoredProfilesDir }) {
  assertExactKeys(value, ["schemaVersion", "targets"], "device targets");
  if (value.schemaVersion !== 2) fail("device targets schemaVersion must be 2");
  if (!Array.isArray(value.targets)) fail("device targets must be an array");
  const seenIds = new Set();
  for (const target of value.targets) {
    assertExactKeys(target, TARGET_FIELDS, "device target");
    assertString(target.id, "device target id");
    if (!TARGET_ID_PATTERN.test(target.id)) {
      fail("device target id format is invalid");
    }
    if (seenIds.has(target.id)) fail(`duplicate device target id ${target.id}`);
    seenIds.add(target.id);
    assertFactWrapper(target, "profileId", assertString);
    for (const field of ["manufacturer", "model", "androidVersion", "abiSocClass", "firmwareBuild"]) {
      assertFactWrapper(target, field, assertString);
    }
    assertFactWrapper(target, "androidApi", assertPositiveInteger);
    const rootState = assertFactWrapper(target, "rootState", assertString);
    if (!ROOT_STATES.has(rootState.value)) {
      fail(`device target rootState value ${rootState.value} is not supported`);
    }
    const connectionType = assertFactWrapper(target, "connectionType", assertString);
    if (!CONNECTION_TYPES.has(connectionType.value)) {
      fail(`device target connectionType value ${connectionType.value} is not supported`);
    }
    if (target.id !== deviceTargetId(target)) {
      fail("device target id does not match canonical target identity");
    }
    assertStringArray(target.capabilities, "device target capabilities");
    assertStringArray(target.deferredWorkflows, "device target deferredWorkflows");
    const profilePath = path.join(authoredProfilesDir, `${targetFactValue(target, "profileId")}.yaml`);
    if (!existsSync(profilePath)) {
      fail(`unknown authored device profile ${targetFactValue(target, "profileId")}`);
    }
  }
  return value;
}

export function loadWorkflowCatalog(catalogPath) {
  return validateWorkflowCatalog(JSON.parse(readFileSync(catalogPath, "utf8")));
}

export function loadDeviceTargets(targetsPath, options) {
  return validateDeviceTargets(JSON.parse(readFileSync(targetsPath, "utf8")), options);
}

function assertRfc3339(value, label) {
  assertString(value, label);
  if (!RFC3339_PATTERN.test(value) || Number.isNaN(Date.parse(value))) {
    fail(`${label} must be an RFC3339 timestamp`);
  }
}

export function evidenceRecordDigest(record) {
  const canonical = structuredClone(record);
  delete canonical.recordDigest;
  return canonicalDigest(canonical);
}

export function qualificationRunId(unsealedRecord) {
  const identity = structuredClone(unsealedRecord);
  delete identity.runId;
  delete identity.recordDigest;
  delete identity.fingerprintDigest;
  return `qualification-run-sha256:${canonicalDigest(identity).slice("sha256:".length)}`;
}

export function sealEvidenceRecord(unsealedRecord) {
  const record = structuredClone(unsealedRecord);
  record.fingerprintDigest = evidenceFingerprintDigest(record.fingerprint);
  record.runId = qualificationRunId(record);
  record.recordDigest = evidenceRecordDigest(record);
  return record;
}

function validateFingerprint(fingerprint) {
  assertExactKeys(fingerprint, FINGERPRINT_FIELDS, "fingerprint");
  if (fingerprint.schemaVersion !== 2) fail("fingerprint schemaVersion must be 2");
  validateBuildIdentity(fingerprint.emuchefBuild, "fingerprint emuchefBuild");
  for (const field of ["runtimeContract", "deviceProfile", "firmwareBuild", "abiSocClass"]) {
    assertString(fingerprint[field], `fingerprint ${field}`);
  }
  if (fingerprint.runtimeContract !== RUNTIME_CONTRACT) {
    fail(`fingerprint runtimeContract must be ${RUNTIME_CONTRACT}`);
  }
  assertPositiveInteger(fingerprint.workflowVersion, "fingerprint workflowVersion");
  assertPositiveInteger(fingerprint.androidApi, "fingerprint androidApi");
  if (!ROOT_STATES.has(fingerprint.rootState)) fail(`fingerprint rootState ${fingerprint.rootState} is not supported`);
  if (!CONNECTION_TYPES.has(fingerprint.connectionType)) {
    fail(`fingerprint connectionType ${fingerprint.connectionType} is not supported`);
  }
  if (!Array.isArray(fingerprint.authoredContent)) {
    fail("fingerprint authoredContent must be an array");
  }
  const seenIds = new Set();
  for (const entry of fingerprint.authoredContent) {
    assertExactKeys(entry, AUTHORED_CONTENT_ENTRY_FIELDS, "authored content entry");
    assertString(entry.id, "authored content entry id");
    if (seenIds.has(entry.id)) fail(`duplicate authored content id ${entry.id}`);
    seenIds.add(entry.id);
    if (typeof entry.sha256 !== "string" || !SHA256_PATTERN.test(entry.sha256)) {
      fail("authored content entry sha256 must be a 64-character lowercase hex digest");
    }
  }
  return true;
}

export function evidenceFingerprintDigest(fingerprint) {
  validateFingerprint(fingerprint);
  return canonicalDigest(fingerprint);
}

function compareDimension(dimension, currentFingerprint, evidenceFingerprint) {
  const field = DIMENSION_FIELDS[dimension];
  if (!field) return "not_applicable";
  if (dimension === "emuchef_build") {
    return compareBuildIdentity(currentFingerprint.emuchefBuild, evidenceFingerprint.emuchefBuild);
  }
  if (dimension === "authored_content") {
    return equalJson(currentFingerprint.authoredContent, evidenceFingerprint.authoredContent)
      ? "compatible"
      : "invalidating";
  }
  return currentFingerprint[field] === evidenceFingerprint[field]
    ? "compatible"
    : "invalidating";
}

function buildEvidenceDeviceTarget(target) {
  return Object.fromEntries(
    EVIDENCE_DEVICE_TARGET_FIELDS.map((field) => [field, structuredClone(target[field])]),
  );
}

export function classifyCompatibility({ workflow, currentFingerprint, evidenceFingerprint }) {
  for (const dimension of workflow.compatibilityDimensions) {
    if (compareDimension(dimension, currentFingerprint, evidenceFingerprint) === "invalidating") {
      return "invalidating";
    }
  }
  return "compatible";
}

function assertEvidenceDeviceTarget(deviceTarget, target) {
  const observed = {};
  const registered = {};
  for (const field of EVIDENCE_DEVICE_TARGET_FIELDS) {
    observed[field] = deviceTarget[field];
    registered[field] = target[field];
  }
  if (!equalJson(observed, registered)) {
    fail("device target facts do not match the registered target");
  }
}

function validateAutomatedObservationRecords(record, workflow) {
  const declared = new Set(workflow.automatedObservations.map((observation) => observation.id));
  if (!Array.isArray(record.automatedObservations)) {
    fail("automatedObservations must be an array");
  }
  const seen = new Set();
  for (const observation of record.automatedObservations) {
    assertExactKeys(observation, AUTOMATED_OBSERVATION_RECORD_FIELDS, "automated observation record");
    assertString(observation.id, "automated observation record id");
    if (!declared.has(observation.id)) {
      fail(`automated observation id ${observation.id} is not declared by the workflow`);
    }
    if (seen.has(observation.id)) fail(`duplicate automated observation ${observation.id}`);
    seen.add(observation.id);
    if (!["passed", "failed"].includes(observation.outcome)) {
      fail(`automated observation ${observation.id} outcome ${observation.outcome} is not supported`);
    }
    assertRfc3339(observation.observedAt, `automated observation ${observation.id} observedAt`);
  }
  return seen;
}

function validateEvidenceArtifacts(record) {
  if (!Array.isArray(record.artifacts)) {
    fail("artifacts must be an array");
  }
  if (record.artifacts.length > 1) {
    fail("artifacts may contain at most one execution-report entry");
  }
  const seen = new Set();
  for (const artifact of record.artifacts) {
    assertExactKeys(artifact, EVIDENCE_ARTIFACT_FIELDS, "evidence artifact");
    assertString(artifact.id, "evidence artifact id");
    if (seen.has(artifact.id)) fail(`duplicate evidence artifact ${artifact.id}`);
    seen.add(artifact.id);
    if (artifact.id !== EXECUTION_REPORT_ARTIFACT.id) {
      fail(`evidence artifact id ${artifact.id} is not supported`);
    }
    if (artifact.kind !== EXECUTION_REPORT_ARTIFACT.kind) {
      fail(`evidence artifact kind ${artifact.kind} is not supported`);
    }
    if (artifact.path !== EXECUTION_REPORT_ARTIFACT.path) {
      fail("execution-report artifact path must be exactly execution-report.json");
    }
    if (!SHA256_PATTERN.test(artifact.sha256)) {
      fail("execution-report artifact sha256 must be a 64-character lowercase hex digest");
    }
  }
  return record.artifacts.length === 1 ? record.artifacts[0] : null;
}

function validateHumanCheckpointRecords(record, workflow) {
  const declared = new Map(
    workflow.humanCheckpoints.map((checkpoint) => [checkpoint.id, checkpoint]),
  );
  if (!Array.isArray(record.humanCheckpoints)) {
    fail("humanCheckpoints must be an array");
  }
  const seen = new Set();
  for (const checkpoint of record.humanCheckpoints) {
    assertExactKeys(checkpoint, HUMAN_CHECKPOINT_RECORD_FIELDS, "human checkpoint record");
    assertString(checkpoint.checkpointId, "human checkpoint record checkpointId");
    const contract = declared.get(checkpoint.checkpointId);
    if (!contract) {
      fail(`human checkpoint id ${checkpoint.checkpointId} is not declared by the workflow`);
    }
    if (seen.has(checkpoint.checkpointId)) fail(`duplicate human checkpoint ${checkpoint.checkpointId}`);
    seen.add(checkpoint.checkpointId);
    if (!CHECKPOINT_OUTCOME_SET.has(checkpoint.outcome)) {
      fail(`human checkpoint ${checkpoint.checkpointId} outcome ${checkpoint.outcome} is not supported`);
    }
    if (!contract.allowedOutcomes.includes(checkpoint.outcome)) {
      fail(`human checkpoint ${checkpoint.checkpointId} outcome ${checkpoint.outcome} is not allowed by its contract`);
    }
    assertRfc3339(checkpoint.observedAt, `human checkpoint ${checkpoint.checkpointId} observedAt`);
    if (
      contract.required
      && checkpoint.outcome === "unable_to_verify"
      && (record.runValidity !== "invalid" || record.qualificationOutcome !== "not_observed")
    ) {
      fail('required checkpoint unable_to_verify requires runValidity "invalid" and qualificationOutcome "not_observed"');
    }
  }
  return seen;
}

export function validateEvidenceRecord(record, context) {
  assertExactKeys(record, EVIDENCE_RECORD_FIELDS, "evidence record");
  if (record.schemaVersion !== 2) fail("evidence record schemaVersion must be 2");
  if (!RUN_ID_PATTERN.test(record.runId)) fail("evidence record runId format is invalid");
  if (!DIGEST_PATTERN.test(record.recordDigest)) fail("evidence record recordDigest format is invalid");
  assertRfc3339(record.capturedAt, "capturedAt");
  assertString(record.workflowId, "workflowId");
  assertPositiveInteger(record.workflowVersion, "workflowVersion");
  const workflow = context.workflowCatalog.workflows.find(
    (candidate) => candidate.id === record.workflowId,
  );
  if (!workflow) fail(`unknown workflow id ${record.workflowId}`);
  if (record.workflowVersion !== workflow.version) {
    fail(`workflow version ${record.workflowVersion} does not match catalog version ${workflow.version}`);
  }
  const target = context.targets.find((candidate) => candidate.id === record.deviceTarget?.id);
  if (!target) fail(`unknown device target id ${record.deviceTarget?.id}`);
  assertExactKeys(record.deviceTarget, EVIDENCE_DEVICE_TARGET_FIELDS, "evidence device target");
  if (!TARGET_ID_PATTERN.test(record.deviceTarget.id)) {
    fail("evidence device target id format is invalid");
  }
  const observedFacts = new Map();
  for (const field of TARGET_FACT_FIELDS) {
    const validator = field === "androidApi" ? assertPositiveInteger : assertString;
    observedFacts.set(field, assertFactWrapper(record.deviceTarget, field, validator));
  }
  if (!ROOT_STATES.has(observedFacts.get("rootState").value)) {
    fail(`evidence device target rootState value ${observedFacts.get("rootState").value} is not supported`);
  }
  if (!CONNECTION_TYPES.has(observedFacts.get("connectionType").value)) {
    fail(`evidence device target connectionType value ${observedFacts.get("connectionType").value} is not supported`);
  }
  assertEvidenceDeviceTarget(record.deviceTarget, target);
  validateFingerprint(record.fingerprint);
  if (record.fingerprint.workflowVersion !== workflow.version) {
    fail("fingerprint workflowVersion does not match the workflow version");
  }
  if (record.fingerprint.deviceProfile !== targetFactValue(target, "profileId")) {
    fail("fingerprint deviceProfile does not match the registered target");
  }
  if (
    record.fingerprint.androidApi !== targetFactValue(target, "androidApi")
    || record.fingerprint.firmwareBuild !== targetFactValue(target, "firmwareBuild")
    || record.fingerprint.abiSocClass !== targetFactValue(target, "abiSocClass")
    || record.fingerprint.rootState !== targetFactValue(target, "rootState")
    || record.fingerprint.connectionType !== targetFactValue(target, "connectionType")
  ) {
    fail("fingerprint device facts do not match the registered target");
  }
  const actualRecipeIds = record.fingerprint.authoredContent.map((entry) => entry.id);
  if (JSON.stringify(actualRecipeIds) !== JSON.stringify(workflow.productionRecipes)) {
    fail("fingerprint authoredContent does not match the workflow production recipes");
  }
  if (record.fingerprintDigest !== evidenceFingerprintDigest(record.fingerprint)) {
    fail("fingerprintDigest does not match canonical fingerprint");
  }
  if (record.runId !== qualificationRunId(record)) {
    fail("runId does not match canonical qualification run identity");
  }
  if (record.recordDigest !== evidenceRecordDigest(record)) {
    fail("canonical record content digest does not match the evidence record");
  }
  if (!RUN_VALIDITIES.includes(record.runValidity)) {
    fail(`runValidity ${record.runValidity} is not supported`);
  }
  if (!QUALIFICATION_OUTCOMES.includes(record.qualificationOutcome)) {
    fail(`qualificationOutcome ${record.qualificationOutcome} is not supported`);
  }
  if (!TARGET_WIDE_FAILURES.includes(record.targetWideFailure)) {
    fail(`targetWideFailure ${record.targetWideFailure} is not supported`);
  }
  if (!Array.isArray(record.limitations) || record.limitations.some((item) => typeof item !== "string" || item.length === 0)) {
    fail("limitations must be an array of non-empty strings");
  }
  const executionReportArtifact = validateEvidenceArtifacts(record);
  validateAutomatedObservationRecords(record, workflow);
  validateHumanCheckpointRecords(record, workflow);
  const executionReportObservation = record.automatedObservations.find((observation) => observation.id === EXECUTION_REPORT_ARTIFACT.id);
  for (const checkpoint of record.humanCheckpoints) {
    if (
      PRE_EXECUTION_PREREQUISITE_CHECKPOINTS.has(checkpoint.checkpointId)
      && checkpoint.outcome !== "pass"
      && (record.runValidity !== "invalid" || record.qualificationOutcome !== "not_observed")
    ) {
      fail(`pre-execution checkpoint ${checkpoint.checkpointId} requires an invalid not_observed record when it does not pass`);
    }
  }
  if (record.runValidity === "invalid") {
    if (record.qualificationOutcome !== "not_observed") {
      fail('invalid run must use qualificationOutcome "not_observed"');
    }
    if (record.targetWideFailure !== null) {
      fail("an invalid run cannot record a target-wide failure");
    }
    if (executionReportArtifact === null && executionReportObservation) {
      fail("invalid run cannot record an execution-report observation without an execution-report artifact");
    }
    return true;
  }
  if (record.qualificationOutcome === "not_observed") {
    fail('valid run must use qualificationOutcome "passed" or "failed"');
  }
  if (executionReportArtifact === null) {
    fail("valid run must include an execution-report artifact");
  }
  if (!executionReportObservation) {
    fail("missing required automated observation execution-report");
  }
  if (record.targetWideFailure !== null && record.qualificationOutcome !== "failed") {
    fail("targetWideFailure requires a failed valid record");
  }
  const observedIds = new Set(record.automatedObservations.map((observation) => observation.id));
  for (const observation of workflow.automatedObservations) {
    if (observation.required && !observedIds.has(observation.id)) {
      fail(`missing required automated observation ${observation.id}`);
    }
  }
  const checkpointIds = new Set(record.humanCheckpoints.map((checkpoint) => checkpoint.checkpointId));
  for (const checkpoint of workflow.humanCheckpoints) {
    if (checkpoint.required && !checkpointIds.has(checkpoint.id)) {
      fail(`missing required human checkpoint ${checkpoint.id}`);
    }
  }
  if (record.qualificationOutcome === "passed") {
    for (const observation of workflow.automatedObservations) {
      if (observation.required) {
        const result = record.automatedObservations.find((item) => item.id === observation.id);
        if (!result || result.outcome !== "passed") {
          fail(`required automated observation ${observation.id} did not pass`);
        }
      }
    }
    for (const checkpoint of workflow.humanCheckpoints) {
      if (checkpoint.required) {
        const result = record.humanCheckpoints.find((item) => item.checkpointId === checkpoint.id);
        if (!result || result.outcome !== "pass") {
          fail(`required human checkpoint ${checkpoint.id} did not pass`);
        }
      }
    }
    if (
      record.automatedObservations.some((item) => item.outcome === "failed")
      || record.humanCheckpoints.some((item) => item.outcome === "fail")
    ) {
      fail("a passed record cannot contain failed observations or checkpoints");
    }
  } else {
    const hasFailure = record.automatedObservations.some((item) => item.outcome === "failed")
      || record.humanCheckpoints.some((item) => item.outcome === "fail")
      || record.targetWideFailure !== null;
    if (!hasFailure) {
      fail("failed record has no failed observation, failed checkpoint, or target-wide failure");
    }
  }
  return true;
}

export function validateEvidenceBundle(bundle, context) {
  assertObject(bundle, "evidence bundle");
  const { record, reportBytes } = bundle;
  validateEvidenceRecord(record, context);
  const executionReportArtifact = record.artifacts[0] ?? null;
  if (executionReportArtifact === null) {
    if (reportBytes !== null) {
      fail("bundle must not include execution report bytes when no execution-report artifact is referenced");
    }
    if (record.runValidity !== "invalid" || record.qualificationOutcome !== "not_observed") {
      fail("only invalid not_observed evidence may omit an execution-report artifact");
    }
    return true;
  }
  if (!(reportBytes instanceof Buffer)) {
    fail("execution-report artifact requires execution report bytes");
  }
  const actualSha256 = createHash("sha256").update(reportBytes).digest("hex");
  if (actualSha256 !== executionReportArtifact.sha256) {
    fail("execution report digest does not match the bound artifact sha256");
  }
  return true;
}

function validateBuildIdentitySchemaContract(schema) {
  const definition = schema.$defs?.qualificationBuildIdentity;
  assertObject(definition, "evidence schema qualificationBuildIdentity");
  if (definition.additionalProperties !== false) {
    fail("evidence schema qualificationBuildIdentity must reject additional properties");
  }
  if (!equalJson(definition.required, BUILD_IDENTITY_FIELDS)) {
    fail("evidence schema qualificationBuildIdentity required fields drifted from the validator");
  }
  assertObject(definition.properties, "evidence schema qualificationBuildIdentity properties");
  if (!equalJson(Object.keys(definition.properties), BUILD_IDENTITY_FIELDS)) {
    fail("evidence schema qualificationBuildIdentity properties drifted from the validator");
  }
  if (!equalJson(definition.properties.appVersion, { type: "string", minLength: 1 })) {
    fail("evidence schema qualificationBuildIdentity appVersion drifted from the validator");
  }
  if (!equalJson(definition.properties.gitCommit, { type: "string", pattern: GIT_COMMIT_PATTERN.source })) {
    fail("evidence schema qualificationBuildIdentity gitCommit drifted from the validator");
  }
  if (!equalJson(definition.properties.materialBuildDigest, { type: "string", pattern: DIGEST_PATTERN.source })) {
    fail("evidence schema qualificationBuildIdentity materialBuildDigest drifted from the validator");
  }
  if (!equalJson(definition.properties.realExecutionEnabled, { type: "boolean" })) {
    fail("evidence schema qualificationBuildIdentity realExecutionEnabled drifted from the validator");
  }
  if (!equalJson(definition.properties.qualificationContract, { type: "integer", minimum: 1 })) {
    fail("evidence schema qualificationBuildIdentity qualificationContract drifted from the validator");
  }
}

export function validateEvidenceSchemaContract(schema) {
  assertObject(schema, "evidence schema");
  if (!equalJson(schema.required, EVIDENCE_RECORD_FIELDS)) {
    fail("evidence schema top-level fields drifted from the validator");
  }
  if (!equalJson(schema.properties.runValidity.enum, RUN_VALIDITIES)) {
    fail("evidence schema runValidity enum drifted from the validator");
  }
  if (!equalJson(schema.properties.qualificationOutcome.enum, QUALIFICATION_OUTCOMES)) {
    fail("evidence schema qualificationOutcome enum drifted from the validator");
  }
  if (!equalJson(
    schema.properties.targetWideFailure.oneOf[1].enum,
    TARGET_WIDE_FAILURES.filter((value) => value !== null),
  )) {
    fail("evidence schema targetWideFailure enum drifted from the validator");
  }
  if (!equalJson(schema.$defs.fingerprint.required, FINGERPRINT_FIELDS)) {
    fail("evidence schema fingerprint fields drifted from the validator");
  }
  validateBuildIdentitySchemaContract(schema);
  if (!equalJson(schema.$defs.deviceTarget.required, EVIDENCE_DEVICE_TARGET_FIELDS)) {
    fail("evidence schema deviceTarget fields drifted from the validator");
  }
  if (!equalJson(schema.$defs.automatedObservation.required, AUTOMATED_OBSERVATION_RECORD_FIELDS)) {
    fail("evidence schema automated observation fields drifted from the validator");
  }
  if (!equalJson(schema.$defs.humanCheckpoint.required, HUMAN_CHECKPOINT_RECORD_FIELDS)) {
    fail("evidence schema human checkpoint fields drifted from the validator");
  }
  if (!equalJson(schema.$defs.automatedObservation.properties.outcome.enum, ["passed", "failed"])) {
    fail("evidence schema automated observation outcomes drifted from the validator");
  }
  if (!equalJson(schema.$defs.humanCheckpoint.properties.outcome.enum, CHECKPOINT_OUTCOMES)) {
    fail("evidence schema human checkpoint outcomes drifted from the validator");
  }
  if (!equalJson(schema.$defs.evidenceArtifact.required, EVIDENCE_ARTIFACT_FIELDS)) {
    fail("evidence schema artifact fields drifted from the validator");
  }
  if (!equalJson(schema.properties.artifacts.items, { $ref: "#/$defs/evidenceArtifact" })) {
    fail("evidence schema artifacts item reference drifted from the validator");
  }
  return true;
}

export function buildCurrentFingerprint({
  workflow,
  target,
  currentBuild,
  runtimeContract,
  authoredContentDigests,
}) {
  const authoredContent = workflow.productionRecipes.map((recipeId) => {
    const sha256 = authoredContentDigests[recipeId];
    if (typeof sha256 !== "string" || !SHA256_PATTERN.test(sha256)) {
      fail(`authored content digest is missing or invalid for ${recipeId}`);
    }
    return { id: recipeId, sha256 };
  });
  return {
    schemaVersion: 2,
    emuchefBuild: currentBuild,
    workflowVersion: workflow.version,
    authoredContent,
    runtimeContract,
    deviceProfile: targetFactValue(target, "profileId"),
    androidApi: targetFactValue(target, "androidApi"),
    firmwareBuild: targetFactValue(target, "firmwareBuild"),
    abiSocClass: targetFactValue(target, "abiSocClass"),
    rootState: targetFactValue(target, "rootState"),
    connectionType: targetFactValue(target, "connectionType"),
  };
}

export function deriveApplicability(workflow, target) {
  if (target.deferredWorkflows.includes(workflow.id)) {
    return { state: "deferred", reason: "explicitly_deferred" };
  }
  const missing = workflow.requiredCapabilities.filter(
    (capability) => !target.capabilities.includes(capability),
  );
  if (missing.length > 0) {
    return { state: "not_applicable", reason: `missing_capabilities:${missing.join(",")}` };
  }
  return { state: "required", reason: "production_intent_and_capabilities" };
}

function byDateThenId(left, right) {
  return String(left.capturedAt).localeCompare(String(right.capturedAt))
    || String(left.runId).localeCompare(String(right.runId));
}

export function selectCurrentEvidence({
  workflow,
  target,
  currentFingerprint,
  records,
}) {
  const eligible = records
    .filter((record) => (
      record.deviceTarget?.id === target.id
      && record.workflowId === workflow.id
      && record.workflowVersion === workflow.version
      && record.runValidity === "valid"
      && classifyCompatibility({
        workflow,
        currentFingerprint,
        evidenceFingerprint: record.fingerprint,
      }) === "compatible"
    ))
    .sort(byDateThenId);
  return eligible.length > 0 ? eligible[eligible.length - 1] : null;
}

function newestHistoricalRecord({ workflow, target, records }) {
  return records
    .filter((record) => (
      record.deviceTarget?.id === target.id
      && record.workflowId === workflow.id
      && record.workflowVersion === workflow.version
      && record.runValidity === "valid"
    ))
    .sort(byDateThenId)
    .at(-1) ?? null;
}

export function deriveWorkflowState({
  workflow,
  target,
  currentFingerprint,
  records,
}) {
  const applicability = deriveApplicability(workflow, target);
  if (applicability.state !== "required") {
    return {
      workflowId: workflow.id,
      applicability: applicability.state,
      state: applicability.state,
      reason: applicability.reason,
      runId: null,
      capturedAt: null,
      targetWideFailure: null,
    };
  }
  const current = selectCurrentEvidence({
    workflow,
    target,
    currentFingerprint,
    records,
  });
  if (current) {
    return {
      workflowId: workflow.id,
      applicability: "required",
      state: current.qualificationOutcome === "passed" ? "qualified" : "failed",
      reason: current.limitations.length > 0 ? current.limitations.join("; ") : null,
      runId: current.runId,
      capturedAt: current.capturedAt,
      targetWideFailure: current.targetWideFailure,
    };
  }
  const historical = newestHistoricalRecord({ workflow, target, records });
  if (historical) {
    return {
      workflowId: workflow.id,
      applicability: "required",
      state: "stale",
      reason: "no current compatible evidence; historical valid evidence exists",
      runId: historical.runId,
      capturedAt: historical.capturedAt,
      targetWideFailure: null,
    };
  }
  return {
    workflowId: workflow.id,
    applicability: "required",
    state: "missing",
    reason: "no applicable valid physical evidence",
    runId: null,
    capturedAt: null,
    targetWideFailure: null,
  };
}

export function deriveDeviceSupportTier(workflowStates) {
  if (workflowStates.some((row) => row.targetWideFailure !== null)) return "unqualified";
  const required = workflowStates.filter((row) => row.applicability === "required");
  if (required.length === 0) return "unqualified";
  if (required.every((row) => row.state === "qualified")) return "qualified";
  if (required.some((row) => row.state === "qualified")) return "limited";
  return "unqualified";
}

function qualificationPaths(repoRoot) {
  const authoredProfilesDir = path.join(repoRoot, "authored/device_profiles");
  return {
    repoRoot,
    authoredProfilesDir,
    workflowCatalogPath: path.join(repoRoot, "docs/testing/device-qualification/workflow-catalog.json"),
    deviceTargetsPath: path.join(repoRoot, "docs/testing/device-qualification/device-targets.json"),
    evidenceRoot: path.join(repoRoot, "docs/testing/device-qualification/evidence"),
    matrixPath: path.join(repoRoot, "docs/qualification/device-qualification-matrix.md"),
    candidateRoot: path.join(repoRoot, ".emuchef_runtime/qualification-candidates"),
  };
}

function candidateDirectory(candidateId, repoRoot = REPO_ROOT) {
  if (!CANDIDATE_ID_PATTERN.test(candidateId)) fail("qualification candidate id is invalid");
  return path.join(qualificationPaths(repoRoot).candidateRoot, candidateId);
}

function loadCandidateJson(candidateId, repoRoot = REPO_ROOT) {
  const directory = candidateDirectory(candidateId, repoRoot);
  const candidatePath = path.join(directory, "candidate.json");
  if (!existsSync(candidatePath)) {
    fail(`qualification candidate ${candidateId} does not exist`);
  }
  const candidate = JSON.parse(readFileSync(candidatePath, "utf8"));
  if (candidate.candidateId !== candidateId) {
    fail("qualification candidate payload does not match its directory id");
  }
  return {
    directory,
    candidate,
  };
}

function validateCandidateEnvelope(candidate, { kind, fields }) {
  assertExactKeys(candidate, fields, `${kind} candidate`);
  if (candidate.candidateSchemaVersion !== 1) {
    fail(`${kind} candidate schema version must be 1`);
  }
  if (!CANDIDATE_ID_PATTERN.test(candidate.candidateId)) {
    fail(`${kind} candidate id is invalid`);
  }
  if (candidate.kind !== kind) {
    fail(`${kind} candidate kind must be ${kind}`);
  }
  assertRfc3339(candidate.capturedAt, `${kind} candidate capturedAt`);
  validateBuildIdentity(candidate.build, `${kind} candidate build`);
}

function validateTargetRegistrationCandidate(candidate, repoRoot = REPO_ROOT) {
  validateCandidateEnvelope(candidate, {
    kind: "target_registration",
    fields: TARGET_REGISTRATION_CANDIDATE_FIELDS,
  });
  const target = structuredClone(candidate.target);
  target.id = deviceTargetId(target);
  validateDeviceTargets(
    { schemaVersion: 2, targets: [target] },
    { authoredProfilesDir: path.join(repoRoot, "authored/device_profiles") },
  );
  return target;
}

function validateQualificationRunCandidate(candidate) {
  validateCandidateEnvelope(candidate, {
    kind: "qualification_run",
    fields: QUALIFICATION_RUN_CANDIDATE_FIELDS,
  });
  if (!TARGET_ID_PATTERN.test(candidate.deviceTargetId)) {
    fail("qualification_run candidate deviceTargetId format is invalid");
  }
  validateFingerprint(candidate.fingerprint);
  if (!RUN_VALIDITIES.includes(candidate.runValidity)) {
    fail(`qualification_run candidate runValidity ${candidate.runValidity} is not supported`);
  }
  if (!QUALIFICATION_OUTCOMES.includes(candidate.qualificationOutcome)) {
    fail(`qualification_run candidate qualificationOutcome ${candidate.qualificationOutcome} is not supported`);
  }
  if (!TARGET_WIDE_FAILURES.includes(candidate.targetWideFailure)) {
    fail(`qualification_run candidate targetWideFailure ${candidate.targetWideFailure} is not supported`);
  }
  if (!Array.isArray(candidate.limitations) || candidate.limitations.some((item) => typeof item !== "string" || item.length === 0)) {
    fail("qualification_run candidate limitations must be an array of non-empty strings");
  }
  validateEvidenceArtifacts(candidate);
  return candidate;
}

function validatePromotionBuildState(candidateBuild, repoRoot) {
  if (trackedWorktreeStatus(repoRoot) !== "") {
    fail("device qualification requires a clean tracked worktree");
  }
  const currentCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim();
  if (candidateBuild.gitCommit !== currentCommit) {
    fail("qualification candidate git commit no longer matches HEAD");
  }
  const currentBuild = buildMaterialIdentity({ repoRoot, requireClean: false });
  if (candidateBuild.materialBuildDigest !== currentBuild.materialBuildDigest) {
    fail("qualification candidate material build digest no longer matches the current repository state");
  }
  if (candidateBuild.qualificationContract !== QUALIFICATION_CONTRACT_VERSION) {
    fail(`qualification candidate qualificationContract must be ${QUALIFICATION_CONTRACT_VERSION}`);
  }
  if (candidateBuild.realExecutionEnabled !== true) {
    fail("qualification candidate must be captured from a real-execution-enabled build");
  }
  return currentBuild;
}

function createTemporaryPath(targetPath) {
  return `${targetPath}.tmp-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function writeTemporaryFile(targetPath, content, fsOps) {
  fsOps.mkdirSync(path.dirname(targetPath), { recursive: true });
  const temporaryPath = createTemporaryPath(targetPath);
  fsOps.writeFileSync(temporaryPath, content, "utf8");
  return temporaryPath;
}

function renderMatrixForState({ workflowCatalog, targets, records, currentBuild, runtimeContract, authoredContentDigests }) {
  return renderQualificationMatrix(projectQualificationState({
    workflowCatalog,
    targets,
    records,
    currentBuild,
    runtimeContract,
    authoredContentDigests,
  }));
}

function ensureDirectoryAbsent(targetPath) {
  if (existsSync(targetPath)) {
    fail(`canonical evidence directory ${path.basename(targetPath)} already exists`);
  }
}

function currentQualificationContext(repoRoot) {
  const paths = qualificationPaths(repoRoot);
  const workflowCatalog = loadWorkflowCatalog(paths.workflowCatalogPath);
  const targetFile = loadDeviceTargets(paths.deviceTargetsPath, { authoredProfilesDir: paths.authoredProfilesDir });
  return {
    paths,
    workflowCatalog,
    targets: targetFile.targets,
    authoredContentDigests: productionAuthoredContentDigests(workflowCatalog, repoRoot),
  };
}

export function loadEvidenceBundle(bundlePath) {
  const resolved = path.resolve(bundlePath);
  if (!existsSync(resolved)) {
    fail(`evidence bundle ${resolved} does not exist`);
  }
  const evidencePath = path.join(resolved, "evidence.json");
  if (!existsSync(evidencePath)) {
    fail(`evidence bundle ${resolved} must contain evidence.json`);
  }
  const record = JSON.parse(readFileSync(evidencePath, "utf8"));
  const reportPath = path.join(resolved, EXECUTION_REPORT_ARTIFACT.path);
  return {
    record,
    reportBytes: existsSync(reportPath) ? readFileSync(reportPath) : null,
  };
}

export function loadEvidenceDirectory(evidenceDir, { fixtureMode }) {
  const resolved = path.resolve(evidenceDir);
  const fixtureRoot = path.resolve(REPO_ROOT, "tests/fixtures");
  if (
    !fixtureMode
    && (resolved === fixtureRoot || resolved.startsWith(`${fixtureRoot}${path.sep}`))
  ) {
    fail("synthetic fixture path cannot be used as production evidence");
  }
  if (!existsSync(resolved)) return [];
  const bundles = [];
  const seenRunIds = new Set();
  for (const entry of readdirSync(resolved, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
    if (entry.name === "README.md") continue;
    let bundle;
    if (entry.isDirectory()) {
      bundle = loadEvidenceBundle(path.join(resolved, entry.name));
    } else if (fixtureMode && entry.isFile() && entry.name.endsWith(".json")) {
      bundle = {
        record: JSON.parse(readFileSync(path.join(resolved, entry.name), "utf8")),
        reportBytes: null,
      };
    } else {
      continue;
    }
    const record = bundle.record;
    if (seenRunIds.has(record.runId)) fail(`duplicate run identity ${record.runId}`);
    seenRunIds.add(record.runId);
    bundles.push(bundle);
  }
  return bundles.map((bundle) => bundle.record);
}

export function projectQualificationState({
  workflowCatalog,
  targets,
  records,
  currentBuild,
  runtimeContract,
  authoredContentDigests,
}) {
  return {
    targets: targets.map((target) => {
      const rows = workflowCatalog.workflows.map((workflow) => deriveWorkflowState({
        workflow,
        target,
        currentFingerprint: buildCurrentFingerprint({
          workflow,
          target,
          currentBuild,
          runtimeContract,
          authoredContentDigests,
        }),
        records,
      }));
      return {
        target,
        tier: deriveDeviceSupportTier(rows),
        workflows: rows.filter((row) => row.state !== "not_applicable"),
      };
    }),
  };
}

function renderTargetSection(entry) {
  const { target, tier, workflows } = entry;
  const lines = [
    `## ${target.id}`,
    "",
    `- Configuration: ${targetFactValue(target, "manufacturer")} ${targetFactValue(target, "model")}, Android ${targetFactValue(target, "androidVersion")} (API ${targetFactValue(target, "androidApi")}), ${targetFactValue(target, "abiSocClass")}, ${targetFactValue(target, "rootState")}, ${targetFactValue(target, "connectionType")}`,
    `- Authored profile: ${targetFactValue(target, "profileId")}`,
    `- Support tier: ${tier}`,
    "",
    "| Workflow | State | Current evidence | Reason / limitation |",
    "|---|---|---|---|",
  ];
  for (const row of workflows) {
    const evidence = row.runId ? `${row.runId} (${row.capturedAt})` : "—";
    const reason = row.reason ?? "—";
    lines.push(`| ${row.workflowId} | ${row.state} | ${evidence} | ${reason} |`);
  }
  return lines.join("\n");
}

export function renderQualificationMatrix(projection) {
  const lines = [
    "# Phase 6F Physical-Device Qualification Matrix",
    "",
    "Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.",
    "",
  ];
  if (projection.targets.length === 0) {
    lines.push(
      "No physical-device qualification targets have been registered yet. Phase 6F foundation infrastructure exists, but no device or workflow is physically qualified by this repository state.",
    );
    lines.push("");
    return lines.join("\n");
  }
  for (const entry of projection.targets) {
    lines.push(renderTargetSection(entry), "");
  }
  return lines.join("\n");
}

function productionAuthoredContentDigests(workflowCatalog, repoRoot = REPO_ROOT) {
  const digests = {};
  for (const workflow of workflowCatalog.workflows) {
    for (const recipeId of workflow.productionRecipes) {
      const recipePath = path.join(repoRoot, "authored/recipes", `${recipeId}.yaml`);
      digests[recipeId] = createHash("sha256").update(readFileSync(recipePath)).digest("hex");
    }
  }
  return digests;
}

export function registerQualificationTargetCandidate(candidateId, { repoRoot = REPO_ROOT, fsOps = defaultFsOps } = {}) {
  const { candidate } = loadCandidateJson(candidateId, repoRoot);
  const target = validateTargetRegistrationCandidate(candidate, repoRoot);
  const paths = qualificationPaths(repoRoot);
  const targetFile = loadDeviceTargets(paths.deviceTargetsPath, { authoredProfilesDir: paths.authoredProfilesDir });
  if (targetFile.targets.some((entry) => entry.id === target.id)) {
    fail(`canonical device target ${target.id} already exists`);
  }
  const currentBuild = validatePromotionBuildState(candidate.build, repoRoot);
  const workflowCatalog = loadWorkflowCatalog(paths.workflowCatalogPath);
  const authoredContentDigests = productionAuthoredContentDigests(workflowCatalog, repoRoot);
  const nextTargets = {
    schemaVersion: targetFile.schemaVersion,
    targets: [...targetFile.targets, target],
  };
  const matrixText = renderMatrixForState({
    workflowCatalog,
    targets: nextTargets.targets,
    records: loadEvidenceDirectory(paths.evidenceRoot, { fixtureMode: false }),
    currentBuild,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests,
  });
  const originalTargets = readFileSync(paths.deviceTargetsPath, "utf8");
  const originalMatrix = readFileSync(paths.matrixPath, "utf8");
  const tempTargetsPath = writeTemporaryFile(
    paths.deviceTargetsPath,
    `${JSON.stringify(nextTargets, null, 2)}\n`,
    fsOps,
  );
  const tempMatrixPath = writeTemporaryFile(paths.matrixPath, matrixText, fsOps);
  try {
    fsOps.renameSync(tempTargetsPath, paths.deviceTargetsPath);
    fsOps.renameSync(tempMatrixPath, paths.matrixPath);
  } catch (error) {
    fsOps.writeFileSync(paths.deviceTargetsPath, originalTargets, "utf8");
    fsOps.writeFileSync(paths.matrixPath, originalMatrix, "utf8");
    fsOps.rmSync(tempTargetsPath, { force: true, recursive: true });
    fsOps.rmSync(tempMatrixPath, { force: true, recursive: true });
    throw error;
  }
  return target;
}

export function recordQualificationRunCandidate(candidateId, { repoRoot = REPO_ROOT, fsOps = defaultFsOps } = {}) {
  const { directory, candidate } = loadCandidateJson(candidateId, repoRoot);
  validateQualificationRunCandidate(candidate);
  const { paths, workflowCatalog, targets, authoredContentDigests } = currentQualificationContext(repoRoot);
  const workflow = workflowCatalog.workflows.find((item) => item.id === candidate.workflowId);
  if (!workflow) fail(`unknown workflow id ${candidate.workflowId}`);
  if (candidate.workflowVersion !== workflow.version) {
    fail(`qualification candidate workflow version ${candidate.workflowVersion} does not match catalog version ${workflow.version}`);
  }
  const target = targets.find((item) => item.id === candidate.deviceTargetId);
  if (!target) fail(`unknown device target id ${candidate.deviceTargetId}`);
  const provisionalRecord = sealEvidenceRecord({
    schemaVersion: 2,
    capturedAt: candidate.capturedAt,
    workflowId: workflow.id,
    workflowVersion: workflow.version,
    deviceTarget: buildEvidenceDeviceTarget(target),
    fingerprint: structuredClone(candidate.fingerprint),
    runValidity: candidate.runValidity,
    qualificationOutcome: candidate.qualificationOutcome,
    automatedObservations: structuredClone(candidate.automatedObservations),
    humanCheckpoints: structuredClone(candidate.humanCheckpoints),
    targetWideFailure: candidate.targetWideFailure,
    limitations: structuredClone(candidate.limitations),
    artifacts: structuredClone(candidate.artifacts),
  });
  const bundleDir = path.join(paths.evidenceRoot, provisionalRecord.runId);
  ensureDirectoryAbsent(bundleDir);
  const currentBuild = validatePromotionBuildState(candidate.build, repoRoot);
  const expectedFingerprint = buildCurrentFingerprint({
    workflow,
    target,
    currentBuild,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests,
  });
  if (!equalJson(candidate.fingerprint, expectedFingerprint)) {
    fail("qualification candidate fingerprint no longer matches the current canonical workflow, target, and authored content");
  }
  const reportPath = path.join(directory, EXECUTION_REPORT_ARTIFACT.path);
  const reportBytes = existsSync(reportPath) ? readFileSync(reportPath) : null;
  const sealedRecord = sealEvidenceRecord({
    schemaVersion: 2,
    capturedAt: candidate.capturedAt,
    workflowId: workflow.id,
    workflowVersion: workflow.version,
    deviceTarget: buildEvidenceDeviceTarget(target),
    fingerprint: expectedFingerprint,
    runValidity: candidate.runValidity,
    qualificationOutcome: candidate.qualificationOutcome,
    automatedObservations: structuredClone(candidate.automatedObservations),
    humanCheckpoints: structuredClone(candidate.humanCheckpoints),
    targetWideFailure: candidate.targetWideFailure,
    limitations: structuredClone(candidate.limitations),
    artifacts: structuredClone(candidate.artifacts),
  });
  validateEvidenceBundle({ record: sealedRecord, reportBytes }, { workflowCatalog, targets });
  const existingRecords = loadEvidenceDirectory(paths.evidenceRoot, { fixtureMode: false });
  const matrixText = renderMatrixForState({
    workflowCatalog,
    targets,
    records: [...existingRecords, sealedRecord],
    currentBuild,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests,
  });
  const temporaryBundleDir = createTemporaryPath(bundleDir);
  fsOps.mkdirSync(temporaryBundleDir, { recursive: true });
  fsOps.writeFileSync(path.join(temporaryBundleDir, "evidence.json"), `${JSON.stringify(sealedRecord, null, 2)}\n`, "utf8");
  if (reportBytes !== null) {
    fsOps.writeFileSync(path.join(temporaryBundleDir, EXECUTION_REPORT_ARTIFACT.path), reportBytes);
  }
  const tempMatrixPath = writeTemporaryFile(paths.matrixPath, matrixText, fsOps);
  try {
    fsOps.renameSync(temporaryBundleDir, bundleDir);
    fsOps.renameSync(tempMatrixPath, paths.matrixPath);
  } catch (error) {
    fsOps.rmSync(temporaryBundleDir, { recursive: true, force: true });
    fsOps.rmSync(bundleDir, { recursive: true, force: true });
    fsOps.rmSync(tempMatrixPath, { recursive: true, force: true });
    throw error;
  }
  return sealedRecord;
}

function projectProductionQualification() {
  const workflowCatalog = loadWorkflowCatalog(path.join(QUALIFICATION_ROOT, "workflow-catalog.json"));
  const targetsFile = loadDeviceTargets(
    path.join(QUALIFICATION_ROOT, "device-targets.json"),
    { authoredProfilesDir: path.join(REPO_ROOT, "authored/device_profiles") },
  );
  const schema = JSON.parse(readFileSync(
    path.join(QUALIFICATION_ROOT, "evidence-schema.json"),
    "utf8",
  ));
  validateEvidenceSchemaContract(schema);
  const bundles = [];
  if (existsSync(EVIDENCE_ROOT)) {
    for (const entry of readdirSync(EVIDENCE_ROOT, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
      if (entry.name === "README.md" || !entry.isDirectory()) continue;
      const bundle = loadEvidenceBundle(path.join(EVIDENCE_ROOT, entry.name));
      validateEvidenceBundle(bundle, { workflowCatalog, targets: targetsFile.targets });
      bundles.push(bundle);
    }
  }
  const records = bundles.map((bundle) => bundle.record);
  return projectQualificationState({
    workflowCatalog,
    targets: targetsFile.targets,
    records,
    currentBuild: buildMaterialIdentity({ repoRoot: REPO_ROOT, requireClean: false }),
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests: productionAuthoredContentDigests(workflowCatalog),
  });
}

function repositoryDescription() {
  return {
    schemaVersion: 1,
    runtimeContract: RUNTIME_CONTRACT,
    qualificationContract: QUALIFICATION_CONTRACT_VERSION,
    build: buildMaterialIdentity({ repoRoot: REPO_ROOT, requireClean: false }),
    workflowCatalog: loadWorkflowCatalog(path.join(QUALIFICATION_ROOT, "workflow-catalog.json")),
    deviceTargets: loadDeviceTargets(
      path.join(QUALIFICATION_ROOT, "device-targets.json"),
      { authoredProfilesDir: path.join(REPO_ROOT, "authored/device_profiles") },
    ),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  const candidateActions = new Set(["--register-target", "--record-run"]);
  const allowedArgs = new Set(["--build-identity", "--require-clean", "--describe", "--check", "--write-matrix", ...candidateActions]);
  const actionArgs = args.filter((arg) => ["--build-identity", "--describe", "--check", "--write-matrix", ...candidateActions].includes(arg));
  const registerTargetIndex = args.indexOf("--register-target");
  const recordRunIndex = args.indexOf("--record-run");
  const candidateValueIndexes = new Set();
  if (registerTargetIndex !== -1) candidateValueIndexes.add(registerTargetIndex + 1);
  if (recordRunIndex !== -1) candidateValueIndexes.add(recordRunIndex + 1);
  if (
    args.some((arg, index) => !allowedArgs.has(arg) && !candidateValueIndexes.has(index))
    || actionArgs.length !== 1
    || (args.includes("--require-clean") && !args.includes("--build-identity"))
    || (registerTargetIndex !== -1 && args.length !== 2)
    || (recordRunIndex !== -1 && args.length !== 2)
  ) {
    process.stderr.write("usage: node tools/device-qualification.mjs [--build-identity [--require-clean]|--describe|--check|--write-matrix|--register-target <candidate-id>|--record-run <candidate-id>]\n");
    process.exitCode = 1;
  } else {
    try {
      if (args.includes("--build-identity")) {
        process.stdout.write(`${JSON.stringify(buildMaterialIdentity({
          repoRoot: REPO_ROOT,
          requireClean: args.includes("--require-clean"),
        }))}\n`);
      } else if (args.includes("--describe")) {
        process.stdout.write(`${JSON.stringify(repositoryDescription())}\n`);
      } else if (registerTargetIndex !== -1) {
        const target = registerQualificationTargetCandidate(args[registerTargetIndex + 1], { repoRoot: REPO_ROOT });
        process.stdout.write(`${JSON.stringify(target)}\n`);
      } else if (recordRunIndex !== -1) {
        const record = recordQualificationRunCandidate(args[recordRunIndex + 1], { repoRoot: REPO_ROOT });
        process.stdout.write(`${JSON.stringify(record)}\n`);
      } else {
        const projection = projectProductionQualification();
        const rendered = renderQualificationMatrix(projection);
        const matrixPath = path.join(REPO_ROOT, "docs/qualification/device-qualification-matrix.md");
        if (args.includes("--write-matrix")) {
          writeFileSync(matrixPath, rendered, "utf8");
        }
        const committed = readFileSync(matrixPath, "utf8");
        if (committed !== rendered) {
          fail("generated Phase 6F qualification matrix is out of date; run --write-matrix");
        }
        process.stdout.write("Phase 6F qualification foundation check passed.\n");
      }
    } catch (error) {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
      process.exitCode = 1;
    }
  }
}

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
 * The standalone CLI has no JSON Schema package dependency. The adjacent
 * execution-report module reads the checked-in schema, while this tool checks
 * the record and cross-record bindings that the schema cannot express.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  closeSync,
  constants as fsConstants,
  existsSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { validateExecutionReportForInventory } from "./device-qualification-execution-report.mjs";
export { validateExecutionReport } from "./device-qualification-execution-report.mjs";

const REPO_ROOT = fileURLToPath(new URL("../", import.meta.url));
const QUALIFICATION_ROOT = path.join(
  REPO_ROOT,
  "docs/testing/device-qualification",
);
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
  [
    "connectionType",
    new Set(["operator_attestation", "production_observation"]),
  ],
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
  "docs/testing/device-qualification/execution-report.schema.json",
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
const EVIDENCE_DEVICE_TARGET_FIELDS = ["id", ...TARGET_FACT_FIELDS];
const AUTOMATED_OBSERVATION_RECORD_FIELDS = ["id", "outcome", "observedAt"];
const HUMAN_CHECKPOINT_RECORD_FIELDS = [
  "checkpointId",
  "outcome",
  "observedAt",
];
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
const RFC3339_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;
const GIT_COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const EXECUTION_REPORT_ARTIFACT = {
  id: "execution-report",
  kind: "production_execution_report",
  path: "execution-report.json",
};
const ROM_MERGE_EVIDENCE_ARTIFACT = {
  id: "rom-merge-evidence",
  kind: "rom_merge_evidence",
  path: "rom-merge-evidence.json",
};
const ROM_MERGE_EVIDENCE_FIELDS = [
  "schemaVersion",
  "candidateHandle",
  "sessionId",
  "executionReportSha256",
  "deviceTargetId",
  "deviceProfileId",
  "androidApi",
  "firmwareBuild",
  "abiSocClass",
  "connectionType",
  "observedRootState",
  "buildIdentity",
  "reviewedPlanId",
  "reviewedPlanDigest",
  "terminalOperation",
  "copyPolicy",
  "strategyDigest",
  "sourceSnapshot",
  "destination",
  "sentinel",
  "retroArch",
  "rootAuthorityUsed",
  "oemCompilerUsed",
  "oemStageUsed",
];
const ROM_MERGE_SESSION_ID_PATTERN = /^qualification-session-[0-9a-f]{32}$/;
const ROM_MERGE_PIN_PATH = "docs/testing/device-qualification/rom-merge-apk-pin.json";
const ROM_MERGE_SENTINEL_PATH = "qualification-sentinel.keep";
const ROM_MERGE_STRATEGY_DOMAIN = "emuchef.rom-merge.strategy";
// Keep this sidecar limit aligned with the Rust attachment command.
const MAX_ROM_MERGE_EVIDENCE_BYTES = 1024 * 1024;
// These are the effective capabilities the restricted qualification session emits.
const ROM_MERGE_UNPRIVILEGED_CAPABILITIES = new Set([
  "adb_available",
  "apk_install",
  "shared_storage_write",
  "app_launch",
  "shell_command",
  "package_remove_for_user",
]);
const CANDIDATE_ENVELOPE_FIELDS = [
  "candidateHandle",
  "kind",
  "capturedAt",
  "build",
  "payload",
  "report",
];
const CANDIDATE_REPORT_FIELDS = ["path", "byteLength", "sha256"];
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
  if (
    !Array.isArray(value) ||
    value.some((item) => typeof item !== "string" || item.length === 0)
  ) {
    fail(`${label} must be an array of non-empty strings`);
  }
  if (new Set(value).size !== value.length) {
    fail(`${label} must not contain duplicates`);
  }
}

function assertPositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(`${label} must be a positive integer`);
  }
}

function assertNonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    fail(`${label} must be a non-negative integer`);
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
    fail(
      `device target ${field} source must be one of ${[...legalSources].join(", ")}`,
    );
  }
  validator(fact.value, `device target ${field} value`);
  return fact;
}

export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalize(value[key])]),
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
  return (
    MATERIAL_EXACT_FILES.has(relativePath) ||
    MATERIAL_ROOTS.some((prefix) => relativePath.startsWith(prefix))
  );
}

export function materialBuildDigestFromEntries(entries) {
  const material = entries
    .filter((entry) => isMaterialBuildPath(entry.path))
    .sort((left, right) => left.path.localeCompare(right.path));
  return canonicalDigest(material);
}

function trackedFiles(repoRoot) {
  const output = execFileSync("git", ["ls-files", "-z"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return output.split("\0").filter(Boolean);
}

function trackedWorktreeStatus(repoRoot) {
  return execFileSync(
    "git",
    ["status", "--porcelain", "--untracked-files=no"],
    { cwd: repoRoot, encoding: "utf8" },
  );
}

function sourceWorktreeStatus(repoRoot) {
  return execFileSync(
    "git",
    ["status", "--porcelain", "--untracked-files=all"],
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
  assertString(
    buildIdentity.materialBuildDigest,
    `${label} materialBuildDigest`,
  );
  if (!DIGEST_PATTERN.test(buildIdentity.materialBuildDigest)) {
    fail(`${label} materialBuildDigest must be a sha256 digest`);
  }
  assertBoolean(
    buildIdentity.realExecutionEnabled,
    `${label} realExecutionEnabled`,
  );
  assertPositiveInteger(
    buildIdentity.qualificationContract,
    `${label} qualificationContract`,
  );
  return buildIdentity;
}

export function compareBuildIdentity(current, evidence) {
  validateBuildIdentity(current, "current build identity");
  validateBuildIdentity(evidence, "evidence build identity");
  return current.appVersion === evidence.appVersion &&
    current.materialBuildDigest === evidence.materialBuildDigest &&
    current.realExecutionEnabled === evidence.realExecutionEnabled &&
    current.qualificationContract === evidence.qualificationContract
    ? "compatible"
    : "invalidating";
}

export function buildMaterialIdentity({ repoRoot, requireClean = false }) {
  if (requireClean && sourceWorktreeStatus(repoRoot) !== "") {
    fail("device qualification requires a clean source worktree");
  }
  const packageJson = JSON.parse(
    readFileSync(path.join(repoRoot, "apps/emuchef-app/package.json"), "utf8"),
  );
  return {
    appVersion: packageJson.version,
    gitCommit: execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: repoRoot,
      encoding: "utf8",
    }).trim(),
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
  return (
    JSON.stringify(canonicalize(left)) === JSON.stringify(canonicalize(right))
  );
}

export function validateWorkflowCatalog(value) {
  assertExactKeys(value, ["schemaVersion", "workflows"], "workflow catalog");
  if (value.schemaVersion !== 1)
    fail("workflow catalog schemaVersion must be 1");
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
    assertStringArray(
      workflow.requiredCapabilities,
      "workflow requiredCapabilities",
    );
    assertStringArray(workflow.prerequisites, "workflow prerequisites");
    assertStringArray(
      workflow.compatibilityDimensions,
      "workflow compatibilityDimensions",
    );
    if (workflow.compatibilityDimensions.length === 0) {
      fail("workflow compatibilityDimensions must not be empty");
    }
    for (const dimension of workflow.compatibilityDimensions) {
      if (!COMPATIBILITY_DIMENSIONS.has(dimension)) {
        fail(`workflow compatibility dimension ${dimension} is not supported`);
      }
    }
    if (
      !Array.isArray(workflow.automatedObservations) ||
      workflow.automatedObservations.length === 0
    ) {
      fail("workflow automatedObservations must contain at least one entry");
    }
    const observationIds = new Set();
    for (const observation of workflow.automatedObservations) {
      assertExactKeys(
        observation,
        AUTOMATED_OBSERVATION_FIELDS,
        "automated observation",
      );
      assertString(observation.id, "automated observation id");
      if (observationIds.has(observation.id))
        fail(`duplicate automated observation id ${observation.id}`);
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
      if (checkpointIds.has(checkpoint.id))
        fail(`duplicate human checkpoint id ${checkpoint.id}`);
      checkpointIds.add(checkpoint.id);
      assertString(checkpoint.instruction, "human checkpoint instruction");
      assertString(checkpoint.fact, "human checkpoint fact");
      if (
        !Array.isArray(checkpoint.allowedOutcomes) ||
        checkpoint.allowedOutcomes.length === 0
      ) {
        fail(
          "human checkpoint allowedOutcomes must contain at least one outcome",
        );
      }
      for (const outcome of checkpoint.allowedOutcomes) {
        if (!CHECKPOINT_OUTCOME_SET.has(outcome))
          fail(`human checkpoint outcome ${outcome} is not supported`);
      }
      if (
        new Set(checkpoint.allowedOutcomes).size !==
        checkpoint.allowedOutcomes.length
      ) {
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
    for (const field of [
      "manufacturer",
      "model",
      "androidVersion",
      "abiSocClass",
      "firmwareBuild",
    ]) {
      assertFactWrapper(target, field, assertString);
    }
    assertFactWrapper(target, "androidApi", assertPositiveInteger);
    const rootState = assertFactWrapper(target, "rootState", assertString);
    if (!ROOT_STATES.has(rootState.value)) {
      fail(`device target rootState value ${rootState.value} is not supported`);
    }
    const connectionType = assertFactWrapper(
      target,
      "connectionType",
      assertString,
    );
    if (!CONNECTION_TYPES.has(connectionType.value)) {
      fail(
        `device target connectionType value ${connectionType.value} is not supported`,
      );
    }
    if (target.id !== deviceTargetId(target)) {
      fail("device target id does not match canonical target identity");
    }
    assertStringArray(target.capabilities, "device target capabilities");
    assertStringArray(
      target.deferredWorkflows,
      "device target deferredWorkflows",
    );
    const profilePath = path.join(
      authoredProfilesDir,
      `${targetFactValue(target, "profileId")}.yaml`,
    );
    if (!existsSync(profilePath)) {
      fail(
        `unknown authored device profile ${targetFactValue(target, "profileId")}`,
      );
    }
  }
  return value;
}

export function loadWorkflowCatalog(catalogPath) {
  return validateWorkflowCatalog(JSON.parse(readFileSync(catalogPath, "utf8")));
}

export function loadDeviceTargets(targetsPath, options) {
  return validateDeviceTargets(
    JSON.parse(readFileSync(targetsPath, "utf8")),
    options,
  );
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
  if (fingerprint.schemaVersion !== 2)
    fail("fingerprint schemaVersion must be 2");
  validateBuildIdentity(fingerprint.emuchefBuild, "fingerprint emuchefBuild");
  for (const field of [
    "runtimeContract",
    "deviceProfile",
    "firmwareBuild",
    "abiSocClass",
  ]) {
    assertString(fingerprint[field], `fingerprint ${field}`);
  }
  if (fingerprint.runtimeContract !== RUNTIME_CONTRACT) {
    fail(`fingerprint runtimeContract must be ${RUNTIME_CONTRACT}`);
  }
  assertPositiveInteger(
    fingerprint.workflowVersion,
    "fingerprint workflowVersion",
  );
  assertPositiveInteger(fingerprint.androidApi, "fingerprint androidApi");
  if (!ROOT_STATES.has(fingerprint.rootState))
    fail(`fingerprint rootState ${fingerprint.rootState} is not supported`);
  if (!CONNECTION_TYPES.has(fingerprint.connectionType)) {
    fail(
      `fingerprint connectionType ${fingerprint.connectionType} is not supported`,
    );
  }
  if (!Array.isArray(fingerprint.authoredContent)) {
    fail("fingerprint authoredContent must be an array");
  }
  const seenIds = new Set();
  for (const entry of fingerprint.authoredContent) {
    assertExactKeys(
      entry,
      AUTHORED_CONTENT_ENTRY_FIELDS,
      "authored content entry",
    );
    assertString(entry.id, "authored content entry id");
    if (seenIds.has(entry.id))
      fail(`duplicate authored content id ${entry.id}`);
    seenIds.add(entry.id);
    if (
      typeof entry.sha256 !== "string" ||
      !SHA256_PATTERN.test(entry.sha256)
    ) {
      fail(
        "authored content entry sha256 must be a 64-character lowercase hex digest",
      );
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
    return compareBuildIdentity(
      currentFingerprint.emuchefBuild,
      evidenceFingerprint.emuchefBuild,
    );
  }
  if (dimension === "authored_content") {
    return equalJson(
      currentFingerprint.authoredContent,
      evidenceFingerprint.authoredContent,
    )
      ? "compatible"
      : "invalidating";
  }
  return currentFingerprint[field] === evidenceFingerprint[field]
    ? "compatible"
    : "invalidating";
}

function buildEvidenceDeviceTarget(target) {
  return Object.fromEntries(
    EVIDENCE_DEVICE_TARGET_FIELDS.map((field) => [
      field,
      structuredClone(target[field]),
    ]),
  );
}

export function classifyCompatibility({
  workflow,
  currentFingerprint,
  evidenceFingerprint,
}) {
  for (const dimension of workflow.compatibilityDimensions) {
    if (
      compareDimension(dimension, currentFingerprint, evidenceFingerprint) ===
      "invalidating"
    ) {
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
  const declared = new Set(
    workflow.automatedObservations.map((observation) => observation.id),
  );
  if (!Array.isArray(record.automatedObservations)) {
    fail("automatedObservations must be an array");
  }
  const seen = new Set();
  for (const observation of record.automatedObservations) {
    assertExactKeys(
      observation,
      AUTOMATED_OBSERVATION_RECORD_FIELDS,
      "automated observation record",
    );
    assertString(observation.id, "automated observation record id");
    if (!declared.has(observation.id)) {
      fail(
        `automated observation id ${observation.id} is not declared by the workflow`,
      );
    }
    if (seen.has(observation.id))
      fail(`duplicate automated observation ${observation.id}`);
    seen.add(observation.id);
    if (!["passed", "failed"].includes(observation.outcome)) {
      fail(
        `automated observation ${observation.id} outcome ${observation.outcome} is not supported`,
      );
    }
    assertRfc3339(
      observation.observedAt,
      `automated observation ${observation.id} observedAt`,
    );
  }
  return seen;
}

function validateEvidenceArtifacts(record) {
  if (!Array.isArray(record.artifacts)) {
    fail("artifacts must be an array");
  }
  if (record.artifacts.length > 2) {
    fail("artifacts may contain at most two entries");
  }
  const seen = new Set();
  for (const artifact of record.artifacts) {
    assertExactKeys(artifact, EVIDENCE_ARTIFACT_FIELDS, "evidence artifact");
    assertString(artifact.id, "evidence artifact id");
    if (seen.has(artifact.id))
      fail(`duplicate evidence artifact ${artifact.id}`);
    seen.add(artifact.id);
    if (!SHA256_PATTERN.test(artifact.sha256)) {
      fail(
        `evidence artifact ${artifact.id} sha256 must be a 64-character lowercase hex digest`,
      );
    }
    const expected =
      artifact.id === EXECUTION_REPORT_ARTIFACT.id
        ? EXECUTION_REPORT_ARTIFACT
        : artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id
          ? ROM_MERGE_EVIDENCE_ARTIFACT
          : null;
    if (expected === null) {
      fail(`evidence artifact id ${artifact.id} is not supported`);
    }
    if (artifact.kind !== expected.kind) {
      fail(`evidence artifact kind ${artifact.kind} is not supported`);
    }
    if (artifact.path !== expected.path) {
      fail(`evidence artifact ${artifact.id} path must exactly ${expected.path}`);
    }
  }
  return (
    record.artifacts.find(
      (artifact) => artifact.id === EXECUTION_REPORT_ARTIFACT.id,
    ) ?? null
  );
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
    assertExactKeys(
      checkpoint,
      HUMAN_CHECKPOINT_RECORD_FIELDS,
      "human checkpoint record",
    );
    assertString(
      checkpoint.checkpointId,
      "human checkpoint record checkpointId",
    );
    const contract = declared.get(checkpoint.checkpointId);
    if (!contract) {
      fail(
        `human checkpoint id ${checkpoint.checkpointId} is not declared by the workflow`,
      );
    }
    if (seen.has(checkpoint.checkpointId))
      fail(`duplicate human checkpoint ${checkpoint.checkpointId}`);
    seen.add(checkpoint.checkpointId);
    if (!CHECKPOINT_OUTCOME_SET.has(checkpoint.outcome)) {
      fail(
        `human checkpoint ${checkpoint.checkpointId} outcome ${checkpoint.outcome} is not supported`,
      );
    }
    if (!contract.allowedOutcomes.includes(checkpoint.outcome)) {
      fail(
        `human checkpoint ${checkpoint.checkpointId} outcome ${checkpoint.outcome} is not allowed by its contract`,
      );
    }
    assertRfc3339(
      checkpoint.observedAt,
      `human checkpoint ${checkpoint.checkpointId} observedAt`,
    );
    if (
      contract.required &&
      checkpoint.outcome === "unable_to_verify" &&
      (record.runValidity !== "invalid" ||
        record.qualificationOutcome !== "not_observed")
    ) {
      fail(
        'required checkpoint unable_to_verify requires runValidity "invalid" and qualificationOutcome "not_observed"',
      );
    }
  }
  return seen;
}

export function validateEvidenceRecord(record, context) {
  assertExactKeys(record, EVIDENCE_RECORD_FIELDS, "evidence record");
  if (record.schemaVersion !== 2)
    fail("evidence record schemaVersion must be 2");
  if (!RUN_ID_PATTERN.test(record.runId))
    fail("evidence record runId format is invalid");
  if (!DIGEST_PATTERN.test(record.recordDigest))
    fail("evidence record recordDigest format is invalid");
  assertRfc3339(record.capturedAt, "capturedAt");
  assertString(record.workflowId, "workflowId");
  assertPositiveInteger(record.workflowVersion, "workflowVersion");
  const workflow = context.workflowCatalog.workflows.find(
    (candidate) => candidate.id === record.workflowId,
  );
  if (!workflow) fail(`unknown workflow id ${record.workflowId}`);
  if (record.workflowVersion !== workflow.version) {
    fail(
      `workflow version ${record.workflowVersion} does not match catalog version ${workflow.version}`,
    );
  }
  const target = context.targets.find(
    (candidate) => candidate.id === record.deviceTarget?.id,
  );
  if (!target) fail(`unknown device target id ${record.deviceTarget?.id}`);
  assertExactKeys(
    record.deviceTarget,
    EVIDENCE_DEVICE_TARGET_FIELDS,
    "evidence device target",
  );
  if (!TARGET_ID_PATTERN.test(record.deviceTarget.id)) {
    fail("evidence device target id format is invalid");
  }
  const observedFacts = new Map();
  for (const field of TARGET_FACT_FIELDS) {
    const validator =
      field === "androidApi" ? assertPositiveInteger : assertString;
    observedFacts.set(
      field,
      assertFactWrapper(record.deviceTarget, field, validator),
    );
  }
  if (!ROOT_STATES.has(observedFacts.get("rootState").value)) {
    fail(
      `evidence device target rootState value ${observedFacts.get("rootState").value} is not supported`,
    );
  }
  if (!CONNECTION_TYPES.has(observedFacts.get("connectionType").value)) {
    fail(
      `evidence device target connectionType value ${observedFacts.get("connectionType").value} is not supported`,
    );
  }
  assertEvidenceDeviceTarget(record.deviceTarget, target);
  validateFingerprint(record.fingerprint);
  if (record.fingerprint.workflowVersion !== workflow.version) {
    fail("fingerprint workflowVersion does not match the workflow version");
  }
  if (
    record.fingerprint.deviceProfile !== targetFactValue(target, "profileId")
  ) {
    fail("fingerprint deviceProfile does not match the registered target");
  }
  if (
    record.fingerprint.androidApi !== targetFactValue(target, "androidApi") ||
    record.fingerprint.firmwareBuild !==
      targetFactValue(target, "firmwareBuild") ||
    record.fingerprint.abiSocClass !== targetFactValue(target, "abiSocClass") ||
    record.fingerprint.rootState !== targetFactValue(target, "rootState") ||
    record.fingerprint.connectionType !==
      targetFactValue(target, "connectionType")
  ) {
    fail("fingerprint device facts do not match the registered target");
  }
  const actualRecipeIds = record.fingerprint.authoredContent.map(
    (entry) => entry.id,
  );
  if (
    JSON.stringify(actualRecipeIds) !==
    JSON.stringify(workflow.productionRecipes)
  ) {
    fail(
      "fingerprint authoredContent does not match the workflow production recipes",
    );
  }
  if (
    record.fingerprintDigest !== evidenceFingerprintDigest(record.fingerprint)
  ) {
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
    fail(
      `qualificationOutcome ${record.qualificationOutcome} is not supported`,
    );
  }
  if (!TARGET_WIDE_FAILURES.includes(record.targetWideFailure)) {
    fail(`targetWideFailure ${record.targetWideFailure} is not supported`);
  }
  if (
    !Array.isArray(record.limitations) ||
    record.limitations.some(
      (item) => typeof item !== "string" || item.length === 0,
    )
  ) {
    fail("limitations must be an array of non-empty strings");
  }
  const executionReportArtifact = validateEvidenceArtifacts(record);
  const hasRomMergeArtifact = record.artifacts.some(
    (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
  );
  if (
    hasRomMergeArtifact &&
    (record.runValidity !== "valid" ||
      record.qualificationOutcome !== "passed" ||
      workflow.id !== "rom-library-sync" ||
      record.deviceTarget.profileId.value !== "konkr.pocket_advance")
  ) {
    fail("ROM Merge artifact is not supported for this workflow and device profile");
  }
  validateAutomatedObservationRecords(record, workflow);
  validateHumanCheckpointRecords(record, workflow);
  const executionReportObservation = record.automatedObservations.find(
    (observation) => observation.id === EXECUTION_REPORT_ARTIFACT.id,
  );
  for (const checkpoint of record.humanCheckpoints) {
    if (
      PRE_EXECUTION_PREREQUISITE_CHECKPOINTS.has(checkpoint.checkpointId) &&
      checkpoint.outcome !== "pass" &&
      (record.runValidity !== "invalid" ||
        record.qualificationOutcome !== "not_observed")
    ) {
      fail(
        `pre-execution checkpoint ${checkpoint.checkpointId} requires an invalid not_observed record when it does not pass`,
      );
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
      fail(
        "invalid run cannot record an execution-report observation without an execution-report artifact",
      );
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
  if (
    record.targetWideFailure !== null &&
    record.qualificationOutcome !== "failed"
  ) {
    fail("targetWideFailure requires a failed valid record");
  }
  const observedIds = new Set(
    record.automatedObservations.map((observation) => observation.id),
  );
  for (const observation of workflow.automatedObservations) {
    if (observation.required && !observedIds.has(observation.id)) {
      fail(`missing required automated observation ${observation.id}`);
    }
  }
  const checkpointIds = new Set(
    record.humanCheckpoints.map((checkpoint) => checkpoint.checkpointId),
  );
  for (const checkpoint of workflow.humanCheckpoints) {
    if (checkpoint.required && !checkpointIds.has(checkpoint.id)) {
      fail(`missing required human checkpoint ${checkpoint.id}`);
    }
  }
  if (record.qualificationOutcome === "passed") {
    for (const observation of workflow.automatedObservations) {
      if (observation.required) {
        const result = record.automatedObservations.find(
          (item) => item.id === observation.id,
        );
        if (!result || result.outcome !== "passed") {
          fail(`required automated observation ${observation.id} did not pass`);
        }
      }
    }
    for (const checkpoint of workflow.humanCheckpoints) {
      if (checkpoint.required) {
        const result = record.humanCheckpoints.find(
          (item) => item.checkpointId === checkpoint.id,
        );
        if (!result || result.outcome !== "pass") {
          fail(`required human checkpoint ${checkpoint.id} did not pass`);
        }
      }
    }
    if (
      record.automatedObservations.some((item) => item.outcome === "failed") ||
      record.humanCheckpoints.some((item) => item.outcome === "fail")
    ) {
      fail("a passed record cannot contain failed observations or checkpoints");
    }
  } else {
    const hasFailure =
      record.automatedObservations.some((item) => item.outcome === "failed") ||
      record.humanCheckpoints.some((item) => item.outcome === "fail") ||
      record.targetWideFailure !== null;
    if (!hasFailure) {
      fail(
        "failed record has no failed observation, failed checkpoint, or target-wide failure",
      );
    }
  }
  return true;
}

function validateCanonicalExecutionReportBinding(report, record) {
  if (
    typeof report.plan.planId !== "string" ||
    typeof report.plan.planDigest !== "string" ||
    !SHA256_PATTERN.test(report.plan.planDigest)
  ) {
    fail("canonical execution report requires plan identity");
  }

  if (record.runValidity !== "valid") return;

  if (
    report.execution.simulated !== false ||
    report.execution.verificationScope !== "real_device"
  ) {
    fail(
      "valid canonical execution report requires simulated=false and real_device verification",
    );
  }

  const status = report.execution.status;
  const observationOutcome =
    status === "succeeded" || status === "succeeded_with_warnings"
      ? "passed"
      : "failed";
  const executionReportObservation = record.automatedObservations.find(
    (observation) => observation.id === EXECUTION_REPORT_ARTIFACT.id,
  );
  if (executionReportObservation?.outcome !== observationOutcome) {
    fail("execution report status does not match its recorded observation");
  }

  const expectedQualificationOutcome =
    status === "failed" ||
    status === "cancelled" ||
    record.targetWideFailure !== null ||
    record.humanCheckpoints.some((checkpoint) => checkpoint.outcome === "fail")
      ? "failed"
      : "passed";
  if (record.qualificationOutcome !== expectedQualificationOutcome) {
    fail("execution report status and checkpoints do not match qualificationOutcome");
  }
}

function validateScopedQualificationVeto(record, workflow) {
  if (
    record.runValidity === "valid" &&
    record.qualificationOutcome === "passed" &&
    workflow.id === "rom-library-sync" &&
    record.deviceTarget.profileId.value === "konkr.pocket_advance"
  ) {
    const hasRomMergeEvidence = record.artifacts.some(
      (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
    );
    if (!hasRomMergeEvidence) {
      fail("ROM Merge evidence is required for konkr.pocket_advance");
    }
  }
}

function validateRestrictedQualificationReport(report, record, workflow) {
  if (!Object.hasOwn(report, "qualification")) return;

  const qualification = report.qualification;
  if (report.execution.simulated !== false) {
    fail("restricted qualification execution must not be simulated");
  }
  if (qualification.observedRootState !== record.deviceTarget.rootState.value) {
    fail("restricted qualification observedRootState does not match the evidence target");
  }
  if (
    qualification.strategy !== "adb_unprivileged" ||
    qualification.restriction !== "privileged_execution_disabled"
  ) {
    fail("restricted qualification strategy or restriction is unsupported");
  }
  const capabilities = new Set(qualification.effectiveCapabilities);
  if (capabilities.has("root_shell") || capabilities.has("app_data_write")) {
    fail("restricted qualification includes a privileged effective capability");
  }
  const runtimeRestriction = report.runtime.executionRestriction;
  if (
    report.runtime.status !== "ready" ||
    runtimeRestriction === undefined ||
    runtimeRestriction.enabled !== true ||
    runtimeRestriction.strategy !== "adb_shell" ||
    runtimeRestriction.rootShell !== false ||
    runtimeRestriction.appDataWrite !== false
  ) {
    fail("restricted qualification requires the verified runtime restriction");
  }

  if (!equalJson(runtimeRestriction.buildIdentity, qualification.buildIdentity)) {
    fail("restricted qualification runtime build identity does not match its claim");
  }
  if (!qualification.buildIdentity.realExecutionEnabled) {
    fail("restricted qualification build must enable real execution");
  }
  for (const field of BUILD_IDENTITY_FIELDS) {
    if (
      qualification.buildIdentity[field] !==
      record.fingerprint.emuchefBuild[field]
    ) {
      fail(`restricted qualification buildIdentity ${field} does not match the evidence record`);
    }
  }
  if (report.app.version !== qualification.buildIdentity.appVersion) {
    fail("restricted qualification app version does not match its build identity");
  }
  if (qualification.deviceTargetId !== record.deviceTarget.id) {
    fail("restricted qualification deviceTargetId does not match the evidence record");
  }
  if (qualification.deviceProfileId !== record.fingerprint.deviceProfile) {
    fail("restricted qualification deviceProfileId does not match the evidence record");
  }

  if (qualification.reviewedPlanId !== report.plan.planId) {
    fail("restricted qualification reviewedPlanId does not match the report plan");
  }
  if (qualification.reviewedPlanDigest !== report.plan.planDigest) {
    fail("restricted qualification reviewedPlanDigest does not match the report plan");
  }

  const requiredOperationIds = qualification.requiredOperationIds;
  const requiredIds = new Set(requiredOperationIds);
  if (
    JSON.stringify(requiredOperationIds) !==
    JSON.stringify(workflow.productionRecipes)
  ) {
    fail(
      "restricted qualification requiredOperationIds do not match the workflow productionRecipes",
    );
  }
  const outcomeIds = new Set();
  for (const outcome of qualification.operationOutcomes) {
    if (!requiredIds.has(outcome.operationId)) {
      fail(`unexpected restricted qualification operation ${outcome.operationId}`);
    }
    if (outcomeIds.has(outcome.operationId)) {
      fail(`duplicate restricted qualification outcome ${outcome.operationId}`);
    }
    if (outcome.outcome === "executed") {
      const matchingRecipes = report.execution.recipes.filter(
        (candidate) => candidate?.recipeId === outcome.operationId,
      );
      const recipe = matchingRecipes[0];
      const hasSuccessfulStep = Array.isArray(recipe?.steps) && recipe.steps.some(
        (step) =>
          step?.recipeId === outcome.operationId &&
          typeof step.stepId === "string" &&
          step.stepId.length > 0 &&
          step.status === "succeeded",
      );
      if (
        matchingRecipes.length !== 1 ||
        !["succeeded", "succeeded_with_warnings"].includes(recipe.status) ||
        !hasSuccessfulStep
      ) {
        fail(
          `restricted qualification operation ${outcome.operationId} has no matching successful step result`,
        );
      }
    }
    if (
      record.qualificationOutcome === "passed" &&
      outcome.outcome !== "executed"
    ) {
      fail(`passed restricted qualification operation ${outcome.operationId} was not executed`);
    }
    outcomeIds.add(outcome.operationId);
  }
  if (outcomeIds.size !== requiredIds.size) {
    fail("restricted qualification is missing a required operation outcome");
  }
}

function loadRomMergeApkPin(repoRoot) {
  const pinPath = path.join(repoRoot, ROM_MERGE_PIN_PATH);
  if (!assertRegularCandidateFile(pinPath, "RetroArch APK pin manifest", { required: false })) {
    fail("ROM Merge APK pin file is missing from the repository-controlled pin location");
  }
  let manifest;
  try {
    manifest = JSON.parse(
      readRegularCandidateFile(pinPath, "RetroArch APK pin manifest", "utf8"),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      fail("RetroArch APK pin manifest is malformed");
    }
    throw error;
  }
  assertExactKeys(manifest, ["schemaVersion", "retroarch"], "RetroArch APK pin manifest");
  if (manifest.schemaVersion !== 1) {
    fail("RetroArch APK pin manifest schemaVersion must be 1");
  }
  if (manifest.retroarch === null) {
    fail("ROM Merge promotion is disabled because the repository APK pin is null");
  }
  assertExactKeys(
    manifest.retroarch,
    ["packageId", "source", "version", "sha256"],
    "RetroArch APK pin",
  );
  const pin = manifest.retroarch;
  if (pin.packageId !== "com.retroarch.aarch64") {
    fail("RetroArch APK pin packageId is unsupported");
  }
  assertString(pin.source, "RetroArch APK pin source");
  if (!pin.source.startsWith("https://")) {
    fail("RetroArch APK pin source must use HTTPS");
  }
  assertString(pin.version, "RetroArch APK pin version");
  if (typeof pin.sha256 !== "string" || !SHA256_PATTERN.test(pin.sha256)) {
    fail("RetroArch APK pin sha256 must be lowercase 64-character hex");
  }
  return pin;
}

function validateRomMergeManifestEntries(entries, label, { nonEmpty = false } = {}) {
  if (!Array.isArray(entries) || (nonEmpty && entries.length === 0)) {
    fail(`${label} must be ${nonEmpty ? "a non-empty" : "an"} array of manifest entries`);
  }
  const byPath = new Map();
  for (const entry of entries) {
    assertExactKeys(entry, ["relativePath", "sha256"], `${label} entry`);
    assertString(entry.relativePath, `${label} relativePath`);
    if (
      entry.relativePath.includes("\\") ||
      entry.relativePath.includes("\0") ||
      entry.relativePath.endsWith("/") ||
      path.posix.isAbsolute(entry.relativePath) ||
      path.posix.normalize(entry.relativePath) !== entry.relativePath ||
      entry.relativePath.split("/").some((part) => part === "" || part === "." || part === "..")
    ) {
      fail(`${label} relativePath must be a normalized relative POSIX file path`);
    }
    if (typeof entry.sha256 !== "string" || !SHA256_PATTERN.test(entry.sha256)) {
      fail(`${label} sha256 must be lowercase 64-character hex`);
    }
    if (byPath.has(entry.relativePath)) {
      fail(`${label} contains duplicate path ${entry.relativePath}`);
    }
    byPath.set(entry.relativePath, entry.sha256);
  }
  return byPath;
}

function validateRomMergeTerminalOperation(report, evidence) {
  assertExactKeys(
    evidence.terminalOperation,
    ["recipeId", "stepId", "outcome"],
    "ROM Merge terminalOperation",
  );
  if (
    evidence.terminalOperation.recipeId !== "feature.copy_roms" ||
    evidence.terminalOperation.stepId !== "feature.copy_roms/copy_rom_library" ||
    evidence.terminalOperation.outcome !== "succeeded"
  ) {
    fail("ROM Merge terminal operation must be the successful copy_rom_library step");
  }
  const recipes = report.execution.recipes.filter(
    (recipe) => recipe?.recipeId === "feature.copy_roms",
  );
  const matchingSteps = recipes.flatMap((recipe) =>
    (Array.isArray(recipe.steps) ? recipe.steps : []).filter(
      (step) =>
        step?.recipeId === "feature.copy_roms" &&
        step.stepId === "feature.copy_roms/copy_rom_library",
    ),
  );
  if (
    recipes.length !== 1 ||
    matchingSteps.length !== 1 ||
    matchingSteps[0].status !== "succeeded" ||
    !["succeeded", "succeeded_with_warnings"].includes(recipes[0].status)
  ) {
    fail("ROM Merge report must contain one successful terminal copy_rom_library step");
  }
}

function validateRomMergeCopyResult(evidence) {
  assertExactKeys(evidence.sourceSnapshot, ["sourceIdentity", "entries"], "ROM Merge sourceSnapshot");
  assertString(evidence.sourceSnapshot.sourceIdentity, "ROM Merge sourceIdentity");
  const source = validateRomMergeManifestEntries(
    evidence.sourceSnapshot.entries,
    "ROM Merge sourceSnapshot entries",
    { nonEmpty: true },
  );

  assertExactKeys(
    evidence.destination,
    ["path", "setupLocation", "volumeIdentity", "before", "after"],
    "ROM Merge destination",
  );
  assertString(evidence.destination.path, "ROM Merge destination path");
  assertString(evidence.destination.setupLocation, "ROM Merge destination setupLocation");
  assertString(evidence.destination.volumeIdentity, "ROM Merge destination volumeIdentity");
  const before = validateRomMergeManifestEntries(
    evidence.destination.before,
    "ROM Merge destination before",
  );
  const after = validateRomMergeManifestEntries(
    evidence.destination.after,
    "ROM Merge destination after",
  );

  for (const [relativePath, sha256] of source) {
    if (after.get(relativePath) !== sha256) {
      fail(`ROM Merge result is missing source entry ${relativePath}`);
    }
  }
  for (const [relativePath, sha256] of before) {
    if (!source.has(relativePath) && after.get(relativePath) !== sha256) {
      fail(`ROM Merge result did not preserve destination entry ${relativePath}`);
    }
  }
  for (const relativePath of after.keys()) {
    if (!source.has(relativePath) && !before.has(relativePath)) {
      fail(`ROM Merge result contains an unexpected destination entry ${relativePath}`);
    }
  }

  assertExactKeys(evidence.sentinel, ["relativePath", "expectedSha256"], "ROM Merge sentinel");
  if (evidence.sentinel.relativePath !== ROM_MERGE_SENTINEL_PATH) {
    fail("ROM Merge sentinel path must be qualification-sentinel.keep");
  }
  if (
    typeof evidence.sentinel.expectedSha256 !== "string" ||
    !SHA256_PATTERN.test(evidence.sentinel.expectedSha256)
  ) {
    fail("ROM Merge sentinel expectedSha256 must be lowercase 64-character hex");
  }
  if (
    source.has(ROM_MERGE_SENTINEL_PATH) ||
    before.get(ROM_MERGE_SENTINEL_PATH) !== evidence.sentinel.expectedSha256 ||
    after.get(ROM_MERGE_SENTINEL_PATH) !== evidence.sentinel.expectedSha256
  ) {
    fail("ROM Merge sentinel must be preserved unchanged and absent from the source");
  }
}

/** Bind the retained manifest to the report, registered device, candidate, and approved APK pin. */
function validateRomMergeEvidenceBytes({
  bytes,
  artifact,
  reportBytes,
  report,
  record,
  workflow,
  context,
  candidate,
  repoRoot,
  allowHistoricalPin = false,
}) {
  if (!(bytes instanceof Buffer)) {
    fail("ROM Merge evidence artifact requires retained rom-merge-evidence.json bytes");
  }
  if (bytes.length === 0 || bytes.length > MAX_ROM_MERGE_EVIDENCE_BYTES) {
    fail("ROM Merge evidence bytes are outside the supported size bound");
  }
  if (createHash("sha256").update(bytes).digest("hex") !== artifact.sha256) {
    fail("ROM Merge evidence digest does not match the bound artifact sha256");
  }
  let evidenceText;
  try {
    evidenceText = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    fail("ROM Merge evidence is not valid UTF-8");
  }
  let evidence;
  try {
    evidence = JSON.parse(evidenceText);
  } catch (error) {
    if (error instanceof SyntaxError) fail("ROM Merge evidence JSON is malformed");
    throw error;
  }
  assertExactKeys(evidence, ROM_MERGE_EVIDENCE_FIELDS, "ROM Merge evidence");
  if (evidence.schemaVersion !== 1) {
    fail("ROM Merge evidence schemaVersion must be 1");
  }
  if (!report.qualification || !report.runtime.executionRestriction) {
    fail("ROM Merge evidence requires the restricted execution-report claim");
  }
  if (!CANDIDATE_ID_PATTERN.test(evidence.candidateHandle)) {
    fail("ROM Merge evidence candidateHandle is invalid");
  }
  if (!ROM_MERGE_SESSION_ID_PATTERN.test(evidence.sessionId)) {
    fail("ROM Merge evidence sessionId is invalid");
  }
  if (
    typeof evidence.executionReportSha256 !== "string" ||
    !SHA256_PATTERN.test(evidence.executionReportSha256) ||
    evidence.executionReportSha256 !== createHash("sha256").update(reportBytes).digest("hex") ||
    evidence.executionReportSha256 !==
      record.artifacts.find((item) => item.id === EXECUTION_REPORT_ARTIFACT.id).sha256
  ) {
    fail("ROM Merge evidence does not match the retained execution report digest");
  }

  const target = context.targets.find((item) => item.id === record.deviceTarget.id);
  const fingerprint = record.fingerprint;
  const qualification = report.qualification;
  if (
    workflow.id !== "rom-library-sync" ||
    record.deviceTarget.profileId.value !== "konkr.pocket_advance" ||
    targetFactValue(target, "profileId") !== "konkr.pocket_advance"
  ) {
    fail("ROM Merge artifact is not supported for this workflow and device profile");
  }
  assertStringArray(
    qualification.effectiveCapabilities,
    "ROM Merge effectiveCapabilities",
  );
  const effectiveCapabilities = new Set(qualification.effectiveCapabilities);
  if (
    effectiveCapabilities.has("root_shell") ||
    effectiveCapabilities.has("app_data_write")
  ) {
    fail("ROM Merge evidence includes an unsupported privileged capability");
  }
  if (
    qualification.effectiveCapabilities.some(
      (capability) => !ROM_MERGE_UNPRIVILEGED_CAPABILITIES.has(capability),
    )
  ) {
    fail("ROM Merge evidence includes an unsupported effective capability");
  }
  if (
    qualification.effectiveCapabilities.some(
      (capability) => !target.capabilities.includes(capability),
    )
  ) {
    fail("ROM Merge evidence includes an unsupported capability");
  }
  if (
    (workflow.requiredCapabilities ?? []).some(
      (capability) => !effectiveCapabilities.has(capability),
    )
  ) {
    fail("ROM Merge evidence omits a required workflow capability");
  }
  if (
    evidence.observedRootState !== fingerprint.rootState ||
    evidence.observedRootState !== record.deviceTarget.rootState.value ||
    evidence.observedRootState !== qualification.observedRootState
  ) {
    fail("ROM Merge evidence observedRootState does not match the candidate and execution report");
  }
  if (
    evidence.candidateHandle !== qualification.attemptId ||
    evidence.deviceTargetId !== record.deviceTarget.id ||
    evidence.deviceTargetId !== qualification.deviceTargetId ||
    evidence.deviceProfileId !== fingerprint.deviceProfile ||
    evidence.deviceProfileId !== qualification.deviceProfileId ||
    evidence.deviceProfileId !== targetFactValue(target, "profileId") ||
    evidence.androidApi !== fingerprint.androidApi ||
    evidence.androidApi !== targetFactValue(target, "androidApi") ||
    evidence.firmwareBuild !== fingerprint.firmwareBuild ||
    evidence.firmwareBuild !== targetFactValue(target, "firmwareBuild") ||
    evidence.abiSocClass !== fingerprint.abiSocClass ||
    evidence.abiSocClass !== targetFactValue(target, "abiSocClass") ||
    evidence.connectionType !== fingerprint.connectionType ||
    evidence.connectionType !== targetFactValue(target, "connectionType")
  ) {
    fail("ROM Merge evidence device and candidate bindings do not match the report and registered target");
  }
  if (candidate !== undefined) {
    if (
      candidate.candidateSchemaVersion !== 2 ||
      candidate.candidateId !== evidence.candidateHandle ||
      candidate.sessionId !== evidence.sessionId
    ) {
      fail("ROM Merge evidence candidateHandle or sessionId does not match the v2 candidate payload");
    }
    for (const [field, expected] of Object.entries({
      deviceTargetId: candidate.deviceTargetId,
      deviceProfileId: candidate.fingerprint.deviceProfile,
      androidApi: candidate.fingerprint.androidApi,
      firmwareBuild: candidate.fingerprint.firmwareBuild,
      abiSocClass: candidate.fingerprint.abiSocClass,
      connectionType: candidate.fingerprint.connectionType,
      observedRootState: candidate.fingerprint.rootState,
    })) {
      if (evidence[field] !== expected) {
        fail(`ROM Merge evidence ${field} does not match the v2 candidate payload`);
      }
    }
    if (!equalJson(evidence.buildIdentity, candidate.build)) {
      fail("ROM Merge evidence buildIdentity does not match the candidate payload");
    }
  }
  validateBuildIdentity(evidence.buildIdentity, "ROM Merge evidence buildIdentity");
  if (
    !equalJson(evidence.buildIdentity, fingerprint.emuchefBuild) ||
    !equalJson(evidence.buildIdentity, qualification.buildIdentity) ||
    !equalJson(evidence.buildIdentity, report.runtime.executionRestriction.buildIdentity)
  ) {
    fail("ROM Merge evidence buildIdentity does not match the report and evidence record");
  }

  assertString(evidence.reviewedPlanId, "ROM Merge reviewedPlanId");
  if (
    typeof evidence.reviewedPlanDigest !== "string" ||
    !SHA256_PATTERN.test(evidence.reviewedPlanDigest) ||
    evidence.reviewedPlanId !== report.plan.planId ||
    evidence.reviewedPlanId !== qualification.reviewedPlanId ||
    evidence.reviewedPlanDigest !== report.plan.planDigest ||
    evidence.reviewedPlanDigest !== qualification.reviewedPlanDigest
  ) {
    fail("ROM Merge reviewed plan binding does not match the execution report");
  }
  if (evidence.copyPolicy !== "merge") {
    fail("ROM Merge evidence copyPolicy must be merge");
  }
  const expectedStrategyDigest = canonicalDigest({
    domain: ROM_MERGE_STRATEGY_DOMAIN,
    version: 1,
    strategy: qualification.strategy,
    restriction: qualification.restriction,
    runtimeRestriction: report.runtime.executionRestriction,
    effectiveCapabilities: qualification.effectiveCapabilities,
  });
  if (evidence.strategyDigest !== expectedStrategyDigest) {
    fail("ROM Merge evidence strategyDigest does not match the report restriction and capabilities");
  }
  validateRomMergeTerminalOperation(report, evidence);
  validateRomMergeCopyResult(evidence);

  assertExactKeys(
    evidence.retroArch,
    [
      "packageId",
      "installedVersion",
      "pinnedSource",
      "pinnedVersion",
      "installedApkSha256",
      "pinnedArtifactSha256",
    ],
    "ROM Merge RetroArch evidence",
  );
  const recordedPin = evidence.retroArch;
  if (
    recordedPin.packageId !== "com.retroarch.aarch64" ||
    typeof recordedPin.pinnedSource !== "string" ||
    !recordedPin.pinnedSource.startsWith("https://") ||
    typeof recordedPin.pinnedVersion !== "string" ||
    recordedPin.pinnedVersion.length === 0 ||
    typeof recordedPin.installedVersion !== "string" ||
    recordedPin.installedVersion !== recordedPin.pinnedVersion ||
    typeof recordedPin.pinnedArtifactSha256 !== "string" ||
    !SHA256_PATTERN.test(recordedPin.pinnedArtifactSha256) ||
    typeof recordedPin.installedApkSha256 !== "string" ||
    recordedPin.installedApkSha256 !== recordedPin.pinnedArtifactSha256
  ) {
    fail("ROM Merge RetroArch evidence has an inconsistent recorded APK identity");
  }
  let matchesCurrentPin = false;
  try {
    const pin = loadRomMergeApkPin(repoRoot);
    matchesCurrentPin =
      evidence.retroArch.packageId === pin.packageId &&
      evidence.retroArch.pinnedSource === pin.source &&
      evidence.retroArch.pinnedVersion === pin.version &&
      evidence.retroArch.installedVersion === pin.version &&
      evidence.retroArch.installedApkSha256 === pin.sha256 &&
      evidence.retroArch.pinnedArtifactSha256 === pin.sha256;
  } catch (error) {
    if (!allowHistoricalPin) throw error;
    // A current pin that cannot be loaded affects eligibility, not archive integrity.
  }
  if (!matchesCurrentPin && !allowHistoricalPin) {
    fail("ROM Merge RetroArch evidence does not match the repository-approved APK pin");
  }

  for (const field of ["rootAuthorityUsed", "oemCompilerUsed", "oemStageUsed"]) {
    if (evidence[field] !== false) {
      fail(`ROM Merge evidence ${field} must be false`);
    }
  }
  return matchesCurrentPin;
}

function validateEvidenceBundleInternal(
  bundle,
  context,
  {
    allowHistoricalV1 = false,
    allowHistoricalRomMergePin = false,
    repoRoot = REPO_ROOT,
    candidate,
  } = {},
) {
  assertObject(bundle, "evidence bundle");
  const {
    record,
    reportBytes = null,
    romMergeEvidenceBytes = null,
  } = bundle;
  validateEvidenceRecord(record, context);
  const executionReportArtifact =
    record.artifacts.find(
      (artifact) => artifact.id === EXECUTION_REPORT_ARTIFACT.id,
    ) ?? null;
  const romMergeEvidenceArtifact =
    record.artifacts.find(
      (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
    ) ?? null;
  if (romMergeEvidenceArtifact === null && romMergeEvidenceBytes !== null) {
    fail("bundle includes ROM Merge evidence bytes without a referenced artifact");
  }
  if (executionReportArtifact === null) {
    if (reportBytes !== null) {
      fail(
        "bundle must not include execution report bytes when no execution-report artifact is referenced",
      );
    }
    if (romMergeEvidenceArtifact !== null) {
      fail("ROM Merge evidence artifact requires a referenced execution-report artifact");
    }
    if (
      record.runValidity !== "invalid" ||
      record.qualificationOutcome !== "not_observed"
    ) {
      fail(
        "only invalid not_observed evidence may omit an execution-report artifact",
      );
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
  const workflow = context.workflowCatalog.workflows.find(
    (candidate) => candidate.id === record.workflowId,
  );
  const report = validateExecutionReportForInventory(reportBytes, {
    allowHistoricalV1,
  });
  if (report === null) {
    if (romMergeEvidenceArtifact !== null) {
      fail("ROM Merge evidence requires a current execution-report v2");
    }
    validateScopedQualificationVeto(record, workflow);
    return false;
  }
  validateScopedQualificationVeto(record, workflow);
  validateCanonicalExecutionReportBinding(report, record);
  validateRestrictedQualificationReport(report, record, workflow);
  let matchesCurrentRomMergePin = true;
  if (romMergeEvidenceArtifact !== null) {
    matchesCurrentRomMergePin = validateRomMergeEvidenceBytes({
      bytes: romMergeEvidenceBytes,
      artifact: romMergeEvidenceArtifact,
      reportBytes,
      report,
      record,
      workflow,
      context,
      candidate: candidate ?? bundle.candidate,
      repoRoot,
      allowHistoricalPin: allowHistoricalRomMergePin,
    });
  }
  return matchesCurrentRomMergePin;
}

/**
 * Validate evidence for current qualification use against the approved APK pin.
 * Historical archive inspection follows a separate path that preserves old records.
 */
export function validateEvidenceBundle(
  bundle,
  context,
  { allowHistoricalV1 = false, repoRoot = REPO_ROOT, candidate } = {},
) {
  validateEvidenceBundleInternal(bundle, context, {
    allowHistoricalV1,
    repoRoot,
    candidate,
  });
  return true;
}

function validateBuildIdentitySchemaContract(schema) {
  const definition = schema.$defs?.qualificationBuildIdentity;
  assertObject(definition, "evidence schema qualificationBuildIdentity");
  if (definition.additionalProperties !== false) {
    fail(
      "evidence schema qualificationBuildIdentity must reject additional properties",
    );
  }
  if (!equalJson(definition.required, BUILD_IDENTITY_FIELDS)) {
    fail(
      "evidence schema qualificationBuildIdentity required fields drifted from the validator",
    );
  }
  assertObject(
    definition.properties,
    "evidence schema qualificationBuildIdentity properties",
  );
  if (!equalJson(Object.keys(definition.properties), BUILD_IDENTITY_FIELDS)) {
    fail(
      "evidence schema qualificationBuildIdentity properties drifted from the validator",
    );
  }
  if (
    !equalJson(definition.properties.appVersion, {
      type: "string",
      minLength: 1,
    })
  ) {
    fail(
      "evidence schema qualificationBuildIdentity appVersion drifted from the validator",
    );
  }
  if (
    !equalJson(definition.properties.gitCommit, {
      type: "string",
      pattern: GIT_COMMIT_PATTERN.source,
    })
  ) {
    fail(
      "evidence schema qualificationBuildIdentity gitCommit drifted from the validator",
    );
  }
  if (
    !equalJson(definition.properties.materialBuildDigest, {
      type: "string",
      pattern: DIGEST_PATTERN.source,
    })
  ) {
    fail(
      "evidence schema qualificationBuildIdentity materialBuildDigest drifted from the validator",
    );
  }
  if (
    !equalJson(definition.properties.realExecutionEnabled, { type: "boolean" })
  ) {
    fail(
      "evidence schema qualificationBuildIdentity realExecutionEnabled drifted from the validator",
    );
  }
  if (
    !equalJson(definition.properties.qualificationContract, {
      type: "integer",
      minimum: 1,
    })
  ) {
    fail(
      "evidence schema qualificationBuildIdentity qualificationContract drifted from the validator",
    );
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
  if (
    !equalJson(
      schema.properties.qualificationOutcome.enum,
      QUALIFICATION_OUTCOMES,
    )
  ) {
    fail(
      "evidence schema qualificationOutcome enum drifted from the validator",
    );
  }
  if (
    !equalJson(
      schema.properties.targetWideFailure.oneOf[1].enum,
      TARGET_WIDE_FAILURES.filter((value) => value !== null),
    )
  ) {
    fail("evidence schema targetWideFailure enum drifted from the validator");
  }
  if (!equalJson(schema.$defs.fingerprint.required, FINGERPRINT_FIELDS)) {
    fail("evidence schema fingerprint fields drifted from the validator");
  }
  validateBuildIdentitySchemaContract(schema);
  if (
    !equalJson(
      schema.$defs.deviceTarget.required,
      EVIDENCE_DEVICE_TARGET_FIELDS,
    )
  ) {
    fail("evidence schema deviceTarget fields drifted from the validator");
  }
  if (
    !equalJson(
      schema.$defs.automatedObservation.required,
      AUTOMATED_OBSERVATION_RECORD_FIELDS,
    )
  ) {
    fail(
      "evidence schema automated observation fields drifted from the validator",
    );
  }
  if (
    !equalJson(
      schema.$defs.humanCheckpoint.required,
      HUMAN_CHECKPOINT_RECORD_FIELDS,
    )
  ) {
    fail("evidence schema human checkpoint fields drifted from the validator");
  }
  if (
    !equalJson(schema.$defs.automatedObservation.properties.outcome.enum, [
      "passed",
      "failed",
    ])
  ) {
    fail(
      "evidence schema automated observation outcomes drifted from the validator",
    );
  }
  if (
    !equalJson(
      schema.$defs.humanCheckpoint.properties.outcome.enum,
      CHECKPOINT_OUTCOMES,
    )
  ) {
    fail(
      "evidence schema human checkpoint outcomes drifted from the validator",
    );
  }
  if (
    !equalJson(schema.properties.artifacts, {
      type: "array",
      maxItems: 2,
      items: { $ref: "#/$defs/evidenceArtifact" },
    })
  ) {
    fail("evidence schema artifacts maxItems or item reference drifted from the validator");
  }
  const expectedArtifactAlternatives = [
    EXECUTION_REPORT_ARTIFACT,
    ROM_MERGE_EVIDENCE_ARTIFACT,
  ].map(({ id, kind, path: artifactPath }) => ({
    type: "object",
    additionalProperties: false,
    required: EVIDENCE_ARTIFACT_FIELDS,
    properties: {
      id: { const: id },
      kind: { const: kind },
      path: { const: artifactPath },
      sha256: { type: "string", pattern: "^[0-9a-f]{64}$" },
    },
  }));
  if (
    !equalJson(
      schema.$defs.evidenceArtifact,
      { oneOf: expectedArtifactAlternatives },
    )
  ) {
    fail("evidence schema artifact alternatives drifted from the validator");
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
    return {
      state: "not_applicable",
      reason: `missing_capabilities:${missing.join(",")}`,
    };
  }
  return { state: "required", reason: "production_intent_and_capabilities" };
}

function byDateThenId(left, right) {
  return (
    String(left.capturedAt).localeCompare(String(right.capturedAt)) ||
    String(left.runId).localeCompare(String(right.runId))
  );
}

export function selectCurrentEvidence({
  workflow,
  target,
  currentFingerprint,
  records,
}) {
  const eligible = records
    .filter(
      (record) =>
        record.deviceTarget?.id === target.id &&
        record.workflowId === workflow.id &&
        record.workflowVersion === workflow.version &&
        record.runValidity === "valid" &&
        classifyCompatibility({
          workflow,
          currentFingerprint,
          evidenceFingerprint: record.fingerprint,
        }) === "compatible",
    )
    .sort(byDateThenId);
  return eligible.length > 0 ? eligible[eligible.length - 1] : null;
}

function newestHistoricalRecord({ workflow, target, records }) {
  return (
    records
      .filter(
        (record) =>
          record.deviceTarget?.id === target.id &&
          record.workflowId === workflow.id &&
          record.workflowVersion === workflow.version &&
          record.runValidity === "valid",
      )
      .sort(byDateThenId)
      .at(-1) ?? null
  );
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
      reason:
        current.limitations.length > 0 ? current.limitations.join("; ") : null,
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
      reason:
        "no current compatible evidence; historical valid evidence exists",
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

export function deriveDeviceSupportTier(workflowStates, profileId = null) {
  if (workflowStates.some((row) => row.targetWideFailure !== null))
    return "unqualified";
  const required = workflowStates.filter(
    (row) => row.applicability === "required",
  );
  if (required.length === 0) return "unqualified";
  if (required.every((row) => row.state === "qualified")) {
    return profileId === "konkr.pocket_advance" ? "limited" : "qualified";
  }
  if (required.some((row) => row.state === "qualified")) return "limited";
  return "unqualified";
}

function qualificationPaths(repoRoot) {
  const authoredProfilesDir = path.join(repoRoot, "authored/device_profiles");
  return {
    repoRoot,
    authoredProfilesDir,
    workflowCatalogPath: path.join(
      repoRoot,
      "docs/testing/device-qualification/workflow-catalog.json",
    ),
    romMergeApkPinPath: path.join(repoRoot, ROM_MERGE_PIN_PATH),
    deviceTargetsPath: path.join(
      repoRoot,
      "docs/testing/device-qualification/device-targets.json",
    ),
    evidenceRoot: path.join(
      repoRoot,
      "docs/testing/device-qualification/evidence",
    ),
    matrixPath: path.join(
      repoRoot,
      "docs/qualification/device-qualification-matrix.md",
    ),
    candidateRoot: path.join(
      repoRoot,
      ".emuchef_runtime/qualification-candidates",
    ),
  };
}

function candidateDirectory(candidateId, repoRoot = REPO_ROOT) {
  if (!CANDIDATE_ID_PATTERN.test(candidateId))
    fail("qualification candidate id is invalid");
  return path.join(qualificationPaths(repoRoot).candidateRoot, candidateId);
}

function lstatIfPresent(targetPath) {
  try {
    return lstatSync(targetPath);
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

function assertCandidateRootIsSafe(repoRoot) {
  const { candidateRoot } = qualificationPaths(repoRoot);
  const resolvedRepoRoot = path.resolve(repoRoot);
  const resolvedCandidateRoot = path.resolve(candidateRoot);
  const relative = path.relative(resolvedRepoRoot, resolvedCandidateRoot);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    fail("qualification candidate root is outside the repository");
  }
  const repoMetadata = lstatIfPresent(resolvedRepoRoot);
  if (repoMetadata?.isSymbolicLink())
    fail("qualification repository root is a symlink");
  if (repoMetadata !== null && !repoMetadata?.isDirectory()) {
    fail("qualification repository root is not a directory");
  }
  let current = resolvedRepoRoot;
  for (const component of relative ? relative.split(path.sep) : []) {
    current = path.join(current, component);
    const metadata = lstatIfPresent(current);
    if (metadata?.isSymbolicLink())
      fail("qualification candidate root contains a symlink");
    if (metadata !== null && !metadata.isDirectory()) {
      fail("qualification candidate root contains a non-directory");
    }
    if (metadata === null) break;
  }
}

function assertRegularCandidateFile(filePath, label, { required = true } = {}) {
  const metadata = lstatIfPresent(filePath);
  if (metadata === null) {
    if (required) fail(`${label} does not exist`);
    return false;
  }
  if (metadata.isSymbolicLink()) fail(`${label} is a symlink`);
  if (!metadata.isFile()) fail(`${label} is not a regular file`);
  return true;
}

function readRegularCandidateFile(filePath, label, encoding = null) {
  assertRegularCandidateFile(filePath, label);
  const noFollow = fsConstants.O_NOFOLLOW ?? 0;
  let descriptor;
  try {
    descriptor = openSync(filePath, fsConstants.O_RDONLY | noFollow);
    const metadata = fstatSync(descriptor);
    if (!metadata.isFile()) fail(`${label} is not a regular file`);
    return readFileSync(descriptor, encoding ?? undefined);
  } catch (error) {
    if (error?.code === "ELOOP") fail(`${label} is a symlink`);
    throw error;
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function validateCandidateReportMetadata(report, reportBytes) {
  if (report === null) {
    if (reportBytes !== null)
      fail("qualification execution report is not declared by the candidate");
    return;
  }
  assertExactKeys(
    report,
    CANDIDATE_REPORT_FIELDS,
    "qualification candidate report metadata",
  );
  if (report.path !== EXECUTION_REPORT_ARTIFACT.path) {
    fail("qualification candidate report path must be execution-report.json");
  }
  if (!Number.isSafeInteger(report.byteLength) || report.byteLength < 0) {
    fail(
      "qualification candidate report byteLength must be a non-negative integer",
    );
  }
  if (
    typeof report.sha256 !== "string" ||
    !SHA256_PATTERN.test(report.sha256)
  ) {
    fail("qualification candidate report sha256 is invalid");
  }
  if (reportBytes === null) fail("qualification execution report is missing");
  if (report.byteLength !== reportBytes.length)
    fail("qualification execution report byte length does not match metadata");
  const actualSha256 = createHash("sha256").update(reportBytes).digest("hex");
  if (report.sha256 !== actualSha256)
    fail("qualification execution report digest does not match metadata");
}

function loadCandidateJson(candidateId, repoRoot = REPO_ROOT) {
  const directory = candidateDirectory(candidateId, repoRoot);
  assertCandidateRootIsSafe(repoRoot);
  const directoryMetadata = lstatIfPresent(directory);
  if (directoryMetadata?.isSymbolicLink())
    fail("qualification candidate directory is a symlink");
  if (directoryMetadata === null) {
    fail(`qualification candidate ${candidateId} does not exist`);
  }
  if (!directoryMetadata.isDirectory())
    fail("qualification candidate directory is invalid");
  const candidatePath = path.join(directory, "candidate.json");
  assertRegularCandidateFile(candidatePath, "qualification candidate file");
  const envelope = JSON.parse(
    readRegularCandidateFile(
      candidatePath,
      "qualification candidate file",
      "utf8",
    ),
  );
  assertObject(envelope, "qualification candidate envelope");
  const envelopeFields = Object.hasOwn(envelope, "romMergeEvidence")
    ? [...CANDIDATE_ENVELOPE_FIELDS, "romMergeEvidence"]
    : CANDIDATE_ENVELOPE_FIELDS;
  assertExactKeys(envelope, envelopeFields, "qualification candidate envelope");
  if (envelope.candidateHandle !== candidateId) {
    fail("qualification candidate envelope does not match its directory id");
  }
  if (!["target_registration", "qualification_run"].includes(envelope.kind)) {
    fail("qualification candidate envelope kind is invalid");
  }
  assertObject(envelope.payload, "qualification candidate payload");
  const candidate = envelope.payload;
  if (candidate.candidateId !== candidateId) {
    fail("qualification candidate payload does not match its directory id");
  }
  if (candidate.kind !== envelope.kind)
    fail("qualification candidate kind metadata is inconsistent");
  if (!equalJson(envelope.capturedAt, candidate.capturedAt ?? null)) {
    fail("qualification candidate capturedAt metadata is inconsistent");
  }
  if (!equalJson(envelope.build, candidate.build ?? null)) {
    fail("qualification candidate build metadata is inconsistent");
  }
  const reportPath = path.join(directory, EXECUTION_REPORT_ARTIFACT.path);
  const reportExists = assertRegularCandidateFile(
    reportPath,
    "qualification execution report",
    { required: false },
  );
  const reportBytes = reportExists
    ? readRegularCandidateFile(reportPath, "qualification execution report")
    : null;
  validateCandidateReportMetadata(envelope.report, reportBytes);
  const romMergeEvidencePath = path.join(
    directory,
    ROM_MERGE_EVIDENCE_ARTIFACT.path,
  );
  const romMergeEvidenceExists = assertRegularCandidateFile(
    romMergeEvidencePath,
    "ROM Merge evidence",
    { required: false },
  );
  const romMergeEvidenceMetadata = envelope.romMergeEvidence ?? null;
  const romMergeEvidenceDeclared = romMergeEvidenceMetadata !== null;
  let romMergeEvidenceBytes = null;
  if (!romMergeEvidenceDeclared) {
    if (romMergeEvidenceExists) {
      fail("ROM Merge evidence file is not declared by candidate metadata");
    }
  } else {
    assertExactKeys(
      romMergeEvidenceMetadata,
      ["path", "byteLength", "sha256"],
      "ROM Merge evidence metadata",
    );
    if (romMergeEvidenceMetadata.path !== ROM_MERGE_EVIDENCE_ARTIFACT.path) {
      fail("ROM Merge evidence metadata path must be rom-merge-evidence.json");
    }
    if (
      !Number.isSafeInteger(romMergeEvidenceMetadata.byteLength) ||
      romMergeEvidenceMetadata.byteLength < 1 ||
      romMergeEvidenceMetadata.byteLength > MAX_ROM_MERGE_EVIDENCE_BYTES
    ) {
      fail("ROM Merge evidence metadata byteLength is outside the supported size bound");
    }
    if (
      typeof romMergeEvidenceMetadata.sha256 !== "string" ||
      !SHA256_PATTERN.test(romMergeEvidenceMetadata.sha256)
    ) {
      fail("ROM Merge evidence metadata sha256 must be lowercase 64-character hex");
    }
    if (!romMergeEvidenceExists) {
      fail("ROM Merge evidence file is missing from candidate directory");
    }
    const evidenceFileMetadata = lstatIfPresent(romMergeEvidencePath);
    if (evidenceFileMetadata.size > MAX_ROM_MERGE_EVIDENCE_BYTES) {
      fail("ROM Merge evidence file exceeds the supported size bound");
    }
    romMergeEvidenceBytes = readRegularCandidateFile(
      romMergeEvidencePath,
      "ROM Merge evidence",
    );
    if (romMergeEvidenceBytes.length !== romMergeEvidenceMetadata.byteLength) {
      fail("ROM Merge evidence byte length does not match candidate metadata");
    }
    const actualSha256 = createHash("sha256")
      .update(romMergeEvidenceBytes)
      .digest("hex");
    if (actualSha256 !== romMergeEvidenceMetadata.sha256) {
      fail("ROM Merge evidence digest does not match candidate metadata");
    }
  }
  return {
    directory,
    candidate,
    reportBytes,
    romMergeEvidenceBytes,
    romMergeEvidenceMetadata,
  };
}

function assertCandidateMarkersAllowPromotion(
  candidateId,
  repoRoot = REPO_ROOT,
) {
  const directory = candidateDirectory(candidateId, repoRoot);
  assertCandidateRootIsSafe(repoRoot);
  const directoryMetadata = lstatIfPresent(directory);
  if (directoryMetadata?.isSymbolicLink())
    fail("qualification candidate directory is a symlink");
  if (directoryMetadata === null || !directoryMetadata.isDirectory()) {
    fail("qualification candidate directory is invalid");
  }
  for (const marker of ["session-poison.json", "session-audit-only"]) {
    const markerPath = path.join(directory, marker);
    if (
      assertRegularCandidateFile(
        markerPath,
        "qualification non-promotable marker",
        { required: false },
      )
    ) {
      fail("qualification candidate is marked non-promotable");
    }
  }
}

function validateCandidateEnvelope(candidate, { kind, fields }) {
  assertObject(candidate, `${kind} candidate`);
  if (kind === "qualification_run" && candidate.candidateSchemaVersion === 2) {
    assertExactKeys(candidate, [...fields, "sessionId"], `${kind} candidate`);
    assertString(candidate.sessionId, `${kind} candidate sessionId`);
    if (!ROM_MERGE_SESSION_ID_PATTERN.test(candidate.sessionId)) {
      fail(`${kind} candidate sessionId is invalid`);
    }
  } else {
    assertExactKeys(candidate, fields, `${kind} candidate`);
    if (candidate.candidateSchemaVersion !== 1) {
      fail(`${kind} candidate schema version must be 1 or supported version 2`);
    }
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
    fail(
      `qualification_run candidate runValidity ${candidate.runValidity} is not supported`,
    );
  }
  if (!QUALIFICATION_OUTCOMES.includes(candidate.qualificationOutcome)) {
    fail(
      `qualification_run candidate qualificationOutcome ${candidate.qualificationOutcome} is not supported`,
    );
  }
  if (!TARGET_WIDE_FAILURES.includes(candidate.targetWideFailure)) {
    fail(
      `qualification_run candidate targetWideFailure ${candidate.targetWideFailure} is not supported`,
    );
  }
  if (
    !Array.isArray(candidate.limitations) ||
    candidate.limitations.some(
      (item) => typeof item !== "string" || item.length === 0,
    )
  ) {
    fail(
      "qualification_run candidate limitations must be an array of non-empty strings",
    );
  }
  validateEvidenceArtifacts(candidate);
  const hasRomMergeArtifact = candidate.artifacts.some(
    (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
  );
  if (
    hasRomMergeArtifact &&
    (candidate.candidateSchemaVersion !== 2 ||
      candidate.workflowId !== "rom-library-sync" ||
      candidate.fingerprint.deviceProfile !== "konkr.pocket_advance" ||
      candidate.runValidity !== "valid" ||
      candidate.qualificationOutcome !== "passed")
  ) {
    fail("ROM Merge artifact is not supported for this workflow and device profile");
  }
  return candidate;
}

function validatePromotionBuildState(candidateBuild, repoRoot) {
  if (trackedWorktreeStatus(repoRoot) !== "") {
    fail("device qualification requires a clean tracked worktree");
  }
  const currentCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).trim();
  if (candidateBuild.gitCommit !== currentCommit) {
    fail("qualification candidate git commit no longer matches HEAD");
  }
  const currentBuild = buildMaterialIdentity({ repoRoot, requireClean: false });
  if (candidateBuild.materialBuildDigest !== currentBuild.materialBuildDigest) {
    fail(
      "qualification candidate material build digest no longer matches the current repository state",
    );
  }
  if (candidateBuild.qualificationContract !== QUALIFICATION_CONTRACT_VERSION) {
    fail(
      `qualification candidate qualificationContract must be ${QUALIFICATION_CONTRACT_VERSION}`,
    );
  }
  if (candidateBuild.realExecutionEnabled !== true) {
    fail(
      "qualification candidate must be captured from a real-execution-enabled build",
    );
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

function renderMatrixForState({
  workflowCatalog,
  targets,
  records,
  currentBuild,
  runtimeContract,
  authoredContentDigests,
}) {
  return renderQualificationMatrix(
    projectQualificationState({
      workflowCatalog,
      targets,
      records,
      currentBuild,
      runtimeContract,
      authoredContentDigests,
    }),
  );
}

function reserveCreateNewDirectory(targetPath, fsOps) {
  fsOps.mkdirSync(path.dirname(targetPath), { recursive: true });
  try {
    fsOps.mkdirSync(targetPath);
  } catch (error) {
    if (error?.code === "EEXIST" || error?.code === "ENOTEMPTY") {
      fail(
        `canonical evidence directory ${path.basename(targetPath)} already exists`,
      );
    }
    throw error;
  }
}

function currentQualificationContext(repoRoot) {
  const paths = qualificationPaths(repoRoot);
  const workflowCatalog = loadWorkflowCatalog(paths.workflowCatalogPath);
  const targetFile = loadDeviceTargets(paths.deviceTargetsPath, {
    authoredProfilesDir: paths.authoredProfilesDir,
  });
  return {
    paths,
    workflowCatalog,
    targets: targetFile.targets,
    authoredContentDigests: productionAuthoredContentDigests(
      workflowCatalog,
      repoRoot,
    ),
  };
}

export function loadEvidenceBundle(bundlePath) {
  const resolved = path.resolve(bundlePath);
  if (!existsSync(resolved)) {
    fail(`evidence bundle ${resolved} does not exist`);
  }
  const evidencePath = path.join(resolved, "evidence.json");
  if (!assertRegularCandidateFile(evidencePath, "canonical evidence record", { required: false })) {
    fail(`evidence bundle ${resolved} must contain evidence.json`);
  }
  const record = JSON.parse(
    readRegularCandidateFile(evidencePath, "canonical evidence record", "utf8"),
  );
  const reportPath = path.join(resolved, EXECUTION_REPORT_ARTIFACT.path);
  const reportExists = assertRegularCandidateFile(
    reportPath,
    "canonical execution report",
    { required: false },
  );
  const romMergeEvidencePath = path.join(
    resolved,
    ROM_MERGE_EVIDENCE_ARTIFACT.path,
  );
  const romMergeEvidenceExists = assertRegularCandidateFile(
    romMergeEvidencePath,
    "canonical ROM Merge evidence",
    { required: false },
  );
  if (
    romMergeEvidenceExists &&
    lstatIfPresent(romMergeEvidencePath).size > MAX_ROM_MERGE_EVIDENCE_BYTES
  ) {
    fail("canonical ROM Merge evidence exceeds the supported size bound");
  }
  return {
    record,
    reportBytes: reportExists
      ? readRegularCandidateFile(reportPath, "canonical execution report")
      : null,
    romMergeEvidenceBytes: romMergeEvidenceExists
      ? readRegularCandidateFile(
          romMergeEvidencePath,
          "canonical ROM Merge evidence",
        )
      : null,
  };
}

function loadEvidenceBundlesInDirectory(evidenceDir, { fixtureMode }) {
  const resolved = path.resolve(evidenceDir);
  const fixtureRoot = path.resolve(REPO_ROOT, "tests/fixtures");
  if (
    !fixtureMode &&
    (resolved === fixtureRoot ||
      resolved.startsWith(`${fixtureRoot}${path.sep}`))
  ) {
    fail("synthetic fixture path cannot be used as production evidence");
  }
  if (!existsSync(resolved)) return [];
  const bundles = [];
  const seenRunIds = new Set();
  for (const entry of readdirSync(resolved, { withFileTypes: true }).sort(
    (left, right) => left.name.localeCompare(right.name),
  )) {
    if (entry.name === "README.md") continue;
    let bundle;
    if (entry.isDirectory()) {
      bundle = loadEvidenceBundle(path.join(resolved, entry.name));
    } else if (fixtureMode && entry.isFile() && entry.name.endsWith(".json")) {
      bundle = {
        record: JSON.parse(
          readFileSync(path.join(resolved, entry.name), "utf8"),
        ),
        reportBytes: null,
      };
    } else {
      continue;
    }
    const record = bundle.record;
    if (seenRunIds.has(record.runId))
      fail(`duplicate run identity ${record.runId}`);
    seenRunIds.add(record.runId);
    bundles.push(bundle);
  }
  return bundles;
}

/**
 * Return validated v2 records for the current eligibility projection.
 *
 * Canonical v1 reports remain historical after their record identity and
 * artifact digest pass validation.
 */
export function loadValidatedEvidenceRecords(
  evidenceDir,
  { fixtureMode, workflowCatalog, targets, repoRoot = REPO_ROOT },
) {
  const bundles = loadEvidenceBundlesInDirectory(evidenceDir, { fixtureMode });
  const records = [];
  for (const bundle of bundles) {
    const isCurrent = validateEvidenceBundleInternal(
      bundle,
      { workflowCatalog, targets },
      {
        allowHistoricalV1: fixtureMode === false,
        allowHistoricalRomMergePin: fixtureMode === false,
        repoRoot,
      },
    );
    if (isCurrent) records.push(bundle.record);
  }
  return records;
}

/**
 * Load archived evidence for inspection, including records bound to an older APK pin.
 * Use loadValidatedEvidenceRecords to select records eligible for current qualification.
 */
export function loadEvidenceDirectory(
  evidenceDir,
  {
    fixtureMode,
    workflowCatalog,
    targets,
    repoRoot = REPO_ROOT,
  },
) {
  const bundles = loadEvidenceBundlesInDirectory(evidenceDir, { fixtureMode });
  const requiresRomMergeValidation = bundles.some((bundle) => {
    const record = bundle.record;
    return (
      bundle.romMergeEvidenceBytes != null ||
      record.artifacts?.some(
        (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
      ) ||
      (record.workflowId === "rom-library-sync" &&
        record.runValidity === "valid" &&
        record.qualificationOutcome === "passed" &&
        record.deviceTarget?.profileId?.value === "konkr.pocket_advance")
    );
  });
  if (!requiresRomMergeValidation) return bundles.map((bundle) => bundle.record);
  const context =
    workflowCatalog && targets
      ? { workflowCatalog, targets }
      : currentQualificationContext(repoRoot);
  for (const bundle of bundles) {
    validateEvidenceBundleInternal(bundle, context, {
      allowHistoricalV1: fixtureMode === false,
      allowHistoricalRomMergePin: fixtureMode === false,
      repoRoot,
    });
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
      const rows = workflowCatalog.workflows.map((workflow) =>
        deriveWorkflowState({
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
        }),
      );
      return {
        target,
          tier: deriveDeviceSupportTier(rows, target.profileId.value),
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
    lines.push(
      `| ${row.workflowId} | ${row.state} | ${evidence} | ${reason} |`,
    );
  }
  return lines.join("\n");
}

export function renderQualificationMatrix(projection) {
  const lines = [
    "# Device Qualification Matrix",
    "",
    "Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.",
    "",
  ];
  if (projection.targets.length === 0) {
    lines.push(
      "No physical-device qualification targets have been registered yet. The production-bound harness is available for future physical runs, but no device or workflow is physically qualified by this repository state.",
    );
    lines.push("");
    return lines.join("\n");
  }
  for (const entry of projection.targets) {
    lines.push(renderTargetSection(entry), "");
  }
  return lines.join("\n");
}

function productionAuthoredContentDigests(
  workflowCatalog,
  repoRoot = REPO_ROOT,
) {
  const digests = {};
  for (const workflow of workflowCatalog.workflows) {
    for (const recipeId of workflow.productionRecipes) {
      const recipePath = path.join(
        repoRoot,
        "authored/recipes",
        `${recipeId}.yaml`,
      );
      digests[recipeId] = createHash("sha256")
        .update(readFileSync(recipePath))
        .digest("hex");
    }
  }
  return digests;
}

export function registerQualificationTargetCandidate(
  candidateId,
  { repoRoot = REPO_ROOT, fsOps = defaultFsOps } = {},
) {
  const { candidate, romMergeEvidenceBytes, romMergeEvidenceMetadata } =
    loadCandidateJson(candidateId, repoRoot);
  if (romMergeEvidenceMetadata !== null || romMergeEvidenceBytes !== null) {
    fail("target registration candidates cannot carry ROM Merge evidence");
  }
  const target = validateTargetRegistrationCandidate(candidate, repoRoot);
  const paths = qualificationPaths(repoRoot);
  const targetFile = loadDeviceTargets(paths.deviceTargetsPath, {
    authoredProfilesDir: paths.authoredProfilesDir,
  });
  if (targetFile.targets.some((entry) => entry.id === target.id)) {
    fail(`canonical device target ${target.id} already exists`);
  }
  const currentBuild = validatePromotionBuildState(candidate.build, repoRoot);
  const workflowCatalog = loadWorkflowCatalog(paths.workflowCatalogPath);
  const authoredContentDigests = productionAuthoredContentDigests(
    workflowCatalog,
    repoRoot,
  );
  const existingRecords = loadValidatedEvidenceRecords(paths.evidenceRoot, {
    fixtureMode: false,
    workflowCatalog,
    targets: targetFile.targets,
    repoRoot,
  });
  const nextTargets = {
    schemaVersion: targetFile.schemaVersion,
    targets: [...targetFile.targets, target],
  };
  const matrixText = renderMatrixForState({
    workflowCatalog,
    targets: nextTargets.targets,
    records: existingRecords,
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
  const tempMatrixPath = writeTemporaryFile(
    paths.matrixPath,
    matrixText,
    fsOps,
  );
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

export function recordQualificationRunCandidate(
  candidateId,
  { repoRoot = REPO_ROOT, fsOps = defaultFsOps } = {},
) {
  const {
    candidate,
    reportBytes,
    romMergeEvidenceBytes,
    romMergeEvidenceMetadata,
  } = loadCandidateJson(candidateId, repoRoot);
  assertCandidateMarkersAllowPromotion(candidateId, repoRoot);
  validateQualificationRunCandidate(candidate);
  const artifacts = structuredClone(candidate.artifacts);
  const referencedRomMergeEvidence = artifacts.find(
    (artifact) => artifact.id === ROM_MERGE_EVIDENCE_ARTIFACT.id,
  );
  if (romMergeEvidenceMetadata !== null) {
    if (candidate.candidateSchemaVersion !== 2 || reportBytes === null) {
      fail("ROM Merge evidence requires a v2 qualification candidate and retained execution report");
    }
    if (referencedRomMergeEvidence) {
      fail("ROM Merge evidence must be declared through candidate metadata only");
    }
    artifacts.push({
      ...ROM_MERGE_EVIDENCE_ARTIFACT,
      sha256: romMergeEvidenceMetadata.sha256,
    });
  } else if (referencedRomMergeEvidence) {
    fail("ROM Merge evidence artifact requires Rust-owned candidate metadata");
  }
  const { paths, workflowCatalog, targets, authoredContentDigests } =
    currentQualificationContext(repoRoot);
  const workflow = workflowCatalog.workflows.find(
    (item) => item.id === candidate.workflowId,
  );
  if (!workflow) fail(`unknown workflow id ${candidate.workflowId}`);
  if (candidate.workflowVersion !== workflow.version) {
    fail(
      `qualification candidate workflow version ${candidate.workflowVersion} does not match catalog version ${workflow.version}`,
    );
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
    artifacts,
  });
  const existingRecords = loadValidatedEvidenceRecords(paths.evidenceRoot, {
    fixtureMode: false,
    workflowCatalog,
    targets,
    repoRoot,
  });
  if (
    existingRecords.some((record) => record.runId === provisionalRecord.runId)
  ) {
    fail(
      `canonical evidence directory ${provisionalRecord.runId} already exists`,
    );
  }
  const currentBuild = validatePromotionBuildState(candidate.build, repoRoot);
  const expectedFingerprint = buildCurrentFingerprint({
    workflow,
    target,
    currentBuild,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests,
  });
  if (!equalJson(candidate.fingerprint, expectedFingerprint)) {
    fail(
      "qualification candidate fingerprint no longer matches the current canonical workflow, target, and authored content",
    );
  }
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
    artifacts,
  });
  validateEvidenceBundle(
    {
      record: sealedRecord,
      reportBytes,
      romMergeEvidenceBytes,
      candidate,
    },
    { workflowCatalog, targets },
    { repoRoot },
  );
  const bundleDir = path.join(paths.evidenceRoot, sealedRecord.runId);
  const matrixText = renderMatrixForState({
    workflowCatalog,
    targets,
    records: [...existingRecords, sealedRecord],
    currentBuild,
    runtimeContract: RUNTIME_CONTRACT,
    authoredContentDigests,
  });
  let tempMatrixPath = null;
  let reservedBundleDir = false;
  try {
    reserveCreateNewDirectory(bundleDir, fsOps);
    reservedBundleDir = true;
    fsOps.writeFileSync(
      path.join(bundleDir, "evidence.json"),
      `${JSON.stringify(sealedRecord, null, 2)}\n`,
      "utf8",
    );
    if (reportBytes !== null) {
      fsOps.writeFileSync(
        path.join(bundleDir, EXECUTION_REPORT_ARTIFACT.path),
        reportBytes,
      );
    }
    if (romMergeEvidenceBytes !== null) {
      fsOps.writeFileSync(
        path.join(bundleDir, ROM_MERGE_EVIDENCE_ARTIFACT.path),
        romMergeEvidenceBytes,
      );
    }
    tempMatrixPath = writeTemporaryFile(paths.matrixPath, matrixText, fsOps);
    fsOps.renameSync(tempMatrixPath, paths.matrixPath);
  } catch (error) {
    if (reservedBundleDir) {
      fsOps.rmSync(bundleDir, { recursive: true, force: true });
    }
    if (tempMatrixPath !== null) {
      fsOps.rmSync(tempMatrixPath, { recursive: true, force: true });
    }
    throw error;
  }
  return sealedRecord;
}

function projectProductionQualification() {
  const workflowCatalog = loadWorkflowCatalog(
    path.join(QUALIFICATION_ROOT, "workflow-catalog.json"),
  );
  const targetsFile = loadDeviceTargets(
    path.join(QUALIFICATION_ROOT, "device-targets.json"),
    { authoredProfilesDir: path.join(REPO_ROOT, "authored/device_profiles") },
  );
  const schema = JSON.parse(
    readFileSync(path.join(QUALIFICATION_ROOT, "evidence-schema.json"), "utf8"),
  );
  validateEvidenceSchemaContract(schema);
  return projectQualificationState({
    workflowCatalog,
    targets: targetsFile.targets,
    records: loadValidatedEvidenceRecords(EVIDENCE_ROOT, {
      fixtureMode: false,
      workflowCatalog,
      targets: targetsFile.targets,
    }),
    currentBuild: buildMaterialIdentity({
      repoRoot: REPO_ROOT,
      requireClean: false,
    }),
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
    workflowCatalog: loadWorkflowCatalog(
      path.join(QUALIFICATION_ROOT, "workflow-catalog.json"),
    ),
    deviceTargets: loadDeviceTargets(
      path.join(QUALIFICATION_ROOT, "device-targets.json"),
      { authoredProfilesDir: path.join(REPO_ROOT, "authored/device_profiles") },
    ),
  };
}

function qualificationOperationResult(
  operation,
  candidateHandle,
  candidateKind,
  payload,
) {
  return {
    operation,
    candidateHandle,
    candidateKind,
    payload,
  };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  const args = process.argv.slice(2);
  const candidateActions = new Set(["--register-target", "--record-run"]);
  const allowedArgs = new Set([
    "--build-identity",
    "--require-clean",
    "--describe",
    "--check",
    "--write-matrix",
    ...candidateActions,
  ]);
  const actionArgs = args.filter((arg) =>
    [
      "--build-identity",
      "--describe",
      "--check",
      "--write-matrix",
      ...candidateActions,
    ].includes(arg),
  );
  const registerTargetIndex = args.indexOf("--register-target");
  const recordRunIndex = args.indexOf("--record-run");
  const candidateValueIndexes = new Set();
  if (registerTargetIndex !== -1)
    candidateValueIndexes.add(registerTargetIndex + 1);
  if (recordRunIndex !== -1) candidateValueIndexes.add(recordRunIndex + 1);
  if (
    args.some(
      (arg, index) =>
        !allowedArgs.has(arg) && !candidateValueIndexes.has(index),
    ) ||
    actionArgs.length !== 1 ||
    (args.includes("--require-clean") && !args.includes("--build-identity")) ||
    (registerTargetIndex !== -1 && args.length !== 2) ||
    (recordRunIndex !== -1 && args.length !== 2)
  ) {
    process.stderr.write(
      "usage: node tools/device-qualification.mjs [--build-identity [--require-clean]|--describe|--check|--write-matrix|--register-target <candidate-id>|--record-run <candidate-id>]\n",
    );
    process.exitCode = 1;
  } else {
    try {
      if (args.includes("--build-identity")) {
        process.stdout.write(
          `${JSON.stringify(
            buildMaterialIdentity({
              repoRoot: REPO_ROOT,
              requireClean: args.includes("--require-clean"),
            }),
          )}\n`,
        );
      } else if (args.includes("--describe")) {
        process.stdout.write(`${JSON.stringify(repositoryDescription())}\n`);
      } else if (registerTargetIndex !== -1) {
        const candidateHandle = args[registerTargetIndex + 1];
        const target = registerQualificationTargetCandidate(candidateHandle, {
          repoRoot: REPO_ROOT,
        });
        process.stdout.write(
          `${JSON.stringify(
            qualificationOperationResult(
              "register_target",
              candidateHandle,
              "target_registration",
              target,
            ),
          )}\n`,
        );
      } else if (recordRunIndex !== -1) {
        const candidateHandle = args[recordRunIndex + 1];
        const record = recordQualificationRunCandidate(candidateHandle, {
          repoRoot: REPO_ROOT,
        });
        process.stdout.write(
          `${JSON.stringify(
            qualificationOperationResult(
              "record_run",
              candidateHandle,
              "qualification_run",
              record,
            ),
          )}\n`,
        );
      } else {
        const projection = projectProductionQualification();
        const rendered = renderQualificationMatrix(projection);
        const matrixPath = path.join(
          REPO_ROOT,
          "docs/qualification/device-qualification-matrix.md",
        );
        if (args.includes("--write-matrix")) {
          writeFileSync(matrixPath, rendered, "utf8");
        }
        const committed = readFileSync(matrixPath, "utf8");
        if (committed !== rendered) {
          fail(
            "generated device qualification matrix is out of date; run --write-matrix",
          );
        }
        process.stdout.write("Device qualification check passed.\n");
      }
    } catch (error) {
      process.stderr.write(
        `${error instanceof Error ? error.message : String(error)}\n`,
      );
      process.exitCode = 1;
    }
  }
}

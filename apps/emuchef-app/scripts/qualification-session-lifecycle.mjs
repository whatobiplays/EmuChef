import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  acquireSidecarBuildLock,
  qualificationStateMarkerPath,
  sidecarBuildLockPath,
  pathEntryPresent,
} from "./sidecar-build-contract.mjs";
import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";
import { verifyQualificationSidecar } from "./verify-qualification-sidecar.mjs";

export const CONFIG_CREATE_SESSION_FILENAME = "retroarch-config-create-qualification-session.json";
const MANIFEST_PATH =
  "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json";
const EXPECTED_BACKEND_FEATURES = [
  "real-execution",
  "android-copy-commit-helper",
];

function digest(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function parseRecord(bytes, label = "configuration-create qualification session") {
  let record;
  try {
    record = JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${label} is malformed: ${error.message}`);
  }
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    throw new Error(`${label} is malformed`);
  }
  return record;
}

function validateProcessOwner(owner, label) {
  if (
    !owner ||
    !Number.isSafeInteger(owner.pid) ||
    owner.pid <= 0 ||
    typeof owner.host !== "string" ||
    owner.host.length === 0
  ) {
    throw new Error(`${label} process identity is incomplete`);
  }
}

function processState(owner, label) {
  validateProcessOwner(owner, label);
  if (owner.host !== os.hostname()) {
    throw new Error(`${label} process host is not available for verification`);
  }
  try {
    process.kill(owner.pid, 0);
    return "alive";
  } catch (error) {
    if (error.code === "ESRCH") return "ended";
    if (error.code === "EPERM") return "alive";
    throw new Error(`${label} process state is uncertain: ${error.message}`);
  }
}

function assertSafeSessionPath(sessionPath) {
  const metadata = fs.lstatSync(sessionPath);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error("configuration-create session lease is not a regular file");
  }
}

function parseSessionRecord(bytes) {
  const record = parseRecord(bytes);
  if (
    record.schemaVersion !== 1 ||
    typeof record.qualificationTargetId !== "string" ||
    typeof record.token !== "string" ||
    !record.token ||
    !new Set([
      "launching",
      "verified",
      "active",
      "ended-cleanly",
      "cancelled-before-start",
    ]).has(record.status)
  ) {
    throw new Error("configuration-create session lease has an unsupported or incomplete record");
  }
  validateProcessOwner(record.launchOwner, "configuration-create launch owner");
  if (record.appProcess !== null) {
    validateProcessOwner(record.appProcess, "configuration-create application");
  }
  if (
    (new Set(["launching", "verified", "cancelled-before-start"]).has(
      record.status,
    ) &&
      record.appProcess !== null) ||
    (new Set(["active", "ended-cleanly"]).has(record.status) &&
      record.appProcess === null)
  ) {
    throw new Error("configuration-create session lease process state is inconsistent");
  }
  return { bytes, record };
}

function readSession(sessionPath) {
  assertSafeSessionPath(sessionPath);
  return parseSessionRecord(fs.readFileSync(sessionPath));
}

function writeRecordAtomically(sessionPath, record) {
  const temporaryPath = `${sessionPath}.tmp-${process.pid}-${randomUUID()}`;
  const bytes = Buffer.from(`${JSON.stringify(record, null, 2)}\n`);
  fs.writeFileSync(temporaryPath, bytes, { flag: "wx", mode: 0o600 });
  try {
    fs.renameSync(temporaryPath, sessionPath);
  } catch (error) {
    fs.rmSync(temporaryPath, { force: true });
    throw error;
  }
}

/** Publish the first lease with an atomic no-clobber link. */
function writeInitialSessionLease(sessionPath, record) {
  const temporaryPath = `${sessionPath}.tmp-${process.pid}-${randomUUID()}`;
  const bytes = Buffer.from(`${JSON.stringify(record, null, 2)}\n`);
  let published = false;
  let operationError;

  try {
    fs.writeFileSync(temporaryPath, bytes, { flag: "wx", mode: 0o600 });
    const descriptor = fs.openSync(temporaryPath, "r");
    try {
      const metadata = fs.fstatSync(descriptor);
      if (!metadata.isFile()) {
        throw new Error("configuration-create session lease staging entry is not a regular file");
      }
      fs.fsyncSync(descriptor);
    } finally {
      fs.closeSync(descriptor);
    }

    fs.linkSync(temporaryPath, sessionPath);
    published = true;
  } catch (error) {
    operationError = error;
  }

  let cleanupError;
  try {
    fs.unlinkSync(temporaryPath);
  } catch (error) {
    if (error.code !== "ENOENT") cleanupError = error;
  }

  if (operationError && cleanupError) {
    throw new AggregateError(
      [operationError, cleanupError],
      "configuration-create session lease publication failed and its staging entry remains",
    );
  }
  if (cleanupError) {
    const message = published
      ? "configuration-create session lease was published but its staging entry remains"
      : "configuration-create session lease staging entry could not be removed";
    throw new Error(message, { cause: cleanupError });
  }
  if (operationError) throw operationError;
}

function acquireLifecycleLock({
  repoRoot,
  targetId,
  lockPath,
}) {
  return acquireSidecarBuildLock({
    repoRoot,
    profile: "qualification-lifecycle",
    targetId,
    artifactPaths: [MANIFEST_PATH],
    lockPath,
  });
}

/** Return the session lease path beside this worktree's qualification marker. */
export function qualificationSessionLeasePath(
  repoRoot,
  markerPath = qualificationStateMarkerPath(repoRoot),
) {
  return path.join(path.dirname(markerPath), CONFIG_CREATE_SESSION_FILENAME);
}

/** Treat dangling or symbolic lease entries as present so they cannot bypass a build guard. */
export function qualificationSessionLeasePresent(
  repoRoot,
  markerPath = qualificationStateMarkerPath(repoRoot),
) {
  const sessionPath = qualificationSessionLeasePath(repoRoot, markerPath);
  return pathEntryPresent(sessionPath);
}

/**
 * Create a durable launch lease before any pinned-sidecar verification begins.
 * The shared lock protects target validation and exclusive lease creation.
 */
export function beginQualifiedLaunch({
  repoRoot,
  targetId,
  env = process.env,
  ownerPid = process.pid,
  host = os.hostname(),
  markerPath = qualificationStateMarkerPath(repoRoot),
  sessionPath = qualificationSessionLeasePath(repoRoot, markerPath),
  lockPath = sidecarBuildLockPath(repoRoot),
}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  if (
    env.EMUCHEF_CONFIG_CREATE_QUALIFICATION !== "1" ||
    env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID !== target.qualificationTargetId
  ) {
    throw new Error("configuration-create launch requires the explicit qualification target");
  }
  const releaseLock = acquireLifecycleLock({
    repoRoot,
    targetId: target.qualificationTargetId,
    lockPath,
  });
  try {
    const markerBytes = fs.readFileSync(markerPath);
    const marker = parseRecord(markerBytes, "configuration-create qualification state marker");
    if (
      marker.schemaVersion !== 1 ||
      marker.qualificationTargetId !== target.qualificationTargetId ||
      marker.manifestPath !== MANIFEST_PATH
    ) {
      throw new Error("configuration-create qualification state does not match the selected target");
    }
    if (fs.existsSync(sessionPath)) {
      throw new Error(
        "an existing configuration-create session lease must be reviewed and archived before launch",
      );
    }
    if (marker.sessionLaunchAttempted !== false) {
      throw new Error(
        "configuration-create launch history is already used or ambiguous; preserve its state before recovery",
      );
    }
    const token = randomUUID();
    const record = {
      schemaVersion: 1,
      qualificationTargetId: target.qualificationTargetId,
      token,
      status: "launching",
      launchOwner: { pid: ownerPid, host },
      appProcess: null,
      verifiedArtifact: null,
    };
    validateProcessOwner(record.launchOwner, "configuration-create launch owner");
  writeInitialSessionLease(sessionPath, record);
    writeRecordAtomically(markerPath, {
      ...marker,
      sessionLaunchAttempted: true,
    });
    return {
      token,
      sessionPath,
      lockPath,
      environment: {
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE: sessionPath,
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_LOCK_FILE: lockPath,
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_TOKEN: token,
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_HOST: host,
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_LAUNCH_PID: String(ownerPid),
      },
    };
  } finally {
    releaseLock();
  }
}

/**
 * Verify the pinned sidecar under the shared lock and record its full identity.
 * Only the launcher that created the still-pending lease can advance it.
 */
export function markQualifiedLaunchVerified({
  repoRoot,
  targetId,
  token,
  env = process.env,
  verify = verifyQualificationSidecar,
  ownerPid = process.pid,
  host = os.hostname(),
  markerPath = qualificationStateMarkerPath(repoRoot),
  sessionPath = qualificationSessionLeasePath(repoRoot, markerPath),
  lockPath = sidecarBuildLockPath(repoRoot),
}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  const releaseLock = acquireLifecycleLock({
    repoRoot,
    targetId: target.qualificationTargetId,
    lockPath,
  });
  try {
    const { record } = readSession(sessionPath);
    if (
      record.token !== token ||
      record.qualificationTargetId !== target.qualificationTargetId ||
      record.status !== "launching" ||
      record.launchOwner.pid !== ownerPid ||
      record.launchOwner.host !== host
    ) {
      throw new Error("configuration-create launch lease owner or state does not match");
    }
    const verified = verify({
      repoRoot,
      env: {
        ...env,
        EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: target.qualificationTargetId,
      },
    });
    const manifest = verified?.manifest;
    if (
      manifest?.schemaVersion !== 1 ||
      manifest.qualificationTargetId !== target.qualificationTargetId ||
      manifest.sourceWorktreeClean !== true ||
      !/^[0-9a-f]{40}$/.test(manifest.gitCommit ?? "") ||
      !/^sha256:[0-9a-f]{64}$/.test(manifest.materialBuildDigest ?? "") ||
      !/^sha256:[0-9a-f]{64}$/.test(manifest.materialInputsDigest ?? "") ||
      JSON.stringify(manifest.backendFeatures) !==
        JSON.stringify(EXPECTED_BACKEND_FEATURES) ||
      !/^sha256:[0-9a-f]{64}$/.test(manifest.arm64HelperSha256 ?? "") ||
      !/^sha256:[0-9a-f]{64}$/.test(manifest.sidecarSha256 ?? "") ||
      manifest.runtimeSidecarSha256 !== manifest.sidecarSha256
    ) {
      throw new Error("verified configuration-create sidecar manifest has incomplete provenance");
    }
    const manifestPath = path.join(repoRoot, ...MANIFEST_PATH.split("/"));
    const manifestBytes = fs.readFileSync(manifestPath);
    if (
      JSON.stringify(parseRecord(manifestBytes, "configuration-create sidecar manifest")) !==
      JSON.stringify(manifest)
    ) {
      throw new Error("verified configuration-create sidecar manifest changed before launch");
    }
    record.status = "verified";
    record.verifiedArtifact = {
      manifestSha256: digest(manifestBytes),
      gitCommit: manifest.gitCommit,
      materialBuildDigest: manifest.materialBuildDigest,
      materialInputsDigest: manifest.materialInputsDigest,
      backendFeatures: manifest.backendFeatures,
      arm64HelperSha256: manifest.arm64HelperSha256,
      sidecarSha256: manifest.sidecarSha256,
      runtimeSidecarSha256: manifest.runtimeSidecarSha256,
    };
    writeRecordAtomically(sessionPath, record);
    return record.verifiedArtifact;
  } finally {
    releaseLock();
  }
}

/**
 * Preserve a canceled-before-start lease when preparation fails before Tauri.
 * The ownership record remains available for guarded release and recovery.
 */
export function cancelQualifiedLaunchBeforeTauri({
  repoRoot,
  targetId,
  token,
  ownerPid = process.pid,
  host = os.hostname(),
  markerPath = qualificationStateMarkerPath(repoRoot),
  sessionPath = qualificationSessionLeasePath(repoRoot, markerPath),
  lockPath = sidecarBuildLockPath(repoRoot),
}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  const releaseLock = acquireLifecycleLock({
    repoRoot,
    targetId: target.qualificationTargetId,
    lockPath,
  });
  try {
    const { record } = readSession(sessionPath);
    if (
      record.token !== token ||
      record.qualificationTargetId !== target.qualificationTargetId ||
      record.status !== "launching" ||
      record.launchOwner.pid !== ownerPid ||
      record.launchOwner.host !== host
    ) {
      throw new Error("only the matching pre-start configuration-create launch may be canceled");
    }
    record.status = "cancelled-before-start";
    record.cancelledAt = new Date().toISOString();
    writeRecordAtomically(sessionPath, record);
  } finally {
    releaseLock();
  }
}

/**
 * Require a matching native clean-exit record and a terminated app process.
 */
export function assertQualifiedCleanExit({
  targetId,
  token,
  sessionPath,
}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  const { record } = readSession(sessionPath);
  if (
    record.qualificationTargetId !== target.qualificationTargetId ||
    record.token !== token ||
    record.status !== "ended-cleanly" ||
    !record.appProcess
  ) {
    throw new Error("configuration-create app did not record a matching clean native exit");
  }
  if (processState(record.appProcess, "configuration-create application") !== "ended") {
    throw new Error("configuration-create app process has not ended after clean shutdown");
  }
  return record;
}

/**
 * Check that session ownership permits finished or explicitly abandoned release.
 * Live, remote, malformed, and uncertain process records fail closed.
 */
export function assertQualificationSessionReleasable({
  sessionPath,
  targetId,
  disposition,
  reason = "",
  sessionLaunchAttempted,
  archivedSessionBytes,
}) {
  let leasePresent = archivedSessionBytes !== undefined;
  if (!leasePresent) {
    try {
      fs.lstatSync(sessionPath);
      leasePresent = true;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  if (!leasePresent) {
    if (disposition === "finished") {
      throw new Error(
        "finished qualification release requires a clean native session record",
      );
    }
    if (sessionLaunchAttempted !== false) {
      throw new Error(
        "session lease is missing after launch was attempted or state is ambiguous",
      );
    }
    return null;
  }
  const { bytes, record } =
    archivedSessionBytes === undefined
      ? readSession(sessionPath)
      : parseSessionRecord(Buffer.from(archivedSessionBytes));
  if (record.qualificationTargetId !== targetId) {
    throw new Error("configuration-create session lease belongs to a different target");
  }
  if (record.status === "ended-cleanly") {
    if (!record.appProcess) {
      throw new Error("clean configuration-create session lacks its native process identity");
    }
    if (processState(record.appProcess, "configuration-create application") !== "ended") {
      throw new Error("configuration-create application process is still alive after clean exit");
    }
    if (processState(record.launchOwner, "configuration-create launch owner") !== "ended") {
      throw new Error("configuration-create launch owner is still alive after app exit");
    }
    return { bytes, record };
  }

  const owner =
    record.status === "active" ? record.appProcess : record.launchOwner;
  if (!owner) throw new Error("configuration-create session lease lacks its process owner");
  if (processState(owner, "configuration-create session") !== "ended") {
    throw new Error(
      record.status === "active"
        ? "active configuration-create application session is still running"
        : "configuration-create launch is still owned by a live process",
    );
  }
  if (
    record.status === "active" &&
    processState(record.launchOwner, "configuration-create launch owner") !== "ended"
  ) {
    throw new Error("configuration-create launch owner is still alive during session recovery");
  }
  if (disposition !== "abandoned" || !reason.trim()) {
    throw new Error(
      "interrupted configuration-create sessions require an explicit abandoned release and reason",
    );
  }
  return { bytes, record };
}

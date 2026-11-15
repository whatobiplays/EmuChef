import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  beginQualifiedLaunch,
  assertQualifiedCleanExit,
  cancelQualifiedLaunchBeforeTauri,
  markQualifiedLaunchVerified,
  qualificationSessionLeasePath,
} from "./qualification-session-lifecycle.mjs";
import {
  qualificationStateMarkerPath,
  sidecarBuildLockPath,
} from "./sidecar-build-contract.mjs";

const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";
const BACKEND_FEATURES = ["real-execution", "android-copy-commit-helper"];
const digest = (letter) => `sha256:${letter.repeat(64)}`;

function fixture(t) {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-configuration-create-session-"));
  execFileSync("git", ["init", "-q"], { cwd: repoRoot });
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  const markerPath = qualificationStateMarkerPath(repoRoot);
  const lockPath = sidecarBuildLockPath(repoRoot);
  const sessionPath = qualificationSessionLeasePath(repoRoot, markerPath);
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  fs.mkdirSync(path.dirname(markerPath), { recursive: true });
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.writeFileSync(
    markerPath,
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: TARGET_ID,
      manifestPath:
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      sessionLaunchAttempted: false,
    }),
  );
  const manifest = {
    schemaVersion: 1,
    qualificationTargetId: TARGET_ID,
    gitCommit: "a".repeat(40),
    materialBuildDigest: digest("b"),
    materialInputsDigest: digest("c"),
    sourceWorktreeClean: true,
    backendFeatures: BACKEND_FEATURES,
    arm64HelperSha256: digest("d"),
    sidecarSha256: digest("e"),
    runtimeSidecarSha256: digest("e"),
  };
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  const env = {
    EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
  };
  return {
    repoRoot,
    markerPath,
    lockPath,
    sessionPath,
    manifest,
    manifestPath,
    env,
    begin: () =>
      beginQualifiedLaunch({
        repoRoot,
        targetId: TARGET_ID,
        env,
        markerPath,
        sessionPath,
        lockPath,
      }),
  };
}

test("launch creates an owned lease before sidecar verification", (t) => {
  const state = fixture(t);
  const launch = state.begin();
  const record = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));

  assert.equal(record.status, "launching");
  assert.equal(record.qualificationTargetId, TARGET_ID);
  assert.equal(record.token, launch.token);
  assert.equal(record.launchOwner.pid, process.pid);
  assert.equal(record.launchOwner.host, os.hostname());
  assert.equal(record.appProcess, null);
  assert.equal(
    JSON.parse(fs.readFileSync(state.markerPath, "utf8")).sessionLaunchAttempted,
    true,
  );
  assert.equal(
    launch.environment.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE,
    state.sessionPath,
  );
  assert.equal(fs.existsSync(state.lockPath), false);
});

test("interrupted initial lease write leaves no partial final record", (t) => {
  const state = fixture(t);
  const originalWriteFileSync = fs.writeFileSync;
  let injected = false;
  fs.writeFileSync = function (filePath, ...args) {
    if (String(filePath).startsWith(`${state.sessionPath}.tmp-`)) {
      injected = true;
      originalWriteFileSync.call(fs, filePath, Buffer.from('{"partial":'), {
        flag: "wx",
        mode: 0o600,
      });
      throw Object.assign(new Error("injected partial session write"), {
        code: "EIO",
      });
    }
    return originalWriteFileSync.call(fs, filePath, ...args);
  };
  try {
    assert.throws(state.begin, /injected partial session write/);
  } finally {
    fs.writeFileSync = originalWriteFileSync;
  }

  assert.equal(injected, true);
  assert.equal(fs.existsSync(state.sessionPath), false);
  assert.equal(
    fs.readdirSync(path.dirname(state.sessionPath)).some((entry) =>
      entry.startsWith(`${path.basename(state.sessionPath)}.tmp-`),
    ),
    false,
  );
  assert.equal(
    JSON.parse(fs.readFileSync(state.markerPath, "utf8")).sessionLaunchAttempted,
    false,
  );
  assert.equal(fs.existsSync(state.lockPath), false);
});

test("launch rejects target mismatch and never overwrites an existing lease", (t) => {
  const state = fixture(t);
  assert.throws(
    () =>
      beginQualifiedLaunch({
        repoRoot: state.repoRoot,
        targetId: "another-target",
        env: state.env,
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /target/i,
  );

  const launch = state.begin();
  const leaseBytes = fs.readFileSync(state.sessionPath);
  assert.throws(state.begin, /existing configuration-create session lease/i);
  assert.equal(fs.readFileSync(state.sessionPath).equals(leaseBytes), true);
  assert.equal(launch.token.length > 0, true);
});

test("verification binds the current owner to the pinned artifact identity", (t) => {
  const state = fixture(t);
  const launch = state.begin();
  const verify = () => ({ manifest: state.manifest });

  assert.throws(
    () =>
      markQualifiedLaunchVerified({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        token: "wrong-token",
        env: state.env,
        verify,
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /owner or state/i,
  );
  assert.throws(
    () =>
      markQualifiedLaunchVerified({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        token: launch.token,
        env: state.env,
        verify,
        ownerPid: process.pid + 1,
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /owner or state/i,
  );

  const verified = markQualifiedLaunchVerified({
    repoRoot: state.repoRoot,
    targetId: TARGET_ID,
    token: launch.token,
    env: state.env,
    verify,
    markerPath: state.markerPath,
    sessionPath: state.sessionPath,
    lockPath: state.lockPath,
  });
  const record = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  assert.equal(record.status, "verified");
  assert.deepEqual(record.verifiedArtifact.backendFeatures, BACKEND_FEATURES);
  assert.equal(record.verifiedArtifact.gitCommit, state.manifest.gitCommit);
  assert.equal(record.verifiedArtifact.arm64HelperSha256, digest("d"));
  assert.equal(record.verifiedArtifact.sidecarSha256, digest("e"));
  assert.equal(verified.manifestSha256.startsWith("sha256:"), true);
});

test("launch verification rejects wrong target and incomplete provenance", (t) => {
  const state = fixture(t);
  const launch = state.begin();
  const wrongTarget = { ...state.manifest, qualificationTargetId: "other" };
  assert.throws(
    () =>
      markQualifiedLaunchVerified({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        token: launch.token,
        env: state.env,
        verify: () => ({ manifest: wrongTarget }),
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /incomplete provenance/i,
  );
  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "launching",
  );
});

test("only the matching pre-start owner can cancel a launch lease", (t) => {
  const state = fixture(t);
  const launch = state.begin();
  const leaseBytes = fs.readFileSync(state.sessionPath);

  assert.throws(
    () =>
      cancelQualifiedLaunchBeforeTauri({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        token: "wrong-token",
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /matching pre-start/i,
  );
  assert.equal(fs.readFileSync(state.sessionPath).equals(leaseBytes), true);

  cancelQualifiedLaunchBeforeTauri({
    repoRoot: state.repoRoot,
    targetId: TARGET_ID,
    token: launch.token,
    markerPath: state.markerPath,
    sessionPath: state.sessionPath,
    lockPath: state.lockPath,
  });
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "cancelled-before-start",
  );
});

test("clean-exit verification requires the actual app process to have ended", (t) => {
  const state = fixture(t);
  const launch = state.begin();
  const record = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  record.status = "ended-cleanly";
  record.appProcess = { pid: process.pid, host: os.hostname() };
  fs.writeFileSync(state.sessionPath, JSON.stringify(record));

  assert.throws(
    () =>
      assertQualifiedCleanExit({
        targetId: TARGET_ID,
        token: launch.token,
        sessionPath: state.sessionPath,
      }),
    /has not ended/i,
  );
  const exitedPid = spawnSync(process.execPath, ["-e", ""], {
    encoding: "utf8",
  }).pid;
  record.appProcess.pid = exitedPid;
  fs.writeFileSync(state.sessionPath, JSON.stringify(record));
  assert.doesNotThrow(() =>
    assertQualifiedCleanExit({
      targetId: TARGET_ID,
      token: launch.token,
      sessionPath: state.sessionPath,
    }),
  );
});

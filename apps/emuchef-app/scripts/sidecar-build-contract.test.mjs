import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { syncBuiltinESMExports } from "node:module";
import fs from "node:fs";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";
import {
  acquireSidecarBuildCoordinationGate,
  acquireSidecarBuildLock,
  assertReleasePackageLock,
  assertQualificationArtifactPath,
  inspectSidecarBuildBorrowers,
  QUALIFICATION_BACKEND_FEATURES,
  SIDECAR_BUILD_LOCK_TOKEN_ENV,
  requireConfigCreateQualificationTarget,
  sidecarBuildBorrowerDirectoryPath,
  sidecarBuildLockPath,
  validateQualificationSidecarManifest,
  withReleasePackageLock,
} from "./sidecar-build-contract.mjs";

const digest = (character) => `sha256:${character.repeat(64)}`;

function fixture() {
  const sourceIdentity = {
    gitCommit: "281d0929c253f187ef622e8be1dd6f438c566f5d",
    materialBuildDigest: digest("a"),
  };
  const provenance = {
    schemaVersion: 1,
    backendFeatures: [...QUALIFICATION_BACKEND_FEATURES],
    arm64HelperSha256: digest("b"),
    gitCommit: sourceIdentity.gitCommit,
    materialBuildDigest: sourceIdentity.materialBuildDigest,
    materialInputsDigest: digest("c"),
  };
  const manifest = {
    schemaVersion: 1,
    sourceWorktreeClean: true,
    gitCommit: sourceIdentity.gitCommit,
    materialBuildDigest: sourceIdentity.materialBuildDigest,
    materialInputsDigest: digest("c"),
    qualificationTargetId: "konkr-pocket-advance-retroarch-config-create",
    backendFeatures: [...provenance.backendFeatures],
    arm64HelperSha256: provenance.arm64HelperSha256,
    sidecarSha256: digest("d"),
  };
  const expected = {
    sourceIdentity,
    materialInputsDigest: manifest.materialInputsDigest,
    sidecarSha256: manifest.sidecarSha256,
    provenance,
    targetId: manifest.qualificationTargetId,
  };
  return { manifest, expected };
}

test("qualification artifacts reject symlinked files and parent directories", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "emuchef-configuration-create-artifact-path-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const outside = path.join(root, "outside");
  const artifact = path.join(root, "build/artifact");
  mkdirSync(path.dirname(artifact));
  writeFileSync(outside, "outside bytes");
  symlinkSync(outside, artifact);
  assert.throws(
    () => assertQualificationArtifactPath(root, artifact, { mustExist: true }),
    /symlink/,
  );

  const linkedDirectory = path.join(root, "linked-build");
  symlinkSync(path.dirname(artifact), linkedDirectory, "dir");
  assert.throws(
    () =>
      assertQualificationArtifactPath(
        root,
        path.join(linkedDirectory, "artifact"),
      ),
    /symlink/,
  );
});

async function startLockHolder(lockPath) {
  const moduleUrl = new URL("./sidecar-build-contract.mjs", import.meta.url);
  const childSource = [
    "import { acquireSidecarBuildLock } from " +
      JSON.stringify(moduleUrl.href) +
      ";",
    "const release = acquireSidecarBuildLock({ lockPath: process.env.EMUCHEF_TEST_LOCK_PATH, profile: 'debug' });",
    "process.stdout.write('lock-held\\n');",
    "await new Promise((resolve) => process.stdin.once('data', resolve));",
    "release();",
  ].join("\n");
  const child = spawn(
    process.execPath,
    ["--input-type=module", "--eval", childSource],
    {
      env: { ...process.env, EMUCHEF_TEST_LOCK_PATH: lockPath },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  const errors = [];
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => errors.push(chunk));
  const readyLine = await new Promise((resolve, reject) => {
    const cleanup = () => {
      output.removeListener("line", onLine);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onLine = (line) => {
      cleanup();
      resolve(line);
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      reject(
        new Error(
          "lock holder exited before acquiring lock (code=" +
            code +
            ", signal=" +
            signal +
            "): " +
            errors.join(""),
        ),
      );
    };
    output.once("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });
  assert.equal(readyLine, "lock-held");
  return { child, output };
}

async function stopLockHolder(holder, signal = null) {
  if (
    holder.child.exitCode !== null ||
    holder.child.signalCode !== null ||
    holder.child.pid === undefined
  ) {
    return;
  }
  const exited = once(holder.child, "exit");
  if (signal) {
    holder.child.kill(signal);
  } else {
    holder.child.stdin.end("release\n");
  }
  await exited;
}

test("qualification sidecar manifest binds source, target, features, helper, and external binary", () => {
  const { manifest, expected } = fixture();
  assert.equal(
    validateQualificationSidecarManifest(manifest, expected),
    manifest,
  );
});

test("qualification sidecar manifest rejects changed source, target, helper, or external binary", () => {
  const checks = [
    [
      (manifest) => {
        manifest.gitCommit = "different";
      },
      /source identity does not match/,
    ],
    [
      (_manifest, expected) => {
        expected.targetId = "different-target";
      },
      /target identity does not match/,
    ],
    [
      (_manifest, expected) => {
        expected.sidecarSha256 = digest("e");
      },
      /external binary changed/,
    ],
    [
      (manifest) => {
        manifest.arm64HelperSha256 = digest("f");
      },
      /helper digest differs/,
    ],
    [
      (_manifest, expected) => {
        expected.provenance.backendFeatures = ["real-execution"];
      },
      /required backend features/,
    ],
  ];

  for (const [mutate, message] of checks) {
    const { manifest, expected } = fixture();
    mutate(manifest, expected);
    assert.throws(
      () => validateQualificationSidecarManifest(manifest, expected),
      message,
    );
  }
});

test("qualification sidecar manifest requires compiled provenance for its exact source", () => {
  const { manifest, expected } = fixture();
  expected.provenance.materialInputsDigest = digest("e");

  assert.throws(
    () => validateQualificationSidecarManifest(manifest, expected),
    /compiled qualification material inputs/i,
  );
});

test("qualification sidecar manifest rejects dirty source state and changed material inputs", () => {
  const dirty = fixture();
  dirty.manifest.sourceWorktreeClean = false;
  assert.throws(
    () => validateQualificationSidecarManifest(dirty.manifest, dirty.expected),
    /clean source worktree/,
  );

  const changedInputs = fixture();
  changedInputs.expected.materialInputsDigest = digest("e");
  assert.throws(
    () =>
      validateQualificationSidecarManifest(
        changedInputs.manifest,
        changedInputs.expected,
      ),
    /source identity does not match/,
  );
});

test("qualification sidecar builds require the explicit configuration-create target", () => {
  assert.throws(
    () => requireConfigCreateQualificationTarget("qualification", {}),
    /explicit configuration-create qualification mode/,
  );
  assert.throws(
    () =>
      requireConfigCreateQualificationTarget("qualification", {
        EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: "other-target",
      }),
    /explicit configuration-create target ID is required/,
  );
  assert.equal(
    requireConfigCreateQualificationTarget("qualification", {
      EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
      EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: "konkr-pocket-advance-retroarch-config-create",
    }),
    "konkr-pocket-advance-retroarch-config-create",
  );
  assert.equal(requireConfigCreateQualificationTarget("debug", {}), null);
});

test("sidecar build lock serializes artifact publication and releases after the operation", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "emuchef-sidecar-lock-"));
  const lockPath = path.join(root, "sidecar-build.lock");
  try {
    const release = acquireSidecarBuildLock({ lockPath, profile: "debug" });
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath, profile: "qualification" }),
      /sidecar build lock already exists/,
    );
    release();
    assert.doesNotThrow(() =>
      acquireSidecarBuildLock({ lockPath, profile: "debug" })(),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("borrower records reject incomplete and mismatched identity and treat reused PIDs as uncertain", (t) => {
  const repoRoot = mkdtempSync(path.join(os.tmpdir(), "emuchef-borrower-records-"));
  t.after(() => rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: repoRoot });
  const lockPath = sidecarBuildLockPath(repoRoot);
  const releaseOwner = acquireSidecarBuildLock({
    repoRoot,
    profile: "release-package",
  });
  const owner = JSON.parse(readFileSync(lockPath, "utf8"));
  const directory = sidecarBuildBorrowerDirectoryPath(lockPath);
  const token = "00000000-0000-4000-8000-000000000001";
  const recordPath = path.join(directory, `${token}.json`);
  const baseRecord = {
    schemaVersion: 1,
    kind: "sidecar-build-borrower",
    pid: process.pid,
    host: os.hostname(),
    token,
    ownerToken: owner.token,
    parentDevice: releaseOwner.lockIdentity.dev,
    parentInode: releaseOwner.lockIdentity.ino,
    artifactPaths: [],
    startedAt: new Date().toISOString(),
  };
  const inspect = () => inspectSidecarBuildBorrowers({
    lockPath,
    owner,
    lockIdentity: releaseOwner.lockIdentity,
  });
  assert.deepEqual(inspect(), []);
  assert.throws(
    () => assertReleasePackageLock({
      repoRoot,
      env: { [SIDECAR_BUILD_LOCK_TOKEN_ENV]: owner.token },
      lockPath: path.join(repoRoot, "missing-sidecar-build-lock"),
    }),
    /unavailable/,
  );
  assert.throws(
    () => acquireSidecarBuildLock({
      repoRoot,
      profile: "release",
      inheritedToken: "wrong-owner-token",
    }),
    /ownership does not match/,
  );
  assert.throws(
    () => acquireSidecarBuildLock({
      repoRoot,
      profile: "release",
      inheritedToken: owner.token,
      artifactPaths: ["outside/target"],
    }),
    /outside the owner lock scope/,
  );
  const releaseGate = acquireSidecarBuildCoordinationGate(lockPath);
  mkdirSync(directory, { mode: 0o700 });
  try {
    writeFileSync(path.join(directory, `.tmp-${process.pid}-partial`), "partial");
    assert.throws(inspect, /unexpected entry/);
    fs.rmSync(directory, { recursive: true, force: true });
    mkdirSync(directory, { mode: 0o700 });

    writeFileSync(recordPath, "{", { flag: "wx" });
    assert.throws(inspect, /malformed/);
    fs.unlinkSync(recordPath);

    writeFileSync(
      recordPath,
      JSON.stringify({ ...baseRecord, ownerToken: "different-owner" }),
      { flag: "wx" },
    );
    assert.throws(inspect, /does not match its parent lock authority/);
    fs.unlinkSync(recordPath);

    writeFileSync(
      recordPath,
      JSON.stringify({ ...baseRecord, parentInode: baseRecord.parentInode + 1 }),
      { flag: "wx" },
    );
    assert.throws(inspect, /does not match its parent lock authority/);
    fs.unlinkSync(recordPath);

    writeFileSync(
      recordPath,
      JSON.stringify({ ...baseRecord, artifactPaths: ["outside/target"] }),
      { flag: "wx" },
    );
    assert.throws(inspect, /outside the owner lock scope/);
    fs.unlinkSync(recordPath);

    writeFileSync(recordPath, JSON.stringify(baseRecord), { flag: "wx" });
    const reusedPid = inspect();
    assert.equal(reusedPid.length, 1);
    assert.deepEqual(reusedPid[0].holders, []);
    assert.equal(reusedPid[0].state, "uncertain");
    fs.unlinkSync(recordPath);

    const missingPidRecord = {
      ...baseRecord,
      pid: 9_999_999,
      token: "00000000-0000-4000-8000-000000000002",
    };
    const stalePath = path.join(directory, `${missingPidRecord.token}.json`);
    writeFileSync(stalePath, JSON.stringify(missingPidRecord), { flag: "wx" });
    const stale = inspect();
    assert.equal(stale.length, 1);
    assert.equal(stale[0].state, "stale");
    fs.unlinkSync(stalePath);
  } finally {
    releaseGate();
    fs.rmSync(directory, { recursive: true, force: true });
    releaseOwner();
  }
});

test("temporary lock cleanup failure keeps a releasable canonical lock", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "emuchef-sidecar-lock-cleanup-"));
  const lockPath = path.join(root, "sidecar-build.lock");
  const originalUnlinkSync = fs.unlinkSync;
  let temporaryPath;
  try {
    fs.unlinkSync = (filePath, ...args) => {
      const candidate = String(filePath);
      if (!temporaryPath && candidate.startsWith(`${lockPath}.tmp-`)) {
        temporaryPath = candidate;
        const error = new Error("injected temporary lock cleanup failure");
        error.code = "EACCES";
        throw error;
      }
      return originalUnlinkSync.call(fs, filePath, ...args);
    };
    syncBuiltinESMExports();

    const release = acquireSidecarBuildLock({ lockPath, profile: "debug" });
    assert.ok(temporaryPath);
    assert.equal(typeof release, "function");
    assert.equal(fs.existsSync(lockPath), true);
    assert.equal(fs.lstatSync(temporaryPath).isFile(), true);
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath, profile: "qualification" }),
      /sidecar build lock already exists/,
    );

    release();
    assert.equal(fs.existsSync(lockPath), false);
    assert.equal(fs.existsSync(temporaryPath), false);
    acquireSidecarBuildLock({ lockPath, profile: "debug" })();
  } finally {
    fs.unlinkSync = originalUnlinkSync;
    syncBuiltinESMExports();
    rmSync(root, { recursive: true, force: true });
  }
});

test("sidecar build lock excludes a second process until the owner releases it", async () => {
  const root = mkdtempSync(
    path.join(os.tmpdir(), "emuchef-sidecar-lock-process-"),
  );
  const lockPath = path.join(root, "sidecar-build.lock");
  let holder;
  try {
    holder = await startLockHolder(lockPath);
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath, profile: "qualification" }),
      /sidecar build lock already exists/,
    );

    const exited = once(holder.child, "exit");
    holder.child.stdin.end("release\n");
    const [code, signal] = await exited;
    assert.equal(code, 0);
    assert.equal(signal, null);
    assert.doesNotThrow(() =>
      acquireSidecarBuildLock({ lockPath, profile: "debug" })(),
    );
  } finally {
    if (holder) await stopLockHolder(holder);
    rmSync(root, { recursive: true, force: true });
  }
});

test("an interrupted lock owner leaves the lock for explicit recovery", async () => {
  const root = mkdtempSync(
    path.join(os.tmpdir(), "emuchef-sidecar-lock-interrupted-"),
  );
  const lockPath = path.join(root, "sidecar-build.lock");
  let holder;
  try {
    holder = await startLockHolder(lockPath);
    const owner = JSON.parse(readFileSync(lockPath, "utf8"));
    assert.equal(owner.pid, holder.child.pid);
    assert.equal(owner.host, os.hostname());

    const exited = once(holder.child, "exit");
    holder.child.kill("SIGKILL");
    const [code, signal] = await exited;
    assert.equal(code, null);
    assert.equal(signal, "SIGKILL");
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath, profile: "debug" }),
      /sidecar build lock already exists/,
    );
  } finally {
    if (holder) await stopLockHolder(holder, "SIGKILL");
    rmSync(root, { recursive: true, force: true });
  }
});

test("release package lock remains held through the Tauri child lifetime", async (t) => {
  const repoRoot = mkdtempSync(path.join(os.tmpdir(), "emuchef-release-lock-"));
  t.after(() => rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: repoRoot });

  const contractUrl = new URL("./sidecar-build-contract.mjs", import.meta.url).href;
  const childCode = `
    import { acquireSidecarBuildLock, assertReleasePackageLock } from ${JSON.stringify(contractUrl)};
    import fs from "node:fs";
    const repoRoot = process.env.EMUCHEF_TEST_REPO_ROOT;
    const lockPath = process.env.EMUCHEF_TEST_LOCK_PATH;
    const inheritedLock = fs.fstatSync(3);
    const currentLock = fs.statSync(lockPath);
    if (inheritedLock.dev !== currentLock.dev || inheritedLock.ino !== currentLock.ino) throw new Error("Tauri child did not inherit the exact owner lock descriptor");
    assertReleasePackageLock({ repoRoot, env: process.env, lockPath });
    const release = acquireSidecarBuildLock({
      repoRoot,
      profile: "release",
      inheritedToken: process.env.EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN,
      lockPath,
    });
    process.stdout.write("tauri-package-ready\\n");
    await new Promise((resolve) => process.stdin.once("data", resolve));
    release();
  `;

  let child;
  let allowPackageToFinish;
  const packageMayFinish = new Promise((resolve) => {
    allowPackageToFinish = resolve;
  });
  let signalPackageReady;
  let rejectPackageReady;
  const packageReady = new Promise((resolve, reject) => {
    signalPackageReady = resolve;
    rejectPackageReady = reject;
  });

  const packageOperation = withReleasePackageLock({
    repoRoot,
    env: process.env,
    run: async (env, { lockFileDescriptor }) => {
      child = spawn(process.execPath, ["--input-type=module", "--eval", childCode], {
        env: {
          ...env,
          EMUCHEF_TEST_REPO_ROOT: repoRoot,
          EMUCHEF_TEST_LOCK_PATH: path.join(
            repoRoot,
            ".git",
            "emuchef",
            "retroarch-config-create-sidecar-build.lock",
          ),
        },
        stdio: ["pipe", "pipe", "pipe", lockFileDescriptor],
      });
      const output = createInterface({ input: child.stdout });
      const childReady = new Promise((resolve, reject) => {
        output.once("line", resolve);
        child.once("error", reject);
        child.once("exit", (code, signal) => {
          reject(
            new Error(
              `Tauri child exited before the barrier with ${code}/${signal}`,
            ),
          );
        });
      });
      try {
        assert.equal(await childReady, "tauri-package-ready");
      } catch (error) {
        rejectPackageReady(error);
        throw error;
      }
      signalPackageReady();
      await packageMayFinish;
      child.stdin.end("finish\\n");
      const [code, signal] = await once(child, "exit");
      output.close();
      assert.equal(code, 0);
      assert.equal(signal, null);
    },
  });

  try {
    await packageReady;
    assert.throws(
      () => acquireSidecarBuildLock({ repoRoot, profile: "qualification" }),
      /sidecar build lock already exists/,
    );
    assert.throws(
      () => acquireSidecarBuildLock({ repoRoot, profile: "release" }),
      /sidecar build lock already exists/,
    );
    assert.throws(
      () =>
        assertReleasePackageLock({
          repoRoot,
          env: {
            EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN: "wrong-token",
          },
        }),
      /ownership does not match/,
    );
    allowPackageToFinish();
    await packageOperation;
    const release = acquireSidecarBuildLock({ repoRoot, profile: "qualification" });
    release();
  } finally {
    allowPackageToFinish();
    if (child && child.exitCode === null) child.kill("SIGKILL");
  }
});

test("release owner cannot retire the lock while a sidecar hook borrows it", async (t) => {
  const repoRoot = mkdtempSync(path.join(os.tmpdir(), "emuchef-release-borrower-"));
  t.after(() => rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: repoRoot });
  const lockPath = path.join(repoRoot, "sidecar-build.lock");
  const releaseOwner = acquireSidecarBuildLock({
    repoRoot,
    profile: "release-package",
    lockPath,
  });
  const owner = JSON.parse(readFileSync(lockPath, "utf8"));
  const contractUrl = new URL("./sidecar-build-contract.mjs", import.meta.url).href;
  const childCode = `
    import { acquireSidecarBuildLock, assertReleasePackageLock } from ${JSON.stringify(contractUrl)};
    const repoRoot = process.env.EMUCHEF_TEST_REPO_ROOT;
    const lockPath = process.env.EMUCHEF_TEST_LOCK_PATH;
    assertReleasePackageLock({ repoRoot, env: process.env, lockPath });
    const releaseBorrow = acquireSidecarBuildLock({
      repoRoot,
      profile: "release",
      inheritedToken: process.env.EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN,
      lockPath,
    });
    process.stdout.write("borrowed\\n");
    await new Promise((resolve) => process.stdin.once("data", resolve));
    releaseBorrow();
  `;
  const borrower = spawn(
    process.execPath,
    ["--input-type=module", "--eval", childCode],
    {
      env: {
        ...process.env,
        EMUCHEF_TEST_REPO_ROOT: repoRoot,
        EMUCHEF_TEST_LOCK_PATH: lockPath,
        [SIDECAR_BUILD_LOCK_TOKEN_ENV]: owner.token,
      },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  const output = createInterface({ input: borrower.stdout, crlfDelay: Infinity });
  let borrowerStderr = "";
  borrower.stderr.setEncoding("utf8").on("data", (chunk) => {
    borrowerStderr += chunk;
  });
  const childReady = new Promise((resolve, reject) => {
    output.once("line", resolve);
    borrower.once("error", reject);
    borrower.once("exit", (code, signal) => {
      reject(
        new Error(
          `borrower exited before barrier with ${code}/${signal}: ${borrowerStderr}`,
        ),
      );
    });
  });
  let competingPublisherRelease;
  let ownerReleasedWhileBorrowerActive = false;
  try {
    assert.equal(await childReady, "borrowed");
    try {
      releaseOwner();
      ownerReleasedWhileBorrowerActive = true;
    } catch (error) {
      assert.match(error.message, /borrower/i);
    }
    assert.equal(ownerReleasedWhileBorrowerActive, false);
    assert.equal(fs.existsSync(lockPath), true);
    assert.throws(
      () => acquireSidecarBuildLock({ repoRoot, profile: "debug", lockPath }),
    /sidecar build lock already exists/,
    );

    borrower.stdin.end("finish\\n");
    const [code, signal] = await once(borrower, "exit");
    output.close();
    assert.equal(code, 0);
    assert.equal(signal, null);
    releaseOwner();
    competingPublisherRelease = acquireSidecarBuildLock({
      repoRoot,
      profile: "debug",
      lockPath,
    });
  } finally {
    if (borrower.exitCode === null && borrower.signalCode === null) {
      borrower.stdin.end("finish\\n");
      borrower.kill("SIGTERM");
    }
    if (competingPublisherRelease) competingPublisherRelease();
    try {
      releaseOwner();
    } catch {}
    output.close();
  }
});

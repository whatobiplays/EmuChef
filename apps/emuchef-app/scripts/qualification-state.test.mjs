import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import test from "node:test";

import { archiveAndReleaseQualificationState } from "./qualification-state.mjs";
import { targetExecutableExtension } from "../../../scripts/sidecar-target-contract.mjs";
import {
  acquireSidecarBuildCoordinationGate,
  acquireSidecarBuildLock,
  inspectSidecarBuildBorrowers,
  inspectSidecarBuildLockOwner,
  SIDECAR_BUILD_LOCK_TOKEN_ENV,
  sidecarBuildBorrowerDirectoryPath,
  sidecarBuildDisposition,
} from "./sidecar-build-contract.mjs";
import { beginQualifiedLaunch } from "./qualification-session-lifecycle.mjs";
import { prepareRustSidecar } from "./prepare-rust-sidecar.mjs";
import {
  BINARY_BASENAME,
  externalBinArtifactName,
} from "./sidecar-packaging.mjs";

const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";
const digest = (bytes) =>
  `sha256:${createHash("sha256").update(bytes).digest("hex")}`;

function fixture(t) {
  const repoRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "emuchef-configuration-create-state-"),
  );
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "-q"], { cwd: repoRoot });
  const markerPath = path.join(
    repoRoot,
    ".git/emuchef/configuration-create-qualification-state.json",
  );
  const sessionPath = path.join(
    repoRoot,
    ".git/emuchef/retroarch-config-create-qualification-session.json",
  );
  const lockPath = path.join(repoRoot, ".git/emuchef/retroarch-config-create-sidecar-build.lock");
  const target = execFileSync("rustc", ["--print", "host-tuple"], {
    encoding: "utf8",
  }).trim();
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  const externalPath = path.join(
    repoRoot,
    "apps/emuchef-app/src-tauri/binaries",
    externalBinArtifactName(target),
  );
  const runtimePath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug",
    `${BINARY_BASENAME}${targetExecutableExtension(target)}`,
  );
  const evidenceDirectory = path.join(
    path.dirname(repoRoot),
    `${path.basename(repoRoot)}-evidence`,
  );
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));
  const externalBytes = Buffer.from("external sidecar");
  const runtimeBytes = Buffer.from("external sidecar");
  const manifest = {
    schemaVersion: 1,
    sourceWorktreeClean: true,
    qualificationTargetId: TARGET_ID,
    gitCommit: "a".repeat(40),
    materialBuildDigest: digest(Buffer.from("build identity")),
    materialInputsDigest: digest(Buffer.from("material inputs")),
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: `sha256:${"b".repeat(64)}`,
    sidecarPath: path
      .relative(repoRoot, externalPath)
      .split(path.sep)
      .join("/"),
    runtimeSidecarPath: path
      .relative(repoRoot, runtimePath)
      .split(path.sep)
      .join("/"),
    sidecarSha256: digest(externalBytes),
    runtimeSidecarSha256: digest(runtimeBytes),
  };

  fs.mkdirSync(path.dirname(markerPath), { recursive: true });
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.mkdirSync(path.dirname(externalPath), { recursive: true });
  fs.mkdirSync(path.dirname(runtimePath), { recursive: true });
  fs.writeFileSync(
    markerPath,
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: TARGET_ID,
      manifestPath:
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      sessionLaunchAttempted: true,
    }),
  );
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  fs.writeFileSync(externalPath, externalBytes);
  fs.writeFileSync(runtimePath, runtimeBytes);
  const exitedOwnerPid = spawnSync(process.execPath, ["-e", ""], {
    encoding: "utf8",
  }).pid;
  fs.writeFileSync(
    sessionPath,
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: TARGET_ID,
      token: "fixture-session-token",
      status: "ended-cleanly",
      launchOwner: { pid: exitedOwnerPid, host: os.hostname() },
      appProcess: { pid: exitedOwnerPid, host: os.hostname() },
    }),
  );

  return {
    repoRoot,
    markerPath,
    sessionPath,
    lockPath,
    manifest,
    manifestPath,
    externalPath,
    runtimePath,
    evidenceDirectory,
    options: {
      repoRoot,
      markerPath,
      sessionPath,
      lockPath,
      targetId: TARGET_ID,
      confirmEndedTarget: TARGET_ID,
      evidenceDirectory,
      disposition: "finished",
    },
  };
}

function recoverDocumentedStaleLock({ repoRoot, lockPath, evidenceDirectory }) {
  const releaseCoordination = acquireSidecarBuildCoordinationGate(lockPath);
  try {
    return recoverDocumentedStaleLockUnderGate({ repoRoot, lockPath, evidenceDirectory });
  } finally {
    releaseCoordination();
  }
}

function recoverDocumentedStaleLockUnderGate({ repoRoot, lockPath, evidenceDirectory }) {
  const lockState = inspectSidecarBuildLockOwner({ lockPath });
  if (lockState.state !== "stale") {
    throw new Error(
      "lock owner or protected child is " + lockState.state + " during stale-lock recovery",
    );
  }
  const initialBytes = fs.readFileSync(lockPath);
  const owner = JSON.parse(initialBytes.toString("utf8"));
  if (
    owner.schemaVersion !== 1 ||
    owner.host !== os.hostname() ||
    !Number.isSafeInteger(owner.pid) ||
    owner.pid <= 0 ||
    typeof owner.token !== "string" ||
    !owner.token
  ) {
    throw new Error("lock owner identity is incomplete or belongs to another host");
  }
  const initialMetadata = fs.lstatSync(lockPath);
  if (
    initialMetadata.isSymbolicLink() ||
    initialMetadata.dev !== lockState.identity.dev ||
    initialMetadata.ino !== lockState.identity.ino ||
    owner.token !== lockState.owner.token
  ) {
    throw new Error("lock owner changed before stale-lock recovery began");
  }
  const borrowers = inspectSidecarBuildBorrowers({
    lockPath,
    owner,
    lockIdentity: lockState.identity,
  });
  if (borrowers.some((borrower) => borrower.state !== "stale")) {
    throw new Error("borrower process state is live or uncertain during stale-lock recovery");
  }

  const repositoryRoot = fs.realpathSync(repoRoot);
  const copiedArtifacts = [];
  const missingArtifacts = [];
  const availableArtifacts = [];
  for (const relativePath of owner.artifactPaths) {
    if (typeof relativePath !== "string" || !relativePath || path.isAbsolute(relativePath)) {
      throw new Error("lock record contains an artifact path outside the repository");
    }
    const sourcePath = path.resolve(repositoryRoot, relativePath);
    const repositoryPath = path.relative(repositoryRoot, sourcePath);
    if (
      repositoryPath === ".." ||
      repositoryPath.startsWith(`..${path.sep}`) ||
      repositoryPath === "" ||
      path.isAbsolute(repositoryPath)
    ) {
      throw new Error("lock record contains an artifact path outside the repository");
    }

    let currentPath = repositoryRoot;
    const pathParts = repositoryPath.split(path.sep).filter(Boolean);
    let missing = false;
    for (const [index, part] of pathParts.entries()) {
      currentPath = path.join(currentPath, part);
      let metadata;
      try {
        metadata = fs.lstatSync(currentPath);
      } catch (error) {
        if (error.code === "ENOENT") {
          missing = true;
          break;
        }
        throw error;
      }
      if (metadata.isSymbolicLink()) {
        throw new Error("listed lock artifact traverses a symbolic link");
      }
      const isLastPathPart = index === pathParts.length - 1;
      if (!isLastPathPart && !metadata.isDirectory()) {
        throw new Error("listed lock artifact has a non-directory parent");
      }
      if (isLastPathPart && !metadata.isFile()) {
        throw new Error("listed lock artifact is not a regular file");
      }
    }
    if (missing) {
      missingArtifacts.push(relativePath);
    } else {
      availableArtifacts.push({ relativePath, sourcePath: currentPath });
    }
  }

  const absoluteEvidenceDirectory = path.resolve(evidenceDirectory);
  const evidenceDirectoryName = path.basename(absoluteEvidenceDirectory);
  let resolvedEvidenceParent;
  try {
    resolvedEvidenceParent = fs.realpathSync(
      path.dirname(absoluteEvidenceDirectory),
    );
  } catch {
    throw new Error("lock evidence directory parent is unavailable");
  }
  const resolvedEvidenceDirectory = path.join(
    resolvedEvidenceParent,
    evidenceDirectoryName,
  );
  const relativeEvidence = path.relative(
    repositoryRoot,
    resolvedEvidenceDirectory,
  );
  if (
    !path.isAbsolute(evidenceDirectory) ||
    !evidenceDirectoryName ||
    relativeEvidence === "" ||
    (!relativeEvidence.startsWith(`..${path.sep}`) && relativeEvidence !== "..")
  ) {
    throw new Error(
      "lock evidence directory must resolve outside the repository",
    );
  }
  fs.mkdirSync(resolvedEvidenceDirectory);
  const lockArchivePath = path.join(
    resolvedEvidenceDirectory,
    "sidecar-build-lock.json",
  );
  fs.writeFileSync(lockArchivePath, initialBytes, { flag: "wx" });
  if (!fs.readFileSync(lockArchivePath).equals(initialBytes)) {
    throw new Error("lock record archive failed readback");
  }

  const archivedBorrowers = [];
  for (const borrower of borrowers) {
    const archivePath = path.join(
      resolvedEvidenceDirectory,
      "borrowers",
      borrower.name,
    );
    fs.mkdirSync(path.dirname(archivePath), { recursive: true });
    fs.writeFileSync(archivePath, borrower.bytes, { flag: "wx" });
    if (!fs.readFileSync(archivePath).equals(borrower.bytes)) {
      throw new Error("borrower record archive failed readback");
    }
    archivedBorrowers.push({
      token: borrower.record.token,
      sha256: digest(borrower.bytes),
    });
  }
  for (const { relativePath, sourcePath } of availableArtifacts) {
    const bytes = fs.readFileSync(sourcePath);
    const archivePath = path.join(
      resolvedEvidenceDirectory,
      "artifacts",
      relativePath,
    );
    fs.mkdirSync(path.dirname(archivePath), { recursive: true });
    fs.writeFileSync(archivePath, bytes, { flag: "wx" });
    if (!fs.readFileSync(archivePath).equals(bytes)) {
      throw new Error(`listed artifact archive failed readback: ${relativePath}`);
    }
    copiedArtifacts.push({ path: relativePath, sha256: digest(bytes) });
  }

  const receipt = {
    schemaVersion: 1,
    ownerToken: owner.token,
    host: owner.host,
    pid: owner.pid,
    recoveryComplete: missingArtifacts.length === 0,
    qualificationClaimed: false,
    copiedArtifacts,
    archivedBorrowers,
    missingArtifacts,
  };
  const receiptBytes = Buffer.from(`${JSON.stringify(receipt, null, 2)}\n`);
  const receiptPath = path.join(
    resolvedEvidenceDirectory,
    "recovery-receipt.json",
  );
  fs.writeFileSync(receiptPath, receiptBytes, { flag: "wx" });
  if (!fs.readFileSync(receiptPath).equals(receiptBytes)) {
    throw new Error("lock recovery receipt failed readback");
  }

  const currentBytes = fs.readFileSync(lockPath);
  const currentOwner = JSON.parse(currentBytes.toString("utf8"));
  if (!currentBytes.equals(initialBytes) || currentOwner.token !== owner.token) {
    throw new Error("lock owner changed during stale-lock recovery");
  }
  for (const borrower of borrowers) {
    const currentMetadata = fs.lstatSync(borrower.path);
    const currentBorrowerBytes = fs.readFileSync(borrower.path);
    if (
      currentMetadata.isSymbolicLink() ||
      currentMetadata.dev !== borrower.identity.dev ||
      currentMetadata.ino !== borrower.identity.ino ||
      !currentBorrowerBytes.equals(borrower.bytes)
    ) {
      throw new Error("borrower record changed during stale-lock recovery");
    }
  }
  for (const borrower of borrowers) fs.unlinkSync(borrower.path);
  const borrowerDirectory = sidecarBuildBorrowerDirectoryPath(lockPath);
  if (fs.existsSync(borrowerDirectory)) {
    if (fs.readdirSync(borrowerDirectory).length !== 0) {
      throw new Error("borrower registry changed during stale-lock recovery");
    }
    fs.rmdirSync(borrowerDirectory);
  }
  fs.unlinkSync(lockPath);
  return receipt;
}

async function startLockOwner({
  lockPath,
  artifactPaths,
  profile = "qualification",
}) {
  const moduleUrl = new URL("./sidecar-build-contract.mjs", import.meta.url);
  const childSource = [
    `import { acquireSidecarBuildLock } from ${JSON.stringify(moduleUrl.href)};`,
    "const release = acquireSidecarBuildLock({ lockPath: process.env.EMUCHEF_TEST_LOCK_PATH, profile: process.env.EMUCHEF_TEST_PROFILE, targetId: process.env.EMUCHEF_TEST_TARGET_ID, artifactPaths: JSON.parse(process.env.EMUCHEF_TEST_ARTIFACTS) });",
    "process.stdout.write('lock-held\\n');",
    "await new Promise((resolve) => process.stdin.once('data', resolve));",
    "release();",
  ].join("\n");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    env: {
      ...process.env,
      EMUCHEF_TEST_LOCK_PATH: lockPath,
      EMUCHEF_TEST_PROFILE: profile,
      EMUCHEF_TEST_TARGET_ID: TARGET_ID,
      EMUCHEF_TEST_ARTIFACTS: JSON.stringify(artifactPaths),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  let errors = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    errors += chunk;
  });
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      output.removeListener("line", onLine);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onLine = (line) => {
      cleanup();
      if (line === "lock-held") resolve();
      else reject(new Error(`unexpected child output: ${line}`));
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      reject(new Error(`lock owner exited before readiness: ${code}/${signal} ${errors}`));
    };
    output.on("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });
  return { child, output };
}

async function startReleaseBorrower({ repoRoot, lockPath, ownerToken }) {
  const moduleUrl = new URL("./sidecar-build-contract.mjs", import.meta.url);
  const childSource = [
    `import { acquireSidecarBuildLock, assertReleasePackageLock } from ${JSON.stringify(moduleUrl.href)};`,
    "const repoRoot = process.env.EMUCHEF_TEST_REPO_ROOT;",
    "const lockPath = process.env.EMUCHEF_TEST_LOCK_PATH;",
    "assertReleasePackageLock({ repoRoot, env: process.env, lockPath });",
    "const release = acquireSidecarBuildLock({ repoRoot, profile: 'release', inheritedToken: process.env.EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN, lockPath });",
    "process.stdout.write('borrower-held\\n');",
    "await new Promise((resolve) => process.stdin.once('data', resolve));",
    "release();",
  ].join("\n");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    env: {
      ...process.env,
      EMUCHEF_TEST_REPO_ROOT: repoRoot,
      EMUCHEF_TEST_LOCK_PATH: lockPath,
      EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN: ownerToken,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  let errors = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    errors += chunk;
  });
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      output.removeListener("line", onLine);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onLine = (line) => {
      cleanup();
      if (line === "borrower-held") resolve();
      else reject(new Error(`unexpected borrower output: ${line}`));
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      reject(new Error(`borrower exited before readiness: ${code}/${signal} ${errors}`));
    };
    output.on("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });
  return { child, output };
}

async function startVerificationAtBarrier(state, launch) {
  const moduleUrl = new URL("./qualification-session-lifecycle.mjs", import.meta.url);
  const childSource = [
    "import fs from 'node:fs';",
    "const manifest = JSON.parse(process.env.EMUCHEF_TEST_MANIFEST);",
    "const { markQualifiedLaunchVerified } = await import(process.env.EMUCHEF_TEST_SESSION_MODULE);",
    "const verify = () => {",
    "  process.stdout.write('verification-held\\n');",
    "  const byte = Buffer.alloc(1);",
    "  let line = '';",
    "  while (!line.endsWith('\\n')) {",
    "    const count = fs.readSync(0, byte, 0, 1, null);",
    "    if (count === 0) throw new Error('verification barrier input closed');",
    "    line += byte.toString('utf8', 0, count);",
    "  }",
    "  return { manifest };",
    "};",
    "markQualifiedLaunchVerified({ repoRoot: process.env.EMUCHEF_TEST_REPO_ROOT, targetId: process.env.EMUCHEF_TEST_TARGET_ID, token: process.env.EMUCHEF_TEST_SESSION_TOKEN, env: { EMUCHEF_CONFIG_CREATE_QUALIFICATION: '1', EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: process.env.EMUCHEF_TEST_TARGET_ID }, ownerPid: Number(process.env.EMUCHEF_TEST_OWNER_PID), host: process.env.EMUCHEF_TEST_HOST, markerPath: process.env.EMUCHEF_TEST_MARKER_PATH, sessionPath: process.env.EMUCHEF_TEST_SESSION_PATH, lockPath: process.env.EMUCHEF_TEST_LOCK_PATH, verify });",
    "process.stdout.write('verification-complete\\n');",
  ].join("\n");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    env: {
      ...process.env,
      EMUCHEF_TEST_MANIFEST: JSON.stringify(state.manifest),
      EMUCHEF_TEST_SESSION_MODULE: moduleUrl.href,
      EMUCHEF_TEST_REPO_ROOT: state.repoRoot,
      EMUCHEF_TEST_TARGET_ID: TARGET_ID,
      EMUCHEF_TEST_SESSION_TOKEN: launch.token,
      EMUCHEF_TEST_OWNER_PID: String(process.pid),
      EMUCHEF_TEST_HOST: os.hostname(),
      EMUCHEF_TEST_MARKER_PATH: state.markerPath,
      EMUCHEF_TEST_SESSION_PATH: state.sessionPath,
      EMUCHEF_TEST_LOCK_PATH: state.lockPath,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  let errors = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    errors += chunk;
  });
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      output.removeListener("line", onLine);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onLine = (line) => {
      cleanup();
      if (line === "verification-held") resolve();
      else reject(new Error(`unexpected verification output: ${line}`));
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      reject(new Error(`verification exited before barrier: ${code}/${signal} ${errors}`));
    };
    output.on("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });
  return { child, output, errors: () => errors };
}

async function startReleaseAtArchiveBarrier(state) {
  const moduleUrl = new URL("./qualification-state.mjs", import.meta.url);
  const childSource = [
    "import fs from 'node:fs';",
    "import path from 'node:path';",
    "const originalWriteFileSync = fs.writeFileSync;",
    "let paused = false;",
    "fs.writeFileSync = function (filePath, ...args) {",
    "  const writeDir = path.dirname(String(filePath));",
    "  const evidenceDir = process.env.EMUCHEF_TEST_EVIDENCE_DIR;",
    "  const stagePrefix = path.join(path.dirname(evidenceDir), `.${path.basename(evidenceDir)}.partial-`);",
    "  if (!paused && writeDir.startsWith(stagePrefix)) {",
    "    paused = true;",
    "    process.stdout.write('archive-held\\n');",
    "    const byte = Buffer.alloc(1);",
    "    let line = '';",
    "    while (!line.endsWith('\\n')) {",
    "      const count = fs.readSync(0, byte, 0, 1, null);",
    "      if (count === 0) throw new Error('archive barrier input closed');",
    "      line += byte.toString('utf8', 0, count);",
    "    }",
    "  }",
    "  return originalWriteFileSync.call(this, filePath, ...args);",
    "};",
    "const { archiveAndReleaseQualificationState } = await import(process.env.EMUCHEF_TEST_STATE_MODULE);",
    "archiveAndReleaseQualificationState(JSON.parse(process.env.EMUCHEF_TEST_RELEASE_OPTIONS));",
    "process.stdout.write('release-complete\\n');",
  ].join("\n");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    env: {
      ...process.env,
      EMUCHEF_TEST_EVIDENCE_DIR: state.evidenceDirectory,
      EMUCHEF_TEST_STATE_MODULE: moduleUrl.href,
      EMUCHEF_TEST_RELEASE_OPTIONS: JSON.stringify(state.options),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  let errors = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    errors += chunk;
  });
  await new Promise((resolve, reject) => {
    const cleanup = () => {
      output.removeListener("line", onLine);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const onLine = (line) => {
      cleanup();
      if (line === "archive-held") resolve();
      else reject(new Error(`unexpected release output: ${line}`));
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    const onExit = (code, signal) => {
      cleanup();
      reject(new Error(`release exited before archive barrier: ${code}/${signal} ${errors}`));
    };
    output.on("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });
  return { child, output, errors: () => errors };
}

test("completed release archives verified evidence before removing build pins", (t) => {
  const state = fixture(t);

  const result = archiveAndReleaseQualificationState(state.options);
  const receipt = JSON.parse(
    fs.readFileSync(
      path.join(state.evidenceDirectory, "release-receipt.json"),
      "utf8",
    ),
  );

  assert.equal(result.targetId, TARGET_ID);
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.sessionPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(
    sidecarBuildDisposition(
      "debug",
      {},
      fs.existsSync(state.markerPath) || fs.existsSync(state.manifestPath),
    ),
    "build",
  );
  assert.equal(fs.readFileSync(state.externalPath, "utf8"), "external sidecar");
  assert.equal(fs.readFileSync(state.runtimePath, "utf8"), "external sidecar");
  assert.equal(
    fs.readFileSync(
      path.join(state.evidenceDirectory, "qualification-state.json"),
      "utf8",
    ).length > 0,
    true,
  );
  assert.equal(
    fs.readFileSync(
      path.join(state.evidenceDirectory, "retroarch-config-create-sidecar-build.json"),
      "utf8",
    ).length > 0,
    true,
  );
  assert.ok(
    fs.readFileSync(
      path.join(state.evidenceDirectory, "qualification-session.json"),
    ).length > 0,
  );
  assert.deepEqual(
    receipt.archived.map((entry) => entry.sha256),
    [
      digest(
        fs.readFileSync(
          path.join(state.evidenceDirectory, "qualification-state.json"),
        ),
      ),
      digest(
        fs.readFileSync(
          path.join(state.evidenceDirectory, "retroarch-config-create-sidecar-build.json"),
        ),
      ),
digest(
fs.readFileSync(
path.join(state.evidenceDirectory, "qualification-session.json"),
),
),
      digest(
        fs.readFileSync(
          path.join(state.evidenceDirectory, "tauri-external-sidecar.bin"),
        ),
      ),
      digest(
        fs.readFileSync(
          path.join(state.evidenceDirectory, "runtime-sidecar.bin"),
        ),
      ),
    ],
  );
});

test("release refuses an active configuration-create app process and preserves its lease", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.status = "active";
  session.appProcess.pid = process.pid;
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));
  const sessionBytes = fs.readFileSync(state.sessionPath);

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /active configuration-create application session/i,
  );
  assert.equal(fs.readFileSync(state.sessionPath).equals(sessionBytes), true);
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release refuses a live launch owner before Tauri startup", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.status = "verified";
  session.launchOwner.pid = process.pid;
  session.appProcess = null;
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /configuration-create launch is still owned by a live process/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("clean app exit does not release pins while its launcher process is alive", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.launchOwner.pid = process.pid;
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /configuration-create launch owner is still alive/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("abandoned recovery waits for both the app and launch processes to end", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.status = "active";
  session.launchOwner.pid = process.pid;
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));
  const leaseBytes = fs.readFileSync(state.sessionPath);

  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...state.options,
        disposition: "abandoned",
        reason: "Owner confirmed the app ended.",
      }),
    /launch owner is still alive during session recovery/i,
  );
  assert.equal(fs.readFileSync(state.sessionPath).equals(leaseBytes), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release refuses a session lease for a different qualification target", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.qualificationTargetId = "another-target";
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /session lease belongs to a different target/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("finished release requires a native clean-exit session record", (t) => {
  const state = fixture(t);
  fs.unlinkSync(state.sessionPath);

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /finished qualification release requires a clean native session record/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("abandoned release refuses a missing lease after launch was attempted", (t) => {
  const state = fixture(t);
  const marker = JSON.parse(fs.readFileSync(state.markerPath, "utf8"));
  marker.sessionLaunchAttempted = true;
  fs.writeFileSync(state.markerPath, JSON.stringify(marker));
  fs.unlinkSync(state.sessionPath);

  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...state.options,
        disposition: "abandoned",
        reason: "Owner confirmed interrupted qualification.",
      }),
    /session lease is missing after launch was attempted/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("abandoned release can retire a prepared target that never launched", (t) => {
  const state = fixture(t);
  const marker = JSON.parse(fs.readFileSync(state.markerPath, "utf8"));
  marker.sessionLaunchAttempted = false;
  fs.writeFileSync(state.markerPath, JSON.stringify(marker));
  fs.unlinkSync(state.sessionPath);

  const result = archiveAndReleaseQualificationState({
    ...state.options,
    disposition: "abandoned",
    reason: "Qualification was prepared but never launched.",
  });

  assert.equal(result.disposition, "abandoned");
  assert.equal(result.receipt.sessionLeasePresent, false);
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
});

test("failed lease creation leaves prepared state releasable", (t) => {
  const state = fixture(t);
  const marker = JSON.parse(fs.readFileSync(state.markerPath, "utf8"));
  marker.sessionLaunchAttempted = false;
  fs.writeFileSync(state.markerPath, JSON.stringify(marker));
  fs.unlinkSync(state.sessionPath);
  const missingParentSessionPath = path.join(
    path.dirname(state.sessionPath),
    "missing-session-parent",
    "retroarch-config-create-qualification-session.json",
  );

  assert.throws(
    () =>
      beginQualifiedLaunch({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        env: {
          EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
          EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
        },
        markerPath: state.markerPath,
        sessionPath: missingParentSessionPath,
        lockPath: state.lockPath,
      }),
    /ENOENT/,
  );

  assert.equal(
    JSON.parse(fs.readFileSync(state.markerPath, "utf8")).sessionLaunchAttempted,
    false,
  );
  const result = archiveAndReleaseQualificationState({
    ...state.options,
    sessionPath: missingParentSessionPath,
    disposition: "abandoned",
    reason: "The launch lease could not be created before Tauri started.",
  });
  assert.equal(result.disposition, "abandoned");
  assert.equal(result.receipt.sessionLeasePresent, false);
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
});

test("failed launch marker update preserves a lease for guarded recovery", (t) => {
  const state = fixture(t);
  const marker = JSON.parse(fs.readFileSync(state.markerPath, "utf8"));
  marker.sessionLaunchAttempted = false;
  fs.writeFileSync(state.markerPath, JSON.stringify(marker));
  fs.unlinkSync(state.sessionPath);
  const endedOwnerPid = spawnSync(process.execPath, ["-e", ""], {
    encoding: "utf8",
  }).pid;
  const originalRenameSync = fs.renameSync;
  t.after(() => {
    fs.renameSync = originalRenameSync;
  });
  fs.renameSync = (sourcePath, destinationPath) => {
    if (destinationPath === state.markerPath) {
      const error = new Error("injected marker update failure");
      error.code = "EIO";
      throw error;
    }
    return originalRenameSync(sourcePath, destinationPath);
  };

  assert.throws(
    () =>
      beginQualifiedLaunch({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        env: {
          EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
          EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
        },
        ownerPid: endedOwnerPid,
        markerPath: state.markerPath,
        sessionPath: state.sessionPath,
        lockPath: state.lockPath,
      }),
    /injected marker update failure/,
  );
  fs.renameSync = originalRenameSync;

  assert.equal(
    JSON.parse(fs.readFileSync(state.markerPath, "utf8")).sessionLaunchAttempted,
    false,
  );
  const lease = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  assert.equal(lease.status, "launching");
  assert.equal(lease.launchOwner.pid, endedOwnerPid);
  const result = archiveAndReleaseQualificationState({
    ...state.options,
    disposition: "abandoned",
    reason: "The launch marker update failed before Tauri started.",
  });
  assert.equal(result.disposition, "abandoned");
  assert.equal(result.receipt.sessionLeasePresent, true);
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.sessionPath), false);
});

test("abandoned release archives an interrupted session before removing it", (t) => {
  const state = fixture(t);
  const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  session.status = "active";
  fs.writeFileSync(state.sessionPath, JSON.stringify(session));

  const result = archiveAndReleaseQualificationState({
    ...state.options,
    disposition: "abandoned",
    reason: "Owner confirmed the interrupted app process has ended.",
  });

  assert.equal(result.disposition, "abandoned");
  assert.equal(fs.existsSync(state.sessionPath), false);
  const archivedSession = fs.readFileSync(
    path.join(state.evidenceDirectory, "qualification-session.json"),
  );
  assert.equal(archivedSession.toString("utf8"), JSON.stringify(session));
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
});

test("release resumes after interruption during guarded pin retirement", (t) => {
  const state = fixture(t);
  const originalUnlink = fs.unlinkSync;
  let interrupted = false;

  fs.unlinkSync = function (filePath, ...args) {
    const result = originalUnlink.call(fs, filePath, ...args);
    if (path.resolve(filePath) === path.resolve(state.manifestPath)) {
      interrupted = true;
      throw new Error("simulated interruption after manifest removal");
    }
    return result;
  };

  try {
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
      /could not retire retroarch-config-create-sidecar-build.json/i,
    );
  } finally {
    fs.unlinkSync = originalUnlink;
  }

  assert.equal(interrupted, true);
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(fs.existsSync(state.sessionPath), true);

  const result = archiveAndReleaseQualificationState(state.options);

  assert.equal(result.disposition, "finished");
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(fs.existsSync(state.sessionPath), false);
  assert.equal(
    fs.existsSync(path.join(state.evidenceDirectory, "release-receipt.json")),
    true,
  );
});

test("release uses archived lease evidence when interrupted after lease removal", (t) => {
  const state = fixture(t);
  const originalUnlink = fs.unlinkSync;
  let interrupted = false;

  fs.unlinkSync = function (filePath, ...args) {
    const result = originalUnlink.call(fs, filePath, ...args);
    if (path.resolve(filePath) === path.resolve(state.sessionPath)) {
      interrupted = true;
      throw new Error("simulated interruption after lease removal");
    }
    return result;
  };

  try {
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
      /could not retire qualification-session.json/i,
    );
  } finally {
    fs.unlinkSync = originalUnlink;
  }

  assert.equal(interrupted, true);
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(fs.existsSync(state.sessionPath), false);

  const result = archiveAndReleaseQualificationState(state.options);

  assert.equal(result.disposition, "finished");
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(fs.existsSync(state.sessionPath), false);
});

test("release holds the publication lock throughout evidence archival", (t) => {
  const state = fixture(t);
  const originalWriteFileSync = fs.writeFileSync;
  let competingLockError = null;
  let competingRelease = null;
  let attempted = false;

  fs.writeFileSync = function (filePath, ...args) {
    const archiveWriteDirectory = path.dirname(String(filePath));
    const stagingPrefix = path.join(
      path.dirname(state.evidenceDirectory),
      `.${path.basename(state.evidenceDirectory)}.partial-`,
    );
    if (!attempted && archiveWriteDirectory.startsWith(stagingPrefix)) {
      attempted = true;
      try {
        competingRelease = acquireSidecarBuildLock({
          repoRoot: state.repoRoot,
          profile: "release",
          lockPath: state.lockPath,
        });
      } catch (error) {
        competingLockError = error;
      }
    }
    return originalWriteFileSync.call(this, filePath, ...args);
  };

  try {
    archiveAndReleaseQualificationState(state.options);
  } finally {
    fs.writeFileSync = originalWriteFileSync;
    competingRelease?.();
  }

  assert.equal(attempted, true);
  assert.match(competingLockError?.message ?? "", /lock (?:is held|already exists)/i);
  assert.equal(fs.existsSync(state.lockPath), false);
  assert.equal(fs.existsSync(state.markerPath), false);
});

test("launch lease blocks release during verification and before Tauri startup", async (t) => {
  const state = fixture(t);
  fs.unlinkSync(state.sessionPath);
  const marker = JSON.parse(fs.readFileSync(state.markerPath, "utf8"));
  marker.sessionLaunchAttempted = false;
  fs.writeFileSync(state.markerPath, JSON.stringify(marker));
  const launch = beginQualifiedLaunch({
    repoRoot: state.repoRoot,
    targetId: TARGET_ID,
    env: {
      EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
      EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
    },
    markerPath: state.markerPath,
    sessionPath: state.sessionPath,
    lockPath: state.lockPath,
  });
  const holder = await startVerificationAtBarrier(state, launch);
  try {
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
      /lock (?:is held|already exists)/i,
    );
    assert.equal(fs.existsSync(state.sessionPath), true);
  } finally {
    const exited = once(holder.child, "exit");
    holder.child.stdin.end("continue\n");
    const [code, signal] = await exited;
    assert.equal(signal, null, holder.errors());
    assert.equal(code, 0, holder.errors());
    holder.output.close();
  }

  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "verified",
  );
  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /launch is still owned by a live process/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release refuses a mismatched target without changing qualification state", (t) => {
  const state = fixture(t);

  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...state.options,
        targetId: "another-target",
      }),
    /target.*confirmation/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release refuses while a sidecar build holds the publication lock", (t) => {
  const state = fixture(t);
  fs.writeFileSync(state.lockPath, JSON.stringify({ pid: process.pid }));

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /sidecar build lock (?:is held|already exists)/,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release refuses to overwrite an existing evidence directory", (t) => {
  const state = fixture(t);
  fs.mkdirSync(state.evidenceDirectory);
  fs.writeFileSync(path.join(state.evidenceDirectory, "keep.txt"), "preserve");

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /already exists/i,
  );
  assert.equal(
    fs.readFileSync(path.join(state.evidenceDirectory, "keep.txt"), "utf8"),
    "preserve",
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
});

test("release refuses a symlinked state marker and preserves its target", (t) => {
  const state = fixture(t);
  const outsideDirectory = fs.mkdtempSync(
    path.join(os.tmpdir(), "configuration-create-marker-symlink-"),
  );
  const outsideMarkerPath = path.join(
    outsideDirectory,
    "qualification-state.json",
  );
  const markerBytes = fs.readFileSync(state.markerPath);
  t.after(() => fs.rmSync(outsideDirectory, { recursive: true, force: true }));
  fs.writeFileSync(outsideMarkerPath, markerBytes);
  fs.unlinkSync(state.markerPath);
  fs.symlinkSync(outsideMarkerPath, state.markerPath);

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /configuration-create qualification state marker is not a regular file/i,
  );
  assert.equal(fs.readlinkSync(state.markerPath), outsideMarkerPath);
  assert.equal(fs.readFileSync(outsideMarkerPath).equals(markerBytes), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release rejects symlinked artifact parents that redirect within the repository", (t) => {
  for (const [artifactName, artifactPath] of [
    ["external", (state) => state.externalPath],
    ["runtime", (state) => state.runtimePath],
  ]) {
    const state = fixture(t);
    const sourcePath = artifactPath(state);
    const parentPath = path.dirname(sourcePath);
    const redirectedParent = path.join(
      state.repoRoot,
      `${artifactName}-redirected-artifacts`,
    );
    const artifactBytes = fs.readFileSync(sourcePath);

    fs.rmSync(parentPath, { recursive: true, force: true });
    fs.mkdirSync(redirectedParent);
    fs.writeFileSync(
      path.join(redirectedParent, path.basename(sourcePath)),
      artifactBytes,
    );
    fs.symlinkSync(redirectedParent, parentPath, "dir");

    const markerBytes = fs.readFileSync(state.markerPath);
    const manifestBytes = fs.readFileSync(state.manifestPath);
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
      /symlink/i,
      `${artifactName} artifact parent must not redirect through a symlink`,
    );
    assert.equal(fs.readFileSync(state.markerPath).equals(markerBytes), true);
    assert.equal(fs.readFileSync(state.manifestPath).equals(manifestBytes), true);
    assert.equal(fs.existsSync(state.evidenceDirectory), false);
    assert.equal(
      fs
        .readFileSync(path.join(redirectedParent, path.basename(sourcePath)))
        .equals(artifactBytes),
      true,
    );
  }
});

test("release preserves a manifest redirected through a symlinked parent", (t) => {
  const state = fixture(t);
  const outsideDirectory = fs.mkdtempSync(
    path.join(os.tmpdir(), "configuration-create-manifest-symlink-"),
  );
  const outsideManifestPath = path.join(
    outsideDirectory,
    path.basename(state.manifestPath),
  );
  const manifestBytes = fs.readFileSync(state.manifestPath);
  const markerBytes = fs.readFileSync(state.markerPath);
  const manifestParent = path.dirname(state.manifestPath);
  t.after(() => fs.rmSync(outsideDirectory, { recursive: true, force: true }));

  fs.writeFileSync(outsideManifestPath, manifestBytes);
  fs.rmSync(manifestParent, { recursive: true, force: true });
  fs.symlinkSync(outsideDirectory, manifestParent, "dir");

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /symlink/i,
  );
  assert.equal(fs.readFileSync(outsideManifestPath).equals(manifestBytes), true);
  assert.equal(fs.readFileSync(state.markerPath).equals(markerBytes), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release requires the evidence archive to live outside the worktree", (t) => {
  const state = fixture(t);

  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...state.options,
        evidenceDirectory: path.join(state.repoRoot, "evidence"),
      }),
    /outside the repository/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
});

test("release refuses corrupted pinned artifacts and retains the marker", (t) => {
  const state = fixture(t);
  fs.writeFileSync(state.runtimePath, "modified runtime sidecar");

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /digest/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release rejects manifest artifact paths that escape the sidecar directories", (t) => {
  const state = fixture(t);
  const unrelatedBytes = Buffer.from("unrelated in-repository artifact\n");
  const unrelatedPath = path.join(
    state.repoRoot,
    "authored/device_profiles/konkr.pocket_advance.yaml",
  );
  fs.mkdirSync(path.dirname(unrelatedPath), { recursive: true });
  fs.writeFileSync(unrelatedPath, unrelatedBytes);

  const manifest = JSON.parse(fs.readFileSync(state.manifestPath, "utf8"));
  manifest.sidecarPath =
    "apps/emuchef-app/src-tauri/binaries/../../../../authored/device_profiles/konkr.pocket_advance.yaml";
  manifest.runtimeSidecarPath =
    "crates/emuchef-rust-backend/target/debug/../../../../authored/device_profiles/konkr.pocket_advance.yaml";
  manifest.sidecarSha256 = digest(unrelatedBytes);
  manifest.runtimeSidecarSha256 = digest(unrelatedBytes);
  fs.writeFileSync(state.manifestPath, JSON.stringify(manifest));
  const manifestBytes = fs.readFileSync(state.manifestPath);
  const markerBytes = fs.readFileSync(state.markerPath);

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /unexpected artifact path|expected sidecar directory/i,
  );
  assert.equal(fs.readFileSync(state.markerPath).equals(markerBytes), true);
  assert.equal(fs.readFileSync(state.manifestPath).equals(manifestBytes), true);
  assert.equal(fs.readFileSync(unrelatedPath).equals(unrelatedBytes), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("release rejects unexpected sidecar filenames inside the allowed directories", (t) => {
  const state = fixture(t);
  const unrelatedBytes = Buffer.from("unexpected sidecar bytes");
  const unexpectedExternalPath = path.join(
    path.dirname(state.externalPath),
    "emuchef-wrong-target",
  );
  const unexpectedRuntimePath = path.join(
    path.dirname(state.runtimePath),
    "emuchef-wrong-target",
  );
  fs.writeFileSync(unexpectedExternalPath, unrelatedBytes);
  fs.writeFileSync(unexpectedRuntimePath, unrelatedBytes);

  const manifest = JSON.parse(fs.readFileSync(state.manifestPath, "utf8"));
  manifest.sidecarPath = path
    .relative(state.repoRoot, unexpectedExternalPath)
    .split(path.sep)
    .join("/");
  manifest.runtimeSidecarPath = path
    .relative(state.repoRoot, unexpectedRuntimePath)
    .split(path.sep)
    .join("/");
  manifest.sidecarSha256 = digest(unrelatedBytes);
  manifest.runtimeSidecarSha256 = digest(unrelatedBytes);
  fs.writeFileSync(state.manifestPath, JSON.stringify(manifest));
  const manifestBytes = fs.readFileSync(state.manifestPath);
  const markerBytes = fs.readFileSync(state.markerPath);

  assert.throws(
    () => archiveAndReleaseQualificationState(state.options),
    /unexpected artifact path|expected target/i,
  );
  assert.equal(fs.readFileSync(state.markerPath).equals(markerBytes), true);
  assert.equal(fs.readFileSync(state.manifestPath).equals(manifestBytes), true);
  assert.equal(fs.readFileSync(unexpectedExternalPath).equals(unrelatedBytes), true);
  assert.equal(fs.readFileSync(unexpectedRuntimePath).equals(unrelatedBytes), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);
});

test("abandonment requires a reason and exact ended-target confirmation", (t) => {
  const state = fixture(t);
  const abandoned = {
    ...state.options,
    disposition: "abandoned",
  };

  assert.throws(
    () => archiveAndReleaseQualificationState({ ...abandoned, reason: "" }),
    /reason/i,
  );
  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...abandoned,
        confirmEndedTarget: undefined,
        reason: "operator stopped run",
      }),
    /ended-target confirmation/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.manifestPath), true);
});

test("abandoned state without a marker archives the manifest-only build pin", (t) => {
  const state = fixture(t);
  fs.unlinkSync(state.markerPath);

  const result = archiveAndReleaseQualificationState({
    ...state.options,
    disposition: "abandoned",
    reason: "qualification process exited before marker creation",
  });
  const receipt = JSON.parse(
    fs.readFileSync(
      path.join(state.evidenceDirectory, "release-receipt.json"),
      "utf8",
    ),
  );

  assert.equal(result.disposition, "abandoned");
  assert.equal(receipt.stateMarkerPresent, false);
  assert.equal(
    fs.existsSync(
      path.join(state.evidenceDirectory, "qualification-state.json"),
    ),
    false,
  );
  assert.equal(fs.existsSync(state.manifestPath), false);
  assert.equal(fs.existsSync(state.markerPath), false);
});

test("stale-lock recovery retains the lock while a borrowed sidecar hook is active", async (t) => {
  const state = fixture(t);
  const artifactPaths = [
    path.relative(state.repoRoot, state.externalPath),
    path.relative(state.repoRoot, state.runtimePath),
  ];
  const evidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-borrower-recovery`,
  );
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));

  let owner;
  let borrower;
  try {
    owner = await startLockOwner({
      lockPath: state.lockPath,
      artifactPaths,
      profile: "release-package",
    });
    const lockBytes = fs.readFileSync(state.lockPath);
    const ownerRecord = JSON.parse(lockBytes.toString("utf8"));
    borrower = await startReleaseBorrower({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      ownerToken: ownerRecord.token,
    });

    const ownerExited = once(owner.child, "exit");
    owner.child.kill("SIGKILL");
    const [ownerCode, ownerSignal] = await ownerExited;
    assert.equal(ownerCode, null);
    assert.equal(ownerSignal, "SIGKILL");
    owner.output.close();
    owner = undefined;

    let recoveryError;
    try {
      recoverDocumentedStaleLock({
        repoRoot: state.repoRoot,
        lockPath: state.lockPath,
        evidenceDirectory,
      });
    } catch (error) {
      recoveryError = error;
    }
    assert.match(recoveryError?.message ?? "", /borrower/i);
    assert.equal(fs.readFileSync(state.lockPath).equals(lockBytes), true);
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath: state.lockPath, profile: "debug" }),
      /sidecar build lock already exists/,
    );

    borrower.child.stdin.end("finish\\n");
    const [borrowerCode, borrowerSignal] = await once(borrower.child, "exit");
    assert.equal(borrowerCode, 0);
    assert.equal(borrowerSignal, null);
    borrower.output.close();
    borrower = undefined;

    const receipt = recoverDocumentedStaleLock({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      evidenceDirectory,
    });
    assert.equal(receipt.recoveryComplete, true);
    assert.equal(fs.existsSync(state.lockPath), false);
  } finally {
    if (owner?.child.exitCode === null && owner.child.signalCode === null) {
      owner.child.kill("SIGKILL");
    }
    if (borrower?.child.exitCode === null && borrower.child.signalCode === null) {
      borrower.child.kill("SIGKILL");
      await once(borrower.child, "exit");
    }
    owner?.output.close();
    borrower?.output.close();
  }
});

test("stale recovery archives an interrupted borrower after every process exits", async (t) => {
  const state = fixture(t);
  const evidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-interrupted-borrower-evidence`,
  );
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));
  let owner;
  let borrower;
  try {
    owner = await startLockOwner({
      lockPath: state.lockPath,
      artifactPaths: [],
      profile: "release-package",
    });
    const ownerRecord = JSON.parse(fs.readFileSync(state.lockPath, "utf8"));
    borrower = await startReleaseBorrower({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      ownerToken: ownerRecord.token,
    });
    const borrowerDirectory = sidecarBuildBorrowerDirectoryPath(state.lockPath);
    const [recordName] = fs.readdirSync(borrowerDirectory);
    const borrowerBytes = fs.readFileSync(path.join(borrowerDirectory, recordName));
    const borrowerRecord = JSON.parse(borrowerBytes.toString("utf8"));

    const borrowerExited = once(borrower.child, "exit");
    borrower.child.kill("SIGKILL");
    const [borrowerCode, borrowerSignal] = await borrowerExited;
    assert.equal(borrowerCode, null);
    assert.equal(borrowerSignal, "SIGKILL");
    borrower.output.close();
    borrower = undefined;
    assert.equal(
      fs.readFileSync(path.join(borrowerDirectory, recordName)).equals(borrowerBytes),
      true,
    );

    const ownerExited = once(owner.child, "exit");
    owner.child.kill("SIGKILL");
    const [ownerCode, ownerSignal] = await ownerExited;
    assert.equal(ownerCode, null);
    assert.equal(ownerSignal, "SIGKILL");
    owner.output.close();
    owner = undefined;

    const receipt = recoverDocumentedStaleLock({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      evidenceDirectory,
    });
    const archivedRecord = path.join(evidenceDirectory, "borrowers", recordName);
    assert.equal(fs.readFileSync(archivedRecord).equals(borrowerBytes), true);
    assert.equal(receipt.qualificationClaimed, false);
    assert.deepEqual(receipt.archivedBorrowers, [
      { token: borrowerRecord.token, sha256: digest(borrowerBytes) },
    ]);
    assert.equal(fs.existsSync(borrowerDirectory), false);
    assert.equal(fs.existsSync(state.lockPath), false);
  } finally {
    for (const child of [borrower?.child, owner?.child]) {
      if (child && child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await once(child, "exit");
      }
    }
    borrower?.output.close();
    owner?.output.close();
  }
});

test("coordination gate serializes borrower registration against stale recovery", async (t) => {
  const state = fixture(t);
  const evidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-registration-recovery-evidence`,
  );
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));
  const owner = await startLockOwner({
    lockPath: state.lockPath,
    artifactPaths: [],
    profile: "release-package",
  });
  const ownerRecord = JSON.parse(fs.readFileSync(state.lockPath, "utf8"));
  const releaseGate = acquireSidecarBuildCoordinationGate(state.lockPath);
  const contractUrl = new URL("./sidecar-build-contract.mjs", import.meta.url).href;
  const childCode = `
    import { acquireSidecarBuildLock } from ${JSON.stringify(contractUrl)};
    process.stdout.write("registration-attempting\\n");
    try {
      acquireSidecarBuildLock({
        repoRoot: process.env.EMUCHEF_TEST_REPO_ROOT,
        profile: "release",
        lockPath: process.env.EMUCHEF_TEST_LOCK_PATH,
        inheritedToken: process.env.EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN,
      });
      process.stdout.write("registration-unexpectedly-succeeded\\n");
    } catch {
      process.stdout.write("registration-rejected\\n");
    }
  `;
  const borrower = spawn(process.execPath, ["--input-type=module", "--eval", childCode], {
    env: {
      ...process.env,
      EMUCHEF_TEST_REPO_ROOT: state.repoRoot,
      EMUCHEF_TEST_LOCK_PATH: state.lockPath,
      [SIDECAR_BUILD_LOCK_TOKEN_ENV]: ownerRecord.token,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const output = createInterface({ input: borrower.stdout, crlfDelay: Infinity });
  const lines = [];
  const waitForLine = (expected) => new Promise((resolve, reject) => {
    const onLine = (line) => {
      lines.push(line);
      if (line === expected) {
        output.removeListener("line", onLine);
        resolve(line);
      } else if (line.startsWith("registration-unexpectedly")) {
        output.removeListener("line", onLine);
        reject(new Error("borrower registered while recovery held the coordination gate"));
      }
    };
    output.on("line", onLine);
    borrower.once("error", reject);
    borrower.once("exit", (code, signal) => {
      if (!lines.includes(expected)) reject(new Error(`borrower exited before ${expected}: ${code}/${signal}`));
    });
  });
  try {
    await waitForLine("registration-attempting");
    const ownerExited = once(owner.child, "exit");
    owner.child.kill("SIGKILL");
    const [ownerCode, ownerSignal] = await ownerExited;
    assert.equal(ownerCode, null);
    assert.equal(ownerSignal, "SIGKILL");
    owner.output.close();

    const receipt = recoverDocumentedStaleLockUnderGate({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      evidenceDirectory,
    });
    assert.equal(receipt.recoveryComplete, true);
    releaseGate();
    assert.equal(await waitForLine("registration-rejected"), "registration-rejected");
    const [borrowerCode, borrowerSignal] = await once(borrower, "exit");
    assert.equal(borrowerCode, 0);
    assert.equal(borrowerSignal, null);
    assert.equal(fs.existsSync(state.lockPath), false);
    assert.equal(fs.existsSync(sidecarBuildBorrowerDirectoryPath(state.lockPath)), false);
  } finally {
    try {
      releaseGate();
    } catch {}
    if (owner.child.exitCode === null && owner.child.signalCode === null) {
      owner.child.kill("SIGKILL");
      await once(owner.child, "exit");
    }
    if (borrower.exitCode === null && borrower.signalCode === null) {
      borrower.kill("SIGKILL");
      await once(borrower, "exit");
    }
    owner.output.close();
    output.close();
  }
});

test("documented stale-lock recovery preserves evidence before guarded release", async (t) => {
  const state = fixture(t);
  const artifactPaths = [
    path.relative(state.repoRoot, state.externalPath),
    path.relative(state.repoRoot, state.runtimePath),
    "crates/emuchef-rust-backend/target/qualification/interrupted-partial.bin",
  ];
  const originalArtifacts = new Map(
    artifactPaths
      .slice(0, 2)
      .map((relativePath) => [
        relativePath,
        fs.readFileSync(path.join(state.repoRoot, relativePath)),
      ]),
  );
  const otherLockPath = path.join(path.dirname(state.lockPath), "unrelated.lock");
  const otherLockBytes = Buffer.from("another operation owns this file\n");
  fs.writeFileSync(otherLockPath, otherLockBytes);
  const lockEvidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-lock-recovery`,
  );
  const releaseEvidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-post-recovery-release`,
  );
  t.after(() => fs.rmSync(lockEvidenceDirectory, { recursive: true, force: true }));
  t.after(() => fs.rmSync(releaseEvidenceDirectory, { recursive: true, force: true }));

  let holder;
  try {
    holder = await startLockOwner({ lockPath: state.lockPath, artifactPaths });
    const liveLockBytes = fs.readFileSync(state.lockPath);
    const owner = JSON.parse(liveLockBytes.toString("utf8"));
    assert.equal(owner.pid, holder.child.pid);
    assert.equal(owner.host, os.hostname());
    assert.doesNotThrow(() => process.kill(owner.pid, 0));
    assert.throws(
      () => recoverDocumentedStaleLock({
        repoRoot: state.repoRoot,
        lockPath: state.lockPath,
        evidenceDirectory: lockEvidenceDirectory,
      }),
    /active during stale-lock recovery/,
    );
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
    /sidecar build lock (?:is held|already exists)/,
    );
    assert.throws(
      () => acquireSidecarBuildLock({ lockPath: state.lockPath, profile: "debug" }),
      /sidecar build lock already exists/,
    );
    assert.equal(fs.readFileSync(state.lockPath).equals(liveLockBytes), true);
    assert.equal(fs.existsSync(lockEvidenceDirectory), false);

    const exited = once(holder.child, "exit");
    holder.child.kill("SIGKILL");
    const [code, signal] = await exited;
    assert.equal(code, null);
    assert.equal(signal, "SIGKILL");
    holder.output.close();
    holder = undefined;
    assert.throws(() => process.kill(owner.pid, 0), { code: "ESRCH" });

    const receipt = recoverDocumentedStaleLock({
      repoRoot: state.repoRoot,
      lockPath: state.lockPath,
      evidenceDirectory: lockEvidenceDirectory,
    });
    assert.equal(receipt.recoveryComplete, false);
    assert.equal(receipt.qualificationClaimed, false);
    assert.deepEqual(receipt.missingArtifacts, [artifactPaths[2]]);
    assert.equal(
      fs.readFileSync(path.join(lockEvidenceDirectory, "sidecar-build-lock.json")).equals(
        liveLockBytes,
      ),
      true,
    );
    for (const relativePath of artifactPaths.slice(0, 2)) {
      assert.equal(
        fs.readFileSync(
          path.join(lockEvidenceDirectory, "artifacts", relativePath),
        ).equals(originalArtifacts.get(relativePath)),
        true,
      );
      assert.equal(
        fs.readFileSync(path.join(state.repoRoot, relativePath)).equals(
          originalArtifacts.get(relativePath),
        ),
        true,
      );
    }
    assert.equal(fs.existsSync(state.lockPath), false);
    assert.equal(fs.readFileSync(otherLockPath).equals(otherLockBytes), true);

    const released = archiveAndReleaseQualificationState({
      ...state.options,
      evidenceDirectory: releaseEvidenceDirectory,
      disposition: "abandoned",
      reason: "interrupted qualification sidecar build",
    });
    const releaseReceipt = JSON.parse(
      fs.readFileSync(path.join(releaseEvidenceDirectory, "release-receipt.json"), "utf8"),
    );
    assert.equal(released.disposition, "abandoned");
    assert.equal(releaseReceipt.disposition, "abandoned");
    assert.equal(fs.existsSync(state.markerPath), false);
    assert.equal(fs.existsSync(state.manifestPath), false);
    for (const [relativePath, bytes] of originalArtifacts) {
      assert.equal(fs.readFileSync(path.join(state.repoRoot, relativePath)).equals(bytes), true);
    }
  } finally {
    if (holder && holder.child.exitCode === null && holder.child.signalCode === null) {
      const exited = once(holder.child, "exit");
      holder.child.kill("SIGKILL");
      await exited;
      holder.output.close();
    }
  }
});

test("documented stale-lock recovery rejects symlinked artifact parents", async (t) => {
  const state = fixture(t);
  const outsideRoot = fs.mkdtempSync(path.join(os.tmpdir(), "configuration-create-lock-symlink-"));
  const outsideArtifact = path.join(outsideRoot, "artifact.bin");
  const outsideBytes = Buffer.from("artifact outside the worktree\n");
  fs.writeFileSync(outsideArtifact, outsideBytes);
  const linkedParent = path.join(state.repoRoot, "linked-output");
  fs.symlinkSync(outsideRoot, linkedParent, "dir");

  const evidenceDirectory = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-symlink-recovery`,
  );
  t.after(() => fs.rmSync(outsideRoot, { recursive: true, force: true }));
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));

  let holder;
  try {
    holder = await startLockOwner({
      lockPath: state.lockPath,
      artifactPaths: ["linked-output/artifact.bin"],
    });
    const lockBytes = fs.readFileSync(state.lockPath);
    const exited = once(holder.child, "exit");
    holder.child.kill("SIGKILL");
    const [code, signal] = await exited;
    assert.equal(code, null);
    assert.equal(signal, "SIGKILL");
    holder.output.close();
    holder = undefined;

    assert.throws(
      () => recoverDocumentedStaleLock({
        repoRoot: state.repoRoot,
        lockPath: state.lockPath,
        evidenceDirectory,
      }),
      /symbolic link/i,
    );
    assert.equal(fs.readFileSync(state.lockPath).equals(lockBytes), true);
    assert.equal(fs.readFileSync(outsideArtifact).equals(outsideBytes), true);
    assert.equal(fs.existsSync(evidenceDirectory), false);
  } finally {
    if (holder && holder.child.exitCode === null && holder.child.signalCode === null) {
      const exited = once(holder.child, "exit");
      holder.child.kill("SIGKILL");
      await exited;
      holder.output.close();
    }
  }
});

test("documented stale-lock recovery rejects an archive path redirected into the repository", async (t) => {
  const state = fixture(t);
  const evidenceLink = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-evidence-link`,
  );
  fs.symlinkSync(state.repoRoot, evidenceLink, "dir");
  const evidenceDirectory = path.join(evidenceLink, "stale-lock-evidence");
  t.after(() => fs.rmSync(evidenceLink, { force: true }));

  let holder;
  try {
    holder = await startLockOwner({ lockPath: state.lockPath, artifactPaths: [] });
    const lockBytes = fs.readFileSync(state.lockPath);
    const exited = once(holder.child, "exit");
    holder.child.kill("SIGKILL");
    const [code, signal] = await exited;
    assert.equal(code, null);
    assert.equal(signal, "SIGKILL");
    holder.output.close();
    holder = undefined;

    assert.throws(
      () => recoverDocumentedStaleLock({
        repoRoot: state.repoRoot,
        lockPath: state.lockPath,
        evidenceDirectory,
      }),
      /evidence directory must resolve outside the repository/i,
    );
    assert.equal(fs.readFileSync(state.lockPath).equals(lockBytes), true);
    assert.equal(fs.existsSync(path.join(state.repoRoot, "stale-lock-evidence")), false);
  } finally {
    if (holder && holder.child.exitCode === null && holder.child.signalCode === null) {
      const exited = once(holder.child, "exit");
      holder.child.kill("SIGKILL");
      await exited;
      holder.output.close();
    }
  }
});

test("guarded qualification release rejects an archive path redirected into the repository", (t) => {
  const state = fixture(t);
  const evidenceLink = path.join(
    path.dirname(state.repoRoot),
    `${path.basename(state.repoRoot)}-release-evidence-link`,
  );
  fs.symlinkSync(state.repoRoot, evidenceLink, "dir");
  const evidenceDirectory = path.join(evidenceLink, "release-evidence");
  t.after(() => fs.rmSync(evidenceLink, { force: true }));

  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        ...state.options,
        evidenceDirectory,
        disposition: "abandoned",
        reason: "interrupted qualification session",
      }),
    /evidence directory must resolve outside the repository/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(path.join(state.repoRoot, "release-evidence")), false);
});

test("release process barrier excludes launch and competing publishers during archive", async (t) => {
  const state = fixture(t);
  const holder = await startReleaseAtArchiveBarrier(state);
  try {
    assert.throws(
      () =>
        acquireSidecarBuildLock({
          repoRoot: state.repoRoot,
          profile: "release",
          lockPath: state.lockPath,
        }),
      /lock (?:is held|already exists)/i,
    );
    assert.throws(
      () => archiveAndReleaseQualificationState(state.options),
      /lock (?:is held|already exists)/i,
    );
    let cargoCalls = 0;
    assert.throws(
      () =>
        prepareRustSidecar(
          "release",
          {},
          {
            appDir: path.join(state.repoRoot, "apps/emuchef-app"),
            runCommand(program) {
              if (program === "rustc") {
                return {
                  status: 0,
                  stdout: `${execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim()}\n`,
                };
              }
              if (program === "cargo") cargoCalls += 1;
              throw new Error(`unexpected command before lock acquisition: ${program}`);
            },
          },
        ),
      /sidecar build lock (?:is held|already exists)/i,
    );
    assert.equal(cargoCalls, 0);
    assert.throws(
      () =>
        beginQualifiedLaunch({
          repoRoot: state.repoRoot,
          targetId: TARGET_ID,
          env: {
            EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
            EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
          },
          markerPath: state.markerPath,
          sessionPath: state.sessionPath,
          lockPath: state.lockPath,
        }),
      /lock (?:is held|already exists)/i,
    );
    assert.equal(fs.existsSync(state.markerPath), true);
    assert.equal(fs.existsSync(state.sessionPath), true);
  } finally {
    const exited = once(holder.child, "exit");
    holder.child.stdin.end("continue\n");
    const [code, signal] = await exited;
    assert.equal(signal, null, holder.errors());
    assert.equal(code, 0, holder.errors());
    holder.output.close();
  }
  assert.equal(fs.existsSync(state.lockPath), false);
  assert.equal(fs.existsSync(state.markerPath), false);
});

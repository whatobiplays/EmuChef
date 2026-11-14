import { createHash, randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";

import { canonicalDigest } from "../../../tools/device-qualification.mjs";
import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";

export const QUALIFICATION_BACKEND_FEATURES = Object.freeze([
  "real-execution",
  "android-copy-commit-helper",
]);
export const SIDECAR_BUILD_LOCK_TOKEN_ENV = "EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN";

/** Treat dangling links and symlinked parents as present protected state. */
export function pathEntryPresent(filePath) {
  const resolvedPath = path.resolve(filePath);
  let currentPath = resolvedPath;
  while (true) {
    try {
      const metadata = lstatSync(currentPath);
      return currentPath === resolvedPath || metadata.isSymbolicLink();
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
      const parentPath = path.dirname(currentPath);
      if (parentPath === currentPath) return false;
      currentPath = parentPath;
    }
  }
}

/** Reject symlinks and non-file ancestors for a managed worktree artifact. */
export function assertQualificationArtifactPath(
  repoRoot,
  artifactPath,
  { mustExist = false } = {},
) {
  const resolvedRoot = path.resolve(repoRoot);
  const resolvedPath = path.resolve(resolvedRoot, artifactPath);
  const relative = path.relative(resolvedRoot, resolvedPath);
  if (
    !relative ||
    relative === ".." ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  ) {
    throw new Error("qualification artifact path is outside the source worktree");
  }

  const rootMetadata = lstatSync(resolvedRoot);
  if (rootMetadata.isSymbolicLink() || !rootMetadata.isDirectory()) {
    throw new Error("qualification source worktree is not a real directory");
  }

  const components = relative.split(path.sep);
  let currentPath = resolvedRoot;
  for (const [index, component] of components.entries()) {
    currentPath = path.join(currentPath, component);
    let metadata;
    try {
      metadata = lstatSync(currentPath);
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
      if (mustExist) throw new Error("qualification artifact is missing");
      return resolvedPath;
    }
    if (metadata.isSymbolicLink()) {
      throw new Error("qualification artifact path contains a symlink");
    }
    const isLeaf = index === components.length - 1;
    if (isLeaf ? !metadata.isFile() : !metadata.isDirectory()) {
      throw new Error(
        isLeaf
          ? "qualification artifact is not a regular file"
          : "qualification artifact path contains a non-directory",
      );
    }
  }
  return resolvedPath;
}

export function qualificationStateMarkerPath(repoRoot) {
  const gitPath = execFileSync(
    "git",
    ["rev-parse", "--git-path", "emuchef/configuration-create-qualification-state.json"],
    { cwd: repoRoot, encoding: "utf8" },
  ).trim();
  if (!gitPath)
    throw new Error("Git did not return the configuration-create qualification state path");
  return path.resolve(repoRoot, gitPath);
}

export function sidecarBuildLockPath(repoRoot) {
  const gitPath = execFileSync(
    "git",
    ["rev-parse", "--git-path", "emuchef/retroarch-config-create-sidecar-build.lock"],
    { cwd: repoRoot, encoding: "utf8" },
  ).trim();
  if (!gitPath)
    throw new Error("Git did not return configuration-create sidecar build lock path");
  return path.resolve(repoRoot, gitPath);
}

export function sidecarBuildCoordinationGatePath(lockPath) {
  return `${path.resolve(lockPath)}.coordination`;
}

export function sidecarBuildBorrowerDirectoryPath(lockPath) {
  return `${path.resolve(lockPath)}.borrowers`;
}

function sameFileIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function canonicalRelativeArtifactPath(value, label) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    path.isAbsolute(value) ||
    value.includes("\\")
  ) {
    throw new Error(`${label} is not a safe repository-relative path`);
  }
  const components = value.split("/");
  if (components.some((component) => !component || component === "." || component === "..")) {
    throw new Error(`${label} is not a safe repository-relative path`);
  }
  return components.join("/");
}

function assertBorrowerDestinationsWithinOwner(ownerPaths, borrowerPaths) {
  if (!Array.isArray(ownerPaths) || !Array.isArray(borrowerPaths)) {
    throw new Error("sidecar lock artifact scope is malformed");
  }
  const allowedRoots = ownerPaths.map((item) =>
    canonicalRelativeArtifactPath(item, "owner artifact scope"),
  );
  for (const item of borrowerPaths) {
    const destination = canonicalRelativeArtifactPath(item, "borrower artifact path");
    if (!allowedRoots.some((root) => destination === root || destination.startsWith(root + "/"))) {
      throw new Error("borrower artifact path is outside the owner lock scope");
    }
  }
}

function assertRealDirectory(directoryPath, label) {
  const metadata = lstatSync(directoryPath);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error(`${label} is not a real directory`);
  }
  return metadata;
}

function openVerifiedRegularFile(filePath, label) {
  let pathMetadata;
  try {
    pathMetadata = lstatSync(filePath);
  } catch (error) {
    throw new Error(`${label} is unavailable: ${error.message}`);
  }
  if (pathMetadata.isSymbolicLink() || !pathMetadata.isFile()) {
    throw new Error(`${label} is not a regular file`);
  }
  if (typeof fsConstants.O_NOFOLLOW !== "number") {
    throw new Error("the host cannot open sidecar lock records without following links");
  }
  let descriptor;
  try {
    descriptor = openSync(filePath, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
    const descriptorMetadata = fstatSync(descriptor);
    const currentPathMetadata = lstatSync(filePath);
    if (
      !descriptorMetadata.isFile() ||
      currentPathMetadata.isSymbolicLink() ||
      !currentPathMetadata.isFile() ||
      !sameFileIdentity(descriptorMetadata, pathMetadata) ||
      !sameFileIdentity(currentPathMetadata, descriptorMetadata)
    ) {
      throw new Error(`${label} changed while it was opened`);
    }
    if (descriptorMetadata.size > 64 * 1024) {
      throw new Error(`${label} exceeds the supported record size`);
    }
    return {
      descriptor,
      metadata: descriptorMetadata,
      bytes: readFileSync(descriptor),
    };
  } catch (error) {
    if (descriptor !== undefined) closeSync(descriptor);
    throw error;
  }
}

function readVerifiedJsonFile(filePath, label) {
  const opened = openVerifiedRegularFile(filePath, label);
  try {
    const value = JSON.parse(opened.bytes.toString("utf8"));
    return { ...opened, value };
  } catch (error) {
    closeSync(opened.descriptor);
    throw new Error(`${label} is malformed: ${error.message}`);
  }
}

export function acquireSidecarBuildCoordinationGate(lockPath) {
  const absoluteLockPath = path.resolve(lockPath);
  const parentDirectory = path.dirname(absoluteLockPath);
  mkdirSync(parentDirectory, { recursive: true, mode: 0o700 });
  assertRealDirectory(parentDirectory, "sidecar lock directory");

  const gatePath = sidecarBuildCoordinationGatePath(absoluteLockPath);
  const token = randomUUID();
  const owner = {
    schemaVersion: 1,
    kind: "sidecar-build-coordination",
    pid: process.pid,
    host: os.hostname(),
    token,
    startedAt: new Date().toISOString(),
  };
  const temporaryPath = `${gatePath}.tmp-${process.pid}-${token}`;
  writeFileSync(temporaryPath, `${JSON.stringify(owner, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  const openedTemporary = openVerifiedRegularFile(
    temporaryPath,
    "sidecar coordination gate candidate",
  );
  try {
    const deadline = Date.now() + 5000;
    while (true) {
      try {
        linkSync(temporaryPath, gatePath);
        break;
      } catch (error) {
        if (error.code !== "EEXIST") throw error;
        if (Date.now() >= deadline) {
          throw new Error(
            "sidecar coordination gate is busy or stale; preserve its record and verify its owner before recovery",
          );
        }
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5);
      }
    }
    const published = openVerifiedRegularFile(
      gatePath,
      "sidecar coordination gate",
    );
    closeSync(published.descriptor);
    if (!sameFileIdentity(published.metadata, openedTemporary.metadata)) {
      throw new Error("sidecar coordination gate identity changed during publication");
    }
    unlinkSync(temporaryPath);
  } catch (error) {
    closeSync(openedTemporary.descriptor);
    try {
      unlinkSync(temporaryPath);
    } catch (cleanupError) {
      if (cleanupError?.code !== "ENOENT") {
        throw new Error(
          `coordination gate failed and its temporary record could not be removed: ${cleanupError.message}`,
        );
      }
    }
    throw error;
  }

  let released = false;
  const release = () => {
    if (released) return;
    const current = readVerifiedJsonFile(gatePath, "sidecar coordination gate");
    if (
      !sameFileIdentity(current.metadata, openedTemporary.metadata) ||
      current.value.kind !== owner.kind ||
      current.value.token !== token
    ) {
      throw new Error("sidecar coordination gate ownership changed before release");
    }
    unlinkSync(gatePath);
    closeSync(openedTemporary.descriptor);
    released = true;
  };
  Object.defineProperty(release, "token", { value: token });
  return release;
}

function processIdentityState(pid) {
  try {
    process.kill(pid, 0);
    return "present-or-reused";
  } catch (error) {
    if (error?.code === "ESRCH") return "absent";
    return "uncertain";
  }
}

function openFileHolderPids(filePath) {
  let output;
  try {
    output = execFileSync("lsof", ["-t", "--", filePath], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = String(error?.stderr ?? "").trim();
    if (error?.status === 1 && stderr.length === 0) return [];
    throw new Error(
      `cannot establish whether a borrower holds its record descriptor: ${stderr || error.message}`,
    );
  }
  const lines = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.some((line) => !/^[1-9]\d*$/.test(line))) {
    throw new Error("borrower descriptor inspection returned ambiguous process identities");
  }
  return [...new Set(lines.map(Number))];
}

/** Inspect durable borrower records while the coordination gate is held. */
/** Read the registry only while the coordination gate is held. */
export function inspectSidecarBuildBorrowers({
  lockPath,
  owner,
  lockIdentity,
}) {
  const borrowerDirectory = sidecarBuildBorrowerDirectoryPath(lockPath);
  let directoryMetadata;
  try {
    directoryMetadata = lstatSync(borrowerDirectory);
  } catch (error) {
    if (error?.code === "ENOENT") return [];
    throw error;
  }
  if (directoryMetadata.isSymbolicLink() || !directoryMetadata.isDirectory()) {
    throw new Error("sidecar borrower registry is not a real directory");
  }

  const names = readdirSync(borrowerDirectory).sort();
  const borrowers = [];
  for (const name of names) {
    if (!/^[0-9a-f-]{36}\.json$/.test(name)) {
      throw new Error(`sidecar borrower registry contains an unexpected entry: ${name}`);
    }
    const recordPath = path.join(borrowerDirectory, name);
    const opened = openVerifiedRegularFile(recordPath, "sidecar borrower record");
    let record;
    try {
      record = JSON.parse(opened.bytes.toString("utf8"));
    } catch (error) {
      closeSync(opened.descriptor);
      throw new Error(`sidecar borrower record is malformed: ${error.message}`);
    }
    closeSync(opened.descriptor);
    if (
      record?.schemaVersion !== 1 ||
      record.kind !== "sidecar-build-borrower" ||
      typeof record.token !== "string" ||
      name !== `${record.token}.json` ||
      record.ownerToken !== owner.token ||
      record.parentDevice !== lockIdentity.dev ||
      record.parentInode !== lockIdentity.ino ||
      !Array.isArray(record.artifactPaths) ||
      !Number.isSafeInteger(record.pid) ||
      record.pid <= 0 ||
      typeof record.host !== "string" ||
      !record.host ||
      typeof record.startedAt !== "string"
    ) {
      throw new Error("sidecar borrower record does not match its parent lock authority");
    }
    assertBorrowerDestinationsWithinOwner(owner.artifactPaths, record.artifactPaths);
    if (record.host !== os.hostname()) {
      throw new Error("sidecar borrower host identity is uncertain");
    }
    const holders = openFileHolderPids(recordPath);
    const pidState = processIdentityState(record.pid);
    const state = holders.length > 0
      ? "active"
      : pidState === "absent"
        ? "stale"
        : "uncertain";
    borrowers.push({
      path: recordPath,
      name,
      record,
      bytes: opened.bytes,
      identity: opened.metadata,
      holders,
      state,
    });
  }
  return borrowers;
}

function assertNoSidecarBuildBorrowers({ lockPath, owner, lockIdentity }) {
  const borrowers = inspectSidecarBuildBorrowers({ lockPath, owner, lockIdentity });
  if (borrowers.length === 0) return;
  const states = [...new Set(borrowers.map((borrower) => borrower.state))].join(", ");
  throw new Error(
    `sidecar build lock cannot be released while borrower records remain (${states}); preserve and inspect them through stale-lock recovery`,
  );
}

export function requireConfigCreateQualificationTarget(profile, env = process.env) {
  if (profile !== "qualification") return null;
  if (env.EMUCHEF_CONFIG_CREATE_QUALIFICATION !== "1") {
    throw new Error(
      "qualification sidecar requires explicit configuration-create qualification mode",
    );
  }
  return selectConfigCreateQualificationTarget(env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID).qualificationTargetId;
}

/** Serialize sidecar publication across Tauri hooks and qualification setup. */
export function acquireSidecarBuildLock({
  repoRoot,
  profile,
  targetId = null,
  artifactPaths = [],
  lockPath = sidecarBuildLockPath(repoRoot),
  inheritedToken = null,
}) {
  const absoluteLockPath = path.resolve(lockPath);
  const releaseCoordination = acquireSidecarBuildCoordinationGate(absoluteLockPath);
  try {
    const parentDirectory = path.dirname(absoluteLockPath);
    mkdirSync(parentDirectory, { recursive: true, mode: 0o700 });
    assertRealDirectory(parentDirectory, "sidecar lock directory");

    if (inheritedToken !== null) {
      if (profile !== "release") {
        throw new Error("only a release sidecar may borrow a release package lock");
      }
      const parent = readVerifiedJsonFile(absoluteLockPath, "release package lock");
      const owner = parent.value;
      const ownerIdentity = parent.metadata;
      closeSync(parent.descriptor);
      assertReleasePackageLock({
        repoRoot,
        env: { [SIDECAR_BUILD_LOCK_TOKEN_ENV]: inheritedToken },
        lockPath: absoluteLockPath,
      });
      assertBorrowerDestinationsWithinOwner(owner.artifactPaths, artifactPaths);

      const borrowerDirectory = sidecarBuildBorrowerDirectoryPath(absoluteLockPath);
      mkdirSync(borrowerDirectory, { recursive: true, mode: 0o700 });
      assertRealDirectory(borrowerDirectory, "sidecar borrower registry");
      const borrowerToken = randomUUID();
      const borrower = {
        schemaVersion: 1,
        kind: "sidecar-build-borrower",
        pid: process.pid,
        host: os.hostname(),
        token: borrowerToken,
        ownerToken: inheritedToken,
        parentDevice: ownerIdentity.dev,
        parentInode: ownerIdentity.ino,
        artifactPaths,
        startedAt: new Date().toISOString(),
      };
      const temporaryPath = path.join(
        borrowerDirectory,
        ".tmp-" + process.pid + "-" + borrowerToken,
      );
      const borrowerPath = path.join(borrowerDirectory, borrowerToken + ".json");
      writeFileSync(temporaryPath, JSON.stringify(borrower, null, 2) + "\n", {
        flag: "wx",
        mode: 0o600,
      });
      const lease = openVerifiedRegularFile(
        temporaryPath,
        "sidecar borrower record candidate",
      );
      try {
        linkSync(temporaryPath, borrowerPath);
        const published = openVerifiedRegularFile(
          borrowerPath,
          "sidecar borrower record",
        );
        closeSync(published.descriptor);
        if (!sameFileIdentity(lease.metadata, published.metadata)) {
          throw new Error("sidecar borrower record identity changed during publication");
        }
        unlinkSync(temporaryPath);
        const currentParent = readVerifiedJsonFile(
          absoluteLockPath,
          "release package lock",
        );
        try {
          if (
            !sameFileIdentity(currentParent.metadata, ownerIdentity) ||
            currentParent.value.token !== inheritedToken ||
            currentParent.value.profile !== "release-package"
          ) {
            throw new Error("release package lock changed during borrower registration");
          }
        } finally {
          closeSync(currentParent.descriptor);
        }
      } catch (error) {
        closeSync(lease.descriptor);
        throw error;
      }

      let released = false;
      const releaseBorrower = () => {
        if (released) return;
        const releaseGate = acquireSidecarBuildCoordinationGate(absoluteLockPath);
        try {
          const currentParent = readVerifiedJsonFile(
            absoluteLockPath,
            "release package lock",
          );
          try {
            if (
              !sameFileIdentity(currentParent.metadata, ownerIdentity) ||
              currentParent.value.token !== inheritedToken
            ) {
              throw new Error("release package lock changed before borrower release");
            }
          } finally {
            closeSync(currentParent.descriptor);
          }
          const currentBorrower = readVerifiedJsonFile(
            borrowerPath,
            "sidecar borrower record",
          );
          try {
            if (
              !sameFileIdentity(currentBorrower.metadata, lease.metadata) ||
              currentBorrower.value.token !== borrowerToken ||
              currentBorrower.value.ownerToken !== inheritedToken
            ) {
              throw new Error("sidecar borrower record changed before release");
            }
          } finally {
            closeSync(currentBorrower.descriptor);
          }
          unlinkSync(borrowerPath);
          closeSync(lease.descriptor);
          released = true;
        } finally {
          releaseGate();
        }
      };
      Object.defineProperties(releaseBorrower, {
        token: { value: borrowerToken },
        leaseFileDescriptor: { value: lease.descriptor },
        leasePath: { value: borrowerPath },
      });
      return releaseBorrower;
    }

    if (pathEntryPresent(absoluteLockPath)) {
      throw new Error("sidecar build lock already exists; preserve and inspect its owner record");
    }
    const token = randomUUID();
    const owner = {
      schemaVersion: 1,
      pid: process.pid,
      host: os.hostname(),
      token,
      profile,
      qualificationTargetId: targetId,
      artifactPaths,
      startedAt: new Date().toISOString(),
    };
    const temporaryPath = absoluteLockPath + ".tmp-" + process.pid + "-" + token;
    writeFileSync(temporaryPath, JSON.stringify(owner, null, 2) + "\n", {
      flag: "wx",
      mode: 0o600,
    });
    const ownerFile = openVerifiedRegularFile(
      temporaryPath,
      "sidecar build lock candidate",
    );
    let temporaryLinkNeedsCleanup = false;
    try {
      linkSync(temporaryPath, absoluteLockPath);
      const published = openVerifiedRegularFile(
        absoluteLockPath,
        "sidecar build lock",
      );
      closeSync(published.descriptor);
      if (!sameFileIdentity(ownerFile.metadata, published.metadata)) {
        throw new Error("sidecar build lock identity changed during publication");
      }
      try {
        unlinkSync(temporaryPath);
      } catch {
        temporaryLinkNeedsCleanup = true;
      }
    } catch (error) {
      closeSync(ownerFile.descriptor);
      try {
        unlinkSync(temporaryPath);
      } catch (cleanupError) {
        if (cleanupError?.code !== "ENOENT") {
          throw new Error(
            "sidecar build lock publication failed and its temporary record could not be removed: " +
              cleanupError.message,
          );
        }
      }
      if (error?.code === "EEXIST") {
        throw new Error("sidecar build lock already exists; preserve and inspect its owner record");
      }
      throw error;
    }

    let released = false;
    const releaseOwner = () => {
      if (released) return;
      const releaseGate = acquireSidecarBuildCoordinationGate(absoluteLockPath);
      try {
        const current = readVerifiedJsonFile(absoluteLockPath, "sidecar build lock");
        try {
          if (
            !sameFileIdentity(current.metadata, ownerFile.metadata) ||
            current.value.token !== token
          ) {
            throw new Error("sidecar build lock ownership changed before release");
          }
        } finally {
          closeSync(current.descriptor);
        }
        assertNoSidecarBuildBorrowers({
          lockPath: absoluteLockPath,
          owner,
          lockIdentity: ownerFile.metadata,
        });
        unlinkSync(absoluteLockPath);
        closeSync(ownerFile.descriptor);
        released = true;
        if (temporaryLinkNeedsCleanup) {
          try {
            const temporary = readVerifiedJsonFile(
              temporaryPath,
              "temporary sidecar build lock record",
            );
            try {
              if (
                !sameFileIdentity(temporary.metadata, ownerFile.metadata) ||
                temporary.value.token !== token
              ) {
                throw new Error("temporary sidecar build lock record changed before cleanup");
              }
            } finally {
              closeSync(temporary.descriptor);
            }
            unlinkSync(temporaryPath);
          } catch (error) {
            if (error?.code !== "ENOENT") {
              process.emitWarning(
                "could not remove temporary sidecar build lock record after release: " +
                  error.message,
              );
            }
          }
        }
      } finally {
        releaseGate();
      }
    };
    Object.defineProperties(releaseOwner, {
      token: { value: token },
      lockIdentity: { value: ownerFile.metadata },
      lockFileDescriptor: { value: ownerFile.descriptor },
    });
    return releaseOwner;
  } finally {
    releaseCoordination();
  }
}

export function assertReleasePackageLock({
  repoRoot,
  env = process.env,
  lockPath = sidecarBuildLockPath(repoRoot),
}) {
  const token = env[SIDECAR_BUILD_LOCK_TOKEN_ENV];
  if (typeof token !== "string" || token.length === 0) {
    throw new Error("release packaging requires the Tauri wrapper lock token");
  }
  const opened = openVerifiedRegularFile(lockPath, "release package lock");
  let owner;
  try {
    owner = JSON.parse(opened.bytes.toString("utf8"));
  } catch (error) {
    closeSync(opened.descriptor);
    throw new Error("release package lock is malformed: " + error.message);
  }
  closeSync(opened.descriptor);
  if (
    owner?.schemaVersion !== 1 ||
    owner.profile !== "release-package" ||
    owner.token !== token ||
    !Number.isSafeInteger(owner.pid) ||
    owner.pid <= 0 ||
    owner.host !== os.hostname()
  ) {
    throw new Error("release package lock ownership does not match this Tauri build");
  }
  try {
    process.kill(owner.pid, 0);
  } catch (error) {
    if (error?.code === "ESRCH") {
      throw new Error("release package lock owner process has ended");
    }
    if (error?.code !== "EPERM") {
      throw new Error(
        "release package lock owner process state is uncertain: " + error.message,
      );
    }
  }
  return owner;
}

/** Read the owner record and identify processes that still hold its file. */
export function inspectSidecarBuildLockOwner({ lockPath }) {
  const opened = readVerifiedJsonFile(lockPath, "sidecar build lock");
  const owner = opened.value;
  const identity = opened.metadata;
  closeSync(opened.descriptor);
  if (
    owner?.schemaVersion !== 1 ||
    !Number.isSafeInteger(owner.pid) ||
    owner.pid <= 0 ||
    owner.host !== os.hostname() ||
    typeof owner.token !== "string" ||
    !owner.token
  ) {
    throw new Error("sidecar build lock owner record is malformed or belongs to another host");
  }
  const holders = openFileHolderPids(path.resolve(lockPath));
  const pidState = processIdentityState(owner.pid);
  return {
    owner,
    identity,
    holders,
    state: holders.length > 0
      ? "active"
      : pidState === "absent"
        ? "stale"
        : "uncertain",
  };
}

export async function withReleasePackageLock({
  repoRoot,
  env = process.env,
  artifactPaths = [],
  run,
}) {
  if (typeof run !== "function") {
    throw new Error("release package lock requires a build operation");
  }
  const release = acquireSidecarBuildLock({
    repoRoot,
    profile: "release-package",
    artifactPaths,
  });
  const packageEnv = {
    ...env,
    [SIDECAR_BUILD_LOCK_TOKEN_ENV]: release.token,
  };
  try {
    return await run(packageEnv, {
      lockFileDescriptor: release.lockFileDescriptor,
    });
  } finally {
    release();
  }
}

export function sidecarBuildDisposition(
  profile,
  env = process.env,
  qualificationStatePresent = false,
) {
  if (profile === "qualification" && qualificationStatePresent) {
    return "refuse-qualification-build";
  }
  if (profile === "qualification") return "build";
  if (
    profile === "release" &&
    (env.EMUCHEF_CONFIG_CREATE_QUALIFICATION === "1" ||
      qualificationStatePresent)
  ) {
    return "refuse-release-qualification";
  }
  if (env.EMUCHEF_CONFIG_CREATE_QUALIFICATION === "1") {
    return "verify-existing";
  }
  if (qualificationStatePresent) return "refuse-ordinary-build";
  return "build";
}

const CONFIG_CREATE_MATERIAL_ROOTS = [
  "authored/",
  "apps/emuchef-app/",
  "crates/emuchef-rust-backend/",
  "scripts/sidecar-target-contract.mjs",
  "tools/device-qualification.mjs",
  "docs/qualification/retroarch-config-create/target.json",
];

export function retroarchConfigCreateMaterialInputs(repoRoot) {
  const paths = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { cwd: repoRoot, encoding: "utf8" },
  )
    .split("\0")
    .filter((relativePath) => {
      if (!relativePath) return false;
      if (
        [
          "apps/emuchef-app/node_modules/",
          "apps/emuchef-app/dist/",
          "apps/emuchef-app/src-tauri/target/",
          "apps/emuchef-app/src-tauri/binaries/",
        ].some((root) => relativePath.startsWith(root))
      )
        return false;
      return CONFIG_CREATE_MATERIAL_ROOTS.some((root) => relativePath.startsWith(root));
    })
    .sort((left, right) => left.localeCompare(right));
  const entries = paths.map((relativePath) => ({
    path: relativePath,
    sha256: createHash("sha256")
      .update(readFileSync(path.join(repoRoot, relativePath)))
      .digest("hex"),
  }));
  return {
    paths,
    digest: canonicalDigest(entries),
  };
}

export function sidecarCargoBuildArgs(profile, manifest) {
  const args = ["build", "--manifest-path", manifest, "--bin", "emuchef"];
  if (profile === "debug") return args;
  if (profile === "release") {
    return [...args, "--release", "--features", "android-copy-commit-helper"];
  }
  if (profile === "qualification") {
    return [
      ...args,
      "--profile",
      "qualification",
      "--features",
      QUALIFICATION_BACKEND_FEATURES.join(","),
    ];
  }
  throw new Error(`unsupported sidecar profile: ${profile}`);
}

export function validateQualificationSidecarProvenance(
  value,
  { sourceIdentity = null, materialInputsDigest = null } = {},
) {
  if (!value || value.schemaVersion !== 1) {
    throw new Error("qualification sidecar provenance schema is unsupported");
  }
  if (
    JSON.stringify(value.backendFeatures) !==
    JSON.stringify(QUALIFICATION_BACKEND_FEATURES)
  ) {
    throw new Error(
      "qualification sidecar does not report the required backend features",
    );
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(value.arm64HelperSha256 ?? "")) {
    throw new Error(
      "qualification sidecar did not report a valid embedded ARM64 helper digest",
    );
  }
  if (
    !/^[0-9a-f]{40}$/.test(value.gitCommit ?? "") ||
    !/^sha256:[0-9a-f]{64}$/.test(value.materialBuildDigest ?? "") ||
    !/^sha256:[0-9a-f]{64}$/.test(value.materialInputsDigest ?? "")
  ) {
    throw new Error(
      "qualification sidecar did not report its compiled source identity",
    );
  }
  if (
    sourceIdentity !== null &&
    (value.gitCommit !== sourceIdentity.gitCommit ||
      value.materialBuildDigest !== sourceIdentity.materialBuildDigest)
  ) {
    throw new Error(
      "compiled qualification source identity differs from the requested build inputs",
    );
  }
  if (
    materialInputsDigest !== null &&
    value.materialInputsDigest !== materialInputsDigest
  ) {
    throw new Error(
      "compiled qualification material inputs differ from the requested build inputs",
    );
  }
  return value;
}

export function validateQualificationSidecarManifest(
  manifest,
  { sourceIdentity, materialInputsDigest, sidecarSha256, provenance, targetId },
) {
  if (manifest?.schemaVersion !== 1 || manifest.sourceWorktreeClean !== true) {
    throw new Error(
      "qualification sidecar was not built from a clean source worktree",
    );
  }
  if (
    manifest.gitCommit !== sourceIdentity.gitCommit ||
    manifest.materialBuildDigest !== sourceIdentity.materialBuildDigest ||
    manifest.materialInputsDigest !== materialInputsDigest
  ) {
    throw new Error(
      "qualification sidecar source identity does not match the current checkout",
    );
  }
  if (manifest.qualificationTargetId !== targetId) {
    throw new Error(
      "qualification sidecar target identity does not match the explicit selection",
    );
  }
  if (manifest.sidecarSha256 !== sidecarSha256) {
    throw new Error(
      "Tauri external binary changed after qualification sidecar preparation",
    );
  }
  const checkedProvenance = validateQualificationSidecarProvenance(
    provenance,
    { sourceIdentity, materialInputsDigest },
  );
  if (manifest.arm64HelperSha256 !== checkedProvenance.arm64HelperSha256) {
    throw new Error(
      "embedded ARM64 helper digest differs from the recorded build identity",
    );
  }
  if (
    JSON.stringify(manifest.backendFeatures) !==
    JSON.stringify(checkedProvenance.backendFeatures)
  ) {
    throw new Error(
      "compiled backend features differ from the recorded build identity",
    );
  }
  return manifest;
}

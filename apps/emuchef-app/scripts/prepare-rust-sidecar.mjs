#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  targetExecutableExtension,
  validateTargetTriple,
} from "../../../scripts/sidecar-target-contract.mjs";
import { buildMaterialIdentity } from "../../../tools/device-qualification.mjs";
import {
  BINARY_BASENAME,
  externalBinArtifactName,
} from "./sidecar-packaging.mjs";
import { verifyQualificationSidecar } from "./verify-qualification-sidecar.mjs";
import {
  sidecarBuildDisposition,
  sidecarCargoBuildArgs,
  retroarchConfigCreateMaterialInputs,
  assertQualificationArtifactPath,
  validateQualificationSidecarProvenance,
  acquireSidecarBuildLock,
  assertReleasePackageLock,
  pathEntryPresent,
  qualificationStateMarkerPath,
  requireConfigCreateQualificationTarget,
  SIDECAR_BUILD_LOCK_TOKEN_ENV,
} from "./sidecar-build-contract.mjs";
import {
  qualificationSessionLeasePresent,
} from "./qualification-session-lifecycle.mjs";

const DEFAULT_APP_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

function sidecarDigest(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function removeQualificationStagingPaths(paths, label, cause) {
  const cleanupErrors = [];
  for (const stagingPath of paths) {
    try {
      fs.rmSync(stagingPath, { force: true });
    } catch (error) {
      if (error.code !== "ENOENT") {
        cleanupErrors.push(`${stagingPath}: ${error.message}`);
      }
    }
  }

  if (cleanupErrors.length > 0) {
    const error = new Error(
      `${label} cleanup failed; preserve build lock and artifacts: ${cleanupErrors.join("; ")}`,
      { cause },
    );
    error.preserveBuildLock = true;
    throw error;
  }
}

function removeStagedArtifacts(stagedArtifacts, label, cause) {
  const paths = stagedArtifacts.flatMap((artifact) =>
    artifact.hadOriginal
      ? [artifact.stagedPath, artifact.backupPath]
      : [artifact.stagedPath],
  );
  removeQualificationStagingPaths(paths, label, cause);
}

function stageSidecar({ repoRoot, source, destination, token, extension }) {
  assertQualificationArtifactPath(repoRoot, destination);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  assertQualificationArtifactPath(repoRoot, destination);

  const stagedPath = `${destination}.tmp-${token}`;
  const backupPath = `${destination}.tmp-${token}.backup`;
  const hadOriginal = pathEntryPresent(destination);
  if (hadOriginal) {
    assertQualificationArtifactPath(repoRoot, destination, { mustExist: true });
  }
  try {
    if (hadOriginal) {
      fs.copyFileSync(destination, backupPath, fs.constants.COPYFILE_EXCL);
    }
    fs.copyFileSync(source, stagedPath, fs.constants.COPYFILE_EXCL);
    if (extension !== ".exe") fs.chmodSync(stagedPath, 0o755);
    assertQualificationArtifactPath(repoRoot, stagedPath, { mustExist: true });
    return {
      backupPath,
      destination,
      hadOriginal,
      stagedBytes: fs.readFileSync(stagedPath),
      stagedPath,
    };
  } catch (error) {
    removeQualificationStagingPaths(
      hadOriginal ? [stagedPath, backupPath] : [stagedPath],
      "qualification sidecar staging",
      error,
    );
    throw error;
  }
}

function assertSidecarBuildAllowed(profile, env, statePresent) {
  const disposition = sidecarBuildDisposition(profile, env, statePresent);
  if (disposition === "refuse-release-qualification") {
    throw new Error(
      "release sidecar packaging cannot select a configuration-create qualification artifact; clear the configuration-create environment and archive/release pinned qualification state before building a normal release sidecar",
    );
  }
  if (
    disposition === "refuse-ordinary-build" ||
    disposition === "refuse-qualification-build"
  ) {
    throw new Error(
      "configuration-create qualification artifacts are pinned; use the explicit configuration-create Tauri command or archive and release the state with device-qualification:retroarch-config-create:release-state before rebuilding",
    );
  }
  return disposition;
}

export function assertReleaseSidecarBuildAllowed({ repoRoot, env = process.env }) {
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  const statePath = qualificationStateMarkerPath(repoRoot);
  const statePresent =
    pathEntryPresent(manifestPath) ||
    pathEntryPresent(statePath) ||
    qualificationSessionLeasePresent(repoRoot, statePath);
  return assertSidecarBuildAllowed(
    "release",
    env,
    statePresent,
  );
}

function createOwnedFileNoClobber(filePath, bytes, label) {
  const temporaryPath = `${filePath}.tmp-${process.pid}-${randomUUID()}`;
  let descriptor = null;
  let published = false;
  let identity = null;
  try {
    descriptor = fs.openSync(temporaryPath, "wx", 0o600);
    fs.writeFileSync(descriptor, bytes);
    fs.fsyncSync(descriptor);
    const metadata = fs.fstatSync(descriptor);
    if (!metadata.isFile()) {
      throw new Error(`${label} staging entry is not a regular file`);
    }
    identity = { dev: metadata.dev, ino: metadata.ino };
    fs.closeSync(descriptor);
    descriptor = null;

    // Hard-link publication is atomic and fails if another owner created the pin.
    fs.linkSync(temporaryPath, filePath);
    published = true;
    return identity;
  } finally {
    if (descriptor !== null) fs.closeSync(descriptor);
    try {
      fs.unlinkSync(temporaryPath);
    } catch (error) {
      if (error.code !== "ENOENT" && !published) {
        const cleanupError = new Error(
          `${label} staging file could not be removed: ${error.message}`,
          { cause: error },
        );
        cleanupError.preserveBuildLock = true;
        throw cleanupError;
      }
    }
  }
}

function removeOwnedFile(filePath, expectedBytes, expectedIdentity, label) {
  if (!pathEntryPresent(filePath)) return;
  const metadata = fs.lstatSync(filePath);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`${label} changed type during qualification publication`);
  }
  if (
    metadata.dev !== expectedIdentity.dev ||
    metadata.ino !== expectedIdentity.ino
  ) {
    throw new Error(`${label} ownership changed during qualification publication`);
  }
  if (!fs.readFileSync(filePath).equals(expectedBytes)) {
    throw new Error(`${label} changed during qualification publication`);
  }
  fs.unlinkSync(filePath);
}

function publishQualificationArtifacts({
  repoRoot,
  destination,
  runtimeDestination,
  source,
  extension,
  targetId,
  identity,
  materialInputs,
  sourceState,
  backendProvenance,
  manifestPath,
  statePath,
}) {
  const token = randomUUID();
  const stagedArtifacts = [];
  try {
    stagedArtifacts.push(
      stageSidecar({ repoRoot, source, destination, token, extension }),
    );
    stagedArtifacts.push(
      stageSidecar({
        repoRoot,
        source,
        destination: runtimeDestination,
        token,
        extension,
      }),
    );
  } catch (error) {
    removeStagedArtifacts(stagedArtifacts, "qualification sidecar staging", error);
    throw error;
  }
  const sidecarSha256 = sidecarDigest(stagedArtifacts[0].stagedBytes);
  const runtimeSidecarSha256 = sidecarDigest(stagedArtifacts[1].stagedBytes);
  if (runtimeSidecarSha256 !== sidecarSha256) {
    const error = new Error(
      "runtime sidecar staging copy differs from the Tauri binary",
    );
    removeStagedArtifacts(stagedArtifacts, "qualification sidecar staging", error);
    throw error;
  }

  const manifest = {
    schemaVersion: 1,
    qualificationTargetId: targetId,
    gitCommit: identity.gitCommit,
    materialBuildDigest: identity.materialBuildDigest,
    materialInputsDigest: materialInputs.digest,
    sourceWorktreeClean: sourceState.stdout.length === 0,
    backendFeatures: backendProvenance.backendFeatures,
    arm64HelperSha256: backendProvenance.arm64HelperSha256,
    sidecarSha256,
    sidecarPath: path.relative(repoRoot, destination).split(path.sep).join("/"),
    runtimeSidecarSha256,
    runtimeSidecarPath: path
      .relative(repoRoot, runtimeDestination)
      .split(path.sep)
      .join("/"),
  };
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  const stateBytes = Buffer.from(
    `${JSON.stringify(
      {
        schemaVersion: 1,
        qualificationTargetId: targetId,
        manifestPath: path.relative(repoRoot, manifestPath).split(path.sep).join("/"),
        sessionLaunchAttempted: false,
      },
      null,
      2,
    )}\n`,
  );
  const publishedArtifacts = [];
  let manifestOwnership = null;
  let stateOwnership = null;

  try {
    for (const artifact of stagedArtifacts) {
      fs.renameSync(artifact.stagedPath, artifact.destination);
      publishedArtifacts.push(artifact);
      assertQualificationArtifactPath(repoRoot, artifact.destination, {
        mustExist: true,
      });
    }

    assertQualificationArtifactPath(repoRoot, manifestPath);
    fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
    assertQualificationArtifactPath(repoRoot, manifestPath);
    manifestOwnership = createOwnedFileNoClobber(
      manifestPath,
      manifestBytes,
      "qualification sidecar manifest",
    );
    assertQualificationArtifactPath(repoRoot, manifestPath, { mustExist: true });

    fs.mkdirSync(path.dirname(statePath), { recursive: true });
    stateOwnership = createOwnedFileNoClobber(
      statePath,
      stateBytes,
      "qualification state marker",
    );
    assertQualificationArtifactPath(repoRoot, statePath, { mustExist: true });

  } catch (cause) {
    const rollbackErrors = [];
    for (const artifact of [...publishedArtifacts].reverse()) {
      try {
        const current = fs.readFileSync(artifact.destination);
        if (!current.equals(artifact.stagedBytes)) {
          throw new Error("published sidecar changed before rollback");
        }
        if (artifact.hadOriginal) {
          fs.renameSync(artifact.backupPath, artifact.destination);
        } else {
          fs.unlinkSync(artifact.destination);
        }
      } catch (error) {
        rollbackErrors.push(error.message);
      }
    }
    try {
      if (stateOwnership) {
        removeOwnedFile(
          statePath,
          stateBytes,
          stateOwnership,
          "qualification state marker",
        );
      }
      if (manifestOwnership) {
        removeOwnedFile(
          manifestPath,
          manifestBytes,
          manifestOwnership,
          "qualification sidecar manifest",
        );
      }
    } catch (error) {
      rollbackErrors.push(error.message);
    }
    if (rollbackErrors.length === 0) {
      removeStagedArtifacts(
        stagedArtifacts,
        "qualification sidecar rollback staging",
        cause,
      );
    } else {
      const error = new Error(
        `qualification sidecar publication failed and rollback is incomplete: ${rollbackErrors.join("; ")}; preserve the build lock and artifacts`,
        { cause },
      );
      error.preserveBuildLock = true;
      throw error;
    }
    throw new Error(`qualification sidecar publication failed: ${cause.message}`, {
      cause,
    });
  }

  for (const artifact of stagedArtifacts) {
    if (artifact.hadOriginal) {
      try {
        fs.rmSync(artifact.backupPath, { force: true });
      } catch {
        // The completed qualification pins keep any leftover backup protected.
      }
    }
  }

  return { sidecarSha256, runtimeSidecarSha256 };
}

export function prepareRustSidecar(
  profile,
  env = process.env,
  {
    appDir = DEFAULT_APP_DIR,
    runCommand = spawnSync,
    verify = verifyQualificationSidecar,
    buildIdentity = buildMaterialIdentity,
    readMaterialInputs = retroarchConfigCreateMaterialInputs,
  } = {},
) {
  if (!new Set(["debug", "release", "qualification"]).has(profile)) {
    throw new Error("--profile must be debug, release, or qualification");
  }
  const repoRoot = path.resolve(appDir, "../..");
  const targetId = requireConfigCreateQualificationTarget(profile, env);
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/Cargo.toml",
  );
  const targetResult = runCommand("rustc", ["--print", "host-tuple"], {
    encoding: "utf8",
  });
  if (targetResult.error || targetResult.status !== 0) {
    throw new Error("rustc could not report its host target");
  }
  const target = validateTargetTriple(targetResult.stdout.trim());
  const extension = targetExecutableExtension(target);
  const destination = path.join(
    appDir,
    "src-tauri/binaries",
    externalBinArtifactName(target),
  );
  const runtimeDestination = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug",
    `${BINARY_BASENAME}${extension}`,
  );
  const qualificationManifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  const qualificationStatePath = qualificationStateMarkerPath(repoRoot);
  const inheritedToken = env[SIDECAR_BUILD_LOCK_TOKEN_ENV] ?? null;
  const releaseBuildLock = acquireSidecarBuildLock({
    repoRoot,
    profile,
    targetId,
    inheritedToken,
    artifactPaths: [
      path.relative(repoRoot, destination).split(path.sep).join("/"),
      ...(profile === "qualification"
        ? [
            path
              .relative(repoRoot, runtimeDestination)
              .split(path.sep)
              .join("/"),
          ]
        : []),
    ],
  });

  let preserveBuildLock = false;
  try {
    const statePresent =
      pathEntryPresent(qualificationManifestPath) ||
      pathEntryPresent(qualificationStatePath) ||
      qualificationSessionLeasePresent(repoRoot, qualificationStatePath);
    const disposition = assertSidecarBuildAllowed(profile, env, statePresent);
    if (disposition === "verify-existing") {
      const verified = verify({ env });
      console.log(
        `Verified existing configuration-create qualification sidecar: ${verified.sidecarPath}`,
      );
      return;
    }

    let qualificationSourceIdentity = null;
    let qualificationMaterialInputs = null;
    if (profile === "qualification") {
      qualificationSourceIdentity = buildIdentity({
        repoRoot,
        requireClean: true,
      });
      qualificationMaterialInputs = readMaterialInputs(repoRoot);
      const sourceStateBeforeBuild = runCommand(
        "git",
        ["status", "--porcelain", "--untracked-files=all"],
        { cwd: repoRoot, encoding: "utf8" },
      );
      if (
        sourceStateBeforeBuild.error ||
        sourceStateBeforeBuild.status !== 0 ||
        sourceStateBeforeBuild.stdout.length !== 0
      ) {
        throw new Error(
          "qualification sidecar requires clean source before the build",
        );
      }
    }

    const args = sidecarCargoBuildArgs(profile, manifestPath);
    const lockDescriptor =
      releaseBuildLock.leaseFileDescriptor ?? releaseBuildLock.lockFileDescriptor;
    const buildStdio = lockDescriptor === undefined
      ? "inherit"
      : ["inherit", "inherit", "inherit", lockDescriptor];
    const build = runCommand(
      "cargo",
      args,
      profile === "qualification"
        ? {
            stdio: buildStdio,
            env: {
              ...env,
              EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_GIT_COMMIT:
                qualificationSourceIdentity.gitCommit,
              EMUCHEF_CONFIG_CREATE_QUALIFICATION_MATERIAL_DIGEST:
                qualificationSourceIdentity.materialBuildDigest,
              EMUCHEF_CONFIG_CREATE_QUALIFICATION_MATERIAL_INPUTS_DIGEST:
                qualificationMaterialInputs.digest,
            },
          }
        : { stdio: buildStdio },
    );
    if (build.error || build.status !== 0) {
      throw new Error(
        `cargo sidecar build failed${build.status === null ? "" : ` with status ${build.status}`}`,
      );
    }

    const cargoProfileDirectory =
      profile === "release"
        ? "release"
        : profile === "debug"
          ? "debug"
          : "qualification";
    const source = path.join(
      repoRoot,
      "crates/emuchef-rust-backend/target",
      cargoProfileDirectory,
      `${BINARY_BASENAME}${extension}`,
    );
    assertQualificationArtifactPath(repoRoot, source, { mustExist: true });
    if (!fs.statSync(source).isFile()) {
      throw new Error(`cargo sidecar binary is missing: ${source}`);
    }

    let backendProvenance = null;
    let identity = null;
    let materialInputs = null;
    let sourceState = null;
    if (profile === "qualification") {
      const identityAfterBuild = buildIdentity({
        repoRoot,
        requireClean: true,
      });
      const materialInputsAfterBuild = readMaterialInputs(repoRoot);
      sourceState = runCommand(
        "git",
        ["status", "--porcelain", "--untracked-files=all"],
        { cwd: repoRoot, encoding: "utf8" },
      );
      if (
        sourceState.error ||
        sourceState.status !== 0 ||
        sourceState.stdout.length !== 0
      ) {
        throw new Error(
          "qualification sidecar requires clean source after the build",
        );
      }
      if (
        identityAfterBuild.gitCommit !== qualificationSourceIdentity.gitCommit ||
        identityAfterBuild.materialBuildDigest !==
          qualificationSourceIdentity.materialBuildDigest ||
        materialInputsAfterBuild.digest !== qualificationMaterialInputs.digest
      ) {
        throw new Error(
          "source identity changed during qualification sidecar build",
        );
      }

      const provenanceResult = runCommand(
        source,
        ["--qualification-build-provenance"],
        {
          encoding: "utf8",
        },
      );
      if (provenanceResult.error || provenanceResult.status !== 0) {
        throw new Error(
          "qualification sidecar did not report its compiled feature and helper identity",
        );
      }
      try {
        backendProvenance = validateQualificationSidecarProvenance(
          JSON.parse(provenanceResult.stdout),
          {
            sourceIdentity: qualificationSourceIdentity,
            materialInputsDigest: qualificationMaterialInputs.digest,
          },
        );
      } catch (error) {
        throw new Error(
          `qualification sidecar provenance was invalid: ${error.message}`,
        );
      }
      identity = qualificationSourceIdentity;
      materialInputs = qualificationMaterialInputs;
    }

    if (profile === "qualification") {
      const { sidecarSha256 } = publishQualificationArtifacts({
        repoRoot,
        destination,
        runtimeDestination,
        source,
        extension,
        targetId,
        identity,
        materialInputs,
        sourceState,
        backendProvenance,
        manifestPath: qualificationManifestPath,
        statePath: qualificationStatePath,
      });
      console.log(`Prepared ${profile} Rust sidecar: ${destination}`);
      console.log(
        `Recorded qualification sidecar provenance: ${qualificationManifestPath}`,
      );
    } else {
      assertQualificationArtifactPath(repoRoot, destination);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      assertQualificationArtifactPath(repoRoot, destination);
      fs.copyFileSync(source, destination);
      if (extension !== ".exe") fs.chmodSync(destination, 0o755);
      assertQualificationArtifactPath(repoRoot, destination, { mustExist: true });
      console.log(`Prepared ${profile} Rust sidecar: ${destination}`);
    }
  } catch (error) {
    preserveBuildLock = error?.preserveBuildLock === true;
    throw error;
  } finally {
    if (!preserveBuildLock) releaseBuildLock();
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  const repoRoot = path.resolve(DEFAULT_APP_DIR, "../..");
  if (process.argv.includes("--require-release-package-lock")) {
    assertReleasePackageLock({ repoRoot });
  }
  const profileIndex = process.argv.indexOf("--profile");
  const profile = profileIndex >= 0 ? process.argv[profileIndex + 1] : "debug";
  prepareRustSidecar(profile);
}

#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { targetExecutableExtension } from "../../../scripts/sidecar-target-contract.mjs";
import {
  assertQualificationArtifactPath,
  QUALIFICATION_BACKEND_FEATURES,
  sidecarBuildLockPath,
  acquireSidecarBuildLock,
  qualificationStateMarkerPath,
  pathEntryPresent,
} from "./sidecar-build-contract.mjs";
import {
  BINARY_BASENAME,
  externalBinArtifactName,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";
import {
  assertQualificationSessionReleasable,
  qualificationSessionLeasePath,
} from "./qualification-session-lifecycle.mjs";

const QUALIFICATION_MANIFEST =
  "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json";
const ALLOWED_SIDECAR_PATHS = [
  "apps/emuchef-app/src-tauri/binaries/",
  "crates/emuchef-rust-backend/target/debug/",
];

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function resolveRepositoryFile(repoRoot, relativePath, expectedRoot) {
  if (
    typeof relativePath !== "string" ||
    path.isAbsolute(relativePath) ||
    !relativePath.startsWith(expectedRoot)
  ) {
    throw new Error("qualification manifest names an unexpected artifact path");
  }
  const absolutePath = path.resolve(repoRoot, relativePath);
  const repositoryRelativePath = path.relative(
    path.resolve(repoRoot),
    absolutePath,
  );
  if (
    repositoryRelativePath === "" ||
    repositoryRelativePath === ".." ||
    repositoryRelativePath.startsWith(`..${path.sep}`) ||
    path.isAbsolute(repositoryRelativePath)
  ) {
    throw new Error("qualification artifact path escapes the repository");
  }
  const expectedRootPath = path.resolve(repoRoot, expectedRoot);
  const expectedRootRelativePath = path.relative(
    expectedRootPath,
    absolutePath,
  );
  if (
    expectedRootRelativePath === "" ||
    expectedRootRelativePath === ".." ||
    expectedRootRelativePath.startsWith(`..${path.sep}`) ||
    path.isAbsolute(expectedRootRelativePath)
  ) {
    throw new Error(
      "qualification artifact path escapes expected sidecar directory",
    );
  }
  assertQualificationArtifactPath(repoRoot, relativePath, { mustExist: true });
  return absolutePath;
}

function parseJson(bytes, label) {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    throw new Error(`${label} is not valid JSON`);
  }
}

function requireDigest(actual, expected, label) {
  if (!/^sha256:[0-9a-f]{64}$/.test(expected ?? "") || actual !== expected) {
    throw new Error(
      `${label} digest does not match the qualification manifest`,
    );
  }
}

function validateReleaseArguments({
  repoRoot,
  targetId,
  confirmEndedTarget,
  evidenceDirectory,
  disposition,
  reason,
}) {
  if (!targetId || confirmEndedTarget !== targetId) {
    throw new Error(
      "exact ended-target confirmation must match the selected target ID",
    );
  }
  if (!new Set(["finished", "abandoned"]).has(disposition)) {
    throw new Error("disposition must be finished or abandoned");
  }
  if (disposition === "abandoned" && !reason?.trim()) {
    throw new Error("abandoned qualification state requires a reason");
  }
  if (!path.isAbsolute(evidenceDirectory ?? "")) {
    throw new Error("evidence directory must be an absolute host path");
  }
  const resolvedRepoRoot = path.resolve(repoRoot);
  const resolvedEvidenceDirectory = path.resolve(evidenceDirectory);
  const relativeEvidencePath = path.relative(
    resolvedRepoRoot,
    resolvedEvidenceDirectory,
  );
  if (
    relativeEvidencePath === "" ||
    (!relativeEvidencePath.startsWith(`..${path.sep}`) &&
      relativeEvidencePath !== "..") ||
    path.isAbsolute(relativeEvidencePath)
  ) {
    throw new Error(
      "evidence directory must be outside the repository worktree",
    );
  }
  if (!fs.existsSync(path.dirname(resolvedEvidenceDirectory))) {
    throw new Error("evidence directory parent must already exist");
  }
  if (!fs.statSync(path.dirname(resolvedEvidenceDirectory)).isDirectory()) {
    throw new Error("evidence directory parent must be a directory");
  }
  const realRepoRoot = fs.realpathSync(resolvedRepoRoot);
  const realEvidenceDirectory = path.join(
    fs.realpathSync(path.dirname(resolvedEvidenceDirectory)),
    path.basename(resolvedEvidenceDirectory),
  );
  const realEvidenceRelativePath = path.relative(
    realRepoRoot,
    realEvidenceDirectory,
  );
  if (
    realEvidenceRelativePath === "" ||
    (!realEvidenceRelativePath.startsWith(`..${path.sep}`) &&
      realEvidenceRelativePath !== "..") ||
    path.isAbsolute(realEvidenceRelativePath)
  ) {
    throw new Error(
      "evidence directory must resolve outside the repository worktree",
    );
  }
  try {
    const existingEvidence = fs.lstatSync(resolvedEvidenceDirectory);
    if (existingEvidence.isSymbolicLink() || !existingEvidence.isDirectory()) {
      throw new Error("existing evidence path is not a regular directory");
    }
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  return resolvedEvidenceDirectory;
}

function readRegularFile(filePath, label) {
  const metadata = fs.lstatSync(filePath);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`${label} is not a regular file`);
  }
  return fs.readFileSync(filePath);
}

function releaseArtifactSources({
  repoRoot,
  hostTarget,
  markerPath,
  sessionPath,
}) {
  return new Map([
    [
      "qualification-state.json",
      markerPath,
    ],
    [
      "qualification-session.json",
      sessionPath,
    ],
    [
      "retroarch-config-create-sidecar-build.json",
      path.join(repoRoot, QUALIFICATION_MANIFEST),
    ],
    [
      "tauri-external-sidecar.bin",
      path.join(
        repoRoot,
        "apps/emuchef-app/src-tauri/binaries",
        externalBinArtifactName(hostTarget),
      ),
    ],
    [
      "runtime-sidecar.bin",
      path.join(
        repoRoot,
        "crates/emuchef-rust-backend/target/debug",
        `${BINARY_BASENAME}${targetExecutableExtension(hostTarget)}`,
      ),
    ],
  ]);
}

function validateArchivedRelease({
  archiveDirectory,
  repoRoot,
  targetId,
  confirmEndedTarget,
  disposition,
  reason,
  markerPath,
  sessionPath,
  hostTarget,
}) {
  const receiptPath = path.join(archiveDirectory, "release-receipt.json");
  let receiptBytes;
  try {
    receiptBytes = readRegularFile(receiptPath, "qualification release receipt");
  } catch (error) {
    if (error.code === "ENOENT") {
      throw new Error(
        "evidence directory already exists without a release receipt; refusing to overwrite it",
      );
    }
    throw error;
  }
  const receipt = parseJson(
    receiptBytes,
    "qualification release receipt",
  );
  if (
    receipt.schemaVersion !== 1 ||
    receipt.qualificationTargetId !== targetId ||
    targetId !== confirmEndedTarget ||
    receipt.disposition !== disposition ||
    (receipt.reason ?? "") !== reason.trim() ||
    typeof receipt.stateMarkerPresent !== "boolean" ||
    typeof receipt.sessionLeasePresent !== "boolean" ||
    !Array.isArray(receipt.archived)
  ) {
    throw new Error("existing evidence archive does not match this release request");
  }

  const sources = releaseArtifactSources({
    repoRoot,
    hostTarget,
    markerPath,
    sessionPath,
  });
  const expectedNames = [
    "retroarch-config-create-sidecar-build.json",
    "tauri-external-sidecar.bin",
    "runtime-sidecar.bin",
    ...(receipt.stateMarkerPresent ? ["qualification-state.json"] : []),
    ...(receipt.sessionLeasePresent ? ["qualification-session.json"] : []),
  ];
  const entriesByName = new Map();
  for (const entry of receipt.archived) {
    if (
      !entry ||
      typeof entry.name !== "string" ||
      !sources.has(entry.name) ||
      entriesByName.has(entry.name) ||
      typeof entry.sourcePath !== "string" ||
      !/^sha256:[0-9a-f]{64}$/.test(entry.sha256 ?? "")
    ) {
      throw new Error("qualification release receipt has an invalid artifact entry");
    }
    const sourcePath = path
      .relative(repoRoot, sources.get(entry.name))
      .split(path.sep)
      .join("/");
    if (entry.sourcePath !== sourcePath) {
      throw new Error("qualification release receipt names an unexpected source path");
    }
    const bytes = readRegularFile(
      path.join(archiveDirectory, entry.name),
      `archived ${entry.name}`,
    );
    requireDigest(sha256(bytes), entry.sha256, `archived ${entry.name}`);
    entriesByName.set(entry.name, { entry, bytes });
  }
  if (
    entriesByName.size !== expectedNames.length ||
    expectedNames.some((name) => !entriesByName.has(name))
  ) {
    throw new Error("qualification release archive is incomplete");
  }
  const allowedArchiveNames = new Set([
    ...expectedNames,
    "release-receipt.json",
  ]);
  if (
    fs.readdirSync(archiveDirectory).some((name) => !allowedArchiveNames.has(name))
  ) {
    throw new Error("qualification release archive contains unexpected entries");
  }

  const manifestBytes = entriesByName.get("retroarch-config-create-sidecar-build.json").bytes;
  const manifest = parseJson(manifestBytes, "configuration-create qualification build manifest");
  const externalBytes = entriesByName.get("tauri-external-sidecar.bin").bytes;
  const runtimeBytes = entriesByName.get("runtime-sidecar.bin").bytes;
  const expectedExternalPath = path
    .relative(repoRoot, sources.get("tauri-external-sidecar.bin"))
    .split(path.sep)
    .join("/");
  const expectedRuntimePath = path
    .relative(repoRoot, sources.get("runtime-sidecar.bin"))
    .split(path.sep)
    .join("/");
  if (
    manifest.schemaVersion !== 1 ||
    manifest.sourceWorktreeClean !== true ||
    manifest.qualificationTargetId !== targetId ||
    JSON.stringify(manifest.backendFeatures) !==
      JSON.stringify(QUALIFICATION_BACKEND_FEATURES) ||
    !/^sha256:[0-9a-f]{64}$/.test(manifest.arm64HelperSha256 ?? "") ||
    manifest.sidecarPath !== expectedExternalPath ||
    manifest.runtimeSidecarPath !== expectedRuntimePath
  ) {
    throw new Error("archived qualification manifest has invalid provenance");
  }
  requireDigest(sha256(externalBytes), manifest.sidecarSha256, "archived external sidecar");
  requireDigest(
    sha256(runtimeBytes),
    manifest.runtimeSidecarSha256,
    "archived runtime sidecar",
  );
  if (manifest.sidecarSha256 !== manifest.runtimeSidecarSha256) {
    throw new Error("archived pinned sidecar copies do not have the same digest");
  }

  let archivedMarker = null;
  const markerEntry = entriesByName.get("qualification-state.json");
  if (markerEntry) {
    archivedMarker = parseJson(markerEntry.bytes, "archived qualification state marker");
    if (
      archivedMarker.schemaVersion !== 1 ||
      archivedMarker.qualificationTargetId !== targetId ||
      archivedMarker.manifestPath !== QUALIFICATION_MANIFEST
    ) {
      throw new Error("archived qualification state marker has invalid identity");
    }
  }
  const archivedSession = entriesByName.get("qualification-session.json");
  const sessionEvidence = assertQualificationSessionReleasable({
    sessionPath,
    targetId,
    disposition,
    reason,
    sessionLaunchAttempted: archivedMarker?.sessionLaunchAttempted,
    ...(archivedSession ? { archivedSessionBytes: archivedSession.bytes } : {}),
  });
  if (Boolean(sessionEvidence) !== receipt.sessionLeasePresent) {
    throw new Error("qualification release receipt session evidence is inconsistent");
  }

  const statePaths = [markerPath, sources.get("retroarch-config-create-sidecar-build.json"), sessionPath];
  const stateStillPinned = statePaths.some(pathEntryPresent);
  for (const name of ["tauri-external-sidecar.bin", "runtime-sidecar.bin"]) {
    const sourcePath = sources.get(name);
    if (stateStillPinned) {
      const currentBytes = readRegularFile(sourcePath, `pinned ${name}`);
      if (!currentBytes.equals(entriesByName.get(name).bytes)) {
        throw new Error(`pinned ${name} changed before guarded release`);
      }
    }
  }
  for (const [name, sourcePath] of [
    ["qualification-state.json", markerPath],
    ["retroarch-config-create-sidecar-build.json", sources.get("retroarch-config-create-sidecar-build.json")],
    ["qualification-session.json", sessionPath],
  ]) {
    const archivedEntry = entriesByName.get(name);
    if (pathEntryPresent(sourcePath)) {
      if (!archivedEntry) {
        throw new Error(`unexpected ${name} appeared during qualification release`);
      }
      const currentBytes = readRegularFile(sourcePath, name);
      if (!currentBytes.equals(archivedEntry.bytes)) {
        throw new Error(`${name} changed during qualification release`);
      }
    }
  }

  for (const [name, sourcePath] of [
    ["retroarch-config-create-sidecar-build.json", sources.get("retroarch-config-create-sidecar-build.json")],
    ["qualification-session.json", sessionPath],
    ["qualification-state.json", markerPath],
  ]) {
    if (!pathEntryPresent(sourcePath)) continue;
    const archivedEntry = entriesByName.get(name);
    if (!archivedEntry) {
      throw new Error(`cannot retire ${name} without archived evidence`);
    }
    try {
      fs.unlinkSync(sourcePath);
    } catch (error) {
      throw new Error(`could not retire ${name}; build guard remains active: ${error.message}`);
    }
  }
  return { receipt, targetId, disposition, evidenceDirectory: archiveDirectory };
}

/** Archive pinned host artifacts before releasing the configuration-create build guard. */
export function archiveAndReleaseQualificationState({
  repoRoot,
  targetId,
  markerPath = qualificationStateMarkerPath(repoRoot),
  sessionPath = qualificationSessionLeasePath(repoRoot, markerPath),
  lockPath = sidecarBuildLockPath(repoRoot),
  ...options
}) {
  const hostTargetResult = spawnSync("rustc", ["--print", "host-tuple"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (hostTargetResult.error || hostTargetResult.status !== 0) {
    throw new Error("rustc could not report the configuration-create sidecar target");
  }
  const hostTarget = validateTargetTriple(hostTargetResult.stdout.trim());
  const artifactPaths = [
    QUALIFICATION_MANIFEST,
    path.join(
      "apps/emuchef-app/src-tauri/binaries",
      externalBinArtifactName(hostTarget),
    ),
    path.join(
      "crates/emuchef-rust-backend/target/debug",
      `${BINARY_BASENAME}${targetExecutableExtension(hostTarget)}`,
    ),
  ].map((artifactPath) => artifactPath.split(path.sep).join("/"));
  const releaseLock = acquireSidecarBuildLock({
    repoRoot,
    profile: "qualification-release",
    targetId,
    artifactPaths,
    lockPath,
  });

  try {
    return archiveQualificationStateUnderLock({
      ...options,
      repoRoot,
      targetId,
      markerPath,
      sessionPath,
      lockPath,
      hostTarget,
    });
  } finally {
    releaseLock();
  }
}

function archiveQualificationStateUnderLock({
  repoRoot,
  targetId,
  confirmEndedTarget,
  evidenceDirectory,
  disposition,
  reason = "",
  markerPath = qualificationStateMarkerPath(repoRoot),
  sessionPath = qualificationSessionLeasePath(repoRoot, markerPath),
  hostTarget,
}) {
  const archiveDirectory = validateReleaseArguments({
    repoRoot,
    targetId,
    confirmEndedTarget,
    evidenceDirectory,
    disposition,
    reason,
  });
  if (fs.existsSync(archiveDirectory)) {
    return validateArchivedRelease({
      archiveDirectory,
      repoRoot,
      targetId,
      confirmEndedTarget,
      disposition,
      reason,
      markerPath,
      sessionPath,
      hostTarget,
    });
  }
  const markerPresent = pathEntryPresent(markerPath);
  if (!markerPresent && disposition !== "abandoned") {
    throw new Error("finished qualification release requires its state marker");
  }
  const markerBytes = markerPresent
    ? readRegularFile(markerPath, "configuration-create qualification state marker")
    : null;
  let marker = null;
  if (markerBytes) {
    marker = parseJson(markerBytes, "configuration-create qualification state marker");
    if (
      marker.schemaVersion !== 1 ||
      marker.qualificationTargetId !== targetId
    ) {
      throw new Error(
        "qualification state marker does not match the confirmed target",
      );
    }
    if (marker.manifestPath !== QUALIFICATION_MANIFEST) {
      throw new Error(
        "qualification state marker names an unexpected manifest path",
      );
    }
  }
  const sessionEvidence = assertQualificationSessionReleasable({
    sessionPath,
    targetId,
    disposition,
    reason,
    sessionLaunchAttempted: marker?.sessionLaunchAttempted,
  });

  const manifestPath = assertQualificationArtifactPath(
    repoRoot,
    QUALIFICATION_MANIFEST,
    { mustExist: true },
  );
  const manifestBytes = fs.readFileSync(manifestPath);
  const manifest = parseJson(
    manifestBytes,
    "configuration-create qualification build manifest",
  );
  if (
    manifest.schemaVersion !== 1 ||
    manifest.sourceWorktreeClean !== true ||
    manifest.qualificationTargetId !== targetId
  ) {
    throw new Error(
      "qualification build manifest does not match the confirmed target",
    );
  }
  if (
    JSON.stringify(manifest.backendFeatures) !==
      JSON.stringify(QUALIFICATION_BACKEND_FEATURES) ||
    !/^sha256:[0-9a-f]{64}$/.test(manifest.arm64HelperSha256 ?? "")
  ) {
    throw new Error(
      "qualification build manifest lacks the required feature or helper identity",
    );
  }

  const expectedSidecarPath = path
    .relative(
      repoRoot,
      path.join(
        repoRoot,
        "apps/emuchef-app/src-tauri/binaries",
        externalBinArtifactName(hostTarget),
      ),
    )
    .split(path.sep)
    .join("/");
  const expectedRuntimeSidecarPath = path
    .relative(
      repoRoot,
      path.join(
        repoRoot,
        "crates/emuchef-rust-backend/target/debug",
        `${BINARY_BASENAME}${targetExecutableExtension(hostTarget)}`,
      ),
    )
    .split(path.sep)
    .join("/");
  if (
    manifest.sidecarPath !== expectedSidecarPath ||
    manifest.runtimeSidecarPath !== expectedRuntimeSidecarPath
  ) {
    throw new Error(
      "qualification manifest names an unexpected artifact path for the current target",
    );
  }

  const externalPath = resolveRepositoryFile(
    repoRoot,
    manifest.sidecarPath,
    ALLOWED_SIDECAR_PATHS[0],
  );
  const runtimePath = resolveRepositoryFile(
    repoRoot,
    manifest.runtimeSidecarPath,
    ALLOWED_SIDECAR_PATHS[1],
  );
  const externalBytes = fs.readFileSync(externalPath);
  const runtimeBytes = fs.readFileSync(runtimePath);
  requireDigest(
    sha256(externalBytes),
    manifest.sidecarSha256,
    "external sidecar",
  );
  requireDigest(
    sha256(runtimeBytes),
    manifest.runtimeSidecarSha256,
    "runtime sidecar",
  );
  if (manifest.sidecarSha256 !== manifest.runtimeSidecarSha256) {
    throw new Error("pinned sidecar copies do not have the same digest");
  }

  const archived = markerBytes
    ? [
        {
          name: "qualification-state.json",
          sourcePath: markerPath,
          bytes: markerBytes,
        },
      ]
    : [];
  archived.push(
    {
      name: "retroarch-config-create-sidecar-build.json",
      sourcePath: manifestPath,
      bytes: manifestBytes,
    },
    ...(sessionEvidence
      ? [
          {
            name: "qualification-session.json",
            sourcePath: sessionPath,
            bytes: sessionEvidence.bytes,
          },
        ]
      : []),
    {
      name: "tauri-external-sidecar.bin",
      sourcePath: externalPath,
      bytes: externalBytes,
    },
    {
      name: "runtime-sidecar.bin",
      sourcePath: runtimePath,
      bytes: runtimeBytes,
    },
  );

  const stagingDirectory = fs.mkdtempSync(
    path.join(
      path.dirname(archiveDirectory),
      `.${path.basename(archiveDirectory)}.partial-`,
    ),
  );
  for (const entry of archived) {
    const archivePath = path.join(stagingDirectory, entry.name);
    fs.writeFileSync(archivePath, entry.bytes, { flag: "wx" });
    if (sha256(fs.readFileSync(archivePath)) !== sha256(entry.bytes)) {
      throw new Error(
        `archived qualification artifact failed readback: ${entry.name}`,
      );
    }
  }
  const receipt = {
    schemaVersion: 1,
    qualificationTargetId: targetId,
    disposition,
    stateMarkerPresent: markerPresent,
    sessionLeasePresent: Boolean(sessionEvidence),
    ...(reason.trim() ? { reason: reason.trim() } : {}),
    archived: archived.map((entry) => ({
      name: entry.name,
      sourcePath: path
        .relative(repoRoot, entry.sourcePath)
        .split(path.sep)
        .join("/"),
      sha256: sha256(entry.bytes),
    })),
  };
  const receiptBytes = Buffer.from(`${JSON.stringify(receipt, null, 2)}\n`);
  const receiptPath = path.join(stagingDirectory, "release-receipt.json");
  fs.writeFileSync(receiptPath, receiptBytes, { flag: "wx" });
  if (!fs.readFileSync(receiptPath).equals(receiptBytes)) {
    throw new Error("qualification release receipt failed readback");
  }

  if (
    markerPresent &&
    (!pathEntryPresent(markerPath) ||
      !fs.readFileSync(markerPath).equals(markerBytes))
  ) {
    throw new Error(
      "qualification state changed during archive; build guard remains active",
    );
  }
  if (!markerPresent && pathEntryPresent(markerPath)) {
    throw new Error(
      "qualification state appeared during archive; build guard remains active",
    );
  }
  if (!fs.readFileSync(manifestPath).equals(manifestBytes)) {
    throw new Error(
      "qualification manifest changed during archive; build guard remains active",
    );
  }
  for (const entry of archived) {
    const archivedPath = path.join(stagingDirectory, entry.name);
    if (
      !fs.readFileSync(entry.sourcePath).equals(entry.bytes) ||
      !fs.readFileSync(archivedPath).equals(entry.bytes)
    ) {
      throw new Error(
        `qualification evidence changed during archive: ${entry.name}`,
      );
    }
  }
  fs.renameSync(stagingDirectory, archiveDirectory);
  return validateArchivedRelease({
    archiveDirectory,
    repoRoot,
    targetId,
    confirmEndedTarget,
    disposition,
    reason,
    markerPath,
    sessionPath,
    hostTarget,
  });
}

function parseArguments(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!key?.startsWith("--") || !value || options[key]) {
      throw new Error("expected one value for each configuration-create state-release option");
    }
    options[key] = value;
  }
  const expected = new Set([
    "--target-id",
    "--confirm-ended-target",
    "--evidence-dir",
    "--disposition",
    "--reason",
  ]);
  if (Object.keys(options).some((key) => !expected.has(key))) {
    throw new Error("unknown configuration-create state-release option");
  }
  return options;
}

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const REPO_ROOT = path.resolve(path.dirname(SCRIPT_PATH), "../../..");
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  try {
    const options = parseArguments(process.argv.slice(2));
    const result = archiveAndReleaseQualificationState({
      repoRoot: REPO_ROOT,
      targetId: options["--target-id"],
      confirmEndedTarget: options["--confirm-ended-target"],
      evidenceDirectory: options["--evidence-dir"],
      disposition: options["--disposition"],
      reason: options["--reason"] ?? "",
      lockPath: sidecarBuildLockPath(REPO_ROOT),
    });
    console.log(
      `Archived configuration-create ${result.disposition} evidence to ${result.evidenceDirectory}`,
    );
    console.log("Removed the qualification manifest and build-state marker.");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

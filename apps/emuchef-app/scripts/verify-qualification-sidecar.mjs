#!/usr/bin/env node

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { buildMaterialIdentity } from "../../../tools/device-qualification.mjs";
import { targetExecutableExtension } from "../../../scripts/sidecar-target-contract.mjs";
import {
  BINARY_BASENAME,
  externalBinArtifactName,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";
import {
  assertQualificationArtifactPath,
  retroarchConfigCreateMaterialInputs,
  validateQualificationSidecarManifest,
} from "./sidecar-build-contract.mjs";
import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";

const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = path.resolve(APP_ROOT, "../..");
const MANIFEST_PATH = path.join(
  REPO_ROOT,
  "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
);

export function verifyQualificationSidecar({
  repoRoot = REPO_ROOT,
  appRoot = APP_ROOT,
  manifestPath = MANIFEST_PATH,
  env = process.env,
  spawn = spawnSync,
  buildIdentity = buildMaterialIdentity,
  materialInputs = retroarchConfigCreateMaterialInputs,
} = {}) {
  const configCreateQualificationEnabled =
    env.EMUCHEF_CONFIG_CREATE_QUALIFICATION === "1";
  const requestedTargetId = env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID ?? null;
  if (configCreateQualificationEnabled) {
    selectConfigCreateQualificationTarget(requestedTargetId);
  }
  else if (requestedTargetId !== null) {
    throw new Error(
      "A qualification target ID requires an active RetroArch configuration-create qualification.",
    );
  }

  const identity = buildIdentity({ repoRoot, requireClean: true });
  assertQualificationArtifactPath(repoRoot, manifestPath, { mustExist: true });
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  const targetId = requestedTargetId ?? manifest.qualificationTargetId ?? null;
  const targetResult = spawn("rustc", ["--print", "host-tuple"], { encoding: "utf8" });
  if (targetResult.error || targetResult.status !== 0) {
    throw new Error("rustc could not report the Tauri host target");
  }
  const target = validateTargetTriple(targetResult.stdout.trim());
  const expectedSidecarPath = path.join(appRoot, "src-tauri/binaries", externalBinArtifactName(target));
  const expectedRelativePath = path.relative(repoRoot, expectedSidecarPath).split(path.sep).join("/");
  if (manifest.sidecarPath !== expectedRelativePath) {
    throw new Error("qualification sidecar manifest names a different Tauri external binary");
  }
  assertQualificationArtifactPath(repoRoot, expectedSidecarPath, { mustExist: true });
  const expectedRuntimeSidecarPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug",
    `${BINARY_BASENAME}${targetExecutableExtension(target)}`,
  );
  const expectedRuntimeRelativePath = path
    .relative(repoRoot, expectedRuntimeSidecarPath)
    .split(path.sep)
    .join("/");
  if (manifest.runtimeSidecarPath !== expectedRuntimeRelativePath) {
    throw new Error("qualification sidecar manifest names a different Tauri debug runtime binary");
  }
  assertQualificationArtifactPath(repoRoot, expectedRuntimeSidecarPath, {
    mustExist: true,
  });
  const actualSidecarSha256 = `sha256:${createHash("sha256").update(fs.readFileSync(expectedSidecarPath)).digest("hex")}`;
  const actualRuntimeSidecarSha256 = `sha256:${createHash("sha256").update(fs.readFileSync(expectedRuntimeSidecarPath)).digest("hex")}`;
  if (
    manifest.runtimeSidecarSha256 !== actualRuntimeSidecarSha256 ||
    actualRuntimeSidecarSha256 !== actualSidecarSha256
  ) {
    throw new Error("Tauri debug runtime sidecar differs from the qualification external binary");
  }
  assertQualificationArtifactPath(repoRoot, expectedSidecarPath, { mustExist: true });
  const provenanceResult = spawn(expectedSidecarPath, ["--qualification-build-provenance"], {
    encoding: "utf8",
  });
  if (provenanceResult.error || provenanceResult.status !== 0) {
    throw new Error("Tauri external binary lacks qualification build provenance");
  }
  const provenance = JSON.parse(provenanceResult.stdout);
  validateQualificationSidecarManifest(manifest, {
    sourceIdentity: identity,
    materialInputsDigest: materialInputs(repoRoot).digest,
    sidecarSha256: actualSidecarSha256,
    provenance,
    targetId,
  });
  return { manifest, sidecarPath: expectedSidecarPath };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const result = verifyQualificationSidecar();
    console.log(`Verified qualification sidecar: ${result.sidecarPath}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

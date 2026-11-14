#!/usr/bin/env node

import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";
import { verifyQualificationSidecar } from "./verify-qualification-sidecar.mjs";

export function ensureConfigCreateQualification({
  env = process.env,
  targetId,
  verify = verifyQualificationSidecar,
} = {}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  const qualificationEnv = {
    ...env,
    EMUCHEF_DEVICE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: target.qualificationTargetId,
  };
  const verified = verify({ env: qualificationEnv });
  if (verified.manifest.qualificationTargetId !== target.qualificationTargetId) {
    throw new Error("pinned qualification sidecar belongs to a different configuration-create target");
  }
  return verified;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  const targetIndex = process.argv.indexOf("--target-id");
  const targetId = targetIndex >= 0 ? process.argv[targetIndex + 1] : undefined;
  try {
    const result = ensureConfigCreateQualification({ targetId });
    console.log(`Reused verified configuration-create qualification sidecar: ${result.sidecarPath}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}

#!/usr/bin/env node

import path from "node:path";
import { pathToFileURL } from "node:url";
import { APP_ROOT, run as runStep } from "./run-device-qualification.mjs";
import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";

export function prepareConfigCreateQualification({
  cwd = APP_ROOT,
  env = process.env,
  spawnSync,
  exit,
  targetId,
} = {}) {
  const target = selectConfigCreateQualificationTarget(targetId);
  const qualificationEnv = {
    ...env,
    EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: target.qualificationTargetId,
  };
  const dependencies = { cwd, env: qualificationEnv, spawnSync, exit };
  if (!runStep("npm", ["run", "build"], dependencies)) return false;
  if (!runStep("npm", ["run", "sidecar:qualification"], dependencies)) return false;
  if (!runStep("node", ["scripts/verify-qualification-sidecar.mjs"], dependencies)) return false;
  return runStep(
    "cargo",
    ["check", "--manifest-path", "src-tauri/Cargo.toml", "--features", "real-execution"],
    {
      ...dependencies,
      env: { ...qualificationEnv, EMUCHEF_DEVICE_QUALIFICATION: "1" },
    },
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const targetIndex = process.argv.indexOf("--target-id");
  const targetId = targetIndex >= 0 ? process.argv[targetIndex + 1] : undefined;
  try {
    prepareConfigCreateQualification({ targetId });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}

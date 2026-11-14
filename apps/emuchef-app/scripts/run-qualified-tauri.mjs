#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { selectConfigCreateQualificationTarget } from "./qualification-target.mjs";
import {
  assertQualifiedCleanExit,
  beginQualifiedLaunch,
  cancelQualifiedLaunchBeforeTauri,
  markQualifiedLaunchVerified,
} from "./qualification-session-lifecycle.mjs";
import { verifyQualificationSidecar } from "./verify-qualification-sidecar.mjs";

const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function qualificationTauriPlan(command, env = process.env) {
  if (command !== "dev") {
    throw new Error("configuration-create qualification supports a development-only Tauri launch");
  }

  const targetId = env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID;
  selectConfigCreateQualificationTarget(targetId);
  const qualificationEnv = {
    ...env,
    EMUCHEF_DEVICE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: targetId,
  };

  return [
    {
      executable: process.execPath,
      args: [
        "scripts/ensure-retroarch-config-create-qualification.mjs",
        "--target-id",
        targetId,
      ],
      env: qualificationEnv,
    },
    {
      executable: "tauri",
      args: [
        command,
        "--config",
        "src-tauri/tauri.dev.conf.json",
        "--features",
        "real-execution",
      ],
      env: qualificationEnv,
    },
  ];
}

export function runQualifiedTauri(
  command,
  {
    env = process.env,
    spawn = spawnSync,
    cwd = APP_ROOT,
    repoRoot = path.resolve(cwd, "../.."),
    verify = verifyQualificationSidecar,
  } = {},
) {
  const initialPlan = qualificationTauriPlan(command, env);
  const targetId = env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID;
  const launch = beginQualifiedLaunch({
    repoRoot,
    targetId,
    env: initialPlan[0].env,
  });
  const plan = qualificationTauriPlan(command, {
    ...initialPlan[0].env,
    ...launch.environment,
  });
  let tauriStarted = false;

  try {
    const preparation = plan[0];
    const preparationResult = spawn(
      preparation.executable,
      preparation.args,
      { cwd, env: preparation.env, stdio: "inherit" },
    );
    if (preparationResult.error) throw preparationResult.error;
    if (preparationResult.status !== 0) {
      cancelQualifiedLaunchBeforeTauri({ repoRoot, targetId, token: launch.token });
      return preparationResult.status ?? 1;
    }

    markQualifiedLaunchVerified({
      repoRoot,
      targetId,
      token: launch.token,
      env: plan[0].env,
      verify,
    });

    const step = plan[1];
    tauriStarted = true;
    const result = spawn(step.executable, step.args, {
      cwd,
      env: step.env,
      stdio: "inherit",
    });
    if (result.error) throw result.error;
    if (result.status !== 0) return result.status ?? 1;
    assertQualifiedCleanExit({
      targetId,
      token: launch.token,
      sessionPath: launch.sessionPath,
    });
    return 0;
  } catch (error) {
    if (!tauriStarted) {
      try {
        cancelQualifiedLaunchBeforeTauri({ repoRoot, targetId, token: launch.token });
      } catch (cancelError) {
        throw new AggregateError(
          [error, cancelError],
          "configuration-create launch preparation failed and its lease requires recovery",
        );
      }
    }
    throw error;
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  try {
    const status = runQualifiedTauri(process.argv[2]);
    if (status !== 0) process.exit(status);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

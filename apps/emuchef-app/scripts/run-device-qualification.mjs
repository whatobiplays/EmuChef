#!/usr/bin/env node

/**
 * Launches a recordable qualification build from a clean, non-hot-reloaded
 * application workflow. The build and sidecar preparation complete before
 * the Tauri application starts with real execution explicitly compiled in.
 */
import { spawnSync as nodeSpawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Runs one launcher step and exits the process when it cannot complete.
 *
 * The injectable dependencies keep the sequencing and failure boundary
 * testable without starting a build or a Tauri application.
 */
export function run(
  command,
  args,
  {
    cwd = APP_ROOT,
    env = process.env,
    spawnSync = nodeSpawnSync,
    exit = process.exit,
  } = {},
) {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error || result.status !== 0) {
    exit(result.status ?? 1);
    return false;
  }
  return true;
}

/**
 * Runs the pinned qualification sequence in its production application
 * working directory. The qualification opt-in is scoped to the final app
 * process and is not leaked into the preceding preparation steps.
 */
export function runQualification({
  cwd = APP_ROOT,
  env = process.env,
  spawnSync = nodeSpawnSync,
  exit = process.exit,
} = {}) {
  if (!run("npm", ["run", "build"], { cwd, env, spawnSync, exit })) return false;
  if (!run("npm", ["run", "sidecar:dev"], { cwd, env, spawnSync, exit })) return false;
  return run(
    "cargo",
    ["run", "--manifest-path", "src-tauri/Cargo.toml", "--features", "real-execution"],
    {
      cwd,
      env: { ...env, EMUCHEF_DEVICE_QUALIFICATION: "1" },
      spawnSync,
      exit,
    },
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  runQualification();
}

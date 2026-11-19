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

const RESTRICTION_FLAG = "--restrict-privileged-execution";

/**
 * @typedef {object} RunQualificationOptions
 * @property {string} [cwd]
 * @property {NodeJS.ProcessEnv} [env]
 * @property {typeof nodeSpawnSync} [spawnSync]
 * @property {(status: number) => void} [exit]
 * @property {boolean} [restrictPrivilegedExecution]
 */

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
 * Runs the qualification sequence in its production application working
 * directory. Restriction is opt-in and applies to sidecar preparation and
 * the final app process only.
 *
 * @param {RunQualificationOptions} [options]
 * @returns {boolean}
 */
export function runQualification({
  cwd = APP_ROOT,
  env = process.env,
  spawnSync = nodeSpawnSync,
  exit = process.exit,
  restrictPrivilegedExecution = false,
} = {}) {
  if (typeof restrictPrivilegedExecution !== "boolean") {
    throw new TypeError("restrictPrivilegedExecution must be a boolean");
  }

  const preparationEnvironment = { ...env };
  delete preparationEnvironment.EMUCHEF_DEVICE_QUALIFICATION;
  delete preparationEnvironment.EMUCHEF_RESTRICTED_EXECUTION_BUILD;
  if (!run("npm", ["run", "build"], {
    cwd,
    env: preparationEnvironment,
    spawnSync,
    exit,
  })) return false;
  const sidecarEnvironment = restrictPrivilegedExecution
    ? {
        ...preparationEnvironment,
        EMUCHEF_RESTRICTED_EXECUTION_BUILD: "1",
      }
    : preparationEnvironment;
  if (!run("npm", ["run", "sidecar:dev"], {
    cwd,
    env: sidecarEnvironment,
    spawnSync,
    exit,
  })) return false;

  const appArguments = [
    "run",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--features",
    "real-execution",
  ];
  if (restrictPrivilegedExecution) {
    appArguments.push("--", RESTRICTION_FLAG);
  }
  return run(
    "cargo",
    appArguments,
    {
      cwd,
      env: { ...preparationEnvironment, EMUCHEF_DEVICE_QUALIFICATION: "1" },
      spawnSync,
      exit,
    },
  );
}

/**
 * Parses launcher arguments before starting any build or application process.
 *
 * @param {string[]} [args]
 * @param {RunQualificationOptions} [dependencies]
 * @returns {boolean}
 */
export function runQualificationCli(
  args = process.argv.slice(2),
  dependencies = {},
) {
  let restrictPrivilegedExecution = false;
  for (const argument of args) {
    if (argument === RESTRICTION_FLAG) {
      restrictPrivilegedExecution = true;
      continue;
    }
    throw new Error(
      `Unknown option "${argument}". Supported option: ${RESTRICTION_FLAG}`,
    );
  }

  return runQualification({ ...dependencies, restrictPrivilegedExecution });
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    runQualificationCli();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  }
}

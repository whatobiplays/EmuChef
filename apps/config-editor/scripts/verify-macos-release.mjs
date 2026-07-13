#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  assertNoRepositoryCredentialFiles,
  validateAppleReleaseEnvironment,
} from "./check-apple-release-env.mjs";
import { inspectMacosBundle } from "./check-macos-bundle.mjs";
import { verifySignedMacosRelease } from "./check-signed-macos-release.mjs";
import { generateMacosReleaseManifest } from "./generate-macos-release-manifest.mjs";

const FULL_SHA_PATTERN = /^[0-9a-fA-F]{40}$/;

function pathsFromScript() {
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
  const appDirectory = path.resolve(scriptDirectory, "..");
  return {
    appDirectory,
    repositoryRoot: path.resolve(appDirectory, "../.."),
    scriptDirectory,
  };
}

export function parseVerifyArguments(argv) {
  const positional = [];
  let buildCommit;
  let outputPath;
  let skipEnv = false;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--skip-env") {
      if (skipEnv) throw new Error("--skip-env may be supplied only once");
      skipEnv = true;
    } else if (argument === "--output" || argument === "--build-commit") {
      if (index + 1 >= argv.length) {
        throw new Error(`${argument} requires a value`);
      }
      const value = argv[index + 1];
      index += 1;
      if (argument === "--output") {
        if (outputPath !== undefined) throw new Error("--output may be supplied only once");
        outputPath = path.resolve(value);
      } else {
        if (buildCommit !== undefined) {
          throw new Error("--build-commit may be supplied only once");
        }
        if (!FULL_SHA_PATTERN.test(value)) {
          throw new Error("--build-commit must be a full 40-character hexadecimal SHA");
        }
        buildCommit = value;
      }
    } else if (argument.startsWith("--")) {
      throw new Error(`unknown option ${argument}`);
    } else {
      positional.push(argument);
    }
  }

  if (positional.length !== 0 && positional.length !== 3) {
    throw new Error(
      "usage: verify-macos-release.mjs [<app-path> <dmg-path> <manifest-path>] [--output <path>] [--build-commit <full-sha>] [--skip-env]",
    );
  }
  if (positional.length === 3 && outputPath !== undefined) {
    throw new Error("--output cannot be combined with an explicit manifest path");
  }
  if (positional.length === 3) {
    return {
      mode: "explicit",
      appPath: path.resolve(positional[0]),
      dmgPath: path.resolve(positional[1]),
      outputPath: path.resolve(positional[2]),
      buildCommit,
      skipEnv,
    };
  }
  return { mode: "discovery", buildCommit, outputPath, skipEnv };
}

function candidatesIn(directory, suffix, fsApi) {
  try {
    return fsApi
      .readdirSync(directory, { withFileTypes: true })
      .filter((entry) =>
        suffix === ".app"
          ? entry.isDirectory() && entry.name.endsWith(suffix)
          : entry.isFile() && entry.name.endsWith(suffix),
      )
      .map((entry) => path.join(directory, entry.name))
      .sort();
  } catch {
    return [];
  }
}

/** Discovers exactly one normal Tauri app and DMG output. */
export function discoverTauriArtifacts(appDirectory, fsApi = fs) {
  const bundleRoot = path.join(appDirectory, "src-tauri", "target", "release", "bundle");
  const apps = candidatesIn(path.join(bundleRoot, "macos"), ".app", fsApi);
  const dmgs = candidatesIn(path.join(bundleRoot, "dmg"), ".dmg", fsApi);
  if (apps.length !== 1) {
    throw new Error(`release discovery requires exactly one application bundle; found ${apps.length}`);
  }
  if (dmgs.length !== 1) {
    throw new Error(`release discovery requires exactly one disk image; found ${dmgs.length}`);
  }
  return { appPath: apps[0], dmgPath: dmgs[0] };
}

function defaultOutputPath(appDirectory, fsApi) {
  let config;
  try {
    config = JSON.parse(
      fsApi.readFileSync(path.join(appDirectory, "src-tauri", "tauri.conf.json"), "utf8"),
    );
  } catch {
    throw new Error("default release manifest metadata could not be read");
  }
  if (typeof config.version !== "string" || config.version.trim() === "") {
    throw new Error("Tauri release version is missing");
  }
  const architecture = process.arch === "x64" ? "x86_64" : process.arch;
  return path.join(
    appDirectory,
    "release-artifacts",
    `emuchef-config-editor-${config.version}-macos-${architecture}.json`,
  );
}

function runCapturedNodeScript(run, scriptPath, args, label) {
  const result = run(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${label} failed`);
  }
}

/**
 * Executes the maintained release checks in a fixed order. Child output is
 * captured so private absolute paths and raw tool output never reach the final
 * summary.
 */
export function verifyMacosRelease(
  options,
  {
    appDirectory = pathsFromScript().appDirectory,
    repositoryRoot = pathsFromScript().repositoryRoot,
    scriptDirectory = pathsFromScript().scriptDirectory,
    fsApi = fs,
    run = spawnSync,
    discover = discoverTauriArtifacts,
    validateEnvironment = validateAppleReleaseEnvironment,
    scanRepository = assertNoRepositoryCredentialFiles,
    verifySigned = verifySignedMacosRelease,
    inspectBundle = inspectMacosBundle,
    generateManifest = generateMacosReleaseManifest,
    runSmoke = runCapturedNodeScript,
  } = {},
) {
  let appPath = options.appPath;
  let dmgPath = options.dmgPath;
  let outputPath = options.outputPath;
  if (options.mode === "discovery") {
    ({ appPath, dmgPath } = discover(appDirectory, fsApi));
    outputPath ??= defaultOutputPath(appDirectory, fsApi);
  }

  let environment = "passed";
  if (options.skipEnv) {
    scanRepository(repositoryRoot, fsApi);
    environment = "skipped_after_repository_scan";
  } else {
    validateEnvironment({ repositoryRoot, fsApi, run });
  }

  const signedResult = verifySigned(appPath, dmgPath, { fsApi, run });
  inspectBundle(appPath);
  runSmoke(
    run,
    path.join(scriptDirectory, "smoke-macos-packaged-app.mjs"),
    [appPath],
    "packaged application and sidecar smoke",
  );
  runSmoke(
    run,
    path.join(scriptDirectory, "smoke-packaged-runtime-network.mjs"),
    [appPath],
    "packaged-runtime network smoke",
  );
  const manifest = generateManifest(
    { appPath, dmgPath, outputPath, buildCommit: options.buildCommit },
    {
      fsApi,
      repositoryRoot,
      run,
      verify: () => signedResult,
    },
  );

  return {
    kind: "macos_release_verification",
    status: "passed",
    mode: options.mode,
    checks: {
      environment,
      signedArtifacts: "passed",
      bundleInspection: "passed",
      packagedApplicationSmoke: "passed",
      packagedRuntimeNetworkSmoke: "passed",
      manifest: "generated",
    },
    manifest: path.basename(outputPath),
    manifestSchemaVersion: manifest.schemaVersion,
  };
}

function main() {
  const options = parseVerifyArguments(process.argv.slice(2));
  console.log(JSON.stringify(verifyMacosRelease(options), null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`verify-macos-release: ${error.message}`);
    process.exit(1);
  }
}

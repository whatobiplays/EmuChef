#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { parseAppPath } from "./check-macos-bundle.mjs";

export function packagedRuntimePath(appPath) {
  return path.join(appPath, "Contents", "MacOS", "emuchef");
}

function main() {
  const appPath = parseAppPath(process.argv.slice(2));
  const binaryPath = packagedRuntimePath(appPath);
  if (!fs.existsSync(binaryPath) || !fs.statSync(binaryPath).isFile()) {
    throw new Error(`bundled Rust executable was not found at '${binaryPath}'`);
  }
  fs.accessSync(binaryPath, fs.constants.X_OK);

  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(scriptDir, "../../..");
  const manifestPath = path.join(repoRoot, "crates", "emuchef-rust-backend", "Cargo.toml");
  const result = spawnSync(
    "cargo",
    [
      "test",
      "--manifest-path",
      manifestPath,
      "--test",
      "network_artifact_cli",
      "--",
      "--nocapture",
    ],
    {
      cwd: repoRoot,
      env: { ...process.env, EMUCHEF_TEST_BINARY: binaryPath },
      stdio: "inherit",
    },
  );
  if (result.error) {
    throw new Error(`failed to run packaged-runtime network smoke: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`packaged-runtime network smoke exited ${result.status}`);
  }
  console.log(
    JSON.stringify({ kind: "packaged_runtime_network_smoke", status: "passed", appPath }, null, 2),
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`smoke-packaged-runtime-network: ${error.message}`);
    process.exit(1);
  }
}


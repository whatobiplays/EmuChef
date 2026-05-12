#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  externalBinArtifactName,
  isWindowsTargetTriple,
  packagedBinaryNameForTargetTriple,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";

export function inspectPreparedSidecarBundleInput({ appDir, profile, targetTriple }) {
  const artifactName = externalBinArtifactName(targetTriple);
  const artifactPath = path.join(appDir, "src-tauri", "binaries", artifactName);
  if (!fs.existsSync(artifactPath) || !fs.statSync(artifactPath).isFile()) {
    throw new Error(`prepared sidecar artifact was not found at '${artifactPath}'`);
  }

  const metadataPath = `${artifactPath}.metadata.json`;
  if (!fs.existsSync(metadataPath) || !fs.statSync(metadataPath).isFile()) {
    throw new Error(`prepared sidecar metadata was not found at '${metadataPath}'`);
  }

  const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
  if (metadata.profile !== profile) {
    throw new Error(
      `prepared sidecar metadata profile '${metadata.profile}' did not match expected '${profile}'`,
    );
  }
  if (metadata.targetTriple !== targetTriple) {
    throw new Error(
      `prepared sidecar metadata target '${metadata.targetTriple}' did not match expected '${targetTriple}'`,
    );
  }

  const mode = fs.statSync(artifactPath).mode;
  const executable = isWindowsTargetTriple(targetTriple) ? true : (mode & 0o111) !== 0;
  if (!executable) {
    throw new Error(`prepared sidecar artifact is not executable: '${artifactPath}'`);
  }

  return {
    artifactName,
    artifactPath,
    executable,
    metadata,
    metadataPath,
    packagedName: packagedBinaryNameForTargetTriple(targetTriple),
    profile,
    targetTriple,
  };
}

function parseArgs(argv) {
  const options = { profile: "debug", targetTriple: null };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--profile") {
      options.profile = argv[index + 1];
      index += 1;
    } else if (arg.startsWith("--profile=")) {
      options.profile = arg.slice("--profile=".length);
    } else if (arg === "--target-triple" || arg === "--target") {
      options.targetTriple = argv[index + 1];
      index += 1;
    } else if (arg.startsWith("--target-triple=")) {
      options.targetTriple = arg.slice("--target-triple=".length);
    } else if (arg.startsWith("--target=")) {
      options.targetTriple = arg.slice("--target=".length);
    } else {
      throw new Error(`unknown argument '${arg}'`);
    }
  }
  if (!["debug", "release"].includes(options.profile)) {
    throw new Error("--profile must be 'debug' or 'release'");
  }
  return options;
}

function hostTargetTriple() {
  const result = spawnSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" });
  if (result.error) {
    throw new Error(`failed to run 'rustc --print host-tuple': ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`'rustc --print host-tuple' exited ${result.status}: ${result.stderr.trim()}`);
  }
  return validateTargetTriple(result.stdout.trim(), "rustc --print host-tuple");
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const appDir = path.resolve(scriptDir, "..");
  const targetTriple = validateTargetTriple(
    options.targetTriple ?? hostTargetTriple(),
    options.targetTriple ? "command line" : "rustc --print host-tuple",
  );
  const result = inspectPreparedSidecarBundleInput({
    appDir,
    profile: options.profile,
    targetTriple,
  });

  console.log(`Prepared Rust sidecar externalBin input verified: ${result.artifactPath}`);
  console.log(`Packaged sidecar launch name after Tauri bundling: ${result.packagedName}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`inspect-sidecar-bundle-input: ${error.message}`);
    process.exit(1);
  }
}

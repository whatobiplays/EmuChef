#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  BINARY_BASENAME,
  binaryExtensionForTargetTriple,
  externalBinArtifactName,
  isWindowsTargetTriple,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";

function fail(message) {
  console.error(`prepare-rust-sidecar: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const options = {
    profile: "debug",
    targetTriple: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--release") {
      options.profile = "release";
    } else if (arg === "--profile") {
      const value = argv[index + 1];
      if (!value) {
        fail("--profile requires 'debug' or 'release'");
      }
      options.profile = value;
      index += 1;
    } else if (arg.startsWith("--profile=")) {
      options.profile = arg.slice("--profile=".length);
    } else if (arg === "--target-triple" || arg === "--target") {
      const value = argv[index + 1];
      if (!value) {
        fail(`${arg} requires a target triple`);
      }
      options.targetTriple = value;
      index += 1;
    } else if (arg.startsWith("--target-triple=")) {
      options.targetTriple = arg.slice("--target-triple=".length);
    } else if (arg.startsWith("--target=")) {
      options.targetTriple = arg.slice("--target=".length);
    } else {
      fail(`unknown argument '${arg}'`);
    }
  }

  if (!["debug", "release"].includes(options.profile)) {
    fail("--profile must be 'debug' or 'release'");
  }

  return options;
}

function hostTargetTriple() {
  const result = spawnSync("rustc", ["--print", "host-tuple"], {
    encoding: "utf8",
  });
  if (result.error) {
    fail(`failed to run 'rustc --print host-tuple': ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(
      `'rustc --print host-tuple' exited ${result.status}: ${result.stderr.trim()}`,
    );
  }
  return checkedTargetTriple(result.stdout.trim(), "rustc --print host-tuple");
}

function runCargoBuild({ manifestPath, profile }) {
  const args = ["build", "--manifest-path", manifestPath];
  if (profile === "release") {
    args.push("--release");
  }
  const result = spawnSync("cargo", args, { stdio: "inherit" });
  if (result.error) {
    fail(`failed to run cargo build: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`cargo build exited ${result.status}`);
  }
}

function checkedTargetTriple(value, source) {
  try {
    return validateTargetTriple(value, source);
  } catch (error) {
    fail(error.message);
  }
}

function ensureFreshCopy({ source, destination, targetTriple, profile }) {
  if (!fs.existsSync(source)) {
    fail(`cargo build completed but sidecar binary was not found at '${source}'`);
  }

  fs.mkdirSync(path.dirname(destination), { recursive: true });

  const sourceStat = fs.statSync(source);
  let copied = false;
  if (fs.existsSync(destination)) {
    const destinationStat = fs.statSync(destination);
    const sameSize = sourceStat.size === destinationStat.size;
    const sameBytes =
      sameSize &&
      fs.readFileSync(source).equals(fs.readFileSync(destination));
    if (!sameBytes || destinationStat.mtimeMs + 1 < sourceStat.mtimeMs) {
      fs.copyFileSync(source, destination);
      copied = true;
    } else {
      fs.utimesSync(destination, new Date(), new Date());
    }
  } else {
    fs.copyFileSync(source, destination);
    copied = true;
  }

  if (!isWindowsTargetTriple(targetTriple)) {
    fs.chmodSync(destination, 0o755);
  }

  const destinationStat = fs.statSync(destination);
  if (destinationStat.mtimeMs + 1 < sourceStat.mtimeMs) {
    fail(
      `copied sidecar '${destination}' is older than source binary '${source}'`,
    );
  }

  const metadataPath = `${destination}.metadata.json`;
  fs.writeFileSync(
    metadataPath,
    `${JSON.stringify(
      {
        binary: BINARY_BASENAME,
        profile,
        targetTriple,
        source,
        destination,
        sourceMtimeMs: sourceStat.mtimeMs,
        sourceSize: sourceStat.size,
        destinationMtimeMs: destinationStat.mtimeMs,
        destinationSize: destinationStat.size,
        copied,
        generatedAt: new Date().toISOString(),
      },
      null,
      2,
    )}\n`,
  );

  return copied;
}

const options = parseArgs(process.argv.slice(2));
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(appDir, "../..");
const crateDir = path.join(repoRoot, "crates", "emuchef-rust-backend");
const manifestPath = path.join(crateDir, "Cargo.toml");
const hostTriple = hostTargetTriple();
const targetTriple = checkedTargetTriple(
  options.targetTriple ?? hostTriple,
  options.targetTriple ? "command line" : "rustc --print host-tuple",
);

if (targetTriple !== hostTriple) {
  fail(
    `6V only prepares host-target sidecars. Host target is '${hostTriple}', requested '${targetTriple}'.`,
  );
}

runCargoBuild({ manifestPath, profile: options.profile });

const profileDir = options.profile === "release" ? "release" : "debug";
const extension = binaryExtensionForTargetTriple(targetTriple);
const source = path.join(crateDir, "target", profileDir, `${BINARY_BASENAME}${extension}`);
const destination = path.join(
  appDir,
  "src-tauri",
  "binaries",
  externalBinArtifactName(targetTriple),
);
const copied = ensureFreshCopy({
  source,
  destination,
  targetTriple,
  profile: options.profile,
});

console.log(
  `Prepared ${options.profile} Rust sidecar for ${targetTriple}: ${destination}${
    copied ? "" : " (already current)"
  }`,
);

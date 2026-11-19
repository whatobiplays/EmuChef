#!/usr/bin/env node

/** Runs the built host sidecar handshake without connecting to a device. */

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { targetExecutableExtension } from "../../../scripts/sidecar-target-contract.mjs";
import { buildMaterialIdentity } from "../../../tools/device-qualification.mjs";
import { BINARY_BASENAME, externalBinArtifactName } from "./sidecar-packaging.mjs";

const APP_DIRECTORY = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPOSITORY_ROOT = path.resolve(APP_DIRECTORY, "../..");
const PROCESS_TIMEOUT_MS = 10_000;

function runChecked(
  command,
  args,
  { env = process.env, timeoutMs = PROCESS_TIMEOUT_MS } = {},
) {
  const result = spawnSync(command, args, {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    env,
    timeout: timeoutMs,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const output = [result.stderr, result.stdout].filter(Boolean).join("\n").trim();
    throw new Error(`${command} failed (${result.status}): ${output}`);
  }
  return result.stdout.trim();
}

function sidecarArguments(name, tempDirectory, adbPath, identityText) {
  const runtimeRoot = path.join(tempDirectory, `${name}-runtime`);
  const cacheRoot = path.join(tempDirectory, `${name}-cache`);
  mkdirSync(runtimeRoot);
  mkdirSync(cacheRoot);
  const args = [
    "--sidecar",
    "--runtime-root",
    runtimeRoot,
    "--cache-root",
    cacheRoot,
    "--adb",
    adbPath,
  ];
  if (identityText !== null) {
    args.push(
      "--restrict-privileged-execution",
      "--qualification-source-identity",
      identityText,
      "--runtime-generation",
      "1",
    );
  }
  return args;
}

function runSidecar(binaryPath, args, env, request = null) {
  const result = spawnSync(binaryPath, args, {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    env,
    input: request === null ? undefined : `${JSON.stringify(request)}\n`,
    timeout: PROCESS_TIMEOUT_MS,
  });
  if (result.error) throw result.error;
  return result;
}

function responseFrom(result, expectedId) {
  const lines = result.stdout.split(/\r?\n/).filter(Boolean);
  assert.equal(lines.length, 1, `expected one JSONL response, got: ${result.stdout}`);
  const response = JSON.parse(lines[0]);
  assert.equal(response.id, expectedId);
  return response;
}

function hello(id, runtimeGeneration) {
  return {
    id,
    type: "hello",
    payload: runtimeGeneration === null ? {} : { runtimeGeneration },
  };
}

/** Verifies both Android helper ABIs in a restricted backend build. */
function assertRestrictedHelperAbis(sourceIdentity) {
  const testOutput = runChecked(
    "cargo",
    [
      "test",
      "--locked",
      "--color",
      "never",
      "--manifest-path",
      path.join(REPOSITORY_ROOT, "crates/emuchef-rust-backend/Cargo.toml"),
      "--no-default-features",
      "--lib",
      "--features",
      "real-execution",
      "executor::adb::tests::packaged_android_helper_materializes_outside_the_cargo_build_tree",
      "--",
      "--exact",
    ],
    {
      env: {
        ...process.env,
        EMU_CHEF_RESTRICTED_EXECUTION_BUILD_IDENTITY:
          JSON.stringify(sourceIdentity),
      },
      timeoutMs: 300_000,
    },
  );
  assert.match(
    testOutput,
    /test result: ok\. 1 passed; 0 failed;/,
    "restricted Cargo helper ABI test did not run exactly once and pass",
  );
}

function main() {
  const sourceIdentity = buildMaterialIdentity({
    repoRoot: REPOSITORY_ROOT,
    requireClean: true,
  });
  const target = runChecked("rustc", ["--print", "host-tuple"]);
  const binaryPath = path.join(
    APP_DIRECTORY,
    "src-tauri",
    "binaries",
    externalBinArtifactName(target),
  );
  assert.ok(existsSync(binaryPath), `built sidecar is missing: ${binaryPath}`);
  assert.ok(statSync(binaryPath).isFile());
  accessSync(binaryPath, constants.X_OK);

  const binaryBytes = readFileSync(binaryPath);
  const binaryDigest = createHash("sha256").update(binaryBytes).digest("hex");
  const cargoExecutablePath = path.join(
    REPOSITORY_ROOT,
    "crates/emuchef-rust-backend/target/debug",
    `${BINARY_BASENAME}${targetExecutableExtension(target)}`,
  );
  assert.ok(
    existsSync(cargoExecutablePath),
    `Cargo debug executable is missing: ${cargoExecutablePath}`,
  );
  assert.ok(
    binaryBytes.equals(readFileSync(cargoExecutablePath)),
    "copied sidecar differs from Cargo's debug executable",
  );
  assertRestrictedHelperAbis(sourceIdentity);

  const tempDirectory = mkdtempSync(
    path.join(os.tmpdir(), "emuchef-restricted-sidecar-smoke-"),
  );
  try {
    const adbDirectory = path.join(tempDirectory, "bin");
    const adbPath = path.join(adbDirectory, "adb");
    const adbLogPath = path.join(tempDirectory, "adb-invocations.log");
    mkdirSync(adbDirectory);
    writeFileSync(
      adbPath,
      '#!/bin/sh\nprintf "%s\\n" "$*" >> "$EMUCHEF_SMOKE_ADB_LOG"\nexit 97\n',
      { mode: 0o700 },
    );

    const baseEnvironment = {
      ...process.env,
      PATH: adbDirectory,
      EMUCHEF_SMOKE_ADB_LOG: adbLogPath,
    };
    const restrictedEnvironment = {
      ...baseEnvironment,
      EMUCHEF_DEVICE_QUALIFICATION: "1",
    };
    const ordinaryEnvironment = { ...baseEnvironment };
    delete ordinaryEnvironment.EMUCHEF_DEVICE_QUALIFICATION;
    delete ordinaryEnvironment.EMUCHEF_RESTRICTED_EXECUTION_BUILD;

    const identityText = JSON.stringify(sourceIdentity);
    const positiveId = "app-1";
    const positive = runSidecar(
      binaryPath,
      sidecarArguments("restricted-positive", tempDirectory, adbPath, identityText),
      restrictedEnvironment,
      hello(positiveId, 1),
    );
    assert.equal(positive.status, 0, positive.stderr);
    const positiveResponse = responseFrom(positive, positiveId);
    assert.equal(positiveResponse.ok, true, JSON.stringify(positiveResponse));
    assert.deepEqual(positiveResponse.result.executionRestriction, {
      enabled: true,
      buildIdentity: identityText,
      runtimeGeneration: 1,
      strategy: "adb_shell",
      rootShell: false,
      appDataWrite: false,
    });

    const mismatchPrefix = sourceIdentity.gitCommit[0] === "0" ? "1" : "0";
    const mismatchedIdentity = {
      ...sourceIdentity,
      gitCommit: `${mismatchPrefix}${sourceIdentity.gitCommit.slice(1)}`,
    };
    const identityMismatch = runSidecar(
      binaryPath,
      sidecarArguments(
        "restricted-identity-mismatch",
        tempDirectory,
        adbPath,
        JSON.stringify(mismatchedIdentity),
      ),
      restrictedEnvironment,
    );
    assert.equal(identityMismatch.status, 2, identityMismatch.stderr);
    assert.match(identityMismatch.stderr.toLowerCase(), /build identity does not match/);
    assert.equal(identityMismatch.stdout.trim(), "");

    const generationMismatch = runSidecar(
      binaryPath,
      sidecarArguments("restricted-generation-mismatch", tempDirectory, adbPath, identityText),
      restrictedEnvironment,
      hello("app-1", 2),
    );
    assert.equal(generationMismatch.status, 0, generationMismatch.stderr);
    const generationResponse = responseFrom(generationMismatch, "app-1");
    assert.equal(generationResponse.ok, false, JSON.stringify(generationResponse));
    assert.match(JSON.stringify(generationResponse).toLowerCase(), /generation/);
    assert.equal(generationResponse.result?.executionRestriction, undefined);

    const ordinary = runSidecar(
      binaryPath,
      sidecarArguments("ordinary-default", tempDirectory, adbPath, null),
      ordinaryEnvironment,
      hello("app-1", null),
    );
    assert.equal(ordinary.status, 0, ordinary.stderr);
    const ordinaryResponse = responseFrom(ordinary, "app-1");
    assert.equal(ordinaryResponse.ok, true, JSON.stringify(ordinaryResponse));
    assert.equal(ordinaryResponse.result.executionRestriction.enabled, false);
    assert.equal(ordinaryResponse.result.executionRestriction.buildIdentity, null);

    assert.equal(existsSync(adbLogPath), false, "the host handshake invoked the ADB stub");
    assert.deepEqual(
      buildMaterialIdentity({ repoRoot: REPOSITORY_ROOT, requireClean: true }),
      sourceIdentity,
      "source identity changed during the smoke run",
    );
    assert.equal(
      createHash("sha256").update(readFileSync(binaryPath)).digest("hex"),
      binaryDigest,
      "built sidecar changed during the smoke run",
    );
  } finally {
    rmSync(tempDirectory, { recursive: true, force: true });
  }

  console.log(
    `Restricted sidecar host handshake passed for ${sourceIdentity.gitCommit} (${target}).`,
  );
}

try {
  main();
} catch (error) {
  console.error(error.stack ?? error.message);
  process.exitCode = 1;
}

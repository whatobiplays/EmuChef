import assert from "node:assert/strict";
import test from "node:test";

import {
  sidecarBuildDisposition,
  sidecarCargoBuildArgs,
} from "./sidecar-build-contract.mjs";

import {
  externalBinArtifactName,
  macosArchitecture,
  requireQualifiedMacosTarget,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";


test("validates safe target triples", () => {
  assert.equal(validateTargetTriple("aarch64-apple-darwin"), "aarch64-apple-darwin");
  assert.equal(validateTargetTriple("x86_64-unknown-linux-gnu"), "x86_64-unknown-linux-gnu");
  for (const invalid of ["", "darwin", "aarch64/apple/darwin", "aarch64 apple darwin", null]) {
    assert.throws(() => validateTargetTriple(invalid), /unexpected target triple/);
  }
  assert.throws(() => validateTargetTriple("aarch64-apple-darwin-extra"), /unexpected target triple/);
});

test("maps known macOS architectures and qualifies only Apple Silicon", () => {
  assert.equal(macosArchitecture("aarch64-apple-darwin"), "arm64");
  assert.equal(macosArchitecture("x86_64-apple-darwin"), "x86_64");
  assert.equal(macosArchitecture("universal-apple-darwin"), "universal");
  assert.throws(() => macosArchitecture("aarch64-unknown-linux-gnu"), /Unsupported macOS/);
  assert.equal(requireQualifiedMacosTarget("aarch64-apple-darwin"), "arm64");
  assert.throws(() => requireQualifiedMacosTarget("x86_64-apple-darwin"), /qualifies only/);
});

test("uses the Tauri external-binary target suffix", () => {
  assert.equal(externalBinArtifactName("aarch64-apple-darwin"), "emuchef-aarch64-apple-darwin");
  assert.equal(externalBinArtifactName("x86_64-pc-windows-msvc"), "emuchef-x86_64-pc-windows-msvc.exe");
});

test("release sidecar builds include the Android helper without enabling real execution", () => {
  const args = sidecarCargoBuildArgs("release", "/repo/backend/Cargo.toml");
  assert.deepEqual(args.slice(-3), ["--release", "--features", "android-copy-commit-helper"]);
  assert.equal(args.includes("real-execution"), false);
});

test("qualification sidecar builds both required backend features in an isolated profile", () => {
  assert.deepEqual(
    sidecarCargoBuildArgs("qualification", "/repo/backend/Cargo.toml"),
    [
      "build",
      "--manifest-path",
      "/repo/backend/Cargo.toml",
      "--bin",
      "emuchef",
      "--profile",
      "qualification",
      "--features",
      "real-execution,android-copy-commit-helper",
    ],
  );
});

test("ordinary sidecar profiles keep their existing feature defaults", () => {
  assert.deepEqual(
    sidecarCargoBuildArgs("debug", "/repo/backend/Cargo.toml"),
    ["build", "--manifest-path", "/repo/backend/Cargo.toml", "--bin", "emuchef"],
  );
  assert.deepEqual(
    sidecarCargoBuildArgs("release", "/repo/backend/Cargo.toml"),
    [
      "build",
      "--manifest-path",
      "/repo/backend/Cargo.toml",
      "--bin",
      "emuchef",
      "--release",
      "--features",
      "android-copy-commit-helper",
    ],
  );
});

test("release hooks refuse qualification selection and pinned artifacts", () => {
  const env = { EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1" };
  assert.equal(sidecarBuildDisposition("qualification", env), "build");
  assert.equal(sidecarBuildDisposition("debug", env), "verify-existing");
  assert.equal(
    sidecarBuildDisposition("release", env),
    "refuse-release-qualification",
  );
  assert.equal(
    sidecarBuildDisposition("release", {}, true),
    "refuse-release-qualification",
  );
  assert.equal(sidecarBuildDisposition("release", {}), "build");
  assert.equal(sidecarBuildDisposition("debug", {}), "build");
});

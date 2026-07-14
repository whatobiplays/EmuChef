import assert from "node:assert/strict";
import test from "node:test";

import {
  externalBinArtifactName,
  macosArchitecture,
  requireQualifiedMacosTarget,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";

test("validates safe target triples", () => {
  assert.equal(validateTargetTriple("aarch64-apple-darwin"), "aarch64-apple-darwin");
  for (const invalid of ["", "darwin", "aarch64/apple/darwin", "aarch64 apple darwin", null]) {
    assert.throws(() => validateTargetTriple(invalid), /unexpected target triple/);
  }
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

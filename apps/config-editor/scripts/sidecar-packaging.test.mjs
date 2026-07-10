import assert from "node:assert/strict";
import test from "node:test";

import {
  BINARY_BASENAME,
  binaryExtensionForTargetTriple,
  externalBinArtifactName,
  packagedBinaryNameForTargetTriple,
  validateTargetTriple,
} from "./sidecar-packaging.mjs";

test("builds Tauri externalBin input names for Unix-like target triples", () => {
  assert.equal(BINARY_BASENAME, "emuchef");
  assert.equal(binaryExtensionForTargetTriple("aarch64-apple-darwin"), "");
  assert.equal(
    externalBinArtifactName("aarch64-apple-darwin"),
    "emuchef-aarch64-apple-darwin",
  );
  assert.equal(
    packagedBinaryNameForTargetTriple("aarch64-apple-darwin"),
    "emuchef",
  );
});

test("builds Tauri externalBin input and packaged names for Windows target triples", () => {
  assert.equal(binaryExtensionForTargetTriple("x86_64-pc-windows-msvc"), ".exe");
  assert.equal(
    externalBinArtifactName("x86_64-pc-windows-msvc"),
    "emuchef-x86_64-pc-windows-msvc.exe",
  );
  assert.equal(
    packagedBinaryNameForTargetTriple("x86_64-pc-windows-msvc"),
    "emuchef.exe",
  );
});

test("rejects malformed or unsafe target triples", () => {
  for (const value of ["", "darwin", "aarch64 apple darwin", "../aarch64-apple-darwin", "aarch64\\apple\\darwin"]) {
    assert.throws(
      () => validateTargetTriple(value, "test input"),
      /unexpected target triple|unsafe target triple|empty target triple/,
      `expected '${value}' to be rejected`,
    );
  }
});

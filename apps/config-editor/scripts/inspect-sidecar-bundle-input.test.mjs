import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { inspectPreparedSidecarBundleInput } from "./inspect-sidecar-bundle-input.mjs";

test("inspects prepared externalBin source artifact and metadata", () => {
  const appDir = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-bundle-input-"));
  const targetTriple = "aarch64-apple-darwin";
  const profile = "debug";
  const binariesDir = path.join(appDir, "src-tauri", "binaries");
  const artifactPath = path.join(binariesDir, "emuchef-aarch64-apple-darwin");
  fs.mkdirSync(binariesDir, { recursive: true });
  fs.writeFileSync(artifactPath, "sidecar");
  fs.chmodSync(artifactPath, 0o755);
  fs.writeFileSync(
    `${artifactPath}.metadata.json`,
    `${JSON.stringify({ profile, targetTriple, destination: artifactPath }, null, 2)}\n`,
  );

  const result = inspectPreparedSidecarBundleInput({ appDir, profile, targetTriple });

  assert.equal(result.artifactName, "emuchef-aarch64-apple-darwin");
  assert.equal(result.packagedName, "emuchef");
  assert.equal(result.metadata.profile, profile);
  assert.equal(result.metadata.targetTriple, targetTriple);
  assert.equal(result.executable, true);
});

test("reports missing prepared externalBin source artifact", () => {
  const appDir = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-bundle-input-missing-"));

  assert.throws(
    () =>
      inspectPreparedSidecarBundleInput({
        appDir,
        profile: "release",
        targetTriple: "aarch64-apple-darwin",
      }),
    /prepared sidecar artifact was not found/,
  );
});

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { verifyQualificationSidecar } from "./verify-qualification-sidecar.mjs";

const TARGET = "aarch64-apple-darwin";
const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";
const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;

function createFixture(t) {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-configuration-create-verifier-"));
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));

  const appRoot = path.join(repoRoot, "apps/emuchef-app");
  const sidecarPath = path.join(
    appRoot,
    "src-tauri/binaries/emuchef-aarch64-apple-darwin",
  );
  const runtimeSidecarPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug/emuchef",
  );
  const manifestPath = path.join(repoRoot, "qualification.json");
  const sidecarBytes = Buffer.from("qualified sidecar bytes");
  const helperDigest = `sha256:${"b".repeat(64)}`;
  const materialInputsDigest = `sha256:${"c".repeat(64)}`;
  const sourceIdentity = {
    gitCommit: "281d0929c253f187ef622e8be1dd6f438c566f5d",
    materialBuildDigest: `sha256:${"a".repeat(64)}`,
  };
  const provenance = {
    schemaVersion: 1,
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: helperDigest,
    gitCommit: sourceIdentity.gitCommit,
    materialBuildDigest: sourceIdentity.materialBuildDigest,
    materialInputsDigest,
  };

  fs.mkdirSync(path.dirname(sidecarPath), { recursive: true });
  fs.mkdirSync(path.dirname(runtimeSidecarPath), { recursive: true });
  fs.writeFileSync(sidecarPath, sidecarBytes);
  fs.writeFileSync(runtimeSidecarPath, sidecarBytes);
  fs.writeFileSync(
    manifestPath,
    JSON.stringify({
      schemaVersion: 1,
      sourceWorktreeClean: true,
      gitCommit: sourceIdentity.gitCommit,
      materialBuildDigest: sourceIdentity.materialBuildDigest,
      materialInputsDigest,
      qualificationTargetId: TARGET_ID,
      backendFeatures: provenance.backendFeatures,
      arm64HelperSha256: helperDigest,
      sidecarPath: "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
      runtimeSidecarPath: "crates/emuchef-rust-backend/target/debug/emuchef",
      sidecarSha256: digest(sidecarBytes),
      runtimeSidecarSha256: digest(sidecarBytes),
    }),
  );

  const calls = [];
  const spawn = (executable, args) => {
    calls.push({ executable, args });
    if (executable === "rustc") return { status: 0, stdout: `${TARGET}\n` };
    return { status: 0, stdout: JSON.stringify(provenance) };
  };
  const options = {
    repoRoot,
    appRoot,
    manifestPath,
    env: {},
    spawn,
    buildIdentity: () => sourceIdentity,
    materialInputs: () => ({ digest: materialInputsDigest }),
  };
  return { options, calls, provenance, sidecarPath, runtimeSidecarPath };
}

test("verifier resolves both sidecar paths and validates their bytes and provenance", (t) => {
  const fixture = createFixture(t);
  const result = verifyQualificationSidecar(fixture.options);

  assert.equal(result.sidecarPath, fixture.sidecarPath);
  assert.deepEqual(fixture.calls, [
    { executable: "rustc", args: ["--print", "host-tuple"] },
    {
      executable: fixture.sidecarPath,
      args: ["--qualification-build-provenance"],
    },
  ]);
});

test("verifier rejects changed qualification sidecar bytes", (t) => {
  const fixture = createFixture(t);
  fs.writeFileSync(fixture.sidecarPath, "modified sidecar bytes");

  assert.throws(
    () => verifyQualificationSidecar(fixture.options),
    /sidecar differs|binary changed/,
  );
});

test("verifier rejects binary provenance bound to a different source revision", (t) => {
  const fixture = createFixture(t);
  fixture.provenance.gitCommit = "f".repeat(40);

  assert.throws(
    () => verifyQualificationSidecar(fixture.options),
    /compiled qualification source identity/i,
  );
});

test("verifier rejects a runtime sidecar that differs from the packaged sidecar", (t) => {
  const fixture = createFixture(t);
  fs.writeFileSync(fixture.runtimeSidecarPath, "modified runtime bytes");

  assert.throws(
    () => verifyQualificationSidecar(fixture.options),
    /runtime sidecar differs/,
  );
});

test("verifier rejects symlinked sidecar artifacts before executing them", (t) => {
  const packaged = createFixture(t);
  const externalSidecar = path.join(packaged.options.repoRoot, "external-sidecar");
  fs.writeFileSync(externalSidecar, "external bytes");
  fs.unlinkSync(packaged.sidecarPath);
  fs.symlinkSync(externalSidecar, packaged.sidecarPath);

  assert.throws(
    () => verifyQualificationSidecar(packaged.options),
    /symlink/,
  );
  assert.deepEqual(packaged.calls, [
    { executable: "rustc", args: ["--print", "host-tuple"] },
  ]);

  const runtime = createFixture(t);
  const externalRuntime = path.join(runtime.options.repoRoot, "external-runtime");
  fs.writeFileSync(externalRuntime, "external bytes");
  fs.unlinkSync(runtime.runtimeSidecarPath);
  fs.symlinkSync(externalRuntime, runtime.runtimeSidecarPath);

  assert.throws(
    () => verifyQualificationSidecar(runtime.options),
    /symlink/,
  );
  assert.deepEqual(runtime.calls, [
    { executable: "rustc", args: ["--print", "host-tuple"] },
  ]);
});

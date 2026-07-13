import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  assertNoRepositoryCredentialFiles,
  findRepositoryCredentialFiles,
  validateAppleReleaseEnvironment,
} from "./check-apple-release-env.mjs";

const IDENTITY = "Developer ID Application: REDACTED";

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-release-env-"));
  const repositoryRoot = path.join(root, "repo");
  const privateDirectory = path.join(root, "private");
  fs.mkdirSync(repositoryRoot);
  fs.mkdirSync(privateDirectory);
  const keyPath = path.join(privateDirectory, "release-key.private");
  fs.writeFileSync(keyPath, "test-only-key-material");
  fs.chmodSync(keyPath, 0o600);
  const env = {
    APPLE_SIGNING_IDENTITY: IDENTITY,
    APPLE_API_ISSUER: "issuer-secret",
    APPLE_API_KEY: "key-secret",
    APPLE_API_KEY_PATH: keyPath,
  };
  const run = () => ({ status: 0, stdout: `1) HASH \"${IDENTITY}\"\n`, stderr: "" });
  return { env, keyPath, repositoryRoot, root, run };
}

function cleanup(value) {
  fs.rmSync(value.root, { force: true, recursive: true });
}

test("accepts a protected external key and installed Developer ID identity", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  const result = validateAppleReleaseEnvironment(value);
  assert.equal(result.status, "passed");
  assert.equal(result.checks.repositoryCredentialFilesAbsent, true);
  assert.doesNotMatch(JSON.stringify(result), /issuer-secret|key-secret|private/);
});

test("requires every release variable by name without exposing other values", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  for (const name of [
    "APPLE_SIGNING_IDENTITY",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_KEY_PATH",
  ]) {
    const env = { ...value.env, [name]: "" };
    assert.throws(
      () => validateAppleReleaseEnvironment({ ...value, env }),
      (error) => error.message.includes(name) && !error.message.includes("issuer-secret"),
    );
  }
});

test("rejects missing, directory, symlink, and in-repository key paths", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  const missing = { ...value.env, APPLE_API_KEY_PATH: path.join(value.root, "missing") };
  assert.throws(() => validateAppleReleaseEnvironment({ ...value, env: missing }), /existing/);

  const directory = { ...value.env, APPLE_API_KEY_PATH: path.dirname(value.keyPath) };
  assert.throws(() => validateAppleReleaseEnvironment({ ...value, env: directory }), /regular/);

  const linkPath = path.join(value.root, "key-link");
  fs.symlinkSync(value.keyPath, linkPath);
  const symlink = { ...value.env, APPLE_API_KEY_PATH: linkPath };
  assert.throws(() => validateAppleReleaseEnvironment({ ...value, env: symlink }), /regular/);

  const internalPath = path.join(value.repositoryRoot, "private-key.data");
  fs.writeFileSync(internalPath, "test-only");
  fs.chmodSync(internalPath, 0o600);
  const internal = { ...value.env, APPLE_API_KEY_PATH: internalPath };
  assert.throws(() => validateAppleReleaseEnvironment({ ...value, env: internal }), /outside/);
});

test("rejects group or world key permissions without printing the path", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  fs.chmodSync(value.keyPath, 0o640);
  assert.throws(
    () => validateAppleReleaseEnvironment(value),
    (error) => /group or world/.test(error.message) && !error.message.includes(value.keyPath),
  );
});

test("requires an exact installed Developer ID Application identity", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  assert.throws(
    () =>
      validateAppleReleaseEnvironment({
        ...value,
        env: { ...value.env, APPLE_SIGNING_IDENTITY: "Apple Development: REDACTED" },
      }),
    /Developer ID Application/,
  );
  assert.throws(
    () => validateAppleReleaseEnvironment({ ...value, run: () => ({ status: 0, stdout: "" }) }),
    /not found/,
  );
  assert.throws(
    () =>
      validateAppleReleaseEnvironment({
        ...value,
        run: () => ({ status: 1, stdout: IDENTITY, stderr: "sensitive output" }),
      }),
    (error) => /could not enumerate/.test(error.message) && !error.message.includes(IDENTITY),
  );
});

test("finds forbidden extensions and obvious Apple credential names", (t) => {
  const value = fixture();
  t.after(() => cleanup(value));
  fs.mkdirSync(path.join(value.repositoryRoot, ".git"));
  fs.writeFileSync(path.join(value.repositoryRoot, ".git", "ignored.p8"), "ignored metadata");
  fs.writeFileSync(path.join(value.repositoryRoot, "certificate.CER"), "test-only");
  fs.writeFileSync(path.join(value.repositoryRoot, "AuthKey_release.txt"), "test-only");
  fs.writeFileSync(path.join(value.repositoryRoot, "ordinary.txt"), "safe");
  assert.equal(findRepositoryCredentialFiles(value.repositoryRoot).length, 2);
  assert.throws(() => assertNoRepositoryCredentialFiles(value.repositoryRoot), /2 forbidden/);
});

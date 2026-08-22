import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

const SCRIPT = path.join(import.meta.dirname, "cargo-test-freshness.mjs");
const OLD_MTIME = new Date("2020-01-02T03:04:05Z");

function makeCrate() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-freshness-"));
  fs.mkdirSync(path.join(directory, "src"), { recursive: true });
  fs.mkdirSync(path.join(directory, "tests"), { recursive: true });
  fs.writeFileSync(
    path.join(directory, "Cargo.toml"),
    '[package]\nname = "emuchef-rust-backend"\nversion = "0.1.0"\nedition = "2021"\n',
  );
  fs.writeFileSync(path.join(directory, "Cargo.lock"), "# lockfile\n");
  fs.writeFileSync(
    path.join(directory, "src", "lib.rs"),
    "pub fn value() -> u8 { 1 }\n",
  );
  fs.writeFileSync(
    path.join(directory, "tests", "contract.rs"),
    "use fixture::value;\n#[test]\nfn works() { assert_eq!(value(), 1); }\n",
  );
  return directory;
}

function runFreshness(crateDir, stampFile) {
  return spawnSync(process.execPath, [SCRIPT, crateDir, stampFile], {
    encoding: "utf8",
  });
}

function setOldMtimes(directory) {
  const walk = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else {
        fs.utimesSync(full, OLD_MTIME, OLD_MTIME);
      }
    }
  };
  walk(directory);
}

function allSourceMtimesOld(crateDir) {
  const paths = ["Cargo.toml", "Cargo.lock", "src/lib.rs", "tests/contract.rs"];
  return paths.every(
    (relative) => fs.statSync(path.join(crateDir, relative)).mtimeMs === OLD_MTIME.getTime(),
  );
}

function stampDigest(stampFile) {
  return fs.readFileSync(stampFile, "utf8").trim();
}

test("missing stamp invalidates and records the current digest", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");

  const result = runFreshness(crateDir, stampFile);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /invalidate/);
  assert.ok(fs.existsSync(stampFile));
  assert.match(stampDigest(stampFile), /^[0-9a-f]{64}$/);
  assert.ok(fs.existsSync(`${stampFile}.pending`));
});

test("unchanged digest with old mtimes stays fresh and untouched", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");

  runFreshness(crateDir, stampFile);
  fs.rmSync(`${stampFile}.pending`);
  setOldMtimes(crateDir);
  const before = stampDigest(stampFile);

  const result = runFreshness(crateDir, stampFile);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /fresh/);
  assert.equal(stampDigest(stampFile), before);
  assert.ok(!fs.existsSync(`${stampFile}.pending`));
  assert.ok(allSourceMtimesOld(crateDir));
});

test("changed content with an unchanged old mtime invalidates", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");
  const libFile = path.join(crateDir, "src", "lib.rs");

  runFreshness(crateDir, stampFile);
  fs.rmSync(`${stampFile}.pending`);
  setOldMtimes(crateDir);
  const before = stampDigest(stampFile);
  fs.writeFileSync(libFile, "pub fn value() -> u8 { 2 }\n");
  fs.utimesSync(libFile, OLD_MTIME, OLD_MTIME);

  const result = runFreshness(crateDir, stampFile);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /invalidate/);
  assert.notEqual(stampDigest(stampFile), before);
  assert.ok(fs.existsSync(`${stampFile}.pending`));
  assert.equal(fs.statSync(libFile).mtimeMs, OLD_MTIME.getTime());
});

test("existing pending marker with a matching digest still invalidates", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");

  runFreshness(crateDir, stampFile);
  const before = stampDigest(stampFile);
  assert.ok(fs.existsSync(`${stampFile}.pending`));

  const result = runFreshness(crateDir, stampFile);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /invalidate/);
  assert.equal(stampDigest(stampFile), before);
  assert.ok(fs.existsSync(`${stampFile}.pending`));
});

test("ignored files do not alter the digest", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");

  runFreshness(crateDir, stampFile);
  fs.rmSync(`${stampFile}.pending`);
  setOldMtimes(crateDir);
  assert.match(runFreshness(crateDir, stampFile).stdout, /fresh/);

  fs.mkdirSync(path.join(crateDir, "target", "deep"), { recursive: true });
  fs.writeFileSync(path.join(crateDir, "target", "deep", "x.bin"), "ignored");
  fs.mkdirSync(path.join(crateDir, ".git"), { recursive: true });
  fs.writeFileSync(path.join(crateDir, ".git", "HEAD"), "ref: refs/heads/main\n");
  fs.writeFileSync(path.join(crateDir, ".DS_Store"), "junk");
  fs.mkdirSync(path.join(crateDir, "gen"), { recursive: true });
  fs.writeFileSync(path.join(crateDir, "gen", "out.txt"), "generated");

  const first = runFreshness(crateDir, stampFile);
  assert.match(first.stdout, /fresh/);

  fs.writeFileSync(path.join(crateDir, "target", "deep", "x.bin"), "changed");
  const second = runFreshness(crateDir, stampFile);
  assert.match(second.stdout, /fresh/);
});

test("digest computation failure fails closed", (t) => {
  const crateDir = makeCrate();
  t.after(() => fs.rmSync(crateDir, { recursive: true, force: true }));
  const stampFile = path.join(crateDir, "target", ".fixture-source.sha256");
  // Replacing the src directory with a regular file makes readdirSync throw
  // (ENOTDIR) on every platform and user. Unlike chmod 0o000, this does not
  // rely on mode-bit enforcement, which root, Windows, or permissive
  // filesystems may not honor.
  fs.rmSync(path.join(crateDir, "src"), { recursive: true, force: true });
  fs.writeFileSync(path.join(crateDir, "src"), "not a directory\n");

  const result = runFreshness(crateDir, stampFile);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /cargo-test-freshness/);
  assert.ok(!fs.existsSync(`${stampFile}.pending`));
});

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { APP_ROOT } from "./run-device-qualification.mjs";

const REPO_ROOT = path.resolve(APP_ROOT, "../..");
const BUILD_SCRIPT_SOURCE = path.join(APP_ROOT, "src-tauri", "build.rs");
const TOOL_SOURCE = path.join(REPO_ROOT, "tools/device-qualification.mjs");

function writeFixtureFile(fixtureRoot, relativePath, contents) {
  const destination = path.join(fixtureRoot, relativePath);
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, contents, "utf8");
}

function createCargoFixture() {
  const fixtureRoot = mkdtempSync(path.join(tmpdir(), "emuchef-qualification-cargo-"));
  const buildScriptPath = path.join(fixtureRoot, "apps/emuchef-app/src-tauri/build.rs");
  const toolPath = path.join(fixtureRoot, "tools/device-qualification.mjs");

  try {
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/package.json",
      `${JSON.stringify({ name: "qualification-build-fixture", version: "0.1.0" }, null, 2)}\n`,
    );
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/package-lock.json",
      `${JSON.stringify({
        name: "qualification-build-fixture",
        version: "0.1.0",
        lockfileVersion: 3,
        requires: true,
        packages: {
          "": { name: "qualification-build-fixture", version: "0.1.0" },
        },
      }, null, 2)}\n`,
    );
    writeFixtureFile(fixtureRoot, "apps/emuchef-app/src/main.js", "export const fixture = true;\n");
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/src/material.rs",
      "pub const MATERIAL_VALUE: u8 = 1;\n",
    );
    writeFixtureFile(fixtureRoot, "authored/input.yaml", "id: fixture-input\n");
    writeFixtureFile(
      fixtureRoot,
      "crates/emuchef-rust-backend/src/lib.rs",
      "pub const FIXTURE_BACKEND: bool = true;\n",
    );
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/Cargo.toml",
      `[package]
name = "qualification-build-fixture"
version = "0.1.0"
edition = "2021"
build = "build.rs"

[features]
default = []
real-execution = []

[dependencies]
tauri = { path = "../../../tauri-fixture" }

[build-dependencies]
serde_json = { version = "1.0", features = ["preserve_order"] }
tauri-build = { version = "2.5.1", features = [] }

[lib]
path = "src/lib.rs"
`,
    );
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/src/lib.rs",
      "pub fn fixture_library() {}\n",
    );
    writeFixtureFile(
      fixtureRoot,
      "tauri-fixture/Cargo.toml",
      `[package]
name = "tauri"
version = "2.11.0"
edition = "2021"
links = "tauri"
build = "build.rs"
`,
    );
    writeFixtureFile(
      fixtureRoot,
      "tauri-fixture/build.rs",
      'fn main() { println!("cargo:dev=true"); }\n',
    );
    writeFixtureFile(fixtureRoot, "tauri-fixture/src/lib.rs", "pub fn fixture_tauri() {}\n");
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/tauri.conf.json",
      `${JSON.stringify({
        "$schema": "https://schema.tauri.app/config/2",
        productName: "Qualification Build Fixture",
        version: "0.1.0",
        identifier: "com.emuchef.qualificationbuildfixture",
        build: { frontendDist: "." },
      }, null, 2)}\n`,
    );
    copyFileSync(BUILD_SCRIPT_SOURCE, buildScriptPath);
    mkdirSync(path.dirname(toolPath), { recursive: true });
    copyFileSync(TOOL_SOURCE, toolPath);
    execFileSync("cargo", [
      "generate-lockfile",
      "--manifest-path",
      path.join(fixtureRoot, "apps/emuchef-app/src-tauri/Cargo.toml"),
    ], {
      cwd: fixtureRoot,
      env: { ...process.env, CARGO_NET_OFFLINE: "true" },
      stdio: "pipe",
    });

    execFileSync("git", ["init"], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync("git", ["config", "user.name", "Codex"], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync("git", ["config", "user.email", "codex@example.com"], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync("git", ["add", "."], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync("git", ["commit", "-m", "initial Cargo qualification fixture"], {
      cwd: fixtureRoot,
      stdio: "pipe",
    });

    return {
      fixtureRoot,
      materialPath: path.join(fixtureRoot, "apps/emuchef-app/src-tauri/src/material.rs"),
      manifestPath: path.join(fixtureRoot, "apps/emuchef-app/src-tauri/Cargo.toml"),
      targetPath: path.join(fixtureRoot, "cargo-target"),
    };
  } catch (error) {
    rmSync(fixtureRoot, { recursive: true, force: true });
    throw error;
  }
}

function cargoQualificationCheck(fixture) {
  return spawnSync(
    "cargo",
    ["check", "--locked", "--manifest-path", fixture.manifestPath, "--features", "real-execution"],
    {
      cwd: fixture.fixtureRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        CARGO_NET_OFFLINE: "true",
        CARGO_TARGET_DIR: fixture.targetPath,
        EMUCHEF_DEVICE_QUALIFICATION: "1",
      },
    },
  );
}

function commandOutput(result) {
  return `${result.stdout}\n${result.stderr}`;
}

test("opted-in Cargo builds rerun build.rs after tracked material changes", () => {
  const fixture = createCargoFixture();

  try {
    const cleanBuild = cargoQualificationCheck(fixture);
    assert.equal(cleanBuild.status, 0, commandOutput(cleanBuild));

    writeFileSync(fixture.materialPath, "pub const MATERIAL_VALUE: u8 = 2;\n", "utf8");
    const dirtyFiles = execFileSync(
      "git",
      ["status", "--porcelain", "--untracked-files=no"],
      { cwd: fixture.fixtureRoot, encoding: "utf8" },
    );
    assert.match(dirtyFiles, /apps\/emuchef-app\/src-tauri\/src\/material\.rs/);

    const dirtyBuild = cargoQualificationCheck(fixture);
    assert.notEqual(dirtyBuild.status, 0, commandOutput(dirtyBuild));
    assert.match(commandOutput(dirtyBuild), /device qualification requires a clean tracked worktree/);
  } finally {
    rmSync(fixture.fixtureRoot, { recursive: true, force: true });
  }
});

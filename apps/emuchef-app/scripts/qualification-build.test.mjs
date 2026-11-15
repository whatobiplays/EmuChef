import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  cpSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import { APP_ROOT } from "./run-device-qualification.mjs";

const REPO_ROOT = path.resolve(APP_ROOT, "../..");
const CARGO_FIXTURE_ROOT = path.join(
  APP_ROOT,
  "scripts/fixtures/qualification-build",
);
const CARGO_FIXTURE_MANIFEST = path.join(
  CARGO_FIXTURE_ROOT,
  "apps/emuchef-app/src-tauri/Cargo.toml",
);
const BUILD_SCRIPT_SOURCE = path.join(APP_ROOT, "src-tauri", "build.rs");
const BUILD_MODULE_SOURCE = path.join(
  APP_ROOT,
  "src-tauri",
  "src/qualification_build.rs",
);
const TOOL_SOURCE = path.join(REPO_ROOT, "tools/device-qualification.mjs");
const INHERITED_BUILD_IDENTITY = JSON.stringify({
  appVersion: "0.1.0",
  gitCommit: "1".repeat(40),
  materialBuildDigest: `sha256:${"a".repeat(64)}`,
  realExecutionEnabled: true,
  qualificationContract: 1,
});

function writeFixtureFile(fixtureRoot, relativePath, contents) {
  const destination = path.join(fixtureRoot, relativePath);
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, contents, "utf8");
}

function createCargoFixture() {
  const fixtureRoot = mkdtempSync(
    path.join(tmpdir(), "emuchef-qualification-cargo-"),
  );
  const buildScriptPath = path.join(
    fixtureRoot,
    "apps/emuchef-app/src-tauri/build.rs",
  );
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
      `${JSON.stringify(
        {
          name: "qualification-build-fixture",
          version: "0.1.0",
          lockfileVersion: 3,
          requires: true,
          packages: {
            "": { name: "qualification-build-fixture", version: "0.1.0" },
          },
        },
        null,
        2,
      )}\n`,
    );
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src/main.js",
      "export const fixture = true;\n",
    );
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
    const fixtureManifestPath = path.join(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/Cargo.toml",
    );
    copyFileSync(CARGO_FIXTURE_MANIFEST, fixtureManifestPath);
    copyFileSync(
      path.join(path.dirname(CARGO_FIXTURE_MANIFEST), "Cargo.lock"),
      path.join(path.dirname(fixtureManifestPath), "Cargo.lock"),
    );
    cpSync(
      path.join(CARGO_FIXTURE_ROOT, "tauri-fixture"),
      path.join(fixtureRoot, "tauri-fixture"),
      { recursive: true },
    );
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/src/lib.rs",
      `#[path = "qualification_build.rs"]
mod qualification_build;

pub fn fixture_library() {}
pub fn fixture_build_state() -> (bool, bool) {
    (
        qualification_build::embedded_build_identity().is_some(),
        qualification_build::qualification_mode_enabled_at_runtime(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn inherited_identity_is_ignored_without_runtime_opt_in() {
        let (identity_present, mode_enabled) = super::fixture_build_state();
        assert!(!identity_present);
        assert!(!mode_enabled);
    }
}
`,
    );
    copyFileSync(
      BUILD_MODULE_SOURCE,
      path.join(
        fixtureRoot,
        "apps/emuchef-app/src-tauri/src/qualification_build.rs",
      ),
    );
    writeFixtureFile(fixtureRoot, ".gitignore", "/cargo-target/\n");
    writeFixtureFile(
      fixtureRoot,
      "apps/emuchef-app/src-tauri/tauri.conf.json",
      `${JSON.stringify(
        {
          $schema: "https://schema.tauri.app/config/2",
          productName: "Qualification Build Fixture",
          version: "0.1.0",
          identifier: "com.emuchef.qualificationbuildfixture",
          build: { frontendDist: "." },
        },
        null,
        2,
      )}\n`,
    );
    copyFileSync(BUILD_SCRIPT_SOURCE, buildScriptPath);
    mkdirSync(path.dirname(toolPath), { recursive: true });
    copyFileSync(TOOL_SOURCE, toolPath);
    execFileSync("git", ["init"], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync("git", ["config", "user.name", "Codex"], {
      cwd: fixtureRoot,
      stdio: "pipe",
    });
    execFileSync("git", ["config", "user.email", "codex@example.com"], {
      cwd: fixtureRoot,
      stdio: "pipe",
    });
    execFileSync("git", ["add", "."], { cwd: fixtureRoot, stdio: "pipe" });
    execFileSync(
      "git",
      ["commit", "-m", "initial Cargo qualification fixture"],
      {
        cwd: fixtureRoot,
        stdio: "pipe",
      },
    );

    return {
      fixtureRoot,
      materialPath: path.join(
        fixtureRoot,
        "apps/emuchef-app/src-tauri/src/material.rs",
      ),
      manifestPath: path.join(
        fixtureRoot,
        "apps/emuchef-app/src-tauri/Cargo.toml",
      ),
      targetPath: path.join(fixtureRoot, "cargo-target"),
    };
  } catch (error) {
    rmSync(fixtureRoot, { recursive: true, force: true });
    throw error;
  }
}

function cargoQualificationCheck(fixture, { cargoHome } = {}) {
  return spawnSync(
    "cargo",
    [
      "check",
      "--locked",
      "--manifest-path",
      fixture.manifestPath,
      "--features",
      "real-execution",
    ],
    {
      cwd: fixture.fixtureRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        ...(cargoHome ? { CARGO_HOME: cargoHome } : {}),
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

function cargoQualificationTestWithInheritedIdentity(fixture) {
  const environment = {
    ...process.env,
    CARGO_NET_OFFLINE: "true",
    CARGO_TARGET_DIR: fixture.targetPath,
    EMUCHEF_QUALIFICATION_BUILD_IDENTITY: INHERITED_BUILD_IDENTITY,
  };
  delete environment.EMUCHEF_DEVICE_QUALIFICATION;
  return spawnSync(
    "cargo",
    [
      "test",
      "--locked",
      "--manifest-path",
      fixture.manifestPath,
      "--features",
      "real-execution",
    ],
    {
      cwd: fixture.fixtureRoot,
      encoding: "utf8",
      env: environment,
    },
  );
}

test("ordinary Cargo builds reject inherited qualification identity metadata", () => {
  const fixture = createCargoFixture();

  try {
    const ordinaryBuild = cargoQualificationTestWithInheritedIdentity(fixture);
    assert.equal(ordinaryBuild.status, 0, commandOutput(ordinaryBuild));
  } finally {
    rmSync(fixture.fixtureRoot, { recursive: true, force: true });
  }
});

test("a cold offline Cargo home reports the missing locked dependency", () => {
  const fixture = createCargoFixture();
  const cargoHome = mkdtempSync(path.join(tmpdir(), "emuchef-cargo-cold-"));

  try {
    const coldBuild = cargoQualificationCheck(fixture, { cargoHome });
    const output = commandOutput(coldBuild);
    assert.notEqual(coldBuild.status, 0, output);
    assert.match(
      output,
      /no matching package named `serde` found|failed to download `serde v[^`]+`/i,
    );
    assert.match(output, /offline|--offline/i);
  } finally {
    rmSync(fixture.fixtureRoot, { recursive: true, force: true });
    rmSync(cargoHome, { recursive: true, force: true });
  }
});

test("opted-in Cargo builds rerun build.rs after tracked material changes", () => {
  const fixture = createCargoFixture();

  try {
    const cleanBuild = cargoQualificationCheck(fixture);
    assert.equal(cleanBuild.status, 0, commandOutput(cleanBuild));

    writeFileSync(
      fixture.materialPath,
      "pub const MATERIAL_VALUE: u8 = 2;\n",
      "utf8",
    );
    const dirtyFiles = execFileSync(
      "git",
      ["status", "--porcelain", "--untracked-files=all"],
      { cwd: fixture.fixtureRoot, encoding: "utf8" },
    );
    assert.match(dirtyFiles, /apps\/emuchef-app\/src-tauri\/src\/material\.rs/);

    const dirtyBuild = cargoQualificationCheck(fixture);
    assert.notEqual(dirtyBuild.status, 0, commandOutput(dirtyBuild));
    assert.match(
      commandOutput(dirtyBuild),
      /device qualification requires a clean source worktree/,
    );
  } finally {
    rmSync(fixture.fixtureRoot, { recursive: true, force: true });
  }
});

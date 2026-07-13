import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  discoverTauriArtifacts,
  parseVerifyArguments,
  verifyMacosRelease,
} from "./verify-macos-release.mjs";

const SHA = "93f816fc1ea59cd034a40432e4e2a269e11eead7";

function discoveryFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-release-verify-"));
  const appDirectory = path.join(root, "app");
  const macos = path.join(appDirectory, "src-tauri", "target", "release", "bundle", "macos");
  const dmg = path.join(appDirectory, "src-tauri", "target", "release", "bundle", "dmg");
  fs.mkdirSync(macos, { recursive: true });
  fs.mkdirSync(dmg, { recursive: true });
  fs.mkdirSync(path.join(macos, "Example.app"));
  fs.writeFileSync(path.join(dmg, "Example.dmg"), "test-only");
  fs.mkdirSync(path.join(appDirectory, "src-tauri"), { recursive: true });
  fs.writeFileSync(
    path.join(appDirectory, "src-tauri", "tauri.conf.json"),
    JSON.stringify({ version: "1.2.3" }),
  );
  return { appDirectory, root };
}

function stageDependencies(stages) {
  const signedResult = {
    app: { signed: true, notarized: true, stapled: true, gatekeeperAccepted: true },
    dmg: { signed: true, notarized: true, stapled: true, gatekeeperAccepted: true },
  };
  return {
    validateEnvironment: () => stages.push("environment"),
    scanRepository: () => stages.push("repository-scan"),
    verifySigned: () => {
      stages.push("signed");
      return signedResult;
    },
    inspectBundle: () => stages.push("bundle"),
    runSmoke: (_run, scriptPath) =>
      stages.push(scriptPath.includes("runtime-network") ? "network" : "application"),
    generateManifest: (_options, dependencies) => {
      stages.push("manifest");
      assert.deepEqual(dependencies.verify(), signedResult);
      return { schemaVersion: 1 };
    },
  };
}

test("parses discovery and explicit modes with release options", () => {
  assert.deepEqual(parseVerifyArguments(["--skip-env"]), {
    mode: "discovery",
    buildCommit: undefined,
    outputPath: undefined,
    skipEnv: true,
  });
  const explicit = parseVerifyArguments([
    "Example.app",
    "Example.dmg",
    "manifest.json",
    "--build-commit",
    SHA.toUpperCase(),
  ]);
  assert.equal(explicit.mode, "explicit");
  assert.equal(explicit.buildCommit, SHA.toUpperCase());
  assert.throws(
    () => parseVerifyArguments(["a.app", "a.dmg", "manifest", "--output", "other"]),
    /cannot be combined/,
  );
  assert.throws(() => parseVerifyArguments(["a.app"]), /usage/);
  assert.throws(() => parseVerifyArguments(["--build-commit", "short"]), /40-character/);
});

test("discovers exactly one app and disk image", (t) => {
  const value = discoveryFixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  const result = discoverTauriArtifacts(value.appDirectory);
  assert.match(result.appPath, /Example\.app$/);
  assert.match(result.dmgPath, /Example\.dmg$/);
  fs.mkdirSync(
    path.join(
      value.appDirectory,
      "src-tauri",
      "target",
      "release",
      "bundle",
      "macos",
      "Second.app",
    ),
  );
  assert.throws(() => discoverTauriArtifacts(value.appDirectory), /found 2/);
});

test("explicit paths override discovery and run stages in order", () => {
  const stages = [];
  const options = parseVerifyArguments(["Example.app", "Example.dmg", "manifest.json"]);
  const result = verifyMacosRelease(options, {
    ...stageDependencies(stages),
    discover: () => {
      throw new Error("discovery must not run");
    },
    repositoryRoot: "/repo",
    scriptDirectory: "/scripts",
  });
  assert.deepEqual(stages, ["environment", "signed", "bundle", "application", "network", "manifest"]);
  assert.equal(result.mode, "explicit");
  assert.equal(result.manifest, "manifest.json");
});

test("discovery mode resolves a default manifest output", (t) => {
  const value = discoveryFixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  const stages = [];
  let generatedOutput;
  const dependencies = stageDependencies(stages);
  dependencies.generateManifest = (options) => {
    stages.push("manifest");
    generatedOutput = options.outputPath;
    return { schemaVersion: 1 };
  };
  const result = verifyMacosRelease(parseVerifyArguments([]), {
    ...dependencies,
    appDirectory: value.appDirectory,
    repositoryRoot: value.root,
    scriptDirectory: "/scripts",
  });
  assert.match(generatedOutput, /release-artifacts\/emuchef-config-editor-1\.2\.3-macos-/);
  assert.equal(result.mode, "discovery");
});

test("skip-env retains the repository credential scan", () => {
  const stages = [];
  const options = parseVerifyArguments([
    "Example.app",
    "Example.dmg",
    "manifest.json",
    "--skip-env",
  ]);
  const result = verifyMacosRelease(options, {
    ...stageDependencies(stages),
    repositoryRoot: "/repo",
    scriptDirectory: "/scripts",
  });
  assert.equal(stages[0], "repository-scan");
  assert.equal(result.checks.environment, "skipped_after_repository_scan");
});

test("stops before the manifest when a verification stage fails", () => {
  const stages = [];
  const dependencies = stageDependencies(stages);
  dependencies.verifySigned = () => {
    stages.push("signed");
    throw new Error("signed artifact verification failed safely");
  };
  assert.throws(
    () =>
      verifyMacosRelease(
        parseVerifyArguments(["Example.app", "Example.dmg", "manifest.json"]),
        { ...dependencies, repositoryRoot: "/repo", scriptDirectory: "/scripts" },
      ),
    /failed safely/,
  );
  assert.deepEqual(stages, ["environment", "signed"]);
});

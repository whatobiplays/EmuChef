import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { prepareRustSidecar } from "./prepare-rust-sidecar.mjs";
import { runQualifiedTauri } from "./run-qualified-tauri.mjs";
import {
  acquireSidecarBuildLock,
  SIDECAR_BUILD_LOCK_TOKEN_ENV,
  sidecarBuildBorrowerDirectoryPath,
  sidecarBuildLockPath,
} from "./sidecar-build-contract.mjs";
import { externalBinArtifactName } from "./sidecar-packaging.mjs";

const TARGET = "aarch64-apple-darwin";

function fixture(t, { pinned = true } = {}) {
  const repoRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "emuchef-release-sidecar-"),
  );
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: repoRoot });

  const appDir = path.join(repoRoot, "apps/emuchef-app");
  const externalBin = path.join(
    appDir,
    "src-tauri/binaries",
    externalBinArtifactName(TARGET),
  );
  const releaseSource = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/release/emuchef",
  );
  const markerPath = path.resolve(
    repoRoot,
    execFileSync("git", ["rev-parse", "--git-path", "emuchef/configuration-create-qualification-state.json"], {
      cwd: repoRoot,
      encoding: "utf8",
    }).trim(),
  );
  fs.mkdirSync(path.dirname(externalBin), { recursive: true });
  fs.writeFileSync(externalBin, "pinned qualification sidecar");
  if (pinned) {
    fs.mkdirSync(path.dirname(markerPath), { recursive: true });
    fs.writeFileSync(markerPath, "pinned qualification state");
  }

  const calls = [];
  const runCommand = (program, args, options) => {
    calls.push({ program, args, options });
    if (program === "rustc") {
      return { status: 0, stdout: `${TARGET}\n` };
    }
    if (program === "cargo") {
      fs.mkdirSync(path.dirname(releaseSource), { recursive: true });
      fs.writeFileSync(releaseSource, "ordinary release sidecar");
      return { status: 0 };
    }
    throw new Error(`unexpected command: ${program}`);
  };

  return {
    appDir,
    calls,
    externalBin,
    markerPath,
    repoRoot,
    sessionPath: path.join(
      path.dirname(markerPath),
      "retroarch-config-create-qualification-session.json",
    ),
    runCommand,
  };
}

test("release hook refuses qualification selection without touching the pin", (t) => {
  const state = fixture(t);
  let verificationCalls = 0;

  assert.throws(
    () =>
      prepareRustSidecar(
        "release",
        { EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1" },
        {
          appDir: state.appDir,
          runCommand: state.runCommand,
          verify: () => {
            verificationCalls += 1;
            return { sidecarPath: state.externalBin };
          },
        },
      ),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );

  assert.equal(verificationCalls, 0);
  assert.deepEqual(
    state.calls.map(({ program }) => program),
    ["rustc"],
  );
  assert.equal(
    fs.readFileSync(state.externalBin, "utf8"),
    "pinned qualification sidecar",
  );
});

test("release packaging hook refuses direct Tauri builds without the wrapper lock", () => {
  const env = { ...process.env };
  delete env.EMUCHEF_SIDECAR_BUILD_LOCK_TOKEN;
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL("./prepare-rust-sidecar.mjs", import.meta.url)),
      "--profile",
      "release",
      "--require-release-package-lock",
    ],
    { env, encoding: "utf8" },
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /release packaging requires the Tauri wrapper lock token/i);
});

test("release hook refuses a pinned qualification state without opt-in", (t) => {
  const state = fixture(t);

  assert.throws(
    () =>
      prepareRustSidecar("release", {}, {
        appDir: state.appDir,
        runCommand: state.runCommand,
      }),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );

  assert.deepEqual(
    state.calls.map(({ program }) => program),
    ["rustc"],
  );
  assert.equal(
    fs.readFileSync(state.externalBin, "utf8"),
    "pinned qualification sidecar",
  );
});

test("release hook treats a dangling qualification marker as pinned state", (t) => {
  const state = fixture(t, { pinned: false });
  fs.mkdirSync(path.dirname(state.markerPath), { recursive: true });
  fs.symlinkSync(
    path.join(path.dirname(state.markerPath), "missing-marker-target"),
    state.markerPath,
  );
  const pinnedBytes = fs.readFileSync(state.externalBin);

  assert.throws(
    () =>
      prepareRustSidecar("release", {}, {
        appDir: state.appDir,
        runCommand: state.runCommand,
      }),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );

  assert.deepEqual(state.calls.map((call) => call.program), ["rustc"]);
  assert.equal(fs.readFileSync(state.externalBin).equals(pinnedBytes), true);
});

test("release hook treats a dangling qualification manifest as pinned state", (t) => {
  const state = fixture(t, { pinned: false });
  const repoRoot = path.resolve(state.appDir, "../..");
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.symlinkSync(path.join(path.dirname(manifestPath), "missing-manifest"), manifestPath);
  const pinnedBytes = fs.readFileSync(state.externalBin);

  assert.throws(
    () =>
      prepareRustSidecar("release", {}, {
        appDir: state.appDir,
        runCommand: state.runCommand,
      }),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );

  assert.deepEqual(state.calls.map((call) => call.program), ["rustc"]);
  assert.equal(fs.readFileSync(state.externalBin).equals(pinnedBytes), true);
});

test("ordinary release hook selects the release binary for Tauri externalBin", (t) => {
  const state = fixture(t, { pinned: false });

  prepareRustSidecar("release", {}, {
    appDir: state.appDir,
    runCommand: state.runCommand,
  });

  const cargo = state.calls.find(({ program }) => program === "cargo");
  assert.ok(cargo);
  assert.deepEqual(cargo.args.slice(-3), [
    "--release",
    "--features",
    "android-copy-commit-helper",
  ]);
  assert.equal(cargo.args.includes("real-execution"), false);
  assert.equal(
    fs.readFileSync(state.externalBin, "utf8"),
    "ordinary release sidecar",
  );

  const packageJson = JSON.parse(
    fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"),
  );
  const tauriConfig = JSON.parse(
    fs.readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  );
  assert.equal(
    packageJson.scripts["sidecar:build"],
    "node scripts/prepare-rust-sidecar.mjs --profile release --require-release-package-lock",
  );
  assert.equal(
    packageJson.scripts["tauri:build"],
    "node scripts/macos-package.mjs build",
  );
  assert.match(tauriConfig.build.beforeBuildCommand, /npm run sidecar:build/);
  assert.deepEqual(tauriConfig.bundle.externalBin, ["binaries/emuchef"]);
  assert.equal(path.basename(state.externalBin), "emuchef-aarch64-apple-darwin");
});

test("release sidecar compiler inherits its exact borrower lease descriptor", (t) => {
  const state = fixture(t, { pinned: false });
  const packageLock = acquireSidecarBuildLock({
    repoRoot: state.repoRoot,
    profile: "release-package",
    artifactPaths: ["apps/emuchef-app/src-tauri/binaries"],
  });
  let cargoDescriptorIdentity;
  const runCommand = (program, args, options) => {
    if (program === "rustc") return { status: 0, stdout: `${TARGET}\n` };
    if (program === "cargo") {
      assert.ok(Array.isArray(options.stdio));
      const descriptorMetadata = fs.fstatSync(options.stdio[3]);
      const borrowerDirectory = sidecarBuildBorrowerDirectoryPath(
        sidecarBuildLockPath(state.repoRoot),
      );
      const [recordName] = fs.readdirSync(borrowerDirectory);
      const recordMetadata = fs.lstatSync(path.join(borrowerDirectory, recordName));
      assert.equal(descriptorMetadata.dev, recordMetadata.dev);
      assert.equal(descriptorMetadata.ino, recordMetadata.ino);
      cargoDescriptorIdentity = { dev: descriptorMetadata.dev, ino: descriptorMetadata.ino };
      const releaseSource = path.join(
        state.repoRoot,
        "crates/emuchef-rust-backend/target/release/emuchef",
      );
      fs.mkdirSync(path.dirname(releaseSource), { recursive: true });
      fs.writeFileSync(releaseSource, "release sidecar");
      return { status: 0 };
    }
    throw new Error(`unexpected command: ${program}`);
  };

  try {
    prepareRustSidecar(
      "release",
      { [SIDECAR_BUILD_LOCK_TOKEN_ENV]: packageLock.token },
      { appDir: state.appDir, runCommand },
    );
  } finally {
    packageLock();
  }
  assert.ok(cargoDescriptorIdentity);
  assert.equal(fs.existsSync(sidecarBuildBorrowerDirectoryPath(sidecarBuildLockPath(state.repoRoot))), true);
  assert.deepEqual(fs.readdirSync(sidecarBuildBorrowerDirectoryPath(sidecarBuildLockPath(state.repoRoot))), []);
});

test("partial sidecar backup is removed after an interrupted copy", (t) => {
  const state = fixture(t, { pinned: false });
  const repoRoot = path.resolve(state.appDir, "../..");
  const source = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/emuchef",
  );
  const originalCopyFileSync = fs.copyFileSync;
  const identity = {
    gitCommit: "a".repeat(40),
    materialBuildDigest: `sha256:${"b".repeat(64)}`,
  };
  const materialInputs = { digest: `sha256:${"c".repeat(64)}` };
  const backendProvenance = {
    schemaVersion: 1,
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: `sha256:${"d".repeat(64)}`,
    gitCommit: identity.gitCommit,
    materialBuildDigest: identity.materialBuildDigest,
    materialInputsDigest: materialInputs.digest,
  };
  const runCommand = (program) => {
    if (program === "rustc") return { status: 0, stdout: `${TARGET}\n` };
    if (program === "git") return { status: 0, stdout: "" };
    if (program === "cargo") {
      fs.mkdirSync(path.dirname(source), { recursive: true });
      fs.writeFileSync(source, "qualification candidate binary");
      return { status: 0 };
    }
    if (program === source) {
      return { status: 0, stdout: JSON.stringify(backendProvenance) };
    }
    throw new Error(`unexpected command: ${program}`);
  };
  let backupFailureInjected = false;

  fs.copyFileSync = (copySource, destination, flags) => {
    if (destination.endsWith(".backup")) {
      fs.writeFileSync(destination, "partial backup");
      backupFailureInjected = true;
      throw Object.assign(new Error("simulated EIO"), { code: "EIO" });
    }
    return originalCopyFileSync.call(fs, copySource, destination, flags);
  };

  try {
    assert.throws(
      () =>
        prepareRustSidecar(
          "qualification",
          {
            EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
            EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID:
              "konkr-pocket-advance-retroarch-config-create",
          },
          {
            appDir: state.appDir,
            runCommand,
            buildIdentity: () => identity,
            readMaterialInputs: () => materialInputs,
          },
        ),
      /simulated EIO/,
    );
  } finally {
    fs.copyFileSync = originalCopyFileSync;
  }

  assert.equal(backupFailureInjected, true);
  assert.equal(fs.readFileSync(state.externalBin, "utf8"), "pinned qualification sidecar");
  assert.deepEqual(
    fs.readdirSync(path.dirname(state.externalBin)).filter((name) => name.endsWith(".backup")),
    [],
  );
});

test("qualification source identity changes during Cargo build reject publication", (t) => {
  const state = fixture(t, { pinned: false });
  const repoRoot = path.resolve(state.appDir, "../..");
  const source = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/emuchef",
  );
  const oldDestination = fs.readFileSync(state.externalBin);
  const sourceIdentityA = {
    gitCommit: "a".repeat(40),
    materialBuildDigest: `sha256:${"b".repeat(64)}`,
  };
  const sourceIdentityB = {
    gitCommit: "c".repeat(40),
    materialBuildDigest: `sha256:${"d".repeat(64)}`,
  };
  const materialInputsA = { digest: `sha256:${"e".repeat(64)}` };
  const materialInputsB = { digest: `sha256:${"f".repeat(64)}` };
  let identityReads = 0;
  let inputReads = 0;
  const calls = [];
  const runCommand = (program, args, options) => {
    calls.push({ program, args, options });
    if (program === "rustc") return { status: 0, stdout: `${TARGET}\n` };
    if (program === "git") return { status: 0, stdout: "" };
    if (program === "cargo") {
      fs.mkdirSync(path.dirname(source), { recursive: true });
      fs.writeFileSync(source, "qualification candidate binary");
      return { status: 0 };
    }
    if (program === source) {
      return {
        status: 0,
        stdout: JSON.stringify({
          schemaVersion: 1,
          backendFeatures: ["real-execution", "android-copy-commit-helper"],
          arm64HelperSha256: `sha256:${"1".repeat(64)}`,
          gitCommit: sourceIdentityA.gitCommit,
          materialBuildDigest: sourceIdentityA.materialBuildDigest,
          materialInputsDigest: materialInputsA.digest,
        }),
      };
    }
    throw new Error(`unexpected command: ${program}`);
  };

  assert.throws(
    () =>
      prepareRustSidecar(
        "qualification",
        {
          EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
          EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: "konkr-pocket-advance-retroarch-config-create",
        },
        {
          appDir: state.appDir,
          runCommand,
          buildIdentity: () =>
            identityReads++ === 0 ? sourceIdentityA : sourceIdentityB,
          readMaterialInputs: () =>
            inputReads++ === 0 ? materialInputsA : materialInputsB,
        },
      ),
    /source identity changed during qualification sidecar build/i,
  );

  const cargo = calls.find((call) => call.program === "cargo");
  assert.ok(cargo);
  assert.equal(
    cargo.options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_GIT_COMMIT,
    sourceIdentityA.gitCommit,
  );
  assert.equal(fs.readFileSync(state.externalBin).equals(oldDestination), true);
  assert.equal(
    fs.existsSync(
      path.join(
        repoRoot,
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      ),
    ),
    false,
  );
});

test("qualification publication rolls back sidecars when state pin creation loses a race", (t) => {
  const state = fixture(t, { pinned: false });
  const repoRoot = path.resolve(state.appDir, "../..");
  const source = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/emuchef",
  );
  const runtimePath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug/emuchef",
  );
  const oldExternal = fs.readFileSync(state.externalBin);
  const oldRuntime = Buffer.from("previous runtime sidecar");
  const foreignMarker = Buffer.from("concurrent qualification state owner");
  fs.mkdirSync(path.dirname(runtimePath), { recursive: true });
  fs.writeFileSync(runtimePath, oldRuntime);

  const identity = {
    gitCommit: "a".repeat(40),
    materialBuildDigest: `sha256:${"b".repeat(64)}`,
  };
  const materialInputs = { digest: `sha256:${"c".repeat(64)}` };
  const backendProvenance = {
    schemaVersion: 1,
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: `sha256:${"d".repeat(64)}`,
    gitCommit: identity.gitCommit,
    materialBuildDigest: identity.materialBuildDigest,
    materialInputsDigest: materialInputs.digest,
  };
  const runCommand = (program, args) => {
    if (program === "rustc") return { status: 0, stdout: `${TARGET}\n` };
    if (program === "git") return { status: 0, stdout: "" };
    if (program === "cargo") {
      fs.mkdirSync(path.dirname(source), { recursive: true });
      fs.writeFileSync(source, "qualification candidate binary");
      fs.writeFileSync(state.markerPath, foreignMarker, { flag: "wx" });
      return { status: 0 };
    }
    if (program === source) {
      return { status: 0, stdout: JSON.stringify(backendProvenance) };
    }
    throw new Error(`unexpected command: ${program}`);
  };

  assert.throws(
    () =>
      prepareRustSidecar(
        "qualification",
        {
          EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
          EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: "konkr-pocket-advance-retroarch-config-create",
        },
        {
          appDir: state.appDir,
          runCommand,
          buildIdentity: () => identity,
          readMaterialInputs: () => materialInputs,
        },
      ),
    /qualification sidecar publication failed:.*EEXIST/i,
  );

  assert.equal(fs.readFileSync(state.externalBin).equals(oldExternal), true);
  assert.equal(fs.readFileSync(runtimePath).equals(oldRuntime), true);
  assert.equal(fs.readFileSync(state.markerPath).equals(foreignMarker), true);
  assert.equal(
    fs.existsSync(
      path.join(
        repoRoot,
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      ),
    ),
    false,
  );
});

test("publication rollback preserves the build lock when staged cleanup fails", (t) => {
  const state = fixture(t, { pinned: false });
  const repoRoot = path.resolve(state.appDir, "../..");
  const source = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/emuchef",
  );
  const runtimePath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug/emuchef",
  );
  const oldExternal = fs.readFileSync(state.externalBin);
  const oldRuntime = Buffer.from("previous runtime sidecar");
  fs.mkdirSync(path.dirname(runtimePath), { recursive: true });
  fs.writeFileSync(runtimePath, oldRuntime);

  const identity = {
    gitCommit: "a".repeat(40),
    materialBuildDigest: `sha256:${"b".repeat(64)}`,
  };
  const materialInputs = { digest: `sha256:${"c".repeat(64)}` };
  const backendProvenance = {
    schemaVersion: 1,
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: `sha256:${"d".repeat(64)}`,
    gitCommit: identity.gitCommit,
    materialBuildDigest: identity.materialBuildDigest,
    materialInputsDigest: materialInputs.digest,
  };
  const originalWriteFileSync = fs.writeFileSync;
  const originalRenameSync = fs.renameSync;
  const originalRmSync = fs.rmSync;
  let markerFailureInjected = false;
  let publicationFailureInjected = false;
  let cleanupFailureInjected = false;
  t.after(() => {
    fs.writeFileSync = originalWriteFileSync;
    fs.renameSync = originalRenameSync;
    fs.rmSync = originalRmSync;
  });
  fs.writeFileSync = function writeFileSyncWithPartialStateFailure(
    destination,
    data,
    options,
  ) {
    const bytes = Buffer.isBuffer(data) ? data : Buffer.from(String(data));
    if (
      !markerFailureInjected &&
      bytes.includes(Buffer.from('"sessionLaunchAttempted"'))
    ) {
      markerFailureInjected = true;
      originalWriteFileSync.call(fs, destination, bytes.subarray(0, 12), options);
      const error = new Error("injected partial state-pin write");
      error.code = "EIO";
      throw error;
    }
    return originalWriteFileSync.call(fs, destination, data, options);
  };

  const runCommand = (program) => {
    if (program === "rustc") {
      return { status: 0, stdout: TARGET + String.fromCharCode(10) };
    }
    if (program === "git") return { status: 0, stdout: "" };
    if (program === "cargo") {
      fs.mkdirSync(path.dirname(source), { recursive: true });
      fs.writeFileSync(source, "qualification candidate binary");
      return { status: 0 };
    }
    if (program === source) {
      return { status: 0, stdout: JSON.stringify(backendProvenance) };
    }
    throw new Error(`unexpected command: ${program}`);
  };
  const prepare = () =>
    prepareRustSidecar(
      "qualification",
      {
        EMUCHEF_CONFIG_CREATE_QUALIFICATION: "1",
        EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: "konkr-pocket-advance-retroarch-config-create",
      },
      {
        appDir: state.appDir,
        runCommand,
        buildIdentity: () => identity,
        readMaterialInputs: () => materialInputs,
      },
    );

  assert.throws(
    prepare,
    /qualification sidecar publication failed:.*partial state-pin write/i,
  );
  fs.writeFileSync = originalWriteFileSync;
  assert.equal(markerFailureInjected, true);
  assert.equal(fs.readFileSync(state.externalBin).equals(oldExternal), true);
  assert.equal(fs.readFileSync(runtimePath).equals(oldRuntime), true);
  assert.equal(fs.existsSync(state.markerPath), false);
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  assert.equal(fs.existsSync(manifestPath), false);
  assert.equal(fs.existsSync(sidecarBuildLockPath(repoRoot)), false);

  fs.renameSync = function renameSyncWithRuntimePublishFailure(sourcePath, targetPath, ...args) {
    if (
      !publicationFailureInjected &&
      String(sourcePath).startsWith(`${runtimePath}.tmp-`) &&
      targetPath === runtimePath
    ) {
      publicationFailureInjected = true;
      const error = new Error("injected runtime sidecar publish failure");
      error.code = "EIO";
      throw error;
    }
    return originalRenameSync.call(fs, sourcePath, targetPath, ...args);
  };
  fs.rmSync = function rmSyncWithRuntimeStageCleanupFailure(target, ...args) {
    if (
      publicationFailureInjected &&
      !cleanupFailureInjected &&
      String(target).startsWith(`${runtimePath}.tmp-`) &&
      !String(target).endsWith(".backup")
    ) {
      cleanupFailureInjected = true;
      const error = new Error("injected staged cleanup failure");
      error.code = "EIO";
      throw error;
    }
    return originalRmSync.call(fs, target, ...args);
  };
  try {
    assert.throws(prepare, /rollback staging cleanup failed.*preserve build lock/i);
  } finally {
    fs.renameSync = originalRenameSync;
    fs.rmSync = originalRmSync;
  }

  assert.equal(publicationFailureInjected, true);
  assert.equal(cleanupFailureInjected, true);
  assert.equal(fs.readFileSync(state.externalBin).equals(oldExternal), true);
  assert.equal(fs.readFileSync(runtimePath).equals(oldRuntime), true);
  assert.equal(fs.existsSync(state.markerPath), false);
  assert.equal(fs.existsSync(manifestPath), false);
  assert.equal(fs.existsSync(sidecarBuildLockPath(repoRoot)), true);
  assert.equal(
    fs.readdirSync(path.dirname(runtimePath)).some((entry) =>
      entry.startsWith(`${path.basename(runtimePath)}.tmp-`),
    ),
    true,
  );
});

test("release hook refuses an outstanding session lease without marker or manifest", (t) => {
  const state = fixture(t, { pinned: false });
  fs.mkdirSync(path.dirname(state.sessionPath), { recursive: true });
  const leaseBytes = Buffer.from(
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: "konkr-pocket-advance-retroarch-config-create",
      token: "unfinished-session",
      status: "verified",
    }),
  );
  fs.writeFileSync(state.sessionPath, leaseBytes);
  const pinnedBytes = fs.readFileSync(state.externalBin);

  assert.throws(
    () =>
      prepareRustSidecar("release", {}, {
        appDir: state.appDir,
        runCommand: state.runCommand,
      }),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );

  assert.deepEqual(state.calls.map((call) => call.program), ["rustc"]);
  assert.equal(fs.readFileSync(state.sessionPath).equals(leaseBytes), true);
  assert.equal(fs.readFileSync(state.externalBin).equals(pinnedBytes), true);
});

test("release hook treats a dangling session lease link as pinned state", (t) => {
  const state = fixture(t, { pinned: false });
  fs.mkdirSync(path.dirname(state.sessionPath), { recursive: true });
  fs.symlinkSync(path.join(path.dirname(state.sessionPath), "missing-lease-target"), state.sessionPath);

  assert.throws(
    () =>
      prepareRustSidecar("release", {}, {
        appDir: state.appDir,
        runCommand: state.runCommand,
      }),
    /release sidecar packaging cannot select a configuration-create qualification artifact/i,
  );
  assert.deepEqual(state.calls.map((call) => call.program), ["rustc"]);
});

test("prepared pin is verified by launch and reused by Tauri's dev sidecar hook", (t) => {
  const state = fixture(t);
  const repoRoot = path.resolve(state.appDir, "../..");
  const runtimePath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/debug/emuchef",
  );
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  const sidecarBytes = fs.readFileSync(state.externalBin);
  const sidecarSha256 =
    "sha256:" + createHash("sha256").update(sidecarBytes).digest("hex");
  fs.mkdirSync(path.dirname(runtimePath), { recursive: true });
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.writeFileSync(runtimePath, sidecarBytes);
  const manifest = {
    schemaVersion: 1,
    qualificationTargetId: "konkr-pocket-advance-retroarch-config-create",
    gitCommit: "a".repeat(40),
    materialBuildDigest: "sha256:" + "b".repeat(64),
    materialInputsDigest: "sha256:" + "c".repeat(64),
    sourceWorktreeClean: true,
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: "sha256:" + "d".repeat(64),
    sidecarSha256,
    runtimeSidecarSha256: sidecarSha256,
    sidecarPath: path
      .relative(repoRoot, state.externalBin)
      .split(path.sep)
      .join("/"),
    runtimeSidecarPath: path
      .relative(repoRoot, runtimePath)
      .split(path.sep)
      .join("/"),
  };
  fs.writeFileSync(manifestPath, JSON.stringify(manifest));
  fs.writeFileSync(
    state.markerPath,
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: manifest.qualificationTargetId,
      manifestPath:
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      sessionLaunchAttempted: false,
    }),
  );
  const runtimeBytes = fs.readFileSync(runtimePath);
  let verificationCalls = 0;
  const verify = () => {
    verificationCalls += 1;
    return { manifest, sidecarPath: state.externalBin };
  };

  const status = runQualifiedTauri("dev", {
    repoRoot,
    cwd: state.appDir,
    env: { EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: manifest.qualificationTargetId },
    verify,
    spawn(_executable, args, options) {
      if (args[0] === "scripts/ensure-retroarch-config-create-qualification.mjs") {
        return { status: 0 };
      }

      assert.equal(args[0], "dev");
      assert.equal(args.at(-1), "real-execution");
      const leasePath = options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE;
      const lease = JSON.parse(fs.readFileSync(leasePath, "utf8"));
      assert.equal(lease.status, "verified");

      prepareRustSidecar("debug", options.env, {
        appDir: state.appDir,
        runCommand: state.runCommand,
        verify,
      });
      assert.equal(
        state.calls.some((call) => call.program === "cargo"),
        false,
      );
      assert.equal(fs.readFileSync(state.externalBin).equals(sidecarBytes), true);
      assert.equal(fs.readFileSync(runtimePath).equals(runtimeBytes), true);

      const exitedPid = spawnSync(process.execPath, ["-e", ""], {
        encoding: "utf8",
      }).pid;
      lease.status = "ended-cleanly";
      lease.appProcess = { pid: exitedPid, host: os.hostname() };
      fs.writeFileSync(leasePath, JSON.stringify(lease));
      return { status: 0 };
    },
  });

  assert.equal(status, 0);
  assert.equal(verificationCalls, 2);
  assert.equal(fs.readFileSync(state.externalBin).equals(sidecarBytes), true);
  assert.equal(fs.readFileSync(runtimePath).equals(runtimeBytes), true);
});

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { sidecarBuildDisposition } from "./sidecar-build-contract.mjs";
import { ensureConfigCreateQualification } from "./ensure-retroarch-config-create-qualification.mjs";
import { prepareConfigCreateQualification } from "./prepare-retroarch-config-create-qualification.mjs";
import { APP_ROOT } from "./run-device-qualification.mjs";
import { runQualifiedTauri } from "./run-qualified-tauri.mjs";

const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";
const MANIFEST_RELATIVE_PATH =
  "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json";

function createLaunchFixture(t) {
  const repoRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "emuchef-configuration-create-launch-reuse-"),
  );
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: repoRoot });

  const gitStatePath = execFileSync(
    "git",
    ["rev-parse", "--git-path", "emuchef/configuration-create-qualification-state.json"],
    { cwd: repoRoot, encoding: "utf8" },
  ).trim();
  const markerPath = path.resolve(repoRoot, gitStatePath);
  fs.mkdirSync(path.dirname(markerPath), { recursive: true });
  fs.writeFileSync(
    markerPath,
    `${JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: TARGET_ID,
      manifestPath: MANIFEST_RELATIVE_PATH,
      sessionLaunchAttempted: false,
    })}\n`,
  );

  const appDir = path.join(repoRoot, "apps/emuchef-app");
  fs.mkdirSync(path.join(appDir, "src-tauri"), { recursive: true });
  fs.copyFileSync(
    path.join(APP_ROOT, "src-tauri/tauri.dev.conf.json"),
    path.join(appDir, "src-tauri/tauri.dev.conf.json"),
  );

  return { appDir, markerPath, repoRoot };
}

test("prepare then Tauri launch verifies and reuses the pinned sidecar", (t) => {
  const fixture = createLaunchFixture(t);
  const env = { PATH: "/qualification/bin" };
  const preparationCalls = [];
  let sidecarBuilds = 0;
  let verificationCalls = 0;
  const pinnedBytes = Buffer.from("immutable qualification sidecar bytes\n");
  const pinnedDigest =
    "sha256:" + createHash("sha256").update(pinnedBytes).digest("hex");
  const manifest = {
    schemaVersion: 1,
    qualificationTargetId: TARGET_ID,
    sourceWorktreeClean: true,
    gitCommit: "a".repeat(40),
    materialBuildDigest: "sha256:" + "c".repeat(64),
    materialInputsDigest: "sha256:" + "d".repeat(64),
    backendFeatures: ["real-execution", "android-copy-commit-helper"],
    arm64HelperSha256: "sha256:" + "b".repeat(64),
    sidecarSha256: pinnedDigest,
    sidecarPath:
      "apps/emuchef-app/src-tauri/binaries/emuchef-aarch64-apple-darwin",
    runtimeSidecarSha256: pinnedDigest,
    runtimeSidecarPath: "crates/emuchef-rust-backend/target/debug/emuchef",
  };
  const manifestPath = path.join(fixture.repoRoot, MANIFEST_RELATIVE_PATH);
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest)}\n`);

  const verifyPinned = ({ env: verificationEnv }) => {
    verificationCalls += 1;
    assert.equal(verificationEnv.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID, TARGET_ID);
    assert.equal(verificationEnv.EMUCHEF_CONFIG_CREATE_QUALIFICATION, "1");
    assert.match(manifest.gitCommit, /^[0-9a-f]{40}$/);
    assert.match(manifest.materialBuildDigest, /^sha256:[0-9a-f]{64}$/);
    assert.match(manifest.materialInputsDigest, /^sha256:[0-9a-f]{64}$/);
    assert.deepEqual(manifest.backendFeatures, [
      "real-execution",
      "android-copy-commit-helper",
    ]);
    assert.match(manifest.arm64HelperSha256, /^sha256:[0-9a-f]{64}$/);
    assert.equal(manifest.sidecarSha256, manifest.runtimeSidecarSha256);
    return { manifest, sidecarPath: "/host-fixture/pinned-sidecar" };
  };

  assert.equal(
    prepareConfigCreateQualification({
      cwd: fixture.appDir,
      targetId: TARGET_ID,
      env,
      spawnSync(command, args, options) {
        preparationCalls.push({ command, args });
        if (command === "npm" && args[1] === "sidecar:qualification") {
          sidecarBuilds += 1;
        }
        if (
          command === "node" &&
          args[0] === "scripts/verify-qualification-sidecar.mjs"
        ) {
          verifyPinned({ env: options.env });
        }
        return { status: 0 };
      },
      exit(status) {
        throw new Error(`unexpected exit ${status}`);
      },
    }),
    true,
  );
  assert.equal(sidecarBuilds, 1);
  assert.equal(verificationCalls, 1);

  const launchCalls = [];
  const status = runQualifiedTauri("dev", {
    cwd: fixture.appDir,
    repoRoot: fixture.repoRoot,
    env: { ...env, EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID },
    verify: verifyPinned,
    spawn(executable, args, options) {
      launchCalls.push({ executable, args, options });
      if (args[0] === "scripts/ensure-retroarch-config-create-qualification.mjs") {
        ensureConfigCreateQualification({
          targetId: args[2],
          env: options.env,
          verify: verifyPinned,
        });
        return { status: 0 };
      }

      assert.equal(executable, "tauri");
      assert.deepEqual(args, [
        "dev",
        "--config",
        "src-tauri/tauri.dev.conf.json",
        "--features",
        "real-execution",
      ]);
      const config = JSON.parse(
        fs.readFileSync(
          path.join(fixture.appDir, "src-tauri/tauri.dev.conf.json"),
          "utf8",
        ),
      );
      assert.match(config.build.beforeDevCommand, /sidecar:dev/);
      assert.equal(
        sidecarBuildDisposition("debug", options.env, true),
        "verify-existing",
        "Tauri's development hook must verify the pin rather than rebuild it",
      );
      verifyPinned({ env: options.env });

      const appExit = spawnSync(process.execPath, ["-e", "process.exit(0)"], {
        encoding: "utf8",
      });
      const sessionPath = options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE;
      const session = JSON.parse(fs.readFileSync(sessionPath, "utf8"));
      session.status = "active";
      session.appProcess = { pid: appExit.pid, host: os.hostname() };
      session.status = "ended-cleanly";
      fs.writeFileSync(sessionPath, `${JSON.stringify(session)}\n`);
      return { status: 0 };
    },
  });

  assert.equal(status, 0);
  assert.equal(launchCalls.length, 2);
  assert.equal(launchCalls[0].args[2], TARGET_ID);
  assert.equal(launchCalls[1].executable, "tauri");
  assert.equal(sidecarBuilds, 1);
  assert.equal(verificationCalls, 4);
  assert.equal(pinnedBytes.toString(), "immutable qualification sidecar bytes\n");
  assert.equal(
    "sha256:" + createHash("sha256").update(pinnedBytes).digest("hex"),
    manifest.sidecarSha256,
  );
  assert.equal(
    preparationCalls.filter(
      ({ command, args }) =>
        command === "npm" && args[1] === "sidecar:qualification",
    ).length,
    1,
  );
  const session = JSON.parse(
    fs.readFileSync(launchCalls[0].options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE, "utf8"),
  );
  assert.equal(session.status, "ended-cleanly");
});

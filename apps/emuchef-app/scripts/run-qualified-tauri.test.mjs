import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";

import {
  qualificationTauriPlan,
  runQualifiedTauri,
} from "./run-qualified-tauri.mjs";
import { archiveAndReleaseQualificationState } from "./qualification-state.mjs";
import {
  qualificationStateMarkerPath,
  sidecarBuildLockPath,
} from "./sidecar-build-contract.mjs";
import { qualificationSessionLeasePath } from "./qualification-session-lifecycle.mjs";

const TARGET_ID = "konkr-pocket-advance-retroarch-config-create";
const BACKEND_FEATURES = ["real-execution", "android-copy-commit-helper"];

function launchFixture(t) {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-configuration-create-launch-"));
  execFileSync("git", ["init", "-q"], { cwd: repoRoot });
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  const markerPath = qualificationStateMarkerPath(repoRoot);
  const sessionPath = qualificationSessionLeasePath(repoRoot, markerPath);
  const lockPath = sidecarBuildLockPath(repoRoot);
  const manifestPath = path.join(
    repoRoot,
    "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
  );
  fs.mkdirSync(path.dirname(markerPath), { recursive: true });
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.mkdirSync(path.join(repoRoot, "apps/emuchef-app"), { recursive: true });
  fs.writeFileSync(
    markerPath,
    JSON.stringify({
      schemaVersion: 1,
      qualificationTargetId: TARGET_ID,
      manifestPath:
        "crates/emuchef-rust-backend/target/qualification/retroarch-config-create-sidecar-build.json",
      sessionLaunchAttempted: false,
    }),
  );
  const manifest = {
    schemaVersion: 1,
    qualificationTargetId: TARGET_ID,
    gitCommit: "a".repeat(40),
    materialBuildDigest: `sha256:${"b".repeat(64)}`,
    materialInputsDigest: `sha256:${"c".repeat(64)}`,
    sourceWorktreeClean: true,
    backendFeatures: BACKEND_FEATURES,
    arm64HelperSha256: `sha256:${"d".repeat(64)}`,
    sidecarSha256: `sha256:${"e".repeat(64)}`,
    runtimeSidecarSha256: `sha256:${"e".repeat(64)}`,
  };
  fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  const evidenceDirectory = path.join(
    path.dirname(repoRoot),
    `${path.basename(repoRoot)}-release-evidence`,
  );
  t.after(() => fs.rmSync(evidenceDirectory, { recursive: true, force: true }));
  return {
    repoRoot,
    cwd: path.join(repoRoot, "apps/emuchef-app"),
    markerPath,
    sessionPath,
    manifest,
    evidenceDirectory,
    env: { EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID },
  };
}

test("configuration-create development launch enables real execution and inherits explicit identity", () => {
  const plan = qualificationTauriPlan("dev", {
    PATH: "/qualification/bin",
    EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID,
  });

  assert.equal(plan.length, 2);
  assert.deepEqual(plan[0].args, [
    "scripts/ensure-retroarch-config-create-qualification.mjs",
    "--target-id",
    TARGET_ID,
  ]);
  assert.deepEqual(plan[1].args, [
    "dev",
    "--config",
    "src-tauri/tauri.dev.conf.json",
    "--features",
    "real-execution",
  ]);
  for (const step of plan) {
    assert.equal(step.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION, "1");
    assert.equal(step.env.EMUCHEF_DEVICE_QUALIFICATION, "1");
    assert.equal(step.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID, TARGET_ID);
    assert.equal(step.env.PATH, "/qualification/bin");
  }
});

test("configuration-create runner rejects missing, ambiguous, or unsupported selections", () => {
  assert.throws(() => qualificationTauriPlan("dev", {}), /target/i);
  assert.throws(
    () => qualificationTauriPlan("preview", { EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID }),
    /development-only/,
  );
  assert.throws(
    () => qualificationTauriPlan("build", { EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: TARGET_ID }),
    /development-only/,
  );
});

test("prepare then launch reuses the pin and carries its lease into Tauri", (t) => {
  const state = launchFixture(t);
  const calls = [];
  const result = runQualifiedTauri("dev", {
    ...state,
    verify: () => ({ manifest: state.manifest }),
    spawn(executable, args, options) {
      calls.push({ executable, args, options });
      if (calls.length === 1) {
        assert.equal(
          JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
          "launching",
        );
        return { status: 0 };
      }

      assert.equal(
        JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
        "verified",
      );
      assert.deepEqual(args, [
        "dev",
        "--config",
        "src-tauri/tauri.dev.conf.json",
        "--features",
        "real-execution",
      ]);
      assert.equal(options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_TOKEN.length > 0, true);
      assert.equal(options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE, state.sessionPath);
      assert.equal(options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_LOCK_FILE.length > 0, true);
      assert.throws(
        () =>
          archiveAndReleaseQualificationState({
            repoRoot: state.repoRoot,
            targetId: TARGET_ID,
            confirmEndedTarget: TARGET_ID,
            evidenceDirectory: state.evidenceDirectory,
            disposition: "finished",
          }),
        /launch is still owned by a live process/i,
        "release must remain blocked after verification and before app startup",
      );
      assert.equal(fs.existsSync(state.markerPath), true);
      assert.equal(fs.existsSync(state.evidenceDirectory), false);
      const exitedPid = spawnSync(process.execPath, ["-e", ""], {
        encoding: "utf8",
      }).pid;
      const session = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
      session.status = "ended-cleanly";
      session.appProcess = { pid: exitedPid, host: os.hostname() };
      fs.writeFileSync(state.sessionPath, JSON.stringify(session));
      return { status: 0 };
    },
  });

  assert.equal(result, 0);
  assert.equal(calls.length, 2);
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "ended-cleanly",
  );
});

test("release stays blocked between process verification and Tauri startup", async (t) => {
  const state = launchFixture(t);
  const runnerUrl = new URL("./run-qualified-tauri.mjs", import.meta.url);
  const childSource = [
    'import fs from "node:fs";',
    'import { spawnSync } from "node:child_process";',
    `const { runQualifiedTauri } = await import(${JSON.stringify(runnerUrl.href)});`,
    "const status = runQualifiedTauri('dev', {",
    "  cwd: process.env.EMUCHEF_TEST_CWD,",
    "  repoRoot: process.env.EMUCHEF_TEST_REPO_ROOT,",
    "  env: { ...process.env, EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID: process.env.EMUCHEF_TEST_TARGET_ID },",
    "  verify: () => ({ manifest: JSON.parse(process.env.EMUCHEF_TEST_MANIFEST) }),",
    "  spawn(executable, args, options) {",
    "    if (executable === process.execPath) return { status: 0 };",
    '    process.stdout.write("tauri-start-held\\n");',
    "    const byte = Buffer.alloc(1);",
    "    let line = '';",
    "    while (!line.endsWith('\\n')) {",
    "      const count = fs.readSync(0, byte, 0, 1, null);",
    "      if (count === 0) throw new Error('Tauri startup barrier input closed');",
    "      line += byte.toString('utf8', 0, count);",
    "    }",
    "    const sessionPath = options.env.EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE;",
    "    const record = JSON.parse(fs.readFileSync(sessionPath, 'utf8'));",
    "    const appPid = spawnSync(process.execPath, ['-e', ''], { encoding: 'utf8' }).pid;",
    "    record.status = 'active';",
    "    record.appProcess = { pid: appPid, host: process.env.EMUCHEF_TEST_HOST };",
    "    fs.writeFileSync(sessionPath, JSON.stringify(record));",
    "    record.status = 'ended-cleanly';",
    "    fs.writeFileSync(sessionPath, JSON.stringify(record));",
    "    return { status: 0 };",
    "  },",
    "});",
    "if (status !== 0) throw new Error(`qualification launch returned ${status}`);",
    'process.stdout.write("launch-complete\\n");',
  ].join("\n");
  const child = spawn(process.execPath, ["--input-type=module", "--eval", childSource], {
    env: {
      ...process.env,
      EMUCHEF_TEST_CWD: state.cwd,
      EMUCHEF_TEST_REPO_ROOT: state.repoRoot,
      EMUCHEF_TEST_TARGET_ID: TARGET_ID,
      EMUCHEF_TEST_MANIFEST: JSON.stringify(state.manifest),
      EMUCHEF_TEST_HOST: os.hostname(),
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = createInterface({ input: child.stdout, crlfDelay: Infinity });
  const lines = [];
  let errors = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    errors += chunk;
  });
  output.on("line", (line) => lines.push(line));
  const exited = new Promise((resolve) => {
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  t.after(() => {
    output.close();
    if (child.exitCode === null) child.kill("SIGKILL");
  });
  await new Promise((resolve, reject) => {
    const onLine = (line) => {
      if (line === "tauri-start-held") {
        output.removeListener("line", onLine);
        resolve();
      }
    };
    const onError = (error) => reject(error);
    const onExit = (code, signal) => {
      reject(new Error(`launcher exited before Tauri startup: ${code}/${signal} ${errors}`));
    };
    output.on("line", onLine);
    child.once("error", onError);
    child.once("exit", onExit);
  });

  const sessionBeforeRelease = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  assert.equal(sessionBeforeRelease.status, "verified");
  assert.equal(sessionBeforeRelease.launchOwner.pid, child.pid);
  assert.throws(
    () =>
      archiveAndReleaseQualificationState({
        repoRoot: state.repoRoot,
        targetId: TARGET_ID,
        confirmEndedTarget: TARGET_ID,
        evidenceDirectory: state.evidenceDirectory,
        disposition: "finished",
      }),
    /launch is still owned by a live process/i,
  );
  assert.equal(fs.existsSync(state.markerPath), true);
  assert.equal(fs.existsSync(state.evidenceDirectory), false);

  child.stdin.write("continue\n");
  const result = await exited;
  assert.equal(result.code, 0, `launcher failed: ${result.signal} ${errors}`);
  assert.equal(lines.includes("launch-complete"), true);
  const completedSession = JSON.parse(fs.readFileSync(state.sessionPath, "utf8"));
  assert.equal(completedSession.status, "ended-cleanly");
  assert.equal(completedSession.launchOwner.pid, child.pid);
});

test("verification failure cancels only the pre-start launch lease", (t) => {
  const state = launchFixture(t);
  let calls = 0;

  assert.throws(
    () =>
      runQualifiedTauri("dev", {
        ...state,
        verify: () => {
          throw new Error("pinned artifact verification failed");
        },
        spawn() {
          calls += 1;
          return { status: 0 };
        },
      }),
    /pinned artifact verification failed/,
  );
  assert.equal(calls, 1);
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "cancelled-before-start",
  );
});

test("Tauri success without native lease closure remains uncertain", (t) => {
  const state = launchFixture(t);

  assert.throws(
    () =>
      runQualifiedTauri("dev", {
        ...state,
        verify: () => ({ manifest: state.manifest }),
        spawn() {
          return { status: 0 };
        },
      }),
    /did not record a matching clean native exit/i,
  );
  assert.equal(fs.existsSync(state.sessionPath), true);
  assert.equal(
    JSON.parse(fs.readFileSync(state.sessionPath, "utf8")).status,
    "verified",
  );
});

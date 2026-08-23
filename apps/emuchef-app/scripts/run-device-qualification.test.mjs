import assert from "node:assert/strict";
import test from "node:test";

import { APP_ROOT, runQualification } from "./run-device-qualification.mjs";

test("qualification launcher runs the clean build, sidecar, and pinned real app in order", () => {
  const calls = [];
  const environment = { PATH: "/test/bin" };

  runQualification({
    env: environment,
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: 0 };
    },
    exit(status) {
      throw new Error(`unexpected exit ${status}`);
    },
  });

  assert.deepEqual(
    calls.map(({ command, args }) => [command, args]),
    [
      ["npm", ["run", "build"]],
      ["npm", ["run", "sidecar:dev"]],
      ["cargo", ["run", "--manifest-path", "src-tauri/Cargo.toml", "--features", "real-execution"]],
    ],
  );
  assert.equal(calls[0].options.cwd, APP_ROOT);
  assert.deepEqual(calls[0].options.env, environment);
  assert.deepEqual(calls[2].options.env, {
    ...environment,
    EMUCHEF_DEVICE_QUALIFICATION: "1",
  });
});

test("qualification launcher stops at the first failed step", () => {
  const calls = [];
  let exitStatus = null;

  runQualification({
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: 17 };
    },
    exit(status) {
      exitStatus = status;
    },
  });

  assert.equal(exitStatus, 17);
  assert.equal(calls.length, 1);
});

test("qualification launcher treats a process-start error as a failed step", () => {
  const calls = [];
  let exitStatus = null;

  runQualification({
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: null, error: new Error("spawn failed") };
    },
    exit(status) {
      exitStatus = status;
    },
  });

  assert.equal(exitStatus, 1);
  assert.equal(calls.length, 1);
});

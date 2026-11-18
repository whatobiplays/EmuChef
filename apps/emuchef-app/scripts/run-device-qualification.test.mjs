import assert from "node:assert/strict";
import test from "node:test";
import * as launcher from "./run-device-qualification.mjs";

const { APP_ROOT, runQualification, runQualificationCli } = launcher;

function captureRun(run, options = {}) {
  const calls = [];
  let exitStatus = null;
  const result = run({
    ...options,
    spawnSync(command, args, spawnOptions) {
      calls.push({ command, args, options: spawnOptions });
      return { status: 0 };
    },
    exit(status) {
      exitStatus = status;
    },
  });

  return { calls, exitStatus, result };
}

const inheritedEnvironment = {
  PATH: "/test/bin",
  EMUCHEF_DEVICE_QUALIFICATION: "inherited",
  EMUCHEF_RESTRICTED_EXECUTION_BUILD: "inherited",
};

test("qualification launcher keeps the ordinary workflow by default", () => {
  const preparationEnvironment = { PATH: "/test/bin" };
  const { calls, exitStatus, result } = captureRun(
    (options) => runQualification(options),
    { env: inheritedEnvironment },
  );

  assert.equal(result, true);
  assert.equal(exitStatus, null);
  assert.deepEqual(
    calls.map(({ command, args }) => [command, args]),
    [
      ["npm", ["run", "build"]],
      ["npm", ["run", "sidecar:dev"]],
      [
        "cargo",
        ["run", "--manifest-path", "src-tauri/Cargo.toml", "--features", "real-execution"],
      ],
    ],
  );
  assert.equal(calls[0].options.cwd, APP_ROOT);
  assert.deepEqual(calls[0].options.env, preparationEnvironment);
  assert.deepEqual(calls[1].options.env, preparationEnvironment);
  assert.deepEqual(calls[2].options.env, {
    ...preparationEnvironment,
    EMUCHEF_DEVICE_QUALIFICATION: "1",
  });
});

test("programmatic restriction opt-in prepares and launches the restricted app", () => {
  const preparationEnvironment = { PATH: "/test/bin" };
  const { calls } = captureRun(
    (options) => runQualification({ ...options, restrictPrivilegedExecution: true }),
    { env: inheritedEnvironment },
  );

  assert.deepEqual(calls[0].options.env, preparationEnvironment);
  assert.deepEqual(calls[1].options.env, {
    ...preparationEnvironment,
    EMUCHEF_RESTRICTED_EXECUTION_BUILD: "1",
  });
  assert.deepEqual(calls[2].args, [
    "run",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--features",
    "real-execution",
    "--",
    "--restrict-privileged-execution",
  ]);
  assert.deepEqual(calls[2].options.env, {
    ...preparationEnvironment,
    EMUCHEF_DEVICE_QUALIFICATION: "1",
  });
});

test("CLI restriction flag selects the restricted workflow", () => {
  const { calls } = captureRun(
    (options) => runQualificationCli(["--restrict-privileged-execution"], options),
    { env: inheritedEnvironment },
  );

  assert.equal(calls[1].options.env.EMUCHEF_RESTRICTED_EXECUTION_BUILD, "1");
  assert.equal(calls[2].args.at(-1), "--restrict-privileged-execution");
});

test("CLI rejects unknown options deterministically before spawning", () => {
  const calls = [];
  let exitStatus = null;

  assert.throws(
    () => runQualificationCli(["--unknown"], {
      env: inheritedEnvironment,
      spawnSync(command, args, options) {
        calls.push({ command, args, options });
        return { status: 0 };
      },
      exit(status) {
        exitStatus = status;
      },
    }),
    {
      message:
        'Unknown option "--unknown". Supported option: --restrict-privileged-execution',
    },
  );
  assert.deepEqual(calls, []);
  assert.equal(exitStatus, null);
});

test("programmatic restriction input must be a boolean", () => {
  assert.throws(
    () => runQualification({ restrictPrivilegedExecution: "true" }),
    { message: "restrictPrivilegedExecution must be a boolean" },
  );
});

test("qualification launcher stops after the first failed step", () => {
  const calls = [];
  let exitStatus = null;
  const result = runQualification({
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: 17 };
    },
    exit(status) {
      exitStatus = status;
    },
  });

  assert.equal(result, false);
  assert.equal(exitStatus, 17);
  assert.equal(calls.length, 1);
});

test("qualification launcher treats process-start errors as failed steps", () => {
  const calls = [];
  let exitStatus = null;
  const result = runQualification({
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      return { status: null, error: new Error("spawn failed") };
    },
    exit(status) {
      exitStatus = status;
    },
  });

  assert.equal(result, false);
  assert.equal(exitStatus, 1);
  assert.equal(calls.length, 1);
});

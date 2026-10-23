import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { spawnSync } from "node:child_process";
import {
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import * as runner from "./qualify-armsx1-emulator.mjs";

const REPOSITORY_ROOT = fileURLToPath(new URL("..", import.meta.url));
const RELATIVE_RUNNER_PATH = "tools/qualify-armsx1-emulator.mjs";
const EXPECTED_AVD = "EmuChef-ARMSX1-Android16-ARM64";
const APPROVED_EVIDENCE_DIRECTORY = join(
  REPOSITORY_ROOT,
  "docs/testing/recipe-qualification/armsx1",
);

async function waitFor(predicate, description) {
  const deadline = Date.now() + 1_000;
  while (!predicate()) {
    if (Date.now() >= deadline)
      assert.fail(`Timed out waiting for ${description}`);
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
}

function validProfile() {
  return {
    state: "device",
    bootCompleted: "1",
    apiLevel: "36",
    androidRelease: "16",
    abi: "arm64-v8a",
    brand: "google",
    manufacturer: "Google",
    model: "sdk_gphone64_arm64",
    qemu: "1",
    uid: "2000",
  };
}

test("rejects an invocation without explicit live opt-in before probing anything", () => {
  const calls = [];
  const result = runner.validateInvocation(
    {},
    { record: (event) => calls.push(event) },
  );

  assert.equal(result.ok, false);
  assert.equal(result.status, "blocked");
  assert.deepEqual(calls, [
    {
      status: "blocked",
      liveStatus: "unverified",
      reason: "explicit_opt_in_missing",
    },
  ]);
});

test("requires explicit target, dedicated AVD attestation and package allowlist", () => {
  const result = runner.validateInvocation({
    optIn: true,
    serial: "emulator-5554",
    avdName: EXPECTED_AVD,
    dedicatedAttestation: true,
    packageAllowlist: ["com.nanodata.armsx"],
  });

  assert.equal(result.ok, true);
  assert.equal(result.target.serial, "emulator-5554");
});

test("live invocation retains general syntactically valid emulator serial selection", () => {
  const result = runner.validateInvocation({
    optIn: true,
    serial: "emulator-5556",
    avdName: EXPECTED_AVD,
    dedicatedAttestation: true,
    packageAllowlist: ["com.nanodata.armsx"],
  });

  assert.equal(result.ok, true);
  assert.equal(result.target.serial, "emulator-5556");
});

test("rejects prefix-only emulator identity and non-exact package allowlists", () => {
  for (const input of [
    {
      optIn: true,
      serial: "emulator-5554",
      avdName: EXPECTED_AVD,
      dedicatedAttestation: true,
      packageAllowlist: [],
    },
    {
      optIn: true,
      serial: "emulator-5554",
      avdName: EXPECTED_AVD,
      dedicatedAttestation: true,
      packageAllowlist: ["com.nanodata.armsx", "com.other.app"],
    },
  ]) {
    assert.equal(runner.validateInvocation(input).ok, false);
  }
});

test("requires all independent runner gates before enabling live execution", () => {
  const base = {
    optIn: true,
    serial: "emulator-5554",
    avdName: EXPECTED_AVD,
    dedicatedAttestation: true,
    packageAllowlist: ["com.nanodata.armsx"],
  };
  for (const field of [
    "optIn",
    "serial",
    "avdName",
    "dedicatedAttestation",
    "packageAllowlist",
  ]) {
    const input = {
      ...base,
      [field]:
        field === "packageAllowlist"
          ? []
          : field === "serial" || field === "avdName"
            ? ""
            : false,
    };
    assert.equal(
      runner.validateInvocation(input).ok,
      false,
      `must reject missing ${field}`,
    );
  }
});

test("requires the exact operator-supplied AVD name", () => {
  const base = {
    optIn: true,
    serial: "emulator-5554",
    dedicatedAttestation: true,
    packageAllowlist: ["com.nanodata.armsx"],
  };
  assert.equal(
    runner.validateInvocation(base).reason,
    "designated_avd_name_missing",
  );
  assert.equal(
    runner.validateInvocation({ ...base, avdName: "" }).reason,
    "designated_avd_name_missing",
  );
  assert.equal(
    runner.validateInvocation({ ...base, avdName: "other-avd" }).reason,
    "designated_avd_name_mismatch",
  );
});

test("assigns stable invocation codes to each rejected operator input", () => {
  const valid = {
    optIn: true,
    serial: "emulator-5554",
    avdName: EXPECTED_AVD,
    dedicatedAttestation: true,
    packageAllowlist: ["com.nanodata.armsx"],
  };
  const cases = [
    [{ ...valid, optIn: false }, "invocation_opt_in_missing"],
    [{ ...valid, serial: "" }, "invocation_serial_missing"],
    [{ ...valid, serial: "device-1" }, "invocation_serial_invalid"],
    [{ ...valid, avdName: undefined }, "invocation_avd_name_missing"],
    [{ ...valid, avdName: "other-avd" }, "invocation_avd_name_mismatch"],
    [
      { ...valid, dedicatedAttestation: false },
      "invocation_attestation_missing",
    ],
    [
      { ...valid, packageAllowlist: [] },
      "invocation_package_allowlist_mismatch",
    ],
  ];
  for (const [input, expectedCode] of cases) {
    assert.equal(runner.validateInvocation(input).failureCode, expectedCode);
  }
});

test("diagnostic rejects missing or incorrect AVD names before subprocess execution", async () => {
  const validEnvironment = {
    EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
    EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
    EMUCHEF_ARMSX1_DEDICATED_AVD:
      "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
    EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
  };

  for (const [avdName, expectedReason] of [
    [undefined, "designated_avd_name_missing"],
    ["", "designated_avd_name_missing"],
    ["other-avd", "designated_avd_name_mismatch"],
  ]) {
    let subprocessCalls = 0;
    const evidence = [];
    const result = await runner.runPreflightDiagnostic({
      env: {
        ...validEnvironment,
        ...(avdName === undefined ? {} : { EMUCHEF_ARMSX1_AVD_NAME: avdName }),
      },
      runAdb: () => assert.fail("Node diagnostic must not run ADB itself"),
      runCommand: async () => {
        subprocessCalls += 1;
        assert.fail("invalid operator input must stop before cargo");
      },
      writeEvidence: (event) => evidence.push(event),
    });

    assert.equal(result, 2);
    assert.equal(subprocessCalls, 0);
    assert.equal(evidence.at(-1).reason, expectedReason);
    assert.equal(evidence.at(-1).live_status, "unverified");
  }
});

test("diagnostic rejects any serial except emulator-5554 before subprocess execution", async () => {
  let subprocessCalls = 0;
  const evidence = [];
  const result = await runner.runPreflightDiagnostic({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5556",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: () => assert.fail("Node diagnostic must not run ADB itself"),
    runCommand: async () => {
      subprocessCalls += 1;
      assert.fail("a different emulator serial must stop before cargo");
    },
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 2);
  assert.equal(subprocessCalls, 0);
  assert.equal(evidence.at(-1).failure_code, "invocation_serial_invalid");
  assert.equal(evidence.at(-1).reason, "exact_emulator_serial_invalid");
});

test("diagnostic dispatches only the exact ignored Rust diagnostic test", async () => {
  const calls = [];
  const evidence = [];
  const result = await runner.runPreflightDiagnostic({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: () => assert.fail("Node diagnostic must not run ADB itself"),
    runCommand: async (command, args) => {
      calls.push({ command, args });
      return {
        status: 0,
        outcome: "completed",
        stdout:
          "test recipe_qualification_armsx1_emulator_tests::armsx1_read_only_preflight_diagnostic ... ok",
        stderr: "",
      };
    },
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 0);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "cargo");
  assert.deepEqual(calls[0].args, [
    "test",
    "--offline",
    "--manifest-path",
    "crates/emuchef-rust-backend/Cargo.toml",
    "recipe_qualification_armsx1_emulator_tests::armsx1_read_only_preflight_diagnostic",
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
  ]);
  assert.deepEqual(evidence, [
    {
      status: "completed",
      live_status: "unverified",
      execution: "DIAGNOSTIC_ONLY",
      reason: "preflight_diagnostic_completed",
      diagnostic_result: "recorded",
    },
  ]);
});

test("durably records diagnostic completion without marking live qualification passed", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "diagnostic-runner-evidence.json");
    const result = await runner.runPreflightDiagnostic({
      env: {
        EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
        EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
        EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
        EMUCHEF_ARMSX1_DEDICATED_AVD:
          "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
        EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
      },
      runAdb: () => assert.fail("Node diagnostic must not run ADB itself"),
      runCommand: async () => ({
        status: 0,
        outcome: "completed",
        stdout:
          "test recipe_qualification_armsx1_emulator_tests::armsx1_read_only_preflight_diagnostic ... ok",
        stderr: "",
      }),
      evidencePath,
    });

    assert.equal(result, 0);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "completed");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.execution, "DIAGNOSTIC_ONLY");
    assert.equal(evidence.reason, "preflight_diagnostic_completed");
    assert.equal(evidence.diagnostic_result, "recorded");
  });
});

test("CLI diagnostic option rejects an AVD-name mismatch without starting cargo", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "diagnostic-runner-evidence.json");
    const operationMarker = join(directory, "unexpected-subprocess.txt");
    const preloadPath = join(directory, "block-subprocess.cjs");
    await writeFile(
      preloadPath,
      `const fs = require("node:fs");
const childProcess = require("node:child_process");
const record = (name) => (...args) => {
  fs.appendFileSync(process.env.ARMSX1_OPERATION_MARKER, name + "\\n");
  throw new Error("Unexpected subprocess in diagnostic CLI test.");
};
childProcess.spawn = record("spawn");
childProcess.spawnSync = record("spawnSync");
require("node:module").syncBuiltinESMExports();
`,
    );
    const env = {
      ...process.env,
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: "other-avd",
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
      EMUCHEF_ARMSX1_DIAGNOSTIC_RUNNER_EVIDENCE_PATH: evidencePath,
      ARMSX1_OPERATION_MARKER: operationMarker,
    };
    const child = spawnSync(
      process.execPath,
      [
        "--require",
        preloadPath,
        join(REPOSITORY_ROOT, RELATIVE_RUNNER_PATH),
        "--diagnose-preflight",
      ],
      { cwd: REPOSITORY_ROOT, encoding: "utf8", env, timeout: 10_000 },
    );

    assert.ifError(child.error);
    assert.equal(child.status, 2, child.stderr);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.failure_code, "invocation_avd_name_mismatch");
    assert.equal(evidence.live_status, "unverified");
    await assert.rejects(
      readFile(operationMarker),
      (error) => error.code === "ENOENT",
    );
  });
});

test("unexpected diagnostic CLI rejection never writes to live runner evidence", async () => {
  const directory = await mkdtemp(
    join(APPROVED_EVIDENCE_DIRECTORY, "diagnostic-cli-catch-test-"),
  );
  try {
    const diagnosticEvidencePath = directory;
    const liveEvidencePath = join(directory, "live-runner-evidence.json");
    const liveRecord = "preserve-live-runner-record\n";
    await writeFile(liveEvidencePath, liveRecord);
    const env = { ...process.env };
    delete env.EMUCHEF_RUN_ARMSX1_EMULATOR;
    env.EMUCHEF_ARMSX1_DIAGNOSTIC_RUNNER_EVIDENCE_PATH = diagnosticEvidencePath;
    env.EMUCHEF_ARMSX1_RUNNER_EVIDENCE_PATH = liveEvidencePath;

    const child = spawnSync(
      process.execPath,
      [join(REPOSITORY_ROOT, RELATIVE_RUNNER_PATH), "--diagnose-preflight"],
      { cwd: REPOSITORY_ROOT, encoding: "utf8", env, timeout: 10_000 },
    );

    assert.ifError(child.error);
    assert.equal(child.status, 2, child.stderr);
    assert.equal(await readFile(liveEvidencePath, "utf8"), liveRecord);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("accepts the emulator console name followed by its OK response", () => {
  assert.equal(typeof runner.parseAvdNameResponse, "function");
  assert.deepEqual(
    runner.parseAvdNameResponse({
      status: 0,
      stdout: `${EXPECTED_AVD}\r\nOK\r\n`,
      stderr: "",
    }),
    { state: "valid", name: EXPECTED_AVD },
  );
});

test("rejects KO, error and malformed emulator console responses", () => {
  assert.equal(typeof runner.parseAvdNameResponse, "function");
  for (const stdout of [
    "KO: emulator not ready\r\n",
    "KO: emulator not ready\r\nOK\r\n",
    "ERROR: no AVD\r\n",
    "ERROR: no AVD\r\nOK\r\n",
    `${EXPECTED_AVD}\r\n`,
    `OK\r\n${EXPECTED_AVD}\r\n`,
    `${EXPECTED_AVD}\r\nOK\r\nERROR: trailing output\r\n`,
    `${EXPECTED_AVD}\r\nOK\r\nOK\r\n`,
    ` ${EXPECTED_AVD}\r\nOK\r\n`,
    `${EXPECTED_AVD} \r\nOK\r\n`,
    `\t\r\nOK\r\n`,
  ]) {
    assert.equal(
      runner.parseAvdNameResponse({ status: 0, stdout, stderr: "" }).state,
      "invalid",
    );
  }
});

test("uses the boot AVD property only when console output is unavailable and emulator identity is trusted", () => {
  assert.equal(typeof runner.resolveAvdName, "function");
  const calls = [];
  const result = runner.resolveAvdName((args) => {
    calls.push(args);
    if (args[0] === "emu")
      return {
        status: 1,
        stdout: "",
        stderr: "",
        error: new Error("console unavailable"),
      };
    return { status: 0, stdout: `${EXPECTED_AVD}\n`, stderr: "" };
  }, validProfile());

  assert.deepEqual(result, {
    ok: true,
    name: EXPECTED_AVD,
    source: "validated_emulator_property",
  });
  assert.deepEqual(calls, [
    ["emu", "avd", "name"],
    ["shell", "getprop", "ro.boot.qemu.avd_name"],
  ]);
});

test("uses the property fallback after a console command throws only for a validated profile", () => {
  const calls = [];
  const result = runner.resolveAvdName((args) => {
    calls.push(args);
    if (args[0] === "emu") throw new Error("console command unavailable");
    return { status: 0, stdout: `${EXPECTED_AVD}\n`, stderr: "" };
  }, validProfile());

  assert.deepEqual(result, {
    ok: true,
    name: EXPECTED_AVD,
    source: "validated_emulator_property",
  });
  assert.deepEqual(calls, [
    ["emu", "avd", "name"],
    ["shell", "getprop", "ro.boot.qemu.avd_name"],
  ]);
});

test("fails closed on unavailable console output when emulator identity is untrusted", () => {
  const calls = [];
  const profile = { ...validProfile(), brand: "generic" };
  const result = runner.resolveAvdName((args) => {
    calls.push(args);
    return { status: 1, stdout: "", stderr: "" };
  }, profile);

  assert.deepEqual(result, { ok: false, reason: "avd_name_unavailable" });
  assert.deepEqual(calls, [["emu", "avd", "name"]]);
});

test("does not use the property fallback after KO, error, or malformed console output", () => {
  for (const consoleResult of [
    { status: 0, stdout: "KO: emulator unavailable\nOK\n", stderr: "" },
    { status: 0, stdout: "ERROR: no AVD\nOK\n", stderr: "" },
    { status: 0, stdout: `${EXPECTED_AVD}\nOK\nextra\n`, stderr: "" },
    { status: 1, stdout: "", stderr: "error: target unavailable" },
  ]) {
    const calls = [];
    const result = runner.resolveAvdName((args) => {
      calls.push(args);
      return consoleResult;
    }, validProfile());

    assert.equal(result.ok, false);
    assert.deepEqual(calls, [["emu", "avd", "name"]]);
  }
});

test("does not use the property fallback to accept a different AVD", () => {
  const result = runner.resolveAvdName(
    (args) =>
      args[0] === "emu"
        ? { status: 1, stdout: "", stderr: "" }
        : { status: 0, stdout: "Other-AVD\n", stderr: "" },
    validProfile(),
  );

  assert.deepEqual(result, { ok: false, reason: "avd_name_mismatch" });
});

test("fails closed when the validated AVD property fallback command throws", () => {
  const calls = [];
  const result = runner.resolveAvdName((args) => {
    calls.push(args);
    if (args[0] === "emu") return { status: 1, stdout: "", stderr: "" };
    throw new Error("property command unavailable");
  }, validProfile());

  assert.deepEqual(result, {
    ok: false,
    reason: "avd_name_property_unavailable",
  });
  assert.deepEqual(calls, [
    ["emu", "avd", "name"],
    ["shell", "getprop", "ro.boot.qemu.avd_name"],
  ]);
});

test("rejects malformed or ambiguous AVD property fallback values", () => {
  for (const stdout of [
    "",
    "\n",
    ` ${EXPECTED_AVD}\n`,
    `${EXPECTED_AVD}\nOther-AVD\n`,
    `${EXPECTED_AVD}\nOK\n`,
  ]) {
    const result = runner.resolveAvdName(
      (args) =>
        args[0] === "emu"
          ? { status: 1, stdout: "", stderr: "" }
          : { status: 0, stdout, stderr: "" },
      validProfile(),
    );

    assert.equal(result.ok, false, JSON.stringify(stdout));
  }
});

test("evidence destination is approved, external, absolute, and symlink checked", async () => {
  const approvedPath = join(
    REPOSITORY_ROOT,
    "docs/testing/recipe-qualification/armsx1/runner-evidence.json",
  );
  assert.equal(runner.DEFAULT_RUNNER_EVIDENCE_PATH, approvedPath);
  assert.equal(
    await runner.resolveEvidenceDestination(approvedPath),
    approvedPath,
  );

  await withEvidenceDirectory(async (directory) => {
    const externalPath = join(directory, "runner-evidence.json");
    const canonicalExternalPath = join(
      await realpath(directory),
      "runner-evidence.json",
    );
    assert.equal(
      await runner.resolveEvidenceDestination(externalPath),
      canonicalExternalPath,
    );
    await assert.rejects(
      runner.resolveEvidenceDestination(
        join(REPOSITORY_ROOT, ".chatgpt", "forbidden-evidence.json"),
      ),
    );
    await assert.rejects(
      runner.resolveEvidenceDestination(
        join(REPOSITORY_ROOT, "unauthorized-evidence.json"),
      ),
    );
    await assert.rejects(
      runner.resolveEvidenceDestination("relative-evidence.json"),
    );

    const repositoryAlias = join(directory, "repository-alias");
    await symlink(REPOSITORY_ROOT, repositoryAlias, "dir");
    await assert.rejects(
      runner.resolveEvidenceDestination(
        join(repositoryAlias, "unauthorized-evidence.json"),
      ),
    );
  });
});

test("requires the Google Android 16 brand and release profile before AVD fallback", () => {
  assert.equal(typeof runner.validateEmulatorProfile, "function");
  assert.deepEqual(runner.validateEmulatorProfile(validProfile()), {
    ok: true,
  });
  for (const [field, value] of [
    ["state", "offline"],
    ["bootCompleted", "0"],
    ["apiLevel", "35"],
    ["androidRelease", "15"],
    ["abi", "x86_64"],
    ["brand", "generic"],
    ["manufacturer", "Example"],
    ["model", "generic"],
    ["qemu", "0"],
    ["uid", "0"],
  ]) {
    assert.equal(
      runner.validateEmulatorProfile({ ...validProfile(), [field]: value }).ok,
      false,
      field,
    );
  }
});

test("runs read-only preflight against a mocked ADB subprocess", () => {
  const values = new Map([
    ["get-state", "device"],
    ["sys.boot_completed", "1"],
    ["ro.build.version.sdk", "36"],
    ["ro.build.version.release", "16"],
    ["ro.product.cpu.abi", "arm64-v8a"],
    ["ro.product.brand", "google"],
    ["ro.product.manufacturer", "Google"],
    ["ro.product.model", "sdk_gphone64_arm64"],
    ["ro.kernel.qemu", "1"],
    ["id", "2000"],
  ]);
  const calls = [];
  const result = runner.preflightEmulator((args) => {
    calls.push(args);
    if (args[0] === "emu")
      return { status: 0, stdout: `${EXPECTED_AVD}\nOK\n`, stderr: "" };
    const key =
      args[0] === "get-state"
        ? "get-state"
        : args[1] === "id"
          ? "id"
          : args.at(-1);
    return { status: 0, stdout: `${values.get(key)}\n`, stderr: "" };
  });

  assert.equal(result.ok, true);
  assert.equal(result.avdName, EXPECTED_AVD);
  assert.equal(
    calls.some((args) => args.includes("ro.product.brand")),
    true,
  );
  assert.equal(
    calls.some((args) => args.includes("ro.build.version.release")),
    true,
  );
});

test("blocks mocked preflight when Android brand or release does not match", () => {
  const values = new Map([
    ["get-state", "device"],
    ["sys.boot_completed", "1"],
    ["ro.build.version.sdk", "36"],
    ["ro.build.version.release", "16"],
    ["ro.product.cpu.abi", "arm64-v8a"],
    ["ro.product.brand", "google"],
    ["ro.product.manufacturer", "Google"],
    ["ro.product.model", "sdk_gphone64_arm64"],
    ["ro.kernel.qemu", "1"],
    ["id", "2000"],
  ]);
  for (const [property, value] of [
    ["ro.product.brand", "generic"],
    ["ro.build.version.release", "15"],
  ]) {
    values.set(property, value);
    const calls = [];
    const result = runner.preflightEmulator((args) => {
      calls.push(args);
      const key =
        args[0] === "get-state"
          ? "get-state"
          : args[1] === "id"
            ? "id"
            : args.at(-1);
      return { status: 0, stdout: `${values.get(key)}\n`, stderr: "" };
    });

    assert.deepEqual(result, {
      ok: false,
      status: "blocked",
      liveStatus: "unverified",
      reason: "emulator_profile_mismatch",
    });
    assert.equal(
      calls.some((args) => args[0] === "emu"),
      false,
    );
    values.set(property, property === "ro.product.brand" ? "google" : "16");
  }
});

test("does not report a zero-test filtered cargo invocation as live qualification success", async () => {
  const evidence = [];
  const result = await runner.runQualification({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: (_serial, args) =>
      args[0] === "emu"
        ? { status: 0, stdout: `${EXPECTED_AVD}\nOK\n`, stderr: "" }
        : {
            status: 0,
            stdout: `${new Map([
              ["get-state", "device"],
              ["sys.boot_completed", "1"],
              ["ro.build.version.sdk", "36"],
              ["ro.build.version.release", "16"],
              ["ro.product.cpu.abi", "arm64-v8a"],
              ["ro.product.brand", "google"],
              ["ro.product.manufacturer", "Google"],
              ["ro.product.model", "sdk_gphone64_arm64"],
              ["ro.kernel.qemu", "1"],
              ["id", "2000"],
            ]).get(
              args[0] === "get-state"
                ? "get-state"
                : args[1] === "id"
                  ? "id"
                  : args.at(-1),
            )}\n`,
            stderr: "",
          },
    runCommand: async () => ({
      status: 0,
      outcome: "completed",
      stdout: "test result: ok. 0 passed; 1 filtered out",
      stderr: "",
    }),
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 2);
  assert.deepEqual(evidence, [
    {
      status: "failed",
      live_status: "unverified",
      execution: "FAILED",
      reason: "live_test_not_executed",
      termination: undefined,
    },
  ]);
});

test("records process-tree verification failures as blocked and unverified", async () => {
  const evidence = [];
  const values = new Map([
    ["get-state", "device"],
    ["sys.boot_completed", "1"],
    ["ro.build.version.sdk", "36"],
    ["ro.build.version.release", "16"],
    ["ro.product.cpu.abi", "arm64-v8a"],
    ["ro.product.brand", "google"],
    ["ro.product.manufacturer", "Google"],
    ["ro.product.model", "sdk_gphone64_arm64"],
    ["ro.kernel.qemu", "1"],
    ["id", "2000"],
  ]);
  const result = await runner.runQualification({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: (_serial, args) =>
      args[0] === "emu"
        ? { status: 0, stdout: `${EXPECTED_AVD}\nOK\n`, stderr: "" }
        : {
            status: 0,
            stdout: `${values.get(args[0] === "get-state" ? "get-state" : args[1] === "id" ? "id" : args.at(-1))}\n`,
            stderr: "",
          },
    runCommand: async () => {
      const error = new Error("sanitized process supervision failure");
      error.code = "PROCESS_TREE_TERMINATION_UNVERIFIED";
      error.outcome = "unverified";
      error.liveStatus = "unverified";
      error.termination =
        "process_group_disappearance_and_child_close_confirmed";
      throw error;
    },
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 2);
  assert.deepEqual(evidence, [
    {
      status: "blocked",
      live_status: "unverified",
      execution: "INTERRUPTED",
      reason: "process_tree_termination_unverified",
      termination: "process_group_disappearance_and_child_close_confirmed",
    },
  ]);
});

test("does not accept similar or duplicate ignored test output as live qualification success", async () => {
  const evidence = [];
  const values = new Map([
    ["get-state", "device"],
    ["sys.boot_completed", "1"],
    ["ro.build.version.sdk", "36"],
    ["ro.build.version.release", "16"],
    ["ro.product.cpu.abi", "arm64-v8a"],
    ["ro.product.brand", "google"],
    ["ro.product.manufacturer", "Google"],
    ["ro.product.model", "sdk_gphone64_arm64"],
    ["ro.kernel.qemu", "1"],
    ["id", "2000"],
  ]);
  const result = await runner.runQualification({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: (_serial, args) =>
      args[0] === "emu"
        ? { status: 0, stdout: `${EXPECTED_AVD}\nOK\n`, stderr: "" }
        : {
            status: 0,
            stdout: `${values.get(args[0] === "get-state" ? "get-state" : args[1] === "id" ? "id" : args.at(-1))}\n`,
            stderr: "",
          },
    runCommand: async () => ({
      status: 0,
      outcome: "completed",
      stdout: [
        "test different_module::recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
        "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
        "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
      ].join("\n"),
      stderr: "",
    }),
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 2);
  assert.deepEqual(evidence, [
    {
      status: "failed",
      live_status: "unverified",
      execution: "FAILED",
      reason: "live_test_result_ambiguous",
      termination: undefined,
    },
  ]);
});

test("binds the validated AVD property fallback and live test to the explicit ADB target", async () => {
  const calls = [];
  const evidence = [];
  const values = new Map([
    ["get-state", "device"],
    ["sys.boot_completed", "1"],
    ["ro.build.version.sdk", "36"],
    ["ro.build.version.release", "16"],
    ["ro.product.cpu.abi", "arm64-v8a"],
    ["ro.product.brand", "google"],
    ["ro.product.manufacturer", "Google"],
    ["ro.product.model", "sdk_gphone64_arm64"],
    ["ro.kernel.qemu", "1"],
    ["id", "2000"],
    ["ro.boot.qemu.avd_name", EXPECTED_AVD],
  ]);
  const result = await runner.runQualification({
    env: {
      EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
      EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
      EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
      EMUCHEF_ARMSX1_DEDICATED_AVD:
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
    },
    runAdb: (serial, args) => {
      calls.push({ serial, args });
      if (args[0] === "emu")
        return {
          status: 1,
          stdout: "",
          stderr: "",
          error: new Error("console unavailable"),
        };
      const key =
        args[0] === "get-state"
          ? "get-state"
          : args[1] === "id"
            ? "id"
            : args.at(-1);
      return { status: 0, stdout: `${values.get(key)}\n`, stderr: "" };
    },
    runCommand: async () => ({
      status: 0,
      outcome: "completed",
      stdout:
        "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
      stderr: "",
    }),
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 0);
  assert.deepEqual(evidence, [
    {
      status: "passed",
      live_status: "passed",
      execution: "PASSED",
      reason: "live_qualification_passed",
      termination: "process_group_disappearance_and_child_close_confirmed",
    },
  ]);
  assert.ok(calls.some(({ args }) => args.at(-1) === "ro.boot.qemu.avd_name"));
  assert.ok(calls.every(({ serial }) => serial === "emulator-5554"));
});

test("returns failure evidence when the explicit opt-in is missing", async () => {
  const evidence = [];
  const result = await runner.runQualification({
    env: {},
    runAdb: () => assert.fail("ADB must not run"),
    runCommand: () => assert.fail("cargo must not run"),
    writeEvidence: (event) => evidence.push(event),
  });

  assert.equal(result, 2);
  assert.deepEqual(evidence, [
    {
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: "explicit_opt_in_missing",
    },
  ]);
});

test("cancels escalation and resolves when the whole process group exits after SIGTERM", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_001;
  const signals = [];
  let processGroupAlive = true;
  const command = runner.runManagedCommand("mock-cargo", ["test"], {
    timeoutMs: 5,
    spawnProcess: (command, args, spawnOptions) => {
      assert.equal(command, "mock-cargo");
      assert.deepEqual(args, ["test"]);
      assert.equal(
        spawnOptions.detached,
        true,
        "the wrapper must start in its own process group",
      );
      return child;
    },
    signalProcessTree: (target, signal, processGroupId) => {
      assert.equal(target, child);
      assert.equal(processGroupId, child.pid);
      signals.push(signal);
      if (signal === "SIGTERM") {
        processGroupAlive = false;
        setImmediate(() => {
          child.emit("exit", null, signal);
          setTimeout(() => child.emit("close", null, signal), 20);
        });
      }
    },
    isProcessGroupAlive: (processGroupId) => {
      assert.equal(processGroupId, child.pid);
      return processGroupAlive;
    },
  });

  let deadline;
  const result = await Promise.race([
    command,
    new Promise((resolve) => {
      deadline = setTimeout(() => resolve(undefined), 100);
    }),
  ]);
  clearTimeout(deadline);

  assert.notEqual(
    result,
    undefined,
    "the command should resolve after its close event",
  );
  assert.equal(result.outcome, "timed_out");
  assert.deepEqual(signals, ["SIGTERM"]);
});

test("escalates after direct-child exit while a descendant keeps the process group alive", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_003;
  const events = [];
  let processGroupAlive = true;
  let termRequestedAt;
  let commandResolved = false;
  let resolvedBeforeClose;
  let resolvedWhileGroupAlive;
  const command = runner
    .runManagedCommand("mock-cargo", ["test"], {
      timeoutMs: 5,
      spawnProcess: () => child,
      isProcessGroupAlive: (processGroupId) => {
        assert.equal(processGroupId, child.pid);
        return processGroupAlive;
      },
      signalProcessTree: (target, signal, processGroupId) => {
        assert.equal(target, child);
        assert.equal(
          processGroupId,
          child.pid,
          "signals must target the original process group",
        );
        events.push(signal);
        if (signal === "SIGTERM") {
          termRequestedAt = Date.now();
          setImmediate(() => {
            events.push("child-exit");
            child.emit("exit", null, signal);
          });
        }
        if (signal === "SIGKILL") {
          assert.ok(
            Date.now() - termRequestedAt >= 900,
            "SIGKILL must wait for the grace timeout",
          );
          setTimeout(() => {
            resolvedBeforeClose = commandResolved;
            events.push("close");
            child.emit("close", null, signal);
            setTimeout(() => {
              processGroupAlive = false;
              events.push("group-gone");
            }, 20);
          }, 20);
        }
      },
    })
    .then((result) => {
      commandResolved = true;
      resolvedWhileGroupAlive = processGroupAlive;
      return result;
    });

  let deadline;
  const result = await Promise.race([
    command,
    new Promise((resolve) => {
      deadline = setTimeout(() => resolve(undefined), 1_250);
    }),
  ]);
  clearTimeout(deadline);

  assert.notEqual(
    result,
    undefined,
    "the runner should resolve after killing the lingering process group and reaping the child",
  );
  assert.equal(result.outcome, "timed_out");
  assert.deepEqual(events, [
    "SIGTERM",
    "child-exit",
    "SIGKILL",
    "close",
    "group-gone",
  ]);
  assert.equal(resolvedBeforeClose, false);
  assert.equal(resolvedWhileGroupAlive, false);
  assert.equal(commandResolved, true);
});

test("sends SIGKILL after the grace timeout and waits for the child close event", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_004;
  const events = [];
  let processGroupAlive = true;
  let termRequestedAt;
  let commandResolved = false;
  let resolvedBeforeClose;
  const command = runner
    .runManagedCommand("mock-cargo", ["test"], {
      timeoutMs: 5,
      spawnProcess: () => child,
      isProcessGroupAlive: (processGroupId) => {
        assert.equal(processGroupId, child.pid);
        return processGroupAlive;
      },
      signalProcessTree: (target, signal) => {
        assert.equal(target, child);
        events.push(signal);
        if (signal === "SIGTERM") termRequestedAt = Date.now();
        if (signal === "SIGKILL") {
          assert.ok(
            Date.now() - termRequestedAt >= 900,
            "SIGKILL must wait for the grace timeout",
          );
          processGroupAlive = false;
          setImmediate(() => {
            child.emit("exit", null, signal);
            setTimeout(() => {
              resolvedBeforeClose = commandResolved;
              events.push("close");
              child.emit("close", null, signal);
            }, 20);
          });
        }
      },
    })
    .then((result) => {
      commandResolved = true;
      return result;
    });

  const result = await command;

  assert.equal(result.outcome, "timed_out");
  assert.deepEqual(events, ["SIGTERM", "SIGKILL", "close"]);
  assert.equal(resolvedBeforeClose, false);
  assert.equal(commandResolved, true);
});

test("does not signal a process group that is already gone", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_005;
  const signals = [];
  const command = runner.runManagedCommand("mock-cargo", ["test"], {
    timeoutMs: 5,
    spawnProcess: () => child,
    isProcessGroupAlive: (processGroupId) => {
      assert.equal(processGroupId, child.pid);
      return false;
    },
    signalProcessTree: (target, signal) => {
      assert.equal(target, child);
      signals.push(signal);
    },
  });

  setTimeout(() => {
    child.emit("exit", 0, null);
    child.emit("close", 0, null);
  }, 20);

  const result = await command;

  assert.equal(result.outcome, "timed_out");
  assert.deepEqual(signals, []);
});

test("keeps signal handlers installed until process-group disappearance and child close", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_008;
  let processGroupAlive = true;
  let childClosed = false;
  const signalCalls = [];
  const originalSignalListenerCounts = ["SIGINT", "SIGTERM"].map((signal) =>
    process.listenerCount(signal),
  );
  const command = runner.runManagedCommand("mock-cargo", ["test"], {
    timeoutMs: 10_000,
    terminationGraceMs: 10_000,
    spawnProcess: () => child,
    isProcessGroupAlive: () => processGroupAlive,
    signalProcessTree: (target, signal, processGroupId) => {
      assert.equal(target, child);
      assert.equal(processGroupId, child.pid);
      signalCalls.push(signal);
    },
  });
  const outcome = command.then(
    (result) => ({ type: "resolved", result }),
    (error) => ({ type: "rejected", error }),
  );

  try {
    for (const signal of ["SIGINT", "SIGTERM"]) {
      const listener = process.rawListeners(signal).at(-1);
      assert.equal(typeof listener, "function");
      listener.call(process);
    }
    assert.deepEqual(signalCalls, ["SIGTERM"]);
    assert.deepEqual(
      ["SIGINT", "SIGTERM"].map((signal) => process.listenerCount(signal)),
      originalSignalListenerCounts.map((count) => count + 1),
      "signal handlers must remain installed while the process group can still be alive",
    );

    processGroupAlive = false;
    child.emit("exit", null, "SIGTERM");
    childClosed = true;
    child.emit("close", null, "SIGTERM");
    const settled = await outcome;
    assert.equal(settled.type, "resolved");
    assert.equal(settled.result.outcome, "interrupted");
  } finally {
    processGroupAlive = false;
    if (!childClosed) {
      child.emit("exit", null, "SIGTERM");
      child.emit("close", null, "SIGTERM");
    }
  }

  assert.deepEqual(
    ["SIGINT", "SIGTERM"].map((signal) => process.listenerCount(signal)),
    originalSignalListenerCounts,
    "signal handlers must be removed after verified process cleanup",
  );
});

test("retains supervision through unknown process-group liveness and permission errors", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_006;
  const signals = [];
  let processGroupState = "unknown";
  let unknownObservations = 0;
  let processGroupGoneObserved = false;
  let closeEmitted = false;
  let commandSettled = false;
  const originalSignalListenerCounts = ["SIGINT", "SIGTERM"].map((signal) =>
    process.listenerCount(signal),
  );
  const command = runner.runManagedCommand("mock-cargo", ["test"], {
    timeoutMs: 5,
    terminationGraceMs: 15,
    processGroupPollIntervalMs: 5,
    verificationTimeoutMs: 10,
    spawnProcess: () => child,
    isProcessGroupAlive: (processGroupId) => {
      assert.equal(processGroupId, child.pid);
      if (processGroupState === "gone") {
        processGroupGoneObserved = true;
        return false;
      }
      unknownObservations += 1;
      const error = new Error("permission denied");
      error.code = "EPERM";
      throw error;
    },
    signalProcessTree: (target, signal, processGroupId) => {
      assert.equal(target, child);
      assert.equal(processGroupId, child.pid);
      signals.push(signal);
      const error = new Error("permission denied");
      error.code = "EPERM";
      throw error;
    },
  });
  const observedCommand = command.then(
    (result) => {
      commandSettled = true;
      return { type: "resolved", result };
    },
    (error) => {
      commandSettled = true;
      return { type: "rejected", error };
    },
  );

  try {
    await waitFor(
      () =>
        (unknownObservations >= 2 &&
          signals.includes("SIGTERM") &&
          signals.includes("SIGKILL")) ||
        commandSettled,
      "repeated unknown liveness observations and termination attempts",
    );
    assert.ok(
      unknownObservations >= 2,
      "unknown liveness must be retried instead of releasing process ownership",
    );
    assert.equal(
      commandSettled,
      false,
      "unknown liveness must not settle while the process group may be alive",
    );
    assert.equal(
      child.listenerCount("close"),
      1,
      "child reaping supervision must remain attached",
    );

    processGroupState = "gone";
    await waitFor(
      () => processGroupGoneObserved || commandSettled,
      "confirmed process-group disappearance",
    );
    assert.equal(processGroupGoneObserved, true);
    assert.equal(
      commandSettled,
      false,
      "the promise must wait for the direct child close/reap event",
    );
    assert.deepEqual(
      ["SIGINT", "SIGTERM"].map((signal) => process.listenerCount(signal)),
      originalSignalListenerCounts.map((count) => count + 1),
      "signal supervision must remain attached until the child close/reap event",
    );
  } finally {
    processGroupState = "gone";
    if (!closeEmitted) {
      child.emit("exit", null, "SIGKILL");
      closeEmitted = true;
      child.emit("close", null, "SIGKILL");
    }
  }

  const outcome = await observedCommand;
  assert.equal(outcome.type, "rejected");
  assert.equal(outcome.error.code, "PROCESS_TREE_TERMINATION_UNVERIFIED");
  assert.equal(outcome.error.outcome, "unverified");
  assert.equal(outcome.error.liveStatus, "unverified");
  assert.equal(
    outcome.error.termination,
    "process_group_disappearance_and_child_close_confirmed",
  );
  assert.doesNotMatch(outcome.error.message, /permission denied/i);
  assert.equal(child.stdout.listenerCount("data"), 0);
  assert.equal(child.stderr.listenerCount("data"), 0);
  assert.equal(child.listenerCount("error"), 0);
  assert.equal(child.listenerCount("exit"), 0);
  assert.equal(child.listenerCount("close"), 0);
  assert.deepEqual(
    ["SIGINT", "SIGTERM"].map((signal) => process.listenerCount(signal)),
    originalSignalListenerCounts,
  );
});

test("does not settle while a SIGKILLed process group stays alive", async () => {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = 31_007;
  const signals = [];
  let processGroupAlive = true;
  let processGroupObservations = 0;
  let processGroupGoneObserved = false;
  let closeEmitted = false;
  let firstKillAt;
  let observationsAtFirstKill;
  let commandSettled = false;
  const command = runner.runManagedCommand("mock-cargo", ["test"], {
    timeoutMs: 5,
    terminationGraceMs: 5,
    processGroupPollIntervalMs: 5,
    verificationTimeoutMs: 10,
    spawnProcess: () => child,
    isProcessGroupAlive: (processGroupId) => {
      assert.equal(processGroupId, child.pid);
      processGroupObservations += 1;
      if (!processGroupAlive) processGroupGoneObserved = true;
      return processGroupAlive;
    },
    signalProcessTree: (target, signal, processGroupId) => {
      signals.push(signal);
      assert.equal(target, child);
      assert.equal(processGroupId, child.pid);
      if (signal === "SIGTERM") {
        setImmediate(() => {
          child.emit("exit", null, signal);
          closeEmitted = true;
          child.emit("close", null, signal);
        });
      }
      if (signal === "SIGKILL") {
        firstKillAt ??= Date.now();
        observationsAtFirstKill ??= processGroupObservations;
      }
    },
  });
  const observedCommand = command.then(
    (result) => {
      commandSettled = true;
      return { type: "resolved", result };
    },
    (error) => {
      commandSettled = true;
      return { type: "rejected", error };
    },
  );

  try {
    await waitFor(
      () => firstKillAt !== undefined || commandSettled,
      "SIGKILL escalation",
    );
    assert.notEqual(firstKillAt, undefined);
    await new Promise((resolve) => setTimeout(resolve, 40));
    assert.equal(
      commandSettled,
      false,
      "an alive process group must retain supervision after SIGKILL verification times out",
    );
    assert.equal(processGroupAlive, true);
    assert.equal(
      closeEmitted,
      true,
      "the direct child close event alone must not release process-group ownership",
    );
    assert.equal(child.listenerCount("close"), 1);
    assert.ok(
      processGroupObservations > observationsAtFirstKill,
      "process-group liveness should continue to be polled",
    );

    processGroupAlive = false;
    await waitFor(
      () => processGroupGoneObserved || commandSettled,
      "confirmed process-group disappearance",
    );
    assert.equal(processGroupGoneObserved, true);
  } finally {
    processGroupAlive = false;
    if (!closeEmitted) {
      child.emit("exit", null, "SIGTERM");
      closeEmitted = true;
      child.emit("close", null, "SIGTERM");
    }
  }

  const outcome = await observedCommand;
  assert.equal(outcome.type, "rejected");
  assert.equal(outcome.error.code, "PROCESS_TREE_TERMINATION_UNVERIFIED");
  assert.equal(outcome.error.outcome, "unverified");
  assert.equal(outcome.error.liveStatus, "unverified");
  assert.equal(
    outcome.error.termination,
    "process_group_disappearance_and_child_close_confirmed",
  );
  assert.equal(child.stdout.listenerCount("data"), 0);
  assert.equal(child.stderr.listenerCount("data"), 0);
  assert.equal(child.listenerCount("error"), 0);
  assert.equal(child.listenerCount("exit"), 0);
  assert.equal(child.listenerCount("close"), 0);
});

test("does not spawn a live command when process-tree management is unsupported", async () => {
  let spawned = false;
  const result = await runner.runManagedCommand("mock-cargo", ["test"], {
    platform: "win32",
    spawnProcess: () => {
      spawned = true;
      throw new Error("must not start");
    },
  });

  assert.equal(spawned, false);
  assert.equal(result.outcome, "spawn_failed");
  assert.equal(result.liveStatus, "unverified");
  assert.equal(result.status, null);
  assert.match(result.stderr, /unsupported on Windows.*not started/);
});

const LIVE_ENV = {
  EMUCHEF_RUN_ARMSX1_EMULATOR: "1",
  EMUCHEF_ARMSX1_AVD_SERIAL: "emulator-5554",
  EMUCHEF_ARMSX1_AVD_NAME: EXPECTED_AVD,
  EMUCHEF_ARMSX1_DEDICATED_AVD:
    "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
  EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST: "com.nanodata.armsx",
};

const PREFLIGHT_VALUES = new Map([
  ["get-state", "device"],
  ["sys.boot_completed", "1"],
  ["ro.build.version.sdk", "36"],
  ["ro.build.version.release", "16"],
  ["ro.product.cpu.abi", "arm64-v8a"],
  ["ro.product.brand", "google"],
  ["ro.product.manufacturer", "Google"],
  ["ro.product.model", "sdk_gphone64_arm64"],
  ["ro.kernel.qemu", "1"],
  ["id", "2000"],
]);

async function withEvidenceDirectory(body) {
  const directory = await mkdtemp(join(tmpdir(), "armsx1-runner-evidence-"));
  try {
    await body(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

function mockedPreflightAdb(failedCheck) {
  return (_serial, args) => {
    if (args[0] === "emu") {
      return { status: 0, stdout: `${EXPECTED_AVD}\nOK\n`, stderr: "" };
    }
    const key =
      args[0] === "get-state"
        ? "get-state"
        : args[1] === "id"
          ? "id"
          : args.at(-1);
    if (key === failedCheck) {
      return {
        status: 1,
        stdout: "",
        stderr: `serial=emulator-5554 avd=${EXPECTED_AVD} token=private-test-token`,
      };
    }
    return { status: 0, stdout: `${PREFLIGHT_VALUES.get(key)}\n`, stderr: "" };
  };
}

async function readSanitizedEvidence(evidencePath) {
  const text = await readFile(evidencePath, "utf8");
  const evidence = JSON.parse(text);
  assert.equal(text.endsWith("\n"), true);
  for (const secret of ["emulator-5554", EXPECTED_AVD, "private-test-token"]) {
    assert.equal(text.includes(secret), false, `evidence leaked ${secret}`);
  }
  assert.equal(text.includes("stderr"), false);
  return { evidence, text };
}

test("atomically persists sanitized invocation-blocked evidence", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    await writeFile(
      evidencePath,
      '{"status":"passed","live_status":"passed"}\n',
    );

    const result = await runner.runQualification({
      env: {},
      evidencePath,
    });

    assert.equal(result, 2);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "blocked");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.reason, "explicit_opt_in_missing");
    assert.equal(evidence.failure_category, "invocation");
    assert.deepEqual(evidence.preflight_outcomes, [
      { check: "preflight", outcome: "not_run" },
    ]);
    assert.equal(evidence.termination_state, "not_started");
    assert.equal(evidence.cleanup_state, "not_started");

    const entries = await readdir(directory);
    assert.deepEqual(entries, [basename(evidencePath)]);
    const metadata = await stat(evidencePath);
    assert.equal(metadata.mode & 0o777, 0o600);
  });
});

test("unsafe evidence destination blocks before ADB and uses a safe evidence sink", async () => {
  const events = [];
  let adbCalls = 0;
  const result = await runner.runQualification({
    env: LIVE_ENV,
    evidencePath: join(
      REPOSITORY_ROOT,
      ".chatgpt",
      "forbidden-runner-evidence.json",
    ),
    runAdb: () => {
      adbCalls += 1;
      throw new Error("ADB must not run for an unsafe evidence path.");
    },
    writeEvidence: (event) => events.push(event),
  });

  assert.equal(result, 2);
  assert.equal(adbCalls, 0);
  assert.deepEqual(events, [
    {
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: "evidence_destination_unsafe",
    },
  ]);
});

test("durably records mocked ADB preflight failure without raw output", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const result = await runner.runQualification({
      env: LIVE_ENV,
      runAdb: mockedPreflightAdb("get-state"),
      evidencePath,
    });

    assert.equal(result, 2);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "blocked");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.failure_category, "preflight");
    assert.deepEqual(evidence.preflight_outcomes, [
      { check: "state", outcome: "blocked" },
    ]);
    assert.equal(evidence.termination_state, "not_started");
    assert.equal(evidence.cleanup_state, "not_started");
  });
});

test("durably records mocked command spawn failure as blocked and not run", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const result = await runner.runQualification({
      env: LIVE_ENV,
      runAdb: mockedPreflightAdb(),
      runCommand: async () => ({
        status: null,
        signal: null,
        outcome: "spawn_failed",
        stdout: "",
        stderr: `serial=emulator-5554 avd=${EXPECTED_AVD} token=private-test-token`,
      }),
      evidencePath,
    });

    assert.equal(result, 2);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "blocked");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.execution, "NOT_RUN");
    assert.equal(evidence.failure_category, "spawn");
    assert.equal(evidence.termination_state, "not_started");
    assert.equal(evidence.cleanup_state, "not_started");
    assert.equal(
      evidence.preflight_outcomes.every(({ outcome }) => outcome === "passed"),
      true,
    );
  });
});

test("durably persists sanitized success only after the exact live test passes", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const result = await runner.runQualification({
      env: LIVE_ENV,
      runAdb: mockedPreflightAdb(),
      runCommand: async () => ({
        status: 0,
        outcome: "completed",
        stdout:
          "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
        stderr: `serial=emulator-5554 avd=${EXPECTED_AVD} token=private-test-token`,
      }),
      evidencePath,
    });

    assert.equal(result, 0);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "passed");
    assert.equal(evidence.live_status, "passed");
    assert.equal(evidence.execution, "PASSED");
    assert.equal(evidence.reason, "live_qualification_passed");
    assert.equal(evidence.failure_category, "none");
    assert.equal(evidence.termination_state, "confirmed");
    assert.equal(evidence.cleanup_state, "confirmed");
    assert.equal(
      evidence.preflight_outcomes.every(({ outcome }) => outcome === "passed"),
      true,
    );
  });
});

test("durably records verified process-tree termination as unverified", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const result = await runner.runQualification({
      env: LIVE_ENV,
      runAdb: mockedPreflightAdb(),
      runCommand: async () => {
        const error = new Error(
          `serial=emulator-5554 avd=${EXPECTED_AVD} token=private-test-token`,
        );
        error.code = "PROCESS_TREE_TERMINATION_UNVERIFIED";
        error.outcome = "unverified";
        error.liveStatus = "unverified";
        error.termination =
          "process_group_disappearance_and_child_close_confirmed";
        throw error;
      },
      evidencePath,
    });

    assert.equal(result, 2);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "blocked");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.execution, "INTERRUPTED");
    assert.equal(evidence.reason, "process_tree_termination_unverified");
    assert.equal(evidence.failure_category, "termination");
    assert.equal(evidence.termination_state, "confirmed");
    assert.equal(evidence.cleanup_state, "confirmed");
    assert.equal(
      evidence.preflight_outcomes.every(({ outcome }) => outcome === "passed"),
      true,
    );
  });
});

test("evidence persistence failure rejects instead of returning qualification pass", async () => {
  await withEvidenceDirectory(async (directory) => {
    const obstruction = join(directory, "not-a-directory");
    await writeFile(obstruction, "occupied");
    let commandRan = false;

    await assert.rejects(
      runner.runQualification({
        env: LIVE_ENV,
        runAdb: mockedPreflightAdb(),
        runCommand: async () => {
          commandRan = true;
          return {
            status: 0,
            outcome: "completed",
            stdout:
              "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
            stderr: "",
          };
        },
        evidencePath: join(obstruction, "runner-evidence.json"),
      }),
      (error) =>
        error.code === "RUNNER_EVIDENCE_WRITE_FAILED" &&
        error.message === "Runner evidence could not be durably written.",
    );
    assert.equal(commandRan, true);
  });
});

test("evidence write failure after passing live test rejects with its failure code", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const evidenceWriteError = Object.assign(
      new Error("Injected evidence write failure."),
      { code: "RUNNER_EVIDENCE_WRITE_FAILED" },
    );
    let liveTestRan = false;
    let attemptedEvidence;

    await assert.rejects(
      runner.runQualification({
        env: LIVE_ENV,
        runAdb: mockedPreflightAdb(),
        runCommand: async () => {
          liveTestRan = true;
          return {
            status: 0,
            outcome: "completed",
            stdout:
              "test recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification ... ok",
            stderr: "",
          };
        },
        writeEvidence: async (event) => {
          attemptedEvidence = event;
          throw evidenceWriteError;
        },
        evidencePath,
      }),
      (error) => error.code === "RUNNER_EVIDENCE_WRITE_FAILED",
    );

    assert.equal(liveTestRan, true);
    assert.equal(attemptedEvidence.status, "passed");
    assert.equal(attemptedEvidence.live_status, "passed");
  });
});

test("relative runner invocation blocks before ADB or network and persists evidence", async () => {
  await withEvidenceDirectory(async (directory) => {
    const evidencePath = join(directory, "runner-evidence.json");
    const operationMarker = join(directory, "unexpected-operation.txt");
    const relativeArgvPreload = join(directory, "preserve-relative-argv.cjs");
    await writeFile(
      relativeArgvPreload,
      `process.argv[1] = ${JSON.stringify(RELATIVE_RUNNER_PATH)};
const fs = require("node:fs");
const childProcess = require("node:child_process");
const recordUnexpectedOperation = (name) => (...args) => {
  fs.appendFileSync(process.env.ARMSX1_OPERATION_MARKER, name + "\\n");
  throw new Error("Unexpected subprocess in runner safety test.");
};
childProcess.spawn = recordUnexpectedOperation("spawn");
childProcess.spawnSync = recordUnexpectedOperation("spawnSync");
require("node:module").syncBuiltinESMExports();
globalThis.fetch = recordUnexpectedOperation("fetch");
`,
    );

    const childEnvironment = { ...process.env };
    delete childEnvironment.EMUCHEF_RUN_ARMSX1_EMULATOR;
    delete childEnvironment.EMUCHEF_ARMSX1_AVD_SERIAL;
    delete childEnvironment.EMUCHEF_ARMSX1_DEDICATED_AVD;
    delete childEnvironment.EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST;
    childEnvironment.ARMSX1_OPERATION_MARKER = operationMarker;
    childEnvironment.EMUCHEF_ARMSX1_RUNNER_EVIDENCE_PATH = evidencePath;

    const child = spawnSync(
      process.execPath,
      ["--require", relativeArgvPreload, RELATIVE_RUNNER_PATH],
      {
        cwd: REPOSITORY_ROOT,
        encoding: "utf8",
        env: childEnvironment,
        timeout: 10_000,
      },
    );

    assert.ifError(child.error);
    assert.equal(child.status, 2, child.stderr);
    const { evidence } = await readSanitizedEvidence(evidencePath);
    assert.equal(evidence.status, "blocked");
    assert.equal(evidence.live_status, "unverified");
    assert.equal(evidence.execution, "NOT_RUN");
    assert.equal(evidence.reason, "explicit_opt_in_missing");
    assert.equal(evidence.failure_category, "invocation");
    assert.deepEqual(evidence.preflight_outcomes, [
      { check: "preflight", outcome: "not_run" },
    ]);
    await assert.rejects(
      readFile(operationMarker),
      (error) => error.code === "ENOENT",
    );
  });
});

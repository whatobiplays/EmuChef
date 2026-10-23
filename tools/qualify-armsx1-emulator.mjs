#!/usr/bin/env node
import { randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, open, realpath, rename, unlink } from "node:fs/promises";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REQUIRED_AVD = "EmuChef-ARMSX1-Android16-ARM64";
const REQUIRED_DIAGNOSTIC_SERIAL = "emulator-5554";
const ALLOWED_PACKAGES = ["com.nanodata.armsx"];
const LIVE_TEST_NAME =
  "recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification";
const DIAGNOSTIC_TEST_NAME =
  "recipe_qualification_armsx1_emulator_tests::armsx1_read_only_preflight_diagnostic";
const ANDROID_PROPERTIES = [
  ["state", ["get-state"]],
  ["bootCompleted", ["shell", "getprop", "sys.boot_completed"]],
  ["apiLevel", ["shell", "getprop", "ro.build.version.sdk"]],
  ["androidRelease", ["shell", "getprop", "ro.build.version.release"]],
  ["abi", ["shell", "getprop", "ro.product.cpu.abi"]],
  ["brand", ["shell", "getprop", "ro.product.brand"]],
  ["manufacturer", ["shell", "getprop", "ro.product.manufacturer"]],
  ["model", ["shell", "getprop", "ro.product.model"]],
  ["qemu", ["shell", "getprop", "ro.kernel.qemu"]],
  ["uid", ["shell", "id", "-u"]],
];
const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const DEFAULT_RUNNER_EVIDENCE_PATH = resolve(
  REPOSITORY_ROOT,
  "docs/testing/recipe-qualification/armsx1/runner-evidence.json",
);
export const DEFAULT_DIAGNOSTIC_RUNNER_EVIDENCE_PATH = resolve(
  REPOSITORY_ROOT,
  "docs/testing/recipe-qualification/armsx1/preflight-diagnostic-runner-evidence.json",
);
const APPROVED_EVIDENCE_DIRECTORY = resolve(
  REPOSITORY_ROOT,
  "docs/testing/recipe-qualification/armsx1",
);
const SAFE_REASONS = new Set([
  "explicit_opt_in_missing",
  "exact_emulator_serial_missing",
  "exact_emulator_serial_invalid",
  "dedicated_avd_attestation_missing",
  "package_allowlist_mismatch",
  "designated_avd_name_missing",
  "designated_avd_name_mismatch",
  "adb_preflight_failed",
  "emulator_profile_mismatch",
  "avd_name_response_invalid",
  "avd_name_mismatch",
  "avd_name_unavailable",
  "avd_name_property_unavailable",
  "avd_name_property_invalid",
  "process_tree_termination_unverified",
  "live_runner_timed_out",
  "live_runner_interrupted",
  "live_runner_unavailable",
  "live_test_result_ambiguous",
  "live_test_not_executed",
  "live_qualification_failed",
  "live_qualification_passed",
  "preflight_diagnostic_completed",
  "preflight_diagnostic_not_completed",
  "evidence_destination_unsafe",
  "runner_internal_failure",
]);
const PREFLIGHT_CHECK_NAMES = new Set([
  ...ANDROID_PROPERTIES.map(([name]) => name),
  "emulator_profile",
  "avd_identity",
  "preflight",
]);
const PREFLIGHT_OUTCOMES = new Set(["passed", "blocked", "not_run"]);
const EXECUTION_STATES = new Set([
  "NOT_RUN",
  "INTERRUPTED",
  "FAILED",
  "TIMED_OUT",
  "PASSED",
  "DIAGNOSTIC_ONLY",
]);
const DIAGNOSTIC_FAILURE_CODES = new Set([
  "invocation_opt_in_missing",
  "invocation_serial_missing",
  "invocation_serial_invalid",
  "invocation_attestation_missing",
  "invocation_package_allowlist_mismatch",
  "invocation_avd_name_missing",
  "invocation_avd_name_mismatch",
]);
const EVIDENCE_STATES = new Set([
  "not_started",
  "not_required",
  "confirmed",
  "unverified",
]);

/** Validate operator intent before invoking ADB or a network-capable test. */
export function validateInvocation(input, hooks = {}) {
  const fail = (reason, failureCode = reason) => {
    hooks.record?.({ status: "blocked", liveStatus: "unverified", reason });
    return {
      ok: false,
      status: "blocked",
      liveStatus: "unverified",
      reason,
      failureCode,
    };
  };
  if (input.optIn !== true)
    return fail("explicit_opt_in_missing", "invocation_opt_in_missing");
  if (!input.serial)
    return fail("exact_emulator_serial_missing", "invocation_serial_missing");
  if (
    !/^emulator-[0-9]+$/.test(input.serial) ||
    (hooks.requiredSerial !== undefined &&
      input.serial !== hooks.requiredSerial)
  )
    return fail("exact_emulator_serial_invalid", "invocation_serial_invalid");
  if (typeof input.avdName !== "string" || input.avdName.length === 0)
    return fail("designated_avd_name_missing", "invocation_avd_name_missing");
  if (input.avdName !== REQUIRED_AVD)
    return fail("designated_avd_name_mismatch", "invocation_avd_name_mismatch");
  if (input.dedicatedAttestation !== true)
    return fail(
      "dedicated_avd_attestation_missing",
      "invocation_attestation_missing",
    );
  if (
    !Array.isArray(input.packageAllowlist) ||
    input.packageAllowlist.length !== 1 ||
    input.packageAllowlist[0] !== ALLOWED_PACKAGES[0]
  ) {
    return fail(
      "package_allowlist_mismatch",
      "invocation_package_allowlist_mismatch",
    );
  }
  return {
    ok: true,
    status: "ready_for_preflight",
    target: { serial: input.serial, packages: ALLOWED_PACKAGES },
  };
}

/** Parse the emulator console's name and success response without accepting partial output. */
export function parseAvdNameResponse(result) {
  const stdout = typeof result?.stdout === "string" ? result.stdout : "";
  const stderr = typeof result?.stderr === "string" ? result.stderr : "";
  if (stdout.length === 0 && stderr.length === 0)
    return { state: "unavailable" };
  if (result?.error || result?.status !== 0 || stderr.length > 0)
    return { state: "invalid" };

  const match = /^([^\r\n]+)\r?\nOK\r?\n?$/.exec(stdout);
  if (!match) return { state: "invalid" };

  const name = match[1];
  if (
    name.trim() !== name ||
    /[\u0000-\u001f\u007f]/.test(name) ||
    /^(?:KO|ERROR)(?:\s|:|$)/i.test(name)
  ) {
    return { state: "invalid" };
  }
  return { state: "valid", name };
}

/** Check the fixed Google ARM64 Android 16 emulator profile using read-only observations. */
export function validateEmulatorProfile(profile) {
  const normalized = {
    brand: String(profile?.brand ?? "").toLowerCase(),
    manufacturer: String(profile?.manufacturer ?? "").toLowerCase(),
  };
  if (
    profile?.state !== "device" ||
    profile?.bootCompleted !== "1" ||
    profile?.apiLevel !== "36" ||
    profile?.androidRelease !== "16" ||
    profile?.abi !== "arm64-v8a" ||
    normalized.brand !== "google" ||
    !normalized.manufacturer.includes("google") ||
    profile?.model !== "sdk_gphone64_arm64" ||
    profile?.qemu !== "1" ||
    profile?.uid !== "2000"
  ) {
    return { ok: false, reason: "emulator_profile_mismatch" };
  }
  return { ok: true };
}

/** Resolve the designated AVD from the console or a trusted boot property fallback. */
export function resolveAvdName(runCommand, profile) {
  let consoleResult;
  try {
    consoleResult = runCommand(["emu", "avd", "name"]);
  } catch {
    consoleResult = undefined;
  }
  const parsed = parseAvdNameResponse(consoleResult);
  if (parsed.state === "invalid")
    return { ok: false, reason: "avd_name_response_invalid" };
  if (parsed.state === "valid") {
    return parsed.name === REQUIRED_AVD
      ? { ok: true, name: parsed.name, source: "emulator_console" }
      : { ok: false, reason: "avd_name_mismatch" };
  }

  if (!validateEmulatorProfile(profile).ok)
    return { ok: false, reason: "avd_name_unavailable" };
  let propertyResult;
  try {
    propertyResult = runCommand(["shell", "getprop", "ro.boot.qemu.avd_name"]);
  } catch {
    return { ok: false, reason: "avd_name_property_unavailable" };
  }
  const stdout =
    typeof propertyResult?.stdout === "string" ? propertyResult.stdout : "";
  const stderr =
    typeof propertyResult?.stderr === "string" ? propertyResult.stderr : "";
  if (
    propertyResult?.error ||
    propertyResult?.status !== 0 ||
    stderr.length > 0
  ) {
    return { ok: false, reason: "avd_name_property_unavailable" };
  }
  const match = /^([^\r\n]+)\r?\n?$/.exec(stdout);
  if (!match) return { ok: false, reason: "avd_name_property_invalid" };
  return match[1] === REQUIRED_AVD
    ? { ok: true, name: match[1], source: "validated_emulator_property" }
    : { ok: false, reason: "avd_name_mismatch" };
}

function pathIsWithin(parent, candidate) {
  const childPath = relative(parent, candidate);
  return (
    childPath === "" ||
    (childPath !== ".." &&
      !childPath.startsWith(".." + sep) &&
      !isAbsolute(childPath))
  );
}

function hasChatgptPathComponent(path) {
  return path
    .split(sep)
    .some((component) => component.toLowerCase() === ".chatgpt");
}

async function canonicalizePathWithMissingTail(path) {
  let parent = dirname(path);
  const tail = [basename(path)];
  while (true) {
    try {
      return resolve(await realpath(parent), ...tail);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      const component = basename(parent);
      if (!component || parent === dirname(parent)) throw error;
      tail.unshift(component);
      parent = dirname(parent);
    }
  }
}

/**
 * Resolve an evidence file only inside the approved qualification directory or
 * outside the repository. Existing parent symlinks are resolved before admission.
 */
export async function resolveEvidenceDestination(destination) {
  if (typeof destination !== "string" || !isAbsolute(destination)) {
    throw new Error("Evidence destination must be an absolute path.");
  }
  const requestedPath = resolve(destination);
  if (hasChatgptPathComponent(requestedPath)) {
    throw new Error("Evidence destination is not permitted.");
  }

  const [actualPath, repositoryRoot, approvedEvidencePath] = await Promise.all([
    canonicalizePathWithMissingTail(requestedPath),
    realpath(REPOSITORY_ROOT),
    canonicalizePathWithMissingTail(
      join(APPROVED_EVIDENCE_DIRECTORY, ".armsx1-evidence-path"),
    ),
  ]);
  const approvedDirectory = dirname(approvedEvidencePath);
  if (
    hasChatgptPathComponent(actualPath) ||
    (pathIsWithin(repositoryRoot, actualPath) &&
      !pathIsWithin(approvedDirectory, actualPath))
  ) {
    throw new Error("Evidence destination is not permitted.");
  }
  return actualPath;
}

/** Run mocked or real read-only ADB checks and return only sanitized observations. */
export function preflightEmulator(runCommand) {
  const profile = {};
  for (const [key, args] of ANDROID_PROPERTIES) {
    let result;
    try {
      result = runCommand(args);
    } catch {
      return {
        ok: false,
        status: "blocked",
        liveStatus: "unverified",
        reason: "adb_preflight_failed",
        check: key,
      };
    }
    if (
      result?.error ||
      result?.status !== 0 ||
      typeof result.stdout !== "string"
    ) {
      return {
        ok: false,
        status: "blocked",
        liveStatus: "unverified",
        reason: "adb_preflight_failed",
        check: key,
      };
    }
    profile[key] = result.stdout.trim();
  }

  const validation = validateEmulatorProfile(profile);
  if (!validation.ok) {
    return {
      ok: false,
      status: "blocked",
      liveStatus: "unverified",
      reason: validation.reason,
    };
  }
  const avd = resolveAvdName(runCommand, profile);
  if (!avd.ok) {
    return {
      ok: false,
      status: "blocked",
      liveStatus: "unverified",
      reason: avd.reason,
    };
  }
  return {
    ok: true,
    status: "ready_for_live_test",
    liveStatus: "unverified",
    avdName: avd.name,
    avdNameSource: avd.source,
    profile: {
      apiLevel: profile.apiLevel,
      androidRelease: profile.androidRelease,
      abi: profile.abi,
    },
  };
}

function runAdb(serial, args) {
  return spawnSync("adb", ["-s", serial, ...args], {
    encoding: "utf8",
    timeout: 10_000,
    maxBuffer: 256 * 1024,
  });
}

function runnerPreflightOutcomes(preflight) {
  if (!preflight) {
    return [{ check: "preflight", outcome: "not_run" }];
  }

  const propertyNames = ANDROID_PROPERTIES.map(([name]) => name);
  if (preflight.ok) {
    return [
      ...propertyNames.map((check) => ({ check, outcome: "passed" })),
      { check: "emulator_profile", outcome: "passed" },
      { check: "avd_identity", outcome: "passed" },
    ];
  }

  const outcomes = [];
  const failedPropertyIndex = propertyNames.indexOf(preflight.check);
  if (failedPropertyIndex >= 0) {
    for (let index = 0; index < failedPropertyIndex; index += 1) {
      outcomes.push({ check: propertyNames[index], outcome: "passed" });
    }
    outcomes.push({
      check: propertyNames[failedPropertyIndex],
      outcome: "blocked",
    });
    return outcomes;
  }

  outcomes.push(
    ...propertyNames.map((check) => ({ check, outcome: "passed" })),
  );
  if (preflight.reason === "emulator_profile_mismatch") {
    outcomes.push({ check: "emulator_profile", outcome: "blocked" });
  } else {
    outcomes.push({ check: "emulator_profile", outcome: "passed" });
    outcomes.push({ check: "avd_identity", outcome: "blocked" });
  }
  return outcomes;
}

function classifyRunnerFailure(reason, status) {
  if (reason.startsWith("preflight_diagnostic_")) return "preflight_diagnostic";
  if (
    reason === "explicit_opt_in_missing" ||
    reason === "exact_emulator_serial_missing" ||
    reason === "exact_emulator_serial_invalid" ||
    reason === "dedicated_avd_attestation_missing" ||
    reason === "package_allowlist_mismatch" ||
    reason === "designated_avd_name_missing" ||
    reason === "designated_avd_name_mismatch"
  ) {
    return "invocation";
  }
  if (status === "passed") return "none";
  if (
    [
      "explicit_opt_in_missing",
      "exact_emulator_serial_missing",
      "dedicated_avd_attestation_missing",
      "package_allowlist_mismatch",
    ].includes(reason)
  ) {
    return "invocation";
  }
  if (
    reason === "adb_preflight_failed" ||
    reason === "emulator_profile_mismatch" ||
    reason?.startsWith("avd_")
  ) {
    return "preflight";
  }
  if (reason === "live_runner_unavailable") return "spawn";
  if (reason === "evidence_destination_unsafe") return "configuration";
  if (
    reason === "process_tree_termination_unverified" ||
    reason === "live_runner_timed_out" ||
    reason === "live_runner_interrupted"
  ) {
    return "termination";
  }
  if (
    reason === "live_test_result_ambiguous" ||
    reason === "live_test_not_executed" ||
    reason === "live_qualification_failed"
  ) {
    return "live_qualification";
  }
  return "internal";
}

function sanitizeRunnerEvidence(event) {
  const status = ["passed", "failed", "blocked", "completed"].includes(
    event.status,
  )
    ? event.status
    : "blocked";
  const reason = SAFE_REASONS.has(event.reason)
    ? event.reason
    : "runner_internal_failure";
  const execution = EXECUTION_STATES.has(event.execution)
    ? event.execution
    : "FAILED";
  const preflightOutcomes = Array.isArray(event.preflight_outcomes)
    ? event.preflight_outcomes
        .filter(
          (item) =>
            PREFLIGHT_CHECK_NAMES.has(item?.check) &&
            PREFLIGHT_OUTCOMES.has(item?.outcome),
        )
        .slice(0, PREFLIGHT_CHECK_NAMES.size)
    : [{ check: "preflight", outcome: "not_run" }];
  const terminationState = EVIDENCE_STATES.has(event.termination_state)
    ? event.termination_state
    : [
          "process_group_disappearance_and_child_close_confirmed",
          "process_tree_terminated_and_child_reaped",
        ].includes(event.termination)
      ? "confirmed"
      : execution === "NOT_RUN"
        ? "not_started"
        : "unverified";
  const cleanupState = EVIDENCE_STATES.has(event.cleanup_state)
    ? event.cleanup_state
    : [
          "process_group_disappearance_and_child_close_confirmed",
          "process_tree_terminated_and_child_reaped",
        ].includes(event.termination)
      ? "confirmed"
      : execution === "NOT_RUN"
        ? "not_started"
        : "unverified";

  const evidence = {
    schema_version: 1,
    recorded_at: new Date().toISOString(),
    status,
    live_status:
      status === "passed" && event.live_status === "passed"
        ? "passed"
        : "unverified",
    execution,
    reason,
    failure_category: classifyRunnerFailure(reason, status),
    preflight_outcomes: preflightOutcomes,
    termination_state: terminationState,
    cleanup_state: cleanupState,
  };
  if (DIAGNOSTIC_FAILURE_CODES.has(event.failure_code)) {
    evidence.failure_code = event.failure_code;
  }
  if (event.diagnostic_result === "recorded") {
    evidence.diagnostic_result = "recorded";
  }
  return evidence;
}

async function persistRunnerEvidence(event, requestedEvidencePath) {
  const evidence = event;
  const evidencePath = await resolveEvidenceDestination(requestedEvidencePath);
  const directory = dirname(evidencePath);
  const temporaryPath = join(
    directory,
    `.${basename(evidencePath)}.${process.pid}.${randomUUID()}.tmp`,
  );
  let file;
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
    file = await open(temporaryPath, "wx", 0o600);
    await file.writeFile(`${JSON.stringify(evidence)}\n`, "utf8");
    await file.sync();
    await file.close();
    file = undefined;
    await rename(temporaryPath, evidencePath);
    const directoryHandle = await open(directory, "r");
    try {
      await directoryHandle.sync();
    } finally {
      await directoryHandle.close();
    }
  } catch {
    await file?.close().catch(() => {});
    await unlink(temporaryPath).catch(() => {});
    const error = new Error("Runner evidence could not be durably written.");
    error.code = "RUNNER_EVIDENCE_WRITE_FAILED";
    throw error;
  }
}

async function outputEvidence(event, evidencePath) {
  const evidence = sanitizeRunnerEvidence(event);
  await persistRunnerEvidence(evidence, evidencePath);
  process.stdout.write(`${JSON.stringify(evidence)}\n`);
}

/**
 * Run a child process in a dedicated POSIX process group and resolve only after the child closes and the group disappears.
 * Reject with an unverified termination error when process-group liveness or cleanup cannot be confirmed.
 * Windows is rejected before spawn because this runner cannot safely verify process-tree termination there.
 */
export function runManagedCommand(command, args, options = {}) {
  const platform = options.platform ?? process.platform;
  if (platform === "win32") {
    return Promise.resolve({
      status: null,
      signal: null,
      outcome: "spawn_failed",
      liveStatus: "unverified",
      stdout: "",
      stderr:
        "Safe process-tree management is unsupported on Windows; the command was not started.",
    });
  }

  const timeoutMs = Number.isFinite(options.timeoutMs)
    ? options.timeoutMs
    : 30 * 60 * 1000;
  const terminationGraceMs =
    Number.isFinite(options.terminationGraceMs) &&
    options.terminationGraceMs >= 0
      ? options.terminationGraceMs
      : 1_000;
  const processGroupPollIntervalMs =
    Number.isFinite(options.processGroupPollIntervalMs) &&
    options.processGroupPollIntervalMs > 0
      ? options.processGroupPollIntervalMs
      : 25;
  const verificationTimeoutMs =
    Number.isFinite(options.verificationTimeoutMs) &&
    options.verificationTimeoutMs > 0
      ? options.verificationTimeoutMs
      : 5_000;
  const spawnChild = options.spawnProcess ?? spawn;
  const signalProcessTree =
    options.signalProcessTree ??
    ((_target, signal, processGroupId) => {
      process.kill(-processGroupId, signal);
    });
  const isProcessGroupAlive =
    options.isProcessGroupAlive ??
    ((processGroupId) => {
      try {
        process.kill(-processGroupId, 0);
        return true;
      } catch (error) {
        if (error.code === "ESRCH") return false;
        throw error;
      }
    });

  let child;
  try {
    child = spawnChild(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    });
  } catch (error) {
    return Promise.resolve({
      status: null,
      signal: null,
      outcome: "spawn_failed",
      stdout: "",
      stderr: error.message,
    });
  }

  const processGroupId = child.pid;

  return new Promise((resolve, reject) => {
    const chunks = { stdout: [], stderr: [] };
    let outputBytes = 0;
    let closeResult;
    let termination;
    let terminationSignalNeedsRetry = false;
    let forceKillRequested = false;
    let spawnFailed = false;
    let settled = false;
    let processGroupGone = false;
    let timeout;
    let forceKillTimer;
    let groupPollTimer;
    let killVerificationTimer;
    let unverifiedFailure;
    const maxCapturedOutput = 1024 * 1024;

    const capture = (key, chunk) => {
      if (outputBytes >= maxCapturedOutput) return;
      const remaining = maxCapturedOutput - outputBytes;
      const retained = chunk.subarray(0, remaining);
      chunks[key].push(retained);
      outputBytes += retained.length;
    };
    const captureStdout = (chunk) => capture("stdout", chunk);
    const captureStderr = (chunk) => capture("stderr", chunk);

    const removeSignalListeners = () => {
      process.removeListener("SIGINT", interrupt);
      process.removeListener("SIGTERM", terminate);
    };
    const clearTimers = () => {
      clearTimeout(timeout);
      clearTimeout(forceKillTimer);
      clearTimeout(groupPollTimer);
      clearTimeout(killVerificationTimer);
      timeout = undefined;
      forceKillTimer = undefined;
      groupPollTimer = undefined;
      killVerificationTimer = undefined;
    };
    const cleanupChildListeners = () => {
      child.stdout.removeListener("data", captureStdout);
      child.stderr.removeListener("data", captureStderr);
      child.removeListener("error", onError);
      child.removeListener("exit", onExit);
      child.removeListener("close", onClose);
      child.stdout.resume?.();
      child.stderr.resume?.();
    };
    const cleanup = () => {
      clearTimers();
      removeSignalListeners();
    };
    const recordUnverified = (message) => {
      if (unverifiedFailure) return;
      const error = new Error(message);
      error.code = "PROCESS_TREE_TERMINATION_UNVERIFIED";
      error.outcome = "unverified";
      error.liveStatus = "unverified";
      unverifiedFailure = error;
    };

    const finish = () => {
      if (settled || !closeResult || !processGroupGone) return;
      settled = true;
      cleanup();
      cleanupChildListeners();
      if (unverifiedFailure) {
        unverifiedFailure.termination =
          "process_group_disappearance_and_child_close_confirmed";
        reject(unverifiedFailure);
        return;
      }
      const stdout = Buffer.concat(chunks.stdout).toString("utf8");
      const stderr = Buffer.concat(chunks.stderr).toString("utf8");
      resolve({
        status: closeResult.code,
        signal: closeResult.signal,
        outcome:
          termination ??
          (spawnFailed
            ? "spawn_failed"
            : closeResult.code === 0
              ? "completed"
              : "failed"),
        stdout,
        stderr,
      });
    };

    const markProcessGroupGone = () => {
      if (processGroupGone) return;
      processGroupGone = true;
      clearTimeout(timeout);
      clearTimeout(forceKillTimer);
      clearTimeout(groupPollTimer);
      clearTimeout(killVerificationTimer);
      timeout = undefined;
      forceKillTimer = undefined;
      groupPollTimer = undefined;
      killVerificationTimer = undefined;
      finish();
    };

    const inspectProcessGroup = () => {
      if (!Number.isInteger(processGroupId) || processGroupId <= 0) {
        return spawnFailed ? { state: "gone" } : { state: "unknown" };
      }
      try {
        const alive = isProcessGroupAlive(processGroupId);
        if (alive === true) return { state: "alive" };
        if (alive === false) return { state: "gone" };
        return { state: "unknown" };
      } catch (error) {
        return { state: "unknown", error };
      }
    };

    const observeProcessGroup = () => {
      if (settled || processGroupGone) return "gone";
      const observation = inspectProcessGroup();
      if (observation.state === "gone") {
        markProcessGroupGone();
        return "gone";
      }
      if (observation.state === "unknown") {
        recordUnverified(
          "Unable to verify process-group liveness; supervision continues until the group and child are confirmed stopped.",
        );
        scheduleProcessGroupPoll();
        return "unknown";
      }
      return "alive";
    };

    const scheduleProcessGroupPoll = () => {
      if (settled || processGroupGone || groupPollTimer !== undefined) return;
      groupPollTimer = setTimeout(() => {
        groupPollTimer = undefined;
        const state = observeProcessGroup();
        if (state === "gone") return;
        if (termination && terminationSignalNeedsRetry) {
          signalGroup(forceKillRequested ? "SIGKILL" : "SIGTERM");
        }
        scheduleProcessGroupPoll();
      }, processGroupPollIntervalMs);
    };

    const signalGroup = (signal) => {
      try {
        signalProcessTree(child, signal, processGroupId);
      } catch (error) {
        if (error.code === "ESRCH") {
          markProcessGroupGone();
          return false;
        }
        recordUnverified(
          "Unable to signal the managed process group; supervision continues until it is confirmed stopped.",
        );
        terminationSignalNeedsRetry = true;
        scheduleProcessGroupPoll();
        return false;
      }
      terminationSignalNeedsRetry = false;
      return true;
    };

    const escalate = () => {
      forceKillTimer = undefined;
      if (settled || processGroupGone || !termination) return;
      forceKillRequested = true;
      if (observeProcessGroup() === "gone") return;
      if (!signalGroup("SIGKILL") || processGroupGone) return;
      killVerificationTimer = setTimeout(() => {
        killVerificationTimer = undefined;
        if (observeProcessGroup() === "alive") {
          recordUnverified(
            "SIGKILL did not promptly confirm process-group disappearance; supervision continues.",
          );
        }
        if (!processGroupGone) scheduleProcessGroupPoll();
      }, verificationTimeoutMs);
      scheduleProcessGroupPoll();
    };

    const requestTermination = (reason) => {
      if (settled || processGroupGone || termination) return;
      termination = reason;
      if (observeProcessGroup() === "gone") return;
      signalGroup("SIGTERM");
      if (processGroupGone) return;
      forceKillTimer = setTimeout(escalate, terminationGraceMs);
      scheduleProcessGroupPoll();
    };

    const interrupt = () => requestTermination("interrupted");
    const terminate = () => requestTermination("interrupted");

    const onExit = () => {
      if (settled || processGroupGone) return;
      const state = observeProcessGroup();
      if (state !== "gone") {
        if (!termination) requestTermination("process_group_leaked");
        else scheduleProcessGroupPoll();
      }
    };
    const onError = () => {
      if (settled) return;
      spawnFailed = true;
      if (!Number.isInteger(processGroupId) || processGroupId <= 0) {
        closeResult = { code: null, signal: null };
        markProcessGroupGone();
        finish();
        return;
      }
      recordUnverified(
        "The managed child reported an error; supervision continues until the group and child are confirmed stopped.",
      );
      requestTermination("process_group_leaked");
    };
    const onClose = (code, signal) => {
      if (settled) return;
      closeResult = { code, signal };
      if (!processGroupGone) {
        const state = observeProcessGroup();
        if (state !== "gone") {
          if (!termination) requestTermination("process_group_leaked");
          else scheduleProcessGroupPoll();
        }
      }
      finish();
    };

    child.stdout.on("data", captureStdout);
    child.stderr.on("data", captureStderr);
    child.on("error", onError);
    child.on("exit", onExit);
    child.on("close", onClose);
    process.on("SIGINT", interrupt);
    process.on("SIGTERM", terminate);
    timeout = setTimeout(() => requestTermination("timed_out"), timeoutMs);
  });
}

/** Validate inputs, perform read-only preflight, and require evidence that the ignored test ran. */
export async function runQualification(options = {}) {
  const env = options.env ?? process.env;
  const requestedEvidencePath =
    options.evidencePath ??
    env.EMUCHEF_ARMSX1_RUNNER_EVIDENCE_PATH ??
    DEFAULT_RUNNER_EVIDENCE_PATH;
  let evidencePath;
  let unsafeEvidenceDestination = false;
  try {
    evidencePath = await resolveEvidenceDestination(requestedEvidencePath);
  } catch {
    unsafeEvidenceDestination = true;
    evidencePath = await resolveEvidenceDestination(
      DEFAULT_RUNNER_EVIDENCE_PATH,
    );
  }
  const writeEvidence = options.writeEvidence;
  let preflight;
  const recordEvidence = async (
    event,
    { terminationState, cleanupState } = {},
  ) => {
    if (writeEvidence) return writeEvidence(event);
    return outputEvidence(
      {
        ...event,
        preflight_outcomes: runnerPreflightOutcomes(preflight),
        ...(terminationState ? { termination_state: terminationState } : {}),
        ...(cleanupState ? { cleanup_state: cleanupState } : {}),
      },
      evidencePath,
    );
  };
  const runAdbCommand = options.runAdb ?? runAdb;
  if (unsafeEvidenceDestination) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: "evidence_destination_unsafe",
    });
    return 2;
  }
  const invocation = validateInvocation({
    optIn: env.EMUCHEF_RUN_ARMSX1_EMULATOR === "1",
    serial: env.EMUCHEF_ARMSX1_AVD_SERIAL,
    avdName: env.EMUCHEF_ARMSX1_AVD_NAME,
    dedicatedAttestation:
      env.EMUCHEF_ARMSX1_DEDICATED_AVD ===
      "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
    packageAllowlist:
      env.EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST?.split(",").filter(Boolean),
  });
  if (!invocation.ok) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: invocation.reason,
    });
    return 2;
  }

  preflight = preflightEmulator((args) =>
    runAdbCommand(invocation.target.serial, args),
  );
  if (!preflight.ok) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: preflight.reason,
      ...(preflight.check ? { check: preflight.check } : {}),
    });
    return 2;
  }

  const command = options.runCommand ?? runManagedCommand;
  let child;
  try {
    child = await command(
      "cargo",
      [
        "test",
        "--manifest-path",
        "crates/emuchef-rust-backend/Cargo.toml",
        "recipe_qualification_armsx1_emulator_tests::armsx1_live_emulator_qualification",
        "--",
        "--ignored",
        "--exact",
        "--nocapture",
      ],
      { env, timeoutMs: 30 * 60 * 1000 },
    );
  } catch (error) {
    if (
      error?.code === "PROCESS_TREE_TERMINATION_UNVERIFIED" &&
      error.outcome === "unverified" &&
      error.liveStatus === "unverified" &&
      error.termination ===
        "process_group_disappearance_and_child_close_confirmed"
    ) {
      await recordEvidence({
        status: "blocked",
        live_status: "unverified",
        execution: "INTERRUPTED",
        reason: "process_tree_termination_unverified",
        termination: error.termination,
      });
      return 2;
    }
    throw error;
  }
  const testOutput = typeof child.stdout === "string" ? child.stdout : "";
  const liveTestResults = [
    ...testOutput.matchAll(
      new RegExp(`^test ${LIVE_TEST_NAME} \\.\\.\\. (ok|FAILED)$`, "gm"),
    ),
  ];
  if (liveTestResults.length !== 1) {
    const interrupted = child.outcome === "interrupted";
    await recordEvidence(
      {
        status: child.outcome === "spawn_failed" ? "blocked" : "failed",
        live_status: "unverified",
        execution:
          child.outcome === "spawn_failed"
            ? "NOT_RUN"
            : interrupted
              ? "INTERRUPTED"
              : "FAILED",
        reason:
          liveTestResults.length > 1
            ? "live_test_result_ambiguous"
            : child.outcome === "timed_out"
              ? "live_runner_timed_out"
              : interrupted
                ? "live_runner_interrupted"
                : child.outcome === "spawn_failed"
                  ? "live_runner_unavailable"
                  : "live_test_not_executed",
        termination:
          child.outcome === "timed_out" || interrupted
            ? "process_tree_terminated_and_child_reaped"
            : undefined,
      },
      {
        terminationState:
          child.outcome === "spawn_failed" ? "not_started" : "confirmed",
        cleanupState:
          child.outcome === "spawn_failed" ? "not_started" : "confirmed",
      },
    );
    return 2;
  }
  if (
    child.outcome !== "completed" ||
    child.status !== 0 ||
    liveTestResults[0][1] !== "ok"
  ) {
    await recordEvidence(
      {
        status: "failed",
        live_status: "unverified",
        execution:
          child.outcome === "timed_out"
            ? "TIMED_OUT"
            : child.outcome === "interrupted"
              ? "INTERRUPTED"
              : "FAILED",
        reason:
          child.outcome === "timed_out"
            ? "live_runner_timed_out"
            : child.outcome === "interrupted"
              ? "live_runner_interrupted"
              : "live_qualification_failed",
        termination:
          child.outcome === "timed_out" || child.outcome === "interrupted"
            ? "process_tree_terminated_and_child_reaped"
            : undefined,
      },
      { terminationState: "confirmed", cleanupState: "confirmed" },
    );
    return 2;
  }
  await recordEvidence(
    {
      status: "passed",
      live_status: "passed",
      execution: "PASSED",
      reason: "live_qualification_passed",
      termination: "process_group_disappearance_and_child_close_confirmed",
    },
    { terminationState: "confirmed", cleanupState: "confirmed" },
  );
  return 0;
}

/** Dispatch the exact ignored Rust test whose call graph ends after read-only preflight. */
export async function runPreflightDiagnostic(options = {}) {
  const env = options.env ?? process.env;
  const requestedEvidencePath =
    options.evidencePath ??
    env.EMUCHEF_ARMSX1_DIAGNOSTIC_RUNNER_EVIDENCE_PATH ??
    DEFAULT_DIAGNOSTIC_RUNNER_EVIDENCE_PATH;
  let evidencePath;
  let unsafeEvidenceDestination = false;
  try {
    evidencePath = await resolveEvidenceDestination(requestedEvidencePath);
  } catch {
    unsafeEvidenceDestination = true;
    evidencePath = await resolveEvidenceDestination(
      DEFAULT_DIAGNOSTIC_RUNNER_EVIDENCE_PATH,
    );
  }
  const writeEvidence = options.writeEvidence;
  const recordEvidence = async (event) => {
    if (writeEvidence) return writeEvidence(event);
    return outputEvidence(event, evidencePath);
  };
  if (unsafeEvidenceDestination) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: "evidence_destination_unsafe",
    });
    return 2;
  }

  const invocation = validateInvocation(
    {
      optIn: env.EMUCHEF_RUN_ARMSX1_EMULATOR === "1",
      serial: env.EMUCHEF_ARMSX1_AVD_SERIAL,
      avdName: env.EMUCHEF_ARMSX1_AVD_NAME,
      dedicatedAttestation:
        env.EMUCHEF_ARMSX1_DEDICATED_AVD ===
        "I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED",
      packageAllowlist:
        env.EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST?.split(",").filter(Boolean),
    },
    { requiredSerial: REQUIRED_DIAGNOSTIC_SERIAL },
  );
  if (!invocation.ok) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: invocation.reason,
      failure_code: invocation.failureCode,
    });
    return 2;
  }

  let child;
  try {
    const command = options.runCommand ?? runManagedCommand;
    child = await command(
      "cargo",
      [
        "test",
        "--offline",
        "--manifest-path",
        "crates/emuchef-rust-backend/Cargo.toml",
        DIAGNOSTIC_TEST_NAME,
        "--",
        "--ignored",
        "--exact",
        "--nocapture",
      ],
      { env, timeoutMs: 5 * 60 * 1000 },
    );
  } catch {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution: "NOT_RUN",
      reason: "preflight_diagnostic_not_completed",
    });
    return 2;
  }

  const output = typeof child?.stdout === "string" ? child.stdout : "";
  const results = [
    ...output.matchAll(
      new RegExp(`^test ${DIAGNOSTIC_TEST_NAME} \\.\\.\\. (ok|FAILED)$`, "gm"),
    ),
  ];
  if (
    child?.outcome !== "completed" ||
    child.status !== 0 ||
    results.length !== 1 ||
    results[0][1] !== "ok"
  ) {
    await recordEvidence({
      status: "blocked",
      live_status: "unverified",
      execution:
        child?.outcome === "timed_out"
          ? "TIMED_OUT"
          : child?.outcome === "interrupted"
            ? "INTERRUPTED"
            : "FAILED",
      reason: "preflight_diagnostic_not_completed",
    });
    return 2;
  }

  await recordEvidence({
    status: "completed",
    live_status: "unverified",
    execution: "DIAGNOSTIC_ONLY",
    reason: "preflight_diagnostic_completed",
    diagnostic_result: "recorded",
  });
  return 0;
}

const invokedScriptUrl = process.argv[1]
  ? pathToFileURL(resolve(process.argv[1])).href
  : undefined;

if (invokedScriptUrl === import.meta.url) {
  const argumentsAfterScript = process.argv.slice(2);
  const diagnosticMode =
    argumentsAfterScript.length === 1 &&
    argumentsAfterScript[0] === "--diagnose-preflight";
  const selectedRun =
    argumentsAfterScript.length === 0
      ? runQualification
      : diagnosticMode
        ? runPreflightDiagnostic
        : undefined;
  if (!selectedRun) {
    process.stderr.write("Unsupported qualification runner option.\n");
    process.exitCode = 2;
  } else {
    selectedRun()
      .then((exitCode) => {
        process.exitCode = exitCode;
      })
      .catch(async () => {
        try {
          const evidencePath = resolve(
            diagnosticMode
              ? (process.env.EMUCHEF_ARMSX1_DIAGNOSTIC_RUNNER_EVIDENCE_PATH ??
                  DEFAULT_DIAGNOSTIC_RUNNER_EVIDENCE_PATH)
              : (process.env.EMUCHEF_ARMSX1_RUNNER_EVIDENCE_PATH ??
                  DEFAULT_RUNNER_EVIDENCE_PATH),
          );
          await outputEvidence(
            {
              status: "failed",
              live_status: "unverified",
              execution: "FAILED",
              reason: "runner_internal_failure",
              termination_state: "unverified",
              cleanup_state: "unverified",
            },
            evidencePath,
          );
        } catch {
          process.stderr.write(
            "Runner evidence could not be durably written.\n",
          );
        }
        process.exitCode = 2;
      });
  }
}

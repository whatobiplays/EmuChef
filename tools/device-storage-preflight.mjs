#!/usr/bin/env node

/**
 * Prepare reversible device-local storage pressure for reviewed qualification
 * scenarios. All mutations are profile-owned and occur through one exact ADB
 * serial; this utility is not part of production execution or evidence capture.
 */
import { execFileSync } from "node:child_process";
import path from "node:path";
import { pathToFileURL } from "node:url";

const KIB_PER_MIB = 1024;
const MAX_COMMAND_OUTPUT_BYTES = 4 * 1024 * 1024;
const ALLOCATION_ROOT = "/sdcard/Download/EmuChefStoragePreflight";
const DEFAULT_ADB = "adb";
const CHUNK_PATTERN = /^chunk-([0-9]{4,})\.bin$/;
const SAFE_SERIAL_PATTERN = /^[A-Za-z0-9._:-]{1,256}$/;
const SAFE_DEVICE_PATH_PATTERN = /^\/[A-Za-z0-9._/-]+$/;

function fail(message) {
  throw new Error(message);
}

function integer(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    fail(`${label} must be a non-negative safe integer`);
  }
  return value;
}

function nonEmptyString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} must be a non-empty string`);
  }
  return value;
}

function validateSerial(serial) {
  if (typeof serial !== "string" || !SAFE_SERIAL_PATTERN.test(serial)) {
    fail("selected serial has an invalid format");
  }
  return serial;
}

function validateDevicePath(devicePath, label = "device path") {
  if (
    typeof devicePath !== "string"
    || !SAFE_DEVICE_PATH_PATTERN.test(devicePath)
    || devicePath.includes("//")
    || devicePath.split("/").includes("..")
  ) {
    fail(`${label} is not a safe absolute device path`);
  }
  return devicePath;
}

function validateAllocationMutationPath(devicePath) {
  validateDevicePath(devicePath, "allocation mutation path");
  if (!devicePath.startsWith(`${ALLOCATION_ROOT}/`)) {
    fail("allocation mutation path is outside the EmuChef allocation root");
  }
  return devicePath;
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

/** Validate and freeze one immutable storage-preflight profile. */
export function validateProfile(candidate) {
  if (candidate === null || typeof candidate !== "object" || Array.isArray(candidate)) {
    fail("storage profile must be an object");
  }
  const name = nonEmptyString(candidate.name, "profile name");
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name)) {
    fail("profile name has an invalid format");
  }
  const packageName = nonEmptyString(candidate.packageName, "profile package name");
  if (!/^[A-Za-z0-9]+(?:\.[A-Za-z0-9_]+)+$/.test(packageName)) {
    fail("profile package name has an invalid format");
  }
  const qualificationPath = validateDevicePath(
    candidate.qualificationPath,
    "qualification path",
  );
  const allocationParent = validateDevicePath(candidate.allocationParent, "allocation parent");
  if (allocationParent !== "/sdcard/Download") {
    fail("allocation parent must be the approved Downloads directory");
  }
  const allocationPath = validateDevicePath(candidate.allocationPath, "allocation path");
  if (allocationPath !== `${ALLOCATION_ROOT}/${name}`) {
    fail("allocation path must be the exact profile-owned Downloads path");
  }
  const markerName = nonEmptyString(candidate.markerName, "ownership marker name");
  if (markerName !== ".emuchef-storage-preflight-owner") {
    fail("ownership marker name is not approved");
  }
  const markerContents = nonEmptyString(candidate.markerContents, "ownership marker contents");
  if (markerContents !== `emuchef-storage-preflight:v1:${name}`) {
    fail("ownership marker contents do not match the profile identity");
  }
  const minAvailableKib = integer(candidate.minAvailableKib, "profile minimum available KiB");
  const maxAvailableKib = integer(candidate.maxAvailableKib, "profile maximum available KiB");
  const targetAvailableKib = integer(candidate.targetAvailableKib, "profile target available KiB");
  if (minAvailableKib >= maxAvailableKib) {
    fail("profile minimum must be below its maximum");
  }
  if (targetAvailableKib < minAvailableKib || targetAvailableKib > maxAvailableKib) {
    fail("profile target must be inside the accepted available-space window");
  }
  if (!Array.isArray(candidate.chunkTiersKib) || candidate.chunkTiersKib.length === 0) {
    fail("profile chunk tiers must be a non-empty array");
  }
  const chunkTiersKib = candidate.chunkTiersKib.map((value, index) => {
    integer(value, `profile chunk tier ${index}`);
    if (value === 0 || value % KIB_PER_MIB !== 0) {
      fail("profile chunk tiers must be positive whole MiB values");
    }
    return value;
  });
  for (let index = 1; index < chunkTiersKib.length; index += 1) {
    if (chunkTiersKib[index - 1] <= chunkTiersKib[index]) {
      fail("profile chunk tiers must be strictly descending");
    }
  }
  const consumptionToleranceKib = integer(
    candidate.consumptionToleranceKib,
    "profile consumption tolerance KiB",
  );
  if (consumptionToleranceKib === 0) {
    fail("profile consumption tolerance must be positive");
  }
  return Object.freeze({
    name,
    packageName,
    qualificationPath,
    allocationParent,
    allocationPath,
    markerName,
    markerContents,
    minAvailableKib,
    maxAvailableKib,
    targetAvailableKib,
    chunkTiersKib: Object.freeze([...chunkTiersKib]),
    consumptionToleranceKib,
  });
}

const PHASE_6D6_LOW_STORAGE_PROFILE = validateProfile({
  name: "phase-6d6-low-storage",
  packageName: "com.emuchef.fixture",
  qualificationPath: "/sdcard/EmuChefQualification/com.emuchef.fixture/output",
  allocationParent: "/sdcard/Download",
  allocationPath: `${ALLOCATION_ROOT}/phase-6d6-low-storage`,
  markerName: ".emuchef-storage-preflight-owner",
  markerContents: "emuchef-storage-preflight:v1:phase-6d6-low-storage",
  minAvailableKib: 4 * 1024 * 1024,
  maxAvailableKib: 5_308_416,
  targetAvailableKib: 4_751_360,
  chunkTiersKib: [
    2 * 1024 * 1024,
    1024 * 1024,
    512 * 1024,
    256 * 1024,
  ],
  consumptionToleranceKib: 64 * 1024,
});

export const STORAGE_PROFILES = Object.freeze({
  "phase-6d6-low-storage": PHASE_6D6_LOW_STORAGE_PROFILE,
});

/** Return one exact reviewed profile. */
export function profileFor(name) {
  const profile = STORAGE_PROFILES[name];
  if (!profile) fail(`unknown storage profile: ${name ?? "<missing>"}`);
  return profile;
}

/** Parse the stable Android `df -k` columns used by this preflight. */
export function parseDfKib(output) {
  if (typeof output !== "string") fail("df output must be text");
  const lines = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length < 2) fail("df output is missing a data row");
  const header = lines[0].split(/\s+/);
  const availableIndex = header.indexOf("Available");
  if (availableIndex < 0) fail("df output is missing the Available column");
  const row = lines.slice(1).find((line) => !line.startsWith("Filesystem"));
  if (!row) fail("df output is missing a filesystem row");
  const fields = row.split(/\s+/);
  if (fields.length < 6 || availableIndex >= fields.length) {
    fail("df filesystem row does not match the reported header");
  }
  const parseField = (index, label) => {
    if (!/^[0-9]+$/.test(fields[index] ?? "")) fail(`df ${label} is not an integer`);
    return integer(Number.parseInt(fields[index], 10), `df ${label}`);
  };
  return {
    filesystem: nonEmptyString(fields[0], "df filesystem"),
    totalKib: parseField(1, "total KiB"),
    usedKib: parseField(2, "used KiB"),
    availableKib: parseField(availableIndex, "available KiB"),
    mountedOn: nonEmptyString(fields.at(-1), "df mount point"),
  };
}

/** Parse `adb devices -l` without accepting daemon chatter as inventory. */
export function parseAdbDevices(output) {
  if (typeof output !== "string") fail("ADB inventory output must be text");
  const rows = [];
  for (const rawLine of output.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line === "List of devices attached" || line.startsWith("*")) continue;
    const match = line.match(/^(\S+)\s+(\S+)(?:\s+(.*))?$/);
    if (!match) fail("ADB inventory contains an unparseable row");
    rows.push({ serial: match[1], state: match[2], details: match[3] ?? "" });
  }
  return rows;
}

/** Require one exact selected row in authorized `device` state. */
export function validateSelectedDevice(rows, serial) {
  validateSerial(serial);
  if (!Array.isArray(rows) || rows.length !== 1) {
    fail("storage preflight requires exactly one ADB device");
  }
  const [row] = rows;
  if (row?.serial !== serial) fail("the only ADB device does not match the selected serial");
  if (row?.state !== "device") fail("the selected ADB device must be in state device");
  return row;
}

function readinessFor(availableKib, profile) {
  if (availableKib < profile.minAvailableKib) return "below-minimum";
  if (availableKib <= profile.maxAvailableKib) return "ready";
  return "needs-preparation";
}

/** Select the largest reviewed chunk that cannot cross the target. */
export function planNextChunkKib({ availableKib, profile }) {
  integer(availableKib, "available KiB");
  const validated = validateProfile(profile);
  const readiness = readinessFor(availableKib, validated);
  if (readiness === "below-minimum") {
    fail("available storage is below the minimum accepted by the profile");
  }
  if (readiness === "ready") return null;
  const excessKib = availableKib - validated.targetAvailableKib;
  const tier = validated.chunkTiersKib.find((candidate) => candidate <= excessKib);
  if (tier !== undefined) return tier;
  const rounded = Math.floor(excessKib / KIB_PER_MIB) * KIB_PER_MIB;
  if (rounded <= 0) fail("no safe whole-MiB allocation remains before the profile target");
  return rounded;
}

/** Validate that one existing directory contains only owned profile artifacts. */
export function validateOwnedEntries(entries, profile) {
  const validated = validateProfile(profile);
  if (!Array.isArray(entries) || entries.some((entry) => typeof entry !== "string")) {
    fail("owned directory entries must be strings");
  }
  const markerCount = entries.filter((entry) => entry === validated.markerName).length;
  if (markerCount !== 1) fail("owned directory is missing the exact ownership marker");
  const chunks = [];
  const chunkIdentities = new Set();
  for (const entry of entries) {
    if (entry === validated.markerName) continue;
    const match = entry.match(CHUNK_PATTERN);
    if (!match) fail(`owned directory contains unexpected entry: ${entry}`);
    const index = Number.parseInt(match[1], 10);
    if (!Number.isSafeInteger(index) || index < 1 || index > 9_999) {
      fail(`owned directory contains invalid chunk identity: ${match[1]}`);
    }
    if (chunkIdentities.has(index)) fail(`owned directory contains duplicate chunk identity: ${index}`);
    chunkIdentities.add(index);
    chunks.push({ entry, index });
  }
  chunks.sort((left, right) => left.index - right.index);
  return chunks.map(({ entry }) => entry);
}

/** Return the next create-new deterministic chunk filename. */
export function nextChunkName(entries, profile) {
  const chunks = validateOwnedEntries(entries, profile);
  const highest = chunks.reduce((maximum, entry) => {
    const match = entry.match(CHUNK_PATTERN);
    return Math.max(maximum, Number.parseInt(match[1], 10));
  }, 0);
  if (highest >= 9_999) fail("owned allocation exhausted the bounded chunk identity space");
  return `chunk-${String(highest + 1).padStart(4, "0")}.bin`;
}

/** Verify that the requested allocation explains the measured free-space delta. */
export function validateObservedConsumption({ beforeKib, afterKib, requestedKib, profile }) {
  integer(beforeKib, "available KiB before allocation");
  integer(afterKib, "available KiB after allocation");
  integer(requestedKib, "requested allocation KiB");
  const validated = validateProfile(profile);
  const observedKib = beforeKib - afterKib;
  const minimum = Math.max(1, requestedKib - validated.consumptionToleranceKib);
  const maximum = requestedKib + validated.consumptionToleranceKib;
  if (observedKib < minimum || observedKib > maximum) {
    fail(
      `observed storage consumption ${observedKib} KiB does not match the requested ${requestedKib} KiB chunk`,
    );
  }
  return observedKib;
}

function markerPath(profile) {
  return `${profile.allocationPath}/${profile.markerName}`;
}

function inspectOwnedDirectory({ device, serial, profile }) {
  const pathType = device.pathType(serial, profile.allocationPath);
  if (pathType === "absent") {
    return { present: false, entries: [], chunks: [] };
  }
  if (pathType !== "directory") {
    fail("profile allocation path exists but is not a directory");
  }
  const entries = device.listEntries(serial, profile.allocationPath);
  const chunks = validateOwnedEntries(entries, profile);
  const marker = device.readText(serial, markerPath(profile));
  if (marker !== profile.markerContents) {
    fail("profile allocation directory has the wrong ownership marker");
  }
  return { present: true, entries, chunks };
}

/** Inspect device, filesystem, package, ownership, and readiness without mutation. */
export function inspectStorage({ device, serial, profile }) {
  validateSerial(serial);
  const validated = validateProfile(profile);
  validateSelectedDevice(device.listDevices(), serial);
  if (!device.packageInstalled(serial, validated.packageName)) {
    fail(`required package ${validated.packageName} is not installed on the selected device`);
  }
  const qualificationDf = device.df(serial, validated.qualificationPath);
  const allocationDf = device.df(serial, validated.allocationParent);
  const allocationParentType = device.pathType(serial, validated.allocationParent);
  if (allocationParentType !== "directory") {
    fail("approved Downloads allocation parent must exist as a directory");
  }
  const allocationRootType = device.pathType(serial, ALLOCATION_ROOT);
  if (allocationRootType !== "absent" && allocationRootType !== "directory") {
    fail("EmuChef allocation root must be absent or an ordinary directory, never a symlink");
  }
  if (
    qualificationDf.filesystem !== allocationDf.filesystem
    || qualificationDf.mountedOn !== allocationDf.mountedOn
  ) {
    fail("qualification destination and preflight allocation must use the same filesystem");
  }
  const owned = inspectOwnedDirectory({ device, serial, profile: validated });
  return {
    profile: validated,
    availableKib: qualificationDf.availableKib,
    readiness: readinessFor(qualificationDf.availableKib, validated),
    filesystem: qualificationDf.filesystem,
    mountedOn: qualificationDf.mountedOn,
    owned,
  };
}

function ensureOwnedDirectory({ device, serial, profile, inspection }) {
  if (inspection.owned.present) return;
  device.createDirectory(serial, profile.allocationPath);
  device.writeText(serial, markerPath(profile), profile.markerContents);
  const verified = inspectOwnedDirectory({ device, serial, profile });
  if (!verified.present) fail("profile allocation directory could not be verified after creation");
}

/** Prepare storage pressure, retaining owned partial chunks for resume on failure. */
export function prepareStorage({
  device,
  serial,
  profile,
  dryRun = false,
  confirmed = false,
  onProgress = () => {},
}) {
  const validated = validateProfile(profile);
  let inspection = inspectStorage({ device, serial, profile: validated });
  if (inspection.readiness === "below-minimum") {
    fail("available storage is below the minimum accepted by the profile");
  }
  if (inspection.readiness === "ready") {
    return { status: "ready", availableKib: inspection.availableKib, chunksWritten: 0 };
  }
  const firstChunk = planNextChunkKib({ availableKib: inspection.availableKib, profile: validated });
  if (dryRun) {
    return {
      status: "dry-run",
      availableKib: inspection.availableKib,
      nextChunkKib: firstChunk,
      chunksWritten: 0,
    };
  }
  if (!confirmed) fail("prepare requires explicit --yes confirmation");
  ensureOwnedDirectory({ device, serial, profile: validated, inspection });

  let chunksWritten = 0;
  for (let iteration = 0; iteration < 512; iteration += 1) {
    inspection = inspectStorage({ device, serial, profile: validated });
    if (inspection.readiness === "ready") {
      return { status: "ready", availableKib: inspection.availableKib, chunksWritten };
    }
    if (inspection.readiness === "below-minimum") {
      fail("available storage fell below the profile minimum during preparation");
    }
    const requestedKib = planNextChunkKib({
      availableKib: inspection.availableKib,
      profile: validated,
    });
    const chunkName = nextChunkName(inspection.owned.entries, validated);
    const chunkPath = `${validated.allocationPath}/${chunkName}`;
    const beforeKib = inspection.availableKib;
    device.writeZeroFile(serial, chunkPath, requestedKib);
    device.sync(serial);
    const afterKib = device.df(serial, validated.qualificationPath).availableKib;
    const observedKib = validateObservedConsumption({
      beforeKib,
      afterKib,
      requestedKib,
      profile: validated,
    });
    if (afterKib < validated.minAvailableKib) {
      fail("allocation crossed below the profile minimum; run the printed cleanup command");
    }
    chunksWritten += 1;
    onProgress({ chunkName, requestedKib, observedKib, beforeKib, afterKib });
  }
  fail("storage preparation exceeded the bounded chunk iteration limit");
}

/** Remove only an exactly owned profile directory and verify absence. */
export function cleanupStorage({ device, serial, profile, dryRun = false }) {
  const validated = validateProfile(profile);
  const inspection = inspectStorage({ device, serial, profile: validated });
  if (!inspection.owned.present) {
    return {
      status: "absent",
      beforeKib: inspection.availableKib,
      afterKib: inspection.availableKib,
    };
  }
  if (dryRun) {
    return {
      status: "dry-run",
      beforeKib: inspection.availableKib,
      afterKib: inspection.availableKib,
      chunks: inspection.owned.chunks.length,
    };
  }
  const beforeKib = inspection.availableKib;
  device.removeTree(serial, validated.allocationPath);
  device.sync(serial);
  if (device.pathType(serial, validated.allocationPath) !== "absent") {
    fail("profile allocation directory remained after cleanup");
  }
  const afterKib = device.df(serial, validated.qualificationPath).availableKib;
  return { status: "cleaned", beforeKib, afterKib };
}

/** Real synchronous Android Platform-Tools adapter. */
export class AdbStorageDevice {
  constructor({ adbPath = DEFAULT_ADB, execFile = execFileSync } = {}) {
    this.adbPath = nonEmptyString(adbPath, "ADB path");
    this.execFile = execFile;
  }

  run(args, { inherit = false } = {}) {
    try {
      const result = this.execFile(this.adbPath, args, inherit
        ? { stdio: ["ignore", "inherit", "inherit"] }
        : {
            encoding: "utf8",
            maxBuffer: MAX_COMMAND_OUTPUT_BYTES,
            stdio: ["ignore", "pipe", "pipe"],
          });
      return typeof result === "string" ? result : "";
    } catch (error) {
      const stderr = error && typeof error === "object" && "stderr" in error
        ? String(error.stderr ?? "").trim()
        : "";
      fail(`ADB command failed${stderr ? `: ${stderr}` : ""}`);
    }
  }

  listDevices() {
    return parseAdbDevices(this.run(["devices", "-l"]));
  }

  packageInstalled(serial, packageName) {
    validateSerial(serial);
    const output = this.run(["-s", serial, "shell", "pm", "path", packageName]);
    return output.split(/\r?\n/).some((line) => line.startsWith("package:"));
  }

  df(serial, devicePath) {
    validateSerial(serial);
    validateDevicePath(devicePath);
    return parseDfKib(this.run(["-s", serial, "shell", "df", "-k", devicePath]));
  }

  pathType(serial, devicePath) {
    validateSerial(serial);
    validateDevicePath(devicePath);
    const quoted = shellQuote(devicePath);
    return this.run([
      "-s", serial, "shell",
      `if [ -L ${quoted} ]; then printf symlink; elif [ -d ${quoted} ]; then printf directory; elif [ -e ${quoted} ]; then printf other; else printf absent; fi`,
    ]).trim();
  }

  listEntries(serial, devicePath) {
    validateSerial(serial);
    validateDevicePath(devicePath);
    const output = this.run(["-s", serial, "shell", "ls", "-1A", devicePath]);
    return output.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  }

  readText(serial, devicePath) {
    validateSerial(serial);
    validateDevicePath(devicePath);
    return this.run(["-s", serial, "shell", "cat", devicePath]);
  }

  createDirectory(serial, devicePath) {
    validateSerial(serial);
    validateAllocationMutationPath(devicePath);
    this.run(["-s", serial, "shell", "mkdir", "-p", devicePath]);
  }

  writeText(serial, devicePath, contents) {
    validateSerial(serial);
    validateAllocationMutationPath(devicePath);
    nonEmptyString(contents, "marker contents");
    this.run([
      "-s", serial, "shell", "sh", "-c",
      `umask 077; printf %s ${shellQuote(contents)} > ${shellQuote(devicePath)}`,
    ]);
  }

  writeZeroFile(serial, devicePath, requestedKib) {
    validateSerial(serial);
    validateAllocationMutationPath(devicePath);
    integer(requestedKib, "requested allocation KiB");
    if (requestedKib === 0 || requestedKib % KIB_PER_MIB !== 0) {
      fail("device-local dd allocation must be a positive whole-MiB value");
    }
    const countMib = requestedKib / KIB_PER_MIB;
    this.run([
      "-s", serial, "shell", "dd",
      "if=/dev/zero",
      `of=${devicePath}`,
      "bs=1048576",
      `count=${countMib}`,
    ], { inherit: true });
  }

  sync(serial) {
    validateSerial(serial);
    this.run(["-s", serial, "shell", "sync"]);
  }

  removeTree(serial, devicePath) {
    validateSerial(serial);
    validateAllocationMutationPath(devicePath);
    this.run(["-s", serial, "shell", "rm", "-rf", devicePath]);
  }
}

function option(args, name, { required = false } = {}) {
  const index = args.indexOf(name);
  if (index < 0) {
    if (required) fail(`missing required option ${name}`);
    return null;
  }
  const value = args[index + 1];
  if (!value || value.startsWith("--")) fail(`missing value for option ${name}`);
  return value;
}

function hasFlag(args, name) {
  return args.includes(name);
}

function validateCliShape(command, args) {
  const valueOptions = new Set(["--serial", "--profile", "--adb"]);
  const flagOptions = new Set(
    command === "prepare"
      ? ["--dry-run", "--yes"]
      : command === "cleanup"
        ? ["--dry-run"]
        : [],
  );
  const seen = new Set();
  for (let index = 1; index < args.length; index += 1) {
    const argument = args[index];
    if (seen.has(argument)) fail(`duplicate option for ${command}: ${argument}`);
    seen.add(argument);
    if (valueOptions.has(argument)) {
      const value = args[index + 1];
      if (!value || value.startsWith("--")) fail(`missing value for option ${argument}`);
      index += 1;
      continue;
    }
    if (flagOptions.has(argument)) continue;
    fail(`unknown option for ${command}: ${argument}`);
  }
  if (command === "prepare" && hasFlag(args, "--dry-run") && hasFlag(args, "--yes")) {
    fail("--dry-run and --yes are mutually exclusive");
  }
}

function formatKib(value) {
  return `${value.toLocaleString("en-US")} KiB (${(value / 1024 / 1024).toFixed(2)} GiB)`;
}

function cleanupCommand(serial, profile) {
  return `node tools/device-storage-preflight.mjs cleanup --serial ${serial} --profile ${profile.name}`;
}

function harnessBlock(serial) {
  return [
    "SENTINEL_DIR=\"$(mktemp -d -t emuchef-phase-6d6)\"",
    "export EMUCHEF_RUN_REAL_ADB_TESTS=1",
    "export EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1",
    "export EMUCHEF_PHASE_6D6_SCENARIO=low_storage",
    "export EMUCHEF_PHASE_6D6_REPETITION=1",
    `export EMUCHEF_TEST_DEVICE_SERIAL=${serial}`,
    "export EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture",
    "export EMUCHEF_PHASE_6D6_SENTINEL_DIR=\"$SENTINEL_DIR\"",
    "export EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE=1",
    "cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::manual_phase_6d6_physical_interruption_qualification -- --ignored --exact --nocapture",
  ].join("\n");
}

function printInspection(stdout, inspection) {
  stdout.write(`Profile: ${inspection.profile.name}\n`);
  stdout.write(`Filesystem: ${inspection.filesystem} mounted on ${inspection.mountedOn}\n`);
  stdout.write(`Available: ${formatKib(inspection.availableKib)}\n`);
  stdout.write(
    `Accepted window: ${formatKib(inspection.profile.minAvailableKib)} through ${formatKib(inspection.profile.maxAvailableKib)}\n`,
  );
  stdout.write(`Readiness: ${inspection.readiness}\n`);
  stdout.write(
    `Owned allocation: ${inspection.owned.present ? `${inspection.owned.chunks.length} chunk(s)` : "absent"}\n`,
  );
}

function helpText() {
  return `Usage:\n  node tools/device-storage-preflight.mjs status --serial <exact-serial> --profile <name>\n  node tools/device-storage-preflight.mjs prepare --serial <exact-serial> --profile <name> [--dry-run | --yes]\n  node tools/device-storage-preflight.mjs cleanup --serial <exact-serial> --profile <name> [--dry-run]\n`;
}

/** Execute one CLI command. The injectable factory keeps tests off ADB. */
export function runCli(args, {
  stdout = process.stdout,
  stderr = process.stderr,
  deviceFactory = (options) => new AdbStorageDevice(options),
} = {}) {
  if (!Array.isArray(args)) fail("CLI arguments must be an array");
  const command = args[0];
  if (!command || command === "help" || command === "--help" || command === "-h") {
    stdout.write(helpText());
    return 0;
  }
  if (!["status", "prepare", "cleanup"].includes(command)) {
    fail(`unknown storage-preflight command: ${command}`);
  }
  validateCliShape(command, args);
  const serial = validateSerial(option(args, "--serial", { required: true }));
  const profile = profileFor(option(args, "--profile", { required: true }));
  const adbPath = option(args, "--adb") ?? DEFAULT_ADB;
  const dryRun = hasFlag(args, "--dry-run");
  const confirmed = hasFlag(args, "--yes");
  if (command === "prepare" && !dryRun && !confirmed) {
    fail("prepare requires --yes; use --dry-run to inspect the next allocation without mutation");
  }
  if (command !== "prepare" && confirmed) fail("--yes is accepted only by prepare");
  const device = deviceFactory({ adbPath });

  if (command === "status") {
    printInspection(stdout, inspectStorage({ device, serial, profile }));
    stdout.write(`Cleanup: ${cleanupCommand(serial, profile)}\n`);
    return 0;
  }

  if (command === "prepare") {
    const result = prepareStorage({
      device,
      serial,
      profile,
      dryRun,
      confirmed,
      onProgress: ({ chunkName, requestedKib, observedKib, afterKib }) => {
        stdout.write(
          `${chunkName}: requested ${formatKib(requestedKib)}, observed ${formatKib(observedKib)}, available ${formatKib(afterKib)}\n`,
        );
      },
    });
    if (result.status === "dry-run") {
      stdout.write(`Dry run: next chunk ${formatKib(result.nextChunkKib)}; no device files were created.\n`);
    } else {
      stdout.write(`Storage preflight ready at ${formatKib(result.availableKib)}.\n`);
      stdout.write("Run the low-storage qualification block:\n");
      stdout.write(`${harnessBlock(serial)}\n`);
    }
    stdout.write(`Cleanup after both repetitions: ${cleanupCommand(serial, profile)}\n`);
    return 0;
  }

  const result = cleanupStorage({ device, serial, profile, dryRun });
  if (result.status === "dry-run") {
    stdout.write(`Dry run: would remove ${result.chunks} owned chunk(s); no files were deleted.\n`);
  } else if (result.status === "absent") {
    stdout.write("Owned storage-preflight allocation is already absent.\n");
  } else {
    stdout.write(
      `Cleaned owned allocation; available storage changed from ${formatKib(result.beforeKib)} to ${formatKib(result.afterKib)}.\n`,
    );
  }
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    process.exitCode = runCli(process.argv.slice(2));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message}\n`);
    const args = process.argv.slice(2);
    let serial = null;
    let profileName = null;
    try {
      serial = option(args, "--serial");
      profileName = option(args, "--profile");
    } catch {
      // Preserve the original CLI error when option recovery parsing also fails.
    }
    if (serial && SAFE_SERIAL_PATTERN.test(serial) && profileName && STORAGE_PROFILES[profileName]) {
      process.stderr.write(
        `Owned-file recovery: ${cleanupCommand(serial, STORAGE_PROFILES[profileName])}\n`,
      );
    }
    process.exitCode = 1;
  }
}

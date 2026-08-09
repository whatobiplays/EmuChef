import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  AdbStorageDevice,
  STORAGE_PROFILES,
  cleanupStorage,
  inspectStorage,
  nextChunkName,
  parseAdbDevices,
  parseDfKib,
  planNextChunkKib,
  prepareStorage,
  profileFor,
  runCli,
  validateObservedConsumption,
  validateOwnedEntries,
  validateProfile,
  validateSelectedDevice,
} from "./device-storage-preflight.mjs";

const SERIAL = "BW02124230000079";
const PROFILE = profileFor("phase-6d6-low-storage");

function compactProfile(overrides = {}) {
  return validateProfile({
    ...PROFILE,
    name: "test-storage-profile",
    allocationPath: "/sdcard/Download/EmuChefStoragePreflight/test-storage-profile",
    markerContents: "emuchef-storage-preflight:v1:test-storage-profile",
    minAvailableKib: 4_096,
    maxAvailableKib: 5_120,
    targetAvailableKib: 4_608,
    chunkTiersKib: [2_048, 1_024],
    consumptionToleranceKib: 64,
    ...overrides,
  });
}

class FakeDevice {
  constructor({
    availableKib = 8_704,
    allocationFilesystem = "/dev/fuse",
    qualificationFilesystem = "/dev/fuse",
    allocationMount = "/storage/emulated",
    qualificationMount = "/storage/emulated",
    pathType = "absent",
    allocationRootType = "absent",
    allocationParentType = "directory",
    entries = [],
    markerContents = null,
    consumptionDeltas = [],
    inventory = [{ serial: SERIAL, state: "device", details: "usb:0-1" }],
    packageInstalled = true,
  } = {}) {
    this.availableKib = availableKib;
    this.allocationFilesystem = allocationFilesystem;
    this.qualificationFilesystem = qualificationFilesystem;
    this.allocationMount = allocationMount;
    this.qualificationMount = qualificationMount;
    this.currentPathType = pathType;
    this.allocationRootType = allocationRootType;
    this.allocationParentType = allocationParentType;
    this.entries = new Set(entries);
    this.markerContents = markerContents;
    this.consumptionDeltas = [...consumptionDeltas];
    this.inventoryRows = inventory;
    this.fixtureInstalled = packageInstalled;
    this.writes = [];
    this.createdDirectories = [];
    this.removedPaths = [];
    this.syncCount = 0;
  }

  listDevices() {
    return this.inventoryRows.map((row) => ({ ...row }));
  }

  packageInstalled() {
    return this.fixtureInstalled;
  }

  df(_serial, path) {
    const allocation = path === PROFILE.allocationParent || path === compactProfile().allocationParent;
    return {
      filesystem: allocation ? this.allocationFilesystem : this.qualificationFilesystem,
      totalKib: 60_000_000,
      usedKib: 60_000_000 - this.availableKib,
      availableKib: this.availableKib,
      mountedOn: allocation ? this.allocationMount : this.qualificationMount,
    };
  }

  pathType(_serial, devicePath) {
    if (devicePath === "/sdcard/Download") return this.allocationParentType;
    if (devicePath === "/sdcard/Download/EmuChefStoragePreflight") {
      return this.allocationRootType;
    }
    return this.currentPathType;
  }

  listEntries() {
    return [...this.entries].sort();
  }

  readText() {
    return this.markerContents;
  }

  createDirectory(_serial, path) {
    this.createdDirectories.push(path);
    this.currentPathType = "directory";
  }

  writeText(_serial, path, contents) {
    this.entries.add(path.split("/").at(-1));
    this.markerContents = contents;
  }

  writeZeroFile(_serial, path, requestedKib) {
    this.writes.push({ path, requestedKib });
    this.entries.add(path.split("/").at(-1));
    const observed = this.consumptionDeltas.length > 0
      ? this.consumptionDeltas.shift()
      : requestedKib;
    this.availableKib -= observed;
  }

  sync() {
    this.syncCount += 1;
  }

  removeTree(_serial, path) {
    this.removedPaths.push(path);
    this.currentPathType = "absent";
    this.entries.clear();
    this.markerContents = null;
    const restored = this.writes.reduce((sum, entry) => sum + entry.requestedKib, 0);
    this.availableKib += restored;
  }
}

test("parses Android df output by the Available header", () => {
  assert.deepEqual(
    parseDfKib(`Filesystem     1K-blocks    Used Available Use% Mounted on\n/dev/fuse       52232748 9183700  42901592  18% /storage/emulated\n`),
    {
      filesystem: "/dev/fuse",
      totalKib: 52_232_748,
      usedKib: 9_183_700,
      availableKib: 42_901_592,
      mountedOn: "/storage/emulated",
    },
  );
  assert.throws(() => parseDfKib("Filesystem Used\n/dev/fuse 1\n"), /Available/);
});

test("parses ADB rows and requires one exact authorized selected device", () => {
  const rows = parseAdbDevices(`List of devices attached\n${SERIAL} device usb:0-1 model:GT78_VN\n`);
  assert.equal(validateSelectedDevice(rows, SERIAL).state, "device");
  assert.throws(
    () => validateSelectedDevice([...rows, { serial: "other", state: "device", details: "" }], SERIAL),
    /exactly one ADB device/,
  );
  assert.throws(
    () => validateSelectedDevice([{ serial: SERIAL, state: "unauthorized", details: "" }], SERIAL),
    /state device/,
  );
  assert.throws(() => validateSelectedDevice(rows, "bad serial"), /serial/);
});

test("commits the Phase 6D.6 profile and validates its immutable bounds", () => {
  assert.equal(STORAGE_PROFILES["phase-6d6-low-storage"], PROFILE);
  assert.equal(PROFILE.minAvailableKib, 4_194_304);
  assert.equal(PROFILE.maxAvailableKib, 5_308_416);
  assert.equal(PROFILE.targetAvailableKib, 4_751_360);
  assert.equal(
    PROFILE.allocationPath,
    "/sdcard/Download/EmuChefStoragePreflight/phase-6d6-low-storage",
  );
  assert.throws(
    () => validateProfile({ ...PROFILE, targetAvailableKib: PROFILE.minAvailableKib - 1 }),
    /target/,
  );
  assert.throws(
    () => validateProfile({ ...PROFILE, allocationPath: "/sdcard/Download/not-owned" }),
    /allocation path/,
  );
});

test("plans large-to-small chunks without crossing the target", () => {
  assert.equal(planNextChunkKib({ availableKib: 20_000_000, profile: PROFILE }), 2_097_152);
  assert.equal(planNextChunkKib({ availableKib: 6_000_000, profile: PROFILE }), 1_048_576);
  assert.equal(planNextChunkKib({ availableKib: PROFILE.maxAvailableKib, profile: PROFILE }), null);
  assert.throws(
    () => planNextChunkKib({ availableKib: PROFILE.minAvailableKib - 1, profile: PROFILE }),
    /below the minimum/,
  );
});

test("accepts only the exact marker and deterministic chunk names", () => {
  const entries = [PROFILE.markerName, "chunk-0001.bin", "chunk-0012.bin"];
  assert.deepEqual(validateOwnedEntries(entries, PROFILE), ["chunk-0001.bin", "chunk-0012.bin"]);
  assert.equal(nextChunkName(entries, PROFILE), "chunk-0013.bin");
  assert.throws(() => validateOwnedEntries([PROFILE.markerName, "photo.jpg"], PROFILE), /unexpected entry/);
  assert.throws(
    () => validateOwnedEntries([PROFILE.markerName, "chunk-0001.bin", "chunk-00001.bin"], PROFILE),
    /duplicate chunk identity/,
  );
  assert.throws(() => validateOwnedEntries(["chunk-0001.bin"], PROFILE), /ownership marker/);
  assert.throws(
    () => validateOwnedEntries([PROFILE.markerName, "chunk-0000.bin"], PROFILE),
    /chunk identity/,
  );
  assert.throws(
    () => validateOwnedEntries([PROFILE.markerName, "chunk-10000.bin"], PROFILE),
    /chunk identity/,
  );
});

test("bounds observed storage consumption around the requested chunk", () => {
  assert.equal(
    validateObservedConsumption({ beforeKib: 10_000, afterKib: 7_960, requestedKib: 2_048, profile: compactProfile() }),
    2_040,
  );
  assert.throws(
    () => validateObservedConsumption({ beforeKib: 10_000, afterKib: 9_000, requestedKib: 2_048, profile: compactProfile() }),
    /observed storage consumption/,
  );
});

test("inspects readiness and rejects allocation on another filesystem", () => {
  const ready = new FakeDevice({ availableKib: 5_000 });
  assert.equal(inspectStorage({ device: ready, serial: SERIAL, profile: compactProfile() }).readiness, "ready");

  const mismatch = new FakeDevice({ allocationFilesystem: "/dev/other" });
  assert.throws(
    () => inspectStorage({ device: mismatch, serial: SERIAL, profile: compactProfile() }),
    /same filesystem/,
  );

  const symlinkedRoot = new FakeDevice({ allocationRootType: "symlink" });
  assert.throws(
    () => inspectStorage({ device: symlinkedRoot, serial: SERIAL, profile: compactProfile() }),
    /allocation root.*directory|allocation root.*symlink/i,
  );
});

test("dry-run reports the next chunk without mutating the device", () => {
  const device = new FakeDevice();
  const result = prepareStorage({
    device,
    serial: SERIAL,
    profile: compactProfile(),
    dryRun: true,
  });
  assert.equal(result.status, "dry-run");
  assert.equal(result.nextChunkKib, 2_048);
  assert.deepEqual(device.writes, []);
  assert.deepEqual(device.createdDirectories, []);
});

test("prepares a fresh owned directory and stops inside the accepted window", () => {
  const device = new FakeDevice();
  const progress = [];
  const result = prepareStorage({
    device,
    serial: SERIAL,
    profile: compactProfile(),
    confirmed: true,
    onProgress: (entry) => progress.push(entry),
  });
  assert.equal(result.status, "ready");
  assert.equal(result.availableKib, 4_608);
  assert.deepEqual(device.writes.map((entry) => entry.path.split("/").at(-1)), [
    "chunk-0001.bin",
    "chunk-0002.bin",
  ]);
  assert.equal(device.markerContents, "emuchef-storage-preflight:v1:test-storage-profile");
  assert.equal(device.syncCount, 2);
  assert.equal(progress.length, 2);
});

test("resumes an owned partial allocation with the next chunk number", () => {
  const profile = compactProfile();
  const device = new FakeDevice({
    availableKib: 6_656,
    pathType: "directory",
    entries: [profile.markerName, "chunk-0001.bin"],
    markerContents: profile.markerContents,
  });
  const result = prepareStorage({ device, serial: SERIAL, profile, confirmed: true });
  assert.equal(result.availableKib, 4_608);
  assert.equal(device.writes[0].path.split("/").at(-1), "chunk-0002.bin");
});

test("fails closed below the minimum or on unexplained concurrent storage change", () => {
  const below = new FakeDevice({ availableKib: 4_000 });
  assert.throws(
    () => prepareStorage({ device: below, serial: SERIAL, profile: compactProfile(), confirmed: true }),
    /below the minimum/,
  );

  const changed = new FakeDevice({ consumptionDeltas: [1_000] });
  assert.throws(
    () => prepareStorage({ device: changed, serial: SERIAL, profile: compactProfile(), confirmed: true }),
    /observed storage consumption/,
  );
});

test("cleanup removes only an exactly owned directory and verifies absence", () => {
  const profile = compactProfile();
  const owned = new FakeDevice({
    availableKib: 4_608,
    pathType: "directory",
    entries: [profile.markerName, "chunk-0001.bin"],
    markerContents: profile.markerContents,
  });
  owned.writes.push({ path: `${profile.allocationPath}/chunk-0001.bin`, requestedKib: 2_048 });
  const result = cleanupStorage({ device: owned, serial: SERIAL, profile });
  assert.equal(result.status, "cleaned");
  assert.deepEqual(owned.removedPaths, [profile.allocationPath]);
  assert.equal(owned.currentPathType, "absent");
  assert.equal(owned.syncCount, 1);

  const unowned = new FakeDevice({
    pathType: "directory",
    entries: [profile.markerName, "chunk-0001.bin"],
    markerContents: "wrong-owner",
  });
  assert.throws(() => cleanupStorage({ device: unowned, serial: SERIAL, profile }), /marker/);
  assert.deepEqual(unowned.removedPaths, []);
});


test("CLI requires explicit confirmation and keeps dry-run non-mutating", () => {
  const device = new FakeDevice({ availableKib: 6_000_000 });
  const output = [];
  const io = {
    stdout: { write: (value) => output.push(String(value)) },
    stderr: { write: (value) => output.push(String(value)) },
    deviceFactory: () => device,
  };
  assert.throws(
    () => runCli(["prepare", "--serial", SERIAL, "--profile", "phase-6d6-low-storage"], io),
    /--yes/,
  );
  assert.equal(
    runCli([
      "prepare",
      "--serial", SERIAL,
      "--profile", "phase-6d6-low-storage",
      "--dry-run",
    ], io),
    0,
  );
  assert.deepEqual(device.writes, []);
  assert.match(output.join(""), /dry run|next chunk/i);
  assert.throws(
    () => runCli(["status", "--serial", SERIAL, "--profile", "unknown-profile"], io),
    /unknown storage profile/,
  );
  assert.throws(
    () => runCli([
      "status",
      "--serial", SERIAL,
      "--profile", "phase-6d6-low-storage",
      "--typo",
    ], io),
    /unknown option/,
  );
  assert.throws(
    () => runCli([
      "status",
      "--serial", SERIAL,
      "--serial", SERIAL,
      "--profile", "phase-6d6-low-storage",
    ], io),
    /duplicate option/,
  );
  assert.throws(
    () => runCli([
      "prepare",
      "--serial", SERIAL,
      "--profile", "phase-6d6-low-storage",
      "--dry-run",
      "--yes",
    ], io),
    /mutually exclusive/,
  );
});

test("uses a portable CLI entry check and device-local allocation commands", () => {
  const source = readFileSync(new URL("./device-storage-preflight.mjs", import.meta.url), "utf8");
  assert.doesNotMatch(source, /import\.meta\.main/);
  assert.match(source, /pathToFileURL/);
  assert.match(source, /\bdd\b/);
  assert.doesNotMatch(source, /mkdtemp|createWriteStream|set_len/);
});


test("real adapter allocates only through device-local dd arguments", () => {
  const calls = [];
  const device = new AdbStorageDevice({
    execFile: (command, args, options) => {
      calls.push({ command, args, options });
      return "";
    },
  });
  const chunkPath = `${PROFILE.allocationPath}/chunk-0001.bin`;
  device.writeZeroFile(SERIAL, chunkPath, 2 * 1024);
  assert.deepEqual(calls[0].args, [
    "-s", SERIAL, "shell", "dd",
    "if=/dev/zero",
    `of=${chunkPath}`,
    "bs=1048576",
    "count=2",
  ]);
  assert.deepEqual(calls[0].options.stdio, ["ignore", "inherit", "inherit"]);
});

test("real adapter sends path-type probes directly through adb shell", () => {
  const calls = [];
  const device = new AdbStorageDevice({
    execFile: (command, args, options) => {
      calls.push({ command, args, options });
      return "directory\n";
    },
  });

  assert.equal(device.pathType(SERIAL, "/sdcard/Download"), "directory");
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].args.slice(0, 3), [
    "-s",
    SERIAL,
    "shell",
  ]);
  assert.equal(calls[0].args.length, 4);
  assert.match(calls[0].args[3], /^if \[ -L /);
  assert.match(calls[0].args[3], /\/sdcard\/Download/);
  assert.match(calls[0].args[3], /then printf symlink/);
  assert.equal(calls[0].args.includes("sh"), false);
  assert.equal(calls[0].args.includes("-c"), false);
});

test("runbook documents the owned preflight lifecycle", () => {
  const runbook = readFileSync(
    new URL("../docs/manual/phase-6d6-physical-interruption-qualification.md", import.meta.url),
    "utf8",
  );
  assert.match(runbook, /device-storage-preflight\.mjs status/);
  assert.match(runbook, /device-storage-preflight\.mjs prepare[\s\S]*--dry-run/);
  assert.match(runbook, /device-storage-preflight\.mjs prepare[\s\S]*--yes/);
  assert.match(runbook, /device-storage-preflight\.mjs cleanup/);
  assert.match(runbook, /EmuChefStoragePreflight\/phase-6d6-low-storage/);
  assert.match(runbook, /not physical evidence/);
});


test("real adapter refuses mutations outside the EmuChef allocation root", () => {
  const calls = [];
  const device = new AdbStorageDevice({
    execFile: (command, args, options) => {
      calls.push({ command, args, options });
      return "";
    },
  });
  assert.throws(
    () => device.writeZeroFile(SERIAL, "/sdcard/Download/unowned.bin", 2 * 1024),
    /allocation root/,
  );
  assert.throws(
    () => device.removeTree(SERIAL, "/sdcard/Download"),
    /allocation root/,
  );
  assert.deepEqual(calls, []);
});

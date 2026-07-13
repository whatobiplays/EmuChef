import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  gatekeeperReportsNotarizedDeveloperId,
  inspectDeveloperIdMetadata,
  parseSignedReleasePaths,
  verifySignedMacosRelease,
} from "./check-signed-macos-release.mjs";

const METADATA = [
  "Authority=Developer ID Application: REDACTED",
  "TeamIdentifier=REDACTED",
  "Timestamp=Jul 11, 2026 at 00:00:00",
  "flags=0x10000(runtime)",
  "Notarization Ticket=stapled",
].join("\n");

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-signed-release-"));
  const appPath = path.join(root, "Example.app");
  const macos = path.join(appPath, "Contents", "MacOS");
  fs.mkdirSync(macos, { recursive: true });
  fs.writeFileSync(path.join(appPath, "Contents", "Info.plist"), "test-only");
  for (const name of ["example-main", "emuchef"]) {
    const executable = path.join(macos, name);
    fs.writeFileSync(executable, "test-only");
    fs.chmodSync(executable, 0o755);
  }
  const dmgPath = path.join(root, "Example.dmg");
  fs.writeFileSync(dmgPath, "test-only");
  const run = (command, args) => {
    if (command === "plutil") {
      return { status: 0, stdout: JSON.stringify({ CFBundleExecutable: "example-main" }) };
    }
    if (command === "codesign" && args[0] === "-dv") {
      return { status: 0, stderr: METADATA };
    }
    if (command === "spctl") {
      return { status: 0, stderr: "accepted\nsource=Notarized Developer ID\n" };
    }
    return { status: 0, stdout: "validation passed" };
  };
  return { appPath, dmgPath, root, run };
}

test("requires an app and DMG path", () => {
  assert.throws(() => parseSignedReleasePaths([]), /usage/);
  assert.throws(() => parseSignedReleasePaths(["one"]), /usage/);
  const parsed = parseSignedReleasePaths(["Example.app", "Example.dmg"]);
  assert.match(parsed.appPath, /Example\.app$/);
  assert.match(parsed.dmgPath, /Example\.dmg$/);
});

test("recognizes required Developer ID metadata without returning values", () => {
  const result = inspectDeveloperIdMetadata(METADATA, {
    requireRuntime: true,
    requireTicket: true,
  });
  assert.deepEqual(result, {
    developerIdAuthority: true,
    runtime: true,
    teamIdentifier: true,
    ticket: true,
    timestamp: true,
  });
  assert.throws(
    () => inspectDeveloperIdMetadata("Authority=Apple Development", { requireRuntime: true }),
    /required marker/,
  );
});

test("derives notarization only from the Gatekeeper source", () => {
  assert.equal(
    gatekeeperReportsNotarizedDeveloperId("accepted\nsource=Notarized Developer ID\n"),
    true,
  );
  assert.throws(() => gatekeeperReportsNotarizedDeveloperId("accepted\nsource=Developer ID\n"));
  assert.throws(() => gatekeeperReportsNotarizedDeveloperId("Notarization Ticket=stapled"));
});

test("verifies app and DMG while emitting only safe booleans", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  const result = verifySignedMacosRelease(value.appPath, value.dmgPath, value);
  assert.deepEqual(result.app, {
    signed: true,
    notarized: true,
    stapled: true,
    gatekeeperAccepted: true,
  });
  assert.deepEqual(result.dmg, result.app);
  const serialized = JSON.stringify(result);
  assert.doesNotMatch(serialized, /REDACTED|TeamIdentifier|emuchef-signed-release/);
});

test("requires expected artifact types and bundle files", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  assert.throws(() => verifySignedMacosRelease("Example", value.dmgPath, value), /end in .app/);
  assert.throws(() => verifySignedMacosRelease(value.appPath, "Example", value), /end in .dmg/);
  fs.rmSync(path.join(value.appPath, "Contents", "MacOS", "emuchef"));
  assert.throws(() => verifySignedMacosRelease(value.appPath, value.dmgPath, value), /sidecar/);
});

test("fails safely when any external verification command fails", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  for (const failedCommand of ["plutil", "codesign", "spctl", "xcrun"]) {
    const run = (command, args, options) => {
      if (command === failedCommand) {
        return { status: 1, stderr: `${value.root} TeamIdentifier=SECRET` };
      }
      return value.run(command, args, options);
    };
    assert.throws(
      () => verifySignedMacosRelease(value.appPath, value.dmgPath, { run }),
      (error) => !error.message.includes(value.root) && !error.message.includes("SECRET"),
    );
  }
});

test("keeps stapler validation independent from Gatekeeper notarization", (t) => {
  const value = fixture();
  t.after(() => fs.rmSync(value.root, { force: true, recursive: true }));
  let staplerCalls = 0;
  const run = (command, args, options) => {
    if (command === "xcrun") {
      staplerCalls += 1;
      return { status: 1, stderr: "ticket missing" };
    }
    return value.run(command, args, options);
  };
  assert.throws(() => verifySignedMacosRelease(value.appPath, value.dmgPath, { run }), /stapler/);
  assert.equal(staplerCalls, 1);
});

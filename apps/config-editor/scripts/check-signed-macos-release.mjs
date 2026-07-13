#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { resolveBundleLayout } from "./check-macos-bundle.mjs";

const MAX_COMMAND_OUTPUT = 64 * 1024 * 1024;

export function parseSignedReleasePaths(argv) {
  if (argv.length !== 2 || !argv[0] || !argv[1]) {
    throw new Error("usage: check-signed-macos-release.mjs <app-path> <dmg-path>");
  }
  return { appPath: path.resolve(argv[0]), dmgPath: path.resolve(argv[1]) };
}

function commandOutput(result) {
  return `${result.stdout || ""}\n${result.stderr || ""}`;
}

function runChecked(run, label, command, args) {
  const result = run(command, args, { encoding: "utf8", maxBuffer: MAX_COMMAND_OUTPUT });
  if (result.error || result.status !== 0) {
    throw new Error(`${label} failed`);
  }
  return commandOutput(result);
}

function requireDirectory(fsApi, candidate, label) {
  try {
    if (!fsApi.statSync(candidate).isDirectory()) {
      throw new Error("not a directory");
    }
  } catch {
    throw new Error(`${label} was not found or was not a directory`);
  }
}

function requireFile(fsApi, candidate, label, executable = false) {
  try {
    if (!fsApi.statSync(candidate).isFile()) {
      throw new Error("not a file");
    }
    if (executable) {
      fsApi.accessSync(candidate, fs.constants.X_OK);
    }
  } catch {
    throw new Error(`${label} was not found or did not meet release requirements`);
  }
}

export function inspectDeveloperIdMetadata(output, { requireRuntime, requireTicket }) {
  const checks = {
    developerIdAuthority: /^Authority=Developer ID Application:/m.test(output),
    teamIdentifier: /^TeamIdentifier=(?!not set$)\S+/m.test(output),
    timestamp: /^Timestamp=(?!none$).+/m.test(output),
    runtime: /flags=.*\bruntime\b/m.test(output),
    ticket: /^Notarization Ticket=stapled$/m.test(output),
  };
  const required = ["developerIdAuthority", "teamIdentifier", "timestamp"];
  if (requireRuntime) {
    required.push("runtime");
  }
  if (requireTicket) {
    required.push("ticket");
  }
  const missing = required.filter((name) => !checks[name]);
  if (missing.length > 0) {
    throw new Error(`code-signing metadata omitted required marker(s): ${missing.join(", ")}`);
  }
  return checks;
}

/** Gatekeeper is the sole source for the notarized result. */
export function gatekeeperReportsNotarizedDeveloperId(output) {
  if (!/(?:^|\n)source=Notarized Developer ID(?:\n|$)/.test(output)) {
    throw new Error("Gatekeeper did not report Notarized Developer ID");
  }
  return true;
}

function readInfoPlist(infoPath, run) {
  const output = runChecked(
    run,
    "Info.plist inspection",
    "plutil",
    ["-convert", "json", "-o", "-", infoPath],
  );
  try {
    return JSON.parse(output);
  } catch {
    throw new Error("Info.plist inspection returned invalid data");
  }
}

/**
 * Verifies a signed release without returning raw command output or artifact
 * paths. Notarization and stapling deliberately use independent commands.
 */
export function verifySignedMacosRelease(
  appPath,
  dmgPath,
  { fsApi = fs, run = spawnSync } = {},
) {
  if (!appPath.endsWith(".app")) {
    throw new Error("application path must end in .app");
  }
  if (!dmgPath.endsWith(".dmg")) {
    throw new Error("disk-image path must end in .dmg");
  }
  requireDirectory(fsApi, appPath, "application bundle");
  requireFile(fsApi, dmgPath, "disk image");

  const infoPath = path.join(appPath, "Contents", "Info.plist");
  requireFile(fsApi, infoPath, "Info.plist");
  const info = readInfoPlist(infoPath, run);
  const layout = resolveBundleLayout(appPath, info);
  requireFile(fsApi, layout.mainPath, "main executable", true);
  requireFile(fsApi, layout.sidecarPath, "Rust sidecar", true);

  runChecked(run, "application signature verification", "codesign", [
    "--verify",
    "--deep",
    "--strict",
    "--verbose=4",
    appPath,
  ]);
  runChecked(run, "main executable signature verification", "codesign", [
    "--verify",
    "--strict",
    layout.mainPath,
  ]);
  runChecked(run, "sidecar signature verification", "codesign", [
    "--verify",
    "--strict",
    layout.sidecarPath,
  ]);
  const appMetadata = runChecked(run, "application signing metadata inspection", "codesign", [
    "-dv",
    "--verbose=4",
    appPath,
  ]);
  inspectDeveloperIdMetadata(appMetadata, { requireRuntime: true, requireTicket: true });

  const appGatekeeper = runChecked(run, "application Gatekeeper assessment", "spctl", [
    "--assess",
    "--type",
    "execute",
    "--verbose=4",
    appPath,
  ]);
  const appNotarized = gatekeeperReportsNotarizedDeveloperId(appGatekeeper);
  runChecked(run, "application stapler validation", "xcrun", ["stapler", "validate", appPath]);

  runChecked(run, "disk-image signature verification", "codesign", [
    "--verify",
    "--verbose=4",
    dmgPath,
  ]);
  const dmgMetadata = runChecked(run, "disk-image signing metadata inspection", "codesign", [
    "-dv",
    "--verbose=4",
    dmgPath,
  ]);
  inspectDeveloperIdMetadata(dmgMetadata, { requireRuntime: false, requireTicket: false });
  const dmgGatekeeper = runChecked(run, "disk-image Gatekeeper assessment", "spctl", [
    "--assess",
    "--type",
    "open",
    "--context",
    "context:primary-signature",
    "--verbose=4",
    dmgPath,
  ]);
  const dmgNotarized = gatekeeperReportsNotarizedDeveloperId(dmgGatekeeper);
  runChecked(run, "disk-image stapler validation", "xcrun", ["stapler", "validate", dmgPath]);

  return {
    kind: "signed_macos_release_verification",
    status: "passed",
    app: {
      signed: true,
      notarized: appNotarized,
      stapled: true,
      gatekeeperAccepted: true,
    },
    dmg: {
      signed: true,
      notarized: dmgNotarized,
      stapled: true,
      gatekeeperAccepted: true,
    },
  };
}

function main() {
  const { appPath, dmgPath } = parseSignedReleasePaths(process.argv.slice(2));
  console.log(JSON.stringify(verifySignedMacosRelease(appPath, dmgPath), null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`check-signed-macos-release: ${error.message}`);
    process.exit(1);
  }
}

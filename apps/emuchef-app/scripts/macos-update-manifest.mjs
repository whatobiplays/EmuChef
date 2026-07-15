#!/usr/bin/env node
/** Credentialed adapter for preparing and finalizing Phase 4B metadata. */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  canonicalUnsignedBytes,
  finalizeManifest,
  prepareUnsignedManifest,
  rejectPathLeakage,
  validateProductionTrust,
} from "./macos-update-manifest-policy.mjs";
import {
  fileSha256,
  verifyCredentialedUpdateArtifacts,
  workspacePaths,
} from "./macos-packaging.mjs";

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let index = 0; index < rest.length; index += 2) {
    const name = rest[index];
    const value = rest[index + 1];
    if (!name?.startsWith("--") || value === undefined) throw new Error("update manifest arguments are invalid");
    options[name.slice(2)] = value;
  }
  return { command, options };
}

function required(options, name) {
  if (!options[name]) throw new Error(`--${name} is required`);
  return options[name];
}

function readTrust(paths) {
  const document = JSON.parse(fs.readFileSync(path.join(paths.tauriDir, "update-trust.json"), "utf8"));
  const fixture = JSON.parse(fs.readFileSync(path.join(paths.appDir, "tests/fixtures/update-trust.json"), "utf8"));
  const trust = validateProductionTrust(document, [fixture]);
  if (!trust) throw new Error("production update trust is unconfigured");
  return trust;
}

function atomicWrite(destination, bytes) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const temporary = `${destination}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, bytes, { mode: 0o600 });
  fs.renameSync(temporary, destination);
}

function prepare(options) {
  const paths = workspacePaths();
  const trust = readTrust(paths);
  const appPath = path.resolve(required(options, "app"));
  const dmgPath = path.resolve(required(options, "dmg"));
  const output = path.resolve(required(options, "output"));
  const verification = verifyCredentialedUpdateArtifacts(appPath, dmgPath);
  const notesPath = path.resolve(required(options, "notes-file"));
  const notes = fs.readFileSync(notesPath, "utf8");
  const manifest = prepareUnsignedManifest({
    version: verification.app.info.CFBundleShortVersionString,
    publishedAt: required(options, "published-at"),
    expiresAt: required(options, "expires-at"),
    notes,
    dmgUrl: required(options, "dmg-url"),
    dmgSizeBytes: fs.statSync(dmgPath).size,
    dmgSha256: fileSha256(dmgPath),
    ...(options["minimum-macos-version"] ? { minimumMacosVersion: options["minimum-macos-version"] } : {}),
  }, trust);
  rejectPathLeakage(manifest, [paths.repoRoot, paths.appDir, appPath, dmgPath, notesPath, output]);
  atomicWrite(output, canonicalUnsignedBytes(manifest));
}

function finalize(options) {
  const paths = workspacePaths();
  const trust = readTrust(paths);
  const unsignedPath = path.resolve(required(options, "unsigned"));
  const signaturePath = path.resolve(required(options, "signature"));
  const output = path.resolve(required(options, "output"));
  const unsignedBytes = fs.readFileSync(unsignedPath);
  const unsigned = JSON.parse(unsignedBytes.toString("utf8"));
  if (!canonicalUnsignedBytes(unsigned).equals(unsignedBytes)) throw new Error("unsigned manifest is not canonical");
  const finalized = finalizeManifest(unsigned, fs.readFileSync(signaturePath, "utf8"), trust);
  rejectPathLeakage(JSON.parse(finalized.toString("utf8")), [paths.repoRoot, unsignedPath, signaturePath, output]);
  atomicWrite(output, finalized);
}

try {
  const { command, options } = parseArgs(process.argv.slice(2));
  if (command === "prepare") prepare(options);
  else if (command === "finalize") finalize(options);
  else throw new Error("expected prepare or finalize");
} catch (error) {
  console.error(`Phase 4B update manifest failed: ${error instanceof Error ? error.message : "unknown error"}`);
  process.exitCode = 1;
}

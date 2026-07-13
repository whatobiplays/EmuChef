#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REQUIRED_VARIABLES = [
  "APPLE_SIGNING_IDENTITY",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
];
const FORBIDDEN_EXTENSIONS = new Set([".p8", ".p12", ".cer", ".mobileprovision"]);
const OBVIOUS_CREDENTIAL_NAMES = [
  /^AuthKey_.+/i,
  /^app(?:-|_)?store(?:-|_)?connect(?:-|_).*(?:key|credential)/i,
  /^apple(?:-|_).*(?:api|signing).*(?:key|credential)/i,
];

function defaultRepositoryRoot() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  return path.resolve(scriptDir, "../../..");
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== "..");
}

function credentialFilename(name) {
  return (
    FORBIDDEN_EXTENSIONS.has(path.extname(name).toLowerCase()) ||
    OBVIOUS_CREDENTIAL_NAMES.some((pattern) => pattern.test(name))
  );
}

/**
 * Finds credential-shaped files without reading their contents or returning
 * their paths to the command-line caller.
 */
export function findRepositoryCredentialFiles(repositoryRoot, fsApi = fs) {
  const matches = [];
  const visit = (directory) => {
    for (const entry of fsApi.readdirSync(directory, { withFileTypes: true })) {
      if (directory === repositoryRoot && entry.name === ".git") {
        continue;
      }
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile() && credentialFilename(entry.name)) {
        matches.push(absolute);
      }
    }
  };
  visit(repositoryRoot);
  return matches;
}

/**
 * Enforces the repository-owned portion of the release credential policy.
 * Errors report only a count so credential-shaped filenames never enter logs.
 */
export function assertNoRepositoryCredentialFiles(repositoryRoot, fsApi = fs) {
  const matches = findRepositoryCredentialFiles(repositoryRoot, fsApi);
  if (matches.length > 0) {
    throw new Error(`repository contains ${matches.length} forbidden credential file(s)`);
  }
  return true;
}

function requireVariable(env, name) {
  if (typeof env[name] !== "string" || env[name].trim() === "") {
    throw new Error(`${name} must be set`);
  }
  return env[name];
}

/**
 * Validates Apple release inputs without returning or printing secret values.
 * The command runner is injectable so unit tests never access the host
 * Keychain.
 */
export function validateAppleReleaseEnvironment({
  env = process.env,
  repositoryRoot = defaultRepositoryRoot(),
  fsApi = fs,
  run = spawnSync,
} = {}) {
  const values = Object.fromEntries(
    REQUIRED_VARIABLES.map((name) => [name, requireVariable(env, name)]),
  );

  const repositoryRealPath = fsApi.realpathSync(repositoryRoot);
  let keyStat;
  let keyRealPath;
  try {
    keyStat = fsApi.lstatSync(values.APPLE_API_KEY_PATH);
    keyRealPath = fsApi.realpathSync(values.APPLE_API_KEY_PATH);
  } catch {
    throw new Error("APPLE_API_KEY_PATH must reference an existing private key file");
  }
  if (!keyStat.isFile() || keyStat.isSymbolicLink()) {
    throw new Error("APPLE_API_KEY_PATH must reference a regular non-symlink file");
  }
  if (isInside(repositoryRealPath, keyRealPath)) {
    throw new Error("APPLE_API_KEY_PATH must be outside the repository");
  }
  if ((keyStat.mode & 0o077) !== 0) {
    throw new Error("APPLE_API_KEY_PATH must not grant group or world permissions");
  }

  const identity = values.APPLE_SIGNING_IDENTITY;
  if (!identity.startsWith("Developer ID Application:")) {
    throw new Error("APPLE_SIGNING_IDENTITY must select a Developer ID Application identity");
  }
  const identityResult = run("security", ["find-identity", "-v", "-p", "codesigning"], {
    encoding: "utf8",
  });
  if (identityResult.error || identityResult.status !== 0) {
    throw new Error("security could not enumerate code-signing identities");
  }
  const identityOutput = `${identityResult.stdout || ""}\n${identityResult.stderr || ""}`;
  if (!identityOutput.includes(identity)) {
    throw new Error("APPLE_SIGNING_IDENTITY was not found in the code-signing Keychain");
  }

  assertNoRepositoryCredentialFiles(repositoryRealPath, fsApi);

  return {
    kind: "apple_release_environment_validation",
    status: "passed",
    checks: {
      requiredVariablesPresent: true,
      keyFileExists: true,
      keyFileIsRegular: true,
      keyFileOutsideRepository: true,
      keyFilePermissionsRestricted: true,
      developerIdApplicationIdentityPresent: true,
      repositoryCredentialFilesAbsent: true,
    },
  };
}

function main() {
  const result = validateAppleReleaseEnvironment();
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`check-apple-release-env: ${error.message}`);
    process.exit(1);
  }
}

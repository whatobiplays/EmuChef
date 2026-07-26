#!/usr/bin/env node

/**
 * Verify the semantic and integrity contract for the Phase 6C.1 Android
 * fixture. The verifier intentionally compares rebuilt APK facts rather than
 * APK bytes because Android packaging timestamps are not reproducible here.
 */
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REPOSITORY_ROOT = fileURLToPath(new URL("../", import.meta.url));
const PRODUCTION_SIGNING_SURFACES = [
  ".github/workflows",
  "apps/emuchef-app/package.json",
  "apps/emuchef-app/scripts",
  "apps/emuchef-app/src-tauri",
  "scripts",
];
const SIGNING_SCAN_EXTENSIONS = new Set([
  ".json",
  ".js",
  ".mjs",
  ".rs",
  ".sh",
  ".toml",
  ".ts",
  ".tsx",
  ".yaml",
  ".yml",
]);
const SIGNING_SCAN_EXCLUSIONS = new Set([
  "scripts/build-phase-6c-android-fixture.sh",
]);

/** Normalize an SDK-tool SHA-256 representation to lowercase hexadecimal. */
export function normalizeSha256(value) {
  const normalized = value.replace(/[\s:]/g, "").toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(normalized)) {
    throw new Error("Expected a 64-character SHA-256 digest.");
  }
  return normalized;
}

/**
 * Parse the small, stable subset of aapt2/apksigner output used by the
 * checked-in fixture contract. Tool output is treated as untrusted input so a
 * missing required fact is an explicit verification failure.
 */
export function fixtureMetadataFromToolOutput({ badging, certificate }) {
  const packageMatch = badging.match(
    /^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'/m,
  );
  const launcherMatch = badging.match(/^launchable-activity: name='([^']+)'/m);
  const minSdkMatch = badging.match(/^minSdkVersion:'([0-9]+)'$/m);
  const targetSdkMatch = badging.match(/^targetSdkVersion:'([0-9]+)'$/m);
  const permissions = [...badging.matchAll(/^uses-permission: name='([^']+)'/gm)]
    .map((match) => match[1])
    .sort();
  const certificateMatch = certificate.match(/certificate SHA-256 digest:\s*([^\n\r]+)/i);

  if (!packageMatch || !launcherMatch) {
    throw new Error("SDK tools did not report the required fixture metadata.");
  }
  if (!minSdkMatch || !targetSdkMatch) {
    throw new Error("SDK tools did not report valid minimum and target SDK metadata.");
  }
  if (!certificateMatch) {
    throw new Error("SDK tools did not report the required fixture signing metadata.");
  }
  const versionCode = Number.parseInt(packageMatch[2], 10);
  const minSdkVersion = Number.parseInt(minSdkMatch[1], 10);
  const targetSdkVersion = Number.parseInt(targetSdkMatch[1], 10);
  if (
    !Number.isSafeInteger(versionCode)
    || !Number.isSafeInteger(minSdkVersion)
    || !Number.isSafeInteger(targetSdkVersion)
  ) {
    throw new Error("SDK tools reported an invalid versionCode.");
  }

  return {
    packageName: packageMatch[1],
    versionCode,
    versionName: packageMatch[3],
    minSdkVersion,
    targetSdkVersion,
    launcherActivity: launcherMatch[1],
    declaredPermissions: permissions,
    signingCertificateSha256: normalizeSha256(certificateMatch[1]),
  };
}

/** Fail closed when a committed fixture fact differs from the expected contract. */
export function validateFixtureMetadata(actual, expected) {
  for (const field of [
    "packageName",
    "versionCode",
    "versionName",
    "minSdkVersion",
    "targetSdkVersion",
    "launcherActivity",
    "signingCertificateSha256",
  ]) {
    if (actual[field] !== expected[field]) {
      throw new Error(`Fixture metadata mismatch for ${field}.`);
    }
  }
  if (
    !Array.isArray(actual.declaredPermissions)
    || !Array.isArray(expected.declaredPermissions)
    || actual.declaredPermissions.length !== expected.declaredPermissions.length
    || actual.declaredPermissions.some((permission, index) => permission !== expected.declaredPermissions[index])
  ) {
    throw new Error("Fixture metadata mismatch for declaredPermissions.");
  }
}

function productionSigningFiles(repositoryRoot) {
  const files = [];
  const visit = (candidate) => {
    const relative = path.relative(repositoryRoot, candidate).split(path.sep).join("/");
    if (SIGNING_SCAN_EXCLUSIONS.has(relative)) return;
    const metadata = lstatSync(candidate);
    if (metadata.isSymbolicLink()) {
      throw new Error(`Production signing scan rejects symbolic link: ${relative}.`);
    }
    if (metadata.isDirectory()) {
      if (["node_modules", "target", "dist", "coverage"].includes(path.basename(candidate))) {
        return;
      }
      for (const entry of readdirSync(candidate).sort()) {
        visit(path.join(candidate, entry));
      }
      return;
    }
    if (
      metadata.isFile()
      && (
        path.basename(candidate) === "package.json"
        || SIGNING_SCAN_EXTENSIONS.has(path.extname(candidate))
      )
    ) {
      files.push(candidate);
    }
  };
  for (const relative of PRODUCTION_SIGNING_SURFACES) {
    visit(path.join(repositoryRoot, relative));
  }
  return files;
}

/**
 * Return production files that reference the fixture-only signing filename or
 * alias. Qualification tooling and fixture build inputs are deliberately not
 * part of the production scan surface.
 */
export function findFixtureSigningReferences(repositoryRoot, signingKeystore) {
  const forbidden = [signingKeystore?.file, signingKeystore?.alias]
    .filter((value) => typeof value === "string" && value.length > 0);
  if (forbidden.length !== 2) {
    throw new Error("Fixture signing metadata must provide a filename and alias.");
  }
  return productionSigningFiles(repositoryRoot)
    .filter((candidate) => {
      const contents = readFileSync(candidate, "utf8");
      return forbidden.some((value) => contents.includes(value));
    })
    .map((candidate) => path.relative(repositoryRoot, candidate).split(path.sep).join("/"))
    .sort();
}

function requiredOption(name, args) {
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1]) {
    throw new Error(`Missing required option ${name}.`);
  }
  return args[index + 1];
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

/** Run the command-line verifier used by local qualification and CI. */
export function verifyFixtureContract(args) {
  const apk = requiredOption("--apk", args);
  const metadataPath = requiredOption("--metadata", args);
  const checksumPath = requiredOption("--checksum", args);
  const aapt2 = requiredOption("--aapt2", args);
  const apksigner = requiredOption("--apksigner", args);
  const expectedMetadata = JSON.parse(readFileSync(metadataPath, "utf8"));
  if (!args.includes("--semantic-only")) {
    const expectedChecksum = normalizeSha256(readFileSync(checksumPath, "utf8"));
    const actualChecksum = sha256File(apk);
    if (actualChecksum !== expectedChecksum) {
      throw new Error("Fixture APK checksum does not match the committed SHA-256 contract.");
    }
  }

  const badging = execFileSync(aapt2, ["dump", "badging", apk], { encoding: "utf8" });
  const certificate = execFileSync(apksigner, ["verify", "--print-certs", apk], {
    encoding: "utf8",
  });
  validateFixtureMetadata(
    fixtureMetadataFromToolOutput({ badging, certificate }),
    expectedMetadata,
  );
  const productionReferences = findFixtureSigningReferences(
    REPOSITORY_ROOT,
    expectedMetadata.signingKeystore,
  );
  if (productionReferences.length > 0) {
    throw new Error("Fixture signing identity is referenced by production configuration.");
  }
}

if (
  process.argv[1]
  && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  try {
    verifyFixtureContract(process.argv.slice(2));
    process.stdout.write("Phase 6C.1 fixture contract verified.\n");
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}

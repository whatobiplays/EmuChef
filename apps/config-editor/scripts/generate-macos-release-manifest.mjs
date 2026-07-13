#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { verifySignedMacosRelease } from "./check-signed-macos-release.mjs";

const FULL_SHA_PATTERN = /^[0-9a-fA-F]{40}$/;

function defaultRepositoryRoot() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  return path.resolve(scriptDir, "../../..");
}

export function parseManifestArguments(argv) {
  const positional = [];
  let buildCommit;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--build-commit") {
      if (buildCommit !== undefined || index + 1 >= argv.length) {
        throw new Error("--build-commit requires exactly one value");
      }
      buildCommit = argv[index + 1];
      index += 1;
    } else if (argument.startsWith("--")) {
      throw new Error(`unknown option ${argument}`);
    } else {
      positional.push(argument);
    }
  }
  if (positional.length !== 3 || positional.some((value) => !value)) {
    throw new Error(
      "usage: generate-macos-release-manifest.mjs <app-path> <dmg-path> <output-path> [--build-commit <full-sha>]",
    );
  }
  if (buildCommit !== undefined && !FULL_SHA_PATTERN.test(buildCommit)) {
    throw new Error("--build-commit must be a full 40-character hexadecimal SHA");
  }
  return {
    appPath: path.resolve(positional[0]),
    dmgPath: path.resolve(positional[1]),
    outputPath: path.resolve(positional[2]),
    buildCommit,
  };
}

function runGit(run, repositoryRoot, args, label) {
  const result = run("git", args, { cwd: repositoryRoot, encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(label);
  }
  return (result.stdout || "").trim();
}

/** Resolves only locally available commits and never accepts abbreviated SHAs. */
export function resolveBuildCommit({ buildCommit, repositoryRoot, run = spawnSync }) {
  if (buildCommit !== undefined) {
    if (!FULL_SHA_PATTERN.test(buildCommit)) {
      throw new Error("--build-commit must be a full 40-character hexadecimal SHA");
    }
    runGit(
      run,
      repositoryRoot,
      ["cat-file", "-e", `${buildCommit}^{commit}`],
      "--build-commit does not resolve to a local commit",
    );
    return buildCommit.toLowerCase();
  }

  const trackedStatus = runGit(
    run,
    repositoryRoot,
    ["status", "--porcelain", "--untracked-files=no"],
    "tracked worktree status could not be determined",
  );
  if (trackedStatus !== "") {
    throw new Error("implicit HEAD selection requires a clean tracked worktree");
  }
  const head = runGit(
    run,
    repositoryRoot,
    ["rev-parse", "--verify", "HEAD"],
    "current HEAD could not be resolved",
  );
  if (!FULL_SHA_PATTERN.test(head)) {
    throw new Error("current HEAD did not resolve to a full commit SHA");
  }
  return head.toLowerCase();
}

export function sha256File(filePath, fsApi = fs) {
  return crypto.createHash("sha256").update(fsApi.readFileSync(filePath)).digest("hex");
}

function regularFiles(root, fsApi) {
  const files = [];
  const visit = (directory) => {
    for (const entry of fsApi.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile()) {
        files.push(absolute);
      }
    }
  };
  visit(root);
  return files;
}

/**
 * Hashes canonical records of each sorted relative regular-file path and its
 * content digest. Absolute paths never influence the result.
 */
export function appTreeDigest(appPath, fsApi = fs) {
  const records = regularFiles(appPath, fsApi)
    .map((absolute) => ({
      absolute,
      relative: path.relative(appPath, absolute).split(path.sep).join("/"),
    }))
    .sort((left, right) =>
      Buffer.compare(Buffer.from(left.relative, "utf8"), Buffer.from(right.relative, "utf8")),
    )
    .map(({ absolute, relative }) => `${sha256File(absolute, fsApi)}  ${relative}\n`)
    .join("");
  return crypto.createHash("sha256").update(records).digest("hex");
}

function runSafe(run, command, args, label) {
  const result = run(command, args, { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(label);
  }
  return `${result.stdout || ""}\n${result.stderr || ""}`.trim();
}

function readInfoPlist(infoPath, run) {
  const output = runSafe(
    run,
    "plutil",
    ["-convert", "json", "-o", "-", infoPath],
    "release metadata could not be read",
  );
  try {
    return JSON.parse(output);
  } catch {
    throw new Error("release metadata was not valid JSON");
  }
}

function requireMetadata(info, name) {
  if (typeof info[name] !== "string" || info[name].trim() === "") {
    throw new Error(`Info.plist must define ${name}`);
  }
  return info[name];
}

function assertSafeManifest(manifest, forbiddenPaths) {
  const serialized = JSON.stringify(manifest);
  for (const forbiddenPath of forbiddenPaths) {
    if (serialized.includes(forbiddenPath)) {
      throw new Error("release manifest must not contain absolute artifact paths");
    }
  }
  if (/TeamIdentifier|submission(?:Id| ID)|certificateSubject|APPLE_API_/i.test(serialized)) {
    throw new Error("release manifest contained forbidden signing metadata");
  }
}

function writeJsonAtomically(outputPath, manifest, fsApi) {
  const directory = path.dirname(outputPath);
  fsApi.mkdirSync(directory, { recursive: true });
  const temporaryPath = path.join(
    directory,
    `.${path.basename(outputPath)}.${process.pid}.${crypto.randomBytes(6).toString("hex")}.tmp`,
  );
  try {
    fsApi.writeFileSync(temporaryPath, `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o644 });
    fsApi.renameSync(temporaryPath, outputPath);
  } catch (error) {
    fsApi.rmSync(temporaryPath, { force: true });
    throw error;
  }
}

/** Generates a verified, path-safe release manifest and returns its data. */
export function generateMacosReleaseManifest(
  { appPath, dmgPath, outputPath, buildCommit },
  {
    fsApi = fs,
    now = () => new Date(),
    repositoryRoot = defaultRepositoryRoot(),
    run = spawnSync,
    verify = verifySignedMacosRelease,
  } = {},
) {
  const verification = verify(appPath, dmgPath, { fsApi, run });
  const resolvedCommit = resolveBuildCommit({ buildCommit, repositoryRoot, run });
  const infoPath = path.join(appPath, "Contents", "Info.plist");
  const info = readInfoPlist(infoPath, run);
  const executableName = requireMetadata(info, "CFBundleExecutable");
  const productName = requireMetadata(info, "CFBundleDisplayName");
  const bundleIdentifier = requireMetadata(info, "CFBundleIdentifier");
  const applicationVersion = requireMetadata(info, "CFBundleShortVersionString");
  const mainRelativePath = `Contents/MacOS/${executableName}`;
  const sidecarRelativePath = "Contents/MacOS/emuchef";
  const hostArchitecture = runSafe(run, "uname", ["-m"], "host architecture could not be read");

  const manifest = {
    schemaVersion: 1,
    productName,
    bundleIdentifier,
    applicationVersion,
    buildCommitSha: resolvedCommit,
    hostArchitecture,
    app: {
      name: path.basename(appPath),
      mainExecutable: {
        relativePath: mainRelativePath,
        sha256: sha256File(path.join(appPath, mainRelativePath), fsApi),
      },
      sidecar: {
        relativePath: sidecarRelativePath,
        sha256: sha256File(path.join(appPath, sidecarRelativePath), fsApi),
      },
      treeSha256: appTreeDigest(appPath, fsApi),
    },
    dmg: {
      name: path.basename(dmgPath),
      sha256: sha256File(dmgPath, fsApi),
    },
    verification: {
      app: verification.app,
      dmg: verification.dmg,
      signingPassed: verification.app.signed && verification.dmg.signed,
      notarizationPassed: verification.app.notarized && verification.dmg.notarized,
      staplingPassed: verification.app.stapled && verification.dmg.stapled,
      gatekeeperPassed:
        verification.app.gatekeeperAccepted && verification.dmg.gatekeeperAccepted,
    },
    generatedAtUtc: now().toISOString(),
  };
  assertSafeManifest(manifest, [appPath, dmgPath, outputPath]);
  writeJsonAtomically(outputPath, manifest, fsApi);
  return manifest;
}

function main() {
  const options = parseManifestArguments(process.argv.slice(2));
  const manifest = generateMacosReleaseManifest(options);
  console.log(
    JSON.stringify(
      {
        kind: "macos_release_manifest_generation",
        status: "passed",
        manifest: path.basename(options.outputPath),
        schemaVersion: manifest.schemaVersion,
      },
      null,
      2,
    ),
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`generate-macos-release-manifest: ${error.message}`);
    process.exit(1);
  }
}

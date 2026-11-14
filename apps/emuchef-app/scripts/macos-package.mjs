#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  APPLE_VARIABLE_ALLOWLIST,
  appTreeDigest,
  compareReleaseManifests,
  createReleaseManifest,
  developerIdBuildEnvironment,
  discoverArtifacts,
  fileSha256,
  inspectNormalizedContent,
  normalizedBuildEnvironment,
  parseOptions,
  provenance,
 resetBundleOutputForBuild,
  runChecked,
  runPreflight,
  validateQualificationProbe,
  verifyCredentialedRelease,
  verifyMacosBundle,
  workspacePaths,
  writeManifest,
} from "./macos-packaging.mjs";
import { assertReleaseSidecarBuildAllowed } from "./prepare-rust-sidecar.mjs";
import { withReleasePackageLock } from "./sidecar-build-contract.mjs";

function artifactOptions(options, paths) {
  if (options.app && options.dmg) {
    return { appPath: path.resolve(options.app), dmgPath: path.resolve(options.dmg) };
  }
  if (options.app || options.dmg) throw new Error("--app and --dmg must be supplied together");
  return discoverArtifacts(paths.bundleRoot);
}

function safeResult(kind, details = {}) {
  console.log(JSON.stringify({ kind, status: "passed", ...details }, null, 2));
}

async function build(options, paths) {
  return withReleasePackageLock({
    repoRoot: paths.repoRoot,
    env: process.env,
    artifactPaths: [
      "apps/emuchef-app/src-tauri/binaries",
      path.relative(paths.repoRoot, paths.bundleRoot).split(path.sep).join("/"),
    ],
    run: async (packageLockEnv, { lockFileDescriptor }) => {
      runPreflight({ paths });
      if (options.mode === "developer-id") developerIdBuildEnvironment(process.env);
      const env = normalizedBuildEnvironment(packageLockEnv, {
        repoRoot: paths.repoRoot,
        homeDir: os.homedir(),
        mode: options.mode,
      });
      resetBundleOutputForBuild({
        paths,
        assertBuildAllowed: () =>
          assertReleaseSidecarBuildAllowed({
            repoRoot: paths.repoRoot,
            env: packageLockEnv,
          }),
      });
      const tauri = path.join(paths.appDir, "node_modules", ".bin", "tauri");
      // The shared sidecar lock remains held while Tauri runs its build hook
      // and copies the release sidecar into the bundle.
      const result = spawnSync(tauri, ["build", "--bundles", "app,dmg", "--ci"], {
        cwd: paths.appDir,
        env,
        encoding: "utf8",
        maxBuffer: 128 * 1024 * 1024,
        stdio: ["inherit", "pipe", "pipe", lockFileDescriptor],
      });
      if (result.error || result.status !== 0) {
        throw new Error("Tauri macOS packaging failed");
      }
      const artifacts = discoverArtifacts(paths.bundleRoot);
      safeResult("macos_package_build", {
        mode: options.mode,
        app: path.relative(paths.repoRoot, artifacts.appPath),
        dmg: path.relative(paths.repoRoot, artifacts.dmgPath),
      });
      return artifacts;
    },
  });
}

function verify(options, paths, artifacts = artifactOptions(options, paths)) {
  const result = verifyMacosBundle(artifacts.appPath, { paths });
  safeResult("macos_bundle_verification", {
    app: path.basename(artifacts.appPath),
    architecture: result.policy.architecture,
    catalogFiles: result.catalog.fileCount,
    catalogSha256: result.catalog.sha256,
    signingState: result.signingState,
  });
  return result;
}

function smoke(options, paths, artifacts = artifactOptions(options, paths), verification) {
  const checked = verification ?? verifyMacosBundle(artifacts.appPath, { paths });
  // Tauri rejects executable paths containing a symlink on macOS. `/var` and
  // `/tmp` are commonly symlinks, so materialize the copy below the canonical
  // temporary root just as a clean installation path would be materialized.
  const temporary = fs.mkdtempSync(
    path.join(fs.realpathSync(os.tmpdir()), "emuchef-clean-qualification-"),
  );
  const copiedApp = path.join(temporary, path.basename(artifacts.appPath));
  try {
    runChecked("ditto", [artifacts.appPath, copiedApp]);
    const before = appTreeDigest(copiedApp);
    const mainPath = path.join(copiedApp, "Contents", "MacOS", checked.info.CFBundleExecutable);
    const home = path.join(temporary, "home");
    const tmp = path.join(temporary, "tmp");
    fs.mkdirSync(home, { recursive: true });
    fs.mkdirSync(tmp, { recursive: true });
    const env = {
      ...process.env,
      HOME: home,
      CFFIXED_USER_HOME: home,
      TMPDIR: tmp,
      XDG_DATA_HOME: path.join(home, ".local", "share"),
      XDG_CACHE_HOME: path.join(home, ".cache"),
    };
    for (const name of APPLE_VARIABLE_ALLOWLIST) delete env[name];
    const probe = spawnSync(mainPath, ["--qualification-probe"], {
      cwd: temporary,
      env,
      encoding: "utf8",
      timeout: 20_000,
      maxBuffer: 1024 * 1024,
    });
    if (probe.error || probe.status !== 0) throw new Error("packaged qualification probe failed");
    const reportLine = probe.stdout
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line.includes('"kind":"macos_packaged_app_qualification"'));
    if (!reportLine) throw new Error("packaged qualification probe returned no report");
    const report = validateQualificationProbe(JSON.parse(reportLine));
    if (appTreeDigest(copiedApp) !== before) throw new Error("qualification modified copied app contents");
    const processList = runChecked("ps", ["-axo", "command="], { allowFailure: true }).stdout;
    if (processList.includes(`${copiedApp}/Contents/MacOS/emuchef --sidecar`)) {
      throw new Error("packaged sidecar survived qualification probe exit");
    }
    safeResult("macos_clean_environment_smoke", {
      app: path.basename(copiedApp),
      runtimeReady: report.runtimeReady,
      catalogLoaded: report.catalogLoaded,
      readOnlyCatalogOperation: report.readOnlyCatalogOperation,
      realExecutionEnabled: report.realExecutionEnabled,
    });
    return report;
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function manifest(options, paths, artifacts = artifactOptions(options, paths), verification) {
  const checked = verification ?? verifyMacosBundle(artifacts.appPath, { paths });
  const normalized = inspectNormalizedContent(checked, { paths });
  const releaseManifest = createReleaseManifest({
    normalized,
    rawArtifacts: {
      app: {
        name: path.basename(artifacts.appPath),
        treeSha256: appTreeDigest(artifacts.appPath),
        mainExecutableSha256: fileSha256(checked.mainPath),
        sidecarSha256: fileSha256(checked.sidecarPath),
        identityScope: "this-produced-build-only",
      },
      dmg: {
        name: path.basename(artifacts.dmgPath),
        sha256: fileSha256(artifacts.dmgPath),
        identityScope: "this-produced-build-only",
      },
    },
    provenance: provenance({ paths }),
    signingState: checked.signingState,
  });
  const output = path.resolve(
    options.manifest ?? path.join(paths.bundleRoot, "emuchef-macos-arm64-qualification.json"),
  );
  const forbiddenValues =
    options.mode === "developer-id"
      ? APPLE_VARIABLE_ALLOWLIST.map((name) => process.env[name]).filter(Boolean)
      : [];
  writeManifest(output, releaseManifest, { forbiddenValues });
  safeResult("macos_release_manifest", {
    manifest: path.relative(paths.repoRoot, output),
    normalizedContentSha256: normalized.normalizedContentSha256,
    rawHashesArePerBuildIdentities: true,
    byteIdenticalSignedArtifactsClaimed: false,
  });
  return { output, releaseManifest };
}

function compare(options) {
  if (options.positional.length !== 2) {
    throw new Error("compare requires two release-manifest paths");
  }
  const [left, right] = options.positional.map((value) =>
    JSON.parse(fs.readFileSync(path.resolve(value), "utf8")),
  );
  const result = compareReleaseManifests(left, right);
  console.log(JSON.stringify({ kind: "macos_normalized_repeatability", status: result.normalizedContentMatches ? "passed" : "failed", ...result }, null, 2));
  if (!result.normalizedContentMatches) process.exitCode = 1;
}

async function main() {
  const [action, ...argv] = process.argv.slice(2);
  const options = parseOptions(argv);
  const paths = workspacePaths();
  switch (action) {
    case "preflight": {
      const result = runPreflight({ paths });
      safeResult("macos_package_preflight", result);
      break;
    }
    case "build":
      await build(options, paths);
      break;
    case "verify":
      verify(options, paths);
      break;
    case "smoke":
      smoke(options, paths);
      break;
    case "manifest": {
      const artifacts = artifactOptions(options, paths);
      const verification = verifyMacosBundle(artifacts.appPath, { paths });
      smoke(options, paths, artifacts, verification);
      manifest(options, paths, artifacts, verification);
      break;
    }
    case "qualify": {
      const artifacts = await build(options, paths);
      const verification = verify(options, paths, artifacts);
      smoke(options, paths, artifacts, verification);
      manifest(options, paths, artifacts, verification);
      break;
    }
    case "release-verify": {
      const artifacts = artifactOptions(options, paths);
      verifyMacosBundle(artifacts.appPath, { paths });
      const result = verifyCredentialedRelease(artifacts.appPath, artifacts.dmgPath);
      safeResult("macos_credentialed_release_verification", result);
      break;
    }
    case "compare":
      compare(options);
      break;
    default:
      throw new Error(
        "usage: macos-package.mjs <preflight|build|verify|smoke|manifest|qualify|release-verify|compare> [options]",
      );
  }
}

main().catch((error) => {
  console.error(`macos-package: ${error.message}`);
  process.exit(1);
});

/**
 * macOS filesystem and command adapters for the pure packaging policy.
 *
 * These adapters are excluded from percentage coverage because they are bound
 * to macOS tools and real bundle layouts. Release assurance for this file is
 * the mandatory package:macos:qualify integration run.
 */
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { validateTargetTriple } from "./sidecar-packaging.mjs";
import * as packagingPolicy from "./macos-packaging-policy.mjs";

export const APPLE_VARIABLE_ALLOWLIST = packagingPolicy.APPLE_VARIABLE_ALLOWLIST;
export const QUALIFICATION_POLICY_VERSION = packagingPolicy.QUALIFICATION_POLICY_VERSION;
const REQUIRED_CATALOG_DIRECTORIES = packagingPolicy.REQUIRED_CATALOG_DIRECTORIES;
const buildNormalizedContent = packagingPolicy.buildNormalizedContent;
const classifySigningState = packagingPolicy.classifySigningState;
const selectArtifacts = packagingPolicy.selectArtifacts;
const validateDeveloperIdMetadata = packagingPolicy.validateDeveloperIdMetadata;
const validateInfoPlist = packagingPolicy.validateInfoPlist;
const validatePackagedPolicy = packagingPolicy.validatePackagedPolicy;

const MAX_COMMAND_OUTPUT = 128 * 1024 * 1024;

export function workspacePaths() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const appDir = path.resolve(scriptDir, "..");
  const repoRoot = path.resolve(appDir, "../..");
  const tauriDir = path.join(appDir, "src-tauri");
  return {
    appDir,
    repoRoot,
    tauriDir,
    bundleRoot: path.join(tauriDir, "target", "release", "bundle"),
    policyInput: path.join(tauriDir, "qualification-policy.json"),
  };
}

/**
 * Produces a local build environment without inspecting or validating Apple
 * credential values. Fixed Tauri variable names are removed and ad-hoc signing
 * is selected explicitly, so ambient credentials cannot select release mode.
 */
export function localBuildEnvironment(env) {
  return packagingPolicy.localBuildEnvironment(env);
}

/** Validates only the documented allowlist after developer-id mode is selected. */
export function validateDeveloperIdEnvironment(env) {
  return packagingPolicy.validateDeveloperIdEnvironment(env);
}

export function developerIdBuildEnvironment(env) {
  return packagingPolicy.developerIdBuildEnvironment(env);
}

/** Clears caller-dependent compiler settings and installs stable path remaps. */
export function normalizedBuildEnvironment(env, { repoRoot, homeDir, mode }) {
  return packagingPolicy.normalizedBuildEnvironment(env, { repoRoot, homeDir, mode });
}

export function parseOptions(argv) {
  return packagingPolicy.parseOptions(argv);
}

export function validatePackagingConfiguration({ packageJson, tauriConfig, cargoToml, targetTriple }) {
  return packagingPolicy.validatePackagingConfiguration({
    packageJson,
    tauriConfig,
    cargoToml,
    targetTriple,
  });
}

export function qualificationPolicy({ appVersion, architecture, targetTriple }) {
  return packagingPolicy.qualificationPolicy({ appVersion, architecture, targetTriple });
}

export function canonicalJson(value) {
  return packagingPolicy.canonicalJson(value);
}

export function sha256Bytes(bytes) {
  return packagingPolicy.sha256Bytes(bytes);
}

export function semanticDigest(value) {
  return packagingPolicy.semanticDigest(value);
}

export function normalizedContentManifest(content) {
  return packagingPolicy.normalizedContentManifest(content);
}

export function createReleaseManifest({ normalized, rawArtifacts, provenance, signingState }) {
  return packagingPolicy.createReleaseManifest({
    normalized,
    rawArtifacts,
    provenance,
    signingState,
  });
}

export function compareReleaseManifests(left, right) {
  return packagingPolicy.compareReleaseManifests(left, right);
}

export function validateQualificationProbe(report) {
  return packagingPolicy.validateQualificationProbe(report);
}

export function assertSafeManifest(manifest, { forbiddenValues = [] } = {}) {
  return packagingPolicy.assertSafeManifest(manifest, { forbiddenValues });
}

export function runChecked(command, args, { cwd, env, allowFailure = false, run = spawnSync } = {}) {
  const result = run(command, args, {
    cwd,
    env,
    encoding: "utf8",
    maxBuffer: MAX_COMMAND_OUTPUT,
  });
  if (result.error) throw new Error(`${command} could not run`);
  if (!allowFailure && result.status !== 0) {
    throw new Error(`${command} failed with exit ${result.status}`);
  }
  return result;
}

export function hostTargetTriple(run = spawnSync) {
  const result = runChecked("rustc", ["--print", "host-tuple"], { run });
  return validateTargetTriple(result.stdout.trim(), "rustc");
}

export function runPreflight({ paths = workspacePaths(), run = spawnSync, fsApi = fs } = {}) {
  const targetTriple = hostTargetTriple(run);
  const packageJson = JSON.parse(fsApi.readFileSync(path.join(paths.appDir, "package.json"), "utf8"));
  const tauriConfig = JSON.parse(
    fsApi.readFileSync(path.join(paths.tauriDir, "tauri.conf.json"), "utf8"),
  );
  const cargoToml = fsApi.readFileSync(path.join(paths.tauriDir, "Cargo.toml"), "utf8");
  const result = validatePackagingConfiguration({ packageJson, tauriConfig, cargoToml, targetTriple });
  for (const icon of ["icons/icon.icns", "icons/icon.png"]) {
    if (!fsApi.statSync(path.join(paths.tauriDir, icon)).isFile()) {
      throw new Error(`required icon '${icon}' is missing`);
    }
  }
  for (const directory of REQUIRED_CATALOG_DIRECTORIES) {
    if (!fsApi.statSync(path.join(paths.repoRoot, "authored", directory)).isDirectory()) {
      throw new Error(`authored catalog directory '${directory}' is missing`);
    }
  }
  const checkedInPolicy = JSON.parse(fsApi.readFileSync(paths.policyInput, "utf8"));
  if (canonicalJson(checkedInPolicy) !== canonicalJson(qualificationPolicy(result))) {
    throw new Error("checked-in qualification policy does not match packaging configuration");
  }
  return result;
}

function regularFiles(root, { rejectSymlinks = false, fsApi = fs } = {}) {
  const files = [];
  const visit = (directory) => {
    for (const entry of fsApi.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      const metadata = fsApi.lstatSync(absolute);
      if (metadata.isSymbolicLink()) {
        if (rejectSymlinks) throw new Error(`bundle contains symlink '${path.relative(root, absolute)}'`);
      } else if (metadata.isDirectory()) {
        visit(absolute);
      } else if (metadata.isFile()) {
        files.push(absolute);
      } else {
        throw new Error(`bundle contains unsupported entry '${path.relative(root, absolute)}'`);
      }
    }
  };
  visit(root);
  return files;
}

export function fileSha256(filePath, fsApi = fs) {
  return sha256Bytes(fsApi.readFileSync(filePath));
}

export function appTreeDigest(appPath, fsApi = fs) {
  const records = regularFiles(appPath, { fsApi })
    .map((absolute) => ({
      absolute,
      relative: path.relative(appPath, absolute).split(path.sep).join("/"),
    }))
    .sort((left, right) => Buffer.compare(Buffer.from(left.relative), Buffer.from(right.relative)))
    .map(({ absolute, relative }) => `${fileSha256(absolute, fsApi)}  ${relative}\n`)
    .join("");
  return sha256Bytes(records);
}

export function catalogDigest(catalogRoot, fsApi = fs) {
  const root = fsApi.realpathSync(catalogRoot);
  const records = [];
  for (const directory of REQUIRED_CATALOG_DIRECTORIES) {
    const directoryPath = path.join(root, directory);
    let metadata;
    try {
      metadata = fsApi.statSync(directoryPath);
    } catch {
      throw new Error(`catalog/${directory} is missing`);
    }
    if (!metadata.isDirectory()) throw new Error(`catalog/${directory} is missing`);
    for (const absolute of regularFiles(directoryPath, { rejectSymlinks: true, fsApi })) {
      if (path.basename(absolute) === ".gitkeep") continue;
      if (!/\.ya?ml$/.test(absolute)) throw new Error("catalog contains unsupported content");
      records.push({ absolute, relative: path.relative(root, absolute).split(path.sep).join("/") });
    }
  }
  records.sort((left, right) => Buffer.compare(Buffer.from(left.relative), Buffer.from(right.relative)));
  const hasher = crypto.createHash("sha256");
  for (const { absolute, relative } of records) {
    const bytes = fsApi.readFileSync(absolute);
    hasher.update(`${Buffer.byteLength(relative)}:${relative}${bytes.length}:`);
    hasher.update(bytes);
  }
  return { fileCount: records.length, sha256: hasher.digest("hex") };
}

function plistInfo(infoPath, run) {
  const result = runChecked("plutil", ["-convert", "json", "-o", "-", infoPath], { run });
  return JSON.parse(result.stdout);
}

function signingStateFor(appPath, run) {
  const result = runChecked("codesign", ["-dv", "--verbose=4", appPath], {
    run,
    allowFailure: true,
  });
  const output = `${result.stdout || ""}\n${result.stderr || ""}`;
  return classifySigningState(result.status, output);
}

export function forbiddenBundleReason(relative) {
  return packagingPolicy.forbiddenBundleReason(relative);
}

export function forbiddenBinaryReason(value) {
  return packagingPolicy.forbiddenBinaryReason(value);
}

export function requireThinArm64(label, fileDescription) {
  return packagingPolicy.requireThinArm64(label, fileDescription);
}

export function verifyMacosBundle(appPath, { paths = workspacePaths(), run = spawnSync, fsApi = fs } = {}) {
  const resolvedApp = path.resolve(appPath);
  if (!resolvedApp.endsWith(".app") || !fsApi.statSync(resolvedApp).isDirectory()) {
    throw new Error("application bundle must be an existing .app directory");
  }
  const files = regularFiles(resolvedApp, { rejectSymlinks: true, fsApi });
  const relatives = files.map((value) => path.relative(resolvedApp, value).split(path.sep).join("/"));
  const forbidden = relatives.find(forbiddenBundleReason);
  if (forbidden) throw new Error(`bundle contains forbidden development content '${forbidden}'`);

  const infoPath = path.join(resolvedApp, "Contents", "Info.plist");
  const info = plistInfo(infoPath, run);
  const tauriConfig = JSON.parse(fsApi.readFileSync(path.join(paths.tauriDir, "tauri.conf.json"), "utf8"));
  validateInfoPlist(info, tauriConfig);
  const mainPath = path.join(resolvedApp, "Contents", "MacOS", info.CFBundleExecutable);
  const sidecarPath = path.join(resolvedApp, "Contents", "MacOS", "emuchef");
  for (const [label, executable] of [["main executable", mainPath], ["sidecar", sidecarPath]]) {
    if (!fsApi.statSync(executable).isFile()) throw new Error(`${label} is missing`);
    fsApi.accessSync(executable, fs.constants.X_OK);
    const type = runChecked("file", [executable], { run }).stdout;
    requireThinArm64(label, type);
    // `otool -L` prefixes its output with the inspected caller path. That
    // presentation-only first line is not an embedded dependency.
    const libraries = runChecked("otool", ["-L", executable], { run })
      .stdout.split("\n")
      .slice(1)
      .join("\n");
    const strings = runChecked("strings", ["-a", executable], { run }).stdout;
    if (forbiddenBinaryReason(`${libraries}\n${strings}`)) {
      throw new Error(`${label} contains a development, repository, or caller path leak`);
    }
    runChecked("codesign", ["--verify", "--strict", executable], { run });
  }
  runChecked("codesign", ["--verify", "--deep", "--strict", "--verbose=4", resolvedApp], { run });

  const catalog = catalogDigest(path.join(resolvedApp, "Contents", "Resources", "catalog"), fsApi);
  const policyPath = path.join(
    resolvedApp,
    "Contents",
    "Resources",
    "qualification",
    "qualification-policy.json",
  );
  const policy = JSON.parse(fsApi.readFileSync(policyPath, "utf8"));
  validatePackagedPolicy(policy, tauriConfig.version);
  return {
    appPath: resolvedApp,
    mainPath,
    sidecarPath,
    info,
    catalog,
    policy,
    signingState: signingStateFor(resolvedApp, run),
  };
}

function unsignedExecutableHash(executable, { run = spawnSync, fsApi = fs } = {}) {
  const temporary = fsApi.mkdtempSync(path.join(os.tmpdir(), "emuchef-unsigned-content-"));
  const copy = path.join(temporary, path.basename(executable));
  try {
    fsApi.copyFileSync(executable, copy);
    runChecked("codesign", ["--remove-signature", copy], { run });
    return fileSha256(copy, fsApi);
  } finally {
    fsApi.rmSync(temporary, { recursive: true, force: true });
  }
}

export function inspectNormalizedContent(verification, { paths = workspacePaths(), run = spawnSync, fsApi = fs } = {}) {
  const capability = JSON.parse(
    fsApi.readFileSync(path.join(paths.tauriDir, "capabilities", "default.json"), "utf8"),
  );
  const tauriConfig = JSON.parse(fsApi.readFileSync(path.join(paths.tauriDir, "tauri.conf.json"), "utf8"));
  return buildNormalizedContent(verification, capability, tauriConfig, {
    main: unsignedExecutableHash(verification.mainPath, { run, fsApi }),
    sidecar: unsignedExecutableHash(verification.sidecarPath, { run, fsApi }),
  });
}

export function discoverArtifacts(bundleRoot, fsApi = fs) {
  const apps = fsApi
    .readdirSync(path.join(bundleRoot, "macos"))
    .filter((name) => name.endsWith(".app"))
    .map((name) => path.join(bundleRoot, "macos", name));
  const dmgs = fsApi
    .readdirSync(path.join(bundleRoot, "dmg"))
    .filter((name) => name.endsWith(".dmg"))
    .map((name) => path.join(bundleRoot, "dmg", name));
  return selectArtifacts(apps, dmgs);
}

function safeVersion(command, args, { cwd, run }) {
  return runChecked(command, args, { cwd, run }).stdout.trim().split("\n")[0];
}

export function provenance({ paths = workspacePaths(), run = spawnSync } = {}) {
  const commitResult = runChecked("git", ["rev-parse", "--verify", "HEAD"], {
    cwd: paths.repoRoot,
    run,
    allowFailure: true,
  });
  const status = runChecked("git", ["status", "--porcelain", "--untracked-files=no"], {
    cwd: paths.repoRoot,
    run,
  }).stdout.trim();
  return {
    sourceCommit: commitResult.status === 0 ? commitResult.stdout.trim() : null,
    trackedWorktreeDirty: status !== "",
    targetTriple: hostTargetTriple(run),
    buildMode: "release",
    toolchain: {
      node: safeVersion("node", ["--version"], { cwd: paths.repoRoot, run }),
      npm: safeVersion("npm", ["--version"], { cwd: paths.repoRoot, run }),
      rustc: safeVersion("rustc", ["--version"], { cwd: paths.repoRoot, run }),
      cargo: safeVersion("cargo", ["--version"], { cwd: paths.repoRoot, run }),
      tauri: safeVersion(path.join(paths.appDir, "node_modules", ".bin", "tauri"), ["--version"], {
        cwd: paths.appDir,
        run,
      }),
    },
  };
}

export function writeManifest(filePath, manifest, { fsApi = fs, forbiddenValues = [] } = {}) {
  assertSafeManifest(manifest, { forbiddenValues });
  fsApi.mkdirSync(path.dirname(filePath), { recursive: true });
  const temporary = `${filePath}.${process.pid}.tmp`;
  fsApi.writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o644 });
  fsApi.renameSync(temporary, filePath);
}

export function verifyCredentialedRelease(appPath, dmgPath, { run = spawnSync } = {}) {
  runChecked("codesign", ["--verify", "--deep", "--strict", "--verbose=4", appPath], { run });
  const metadata = runChecked("codesign", ["-dv", "--verbose=4", appPath], { run });
  const output = `${metadata.stdout || ""}\n${metadata.stderr || ""}`;
  validateDeveloperIdMetadata(output);
  runChecked("spctl", ["--assess", "--type", "execute", "--verbose=4", appPath], { run });
  runChecked("xcrun", ["stapler", "validate", appPath], { run });
  runChecked("codesign", ["--verify", "--verbose=4", dmgPath], { run });
  runChecked("spctl", [
    "--assess",
    "--type",
    "open",
    "--context",
    "context:primary-signature",
    "--verbose=4",
    dmgPath,
  ], { run });
  runChecked("xcrun", ["stapler", "validate", dmgPath], { run });
  return { status: "passed", developerId: true, notarized: true, stapled: true, gatekeeper: true };
}

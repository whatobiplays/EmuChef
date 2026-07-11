#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const MAX_COMMAND_OUTPUT = 64 * 1024 * 1024;
const SIDECAR_NAME = "emuchef";

function run(command, args, { allowFailure = false } = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_COMMAND_OUTPUT,
  });
  if (result.error) {
    throw new Error(`failed to run ${command}: ${result.error.message}`);
  }
  if (!allowFailure && result.status !== 0) {
    throw new Error(
      `${command} exited ${result.status}: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  return result;
}

export function parseAppPath(argv) {
  if (argv.length !== 1 || !argv[0]) {
    throw new Error("usage: check-macos-bundle.mjs <path-to-app-bundle>");
  }
  return path.resolve(argv[0]);
}

export function resolveBundleLayout(appPath, info) {
  if (!appPath.endsWith(".app")) {
    throw new Error(`bundle path must end in .app: '${appPath}'`);
  }
  if (!info || typeof info.CFBundleExecutable !== "string" || !info.CFBundleExecutable) {
    throw new Error("Info.plist must define CFBundleExecutable");
  }
  return {
    appPath,
    infoPath: path.join(appPath, "Contents", "Info.plist"),
    mainPath: path.join(appPath, "Contents", "MacOS", info.CFBundleExecutable),
    sidecarPath: path.join(appPath, "Contents", "MacOS", SIDECAR_NAME),
  };
}

export function validateInfoPlist(info, expected) {
  const mismatches = [];
  for (const [field, expectedValue] of [
    ["CFBundleIdentifier", expected.identifier],
    ["CFBundleShortVersionString", expected.version],
    ["CFBundleVersion", expected.version],
    ["CFBundleDisplayName", expected.productName],
  ]) {
    if (info[field] !== expectedValue) {
      mismatches.push(`${field} was '${info[field]}' instead of '${expectedValue}'`);
    }
  }
  if (info.CFBundlePackageType !== "APPL") {
    mismatches.push(`CFBundlePackageType was '${info.CFBundlePackageType}' instead of 'APPL'`);
  }
  if (mismatches.length > 0) {
    throw new Error(`Info.plist validation failed: ${mismatches.join("; ")}`);
  }
}

export function architectureMatches(fileOutput, hostArchitecture) {
  const aliases = hostArchitecture === "x86_64" ? ["x86_64", "x86-64"] : [hostArchitecture];
  return aliases.some((alias) => fileOutput.includes(alias));
}

export function forbiddenPathReasons(relativePaths) {
  const reasons = [];
  for (const relativePath of relativePaths) {
    const lower = relativePath.toLowerCase();
    const basename = path.basename(lower);
    if (
      basename === "python" ||
      /^python3(?:\.[0-9]+)?$/.test(basename) ||
      lower.endsWith(".py") ||
      lower.endsWith(".pyc") ||
      lower.includes("python.framework")
    ) {
      reasons.push(`Python runtime path: ${relativePath}`);
    }
    if (lower.includes("emuchef-python-legacy") || lower.includes("plan_shadow") || lower.includes("emuchef-plan-shadow")) {
      reasons.push(`legacy or shadow runtime path: ${relativePath}`);
    }
  }
  return reasons;
}

export function forbiddenStringReasons(label, value) {
  const patterns = [
    [/(?:https?:\/\/)?localhost:[0-9]+/i, "localhost development-server URL"],
    [/(?:https?:\/\/)?127\.0\.0\.1:[0-9]+/i, "loopback development-server URL"],
    [/emuchef-python-legacy/i, "legacy Python runtime name"],
    [/emuchef-plan-shadow|plan_shadow/i, "shadow planner name"],
    [/Python\.framework|libpython[0-9.]*\.(?:dylib|so)/i, "Python dynamic runtime"],
  ];
  return patterns
    .filter(([pattern]) => pattern.test(value))
    .map(([, description]) => `${label}: ${description}`);
}

export function signingState(output, status) {
  if (status === 0 && /Signature=adhoc/.test(output)) {
    return "ad-hoc";
  }
  if (status !== 0 && /code object is not signed at all/i.test(output)) {
    return "unsigned";
  }
  throw new Error("bundle must be unsigned or ad-hoc signed for local validation");
}

function listRelativeFiles(root) {
  const files = [];
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else {
        files.push(path.relative(root, absolute));
      }
    }
  };
  visit(root);
  return files.sort();
}

function requireExecutable(filePath, label) {
  if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
    throw new Error(`${label} was not found at '${filePath}'`);
  }
  fs.accessSync(filePath, fs.constants.X_OK);
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function readPlist(infoPath) {
  const result = run("plutil", ["-convert", "json", "-o", "-", infoPath]);
  return JSON.parse(result.stdout);
}

export function inspectMacosBundle(appPath) {
  if (!fs.existsSync(appPath) || !fs.statSync(appPath).isDirectory()) {
    throw new Error(`application bundle was not found at '${appPath}'`);
  }

  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const appDir = path.resolve(scriptDir, "..");
  const tauriConfig = readJson(path.join(appDir, "src-tauri", "tauri.conf.json"));
  const infoPath = path.join(appPath, "Contents", "Info.plist");
  if (!fs.existsSync(infoPath)) {
    throw new Error(`Info.plist was not found at '${infoPath}'`);
  }
  const info = readPlist(infoPath);
  validateInfoPlist(info, {
    identifier: tauriConfig.identifier,
    productName: tauriConfig.productName,
    version: tauriConfig.version,
  });

  const layout = resolveBundleLayout(appPath, info);
  requireExecutable(layout.mainPath, "main executable");
  requireExecutable(layout.sidecarPath, "Rust sidecar");

  const hostArchitecture = run("uname", ["-m"]).stdout.trim();
  const mainFile = run("file", [layout.mainPath]).stdout.trim();
  const sidecarFile = run("file", [layout.sidecarPath]).stdout.trim();
  if (!architectureMatches(mainFile, hostArchitecture)) {
    throw new Error(`main executable architecture did not match host '${hostArchitecture}'`);
  }
  if (!architectureMatches(sidecarFile, hostArchitecture)) {
    throw new Error(`sidecar architecture did not match host '${hostArchitecture}'`);
  }

  const relativeFiles = listRelativeFiles(appPath);
  const reasons = forbiddenPathReasons(relativeFiles);
  const mainStrings = run("strings", ["-a", layout.mainPath]).stdout;
  const sidecarStrings = run("strings", ["-a", layout.sidecarPath]).stdout;
  const mainLibraries = run("otool", ["-L", layout.mainPath]).stdout;
  const sidecarLibraries = run("otool", ["-L", layout.sidecarPath]).stdout;
  reasons.push(
    ...forbiddenStringReasons("main executable", mainStrings),
    ...forbiddenStringReasons("Rust sidecar", sidecarStrings),
    ...forbiddenStringReasons("main dependencies", mainLibraries),
    ...forbiddenStringReasons("sidecar dependencies", sidecarLibraries),
  );
  if (!mainStrings.includes("index.html") || !mainStrings.includes(tauriConfig.productName)) {
    reasons.push("main executable did not contain expected embedded frontend markers");
  }
  if (reasons.length > 0) {
    throw new Error(`bundle contains forbidden or missing content: ${reasons.join("; ")}`);
  }

  const codesign = run("codesign", ["-dv", "--verbose=4", appPath], {
    allowFailure: true,
  });
  const signing = signingState(`${codesign.stdout}\n${codesign.stderr}`, codesign.status);

  return {
    appPath,
    hostArchitecture,
    identifier: info.CFBundleIdentifier,
    mainExecutable: info.CFBundleExecutable,
    mainFile,
    sidecarFile,
    signing,
    version: info.CFBundleShortVersionString,
  };
}

function main() {
  const appPath = parseAppPath(process.argv.slice(2));
  const result = inspectMacosBundle(appPath);
  console.log(JSON.stringify({ kind: "macos_bundle_inspection", status: "passed", ...result }, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`check-macos-bundle: ${error.message}`);
    process.exit(1);
  }
}


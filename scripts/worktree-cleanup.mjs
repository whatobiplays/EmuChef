#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  isCanonicalSidecarArtifactName,
  isCanonicalSidecarMetadataName,
  isKnownCargoTargetTriple,
} from "./sidecar-target-contract.mjs";

/**
 * Worktree-local cleanup allowlists for regenerable Cargo and Tauri outputs.
 * The scanner preserves release bundles, application state, tracked files,
 * unknown entries outside dependency roots, and the pending Cargo freshness
 * marker. Deep cleanup treats untracked dependency contents as regenerable,
 * including nested `.gitignore` files. Cleanup is not coordinated with active
 * builds, so callers should run it while build and test commands are idle.
 */
const TARGET_ROOTS = [
  "crates/emuchef-rust-backend/target",
  "apps/emuchef-app/src-tauri/target",
  "apps/config-editor/src-tauri/target",
];
const GEN_ROOTS = [
  "apps/emuchef-app/src-tauri/gen",
  "apps/config-editor/src-tauri/gen",
];
const TAURI_APP_ROOTS = ["apps/emuchef-app", "apps/config-editor"];
const DEPENDENCY_INSTALL_STAMP = ".emuchef-deps-stamp";
const PROFILE_NAMES = new Set(["debug", "release"]);
const OUTPUT_DIRECTORIES = new Set([".fingerprint", "build", "deps", "incremental"]);
const OBSOLETE_FRESHNESS_STAMP = ".emuchef-cargo-test-source.sha256";
const PENDING_FRESHNESS_MARKER = `${OBSOLETE_FRESHNESS_STAMP}.pending`;
const CARGO_CACHE_MARKERS = new Set(["CACHEDIR.TAG", ".rustc_info.json"]);
const CARGO_HASH_DIRECTORY = /^.+-[a-f0-9]{16}$/;
// Cargo incremental crate names include a 13-character base36 suffix. The
// suffix alone is ambiguous, so recognition also requires a Cargo session
// directory or lock file, and only known cache entries are eligible.
const CARGO_INCREMENTAL_CRATE_DIRECTORY = /^.+-[0-9a-z]{13}$/;
const CARGO_INCREMENTAL_LOCK = /^s-[0-9a-z]{10}-[0-9a-z]{7}\.lock$/;
const CARGO_INCREMENTAL_SESSION_DIRECTORY =
  /^s-[0-9a-z]{10}-[0-9a-z]{7}-(?:[0-9a-z]{25}|working)$/;
const CARGO_INCREMENTAL_OUTPUT_FILE = new Set([
  "dep-graph.bin",
  "dep-graph.part.bin",
  "metadata.rmeta",
  "query-cache.bin",
  "work-products.bin",
]);
const CARGO_INCREMENTAL_OBJECT_FILE = /^[0-9a-z]{25}\.o$/;
const CARGO_DEPENDENCY_FILE =
  /^(?:lib)?[A-Za-z0-9_][A-Za-z0-9_.+-]*-[a-f0-9]{16}(?:\.(?:d|rlib|rmeta|so|dylib|dll|a|lib|exe|pdb))?$/;
const CARGO_BUILD_FILES = new Set([
  "build-script-build",
  "invoked.timestamp",
  "output",
  "root-output",
  "stderr",
]);
const CARGO_FINGERPRINT_FILE =
  /^(?:dep|lib|test|run-build-script|build-script|output)-[A-Za-z0-9_.-]+(?:\.json)?$/;

function git(args, cwd) {
  return spawnSync("git", args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function hasSymlinkedComponent(target) {
  let current = path.resolve(target);
  for (;;) {
    let stat;
    try {
      stat = fs.lstatSync(current);
    } catch {
      return true;
    }
    if (stat.isSymbolicLink()) return true;
    const parent = path.dirname(current);
    if (parent === current) return false;
    current = parent;
  }
}

function verifiedWorktreeRoot() {
  const cwd = process.cwd();
  const logicalCwd = process.env.PWD || cwd;
  const scriptRoot = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "..",
  );
  if (path.resolve(logicalCwd) !== path.resolve(cwd)) {
    return { error: "PWD does not match the working directory" };
  }
  if (hasSymlinkedComponent(logicalCwd) || hasSymlinkedComponent(scriptRoot)) {
    return { error: "worktree root or one of its path components is symlinked" };
  }

  const cwdRoot = git(["rev-parse", "--show-toplevel"], cwd);
  const sourceRoot = git(["-C", scriptRoot, "rev-parse", "--show-toplevel"], scriptRoot);
  if (
    cwdRoot.error ||
    cwdRoot.status !== 0 ||
    sourceRoot.error ||
    sourceRoot.status !== 0
  ) {
    return { error: "Git worktree root could not be verified" };
  }

  try {
    const root = fs.realpathSync(cwdRoot.stdout.trim());
    if (
      root !== fs.realpathSync(cwd) ||
      root !== fs.realpathSync(scriptRoot)
    ) {
      return { error: "invocation is not at the cleanup script's Git worktree root" };
    }
    return { root };
  } catch {
    return { error: "Git worktree root could not be resolved" };
  }
}

function validRelativePath(relativePath) {
  return (
    typeof relativePath === "string" &&
    relativePath.length > 0 &&
    !path.posix.isAbsolute(relativePath) &&
    !relativePath.includes("\\") &&
    relativePath.split("/").every((part) => part !== "" && part !== "." && part !== "..")
  );
}

function createInventory(root) {
  return {
    root,
    trackedPaths: null,
    trackedPathsAvailable: false,
    eligible: [],
    skipped: [],
    errors: [],
    pruneDirectories: new Set(),
  };
}

function addSkip(inventory, relativePath, reason) {
  const key = `${relativePath}: ${reason}`;
  if (!inventory.skipped.includes(key)) inventory.skipped.push(key);
}

function addError(inventory, relativePath, reason) {
  const key = `${relativePath}: ${reason}`;
  if (!inventory.errors.includes(key)) inventory.errors.push(key);
}

function inspectPath(inventory, relativePath) {
  if (!validRelativePath(relativePath)) {
    addError(inventory, String(relativePath), "unsafe path rejected");
    return null;
  }

  let absolutePath = inventory.root;
  let finalStat;
  const parts = relativePath.split("/");
  for (let index = 0; index < parts.length; index += 1) {
    absolutePath = path.join(absolutePath, parts[index]);
    let stat;
    try {
      stat = fs.lstatSync(absolutePath);
    } catch (error) {
      if (error.code === "ENOENT") return null;
      addError(inventory, relativePath, `cannot inspect path (${error.code || error.message})`);
      return null;
    }
    if (stat.isSymbolicLink()) {
      addSkip(
        inventory,
        path.posix.join(...parts.slice(0, index + 1)),
        "symbolic link cleanup candidate skipped",
      );
      return null;
    }
    finalStat = stat;
    if (index < parts.length - 1 && !stat.isDirectory()) {
      addError(inventory, path.posix.join(...parts.slice(0, index + 1)), "expected a directory parent");
      return null;
    }
  }
  return { absolutePath, stat: finalStat };
}

function readDirectory(inventory, relativePath) {
  const inspected = inspectPath(inventory, relativePath);
  if (!inspected) return null;
  if (!inspected.stat.isDirectory()) {
    addError(inventory, relativePath, "expected a directory");
    return null;
  }
  try {
    return fs.readdirSync(inspected.absolutePath).sort();
  } catch (error) {
    addError(inventory, relativePath, `cannot list directory (${error.code || error.message})`);
    return null;
  }
}

function isTracked(inventory, relativePath, preserveGitignore = true) {
  if (inventory.trackedPaths?.has(relativePath)) {
    addSkip(inventory, relativePath, "tracked file preserved");
    return true;
  }
  if (preserveGitignore && path.posix.basename(relativePath) === ".gitignore") {
    addSkip(inventory, relativePath, ".gitignore preserved");
    return true;
  }
  return false;
}

function addEligible(
  inventory,
  relativePath,
  category,
  reportRoot,
  stat,
  preserveGitignore = true,
) {
  if (isTracked(inventory, relativePath, preserveGitignore)) return;
  if (!stat.isFile()) {
    addError(inventory, relativePath, "expected a regular generated file");
    return;
  }
  const item = {
    relativePath,
    category,
    reportRoot,
    size: stat.size,
  };
  inventory.eligible.push(item);
}

function collectExactFile(inventory, relativePath, category) {
  const inspected = inspectPath(inventory, relativePath);
  if (!inspected) return;
  addEligible(inventory, relativePath, category, relativePath, inspected.stat);
}

function hasCargoTargetTripleShape(value) {
  return /^[A-Za-z0-9_.-]+$/.test(value) && value.split("-").length >= 3;
}

function isCargoTargetTriple(value) {
  return hasCargoTargetTripleShape(value) && isKnownCargoTargetTriple(value);
}

function addPrunable(inventory, relativePath) {
  inventory.pruneDirectories.add(relativePath);
}

function isProtectedReleaseArtifact(relativePath) {
  const parts = relativePath.toLowerCase().split("/");
  const basename = parts.at(-1);
  return (
    parts.includes(".git") ||
    parts.includes("bundle") ||
    basename.includes("manifest") ||
    basename.endsWith(".dmg") ||
    basename.endsWith(".app")
  );
}

function hasCargoIncrementalStructure(inventory, relativeDirectory) {
  const names = readDirectory(inventory, relativeDirectory);
  if (!names) return false;

  return names.some((name) => {
    if (!CARGO_INCREMENTAL_LOCK.test(name) && !CARGO_INCREMENTAL_SESSION_DIRECTORY.test(name)) {
      return false;
    }
    const inspected = inspectPath(inventory, path.posix.join(relativeDirectory, name));
    if (!inspected) return false;
    return CARGO_INCREMENTAL_LOCK.test(name)
      ? inspected.stat.isFile()
      : inspected.stat.isDirectory();
  });
}

function walkCargoIncrementalSession(inventory, relativeDirectory, category, reportRoot) {
  const names = readDirectory(inventory, relativeDirectory);
  if (!names) return;
  addPrunable(inventory, relativeDirectory);

  for (const name of names) {
    const childPath = path.posix.join(relativeDirectory, name);
    if (isProtectedReleaseArtifact(childPath)) {
      addSkip(inventory, childPath, "release bundle or manifest preserved");
      continue;
    }
    const inspected = inspectPath(inventory, childPath);
    if (!inspected) continue;
    if (isTracked(inventory, childPath)) continue;

    const recognizedFile =
      CARGO_INCREMENTAL_OUTPUT_FILE.has(name) || CARGO_INCREMENTAL_OBJECT_FILE.test(name);
    if (!inspected.stat.isFile()) {
      if (recognizedFile) {
        addError(inventory, childPath, "unsupported filesystem entry in Cargo incremental output");
      } else {
        addSkip(inventory, childPath, "unrecognized Cargo incremental entry preserved");
      }
      continue;
    }
    if (recognizedFile) {
      addEligible(inventory, childPath, category, reportRoot, inspected.stat);
    } else {
      addSkip(inventory, childPath, "unrecognized Cargo incremental entry preserved");
    }
  }
}

function walkCargoIncrementalCrate(inventory, relativeDirectory, category) {
  const names = readDirectory(inventory, relativeDirectory);
  if (!names) return;
  addPrunable(inventory, relativeDirectory);

  for (const name of names) {
    const childPath = path.posix.join(relativeDirectory, name);
    if (isProtectedReleaseArtifact(childPath)) {
      addSkip(inventory, childPath, "release bundle or manifest preserved");
      continue;
    }
    const inspected = inspectPath(inventory, childPath);
    if (!inspected) continue;
    if (isTracked(inventory, childPath)) continue;

    if (inspected.stat.isFile() && CARGO_INCREMENTAL_LOCK.test(name)) {
      addEligible(inventory, childPath, category, relativeDirectory, inspected.stat);
      continue;
    }
    if (inspected.stat.isDirectory() && CARGO_INCREMENTAL_SESSION_DIRECTORY.test(name)) {
      walkCargoIncrementalSession(inventory, childPath, category, relativeDirectory);
      continue;
    }
    if (CARGO_INCREMENTAL_LOCK.test(name) || CARGO_INCREMENTAL_SESSION_DIRECTORY.test(name)) {
      addError(inventory, childPath, "unsupported Cargo incremental entry type");
      continue;
    }
    addSkip(inventory, childPath, "unrecognized Cargo incremental entry preserved");
  }
}

function walkRecognizedCargoIncrementalTree(inventory, rootRelativePath, category) {
  const topLevel = readDirectory(inventory, rootRelativePath);
  if (!topLevel) return;
  addPrunable(inventory, rootRelativePath);

  for (const name of topLevel) {
    const childPath = path.posix.join(rootRelativePath, name);
    const inspected = inspectPath(inventory, childPath);
    if (!inspected) continue;
    if (isTracked(inventory, childPath)) continue;

    const recognizedDirectory =
      inspected.stat.isDirectory() &&
      CARGO_INCREMENTAL_CRATE_DIRECTORY.test(name) &&
      hasCargoIncrementalStructure(inventory, childPath);
    if (!recognizedDirectory) {
      addSkip(inventory, childPath, "unrecognized Cargo output preserved");
      continue;
    }
    walkCargoIncrementalCrate(inventory, childPath, category);
  }
}

function walkRecognizedCargoTree(inventory, rootRelativePath, kind, category) {
  if (kind === "incremental") {
    walkRecognizedCargoIncrementalTree(inventory, rootRelativePath, category);
    return;
  }

  const visit = (relativeDirectory, withinGeneratedOutput = false) => {
    const names = readDirectory(inventory, relativeDirectory);
    if (!names) return;
    addPrunable(inventory, relativeDirectory);

    for (const name of names) {
      const childPath = path.posix.join(relativeDirectory, name);
      if (isProtectedReleaseArtifact(childPath)) {
        addSkip(inventory, childPath, "release bundle or manifest preserved");
        continue;
      }
      const inspected = inspectPath(inventory, childPath);
      if (!inspected) continue;
      if (isTracked(inventory, childPath)) continue;

      if (inspected.stat.isDirectory()) {
        if (kind === "build" && !withinGeneratedOutput && name !== "out") {
          addSkip(inventory, childPath, "unrecognized Cargo build entry preserved");
          continue;
        }
        if (kind === "fingerprint") {
          addSkip(inventory, childPath, "unrecognized Cargo fingerprint entry preserved");
          continue;
        }
        visit(childPath, withinGeneratedOutput || (kind === "build" && name === "out"));
        continue;
      }
      if (!inspected.stat.isFile()) {
        addError(inventory, childPath, "unsupported filesystem entry in Cargo output");
        continue;
      }

      const basename = path.posix.basename(childPath);
      let recognized = false;
      if (kind === "build") {
        recognized =
          withinGeneratedOutput ||
          CARGO_BUILD_FILES.has(basename) ||
          /^output-[A-Za-z0-9_.-]+$/.test(basename);
      } else if (kind === "fingerprint") {
        recognized = basename === "invoked.timestamp" || CARGO_FINGERPRINT_FILE.test(basename);
      }

      if (recognized) {
        addEligible(inventory, childPath, category, rootRelativePath, inspected.stat);
      } else {
        addSkip(inventory, childPath, "unrecognized Cargo output preserved");
      }
    }
  };

  const topLevel = readDirectory(inventory, rootRelativePath);
  if (!topLevel) return;
  addPrunable(inventory, rootRelativePath);
  for (const name of topLevel) {
    const childPath = path.posix.join(rootRelativePath, name);
    const inspected = inspectPath(inventory, childPath);
    if (!inspected) continue;
    if (isTracked(inventory, childPath)) continue;

    if (kind === "deps") {
      if (inspected.stat.isFile() && CARGO_DEPENDENCY_FILE.test(name)) {
        addEligible(inventory, childPath, category, rootRelativePath, inspected.stat);
      } else {
        addSkip(inventory, childPath, "unrecognized Cargo dependency output preserved");
      }
      continue;
    }

    const recognizedDirectory = inspected.stat.isDirectory() && CARGO_HASH_DIRECTORY.test(name);
    if (!recognizedDirectory) {
      addSkip(inventory, childPath, "unrecognized Cargo output preserved");
      continue;
    }
    addPrunable(inventory, childPath);
    visit(childPath);
  }
};

/** Scans one Cargo profile for recognized Cargo output directories. */
function scanCargoProfile(inventory, profilePath, category) {
  const names = readDirectory(inventory, profilePath);
  if (!names) return;

  for (const name of names) {
    const childPath = path.posix.join(profilePath, name);
    if (name === "bundle") {
      addSkip(inventory, childPath, "release or debug bundle subtree is preserved");
      continue;
    }
    if (OUTPUT_DIRECTORIES.has(name)) {
      const kind = name === ".fingerprint" ? "fingerprint" : name;
      walkRecognizedCargoTree(inventory, childPath, kind, category);
      continue;
    }
    addSkip(inventory, childPath, "outside the Cargo intermediate allowlist");
  }
}

function hasCargoProfileShape(inventory, profilePath) {
  const names = readDirectory(inventory, profilePath);
  return names !== null && names.some((name) => OUTPUT_DIRECTORIES.has(name));
}

/** Finds allowlisted intermediates without removing a target or profile root. */
function collectCargoTarget(inventory, targetRoot) {
  const names = readDirectory(inventory, targetRoot);
  if (!names) return;

  for (const marker of CARGO_CACHE_MARKERS) {
    collectExactFile(inventory, path.posix.join(targetRoot, marker), "cargo-intermediates");
  }
  collectExactFile(
    inventory,
    path.posix.join(targetRoot, OBSOLETE_FRESHNESS_STAMP),
    "cargo-intermediates",
  );
  addSkip(
    inventory,
    path.posix.join(targetRoot, PENDING_FRESHNESS_MARKER),
    "backend-test-fresh pending marker preserved",
  );

  for (const name of names) {
    if (
      CARGO_CACHE_MARKERS.has(name) ||
      name === OBSOLETE_FRESHNESS_STAMP ||
      name === PENDING_FRESHNESS_MARKER
    ) {
      continue;
    }
    const childPath = path.posix.join(targetRoot, name);
    if (PROFILE_NAMES.has(name)) {
      const inspected = inspectPath(inventory, childPath);
      if (inspected?.stat.isDirectory()) scanCargoProfile(inventory, childPath, "cargo-intermediates");
      else if (inspected) addError(inventory, childPath, "Cargo profile is not a directory");
      continue;
    }
    if (isCargoTargetTriple(name)) {
      const inspected = inspectPath(inventory, childPath);
      if (!inspected) continue;
      if (!inspected.stat.isDirectory()) {
        addError(inventory, childPath, "target-triple output root is not a directory");
        continue;
      }
      const profileNames = readDirectory(inventory, childPath) || [];
      for (const profile of profileNames) {
        const profilePath = path.posix.join(childPath, profile);
        if (PROFILE_NAMES.has(profile)) {
          const profileInfo = inspectPath(inventory, profilePath);
          if (!profileInfo) continue;
          if (!profileInfo.stat.isDirectory()) {
            addError(inventory, profilePath, "Cargo profile is not a directory");
            continue;
          }
          if (!hasCargoProfileShape(inventory, profilePath)) {
            addSkip(inventory, profilePath, "target-triple profile lacks recognized Cargo output");
            continue;
          }
          scanCargoProfile(inventory, profilePath, "cargo-intermediates");
        } else {
          addSkip(inventory, profilePath, "outside the target-triple profile allowlist");
        }
      }
      continue;
    }
    if (hasCargoTargetTripleShape(name)) {
      addSkip(inventory, childPath, "unrecognized Cargo target-triple directory preserved");
      continue;
    }
    addSkip(inventory, childPath, "outside the Cargo intermediate allowlist");
  }
}

/** Recognizes only Tauri's desktop schema referenced by this repository. */
function collectGeneratedSchema(inventory, genRoot) {
  const schemaDirectory = path.posix.join(genRoot, "schemas");
  const genEntries = readDirectory(inventory, genRoot);
  if (!genEntries) return;
  for (const name of genEntries) {
    const relativePath = path.posix.join(genRoot, name);
    if (name === "schemas") {
      const schemaEntries = readDirectory(inventory, schemaDirectory);
      if (!schemaEntries) continue;
      for (const schemaName of schemaEntries) {
        const schemaPath = path.posix.join(schemaDirectory, schemaName);
        if (schemaName === "desktop-schema.json") {
          collectExactFile(inventory, schemaPath, "tauri-generated-state");
        } else {
          addSkip(inventory, schemaPath, "unrecognized Tauri generated content preserved");
        }
      }
    } else {
      addSkip(inventory, relativePath, "unrecognized Tauri generated content preserved");
    }
  }
}

/** Collects worktree-owned dependency or frontend build output for deep mode. */
function isKnownDeepFile(relativeRoot, relativePath, category) {
  const withinRoot = path.posix.relative(relativeRoot, relativePath);
  if (category === "frontend-dependencies") return true;
  if (category === "frontend-dist") {
    return (
      withinRoot === "index.html" ||
      /^assets\/.+\.(?:js|css|map|svg|png|webp|woff2?)$/i.test(withinRoot)
    );
  }
  return false;
}

function collectDeepTree(inventory, relativeRoot, category) {
  const rootInfo = inspectPath(inventory, relativeRoot);
  if (!rootInfo) return;
  if (!rootInfo.stat.isDirectory()) {
    addError(inventory, relativeRoot, "expected a directory for deep cleanup");
    return;
  }

  if (category === "frontend-dependencies") {
    const stampPath = path.posix.join(relativeRoot, DEPENDENCY_INSTALL_STAMP);
    if (isTracked(inventory, stampPath)) {
      addError(inventory, stampPath, "tracked dependency-install stamp prevents safe deep cleanup");
      return;
    }
    try {
      const stampStat = fs.lstatSync(path.join(inventory.root, stampPath));
      if (stampStat.isSymbolicLink()) {
        addSkip(
          inventory,
          stampPath,
          "symbolic link cleanup candidate skipped; dependency root cleanup skipped",
        );
        return;
      }
      if (!stampStat.isFile()) {
        addError(inventory, stampPath, "dependency-install stamp is not a regular file");
        return;
      }
    } catch (error) {
      if (error.code !== "ENOENT") {
        addError(inventory, stampPath, `cannot inspect dependency-install stamp (${error.code || error.message})`);
        return;
      }
    }
  }

  const visit = (relativeDirectory) => {
    const names = readDirectory(inventory, relativeDirectory);
    if (!names) return;
    addPrunable(inventory, relativeDirectory);
    for (const name of names) {
      const childPath = path.posix.join(relativeDirectory, name);
      if (name === ".git") {
        addSkip(inventory, childPath, "Git metadata preserved");
        continue;
      }
      if (isTracked(inventory, childPath, category !== "frontend-dependencies")) continue;

      if (category === "frontend-dependencies") {
        const absolutePath = path.join(inventory.root, childPath);
        let stat;
        try {
          stat = fs.lstatSync(absolutePath);
        } catch (error) {
          if (error.code !== "ENOENT") {
            addError(inventory, childPath, `cannot inspect dependency entry (${error.code || error.message})`);
          }
          continue;
        }

        if (stat.isSymbolicLink()) {
        addSkip(inventory, childPath, "symbolic link cleanup candidate skipped");
        } else if (stat.isDirectory()) {
          visit(childPath);
        } else if (stat.isFile()) {
        addEligible(inventory, childPath, category, relativeRoot, stat, false);
        } else {
          addError(inventory, childPath, "unsupported filesystem entry in deep output");
        }
        continue;
      }

      const inspected = inspectPath(inventory, childPath);
      if (!inspected) continue;
      if (inspected.stat.isDirectory()) {
        visit(childPath);
      } else if (inspected.stat.isFile()) {
        if (isKnownDeepFile(relativeRoot, childPath, category)) {
          addEligible(inventory, childPath, category, relativeRoot, inspected.stat);
        } else {
          addSkip(inventory, childPath, "unknown frontend output preserved");
        }
      } else {
        addError(inventory, childPath, "unsupported filesystem entry in deep output");
      }
    }
  };
  visit(relativeRoot);
};

function validSidecarFilename(name) {
  return isCanonicalSidecarArtifactName(name);
}

function validSidecarMetadataFilename(name) {
  return isCanonicalSidecarMetadataName(name);
}

function collectSidecars(inventory, binariesRoot) {
  const names = readDirectory(inventory, binariesRoot);
  if (!names) return;
  for (const name of names) {
    const relativePath = path.posix.join(binariesRoot, name);
    if (name === ".gitignore") {
      addSkip(inventory, relativePath, ".gitignore preserved");
      continue;
    }
    if (!validSidecarFilename(name) && !validSidecarMetadataFilename(name)) {
      addSkip(inventory, relativePath, "unknown Tauri binary-directory content preserved");
      continue;
    }
    const inspected = inspectPath(inventory, relativePath);
    if (inspected) addEligible(inventory, relativePath, "tauri-sidecars", relativePath, inspected.stat);
  }
}

function trackedPathsFor(root, inventory) {
  const result = git(["ls-files", "--cached", "-z"], root);
  if (result.error || result.status !== 0) {
    addError(inventory, ".git", `cannot read tracked paths (${result.error?.message || result.stderr.trim()})`);
    return null;
  }
  return new Set(result.stdout.split("\0").filter(Boolean));
}

function collectInventory(root, includeDeep) {
  const inventory = createInventory(root);
  const trackedPaths = trackedPathsFor(root, inventory);
  if (trackedPaths === null) return inventory;
  inventory.trackedPaths = trackedPaths;
  inventory.trackedPathsAvailable = true;

  for (const targetRoot of TARGET_ROOTS) collectCargoTarget(inventory, targetRoot);
  for (const genRoot of GEN_ROOTS) collectGeneratedSchema(inventory, genRoot);
  if (includeDeep) {
    for (const appRoot of TAURI_APP_ROOTS) {
      collectDeepTree(inventory, `${appRoot}/node_modules`, "frontend-dependencies");
      collectDeepTree(inventory, `${appRoot}/dist`, "frontend-dist");
      collectSidecars(inventory, `${appRoot}/src-tauri/binaries`);
    }
  }
  inventory.eligible.sort((left, right) => left.relativePath.localeCompare(right.relativePath));
  inventory.skipped.sort();
  inventory.errors.sort();
  return inventory;
}

function groupedEligible(inventory) {
  const groups = new Map();
  for (const item of inventory.eligible) {
    const key = `${item.category}\0${item.reportRoot}`;
    const group = groups.get(key) || {
      category: item.category,
      reportRoot: item.reportRoot,
      count: 0,
      bytes: 0,
      examples: [],
    };
    group.count += 1;
    group.bytes += item.size;
    if (group.examples.length < 3) group.examples.push(item.relativePath);
    groups.set(key, group);
  }
  return [...groups.values()].sort((left, right) => left.reportRoot.localeCompare(right.reportRoot));
}

function printInventory(prefix, inventory, includeDeep) {
  process.stdout.write(`${prefix} PREVIEW\n`);
  const groups = groupedEligible(inventory);
  for (const group of groups) {
    process.stdout.write(
      `${prefix} ELIGIBLE [${group.category}] ${group.reportRoot} (${group.count} files; estimated ${group.bytes} bytes)\n`,
    );
    for (const example of group.examples) {
      process.stdout.write(`${prefix}   ${example}\n`);
    }
  }
  if (groups.length === 0) process.stdout.write(`${prefix} ELIGIBLE none found (estimated 0 bytes)\n`);

  for (const skipped of inventory.skipped) process.stdout.write(`${prefix} SKIP ${skipped}\n`);
  for (const error of inventory.errors) process.stderr.write(`${prefix} ERROR ${error}\n`);

  process.stdout.write(
    `${prefix} PRESERVE release outputs: target/**/bundle/**, release artifacts and manifests, release-artifacts/**, docs/testing/**, docs/qualification/**, docs/release/**, snapshots, fixtures, and authored content\n`,
  );
  process.stdout.write(
    `${prefix} PRESERVE backend-test-fresh pending marker: crates/emuchef-rust-backend/target/.emuchef-cargo-test-source.sha256.pending; the obsolete digest stamp is eligible for cleanup\n`,
  );
  process.stdout.write(
    `${prefix} PRESERVE application/runtime/developer state: .emuchef_cache, .emuchef_runtime/**, user data, and agent state\n`,
  );
  process.stdout.write(
    `${prefix} PRESERVE tracked files; .gitignore and unknown content outside node_modules; deep mode removes untracked regular files under node_modules; symlinks are skipped without following targets; nested .git metadata is preserved\n`,
  );
  process.stdout.write(`${prefix} PRESERVE external CARGO_TARGET_DIR; cleanup paths are worktree-local\n`);
  if (!includeDeep) {
    process.stdout.write(
      `${prefix} SKIP deep-only categories: both apps' node_modules and dist, and validated Tauri sidecar copies\n`,
    );
  }
  const estimate = inventory.eligible.reduce((total, item) => total + item.size, 0);
  process.stdout.write(`${prefix} eligible estimated bytes: ${estimate}\n`);
}

function isDependencyInstallStamp(item) {
  return (
    item.category === "frontend-dependencies" &&
    item.relativePath === path.posix.join(item.reportRoot, DEPENDENCY_INSTALL_STAMP)
  );
}

function removeEligibleItem(inventory, item) {
  const errorsBefore = inventory.errors.length;
  const inspected = inspectPath(inventory, item.relativePath);
  if (!inspected) return inventory.errors.length === errorsBefore ? "missing" : "failed";
  if (!inspected.stat.isFile()) {
    addError(inventory, item.relativePath, "candidate changed type before deletion");
    return "failed";
  }

  try {
    fs.unlinkSync(inspected.absolutePath);
    return "removed";
  } catch (error) {
    addError(inventory, item.relativePath, `cannot remove file (${error.code || error.message})`);
    return "failed";
  }
}

function removeEligible(inventory) {
  const removed = [];
  const blockedDependencyRoots = new Set();
  const eligible = [...inventory.eligible].sort((left, right) => {
    const stampOrder = Number(isDependencyInstallStamp(right)) - Number(isDependencyInstallStamp(left));
    return stampOrder || left.relativePath.localeCompare(right.relativePath);
  });
  for (const item of eligible) {
    const isStamp = isDependencyInstallStamp(item);
    if (
      !isStamp &&
      item.category === "frontend-dependencies" &&
      blockedDependencyRoots.has(item.reportRoot)
    ) {
      continue;
    }
    const result = removeEligibleItem(inventory, item);
    if (result === "removed") removed.push(item);
    else if (isStamp && result === "failed") blockedDependencyRoots.add(item.reportRoot);
  }

  const directories = [...inventory.pruneDirectories].sort(
    (left, right) => right.split("/").length - left.split("/").length || right.localeCompare(left),
  );
  for (const relativePath of directories) {
    if (
      [...blockedDependencyRoots].some(
        (root) => relativePath === root || relativePath.startsWith(`${root}/`),
      )
    ) {
      continue;
    }
    const inspected = inspectPath(inventory, relativePath);
    if (!inspected) continue;
    if (!inspected.stat.isDirectory()) continue;
    try {
      if (fs.readdirSync(inspected.absolutePath).length === 0) {
        fs.rmdirSync(inspected.absolutePath);
      }
    } catch (error) {
      if (error.code !== "ENOTEMPTY" && error.code !== "EEXIST") {
        addError(inventory, relativePath, `cannot remove empty directory (${error.code || error.message})`);
      }
    }
  }
  return removed;
}

function printRemovalSummary(prefix, inventory, removed) {
  const groups = new Map();
  for (const item of removed) {
    const key = `${item.category}\0${item.reportRoot}`;
    const group = groups.get(key) || {
      category: item.category,
      reportRoot: item.reportRoot,
      count: 0,
      bytes: 0,
    };
    group.count += 1;
    group.bytes += item.size;
    groups.set(key, group);
  }
  for (const group of [...groups.values()].sort((a, b) => a.reportRoot.localeCompare(b.reportRoot))) {
    process.stdout.write(
      `${prefix} REMOVED [${group.category}] ${group.reportRoot} (${group.count} files; estimated ${group.bytes} bytes)\n`,
    );
  }
  if (groups.size === 0) process.stdout.write(`${prefix} REMOVED none (estimated 0 bytes)\n`);
  for (const error of inventory.errors) process.stderr.write(`${prefix} ERROR ${error}\n`);
  const bytes = removed.reduce((total, item) => total + item.size, 0);
  process.stdout.write(`${prefix} removed estimated bytes: ${bytes}\n`);
  process.stdout.write(`${prefix} skipped paths: ${inventory.skipped.length}\n`);
  process.stdout.write(`${prefix} errors: ${inventory.errors.length}\n`);
}

function run(mode) {
  const rootResult = verifiedWorktreeRoot();
  if (rootResult.error) {
    process.stderr.write(`worktree-cleanup: ${rootResult.error}; no files changed\n`);
    return 2;
  }

  const includeDeep = mode === "deep";
  const inventory = collectInventory(rootResult.root, includeDeep);
  const prefix = mode === "preview" ? "clean-preview:" : mode === "deep" ? "clean-deep:" : "clean:";

  process.stdout.write(`${prefix} Git worktree root verified\n`);
  if (mode === "deep") {
    process.stdout.write(
      "clean-deep: WARNING: this opt-in removes frontend dependencies, frontend dist output, and prepared Tauri sidecar copies.\n",
    );
    process.stdout.write(
      "clean-deep: WARNING: the next build may require dependency installation and sidecar regeneration.\n",
    );
  }
  if (!inventory.trackedPathsAvailable) {
    for (const error of inventory.errors) process.stderr.write(`${prefix} ERROR ${error}\n`);
    process.stdout.write(`${prefix} no files changed\n`);
    return 1;
  }
  printInventory(prefix, inventory, includeDeep);

  if (mode === "preview") {
    process.stdout.write("clean-preview: no files changed\n");
    return inventory.errors.length === 0 ? 0 : 1;
  }

  process.stdout.write(`${prefix} DELETION\n`);
  const removed = removeEligible(inventory);
  printRemovalSummary(prefix, inventory, removed);
  return inventory.errors.length === 0 ? 0 : 1;
}

function main() {
  if (process.argv.length !== 3) {
    process.stderr.write("worktree-cleanup: expected one mode and no path arguments\n");
    return 2;
  }
  const mode = process.argv[2];
  if (!["preview", "default", "deep"].includes(mode)) {
    process.stderr.write("worktree-cleanup: expected preview, default, or deep mode\n");
    return 2;
  }
  return run(mode);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main();
}

export {
  collectInventory,
  isProtectedReleaseArtifact,
  main,
  printInventory,
  printRemovalSummary,
  removeEligible,
  validRelativePath,
  validSidecarFilename,
  validSidecarMetadataFilename,
};

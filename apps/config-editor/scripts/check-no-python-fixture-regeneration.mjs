#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PACKAGE_JSON = "apps/config-editor/package.json";
const CHECK_ROOT_SCRIPT = "check:rust-runtime";
const PYTHON_EXECUTION_RE = /(?:PYTHONPATH\s*=|(?<![A-Za-z0-9_.-])(?:python3?(?:\.exe)?|uv(?:\.exe)?)(?![A-Za-z0-9_.-]))/;
const FIXTURE_GENERATION_RE = /(?:fixture|fixtures|golden|goldens|python_goldens|regen|regenerate|regeneration|refresh|generate|generator|<<\s*'?PY'?|-m\s+emuchef)/i;
const NPM_RUN_RE = /(?:^|&&|\|\||;)\s*npm\s+run\s+([A-Za-z0-9_:@.-]+)/g;
const SCRIPT_PATH_RE = /scripts\/[A-Za-z0-9_.-]+\.mjs/g;
const STATIC_IMPORT_RE = /(?:import\s+(?:[^"'()]+?\s+from\s+)?|import\s*\()\s*["'](\.\/[^"']+\.mjs)["']/g;

export function findForbiddenFixtureRegenerationHits(filePath, content, reason) {
  const hits = [];
  const lines = content.split("\n");
  lines.forEach((line, index) => {
    if (PYTHON_EXECUTION_RE.test(line) && FIXTURE_GENERATION_RE.test(line)) {
      hits.push({
        filePath,
        line: index + 1,
        matchedLine: line,
        reason,
        rule: "Python fixture/golden generator invocation",
      });
    }
  });
  return hits;
}

export function collectActiveCheckSurfacesFromPackage(packageJsonContent) {
  const packageJson = JSON.parse(packageJsonContent);
  const scripts = packageJson.scripts ?? {};
  const surfaces = [];
  const visited = new Set();

  function visit(scriptName) {
    if (visited.has(scriptName)) {
      return;
    }
    visited.add(scriptName);

    const command = scripts[scriptName];
    if (typeof command !== "string") {
      return;
    }

    surfaces.push({
      filePath: "package.json",
      content: command,
      reason: `package script ${scriptName} reachable from ${CHECK_ROOT_SCRIPT}`,
    });

    NPM_RUN_RE.lastIndex = 0;
    for (const match of command.matchAll(NPM_RUN_RE)) {
      visit(match[1]);
    }
  }

  visit(CHECK_ROOT_SCRIPT);
  return surfaces;
}

async function collectActiveScriptFileSurfaces(appDir, packageSurfaces) {
  const pending = [];
  const visited = new Set();
  const surfaces = [];

  for (const surface of packageSurfaces) {
    for (const match of surface.content.matchAll(SCRIPT_PATH_RE)) {
      pending.push({
        relativePath: match[0],
        reason: `${match[0]} invoked by ${surface.reason}`,
      });
    }
  }

  while (pending.length > 0) {
    const { relativePath, reason } = pending.shift();
    const normalizedPath = normalizePath(relativePath);
    if (visited.has(normalizedPath)) {
      continue;
    }
    visited.add(normalizedPath);

    const absolutePath = path.join(appDir, normalizedPath);
    let content;
    try {
      content = await fs.readFile(absolutePath, "utf8");
    } catch (error) {
      if (error.code === "ENOENT") {
        continue;
      }
      throw error;
    }

    if (!normalizedPath.endsWith(".test.mjs")) {
      surfaces.push({ filePath: normalizedPath, content, reason });
    }

    STATIC_IMPORT_RE.lastIndex = 0;
    const directory = path.dirname(normalizedPath);
    for (const match of content.matchAll(STATIC_IMPORT_RE)) {
      pending.push({
        relativePath: normalizePath(path.join(directory, match[1])),
        reason: `${normalizePath(path.join(directory, match[1]))} imported by ${normalizedPath}`,
      });
    }
  }

  return surfaces;
}

async function collectRustFileSurfaces(repoRoot) {
  const surfaces = [];
  await collectFiles(
    path.join(repoRoot, "apps/config-editor/src-tauri"),
    (relativePath) => relativePath.endsWith(".rs") && !relativePath.includes("/target/"),
    "Tauri Rust source/test surface under apps/config-editor/src-tauri",
    surfaces,
    repoRoot,
  );
  await collectFiles(
    path.join(repoRoot, "crates/emuchef-rust-backend/tests"),
    (relativePath) => relativePath.endsWith(".rs"),
    "Rust backend integration test under crates/emuchef-rust-backend/tests",
    surfaces,
    repoRoot,
  );
  await collectFiles(
    path.join(repoRoot, "crates/emuchef-rust-backend/src"),
    (relativePath) => /_tests\.rs$/.test(relativePath),
    "Rust backend crate-local test module under crates/emuchef-rust-backend/src",
    surfaces,
    repoRoot,
  );
  return surfaces;
}

async function collectFiles(directory, includeFile, reason, surfaces, repoRoot) {
  let entries;
  try {
    entries = await fs.readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") {
      return;
    }
    throw error;
  }

  for (const entry of entries) {
    const absolutePath = path.join(directory, entry.name);
    const relativePath = normalizePath(path.relative(repoRoot, absolutePath));
    if (entry.isDirectory()) {
      if (["target", "node_modules"].includes(entry.name)) {
        continue;
      }
      await collectFiles(absolutePath, includeFile, reason, surfaces, repoRoot);
      continue;
    }
    if (!entry.isFile() || !includeFile(relativePath)) {
      continue;
    }
    surfaces.push({
      filePath: relativePath,
      content: await fs.readFile(absolutePath, "utf8"),
      reason,
    });
  }
}

function normalizePath(filePath) {
  return filePath.split(path.sep).join("/");
}

async function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const appDir = path.resolve(scriptDir, "..");
  const repoRoot = path.resolve(appDir, "../..");
  const packageJsonContent = await fs.readFile(path.join(repoRoot, PACKAGE_JSON), "utf8");

  const packageSurfaces = collectActiveCheckSurfacesFromPackage(packageJsonContent);
  const scriptSurfaces = await collectActiveScriptFileSurfaces(appDir, packageSurfaces);
  const rustSurfaces = await collectRustFileSurfaces(repoRoot);
  const surfaces = [...packageSurfaces, ...scriptSurfaces, ...rustSurfaces];

  const hits = surfaces.flatMap((surface) =>
    findForbiddenFixtureRegenerationHits(surface.filePath, surface.content, surface.reason),
  );

  if (hits.length > 0) {
    console.error("check-no-python-fixture-regeneration: forbidden active fixture/golden regeneration found:");
    for (const hit of hits) {
      console.error(`- ${hit.filePath}:${hit.line}`);
      console.error(`  active because: ${hit.reason}`);
      console.error(`  failed rule: ${hit.rule}`);
      console.error(`  matched line: ${hit.matchedLine.trim()}`);
    }
    process.exit(1);
  }

  console.log(
    `check-no-python-fixture-regeneration: checked ${surfaces.length} active Rust/Tauri surfaces; no Python fixture/golden regeneration found.`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`check-no-python-fixture-regeneration: ${error.message}`);
    process.exit(1);
  });
}

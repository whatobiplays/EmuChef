#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PYTHON_NAME_BOUNDARY = String.raw`[A-Za-z0-9_-]`;
const PYSIDE_IMPORT_RE = new RegExp(String.raw`^\s*(?:from|import)\s+(PySide6)(?:\b|\.)`, "gm");
const LEGACY_APP_IMPORT_RE = new RegExp(
  String.raw`^\s*(?:from|import)\s+(emuchef_editor\.app)(?:\b|\.)`,
  "gm",
);
const QT_METADATA_TOKEN_RE = new RegExp(
  String.raw`(?<!${PYTHON_NAME_BOUNDARY})(QtCore|QtGui|QtWidgets|QtTest)(?!${PYTHON_NAME_BOUNDARY})`,
  "g",
);

const PYPROJECT_PATH = "pyproject.toml";
const ACTIVE_SOURCE_ROOTS = ["src"];
const NORMAL_TEST_ROOTS = ["tests"];
const FORMER_PYSIDE_SOURCE_PREFIX = "src/emuchef_editor/app/";
const FORMER_PYSIDE_TEST_PREFIX = "tests/legacy/";
const DOC_PATH_PREFIXES = ["docs/", "README.md", "CONTEXT.md"];
const PYSIDE_FREE_CORE_PATHS = [
  "src/emuchef_editor/core/workspace.py",
  "src/emuchef_editor/core/metadata/tooltips.py",
  "src/emuchef_editor/core/metadata/step_metadata.py",
];

export function parsePyProjectQuarantineState(content) {
  const projectSection = sectionContent(content, "project");
  const scriptsSection = sectionContent(content, "project.scripts");
  const optionalDependenciesSection = sectionContent(content, "project.optional-dependencies");
  const setuptoolsSection = sectionContent(content, "tool.setuptools");
  const baseDependencies = arrayAssignmentValues(projectSection, "dependencies");
  const optionalDependencies = allArrayStringValues(optionalDependenciesSection);

  return {
    basePySideDependencies: baseDependencies.filter((dependency) => dependency.toLowerCase().startsWith("pyside6")),
    optionalPySideDependencies: optionalDependencies.filter((dependency) => dependency.toLowerCase().startsWith("pyside6")),
    hasPublishedEditorScript: /^\s*emuchef-editor\s*=/m.test(scriptsSection),
    disablesImplicitPackageData: booleanAssignmentValue(setuptoolsSection, "include-package-data") === false,
  };
}

export function findForbiddenPySideHits(filePath, content) {
  const normalizedPath = normalizePath(filePath);
  if (isDocumentationPath(normalizedPath)) {
    return [];
  }

  const hits = [];
  const formerLegacyToken = formerLegacyPathToken(normalizedPath);
  if (formerLegacyToken !== null) {
    hits.push({ filePath: normalizedPath, token: formerLegacyToken, line: 0 });
  }
  if (isActivePythonPath(normalizedPath)) {
    hits.push(...findPatternHits(normalizedPath, content, PYSIDE_IMPORT_RE));
    hits.push(...findPatternHits(normalizedPath, content, LEGACY_APP_IMPORT_RE));
  }
  if (PYSIDE_FREE_CORE_PATHS.includes(normalizedPath)) {
    hits.push(...findPatternHits(normalizedPath, content, QT_METADATA_TOKEN_RE));
  }
  return hits.sort((left, right) => left.line - right.line || left.token.localeCompare(right.token));
}

function sectionContent(content, sectionName) {
  const lines = content.split("\n");
  const header = `[${sectionName}]`;
  const startIndex = lines.findIndex((line) => line.trim() === header);
  if (startIndex === -1) {
    return "";
  }

  const sectionLines = [];
  for (let index = startIndex + 1; index < lines.length; index += 1) {
    if (/^\s*\[.+\]\s*$/.test(lines[index])) {
      break;
    }
    sectionLines.push(lines[index]);
  }
  return sectionLines.join("\n");
}

function arrayAssignmentValues(section, key) {
  const match = section.match(new RegExp(String.raw`^\s*${escapeRegExp(key)}\s*=\s*\[([\s\S]*?)\]`, "m"));
  if (match === null) {
    return [];
  }
  return [...match[1].matchAll(/"([^"]+)"/g)].map((valueMatch) => valueMatch[1]);
}

function allArrayStringValues(section) {
  return [...section.matchAll(/=\s*\[([\s\S]*?)\]/g)]
    .flatMap((assignmentMatch) => [...assignmentMatch[1].matchAll(/"([^"]+)"/g)].map((valueMatch) => valueMatch[1]));
}

function booleanAssignmentValue(section, key) {
  const match = section.match(new RegExp(String.raw`^\s*${escapeRegExp(key)}\s*=\s*(true|false)\s*$`, "m"));
  if (match === null) {
    return null;
  }
  return match[1] === "true";
}

function findPatternHits(filePath, content, pattern) {
  pattern.lastIndex = 0;
  const hits = [];
  for (const match of content.matchAll(pattern)) {
    hits.push({
      filePath,
      token: match[1],
      line: lineNumberForIndex(content, match.index ?? 0),
    });
  }
  return hits;
}

function isActivePythonPath(filePath) {
  if (!filePath.endsWith(".py")) {
    return false;
  }
  return ACTIVE_SOURCE_ROOTS.some((root) => filePath.startsWith(`${root}/`))
    || NORMAL_TEST_ROOTS.some((root) => filePath.startsWith(`${root}/`));
}

function formerLegacyPathToken(filePath) {
  if (filePath.startsWith(FORMER_PYSIDE_SOURCE_PREFIX)) {
    return "legacy PySide source path";
  }
  if (filePath.startsWith(FORMER_PYSIDE_TEST_PREFIX)) {
    return "legacy PySide test path";
  }
  return null;
}

function isDocumentationPath(filePath) {
  return DOC_PATH_PREFIXES.some((prefix) => filePath === prefix || filePath.startsWith(prefix));
}

function lineNumberForIndex(content, index) {
  return content.slice(0, index).split("\n").length;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizePath(filePath) {
  return filePath.split(path.sep).join("/");
}

async function listPythonFiles(repoRoot) {
  const files = [];
  for (const root of [...ACTIVE_SOURCE_ROOTS, ...NORMAL_TEST_ROOTS]) {
    await walk(path.join(repoRoot, root), files, repoRoot);
  }
  return files.sort();
}

async function walk(directory, files, repoRoot) {
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
    if (entry.isDirectory()) {
      if (["__pycache__", ".pytest_cache"].includes(entry.name)) {
        continue;
      }
      await walk(absolutePath, files, repoRoot);
      continue;
    }
    if (entry.isFile() && entry.name.endsWith(".py")) {
      files.push(normalizePath(path.relative(repoRoot, absolutePath)));
    }
  }
}

async function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(scriptDir, "../../..");
  const hits = [];

  const pyproject = await fs.readFile(path.join(repoRoot, PYPROJECT_PATH), "utf8");
  const quarantineState = parsePyProjectQuarantineState(pyproject);
  for (const dependency of quarantineState.basePySideDependencies) {
    hits.push({ filePath: PYPROJECT_PATH, token: dependency, line: 0 });
  }
  for (const dependency of quarantineState.optionalPySideDependencies) {
    hits.push({ filePath: PYPROJECT_PATH, token: dependency, line: 0 });
  }
  if (quarantineState.hasPublishedEditorScript) {
    hits.push({ filePath: PYPROJECT_PATH, token: "emuchef-editor", line: 0 });
  }
  if (!quarantineState.disablesImplicitPackageData) {
    hits.push({ filePath: PYPROJECT_PATH, token: "include-package-data must be false", line: 0 });
  }

  for (const relativePath of await listPythonFiles(repoRoot)) {
    const content = await fs.readFile(path.join(repoRoot, relativePath), "utf8");
    hits.push(...findForbiddenPySideHits(relativePath, content));
  }

  if (hits.length > 0) {
    console.error("check-no-pyside-runtime: forbidden PySide active-runtime/test references found:");
    for (const hit of hits) {
      const location = hit.line > 0 ? `${hit.filePath}:${hit.line}` : hit.filePath;
      console.error(`- ${location}: ${hit.token}`);
    }
    process.exit(1);
  }

  console.log("check-no-pyside-runtime: PySide6 is absent from Python dependencies and active source/test paths.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`check-no-pyside-runtime: ${error.message}`);
    process.exit(1);
  });
}

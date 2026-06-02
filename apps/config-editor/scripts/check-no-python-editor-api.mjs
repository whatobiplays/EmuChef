#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PYPROJECT_PATH = "pyproject.toml";
const ACTIVE_PYTHON_ROOTS = ["src", "tests"];
const EDITOR_API_SOURCE_PREFIX = "src/emuchef_editor/api/";
const DOC_PATH_PREFIXES = ["docs/", "README.md", "CONTEXT.md"];
const EDITOR_API_IMPORT_RE = /^\s*(?:from|import)\s+(emuchef_editor\.api)(?:\b|\.)/gm;

export function parsePythonEditorApiState(content) {
  const scriptsSection = sectionContent(content, "project.scripts");
  return {
    hasPublishedEditorApiScript: /^\s*[-A-Za-z0-9_]*editor[-A-Za-z0-9_]*\s*=\s*"emuchef_editor\.api\./m.test(scriptsSection)
      || /^\s*[-A-Za-z0-9_]*api[-A-Za-z0-9_]*\s*=\s*"emuchef_editor\.api\./m.test(scriptsSection),
  };
}

export function findForbiddenPythonEditorApiHits(filePath, content) {
  const normalizedPath = normalizePath(filePath);
  if (isDocumentationPath(normalizedPath)) {
    return [];
  }

  const hits = [];
  if (normalizedPath.startsWith(EDITOR_API_SOURCE_PREFIX)) {
    hits.push({ filePath: normalizedPath, token: "Python editor API source path", line: 0 });
  }
  if (isActivePythonPath(normalizedPath)) {
    hits.push(...findPatternHits(normalizedPath, content, EDITOR_API_IMPORT_RE));
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
  return filePath.endsWith(".py")
    && ACTIVE_PYTHON_ROOTS.some((root) => filePath.startsWith(`${root}/`));
}

function isDocumentationPath(filePath) {
  return DOC_PATH_PREFIXES.some((prefix) => filePath === prefix || filePath.startsWith(prefix));
}

function lineNumberForIndex(content, index) {
  return content.slice(0, index).split("\n").length;
}

function normalizePath(filePath) {
  return filePath.split(path.sep).join("/");
}

async function listPythonFiles(repoRoot) {
  const files = [];
  for (const root of ACTIVE_PYTHON_ROOTS) {
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
  if (parsePythonEditorApiState(pyproject).hasPublishedEditorApiScript) {
    hits.push({ filePath: PYPROJECT_PATH, token: "emuchef_editor.api script entrypoint", line: 0 });
  }

  for (const relativePath of await listPythonFiles(repoRoot)) {
    const content = await fs.readFile(path.join(repoRoot, relativePath), "utf8");
    hits.push(...findForbiddenPythonEditorApiHits(relativePath, content));
  }

  if (hits.length > 0) {
    console.error("check-no-python-editor-api: forbidden Python editor API surface found:");
    for (const hit of hits) {
      const location = hit.line > 0 ? `${hit.filePath}:${hit.line}` : hit.filePath;
      console.error(`- ${location}: ${hit.token}`);
    }
    process.exit(1);
  }

  console.log("check-no-python-editor-api: Python editor API is absent from active source/test and script paths.");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`check-no-python-editor-api: ${error.message}`);
    process.exit(1);
  });
}

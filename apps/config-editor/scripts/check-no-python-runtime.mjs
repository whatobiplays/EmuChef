#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const TOKEN_BOUNDARY = String.raw`[A-Za-z0-9_.-]`;
const FORBIDDEN_COMMAND_TOKEN_RE = new RegExp(
  String.raw`(?<!${TOKEN_BOUNDARY})(python3?(?:\.exe)?|uv(?:\.exe)?)(?!${TOKEN_BOUNDARY})`,
  "g",
);
const FORBIDDEN_MODULE_RE = new RegExp(
  String.raw`(?<!${TOKEN_BOUNDARY})(emuchef_editor\.api\.server|python_bridge)(?!${TOKEN_BOUNDARY})`,
  "g",
);

export const RUNTIME_CHECK_FILES = [
  "package.json",
  "src-tauri/tauri.conf.json",
  "src-tauri/src/sidecar_client.rs",
  "src-tauri/src/commands.rs",
  "src-tauri/src/lib.rs",
];

export function productRuntimeContractErrors({
  pyproject,
  cargoManifest,
  tauriConfig,
  packaging,
  pythonMainExists = false,
}) {
  const errors = [];
  const scriptsSection = sectionContent(pyproject, "project.scripts");
  if (/^\s*emuchef(?:-[A-Za-z0-9_-]+)?\s*=\s*"emuchef\.cli:main"\s*$/m.test(scriptsSection)) {
    errors.push("Python EmuChef console entrypoint");
  }
  if (pythonMainExists) {
    errors.push("Python module entrypoint src/emuchef/__main__.py");
  }
  for (const [label, passed] of [
    ["Cargo default-run emuchef", /^default-run\s*=\s*"emuchef"\s*$/m.test(cargoManifest)],
    ["Cargo emuchef binary target", /\[\[bin\]\][\s\S]*?name\s*=\s*"emuchef"[\s\S]*?path\s*=\s*"src\/main\.rs"/.test(cargoManifest)],
    ["Tauri emuchef externalBin", /"externalBin"\s*:\s*\[\s*"binaries\/emuchef"\s*\]/.test(tauriConfig)],
    ["packaging emuchef basename", /BINARY_BASENAME\s*=\s*"emuchef"/.test(packaging)],
  ]) {
    if (!passed) {
      errors.push(`missing ${label}`);
    }
  }
  return errors;
}

export function findForbiddenRuntimeHits(filePath, content) {
  const runtimeContent = stripNonRuntimeSections(filePath, content);
  const hits = [];
  for (const pattern of [FORBIDDEN_COMMAND_TOKEN_RE, FORBIDDEN_MODULE_RE]) {
    pattern.lastIndex = 0;
    for (const match of runtimeContent.matchAll(pattern)) {
      hits.push({
        filePath,
        token: match[1],
        line: lineNumberForIndex(runtimeContent, match.index ?? 0),
      });
    }
  }
  return hits.sort((left, right) => left.line - right.line);
}

function stripNonRuntimeSections(filePath, content) {
  if (!filePath.endsWith(".rs")) {
    return content;
  }
  return stripRustCfgTestItems(content);
}

function sectionContent(content, sectionName) {
  const lines = content.split("\n");
  const start = lines.findIndex((line) => line.trim() === `[${sectionName}]`);
  if (start === -1) {
    return "";
  }
  const section = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^\s*\[.+\]\s*$/.test(lines[index])) {
      break;
    }
    section.push(lines[index]);
  }
  return section.join("\n");
}

function stripRustCfgTestItems(content) {
  const lines = content.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    if (!/^\s*#\[cfg\(test\)\]/.test(lines[index])) {
      continue;
    }

    lines[index] = "";
    let itemIndex = index + 1;
    while (itemIndex < lines.length && lines[itemIndex].trim() === "") {
      lines[itemIndex] = "";
      itemIndex += 1;
    }
    if (itemIndex >= lines.length) {
      break;
    }

    if (/^\s*mod\s+\w+\s*\{/.test(lines[itemIndex])) {
      for (let skipped = itemIndex; skipped < lines.length; skipped += 1) {
        lines[skipped] = "";
      }
      break;
    }

    let balance = 0;
    let sawBrace = false;
    for (let skipped = itemIndex; skipped < lines.length; skipped += 1) {
      const line = lines[skipped];
      for (const char of line) {
        if (char === "{") {
          sawBrace = true;
          balance += 1;
        } else if (char === "}") {
          balance -= 1;
        }
      }
      lines[skipped] = "";
      if ((sawBrace && balance <= 0) || (!sawBrace && line.trim().endsWith(";"))) {
        index = skipped;
        break;
      }
    }
  }
  return lines.join("\n");
}

function lineNumberForIndex(content, index) {
  return content.slice(0, index).split("\n").length;
}

async function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const appDir = path.resolve(scriptDir, "..");
  const repoRoot = path.resolve(appDir, "../..");
  const hits = [];

  for (const relativePath of RUNTIME_CHECK_FILES) {
    const absolutePath = path.join(appDir, relativePath);
    const content = await fs.readFile(absolutePath, "utf8");
    hits.push(...findForbiddenRuntimeHits(relativePath, content));
  }

  const contractErrors = productRuntimeContractErrors({
    pyproject: await fs.readFile(path.join(repoRoot, "pyproject.toml"), "utf8"),
    cargoManifest: await fs.readFile(
      path.join(repoRoot, "crates/emuchef-rust-backend/Cargo.toml"),
      "utf8",
    ),
    tauriConfig: await fs.readFile(path.join(appDir, "src-tauri/tauri.conf.json"), "utf8"),
    packaging: await fs.readFile(path.join(appDir, "scripts/sidecar-packaging.mjs"), "utf8"),
    pythonMainExists: await fileExists(path.join(repoRoot, "src/emuchef/__main__.py")),
  });
  for (const error of contractErrors) {
    hits.push({ filePath: "product runtime contract", token: error, line: 0 });
  }

  if (hits.length > 0) {
    console.error("check-no-python-runtime: forbidden runtime tokens found:");
    for (const hit of hits) {
      console.error(`- ${hit.filePath}:${hit.line}: ${hit.token}`);
    }
    process.exit(1);
  }

  console.log(
    `check-no-python-runtime: checked ${RUNTIME_CHECK_FILES.length} Tauri runtime/build files; no Python runtime tokens found.`,
  );
}

async function fileExists(filePath) {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`check-no-python-runtime: ${error.message}`);
    process.exit(1);
  });
}

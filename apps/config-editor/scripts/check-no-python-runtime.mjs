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
  const hits = [];

  for (const relativePath of RUNTIME_CHECK_FILES) {
    const absolutePath = path.join(appDir, relativePath);
    const content = await fs.readFile(absolutePath, "utf8");
    hits.push(...findForbiddenRuntimeHits(relativePath, content));
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

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`check-no-python-runtime: ${error.message}`);
    process.exit(1);
  });
}

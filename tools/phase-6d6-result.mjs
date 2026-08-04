#!/usr/bin/env node

/**
 * Dependency-free parser for the run-local CODEX_RESULT contract.
 *
 * The parser is intentionally small and strict: it accepts the exact
 * top-level sections used by this run, requires non-empty automated test
 * evidence, and verifies every listed changed file against run.json's
 * allowlist before a result can be called complete.
 */
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const REQUIRED_SECTIONS = [
  "status",
  "summary",
  "phase_status",
  "changed_files",
  "commands_run",
  "tests",
  "acceptance_criteria",
  "blockers",
  "followups",
];
const ALLOWED_SECTIONS = new Set([...REQUIRED_SECTIONS, "physical_evidence"]);

function fail(message) {
  throw new Error(message);
}

function scalar(text) {
  const value = text.trim();
  if (!value) fail("RESULT scalar values must be non-empty");
  return value;
}

function parseList(lines, start, section) {
  const values = [];
  let index = start;
  while (index < lines.length && !/^[A-Za-z][A-Za-z0-9_]*:\s*/.test(lines[index])) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }
    if (!/^\s+-\s+/.test(line)) fail(section + " must contain list entries");
    const first = line.replace(/^\s+-\s+/, "");
    const item = {};
    if (/^(?:command|result|criterion|status|path|file|scenario|repetition|outcome|details|reason|notes):\s+/.test(first)) {
      const separator = first.indexOf(": ");
      item[first.slice(0, separator)] = scalar(first.slice(separator + 2));
      index += 1;
      while (index < lines.length && /^\s{4,}[A-Za-z][A-Za-z0-9_]*:\s*/.test(lines[index])) {
        const nested = lines[index].trim();
        const nestedSeparator = nested.indexOf(": ");
        item[nested.slice(0, nestedSeparator)] = scalar(nested.slice(nestedSeparator + 2));
        index += 1;
      }
      values.push(item);
    } else {
      values.push(scalar(first));
      index += 1;
    }
  }
  return { values, next: index };
}

export function parseResult(text) {
  if (typeof text !== "string" || !text.startsWith("# CODEX_RESULT\n")) fail("RESULT must begin with # CODEX_RESULT");
  const lines = text.split(/\r?\n/).slice(1);
  const result = {};
  let index = 0;
  while (index < lines.length) {
    if (!lines[index].trim()) {
      index += 1;
      continue;
    }
    const header = lines[index].match(/^([A-Za-z][A-Za-z0-9_]*):\s*(.*)$/);
    if (!header) fail("RESULT contains an invalid line: " + lines[index]);
    const [, name, value] = header;
    if (!ALLOWED_SECTIONS.has(name)) fail("RESULT contains unknown section " + name);
    if (value.trim()) {
      result[name] = scalar(value);
      index += 1;
      continue;
    }
    const parsed = parseList(lines, index + 1, name);
    result[name] = parsed.values;
    index = parsed.next;
  }
  return result;
}

/** Return parser-visible entry counts for the repository review compatibility check. */
export function resultEntryCounts(result) {
  return Object.fromEntries([
    "changed_files", "commands_run", "tests", "acceptance_criteria", "blockers", "followups", "physical_evidence",
  ].map((section) => [section, Array.isArray(result[section]) ? result[section].length : 0]));
}

function globRegex(pattern) {
  let source = "";
  for (let index = 0; index < pattern.length; index += 1) {
    const character = pattern[index];
    if (character === "*") {
      if (pattern[index + 1] === "*") {
        if (pattern[index + 2] === "/") {
          source += "(?:.*/)?";
          index += 2;
        } else {
          source += ".*";
          index += 1;
        }
      } else {
        source += "[^/]*";
      }
    } else {
      source += character.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
    }
  }
  return new RegExp("^" + source + "$");
}

function authorized(pathName, patterns) {
  return patterns.some((pattern) => globRegex(pattern).test(pathName));
}

function assertNonEmptyList(result, name) {
  if (!Array.isArray(result[name]) || result[name].length === 0) fail(name + " must be a non-empty list");
}

function repositoryChangedFiles(root) {
  const tracked = execFileSync("git", ["diff", "--name-only", "--relative", "HEAD", "--"], {
    cwd: root,
    encoding: "utf8",
  });
  const untracked = execFileSync("git", ["ls-files", "--others", "--exclude-standard"], {
    cwd: root,
    encoding: "utf8",
  });
  return [...new Set(`${tracked}\n${untracked}`.split(/\r?\n/).filter(Boolean))].sort();
}

function assertExactChangedFiles(declaredFiles, actualFiles) {
  const declared = new Set(declaredFiles);
  const actual = new Set(actualFiles);
  const omitted = [...actual].filter((file) => !declared.has(file)).sort();
  const extra = [...declared].filter((file) => !actual.has(file)).sort();
  if (omitted.length > 0) fail(`RESULT omitted actual changed files: ${omitted.join(", ")}`);
  if (extra.length > 0) fail(`RESULT declared files that are not actually changed: ${extra.join(", ")}`);
}

export function validateResult(result, options = {}) {
  for (const section of REQUIRED_SECTIONS) {
    if (!(section in result)) fail("RESULT is missing " + section);
  }
  if (!["blocked", "completed"].includes(result.status)) fail("RESULT status must be blocked or completed");
  if (typeof result.summary !== "string" || result.summary.length === 0) fail("RESULT summary must be non-empty");
  if (typeof result.phase_status !== "string" || !/Phase 6D remains In progress/i.test(result.phase_status)) {
    fail("RESULT must keep Phase 6D In progress until physical closure");
  }
  assertNonEmptyList(result, "changed_files");
  assertNonEmptyList(result, "commands_run");
  assertNonEmptyList(result, "tests");
  assertNonEmptyList(result, "acceptance_criteria");
  assertNonEmptyList(result, "blockers");
  assertNonEmptyList(result, "followups");
  for (const entry of [...result.commands_run, ...result.tests]) {
    if (typeof entry === "string") continue;
    if (typeof entry !== "object" || typeof entry.command !== "string" || typeof entry.result !== "string") fail("commands_run and tests entries require a canonical scalar or command/result mapping");
  }
  for (const entry of result.acceptance_criteria) {
    if (typeof entry === "string") continue;
    if (typeof entry !== "object" || typeof entry.criterion !== "string" || typeof entry.status !== "string") fail("acceptance_criteria entries require a canonical scalar or criterion/status mapping");
  }
  const root = options.root ?? fileURLToPath(new URL("../", import.meta.url));
  const runPath = options.runPath ?? path.join(root, ".chatgpt/codex-runs/2026-08-03T213321Z-phase-6d-6-physical-interruption-qualification/run.json");
  const run = options.run ?? JSON.parse(readFileSync(runPath, "utf8"));
  const actualChangedFiles = options.actualChangedFiles ?? repositoryChangedFiles(root);
  assertExactChangedFiles(result.changed_files, actualChangedFiles);
  for (const file of result.changed_files) {
    if (typeof file !== "string" || !authorized(file, run.allowed_paths)) fail("changed file is outside run allowlist: " + file);
    if (!existsSync(path.join(root, file))) fail("changed file does not exist: " + file);
  }
  if (result.status === "completed" && result.blockers.some((entry) => /physical|hardware|operator|in progress/i.test(String(entry)))) {
    fail("a completed RESULT cannot retain physical or operator blockers");
  }
  return true;
}

export function validateResultFile(resultPath) {
  const root = fileURLToPath(new URL("../", import.meta.url));
  return validateResult(parseResult(readFileSync(resultPath, "utf8")), { root });
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    validateResultFile(process.argv[2] ?? path.join(
      fileURLToPath(new URL("../", import.meta.url)),
      ".chatgpt/codex-runs/2026-08-03T213321Z-phase-6d-6-physical-interruption-qualification/RESULT.md",
    ));
    process.stdout.write("Phase 6D.6 RESULT contract valid.\n");
  } catch (error) {
    process.stderr.write(String(error instanceof Error ? error.message : error) + "\n");
    process.exitCode = 1;
  }
}

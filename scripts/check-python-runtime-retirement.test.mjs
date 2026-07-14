import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  TOOLING_PYTHON_ALLOWLIST,
  analyzeRepositoryEntries,
  findForbiddenRuntimeTokens,
  isPythonArtifactPath,
  isPythonProjectPath,
  listRepositoryFiles,
  main,
  normalizeRepositoryPath,
  readRepositoryEntries,
  shouldScanRuntimeContent,
  stripRustTestModules,
} from "./check-python-runtime-retirement.mjs";

test("the tooling-only Python allowlist is intentionally empty", () => {
  assert.deepEqual(TOOLING_PYTHON_ALLOWLIST, []);
});

test("normalizes repository paths and identifies Python artifacts", () => {
  assert.equal(normalizeRepositoryPath("./src\\emuchef\\main.py"), "src/emuchef/main.py");
  for (const filePath of ["tool.py", "tool.pyi", "tool.pyw", "tool.pyc", "tool.pyo", "src/__pycache__/tool.bin"]) {
    assert.equal(isPythonArtifactPath(filePath), true, filePath);
  }
  assert.equal(isPythonArtifactPath("docs/python-runtime.md"), false);
});

test("identifies Python project metadata without matching ordinary manifests", () => {
  for (const filePath of ["pyproject.toml", "setup.py", "setup.cfg", "tox.ini", "Pipfile", "Pipfile.lock", "poetry.lock", "uv.lock", "requirements-dev.txt"]) {
    assert.equal(isPythonProjectPath(filePath), true, filePath);
  }
  assert.equal(isPythonProjectPath("package.json"), false);
});

test("limits content scans to active runtime surfaces", () => {
  assert.equal(shouldScanRuntimeContent("apps/config-editor/package.json"), true);
  assert.equal(shouldScanRuntimeContent(".github/workflows/ci.yml"), true);
  assert.equal(shouldScanRuntimeContent("crates/runtime/src/main.rs"), true);
  assert.equal(shouldScanRuntimeContent("docs/adr/history.md"), false);
  assert.equal(shouldScanRuntimeContent("crates/runtime/tests/fixture.rs"), false);
  assert.equal(shouldScanRuntimeContent("apps/config-editor/scripts/check-macos-bundle.mjs"), false);
  assert.equal(shouldScanRuntimeContent("scripts/check-python-runtime-retirement.mjs"), false);
});

test("detects every retired runtime token category", () => {
  const content = [
    '"dev": "python3 -m emuchef"',
    '"other": "uv run emuchef_editor.api.server"',
    'const bridge = "python_bridge";',
    'const ui = "PySide6";',
    'const legacy = "emuchef-python-legacy emuchef-plan-shadow plan_shadow";',
    'const flags = "--backend --python-runtime --rust-planner-bin";',
    'const env = "EMUCHEF_PLANNER_BACKEND EMUCHEF_PYTHON_RUNTIME";',
  ].join("\n");
  const hits = findForbiddenRuntimeTokens("apps/example/package.json", content);
  assert.deepEqual(new Set(hits.map((hit) => hit.reason)), new Set([
    "Python or uv command",
    "Python module entrypoint",
    "Python runtime module",
    "retired runtime name",
    "backend selector or compatibility flag",
    "backend-selection environment variable",
  ]));
});

test("does not match harmless substrings or historical documentation", () => {
  assert.deepEqual(findForbiddenRuntimeTokens("apps/example/package.json", '"scripts":{"ok":"echo pythonista uvicorn backend"}'), []);
  assert.deepEqual(analyzeRepositoryEntries([
    { path: "docs/adr/0001.md", content: "python -m emuchef and emuchef-plan-shadow are historical" },
    { path: "crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1/result.json", content: "Python Golden Name" },
  ]), []);
});

test("strips Rust cfg-test modules but retains production code", () => {
  const source = 'fn production() { Command::new("cargo"); }\n#[cfg(test)]\nmod tests {\n  const COMMAND: &str = "python";\n  fn nested() { if true { println!("uv"); } }\n}\nfn after() { Command::new("cargo"); }\n';
  const stripped = stripRustTestModules(source);
  assert.match(stripped, /fn production/);
  assert.match(stripped, /fn after/);
  assert.doesNotMatch(stripped, /python|println/);
  assert.deepEqual(findForbiddenRuntimeTokens("crates/runtime/src/main.rs", source), []);
  assert.equal(findForbiddenRuntimeTokens("crates/runtime/src/main.rs", 'fn production() { Command::new("python"); }').length, 1);
});

test("strips an unterminated Rust cfg-test module through end of file", () => {
  const source = 'fn production() {}\n#[cfg(test)]\nmod tests {\n  fn nested() { if true { println!("python"); } }\n';
  assert.equal(stripRustTestModules(source), "fn production() {}");
});

test("reports structural and content violations in stable path order", () => {
  const hits = analyzeRepositoryEntries([
    { path: "z/tool.py", content: "" },
    { path: "pyproject.toml", content: "" },
    { path: "apps/example/package.json", content: '"dev":"python -m emuchef"' },
    { path: "docs/current-state.md" },
  ]);
  assert.deepEqual(hits.map((hit) => hit.path), ["apps/example/package.json", "apps/example/package.json", "pyproject.toml", "z/tool.py"]);
});

test("reads only existing regular files and skips cached deletions", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-retirement-"));
  try {
    fs.mkdirSync(path.join(root, "apps/example"), { recursive: true });
    fs.writeFileSync(path.join(root, "apps/example/package.json"), '{"scripts":{"test":"node test.mjs"}}');
    fs.mkdirSync(path.join(root, "directory"));
    const entries = readRepositoryEntries(root, ["apps/example/package.json", "missing.py", "directory"]);
    assert.deepEqual(entries, [{ path: "apps/example/package.json", content: '{"scripts":{"test":"node test.mjs"}}' }]);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("lists tracked and non-ignored files and runs both CLI outcomes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "emuchef-retirement-git-"));
  try {
    execFileSync("git", ["init", "-q"], { cwd: root });
    fs.writeFileSync(path.join(root, "package.json"), '{"scripts":{"test":"node test.mjs"}}');
    assert.deepEqual(listRepositoryFiles(root), ["package.json"]);

    const output = [];
    const errors = [];
    assert.equal(main({ repoRoot: root, writeOutput: (message) => output.push(message), writeError: (message) => errors.push(message) }), 0);
    assert.equal(output.length, 1);
    assert.deepEqual(errors, []);

    fs.writeFileSync(path.join(root, "runtime.py"), "print('retired')\n");
    assert.equal(main({ repoRoot: root, writeOutput: (message) => output.push(message), writeError: (message) => errors.push(message) }), 1);
    assert.equal(errors.length, 2);
    assert.match(errors[0], /prohibited Python runtime surface/);
    assert.match(errors[1], /runtime\.py: Python source or bytecode/);

    fs.rmSync(path.join(root, "runtime.py"));
    fs.writeFileSync(path.join(root, "package.json"), '{\n  "scripts": { "dev": "python -m emuchef" }\n}\n');
    errors.length = 0;
    assert.equal(main({ repoRoot: root, writeOutput: (message) => output.push(message), writeError: (message) => errors.push(message) }), 1);
    assert.equal(errors.length, 3);
    assert.match(errors.join("\n"), /package\.json:2: Python or uv command: python/);
    assert.match(errors.join("\n"), /package\.json:2: Python module entrypoint: emuchef/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("the default CLI context validates the current repository", () => {
  assert.equal(main(), 0);
});

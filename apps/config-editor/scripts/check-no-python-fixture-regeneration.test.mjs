import assert from "node:assert/strict";
import test from "node:test";

import {
  collectActiveCheckSurfacesFromPackage,
  findForbiddenFixtureRegenerationHits,
} from "./check-no-python-fixture-regeneration.mjs";

test("flags Python fixture generator invocations with active-surface reason", () => {
  const hits = findForbiddenFixtureRegenerationHits(
    "crates/emuchef-rust-backend/tests/generate.rs",
    [
      "let command = \"PYTHONPATH=src python3 - <<'PY'\";",
      "print('write crates/emuchef-rust-backend/tests/fixtures/python_goldens/example.json')",
    ].join("\n"),
    "Rust backend test file",
  );

  assert.deepEqual(hits, [
    {
      filePath: "crates/emuchef-rust-backend/tests/generate.rs",
      line: 1,
      matchedLine: "let command = \"PYTHONPATH=src python3 - <<'PY'\";",
      reason: "Rust backend test file",
      rule: "Python fixture/golden generator invocation",
    },
  ]);
});

test("allows checked-in Python golden consumption in Rust tests", () => {
  const hits = findForbiddenFixtureRegenerationHits(
    "crates/emuchef-rust-backend/tests/phase6g_commands.rs",
    [
      "fn focused_phase6g_results_match_python_goldens() {",
      "    let text = fs::read_to_string(golden_path(\"phase6g.result.json\")).expect(\"Python golden should be readable\");",
      "    assert_eq!(read_golden(\"phase6g.result.json\"), response);",
      "    let value = \"Python Golden Name\";",
      "}",
    ].join("\n"),
    "Rust backend test file",
  );

  assert.deepEqual(hits, []);
});

test("flags active npm scripts that invoke Python fixture regeneration", () => {
  const hits = findForbiddenFixtureRegenerationHits(
    "package.json",
    '"regen:goldens": "PYTHONPATH=src uv run python -m emuchef validate > crates/emuchef-rust-backend/tests/fixtures/python_goldens/example.json"',
    "package script regen:goldens reachable from check:rust-runtime",
  );

  assert.deepEqual(
    hits.map((hit) => ({
      line: hit.line,
      reason: hit.reason,
      rule: hit.rule,
    })),
    [
      {
        line: 1,
        reason: "package script regen:goldens reachable from check:rust-runtime",
        rule: "Python fixture/golden generator invocation",
      },
    ],
  );
});

test("collects scripts reachable from check:rust-runtime without treating docs as active", () => {
  const packageJson = JSON.stringify({
    scripts: {
      "check:rust-runtime": "npm run check:no-python-fixture-regeneration && npm run typecheck",
      "check:no-python-fixture-regeneration": "node --test scripts/check-no-python-fixture-regeneration.test.mjs && node scripts/check-no-python-fixture-regeneration.mjs",
      typecheck: "tsc --noEmit",
      "refresh:goldens": "PYTHONPATH=src python3 - <<'PY'",
    },
  });

  const surfaces = collectActiveCheckSurfacesFromPackage(packageJson);

  assert.deepEqual(
    surfaces.map((surface) => surface.filePath),
    ["package.json", "package.json", "package.json"],
  );
  assert.deepEqual(
    surfaces.map((surface) => surface.reason),
    [
      "package script check:rust-runtime reachable from check:rust-runtime",
      "package script check:no-python-fixture-regeneration reachable from check:rust-runtime",
      "package script typecheck reachable from check:rust-runtime",
    ],
  );
});

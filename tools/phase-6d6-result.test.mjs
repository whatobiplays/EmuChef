import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { parseResult, resultEntryCounts, validateResult } from "./phase-6d6-result.mjs";

const resultPath = ".chatgpt/codex-runs/2026-08-03T213321Z-phase-6d-6-physical-interruption-qualification/RESULT.md";
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const fixtureRun = { allowed_paths: ["tools/phase-6d6-result.mjs"] };
const fixtureText = `# CODEX_RESULT
status: blocked
summary: Parser fixture remains blocked without physical evidence.
phase_status: Phase 6D remains In progress
changed_files:
  - tools/phase-6d6-result.mjs
commands_run:
  - command: node tools/phase-6d6-result.mjs
    result: Passed; parser fixture command completed.
tests:
  - command: node --test tools/phase-6d6-result.test.mjs
    result: Passed; parser fixture test completed.
acceptance_criteria:
  - criterion: parser fixture
    status: implemented
blockers:
  - physical evidence is unavailable in the fixture.
followups:
  - run physical qualification when safely gated.
`;

function loadCurrentResult() {
  if (existsSync(resultPath)) {
    const result = parseResult(readFileSync(resultPath, "utf8"));
    return { result, options: { actualChangedFiles: result.changed_files } };
  }
  return {
    result: parseResult(fixtureText),
    options: { root: repositoryRoot, run: fixtureRun, actualChangedFiles: ["tools/phase-6d6-result.mjs"] },
  };
}

test("the checked-in Phase 6D.6 result parses and has non-empty test evidence", () => {
  const { result, options } = loadCurrentResult();
  assert.equal(result.status, "blocked");
  assert.equal(result.phase_status, "Phase 6D remains In progress");
  assert.ok(result.tests.length > 0);
  assert.doesNotThrow(() => validateResult(result, options));
});

test("the result validator rejects omitted actual changed files", () => {
  const { result, options } = loadCurrentResult();
  assert.throws(
    () => validateResult(result, {
      ...options,
      actualChangedFiles: [...result.changed_files, "unauthorized/omitted.txt"],
    }),
    /omitted actual changed files/,
  );
});

test("the result parser rejects missing tests, unauthorized files, and false closure", () => {
  const { result, options } = loadCurrentResult();
  assert.throws(() => validateResult({ ...result, tests: [] }, options), /tests/);
  assert.throws(
    () => validateResult(
      { ...result, changed_files: [...result.changed_files, "Cargo.lock"] },
      { ...options, actualChangedFiles: [...result.changed_files, "Cargo.lock"] },
    ),
    /outside run allowlist/,
  );
  assert.throws(
    () => validateResult({
      ...result,
      status: "completed",
      blockers: ["physical hardware remains unavailable"],
    }, options),
    /completed RESULT cannot retain physical/,
  );
});

test("the parser validates a repository-independent fixture when run metadata is absent", () => {
  const result = parseResult(fixtureText);
  assert.doesNotThrow(() => validateResult(result, {
    root: repositoryRoot,
    run: fixtureRun,
    actualChangedFiles: ["tools/phase-6d6-result.mjs"],
  }));
});

test("canonical review bullets remain distinct instead of collapsing to one nested entry", () => {
  const canonical = `# CODEX_RESULT
status: blocked
summary: Canonical parser compatibility fixture.
phase_status: Phase 6D remains In progress
changed_files:
  - tools/phase-6d6-result.mjs
  - tools/phase-6d6-result.test.mjs
commands_run:
  - \`rtk cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml\` — passed.
  - \`rtk node --test tools/phase-6d6-evidence.test.mjs\` — passed.
tests:
  - Backend: 704 passed, 14 ignored.
  - Evidence validator: 12 passed.
acceptance_criteria:
  - Host timing evidence is measured and branch-consistent.
  - UI smoke requires two composite repetitions.
blockers:
  - Physical hardware and operator gates are unavailable.
followups:
  - Run the gated physical matrix.
`;
  const parsed = parseResult(canonical);
  const counts = resultEntryCounts(parsed);
  assert.deepEqual(counts, {
    changed_files: 2,
    commands_run: 2,
    tests: 2,
    acceptance_criteria: 2,
    blockers: 1,
    followups: 1,
    physical_evidence: 0,
  });
  assert.doesNotThrow(() => validateResult(parsed, {
    root: repositoryRoot,
    run: { allowed_paths: ["tools/phase-6d6-result.mjs", "tools/phase-6d6-result.test.mjs"] },
    actualChangedFiles: ["tools/phase-6d6-result.mjs", "tools/phase-6d6-result.test.mjs"],
  }));
});

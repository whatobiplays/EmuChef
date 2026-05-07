import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAdvancedInternalsCommand,
  formatJsonDraft,
  parseAdvancedJsonDraft,
  revertJsonDraft,
} from "../src/components/advancedStepInternals.logic.js";

test("buildAdvancedInternalsCommand builds explicit sidecar payloads", () => {
  assert.deepEqual(
    buildAdvancedInternalsCommand(
      "constraints",
      "copy",
      { capabilities: ["shared_storage_write"], conflictsWith: [] },
      {},
    ),
    {
      type: "UpdateStepConstraints",
      stepId: "copy",
      constraints: { capabilities: ["shared_storage_write"], conflictsWith: [] },
    },
  );
  assert.deepEqual(buildAdvancedInternalsCommand("skipIf", "copy", [{ type: "path_exists", params: {} }], []), {
    type: "UpdateStepSkipIf",
    stepId: "copy",
    skipIf: [{ type: "path_exists", params: {} }],
  });
  assert.deepEqual(buildAdvancedInternalsCommand("verify", "copy", [{ type: "path_exists", params: {} }], []), {
    type: "UpdateStepVerify",
    stepId: "copy",
    verify: [{ type: "path_exists", params: {} }],
  });
});

test("invalid JSON does not produce a parsed value", () => {
  assert.deepEqual(parseAdvancedJsonDraft("constraints", "{"), {
    ok: false,
    error: "Enter valid JSON.",
  });
});

test("top-level shape errors do not silently convert null or clear values", () => {
  assert.deepEqual(parseAdvancedJsonDraft("constraints", "null"), {
    ok: false,
    error: "Constraints must be a JSON object.",
  });
  assert.deepEqual(parseAdvancedJsonDraft("skipIf", "null"), {
    ok: false,
    error: "skip_if must be a JSON array.",
  });
  assert.deepEqual(parseAdvancedJsonDraft("verify", "null"), {
    ok: false,
    error: "Verify must be a JSON array.",
  });
});

test("nested JSON null remains a literal JSON value", () => {
  assert.deepEqual(parseAdvancedJsonDraft("verify", '[{"type":"custom","params":{"target":null}}]'), {
    ok: true,
    value: [{ type: "custom", params: { target: null } }],
  });
});

test("unchanged parsed JSON and whitespace-only edits do not produce commands", () => {
  const current = { capabilities: [], conflictsWith: ["copy"] };
  const parsed = parseAdvancedJsonDraft(
    "constraints",
    '{\n  "capabilities": [],\n  "conflictsWith": ["copy"]\n}',
  );

  assert.deepEqual(parsed, { ok: true, value: current });
  assert.equal(buildAdvancedInternalsCommand("constraints", "copy", parsed.ok ? parsed.value : null, current), null);
});

test("revertJsonDraft restores draft from the current document value", () => {
  assert.equal(revertJsonDraft({ capabilities: [], conflictsWith: ["copy"] }), '{\n  "capabilities": [],\n  "conflictsWith": [\n    "copy"\n  ]\n}');
});

test("formatJsonDraft preserves nested ref-shaped objects as ordinary JSON", () => {
  assert.equal(
    formatJsonDraft([{ type: "custom", params: { target: { ref: "inputs.source_dir" } } }]),
    '[\n  {\n    "type": "custom",\n    "params": {\n      "target": {\n        "ref": "inputs.source_dir"\n      }\n    }\n  }\n]',
  );
});

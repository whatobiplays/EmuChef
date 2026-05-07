import assert from "node:assert/strict";
import test from "node:test";

import { normalizeEditableText, textInputGuardProps } from "../src/components/textInputGuards.logic.js";
import { parseAdvancedJsonDraft } from "../src/components/advancedStepInternals.logic.js";

test("normalizeEditableText replaces smart double quotes with ASCII quotes", () => {
  assert.equal(normalizeEditableText("“quoted”"), '"quoted"');
});

test("normalizeEditableText replaces smart single quotes with ASCII apostrophes", () => {
  assert.equal(normalizeEditableText("‘quoted’"), "'quoted'");
});

test("normalizeEditableText leaves already ASCII-stable text unchanged", () => {
  assert.equal(normalizeEditableText('{"key": "value"}'), '{"key": "value"}');
});

test("advanced JSON parsing accepts smart-quoted drafts after input normalization", () => {
  const normalized = normalizeEditableText('{\n  “capabilities”: [],\n  “conflicts_with”: [“install_retroarch”]\n}');

  assert.deepEqual(parseAdvancedJsonDraft("constraints", normalized), {
    ok: true,
    value: {
      capabilities: [],
      conflicts_with: ["install_retroarch"],
    },
  });
});

test("textInputGuardProps disables browser writing aids for editable text controls", () => {
  assert.deepEqual(textInputGuardProps, {
    autoCapitalize: "off",
    autoCorrect: "off",
    spellCheck: false,
  });
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAdvancedInternalsCommand,
  buildAddConstraintValue,
  buildAddVerifyEntry,
  buildUpdateConstraintValue,
  classifyConstraintsDto,
  constraintsCommandValue,
  buildVerifyEntryJsonUpdate,
  buildVerifyKnownFieldUpdate,
  classifyVerifyEntry,
  editorValueForAdvancedField,
  formatJsonDraft,
  moveVerifyEntry,
  parseAdvancedJsonDraft,
  removeConstraintValue,
  removeVerifyEntry,
  revertJsonDraft,
  toAuthoredConstraintsJsonValue,
  verifyFieldForType,
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

test("editorValueForAdvancedField converts constraints DTO keys to authored keys", () => {
  assert.deepEqual(
    editorValueForAdvancedField("constraints", {
      capabilities: ["shared_storage_write"],
      conflictsWith: ["stop_app"],
    }),
    {
      capabilities: ["shared_storage_write"],
      conflicts_with: ["stop_app"],
    },
  );
});

test("classifyConstraintsDto accepts only lossless supported DTO shapes", () => {
  assert.deepEqual(classifyConstraintsDto({}), {
    kind: "structured",
    value: { capabilities: [], conflictsWith: [] },
  });
  assert.deepEqual(classifyConstraintsDto({ capabilities: [] }), {
    kind: "structured",
    value: { capabilities: [], conflictsWith: [] },
  });
  assert.deepEqual(classifyConstraintsDto({ conflictsWith: [] }), {
    kind: "structured",
    value: { capabilities: [], conflictsWith: [] },
  });
  assert.deepEqual(
    classifyConstraintsDto({
      capabilities: ["app_data_write"],
      conflictsWith: ["step_a"],
    }),
    {
      kind: "structured",
      value: {
        capabilities: ["app_data_write"],
        conflictsWith: ["step_a"],
      },
    },
  );
});

test("classifyConstraintsDto routes unsupported constraints to raw JSON fallback", () => {
  assert.deepEqual(classifyConstraintsDto(null), {
    kind: "raw",
    authoredJsonValue: null,
    reason: "not_object",
  });
  assert.deepEqual(classifyConstraintsDto({ custom: true }), {
    kind: "raw",
    authoredJsonValue: { custom: true },
    reason: "unknown_top_level_key",
  });
  assert.deepEqual(classifyConstraintsDto({ capabilities: "app_data_write" }), {
    kind: "raw",
    authoredJsonValue: { capabilities: "app_data_write" },
    reason: "unsupported_field_shape",
  });
  assert.deepEqual(classifyConstraintsDto({ conflictsWith: "step_a" }), {
    kind: "raw",
    authoredJsonValue: { conflicts_with: "step_a" },
    reason: "unsupported_field_shape",
  });
  assert.deepEqual(classifyConstraintsDto({ capabilities: ["app_data_write", 42] }), {
    kind: "raw",
    authoredJsonValue: { capabilities: ["app_data_write", 42] },
    reason: "non_string_capabilities",
  });
  assert.deepEqual(classifyConstraintsDto({ conflictsWith: ["step_a", 42] }), {
    kind: "raw",
    authoredJsonValue: { conflicts_with: ["step_a", 42] },
    reason: "non_string_conflictsWith",
  });
  assert.deepEqual(classifyConstraintsDto({ conflicts_with: [], conflictsWith: [] }), {
    kind: "raw",
    authoredJsonValue: { conflicts_with: [], conflictsWith: [] },
    reason: "ambiguous_conflict_fields",
  });
});

test("constraint structured edits preserve sibling fields and explicit empty arrays", () => {
  const current = { capabilities: ["app_data_write"], conflictsWith: ["step_a"] };

  assert.deepEqual(buildAddConstraintValue(current, "capabilities", "root_shell"), {
    capabilities: ["app_data_write", "root_shell"],
    conflictsWith: ["step_a"],
  });
  assert.deepEqual(buildAddConstraintValue(current, "conflictsWith", "step_b"), {
    capabilities: ["app_data_write"],
    conflictsWith: ["step_a", "step_b"],
  });
  assert.deepEqual(buildUpdateConstraintValue(current, "capabilities", 0, "shared_storage_write"), {
    capabilities: ["shared_storage_write"],
    conflictsWith: ["step_a"],
  });
  assert.deepEqual(buildUpdateConstraintValue(current, "conflictsWith", 0, "step_b"), {
    capabilities: ["app_data_write"],
    conflictsWith: ["step_b"],
  });
  assert.equal(buildUpdateConstraintValue(current, "conflictsWith", 0, "step_a"), null);
  assert.deepEqual(removeConstraintValue(current, "capabilities", 0), {
    capabilities: [],
    conflictsWith: ["step_a"],
  });
  assert.deepEqual(removeConstraintValue(current, "conflictsWith", 0), {
    capabilities: ["app_data_write"],
    conflictsWith: [],
  });
});

test("constraint JSON conversion maps DTO conflictsWith to authored conflicts_with without dropping raw fields", () => {
  assert.deepEqual(toAuthoredConstraintsJsonValue({ capabilities: [], conflictsWith: ["step_a"] }), {
    capabilities: [],
    conflicts_with: ["step_a"],
  });
  assert.deepEqual(
    toAuthoredConstraintsJsonValue({
      capabilities: ["app_data_write", 42],
      conflictsWith: ["step_a", 42],
      custom: { keep: true },
    }),
    {
      capabilities: ["app_data_write", 42],
      conflicts_with: ["step_a", 42],
      custom: { keep: true },
    },
  );
});

test("constraintsCommandValue maps authored conflicts_with to command conflictsWith", () => {
  assert.deepEqual(constraintsCommandValue({ capabilities: [], conflicts_with: ["step_a"] }), {
    ok: true,
    value: { capabilities: [], conflictsWith: ["step_a"] },
  });
  assert.deepEqual(constraintsCommandValue({ capabilities: [], custom: true }), {
    ok: true,
    value: { capabilities: [], custom: true },
  });
  assert.deepEqual(constraintsCommandValue({ conflicts_with: [], conflictsWith: [] }), {
    ok: false,
    error: "Use either conflicts_with or conflictsWith, not both.",
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

test("constraints JSON accepts unsupported raw shapes and rejects ambiguous conflict fields", () => {
  assert.deepEqual(parseAdvancedJsonDraft("constraints", '{"capabilities": [], "custom": true}'), {
    ok: true,
    value: { capabilities: [], custom: true },
  });
  assert.deepEqual(parseAdvancedJsonDraft("constraints", '{"capabilities": [], "conflicts_with": [], "conflictsWith": []}'), {
    ok: false,
    error: "Use either conflicts_with or conflictsWith, not both.",
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
    '{\n  "capabilities": [],\n  "conflicts_with": ["copy"]\n}',
  );

  assert.deepEqual(parsed, { ok: true, value: { capabilities: [], conflictsWith: ["copy"] } });
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

test("classifyVerifyEntry renders only exact supported shapes with string known fields as structured rows", () => {
  assert.deepEqual(classifyVerifyEntry({ type: "path_exists", params: { path: "/sdcard/RetroArch" } }), {
    kind: "structured",
    type: "path_exists",
    fieldName: "path",
    fieldValue: "/sdcard/RetroArch",
  });
  assert.deepEqual(classifyVerifyEntry({ type: "file_exists", params: { path: "/sdcard/retroarch.cfg" } }), {
    kind: "structured",
    type: "file_exists",
    fieldName: "path",
    fieldValue: "/sdcard/retroarch.cfg",
  });
  assert.deepEqual(classifyVerifyEntry({ type: "package_installed", params: { package_name: "org.retroarch" } }), {
    kind: "structured",
    type: "package_installed",
    fieldName: "package_name",
    fieldValue: "org.retroarch",
  });

  assert.deepEqual(classifyVerifyEntry({ type: "path_exists", params: {} }), { kind: "json" });
  assert.deepEqual(classifyVerifyEntry({ type: "path_exists", params: { path: 42 } }), { kind: "json" });
  assert.deepEqual(classifyVerifyEntry({ type: "path_exists", params: { path: "/tmp" }, future: true }), { kind: "json" });
  assert.deepEqual(classifyVerifyEntry({ type: "custom", params: { path: "/tmp" } }), { kind: "json" });
  assert.deepEqual(classifyVerifyEntry("custom"), { kind: "json" });
});

test("verify structured field edits preserve unknown params fields and skip no-op edits", () => {
  const verify = [
    {
      type: "path_exists",
      params: {
        path: "/sdcard/old",
        privileged: true,
        nested: { ref: "inputs.target" },
      },
    },
  ];

  assert.deepEqual(buildVerifyKnownFieldUpdate(verify, 0, "/sdcard/new"), {
    ok: true,
    value: [
      {
        type: "path_exists",
        params: {
          path: "/sdcard/new",
          privileged: true,
          nested: { ref: "inputs.target" },
        },
      },
    ],
  });
  assert.equal(buildVerifyKnownFieldUpdate(verify, 0, "/sdcard/old"), null);
  assert.deepEqual(buildVerifyKnownFieldUpdate(verify, 0, "   "), {
    ok: false,
    error: "path is required.",
  });
});

test("verify per-entry JSON edits round-trip accepted fallback shapes unchanged", () => {
  const verify = [{ type: "custom_future_check", params: { target: null, nested: { ref: "inputs.target" } } }];

  assert.equal(buildVerifyEntryJsonUpdate(verify, 0, formatJsonDraft(verify[0])), null);
  assert.deepEqual(buildVerifyEntryJsonUpdate(verify, 0, '{"type":"custom_future_check","params":{"target":"next"}}'), {
    ok: true,
    value: [{ type: "custom_future_check", params: { target: "next" } }],
  });
});

test("verify per-entry JSON rejects invalid drafts and command-rejected condition shapes", () => {
  const verify = [{ type: "custom", params: {} }];

  assert.deepEqual(buildVerifyEntryJsonUpdate(verify, 0, "{"), { ok: false, error: "Enter valid JSON." });
  assert.deepEqual(buildVerifyEntryJsonUpdate(verify, 0, "null"), {
    ok: false,
    error: "Verify entry must be a JSON object.",
  });
  assert.deepEqual(buildVerifyEntryJsonUpdate(verify, 0, '{"type":"custom","params":[] }'), {
    ok: false,
    error: "Verify entry params must be a JSON object.",
  });
  assert.deepEqual(buildVerifyEntryJsonUpdate(verify, 0, '{"type":"custom","params":{},"future":true}'), {
    ok: false,
    error: "Verify entry supports only type and params.",
  });
});

test("verify remove and reorder helpers preserve unsupported entries", () => {
  const verify = [
    { type: "path_exists", params: { path: "/a" } },
    { type: "custom", params: { value: 1 } },
    { type: "package_installed", params: { package_name: "org.example" } },
  ];

  assert.deepEqual(moveVerifyEntry(verify, 1, 0), [
    { type: "custom", params: { value: 1 } },
    { type: "path_exists", params: { path: "/a" } },
    { type: "package_installed", params: { package_name: "org.example" } },
  ]);
  assert.deepEqual(removeVerifyEntry(verify, 1), [
    { type: "path_exists", params: { path: "/a" } },
    { type: "package_installed", params: { package_name: "org.example" } },
  ]);
});

test("verify add helper creates supported condition entries with the known field", () => {
  assert.deepEqual(buildAddVerifyEntry([], "file_exists", "/sdcard/retroarch.cfg"), {
    ok: true,
    value: [{ type: "file_exists", params: { path: "/sdcard/retroarch.cfg" } }],
  });
  assert.deepEqual(buildAddVerifyEntry([], "package_installed", "org.retroarch"), {
    ok: true,
    value: [{ type: "package_installed", params: { package_name: "org.retroarch" } }],
  });
  assert.deepEqual(buildAddVerifyEntry([], "path_exists", ""), {
    ok: false,
    error: "path is required.",
  });
});

test("verifyFieldForType exposes the structured parameter key for supported types only", () => {
  assert.equal(verifyFieldForType("path_exists"), "path");
  assert.equal(verifyFieldForType("file_exists"), "path");
  assert.equal(verifyFieldForType("package_installed"), "package_name");
  assert.equal(verifyFieldForType("custom"), null);
});

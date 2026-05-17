import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAdvancedInternalsCommand,
  buildAddVerifyEntry,
  buildVerifyEntryJsonUpdate,
  buildVerifyKnownFieldUpdate,
  classifyVerifyEntry,
  editorValueForAdvancedField,
  formatJsonDraft,
  moveVerifyEntry,
  parseAdvancedJsonDraft,
  removeVerifyEntry,
  revertJsonDraft,
  verifyFieldForType,
} from "../src/components/advancedStepInternals.logic.js";

test("buildAdvancedInternalsCommand builds explicit sidecar payloads", () => {
  assert.deepEqual(
    buildAdvancedInternalsCommand(
      "constraints",
      "copy",
      { capabilities: ["shared_storage_write"], conflicts_with: [] },
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

test("constraints JSON rejects unrepresentable authored keys before submit", () => {
  assert.deepEqual(parseAdvancedJsonDraft("constraints", '{"conflictsWith": []}'), {
    ok: false,
    error: "Constraints supports only capabilities and conflicts_with.",
  });
  assert.deepEqual(parseAdvancedJsonDraft("constraints", '{"capabilities": [], "custom": true}'), {
    ok: false,
    error: "Constraints supports only capabilities and conflicts_with.",
  });
});

test("nested JSON null remains a literal JSON value", () => {
  assert.deepEqual(parseAdvancedJsonDraft("verify", '[{"type":"custom","params":{"target":null}}]'), {
    ok: true,
    value: [{ type: "custom", params: { target: null } }],
  });
});

test("unchanged parsed JSON and whitespace-only edits do not produce commands", () => {
  const current = { capabilities: [], conflicts_with: ["copy"] };
  const parsed = parseAdvancedJsonDraft(
    "constraints",
    '{\n  "capabilities": [],\n  "conflicts_with": ["copy"]\n}',
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

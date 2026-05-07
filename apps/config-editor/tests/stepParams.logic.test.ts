import assert from "node:assert/strict";
import test from "node:test";

import type { RefIndexDto, StepSpecDto } from "../src/api/types.js";
import {
  buildClearStepParamCommand,
  buildRefPickerOptions,
  buildUpdateStepParamsCommand,
  isAuthoredRefValue,
  orderedParamNames,
  parseJsonParamDraft,
  parseNumberParamDraft,
} from "../src/components/stepParams.logic.js";

const copySpec: StepSpecDto = {
  type: "copy_files",
  label: "Copy Files",
  supported: true,
  primaryOutputName: "copied_paths",
  outputs: [],
  paramOrder: ["source", "dest", "copy_policy"],
  params: {
    source: { mode: "ref", required: true, enumValues: [] },
    dest: { mode: "literal", required: true, enumValues: [] },
    copy_policy: { mode: "literal", required: false, enumValues: ["merge", "sync"] },
  },
  defaults: {},
  refFilters: { source: ["path_list"] },
};

const refIndex: RefIndexDto = {
  inputRefs: ["inputs.source_dir"],
  artifactRefs: ["artifacts.archive.local_path"],
  stepRefs: ["steps.extract"],
  stepOutputRefs: ["steps.extract.outputs.extracted_paths"],
  allRefs: [
    "inputs.source_dir",
    "artifacts.archive.local_path",
    "steps.extract",
    "steps.extract.outputs.extracted_paths",
  ],
  candidates: [
    {
      ref: "inputs.source_dir",
      label: "Input source_dir",
      valueType: "directory_path",
      sourceKind: "input",
      sourceId: "source_dir",
    },
    {
      ref: "steps.extract.outputs.extracted_paths",
      label: "Extracted paths",
      valueType: "path_list",
      sourceKind: "step_output",
      sourceId: "extract",
    },
  ],
};

test("isAuthoredRefValue only accepts top-level sole-key string refs", () => {
  assert.equal(isAuthoredRefValue({ ref: "steps.extract.outputs.extracted_paths" }), true);
  assert.equal(isAuthoredRefValue({ ref: "steps.extract.outputs.extracted_paths", label: "extra" }), false);
  assert.equal(isAuthoredRefValue({ ref: 1 }), false);
  assert.equal(isAuthoredRefValue([{ ref: "nested.literal" }]), false);
});

test("orderedParamNames renders present spec params first and keeps extra params", () => {
  assert.deepEqual(
    orderedParamNames(
      {
        copy_policy: "sync",
        extra: true,
        source: { ref: "inputs.source_dir" },
      },
      copySpec,
    ),
    ["source", "copy_policy", "extra"],
  );
});

test("buildUpdateStepParamsCommand preserves sibling params and literal null values", () => {
  const command = buildUpdateStepParamsCommand(
    "copy",
    {
      source: { ref: "inputs.source_dir" },
      dest: "/sdcard/Old",
      optional: null,
    },
    "dest",
    "",
  );

  assert.deepEqual(command, {
    type: "UpdateStepParams",
    stepId: "copy",
    params: {
      source: { ref: "inputs.source_dir" },
      dest: "",
      optional: null,
    },
  });
});

test("buildUpdateStepParamsCommand omits unchanged values and clear removes the key explicitly", () => {
  const current = {
    source: { ref: "inputs.source_dir" },
    dest: "/sdcard/Old",
    optional: null,
  };

  assert.equal(buildUpdateStepParamsCommand("copy", current, "dest", "/sdcard/Old"), null);
  assert.deepEqual(buildClearStepParamCommand("copy", current, "optional"), {
    type: "UpdateStepParams",
    stepId: "copy",
    params: {
      source: { ref: "inputs.source_dir" },
      dest: "/sdcard/Old",
    },
  });
});

test("parseNumberParamDraft rejects invalid numbers and preserves integer values", () => {
  assert.deepEqual(parseNumberParamDraft("1500", 1), { ok: true, value: 1500 });
  assert.deepEqual(parseNumberParamDraft("1.5", 1), {
    ok: false,
    error: "Enter a whole number.",
  });
  assert.deepEqual(parseNumberParamDraft("abc", 1), {
    ok: false,
    error: "Enter a valid number.",
  });
});

test("parseJsonParamDraft parses literal null and rejects invalid JSON", () => {
  assert.deepEqual(parseJsonParamDraft("null"), { ok: true, value: null });
  assert.deepEqual(parseJsonParamDraft("{"), {
    ok: false,
    error: "Enter valid JSON.",
  });
});

test("buildRefPickerOptions keeps current incompatible and missing refs visible", () => {
  const incompatibleOptions = buildRefPickerOptions(refIndex, {
    allowedValueTypes: ["path_list"],
    currentRef: "inputs.source_dir",
    showAll: false,
  });

  assert.deepEqual(
    incompatibleOptions.map((option) => ({
      ref: option.ref,
      current: option.current,
      missing: option.missing,
      incompatible: option.incompatible,
    })),
    [
      {
        ref: "inputs.source_dir",
        current: true,
        missing: false,
        incompatible: true,
      },
      {
        ref: "steps.extract.outputs.extracted_paths",
        current: false,
        missing: false,
        incompatible: false,
      },
    ],
  );

  const missingOptions = buildRefPickerOptions(refIndex, {
    allowedValueTypes: ["path_list"],
    currentRef: "steps.missing.outputs.value",
    showAll: false,
  });

  assert.deepEqual(missingOptions[0], {
    ref: "steps.missing.outputs.value",
    label: "steps.missing.outputs.value",
    valueType: null,
    sourceKind: "unknown",
    sourceId: "steps.missing.outputs.value",
    current: true,
    missing: true,
    incompatible: false,
  });
});

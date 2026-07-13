import assert from "node:assert/strict";
import test from "node:test";

import type { RefIndexDto, StepSpecDto } from "../src/api/types.js";
import {
  addObjectListRow,
  addUniqueStringListValue,
  buildObjectListRowFieldUpdate,
  buildClearStepParamCommand,
  buildRefDependencyAction,
  buildStepRefDependencyWarning,
  buildRefPickerOptions,
  buildUpdateStepParamsCommand,
  displayValueForObjectField,
  isAuthoredRefValue,
  literalSeedForParam,
  moveStringListValue,
  orderedParamNames,
  parseJsonParamDraft,
  parseNumberParamDraft,
  removeStringListValue,
  stepProducerRefInfo,
  structuredParamEditorKind,
  valueForObjectListRowFieldDraft,
  updateObjectField,
  updateObjectListRowField,
} from "../src/components/stepParams.logic.js";

const copySpec: StepSpecDto = {
  type: "copy_files",
  label: "Copy Files",
  supported: true,
  primaryOutputName: "copied_paths",
  outputs: [],
  paramOrder: ["source", "dest", "copy_policy"],
  params: {
    source: {
      acceptedSources: ["input_ref", "artifact_ref", "step_output_ref"],
      acceptedValueTypes: ["path_list"],
      required: true,
      enumValues: [],
    },
    dest: {
      acceptedSources: ["literal", "input_ref"],
      acceptedValueTypes: ["device_path"],
      required: true,
      enumValues: [],
    },
    copy_policy: {
      acceptedSources: ["literal", "input_ref"],
      acceptedValueTypes: ["string"],
      required: false,
      enumValues: ["merge", "sync"],
    },
  },
  defaults: {},
};

const resolveSpec: StepSpecDto = {
  type: "resolve_artifacts",
  label: "Resolve Artifacts",
  supported: true,
  primaryOutputName: null,
  outputs: [],
  paramOrder: ["artifacts", "artifact_groups"],
  params: {
    artifacts: {
      acceptedSources: ["literal"],
      acceptedValueTypes: ["string_list"],
      required: false,
      enumValues: [],
      shape: {
        kind: "list",
        itemKind: "string",
        target: "artifact",
        ordered: true,
        unique: true,
        fields: {},
      },
    },
    artifact_groups: {
      acceptedSources: ["literal"],
      acceptedValueTypes: ["string_list"],
      required: false,
      enumValues: [],
      shape: {
        kind: "list",
        itemKind: "string",
        target: "artifact_group",
        ordered: true,
        unique: true,
        fields: {},
      },
    },
  },
  defaults: {},
};

const grantSpec: StepSpecDto = {
  type: "grant_permissions",
  label: "Grant Permissions",
  supported: true,
  primaryOutputName: null,
  outputs: [],
  paramOrder: ["runtime", "appops", "policy"],
  params: {
    runtime: {
      acceptedSources: ["literal"],
      acceptedValueTypes: ["object_list"],
      required: false,
      enumValues: [],
      shape: {
        kind: "list",
        itemKind: "object",
        ordered: true,
        unique: false,
        fields: {
          package_name: { kind: "string", required: true, enumValues: [] },
          name: { kind: "string", required: true, enumValues: [] },
        },
      },
    },
    appops: {
      acceptedSources: ["literal"],
      acceptedValueTypes: ["object_list"],
      required: false,
      enumValues: [],
      shape: {
        kind: "list",
        itemKind: "object",
        ordered: true,
        unique: false,
        fields: {
          package_name: { kind: "string", required: true, enumValues: [] },
          op: { kind: "string", required: true, enumValues: [] },
          mode: { kind: "string", required: true, enumValues: [] },
        },
      },
    },
    policy: {
      acceptedSources: ["literal"],
      acceptedValueTypes: ["object"],
      required: false,
      enumValues: [],
      shape: {
        kind: "object",
        ordered: false,
        unique: false,
        fields: {
          on_failure: { kind: "string", required: false, enumValues: ["warn", "fail"], default: "warn" },
          require_all: { kind: "boolean", required: false, enumValues: [], default: false },
        },
      },
    },
  },
  defaults: {},
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

test("orderedParamNames includes absent structured params without including absent primitive params", () => {
  assert.deepEqual(orderedParamNames({}, grantSpec), ["runtime", "appops", "policy"]);
  assert.deepEqual(orderedParamNames({}, copySpec), []);
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

test("structured param classification requires schema metadata and does not infer from value shape alone", () => {
  assert.equal(structuredParamEditorKind(resolveSpec, "artifacts", ["app_apk"]), "artifact-id-list");
  assert.equal(structuredParamEditorKind(resolveSpec, "artifact_groups", ["core_bundle"]), "artifact-group-id-list");
  assert.equal(structuredParamEditorKind(grantSpec, "runtime", [{ package_name: "com.example", name: "POST_NOTIFICATIONS" }]), "object-list");
  assert.equal(structuredParamEditorKind(grantSpec, "policy", { on_failure: "warn" }), "object");
  assert.equal(structuredParamEditorKind(copySpec, "metadata", ["app_apk", "core_bundle"]), null);
});

test("schema-less string arrays do not become artifact or artifact-group editors from matching ids", () => {
  const schemaLessSpec: StepSpecDto = {
    ...copySpec,
    type: "custom_step",
    paramOrder: ["ids"],
    params: {
      ids: { acceptedSources: ["literal"], acceptedValueTypes: ["object"], required: false, enumValues: [] },
    },
  };

  assert.equal(structuredParamEditorKind(schemaLessSpec, "ids", ["app_apk", "core_bundle"]), null);
});

test("artifact and artifact group list helpers add remove and reorder while preserving missing ids", () => {
  assert.deepEqual(addUniqueStringListValue(["missing_apk"], "app_apk"), {
    ok: true,
    value: ["missing_apk", "app_apk"],
  });
  assert.deepEqual(addUniqueStringListValue(["app_apk"], "app_apk"), {
    ok: false,
    error: "This id is already selected.",
  });
  assert.deepEqual(moveStringListValue(["missing_apk", "app_apk", "core_zip"], 0, 2), [
    "app_apk",
    "core_zip",
    "missing_apk",
  ]);
  assert.deepEqual(removeStringListValue(["missing_group", "core_group"], 0), ["core_group"]);
});

test("runtime and app-op row edits preserve extra keys", () => {
  const runtimeRows = [
    {
      package_name: "com.example",
      name: "READ_MEDIA_VIDEO",
      when: { rooted: false },
      custom: "preserved",
    },
  ];
  assert.deepEqual(updateObjectListRowField(runtimeRows, 0, "name", "POST_NOTIFICATIONS"), {
    ok: true,
    value: [
      {
        package_name: "com.example",
        name: "POST_NOTIFICATIONS",
        when: { rooted: false },
        custom: "preserved",
      },
    ],
  });

  const appopRows = [{ package_name: "com.example", op: "RUN_IN_BACKGROUND", mode: "allow", required: false }];
  assert.deepEqual(updateObjectListRowField(appopRows, 0, "mode", "deny"), {
    ok: true,
    value: [{ package_name: "com.example", op: "RUN_IN_BACKGROUND", mode: "deny", required: false }],
  });
});

test("object list row field update returns null for no-op blur or Enter", () => {
  const runtimeRows = [
    {
      package_name: "com.example",
      name: "READ_MEDIA_VIDEO",
      when: { rooted: false },
    },
  ];

  assert.equal(buildObjectListRowFieldUpdate(runtimeRows, 0, "name", "READ_MEDIA_VIDEO"), null);
});

test("object list row field update changes only edited field and preserves extra keys", () => {
  const runtimeRows = [
    {
      package_name: "com.example",
      name: "READ_MEDIA_VIDEO",
      when: { rooted: false },
      custom: "preserved",
    },
  ];

  assert.deepEqual(buildObjectListRowFieldUpdate(runtimeRows, 0, "name", "POST_NOTIFICATIONS"), {
    ok: true,
    value: [
      {
        package_name: "com.example",
        name: "POST_NOTIFICATIONS",
        when: { rooted: false },
        custom: "preserved",
      },
    ],
  });
});

test("object list row field draft value resets from current row for Escape without a command", () => {
  const row = {
    package_name: "com.example",
    name: "READ_MEDIA_VIDEO",
  };
  const rows = [row];
  const escapedDraft = valueForObjectListRowFieldDraft(row, "name");

  assert.equal(escapedDraft, "READ_MEDIA_VIDEO");
  assert.equal(buildObjectListRowFieldUpdate(rows, 0, "name", escapedDraft), null);
  assert.equal(valueForObjectListRowFieldDraft(row, "missing"), "");
});

test("object list helpers reject incompatible row shapes instead of dropping data", () => {
  assert.equal(structuredParamEditorKind(grantSpec, "runtime", ["READ_MEDIA_VIDEO"]), null);
  assert.deepEqual(addObjectListRow(["READ_MEDIA_VIDEO"], { package_name: "com.example", name: "POST_NOTIFICATIONS" }), {
    ok: false,
    error: "Existing value is not a list of objects.",
  });
});

test("policy updates preserve extra keys and defaults remain display-only until changed", () => {
  assert.deepEqual(displayValueForObjectField({}, "require_all", grantSpec.params.policy.shape?.fields.require_all), {
    value: false,
    defaulted: true,
  });
  assert.deepEqual(updateObjectField({ custom: "preserved" }, "on_failure", "fail"), {
    ok: true,
    value: { custom: "preserved", on_failure: "fail" },
  });
  assert.deepEqual(updateObjectField({ on_failure: "fail" }, "require_all", false), {
    ok: true,
    value: { on_failure: "fail", require_all: false },
  });
});

test("policy-like params remain raw JSON unless schema-backed", () => {
  const schemaLessSpec: StepSpecDto = {
    ...copySpec,
    type: "custom_step",
    paramOrder: ["policy"],
    params: {
      policy: { acceptedSources: ["literal"], acceptedValueTypes: ["object"], required: false, enumValues: [] },
    },
  };

  assert.equal(structuredParamEditorKind(schemaLessSpec, "policy", { on_failure: "warn", require_all: false }), null);
});

test("incompatible or free-form values fall back to raw JSON", () => {
  assert.equal(structuredParamEditorKind(resolveSpec, "artifacts", ["app_apk", 1]), null);
  assert.equal(structuredParamEditorKind(grantSpec, "policy", ["warn"]), null);
  assert.equal(structuredParamEditorKind(copySpec, "metadata", { tags: ["free-form"] }), null);
});

test("structured param updates produce full UpdateStepParams commands and preserve unrelated params", () => {
  const current = {
    artifacts: ["old_apk"],
    metadata: { tags: ["free-form"] },
    policy: { on_failure: "warn", extra: true },
  };
  const nextArtifacts = addUniqueStringListValue(current.artifacts, "new_apk");
  assert.equal(nextArtifacts.ok, true);
  if (!nextArtifacts.ok) {
    return;
  }

  assert.deepEqual(buildUpdateStepParamsCommand("resolve", current, "artifacts", nextArtifacts.value), {
    type: "UpdateStepParams",
    stepId: "resolve",
    params: {
      artifacts: ["old_apk", "new_apk"],
      metadata: { tags: ["free-form"] },
      policy: { on_failure: "warn", extra: true },
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
    allowedSources: ["input_ref", "step_output_ref"],
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
      {
        ref: "steps.extract",
        current: false,
        missing: false,
        incompatible: false,
      },
    ],
  );

  const missingOptions = buildRefPickerOptions(refIndex, {
    allowedSources: ["input_ref", "step_output_ref"],
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

test("buildRefPickerOptions filters ref namespaces as well as value types", () => {
  const options = buildRefPickerOptions(refIndex, {
    allowedSources: ["step_output_ref"],
    allowedValueTypes: ["path_list"],
    currentRef: null,
    showAll: false,
  });

  assert.deepEqual(options.map((option) => option.ref), ["steps.extract.outputs.extracted_paths", "steps.extract"]);
});

test("literalSeedForParam uses defaults, enum values, and stable type seeds", () => {
  assert.equal(literalSeedForParam(["device_path"], [], "/sdcard/ROMs"), "/sdcard/ROMs");
  assert.equal(literalSeedForParam(["string"], ["merge", "sync"], undefined), "merge");
  assert.equal(literalSeedForParam(["boolean"], [], undefined), false);
  assert.deepEqual(literalSeedForParam(["path_list"], [], undefined), []);
});

test("stepProducerRefInfo detects bare step refs and step output refs", () => {
  assert.deepEqual(stepProducerRefInfo(refIndex, "steps.extract_assets"), {
    kind: "step",
    producerStepId: "extract_assets",
    outputName: null,
    shorthandStepRef: true,
  });
  assert.deepEqual(stepProducerRefInfo(refIndex, "steps.extract_assets.outputs.extracted_path"), {
    kind: "step",
    producerStepId: "extract_assets",
    outputName: "extracted_path",
    shorthandStepRef: false,
  });
});

test("stepProducerRefInfo prefers structured step-output candidate metadata", () => {
  assert.deepEqual(stepProducerRefInfo(refIndex, "steps.extract.outputs.extracted_paths"), {
    kind: "step",
    producerStepId: "extract",
    outputName: "extracted_paths",
    shorthandStepRef: false,
  });
});

test("stepProducerRefInfo ignores input refs artifact refs literals and malformed refs", () => {
  assert.deepEqual(stepProducerRefInfo(refIndex, { ref: "steps.extract.outputs.extracted_paths" }), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "inputs.source_dir"), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "artifacts.archive.local_path"), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "/literal/path"), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "steps."), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "steps.extract.outputs"), { kind: "non-step" });
  assert.deepEqual(stepProducerRefInfo(refIndex, "unknown.extract.outputs.path"), { kind: "non-step" });
});

test("buildStepRefDependencyWarning reports missing producer dependencies for current values", () => {
  assert.deepEqual(
    buildStepRefDependencyWarning({
      refIndex,
      currentStepId: "copy",
      dependencyIds: ["prepare"],
      value: { ref: "steps.extract_assets.outputs.extracted_path" },
    }),
    {
      producerStepId: "extract_assets",
      outputName: "extracted_path",
      shorthandStepRef: false,
      message: 'This ref is produced by step "extract_assets", but the current step does not depend on it.',
    },
  );
});

test("buildStepRefDependencyWarning is absent for existing dependencies self refs and ignored refs", () => {
  assert.equal(
    buildStepRefDependencyWarning({
      refIndex,
      currentStepId: "copy",
      dependencyIds: ["extract_assets"],
      value: { ref: "steps.extract_assets.outputs.extracted_path" },
    }),
    null,
  );
  assert.equal(
    buildStepRefDependencyWarning({
      refIndex,
      currentStepId: "extract_assets",
      dependencyIds: [],
      value: { ref: "steps.extract_assets.outputs.extracted_path" },
    }),
    null,
  );
  assert.equal(
    buildStepRefDependencyWarning({
      refIndex,
      currentStepId: "copy",
      dependencyIds: [],
      value: { ref: "inputs.source_dir" },
    }),
    null,
  );
});

test("buildRefDependencyAction appends producer ids and avoids duplicates or self dependencies", () => {
  assert.deepEqual(buildRefDependencyAction(["prepare"], "copy", "extract_assets"), {
    ok: true,
    dependencies: ["prepare", "extract_assets"],
  });
  assert.deepEqual(buildRefDependencyAction(["prepare", "extract_assets"], "copy", "extract_assets"), {
    ok: false,
    reason: "duplicate-dependency",
  });
  assert.deepEqual(buildRefDependencyAction(["prepare"], "copy", "copy"), {
    ok: false,
    reason: "self-dependency",
  });
});

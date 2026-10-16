import assert from "node:assert/strict";
import test from "node:test";

import type { AppCatalogAppDto, AppCatalogDto, StepSpecDto } from "../src/api/types.js";
import type { ArtifactDto, InputDto, RefIndexDto } from "../src/api/types.js";
import {
  apkArtifactRefs,
  appContextControl,
  appRefOptions,
  appArtifactOptions,
  appTargetChoices,
  appTargetOptions,
  catalogApp,
  catalogApps,
  conditionTargetOptions,
  conditionTargetSeed,
  copyPolicyValue,
  inputBindingOptions,
  installApkArtifactRefs,
  launcherUnavailable,
  permissionSetChoices,
  refSourceValueType,
  uniqueArtifactId,
} from "../src/components/appCatalog.logic.js";

const demoApp: AppCatalogAppDto = {
  id: "demo_app",
  name: "Demo App",
  artifacts: [
    { id: "apk", kind: "apk", requiresInput: false },
    { id: "config", kind: "file", requiresInput: true },
  ],
  permissionSets: ["baseline"],
  targets: [
    { id: "system", kind: "directory", location: "shared_storage", path: "Demo/system" },
    { id: "config_file", kind: "file", location: "external_app_data", path: "files/demo.cfg" },
  ],
  launcherActivity: "com.example.demo.MainActivity",
};

const secondApp: AppCatalogAppDto = {
  id: "second_app",
  name: "Second App",
  artifacts: [{ id: "apk", kind: "apk", requiresInput: false }],
  permissionSets: [],
  targets: [],
  launcherActivity: null,
};

const readyCatalog: AppCatalogDto = {
  status: "ready",
  apps: [secondApp, demoApp],
  diagnostics: [],
};

const unboundCatalog: AppCatalogDto = { status: "unbound", apps: [], diagnostics: [] };

const invalidCatalog: AppCatalogDto = { status: "invalid", apps: [], diagnostics: [] };

function stepSpec(appContext: StepSpecDto["appContext"]): StepSpecDto {
  return {
    type: "copy_files",
    label: "Copy Files",
    supported: true,
    appContext,
    primaryOutputName: null,
    outputs: [],
    paramOrder: [],
    params: {},
    defaults: {},
  };
}

test("app context control follows the step spec app context requirement", () => {
  assert.equal(appContextControl(stepSpec("required")), "required");
  assert.equal(appContextControl(stepSpec("optional")), "optional");
  assert.equal(appContextControl(stepSpec("none")), "hidden");
  assert.equal(appContextControl(null), "hidden");
});

test("catalog apps are only offered from a ready projection", () => {
  assert.deepEqual(
    catalogApps(readyCatalog).map((app) => app.id),
    ["demo_app", "second_app"],
  );
  assert.deepEqual(catalogApps(unboundCatalog), []);
  assert.deepEqual(catalogApps(invalidCatalog), []);
});

test("catalog app lookup resolves by identifier only when ready", () => {
  assert.equal(catalogApp(readyCatalog, "demo_app")?.name, "Demo App");
  assert.equal(catalogApp(readyCatalog, "ghost_app"), null);
  assert.equal(catalogApp(invalidCatalog, "demo_app"), null);
  assert.equal(catalogApp(readyCatalog, null), null);
});

test("app ref options keep a retained value that is missing from the catalog", () => {
  assert.deepEqual(appRefOptions(readyCatalog, null), [
    { value: "demo_app", label: "Demo App (demo_app)", retained: false },
    { value: "second_app", label: "Second App (second_app)", retained: false },
  ]);
  assert.deepEqual(appRefOptions(readyCatalog, "ghost_app"), [
    { value: "ghost_app", label: "ghost_app (not in this catalog)", retained: true },
    { value: "demo_app", label: "Demo App (demo_app)", retained: false },
    { value: "second_app", label: "Second App (second_app)", retained: false },
  ]);
  assert.equal(appRefOptions(readyCatalog, "demo_app")[0].retained, false);
  assert.deepEqual(appRefOptions(unboundCatalog, "ghost_app"), [
    { value: "ghost_app", label: "ghost_app (not in this catalog)", retained: true },
  ]);
  assert.deepEqual(appRefOptions(unboundCatalog, null), []);
});

test("apk artifact refs list only the selected app's apk artifacts", () => {
  assert.deepEqual(apkArtifactRefs(demoApp), ["artifacts.apk.local_path"]);
  assert.deepEqual(apkArtifactRefs(secondApp), ["artifacts.apk.local_path"]);
  assert.deepEqual(apkArtifactRefs(null), []);
});

test("permission set choices come from the app and retain unknown values", () => {
  assert.deepEqual(permissionSetChoices(demoApp, null), [
    { value: "baseline", label: "baseline", retained: false },
  ]);
  assert.deepEqual(permissionSetChoices(demoApp, "elevated"), [
    { value: "elevated", label: "elevated (not defined by this App Definition)", retained: true },
    { value: "baseline", label: "baseline", retained: false },
  ]);
  assert.deepEqual(permissionSetChoices(secondApp, "baseline"), [
    { value: "baseline", label: "baseline (not defined by this App Definition)", retained: true },
  ]);
  assert.deepEqual(permissionSetChoices(null, "baseline"), [
    { value: "baseline", label: "baseline (not defined by this App Definition)", retained: true },
  ]);
});

test("app target choices follow the planner copy compatibility rules", () => {
  assert.deepEqual(appTargetChoices(demoApp, { sourceValueType: "file_path", copyPolicy: "merge" }), [
    { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
  ]);
  assert.deepEqual(
    appTargetChoices(demoApp, { sourceValueType: "directory_path", copyPolicy: "merge" }),
    [
      {
        id: "config_file",
        label: "config_file (file)",
        compatibility: "incompatible",
        reason: "A file target needs a single file source.",
      },
      { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
    ],
  );
  assert.deepEqual(appTargetChoices(demoApp, { sourceValueType: "file_path", copyPolicy: "sync" }), [
    {
      id: "config_file",
      label: "config_file (file)",
      compatibility: "incompatible",
      reason: "A sync copy needs a directory target.",
    },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
  ]);
  assert.deepEqual(appTargetChoices(secondApp, { sourceValueType: "file_path", copyPolicy: "merge" }), []);
  assert.deepEqual(appTargetChoices(null, { sourceValueType: "file_path", copyPolicy: "merge" }), []);
});

test("app target compatibility is unknown while the copy source or policy is unresolved", () => {
  assert.deepEqual(appTargetChoices(demoApp, { sourceValueType: null, copyPolicy: "merge" }), [
    {
      id: "config_file",
      label: "config_file (file)",
      compatibility: "unknown",
      reason: "The copy source is not resolved yet, so compatibility is unknown until validation.",
    },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
  ]);
  assert.deepEqual(appTargetChoices(demoApp, { sourceValueType: "file_path", copyPolicy: null }), [
    {
      id: "config_file",
      label: "config_file (file)",
      compatibility: "unknown",
      reason: "The copy policy is not resolved yet, so compatibility is unknown until validation.",
    },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
  ]);
  assert.deepEqual(appTargetChoices(demoApp, { sourceValueType: null, copyPolicy: "sync" }), [
    {
      id: "config_file",
      label: "config_file (file)",
      compatibility: "incompatible",
      reason: "A sync copy needs a directory target.",
    },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null },
  ]);
});

test("launcher availability reflects the selected app definition", () => {
  assert.equal(launcherUnavailable(demoApp), false);
  assert.equal(launcherUnavailable(secondApp), true);
  assert.equal(launcherUnavailable(null), false);
});

test("app target options keep a current target missing from the app definition", () => {
  assert.deepEqual(
    appTargetOptions(demoApp, { sourceValueType: "file_path", copyPolicy: "merge", currentTargetId: null }),
    [
      { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
      { id: "system", label: "system (directory)", compatibility: "compatible", reason: null, retained: false },
    ],
  );
  assert.deepEqual(
    appTargetOptions(demoApp, { sourceValueType: "file_path", copyPolicy: "merge", currentTargetId: "system" }),
    [
      { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
      { id: "system", label: "system (directory)", compatibility: "compatible", reason: null, retained: false },
    ],
  );
  assert.deepEqual(
    appTargetOptions(demoApp, { sourceValueType: "file_path", copyPolicy: "merge", currentTargetId: "ghost_target" }),
    [
      {
        id: "ghost_target",
        label: "ghost_target (not defined by this App Definition)",
        compatibility: "unknown",
        reason: null,
        retained: true,
      },
      { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
      { id: "system", label: "system (directory)", compatibility: "compatible", reason: null, retained: false },
    ],
  );
  assert.deepEqual(
    appTargetOptions(null, { sourceValueType: null, copyPolicy: null, currentTargetId: "ghost_target" }),
    [
      {
        id: "ghost_target",
        label: "ghost_target (not defined by this App Definition)",
        compatibility: "unknown",
        reason: null,
        retained: true,
      },
    ],
  );
  assert.deepEqual(appTargetOptions(null, { sourceValueType: null, copyPolicy: null, currentTargetId: null }), []);
});

const catalogArtifacts: Record<string, ArtifactDto> = {
  demo_apk: { id: "demo_apk", type: "app_artifact", appRef: "demo_app", artifact: "apk", cache: "default" },
  demo_config: { id: "demo_config", type: "app_artifact", appRef: "demo_app", artifact: "config", cache: "default" },
  second_apk: { id: "second_apk", type: "app_artifact", appRef: "second_app", artifact: "apk", cache: "default" },
  remote_apk: { id: "remote_apk", type: "remote_file", url: "https://example.com/app.apk", cache: "default" },
  ghost_app_apk: { id: "ghost_app_apk", type: "app_artifact", appRef: "ghost_app", artifact: "apk", cache: "default" },
  ghost_artifact: { id: "ghost_artifact", type: "app_artifact", appRef: "demo_app", artifact: "missing", cache: "default" },
};

test("install apk refs join recipe app artifacts with the selected app's apk artifacts", () => {
  assert.deepEqual(installApkArtifactRefs(readyCatalog, catalogArtifacts, "demo_app"), [
    "artifacts.demo_apk.local_path",
  ]);
  assert.deepEqual(installApkArtifactRefs(readyCatalog, catalogArtifacts, "second_app"), [
    "artifacts.second_apk.local_path",
  ]);
  assert.deepEqual(installApkArtifactRefs(readyCatalog, catalogArtifacts, "ghost_app"), []);
  assert.deepEqual(installApkArtifactRefs(readyCatalog, catalogArtifacts, null), []);
  assert.deepEqual(installApkArtifactRefs(unboundCatalog, catalogArtifacts, "demo_app"), []);
  assert.deepEqual(installApkArtifactRefs(invalidCatalog, catalogArtifacts, "demo_app"), []);
  assert.deepEqual(
    installApkArtifactRefs(
      readyCatalog,
      {
        zzz: { id: "zzz", type: "app_artifact", appRef: "demo_app", artifact: "apk", cache: "default" },
        aaa: { id: "aaa", type: "app_artifact", appRef: "demo_app", artifact: "apk", cache: "default" },
      },
      "demo_app",
    ),
    ["artifacts.aaa.local_path", "artifacts.zzz.local_path"],
  );
});

const refIndex: RefIndexDto = {
  inputRefs: [],
  artifactRefs: [],
  stepRefs: [],
  stepOutputRefs: [],
  allRefs: [],
  candidates: [
    {
      ref: "artifacts.demo_apk.local_path",
      label: "Artifact · demo_apk.local_path",
      valueType: "file_path",
      sourceKind: "artifact",
      sourceId: "demo_apk",
    },
    {
      ref: "steps.unzip.outputs.extracted_path",
      label: "Step · unzip",
      valueType: "directory_path",
      sourceKind: "step",
      sourceId: "unzip",
    },
  ],
};

test("ref source value type resolves authored refs from the ref index", () => {
  assert.equal(refSourceValueType(refIndex, { ref: "artifacts.demo_apk.local_path" }), "file_path");
  assert.equal(refSourceValueType(refIndex, { ref: "steps.unzip.outputs.extracted_path" }), "directory_path");
  assert.equal(refSourceValueType(refIndex, { ref: "artifacts.unknown.local_path" }), null);
  assert.equal(refSourceValueType(refIndex, "inputs.source"), null);
  assert.equal(refSourceValueType(refIndex, null), null);
});

test("copy policy value reads only literal string policies", () => {
  assert.equal(copyPolicyValue("sync"), "sync");
  assert.equal(copyPolicyValue("merge"), "merge");
  assert.equal(copyPolicyValue({ ref: "inputs.policy" }), null);
  assert.equal(copyPolicyValue(null), null);
  assert.equal(copyPolicyValue(3), null);
});

test("app artifact options retain a current artifact missing from the app definition", () => {
  assert.deepEqual(appArtifactOptions(demoApp, null), [
    { value: "apk", label: "apk (apk)", retained: false },
    { value: "config", label: "config (file)", retained: false },
  ]);
  assert.deepEqual(appArtifactOptions(demoApp, "missing"), [
    { value: "missing", label: "missing (not defined by this App Definition)", retained: true },
    { value: "apk", label: "apk (apk)", retained: false },
    { value: "config", label: "config (file)", retained: false },
  ]);
  assert.deepEqual(appArtifactOptions(demoApp, "apk"), [
    { value: "apk", label: "apk (apk)", retained: false },
    { value: "config", label: "config (file)", retained: false },
  ]);
  assert.deepEqual(appArtifactOptions(null, "apk"), [
    { value: "apk", label: "apk (not defined by this App Definition)", retained: true },
  ]);
  assert.deepEqual(appArtifactOptions(null, null), []);
});

function inputDto(id: string, label: string): InputDto {
  return {
    id,
    recipeId: "demo-recipe",
    inputId: id,
    key: id,
    type: "file",
    role: "source",
    label,
    description: "",
    required: true,
    multiple: false,
    validation: {
      mustExist: true,
      allowedExtensions: [],
      pathKind: "file",
      allowedPrefixes: [],
    },
    default: null,
    options: [],
    sensitive: false,
    advanced: false,
    metadata: {},
  };
}

test("input binding options retain a current input missing from the recipe", () => {
  const inputs = {
    apk_file: inputDto("apk_file", "APK file"),
    extra: inputDto("extra", "Extra file"),
  };
  assert.deepEqual(inputBindingOptions(inputs, null), [
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
    { value: "extra", label: "Extra file (extra)", retained: false },
  ]);
  assert.deepEqual(inputBindingOptions(inputs, "ghost_input"), [
    { value: "ghost_input", label: "ghost_input (not defined in this Recipe)", retained: true },
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
    { value: "extra", label: "Extra file (extra)", retained: false },
  ]);
  assert.deepEqual(inputBindingOptions({}, "ghost_input"), [
    { value: "ghost_input", label: "ghost_input (not defined in this Recipe)", retained: true },
  ]);
});

test("input binding options offer only single-file inputs and retain an incompatible binding", () => {
  const inputs = {
    apk_file: inputDto("apk_file", "APK file"),
    config_folder: { ...inputDto("config_folder", "Config folder"), type: "directory" },
    extra: { ...inputDto("extra", "Extra file"), multiple: true },
  };
  assert.deepEqual(inputBindingOptions(inputs, null), [
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
  ]);
  assert.deepEqual(inputBindingOptions(inputs, "config_folder"), [
    {
      value: "config_folder",
      label: "Config folder (config_folder) (not a single-file input)",
      retained: true,
    },
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
  ]);
  assert.deepEqual(inputBindingOptions(inputs, "extra"), [
    { value: "extra", label: "Extra file (extra) (not a single-file input)", retained: true },
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
  ]);
  assert.deepEqual(inputBindingOptions(inputs, "ghost_input"), [
    { value: "ghost_input", label: "ghost_input (not defined in this Recipe)", retained: true },
    { value: "apk_file", label: "APK file (apk_file)", retained: false },
  ]);
});

test("unique artifact id seeds from the app artifact id without colliding", () => {
  assert.equal(uniqueArtifactId("apk", []), "apk");
  assert.equal(uniqueArtifactId("apk", ["apk"]), "apk_2");
  assert.equal(uniqueArtifactId("apk", ["apk", "apk_2"]), "apk_3");
  assert.equal(uniqueArtifactId("apk", ["apk_2"]), "apk");
  assert.equal(uniqueArtifactId("  apk  ", []), "apk");
});

test("condition App Target options follow planner file_exists restrictions", () => {
  assert.deepEqual(conditionTargetOptions(demoApp, "path_exists"), [
    { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
    { id: "system", label: "system (directory)", compatibility: "compatible", reason: null, retained: false },
  ]);
  assert.deepEqual(conditionTargetOptions(demoApp, "file_exists"), [
    { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
    {
      id: "system",
      label: "system (directory)",
      compatibility: "incompatible",
      reason: "A file_exists check needs a file target.",
      retained: false,
    },
  ]);
  assert.deepEqual(conditionTargetOptions(demoApp, "file_exists", "ghost_target"), [
    {
      id: "ghost_target",
      label: "ghost_target (not defined by this App Definition)",
      compatibility: "unknown",
      reason: null,
      retained: true,
    },
    { id: "config_file", label: "config_file (file)", compatibility: "compatible", reason: null, retained: false },
    {
      id: "system",
      label: "system (directory)",
      compatibility: "incompatible",
      reason: "A file_exists check needs a file target.",
      retained: false,
    },
  ]);
  assert.deepEqual(conditionTargetOptions(secondApp, "path_exists"), []);
  assert.deepEqual(conditionTargetOptions(null, "path_exists"), []);
});

test("condition App Target seeds pick the first planner-compatible target", () => {
  assert.equal(conditionTargetSeed(demoApp, "path_exists"), "config_file");
  assert.equal(conditionTargetSeed(demoApp, "file_exists"), "config_file");
  const directoryOnlyApp: AppCatalogAppDto = { ...demoApp, id: "directory_app", targets: [demoApp.targets[0]] };
  assert.equal(conditionTargetSeed(directoryOnlyApp, "path_exists"), "system");
  assert.equal(conditionTargetSeed(directoryOnlyApp, "file_exists"), null);
  assert.equal(conditionTargetSeed(null, "path_exists"), null);
});


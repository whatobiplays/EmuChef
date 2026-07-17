import assert from "node:assert/strict";
import test from "node:test";

import type {
  AppRecipeDraftResult,
  AppRecipeSaveResult,
} from "../src/api/types.js";
import {
  diagnosticDisplayTitle,
  draftToForm,
  formToRequest,
  initialAppGeneratorState,
  readableNameFromPackage,
  reduceAppGenerator,
  visibleDraftDiagnostics,
} from "../src/components/appGenerator.logic.js";

function draft(): AppRecipeDraftResult {
  return {
    app: {
      schema_version: 1,
      kind: "app_definition",
      id: "example",
      name: "Example",
      category: "utility",
      package: { primary: "com.example.app", aliases: [] },
      install_source: {
        type: "user_provided_apk",
        resolver: "none",
        options: {},
      },
      tracking_source: { type: "local_apk" },
      artifacts: {
        apk: { required: false },
        shared_storage_config: { supported: false },
        app_data_config: { supported: false },
        byo_apk: { required: true },
      },
      provisioning: {
        launch_once_recommended: false,
        shared_storage_paths: [],
        app_data_paths: [],
        config_targets: [],
      },
      inputs: [],
      metadata: {},
    },
    recipe: {
      schemaVersion: 1,
      kind: "recipe",
      id: "app.example.install",
      name: "Install Example",
      description: "",
      recipeDependencies: [],
      provides: { features: ["example_install"] },
      inputs: {},
      artifacts: {},
      artifactGroups: {},
      steps: [],
    },
    recipeEdits: {
      ids: {
        recipeId: "app.example.install",
        inputId: "example_apk",
        featureId: "example_install",
        installStepId: "install_example",
        launchStepId: "launch_example",
      },
      name: "Install Example",
      description: "",
      inputLabel: "Example APK",
      inputDescription: "",
      replaceExisting: false,
      launchEnabled: false,
      launcherActivity: null,
    },
    appCanonicalYaml: "app",
    recipeCanonicalYaml: "recipe",
    appDestination: {
      fileName: "example.yaml",
      relativePath: "apps/example.yaml",
    },
    recipeDestination: {
      fileName: "app.example.install.yaml",
      relativePath: "recipes/app.example.install.yaml",
    },
    evidence: [],
    diagnostics: [],
    blocking: false,
  };
}

test("package fallback creates a readable app name and matching recipe text", () => {
  assert.equal(readableNameFromPackage("dev.eden.eden_emulator"), "Eden Emulator");
  const packageDraft = draft();
  packageDraft.app.name = "dev.eden.eden_emulator";
  packageDraft.app.package.primary = "dev.eden.eden_emulator";
  packageDraft.recipeEdits.name = "Install dev.eden.eden_emulator";
  packageDraft.recipeEdits.description = "Install a user-provided dev.eden.eden_emulator APK.";
  packageDraft.recipeEdits.inputLabel = "dev.eden.eden_emulator APK";
  packageDraft.recipeEdits.inputDescription = "Local dev.eden.eden_emulator APK to install.";
  const form = draftToForm(packageDraft);
  assert.equal(form.app.name, "Eden Emulator");
  assert.equal(form.recipe.name, "Install Eden Emulator");
  assert.equal(form.recipe.inputLabel, "Eden Emulator APK");
});

test("started session restores trusted analyzer and root handles", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
    analyzerHandle: "analyzer",
    analyzerKind: "apkanalyzer",
    analyzerLabel: "Configured apkanalyzer",
    rootHandle: "root",
    rootLabel: "Selected authored root",
  });
  assert.equal(state.analyzerHandle, "analyzer");
  assert.equal(state.analyzerKind, "apkanalyzer");
  assert.equal(state.rootHandle, "root");
});

test("reducer clears stale review when the form changes", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
  });
  state = reduceAppGenerator(state, { type: "drafted", draft: draft() });
  assert.equal(state.phase, "editing");
  assert.ok(state.form);
  state = reduceAppGenerator(state, { type: "form", form: state.form! });
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);
});

test("form conversion rejects duplicate keys at any nesting depth", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.metadata = '{"outer":{"same":1,"same":2}}';
  const result = formToRequest(form);
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.message, /duplicate key/u);
});

test("form conversion retains ordered mapping source text for backend parsing", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.metadata = '{"z":1,"a":2}';
  form.aliases = ["com.example.old", "com.example.legacy"];
  const result = formToRequest(form);
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.mappings.metadata, '{"z":1,"a":2}');
    assert.deepEqual(result.app.package.aliases, [
      "com.example.old",
      "com.example.legacy",
    ]);
  }
});

test("form state preserves an empty package alias row", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.aliases = [""];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.aliases, [""]);
});

test("form state preserves empty shared-storage and app-data path rows", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.sharedStoragePaths = [""];
  form.appDataPaths = [""];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.sharedStoragePaths, [""]);
  assert.deepEqual(next.form?.appDataPaths, [""]);
});

test("root-backed review hides the pre-root validation warning", () => {
  const diagnostics = [
    { severity: "warning" as const, code: "validation_context_limited", message: "limited", field: "authored_root" },
    { severity: "warning" as const, code: "other_warning", message: "keep", field: "app" },
  ];
  assert.deepEqual(visibleDraftDiagnostics(diagnostics, false), diagnostics);
  assert.deepEqual(visibleDraftDiagnostics(diagnostics, true), [diagnostics[1]]);
});

test("diagnostic titles do not expose internal codes", () => {
  assert.equal(diagnosticDisplayTitle("validation_context_limited", "warning"), "Catalog validation not yet available");
  assert.equal(diagnosticDisplayTitle("unknown_internal_code", "error"), "Action required");
});

test("reducer records the opened recipe result after an explicit save", () => {
  const result = {
    appFileName: "example.yaml",
    appRelativePath: "apps/example.yaml",
    recipeFileName: "app.example.install.yaml",
    recipeRelativePath: "recipes/app.example.install.yaml",
    openedRecipe: {
      document: {
        documentId: "document",
        path: "recipes/app.example.install.yaml",
        authoredRoot: null,
        recipe: draft().recipe,
        diagnostics: [],
        yaml: "recipe",
        dirty: false,
        canUndo: false,
        canRedo: false,
        refIndex: {
          inputRefs: [],
          artifactRefs: [],
          stepRefs: [],
          stepOutputRefs: [],
          allRefs: [],
          candidates: [],
        },
      },
    },
  } satisfies AppRecipeSaveResult;
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  state = reduceAppGenerator(state, { type: "saving" });
  assert.equal(state.phase, "saving");
  state = reduceAppGenerator(state, { type: "saved", result });
  assert.equal(state.phase, "saved");
  assert.equal(state.saved?.openedRecipe.document.documentId, "document");
});

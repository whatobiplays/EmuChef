import assert from "node:assert/strict";
import test from "node:test";

import type {
  AppRecipeDraftResult,
  AppRecipeSaveResult,
} from "../src/api/types.js";
import {
  formToRequest,
  initialAppGeneratorState,
  reduceAppGenerator,
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
  form.aliasesText = "com.example.old\ncom.example.legacy";
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

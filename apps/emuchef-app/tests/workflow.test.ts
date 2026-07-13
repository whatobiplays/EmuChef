import assert from "node:assert/strict";
import test from "node:test";

import {
  initialWorkflowState,
  inputDiagnosticsForDisplay,
  pageDiagnosticsForDisplay,
  recipeSelectionDisabled,
  reviewReady,
  runBusyAction,
  updateRecipeSelection,
  workflowReducer,
} from "../src/workflow";

const facts = {
  deviceHandle: "device-opaque",
  manufacturer: "AYANEO",
  brand: "AYANEO",
  model: "Pocket S Mini",
  androidVersion: 13,
  androidApiLevel: 33,
};

const match = {
  confidence: "exact" as const,
  recommendedPlanId: "plan.one",
  requiresExplicitChoice: false,
  candidates: [],
  safeGenericPlans: [],
  blocked: false,
  blockReason: null,
};

test("device selection and probing advance without exposing a serial", () => {
  const selected = workflowReducer(initialWorkflowState, {
    type: "select-device",
    deviceHandle: facts.deviceHandle,
  });
  const probed = workflowReducer(selected, { type: "device-probed", facts, match });
  assert.equal(probed.step, "setup");
  assert.equal(probed.devicePlan, "plan.one");
  assert.equal("serial" in probed.facts!, false);
});

test("stale probe response is ignored", () => {
  const state = workflowReducer(initialWorkflowState, {
    type: "select-device",
    deviceHandle: "new-device",
  });
  assert.equal(workflowReducer(state, { type: "device-probed", facts, match }), state);
});

test("device disappearance invalidates all downstream state", () => {
  const selected = workflowReducer(initialWorkflowState, {
    type: "select-device",
    deviceHandle: facts.deviceHandle,
  });
  const probed = workflowReducer(selected, { type: "device-probed", facts, match });
  const reset = workflowReducer(probed, { type: "device-disappeared" });
  assert.equal(reset.step, "connect");
  assert.equal(reset.deviceHandle, null);
  assert.equal(reset.review, null);
});

test("back navigation preserves choices while leaving transient device loading", () => {
  const selected = workflowReducer(initialWorkflowState, {
    type: "select-device",
    deviceHandle: facts.deviceHandle,
  });
  const probed = workflowReducer(selected, { type: "device-probed", facts, match });
  const described = workflowReducer(
    { ...probed, requestGeneration: 1 },
    {
      type: "description",
      generation: 1,
      description: {
        devicePlan: "plan.one",
        selectedRecipes: ["recipe.one"],
        expandedRecipes: ["recipe.one"],
        recipeOptions: [],
        inputs: [],
        diagnostics: [],
      },
    },
  );
  const backToSetup = workflowReducer(described, { type: "back" });
  assert.equal(backToSetup.step, "setup");
  assert.equal(backToSetup.devicePlan, "plan.one");
  assert.deepEqual(backToSetup.selectedRecipes, ["recipe.one"]);
  assert.equal(workflowReducer(backToSetup, { type: "back" }).step, "connect");
});

test("required unresolved input prevents review", () => {
  const state = {
    ...initialWorkflowState,
    deviceHandle: facts.deviceHandle,
    devicePlan: "plan.one",
    descriptionDirty: false,
    description: {
      devicePlan: "plan.one",
      selectedRecipes: [],
      expandedRecipes: [],
      recipeOptions: [],
      diagnostics: [],
      inputs: [
        {
          key: "recipe/path",
          recipeId: "recipe",
          inputId: "path",
          type: "path",
          label: "Path",
          description: null,
          required: true,
          value: null,
          valueSource: null,
          diagnostics: [],
        },
      ],
    },
  };
  assert.equal(reviewReady(state), false);
});

test("required multiple path input remains unresolved when its list is empty", () => {
  const state = {
    ...initialWorkflowState,
    deviceHandle: facts.deviceHandle,
    devicePlan: "plan.one",
    descriptionDirty: false,
    description: {
      devicePlan: "plan.one",
      selectedRecipes: [],
      expandedRecipes: [],
      recipeOptions: [],
      diagnostics: [],
      inputs: [
        {
          key: "recipe/paths",
          recipeId: "recipe",
          inputId: "paths",
          type: "path",
          label: "Paths",
          description: null,
          required: true,
          multiple: true,
          value: [],
          valueSource: null,
          diagnostics: [],
        },
      ],
    },
  };
  assert.equal(reviewReady(state), false);
});

test("stale configuration descriptions are ignored", () => {
  const state = {
    ...initialWorkflowState,
    step: "inputs" as const,
    requestGeneration: 4,
    descriptionDirty: true,
  };
  const description = {
    devicePlan: "plan.one",
    selectedRecipes: [],
    expandedRecipes: [],
    recipeOptions: [],
    inputs: [],
    diagnostics: [],
  };
  assert.equal(
    workflowReducer(state, { type: "description", description, generation: 3 }),
    state,
  );
});

test("safe generic plans are never auto-selected", () => {
  const selected = workflowReducer(initialWorkflowState, {
    type: "select-device",
    deviceHandle: facts.deviceHandle,
  });
  const genericOnly = {
    ...match,
    confidence: "none" as const,
    recommendedPlanId: null,
    requiresExplicitChoice: true,
    safeGenericPlans: [
      {
        planId: "generic.safe",
        name: "Generic",
        description: null,
        profileId: "profile.generic",
        profileName: "Generic",
        reasons: ["Family matched"],
      },
    ],
  };
  const probed = workflowReducer(selected, {
    type: "device-probed",
    facts,
    match: genericOnly,
  });
  assert.equal(probed.devicePlan, null);
  assert.equal(probed.match?.blocked, false);
});

test("backend-expanded dependency selection is retained and cannot be deselected", () => {
  const dependency = {
    id: "recipe.dependency",
    name: "Dependency",
    description: null,
    selected: false,
    recommended: false,
    dependencyRequired: true,
    available: true,
    unavailableCapabilities: [],
  };
  assert.equal(recipeSelectionDisabled(dependency), true);
  assert.deepEqual(updateRecipeSelection([dependency.id], dependency, false), [dependency.id]);
});

test("unavailable recipes remain immutable while optional recipes can change", () => {
  const unavailable = {
    id: "recipe.unavailable",
    name: "Unavailable",
    description: null,
    selected: false,
    recommended: false,
    dependencyRequired: false,
    available: false,
    unavailableCapabilities: ["root_shell"],
  };
  const optional = { ...unavailable, id: "recipe.optional", available: true };
  assert.deepEqual(updateRecipeSelection([], unavailable, true), []);
  assert.deepEqual(updateRecipeSelection([], optional, true), [optional.id]);
});

test("input and global copies of one diagnostic render once beneath the input", () => {
  const diagnostic = {
    key: "app.xaniteog.install/xaniteog_apk",
    code: "binding_missing",
    message: "A XaniteOG APK is required.",
    severity: "error",
  };
  const input = {
    key: diagnostic.key,
    recipeId: "app.xaniteog.install",
    inputId: "xaniteog_apk",
    type: "file",
    label: "XaniteOG APK",
    description: null,
    required: true,
    value: null,
    valueSource: null,
    diagnostics: [diagnostic],
  };
  const description = {
    devicePlan: "plan.one",
    selectedRecipes: ["app.xaniteog.install"],
    expandedRecipes: ["app.xaniteog.install"],
    recipeOptions: [],
    inputs: [input],
    diagnostics: [{
      code: diagnostic.code,
      message: diagnostic.message,
      severity: diagnostic.severity,
    }],
  };

  assert.equal(inputDiagnosticsForDisplay(input).length, 1);
  assert.deepEqual(pageDiagnosticsForDisplay(description), []);
});

test("distinct keyed diagnostics with identical wording remain separate", () => {
  const description = {
    devicePlan: "plan.one",
    selectedRecipes: [],
    expandedRecipes: [],
    recipeOptions: [],
    inputs: [],
    diagnostics: [
      { key: "recipe.one/file", code: "binding_missing", message: "File required.", severity: "error" },
      { key: "recipe.two/file", code: "binding_missing", message: "File required.", severity: "error" },
    ],
  };

  assert.equal(pageDiagnosticsForDisplay(description).length, 2);
});

test("global-only diagnostic remains visible", () => {
  const diagnostic = {
    code: "device_plan_not_found",
    message: "The selected device plan is unavailable.",
    severity: "error",
  };
  const description = {
    devicePlan: "plan.missing",
    selectedRecipes: [],
    expandedRecipes: [],
    recipeOptions: [],
    inputs: [],
    diagnostics: [diagnostic],
  };

  assert.deepEqual(pageDiagnosticsForDisplay(description), [diagnostic]);
});

test("Platform-Tools picker cancellation resolves and clears frontend busy state", async () => {
  const busyStates: boolean[] = [];
  let completePicker!: (value: { status: "required" }) => void;
  const pickerResult = new Promise<{ status: "required" }>((resolve) => {
    completePicker = resolve;
  });
  let receivedStatus: { status: "required" } | null = null;

  const pending = runBusyAction({
    setBusy: (busy) => busyStates.push(busy),
    action: () => pickerResult,
    onSuccess: (status) => {
      receivedStatus = status;
    },
    onError: () => assert.fail("picker cancellation must not be an error"),
  });

  assert.deepEqual(busyStates, [true]);
  completePicker({ status: "required" });
  await pending;
  assert.deepEqual(receivedStatus, { status: "required" });
  assert.deepEqual(busyStates, [true, false]);
});

test("Platform-Tools import errors settle and clear frontend busy state", async () => {
  const busyStates: boolean[] = [];
  let handledError: unknown;

  await runBusyAction({
    setBusy: (busy) => busyStates.push(busy),
    action: async () => {
      throw new Error("invalid archive");
    },
    onSuccess: () => assert.fail("a failed import must not succeed"),
    onError: (error) => {
      handledError = error;
    },
  });

  assert.match(String(handledError), /invalid archive/);
  assert.deepEqual(busyStates, [true, false]);
});

test("path-picker cancellation settles and clears frontend busy state", async () => {
  const busyStates: boolean[] = [];
  let selected: string[] | null | undefined;

  await runBusyAction({
    setBusy: (busy) => busyStates.push(busy),
    action: async () => null as string[] | null,
    onSuccess: (values) => {
      selected = values;
    },
    onError: () => assert.fail("path-picker cancellation must not be an error"),
  });

  assert.equal(selected, null);
  assert.deepEqual(busyStates, [true, false]);
});

test("path-picker errors settle and clear frontend busy state", async () => {
  const busyStates: boolean[] = [];
  let handledError: unknown;

  await runBusyAction({
    setBusy: (busy) => busyStates.push(busy),
    action: async () => {
      throw new Error("picker unavailable");
    },
    onSuccess: () => assert.fail("a failed path picker must not succeed"),
    onError: (error) => {
      handledError = error;
    },
  });

  assert.match(String(handledError), /picker unavailable/);
  assert.deepEqual(busyStates, [true, false]);
});

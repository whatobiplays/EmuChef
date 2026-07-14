import assert from "node:assert/strict";
import test from "node:test";

import {
  initialWorkflowState,
  filterRepairBindings,
  inputDiagnosticsForDisplay,
  mergeExecutionEvents,
  pageDiagnosticsForDisplay,
  recipeSelectionDisabled,
  reviewReady,
  runBusyAction,
  updateRecipeSelection,
  workflowReducer,
} from "../src/workflow";
import type { ExecutionEvent, ExecutionSnapshot } from "../src/types";
import {
  emptyRealExecutionConfirmation,
  realExecutionConfirmationComplete,
} from "../src/realExecution";

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

const review = {
  reviewHandle: "review-opaque",
  planDigest: "digest",
  target: { manufacturer: "AYANEO", model: "Pocket S Mini", androidVersion: 13, androidApiLevel: 33 },
  groups: [],
  selectedInputs: [],
  warnings: [],
};

function executionSnapshot(
  latestSequence: number,
  status: ExecutionSnapshot["status"] = "running",
  executionHandle = "execution-opaque",
): ExecutionSnapshot {
  const recipeStatus = status === "queued" ? "pending" : status === "failed" ? "blocked" : status;
  const stepStatus = status === "queued"
    ? "pending"
    : status === "failed"
      ? "blocked"
      : status === "succeeded_with_warnings"
        ? "succeeded"
        : status;
  return {
    executionHandle,
    reviewHandle: review.reviewHandle,
    simulated: true,
    verificationScope: "simulation_only",
    status,
    startedAt: "2026-07-13T00:00:00Z",
    finishedAt: status === "running" ? null : "2026-07-13T00:00:01Z",
    latestSequence,
    terminal: status !== "running" && status !== "queued",
    recipes: [{
      recipeId: "recipe.one",
      name: "Recipe One",
      description: null,
      status: recipeStatus,
      steps: [{
        stepId: "step.one",
        name: "Step One",
        note: "Simulate step one",
        status: stepStatus,
        message: null,
      }],
    }],
    warnings: [],
    errors: [],
    completion: {
      classification: status === "failed" ? "failed" : status === "cancelled" ? "cancelled" : status === "succeeded_with_warnings" ? "success_with_warnings" : status === "succeeded" ? "success" : "in_progress",
      counts: {
        total: 1,
        completed: stepStatus === "succeeded" ? 1 : 0,
        skipped: 0,
        blocked: stepStatus === "blocked" ? 1 : 0,
        failed: 0,
        cancelled: stepStatus === "cancelled" ? 1 : 0,
        pending: stepStatus === "pending" ? 1 : 0,
      },
      warningCount: 0,
      partialChangesPossible: false,
      features: [],
    },
  };
}

test("repair keeps authoritative failed and cancelled labels while preserving safe intent", () => {
  for (const status of ["failed", "cancelled"] as const) {
    const snapshot = executionSnapshot(2, status);
    const terminal = {
      ...initialWorkflowState,
      step: "execution" as const,
      deviceHandle: facts.deviceHandle,
      devicePlan: "plan.one",
      selectedRecipes: ["recipe.one"],
      bindings: { "recipe.one/file": "/chosen/file" },
      executionGeneration: 1,
      execution: {
        kind: "terminal" as const,
        generation: 1,
        mode: "simulated" as const,
        snapshot,
        events: [],
        eventCursor: 2,
        cancellationRequested: false,
      },
    };
    const repairing = workflowReducer(terminal, { type: "prepare-repair" });
    assert.equal(snapshot.completion.classification, status);
    assert.equal(repairing.repairIntent, true);
    assert.deepEqual(repairing.selectedRecipes, ["recipe.one"]);
    assert.deepEqual(repairing.bindings, { "recipe.one/file": "/chosen/file" });
    assert.equal(repairing.review, null);
    assert.equal(repairing.execution.kind, "idle");
  }
});

test("repair bindings survive only unchanged input contracts", () => {
  const oldDescription = {
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    expandedRecipes: ["recipe.one"],
    recipeOptions: [],
    diagnostics: [],
    inputs: [
      { key: "same", recipeId: "recipe.one", inputId: "same", type: "file", label: "Same", description: null, required: true, pathKind: "file" as const, value: null, valueSource: null, diagnostics: [] },
      { key: "changed", recipeId: "recipe.one", inputId: "changed", type: "file", label: "Changed", description: null, required: true, pathKind: "file" as const, value: null, valueSource: null, diagnostics: [] },
    ],
  };
  const current = {
    ...oldDescription,
    inputs: [
      oldDescription.inputs[0],
      { ...oldDescription.inputs[1], type: "directory", pathKind: "directory" as const },
    ],
  };
  assert.deepEqual(
    filterRepairBindings(oldDescription, current, { same: "/safe", changed: "/stale", removed: "x" }),
    { same: "/safe" },
  );
});

function executionEvent(sequence: number): ExecutionEvent {
  return {
    sequence,
    timestamp: `2026-07-13T00:00:0${sequence}Z`,
    eventType: "step_progress",
    recipeId: "recipe.one",
    stepId: "step.one",
    phase: "action",
    status: "running",
    note: `Event ${sequence}`,
    message: null,
    issue: null,
  };
}

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

test("simulation start preserves the retained review until start succeeds", () => {
  const reviewed = { ...initialWorkflowState, step: "review" as const, review };
  const starting = workflowReducer(reviewed, { type: "execution-starting", generation: 1 });
  assert.equal(starting.step, "review");
  assert.equal(starting.review, review);
  assert.equal(starting.execution.kind, "starting");

  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: executionSnapshot(2),
  });
  assert.equal(active.step, "execution");
  assert.equal(active.review, review);
  assert.equal(active.execution.kind, "active");
  assert.equal(active.execution.kind === "active" && active.execution.eventCursor, 2);
});

test("authoritative snapshots replace progress and reject older responses", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1 },
  );
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: executionSnapshot(4),
  });
  assert.equal(
    workflowReducer(active, {
      type: "execution-snapshot",
      generation: 1,
      snapshot: executionSnapshot(3, "failed"),
    }),
    active,
  );
  const terminal = workflowReducer(active, {
    type: "execution-snapshot",
    generation: 1,
    snapshot: executionSnapshot(6, "failed"),
  });
  assert.equal(terminal.execution.kind, "terminal");
  assert.equal(terminal.execution.kind === "terminal" && terminal.execution.snapshot.recipes[0].status, "blocked");
  assert.equal(terminal.execution.kind === "terminal" && terminal.execution.eventCursor, 6);
});

test("every Phase 0 terminal status produces a terminal simulated result", () => {
  for (const status of ["succeeded", "succeeded_with_warnings", "failed", "cancelled"] as const) {
    const starting = workflowReducer(
      { ...initialWorkflowState, step: "review", review },
      { type: "execution-starting", generation: 1 },
    );
    const terminal = workflowReducer(starting, {
      type: "execution-started",
      generation: 1,
      snapshot: executionSnapshot(4, status),
    });
    assert.equal(terminal.execution.kind, "terminal", status);
    assert.equal(terminal.execution.kind === "terminal" && terminal.execution.snapshot.status, status);
  }
});

test("events are presentation-only, monotonic, and deduplicated after the snapshot cursor", () => {
  const merged = mergeExecutionEvents(
    [executionEvent(3)],
    [executionEvent(7), executionEvent(5), executionEvent(7), executionEvent(4)],
    4,
  );
  assert.deepEqual(merged.events.map((event) => event.sequence), [3, 5, 7]);
  assert.equal(merged.cursor, 7);
});

test("stale execution handles and generations cannot overwrite a newer run", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 2 },
  );
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 2,
    snapshot: executionSnapshot(2, "running", "new-handle"),
  });
  assert.equal(workflowReducer(active, {
    type: "execution-snapshot",
    generation: 1,
    snapshot: executionSnapshot(20, "failed", "old-handle"),
  }), active);
  assert.equal(workflowReducer(active, {
    type: "execution-snapshot",
    generation: 2,
    snapshot: executionSnapshot(20, "failed", "wrong-handle"),
  }), active);
});

test("cooperative cancellation disables repeats but waits for a terminal snapshot", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1 },
  );
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: executionSnapshot(1),
  });
  const cancelling = workflowReducer(active, { type: "execution-cancellation-requested", generation: 1 });
  assert.equal(cancelling.execution.kind, "active");
  assert.equal(cancelling.execution.kind === "active" && cancelling.execution.cancellationRequested, true);
  const cancelled = workflowReducer(cancelling, {
    type: "execution-snapshot",
    generation: 1,
    snapshot: executionSnapshot(3, "cancelled"),
  });
  assert.equal(cancelled.execution.kind, "terminal");
});

test("device disappearance after start does not erase simulated progress", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1 },
  );
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: executionSnapshot(1),
  });
  assert.equal(workflowReducer(active, { type: "device-disappeared" }), active);
});

test("device polling cannot erase a start that is still being revalidated", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1 },
  );
  assert.equal(workflowReducer(starting, { type: "device-disappeared" }), starting);
});

test("lost in-memory runs retain the review recovery path without claiming resume", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1 },
  );
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: executionSnapshot(1),
  });
  const unavailable = workflowReducer(active, {
    type: "execution-unavailable",
    generation: 1,
    executionHandle: "execution-opaque",
    message: "The in-memory simulated run was lost.",
  });
  assert.equal(unavailable.execution.kind, "unavailable");
  const reviewed = workflowReducer(unavailable, { type: "return-to-review" });
  assert.equal(reviewed.step, "review");
  assert.equal(reviewed.review, review);
});

test("real confirmation requires the exact phrase and every acknowledgment", () => {
  assert.equal(realExecutionConfirmationComplete(emptyRealExecutionConfirmation), false);
  const complete = {
    phrase: "APPLY TO DEVICE",
    irreversibleChangesAcknowledged: true,
    noRollbackAcknowledged: true,
    keepDeviceConnectedAcknowledged: true,
  };
  assert.equal(realExecutionConfirmationComplete(complete), true);
  assert.equal(realExecutionConfirmationComplete({ ...complete, phrase: " APPLY TO DEVICE " }), true);
  assert.equal(realExecutionConfirmationComplete({ ...complete, phrase: "apply to device" }), false);
  assert.equal(realExecutionConfirmationComplete({ ...complete, phrase: "Apply To Device" }), false);
  assert.equal(realExecutionConfirmationComplete({ ...complete, phrase: "APPLY  TO DEVICE" }), false);
  for (const acknowledgment of [
    "irreversibleChangesAcknowledged",
    "noRollbackAcknowledged",
    "keepDeviceConnectedAcknowledged",
  ] as const) {
    assert.equal(realExecutionConfirmationComplete({ ...complete, [acknowledgment]: false }), false);
  }
});

test("real execution state retains mode and invalidates the review on session loss", () => {
  const starting = workflowReducer(
    { ...initialWorkflowState, step: "review", review },
    { type: "execution-starting", generation: 1, mode: "real" },
  );
  const simulated = executionSnapshot(1);
  const realSnapshot = {
    ...simulated,
    simulated: false as const,
    verificationScope: "real_device" as const,
    target: { label: "Connected Android device" as const, androidApiLevel: 33 },
    launchAction: null,
  };
  const active = workflowReducer(starting, {
    type: "execution-started",
    generation: 1,
    snapshot: realSnapshot,
  });
  assert.equal(active.execution.kind === "active" && active.execution.mode, "real");
  const unavailable = workflowReducer(active, {
    type: "execution-unavailable",
    generation: 1,
    executionHandle: realSnapshot.executionHandle,
    message: "Outcome unknown.",
  });
  assert.equal(unavailable.execution.kind, "unavailable");
  assert.equal(unavailable.review, null);
});

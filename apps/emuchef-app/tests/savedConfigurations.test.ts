import assert from "node:assert/strict";
import test from "node:test";

import {
  formatLastOpened,
  resolveUnsavedDecision,
  saveConfigurationDisabledReason,
  savedConfigurationBlocksProgress,
  savedConfigurationDiagnosticSummary,
  savedConfigurationValidationLabel,
  savedDevicePlanAvailable,
} from "../src/savedConfigurations";
import { initialWorkflowState, workflowReducer } from "../src/workflow";
import type { DeviceMatch, SavedConfigurationDocument } from "../src/types";

const document: SavedConfigurationDocument = {
  configurationHandle: "configuration-opaque",
  name: "Travel setup",
  dirty: true,
  revision: 2,
  devicePlan: "plan.saved",
  selectedRecipes: ["recipe.saved", "recipe.removed"],
  bindings: { "recipe.saved/path": "/chosen/path", "recipe.removed/value": true },
  validation: { state: "requires_attention", diagnostics: [] },
};

const match: DeviceMatch = {
  confidence: "high",
  recommendedPlanId: "plan.other",
  requiresExplicitChoice: false,
  candidates: [{
    planId: "plan.saved",
    name: "Saved",
    description: null,
    profileId: "profile.saved",
    profileName: "Saved",
    reasons: [],
  }],
  safeGenericPlans: [],
  blocked: false,
  blockReason: null,
};

test("dirty decisions expose Save, Discard, and Cancel without prompting clean state", () => {
  assert.equal(resolveUnsavedDecision(false, false, false), "discard");
  assert.equal(resolveUnsavedDecision(true, true, false), "save");
  assert.equal(resolveUnsavedDecision(true, false, true), "discard");
  assert.equal(resolveUnsavedDecision(true, false, false), "cancel");
});

test("saved device plan references are never silently substituted", () => {
  assert.equal(savedDevicePlanAvailable(document, match), true);
  assert.equal(
    savedDevicePlanAvailable(
      { ...document, devicePlan: "plan.removed" },
      match,
    ),
    false,
  );
  assert.equal(savedDevicePlanAvailable(document, { ...match, blocked: true }), false);
});

test("blocking saved configurations are gated with user-facing validation copy", () => {
  assert.equal(savedConfigurationBlocksProgress(document), true);
  assert.equal(savedConfigurationBlocksProgress({
    ...document,
    validation: { state: "valid_with_warnings", diagnostics: [] },
  }), false);
  assert.equal(savedConfigurationValidationLabel(document), "Needs repair before continuing");
  assert.equal(savedConfigurationValidationLabel({
    ...document,
    validation: { state: "cannot_use", diagnostics: [] },
  }), "Cannot be used with the current catalog");
  assert.equal(savedConfigurationDiagnosticSummary({
    code: "unknown_recipe",
    message: "Selected recipe recipe.removed was not found",
    severity: "error",
    key: "recipe.removed",
  }), "A recipe used by this configuration is no longer available.");
});

test("saved configurations use neutral Save guidance after successful persistence", () => {
  assert.equal(
    saveConfigurationDisabledReason(document, true, false),
    "Configuration saved. Save becomes available after another change.",
  );
  assert.equal(
    saveConfigurationDisabledReason(null, false, false),
    "Save requires a selected device plan and unsaved portable changes.",
  );
  assert.equal(
    saveConfigurationDisabledReason(document, false, false),
    "Save requires a selected device plan and unsaved portable changes.",
  );
});

test("opening portable intent preserves stale references but resets all runtime authority", () => {
  const loaded = workflowReducer(
    {
      ...initialWorkflowState,
      review: {
        reviewHandle: "old-review",
        setup: { name: "Saved setup" },
        target: { label: "Connected Android device" },
        features: [],
        inputs: [],
        notices: [],
        work: { actionCount: 0 },
        canExecute: true,
      },
    },
    {
      type: "load-portable-intent",
      devicePlan: document.devicePlan,
      selectedRecipes: document.selectedRecipes,
      bindings: document.bindings,
      dirty: document.dirty,
    },
  );
  assert.equal(loaded.step, "connect");
  assert.equal(loaded.deviceHandle, null);
  assert.equal(loaded.facts, null);
  assert.equal(loaded.review, null);
  assert.equal(loaded.execution.kind, "idle");
  assert.equal(loaded.devicePlan, "plan.saved");
  assert.deepEqual(loaded.selectedRecipes, ["recipe.saved", "recipe.removed"]);
  assert.deepEqual(loaded.bindings, document.bindings);
});

test("Platform-Tools invalidation preserves dirty portable edits only", () => {
  const loaded = workflowReducer(initialWorkflowState, {
    type: "load-portable-intent",
    devicePlan: document.devicePlan,
    selectedRecipes: document.selectedRecipes,
    bindings: document.bindings,
    dirty: true,
  });
  const invalidated = workflowReducer(
    { ...loaded, deviceHandle: "device-opaque", facts: null },
    { type: "infrastructure-invalidated" },
  );
  assert.equal(invalidated.step, "connect");
  assert.equal(invalidated.deviceHandle, null);
  assert.equal(invalidated.portableIntentDirty, true);
  assert.equal(invalidated.devicePlan, document.devicePlan);
  assert.deepEqual(invalidated.bindings, document.bindings);
});

test("last-opened display rejects invalid timestamps", () => {
  assert.equal(formatLastOpened(0), "Last opened time unavailable");
  assert.match(formatLastOpened(1_700_000_000_000), /^Last opened /);
});

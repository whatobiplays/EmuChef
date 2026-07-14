import assert from "node:assert/strict";
import test from "node:test";

import { portableIntentSignature, recoveryResultIsCurrent } from "../src/recovery";
import { initialWorkflowState, reviewReady, workflowReducer } from "../src/workflow";

test("portable recovery identity changes only with portable intent", () => {
  const first = portableIntentSignature({
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    bindings: { "recipe.one/b": 2, "recipe.one/a": { z: true, a: false } },
  });
  const reordered = portableIntentSignature({
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    bindings: { "recipe.one/a": { a: false, z: true }, "recipe.one/b": 2 },
  });
  const changed = portableIntentSignature({
    devicePlan: "plan.two",
    selectedRecipes: ["recipe.one"],
    bindings: { "recipe.one/a": { a: false, z: true }, "recipe.one/b": 2 },
  });
  assert.equal(first, reordered);
  assert.notEqual(first, changed);
});

test("stale recovery responses cannot replace newer intent", () => {
  assert.equal(
    recoveryResultIsCurrent({ requestGeneration: 4, draftGeneration: 7 }, 4, 7),
    true,
  );
  assert.equal(
    recoveryResultIsCurrent({ requestGeneration: 3, draftGeneration: 7 }, 4, 7),
    false,
  );
  assert.equal(
    recoveryResultIsCurrent({ requestGeneration: 4, draftGeneration: 6 }, 4, 7),
    false,
  );
});

test("restored intent is dirty, has no transient authority, and blocks review for re-entry", () => {
  const restored = workflowReducer(initialWorkflowState, {
    type: "load-portable-intent",
    devicePlan: "plan.one",
    selectedRecipes: ["recipe.one"],
    bindings: {},
    dirty: true,
    requiredReentryBindings: ["recipe.one/neutral"],
  });
  assert.equal(restored.portableIntentDirty, true);
  assert.equal(restored.deviceHandle, null);
  assert.equal(restored.facts, null);
  assert.equal(restored.review, null);
  assert.deepEqual(restored.execution, { kind: "idle" });
  assert.equal(reviewReady(restored), false);

  const supplied = workflowReducer(restored, {
    type: "set-binding",
    key: "recipe.one/neutral",
    value: "replacement",
  });
  assert.deepEqual(supplied.requiredReentryBindings, []);
});

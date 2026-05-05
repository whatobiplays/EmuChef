import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAddDependencyList,
  buildRemoveDependencyList,
  dependencyEntries,
  selectableDependencySteps,
  stepDependencyIds,
  type StepDependencySummary,
} from "../src/components/stepDependencies.logic.js";

const steps: StepDependencySummary[] = [
  { id: "prepare", name: "Prepare", type: "wait" },
  { id: "extract", name: "Extract Cores", type: "extract_artifacts" },
  { id: "copy", name: "Copy Cores", type: "copy_files" },
];

test("stepDependencyIds treats missing and null dependencies as empty for UI reads", () => {
  assert.deepEqual(stepDependencyIds({}), []);
  assert.deepEqual(stepDependencyIds({ dependencies: null }), []);
});

test("selectableDependencySteps excludes the selected step and existing dependencies", () => {
  const selectable = selectableDependencySteps(steps, "copy", ["prepare"]);

  assert.deepEqual(
    selectable.map((step) => step.id),
    ["extract"],
  );
});

test("buildAddDependencyList appends dependency ids without sorting", () => {
  const result = buildAddDependencyList(["prepare"], "copy", "extract");

  assert.deepEqual(result, { ok: true, dependencies: ["prepare", "extract"] });
});

test("buildAddDependencyList blocks missing, self, and duplicate dependency selections", () => {
  assert.deepEqual(buildAddDependencyList(["prepare"], "copy", null), {
    ok: false,
    reason: "no-dependency",
  });
  assert.deepEqual(buildAddDependencyList(["prepare"], "copy", "copy"), {
    ok: false,
    reason: "self-dependency",
  });
  assert.deepEqual(buildAddDependencyList(["prepare"], "copy", "prepare"), {
    ok: false,
    reason: "duplicate-dependency",
  });
});

test("buildRemoveDependencyList removes missing dependency ids and preserves remaining order", () => {
  assert.deepEqual(buildRemoveDependencyList(["prepare", "missing_step", "extract"], "missing_step"), [
    "prepare",
    "extract",
  ]);
});

test("dependencyEntries preserves raw missing dependency ids for repair", () => {
  const entries = dependencyEntries(["prepare", "missing_step"], steps);

  assert.equal(entries[0].id, "prepare");
  assert.equal(entries[0].step?.name, "Prepare");
  assert.equal(entries[0].missing, false);
  assert.equal(entries[1].id, "missing_step");
  assert.equal(entries[1].step, null);
  assert.equal(entries[1].missing, true);
});

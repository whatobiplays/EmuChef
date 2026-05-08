import assert from "node:assert/strict";
import test from "node:test";

import {
  beginCommand,
  buildActionAvailability,
  classifyOperationFailure,
  decideCloseRequest,
  resolveClosePromptResult,
  resolveOpenAttempt,
  type CommandName,
} from "../src/components/phase5EditorState.logic.js";

test("buildActionAvailability gates document actions on session validity independently of dirty state", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: false,
    }),
    {
      openRecipe: false,
      saveRecipe: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability disables conflicting actions while a command is in flight", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: "saveRecipe",
      documentSessionValid: true,
    }),
    {
      openRecipe: false,
      saveRecipe: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability enables only valid clean-document actions", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      dirty: false,
      canUndo: true,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }),
    {
      openRecipe: true,
      saveRecipe: false,
      undo: true,
      redo: false,
      validate: true,
      refreshYaml: true,
      editDocument: true,
    },
  );
});

test("beginCommand prevents duplicate submissions until the current command completes", () => {
  assert.deepEqual(beginCommand(null, "validate"), { started: true, commandInFlight: "validate" });
  assert.deepEqual(beginCommand("validate", "validate"), { started: false, commandInFlight: "validate" });
  assert.deepEqual(beginCommand("saveRecipe", "undo"), { started: false, commandInFlight: "saveRecipe" });
});

test("resolveOpenAttempt preserves the existing document for picker cancel or failed open", () => {
  const current = { documentId: "old-doc" };
  const next = { documentId: "new-doc" };

  assert.deepEqual(resolveOpenAttempt(current, { kind: "picker-cancelled" }), {
    document: current,
    replaced: false,
    sessionValid: true,
  });
  assert.deepEqual(resolveOpenAttempt(current, { kind: "open-failed", sessionInvalid: false }), {
    document: current,
    replaced: false,
    sessionValid: true,
  });
  assert.deepEqual(resolveOpenAttempt(current, { kind: "open-failed", sessionInvalid: true }), {
    document: current,
    replaced: false,
    sessionValid: false,
  });
  assert.deepEqual(resolveOpenAttempt(current, { kind: "opened", document: next }), {
    document: next,
    replaced: true,
    sessionValid: true,
  });
});

test("classifyOperationFailure distinguishes api errors from fatal sidecar transport failures", () => {
  assert.deepEqual(
    classifyOperationFailure(
      {
        kind: "api-error",
        error: { code: "validation_failed", message: "Recipe is invalid.", details: {} },
      },
      "Open failed.",
    ),
    {
      message: "Open failed. validation_failed: Recipe is invalid.",
      sessionInvalid: false,
    },
  );

  assert.deepEqual(
    classifyOperationFailure({ kind: "transport-error", message: "Python sidecar exited unexpectedly" }, "Save failed."),
    {
      message:
        "Save failed. Python sidecar exited unexpectedly The editor session is no longer valid. Restart the Tauri app and reopen the recipe.",
      sessionInvalid: true,
    },
  );
});

test("command names stay limited to known Phase 5 document operations", () => {
  const names: CommandName[] = ["openRecipe", "saveRecipe", "undo", "redo", "validate", "refreshYaml", "mutation"];

  assert.equal(names.length, 7);
});

test("decideCloseRequest allows clean windows to close without prompting", () => {
  assert.deepEqual(decideCloseRequest({ dirty: false, promptInFlight: false }), {
    kind: "allow",
  });
});

test("decideCloseRequest prompts once for dirty windows", () => {
  assert.deepEqual(decideCloseRequest({ dirty: true, promptInFlight: false }), {
    kind: "prompt",
  });
});

test("decideCloseRequest prevents duplicate close prompts", () => {
  assert.deepEqual(decideCloseRequest({ dirty: true, promptInFlight: true }), {
    kind: "prevent",
  });
});

test("resolveClosePromptResult prevents only cancelled dirty closes", () => {
  assert.deepEqual(resolveClosePromptResult(false), { kind: "prevent" });
  assert.deepEqual(resolveClosePromptResult(true), { kind: "allow" });
});

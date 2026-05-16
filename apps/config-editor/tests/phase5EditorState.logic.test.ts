import assert from "node:assert/strict";
import test from "node:test";

import {
  beginCommand,
  buildCloseConfirmationCopy,
  buildActionAvailability,
  classifySidecarStatus,
  classifyOperationFailure,
  decideCloseRequest,
  formatSidecarStatusLabel,
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
      saveRecipeAs: false,
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
      saveRecipeAs: false,
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
      saveRecipeAs: true,
      undo: true,
      redo: false,
      validate: true,
      refreshYaml: true,
      editDocument: true,
    },
  );
});

test("buildActionAvailability treats compatible backend metadata as editable", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: true,
    }),
    {
      openRecipe: true,
      saveRecipe: true,
      saveRecipeAs: true,
      undo: true,
      redo: true,
      validate: true,
      refreshYaml: true,
      editDocument: true,
    },
  );
});

test("buildActionAvailability disables document actions for incompatible backend metadata", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: false,
    }),
    {
      openRecipe: false,
      saveRecipe: false,
      saveRecipeAs: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability treats unchecked compatibility as neutral", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: null,
    }).openRecipe,
    true,
  );
});

test("buildActionAvailability disables Save As when no document is open", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: false,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }).saveRecipeAs,
    false,
  );
});

test("buildActionAvailability enables Save As for clean and dirty valid documents", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }).saveRecipeAs,
    true,
  );
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }).saveRecipeAs,
    true,
  );
});

test("buildActionAvailability gates Save As on in-flight commands and backend readiness", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: "saveRecipe",
      documentSessionValid: true,
    }).saveRecipeAs,
    false,
  );
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: false,
    }).saveRecipeAs,
    false,
  );
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: false,
    }).saveRecipeAs,
    false,
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
    classifyOperationFailure({ kind: "transport-error", message: "Rust sidecar exited unexpectedly" }, "Save failed."),
    {
      message:
        "Save failed. Rust sidecar exited unexpectedly The editor session is no longer valid. Restart the Tauri app and reopen the recipe.",
      sessionInvalid: true,
    },
  );
});

test("classifyOperationFailure invalidates only unknown_document errors for the current document", () => {
  const failure = {
    kind: "api-error" as const,
    error: { code: "unknown_document", message: "Document was closed.", details: {} },
  };

  assert.deepEqual(classifyOperationFailure(failure, "Save failed."), {
    message: "Save failed. unknown_document: Document was closed.",
    sessionInvalid: false,
  });

  assert.deepEqual(
    classifyOperationFailure(failure, "Save failed.", {
      commandDocumentId: "stale-doc",
      currentDocumentId: "current-doc",
    }),
    {
      message: "Save failed. unknown_document: Document was closed.",
      sessionInvalid: false,
    },
  );

  assert.deepEqual(
    classifyOperationFailure(failure, "Save failed.", {
      commandDocumentId: "current-doc",
      currentDocumentId: "current-doc",
    }),
    {
      message:
        "Save failed. unknown_document: Document was closed. The editor session is no longer valid. Restart the Tauri app and reopen the recipe.",
      sessionInvalid: true,
    },
  );
});

test("classifySidecarStatus invalidates only exited error or incompatible states", () => {
  assert.deepEqual(
    classifySidecarStatus({
      running: false,
      pid: null,
      state: "notStarted",
      compatible: null,
      protocolVersion: null,
      capabilities: [],
      lastError: null,
    }),
    { sessionInvalid: false, message: null },
  );

  assert.deepEqual(
    classifySidecarStatus({
      running: true,
      pid: 123,
      state: "running",
      compatible: true,
      protocolVersion: 1,
      capabilities: ["listStepSpecs"],
      lastError: null,
    }),
    { sessionInvalid: false, message: null },
  );

  assert.deepEqual(
    classifySidecarStatus({
      running: false,
      pid: null,
      state: "incompatible",
      compatible: false,
      protocolVersion: null,
      capabilities: [],
      lastError: "Backend hello response was malformed.",
    }),
    {
      sessionInvalid: true,
      message:
        "Backend hello response was malformed. The editor session is no longer valid. Restart the Tauri app and reopen the recipe.",
    },
  );
});

test("formatSidecarStatusLabel handles compatibility metadata safely", () => {
  assert.equal(formatSidecarStatusLabel(null), "Sidecar: unknown");
  assert.equal(
    formatSidecarStatusLabel({
      running: false,
      pid: null,
      state: "notStarted",
      compatible: null,
      protocolVersion: null,
      capabilities: [],
      lastError: null,
    }),
    "Sidecar: not started",
  );
  assert.equal(
    formatSidecarStatusLabel({
      running: true,
      pid: 123,
      state: "running",
      compatible: true,
      protocolVersion: 1,
      capabilities: ["listStepSpecs"],
      lastError: null,
    }),
    "Sidecar: compatible v1 pid 123",
  );
  assert.equal(
    formatSidecarStatusLabel({
      running: false,
      pid: null,
      state: "incompatible",
      compatible: false,
      protocolVersion: null,
      capabilities: [],
      lastError: "missing capability",
    }),
    "Sidecar: incompatible",
  );
  assert.equal(formatSidecarStatusLabel({ running: true, pid: 7 }), "Sidecar: running pid 7");
});

test("command names stay limited to known Phase 5 document operations", () => {
  const names: CommandName[] = [
    "openRecipe",
    "saveRecipe",
    "saveRecipeAs",
    "undo",
    "redo",
    "validate",
    "refreshYaml",
    "mutation",
  ];

  assert.equal(names.length, 8);
});

test("decideCloseRequest allows clean windows to close without prompting", () => {
  assert.deepEqual(decideCloseRequest({ commandInFlight: null, dirty: false, promptInFlight: false }), {
    kind: "allow",
  });
});

test("decideCloseRequest prompts once for dirty windows", () => {
  assert.deepEqual(decideCloseRequest({ commandInFlight: null, dirty: true, promptInFlight: false }), {
    kind: "prompt",
    reason: "dirty",
  });
});

test("decideCloseRequest prompts for in-flight commands even when the document is clean", () => {
  assert.deepEqual(decideCloseRequest({ commandInFlight: "validate", dirty: false, promptInFlight: false }), {
    kind: "prompt",
    reason: "command-in-flight",
  });
});

test("decideCloseRequest reports both dirty and in-flight close risks together", () => {
  assert.deepEqual(decideCloseRequest({ commandInFlight: "saveRecipe", dirty: true, promptInFlight: false }), {
    kind: "prompt",
    reason: "dirty-and-command-in-flight",
  });
});

test("decideCloseRequest prevents duplicate close prompts", () => {
  assert.deepEqual(decideCloseRequest({ commandInFlight: "validate", dirty: true, promptInFlight: true }), {
    kind: "prevent",
  });
});

test("buildCloseConfirmationCopy mentions both unsaved changes and in-flight work when both are present", () => {
  const copy = buildCloseConfirmationCopy("dirty-and-command-in-flight", "app.example.recipe");

  assert.match(copy.message, /unsaved changes/i);
  assert.match(copy.message, /operation is still in progress/i);
  assert.match(copy.message, /app\.example\.recipe/);
});

test("resolveClosePromptResult prevents only cancelled dirty closes", () => {
  assert.deepEqual(resolveClosePromptResult(false), { kind: "prevent" });
  assert.deepEqual(resolveClosePromptResult(true), { kind: "allow" });
});

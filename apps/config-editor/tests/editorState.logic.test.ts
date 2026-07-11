import assert from "node:assert/strict";
import test from "node:test";

import {
  beginCommand,
  buildCloseConfirmationCopy,
  buildActionAvailability,
  buildInvalidSessionRecovery,
  classifySidecarStatus,
  classifyOperationFailure,
  classifyRestartFailure,
  decideCloseRequest,
  formatSidecarStatusLabel,
  resolveAuthoredRootSelectionAttempt,
  resolveClosePromptResult,
  resolveOpenAttempt,
  resolveReopenAttempt,
  resolveRestartSuccess,
  type ActionAvailabilityState,
  type CommandName,
} from "../src/components/editorState.logic.js";

function recoveryState(overrides: Partial<ActionAvailabilityState>): ActionAvailabilityState {
  return {
    hasDocument: true,
    hasSelectedAuthoredRoot: false,
    hasDocumentAuthoredRoot: false,
    hasDocumentPath: true,
    dirty: false,
    canUndo: false,
    canRedo: false,
    commandInFlight: null,
    documentSessionValid: false,
    backendCompatible: true,
    sidecarRunning: true,
    ...overrides,
  };
}

test("restart recovery offers clean path-backed stale documents for read-only reopen without confirmation", () => {
  const recovery = buildInvalidSessionRecovery(recoveryState({ dirty: false, hasDocumentPath: true }));

  assert.equal(recovery.readOnly, true);
  assert.equal(recovery.reopenFromDisk, true);
  assert.equal(recovery.reopenRequiresConfirmation, false);
  assert.match(recovery.message ?? "", /read-only/i);
});

test("restart recovery warns before reopening dirty path-backed stale documents", () => {
  const recovery = buildInvalidSessionRecovery(recoveryState({ dirty: true, hasDocumentPath: true }));

  assert.equal(recovery.readOnly, true);
  assert.equal(recovery.reopenFromDisk, true);
  assert.equal(recovery.reopenRequiresConfirmation, true);
  assert.match(recovery.message ?? "", /unsaved/i);
});

test("restart recovery keeps untitled stale documents read-only without reopen from disk", () => {
  const recovery = buildInvalidSessionRecovery(recoveryState({ dirty: true, hasDocumentPath: false }));

  assert.equal(recovery.readOnly, true);
  assert.equal(recovery.reopenFromDisk, false);
  assert.equal(recovery.reopenRequiresConfirmation, false);
  assert.match(recovery.message ?? "", /no disk path/i);
});

test("restart success invalidates displayed document sessions and enables recovery only for usable sidecars", () => {
  assert.deepEqual(
    resolveRestartSuccess(
      {
        running: true,
        pid: 123,
        state: "running",
        compatible: true,
        protocolVersion: 1,
        capabilities: ["listStepSpecs"],
        lastError: null,
      },
      true,
    ),
    {
      sessionValid: false,
      message:
        "The sidecar was restarted. The displayed recipe is a stale read-only reference; reopen it from disk to create a new document session.",
      sidecarUsable: true,
      stepSpecsRefreshAllowed: true,
    },
  );

  assert.deepEqual(
    resolveRestartSuccess(
      {
        running: false,
        pid: null,
        state: "incompatible",
        compatible: false,
        protocolVersion: null,
        capabilities: [],
        lastError: "Backend is missing required capabilities.",
      },
      true,
    ),
    {
      sessionValid: false,
      message:
        "Backend is missing required capabilities. The editor session is no longer valid. Restart the sidecar and reopen the recipe.",
      sidecarUsable: false,
      stepSpecsRefreshAllowed: false,
    },
  );
});

test("restart failure preserves current session validity and returns a surfaced error", () => {
  assert.deepEqual(
    classifyRestartFailure({ kind: "transport-error", message: "failed to start sidecar" }, "Sidecar restart failed."),
    {
      message: "Sidecar restart failed. failed to start sidecar",
      sessionInvalid: false,
    },
  );
});

test("action availability allows compatible sidecar recovery while keeping invalid documents read-only", () => {
  const availability = buildActionAvailability(
    recoveryState({
      dirty: true,
      hasDocumentPath: true,
      documentSessionValid: false,
      backendCompatible: true,
      sidecarRunning: true,
    }),
  );

  assert.equal(availability.openRecipe, true);
  assert.equal(availability.reopenFromDisk, true);
  assert.equal(availability.restartSidecar, true);
  assert.equal(availability.editDocument, false);
  assert.equal(availability.saveRecipe, false);
  assert.equal(availability.saveRecipeAs, false);
  assert.equal(availability.validate, false);
  assert.equal(availability.undo, false);
  assert.equal(availability.redo, false);
});

test("action availability keeps recovery opens disabled for incompatible restarted sidecars", () => {
  const availability = buildActionAvailability(
    recoveryState({
      hasDocumentPath: true,
      documentSessionValid: false,
      backendCompatible: false,
      sidecarRunning: false,
    }),
  );

  assert.equal(availability.openRecipe, false);
  assert.equal(availability.reopenFromDisk, false);
  assert.equal(availability.restartSidecar, true);
});

test("action availability disables restart during any in-flight command", () => {
  assert.equal(
    buildActionAvailability(
      recoveryState({
        commandInFlight: "validate",
        documentSessionValid: false,
        backendCompatible: true,
        sidecarRunning: true,
      }),
    ).restartSidecar,
    false,
  );
});

test("reopen resolution replaces stale state only after successful reopen", () => {
  const stale = { documentId: "stale" };
  const reopened = { documentId: "reopened" };

  assert.deepEqual(resolveReopenAttempt(stale, { kind: "opened", document: reopened }), {
    document: reopened,
    sessionValid: true,
    replaced: true,
  });
  assert.deepEqual(resolveReopenAttempt(stale, { kind: "open-failed", sessionInvalid: false }), {
    document: stale,
    sessionValid: false,
    replaced: false,
  });
});

test("buildActionAvailability gates document actions on session validity independently of dirty state", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: false,
    }),
    {
      openRecipe: false,
      restartSidecar: true,
      reopenFromDisk: false,
      saveRecipe: false,
      saveRecipeAs: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      setAuthoredRoot: false,
      clearAuthoredRoot: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability disables conflicting actions while a command is in flight", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: "saveRecipe",
      documentSessionValid: true,
    }),
    {
      openRecipe: false,
      restartSidecar: false,
      reopenFromDisk: false,
      saveRecipe: false,
      saveRecipeAs: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      setAuthoredRoot: false,
      clearAuthoredRoot: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability enables only valid clean-document actions", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: true,
      dirty: false,
      canUndo: true,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }),
    {
      openRecipe: true,
      restartSidecar: true,
      reopenFromDisk: false,
      saveRecipe: false,
      saveRecipeAs: true,
      undo: true,
      redo: false,
      validate: true,
      refreshYaml: true,
      setAuthoredRoot: true,
      clearAuthoredRoot: true,
      editDocument: true,
    },
  );
});

test("buildActionAvailability treats compatible backend metadata as editable", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: false,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: true,
    }),
    {
      openRecipe: true,
      restartSidecar: true,
      reopenFromDisk: false,
      saveRecipe: true,
      saveRecipeAs: true,
      undo: true,
      redo: true,
      validate: true,
      refreshYaml: true,
      setAuthoredRoot: true,
      clearAuthoredRoot: true,
      editDocument: true,
    },
  );
});

test("buildActionAvailability disables document actions for incompatible backend metadata", () => {
  assert.deepEqual(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: true,
      dirty: true,
      canUndo: true,
      canRedo: true,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: false,
    }),
    {
      openRecipe: false,
      restartSidecar: true,
      reopenFromDisk: false,
      saveRecipe: false,
      saveRecipeAs: false,
      undo: false,
      redo: false,
      validate: false,
      refreshYaml: false,
      setAuthoredRoot: false,
      clearAuthoredRoot: false,
      editDocument: false,
    },
  );
});

test("buildActionAvailability treats unchecked compatibility as neutral", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: false,
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

test("buildActionAvailability enables authored-root selection without an open document", () => {
  const availability = buildActionAvailability({
    hasDocument: false,
    hasSelectedAuthoredRoot: false,
    hasDocumentAuthoredRoot: false,
    dirty: false,
    canUndo: false,
    canRedo: false,
    commandInFlight: null,
    documentSessionValid: true,
  });

  assert.equal(availability.setAuthoredRoot, true);
  assert.equal(availability.clearAuthoredRoot, false);
  assert.equal(availability.editDocument, false);
});

test("buildActionAvailability enables clearing selected or document authored roots", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: false,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: false,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }).clearAuthoredRoot,
    true,
  );
  assert.equal(
    buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: false,
      hasDocumentAuthoredRoot: true,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
    }).clearAuthoredRoot,
    true,
  );
});

test("buildActionAvailability gates authored-root actions on backend readiness", () => {
  assert.equal(
    buildActionAvailability({
      hasDocument: false,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: false,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: null,
      documentSessionValid: true,
      backendCompatible: false,
    }).setAuthoredRoot,
    false,
  );
  assert.equal(
    buildActionAvailability({
      hasDocument: false,
      hasSelectedAuthoredRoot: true,
      hasDocumentAuthoredRoot: false,
      dirty: false,
      canUndo: false,
      canRedo: false,
      commandInFlight: "setAuthoredRoot",
      documentSessionValid: true,
    }).clearAuthoredRoot,
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

test("resolveAuthoredRootSelectionAttempt updates selected root only for no-document or successful updates", () => {
  const current = { documentId: "doc-1", authoredRoot: "/old/root" };
  const next = { documentId: "doc-1", authoredRoot: "/new/root" };

  assert.deepEqual(
    resolveAuthoredRootSelectionAttempt("/selected/root", current, { kind: "picker-cancelled" }),
    {
      selectedAuthoredRoot: "/selected/root",
      document: current,
      changed: false,
    },
  );
  assert.deepEqual(
    resolveAuthoredRootSelectionAttempt("/selected/root", current, { kind: "update-failed" }),
    {
      selectedAuthoredRoot: "/selected/root",
      document: current,
      changed: false,
    },
  );
  assert.deepEqual(resolveAuthoredRootSelectionAttempt(null, null, { kind: "no-document", authoredRoot: "/new/root" }), {
    selectedAuthoredRoot: "/new/root",
    document: null,
    changed: true,
  });
  assert.deepEqual(resolveAuthoredRootSelectionAttempt("/old/root", current, { kind: "updated", authoredRoot: null, document: next }), {
    selectedAuthoredRoot: null,
    document: next,
    changed: true,
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
        "Save failed. Rust sidecar exited unexpectedly The editor session is no longer valid. Restart the sidecar and reopen the recipe.",
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
        "Save failed. unknown_document: Document was closed. The editor session is no longer valid. Restart the sidecar and reopen the recipe.",
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
        "Backend hello response was malformed. The editor session is no longer valid. Restart the sidecar and reopen the recipe.",
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

test("command names stay limited to known editor document operations", () => {
  const names: CommandName[] = [
    "openRecipe",
    "saveRecipe",
    "saveRecipeAs",
    "restartSidecar",
    "undo",
    "redo",
    "validate",
    "refreshYaml",
    "setAuthoredRoot",
    "mutation",
  ];

  assert.equal(names.length, 10);
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

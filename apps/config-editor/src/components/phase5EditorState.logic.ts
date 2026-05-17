import type { ApiError, SidecarStatusResult } from "../api/types.js";

export type CommandName =
  | "openRecipe"
  | "saveRecipe"
  | "saveRecipeAs"
  | "undo"
  | "redo"
  | "validate"
  | "refreshYaml"
  | "setAuthoredRoot"
  | "mutation";

export interface ActionAvailabilityState {
  hasDocument: boolean;
  hasSelectedAuthoredRoot: boolean;
  hasDocumentAuthoredRoot: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  commandInFlight: CommandName | null;
  documentSessionValid: boolean;
  backendCompatible?: boolean | null;
}

export interface ActionAvailability {
  openRecipe: boolean;
  saveRecipe: boolean;
  saveRecipeAs: boolean;
  undo: boolean;
  redo: boolean;
  validate: boolean;
  refreshYaml: boolean;
  setAuthoredRoot: boolean;
  clearAuthoredRoot: boolean;
  editDocument: boolean;
}

export interface BeginCommandResult {
  started: boolean;
  commandInFlight: CommandName;
}

export type OpenAttempt<TDocument> =
  | { kind: "picker-cancelled" }
  | { kind: "opened"; document: TDocument }
  | { kind: "open-failed"; sessionInvalid: boolean };

export interface OpenAttemptResolution<TDocument> {
  document: TDocument | null;
  replaced: boolean;
  sessionValid: boolean;
}

export type AuthoredRootSelectionAttempt<TDocument> =
  | { kind: "picker-cancelled" }
  | { kind: "no-document"; authoredRoot: string | null }
  | { kind: "updated"; authoredRoot: string | null; document: TDocument }
  | { kind: "update-failed" };

export interface AuthoredRootSelectionResolution<TDocument> {
  selectedAuthoredRoot: string | null;
  document: TDocument | null;
  changed: boolean;
}

export type OperationFailure =
  | {
      kind: "api-error";
      error: ApiError;
    }
  | {
      kind: "transport-error";
      message: string;
    };

export interface OperationFailureClassification {
  message: string;
  sessionInvalid: boolean;
}

export interface SidecarStatusClassification {
  message: string | null;
  sessionInvalid: boolean;
}

export interface OperationFailureContext {
  commandDocumentId?: string | null;
  currentDocumentId?: string | null;
}

export interface CloseRequestState {
  dirty: boolean;
  commandInFlight: CommandName | null;
  promptInFlight: boolean;
}

export type ClosePromptReason = "dirty" | "command-in-flight" | "dirty-and-command-in-flight";

export type CloseRequestDecision =
  | { kind: "allow" }
  | { kind: "prompt"; reason: ClosePromptReason }
  | { kind: "prevent" };

export interface CloseConfirmationCopy {
  title: string;
  message: string;
}

const INVALID_SESSION_GUIDANCE =
  "The editor session is no longer valid. Restart the Tauri app and reopen the recipe.";

export function buildActionAvailability(state: ActionAvailabilityState): ActionAvailability {
  const commandIdle = state.commandInFlight === null;
  const backendReady = state.backendCompatible !== false;
  const sessionReady = state.documentSessionValid && backendReady && commandIdle;
  const documentReady = state.hasDocument && sessionReady;

  return {
    openRecipe: sessionReady,
    saveRecipe: documentReady && state.dirty,
    saveRecipeAs: documentReady,
    undo: documentReady && state.canUndo,
    redo: documentReady && state.canRedo,
    validate: documentReady,
    refreshYaml: documentReady,
    setAuthoredRoot: sessionReady,
    clearAuthoredRoot: sessionReady && (state.hasSelectedAuthoredRoot || state.hasDocumentAuthoredRoot),
    editDocument: documentReady,
  };
}

export function beginCommand(current: CommandName | null, next: CommandName): BeginCommandResult {
  if (current !== null) {
    return { started: false, commandInFlight: current };
  }
  return { started: true, commandInFlight: next };
}

export function decideCloseRequest(state: CloseRequestState): CloseRequestDecision {
  if (state.promptInFlight) {
    return { kind: "prevent" };
  }
  if (state.dirty && state.commandInFlight !== null) {
    return { kind: "prompt", reason: "dirty-and-command-in-flight" };
  }
  if (state.dirty) {
    return { kind: "prompt", reason: "dirty" };
  }
  if (state.commandInFlight !== null) {
    return { kind: "prompt", reason: "command-in-flight" };
  }
  return { kind: "allow" };
}

export function resolveClosePromptResult(confirmed: boolean): CloseRequestDecision {
  return confirmed ? { kind: "allow" } : { kind: "prevent" };
}

export function buildCloseConfirmationCopy(
  reason: ClosePromptReason,
  recipeId: string | null = null,
): CloseConfirmationCopy {
  const target = recipeId ?? "the open recipe";
  if (reason === "dirty-and-command-in-flight") {
    return {
      title: "Discard changes and close",
      message:
        `An editor operation is still in progress, and ${target} has unsaved changes. ` +
        "Close anyway? Unsaved changes will be lost, and the in-flight operation will not be cancelled.",
    };
  }
  if (reason === "command-in-flight") {
    return {
      title: "Close while operation is in progress",
      message:
        "An editor operation is still in progress. Close anyway? The in-flight operation will not be cancelled.",
    };
  }
  return {
    title: "Discard unsaved changes",
    message: `Discard unsaved changes to ${target} and close the editor?`,
  };
}

export function resolveOpenAttempt<TDocument>(
  currentDocument: TDocument | null,
  attempt: OpenAttempt<TDocument>,
): OpenAttemptResolution<TDocument> {
  if (attempt.kind === "opened") {
    return { document: attempt.document, replaced: true, sessionValid: true };
  }
  if (attempt.kind === "open-failed") {
    return { document: currentDocument, replaced: false, sessionValid: !attempt.sessionInvalid };
  }
  return { document: currentDocument, replaced: false, sessionValid: true };
}

export function resolveAuthoredRootSelectionAttempt<TDocument>(
  currentSelectedAuthoredRoot: string | null,
  currentDocument: TDocument | null,
  attempt: AuthoredRootSelectionAttempt<TDocument>,
): AuthoredRootSelectionResolution<TDocument> {
  if (attempt.kind === "no-document") {
    return {
      selectedAuthoredRoot: attempt.authoredRoot,
      document: currentDocument,
      changed: attempt.authoredRoot !== currentSelectedAuthoredRoot,
    };
  }
  if (attempt.kind === "updated") {
    return {
      selectedAuthoredRoot: attempt.authoredRoot,
      document: attempt.document,
      changed: true,
    };
  }
  return {
    selectedAuthoredRoot: currentSelectedAuthoredRoot,
    document: currentDocument,
    changed: false,
  };
}

export function classifyOperationFailure(
  failure: OperationFailure,
  fallback: string,
  context: OperationFailureContext = {},
): OperationFailureClassification {
  if (failure.kind === "api-error") {
    const message = `${fallback} ${failure.error.code}: ${failure.error.message}`;
    if (
      failure.error.code === "unknown_document" &&
      context.commandDocumentId !== null &&
      context.commandDocumentId !== undefined &&
      context.commandDocumentId === context.currentDocumentId
    ) {
      return {
        message: `${message} ${INVALID_SESSION_GUIDANCE}`,
        sessionInvalid: true,
      };
    }
    return {
      message,
      sessionInvalid: false,
    };
  }

  return {
    message: `${fallback} ${failure.message} ${INVALID_SESSION_GUIDANCE}`,
    sessionInvalid: true,
  };
}

export function classifySidecarStatus(status: SidecarStatusResult): SidecarStatusClassification {
  if (status.compatible === false || status.state === "incompatible") {
    return {
      message: invalidSessionMessage(status.lastError ?? "Backend sidecar is incompatible."),
      sessionInvalid: true,
    };
  }
  if (!status.running && (status.state === "exited" || status.state === "error")) {
    return {
      message: invalidSessionMessage(status.lastError ?? status.message ?? "Backend sidecar is no longer running."),
      sessionInvalid: true,
    };
  }
  return { message: null, sessionInvalid: false };
}

export function formatSidecarStatusLabel(status: SidecarStatusResult | null): string {
  if (status === null) {
    return "Sidecar: unknown";
  }
  if (status.compatible === false || status.state === "incompatible") {
    return "Sidecar: incompatible";
  }
  if (status.running) {
    if (status.compatible === true && typeof status.protocolVersion === "number") {
      return `Sidecar: compatible v${status.protocolVersion}${status.pid === null ? "" : ` pid ${status.pid}`}`;
    }
    return `Sidecar: running${status.pid === null ? "" : ` pid ${status.pid}`}`;
  }
  if (status.state === "notStarted") {
    return "Sidecar: not started";
  }
  if (status.state) {
    return `Sidecar: ${status.state}`;
  }
  return "Sidecar: stopped";
}

export function invalidSessionMessage(reason: string | null = null): string {
  return reason ? `${reason} ${INVALID_SESSION_GUIDANCE}` : INVALID_SESSION_GUIDANCE;
}

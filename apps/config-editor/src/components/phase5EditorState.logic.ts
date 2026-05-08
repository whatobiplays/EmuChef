import type { ApiError } from "../api/types.js";

export type CommandName = "openRecipe" | "saveRecipe" | "undo" | "redo" | "validate" | "refreshYaml" | "mutation";

export interface ActionAvailabilityState {
  hasDocument: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  commandInFlight: CommandName | null;
  documentSessionValid: boolean;
}

export interface ActionAvailability {
  openRecipe: boolean;
  saveRecipe: boolean;
  undo: boolean;
  redo: boolean;
  validate: boolean;
  refreshYaml: boolean;
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

export interface CloseRequestState {
  dirty: boolean;
  promptInFlight: boolean;
}

export type CloseRequestDecision = { kind: "allow" } | { kind: "prompt" } | { kind: "prevent" };

const INVALID_SESSION_GUIDANCE =
  "The editor session is no longer valid. Restart the Tauri app and reopen the recipe.";

export function buildActionAvailability(state: ActionAvailabilityState): ActionAvailability {
  const commandIdle = state.commandInFlight === null;
  const sessionReady = state.documentSessionValid && commandIdle;
  const documentReady = state.hasDocument && sessionReady;

  return {
    openRecipe: sessionReady,
    saveRecipe: documentReady && state.dirty,
    undo: documentReady && state.canUndo,
    redo: documentReady && state.canRedo,
    validate: documentReady,
    refreshYaml: documentReady,
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
  if (!state.dirty) {
    return { kind: "allow" };
  }
  if (state.promptInFlight) {
    return { kind: "prevent" };
  }
  return { kind: "prompt" };
}

export function resolveClosePromptResult(confirmed: boolean): CloseRequestDecision {
  return confirmed ? { kind: "allow" } : { kind: "prevent" };
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

export function classifyOperationFailure(
  failure: OperationFailure,
  fallback: string,
): OperationFailureClassification {
  if (failure.kind === "api-error") {
    return {
      message: `${fallback} ${failure.error.code}: ${failure.error.message}`,
      sessionInvalid: false,
    };
  }

  return {
    message: `${fallback} ${failure.message} ${INVALID_SESSION_GUIDANCE}`,
    sessionInvalid: true,
  };
}

export function invalidSessionMessage(reason: string | null = null): string {
  return reason ? `${reason} ${INVALID_SESSION_GUIDANCE}` : INVALID_SESSION_GUIDANCE;
}

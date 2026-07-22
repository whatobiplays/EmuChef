import type {
  ConfigurationDescription,
  DeviceFacts,
  DeviceMatch,
  ExecutionEvent,
  ExecutionEventBatch,
  AnyExecutionSnapshot,
  InputDescriptor,
  ReviewSummary,
  RecipeOption,
  ValidationDiagnostic,
} from "./types";

export type WorkflowStep = "connect" | "device" | "setup" | "recipes" | "inputs" | "review" | "execution";
export type ExecutionMode = "simulated" | "real";

export type ExecutionWorkflowState =
  | { kind: "idle" }
  | { kind: "starting"; generation: number; mode: ExecutionMode }
  | {
      kind: "active" | "terminal";
      generation: number;
      snapshot: AnyExecutionSnapshot;
      mode: ExecutionMode;
      events: ExecutionEvent[];
      eventCursor: number;
      cancellationRequested: boolean;
    }
  | {
      kind: "unavailable";
      generation: number;
      executionHandle: string;
      message: string;
      mode: ExecutionMode;
    };

export interface WorkflowState {
  step: WorkflowStep;
  deviceHandle: string | null;
  facts: DeviceFacts | null;
  match: DeviceMatch | null;
  devicePlan: string | null;
  selectedRecipes: string[] | null;
  bindings: Record<string, unknown>;
  description: ConfigurationDescription | null;
  descriptionDirty: boolean;
  review: ReviewSummary | null;
  reviewStale: boolean;
  requestGeneration: number;
  executionGeneration: number;
  execution: ExecutionWorkflowState;
  repairIntent: boolean;
  portableIntentDirty: boolean;
  savedIntentLoaded: boolean;
  requiredReentryBindings: string[];
  reconnectDeviceHandle: string | null;
  unsupportedAcknowledged: boolean;
}

export const initialWorkflowState: WorkflowState = {
  step: "connect",
  deviceHandle: null,
  facts: null,
  match: null,
  devicePlan: null,
  selectedRecipes: null,
  bindings: {},
  description: null,
  descriptionDirty: false,
  review: null,
  reviewStale: false,
  requestGeneration: 0,
  executionGeneration: 0,
  execution: { kind: "idle" },
  repairIntent: false,
  portableIntentDirty: false,
  savedIntentLoaded: false,
  requiredReentryBindings: [],
  reconnectDeviceHandle: null,
  unsupportedAcknowledged: false,
};

export type WorkflowAction =
  | { type: "select-device"; deviceHandle: string; preserveIntent?: boolean }
  | { type: "device-probed"; facts: DeviceFacts; match: DeviceMatch }
  | { type: "select-plan"; devicePlan: string; recipeSelection?: "preserve" | "defaults" | "blank" }
  | { type: "description"; description: ConfigurationDescription; generation: number }
  | { type: "set-recipes"; selectedRecipes: string[] }
  | { type: "continue-to-inputs" }
  | { type: "set-binding"; key: string; value: unknown }
  | { type: "remove-binding"; key: string }
  | { type: "review"; review: ReviewSummary }
  | { type: "execution-starting"; generation: number; mode?: ExecutionMode }
  | { type: "execution-started"; generation: number; snapshot: AnyExecutionSnapshot }
  | { type: "execution-start-failed"; generation: number }
  | { type: "execution-snapshot"; generation: number; snapshot: AnyExecutionSnapshot }
  | { type: "execution-events"; generation: number; batch: ExecutionEventBatch }
  | { type: "execution-cancellation-requested"; generation: number }
  | { type: "execution-unavailable"; generation: number; executionHandle: string; message: string }
  | { type: "prepare-repair" }
  | {
      type: "load-portable-intent";
      devicePlan: string;
      selectedRecipes: string[];
      bindings: Record<string, unknown>;
      dirty: boolean;
      requiredReentryBindings?: string[];
    }
  | { type: "portable-intent-saved" }
  | {
      type: "repair-ready";
      facts: DeviceFacts;
      match: DeviceMatch;
      devicePlan: string;
      description: ConfigurationDescription;
      selectedRecipes: string[];
      bindings: Record<string, unknown>;
    }
  | { type: "return-to-review" }
  | { type: "back" }
  | {
      type: "device-disappeared";
      bindings?: Record<string, unknown>;
      requiredReentryBindings?: string[];
    }
  | { type: "set-unsupported-acknowledgment"; acknowledged: boolean }
  | { type: "continue-unsupported" }
  | { type: "infrastructure-invalidated" }
  | { type: "runtime-invalidated" };

const previousStep: Record<WorkflowStep, WorkflowStep> = {
  connect: "connect",
  device: "connect",
  setup: "connect",
  recipes: "setup",
  inputs: "recipes",
  review: "inputs",
  execution: "review",
};

function activeRecipeIdsForSelection(
  description: ConfigurationDescription | null,
  selectedRecipes: string[],
): Set<string> {
  const active = new Set<string>();
  const dependencies = new Map(
    (description?.recipeOptions ?? []).map((recipe) => [recipe.id, recipe.recipeDependencies ?? []]),
  );
  const pending = [...selectedRecipes];
  while (pending.length > 0) {
    const recipeId = pending.pop()!;
    if (active.has(recipeId)) continue;
    active.add(recipeId);
    pending.push(...(dependencies.get(recipeId) ?? []));
  }
  return active;
}

function bindingsForRecipeSelection(
  description: ConfigurationDescription | null,
  selectedRecipes: string[],
  bindings: Record<string, unknown>,
): Record<string, unknown> {
  if (!description) return bindings;
  const activeRecipes = activeRecipeIdsForSelection(description, selectedRecipes);
  const inputs = new Map(description.inputs.map((input) => [input.key, input]));
  return Object.fromEntries(
    Object.entries(bindings).filter(([key]) => {
      const input = inputs.get(key);
      return input !== undefined && activeRecipes.has(input.recipeId);
    }),
  );
}

export function workflowReducer(state: WorkflowState, action: WorkflowAction): WorkflowState {
  switch (action.type) {
    case "select-device":
      if (action.preserveIntent === true || (
        action.preserveIntent === undefined
        && (
          (state.reconnectDeviceHandle !== null && state.reconnectDeviceHandle === action.deviceHandle)
          || state.repairIntent
          || state.savedIntentLoaded
        )
      )) {
        return {
          ...state,
          step: "device",
          deviceHandle: action.deviceHandle,
          facts: null,
          match: null,
          description: null,
          review: null,
          execution: { kind: "idle" },
          unsupportedAcknowledged: false,
          requestGeneration: state.requestGeneration + 1,
        };
      }
      return {
        ...initialWorkflowState,
        step: "device",
        deviceHandle: action.deviceHandle,
        requestGeneration: state.requestGeneration + 1,
      };
    case "device-probed":
      if (state.deviceHandle !== action.facts.deviceHandle) return state;
      return {
        ...state,
        step: deviceIsUnsupported(action.match) ? "device" : "setup",
        facts: action.facts,
        match: action.match,
        devicePlan: deviceIsUnsupported(action.match)
          ? null
          : state.savedIntentLoaded
          ? state.devicePlan
          : state.repairIntent && planStillAvailable(state.devicePlan, action.match)
            ? state.devicePlan
            : action.match.recommendedPlanId,
        reconnectDeviceHandle: null,
        unsupportedAcknowledged: false,
      };
    case "set-unsupported-acknowledgment":
      if (!state.match || !deviceIsUnsupported(state.match)) return state;
      return { ...state, unsupportedAcknowledged: action.acknowledged };
    case "continue-unsupported":
      if (!state.match || !deviceIsUnsupported(state.match) || state.match.safeGenericPlans.length === 0) {
        return state;
      }
      if (!state.unsupportedAcknowledged) return state;
      return { ...state, step: "setup", devicePlan: null };
    case "select-plan":
      return {
        ...state,
        devicePlan: action.devicePlan,
        selectedRecipes: action.recipeSelection === "defaults"
          ? null
          : action.recipeSelection === "blank"
            ? []
            : state.selectedRecipes,
        description: null,
        descriptionDirty: true,
        review: null,
        repairIntent: false,
        portableIntentDirty: true,
        requestGeneration: state.requestGeneration + 1,
      };
    case "description":
      if (action.generation !== state.requestGeneration) return state;
      return {
        ...state,
        step: state.step === "inputs" ? "inputs" : "recipes",
        description: action.description,
        descriptionDirty: false,
        selectedRecipes: action.description.selectedRecipes,
        review: null,
        repairIntent: false,
      };
    case "set-recipes": {
      const bindings = bindingsForRecipeSelection(
        state.description,
        action.selectedRecipes,
        state.bindings,
      );
      const retainedBindings = new Set(Object.keys(bindings));
      return {
        ...state,
        selectedRecipes: action.selectedRecipes,
        bindings,
        requiredReentryBindings: state.requiredReentryBindings.filter((key) => retainedBindings.has(key)),
        descriptionDirty: true,
        review: null,
        portableIntentDirty: true,
        requestGeneration: state.requestGeneration + 1,
      };
    }
    case "continue-to-inputs":
      if (!state.description || state.descriptionDirty || state.description.selectedRecipes.length === 0) {
        return state;
      }
      return { ...state, step: "inputs", review: null };
    case "set-binding":
      return {
        ...state,
        bindings: { ...state.bindings, [action.key]: action.value },
        description: state.description
          ? {
              ...state.description,
              inputs: state.description.inputs.map((input) =>
                input.key === action.key ? { ...input, value: action.value } : input,
              ),
            }
          : null,
        descriptionDirty: true,
        review: null,
        portableIntentDirty: true,
        requiredReentryBindings: state.requiredReentryBindings.filter((key) => key !== action.key),
        requestGeneration: state.requestGeneration + 1,
      };
    case "remove-binding": {
      const bindings = { ...state.bindings };
      delete bindings[action.key];
      return {
        ...state,
        bindings,
        description: state.description
          ? {
              ...state.description,
              inputs: state.description.inputs.map((input) =>
                input.key === action.key ? { ...input, value: null } : input,
              ),
            }
          : null,
        descriptionDirty: true,
        review: null,
        portableIntentDirty: true,
        requiredReentryBindings: state.requiredReentryBindings.filter((key) => key !== action.key),
        requestGeneration: state.requestGeneration + 1,
      };
    }
    case "review":
      return { ...state, step: "review", review: action.review, reviewStale: false };
    case "execution-starting":
      if (!state.review || state.reviewStale || action.generation <= state.executionGeneration) return state;
      return {
        ...state,
        executionGeneration: action.generation,
        execution: { kind: "starting", generation: action.generation, mode: action.mode ?? "simulated" },
      };
    case "execution-started":
      if (!executionGenerationMatches(state, action.generation, "starting")) return state;
      return {
        ...state,
        step: "execution",
        execution: {
          kind: action.snapshot.terminal ? "terminal" : "active",
          mode: action.snapshot.simulated ? "simulated" : "real",
          generation: action.generation,
          snapshot: action.snapshot,
          events: [],
          eventCursor: 0,
          cancellationRequested: false,
        },
      };
    case "execution-start-failed":
      if (!executionGenerationMatches(state, action.generation, "starting")) return state;
      return { ...state, step: "review", execution: { kind: "idle" } };
    case "execution-snapshot": {
      if (!executionResponseMatches(state, action.generation, action.snapshot.executionHandle)) return state;
      const current = state.execution;
      if (current.kind !== "active" && current.kind !== "terminal") return state;
      if (action.snapshot.latestSequence < current.snapshot.latestSequence) return state;
      if (current.kind === "terminal" && !action.snapshot.terminal) return state;
      return {
        ...state,
        execution: {
          ...current,
          kind: action.snapshot.terminal ? "terminal" : "active",
          snapshot: action.snapshot,
        },
      };
    }
    case "execution-events": {
      if (!executionResponseMatches(state, action.generation, action.batch.executionHandle)) return state;
      const current = state.execution;
      if (current.kind !== "active" && current.kind !== "terminal") return state;
      const merged = mergeExecutionEvents(current.events, action.batch.events, current.eventCursor);
      return {
        ...state,
        execution: {
          ...current,
          events: merged.events,
          eventCursor: merged.cursor,
        },
      };
    }
    case "execution-cancellation-requested": {
      if (!executionGenerationMatches(state, action.generation, "active")) return state;
      const current = state.execution;
      if (current.kind !== "active") return state;
      return { ...state, execution: { ...current, cancellationRequested: true } };
    }
    case "execution-unavailable": {
      if (!executionResponseMatches(state, action.generation, action.executionHandle)) return state;
      const current = state.execution;
      if (current.kind !== "active" && current.kind !== "terminal") return state;
      return {
        ...state,
        step: "execution",
        review: current.mode === "real" ? null : state.review,
        execution: {
          kind: "unavailable",
          generation: action.generation,
          executionHandle: action.executionHandle,
          message: action.message,
          mode: current.mode,
        },
      };
    }
    case "prepare-repair":
      return {
        ...state,
        step: state.deviceHandle ? "device" : "connect",
        facts: null,
        match: null,
        description: null,
        descriptionDirty: true,
        review: null,
        requestGeneration: state.requestGeneration + 1,
        executionGeneration: state.executionGeneration + 1,
        execution: { kind: "idle" },
        repairIntent: true,
      };
    case "load-portable-intent":
      return {
        ...initialWorkflowState,
        step: "connect",
        devicePlan: action.devicePlan,
        selectedRecipes: action.selectedRecipes,
        bindings: action.bindings,
        descriptionDirty: true,
        portableIntentDirty: action.dirty,
        savedIntentLoaded: true,
        requiredReentryBindings: action.requiredReentryBindings ?? [],
        requestGeneration: state.requestGeneration + 1,
        executionGeneration: state.executionGeneration + 1,
      };
    case "portable-intent-saved":
      return { ...state, portableIntentDirty: false };
    case "repair-ready":
      return {
        ...state,
        step: "inputs",
        deviceHandle: action.facts.deviceHandle,
        facts: action.facts,
        match: action.match,
        devicePlan: action.devicePlan,
        selectedRecipes: action.selectedRecipes,
        bindings: action.bindings,
        description: action.description,
        descriptionDirty: false,
        review: null,
        requestGeneration: state.requestGeneration + 1,
        execution: { kind: "idle" },
        repairIntent: false,
        portableIntentDirty: true,
      };
    case "return-to-review": {
      if (!state.review) return state;
      const reviewStale = state.execution.kind === "unavailable"
        || (state.execution.kind === "terminal"
          && state.execution.snapshot.status !== "succeeded"
          && state.execution.snapshot.status !== "succeeded_with_warnings");
      return {
        ...state,
        step: "review",
        execution: { kind: "idle" },
        reviewStale: state.reviewStale || reviewStale,
      };
    }
    case "back":
      return {
        ...state,
        step: state.step === "setup" && state.match && deviceIsUnsupported(state.match)
          ? "device"
          : previousStep[state.step],
        review: null,
        unsupportedAcknowledged: state.step === "setup" ? false : state.unsupportedAcknowledged,
      };
    case "device-disappeared":
      if (state.step === "execution" || state.execution.kind === "starting") return state;
      return {
        ...state,
        step: "connect",
        reconnectDeviceHandle: state.deviceHandle,
        deviceHandle: null,
        facts: null,
        match: null,
        description: null,
        descriptionDirty: true,
        review: null,
        executionGeneration: state.executionGeneration + 1,
        execution: { kind: "idle" },
        repairIntent: true,
        bindings: action.bindings ?? state.bindings,
        requiredReentryBindings: action.requiredReentryBindings ?? state.requiredReentryBindings,
        unsupportedAcknowledged: false,
        requestGeneration: state.requestGeneration + 1,
      };
    case "infrastructure-invalidated":
      return {
        ...state,
        step: "connect",
        deviceHandle: null,
        facts: null,
        match: null,
        description: null,
        descriptionDirty: true,
        review: null,
        executionGeneration: state.executionGeneration + 1,
        execution: { kind: "idle" },
        repairIntent: false,
        reconnectDeviceHandle: state.deviceHandle,
        unsupportedAcknowledged: false,
        requestGeneration: state.requestGeneration + 1,
      };
    case "runtime-invalidated":
      return { ...initialWorkflowState, requestGeneration: state.requestGeneration + 1 };
  }
}

function planStillAvailable(planId: string | null, match: DeviceMatch): boolean {
  if (!planId) return false;
  return [...match.candidates, ...match.safeGenericPlans].some((plan) => plan.planId === planId);
}

/** Preserve only bindings whose current input contract still matches the reviewed workflow. */
export function filterRepairBindings(
  previous: ConfigurationDescription | null,
  current: ConfigurationDescription,
  bindings: Record<string, unknown>,
): Record<string, unknown> {
  if (!previous) return {};
  const previousInputs = new Map(previous.inputs.map((input) => [input.key, input]));
  const result: Record<string, unknown> = {};
  for (const input of current.inputs) {
    const old = previousInputs.get(input.key);
    if (
      old
      && old.type === input.type
      && Boolean(old.multiple) === Boolean(input.multiple)
      && old.pathKind === input.pathKind
      && Object.hasOwn(bindings, input.key)
    ) {
      result[input.key] = bindings[input.key];
    }
  }
  return result;
}

function executionGenerationMatches(
  state: WorkflowState,
  generation: number,
  kind: ExecutionWorkflowState["kind"],
): boolean {
  return state.execution.kind === kind && state.executionGeneration === generation;
}

function executionResponseMatches(
  state: WorkflowState,
  generation: number,
  executionHandle: string,
): boolean {
  if (state.executionGeneration !== generation) return false;
  if (state.execution.kind !== "active" && state.execution.kind !== "terminal") return false;
  return state.execution.snapshot.executionHandle === executionHandle;
}

/** Merge presentation-only events once while preserving monotonic sequence order. */
export function mergeExecutionEvents(
  current: ExecutionEvent[],
  incoming: ExecutionEvent[],
  cursor: number,
): { events: ExecutionEvent[]; cursor: number } {
  const bySequence = new Map(current.map((event) => [event.sequence, event]));
  let nextCursor = cursor;
  for (const event of incoming) {
    if (!Number.isSafeInteger(event.sequence) || event.sequence <= cursor) continue;
    bySequence.set(event.sequence, event);
    nextCursor = Math.max(nextCursor, event.sequence);
  }
  return {
    events: [...bySequence.values()].sort((left, right) => left.sequence - right.sequence),
    cursor: nextCursor,
  };
}

export function reviewReady(state: WorkflowState): boolean {
  if (!state.deviceHandle || !state.devicePlan || !state.description) return false;
  if (state.description.selectedRecipes.length === 0) return false;
  if (state.descriptionDirty) return false;
  if (state.requiredReentryBindings.length > 0) return false;
  if (state.description.diagnostics.some((item) => item.severity === "error")) return false;
  return !state.description.inputs.some(
    (input) => input.required && missingRequiredValue(input.value),
  );
}

export function deviceIsUnsupported(match: DeviceMatch): boolean {
  return match.confidence === "none";
}

/**
 * Projects portable binding values using only the current backend-authored
 * input sensitivity contract. Unknown keys and sensitive values fail closed.
 */
export function portableBindingsForTransition(
  description: ConfigurationDescription | null,
  bindings: Record<string, unknown>,
): {
  bindings: Record<string, unknown>;
  requiredReentryBindings: string[];
  requiredReentryLabels: string[];
} {
  const projected: Record<string, unknown> = {};
  const requiredReentryBindings: string[] = [];
  const requiredReentryLabels: string[] = [];
  const inputs = new Map((description?.inputs ?? []).map((input) => [input.key, input]));
  for (const [key, value] of Object.entries(bindings)) {
    const input = inputs.get(key);
    if (input?.sensitive === false) {
      projected[key] = value;
      continue;
    }
    requiredReentryBindings.push(key);
    if (input?.label && !requiredReentryLabels.includes(input.label)) {
      requiredReentryLabels.push(input.label);
    }
  }
  requiredReentryBindings.sort();
  requiredReentryLabels.sort();
  return { bindings: projected, requiredReentryBindings, requiredReentryLabels };
}

/** Returns whether the server-authoritative recipe option is immutable in the UI. */
export function recipeSelectionDisabled(recipe: RecipeOption): boolean {
  return recipe.dependencyRequired || !recipe.available;
}

/** Applies a user selection without allowing dependencies or unavailable recipes to be changed. */
export function updateRecipeSelection(
  selectedRecipes: string[],
  recipe: RecipeOption,
  selected: boolean,
): string[] {
  if (recipeSelectionDisabled(recipe)) return selectedRecipes;
  const next = new Set(selectedRecipes);
  if (selected) next.add(recipe.id);
  else next.delete(recipe.id);
  return [...next];
}

/** Returns deterministic, unique diagnostics rendered beneath one input. */
export function inputDiagnosticsForDisplay(input: InputDescriptor): ValidationDiagnostic[] {
  return uniqueDiagnostics(input.diagnostics, input.key);
}

/**
 * Returns only page-level diagnostics not already represented beneath an input.
 * Key plus code is authoritative when a key is available; code plus message is
 * used only when the global diagnostic has no binding key.
 */
export function pageDiagnosticsForDisplay(
  description: ConfigurationDescription,
): ValidationDiagnostic[] {
  const inputStrongIdentities = new Set<string>();
  const inputFallbackIdentities = new Set<string>();
  for (const input of description.inputs) {
    for (const diagnostic of inputDiagnosticsForDisplay(input)) {
      inputStrongIdentities.add(strongDiagnosticIdentity(diagnostic, input.key));
      inputFallbackIdentities.add(fallbackDiagnosticIdentity(diagnostic));
    }
  }

  const pageIdentities = new Set<string>();
  return description.diagnostics.filter((diagnostic) => {
    const key = diagnostic.key?.trim();
    const identity = key
      ? strongDiagnosticIdentity(diagnostic, key)
      : fallbackDiagnosticIdentity(diagnostic);
    const representedByInput = key
      ? inputStrongIdentities.has(identity)
      : inputFallbackIdentities.has(identity);
    if (representedByInput || pageIdentities.has(identity)) return false;
    pageIdentities.add(identity);
    return true;
  });
}

function uniqueDiagnostics(
  diagnostics: ValidationDiagnostic[],
  inputKey?: string,
): ValidationDiagnostic[] {
  const identities = new Set<string>();
  return diagnostics.filter((diagnostic) => {
    const identity = strongDiagnosticIdentity(diagnostic, diagnostic.key ?? inputKey);
    if (identities.has(identity)) return false;
    identities.add(identity);
    return true;
  });
}

function strongDiagnosticIdentity(diagnostic: ValidationDiagnostic, key?: string | null): string {
  const normalizedKey = key?.trim();
  return normalizedKey
    ? JSON.stringify(["binding-code", normalizedKey, diagnostic.code])
    : fallbackDiagnosticIdentity(diagnostic);
}

function fallbackDiagnosticIdentity(diagnostic: ValidationDiagnostic): string {
  return JSON.stringify(["code-message", diagnostic.code, diagnostic.message]);
}

interface BusyAction<T> {
  setBusy: (busy: boolean) => void;
  action: () => Promise<T>;
  onSuccess: (value: T) => void | Promise<void>;
  onError: (error: unknown) => void | Promise<void>;
}

/**
 * Runs an IPC-backed UI action while guaranteeing that its busy state clears.
 * Cleanup also runs when either the action or its error recovery rejects.
 */
export async function runBusyAction<T>({
  setBusy,
  action,
  onSuccess,
  onError,
}: BusyAction<T>): Promise<void> {
  setBusy(true);
  try {
    await onSuccess(await action());
  } catch (error) {
    await onError(error);
  } finally {
    setBusy(false);
  }
}

function missingRequiredValue(value: unknown): boolean {
  return value === null || value === undefined || value === "" || (Array.isArray(value) && value.length === 0);
}

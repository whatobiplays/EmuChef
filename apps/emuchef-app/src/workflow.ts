import type {
  ConfigurationDescription,
  DeviceFacts,
  DeviceMatch,
  InputDescriptor,
  ReviewSummary,
  RecipeOption,
  ValidationDiagnostic,
} from "./types";

export type WorkflowStep = "connect" | "device" | "setup" | "inputs" | "review";

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
  requestGeneration: number;
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
  requestGeneration: 0,
};

export type WorkflowAction =
  | { type: "select-device"; deviceHandle: string }
  | { type: "device-probed"; facts: DeviceFacts; match: DeviceMatch }
  | { type: "select-plan"; devicePlan: string }
  | { type: "description"; description: ConfigurationDescription; generation: number }
  | { type: "set-recipes"; selectedRecipes: string[] }
  | { type: "set-binding"; key: string; value: unknown }
  | { type: "review"; review: ReviewSummary }
  | { type: "back" }
  | { type: "device-disappeared" }
  | { type: "runtime-invalidated" };

const previousStep: Record<WorkflowStep, WorkflowStep> = {
  connect: "connect",
  device: "connect",
  setup: "connect",
  inputs: "setup",
  review: "inputs",
};

export function workflowReducer(state: WorkflowState, action: WorkflowAction): WorkflowState {
  switch (action.type) {
    case "select-device":
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
        step: "setup",
        facts: action.facts,
        match: action.match,
        devicePlan: action.match.recommendedPlanId,
      };
    case "select-plan":
      return {
        ...state,
        devicePlan: action.devicePlan,
        description: null,
        descriptionDirty: true,
        review: null,
        requestGeneration: state.requestGeneration + 1,
      };
    case "description":
      if (action.generation !== state.requestGeneration) return state;
      return {
        ...state,
        step: action.description.inputs.length > 0 ? "inputs" : "inputs",
        description: action.description,
        descriptionDirty: false,
        selectedRecipes: action.description.selectedRecipes,
        review: null,
      };
    case "set-recipes":
      return {
        ...state,
        selectedRecipes: action.selectedRecipes,
        descriptionDirty: true,
        review: null,
        requestGeneration: state.requestGeneration + 1,
      };
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
        requestGeneration: state.requestGeneration + 1,
      };
    case "review":
      return { ...state, step: "review", review: action.review };
    case "back":
      return { ...state, step: previousStep[state.step], review: null };
    case "device-disappeared":
    case "runtime-invalidated":
      return { ...initialWorkflowState, requestGeneration: state.requestGeneration + 1 };
  }
}

export function reviewReady(state: WorkflowState): boolean {
  if (!state.deviceHandle || !state.devicePlan || !state.description) return false;
  if (state.descriptionDirty) return false;
  if (state.description.diagnostics.some((item) => item.severity === "error")) return false;
  return !state.description.inputs.some(
    (input) => input.required && missingRequiredValue(input.value),
  );
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

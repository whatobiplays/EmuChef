export interface StepDependencySource {
  dependencies?: string[] | null;
}

export interface StepDependencySummary {
  id: string;
  name: string;
  type: string;
}

export interface DependencyEntry {
  id: string;
  step: StepDependencySummary | null;
  missing: boolean;
}

export type AddDependencyResult =
  | { ok: true; dependencies: string[] }
  | { ok: false; reason: "no-step" | "no-dependency" | "self-dependency" | "duplicate-dependency" };

/**
 * Reads authored dependency ids for the editor without normalizing the document.
 * Missing/null values are rendered as empty lists only until the user submits an
 * explicit add or remove command through the sidecar.
 */
export function stepDependencyIds(step: StepDependencySource | null | undefined): string[] {
  return Array.isArray(step?.dependencies) ? step.dependencies : [];
}

export function dependencyEntries(
  dependencyIds: readonly string[],
  steps: readonly StepDependencySummary[],
): DependencyEntry[] {
  const stepsById = new Map(steps.map((step) => [step.id, step]));
  return dependencyIds.map((id) => {
    const step = stepsById.get(id) ?? null;
    return {
      id,
      step,
      missing: step === null,
    };
  });
}

export function selectableDependencySteps(
  steps: readonly StepDependencySummary[],
  selectedStepId: string | null | undefined,
  dependencyIds: readonly string[],
): StepDependencySummary[] {
  const existingDependencies = new Set(dependencyIds);
  return steps.filter((step) => step.id !== selectedStepId && !existingDependencies.has(step.id));
}

export function buildAddDependencyList(
  dependencyIds: readonly string[],
  selectedStepId: string | null | undefined,
  selectedDependencyId: string | null | undefined,
): AddDependencyResult {
  if (!selectedStepId) {
    return { ok: false, reason: "no-step" };
  }
  if (!selectedDependencyId) {
    return { ok: false, reason: "no-dependency" };
  }
  if (selectedDependencyId === selectedStepId) {
    return { ok: false, reason: "self-dependency" };
  }
  if (dependencyIds.includes(selectedDependencyId)) {
    return { ok: false, reason: "duplicate-dependency" };
  }
  return { ok: true, dependencies: [...dependencyIds, selectedDependencyId] };
}

export function buildRemoveDependencyList(dependencyIds: readonly string[], dependencyId: string): string[] {
  return dependencyIds.filter((candidate) => candidate !== dependencyId);
}

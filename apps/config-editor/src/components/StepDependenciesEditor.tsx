import { useEffect, useMemo, useState } from "react";

import type { StepDto } from "../api/types";
import {
  buildAddDependencyList,
  buildRemoveDependencyList,
  dependencyEntries,
  selectableDependencySteps,
  stepDependencyIds,
} from "./stepDependencies.logic";

interface StepDependenciesEditorProps {
  readOnly?: boolean;
  step: StepDto;
  steps: StepDto[];
  onUpdateDependencies: (dependencies: string[]) => Promise<boolean>;
}

const validationMessages = {
  "no-step": "Select a step before editing dependencies.",
  "no-dependency": "Select a dependency to add.",
  "self-dependency": "A step cannot depend on itself.",
  "duplicate-dependency": "That dependency is already listed.",
};

export function StepDependenciesEditor({ readOnly = false, step, steps, onUpdateDependencies }: StepDependenciesEditorProps) {
  const dependencyIds = stepDependencyIds(step);
  const entries = useMemo(() => dependencyEntries(dependencyIds, steps), [dependencyIds, steps]);
  const selectableSteps = useMemo(
    () => selectableDependencySteps(steps, step.id, dependencyIds),
    [dependencyIds, step.id, steps],
  );
  const [selectedDependencyId, setSelectedDependencyId] = useState("");
  const [validationMessage, setValidationMessage] = useState<string | null>(null);

  useEffect(() => {
    if (selectedDependencyId && !selectableSteps.some((candidate) => candidate.id === selectedDependencyId)) {
      setSelectedDependencyId("");
    }
  }, [selectableSteps, selectedDependencyId]);

  async function addDependency() {
    const result = buildAddDependencyList(dependencyIds, step.id, selectedDependencyId || null);
    if (!result.ok) {
      setValidationMessage(validationMessages[result.reason]);
      return;
    }

    const ok = await onUpdateDependencies(result.dependencies);
    if (ok) {
      setSelectedDependencyId("");
      setValidationMessage(null);
    }
  }

  async function removeDependency(dependencyId: string) {
    const ok = await onUpdateDependencies(buildRemoveDependencyList(dependencyIds, dependencyId));
    if (ok) {
      setValidationMessage(null);
    }
  }

  return (
    <section className="grid gap-3" aria-labelledby="step-dependencies-heading">
      <div>
        <h3 id="step-dependencies-heading" className="text-xs font-semibold uppercase tracking-wide text-slate-500">
          Dependencies
        </h3>
        <p className="mt-1 text-xs text-slate-500">
          Step ids listed here are prerequisites. The planner determines final execution order.
        </p>
      </div>

      {entries.length === 0 ? (
        <p className="rounded border border-dashed border-slate-300 px-3 py-2 text-sm text-slate-500">
          No dependencies
        </p>
      ) : (
        <ul className="grid gap-2">
          {entries.map((entry) => (
            <li
              className="flex min-w-0 items-center justify-between gap-3 rounded border border-slate-200 bg-slate-50 px-3 py-2"
              key={entry.id}
            >
              <div className="min-w-0">
                <div className="truncate font-mono text-sm text-slate-950">{entry.id}</div>
                {entry.step ? (
                  <div className="truncate text-xs text-slate-500">
                    {entry.step.name || entry.step.id} ({entry.step.type})
                  </div>
                ) : (
                  <div className="text-xs font-medium text-amber-700">Missing step</div>
                )}
              </div>
              <button
                className="shrink-0 rounded border border-slate-300 bg-white px-2 py-1 text-xs text-slate-700 hover:bg-slate-100 disabled:opacity-40"
                disabled={readOnly}
                type="button"
                onClick={() => void removeDependency(entry.id)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="grid gap-2 border-t border-slate-200 pt-3">
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Add Dependency</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm disabled:bg-slate-100 disabled:text-slate-500"
            disabled={readOnly || selectableSteps.length === 0}
            value={selectedDependencyId}
            onChange={(event) => {
              setSelectedDependencyId(event.target.value);
              setValidationMessage(null);
            }}
          >
            <option value="">
              {selectableSteps.length === 0 ? "No available step ids" : "Select step id..."}
            </option>
            {selectableSteps.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.id} - {candidate.name || candidate.id} ({candidate.type})
              </option>
            ))}
          </select>
        </label>
        <div className="flex items-center gap-3">
          <button
            className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500"
            disabled={readOnly || selectableSteps.length === 0}
            type="button"
            onClick={() => void addDependency()}
          >
            Add
          </button>
          {validationMessage ? <p className="text-sm font-medium text-red-700">{validationMessage}</p> : null}
        </div>
      </div>
    </section>
  );
}

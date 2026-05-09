import { useEffect, useMemo, useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { DiagnosticDto, RecipeDocumentDto, RecipeDto, StepDto, StepSpecDto } from "../api/types";
import { AdvancedStepInternalsEditor, type AdvancedCommandResult } from "./AdvancedStepInternalsEditor";
import { EditableTextField } from "./EditableTextField";
import { ResizableEditorLayout } from "./ResizableEditorLayout";
import { StepDependenciesEditor } from "./StepDependenciesEditor";
import { StepParamsEditor } from "./StepParamsEditor";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

interface StepsEditorProps {
  document: RecipeDocumentDto;
  stepSpecs: StepSpecDto[];
  promptForId: (title: string, initialValue: string) => Promise<string | null>;
  confirmAction: (
    title: string,
    message: string,
    options?: { confirmLabel?: string; destructive?: boolean },
  ) => Promise<boolean>;
  readOnly?: boolean;
  onCommand: (command: EditorCommand) => Promise<boolean>;
  onAdvancedCommand: (command: EditorCommand) => Promise<AdvancedCommandResult>;
}

interface AddStepDraft {
  stepId: string;
  stepType: string;
  name: string;
}

export function StepsEditor({
  document,
  stepSpecs,
  promptForId,
  confirmAction,
  readOnly = false,
  onCommand,
  onAdvancedCommand,
}: StepsEditorProps) {
  const steps = document.recipe.steps;
  const stepIds = useMemo(() => steps.map((step) => step.id), [steps]);
  const diagnosticCounts = useMemo(() => stepDiagnosticCounts(document.diagnostics), [document.diagnostics]);
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const [addDraft, setAddDraft] = useState<AddStepDraft | null>(null);

  const selectedId = selectedStepId && stepIds.includes(selectedStepId) ? selectedStepId : stepIds[0] ?? null;
  const selectedStep = selectedId ? steps.find((step) => step.id === selectedId) ?? null : null;

  useEffect(() => {
    if (selectedStepId !== null && !stepIds.includes(selectedStepId)) {
      setSelectedStepId(stepIds[0] ?? null);
    }
  }, [selectedStepId, stepIds]);

  function openAddStep() {
    const firstType = stepSpecs[0]?.type ?? "";
    setAddDraft({ stepId: "new_step", stepType: firstType, name: "" });
  }

  async function submitAddStep(draft: AddStepDraft) {
    const stepId = draft.stepId.trim();
    const name = draft.name.trim() || stepId;
    const selectedIndex = selectedId ? steps.findIndex((step) => step.id === selectedId) : -1;
    const command: EditorCommand = {
      type: "AddStep",
      stepId,
      stepType: draft.stepType,
      name,
      ...(selectedIndex >= 0 ? { index: selectedIndex + 1 } : {}),
    };
    const ok = await onCommand(command);
    if (ok) {
      setSelectedStepId(stepId);
      setAddDraft(null);
    } else {
      setAddDraft(draft);
    }
  }

  async function duplicateStep(step: StepDto) {
    let attempted = `${step.id}_copy`;
    while (true) {
      const newStepId = await promptForId(`Duplicate step ${step.id}`, attempted);
      if (newStepId === null) {
        return;
      }
      const ok = await onCommand({ type: "DuplicateStep", sourceStepId: step.id, newStepId });
      if (ok) {
        setSelectedStepId(newStepId);
        return;
      }
      attempted = newStepId;
    }
  }

  async function deleteStep(step: StepDto) {
    const confirmed = await confirmAction("Delete step", `Delete step ${step.id}?`, {
      confirmLabel: "Delete",
      destructive: true,
    });
    if (!confirmed) {
      return;
    }
    const currentIndex = steps.findIndex((candidate) => candidate.id === step.id);
    const remainingIds = steps.filter((candidate) => candidate.id !== step.id).map((candidate) => candidate.id);
    const nextId = remainingIds[Math.min(currentIndex, remainingIds.length - 1)] ?? null;
    const ok = await onCommand({ type: "DeleteStep", stepId: step.id });
    if (ok) {
      setSelectedStepId(nextId);
    }
  }

  async function moveStep(step: StepDto, toIndex: number) {
    const ok = await onCommand({ type: "ReorderStep", stepId: step.id, toIndex });
    if (ok) {
      setSelectedStepId(step.id);
    }
  }

  return (
    <ResizableEditorLayout
      defaultSidebarWidth={352}
      maxSidebarWidth={560}
      minSidebarWidth={288}
      resizeLabel="Resize steps list"
      sidebarBody={
        <div className="space-y-2">
          {steps.length === 0 ? <p className="text-sm text-slate-500">No steps</p> : null}
          {steps.map((step, index) => (
            <StepListRow
              diagnosticCount={diagnosticCounts.get(step.id) ?? 0}
              index={index}
              isSelected={step.id === selectedId}
              key={step.id}
              step={step}
              stepCount={steps.length}
              readOnly={readOnly}
              onDelete={() => void deleteStep(step)}
              onDuplicate={() => void duplicateStep(step)}
              onMoveDown={() => void moveStep(step, index + 1)}
              onMoveUp={() => void moveStep(step, index - 1)}
              onSelect={() => setSelectedStepId(step.id)}
            />
          ))}
        </div>
      }
      sidebarHeader={
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Steps</h1>
          <button
            className="rounded border border-slate-300 px-2 py-1 text-sm disabled:opacity-40"
            disabled={readOnly || stepSpecs.length === 0}
            type="button"
            onClick={openAddStep}
          >
            Add
          </button>
        </div>
      }
      storageKey="emuchef.configEditor.steps.sidebarWidth"
    >
      {selectedStep ? (
        <StepDetailPanel
          step={selectedStep}
          stepSpec={stepSpecs.find((spec) => spec.type === selectedStep.type) ?? null}
          steps={steps}
          recipe={document.recipe}
          refIndex={document.refIndex}
          readOnly={readOnly}
          onCommand={onCommand}
          onAdvancedCommand={onAdvancedCommand}
          onUpdateDependencies={(dependencies) =>
            onCommand({
              type: "UpdateStepDependencies",
              stepId: selectedStep.id,
              dependencies,
            })
          }
          onUpdateName={(name) =>
            onCommand({
              type: "UpdateStepBasics",
              stepId: selectedStep.id,
              name,
              description: selectedStep.description || null,
            })
          }
          onUpdateUserToggleable={(userToggleable) =>
            onCommand({ type: "SetStepUserToggleable", stepId: selectedStep.id, userToggleable })
          }
        />
      ) : (
        <p className="text-sm text-slate-500">Select or add a step.</p>
      )}

      {addDraft ? (
        <AddStepModal
          draft={addDraft}
          stepSpecs={stepSpecs}
          onCancel={() => setAddDraft(null)}
          onSubmit={(draft) => void submitAddStep(draft)}
        />
      ) : null}
    </ResizableEditorLayout>
  );
}

function StepListRow({
  step,
  index,
  stepCount,
  isSelected,
  diagnosticCount,
  readOnly,
  onSelect,
  onMoveUp,
  onMoveDown,
  onDuplicate,
  onDelete,
}: {
  step: StepDto;
  index: number;
  stepCount: number;
  isSelected: boolean;
  diagnosticCount: number;
  readOnly: boolean;
  onSelect: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className={`rounded border px-3 py-2 ${
        isSelected ? "border-slate-900 bg-slate-50" : "border-slate-200 bg-white"
      }`}
    >
      <button className="block w-full text-left" type="button" onClick={onSelect}>
        <div className="flex items-start gap-2">
          <span className="mt-0.5 w-7 shrink-0 text-xs tabular-nums text-slate-500">{index + 1}</span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium text-slate-950">{step.name || step.id}</div>
            <div className="truncate text-xs text-slate-500">{step.id}</div>
          </div>
          {diagnosticCount > 0 ? (
            <span className="rounded bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">
              {diagnosticCount}
            </span>
          ) : null}
        </div>
        <div className="mt-2 truncate rounded bg-slate-100 px-2 py-1 text-xs text-slate-700">{step.type}</div>
      </button>
      <div className="mt-2 flex flex-wrap gap-1">
        <button
          className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
          disabled={readOnly || index === 0}
          type="button"
          onClick={onMoveUp}
        >
          Up
        </button>
        <button
          className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
          disabled={readOnly || index === stepCount - 1}
          type="button"
          onClick={onMoveDown}
        >
          Down
        </button>
        <button className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40" disabled={readOnly} type="button" onClick={onDuplicate}>
          Duplicate
        </button>
        <button className="rounded border border-red-300 px-2 py-1 text-xs text-red-700 disabled:opacity-40" disabled={readOnly} type="button" onClick={onDelete}>
          Delete
        </button>
      </div>
    </div>
  );
}

function StepDetailPanel({
  step,
  stepSpec,
  steps,
  recipe,
  refIndex,
  readOnly,
  onCommand,
  onAdvancedCommand,
  onUpdateDependencies,
  onUpdateName,
  onUpdateUserToggleable,
}: {
  step: StepDto;
  stepSpec: StepSpecDto | null;
  steps: StepDto[];
  recipe: RecipeDto;
  refIndex: RecipeDocumentDto["refIndex"];
  readOnly: boolean;
  onCommand: (command: EditorCommand) => Promise<boolean>;
  onAdvancedCommand: (command: EditorCommand) => Promise<AdvancedCommandResult>;
  onUpdateDependencies: (dependencies: string[]) => Promise<boolean>;
  onUpdateName: (name: string) => Promise<boolean>;
  onUpdateUserToggleable: (userToggleable: boolean) => Promise<boolean>;
}) {
  const userToggleable = userToggleableValue(step);

  return (
    <div className="space-y-5">
      <div className="min-w-0">
        <h2 className="truncate text-xl font-semibold text-slate-950">{step.name || step.id}</h2>
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <ReadonlyText label="ID" value={step.id} />
        <ReadonlyText label="Type" value={step.type} />
        <EditableTextField
          label="Display Name"
          readOnly={readOnly}
          value={step.name || step.id}
          onCommit={(value) => {
            if (!value.trim() || value === step.name) {
              return false;
            }
            return onUpdateName(value);
          }}
        />
        {userToggleable === null ? (
          <ReadonlyText label="User Toggleable" value="Unavailable" />
        ) : (
          <CheckboxField
            checked={userToggleable}
            label="User Toggleable"
            disabled={readOnly}
            onChange={(value) => value !== userToggleable && onUpdateUserToggleable(value)}
          />
        )}
        <ReadonlyMultiline label="Description" value={step.description || ""} />
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <StepDependenciesEditor readOnly={readOnly} step={step} steps={steps} onUpdateDependencies={onUpdateDependencies} />
        <StepParamsEditor readOnly={readOnly} recipe={recipe} refIndex={refIndex} step={step} stepSpec={stepSpec} onCommand={onCommand} />
      </div>

      <AdvancedStepInternalsEditor readOnly={readOnly} step={step} onCommand={onAdvancedCommand} />
    </div>
  );
}

function AddStepModal({
  draft,
  stepSpecs,
  onCancel,
  onSubmit,
}: {
  draft: AddStepDraft;
  stepSpecs: StepSpecDto[];
  onCancel: () => void;
  onSubmit: (draft: AddStepDraft) => void;
}) {
  const [stepId, setStepId] = useState(draft.stepId);
  const [stepType, setStepType] = useState(draft.stepType || stepSpecs[0]?.type || "");
  const [name, setName] = useState(draft.name);
  const [validationMessage, setValidationMessage] = useState<string | null>(null);

  useEffect(() => {
    setStepId(draft.stepId);
    setStepType(draft.stepType || stepSpecs[0]?.type || "");
    setName(draft.name);
    setValidationMessage(null);
  }, [draft, stepSpecs]);

  function submit() {
    const trimmedId = stepId.trim();
    if (!trimmedId) {
      setValidationMessage("Step id must not be empty.");
      return;
    }
    if (!stepType) {
      setValidationMessage("Step type must be selected.");
      return;
    }
    onSubmit({ stepId: trimmedId, stepType, name });
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 px-4" role="presentation">
      <div
        aria-modal="true"
        className="w-full max-w-md rounded-lg border border-slate-200 bg-white p-5 shadow-xl"
        role="dialog"
      >
        <div className="mb-4">
          <h2 className="text-base font-semibold text-slate-950">Add Step</h2>
        </div>
        <form
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <label className="grid gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Step ID</span>
            <input
              {...textInputGuardProps}
              autoFocus
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
              value={stepId}
              onChange={(event) => {
                setStepId(normalizeEditableText(event.target.value));
                setValidationMessage(null);
              }}
            />
          </label>
          <label className="grid gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Step Type</span>
            <select
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
              value={stepType}
              onChange={(event) => {
                setStepType(event.target.value);
                setValidationMessage(null);
              }}
            >
              {stepSpecs.map((spec) => (
                <option key={spec.type} value={spec.type}>
                  {spec.label ? `${spec.label} (${spec.type})` : spec.type}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Display Name</span>
            <input
              {...textInputGuardProps}
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
              value={name}
              onChange={(event) => setName(normalizeEditableText(event.target.value))}
            />
          </label>
          {validationMessage ? <p className="text-sm font-medium text-red-700">{validationMessage}</p> : null}
          <div className="flex justify-end gap-2">
            <button
              className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
              type="button"
              onClick={onCancel}
            >
              Cancel
            </button>
            <button className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white" type="submit">
              Add
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function ReadonlyText({ label, value }: { label: string; value: string }) {
  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      <input className="w-full rounded border border-slate-200 bg-slate-100 px-3 py-2 text-sm text-slate-600" readOnly value={value} />
    </label>
  );
}

function ReadonlyMultiline({ label, value }: { label: string; value: string }) {
  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      <textarea
        className="min-h-20 w-full resize-y rounded border border-slate-200 bg-slate-100 px-3 py-2 text-sm text-slate-600"
        readOnly
        value={value}
      />
    </label>
  );
}

function CheckboxField({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => boolean | void | Promise<boolean>;
}) {
  return (
    <label className="flex items-center gap-2 text-sm text-slate-700">
      <input
        checked={checked}
        className="h-4 w-4 rounded border-slate-300"
        disabled={disabled}
        type="checkbox"
        onChange={(event) => void onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}

function stepDiagnosticCounts(diagnostics: DiagnosticDto[]) {
  const counts = new Map<string, number>();
  for (const diagnostic of diagnostics) {
    if (diagnostic.objectKind !== "step" || !diagnostic.objectId) {
      continue;
    }
    counts.set(diagnostic.objectId, (counts.get(diagnostic.objectId) ?? 0) + 1);
  }
  return counts;
}

function userToggleableValue(step: StepDto): boolean | null {
  const rawStep = step as StepDto & { userToggleable?: unknown };
  return typeof rawStep.userToggleable === "boolean" ? rawStep.userToggleable : null;
}

import { useEffect, useMemo, useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { StepDto } from "../api/types";
import {
  buildAdvancedInternalsCommand,
  editorValueForAdvancedField,
  formatJsonDraft,
  parseAdvancedJsonDraft,
  revertJsonDraft,
  type AdvancedInternalsField,
} from "./advancedStepInternals.logic";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

export interface AdvancedCommandResult {
  ok: boolean;
  changed: boolean;
}

interface AdvancedStepInternalsEditorProps {
  step: StepDto;
  onCommand: (command: EditorCommand) => Promise<AdvancedCommandResult>;
}

interface SectionConfig {
  field: AdvancedInternalsField;
  property: keyof StepDto;
  label: string;
  unsetInitialValue: unknown;
}

const SECTIONS: SectionConfig[] = [
  { field: "constraints", property: "constraints", label: "Constraints", unsetInitialValue: {} },
  { field: "skipIf", property: "skipIf", label: "skip_if", unsetInitialValue: [] },
  { field: "verify", property: "verify", label: "Verify", unsetInitialValue: [] },
];

export function AdvancedStepInternalsEditor({ step, onCommand }: AdvancedStepInternalsEditorProps) {
  return (
    <details className="rounded border border-slate-200 bg-white p-4">
      <summary className="cursor-pointer text-sm font-semibold uppercase tracking-wide text-slate-500">
        Advanced
      </summary>
      <div className="mt-4 grid gap-4">
        {SECTIONS.map((section) => {
          const stepRecord = step as unknown as Record<string, unknown>;
          const hasValue =
            Object.prototype.hasOwnProperty.call(stepRecord, section.property) &&
            stepRecord[section.property] !== undefined;
          return (
            <AdvancedJsonSectionEditor
              field={section.field}
              hasValue={hasValue}
              key={`${step.id}:${section.field}`}
              label={section.label}
              stepId={step.id}
              unsetInitialValue={section.unsetInitialValue}
              value={stepRecord[section.property]}
              onCommand={onCommand}
            />
          );
        })}
      </div>
    </details>
  );
}

function AdvancedJsonSectionEditor({
  field,
  label,
  stepId,
  value,
  hasValue,
  unsetInitialValue,
  onCommand,
}: {
  field: AdvancedInternalsField;
  label: string;
  stepId: string;
  value: unknown;
  hasValue: boolean;
  unsetInitialValue: unknown;
  onCommand: (command: EditorCommand) => Promise<AdvancedCommandResult>;
}) {
  const [editingUnset, setEditingUnset] = useState(false);
  const activeValue = hasValue ? value : unsetInitialValue;
  const editorValue = useMemo(() => editorValueForAdvancedField(field, activeValue), [activeValue, field]);
  const formattedValue = useMemo(() => formatJsonDraft(editorValue), [editorValue]);
  const [draft, setDraft] = useState(formattedValue);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(formattedValue);
    setError(null);
    if (hasValue) {
      setEditingUnset(false);
    }
  }, [formattedValue, hasValue]);

  async function apply() {
    if (submitting) {
      return;
    }
    const parsed = parseAdvancedJsonDraft(field, draft);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    const command = buildAdvancedInternalsCommand(field, stepId, parsed.value, hasValue ? editorValue : undefined);
    if (command === null) {
      setError(null);
      return;
    }
    setSubmitting(true);
    const result = await onCommand(command);
    setSubmitting(false);
    if (!result.ok) {
      return;
    }
    if (!result.changed) {
      setError("No document change was produced.");
      return;
    }
    if (result.changed) {
      setError(null);
      setEditingUnset(false);
    }
  }

  function revert() {
    setDraft(revertJsonDraft(editorValue));
    setError(null);
    if (!hasValue) {
      setEditingUnset(false);
    }
  }

  if (!hasValue && !editingUnset) {
    return (
      <section className="grid gap-2">
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</h3>
          <button
            className="rounded border border-slate-300 px-2 py-1 text-xs text-slate-700 hover:bg-slate-50"
            type="button"
            onClick={() => {
              setDraft(formatJsonDraft(unsetInitialValue));
              setError(null);
              setEditingUnset(true);
            }}
          >
            Edit JSON
          </button>
        </div>
        <p className="text-sm text-slate-500">Unset</p>
      </section>
    );
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</h3>
      <textarea
        {...textInputGuardProps}
        className="min-h-28 w-full resize-y rounded border border-slate-300 bg-white px-3 py-2 font-mono text-xs text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        value={draft}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
          disabled={submitting}
          type="button"
          onClick={() => void apply()}
        >
          Apply
        </button>
        <button
          className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
          disabled={submitting}
          type="button"
          onClick={revert}
        >
          Revert
        </button>
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </section>
  );
}

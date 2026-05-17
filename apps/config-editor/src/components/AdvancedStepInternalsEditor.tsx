import { useEffect, useMemo, useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { StepDto } from "../api/types";
import {
  buildAddVerifyEntry,
  buildAdvancedInternalsCommand,
  buildVerifyEntryJsonUpdate,
  buildVerifyKnownFieldUpdate,
  classifyVerifyEntry,
  editorValueForAdvancedField,
  formatJsonDraft,
  moveVerifyEntry,
  parseAdvancedJsonDraft,
  removeVerifyEntry,
  revertJsonDraft,
  type SupportedVerifyType,
  type AdvancedInternalsField,
} from "./advancedStepInternals.logic";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

export interface AdvancedCommandResult {
  ok: boolean;
  changed: boolean;
}

interface AdvancedStepInternalsEditorProps {
  readOnly?: boolean;
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

export function AdvancedStepInternalsEditor({ readOnly = false, step, onCommand }: AdvancedStepInternalsEditorProps) {
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
            section.field === "verify" ? (
              <VerifySectionEditor
                hasValue={hasValue}
                key={`${step.id}:${section.field}`}
                readOnly={readOnly}
                stepId={step.id}
                value={stepRecord[section.property]}
                onCommand={onCommand}
              />
            ) : (
              <AdvancedJsonSectionEditor
                field={section.field}
                hasValue={hasValue}
                key={`${step.id}:${section.field}`}
                label={section.label}
                readOnly={readOnly}
                stepId={step.id}
                unsetInitialValue={section.unsetInitialValue}
                value={stepRecord[section.property]}
                onCommand={onCommand}
              />
            )
          );
        })}
      </div>
    </details>
  );
}

const VERIFY_TYPES: Array<{ type: SupportedVerifyType; label: string; fieldName: "path" | "package_name" }> = [
  { type: "path_exists", label: "Path exists", fieldName: "path" },
  { type: "file_exists", label: "File exists", fieldName: "path" },
  { type: "package_installed", label: "Package installed", fieldName: "package_name" },
];

function verifyFieldLabel(fieldName: "path" | "package_name"): string {
  return fieldName === "package_name" ? "Package name" : "Path";
}

function VerifySectionEditor({
  readOnly,
  stepId,
  value,
  hasValue,
  onCommand,
}: {
  readOnly: boolean;
  stepId: string;
  value: unknown;
  hasValue: boolean;
  onCommand: (command: EditorCommand) => Promise<AdvancedCommandResult>;
}) {
  const verify = Array.isArray(value) ? value : [];
  const [addType, setAddType] = useState<SupportedVerifyType>("path_exists");
  const [addDraft, setAddDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const selectedType = VERIFY_TYPES.find((option) => option.type === addType) ?? VERIFY_TYPES[0];

  useEffect(() => {
    setError(null);
  }, [value]);

  async function submit(nextVerify: unknown[]) {
    if (submitting) {
      return false;
    }
    const command = buildAdvancedInternalsCommand("verify", stepId, nextVerify, hasValue ? verify : undefined);
    if (command === null) {
      setError(null);
      return true;
    }
    setSubmitting(true);
    const result = await onCommand(command);
    setSubmitting(false);
    if (!result.ok) {
      return false;
    }
    if (!result.changed) {
      setError("No document change was produced.");
      return false;
    }
    setError(null);
    return true;
  }

  async function addEntry() {
    const result = buildAddVerifyEntry(verify, addType, addDraft);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    const ok = await submit(result.value);
    if (ok) {
      setAddDraft("");
    }
  }

  async function updateKnownField(index: number, nextValue: string) {
    const result = buildVerifyKnownFieldUpdate(verify, index, nextValue);
    if (result === null) {
      setError(null);
      return true;
    }
    if (!result.ok) {
      setError(result.error);
      return false;
    }
    return submit(result.value);
  }

  async function updateJsonEntry(index: number, draft: string) {
    const result = buildVerifyEntryJsonUpdate(verify, index, draft);
    if (result === null) {
      setError(null);
      return null;
    }
    if (!result.ok) {
      return result;
    }
    const ok = await submit(result.value);
    return ok ? null : { ok: false as const, error: "Edit failed." };
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Verify</h3>
      {verify.length === 0 ? <p className="text-sm text-slate-500">No verify checks</p> : null}
      <div className="grid gap-2">
        {verify.map((entry, index) => {
          const classification = classifyVerifyEntry(entry);
          return classification.kind === "structured" ? (
            <StructuredVerifyRow
              classification={classification}
              disabled={readOnly || submitting}
              index={index}
              key={index}
              rowCount={verify.length}
              onMoveDown={() => submit(moveVerifyEntry(verify, index, index + 1))}
              onMoveUp={() => submit(moveVerifyEntry(verify, index, index - 1))}
              onRemove={() => submit(removeVerifyEntry(verify, index))}
              onUpdate={(nextValue) => updateKnownField(index, nextValue)}
            />
          ) : (
            <JsonVerifyRow
              disabled={readOnly || submitting}
              entry={entry}
              index={index}
              key={index}
              rowCount={verify.length}
              onMoveDown={() => submit(moveVerifyEntry(verify, index, index + 1))}
              onMoveUp={() => submit(moveVerifyEntry(verify, index, index - 1))}
              onRemove={() => submit(removeVerifyEntry(verify, index))}
              onUpdate={(draft) => updateJsonEntry(index, draft)}
            />
          );
        })}
      </div>
      <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2 sm:grid-cols-[minmax(9rem,12rem)_minmax(0,1fr)_auto] sm:items-end">
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Type</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            disabled={readOnly || submitting}
            value={addType}
            onChange={(event) => {
              setAddType(event.target.value as SupportedVerifyType);
              setError(null);
            }}
          >
            {VERIFY_TYPES.map((option) => (
              <option key={option.type} value={option.type}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">
            {verifyFieldLabel(selectedType.fieldName)}
          </span>
          <input
            {...textInputGuardProps}
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
            disabled={readOnly || submitting}
            value={addDraft}
            onChange={(event) => {
              setAddDraft(normalizeEditableText(event.target.value));
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void addEntry();
              }
            }}
          />
        </label>
        <button
          className="w-fit rounded border border-slate-900 bg-slate-900 px-3 py-2 text-sm text-white disabled:opacity-40 sm:w-auto"
          disabled={readOnly || submitting}
          type="button"
          onClick={() => void addEntry()}
        >
          Add check
        </button>
      </div>
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </section>
  );
}

function StructuredVerifyRow({
  classification,
  index,
  rowCount,
  disabled,
  onUpdate,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  classification: Extract<ReturnType<typeof classifyVerifyEntry>, { kind: "structured" }>;
  index: number;
  rowCount: number;
  disabled: boolean;
  onUpdate: (nextValue: string) => Promise<boolean>;
  onMoveUp: () => Promise<boolean>;
  onMoveDown: () => Promise<boolean>;
  onRemove: () => Promise<boolean>;
}) {
  const [draft, setDraft] = useState(classification.fieldValue);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(classification.fieldValue);
    setError(null);
  }, [classification.fieldValue]);

  async function commit() {
    if (draft === classification.fieldValue) {
      setError(null);
      return;
    }
    const ok = await onUpdate(draft);
    if (ok) {
      setError(null);
    } else if (!draft.trim()) {
      setError(`${classification.fieldName} is required.`);
    }
  }

  return (
    <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2">
      <div className="grid gap-2 sm:grid-cols-[minmax(9rem,12rem)_minmax(0,1fr)] sm:items-end">
        <div className="min-w-0">
          <div className="break-words text-sm font-medium text-slate-900">{classification.type}</div>
          <div className="text-xs text-slate-500">Structured verify check</div>
        </div>
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">
            {verifyFieldLabel(classification.fieldName)}
          </span>
          <input
            {...textInputGuardProps}
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
            disabled={disabled}
            value={draft}
            onBlur={() => void commit()}
            onChange={(event) => {
              setDraft(normalizeEditableText(event.target.value));
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void commit();
              }
              if (event.key === "Escape") {
                setDraft(classification.fieldValue);
                setError(null);
              }
            }}
          />
        </label>
      </div>
      <VerifyRowActions
        disabled={disabled}
        index={index}
        rowCount={rowCount}
        onMoveDown={onMoveDown}
        onMoveUp={onMoveUp}
        onRemove={onRemove}
      />
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function JsonVerifyRow({
  entry,
  index,
  rowCount,
  disabled,
  onUpdate,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  entry: unknown;
  index: number;
  rowCount: number;
  disabled: boolean;
  onUpdate: (draft: string) => Promise<{ ok: false; error: string } | null>;
  onMoveUp: () => Promise<boolean>;
  onMoveDown: () => Promise<boolean>;
  onRemove: () => Promise<boolean>;
}) {
  const formattedValue = useMemo(() => formatJsonDraft(entry), [entry]);
  const [draft, setDraft] = useState(formattedValue);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(formattedValue);
    setError(null);
  }, [formattedValue]);

  async function apply() {
    const result = await onUpdate(draft);
    setError(result?.error ?? null);
  }

  return (
    <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-sm font-medium text-slate-900">JSON verify check</div>
          <div className="text-xs text-slate-500">Unsupported shape</div>
        </div>
      </div>
      <textarea
        {...textInputGuardProps}
        className="min-h-24 w-full resize-y rounded border border-slate-300 bg-white px-3 py-2 font-mono text-xs text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        readOnly={disabled}
        value={draft}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="rounded border border-slate-900 bg-slate-900 px-2 py-1.5 text-sm text-white disabled:opacity-40"
          disabled={disabled}
          type="button"
          onClick={() => void apply()}
        >
          Apply JSON
        </button>
        <button
          className="rounded border border-slate-300 px-2 py-1.5 text-sm text-slate-700 hover:bg-slate-50 disabled:opacity-40"
          disabled={disabled}
          type="button"
          onClick={() => {
            setDraft(formattedValue);
            setError(null);
          }}
        >
          Revert
        </button>
        <VerifyRowActions
          disabled={disabled}
          index={index}
          rowCount={rowCount}
          onMoveDown={onMoveDown}
          onMoveUp={onMoveUp}
          onRemove={onRemove}
        />
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </div>
  );
}

function VerifyRowActions({
  index,
  rowCount,
  disabled,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  index: number;
  rowCount: number;
  disabled: boolean;
  onMoveUp: () => Promise<boolean>;
  onMoveDown: () => Promise<boolean>;
  onRemove: () => Promise<boolean>;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <button
        className="rounded border border-slate-300 px-2 py-1.5 text-sm text-slate-700 disabled:opacity-40"
        disabled={disabled || index === 0}
        type="button"
        onClick={() => void onMoveUp()}
      >
        Up
      </button>
      <button
        className="rounded border border-slate-300 px-2 py-1.5 text-sm text-slate-700 disabled:opacity-40"
        disabled={disabled || index === rowCount - 1}
        type="button"
        onClick={() => void onMoveDown()}
      >
        Down
      </button>
      <button
        className="rounded border border-red-300 px-2 py-1.5 text-sm text-red-700 disabled:opacity-40"
        disabled={disabled}
        type="button"
        onClick={() => void onRemove()}
      >
        Remove
      </button>
    </div>
  );
}

function AdvancedJsonSectionEditor({
  field,
  label,
  readOnly,
  stepId,
  value,
  hasValue,
  unsetInitialValue,
  onCommand,
}: {
  field: AdvancedInternalsField;
  label: string;
  readOnly: boolean;
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
            disabled={readOnly}
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
        readOnly={readOnly}
        value={draft}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
          disabled={readOnly || submitting}
          type="button"
          onClick={() => void apply()}
        >
          Apply
        </button>
        <button
          className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
          disabled={readOnly || submitting}
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

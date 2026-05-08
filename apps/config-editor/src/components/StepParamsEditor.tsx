import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { EditorCommand } from "../api/commands";
import type { RefIndexDto, StepDto, StepSpecDto } from "../api/types";
import {
  buildClearStepParamCommand,
  buildRefPickerOptions,
  buildUpdateStepParamsCommand,
  isAuthoredRefValue,
  orderedParamNames,
  parseJsonParamDraft,
  parseNumberParamDraft,
  paramValuesEqual,
  type RefPickerOption,
} from "./stepParams.logic";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

interface StepParamsEditorProps {
  readOnly?: boolean;
  step: StepDto;
  stepSpec: StepSpecDto | null;
  refIndex: RefIndexDto;
  onCommand: (command: EditorCommand) => Promise<boolean>;
}

export function StepParamsEditor({ readOnly = false, step, stepSpec, refIndex, onCommand }: StepParamsEditorProps) {
  const paramNames = useMemo(() => orderedParamNames(step.params, stepSpec), [step.params, stepSpec]);

  async function updateParam(paramName: string, nextValue: unknown): Promise<boolean> {
    const command = buildUpdateStepParamsCommand(step.id, step.params, paramName, nextValue);
    if (command === null) {
      return true;
    }
    return onCommand(command);
  }

  async function clearParam(paramName: string): Promise<boolean> {
    const command = buildClearStepParamCommand(step.id, step.params, paramName);
    if (command === null) {
      return true;
    }
    return onCommand(command);
  }

  return (
    <div className="grid gap-3">
      <div>
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Params</h3>
      </div>
      {paramNames.length === 0 ? (
        <p className="text-sm text-slate-500">No params</p>
      ) : (
        <div className="divide-y divide-slate-200 rounded border border-slate-200">
          {paramNames.map((paramName) => (
            <StepParamRow
              key={paramName}
              paramName={paramName}
              readOnly={readOnly}
              refIndex={refIndex}
              stepSpec={stepSpec}
              value={step.params[paramName]}
              onClear={() => clearParam(paramName)}
              onUpdate={(nextValue) => updateParam(paramName, nextValue)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function StepParamRow({
  paramName,
  value,
  readOnly,
  stepSpec,
  refIndex,
  onUpdate,
  onClear,
}: {
  paramName: string;
  value: unknown;
  readOnly: boolean;
  stepSpec: StepSpecDto | null;
  refIndex: RefIndexDto;
  onUpdate: (nextValue: unknown) => Promise<boolean>;
  onClear: () => Promise<boolean>;
}) {
  const paramSpec = stepSpec?.params[paramName] ?? null;
  const enumValues = typeof value === "string" ? paramSpec?.enumValues ?? [] : [];
  const allowedValueTypes = stepSpec?.refFilters[paramName] ?? [];

  let control: ReactNode;
  if (isAuthoredRefValue(value)) {
    control = (
      <RefPicker
        allowedValueTypes={allowedValueTypes}
        currentRef={value.ref}
        disabled={readOnly}
        refIndex={refIndex}
        onUpdate={(ref) => onUpdate({ ref })}
      />
    );
  } else if (enumValues.length > 0) {
    control = <EnumParamInput disabled={readOnly} enumValues={enumValues} value={value} onUpdate={onUpdate} />;
  } else if (typeof value === "boolean") {
    control = <BooleanParamInput disabled={readOnly} value={value} onUpdate={onUpdate} />;
  } else if (typeof value === "number") {
    control = <NumberParamInput readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  } else if (typeof value === "string") {
    control = <StringParamInput readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  } else {
    control = <JsonValueEditor readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  }

  return (
    <div className="grid gap-3 bg-white p-3 sm:grid-cols-[minmax(8rem,12rem)_minmax(0,1fr)_auto] sm:items-start">
      <div className="min-w-0">
        <div className="break-words text-sm font-medium text-slate-950">{paramName}</div>
        {paramSpec ? (
          <div className="mt-1 text-xs text-slate-500">
            {paramSpec.required ? "Required" : "Optional"} · {paramSpec.mode}
          </div>
        ) : null}
      </div>
      <div className="min-w-0">{control}</div>
      <button
        className="h-9 rounded border border-slate-300 px-2 text-sm text-slate-700 hover:bg-slate-50"
        disabled={readOnly}
        type="button"
        onClick={() => void onClear()}
      >
        Clear
      </button>
    </div>
  );
}

function StringParamInput({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: string; onUpdate: (nextValue: string) => Promise<boolean> }) {
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(value);
    setError(null);
  }, [value]);

  async function commit() {
    if (draft === value || submitting) {
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(draft);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-1">
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        readOnly={readOnly}
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
            setDraft(value);
            setError(null);
          }
        }}
      />
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function NumberParamInput({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: number; onUpdate: (nextValue: number) => Promise<boolean> }) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(String(value));
    setError(null);
  }, [value]);

  async function commit() {
    if (submitting) {
      return;
    }
    const parsed = parseNumberParamDraft(draft, value);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    if (paramValuesEqual(value, parsed.value)) {
      setError(null);
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(parsed.value as number);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-1">
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        inputMode="decimal"
        readOnly={readOnly}
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
            setDraft(String(value));
            setError(null);
          }
        }}
      />
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function EnumParamInput({
  value,
  enumValues,
  disabled = false,
  onUpdate,
}: {
  value: unknown;
  enumValues: string[];
  disabled?: boolean;
  onUpdate: (nextValue: string) => Promise<boolean>;
}) {
  const stringValue = String(value);
  const options = enumValues.includes(stringValue) ? enumValues : [stringValue, ...enumValues];

  return (
    <select
      className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
      disabled={disabled}
      value={stringValue}
      onChange={(event) => {
        if (event.target.value !== stringValue) {
          void onUpdate(event.target.value);
        }
      }}
    >
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

function BooleanParamInput({ disabled = false, value, onUpdate }: { disabled?: boolean; value: boolean; onUpdate: (nextValue: boolean) => Promise<boolean> }) {
  return (
    <label className="flex h-9 items-center gap-2 text-sm text-slate-700">
      <input
        checked={value}
        className="h-4 w-4 rounded border-slate-300"
        disabled={disabled}
        type="checkbox"
        onChange={(event) => {
          if (event.target.checked !== value) {
            void onUpdate(event.target.checked);
          }
        }}
      />
      Enabled
    </label>
  );
}

function RefPicker({
  currentRef,
  allowedValueTypes,
  disabled = false,
  refIndex,
  onUpdate,
}: {
  currentRef: string;
  allowedValueTypes: readonly string[];
  disabled?: boolean;
  refIndex: RefIndexDto;
  onUpdate: (nextRef: string) => Promise<boolean>;
}) {
  const [showAll, setShowAll] = useState(false);
  const options = useMemo(
    () => buildRefPickerOptions(refIndex, { allowedValueTypes, currentRef, showAll }),
    [allowedValueTypes, currentRef, refIndex, showAll],
  );

  return (
    <div className="grid gap-2">
      <select
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
        disabled={disabled}
        value={currentRef}
        onChange={(event) => {
          if (event.target.value !== currentRef) {
            void onUpdate(event.target.value);
          }
        }}
      >
        {options.map((option) => (
          <option key={option.ref} value={option.ref}>
            {refOptionLabel(option)}
          </option>
        ))}
      </select>
      {allowedValueTypes.length > 0 ? (
        <button
          className="w-fit rounded border border-slate-300 px-2 py-1 text-xs text-slate-700 hover:bg-slate-50"
          disabled={disabled}
          type="button"
          onClick={() => setShowAll((current) => !current)}
        >
          {showAll ? "Filter refs" : "Show all refs"}
        </button>
      ) : null}
    </div>
  );
}

function JsonValueEditor({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: unknown; onUpdate: (nextValue: unknown) => Promise<boolean> }) {
  const formattedValue = useMemo(() => JSON.stringify(value, null, 2), [value]);
  const [draft, setDraft] = useState(formattedValue);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(formattedValue);
    setError(null);
  }, [formattedValue]);

  async function apply() {
    if (submitting) {
      return;
    }
    const parsed = parseJsonParamDraft(draft);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    if (paramValuesEqual(value, parsed.value)) {
      setError(null);
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(parsed.value);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-2">
      <textarea
        {...textInputGuardProps}
        className="min-h-24 w-full resize-y rounded border border-slate-300 bg-white px-3 py-2 font-mono text-xs text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        readOnly={readOnly}
        value={draft}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setDraft(formattedValue);
            setError(null);
          }
        }}
      />
      <div className="flex items-center gap-2">
        <button
          className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
          disabled={readOnly || submitting}
          type="button"
          onClick={() => void apply()}
        >
          Apply
        </button>
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </div>
  );
}

function refOptionLabel(option: RefPickerOption): string {
  const suffixes: string[] = [];
  if (option.current) {
    suffixes.push("current");
  }
  if (option.missing) {
    suffixes.push("missing");
  }
  if (option.incompatible) {
    suffixes.push("incompatible");
  }
  const metadata = option.valueType ? `${option.label} · ${option.valueType}` : option.label;
  return suffixes.length > 0 ? `${metadata} (${suffixes.join(", ")})` : metadata;
}

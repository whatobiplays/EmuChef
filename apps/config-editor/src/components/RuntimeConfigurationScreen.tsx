import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { describeConfiguration } from "../api/editorApi";
import type {
  ConfigurationDescriptionResult,
  RuntimeConfigurationInputDto,
  UserConfigurationDocumentDto,
} from "../api/types";
import { parseBindingText } from "./userConfiguration.logic";
import { runtimeControlKind } from "./runtimeConfiguration.logic";

interface RuntimeConfigurationScreenProps {
  document: UserConfigurationDocumentDto;
  disabled: boolean;
  onBind: (key: string, value: unknown) => Promise<void>;
}

/** Renders the catalog-described runtime input surface for an open saved configuration. */
export function RuntimeConfigurationScreen({ document, disabled, onBind }: RuntimeConfigurationScreenProps) {
  const [description, setDescription] = useState<ConfigurationDescriptionResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!document.authoredRoot) {
      setDescription(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void describeConfiguration({
      authoredRoot: document.authoredRoot,
      userConfiguration: document.path,
      deviceContext: {},
    }).then((response) => {
      if (cancelled) {
        return;
      }
      if (response.kind === "success") {
        setDescription(response.result);
        setError(null);
      } else {
        setDescription(null);
        setError(response.kind === "api-error" ? response.error.message : response.message);
      }
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [document.authoredRoot, document.path, document.yaml]);

  if (!document.authoredRoot) {
    return (
      <section className="mt-5 rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-900">
        Set an authored root before opening this document to discover its runtime input form.
      </section>
    );
  }
  if (loading && description === null) {
    return <p className="mt-5 text-sm text-slate-500">Loading runtime configuration...</p>;
  }
  if (error) {
    return <p className="mt-5 rounded border border-red-200 bg-red-50 p-4 text-sm text-red-800">{error}</p>;
  }
  if (!description) {
    return null;
  }

  const groups = description.expandedRecipes.map((recipeId) => ({
    recipeId,
    inputs: description.inputs.filter((input) => input.recipeId === recipeId),
  }));
  return (
    <section className="mt-5 rounded border border-slate-200 bg-white p-4">
      <h2 className="font-semibold">Runtime configuration</h2>
      <p className="mt-1 text-sm text-slate-600">
        Selected recipes: {description.selectedRecipes.length ? description.selectedRecipes.join(", ") : "None"}
      </p>
      <div className="mt-4 space-y-5">
        {groups.map((group) => (
          <div key={group.recipeId}>
            <h3 className="border-b border-slate-200 pb-2 font-mono text-sm font-semibold">{group.recipeId}</h3>
            <div className="mt-3 space-y-4">
              {group.inputs.map((input) => (
                <RuntimeInputControl disabled={disabled} input={input} key={input.key} onBind={onBind} />
              ))}
            </div>
          </div>
        ))}
      </div>
      {description.diagnostics.filter((diagnostic) => diagnostic.key === null).map((diagnostic, index) => (
        <p className="mt-3 rounded border border-red-200 bg-red-50 p-2 text-sm text-red-800" key={`${diagnostic.code}-${index}`}>
          {diagnostic.message}
        </p>
      ))}
    </section>
  );
}

function RuntimeInputControl({
  disabled,
  input,
  onBind,
}: {
  disabled: boolean;
  input: RuntimeConfigurationInputDto;
  onBind: (key: string, value: unknown) => Promise<void>;
}) {
  const control = runtimeControlKind(input);
  const [draft, setDraft] = useState(formatValue(input.value));
  useEffect(() => setDraft(formatValue(input.value)), [input.value]);

  async function chooseHostPath() {
    const selected = await open({
      multiple: input.multiple,
      directory: input.type === "directory",
      filters: input.validation.allowedExtensions.length
        ? [{ name: input.label, extensions: input.validation.allowedExtensions }]
        : undefined,
    });
    if (selected) {
      await onBind(input.key, selected);
    }
  }

  return (
    <div>
      <div className="flex flex-wrap items-baseline gap-2">
        <label className="font-medium" htmlFor={`runtime-${input.key}`}>{input.label}</label>
        <span className="text-xs text-slate-500">{input.required ? "Required" : "Optional"}</span>
        {input.advanced ? <span className="text-xs text-slate-500">Advanced</span> : null}
        {input.sensitive ? <span className="text-xs text-slate-500">Sensitive</span> : null}
        <span className="text-xs text-slate-500">Source: {input.valueSource ?? "unbound"}</span>
      </div>
      {input.description ? <p className="mt-1 text-sm text-slate-600">{input.description}</p> : null}
      <div className="mt-2 flex gap-2">
        {control === "boolean" ? (
          <input id={`runtime-${input.key}`} type="checkbox" checked={input.value === true} disabled={disabled} onChange={(event) => void onBind(input.key, event.target.checked)} />
        ) : null}
        {control === "enum" ? (
          <select className="min-w-0 flex-1 rounded border border-slate-300 px-3 py-2" id={`runtime-${input.key}`} disabled={disabled} value={JSON.stringify(input.value)} onChange={(event) => void onBind(input.key, JSON.parse(event.target.value) as unknown)}>
            {input.value === null ? <option value="null">Select a value</option> : null}
            {input.options.map((option) => <option key={JSON.stringify(option.value)} value={JSON.stringify(option.value)}>{option.label}</option>)}
          </select>
        ) : null}
        {control !== "boolean" && control !== "enum" ? (
          <input
            className="min-w-0 flex-1 rounded border border-slate-300 px-3 py-2 font-mono text-sm"
            disabled={disabled}
            id={`runtime-${input.key}`}
            inputMode={control === "integer" ? "numeric" : undefined}
            type={input.sensitive ? "password" : control === "integer" ? "number" : "text"}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
          />
        ) : null}
        {control === "host_path" ? <button className="rounded border border-slate-300 px-3 py-2 text-sm" disabled={disabled} type="button" onClick={() => void chooseHostPath()}>Choose...</button> : null}
        {control !== "boolean" && control !== "enum" ? <button className="rounded border border-slate-300 px-3 py-2 text-sm" disabled={disabled || draft === formatValue(input.value)} type="button" onClick={() => void onBind(input.key, control === "integer" ? Number(draft) : parseBindingText(draft))}>Apply</button> : null}
      </div>
      {input.diagnostics.map((diagnostic, index) => (
        <p className="mt-2 text-sm text-red-700" key={`${diagnostic.code}-${index}`}>{diagnostic.message}</p>
      ))}
    </div>
  );
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

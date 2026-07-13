import { useState } from "react";

import {
  closeUserConfiguration,
  emitUserConfigurationYaml,
  planConfiguration,
  saveUserConfiguration,
  setUserConfigurationBinding,
  type EditorApiResult,
} from "../api/editorApi";
import type {
  UserConfigurationCommandResult,
  UserConfigurationDocumentDto,
  UserConfigurationDocumentResult,
} from "../api/types";
import { parseBindingText } from "./userConfiguration.logic";
import { RuntimeConfigurationScreen } from "./RuntimeConfigurationScreen";

interface UserConfigurationEditorProps {
  document: UserConfigurationDocumentDto;
  onClose: () => void;
  onDocument: (document: UserConfigurationDocumentDto) => void;
  onError: (message: string) => void;
}

/**
 * Provides the deliberately small first editing surface for persisted runtime configuration.
 * Values remain JSON-compatible, while non-JSON text is stored as a string to match CLI binding
 * parsing. Catalog-aware diagnostics come from the Rust document session.
 */
export function UserConfigurationEditor({ document, onClose, onDocument, onError }: UserConfigurationEditorProps) {
  const [busy, setBusy] = useState(false);
  const [plannedJson, setPlannedJson] = useState<string | null>(null);

  async function updateBinding(key: string, text: string) {
    await updateBindingValue(key, parseBindingText(text));
  }

  async function updateBindingValue(key: string, value: unknown) {
    setBusy(true);
    try {
      handleDocumentResult(await setUserConfigurationBinding(document.documentId, key, value));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    setBusy(true);
    try {
      handleDocumentResult(await saveUserConfiguration(document.documentId));
    } finally {
      setBusy(false);
    }
  }

  async function refreshYaml() {
    setBusy(true);
    try {
      const response = await emitUserConfigurationYaml(document.documentId);
      if (response.kind === "success") {
        onDocument({ ...document, yaml: response.result.yaml });
      } else {
        onError(apiFailureMessage(response, "Canonical YAML refresh failed."));
      }
    } finally {
      setBusy(false);
    }
  }

  async function close() {
    if (document.dirty && !window.confirm("Discard unsaved user-configuration changes?")) {
      return;
    }
    setBusy(true);
    try {
      const response = await closeUserConfiguration(document.documentId);
      if (response.kind === "success") {
        onClose();
      } else {
        onError(apiFailureMessage(response, "User configuration could not be closed."));
      }
    } finally {
      setBusy(false);
    }
  }

  async function plan() {
    if (!document.authoredRoot) {
      onError("Set an authored root before planning this user configuration.");
      return;
    }
    setBusy(true);
    try {
      const response = await planConfiguration({
        authoredRoot: document.authoredRoot,
        userConfiguration: document.path,
        deviceContext: {},
      });
      if (response.kind === "success") {
        setPlannedJson(JSON.stringify(response.result, null, 2));
      } else {
        onError(apiFailureMessage(response, "Runtime configuration planning failed."));
      }
    } finally {
      setBusy(false);
    }
  }

  function handleDocumentResult(
    response: EditorApiResult<UserConfigurationDocumentResult | UserConfigurationCommandResult>,
  ) {
    if (response.kind === "success") {
      onDocument(response.result.document);
    } else {
      onError(apiFailureMessage(response, "User configuration operation failed."));
    }
  }

  return (
    <main className="flex h-screen min-h-0 flex-col bg-slate-50 text-slate-950">
      <header className="flex items-center gap-3 border-b border-slate-200 bg-white px-5 py-3">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">User configuration</p>
          <h1 className="truncate text-lg font-semibold">{document.configuration.name}</h1>
          <p className="truncate text-xs text-slate-500">{document.path}</p>
        </div>
        <span className="text-sm text-slate-600">{document.dirty ? "Unsaved" : "Saved"}</span>
        <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void refreshYaml()}>
          Refresh YAML
        </button>
        <button className="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white" disabled={busy || !document.dirty} onClick={() => void save()}>
          Save
        </button>
        <button className="rounded bg-emerald-700 px-3 py-1.5 text-sm font-medium text-white" disabled={busy || !document.authoredRoot} onClick={() => void plan()}>
          Plan
        </button>
        <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void close()}>
          Close
        </button>
      </header>
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_24rem]">
        <div className="min-h-0 overflow-y-auto p-5">
          <section className="rounded border border-slate-200 bg-white p-4">
            <dl className="grid grid-cols-[10rem_1fr] gap-2 text-sm">
              <dt className="font-medium text-slate-500">ID</dt><dd>{document.configuration.id}</dd>
              <dt className="font-medium text-slate-500">Device plan</dt><dd>{document.configuration.devicePlan}</dd>
              <dt className="font-medium text-slate-500">Selected recipes</dt>
              <dd>{document.configuration.selectedRecipes.length === 0 ? "None (explicit empty selection)" : document.configuration.selectedRecipes.join(", ")}</dd>
            </dl>
          </section>
          <RuntimeConfigurationScreen
            disabled={busy}
            document={document}
            onBind={updateBindingValue}
          />
          <section className="mt-5 rounded border border-slate-200 bg-white p-4">
            <h2 className="font-semibold">Bindings</h2>
            <div className="mt-3 space-y-3">
              {Object.entries(document.configuration.bindings).map(([key, value]) => (
                <BindingEditor disabled={busy} bindingKey={key} key={key} value={value} onUpdate={updateBinding} />
              ))}
              {Object.keys(document.configuration.bindings).length === 0 ? (
                <p className="text-sm text-slate-500">No saved bindings.</p>
              ) : null}
            </div>
          </section>
        </div>
        <aside className="min-h-0 overflow-y-auto border-l border-slate-200 bg-white p-4">
          <h2 className="font-semibold">Diagnostics ({document.diagnostics.length})</h2>
          <div className="mt-3 space-y-2">
            {document.diagnostics.map((diagnostic, index) => (
              <article className="rounded border border-slate-200 p-3 text-sm" key={`${diagnostic.code}-${index}`}>
                <div className="flex gap-2 text-xs"><strong>{diagnostic.severity}</strong><code>{diagnostic.code}</code></div>
                <p className="mt-2">{diagnostic.message}</p>
                <p className="mt-1 text-xs text-slate-500">{diagnostic.key ?? "Configuration"} / {diagnostic.provenance}</p>
              </article>
            ))}
            {document.diagnostics.length === 0 ? <p className="text-sm text-slate-500">No diagnostics.</p> : null}
          </div>
          <h2 className="mt-6 font-semibold">Canonical YAML</h2>
          <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs text-slate-100">{document.yaml}</pre>
          {plannedJson ? (
            <>
              <h2 className="mt-6 font-semibold">In-memory plan result</h2>
              <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs text-slate-100">{plannedJson}</pre>
            </>
          ) : null}
        </aside>
      </div>
    </main>
  );
}

function BindingEditor({
  bindingKey,
  disabled,
  onUpdate,
  value,
}: {
  bindingKey: string;
  disabled: boolean;
  onUpdate: (key: string, text: string) => Promise<void>;
  value: unknown;
}) {
  const [text, setText] = useState(formatBindingValue(value));
  return (
    <label className="block text-sm">
      <span className="font-mono text-xs text-slate-600">{bindingKey}</span>
      <div className="mt-1 flex gap-2">
        <input className="min-w-0 flex-1 rounded border border-slate-300 px-3 py-2 font-mono text-sm" disabled={disabled} value={text} onChange={(event) => setText(event.target.value)} />
        <button className="rounded border border-slate-300 px-3 py-2 text-sm" disabled={disabled || text === formatBindingValue(value)} type="button" onClick={() => void onUpdate(bindingKey, text)}>
          Apply
        </button>
      </div>
    </label>
  );
}

function formatBindingValue(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

function apiFailureMessage<T>(response: Exclude<EditorApiResult<T>, { kind: "success" }>, fallback: string): string {
  return response.kind === "api-error" ? response.error.message : response.message || fallback;
}

import { useEffect, useReducer, useRef } from "react";

import {
  beginAppGenerator,
  cancelAppGenerator,
  checkAppRecipeCollisions,
  chooseAppGeneratorAnalyzer,
  chooseAppGeneratorApk,
  chooseAppGeneratorAuthoredRoot,
  generateAppRecipeDraft,
  inspectAppGeneratorApk,
  saveGeneratedAppRecipe,
  type EditorApiResult,
} from "../api/editorApi";
import type {
  ApkInspectionFactsDto,
  AppDefinitionV1Dto,
  AppMappingEditsDto,
  AppRecipeCollisionResult,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppRecipeSaveResult,
} from "../api/types";
import {
  formToRequest,
  initialAppGeneratorState,
  reduceAppGenerator,
  type AppGeneratorFormState,
} from "./appGenerator.logic";

interface AppGeneratorProps {
  onClose: () => void;
  onSaved: (result: AppRecipeSaveResult) => void;
}

/** Guided local-APK app-definition and recipe generator. */
export function AppGenerator({ onClose, onSaved }: AppGeneratorProps) {
  const [state, dispatch] = useReducer(reduceAppGenerator, initialAppGeneratorState);
  const savedRef = useRef(false);

  useEffect(() => {
    let active = true;
    void beginAppGenerator().then((response) => {
      if (!active) return;
      if (response.kind === "success") {
        dispatch({ type: "started", sessionHandle: response.result.sessionHandle });
      } else {
        dispatch({ type: "failure", message: apiFailure(response) });
      }
    });
    return () => {
      active = false;
    };
  }, []);

  async function chooseApk() {
    if (!state.sessionHandle) return;
    const response = await chooseAppGeneratorApk(state.sessionHandle);
    if (response.kind === "success") {
      if (!response.result.cancelled && response.result.apkHandle && response.result.label) {
        dispatch({ type: "apk-selected", apkHandle: response.result.apkHandle, label: response.result.label });
      }
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function chooseAnalyzer() {
    if (!state.sessionHandle) return;
    const response = await chooseAppGeneratorAnalyzer(state.sessionHandle, state.analyzerKind);
    if (response.kind === "success") {
      if (!response.result.cancelled && response.result.analyzerHandle && response.result.label) {
        dispatch({
          type: "analyzer-selected",
          analyzerHandle: response.result.analyzerHandle,
          label: response.result.label,
        });
      }
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function inspectApk() {
    if (!state.sessionHandle || !state.apkHandle || !state.analyzerHandle) return;
    dispatch({ type: "inspecting" });
    const inspected = await inspectAppGeneratorApk(
      state.sessionHandle,
      state.apkHandle,
      state.analyzerHandle,
    );
    if (inspected.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(inspected) });
      return;
    }
    dispatch({ type: "inspected", inspection: inspected.result });
    if (inspected.result.blocking) return;
    const drafted = await generateAppRecipeDraft(
      state.sessionHandle,
      state.apkHandle,
      null,
      null,
      null,
    );
    if (drafted.kind === "success") {
      dispatch({ type: "drafted", draft: drafted.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(drafted) });
    }
  }

  async function chooseRoot() {
    if (!state.sessionHandle) return;
    const response = await chooseAppGeneratorAuthoredRoot(state.sessionHandle);
    if (response.kind === "success") {
      if (!response.result.cancelled && response.result.rootHandle && response.result.label) {
        dispatch({ type: "root-selected", rootHandle: response.result.rootHandle, label: response.result.label });
      }
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function buildReview(regenerateIdentifiers = false) {
    if (!state.sessionHandle || !state.apkHandle || !state.rootHandle || !state.form) return;
    const request = formToRequest(state.form);
    if (!request.ok) {
      dispatch({ type: "failure", message: request.message });
      return;
    }
    const drafted = await generateAppRecipeDraft(
      state.sessionHandle,
      state.apkHandle,
      request.app,
      request.recipe,
      request.mappings,
      regenerateIdentifiers,
    );
    if (drafted.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(drafted) });
      return;
    }
    const collisions = await checkAppRecipeCollisions(
      state.sessionHandle,
      state.rootHandle,
      drafted.result.app,
      drafted.result.recipe.id,
    );
    if (collisions.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(collisions) });
      return;
    }
    dispatch({ type: "reviewed", draft: drafted.result, collisions: collisions.result });
  }

  async function regenerateIds() {
    if (!state.rootHandle) {
      dispatch({ type: "failure", message: "Choose an authored root before regenerating and reviewing IDs." });
      return;
    }
    await buildReview(true);
  }

  async function save() {
    if (!state.sessionHandle || !state.apkHandle || !state.rootHandle || !state.form) return;
    const request = formToRequest(state.form);
    if (!request.ok) {
      dispatch({ type: "failure", message: request.message });
      return;
    }
    dispatch({ type: "saving" });
    const response = await saveGeneratedAppRecipe(
      state.sessionHandle,
      state.apkHandle,
      state.rootHandle,
      request.app,
      request.recipe,
      request.mappings,
    );
    if (response.kind === "success") {
      savedRef.current = true;
      dispatch({ type: "saved", result: response.result });
      onSaved(response.result);
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function close() {
    if (state.sessionHandle && !savedRef.current) {
      await cancelAppGenerator(state.sessionHandle);
    }
    onClose();
  }

  function updateForm(update: (form: AppGeneratorFormState) => void) {
    if (!state.form) return;
    const form = structuredClone(state.form);
    update(form);
    dispatch({ type: "form", form });
  }

  function changeApp(update: Partial<AppDefinitionV1Dto>) {
    updateForm((form) => {
      form.app = { ...form.app, ...update };
    });
  }

  function changeRecipe(update: Partial<AppRecipeEditsDto>) {
    updateForm((form) => {
      form.recipe = { ...form.recipe, ...update };
    });
  }

  function changeMapping(update: Partial<AppMappingEditsDto>) {
    updateForm((form) => {
      form.mappings = { ...form.mappings, ...update };
    });
  }

  const busy = state.phase === "starting" || state.phase === "inspecting" || state.phase === "saving";
  const saveBlocked =
    busy ||
    !state.draft ||
    state.draft.blocking ||
    !state.collisions ||
    state.collisions.blocking;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/50 p-4">
      <section className="flex max-h-[94vh] w-full max-w-6xl flex-col overflow-hidden rounded-lg bg-white shadow-2xl">
        <header className="flex items-center justify-between border-b border-slate-200 px-5 py-3">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">Local APK generator</p>
            <h1 className="text-lg font-semibold">Generate App and Recipe</h1>
          </div>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void close()}>
            Cancel
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {state.error ? <p className="mb-4 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">{state.error}</p> : null}

          <section className="grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-3">
            <Picker label="APK" value={state.apkLabel} button="Choose APK..." disabled={busy} onClick={() => void chooseApk()} />
            <div>
              <label className="text-xs font-semibold uppercase tracking-wide text-slate-500" htmlFor="apk-analyzer-kind">Analyzer type</label>
              <select
                id="apk-analyzer-kind"
                className="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
                disabled={busy}
                value={state.analyzerKind}
                onChange={(event) => dispatch({ type: "analyzer-kind", kind: event.target.value as "apkanalyzer" | "aapt2" })}
              >
                <option value="apkanalyzer">apkanalyzer</option>
                <option value="aapt2">aapt2</option>
              </select>
              <button className="mt-2 rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void chooseAnalyzer()}>
                Choose executable...
              </button>
              <p className="mt-1 text-xs text-slate-500">{state.analyzerLabel ?? "No analyzer configured"}</p>
            </div>
            <div className="flex items-end">
              <button
                className="w-full rounded bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                disabled={busy || !state.apkHandle || !state.analyzerHandle}
                onClick={() => void inspectApk()}
              >
                {state.phase === "inspecting" ? "Inspecting..." : "Inspect APK"}
              </button>
            </div>
          </section>

          {state.inspection ? <Facts facts={state.inspection.facts} diagnostics={state.inspection.diagnostics} /> : null}
          {state.form ? (
            <>
              <AppFields form={state.form} disabled={busy} updateForm={updateForm} changeApp={changeApp} changeMapping={changeMapping} />
              <RecipeFields form={state.form} facts={state.inspection?.facts ?? null} disabled={busy} changeRecipe={changeRecipe} regenerateIds={() => void regenerateIds()} />
              <section className="mt-4 grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-[1fr_auto]">
                <Picker label="Authored root" value={state.rootLabel} button="Choose authored root..." disabled={busy} onClick={() => void chooseRoot()} />
                <button
                  className="self-end rounded bg-indigo-700 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                  disabled={busy || !state.rootHandle}
                  onClick={() => void buildReview()}
                >
                  Validate and review
                </button>
              </section>
            </>
          ) : null}

          {state.draft ? <Review draft={state.draft} collisions={state.collisions} /> : null}
        </div>

        <footer className="flex items-center justify-between border-t border-slate-200 px-5 py-3">
          <p className="text-xs text-slate-500">Files are created only after explicit save and are never overwritten.</p>
          <button
            className="rounded bg-emerald-700 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
            disabled={saveBlocked}
            onClick={() => void save()}
          >
            {state.phase === "saving" ? "Saving..." : "Save both and open recipe"}
          </button>
        </footer>
      </section>
    </div>
  );
}

function Picker({ label, value, button, disabled, onClick }: { label: string; value: string | null; button: string; disabled: boolean; onClick: () => void }) {
  return (
    <div>
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</p>
      <button className="mt-1 rounded border border-slate-300 px-3 py-2 text-sm" disabled={disabled} onClick={onClick}>{button}</button>
      <p className="mt-1 text-xs text-slate-500">{value ?? "Not selected"}</p>
    </div>
  );
}

function Facts({ facts, diagnostics }: { facts: ApkInspectionFactsDto; diagnostics: Array<{ code: string; message: string; severity: string }> }) {
  const rows: Array<[string, string]> = [
    ["Package", facts.packageName ?? "Missing"],
    ["Label", facts.applicationLabel ?? "Missing"],
    ["Version", [facts.versionName, facts.versionCode].filter(Boolean).join(" / ") || "Missing"],
    ["SDK", `min ${facts.minSdk ?? "?"}, target ${facts.targetSdk ?? "?"}`],
    ["ABIs", facts.abis.join(", ") || "Missing"],
    ["Launchers", facts.launcherActivities.join(", ") || "None verified"],
    ["Permissions", facts.requestedPermissions.join(", ") || "None reported"],
    ["Debuggable", facts.debuggable === null ? "Missing" : String(facts.debuggable)],
    ["Certificate SHA-256", facts.certificateSha256 ?? "Missing"],
  ];
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Verified APK facts</h2>
      <dl className="mt-3 grid grid-cols-[9rem_1fr] gap-2 text-sm">
        {rows.map(([label, value]) => <div className="contents" key={label}><dt className="font-medium text-slate-500">{label}</dt><dd className="break-all">{value}</dd></div>)}
      </dl>
      <Diagnostics items={diagnostics} />
    </section>
  );
}

function AppFields({ form, disabled, updateForm, changeApp, changeMapping }: {
  form: AppGeneratorFormState;
  disabled: boolean;
  updateForm: (update: (form: AppGeneratorFormState) => void) => void;
  changeApp: (update: Partial<AppDefinitionV1Dto>) => void;
  changeMapping: (update: Partial<AppMappingEditsDto>) => void;
}) {
  const app = form.app;
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">App definition</h2>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Fixed label="Schema / kind" value="1 / app_definition" />
        <Field label="App ID" value={app.id} disabled={disabled} onChange={(id) => changeApp({ id })} />
        <Field label="Name" value={app.name} disabled={disabled} onChange={(name) => changeApp({ name })} />
        <Field label="Category (required)" value={app.category} disabled={disabled} onChange={(category) => changeApp({ category })} />
        <Field label="Description" value={app.description ?? ""} disabled={disabled} onChange={(description) => changeApp({ description: description || undefined })} />
        <Field label="Primary package" value={app.package.primary} disabled={disabled} onChange={(primary) => updateForm((next) => { next.app.package.primary = primary; })} />
        <Area label="Package aliases (one per line)" value={form.aliasesText} disabled={disabled} onChange={(aliasesText) => updateForm((next) => { next.aliasesText = aliasesText; })} />
        <Field label="Install source type" value={app.install_source.type} disabled={disabled} onChange={(value) => updateForm((next) => { next.app.install_source.type = value; })} />
        <Field label="Install source resolver" value={app.install_source.resolver} disabled={disabled} onChange={(value) => updateForm((next) => { next.app.install_source.resolver = value; })} />
        <Field label="Tracking source type" value={String(app.tracking_source.type)} disabled={disabled} onChange={(value) => updateForm((next) => { next.app.tracking_source.type = value; })} />
        <Area label="Install-source options (strict JSON object)" value={form.mappings.installSourceOptions} disabled={disabled} onChange={(installSourceOptions) => changeMapping({ installSourceOptions })} />
        <Area label="Tracking-source fields (strict JSON object)" value={form.mappings.trackingSourceFields} disabled={disabled} onChange={(trackingSourceFields) => changeMapping({ trackingSourceFields })} />
        <Area label="Metadata (strict JSON object)" value={form.mappings.metadata} disabled={disabled} onChange={(metadata) => changeMapping({ metadata })} />
        <Area label="Shared-storage paths" value={form.sharedStoragePathsText} disabled={disabled} onChange={(sharedStoragePathsText) => updateForm((next) => { next.sharedStoragePathsText = sharedStoragePathsText; })} />
        <Area label="App-data paths" value={form.appDataPathsText} disabled={disabled} onChange={(appDataPathsText) => updateForm((next) => { next.appDataPathsText = appDataPathsText; })} />
      </div>
      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <MappingList
          disabled={disabled}
          label="Input metadata objects"
          values={form.mappings.inputs}
          onChange={(inputs) => changeMapping({ inputs })}
        />
        <MappingList
          disabled={disabled}
          label="Provisioning config-target objects"
          values={form.mappings.configTargets}
          onChange={(configTargets) => changeMapping({ configTargets })}
        />
      </div>
      <div className="mt-3 grid gap-2 md:grid-cols-4">
        <Check label="APK required" checked={app.artifacts.apk.required} disabled={disabled} onChange={(required) => updateForm((next) => { next.app.artifacts.apk.required = required; })} />
        <Check label="BYO APK required" checked={app.artifacts.byo_apk.required} disabled={disabled} onChange={(required) => updateForm((next) => { next.app.artifacts.byo_apk.required = required; })} />
        <Check label="Shared config" checked={app.artifacts.shared_storage_config.supported} disabled={disabled} onChange={(supported) => updateForm((next) => { next.app.artifacts.shared_storage_config.supported = supported; })} />
        <Check label="App-data config" checked={app.artifacts.app_data_config.supported} disabled={disabled} onChange={(supported) => updateForm((next) => { next.app.artifacts.app_data_config.supported = supported; })} />
      </div>
    </section>
  );
}

function RecipeFields({ form, facts, disabled, changeRecipe, regenerateIds }: {
  form: AppGeneratorFormState;
  facts: ApkInspectionFactsDto | null;
  disabled: boolean;
  changeRecipe: (update: Partial<AppRecipeEditsDto>) => void;
  regenerateIds: () => void;
}) {
  const recipe = form.recipe;
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">Recipe</h2>
        <button className="rounded border border-slate-300 px-3 py-1.5 text-xs" disabled={disabled} onClick={regenerateIds}>Regenerate IDs from app ID</button>
      </div>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Fixed label="Schema / kind" value="1 / recipe" />
        <Fixed label="Recipe ID" value={recipe.ids?.recipeId ?? "Pending"} />
        <Field label="Recipe name" value={recipe.name} disabled={disabled} onChange={(name) => changeRecipe({ name })} />
        <Field label="Recipe description" value={recipe.description} disabled={disabled} onChange={(description) => changeRecipe({ description })} />
        <Field label="APK input label" value={recipe.inputLabel} disabled={disabled} onChange={(inputLabel) => changeRecipe({ inputLabel })} />
        <Field label="APK input description" value={recipe.inputDescription} disabled={disabled} onChange={(inputDescription) => changeRecipe({ inputDescription })} />
      </div>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Check label="Replace existing installation" checked={recipe.replaceExisting} disabled={disabled} onChange={(replaceExisting) => changeRecipe({ replaceExisting })} />
        <Check label="Launch once after installation" checked={recipe.launchEnabled} disabled={disabled || !facts?.launcherActivities.length} onChange={(launchEnabled) => changeRecipe({ launchEnabled, launcherActivity: launchEnabled ? facts?.launcherActivities[0] ?? null : null })} />
      </div>
      {recipe.launchEnabled ? (
        <label className="mt-3 block text-sm">
          <span className="font-medium">Verified launcher component</span>
          <select className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={disabled} value={recipe.launcherActivity ?? ""} onChange={(event) => changeRecipe({ launcherActivity: event.target.value || null })}>
            {(facts?.launcherActivities ?? []).map((value) => <option key={value} value={value}>{value}</option>)}
          </select>
        </label>
      ) : null}
    </section>
  );
}

function Review({ draft, collisions }: { draft: AppRecipeDraftResult; collisions: AppRecipeCollisionResult | null }) {
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Validation, collisions, and canonical YAML</h2>
      <Diagnostics items={draft.diagnostics} />
      <Diagnostics items={(collisions?.collisions ?? []).map((item) => ({ ...item, severity: item.severity === "blocking" ? "error" : "warning" }))} />
      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <Yaml title={draft.appDestination.relativePath ?? "App definition"} value={draft.appCanonicalYaml} />
        <Yaml title={draft.recipeDestination.relativePath ?? "Recipe"} value={draft.recipeCanonicalYaml} />
      </div>
    </section>
  );
}

function Diagnostics({ items }: { items: Array<{ code: string; message: string; severity: string }> }) {
  if (items.length === 0) return <p className="mt-3 text-sm text-emerald-700">No diagnostics.</p>;
  return <div className="mt-3 space-y-2">{items.map((item, index) => <p className={`rounded border p-2 text-sm ${item.severity === "error" || item.severity === "blocking" ? "border-red-300 bg-red-50" : "border-amber-300 bg-amber-50"}`} key={`${item.code}-${index}`}><strong>{item.code}</strong>: {item.message}</p>)}</div>;
}

function Yaml({ title, value }: { title: string; value: string | null }) {
  return <div><h3 className="text-sm font-semibold">{title}</h3><pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs text-slate-100">{value ?? "Resolve validation errors to preview canonical YAML."}</pre></div>;
}

function Field({ label, value, disabled, onChange }: { label: string; value: string; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="text-sm"><span className="font-medium">{label}</span><input className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={disabled} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Area({ label, value, disabled, onChange }: { label: string; value: string; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="text-sm"><span className="font-medium">{label}</span><textarea className="mt-1 min-h-24 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs" disabled={disabled} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Fixed({ label, value }: { label: string; value: string }) {
  return <div className="text-sm"><p className="font-medium">{label}</p><p className="mt-1 rounded bg-slate-100 px-3 py-2 font-mono text-xs">{value}</p></div>;
}

function Check({ label, checked, disabled, onChange }: { label: string; checked: boolean; disabled: boolean; onChange: (value: boolean) => void }) {
  return <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />{label}</label>;
}

function MappingList({ label, values, disabled, onChange }: { label: string; values: string[]; disabled: boolean; onChange: (values: string[]) => void }) {
  return (
    <div>
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium">{label}</p>
        <button className="rounded border border-slate-300 px-2 py-1 text-xs" disabled={disabled} onClick={() => onChange([...values, "{}"])}>Add object</button>
      </div>
      <div className="mt-2 space-y-2">
        {values.map((value, index) => (
          <div className="grid grid-cols-[1fr_auto] gap-2" key={index}>
            <textarea
              aria-label={`${label} ${index + 1}`}
              className="min-h-24 rounded border border-slate-300 px-3 py-2 font-mono text-xs"
              disabled={disabled}
              value={value}
              onChange={(event) => onChange(values.map((item, itemIndex) => itemIndex === index ? event.target.value : item))}
            />
            <button className="self-start rounded border border-red-300 px-2 py-1 text-xs text-red-700" disabled={disabled} onClick={() => onChange(values.filter((_, itemIndex) => itemIndex !== index))}>Remove</button>
          </div>
        ))}
        {values.length === 0 ? <p className="text-xs text-slate-500">No objects.</p> : null}
      </div>
    </div>
  );
}

function apiFailure<T>(response: Exclude<EditorApiResult<T>, { kind: "success" }>): string {
  return response.kind === "api-error" ? response.error.message : response.message;
}

import { useEffect, useReducer, useRef, type Dispatch } from "react";

import {
  beginAppGenerator,
  analyzeAppGeneratorSource,
  cancelAppGenerator,
  checkAppRecipeCollisions,
  chooseAppGeneratorAnalyzer,
  chooseAppGeneratorApk,
  chooseAppGeneratorAuthoredRoot,
  downloadAppGeneratorRemoteApk,
  setAppGeneratorAuthoredRoot,
  generateAppRecipeDraft,
  generateRemoteAppRecipeDraft,
  inspectAppGeneratorApk,
  saveGeneratedAppRecipe,
  saveGeneratedRemoteAppRecipe,
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
  AppGeneratorSourceMode,
  RemoteSourceDescriptorDto,
} from "../api/types";
import {
  diagnosticDisplayTitle,
  formToRequest,
  initialAppGeneratorState,
  reduceAppGenerator,
  visibleDraftDiagnostics,
  type AppGeneratorFormState,
  type AppGeneratorAction,
  type AppGeneratorState,
} from "./appGenerator.logic";

interface AppGeneratorProps {
  initialAuthoredRoot: string | null;
  onAuthoredRootSelected: (path: string) => void | Promise<void>;
  onClose: () => void;
  onSaved: (result: AppRecipeSaveResult) => void;
}

/** Guided local-APK app-definition and recipe generator. */
export function AppGenerator({ initialAuthoredRoot, onAuthoredRootSelected, onClose, onSaved }: AppGeneratorProps) {
  const [state, dispatch] = useReducer(reduceAppGenerator, initialAppGeneratorState);
  const savedRef = useRef(false);
  const initialAuthoredRootRef = useRef(initialAuthoredRoot);

  useEffect(() => {
    let active = true;
    void (async () => {
      const response = await beginAppGenerator();
      if (!active) return;
      if (response.kind !== "success") {
        dispatch({ type: "failure", message: apiFailure(response) });
        return;
      }
      let rootHandle = response.result.rootHandle;
      let rootLabel = response.result.rootLabel;
      if (initialAuthoredRootRef.current) {
        const bound = await setAppGeneratorAuthoredRoot(
          response.result.sessionHandle,
          initialAuthoredRootRef.current,
        );
        if (!active) return;
        if (bound.kind === "success") {
          rootHandle = bound.result.rootHandle;
          rootLabel = bound.result.label;
        }
      }
      dispatch({
        type: "started",
        sessionHandle: response.result.sessionHandle,
        analyzerHandle: response.result.analyzerHandle,
        analyzerKind: response.result.analyzerKind,
        analyzerLabel: response.result.analyzerLabel,
        rootHandle,
        rootLabel,
      });
    })();
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

  async function analyzeRemoteSource() {
    if (!state.sessionHandle || state.sourceMode === "local_apk" || !state.sourceUrl.trim()) return;
    dispatch({ type: "source-analyzing" });
    const response = await analyzeAppGeneratorSource(
      state.sessionHandle,
      state.sourceMode,
      state.sourceUrl,
      state.includePrereleases,
    );
    if (response.kind === "success") {
      dispatch({ type: "source-analyzed", analysis: response.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function downloadRemoteApk() {
    if (!state.sessionHandle || !state.selectedAssetHandle || !state.analyzerHandle) return;
    const selectedAsset = state.sourceAnalysis?.assets.find((asset) => asset.assetHandle === state.selectedAssetHandle);
    if (selectedAsset?.prerelease && !window.confirm("This release is marked as a prerelease. Continue with this APK?")) return;
    dispatch({ type: "downloading" });
    const downloaded = await downloadAppGeneratorRemoteApk(
      state.sessionHandle,
      state.selectedAssetHandle,
    );
    if (downloaded.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(downloaded) });
      return;
    }
    const source: RemoteSourceDescriptorDto = {
      ...downloaded.result.source,
      strategy: state.installStrategy,
    };
    dispatch({
      type: "remote-downloaded",
      apkHandle: downloaded.result.apkHandle,
      label: downloaded.result.label,
      source,
    });
    await inspectApkHandle(downloaded.result.apkHandle, state.selectedAssetHandle, source);
  }

  async function inspectApk() {
    if (!state.apkHandle) return;
    await inspectApkHandle(state.apkHandle, state.selectedAssetHandle, state.remoteSource);
  }

  async function inspectApkHandle(
    apkHandle: string,
    assetHandle: string | null,
    remoteSource: RemoteSourceDescriptorDto | null,
  ) {
    if (!state.sessionHandle || !state.analyzerHandle) return;
    dispatch({ type: "inspecting" });
    const inspected = await inspectAppGeneratorApk(
      state.sessionHandle,
      apkHandle,
      state.analyzerHandle,
    );
    if (inspected.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(inspected) });
      return;
    }
    dispatch({ type: "inspected", inspection: inspected.result });
    if (inspected.result.blocking) return;
    const drafted = remoteSource && assetHandle
      ? await generateRemoteAppRecipeDraft(
          state.sessionHandle,
          apkHandle,
          assetHandle,
          remoteSource.strategy,
          null,
          null,
          null,
        )
      : await generateAppRecipeDraft(
          state.sessionHandle,
          apkHandle,
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
        if (response.result.path) await onAuthoredRootSelected(response.result.path);
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
    const drafted = state.sourceMode === "local_apk"
      ? await generateAppRecipeDraft(
          state.sessionHandle,
          state.apkHandle,
          request.app,
          request.recipe,
          request.mappings,
          regenerateIdentifiers,
        )
      : state.selectedAssetHandle
        ? await generateRemoteAppRecipeDraft(
            state.sessionHandle,
            state.apkHandle,
            state.selectedAssetHandle,
            state.installStrategy,
            request.app,
            request.recipe,
            request.mappings,
            regenerateIdentifiers,
          )
        : null;
    if (!drafted) {
      dispatch({ type: "failure", message: "Select and download an APK asset before reviewing." });
      return;
    }
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
    const response = state.sourceMode === "local_apk"
      ? await saveGeneratedAppRecipe(
          state.sessionHandle,
          state.apkHandle,
          state.rootHandle,
          request.app,
          request.recipe,
          request.mappings,
        )
      : state.selectedAssetHandle
        ? await saveGeneratedRemoteAppRecipe(
            state.sessionHandle,
            state.apkHandle,
            state.selectedAssetHandle,
            state.installStrategy,
            state.rootHandle,
            request.app,
            request.recipe,
            request.mappings,
          )
        : null;
    if (!response) {
      dispatch({ type: "failure", message: "Select and download an APK asset before saving." });
      return;
    }
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

  const busy =
    state.phase === "starting" ||
    state.phase === "downloading" ||
    state.phase === "inspecting" ||
    state.phase === "saving";
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
            <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">App source generator</p>
            <h1 className="text-lg font-semibold">Generate App and Recipe</h1>
          </div>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void close()}>
            Cancel
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {state.error ? <p className="mb-4 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">{state.error}</p> : null}

          <section className="rounded border border-slate-200 p-4">
            <label className="text-sm font-medium" htmlFor="app-source-mode">App source</label>
            <select
              id="app-source-mode"
              className="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm"
              disabled={busy}
              value={state.sourceMode}
              onChange={(event) => dispatch({ type: "source-mode", mode: event.target.value as AppGeneratorSourceMode })}
            >
              <option value="local_apk">Local APK</option>
              <option value="github_repository">GitHub repository</option>
              <option value="github_release">GitHub release</option>
              <option value="direct_apk">Direct APK URL</option>
            </select>
            <p className="mt-1 text-xs text-slate-500">Choose where the APK and update identity come from.</p>

            {state.sourceMode === "local_apk" ? (
              <div className="mt-4 grid gap-4 md:grid-cols-3">
                <Picker label="APK" value={state.apkLabel} button="Choose APK..." disabled={busy} onClick={() => void chooseApk()} />
                <AnalyzerPicker state={state} busy={busy} chooseAnalyzer={chooseAnalyzer} dispatch={dispatch} />
                <div className="flex items-end">
                  <button className="w-full rounded bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300" disabled={busy || !state.apkHandle || !state.analyzerHandle} onClick={() => void inspectApk()}>
                    {state.phase === "inspecting" ? "Inspecting..." : "Inspect APK"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="mt-4 space-y-4">
                <div className="grid gap-3 md:grid-cols-[1fr_auto]">
                  <label className="text-sm">
                    <HelpLabel label={state.sourceMode === "direct_apk" ? "APK download URL" : "GitHub URL"} help={state.sourceMode === "github_repository" ? "Enter a public GitHub repository URL." : state.sourceMode === "github_release" ? "Enter a public GitHub release URL ending in /releases/tag/<tag>." : "Enter a public HTTPS URL that ends in .apk."} />
                    <input className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={busy} value={state.sourceUrl} onChange={(event) => dispatch({ type: "source-url", value: event.target.value })} />
                  </label>
                  <button className="self-end rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300" disabled={busy || !state.sourceUrl.trim()} onClick={() => void analyzeRemoteSource()}>
                    {state.phase === "inspecting" && !state.apkHandle ? "Checking source..." : "Check source"}
                  </button>
                </div>
                {state.sourceMode === "github_repository" ? <Check label="Include prereleases" checked={state.includePrereleases} disabled={busy} onChange={(value) => dispatch({ type: "include-prereleases", value })} /> : null}
                {state.sourceAnalysis ? <RemoteAssetPicker analysis={state.sourceAnalysis} selectedAssetHandle={state.selectedAssetHandle} disabled={busy} onSelect={(assetHandle) => dispatch({ type: "asset-selected", assetHandle })} /> : null}
                <div className="grid gap-4 md:grid-cols-3">
                  <AnalyzerPicker state={state} busy={busy} chooseAnalyzer={chooseAnalyzer} dispatch={dispatch} />
                  <label className="text-sm">
                    <HelpLabel label="Installation method" help="Pinned download creates a recipe that downloads this exact APK. User-provided APK creates a local file input instead." />
                    <select className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={busy} value={state.installStrategy} onChange={(event) => dispatch({ type: "install-strategy", strategy: event.target.value as "pinned_remote_asset" | "user_provided_apk" })}>
                      <option value="pinned_remote_asset">Pinned download</option>
                      <option value="user_provided_apk">User-provided APK</option>
                    </select>
                  </label>
                  <div className="flex items-end">
                    <button
                      className="flex w-full items-center justify-center gap-2 rounded bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                      disabled={busy || !state.selectedAssetHandle || !state.analyzerHandle}
                      onClick={() => void downloadRemoteApk()}
                    >
                      {state.phase === "downloading" || state.phase === "inspecting" ? (
                        <span
                          aria-hidden="true"
                          className="h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white"
                        />
                      ) : null}
                      {state.phase === "downloading"
                        ? "Downloading APK..."
                        : state.phase === "inspecting"
                          ? "Inspecting APK..."
                          : "Download and inspect APK"}
                    </button>
                  </div>
                  {state.phase === "downloading" ? (
                    <p className="text-xs text-slate-600 md:col-span-3" role="status" aria-live="polite">
                      Downloading the selected APK. This may take a moment.
                    </p>
                  ) : null}
                </div>
              </div>
            )}
          </section>

          {state.inspection ? <Facts facts={state.inspection.facts} diagnostics={state.inspection.diagnostics} /> : null}
          {state.form ? (
            <>
              <AppFields sourceMode={state.sourceMode} installStrategy={state.installStrategy} form={state.form} disabled={busy} updateForm={updateForm} changeApp={changeApp} changeMapping={changeMapping} />
              <RecipeFields form={state.form} facts={state.inspection?.facts ?? null} disabled={busy} changeRecipe={changeRecipe} regenerateIds={() => void regenerateIds()} />
              <section className="mt-4 grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-[1fr_auto]">
                <Picker label="Authored root" value={state.rootHandle ? "Selected and ready" : null} button={state.rootHandle ? "Change authored root..." : "Choose authored root..."} disabled={busy} onClick={() => void chooseRoot()} />
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

          {state.draft ? <Review draft={state.draft} collisions={state.collisions} hasSelectedRoot={state.rootHandle !== null} /> : null}
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

function AnalyzerPicker({ state, busy, chooseAnalyzer, dispatch }: { state: AppGeneratorState; busy: boolean; chooseAnalyzer: () => Promise<void>; dispatch: Dispatch<AppGeneratorAction> }) {
  return <div><label className="text-xs font-semibold uppercase tracking-wide text-slate-500" htmlFor="apk-analyzer-kind">Analyzer type</label><select id="apk-analyzer-kind" className="mt-1 w-full rounded border border-slate-300 px-3 py-2 text-sm" disabled={busy} value={state.analyzerKind} onChange={(event) => dispatch({ type: "analyzer-kind", kind: event.target.value as "apkanalyzer" | "aapt2" })}><option value="apkanalyzer">apkanalyzer</option><option value="aapt2">aapt2</option></select><button className="mt-2 rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={busy} onClick={() => void chooseAnalyzer()}>{state.analyzerHandle ? "Change executable..." : "Choose executable..."}</button><p className="mt-1 text-xs text-slate-500">{state.analyzerLabel ?? "No analyzer configured"}</p></div>;
}

function RemoteAssetPicker({ analysis, selectedAssetHandle, disabled, onSelect }: { analysis: import("../api/types").RemoteSourceAnalysisResult; selectedAssetHandle: string | null; disabled: boolean; onSelect: (assetHandle: string) => void }) {
  return <section className="rounded border border-slate-200 bg-slate-50 p-3"><h3 className="text-sm font-semibold">Available APK files</h3>{analysis.repository ? <p className="mt-1 text-sm text-slate-600">{analysis.repository.fullName}{analysis.repository.description ? ` — ${analysis.repository.description}` : ""}</p> : null}<div className="mt-3 space-y-2">{analysis.assets.map((asset) => <label className="flex items-start gap-2 rounded border border-slate-200 bg-white p-3 text-sm" key={asset.assetHandle}><input type="radio" name="remote-apk-asset" checked={selectedAssetHandle === asset.assetHandle} disabled={disabled} onChange={() => onSelect(asset.assetHandle)} /><span><span className="font-medium">{asset.fileName}</span><span className="ml-2 text-xs text-slate-500">{asset.releaseTag ? `${asset.releaseTag}${asset.prerelease ? " (prerelease)" : ""}` : "Direct download"}{asset.size ? ` · ${Math.ceil(asset.size / 1024 / 1024)} MiB` : ""}</span></span></label>)}{analysis.assets.length === 0 ? <p className="text-sm text-amber-700">No eligible APK files were found.</p> : null}</div></section>;
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
      <Diagnostics label="APK inspection" emptyMessage="APK inspection completed successfully." items={diagnostics} />
    </section>
  );
}

function AppFields({ sourceMode, installStrategy, form, disabled, updateForm, changeApp, changeMapping }: {
  sourceMode: AppGeneratorSourceMode;
  installStrategy: "pinned_remote_asset" | "user_provided_apk";
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
        <Field label="App ID" help="Stable authored identifier used in filenames and references. Use lowercase letters, numbers, periods, underscores, or hyphens." value={app.id} disabled={disabled} onChange={(id) => changeApp({ id })} />
        <Field label="Name" help="Human-readable app name shown in EmuChef." value={app.name} disabled={disabled} onChange={(name) => changeApp({ name })} />
        <CategoryField value={app.category} disabled={disabled} onChange={(category) => changeApp({ category: category.toLowerCase() })} />
        <Field label="Description" help="Optional short description of the app." value={app.description ?? ""} disabled={disabled} onChange={(description) => changeApp({ description: description || undefined })} />
        <Field label="Primary package" help="Android application ID verified from the APK manifest. Change only when the manifest information is known to be wrong." value={app.package.primary} disabled={disabled} onChange={(primary) => updateForm((next) => { next.app.package.primary = primary; })} />
        <PackageAliasList values={form.aliases} disabled={disabled} onChange={(aliases) => updateForm((next) => { next.aliases = aliases; })} />
        <Fixed label="Installation method" help="Controls whether the generated recipe downloads this exact APK or asks the user to provide one." value={installStrategy === "pinned_remote_asset" ? "Pinned download" : "User-provided APK"} />
        <Fixed label="Source resolver" help="Describes how the generated app definition identifies its installation source." value={installStrategy === "pinned_remote_asset" ? "Direct HTTPS download" : "None required"} />
        <Fixed label="Update tracking" help="Describes the source identity retained for future catalog review." value={installStrategy === "user_provided_apk" ? "Local APK" : sourceMode.startsWith("github_") ? "GitHub release" : sourceMode === "direct_apk" ? "Direct APK URL" : "Local APK"} />
        <Area label="Install-source options (strict JSON object)" value={form.mappings.installSourceOptions} disabled={disabled} onChange={(installSourceOptions) => changeMapping({ installSourceOptions })} />
        <Area label="Tracking-source fields (strict JSON object)" value={form.mappings.trackingSourceFields} disabled={disabled} onChange={(trackingSourceFields) => changeMapping({ trackingSourceFields })} />
        <Area label="Metadata (strict JSON object)" value={form.mappings.metadata} disabled={disabled} onChange={(metadata) => changeMapping({ metadata })} />
        <StringListField label="Shared-storage paths" help="Shared-storage locations associated with the app, such as folders under Android shared storage." addLabel="Add path" emptyLabel="No shared-storage paths." values={form.sharedStoragePaths} disabled={disabled} onChange={(sharedStoragePaths) => updateForm((next) => { next.sharedStoragePaths = sharedStoragePaths; })} />
        <StringListField label="App-data paths" help="App-private or package-specific data locations associated with the app." addLabel="Add path" emptyLabel="No app-data paths." values={form.appDataPaths} disabled={disabled} onChange={(appDataPaths) => updateForm((next) => { next.appDataPaths = appDataPaths; })} />
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
      <section className="mt-5 rounded border border-slate-200 bg-slate-50 p-3">
        <h3 className="text-sm font-semibold text-slate-700">App capabilities</h3>
        <p className="mt-1 text-xs text-slate-500">Choose which files and configuration types this app definition supports.</p>
        <div className="mt-3 grid gap-3 md:grid-cols-2 lg:grid-cols-4">
          <Check label="APK required" checked={app.artifacts.apk.required} disabled={disabled} onChange={(required) => updateForm((next) => { next.app.artifacts.apk.required = required; })} />
          <Check label="BYO APK required" checked={app.artifacts.byo_apk.required} disabled={disabled} onChange={(required) => updateForm((next) => { next.app.artifacts.byo_apk.required = required; })} />
          <Check label="Shared config" checked={app.artifacts.shared_storage_config.supported} disabled={disabled} onChange={(supported) => updateForm((next) => { next.app.artifacts.shared_storage_config.supported = supported; })} />
          <Check label="App-data config" checked={app.artifacts.app_data_config.supported} disabled={disabled} onChange={(supported) => updateForm((next) => { next.app.artifacts.app_data_config.supported = supported; })} />
        </div>
      </section>
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
        <Fixed label="Recipe ID" help="Derived from the App ID as app.<app-id>.install. Use Regenerate IDs after changing the App ID." value={recipe.ids?.recipeId ?? "Pending"} />
        <Field label="Recipe name" help="Human-readable title for the installation recipe." value={recipe.name} disabled={disabled} onChange={(name) => changeRecipe({ name })} />
        <Field label="Recipe description" help="Short explanation of what the recipe installs." value={recipe.description} disabled={disabled} onChange={(description) => changeRecipe({ description })} />
        <Field label="APK input label" help="Label shown when EmuChef asks the user to choose the APK." value={recipe.inputLabel} disabled={disabled} onChange={(inputLabel) => changeRecipe({ inputLabel })} />
        <Field label="APK input description" help="Guidance shown beside the APK file input." value={recipe.inputDescription} disabled={disabled} onChange={(inputDescription) => changeRecipe({ inputDescription })} />
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

function Review({ draft, collisions, hasSelectedRoot }: { draft: AppRecipeDraftResult; collisions: AppRecipeCollisionResult | null; hasSelectedRoot: boolean }) {
  const validationItems = visibleDraftDiagnostics(draft.diagnostics, hasSelectedRoot);
  const appPath = draft.appDestination.relativePath ?? "App definition";
  const recipePath = draft.recipeDestination.relativePath ?? "Recipe";
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Review generated files</h2>
      {validationItems.length === 0 ? (
        <div className="mt-3 space-y-1 text-sm text-emerald-700">
          <p><strong>{appPath}</strong>: Valid app definition.</p>
          <p><strong>{recipePath}</strong>: Valid recipe.</p>
        </div>
      ) : <Diagnostics label="File checks" items={validationItems} />}
      <Diagnostics label="Existing catalog check" emptyMessage="No conflicting files or IDs found." items={(collisions?.collisions ?? []).map((item) => ({ ...item, severity: item.severity === "blocking" ? "error" : "warning" }))} />
      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <Yaml title={appPath} value={draft.appCanonicalYaml} />
        <Yaml title={recipePath} value={draft.recipeCanonicalYaml} />
      </div>
    </section>
  );
}

function Diagnostics({ label, items, emptyMessage }: { label: string; items: Array<{ code: string; message: string; severity: string }>; emptyMessage?: string }) {
  return <section className="mt-3"><h3 className="text-sm font-semibold text-slate-700">{label}</h3>{items.length === 0 ? <p className="mt-1 text-sm text-emerald-700">{emptyMessage ?? "Validation passed."}</p> : <div className="mt-2 space-y-2">{items.map((item, index) => <div className={`rounded border p-2 text-sm ${item.severity === "error" || item.severity === "blocking" ? "border-red-300 bg-red-50" : "border-amber-300 bg-amber-50"}`} key={`${item.code}-${index}`}><p className="font-semibold">{diagnosticDisplayTitle(item.code, item.severity)}</p><p className="mt-0.5">{item.message}</p></div>)}</div>}</section>;
}

function Yaml({ title, value }: { title: string; value: string | null }) {
  return <div><h3 className="text-sm font-semibold">{title}</h3><pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs text-slate-100">{value ?? "Fix the errors above to preview this file."}</pre></div>;
}

function HelpLabel({ label, help }: { label: string; help?: string }) {
  return <span className="inline-flex items-center gap-1 font-medium">{label}{help ? <span aria-label={`${label} help`} className="cursor-help rounded-full border border-slate-300 px-1 text-[10px] leading-4 text-slate-500" title={help}>?</span> : null}</span>;
}

function CategoryField({ value, disabled, onChange }: { value: string; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="text-sm"><HelpLabel label="Category (required)" help="Groups similar apps. Select a common category or type a new lowercase category." /><input className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={disabled} list="app-category-options" value={value} onChange={(event) => onChange(event.target.value)} /><datalist id="app-category-options"><option value="emulator" /><option value="frontend" /><option value="launcher" /><option value="tool" /><option value="utility" /></datalist></label>;
}

function PackageAliasList({ values, disabled, onChange }: { values: string[]; disabled: boolean; onChange: (values: string[]) => void }) {
  return <div className="text-sm"><div className="flex items-center justify-between"><HelpLabel label="Package aliases" help="Older or alternate Android package IDs that should be treated as this app." /><button className="rounded border border-slate-300 px-2 py-1 text-xs" disabled={disabled} type="button" onClick={() => onChange([...values, ""])}>Add alias</button></div><div className="mt-1 space-y-2">{values.map((alias, index) => <div className="grid grid-cols-[1fr_auto] gap-2" key={index}><input aria-label={`Package alias ${index + 1}`} className="rounded border border-slate-300 px-3 py-2" disabled={disabled} value={alias} onChange={(event) => onChange(values.map((item, itemIndex) => itemIndex === index ? event.target.value : item))} /><button className="rounded border border-red-300 px-2 text-xs text-red-700" disabled={disabled} type="button" onClick={() => onChange(values.filter((_, itemIndex) => itemIndex !== index))}>Remove</button></div>)}{values.length === 0 ? <p className="text-xs text-slate-500">No package aliases.</p> : null}</div></div>;
}

function StringListField({ label, help, addLabel, emptyLabel, values, disabled, onChange }: { label: string; help?: string; addLabel: string; emptyLabel: string; values: string[]; disabled: boolean; onChange: (values: string[]) => void }) {
  return <div className="text-sm"><div className="flex items-center justify-between"><HelpLabel label={label} help={help} /><button className="rounded border border-slate-300 px-2 py-1 text-xs" disabled={disabled} type="button" onClick={() => onChange([...values, ""])}>{addLabel}</button></div><div className="mt-1 space-y-2">{values.map((value, index) => <div className="grid grid-cols-[1fr_auto] gap-2" key={index}><input aria-label={`${label} ${index + 1}`} className="rounded border border-slate-300 px-3 py-2" disabled={disabled} value={value} onChange={(event) => onChange(values.map((item, itemIndex) => itemIndex === index ? event.target.value : item))} /><button className="rounded border border-red-300 px-2 text-xs text-red-700" disabled={disabled} type="button" onClick={() => onChange(values.filter((_, itemIndex) => itemIndex !== index))}>Remove</button></div>)}{values.length === 0 ? <p className="text-xs text-slate-500">{emptyLabel}</p> : null}</div></div>;
}

function Field({ label, help, value, disabled, onChange }: { label: string; help?: string; value: string; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="text-sm"><HelpLabel label={label} help={help} /><input className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={disabled} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Area({ label, help, value, disabled, onChange }: { label: string; help?: string; value: string; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="text-sm"><HelpLabel label={label} help={help} /><textarea className="mt-1 min-h-24 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs" disabled={disabled} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Fixed({ label, help, value }: { label: string; help?: string; value: string }) {
  return <div className="text-sm"><HelpLabel label={label} help={help} /><p className="mt-1 rounded bg-slate-100 px-3 py-2 text-sm">{value}</p></div>;
}

function Check({ label, checked, disabled, onChange }: { label: string; checked: boolean; disabled: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className={`flex items-center gap-2 text-sm ${disabled ? "cursor-not-allowed text-slate-400" : "cursor-pointer"}`}>
      <input
        className={disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer"}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
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
  if (response.kind !== "api-error") return response.message;
  const reason = response.error.details.reason;
  if (typeof reason !== "string" || reason.length === 0) return response.error.message;
  return `${response.error.message} Reason: ${formatAnalyzerFailureReason(reason)}.`;
}

function formatAnalyzerFailureReason(reason: string): string {
  const labels: Record<string, string> = {
    analyzer_start_failed: "the analyzer process could not start",
    analyzer_output_unavailable: "the analyzer output stream was unavailable",
    analyzer_timeout: "the analyzer timed out after 30 seconds",
    analyzer_wait_failed: "the analyzer process could not be monitored",
    analyzer_output_failed: "the analyzer output could not be read",
    analyzer_output_limit: "the analyzer produced more than the 4 MiB output limit",
    analyzer_command_failed: "an analyzer command returned a failure status",
    analyzer_output_invalid: "the analyzer returned invalid text output",
    analyzer_output_malformed: "the analyzer output did not contain the expected APK facts",
  };
  return labels[reason] ?? reason;
}

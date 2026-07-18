import { useEffect, useReducer, useRef } from "react";

import {
  beginAppGenerator,
  analyzeAppGeneratorSource,
  cancelAppGenerator,
  checkAppRecipeCollisions,
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
  ApkInspectionResult,
  ApkPermissionApplicabilityDto,
  ApkPermissionClassification,
  ApkPermissionReviewDto,
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
  assetPatternError,
  diagnosticDisplayTitle,
  eligibleApkAssets,
  formToRequest,
  initialAppGeneratorState,
  matchingAssetNames,
  otherRequestedPermissions,
  parseConnectedDeviceApi,
  parseTrustedSha256,
  reduceAppGenerator,
  visibleDraftDiagnostics,
  type AppGeneratorFormState,
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
    if (!state.sessionHandle || !state.selectedAssetHandle) return;
    const deviceApi = parseConnectedDeviceApi(state.connectedDeviceApiInput);
    if (!deviceApi.ok) {
      dispatch({ type: "failure", message: deviceApi.message });
      return;
    }
    const selectedAsset = state.sourceAnalysis?.assets.find(
      (asset) => asset.assetHandle === state.selectedAssetHandle,
    );
    if (
      selectedAsset?.prerelease &&
      !window.confirm("This release is marked as a prerelease. Continue with this APK?")
    ) {
      return;
    }
    dispatch({ type: "downloading" });
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
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
      assetPattern:
        state.installStrategy === "latest_compatible_release" ? state.assetPattern : null,
      includePrereleases: state.includePrereleases,
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
    if (!state.sessionHandle) return;
    const deviceApi = parseConnectedDeviceApi(state.connectedDeviceApiInput);
    if (!deviceApi.ok) {
      dispatch({ type: "failure", message: deviceApi.message });
      return;
    }
    dispatch({ type: "inspecting" });
    const inspected = await inspectAppGeneratorApk(
      state.sessionHandle,
      apkHandle,
      deviceApi.value,
    );
    if (inspected.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(inspected) });
      return;
    }
    dispatch({ type: "inspected", inspection: inspected.result });
    const drafted = remoteSource && assetHandle
      ? await generateRemoteAppRecipeDraft(
          state.sessionHandle,
          apkHandle,
          assetHandle,
          remoteSource.strategy,
          remoteSource.assetPattern,
          remoteSource.includePrereleases,
          null,
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
    const trustedSha256 = parseTrustedSha256(state.trustedSha256);
    if (!trustedSha256.ok) {
      dispatch({ type: "failure", message: trustedSha256.message });
      return;
    }
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
            state.installStrategy === "latest_compatible_release" ? state.assetPattern : null,
            state.includePrereleases,
            trustedSha256.value,
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
    const trustedSha256 = parseTrustedSha256(state.trustedSha256);
    if (!trustedSha256.ok) {
      dispatch({ type: "failure", message: trustedSha256.message });
      return;
    }
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
            state.installStrategy === "latest_compatible_release" ? state.assetPattern : null,
            state.includePrereleases,
            trustedSha256.value,
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

  const selectedAsset = state.sourceAnalysis?.assets.find(
    (asset) => asset.assetHandle === state.selectedAssetHandle,
  );
  const selectedReleaseFileNames = eligibleApkAssets(
    state.sourceAnalysis,
    selectedAsset?.releaseTag,
  ).map((asset) => asset.fileName);
  const latestPatternError =
    state.installStrategy === "latest_compatible_release"
      ? assetPatternError(state.assetPattern, selectedReleaseFileNames)
      : null;
  const latestPatternMatches = matchingAssetNames(
    state.assetPattern,
    selectedReleaseFileNames,
  );
  const connectedDeviceApi = parseConnectedDeviceApi(state.connectedDeviceApiInput);
  const connectedDeviceApiError = connectedDeviceApi.ok ? null : connectedDeviceApi.message;
  const trustedSha256 = parseTrustedSha256(state.trustedSha256);
  const trustedSha256Error = trustedSha256.ok ? null : trustedSha256.message;

  const busy =
    state.phase === "starting" ||
    state.phase === "downloading" ||
    state.phase === "inspecting" ||
    state.phase === "saving";
  const saveBlocked =
    busy ||
    !state.draft ||
    state.draft.blocking ||
    trustedSha256Error !== null ||
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
              <option value="gitlab_repository">GitLab repository</option>
              <option value="gitlab_release">GitLab release</option>
              <option value="forgejo_repository">Codeberg / Forgejo repository</option>
              <option value="forgejo_release">Codeberg / Forgejo release</option>
              <option value="direct_apk">Direct APK URL</option>
            </select>
            <p className="mt-1 text-xs text-slate-500">Choose where the APK and update identity come from.</p>

            {state.sourceMode === "local_apk" ? (
              <div className="mt-4 grid gap-4 md:grid-cols-3">
                <Picker label="APK" value={state.apkLabel} button="Choose APK..." disabled={busy} onClick={() => void chooseApk()} />
                <ConnectedDeviceApiField value={state.connectedDeviceApiInput} error={connectedDeviceApiError} disabled={busy} onChange={(value) => dispatch({ type: "connected-device-api", value })} />
                <div className="flex items-end">
                  <button className="w-full rounded bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300" disabled={busy || !state.apkHandle || connectedDeviceApiError !== null} onClick={() => void inspectApk()}>
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
                {state.sourceMode.endsWith("_repository") ? <Check label="Include prereleases" checked={state.includePrereleases} disabled={busy} onChange={(value) => dispatch({ type: "include-prereleases", value })} /> : null}
                {state.sourceAnalysis ? <RemoteAssetPicker analysis={state.sourceAnalysis} selectedAssetHandle={state.selectedAssetHandle} disabled={busy} onSelect={(assetHandle) => dispatch({ type: "asset-selected", assetHandle })} /> : null}
                <div className="grid gap-4 md:grid-cols-3">
                  <ConnectedDeviceApiField value={state.connectedDeviceApiInput} error={connectedDeviceApiError} disabled={busy} onChange={(value) => dispatch({ type: "connected-device-api", value })} />
                  {state.installStrategy === "latest_compatible_release" ? (
                    <section className="rounded border border-slate-200 bg-slate-50 p-3 text-sm md:col-span-3">
                      <label>
                        <HelpLabel label="APK filename pattern" help="This regular expression is derived from the selected APK and used to identify exactly one APK in future releases." />
                        <input
                          className="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
                          disabled={busy}
                          value={state.assetPattern}
                          onChange={(event) => dispatch({ type: "asset-pattern", value: event.target.value })}
                        />
                      </label>
                      {latestPatternError ? (
                        <p className="mt-2 text-red-700">{latestPatternError}</p>
                      ) : (
                        <div className="mt-2 text-slate-600">
                          <p>Current release match:</p>
                          <ul className="mt-1 list-disc pl-5">
                            {latestPatternMatches.map((fileName) => <li key={fileName}>{fileName}</li>)}
                          </ul>
                        </div>
                      )}
                      <p className="mt-2 text-xs text-amber-700">Future releases are resolved when provisioning runs. Resolution fails safely if the rule matches zero or multiple APKs.</p>
                    </section>
                  ) : null}
                  <label className="text-sm">
                    <HelpLabel label="APK resolution" help="Pinned installs this exact APK. Latest compatible release resolves a future release using the saved filename rule. User-provided APK creates a local file input." />
                    <select className="mt-1 w-full rounded border border-slate-300 px-3 py-2" disabled={busy} value={state.installStrategy} onChange={(event) => dispatch({ type: "install-strategy", strategy: event.target.value as import("../api/types").AppGeneratorInstallStrategy })}>
                      <option value="pinned_remote_asset">Pinned release</option>
                      {state.sourceAnalysis?.capabilities.latestRelease ? <option value="latest_compatible_release">Latest compatible release</option> : null}
                      <option value="user_provided_apk">User-provided APK</option>
                    </select>
                  </label>
                  <div className="flex items-end">
                    <button
                      className="flex w-full items-center justify-center gap-2 rounded bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                      disabled={busy || !state.selectedAssetHandle || connectedDeviceApiError !== null}
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
                  {state.installStrategy === "pinned_remote_asset" ? (
                    <label className="text-sm md:col-span-3">
                      <HelpLabel
                        label="Trusted publisher SHA-256 (optional)"
                        help="Enter only a checksum obtained from a trusted publisher source for this exact APK."
                      />
                      <input
                        className="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
                        disabled={busy}
                        value={state.trustedSha256}
                        onChange={(event) => dispatch({ type: "trusted-sha256", value: event.target.value })}
                      />
                      {trustedSha256Error ? <p className="mt-1 text-xs text-red-700">{trustedSha256Error}</p> : null}
                      <p className="mt-1 text-xs text-slate-500">
                        EmuChef does not copy the locally calculated inspection hash into this field. Inspection remains not compared; this trusted value is used only by the generated runtime recipe.
                      </p>
                    </label>
                  ) : null}
                  {state.phase === "downloading" ? (
                    <p className="text-xs text-slate-600 md:col-span-3" role="status" aria-live="polite">
                      Downloading the selected APK. This may take a moment.
                    </p>
                  ) : null}
                </div>
              </div>
            )}
          </section>

          {state.inspection ? (
            <InspectionReview
              inspection={state.inspection}
              disabled={busy}
              onRuntimeCandidateChange={(index, selected) => dispatch({ type: "runtime-candidate-selected", index, selected })}
              onAppOpCandidateChange={(index, selected) => dispatch({ type: "app-op-candidate-selected", index, selected })}
            />
          ) : null}
          {state.form ? (
            <>
              <AppFields sourceMode={state.sourceMode} installStrategy={state.installStrategy} form={state.form} disabled={busy} updateForm={updateForm} changeApp={changeApp} changeMapping={changeMapping} />
              <RecipeFields form={state.form} disabled={busy} changeRecipe={changeRecipe} regenerateIds={() => void regenerateIds()} />
              <section className="mt-4 grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-[1fr_auto]">
                <Picker label="Authored root" value={state.rootHandle ? "Selected and ready" : null} button={state.rootHandle ? "Change authored root..." : "Choose authored root..."} disabled={busy} onClick={() => void chooseRoot()} />
                <button
                  className="self-end rounded bg-indigo-700 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                  disabled={busy || !state.rootHandle || latestPatternError !== null}
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

function ConnectedDeviceApiField({ value, error, disabled, onChange }: { value: string; error: string | null; disabled: boolean; onChange: (value: string) => void }) {
  return (
    <label className="text-sm">
      <HelpLabel label="Connected-device API (optional)" help="Enter the Android API level of the device you plan to use. Leave blank to inspect the manifest without permission applicability or automation candidates." />
      <input
        className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
        disabled={disabled}
        inputMode="numeric"
        min="1"
        placeholder="For example, 35"
        type="number"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      {error ? <span className="mt-1 block text-xs text-red-700">{error}</span> : <span className="mt-1 block text-xs text-slate-500">No device probing is performed.</span>}
    </label>
  );
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

function InspectionReview({ inspection, disabled, onRuntimeCandidateChange, onAppOpCandidateChange }: {
  inspection: ApkInspectionResult;
  disabled: boolean;
  onRuntimeCandidateChange: (index: number, selected: boolean) => void;
  onAppOpCandidateChange: (index: number, selected: boolean) => void;
}) {
  const otherPermissions = otherRequestedPermissions(inspection);
  const rows: Array<[string, string]> = [
    ["Package", inspection.manifest.packageName],
    ["Version name", inspection.manifest.versionName ?? "Not declared"],
    ["Version code", inspection.manifest.versionCode ?? "Not declared"],
    ["Minimum SDK", inspection.manifest.minSdkVersion ?? "Not declared"],
    ["Target SDK", inspection.manifest.targetSdkVersion ?? "Not declared"],
  ];
  return (
    <div className="mt-4 space-y-4">
      <section className="rounded border border-slate-200 p-4">
        <h2 className="font-semibold">APK manifest and integrity metadata</h2>
        <p className="mt-1 text-sm text-slate-600">These values were extracted natively from the selected APK manifest.</p>
        <dl className="mt-3 grid grid-cols-[9rem_1fr] gap-2 text-sm">
          {rows.map(([label, value]) => <div className="contents" key={label}><dt className="font-medium text-slate-500">{label}</dt><dd className="break-all">{value}</dd></div>)}
          <div className="contents"><dt className="font-medium text-slate-500">Calculated SHA-256</dt><dd className="break-all font-mono text-xs">{inspection.calculatedSha256}</dd></div>
          <div className="contents"><dt className="font-medium text-slate-500">Checksum comparison</dt><dd>Not compared with a publisher-provided checksum.</dd></div>
          <div className="contents"><dt className="font-medium text-slate-500">Signature verification</dt><dd>Not performed. EmuChef does not cryptographically verify APK signatures.</dd></div>
        </dl>
      </section>

      <section className="rounded border border-slate-200 p-4">
        <h2 className="font-semibold">Permission review</h2>
        <p className="mt-1 text-sm text-slate-600">Selections are review-only in this phase and do not change generated or saved files.</p>
        <PermissionWarnings inspection={inspection} />

        <section className="mt-4 rounded border border-slate-200 bg-slate-50 p-3">
          <h3 className="text-sm font-semibold">Runtime grant candidates</h3>
          <p className="mt-1 text-xs text-slate-600">Runtime grants use <code>pm grant</code>. Candidates marked as not requiring root can run on unrooted devices.</p>
          <div className="mt-3 space-y-2">
            {inspection.runtimeGrantCandidates.map((candidate, index) => (
              <label className="flex items-start gap-2 rounded border border-slate-200 bg-white p-3 text-sm" key={`${candidate.permissionName}-${index}`}>
                <input type="checkbox" checked={candidate.selected} disabled={disabled} onChange={(event) => onRuntimeCandidateChange(index, event.target.checked)} />
                <span><code>{candidate.permissionName}</code><span className="mt-1 block text-xs text-slate-500">{candidate.requiresRoot ? "Root required" : "No root required"}</span></span>
              </label>
            ))}
            {inspection.runtimeGrantCandidates.length === 0 ? <p className="text-sm text-slate-500">No runtime grant candidates for this inspection context.</p> : null}
          </div>
        </section>

        <section className="mt-4 rounded border border-slate-200 bg-slate-50 p-3">
          <h3 className="text-sm font-semibold">App-op candidates</h3>
          <p className="mt-1 text-xs text-slate-600">Root-dependent app-ops may not be available on unrooted devices.</p>
          <div className="mt-3 space-y-2">
            {inspection.appOpCandidates.map((candidate, index) => (
              <label className="flex items-start gap-2 rounded border border-slate-200 bg-white p-3 text-sm" key={`${candidate.permissionName}-${candidate.operationName}-${index}`}>
                <input type="checkbox" checked={candidate.selected} disabled={disabled} onChange={(event) => onAppOpCandidateChange(index, event.target.checked)} />
                <span><code>{candidate.permissionName}</code><span className="mt-1 block text-xs text-slate-500">Operation {candidate.operationName}, mode {candidate.mode}; {candidate.requiresRoot ? "root required" : "no root required"}.</span></span>
              </label>
            ))}
            {inspection.appOpCandidates.length === 0 ? <p className="text-sm text-slate-500">No app-op candidates for this inspection context.</p> : null}
          </div>
        </section>

        <section className="mt-4">
          <h3 className="text-sm font-semibold">Other requested permissions</h3>
          <p className="mt-1 text-xs text-slate-600">Unknown, restricted, privileged, manual, install-time, non-applicable, and indeterminate permissions are not automated here.</p>
          <div className="mt-3 space-y-2">
            {otherPermissions.map((permission, index) => <PermissionDeclaration permission={permission} key={`${permission.name}-${permission.declarationKind}-${permission.maxSdkVersion ?? "none"}-${index}`} />)}
            {otherPermissions.length === 0 ? <p className="text-sm text-slate-500">No other requested permissions.</p> : null}
          </div>
        </section>
      </section>
    </div>
  );
}

function PermissionWarnings({ inspection }: { inspection: ApkInspectionResult }) {
  if (inspection.warnings.length === 0) return null;
  return (
    <section className="mt-3">
      <h3 className="text-sm font-semibold text-amber-800">Inspection context and warnings</h3>
      <div className="mt-2 space-y-2">
        {inspection.warnings.map((warning, index) => (
          <div className="rounded border border-amber-300 bg-amber-50 p-2 text-sm" key={`${warning.code}-${warning.permissionName ?? "global"}-${index}`}>
            {warning.permissionName ? <code className="font-semibold">{warning.permissionName}</code> : null}
            <p className={warning.permissionName ? "mt-1" : ""}>{warning.message}{warning.applicabilityReason ? ` (${applicabilityReasonLabel(warning.applicabilityReason)})` : ""}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

function PermissionDeclaration({ permission }: { permission: ApkPermissionReviewDto }) {
  const applicability = permission.applicability;
  const tone = applicability?.status === "not_applicable"
    ? "border-slate-300 bg-slate-50"
    : applicability?.status === "indeterminate"
      ? "border-amber-300 bg-amber-50"
      : "border-slate-200 bg-white";
  return (
    <div className={`rounded border p-3 text-sm ${tone}`}>
      <code className="font-semibold">{permission.name}</code>
      <p className="mt-1 text-xs text-slate-600">Declaration: {declarationKindLabel(permission.declarationKind)}{permission.maxSdkVersion ? `; maximum SDK ${permission.maxSdkVersion}` : ""}.</p>
      <p className="mt-1 text-xs text-slate-600">Classification: {classificationLabel(permission.classification)}. Applicability: {applicabilityLabel(applicability)}.</p>
    </div>
  );
}

function declarationKindLabel(kind: ApkPermissionReviewDto["declarationKind"]): string {
  return kind === "uses_permission_sdk_23" ? "uses-permission-sdk-23" : "uses-permission";
}

function classificationLabel(classification: ApkPermissionClassification | null): string {
  if (classification === null) return "unavailable without device API context";
  const labels: Record<ApkPermissionClassification, string> = {
    runtime_grantable: "runtime grantable",
    runtime_restricted: "runtime restricted",
    app_op_grantable: "app-op grantable",
    manual_special_access: "manual special access",
    install_time: "install time",
    signature_or_privileged: "signature or privileged",
    unknown: "unknown",
  };
  return labels[classification];
}

function applicabilityLabel(applicability: ApkPermissionApplicabilityDto | null): string {
  if (applicability === null) return "unavailable without device API context";
  const details = applicabilityDetails(applicability);
  if (applicability.status === "applicable") return details ? `applicable (${details})` : "applicable";
  const reason = applicability.reason ? ` — ${applicabilityReasonLabel(applicability.reason)}` : "";
  const suffix = details ? `; ${details}` : "";
  return applicability.status === "not_applicable" ? `not applicable${reason}${suffix}` : `indeterminate${reason}${suffix}`;
}

function applicabilityDetails(applicability: ApkPermissionApplicabilityDto): string {
  const details: string[] = [];
  if (applicability.maximumSdkVersion !== null) details.push(`maximum SDK ${applicability.maximumSdkVersion}`);
  if (applicability.introductionApi !== null) details.push(`introduced in API ${applicability.introductionApi}`);
  if (applicability.minimumDeviceApi !== null) details.push(`minimum device API ${applicability.minimumDeviceApi}`);
  if (applicability.minimumTargetSdk !== null) details.push(`minimum target SDK ${applicability.minimumTargetSdk}`);
  if (applicability.targetSdkState !== null) details.push(`target SDK ${applicability.targetSdkState === "missing" ? "missing" : "non-numeric"}`);
  return details.join(", ");
}

function applicabilityReasonLabel(reason: NonNullable<ApkPermissionApplicabilityDto["reason"]>): string {
  const labels: Record<NonNullable<ApkPermissionApplicabilityDto["reason"]>, string> = {
    declaration_requires_api_23: "declaration requires Android API 23 or newer",
    max_sdk_version_exceeded: "the declaration's maximum SDK was exceeded",
    permission_not_introduced: "permission is not introduced on this device API",
    permission_replaced: "permission was replaced for this Android context",
    target_sdk_below_minimum: "application target SDK is below the required minimum",
    invalid_max_sdk_version: "maximum SDK could not be interpreted",
    target_sdk_unavailable: "target SDK context is unavailable",
    replacement_target_sdk_unavailable: "replacement target SDK context is unavailable",
  };
  return labels[reason];
}

function AppFields({ sourceMode, installStrategy, form, disabled, updateForm, changeApp, changeMapping }: {
  sourceMode: AppGeneratorSourceMode;
  installStrategy: import("../api/types").AppGeneratorInstallStrategy;
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
        <Field label="Primary package" help="Android application ID extracted from the APK manifest. Change only when the manifest information is known to be wrong." value={app.package.primary} disabled={disabled} onChange={(primary) => updateForm((next) => { next.app.package.primary = primary; })} />
        <PackageAliasList values={form.aliases} disabled={disabled} onChange={(aliases) => updateForm((next) => { next.aliases = aliases; })} />
        <Fixed label="Installation method" help="Controls whether the recipe pins an APK, resolves the latest compatible release, or asks for a local file." value={installStrategy === "pinned_remote_asset" ? "Pinned release" : installStrategy === "latest_compatible_release" ? "Latest compatible release" : "User-provided APK"} />
        <Fixed label="Source resolver" help="Describes how the generated app definition identifies its installation source." value={installStrategy === "pinned_remote_asset" ? "Direct HTTPS download" : installStrategy === "latest_compatible_release" ? "Latest provider release" : "None required"} />
        <Fixed label="Update tracking" help="Describes the source identity retained for future catalog review." value={installStrategy === "user_provided_apk" ? "Local APK" : sourceMode.endsWith("_repository") || sourceMode.endsWith("_release") ? "Provider release" : sourceMode === "direct_apk" ? "Direct APK URL" : "Local APK"} />
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

function RecipeFields({ form, disabled, changeRecipe, regenerateIds }: {
  form: AppGeneratorFormState;
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
        <Check label="Launch once after installation" checked={recipe.launchEnabled} disabled onChange={() => undefined} />
      </div>
      <p className="mt-2 text-xs text-slate-500">Launcher activities are unavailable from native manifest inspection, so launch-once generation remains disabled.</p>
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
  return response.kind === "api-error" ? response.error.message : response.message;
}

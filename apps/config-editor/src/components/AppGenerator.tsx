import { useEffect, useReducer, useRef } from "react";
import {
  appGeneratorPublicationStatus,
  beginAppGenerator,
  analyzeAppGeneratorSource,
  cancelAppGenerator,
  chooseAppGeneratorApk,
  chooseAppGeneratorAuthoredRoot,
  downloadAppGeneratorRemoteApk,
  generateAppRecipeDraft,
  generateRemoteAppRecipeDraft,
  inspectAppGeneratorApk,
  listAppGeneratorExistingApps,
  reviewAppGeneratorDraft,
  reviewRemoteAppGeneratorDraft,
  saveAppGeneratorReview,
  selectAppGeneratorExistingApp,
  setAppGeneratorAuthoredRoot,
  type EditorApiResult,
} from "../api/editorApi";
import type {
  ApkInspectionResult,
  ApkPermissionClassification,
  ApkPermissionReviewDto,
  ApkPermissionWarningDto,
  AppArtifactKindDto,
  AppArtifactStrategyDto,
  AppDefinitionV1Dto,
  AppGeneratorPublicationOutcomeDto,
  AppGeneratorPublicationStatusResult,
  AppGeneratorInstallStrategy,
  AppGeneratorRequestContext,
  AppGeneratorReviewResult,
  AppRecipeCollisionResult,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppGeneratorSourceMode,
  AppReleaseProviderDto,
  AppTargetKindDto,
  AppTargetLocationDto,
  ExistingAppSummaryDto,
  ReleasePatternOutcome,
  ReleasePatternPreviewResult,
  RemoteSourceAnalysisResult,
  RemoteSourceDescriptorDto,
  ReviewPermissionActionDto,
} from "../api/types";
import {
  analysisForPrereleasePolicy,
  assetPatternError,
  buildReleasePatternPreview,
  diagnosticDisplayTitle,
  effectiveArtifactId,
  effectiveRecipeId,
  eligibleApkAssets,
  emptyArtifactRow,
  emptyTargetRow,
  formToRequest,
  generatorRequestContext,
  globalInspectionWarnings,
  initialAppGeneratorState,
  inspectionPartition,
  matchingAssetNames,
  mirroredTrustedSha256,
  otherRequestedPermissions,
  parseTrustedSha256,
  permissionApplicabilityLabel,
  permissionApplicabilityReasonLabel,
  permissionSelectionForInspection,
  permissionWarnings,
  publicationFailureRecovery,
  publicationUnresolved,
  reduceAppGenerator,
  releaseAnalysisDescribesReviewedPolicy,
  selectedPackageMismatch,
  visibleDraftDiagnostics,
  type AppArtifactRowEdits,
  type AppGeneratorFormState,
  type AppGeneratorState,
  type AppGeneratorTargetMode,
  type AppMappingRowEdits,
  type AppTargetRowEdits,
} from "./appGenerator.logic";

interface AppGeneratorProps {
  initialAuthoredRoot: string | null;
  onAuthoredRootSelected: (path: string) => void | Promise<void>;
  onClose: () => void;
  onSaved: (result: AppGeneratorPublicationOutcomeDto) => void;
}

/** Guided App-definition and Recipe generator for a new or existing App. */
export function AppGenerator({ initialAuthoredRoot, onAuthoredRootSelected, onClose, onSaved }: AppGeneratorProps) {
  const [state, dispatch] = useReducer(reduceAppGenerator, initialAppGeneratorState);
  const stateRef = useRef(state);
  stateRef.current = state;
  const initialAuthoredRootRef = useRef(initialAuthoredRoot);

  /**
   * Return the next monotonic request id for a preview, review, or save.
   *
   * The reducer stores the highest issued id, and every response is only
   * applied when it matches the pending id of its stream, so a late response
   * for replaced inputs can never overwrite newer author intent.
   */
  function nextRequestId(): number {
    return stateRef.current.lastIssuedRequestId + 1;
  }


/**
 * Build the identity fields one preview or review request carries.
 *
 * The explicit handle argument lets a just-selected existing App seed the form
 * before React has applied the selection to the reducer state.
 */
function requestContext(
  current: AppGeneratorState,
  requestId: number,
  existingAppHandle?: string | null,
): AppGeneratorRequestContext {
  const context = generatorRequestContext(current, requestId);
  if (current.targetMode !== "existing_app") return context;
  return {
    ...context,
    existingAppHandle:
      existingAppHandle !== undefined ? existingAppHandle : current.existingApp?.appHandle ?? null,
  };
}

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const response = await beginAppGenerator();
      if (cancelled) return;
      if (response.kind !== "success") {
        dispatch({ type: "failure", message: apiFailure(response) });
        return;
      }
      dispatch({
        type: "started",
        sessionHandle: response.result.sessionHandle,
        rootHandle: response.result.rootHandle,
        rootLabel: response.result.rootLabel,
      });
      if (initialAuthoredRootRef.current) {
        const bound = await setAppGeneratorAuthoredRoot(
          response.result.sessionHandle,
          initialAuthoredRootRef.current,
        );
        if (cancelled) return;
        if (bound.kind === "success" && bound.result.rootHandle && bound.result.label) {
          dispatch({ type: "root-selected", rootHandle: bound.result.rootHandle, label: bound.result.label });
        } else if (bound.kind === "success") {
          dispatch({ type: "failure", message: "The authored root selection did not return a usable root." });
        } else {
          dispatch({ type: "failure", message: apiFailure(bound) });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    return () => {
      const current = stateRef.current;
      // A recorded publication keeps its session record so a later status
      // lookup can still confirm what happened; every other session is
      // released when the dialog closes.
      if (current.sessionHandle === null || current.publication !== null) return;
      void cancelAppGenerator(current.sessionHandle);
    };
  }, []);

  async function chooseApk() {
    if (!state.sessionHandle) return;
    const response = await chooseAppGeneratorApk(state.sessionHandle);
    if (response.kind === "success") {
      if (!response.result.cancelled && response.result.apkHandle && response.result.label) {
        dispatch({ type: "apk-selected", apkHandle: response.result.apkHandle, label: response.result.label });
        await inspectApkHandle(response.result.apkHandle);
      }
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
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
    const selectedAsset = state.sourceAnalysis?.assets.find(
      (asset) => asset.assetHandle === state.selectedAssetHandle,
    );
    if (
      selectedAsset?.prerelease
      && !window.confirm("This release is marked as prerelease. Continue with this APK?")
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
    // A freshly downloaded artifact carries no author-supplied publisher
    // checksum, so the previous artifact's input is cleared with its source.
    await inspectApkHandle(downloaded.result.apkHandle, source);
  }

  /**
   * Inspect one selected APK and rebuild the editable preview from it.
   *
   * A freshly downloaded remote source is passed explicitly because the state
   * update that records it has not reached a render snapshot yet.
   */
  async function inspectApkHandle(
    apkHandle: string,
    sourceOverride?: RemoteSourceDescriptorDto | null,
  ) {
    if (!state.sessionHandle) return;
    dispatch({ type: "inspecting" });
    const inspected = await inspectAppGeneratorApk(state.sessionHandle, apkHandle);
    if (inspected.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(inspected) });
      return;
    }
    dispatch({ type: "inspected", inspection: inspected.result });
    await generatePreview(undefined, apkHandle, sourceOverride);
  }

  /**
   * Build the editable preview for the selected source.
   *
   * The APK handle and remote source are accepted as explicit arguments so a
   * flow that just selected them does not read a render snapshot taken before
   * that update was applied.
   */
  async function generatePreview(
    existingAppHandle?: string | null,
    apkHandleOverride?: string,
    remoteSourceOverride?: RemoteSourceDescriptorDto | null,
  ) {
    const current = stateRef.current;
    const apkHandle = apkHandleOverride ?? current.apkHandle;
    if (!current.sessionHandle || !apkHandle) return;
    const trustedSha256 = parseTrustedSha256(current.trustedSha256);
    if (!trustedSha256.ok) {
      dispatch({ type: "failure", message: trustedSha256.message });
      return;
    }
    const requestId = nextRequestId();
    dispatch({ type: "preview-request", requestId });
    const context = requestContext(current, requestId, existingAppHandle);
    const remoteSource =
      remoteSourceOverride !== undefined ? remoteSourceOverride : current.remoteSource;
    const drafted = current.sourceMode === "local_apk"
      ? await generateAppRecipeDraft(
          current.sessionHandle,
          apkHandle,
          null,
          null,
          null,
          null,
          false,
          current.rootHandle,
          context,
        )
      : current.selectedAssetHandle && remoteSource
        ? await generateRemoteAppRecipeDraft(
            current.sessionHandle,
            apkHandle,
            current.selectedAssetHandle,
            remoteSource.strategy,
            remoteSource.assetPattern,
            remoteSource.includePrereleases,
            remoteSource.strategy === "pinned_remote_asset" && trustedSha256.ok
              ? trustedSha256.value
              : null,
            null,
            null,
            null,
            null,
            false,
            current.rootHandle,
            context,
          )
        : null;
    if (!drafted) {
      dispatch({ type: "failure", message: "Select and download an APK asset before previewing." });
      return;
    }
    if (drafted.kind === "success") {
      dispatch({ type: "preview-response", requestId, draft: drafted.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(drafted) });
    }
  }

  async function buildReview() {
    const current = stateRef.current;
    if (!current.sessionHandle || !current.apkHandle) return;
    if (!current.rootHandle) {
      dispatch({ type: "failure", message: "Choose an authored root before reviewing." });
      return;
    }
    if (releasePatternPreview?.blocking) {
      dispatch({
        type: "failure",
        message: releasePatternPreview.blockingMessage ?? "Resolve the release-pattern preview before reviewing.",
      });
      return;
    }
    const mismatch = selectedPackageMismatch(current);
    if (mismatch) {
      dispatch({
        type: "failure",
        message: packageMismatchMessage(mismatch),
      });
      return;
    }
    const trustedSha256 = parseTrustedSha256(current.trustedSha256);
    if (!trustedSha256.ok) {
      dispatch({ type: "failure", message: trustedSha256.message });
      return;
    }
    const request = current.form ? formToRequest(current.form) : null;
    if (request && !request.ok) {
      dispatch({ type: "failure", message: request.message });
      return;
    }
    // Existing-App generation preserves the selected App Definition and its
    // artifact and target mappings, so the review carries no App or mapping
    // edits for that target; the trusted selection owns those bytes.
    const existingTarget = current.targetMode === "existing_app";
    const appEdits = existingTarget ? null : request?.ok ? request.app : null;
    const mappingEdits = existingTarget ? null : request?.ok ? request.mappings : null;
    const permissionSelection = current.automationOptIn
      ? permissionSelectionForInspection(current.inspection)
      : null;
    const requestId = nextRequestId();
    dispatch({ type: "review-request", requestId });
    const context = requestContext(current, requestId);
    const remoteSource = current.remoteSource;
    const reviewed = current.sourceMode === "local_apk"
      ? await reviewAppGeneratorDraft({
          sessionHandle: current.sessionHandle,
          apkHandle: current.apkHandle,
          rootHandle: current.rootHandle,
          context,
          app: appEdits,
          recipe: request?.ok ? request.recipe : null,
          mappings: mappingEdits,
          permissionSelection,
          regenerateIdentifiers: false,
        })
      : current.selectedAssetHandle && remoteSource
        ? await reviewRemoteAppGeneratorDraft({
            sessionHandle: current.sessionHandle,
            apkHandle: current.apkHandle,
            rootHandle: current.rootHandle,
            context,
            app: appEdits,
            recipe: request?.ok ? request.recipe : null,
            mappings: mappingEdits,
            permissionSelection,
            regenerateIdentifiers: false,
            assetHandle: current.selectedAssetHandle,
            strategy: remoteSource.strategy,
            assetPattern: remoteSource.assetPattern,
            includePrereleases: remoteSource.includePrereleases,
            trustedSha256:
              remoteSource.strategy === "pinned_remote_asset" && trustedSha256.ok
                ? trustedSha256.value
                : null,
          })
        : null;
    if (!reviewed) {
      dispatch({ type: "failure", message: "Select and download an APK asset before reviewing." });
      return;
    }
    if (reviewed.kind === "success") {
      dispatch({ type: "review-response", requestId, review: reviewed.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(reviewed) });
    }
  }

  async function saveReview() {
    const current = stateRef.current;
    if (!current.sessionHandle || !current.review) return;
    // The native layer accepts a save only for the request id its review was
    // registered under, so the save carries the review-bound id.
    const requestId = current.review.requestId;
    dispatch({ type: "save-request", requestId });
    const response = await saveAppGeneratorReview(
      current.sessionHandle,
      current.review.reviewHandle,
      requestId,
    );
    if (response.kind === "success") {
      dispatch({ type: "publication-response", result: response.result });
      if (response.result.outcome === "saved") onSaved(response.result);
    } else {
      // A lost response is never repeated automatically; the author decides
      // what to do next with the outcome they see.
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function checkPublicationStatus() {
    const operationId = state.publication?.operationId;
    if (!operationId) return;
    dispatch({ type: "publication-status-request" });
    const response = await appGeneratorPublicationStatus(operationId);
    if (response.kind === "success") {
      dispatch({ type: "publication-status-response", result: response.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function loadExistingApps() {
    const current = stateRef.current;
    if (!current.sessionHandle) return;
    const requestId = nextRequestId();
    dispatch({ type: "existing-apps-request", requestId });
    const response = await listAppGeneratorExistingApps(current.sessionHandle, current.rootHandle);
    if (response.kind === "success") {
      dispatch({ type: "existing-apps-response", requestId, result: response.result });
    } else {
      dispatch({ type: "failure", message: apiFailure(response) });
    }
  }

  async function selectExistingApp(appPath: string) {
    const current = stateRef.current;
    if (!current.sessionHandle) return;
    const requestId = nextRequestId();
    dispatch({ type: "existing-app-select-request", requestId });
    const response = await selectAppGeneratorExistingApp(current.sessionHandle, appPath);
    if (response.kind !== "success") {
      dispatch({ type: "failure", message: apiFailure(response) });
      return;
    }
    dispatch({ type: "existing-app-select-response", requestId, result: response.result });
    if (current.apkHandle) {
      // Seed the editable form for the newly selected target.
      await generatePreview(response.result.appHandle);
    }
  }

  function updateForm(update: (form: AppGeneratorFormState) => void) {
    if (!state.form) return;
    const next = structuredClone(state.form);
    update(next);
    dispatch({ type: "form", form: next });
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

  function changeMapping(update: Partial<AppMappingRowEdits>) {
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
  const releaseAnalysisCurrent = releaseAnalysisDescribesReviewedPolicy(state);
  const latestPatternError =
    state.installStrategy === "latest_compatible_release" && releaseAnalysisCurrent
      ? assetPatternError(state.assetPattern, selectedReleaseFileNames)
      : null;
  const latestPatternMatches = matchingAssetNames(
    state.assetPattern,
    selectedReleaseFileNames,
  );
  const filteredSourceAnalysis = state.sourceAnalysis
    ? analysisForPrereleasePolicy(state.sourceAnalysis, state.includePrereleases)
    : null;
  const releasePatternPreview =
    state.sourceMode === "github_repository"
    && state.installStrategy === "latest_compatible_release"
    && state.sourceAnalysis
    && releaseAnalysisCurrent
      ? buildReleasePatternPreview(
          state.sourceAnalysis,
          state.assetPattern,
          state.includePrereleases,
        )
      : null;
  const trustedSha256 = parseTrustedSha256(state.trustedSha256);
  const trustedSha256Error = trustedSha256.ok ? null : trustedSha256.message;
  const packageMismatch = selectedPackageMismatch(state);
  const busy =
    state.phase === "starting" ||
    state.phase === "downloading" ||
    state.phase === "inspecting" ||
    state.previewPending ||
    state.reviewPending ||
    state.savePending ||
    state.existingAppsPending ||
    state.existingAppSelectPending ||
    state.publicationStatusPending;
  const reviewDisabled =
    busy ||
    !state.apkHandle ||
    !state.rootHandle ||
    packageMismatch !== null ||
    releasePatternPreview?.blocking === true ||
    trustedSha256Error !== null;
  const saveDisabled =
    busy ||
    state.review === null ||
    state.review.blocking ||
    state.reviewPending;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/50 p-4">
      <section className="flex max-h-[94vh] w-full max-w-6xl flex-col overflow-hidden rounded-lg bg-white shadow-2xl">
        <header className="flex items-center justify-between border-b border-slate-200 px-5 py-3">
          <div>
            <p className="text-xs font-semibold tracking-wide text-slate-500">App generation</p>
            <h1 className="text-lg font-semibold">App definition and Recipe generator</h1>
          </div>
          <button
            className="rounded border border-slate-300 px-3 py-1.5 text-sm"
            disabled={state.savePending}
            onClick={onClose}
          >
            Close
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {state.error ? (
            <p role="alert" className="mb-4 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">
              {state.error}
            </p>
          ) : null}
          {packageMismatch ? (
            <p role="alert" className="mb-4 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">
              {packageMismatchMessage(packageMismatch)}
            </p>
          ) : null}
          {state.publication ? (
            <PublicationPanel
              publication={state.publication}
              status={state.publicationStatus}
              statusPending={state.publicationStatusPending}
              onCheckStatus={() => void checkPublicationStatus()}
              onReviewAgain={() => void buildReview()}
              reviewDisabled={reviewDisabled}
            />
          ) : null}
          <TargetAppSection
            state={state}
            busy={busy}
            onModeChange={(mode) => {
              dispatch({ type: "target-mode", mode });
              if (mode === "existing_app") void loadExistingApps();
            }}
            onRefreshApps={() => void loadExistingApps()}
            onSelectApp={(appPath) => void selectExistingApp(appPath)}
          />
          <section className="mt-4 rounded border border-slate-200 p-4">
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
              <div className="mt-4 grid gap-4 md:grid-cols-2">
                <Picker
                  label="APK"
                  value={state.apkLabel}
                  button={state.apkHandle ? "Choose a different APK..." : "Choose APK..."}
                  disabled={busy}
                  onClick={() => void chooseApk()}
                />
                <div className="flex items-end">
                  <button
                    className="w-full bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                    disabled={busy}
                    onClick={() => void chooseApk()}
                  >
                    {state.phase === "inspecting" ? "Inspecting APK..." : "Choose and inspect APK"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="mt-4 space-y-4">
                <div className="grid gap-3 md:grid-cols-[1fr_auto]">
                  <label className="text-sm">
                    <HelpLabel
                      label={state.sourceMode === "direct_apk" ? "APK download URL" : "Source URL"}
                      help={
                        state.sourceMode === "github_repository"
                          ? "Enter a public GitHub repository URL."
                          : state.sourceMode === "github_release"
                            ? "Enter a public GitHub release URL ending in /releases/tag/<tag>."
                            : state.sourceMode === "direct_apk"
                              ? "Enter a public HTTPS URL that ends in .apk."
                              : "Enter the public repository or release URL for the selected provider."
                      }
                    />
                    <input
                      className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                      disabled={busy}
                      value={state.sourceUrl}
                      onChange={(event) => dispatch({ type: "source-url", value: event.target.value })}
                    />
                  </label>
                  <button
                    className="self-end rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                    disabled={busy || !state.sourceUrl.trim()}
                    onClick={() => void analyzeRemoteSource()}
                  >
                    {state.phase === "inspecting" && !state.apkHandle ? "Checking source..." : "Check source"}
                  </button>
                </div>
                {state.sourceMode.endsWith("_repository") ? (
                  <Check
                    label="Include prereleases"
                    checked={state.includePrereleases}
                    disabled={busy}
                    onChange={(value) => dispatch({ type: "include-prereleases", value })}
                  />
                ) : null}
                {filteredSourceAnalysis ? (
                  <RemoteAssetPicker
                    analysis={filteredSourceAnalysis}
                    selectedAssetHandle={state.selectedAssetHandle}
                    disabled={busy}
                    onSelect={(assetHandle) => dispatch({ type: "asset-selected", assetHandle })}
                  />
                ) : null}
                <div className="grid gap-4 md:grid-cols-3">
                  {releasePatternPreview ? (
                    <ReleasePatternPreview
                      busy={busy}
                      pattern={state.assetPattern}
                      preview={releasePatternPreview}
                      onPatternChange={(value) => dispatch({ type: "asset-pattern", value })}
                    />
                  ) : state.installStrategy === "latest_compatible_release" ? (
                    <section className="rounded border border-slate-200 bg-slate-50 p-3 text-sm md:col-span-3">
                      <label>
                        <HelpLabel
                          label="APK filename pattern (optional)"
                          help="Leave this blank to accept only an eligible APK release. A regular expression narrows which APK asset is eligible."
                        />
                        <input
                          className="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
                          disabled={busy}
                          value={state.assetPattern}
                          onChange={(event) => dispatch({ type: "asset-pattern", value: event.target.value })}
                        />
                      </label>
                      {latestPatternError ? (
                        <p className="mt-2 text-red-700">{latestPatternError}</p>
                      ) : releaseAnalysisCurrent ? (
                        <div className="mt-2 text-slate-600">
                          <p>Current release match:</p>
                          <ul className="mt-1 list-disc pl-5">
                            {latestPatternMatches.map((fileName) => <li key={fileName}>{fileName}</li>)}
                          </ul>
                        </div>
                      ) : (
                        <p className="mt-2 text-slate-600">
                          The analyzed releases describe a different source, so the pattern cannot be confirmed. Rust reports a retargeted source as a nonblocking stale-analysis warning.
                        </p>
                      )}
                      <p className="mt-2 text-xs text-amber-700">
                        Future releases are resolved when provisioning runs. Resolution fails safely when a release has no eligible APK or more than one.
                      </p>
                    </section>
                  ) : null}
                  <label className="text-sm">
                    <HelpLabel
                      label="APK resolution"
                      help="Pinned installs one exact APK. Latest release resolves the newest eligible release by selecting the only eligible APK, optionally narrowed by a filename pattern. User-provided APK creates a local file input."
                    />
                    <select
                      className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                      disabled={busy}
                      value={state.installStrategy}
                      onChange={(event) => dispatch({ type: "install-strategy", strategy: event.target.value as AppGeneratorInstallStrategy })}
                    >
                      <option value="pinned_remote_asset">Pinned release</option>
                      {state.sourceAnalysis?.capabilities.latestRelease ? (
                        <option value="latest_compatible_release">Latest compatible release</option>
                      ) : null}
                      <option value="user_provided_apk">User-provided APK</option>
                    </select>
                  </label>
                  <div className="flex items-end">
                    <button
                      className="w-full items-center justify-center gap-2 bg-slate-900 px-3 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                      disabled={busy || !state.selectedAssetHandle}
                      onClick={() => void downloadRemoteApk()}
                    >
                      {state.phase === "downloading" ? (
                        <>
                          <span aria-hidden="true" className="h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white" />
                          Downloading APK...
                        </>
                      ) : state.apkHandle
                        ? "Download selected APK again"
                        : "Download and inspect APK"}
                    </button>
                  </div>
                </div>
                {state.installStrategy === "pinned_remote_asset" ? (
                  <label className="text-sm md:col-span-3">
                    <HelpLabel
                      label="Trusted publisher SHA-256 (optional)"
                      help="A checksum published by the APK publisher. EmuChef never computes this value on its own."
                    />
                    <input
                      className="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
                      disabled={busy}
                      value={state.trustedSha256}
                      onChange={(event) => dispatch({ type: "trusted-sha256", value: event.target.value })}
                    />
                    {trustedSha256Error ? <p className="mt-1 text-xs text-red-700">{trustedSha256Error}</p> : null}
                    <p className="mt-1 text-xs text-slate-500">
                      The locally calculated inspection hash stays review evidence and is never promoted to a trusted value. A checksum entered here is stored in the generated App Definition.
                    </p>
                  </label>
                ) : null}
                {state.phase === "downloading" ? (
                  <p className="text-xs text-slate-600 md:col-span-3" role="status" aria-live="polite">
                    Downloading selected APK. This may take a moment.
                  </p>
                ) : null}
              </div>
            )}
          </section>
          {state.inspection ? (
            <InspectionReview
              inspection={state.inspection}
              disabled={busy}
              automationOptIn={state.automationOptIn}
              onAutomationOptInChange={(value) => dispatch({ type: "automation-opt-in", value })}
              onRuntimeCandidateChange={(index, selected) => dispatch({ type: "runtime-candidate-selected", index, selected })}
              onAppOpCandidateChange={(index, selected) => dispatch({ type: "app-op-candidate-selected", index, selected })}
            />
          ) : null}
          {state.inspection ? (
            <IdentityFields
              state={state}
              disabled={busy}
              onArtifactId={(value) => dispatch({ type: "artifact-id", value })}
              onArtifactName={(value) => dispatch({ type: "artifact-name", value })}
              onRecipeId={(value) => dispatch({ type: "recipe-id", value })}
              onReset={() => dispatch({ type: "identity-reset" })}
            />
          ) : null}
          {state.form ? (
            <>
              <AppFields
                sourceMode={state.sourceMode}
                installStrategy={state.installStrategy}
                targetMode={state.targetMode}
                form={state.form}
                disabled={busy}
                updateForm={updateForm}
                changeApp={changeApp}
                changeMapping={changeMapping}
              />
              <RecipeFields form={state.form} disabled={busy} changeRecipe={changeRecipe} />
              <section className="mt-4 grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-[1fr_auto]">
                <Picker
                  label="Authored root"
                  value={state.rootHandle ? "Selected and ready" : null}
                  button={state.rootHandle ? "Change authored root..." : "Choose authored root..."}
                  disabled={busy}
                  onClick={() => void chooseRoot()}
                />
                <div className="flex items-end">
                  <button
                    className="bg-indigo-700 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
                    disabled={busy || !state.rootHandle}
                    onClick={() => void buildReview()}
                  >
                    {state.publication?.outcome === "unknown" ? "Review again" : "Build reviewed output"}
                  </button>
                </div>
              </section>
              <section className="mt-4 rounded border border-slate-200 p-4">
                <h2 className="font-semibold">Preview</h2>
                {state.draft ? (
                  <>
                    <p className="mt-1 text-sm text-slate-600">
                      Editable preview from the current source. A review re-generates the canonical files from the form before anything is saved.
                    </p>
                    <div className="mt-2 grid gap-2 text-sm text-slate-700 md:grid-cols-2">
                      <p>
                        <strong>{state.draft.appDestination.relativePath ?? "App definition"}</strong>
                        {state.draft.appDestination.relativePath ? " (update)" : " (create)"}
                      </p>
                      <p>
                        <strong>{state.draft.recipeDestination.relativePath ?? "Recipe"}</strong>
                        {state.draft.recipeDestination.relativePath ? " (update)" : " (create)"}
                      </p>
                    </div>
                    <Diagnostics
                      label="File checks"
                      items={visibleDraftDiagnostics(state.draft.diagnostics, state.rootHandle !== null)}
                    />
                    <Diagnostics
                      label="Existing or duplicate identities"
                      emptyMessage="No duplicate or existing identities were detected for this preview."
                      items={(state.collisions?.collisions ?? []).map((item) => ({
                        severity: item.severity === "blocking" ? "error" : "warning",
                        code: item.code,
                        message: item.message,
                      }))}
                    />
                  </>
                ) : (
                  <p className="mt-1 text-sm text-slate-600">
                    {state.previewPending
                      ? "Building the editable preview..."
                      : "No preview built yet. Refresh the preview to edit the App and Recipe fields."}
                  </p>
                )}
              </section>
              {state.review ? <ReviewPanel review={state.review} /> : null}
            </>
          ) : state.apkHandle ? (
            <p className="mt-4 text-sm text-slate-600" role="status">
              {state.previewPending ? "Inspecting and building the editable preview..." : "Waiting for a preview."}
            </p>
          ) : null}
        </div>
        <footer className="flex items-center justify-between border-t border-slate-200 px-5 py-3">
          <p className="text-xs text-slate-500">
            Files are only written when the reviewed output is saved.
          </p>
          <div className="flex items-center gap-2">
            <button
              className="rounded border border-slate-300 px-4 py-2 text-sm font-medium disabled:text-slate-400"
              disabled={busy || !state.apkHandle}
              onClick={() => void generatePreview()}
            >
              {state.previewPending ? "Refreshing preview..." : "Refresh preview"}
            </button>
            <button
              className="rounded border border-slate-300 px-4 py-2 text-sm font-medium disabled:text-slate-400"
              disabled={reviewDisabled || Boolean(state.review)}
              onClick={() => void buildReview()}
            >
              {state.reviewPending
                ? "Reviewing..."
                : state.publication?.outcome === "unknown"
                  ? "Review again"
                  : "Review"}
            </button>
            <button
              className="bg-emerald-700 px-4 py-2 text-sm font-medium text-white disabled:bg-slate-300"
              disabled={saveDisabled}
              onClick={() => void saveReview()}
            >
              {state.savePending ? "Saving..." : "Save App and Recipe"}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );
}

/**
 * Return the actionable error for an inspected package that does not match the
 * selected existing App.
 *
 * Generating into an existing App locks its package id, so the author must
 * create a different App rather than updating the selected one.
 */
function packageMismatchMessage(mismatch: { inspected: string; selected: string }): string {
  return "The inspected APK package " + mismatch.inspected + " does not match the selected App package " + mismatch.selected + ". Create a different App instead of updating this one.";
}

const UNKNOWN_PUBLICATION_MESSAGE =
  "Publication status unknown — the catalog may already contain these files. Do not repeat the save.";

function PublicationPanel({ publication, status, statusPending, onCheckStatus, onReviewAgain, reviewDisabled }: {
  publication: AppGeneratorPublicationOutcomeDto;
  status: AppGeneratorPublicationStatusResult | null;
  statusPending: boolean;
  onCheckStatus: () => void;
  onReviewAgain: () => void;
  reviewDisabled: boolean;
}) {
  if (publication.outcome === "saved") {
    const openError = publication.recipe.openError;
    const openFailed = publication.recipe.documentId === null;
    const openFailure = openError
      ? ": " + (openError.message.endsWith(".") ? openError.message : openError.message + ".")
      : ".";
    return (
      <section className="mb-4 rounded border border-emerald-300 bg-emerald-50 p-4 text-sm text-emerald-900">
        <h2 className="font-semibold">Save completed</h2>
        <p className="mt-1">
          Wrote <strong>{publication.app.appPath}</strong> and <strong>{publication.recipe.recipePath}</strong>.
        </p>
        {openFailed ? (
          <p className="mt-1">
            The editor could not open the saved Recipe automatically{openFailure}{" "}
            Open <strong>{publication.recipe.recipePath}</strong> manually to review it.
          </p>
        ) : null}
        {publication.createdFiles.length > 0 ? (
          <p className="mt-1">Created: {publication.createdFiles.join(", ")}.</p>
        ) : null}
        {publication.replacedFiles.length > 0 ? (
          <p className="mt-1">Updated: {publication.replacedFiles.join(", ")}.</p>
        ) : null}
      </section>
    );
  }
  if (publication.outcome === "failed") {
    return (
      <section role="alert" className="mb-4 rounded border border-red-300 bg-red-50 p-4 text-sm text-red-900">
        <h2 className="font-semibold">Save failed</h2>
        <p className="mt-1">{publication.error.message}</p>
        {publication.rollback.recipeRecoveryRequired ? (
          <p className="mt-1">
            The generated Recipe file needs manual recovery
            {publication.rollback.recipePath ? " at " + publication.rollback.recipePath : ""}.
          </p>
        ) : publication.rollback.recipeRemoved ? (
          <p className="mt-1">The Recipe file created by this attempt was removed again.</p>
        ) : null}
        <p className="mt-2">Build a new reviewed output before another save.</p>
      </section>
    );
  }
  const statusResolved = status !== null && status.outcome !== "unknown";
  const failureRecovery =
    status !== null && status.outcome === "failed" ? publicationFailureRecovery(status) : null;
  return (
    <section role="alert" className="mb-4 rounded border border-amber-400 bg-amber-50 p-4 text-sm text-amber-900">
      <h2 className="font-semibold">Uncertain save</h2>
      <p className="mt-1 font-medium">{UNKNOWN_PUBLICATION_MESSAGE}</p>
      <p className="mt-1 text-xs">Operation {publication.operationId}.</p>
      {status !== null ? (
        <>
          <p className="mt-2">
            {status.outcome === "saved"
              ? "A recorded save was confirmed for this operation; the catalog contains these files."
              : status.outcome === "failed"
                ? "This operation is recorded as failed."
                : "No completed record was found for this operation. The catalog may still contain these files even if the record is missing."}
          </p>
          {failureRecovery !== null ? (
            <>
              {failureRecovery.message ? (
                <p className="mt-1">Recorded failure: {failureRecovery.message}</p>
              ) : null}
              {failureRecovery.recipeRecoveryRequired ? (
                <p className="mt-1">
                  The Recipe file this attempt created needs manual recovery
                  {failureRecovery.recipePath ? " at " + failureRecovery.recipePath : ""}.
                </p>
              ) : failureRecovery.recipeRemoved ? (
                <p className="mt-1">The Recipe file this attempt created was removed again.</p>
              ) : null}
            </>
          ) : null}
        </>
      ) : null}
      <div className="mt-3 flex items-center gap-2">
        <button
          className="rounded border border-amber-500 bg-white px-3 py-1.5 text-sm font-medium text-amber-900 disabled:text-amber-400"
          disabled={statusPending}
          onClick={onCheckStatus}
        >
          {statusPending ? "Checking status..." : "Check status"}
        </button>
        <button
          className="rounded border border-amber-500 bg-white px-3 py-1.5 text-sm font-medium text-amber-900 disabled:text-amber-400"
          disabled={reviewDisabled}
          onClick={onReviewAgain}
        >
          Review again
        </button>
        {statusResolved ? null : (
          <span className="text-xs">No automatic retry happens; review again keeps the same ids.</span>
        )}
      </div>
    </section>
  );
}

function TargetAppSection({ state, busy, onModeChange, onRefreshApps, onSelectApp }: {
  state: AppGeneratorState;
  busy: boolean;
  onModeChange: (mode: AppGeneratorTargetMode) => void;
  onRefreshApps: () => void;
  onSelectApp: (appPath: string) => void;
}) {
  const inventoryIncomplete = state.existingAppsInventory?.state === "incomplete";
  return (
    <section className="rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Target App</h2>
      <div className="mt-2 flex flex-wrap gap-4 text-sm">
        <label className="flex items-center gap-2">
          <input
            type="radio"
            name="app-generation-target"
            checked={state.targetMode === "new_app"}
            disabled={busy}
            onChange={() => onModeChange("new_app")}
          />
          Create a new App Definition
        </label>
        <label className="flex items-center gap-2">
          <input
            type="radio"
            name="app-generation-target"
            checked={state.targetMode === "existing_app"}
            disabled={busy}
            onChange={() => onModeChange("existing_app")}
          />
          Add a new artifact to an existing App
        </label>
      </div>
      {state.targetMode === "existing_app" ? (
        <div className="mt-3">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm text-slate-600">
              The selected App keeps its identity and facts. A review adds one new named APK artifact and reports every change.
            </p>
            <button
              className="rounded border border-slate-300 px-3 py-1.5 text-xs"
              disabled={busy}
              onClick={onRefreshApps}
            >
              {state.existingAppsPending ? "Loading Apps..." : "Refresh Apps"}
            </button>
          </div>
          {inventoryIncomplete && state.existingAppsInventory ? (
            <div role="alert" className="mt-3 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">
              <p>The catalog could not be read completely, so existing App selection is unavailable until it is repaired.</p>
              <ul className="mt-1 list-disc pl-5">
                {state.existingAppsInventory.diagnostics.map((diagnostic, index) => (
                  <li key={index}>{diagnostic.file ? diagnostic.file + ": " : ""}{diagnostic.message}</li>
                ))}
              </ul>
            </div>
          ) : null}
          {state.existingApp ? (
            <div className="mt-3 rounded border border-slate-200 bg-slate-50 p-3 text-sm">
              <p className="font-medium">{state.existingApp.name} ({state.existingApp.appId})</p>
              <p className="mt-1 text-slate-600">{state.existingApp.appPath} · package {state.existingApp.packageId}</p>
              <p className="mt-1 text-slate-600">
                {state.existingApp.artifactIds.length} artifact(s) · baseline permissions{" "}
                {state.existingApp.permissionSetSummary.baseline} · elevated {state.existingApp.permissionSetSummary.elevated}
                {state.existingApp.launcherActivity ? " · launcher " + state.existingApp.launcherActivity : ""}
              </p>
              <button
                className="mt-2 rounded border border-slate-300 px-3 py-1.5 text-xs"
                disabled={busy}
                onClick={onRefreshApps}
              >
                Choose a different App
              </button>
            </div>
          ) : null}
          {!inventoryIncomplete && state.existingApps && !state.existingApp ? (
            <ul className="mt-3 space-y-2">
              {state.existingApps.map((app: ExistingAppSummaryDto) => (
                <li key={app.appPath}>
                  <button
                    className="w-full rounded border border-slate-300 px-3 py-2 text-left text-sm disabled:border-slate-200 disabled:text-slate-400"
                    disabled={busy || !app.selectable}
                    onClick={() => onSelectApp(app.appPath)}
                  >
                    <span className="font-medium">{app.name ?? app.appPath}</span>
                    <span className="ml-2 text-xs text-slate-500">
                      {app.appPath}{app.packageId ? " · " + app.packageId : ""}
                    </span>
                    {!app.selectable ? <span className="ml-2 text-xs text-red-700">Not selectable</span> : null}
                  </button>
                  {app.diagnostics.length > 0 ? (
                    <ul className="mt-1 list-disc pl-5 text-xs text-amber-700">
                      {app.diagnostics.map((diagnostic, index) => (
                        <li key={index}>{diagnostic.message}</li>
                      ))}
                    </ul>
                  ) : null}
                </li>
              ))}
              {state.existingApps.length === 0 ? (
                <li className="text-sm text-slate-500">No catalog App Definitions were found.</li>
              ) : null}
            </ul>
          ) : null}
          {!inventoryIncomplete && state.existingAppsPending && !state.existingApps ? (
            <p className="mt-3 text-sm text-slate-600" role="status">Loading catalog Apps...</p>
          ) : null}
        </div>
      ) : (
        <p className="mt-2 text-sm text-slate-600">
          Create a new App Definition from the selected APK or remote source.
        </p>
      )}
    </section>
  );
}

function IdentityFields({ state, disabled, onArtifactId, onArtifactName, onRecipeId, onReset }: {
  state: AppGeneratorState;
  disabled: boolean;
  onArtifactId: (value: string) => void;
  onArtifactName: (value: string) => void;
  onRecipeId: (value: string) => void;
  onReset: () => void;
}) {
  const artifactId = state.appArtifactId ?? state.proposedArtifactId;
  const recipeId = state.recipeId ?? effectiveRecipeId(state);
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">New artifact identity</h2>
        <button
          className="rounded border border-slate-300 px-3 py-1.5 text-xs"
          disabled={disabled}
          onClick={onReset}
        >
          Reset to proposed IDs
        </button>
      </div>
      <p className="mt-1 text-sm text-slate-600">
        The reviewed identity is used exactly as entered. A collision asks for a different id explicitly; nothing is
        renamed, suffixed, or reassigned silently.
      </p>
      <div className="mt-3 grid gap-3 md:grid-cols-3">
        <Field
          label="App Artifact ID"
          help="Identifier of the new APK artifact inside the App Definition."
          value={artifactId}
          disabled={disabled}
          onChange={onArtifactId}
        />
        <Field
          label="Artifact display name (optional)"
          help="Display-only name of the artifact. It never changes ids."
          value={state.artifactName}
          disabled={disabled}
          onChange={onArtifactName}
        />
        <Field
          label="Recipe ID"
          help="Identifier of the generated Recipe. Defaults to app.<app-id>.<artifact-id>.install."
          value={recipeId}
          disabled={disabled}
          onChange={onRecipeId}
        />
      </div>
      {state.publication?.outcome === "unknown" ? (
        <p className="mt-2 text-xs text-amber-700">
          Review again keeps these exact ids so an earlier attempt blocks a duplicate publication even after a restart.
        </p>
      ) : null}
    </section>
  );
}

function destinationLabel(destination: { fileName: string; relativePath: string } | null, fallback: string): string {
  if (!destination) return fallback;
  return destination.relativePath !== null ? destination.relativePath : destination.fileName;
}

function PermissionSetList({ title, actions }: { title: string; actions: ReviewPermissionActionDto[] }) {
  return (
    <section className="rounded border border-slate-200 bg-slate-50 p-3">
      <h3 className="text-sm font-semibold">{title}</h3>
      <ul className="mt-2 space-y-1 text-sm">
        {actions.map((action, index) => (
          <li key={index}><code className="text-xs">{permissionActionLabel(action)}</code></li>
        ))}
        {actions.length === 0 ? <li className="text-slate-500">No actions.</li> : null}
      </ul>
    </section>
  );
}

function permissionActionLabel(action: ReviewPermissionActionDto): string {
  if (action.kind === "runtime_permission") {
    return action.permission;
  }
  const parts = [action.permission];
  if (action.op) parts.push("op " + action.op);
  if (action.mode) parts.push("mode " + action.mode);
  return parts.join(" · ");
}

function ReviewPanel({ review }: { review: AppGeneratorReviewResult }) {
  const impact = review.changeImpact;
  const automation = review.permissionAutomation;
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Reviewed output</h2>
      <p className="mt-1 text-sm text-slate-600">
        {review.target.mode === "existing_app"
          ? "Updates App " + review.target.appId
          : "Creates App " + review.target.appId}
        {review.target.appPath ? " at " + review.target.appPath : ""}
        {review.target.packageLocked ? ". The package id is locked to " + review.target.packageId : ""}
        .
      </p>
      <dl className="mt-3 grid grid-cols-[10rem_1fr] gap-2 text-sm">
        <dt className="font-medium text-slate-500">Artifact</dt>
        <dd>
          {review.artifact ? (
            <>
              <code>{review.artifact.appArtifactId}</code> ({review.artifact.kind}, {review.artifact.change}) · source{" "}
              {review.artifact.source.strategy}
              {review.artifact.name ? " · " + review.artifact.name : ""}
            </>
          ) : (
            "Not resolved. The reviewed App Artifact ID does not name an artifact in this draft; fix the identity reported below and review again."
          )}
        </dd>
        <dt className="font-medium text-slate-500">App file</dt>
        <dd>
          {destinationLabel(review.destinations.app, "Not resolved")}
          {review.target.mode === "existing_app" ? " (replaces the existing file)" : " (create)"}
        </dd>
        <dt className="font-medium text-slate-500">Recipe file</dt>
        <dd>
          {destinationLabel(review.destinations.recipe, "Not resolved")}
          {" (create; an existing destination blocks the save)"}
        </dd>
      </dl>
      {review.blocking ? (
        <p role="alert" className="mt-3 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-800">
          This review is blocking. Choose different identities explicitly and review again; a blocking review cannot be
          saved.
        </p>
      ) : null}
      <Diagnostics
        label="Review diagnostics"
        emptyMessage="No blocking or advisory diagnostics were reported."
        items={review.diagnostics}
      />
      <section className="mt-4">
        <h3 className="text-sm font-semibold text-slate-700">Permission automation</h3>
        {automation.enabled ? (
          <div className="mt-2 grid gap-3 md:grid-cols-2">
            <PermissionSetList title="Baseline (no root)" actions={automation.baseline.actions} />
            <PermissionSetList title="Elevated (root-required, optional)" actions={automation.elevated.actions} />
          </div>
        ) : (
          <p className="mt-1 text-sm text-slate-600">No permission actions are added to the App policy by this review.</p>
        )}
        <p className="mt-1 text-xs text-slate-600">
          Elevated actions require root, stay optional, and are never a launch dependency.
        </p>
      </section>
      <section className="mt-4">
        <h3 className="text-sm font-semibold text-slate-700">Change impact</h3>
        <p className="mt-1 text-sm text-slate-600">
          Added artifacts: {impact.addedArtifactIds.length > 0 ? impact.addedArtifactIds.join(", ") : "none"}. Preserved
          artifacts: {impact.preservedArtifactIds.length > 0 ? impact.preservedArtifactIds.join(", ") : "none"}.
        </p>
        <p className="mt-1 text-sm text-slate-600">
          Added permission actions — baseline: {impact.addedPermissionActions.baseline.length}; elevated:{" "}
          {impact.addedPermissionActions.elevated.length}.
        </p>
        {impact.affectedRecipes.length > 0 ? (
          <div className="mt-2 space-y-2">
            {impact.affectedRecipes.map((recipe) => (
              <div key={recipe.recipeId} className="rounded border border-slate-200 bg-slate-50 p-2 text-sm">
                <p className="font-medium">{recipe.recipeId} ({recipe.status})</p>
                <p className="text-xs text-slate-500">{recipe.recipePath}</p>
                {recipe.diagnostics.length > 0 ? (
                  <ul className="mt-1 list-disc pl-5 text-xs text-amber-700">
                    {recipe.diagnostics.map((diagnostic, index) => (
                      <li key={index}>{diagnostic.message}</li>
                    ))}
                  </ul>
                ) : null}
              </div>
            ))}
          </div>
        ) : (
          <p className="mt-1 text-sm text-slate-600">No other Recipes are affected.</p>
        )}
        {impact.warnings.length > 0 ? (
          <ul className="mt-2 list-disc pl-5 text-xs text-amber-700">
            {impact.warnings.map((warning, index) => (
              <li key={index}>{warning.message}</li>
            ))}
          </ul>
        ) : null}
      </section>
      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <Yaml
          title={review.destinations.app ? review.destinations.app.fileName : "App definition"}
          value={review.appCanonicalYaml}
        />
        <Yaml
          title={review.destinations.recipe ? review.destinations.recipe.fileName : "Recipe"}
          value={review.recipeCanonicalYaml}
        />
      </div>
    </section>
  );
}

interface PermissionEntry {
  key: string;
  permissionName: string;
  detail: string;
  selected: boolean;
  onChange: (selected: boolean) => void;
}

function AppFields({ sourceMode, installStrategy, targetMode, form, disabled, updateForm, changeApp, changeMapping }: {
  sourceMode: AppGeneratorSourceMode;
  installStrategy: AppGeneratorInstallStrategy;
  targetMode: AppGeneratorTargetMode;
  form: AppGeneratorFormState;
  disabled: boolean;
  updateForm: (update: (form: AppGeneratorFormState) => void) => void;
  changeApp: (update: Partial<AppDefinitionV1Dto>) => void;
  changeMapping: (update: Partial<AppMappingRowEdits>) => void;
}) {
  const app = form.app;
  const baselineCount =
    (app.permission_sets?.baseline?.runtime?.length ?? 0)
    + (app.permission_sets?.baseline?.app_ops?.length ?? 0);
  const elevatedCount =
    (app.permission_sets?.elevated?.runtime?.length ?? 0)
    + (app.permission_sets?.elevated?.app_ops?.length ?? 0);
  const identityLocked = targetMode === "existing_app";
  // Existing-App generation preserves the selected App Definition and its
  // mappings, so those edits are unavailable; the artifact and Recipe
  // identities plus the permission opt-in stay editable.
  const appEditsLocked = disabled || identityLocked;
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">App definition</h2>
      {identityLocked ? (
        <p className="mt-1 text-sm text-slate-600">
          The existing App preserves its definition, artifact mappings, and package. Generation adds the named
          artifact and any additive permission actions, and the review reports their effects on existing Recipes.
        </p>
      ) : null}
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Fixed label="Schema / kind" value="1 / app_definition" />
        <Field
          label="App ID"
          help="Stable authored identifier used in filenames and references. Use lowercase letters, numbers, periods, underscores, or hyphens."
          value={app.id}
          disabled={disabled || identityLocked}
          onChange={(id) => changeApp({ id })}
        />
        <Field
          label="Name"
          help="Human-readable app name shown in EmuChef."
          value={app.name}
          disabled={appEditsLocked}
          onChange={(name) => changeApp({ name })}
        />
        <CategoryField
          value={app.category ?? ""}
          disabled={appEditsLocked}
          onChange={(category) => changeApp({ category: category || undefined })}
        />
        <Field
          label="Description"
          help="Optional short description of the app."
          value={app.description ?? ""}
          disabled={appEditsLocked}
          onChange={(description) => changeApp({ description: description || undefined })}
        />
        <Field
          label="Package ID"
          help="Android application ID extracted from the APK manifest. Change it only when the manifest information is known to be wrong."
          value={app.package_id}
          disabled={disabled || identityLocked}
          onChange={(packageId) => changeApp({ package_id: packageId })}
        />
        <Fixed
          label="Installation method"
          help="Controls whether the recipe pins an APK, resolves the latest compatible release, or asks for a local file."
          value={
            installStrategy === "pinned_remote_asset"
              ? "Pinned release"
              : installStrategy === "latest_compatible_release"
                ? "Latest compatible release"
                : "User-provided APK"
          }
        />
        <Fixed
          label="Artifact source strategy"
          help="Describes how the generated App Definition identifies where its files come from."
          value={
            installStrategy === "pinned_remote_asset"
              ? "Direct HTTPS download"
              : installStrategy === "latest_compatible_release"
                ? "Latest provider release"
                : "User-provided file binding"
          }
        />
        <Fixed
          label="Permission sets"
          help="Verified selections are partitioned by the trusted native layer: baseline actions run without root, and elevated actions require root."
          value={
            baselineCount === 0 && elevatedCount === 0
              ? "None selected"
              : "Baseline " + baselineCount + " · Elevated " + elevatedCount
          }
        />
        <Area
          label="Metadata (JSON object)"
          help="Free-form catalog metadata. It must stay a JSON object."
          value={form.mappings.metadata}
          disabled={appEditsLocked}
          onChange={(metadata) => changeMapping({ metadata })}
        />
      </div>
      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <ArtifactList
          values={form.mappings.artifacts}
          disabled={appEditsLocked}
          onChange={(artifacts) => changeMapping({ artifacts })}
        />
        <TargetList
          values={form.mappings.targets}
          disabled={appEditsLocked}
          onChange={(targets) => changeMapping({ targets })}
        />
      </div>
      <p className="mt-3 text-xs text-slate-500">
        {sourceMode === "local_apk"
          ? "The selected APK becomes a user-provided file binding for this App."
          : "The reviewed source becomes the generated artifact's download identity."}
      </p>
    </section>
  );
}

function RecipeFields({ form, disabled, changeRecipe }: {
  form: AppGeneratorFormState;
  disabled: boolean;
  changeRecipe: (update: Partial<AppRecipeEditsDto>) => void;
}) {
  const recipe = form.recipe;
  return (
    <section className="mt-4 rounded border border-slate-200 p-4">
      <h2 className="font-semibold">Recipe</h2>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Fixed label="Schema / kind" value="1 / recipe" />
        <Fixed
          label="Recipe ID"
          help="Edit the Recipe ID in the artifact identity section. It is used exactly as entered."
          value={recipe.ids?.recipeId ?? "Pending"}
        />
        <Field
          label="Recipe name"
          help="Human-readable name shown in plan listings."
          value={recipe.name}
          disabled={disabled}
          onChange={(name) => changeRecipe({ name })}
        />
        <Field
          label="Recipe description"
          help="Short explanation of what this recipe installs."
          value={recipe.description}
          disabled={disabled}
          onChange={(description) => changeRecipe({ description })}
        />
        <Field
          label="APK input label"
          help="Label shown when EmuChef asks the user to choose an APK."
          value={recipe.inputLabel}
          disabled={disabled}
          onChange={(inputLabel) => changeRecipe({ inputLabel })}
        />
        <Field
          label="APK input description"
          help="Guidance shown beside the APK file input."
          value={recipe.inputDescription}
          disabled={disabled}
          onChange={(inputDescription) => changeRecipe({ inputDescription })}
        />
      </div>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <Check
          label="Replace existing installation"
          checked={recipe.replaceExisting}
          disabled={disabled}
          onChange={(replaceExisting) => changeRecipe({ replaceExisting })}
        />
        <Check
          label="Launch once after installation"
          checked={recipe.launchEnabled}
          disabled={disabled}
          onChange={(launchEnabled) => changeRecipe({ launchEnabled })}
        />
      </div>
      <p className="mt-2 text-xs text-slate-500">
        Launch-once is optional and independent of a stored verified launcher activity.
      </p>
    </section>
  );
}

function InspectionReview({ inspection, disabled, automationOptIn, onAutomationOptInChange, onRuntimeCandidateChange, onAppOpCandidateChange }: {
  inspection: ApkInspectionResult;
  disabled: boolean;
  automationOptIn: boolean;
  onAutomationOptInChange: (value: boolean) => void;
  onRuntimeCandidateChange: (index: number, selected: boolean) => void;
  onAppOpCandidateChange: (index: number, selected: boolean) => void;
}) {
  const partition = inspectionPartition(inspection);
  const otherPermissions = otherRequestedPermissions(inspection);
  const rows: Array<[string, string]> = [
    ["Package", inspection.manifest.packageName],
    ["Version name", inspection.manifest.versionName ?? "Not declared"],
    ["Version code", inspection.manifest.versionCode ?? "Not declared"],
    ["Minimum SDK", inspection.manifest.minSdkVersion ?? "Not declared"],
    ["Target SDK", inspection.manifest.targetSdkVersion ?? "Not declared"],
  ];
  function runtimeEntries(indices: number[]): PermissionEntry[] {
    return indices.flatMap((index) => {
      const candidate = inspection.runtimeGrantCandidates[index];
      if (!candidate) return [];
      return [{
        key: "runtime-" + index,
        permissionName: candidate.permissionName,
        detail: androidApiRange(candidate.androidApiMin, candidate.androidApiMax) + ".",
        selected: candidate.selected,
        onChange: (selected: boolean) => onRuntimeCandidateChange(index, selected),
      }];
    });
  }
  function appOpEntries(indices: number[]): PermissionEntry[] {
    return indices.flatMap((index) => {
      const candidate = inspection.appOpCandidates[index];
      if (!candidate) return [];
      return [{
        key: "app-op-" + index,
        permissionName: candidate.permissionName,
        detail:
          "Operation " + candidate.operationName + ", " + candidate.mode + "; "
          + androidApiRange(candidate.androidApiMin, candidate.androidApiMax) + ".",
        selected: candidate.selected,
        onChange: (selected: boolean) => onAppOpCandidateChange(index, selected),
      }];
    });
  }
  return (
    <section className="mt-4 space-y-4">
      <div className="rounded border border-slate-200 p-4">
        <h2 className="font-semibold">APK manifest and integrity metadata</h2>
        <p className="mt-1 text-sm text-slate-600">
          These values were extracted natively from the selected APK manifest.
        </p>
        <dl className="mt-3 grid grid-cols-[9rem_1fr] gap-2 text-sm">
          {rows.map(([label, value]) => (
            <div className="contents" key={label}>
              <dt className="font-medium text-slate-500">{label}</dt>
              <dd className="break-all">{value}</dd>
            </div>
          ))}
          <div className="contents">
            <dt className="font-medium text-slate-500">Calculated SHA-256</dt>
            <dd className="break-all font-mono text-xs">{inspection.calculatedSha256}</dd>
          </div>
          <div className="contents">
            <dt className="font-medium text-slate-500">Checksum comparison</dt>
            <dd>Not compared against a publisher-provided checksum.</dd>
          </div>
          <div className="contents">
            <dt className="font-medium text-slate-500">Signature verification</dt>
            <dd>Not performed. EmuChef does not cryptographically verify APK signatures.</dd>
          </div>
        </dl>
      </div>
      <div className="rounded border border-slate-200 p-4">
        <h2 className="font-semibold">Permission review</h2>
        <p className="mt-1 text-sm text-slate-600">
          Permission automation governs the permission grant steps generation writes. When enabled, the Recipe may
          grant the App's declared permission sets, and the selected candidates are added to App-owned policy. When
          disabled, the App's permission sets are preserved and no permission grant steps are generated. The review
          shows the whole resulting baseline and elevated sets.
        </p>
        <div className="mt-3">
          <Check
            label="Generate permission automation for this App"
            checked={automationOptIn}
            disabled={disabled}
            onChange={onAutomationOptInChange}
          />
        </div>
        <PermissionWarnings inspection={inspection} />
        <div className="mt-4 grid gap-3 md:grid-cols-2">
          <PermissionGroup
            title="Runtime grants — baseline (no root)"
            entries={runtimeEntries(partition.runtime.baseline)}
            disabled={disabled}
          />
          <PermissionGroup
            title="Runtime grants — elevated (root required, optional)"
            entries={runtimeEntries(partition.runtime.elevated)}
            disabled={disabled}
          />
          <PermissionGroup
            title="App-ops — baseline (no root)"
            entries={appOpEntries(partition.appOps.baseline)}
            disabled={disabled}
          />
          <PermissionGroup
            title="App-ops — elevated (root required, optional)"
            entries={appOpEntries(partition.appOps.elevated)}
            disabled={disabled}
          />
        </div>
        <section className="mt-4">
          <h3 className="text-sm font-semibold">Other permissions</h3>
          <p className="mt-1 text-xs text-slate-600">
            Unknown, install-time, non-applicable, and indeterminate permissions are not automated here.
          </p>
          <div className="mt-3 space-y-2">
            {otherPermissions.map((permission, index) => (
              <PermissionDeclaration
                permission={permission}
                warnings={permissionWarnings(inspection, permission.name)}
                key={permission.name + "-" + permission.declarationKind + "-" + (permission.maxSdkVersion ?? "none") + "-" + index}
              />
            ))}
            {otherPermissions.length === 0 ? (
              <p className="text-sm text-slate-500">No other requested permissions.</p>
            ) : null}
          </div>
        </section>
      </div>
    </section>
  );
}

function PermissionGroup({ title, entries, disabled }: {
  title: string;
  entries: PermissionEntry[];
  disabled: boolean;
}) {
  return (
    <section className="rounded border border-slate-200 bg-slate-50 p-3">
      <h3 className="text-sm font-semibold">{title}</h3>
      <div className="mt-2 space-y-2">
        {entries.map((entry) => (
          <label key={entry.key} className="flex items-start gap-2 rounded border border-slate-200 bg-white p-3 text-sm">
            <input
              type="checkbox"
              checked={entry.selected}
              disabled={disabled}
              onChange={(event) => entry.onChange(event.target.checked)}
            />
            <span>
              <code>{entry.permissionName}</code>
              <span className="mt-1 block text-xs text-slate-500">{entry.detail}</span>
            </span>
          </label>
        ))}
        {entries.length === 0 ? <p className="text-sm text-slate-500">No candidates in this set.</p> : null}
      </div>
    </section>
  );
}

function PermissionWarnings({ inspection }: { inspection: ApkInspectionResult }) {
  const warnings = globalInspectionWarnings(inspection);
  if (warnings.length === 0) return null;
  return (
    <section className="mt-3">
      <h3 className="text-sm font-semibold text-amber-800">Inspection context and warnings</h3>
      <div className="mt-2 space-y-2">
        {warnings.map((warning, index) => (
          <div
            className="rounded border border-amber-300 bg-amber-50 p-2 text-sm"
            key={warning.code + "-" + (warning.permissionName ?? "global") + "-" + index}
          >
            {warning.permissionName ? <code className="font-semibold">{warning.permissionName}</code> : null}
            <p className={warning.permissionName ? "mt-1" : ""}>
              {warning.message}
              {warning.applicabilityReason ? " (" + permissionApplicabilityReasonLabel(warning.applicabilityReason) + ")" : ""}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

function PermissionDeclaration({ permission, warnings }: {
  permission: ApkPermissionReviewDto;
  warnings: ApkPermissionWarningDto[];
}) {
  const applicability = permission.applicability;
  const tone = applicability?.status === "not_applicable"
    ? "border-slate-300 bg-slate-50"
    : applicability?.status === "indeterminate"
      ? "border-amber-300 bg-amber-50"
      : "border-slate-200 bg-white";
  return (
    <div className={"rounded border p-3 text-sm " + tone}>
      <code className="font-semibold">{permission.name}</code>
      <p className="mt-1 text-xs text-slate-600">
        Declaration: {declarationKindLabel(permission.declarationKind)}
        {permission.maxSdkVersion ? " (max SDK " + permission.maxSdkVersion + ")" : ""}.
      </p>
      <p className="mt-1 text-xs text-slate-600">
        Classification: {classificationLabel(permission.classification)}.
      </p>
      {applicability ? (
        <p className="mt-1 text-xs text-slate-600">Applicability: {permissionApplicabilityLabel(applicability)}.</p>
      ) : null}
      {warnings.map((warning, index) => (
        <p
          key={warning.code + "-" + index}
          className="mt-2 rounded border border-amber-300 bg-amber-50 p-2 text-xs text-amber-900"
        >
          {warning.message}
          {warning.applicabilityReason
            ? " (" + permissionApplicabilityReasonLabel(warning.applicabilityReason) + ")"
            : ""}
        </p>
      ))}
    </div>
  );
}

function ReleasePatternPreview({ busy, pattern, preview, onPatternChange }: {
  busy: boolean;
  pattern: string;
  preview: ReleasePatternPreviewResult;
  onPatternChange: (value: string) => void;
}) {
  const visibleReleases = preview.releases.slice(0, 10);
  return (
    <section className="rounded border border-slate-200 bg-slate-50 p-3 text-sm md:col-span-3">
      <label>
        <HelpLabel
          label="APK filename pattern (optional)"
          help="Leave this blank to select by APK file type alone. A raw regular expression is previewed against the analyzed releases. Rust performs authoritative validation before generation."
        />
        <input
          className="mt-1 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
          disabled={busy}
          value={pattern}
          onChange={(event) => onPatternChange(event.target.value)}
        />
      </label>
      <p className="mt-2 text-xs text-slate-500">
        Immediate browser preview from already analyzed releases; no network request is made. Rust performs final
        validation.
      </p>
      {preview.patternError ? (
        <p className="mt-2 text-red-700">{preview.patternError}</p>
      ) : (
        <>
          <p className={"mt-2 " + (preview.blocking ? "text-red-700" : "text-slate-700")}>
            {preview.blockingMessage ?? "The newest eligible analyzed release has exactly one eligible APK."}
          </p>
          <p className="mt-2 text-xs text-slate-600">
            {preview.eligibleReleaseCount} eligible release{preview.eligibleReleaseCount === 1 ? "" : "s"}:{" "}
            {preview.uniqueMatchCount} unique, {preview.noMatchCount} no match, {preview.multipleMatchesCount} multiple
            matches.
          </p>
          <div className="mt-3 overflow-x-auto">
            <table className="w-full border-collapse text-left text-xs">
              <thead>
                <tr className="border-b border-slate-300 text-slate-600">
                  <th className="p-2 font-semibold">Release</th>
                  <th className="p-2 font-semibold">Outcome</th>
                  <th className="p-2 font-semibold">Matching APK assets</th>
                </tr>
              </thead>
              <tbody>
                {visibleReleases.map((release, index) => (
                  <tr
                    className={
                      "border-b border-slate-200 "
                      + (release.outcome === "multiple_matches"
                        ? "bg-red-50 text-red-800"
                        : release.outcome === "no_match"
                          ? "bg-amber-50 text-amber-800"
                          : "")
                    }
                    key={release.releaseTag + "-" + index}
                  >
                    <td className="p-2 align-top">
                      <span className="font-medium">{release.releaseTag}</span>
                      {release.prerelease ? <span className="ml-1 text-slate-500">(prerelease)</span> : null}
                      {index === 0 ? <span className="ml-1 font-semibold">— order</span> : null}
                    </td>
                    <td className="p-2 align-top">{releasePatternOutcomeLabel(release.outcome)}</td>
                    <td className="p-2 align-top">
                      {release.matchingAssetNames.length > 0 ? release.matchingAssetNames.join(", ") : "None"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {preview.releases.length > visibleReleases.length ? (
            <p className="mt-2 text-xs text-slate-500">Showing the 10 newest releases.</p>
          ) : null}
          <p className="mt-2 text-xs text-amber-700">
            Future releases are resolved when provisioning runs.
          </p>
        </>
      )}
    </section>
  );
}

function RemoteAssetPicker({ analysis, selectedAssetHandle, disabled, onSelect }: {
  analysis: RemoteSourceAnalysisResult;
  selectedAssetHandle: string | null;
  disabled: boolean;
  onSelect: (assetHandle: string) => void;
}) {
  return (
    <section className="rounded border border-slate-200 bg-slate-50 p-3">
      <h3 className="text-sm font-semibold">Available APK files</h3>
      {analysis.repository ? (
        <p className="mt-1 text-sm text-slate-600">
          {analysis.repository.fullName}
          {analysis.repository.description ? " — " + analysis.repository.description : ""}
        </p>
      ) : null}
      <div className="mt-3 space-y-2">
        {analysis.assets.map((asset) => (
          <label
            className="flex items-start gap-2 rounded border border-slate-200 bg-white p-3 text-sm"
            key={asset.assetHandle}
          >
            <input
              type="radio"
              name="remote-apk-asset"
              checked={selectedAssetHandle === asset.assetHandle}
              disabled={disabled}
              onChange={() => onSelect(asset.assetHandle)}
            />
            <span>
              <span className="font-medium">{asset.fileName}</span>
              <span className="ml-2 text-xs text-slate-500">
                {asset.releaseTag ? asset.releaseTag + (asset.prerelease ? " (prerelease)" : "") : "Direct download"}
                {asset.size ? " · " + Math.ceil(asset.size / 1024 / 1024) + " MiB" : ""}
              </span>
            </span>
          </label>
        ))}
        {analysis.assets.length === 0 ? <p className="text-sm text-amber-700">No eligible APK files were found.</p> : null}
      </div>
    </section>
  );
}

function Picker({ label, value, button, disabled, onClick }: {
  label: string;
  value: string | null;
  button: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <div>
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</p>
      <button className="mt-1 rounded border border-slate-300 px-3 py-2 text-sm" disabled={disabled} onClick={onClick}>
        {button}
      </button>
      <p className="mt-1 text-xs text-slate-500">{value ?? "Not selected"}</p>
    </div>
  );
}

function Yaml({ title, value }: { title: string; value: string | null }) {
  return (
    <div>
      <h3 className="text-sm font-semibold">{title}</h3>
      <pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs text-slate-100">
        {value ?? "Fix the errors above to preview this file."}
      </pre>
    </div>
  );
}

function Diagnostics({ label, items, emptyMessage }: {
  label: string;
  items: Array<{ severity: string; code: string; message: string }>;
  emptyMessage?: string;
}) {
  return (
    <section className="mt-3">
      <h3 className="text-sm font-semibold text-slate-700">{label}</h3>
      {items.length === 0 ? (
        emptyMessage ? <p className="mt-1 text-sm text-emerald-700">{emptyMessage}</p> : null
      ) : (
        <div className="mt-2 space-y-2">
          {items.map((item, index) => (
            <div
              className={
                "rounded p-2 text-sm "
                + (item.severity === "error" || item.severity === "blocking"
                  ? "border border-red-300 bg-red-50"
                  : "border border-amber-300 bg-amber-50")
              }
              key={item.code + "-" + index}
            >
              <p className="font-semibold">{diagnosticDisplayTitle(item.code, item.severity)}</p>
              <p className="mt-0.5">{item.message}</p>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function HelpLabel({ label, help }: { label: string; help?: string }) {
  return (
    <span className="inline-flex items-center gap-1 font-medium">
      {label}
      {help ? (
        <span
          aria-label={label + " help"}
          className="cursor-help rounded-full border border-slate-300 px-1 text-[10px] leading-4 text-slate-500"
          title={help}
        >
          ?
        </span>
      ) : null}
    </span>
  );
}

function CategoryField({ value, disabled, onChange }: {
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="text-sm">
      <HelpLabel
        label="Category (optional)"
        help="Optional free-form label that groups similar apps. Suggestions are common labels; any text is accepted."
      />
      <input
        className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
        disabled={disabled}
        list="app-category-options"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id="app-category-options">
        <option value="emulator" />
        <option value="frontend" />
        <option value="launcher" />
        <option value="tool" />
        <option value="utility" />
      </datalist>
    </label>
  );
}

function ArtifactList({ values, disabled, onChange }: {
  values: AppArtifactRowEdits[];
  disabled: boolean;
  onChange: (values: AppArtifactRowEdits[]) => void;
}) {
  const update = (index: number, patch: Partial<AppArtifactRowEdits>) => {
    onChange(values.map((row, rowIndex) => (rowIndex === index ? { ...row, ...patch } : row)));
  };
  return (
    <div className="text-sm">
      <div className="flex items-center justify-between">
        <HelpLabel
          label="App artifacts"
          help="Files this app owns. Each artifact selects a user-provided file binding, a durable direct HTTPS download, or a latest provider release."
        />
        <button
          className="rounded border border-slate-300 px-2 py-1 text-xs"
          disabled={disabled}
          onClick={() => onChange([...values, emptyArtifactRow()])}
        >
          Add artifact
        </button>
      </div>
      <div className="mt-1 space-y-3">
        {values.map((row, index) => (
          <div className="rounded border border-slate-200 p-3" key={index}>
            <div className="grid gap-2 md:grid-cols-2">
              <Field
                label="ID"
                value={row.id}
                disabled={disabled}
                onChange={(id) => update(index, { id })}
              />
              <label className="text-sm">
                <HelpLabel label="Kind" />
                <select
                  className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                  disabled={disabled}
                  value={row.kind}
                  onChange={(event) => update(index, { kind: event.target.value as AppArtifactKindDto })}
                >
                  <option value="apk">APK</option>
                  <option value="file">File</option>
                </select>
              </label>
              <Field
                label="Name"
                value={row.name}
                disabled={disabled}
                onChange={(name) => update(index, { name })}
              />
              <Field
                label="Description"
                value={row.description}
                disabled={disabled}
                onChange={(description) => update(index, { description })}
              />
              <label className="text-sm">
                <HelpLabel label="Source strategy" />
                <select
                  className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                  disabled={disabled}
                  value={row.strategy}
                  onChange={(event) => update(index, { strategy: event.target.value as AppArtifactStrategyDto })}
                >
                  <option value="user_provided">User-provided file</option>
                  <option value="direct_url">Direct HTTPS URL</option>
                  <option value="latest_release">Latest provider release</option>
                </select>
              </label>
              <div className="flex items-end">
                <button
                  className="w-full rounded border border-red-300 px-2 py-2 text-xs text-red-700"
                  disabled={disabled}
                  onClick={() => onChange(values.filter((_, rowIndex) => rowIndex !== index))}
                >
                  Remove artifact
                </button>
              </div>
            </div>
            {row.strategy === "direct_url" ? (
              <div className="mt-2 grid gap-2 md:grid-cols-2">
                <Field label="HTTPS URL" value={row.url} disabled={disabled} onChange={(url) => update(index, { url })} />
                <Field
                  label="SHA-256"
                  help="Lowercase 64-character hexadecimal checksum copied from a trusted publisher source."
                  value={row.sha256}
                  disabled={disabled}
                  onChange={(sha256) => update(index, { sha256 })}
                />
              </div>
            ) : null}
            {row.strategy === "latest_release" ? (
              <div className="mt-2 grid gap-2 md:grid-cols-2">
                <label className="text-sm">
                  <HelpLabel label="Provider" />
                  <select
                    className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                    disabled={disabled}
                    value={row.provider}
                    onChange={(event) => update(index, { provider: event.target.value as AppReleaseProviderDto })}
                  >
                    <option value="github">GitHub</option>
                    <option value="gitlab">GitLab</option>
                    <option value="forgejo">Forgejo</option>
                  </select>
                </label>
                <Field label="Service origin" value={row.baseUrl} disabled={disabled} onChange={(baseUrl) => update(index, { baseUrl })} />
                <Field
                  label="Repository"
                  help="GitHub and Forgejo use owner/repository. GitLab accepts nested namespaces group/subgroup/project."
                  value={row.repository}
                  disabled={disabled}
                  onChange={(repository) => update(index, { repository })}
                />
                <Field
                  label="APK filename pattern (optional)"
                  help="Leave this blank to accept only an eligible APK asset of the resolved release, whatever its file name is."
                  value={row.assetPattern}
                  disabled={disabled}
                  onChange={(assetPattern) => update(index, { assetPattern })}
                />
                <Check
                  label="Invert filename pattern"
                  checked={row.invertAssetPattern}
                  disabled={disabled || row.assetPattern.trim().length === 0}
                  onChange={(invertAssetPattern) => update(index, { invertAssetPattern })}
                />
                <Check
                  label="Allow prereleases"
                  checked={row.prerelease}
                  disabled={disabled}
                  onChange={(prerelease) => update(index, { prerelease })}
                />
              </div>
            ) : null}
          </div>
        ))}
        {values.length === 0 ? <p className="text-xs text-slate-500">No app artifacts.</p> : null}
      </div>
    </div>
  );
}

function TargetList({ values, disabled, onChange }: {
  values: AppTargetRowEdits[];
  disabled: boolean;
  onChange: (values: AppTargetRowEdits[]) => void;
}) {
  const update = (index: number, patch: Partial<AppTargetRowEdits>) => {
    onChange(values.map((row, rowIndex) => (rowIndex === index ? { ...row, ...patch } : row)));
  };
  return (
    <div className="text-sm">
      <div className="flex items-center justify-between">
        <HelpLabel
          label="App targets"
          help="Files and directories this app owns or uses. Paths are literal and relative to the chosen location."
        />
        <button
          className="rounded border border-slate-300 px-2 py-1 text-xs"
          disabled={disabled}
          onClick={() => onChange([...values, emptyTargetRow()])}
        >
          Add target
        </button>
      </div>
      <div className="mt-1 space-y-2">
        {values.map((row, index) => (
          <div className="grid gap-2 rounded border border-slate-200 p-3 md:grid-cols-2" key={index}>
            <Field label="ID" value={row.id} disabled={disabled} onChange={(id) => update(index, { id })} />
            <label className="text-sm">
              <HelpLabel label="Kind" />
              <select
                className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                disabled={disabled}
                value={row.kind}
                onChange={(event) => update(index, { kind: event.target.value as AppTargetKindDto })}
              >
                <option value="file">File</option>
                <option value="directory">Directory</option>
              </select>
            </label>
            <label className="text-sm">
              <HelpLabel label="Location" />
              <select
                className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
                disabled={disabled}
                value={row.location}
                onChange={(event) => update(index, { location: event.target.value as AppTargetLocationDto })}
              >
                <option value="data">Internal data</option>
                <option value="external_data">External data</option>
                <option value="internal_storage">Internal storage</option>
                <option value="external_storage">External storage</option>
                <option value="sdcard_path">SD card path</option>
              </select>
            </label>
            <Field label="Path" value={row.path} disabled={disabled} onChange={(path) => update(index, { path })} />
            <div className="flex items-end">
              <button
                className="w-full rounded border border-red-300 px-2 py-2 text-xs text-red-700"
                disabled={disabled}
                onClick={() => onChange(values.filter((_, rowIndex) => rowIndex !== index))}
              >
                Remove target
              </button>
            </div>
          </div>
        ))}
        {values.length === 0 ? <p className="text-xs text-slate-500">No app targets.</p> : null}
      </div>
    </div>
  );
}

function Field({ label, help, value, disabled, onChange }: {
  label: string;
  help?: string;
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="text-sm">
      <HelpLabel label={label} help={help} />
      <input
        className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
        disabled={disabled}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function Area({ label, help, value, disabled, onChange }: {
  label: string;
  help?: string;
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="text-sm">
      <HelpLabel label={label} help={help} />
      <textarea
        className="mt-1 min-h-24 w-full rounded border border-slate-300 px-3 py-2 font-mono text-xs"
        disabled={disabled}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function Fixed({ label, help, value }: { label: string; help?: string; value: string }) {
  return (
    <div className="text-sm">
      <HelpLabel label={label} help={help} />
      <p className="mt-1 rounded bg-slate-100 px-3 py-2 text-sm">{value}</p>
    </div>
  );
}

function Check({ label, checked, disabled, onChange }: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className={"flex items-center gap-2 text-sm " + (disabled ? "cursor-not-allowed text-slate-400" : "cursor-pointer")}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        className={disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer"}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}

function androidApiRange(min: number | null, max: number | null): string {
  if (min === null && max === null) return "any Android API level";
  if (max === null) return "Android API " + min + " and newer";
  if (min === null) return "Android API up to " + max;
  return "Android API " + min + " to " + max;
}

function releasePatternOutcomeLabel(outcome: ReleasePatternOutcome): string {
  switch (outcome) {
    case "unique_match":
      return "Unique match";
    case "no_match":
      return "No match";
    case "multiple_matches":
      return "Multiple matches";
  }
}

function declarationKindLabel(kind: ApkPermissionReviewDto["declarationKind"]): string {
  return kind === "uses_permission_sdk_23" ? "uses-permission-sdk-23" : "uses-permission";
}

function classificationLabel(classification: ApkPermissionClassification | null): string {
  switch (classification) {
    case null:
      return "Unknown";
    case "runtime_grantable":
      return "Runtime-grantable";
    case "runtime_restricted":
      return "Runtime-restricted";
    case "app_op_grantable":
      return "App-op-grantable";
    case "manual_special_access":
      return "Manual or special access";
    case "install_time":
      return "Install-time";
    case "signature_or_privileged":
      return "Signature or privileged";
    case "unknown":
      return "Unknown";
  }
}

function apiFailure<T>(response: Exclude<EditorApiResult<T>, { kind: "success" }>): string {
  return response.kind === "api-error" ? response.error.message : response.message;
}

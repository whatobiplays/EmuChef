import type {
  ApkInspectionResult,
  AppDefinitionV1Dto,
  AppMappingEditsDto,
  AppRecipeCollisionResult,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppRecipeSaveResult,
  AppGeneratorDiagnosticDto,
  AppGeneratorInstallStrategy,
  AppGeneratorSourceMode,
  RemoteSourceAnalysisResult,
  RemoteSourceDescriptorDto,
} from "../api/types.js";
import { parseMetadataObject } from "./deviceProfileGenerator.logic.js";

export type AppGeneratorPhase =
  | "starting"
  | "selecting"
  | "downloading"
  | "inspecting"
  | "editing"
  | "reviewing"
  | "saving"
  | "saved";

export interface AppGeneratorFormState {
  app: AppDefinitionV1Dto;
  recipe: AppRecipeEditsDto;
  mappings: AppMappingEditsDto;
  aliases: string[];
  sharedStoragePaths: string[];
  appDataPaths: string[];
}

export interface AppGeneratorState {
  phase: AppGeneratorPhase;
  sessionHandle: string | null;
  apkHandle: string | null;
  apkLabel: string | null;
  analyzerHandle: string | null;
  analyzerLabel: string | null;
  analyzerKind: "apkanalyzer" | "aapt2";
  sourceMode: AppGeneratorSourceMode;
  sourceUrl: string;
  includePrereleases: boolean;
  sourceAnalysis: RemoteSourceAnalysisResult | null;
  selectedAssetHandle: string | null;
  remoteSource: RemoteSourceDescriptorDto | null;
  installStrategy: AppGeneratorInstallStrategy;
  assetPattern: string;
  rootHandle: string | null;
  rootLabel: string | null;
  inspection: ApkInspectionResult | null;
  draft: AppRecipeDraftResult | null;
  form: AppGeneratorFormState | null;
  collisions: AppRecipeCollisionResult | null;
  saved: AppRecipeSaveResult | null;
  error: string | null;
}

export type AppGeneratorAction =
  | {
      type: "started";
      sessionHandle: string;
      analyzerHandle?: string | null;
      analyzerKind?: "apkanalyzer" | "aapt2" | null;
      analyzerLabel?: string | null;
      rootHandle?: string | null;
      rootLabel?: string | null;
    }
  | { type: "apk-selected"; apkHandle: string; label: string }
  | { type: "source-mode"; mode: AppGeneratorSourceMode }
  | { type: "source-url"; value: string }
  | { type: "include-prereleases"; value: boolean }
  | { type: "source-analyzing" }
  | { type: "source-analyzed"; analysis: RemoteSourceAnalysisResult }
  | { type: "asset-selected"; assetHandle: string }
  | { type: "asset-pattern"; value: string }
  | { type: "install-strategy"; strategy: AppGeneratorInstallStrategy }
  | { type: "downloading" }
  | { type: "remote-downloaded"; apkHandle: string; label: string; source: RemoteSourceDescriptorDto }
  | { type: "analyzer-kind"; kind: "apkanalyzer" | "aapt2" }
  | { type: "analyzer-selected"; analyzerHandle: string; label: string }
  | { type: "inspecting" }
  | { type: "inspected"; inspection: ApkInspectionResult }
  | { type: "drafted"; draft: AppRecipeDraftResult }
  | { type: "form"; form: AppGeneratorFormState }
  | { type: "root-selected"; rootHandle: string; label: string }
  | { type: "reviewed"; draft: AppRecipeDraftResult; collisions: AppRecipeCollisionResult }
  | { type: "saving" }
  | { type: "saved"; result: AppRecipeSaveResult }
  | { type: "failure"; message: string };

export const initialAppGeneratorState: AppGeneratorState = {
  phase: "starting",
  sessionHandle: null,
  apkHandle: null,
  apkLabel: null,
  analyzerHandle: null,
  analyzerLabel: null,
  analyzerKind: "apkanalyzer",
  sourceMode: "local_apk",
  sourceUrl: "",
  includePrereleases: false,
  sourceAnalysis: null,
  selectedAssetHandle: null,
  remoteSource: null,
  installStrategy: "pinned_remote_asset",
  assetPattern: "",
  rootHandle: null,
  rootLabel: null,
  inspection: null,
  draft: null,
  form: null,
  collisions: null,
  saved: null,
  error: null,
};

export function reduceAppGenerator(
  state: AppGeneratorState,
  action: AppGeneratorAction,
): AppGeneratorState {
  switch (action.type) {
    case "started":
      return {
        ...state,
        phase: "selecting",
        sessionHandle: action.sessionHandle,
        analyzerHandle: action.analyzerHandle ?? null,
        analyzerKind: action.analyzerKind ?? state.analyzerKind,
        analyzerLabel: action.analyzerLabel ?? null,
        rootHandle: action.rootHandle ?? null,
        rootLabel: action.rootLabel ?? null,
        error: null,
      };
    case "source-mode":
      return {
        ...state,
        phase: "selecting",
        sourceMode: action.mode,
        sourceUrl: "",
        sourceAnalysis: null,
        selectedAssetHandle: null,
        assetPattern: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "source-url":
      return {
        ...state,
        sourceUrl: action.value,
        sourceAnalysis: null,
        selectedAssetHandle: null,
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "include-prereleases":
      return {
        ...state,
        includePrereleases: action.value,
        sourceAnalysis: null,
        selectedAssetHandle: null,
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "source-analyzing":
      return { ...state, phase: "inspecting", error: null };
    case "source-analyzed":
      return {
        ...state,
        phase: "selecting",
        sourceAnalysis: action.analysis,
        selectedAssetHandle: action.analysis.preselectedAssetHandle,
        assetPattern: suggestedPatternForHandle(
          action.analysis,
          action.analysis.preselectedAssetHandle,
        ),
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "asset-selected":
      return {
        ...state,
        selectedAssetHandle: action.assetHandle,
        assetPattern: suggestedPatternForHandle(state.sourceAnalysis, action.assetHandle),
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "asset-pattern":
      return {
        ...state,
        assetPattern: action.value,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "install-strategy":
      return {
        ...state,
        installStrategy: action.strategy,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "downloading":
      return { ...state, phase: "downloading", error: null };
    case "remote-downloaded":
      return {
        ...state,
        apkHandle: action.apkHandle,
        apkLabel: action.label,
        remoteSource: {
          ...action.source,
          strategy: state.installStrategy,
          assetPattern:
            state.installStrategy === "latest_compatible_release" ? state.assetPattern : null,
          includePrereleases: state.includePrereleases,
        },
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "apk-selected":
      return {
        ...state,
        apkHandle: action.apkHandle,
        apkLabel: action.label,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "analyzer-kind":
      return {
        ...state,
        analyzerKind: action.kind,
        analyzerHandle: null,
        analyzerLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
      };
    case "analyzer-selected":
      return {
        ...state,
        analyzerHandle: action.analyzerHandle,
        analyzerLabel: action.label,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "inspecting":
      return { ...state, phase: "inspecting", error: null };
    case "inspected":
      return { ...state, phase: "editing", inspection: action.inspection, error: null };
    case "drafted":
      return {
        ...state,
        phase: "editing",
        draft: action.draft,
        form: draftToForm(action.draft),
        collisions: null,
        error: null,
      };
    case "form":
      return { ...state, form: action.form, draft: null, collisions: null, error: null };
    case "root-selected":
      return {
        ...state,
        rootHandle: action.rootHandle,
        rootLabel: action.label,
        collisions: null,
        error: null,
      };
    case "reviewed":
      return {
        ...state,
        phase: "reviewing",
        draft: action.draft,
        form: state.form
          ? {
              ...state.form,
              app: structuredClone(action.draft.app),
              recipe: structuredClone(action.draft.recipeEdits),
            }
          : draftToForm(action.draft),
        collisions: action.collisions,
        error: null,
      };
    case "saving":
      return { ...state, phase: "saving", error: null };
    case "saved":
      return { ...state, phase: "saved", saved: action.result, error: null };
    case "failure":
      return { ...state, phase: state.phase === "starting" ? "starting" : "editing", error: action.message };
  }
}

export function draftToForm(draft: AppRecipeDraftResult): AppGeneratorFormState {
  const { type: trackingType, ...trackingFields } = draft.app.tracking_source;
  void trackingType;
  const app = structuredClone(draft.app);
  const recipe = structuredClone(draft.recipeEdits);
  if (app.name === app.package.primary) {
    const originalName = app.name;
    const readableName = readableNameFromPackage(app.package.primary);
    app.name = readableName;
    recipe.name = recipe.name.replace(originalName, readableName);
    recipe.description = recipe.description.replace(originalName, readableName);
    recipe.inputLabel = recipe.inputLabel.replace(originalName, readableName);
    recipe.inputDescription = recipe.inputDescription.replace(originalName, readableName);
  }
  return {
    app,
    recipe,
    mappings: {
      installSourceOptions: JSON.stringify(draft.app.install_source.options, null, 2),
      trackingSourceFields: JSON.stringify(trackingFields, null, 2),
      metadata: JSON.stringify(draft.app.metadata, null, 2),
      inputs: draft.app.inputs.map((value) => JSON.stringify(value, null, 2)),
      configTargets: draft.app.provisioning.config_targets.map((value) =>
        JSON.stringify(value, null, 2),
      ),
    },
    aliases: [...draft.app.package.aliases],
    sharedStoragePaths: [...draft.app.provisioning.shared_storage_paths],
    appDataPaths: [...draft.app.provisioning.app_data_paths],
  };
}

export function readableNameFromPackage(packageName: string): string {
  const tail = packageName.split(".").filter(Boolean).at(-1) ?? packageName;
  return tail
    .split(/[_-]+/u)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function visibleDraftDiagnostics(
  diagnostics: AppGeneratorDiagnosticDto[],
  hasRootBackedReview: boolean,
): AppGeneratorDiagnosticDto[] {
  if (!hasRootBackedReview) return diagnostics;
  return diagnostics.filter((diagnostic) => diagnostic.code !== "validation_context_limited");
}

export function diagnosticDisplayTitle(code: string, severity: string): string {
  const titles: Record<string, string> = {
    validation_context_limited: "Catalog validation not yet available",
    apk_certificate_missing: "Signing certificate unavailable",
    apk_label_missing: "Application name derived",
  };
  return titles[code] ?? (severity === "error" || severity === "blocking" ? "Action required" : "Review recommended");
}

export type FormRequestResult =
  | {
      ok: true;
      app: AppDefinitionV1Dto;
      recipe: AppRecipeEditsDto;
      mappings: AppMappingEditsDto;
    }
  | { ok: false; message: string };

export function formToRequest(form: AppGeneratorFormState): FormRequestResult {
  for (const [label, source] of [
    ["Install-source options", form.mappings.installSourceOptions],
    ["Tracking-source fields", form.mappings.trackingSourceFields],
    ["Metadata", form.mappings.metadata],
    ...form.mappings.inputs.map((source, index) => [`Input metadata ${index + 1}`, source]),
    ...form.mappings.configTargets.map((source, index) => [`Config target ${index + 1}`, source]),
  ] as Array<[string, string]>) {
    const parsed = parseMetadataObject(source);
    if (!parsed.ok) {
      return { ok: false, message: `${label}: ${parsed.message}` };
    }
  }
  const normalizeList = (values: string[]) =>
    values.map((item) => item.trim()).filter((item) => item.length > 0);
  const app = structuredClone(form.app);
  app.package.aliases = normalizeList(form.aliases);
  app.provisioning.shared_storage_paths = normalizeList(form.sharedStoragePaths);
  app.provisioning.app_data_paths = normalizeList(form.appDataPaths);
  return {
    ok: true,
    app,
    recipe: structuredClone(form.recipe),
    mappings: structuredClone(form.mappings),
  };
}

export function eligibleApkAssets(
  analysis: RemoteSourceAnalysisResult | null,
  releaseTag?: string | null,
) {
  if (!analysis) return [];
  return analysis.assets.filter(
    (asset) =>
      asset.fileName.toLowerCase().endsWith(".apk") &&
      (!releaseTag || asset.releaseTag === releaseTag),
  );
}

export function matchingAssetNames(pattern: string, fileNames: string[]): string[] {
  try {
    const expression = new RegExp(pattern, "u");
    return fileNames.filter((fileName) => expression.test(fileName));
  } catch {
    return [];
  }
}

export function assetPatternError(pattern: string, fileNames: string[]): string | null {
  if (!pattern.trim()) return "Enter an APK filename pattern.";
  try {
    const expression = new RegExp(pattern, "u");
    const matches = fileNames.filter((fileName) => expression.test(fileName));
    if (matches.length === 0) {
      return "The pattern does not match an APK in the selected release.";
    }
    if (matches.length > 1) {
      return "The pattern matches multiple APKs. Make it more specific.";
    }
    return null;
  } catch {
    return "Enter a valid regular expression.";
  }
}

export function suggestAssetPattern(
  selectedFileName: string,
  siblingFileNames: string[],
): string {
  if (!selectedFileName) return "";
  const escaped = escapeRegex(selectedFileName);
  const versionSegment = selectedFileName.match(/v?\d+(?:[._-]\d+){1,4}/u)?.[0];
  const generalized = versionSegment
    ? escaped.replace(
        escapeRegex(versionSegment),
        "v?\\d+(?:[._-]\\d+){1,4}",
      )
    : escaped;
  const suggested = `^${generalized}$`;
  const matches = matchingAssetNames(suggested, siblingFileNames);
  return matches.length === 1 && matches[0] === selectedFileName
    ? suggested
    : `^${escaped}$`;
}

function suggestedPatternForHandle(
  analysis: RemoteSourceAnalysisResult | null,
  assetHandle: string | null,
): string {
  if (!analysis || !assetHandle) return "";
  const selected = analysis.assets.find((asset) => asset.assetHandle === assetHandle);
  if (!selected) return "";
  const siblings = eligibleApkAssets(analysis, selected.releaseTag).map(
    (asset) => asset.fileName,
  );
  return suggestAssetPattern(selected.fileName, siblings);
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}


import type {
  ApkInspectionResult,
  AppDefinitionV1Dto,
  AppMappingEditsDto,
  AppRecipeCollisionResult,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppRecipeSaveResult,
} from "../api/types.js";
import { parseMetadataObject } from "./deviceProfileGenerator.logic.js";

export type AppGeneratorPhase =
  | "starting"
  | "selecting"
  | "inspecting"
  | "editing"
  | "reviewing"
  | "saving"
  | "saved";

export interface AppGeneratorFormState {
  app: AppDefinitionV1Dto;
  recipe: AppRecipeEditsDto;
  mappings: AppMappingEditsDto;
  aliasesText: string;
  sharedStoragePathsText: string;
  appDataPathsText: string;
}

export interface AppGeneratorState {
  phase: AppGeneratorPhase;
  sessionHandle: string | null;
  apkHandle: string | null;
  apkLabel: string | null;
  analyzerHandle: string | null;
  analyzerLabel: string | null;
  analyzerKind: "apkanalyzer" | "aapt2";
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
  | { type: "started"; sessionHandle: string }
  | { type: "apk-selected"; apkHandle: string; label: string }
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
      return { ...state, phase: "selecting", sessionHandle: action.sessionHandle, error: null };
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
  return {
    app: structuredClone(draft.app),
    recipe: structuredClone(draft.recipeEdits),
    mappings: {
      installSourceOptions: JSON.stringify(draft.app.install_source.options, null, 2),
      trackingSourceFields: JSON.stringify(trackingFields, null, 2),
      metadata: JSON.stringify(draft.app.metadata, null, 2),
      inputs: draft.app.inputs.map((value) => JSON.stringify(value, null, 2)),
      configTargets: draft.app.provisioning.config_targets.map((value) =>
        JSON.stringify(value, null, 2),
      ),
    },
    aliasesText: draft.app.package.aliases.join("\n"),
    sharedStoragePathsText: draft.app.provisioning.shared_storage_paths.join("\n"),
    appDataPathsText: draft.app.provisioning.app_data_paths.join("\n"),
  };
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
  const lines = (value: string) =>
    value
      .split(/\r?\n/u)
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  const app = structuredClone(form.app);
  app.package.aliases = lines(form.aliasesText);
  app.provisioning.shared_storage_paths = lines(form.sharedStoragePathsText);
  app.provisioning.app_data_paths = lines(form.appDataPathsText);
  return {
    ok: true,
    app,
    recipe: structuredClone(form.recipe),
    mappings: structuredClone(form.mappings),
  };
}

import { invoke } from "@tauri-apps/api/core";

import type { EditorCommand } from "./commands";
import {
  runtimeConfigurationInvokeArgs,
  type RuntimeConfigurationRequest,
} from "./runtimeConfiguration";
import type {
  ApiEnvelope,
  ApiError,
  ApplyRecipeCommandResult,
  DocumentResult,
  EmitRecipeYamlFromPathResult,
  EmitYamlResult,
  GetRefIndexResult,
  ListStepSpecsResult,
  OpenRecipeResult,
  SidecarPingResult,
  SidecarRestartResult,
  SidecarStatusResult,
  ValidateResult,
  ValidateRecipePathResult,
  UserConfigurationCommandResult,
  UserConfigurationDocumentResult,
  ConfigurationDescriptionResult,
  ConfigEditorAuthoredRootResult,
  PlanConfigurationResult,
  DeviceProfileCollisionResult,
  DeviceProfileDraftResult,
  DeviceProfileGeneratorDeviceListResult,
  DeviceProfileGeneratorSessionResult,
  DeviceProfileProbeResult,
  DeviceProfileRootSelectionResult,
  DeviceProfileSaveResult,
  DeviceProfileV1Dto,
  AppDefinitionV1Dto,
  AppGeneratorSelectionResult,
  AppGeneratorSessionResult,
  ApkInspectionResult,
  AppMappingEditsDto,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppRecipeSaveResult,
  AppGeneratorSourceMode,
  AppGeneratorInstallStrategy,
  PermissionSelectionRequestDto,
  RemoteApkDownloadResult,
  RemoteSourceAnalysisResult,
  RemoteSourceDescriptorDto,
} from "./types";

export type EditorApiResult<T> =
  | {
      kind: "success";
      result: T;
    }
  | {
      kind: "api-error";
      error: ApiError;
      debug?: unknown;
    }
  | {
      kind: "transport-error";
      message: string;
    };

export async function listStepSpecs(): Promise<EditorApiResult<ListStepSpecsResult>> {
  return callApi<ListStepSpecsResult>("list_step_specs");
}

export async function openRecipe(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<OpenRecipeResult>> {
  return callApi<OpenRecipeResult>("open_recipe", { path, authoredRoot });
}

export async function validateRecipePath(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<ValidateRecipePathResult>> {
  return callApi<ValidateRecipePathResult>("validate_recipe_path", { path, authoredRoot });
}

export async function emitRecipeYamlFromPath(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<EmitRecipeYamlFromPathResult>> {
  return callApi<EmitRecipeYamlFromPathResult>("emit_recipe_yaml_from_path", {
    path,
    authoredRoot,
  });
}

export async function sidecarStatus(): Promise<EditorApiResult<SidecarStatusResult>> {
  return callApi<SidecarStatusResult>("sidecar_status");
}

export async function sidecarPing(): Promise<EditorApiResult<SidecarPingResult>> {
  return callApi<SidecarPingResult>("sidecar_ping");
}

export async function sidecarRestart(): Promise<EditorApiResult<SidecarRestartResult>> {
  return callApi<SidecarRestartResult>("sidecar_restart");
}

export async function getDocument(documentId: string): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("get_document", { documentId });
}

export async function applyRecipeCommand(
  documentId: string,
  command: EditorCommand,
): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("apply_recipe_command", { documentId, command });
}

export async function undo(documentId: string): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("undo", { documentId });
}

export async function redo(documentId: string): Promise<EditorApiResult<ApplyRecipeCommandResult>> {
  return callApi<ApplyRecipeCommandResult>("redo", { documentId });
}

export async function saveRecipe(documentId: string): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("save_recipe", { documentId });
}

export async function saveRecipeAs(
  documentId: string,
  path: string,
): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("save_recipe_as", { documentId, path });
}

export async function validate(documentId: string): Promise<EditorApiResult<ValidateResult>> {
  return callApi<ValidateResult>("validate", { documentId });
}

export async function emitYaml(documentId: string): Promise<EditorApiResult<EmitYamlResult>> {
  return callApi<EmitYamlResult>("emit_yaml", { documentId });
}

export async function getRefIndex(documentId: string): Promise<EditorApiResult<GetRefIndexResult>> {
  return callApi<GetRefIndexResult>("get_ref_index", { documentId });
}

export async function setDocumentAuthoredRoot(
  documentId: string,
  authoredRoot: string | null,
): Promise<EditorApiResult<DocumentResult>> {
  return callApi<DocumentResult>("set_document_authored_root", { documentId, authoredRoot });
}

export async function openUserConfiguration(
  path: string,
  authoredRoot: string | null = null,
): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("open_user_configuration", { path, authoredRoot });
}

export async function createUserConfiguration(args: {
  path: string;
  configurationId: string;
  name: string;
  devicePlan: string;
  selectedRecipes: string[];
  authoredRoot: string | null;
}): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("create_user_configuration", args);
}

export async function getUserConfigurationDocument(
  documentId: string,
): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("get_user_configuration_document", { documentId });
}

export async function saveUserConfiguration(
  documentId: string,
): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("save_user_configuration", { documentId });
}

export async function saveUserConfigurationAs(
  documentId: string,
  path: string,
): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("save_user_configuration_as", { documentId, path });
}

export async function setUserConfigurationBinding(
  documentId: string,
  key: string,
  value: unknown,
): Promise<EditorApiResult<UserConfigurationCommandResult>> {
  return callApi<UserConfigurationCommandResult>("set_user_configuration_binding", { documentId, key, value });
}

export async function removeUserConfigurationBinding(
  documentId: string,
  key: string,
): Promise<EditorApiResult<UserConfigurationCommandResult>> {
  return callApi<UserConfigurationCommandResult>("remove_user_configuration_binding", { documentId, key });
}

export async function setUserConfigurationSelectedRecipes(
  documentId: string,
  selectedRecipes: string[],
): Promise<EditorApiResult<UserConfigurationCommandResult>> {
  return callApi<UserConfigurationCommandResult>("set_user_configuration_selected_recipes", {
    documentId,
    selectedRecipes,
  });
}

export async function setUserConfigurationDevicePlan(
  documentId: string,
  devicePlan: string,
): Promise<EditorApiResult<UserConfigurationCommandResult>> {
  return callApi<UserConfigurationCommandResult>("set_user_configuration_device_plan", { documentId, devicePlan });
}

export async function validateUserConfiguration(
  documentId: string,
): Promise<EditorApiResult<ValidateResult>> {
  return callApi<ValidateResult>("validate_user_configuration", { documentId });
}

export async function emitUserConfigurationYaml(
  documentId: string,
): Promise<EditorApiResult<EmitYamlResult>> {
  return callApi<EmitYamlResult>("emit_user_configuration_yaml", { documentId });
}

export async function setUserConfigurationAuthoredRoot(
  documentId: string,
  authoredRoot: string | null,
): Promise<EditorApiResult<UserConfigurationDocumentResult>> {
  return callApi<UserConfigurationDocumentResult>("set_user_configuration_authored_root", {
    documentId,
    authoredRoot,
  });
}

export async function closeUserConfiguration(documentId: string): Promise<EditorApiResult<Record<string, never>>> {
  return callApi<Record<string, never>>("close_user_configuration", { documentId });
}

export type {
  InlineUserConfiguration,
  InlineUserConfigurationBinding,
  RuntimeConfigurationRequest,
  RuntimeUserConfigurationSource,
} from "./runtimeConfiguration";

export async function describeConfiguration(
  request: RuntimeConfigurationRequest,
): Promise<EditorApiResult<ConfigurationDescriptionResult>> {
  return callApi<ConfigurationDescriptionResult>(
    "describe_configuration",
    runtimeConfigurationInvokeArgs(request),
  );
}

export async function planConfiguration(
  request: RuntimeConfigurationRequest,
): Promise<EditorApiResult<PlanConfigurationResult>> {
  return callApi<PlanConfigurationResult>("plan_configuration", runtimeConfigurationInvokeArgs(request));
}

export async function getConfigEditorAuthoredRoot(): Promise<
  EditorApiResult<ConfigEditorAuthoredRootResult>
> {
  return callApi<ConfigEditorAuthoredRootResult>("get_config_editor_authored_root");
}

export async function setConfigEditorAuthoredRoot(
  authoredRoot: string | null,
): Promise<EditorApiResult<ConfigEditorAuthoredRootResult>> {
  return callApi<ConfigEditorAuthoredRootResult>("set_config_editor_authored_root", {
    authoredRoot,
  });
}

export async function beginDeviceProfileGenerator(): Promise<
  EditorApiResult<DeviceProfileGeneratorSessionResult>
> {
  return callApi<DeviceProfileGeneratorSessionResult>("begin_device_profile_generator");
}

export async function chooseDeviceProfileAuthoredRoot(
  sessionHandle: string,
): Promise<EditorApiResult<DeviceProfileRootSelectionResult>> {
  return callApi<DeviceProfileRootSelectionResult>("choose_device_profile_authored_root", {
    sessionHandle,
  });
}

export async function setDeviceProfileAuthoredRoot(
  sessionHandle: string,
  authoredRoot: string,
): Promise<EditorApiResult<DeviceProfileRootSelectionResult>> {
  return callApi<DeviceProfileRootSelectionResult>("set_device_profile_authored_root", {
    sessionHandle,
    authoredRoot,
  });
}

export async function listDeviceProfileGeneratorDevices(
  sessionHandle: string,
): Promise<EditorApiResult<DeviceProfileGeneratorDeviceListResult>> {
  return callApi<DeviceProfileGeneratorDeviceListResult>("list_device_profile_generator_devices", {
    sessionHandle,
  });
}

export async function probeDeviceProfileGeneratorDevice(
  sessionHandle: string,
  deviceHandle: string,
): Promise<EditorApiResult<DeviceProfileProbeResult>> {
  return callApi<DeviceProfileProbeResult>("probe_device_profile_generator_device", {
    sessionHandle,
    deviceHandle,
  });
}

export async function generateDeviceProfileDraft(
  sessionHandle: string,
  deviceHandle: string,
  profile: DeviceProfileV1Dto | null = null,
): Promise<EditorApiResult<DeviceProfileDraftResult>> {
  return callApi<DeviceProfileDraftResult>("generate_device_profile_draft", {
    sessionHandle,
    deviceHandle,
    profile,
  });
}

export async function checkDeviceProfileCollisions(
  sessionHandle: string,
  deviceHandle: string,
  rootHandle: string,
  profile: DeviceProfileV1Dto,
): Promise<EditorApiResult<DeviceProfileCollisionResult>> {
  return callApi<DeviceProfileCollisionResult>("check_device_profile_collisions", {
    sessionHandle,
    deviceHandle,
    rootHandle,
    profile,
  });
}

export async function saveGeneratedDeviceProfile(
  sessionHandle: string,
  deviceHandle: string,
  rootHandle: string,
  profile: DeviceProfileV1Dto,
): Promise<EditorApiResult<DeviceProfileSaveResult>> {
  return callApi<DeviceProfileSaveResult>("save_generated_device_profile", {
    sessionHandle,
    deviceHandle,
    rootHandle,
    profile,
  });
}

export async function cancelDeviceProfileGenerator(
  sessionHandle: string,
): Promise<EditorApiResult<Record<string, never>>> {
  return callApi<Record<string, never>>("cancel_device_profile_generator", { sessionHandle });
}

export async function beginAppGenerator(): Promise<EditorApiResult<AppGeneratorSessionResult>> {
  return callApi("begin_app_generator");
}

export async function chooseAppGeneratorApk(
  sessionHandle: string,
): Promise<EditorApiResult<AppGeneratorSelectionResult>> {
  return callApi("choose_app_generator_apk", { sessionHandle });
}

export async function chooseAppGeneratorAuthoredRoot(
  sessionHandle: string,
): Promise<EditorApiResult<AppGeneratorSelectionResult>> {
  return callApi("choose_app_generator_authored_root", { sessionHandle });
}

export async function setAppGeneratorAuthoredRoot(
  sessionHandle: string,
  authoredRoot: string,
): Promise<EditorApiResult<AppGeneratorSelectionResult>> {
  return callApi("set_app_generator_authored_root", { sessionHandle, authoredRoot });
}

export async function analyzeAppGeneratorSource(
  sessionHandle: string,
  mode: AppGeneratorSourceMode,
  sourceUrl: string,
  includePrereleases: boolean,
): Promise<EditorApiResult<RemoteSourceAnalysisResult>> {
  return callApi("analyze_app_generator_source", {
    sessionHandle,
    mode,
    sourceUrl,
    includePrereleases,
  });
}

export async function downloadAppGeneratorRemoteApk(
  sessionHandle: string,
  assetHandle: string,
): Promise<EditorApiResult<RemoteApkDownloadResult>> {
  return callApi("download_app_generator_remote_apk", { sessionHandle, assetHandle });
}

export async function inspectAppGeneratorApk(
  sessionHandle: string,
  apkHandle: string,
  connectedDeviceApi: number | null,
): Promise<EditorApiResult<ApkInspectionResult>> {
  return callApi("inspect_app_generator_apk", { sessionHandle, apkHandle, connectedDeviceApi });
}

export async function generateAppRecipeDraft(
  sessionHandle: string,
  apkHandle: string,
  app: AppDefinitionV1Dto | null,
  recipe: AppRecipeEditsDto | null,
  mappings: AppMappingEditsDto | null,
  permissionSelection: PermissionSelectionRequestDto | null,
  regenerateIdentifiers = false,
  rootHandle: string | null = null,
): Promise<EditorApiResult<AppRecipeDraftResult>> {
  return callApi("generate_app_recipe_draft", {
    sessionHandle,
    apkHandle,
    app,
    recipe,
    mappings,
    permissionSelection,
    regenerateIdentifiers,
    rootHandle,
  });
}

export async function generateRemoteAppRecipeDraft(
  sessionHandle: string,
  apkHandle: string,
  assetHandle: string,
  strategy: AppGeneratorInstallStrategy,
  assetPattern: string | null,
  includePrereleases: boolean,
  trustedSha256: string | null,
  app: AppDefinitionV1Dto | null,
  recipe: AppRecipeEditsDto | null,
  mappings: AppMappingEditsDto | null,
  permissionSelection: PermissionSelectionRequestDto | null,
  regenerateIdentifiers = false,
  rootHandle: string | null = null,
): Promise<EditorApiResult<AppRecipeDraftResult>> {
  return callApi("generate_remote_app_recipe_draft", {
    sessionHandle,
    apkHandle,
    assetHandle,
    strategy,
    assetPattern,
    includePrereleases,
    trustedSha256,
    app,
    recipe,
    mappings,
    permissionSelection,
    regenerateIdentifiers,
    rootHandle,
  });
}

export async function saveGeneratedAppRecipe(
  sessionHandle: string,
  apkHandle: string,
  rootHandle: string,
  app: AppDefinitionV1Dto,
  recipe: AppRecipeEditsDto,
  mappings: AppMappingEditsDto,
  permissionSelection: PermissionSelectionRequestDto | null,
): Promise<EditorApiResult<AppRecipeSaveResult>> {
  return callApi("save_generated_app_recipe", {
    sessionHandle,
    apkHandle,
    rootHandle,
    app,
    recipe,
    mappings,
    permissionSelection,
  });
}

export async function saveGeneratedRemoteAppRecipe(
  sessionHandle: string,
  apkHandle: string,
  assetHandle: string,
  strategy: AppGeneratorInstallStrategy,
  assetPattern: string | null,
  includePrereleases: boolean,
  trustedSha256: string | null,
  rootHandle: string,
  app: AppDefinitionV1Dto,
  recipe: AppRecipeEditsDto,
  mappings: AppMappingEditsDto,
  permissionSelection: PermissionSelectionRequestDto | null,
): Promise<EditorApiResult<AppRecipeSaveResult>> {
  return callApi("save_generated_remote_app_recipe", {
    sessionHandle,
    apkHandle,
    assetHandle,
    strategy,
    assetPattern,
    includePrereleases,
    trustedSha256,
    rootHandle,
    app,
    recipe,
    mappings,
    permissionSelection,
  });
}

export async function cancelAppGenerator(
  sessionHandle: string,
): Promise<EditorApiResult<Record<string, never>>> {
  return callApi("cancel_app_generator", { sessionHandle });
}

export interface MenuState {
  hasDocument: boolean;
  hasSelectedAuthoredRoot: boolean;
  hasDocumentAuthoredRoot: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  hasDocumentPath: boolean;
  commandInFlight: boolean;
  documentSessionValid: boolean;
  backendCompatible: boolean | null;
  sidecarRunning: boolean | null;
  generatorActive: boolean;
}

export async function updateMenuState(state: MenuState): Promise<void> {
  await invoke("update_menu_state", { state });
}

async function callApi<T>(command: string, args?: Record<string, unknown>): Promise<EditorApiResult<T>> {
  try {
    const envelope = await invoke<ApiEnvelope<T>>(command, args);
    if (!isApiEnvelope<T>(envelope)) {
      return {
        kind: "transport-error",
        message: `Command ${command} returned an invalid API envelope.`,
      };
    }
    if (envelope.ok) {
      return { kind: "success", result: envelope.result };
    }
    return { kind: "api-error", error: envelope.error, debug: envelope.debug };
  } catch (error) {
    return {
      kind: "transport-error",
      message: errorMessage(error),
    };
  }
}

function isApiEnvelope<T>(value: unknown): value is ApiEnvelope<T> {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const envelope = value as Partial<ApiEnvelope<T>>;
  if (typeof envelope.ok !== "boolean") {
    return false;
  }
  if (envelope.ok) {
    return "result" in envelope;
  }
  return "error" in envelope;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}

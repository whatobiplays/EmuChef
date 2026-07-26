import { invoke } from "@tauri-apps/api/core";

import type {
  AdbSetupStatus,
  PlatformToolsPickerResult,
  CatalogSummary,
  ConfigurationDescription,
  DeviceFacts,
  DeviceMatch,
  DeviceSummary,
  ExecutionCancellation,
  DeviceQualificationSnapshot,
  RootQualificationCheck,
  ExecutionCapabilities,
  ExecutionEventBatch,
  ExecutionSnapshot,
  RealExecutionConfirmation,
  RealExecutionSnapshot,
  ReportExportResult,
  ReviewSummary,
  RuntimeStatus,
  LaunchResult,
  RecentConfiguration,
  SavedConfigurationDialogResult,
  SavedConfigurationDocument,
  SavedConfigurationFileOperationResult,
  SavedConfigurationMutation,
  SavedConfigurationPreviewResult,
  CacheCleanupMode,
  CacheCleanupResult,
  CacheInventory,
  SupportDiagnosticsExportResult,
  SupportSnapshot,
  ResetLocalStateResult,
  AppSessionStart,
  RecoveryRestoreResult,
  RecoveryWriteAck,
  UpdateInteractionSession,
  UpdateStatus,
} from "./types";

export const api = {
  beginAppSession: () => invoke<AppSessionStart>("begin_app_session"),
  stageRecoveryDraft: (request: {
    sessionGeneration: number;
    requestGeneration: number;
    draftGeneration: number;
    displayName: string | null;
    sourceConfigurationHandle: string | null;
    dirty: boolean;
    devicePlan: string;
    selectedRecipes: string[];
    bindings: Record<string, unknown>;
  }) => invoke<RecoveryWriteAck>("stage_recovery_draft", { request }),
  deferRecoveryDraft: (sessionGeneration: number, recordGeneration: number) =>
    invoke<void>("defer_recovery_draft", {
      request: { sessionGeneration, recordGeneration },
    }),
  restoreRecoveryDraft: (
    sessionGeneration: number,
    recordGeneration: number,
    requestGeneration: number,
  ) => invoke<RecoveryRestoreResult>("restore_recovery_draft", {
    request: { sessionGeneration, recordGeneration, requestGeneration },
  }),
  discardRecoveryDraft: (sessionGeneration: number, recordGeneration: number) =>
    invoke<void>("discard_recovery_draft", {
      request: { sessionGeneration, recordGeneration },
    }),
  runtimeStatus: () => invoke<RuntimeStatus>("get_runtime_status"),
  restartRuntime: (expectedGeneration?: number) =>
    invoke<RuntimeStatus>("restart_runtime", { expectedGeneration }),
  catalog: () => invoke<CatalogSummary>("get_catalog"),
  adbStatus: () => invoke<AdbSetupStatus>("get_adb_setup_status"),
  openPlatformToolsPage: () => invoke<void>("open_platform_tools_download_page"),
  pickPlatformToolsZip: () =>
    invoke<PlatformToolsPickerResult>("pick_platform_tools_zip"),
  installPlatformToolsSelection: (selectionHandle: string, expectedRevision?: number) =>
    invoke<AdbSetupStatus>("install_platform_tools_selection", { selectionHandle, expectedRevision }),
  removePlatformTools: (expectedRevision?: number) =>
    invoke<AdbSetupStatus>("remove_platform_tools", { expectedRevision }),
  pollDevices: (expectedGeneration?: number) =>
    invoke<DeviceSummary[]>("poll_devices", { expectedGeneration }),
  probeDevice: (deviceHandle: string) =>
    invoke<DeviceFacts>("probe_device", { deviceHandle }),
  matchDevice: (deviceHandle: string) =>
    invoke<DeviceMatch>("match_device", { deviceHandle }),
  describeConfiguration: (request: {
    deviceHandle: string;
    devicePlan: string;
    selectedRecipes: string[] | null;
    bindings: Record<string, unknown>;
    requestGeneration: number;
  }) => invoke<ConfigurationDescription>("describe_configuration", request),
  createReview: (request: {
    deviceHandle: string;
    devicePlan: string;
    selectedRecipes: string[] | null;
    bindings: Record<string, unknown>;
    requestGeneration: number;
  }) => invoke<ReviewSummary>("create_review", request),
  discardReview: (reviewHandle: string) => invoke<void>("discard_review", { reviewHandle }),
  startSimulatedExecution: (reviewHandle: string) =>
    invoke<ExecutionSnapshot>("start_simulated_execution", { reviewHandle }),
  getSimulatedExecution: (executionHandle: string) =>
    invoke<ExecutionSnapshot>("get_simulated_execution", { executionHandle }),
  getSimulatedExecutionEvents: (executionHandle: string, afterSequence: number) =>
    invoke<ExecutionEventBatch>("get_simulated_execution_events", {
      executionHandle,
      afterSequence,
    }),
  cancelSimulatedExecution: (executionHandle: string) =>
    invoke<ExecutionCancellation>("cancel_simulated_execution", { executionHandle }),
  executionCapabilities: () =>
    invoke<ExecutionCapabilities>("get_execution_capabilities"),
  deviceQualification: (deviceHandle: string | null) =>
    invoke<DeviceQualificationSnapshot>("get_device_qualification", { deviceHandle }),
  checkDeviceRoot: (deviceHandle: string) =>
    invoke<RootQualificationCheck>("check_device_root", { deviceHandle }),
  startRealExecution: (reviewHandle: string, confirmation: RealExecutionConfirmation) =>
    invoke<RealExecutionSnapshot>("start_real_execution", {
      request: { reviewHandle, confirmation },
    }),
  getRealExecution: (executionHandle: string) =>
    invoke<RealExecutionSnapshot>("get_real_execution", { executionHandle }),
  getRealExecutionEvents: (executionHandle: string, afterSequence: number) =>
    invoke<ExecutionEventBatch>("get_real_execution_events", {
      executionHandle,
      afterSequence,
    }),
  cancelRealExecution: (executionHandle: string) =>
    invoke<ExecutionCancellation>("cancel_real_execution", { executionHandle }),
  exportExecutionReport: (executionHandle: string) =>
    invoke<ReportExportResult>("export_execution_report", { executionHandle }),
  launchConfiguredApp: (launchActionHandle: string) =>
    invoke<LaunchResult>("launch_configured_app", { launchActionHandle }),
  pickInputPath: (request: {
    inputKey: string;
    requestGeneration: number;
    mode: "replace_all" | "append" | "replace_entry";
    currentValue: unknown;
    entryIndex: number | null;
  }) => invoke<unknown | null>("pick_input_path", request),
  listRecentConfigurations: () =>
    invoke<RecentConfiguration[]>("list_recent_configurations"),
  createSavedConfiguration: (request: {
    name: string;
    devicePlan: string;
    selectedRecipes: string[];
    bindings: Record<string, unknown>;
  }) => invoke<SavedConfigurationDialogResult>("create_saved_configuration", { request }),
  openSavedConfiguration: () =>
    invoke<SavedConfigurationDialogResult>("open_saved_configuration"),
  previewSavedConfiguration: () =>
    invoke<SavedConfigurationPreviewResult>("preview_saved_configuration"),
  previewRecentConfiguration: (recentHandle: string) =>
    invoke<SavedConfigurationPreviewResult>("preview_recent_configuration", {
      request: { recentHandle },
    }),
  confirmSavedConfigurationPreview: (previewHandle: string) =>
    invoke<SavedConfigurationDocument>("confirm_saved_configuration_preview", {
      request: { previewHandle },
    }),
  cancelSavedConfigurationPreview: (previewHandle: string) =>
    invoke<void>("cancel_saved_configuration_preview", { request: { previewHandle } }),
  compareSavedConfigurationPreview: (request: {
    previewHandle: string;
    devicePlan: string | null;
    selectedRecipes: string[];
    bindings: Record<string, unknown>;
  }) => invoke<{ state: "matches" | "differs" | "requires_repair" | "no_current_intent"; message: string }>(
    "compare_saved_configuration_preview",
    { request },
  ),
  applySavedConfigurationPreviewRepair: (previewHandle: string, repairHandle: string) =>
    invoke<SavedConfigurationPreviewResult>("apply_saved_configuration_preview_repair", {
      request: { previewHandle, repairHandle },
    }),
  openRecentConfiguration: (recentHandle: string) =>
    invoke<SavedConfigurationDocument>("open_recent_configuration", {
      request: { recentHandle },
    }),
  relinkRecentConfiguration: (recentHandle: string) =>
    invoke<RecentConfiguration>("relink_recent_configuration", {
      request: { recentHandle },
    }),
  removeRecentConfiguration: (recentHandle: string) =>
    invoke<void>("remove_recent_configuration", { request: { recentHandle } }),
  updateSavedConfiguration: (
    configurationHandle: string,
    expectedRevision: number,
    mutation: SavedConfigurationMutation,
  ) => invoke<SavedConfigurationDocument>("update_saved_configuration", {
    request: { configurationHandle, expectedRevision, mutation },
  }),
  saveSavedConfiguration: (configurationHandle: string) =>
    invoke<SavedConfigurationDocument>("save_saved_configuration", {
      request: { configurationHandle },
    }),
  saveSavedConfigurationAs: (configurationHandle: string, name: string) =>
    invoke<SavedConfigurationDialogResult>("save_saved_configuration_as", {
      request: { configurationHandle, name },
    }),
  renameSavedConfiguration: (configurationHandle: string, name: string) =>
    invoke<SavedConfigurationDocument>("rename_saved_configuration", {
      request: { configurationHandle, name },
    }),
  duplicateSavedConfiguration: (configurationHandle: string, name: string) =>
    invoke<SavedConfigurationFileOperationResult>("duplicate_saved_configuration", {
      request: { configurationHandle, name },
    }),
  importSavedConfiguration: (previewHandle: string, name: string) =>
    invoke<SavedConfigurationDialogResult>("import_saved_configuration", {
      request: { previewHandle, name },
    }),
  exportSavedConfiguration: (configurationHandle: string, name: string) =>
    invoke<SavedConfigurationFileOperationResult>("export_saved_configuration", {
      request: { configurationHandle, name },
    }),
  updateSavedConfigurationMenu: (state: {
    runtimeReady: boolean;
    commandBlocked: boolean;
    hasDocument: boolean;
    dirty: boolean;
    hasPortableIntent: boolean;
  }) => invoke<void>("update_saved_configuration_menu", { state }),
  closeSavedConfiguration: (configurationHandle: string) =>
    invoke<void>("close_saved_configuration", { request: { configurationHandle } }),
  cacheInventory: () => invoke<CacheInventory>("get_cache_inventory"),
  supportSnapshot: () => invoke<SupportSnapshot>("get_support_snapshot"),
  cleanupCache: (request: {
    mode: CacheCleanupMode;
    inventoryGeneration: string;
    entryHandles: string[];
    confirmation: { confirmed: true; entryCount: number; totalSizeBytes: number };
  }) => invoke<CacheCleanupResult>("cleanup_cache", { request }),
  exportSupportDiagnostics: () =>
    invoke<SupportDiagnosticsExportResult>("export_support_diagnostics"),
  resetLocalAppState: (resetHandle: string) =>
    invoke<ResetLocalStateResult>("reset_local_app_state", {
      request: { resetHandle, confirmed: true },
    }),
  getUpdateStatus: () => invoke<UpdateStatus>("get_update_status"),
  checkForUpdates: () => invoke<UpdateStatus>("check_for_updates"),
  beginUpdateInteractionSession: () =>
    invoke<UpdateInteractionSession>("begin_update_interaction_session"),
  setUpdateInteractionState: (request: {
    sessionId: string;
    generation: number;
    blocked: boolean;
  }) => invoke<void>("set_update_interaction_state", { request }),
  endUpdateInteractionSession: (sessionId: string) =>
    invoke<void>("end_update_interaction_session", { sessionId }),
  openUpdateDownload: () => invoke<void>("open_update_download"),
};

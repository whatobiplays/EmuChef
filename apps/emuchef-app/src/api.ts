import { invoke } from "@tauri-apps/api/core";

import type {
  AdbSetupStatus,
  CatalogSummary,
  ConfigurationDescription,
  DeviceFacts,
  DeviceMatch,
  DeviceSummary,
  ExecutionCancellation,
  ExecutionEventBatch,
  ExecutionSnapshot,
  RealExecutionAvailability,
  RealExecutionConfirmation,
  RealExecutionSnapshot,
  ReportExportResult,
  ReviewSummary,
  RuntimeStatus,
  LaunchResult,
  RecentConfiguration,
  SavedConfigurationDialogResult,
  SavedConfigurationDocument,
  SavedConfigurationMutation,
  CacheCleanupMode,
  CacheCleanupResult,
  CacheInventory,
  SupportDiagnosticsExportResult,
  AppSessionStart,
  RecoveryRestoreResult,
  RecoveryWriteAck,
} from "./types";

export const api = {
  beginAppSession: () => invoke<AppSessionStart>("begin_app_session"),
  stageRecoveryDraft: (request: {
    sessionGeneration: number;
    requestGeneration: number;
    draftGeneration: number;
    displayName: string | null;
    sourceConfigurationHandle: string | null;
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
  finishAppSession: (sessionGeneration: number, currentSessionDirty: boolean) =>
    invoke<void>("finish_app_session", {
      request: { sessionGeneration, currentSessionDirty },
    }),
  runtimeStatus: () => invoke<RuntimeStatus>("get_runtime_status"),
  restartRuntime: () => invoke<RuntimeStatus>("restart_runtime"),
  catalog: () => invoke<CatalogSummary>("get_catalog"),
  adbStatus: () => invoke<AdbSetupStatus>("get_adb_setup_status"),
  openPlatformToolsPage: () => invoke<void>("open_platform_tools_download_page"),
  importPlatformTools: () => invoke<AdbSetupStatus>("import_platform_tools_zip"),
  removePlatformTools: () => invoke<AdbSetupStatus>("remove_platform_tools"),
  pollDevices: () => invoke<DeviceSummary[]>("poll_devices"),
  probeDevice: (deviceHandle: string) =>
    invoke<DeviceFacts>("probe_device", { deviceHandle }),
  matchDevice: (deviceHandle: string) =>
    invoke<DeviceMatch>("match_device", { deviceHandle }),
  describeConfiguration: (request: {
    deviceHandle: string;
    devicePlan: string;
    selectedRecipes: string[] | null;
    bindings: Record<string, unknown>;
  }) => invoke<ConfigurationDescription>("describe_configuration", request),
  createReview: (request: {
    deviceHandle: string;
    devicePlan: string;
    selectedRecipes: string[] | null;
    bindings: Record<string, unknown>;
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
  realExecutionAvailability: () =>
    invoke<RealExecutionAvailability>("get_real_execution_availability"),
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
  pickInputPath: (pathKind: "file" | "directory", multiple: boolean) =>
    invoke<string[] | null>("pick_input_path", { pathKind, multiple }),
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
  openRecentConfiguration: (recentHandle: string) =>
    invoke<SavedConfigurationDocument>("open_recent_configuration", {
      request: { recentHandle },
    }),
  relinkRecentConfiguration: (recentHandle: string) =>
    invoke<SavedConfigurationDialogResult>("relink_recent_configuration", {
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
  closeSavedConfiguration: (configurationHandle: string) =>
    invoke<void>("close_saved_configuration", { request: { configurationHandle } }),
  cacheInventory: () => invoke<CacheInventory>("get_cache_inventory"),
  cleanupCache: (request: {
    mode: CacheCleanupMode;
    inventoryGeneration: string;
    entryHandles: string[];
    confirmation: { confirmed: true; entryCount: number; totalSizeBytes: number };
  }) => invoke<CacheCleanupResult>("cleanup_cache", { request }),
  exportSupportDiagnostics: () =>
    invoke<SupportDiagnosticsExportResult>("export_support_diagnostics"),
};

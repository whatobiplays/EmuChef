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
  ReviewSummary,
  RuntimeStatus,
} from "./types";

export const api = {
  runtimeStatus: () => invoke<RuntimeStatus>("get_runtime_status"),
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
  pickInputPath: (pathKind: "file" | "directory", multiple: boolean) =>
    invoke<string[] | null>("pick_input_path", { pathKind, multiple }),
};

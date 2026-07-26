import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";
import type {
  ConfigurationDescription,
  ExecutionCapabilities,
  SupportSnapshot,
} from "../src/types";

const mockApi = vi.hoisted(() => ({
  adbStatus: vi.fn(),
  beginAppSession: vi.fn(),
  beginUpdateInteractionSession: vi.fn(),
  catalog: vi.fn(),
  describeConfiguration: vi.fn(),
  endUpdateInteractionSession: vi.fn(),
  executionCapabilities: vi.fn(),
  deviceQualification: vi.fn(),
  checkDeviceRoot: vi.fn(),
  installPlatformToolsSelection: vi.fn(),
  listRecentConfigurations: vi.fn(),
  matchDevice: vi.fn(),
  createReview: vi.fn(),
  discardReview: vi.fn(),
  pickInputPath: vi.fn(),
  pickPlatformToolsZip: vi.fn(),
  pollDevices: vi.fn(),
  probeDevice: vi.fn(),
  removePlatformTools: vi.fn(),
  restartRuntime: vi.fn(),
  supportSnapshot: vi.fn(),
  resetLocalAppState: vi.fn(),
  cleanupCache: vi.fn(),
  exportSupportDiagnostics: vi.fn(),
  getUpdateStatus: vi.fn(),
  runtimeStatus: vi.fn(),
  setUpdateInteractionState: vi.fn(),
  startRealExecution: vi.fn(),
  stageRecoveryDraft: vi.fn(),
  restoreRecoveryDraft: vi.fn(),
  updateSavedConfigurationMenu: vi.fn(),
  previewSavedConfiguration: vi.fn(),
  previewRecentConfiguration: vi.fn(),
  confirmSavedConfigurationPreview: vi.fn(),
  cancelSavedConfigurationPreview: vi.fn(),
  compareSavedConfigurationPreview: vi.fn(),
  applySavedConfigurationPreviewRepair: vi.fn(),
  createSavedConfiguration: vi.fn(),
  saveSavedConfiguration: vi.fn(),
  saveSavedConfigurationAs: vi.fn(),
  closeSavedConfiguration: vi.fn(),
  relinkRecentConfiguration: vi.fn(),
  removeRecentConfiguration: vi.fn(),
  renameSavedConfiguration: vi.fn(),
  duplicateSavedConfiguration: vi.fn(),
  importSavedConfiguration: vi.fn(),
  exportSavedConfiguration: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    close: vi.fn(),
    onCloseRequested: vi.fn(async () => () => undefined),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

import { App } from "../src/App";

const readyAdb = {
  status: "ready",
  version: "35.0.2",
  warning: null,
  error: null,
  canImport: false,
  canReplace: true,
  canRemove: true,
};

const missingAdb = {
  status: "missing",
  version: null,
  warning: null,
  error: null,
  canImport: true,
  canReplace: false,
  canRemove: false,
};

const availableDevice = {
  deviceHandle: "device-opaque",
  state: "available",
  displayName: "Supported Handheld",
  maskedSerial: "••••1234",
};

const facts = {
  deviceHandle: availableDevice.deviceHandle,
  manufacturer: "Example",
  brand: "Example",
  model: "Handheld",
  androidVersion: 14,
  androidApiLevel: 34,
};

const safePlan = {
  planId: "generic.safe",
  name: "Backend Safe Generic",
  description: "A conservative generic setup.",
  profileId: "profile.generic",
  profileName: "Generic",
  reasons: ["Backend approved"],
};

const supportedMatch = {
  confidence: "exact",
  recommendedPlanId: "plan.supported",
  requiresExplicitChoice: false,
  candidates: [{ ...safePlan, planId: "plan.supported", name: "Supported setup" }],
  safeGenericPlans: [],
  blocked: false,
  blockReason: null,
};

function supportSnapshot(options: {
  serviceFailure?: boolean;
  platformActions?: boolean;
} = {}): SupportSnapshot {
  return {
    presentationRevision: 1,
    overallSeverity: options.serviceFailure ? "warning" : "healthy",
    overallSummary: options.serviceFailure
      ? "One or more systems need attention."
      : "EmuChef is ready. No troubleshooting issues were found.",
    subsystems: [
      {
        id: "service",
        label: "App service",
        severity: options.serviceFailure ? "failure" : "healthy",
        summary: options.serviceFailure ? "The local app service could not start." : "The local app service is ready.",
        consequence: options.serviceFailure ? "Planning is unavailable." : "Planning is available.",
        supportCode: options.serviceFailure ? "EMUCHEF-SERVICE-START-FAILED" : null,
        actions: options.serviceFailure ? [{
          label: "Restart app service",
          consequence: "Portable setup intent is preserved; transient authority is refreshed.",
          available: true,
          unavailableReason: null,
          destructive: false,
          action: { kind: "restart_service", serviceGeneration: 3 },
        }] : [],
      },
      {
        id: "platform_tools",
        label: "Android Platform-Tools",
        severity: "healthy",
        summary: "Platform-Tools are ready.",
        consequence: "Device detection is available.",
        supportCode: null,
        actions: options.platformActions ? [
          {
            label: "Replace managed Platform-Tools",
            consequence: "Device authority will be refreshed.",
            available: true,
            unavailableReason: null,
            destructive: false,
            action: { kind: "replace_managed_platform_tools", platformToolsRevision: 7 },
          },
          {
            label: "Remove managed Platform-Tools",
            consequence: "Device detection will stop.",
            available: true,
            unavailableReason: null,
            destructive: true,
            action: { kind: "remove_managed_platform_tools", platformToolsRevision: 7 },
          },
        ] : [],
      },
    ],
    cacheInventory: {
      generation: "1",
      entries: [],
      summary: {
        entryCount: 0,
        totalSizeBytes: 0,
        inUseCount: 0,
        removableCount: 0,
        removableSizeBytes: 0,
        unusedRemovableCount: 0,
        unusedRemovableSizeBytes: 0,
        unmanagedCount: 0,
        unmanagedSizeBytes: 0,
      },
      categories: [],
    },
    diagnosticsDisclosure: {
      includedCategories: ["Aggregate app status"],
      excludedCategories: ["Paths and serials"],
      localUntilShared: true,
      uploadsAutomatically: false,
      maximumSizeBytes: 2 * 1024 * 1024,
    },
    resetCategories: [],
  };
}

function descriptionWithTextInput(input: {
  key: string;
  inputId: string;
  label: string;
  required: boolean;
  sensitive: boolean;
}): ConfigurationDescription {
  return {
    devicePlan: "plan.supported",
    selectedRecipes: ["recipe.one"],
    expandedRecipes: ["recipe.one"],
    recipeOptions: [{
      id: "recipe.one",
      name: "Recipe One",
      description: null,
      selected: true,
      recommended: true,
      dependencyRequired: false,
      available: true,
      unavailableCapabilities: [],
    }],
    inputs: [{
      ...input,
      recipeId: "recipe.one",
      type: "string",
      description: null,
      presentationCategory: "Other",
      presentationKind: "Text",
      value: null,
      valueSource: null,
      diagnostics: [],
    }],
    diagnostics: [],
  };
}

function resetApi(): void {
  vi.clearAllMocks();
  mockApi.beginUpdateInteractionSession.mockResolvedValue({ sessionId: "update-session", generation: 0 });
  mockApi.setUpdateInteractionState.mockResolvedValue(undefined);
  mockApi.endUpdateInteractionSession.mockResolvedValue(undefined);
  mockApi.beginAppSession.mockResolvedValue({
    sessionGeneration: 1,
    interruptedSession: false,
    recovery: { state: "none" },
  });
  mockApi.runtimeStatus.mockResolvedValue({ status: "ready", protocolVersion: 1, catalogVersion: "test" });
  mockApi.adbStatus.mockResolvedValue(readyAdb);
  mockApi.executionCapabilities.mockResolvedValue({
    realExecutionCompiled: false,
    platformToolsStatus: "notApplicable",
    executorReadiness: "notCompiled",
  } satisfies ExecutionCapabilities);
  mockApi.deviceQualification.mockResolvedValue({
    state: "notApplicable",
    summary: "Real-device qualification is not compiled in this build.",
    limitations: ["Simulation remains available."],
    androidMajor: null,
    androidApiLevel: null,
    abiClass: null,
    root: null,
    runtimeGeneration: 0,
    qualificationRevision: 0,
    deviceIdentity: null,
  });
  mockApi.checkDeviceRoot.mockResolvedValue({
    qualification: { status: "granted" },
    runtimeGeneration: 0,
    qualificationRevision: 0,
    deviceIdentity: "device-opaque",
  });
  mockApi.catalog.mockResolvedValue({
    catalog: {
      sourceKind: "bundled",
      sourceId: "test",
      version: "test",
      contentDigest: { algorithm: "sha256", value: "digest" },
    },
    recipes: [],
  });
  mockApi.listRecentConfigurations.mockResolvedValue([]);
  mockApi.pollDevices.mockResolvedValue([]);
  mockApi.probeDevice.mockResolvedValue(facts);
  mockApi.matchDevice.mockResolvedValue(supportedMatch);
  mockApi.pickInputPath.mockResolvedValue(null);
  mockApi.describeConfiguration.mockResolvedValue({
    devicePlan: "plan.supported",
    selectedRecipes: [],
    expandedRecipes: [],
    recipeOptions: [],
    inputs: [],
    diagnostics: [],
  });
  mockApi.pickPlatformToolsZip.mockResolvedValue({ outcome: "selected", selectionHandle: "selection-opaque" });
  mockApi.installPlatformToolsSelection.mockResolvedValue(readyAdb);
  mockApi.removePlatformTools.mockResolvedValue(missingAdb);
  mockApi.restartRuntime.mockResolvedValue({ status: "ready", protocolVersion: 1, catalogVersion: "test" });
  mockApi.supportSnapshot.mockResolvedValue(supportSnapshot());
  mockApi.exportSupportDiagnostics.mockResolvedValue({ outcome: "saved" });
  mockApi.getUpdateStatus.mockResolvedValue({ state: "unconfigured" });
  mockApi.updateSavedConfigurationMenu.mockResolvedValue(undefined);
  mockApi.cancelSavedConfigurationPreview.mockResolvedValue(undefined);
  mockApi.compareSavedConfigurationPreview.mockResolvedValue({
    state: "no_current_intent",
    message: "There are no current setup choices to compare.",
  });
  mockApi.removeRecentConfiguration.mockResolvedValue(undefined);
  mockApi.stageRecoveryDraft.mockImplementation(async (request: { requestGeneration: number; draftGeneration: number }) => ({
    requestGeneration: request.requestGeneration,
    draftGeneration: request.draftGeneration,
    recordGeneration: request.draftGeneration,
    omittedBindings: [],
  }));
}

async function renderReadyApp(): Promise<void> {
  render(<App />);
  await screen.findByRole("heading", { name: "Choose an Android device" });
}

async function advanceToInputs(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  mockApi.pollDevices.mockResolvedValue([availableDevice]);
  await renderReadyApp();
  await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
  await screen.findByRole("heading", { name: "Example Handheld" });
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await screen.findByRole("heading", { name: "Choose what to install" });
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await screen.findByRole("heading", { name: "Provide required files and options" });
}

async function openSupportAction(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
  snapshot: SupportSnapshot,
): Promise<HTMLButtonElement> {
  mockApi.supportSnapshot.mockResolvedValue(snapshot);
  await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
  const dialog = await screen.findByRole("dialog", { name: "Troubleshooting and app storage" });
  const action = within(dialog).queryByRole("button", { name });
  if (action) return action as HTMLButtonElement;
  await user.click(within(dialog).getByText("View all troubleshooting status and maintenance actions"));
  return within(dialog).getByRole("button", { name }) as HTMLButtonElement;
}

function deferred<T>(): {
  promise: Promise<T>;
  reject: (reason?: unknown) => void;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolver, rejecter) => {
    resolve = resolver;
    reject = rejecter;
  });
  return { promise, reject, resolve };
}

beforeEach(() => resetApi());

describe("Phase 6A execution capability reporting", () => {
  test("shows the backend-provided feature-disabled capability", async () => {
    await renderReadyApp();

    const status = document.querySelector(".status-panel");
    expect(status?.textContent).toContain("Real-device executionNot compiled");
    expect(status?.textContent).toContain("Platform ToolsNot applicable");
    expect(status?.textContent).toContain("Executor readinessNot compiled");
    expect(status?.textContent).not.toContain("Mode");
  });

  test("shows a concise backend-authored device qualification state in the sidebar", async () => {
    await renderReadyApp();

    const status = document.querySelector(".status-panel");
    expect(status?.textContent).toContain("Device qualificationNot applicable");
    expect(status?.textContent).not.toContain("Real-device qualification is not compiled in this build.");
    expect(mockApi.deviceQualification).toHaveBeenCalledTimes(2);
  });

  test("renders backend-authored qualification facts and limitations in the main device surface", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    mockApi.deviceQualification.mockResolvedValue({
      state: "supported",
      summary: "This device meets the current qualification requirements.",
      limitations: ["Root access was not checked."],
      androidMajor: 14,
      androidApiLevel: 34,
      abiClass: "arm64",
      root: null,
      runtimeGeneration: 0,
      qualificationRevision: 2,
      deviceIdentity: "device-opaque",
    });

    await renderReadyApp();
    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));

    const qualification = await screen.findByRole("heading", { name: "Device qualification" });
    const section = qualification.closest("section");
    expect(section).not.toBeNull();
    expect(section?.textContent).toContain("Supported");
    expect(section?.textContent).toContain("Android version14");
    expect(section?.textContent).toContain("API level34");
    expect(section?.textContent).toContain("Processor architecture64-bit ARM");
    expect(section?.textContent).toContain("Root accessNot checked");
    expect(section?.textContent).toContain("Root access was not checked.");
    expect(section?.textContent).not.toContain("device-identity-opaque");
    await user.click(screen.getByRole("button", { name: "Check root access" }));
    await waitFor(() => expect(mockApi.checkDeviceRoot).toHaveBeenCalledWith("device-opaque"));
    expect(section?.textContent).toContain("Root accessGranted");
  });

  test("blocks every device and explains eligibility when multiple devices are connected", async () => {
    mockApi.pollDevices.mockResolvedValue([
      availableDevice,
      { ...availableDevice, deviceHandle: "device-opaque-two", displayName: "Second Handheld", maskedSerial: "••••5678" },
    ]);
    mockApi.deviceQualification.mockResolvedValue({
      state: "insufficientlyQualified",
      summary: "More than one Android device is connected.",
      limitations: ["Disconnect additional devices before qualification."],
      androidMajor: null,
      androidApiLevel: null,
      abiClass: null,
      root: null,
      runtimeGeneration: 0,
      qualificationRevision: 3,
      deviceIdentity: null,
    });

    await renderReadyApp();

    expect(screen.getByText("No device can be selected while multiple Android devices are connected.")).toBeTruthy();
    expect(screen.getByText("Disconnect all but one device, then refresh discovery.")).toBeTruthy();
    const deviceButtons = [
      screen.getByRole("button", { name: /Supported Handheld.*Connected/ }),
      screen.getByRole("button", { name: /Second Handheld.*Connected/ }),
    ];
    expect(deviceButtons).toHaveLength(2);
    expect(deviceButtons.every((button) => button.hasAttribute("disabled"))).toBe(true);
    expect(document.querySelector(".status-panel")?.textContent).toContain("Device qualificationQualification incomplete");
  });

  test("shows the backend-provided feature-enabled capability without starting execution", async () => {
    mockApi.executionCapabilities.mockResolvedValue({
      realExecutionCompiled: true,
      platformToolsStatus: "ready",
      executorReadiness: "ready",
    } satisfies ExecutionCapabilities);

    await renderReadyApp();

    const status = document.querySelector(".status-panel");
    expect(status?.textContent).toContain("Real-device executionCompiled in");
    expect(status?.textContent).toContain("Platform ToolsReady");
    expect(status?.textContent).toContain("Executor readinessReady");
    expect(mockApi.startRealExecution).not.toHaveBeenCalled();
  });

  test.each([
    ["notApplicable", "Not applicable"],
    ["ready", "Ready"],
    ["notFound", "Not found"],
    ["invalid", "Invalid"],
    ["checkFailed", "Check failed"],
  ] as const)("renders the Rust-authored Platform Tools state %s", async (platformToolsStatus, label) => {
    mockApi.executionCapabilities.mockResolvedValue({
      realExecutionCompiled: true,
      platformToolsStatus,
      executorReadiness: "ready",
    } satisfies ExecutionCapabilities);

    await renderReadyApp();

    expect(document.querySelector(".status-panel")?.textContent).toContain(`Platform Tools${label}`);
  });

  test.each([
    ["notCompiled", "Not compiled"],
    ["ready", "Ready"],
    ["blocked", "Blocked"],
    ["unknown", "Unknown"],
  ] as const)("renders the Rust-authored executor-readiness state %s", async (executorReadiness, label) => {
    mockApi.executionCapabilities.mockResolvedValue({
      realExecutionCompiled: true,
      platformToolsStatus: "ready",
      executorReadiness,
    } satisfies ExecutionCapabilities);

    await renderReadyApp();

    expect(document.querySelector(".status-panel")?.textContent).toContain(`Executor readiness${label}`);
  });

  test("retains the previous valid result while refresh is pending and marks refresh failure separately", async () => {
    const user = userEvent.setup();
    const refresh = deferred<ExecutionCapabilities>();
    mockApi.adbStatus.mockResolvedValue(missingAdb);
    mockApi.executionCapabilities
      .mockResolvedValueOnce({
        realExecutionCompiled: false,
        platformToolsStatus: "notApplicable",
        executorReadiness: "notCompiled",
      } satisfies ExecutionCapabilities)
      .mockReturnValueOnce(refresh.promise);

    render(<App />);
    await screen.findByRole("heading", { name: "Set up Android Platform-Tools" });
    await user.click(screen.getByRole("button", { name: "Select Platform-Tools ZIP…" }));
    const status = await waitFor(() => {
      const panel = document.querySelector(".status-panel");
      expect(panel?.textContent).toContain("Platform ToolsRefreshing…");
      expect(panel?.textContent).toContain("Real-device executionNot compiled");
      return panel as HTMLElement;
    });
    expect(mockApi.startRealExecution).not.toHaveBeenCalled();

    await act(async () => refresh.reject(new Error("capability IPC unavailable")));

    await waitFor(() => expect(status.textContent).toContain("Platform ToolsStatus unavailable"));
    expect(status.textContent).toContain("Executor readinessStatus unavailable");
    expect(status.textContent).toContain("Real-device executionNot compiled");
    expect(mockApi.startRealExecution).not.toHaveBeenCalled();
  });

  test("does not report not compiled when capability IPC fails", async () => {
    mockApi.executionCapabilities.mockRejectedValue(new Error("capability IPC unavailable"));

    render(<App />);
    await waitFor(() => expect(mockApi.executionCapabilities).toHaveBeenCalledTimes(1));

    expect(document.body.textContent).not.toContain("Not compiled");
    expect(document.querySelector(".status-panel")).toBeNull();
  });
});

describe("Phase 5H product polish", () => {
  test("shows readiness without exposing catalog identity or implementation terminology", async () => {
    mockApi.runtimeStatus.mockResolvedValue({
      status: "ready",
      protocolVersion: 99,
      catalogVersion: "phase-internal-catalog",
    });
    mockApi.catalog.mockResolvedValue({
      catalog: {
        sourceKind: "bundled",
        sourceId: "internal-source-id",
        version: "phase-internal-catalog",
        contentDigest: { algorithm: "sha256", value: "internal-digest" },
      },
      recipes: [],
    });

    await renderReadyApp();

    expect(document.querySelector(".runtime-chip")?.textContent).toBe("Ready");
    const status = document.querySelector(".status-panel");
    expect(status?.textContent).toContain("App serviceReady");
    expect(status?.textContent).toContain("Platform ToolsNot applicable");
    expect(status?.textContent).toContain("Setup catalogReady");
    expect(document.querySelector(".configuration-bar")?.textContent).toContain("Unsaved setup");
    expect(document.body.textContent).not.toMatch(/phase-internal-catalog|internal-source-id|internal-digest|Rust runtime|protocol/i);
  });

  test("explains Platform-Tools setup without developer-oriented implementation details", async () => {
    mockApi.adbStatus.mockResolvedValue(missingAdb);
    render(<App />);

    expect(await screen.findByRole("heading", { name: "Set up Android Platform-Tools" })).toBeTruthy();
    expect(screen.getByText(/lets EmuChef find and communicate with your Android device/)).toBeTruthy();
    expect(screen.getByText(/Setup is normally needed only once/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open Google download page" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Select Platform-Tools ZIP…" })).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/\bADB\b|local validation|app bundle|repository|checksum|architecture/i);
  });

  test("keeps unsaved setup actions in an explicit safe-to-destructive order", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/option",
      inputId: "option",
      label: "Optional setting",
      required: false,
      sensitive: false,
    }));
    await advanceToInputs(user);
    await user.type(screen.getByLabelText("Optional setting"), "chosen");
    await user.click(screen.getByRole("button", { name: "Manage saved setups…" }));
    await user.click(screen.getByRole("button", { name: "New" }));

    const dialog = await screen.findByRole("alertdialog", { name: "Save edits before continuing?" });
    const actions = within(dialog).getAllByRole("button");
    expect(actions.map((button) => button.textContent)).toEqual(["Cancel", "Save", "Discard edits"]);
    expect(actions[0].parentElement?.classList.contains("dialog-actions")).toBe(true);
  });
});

describe("Phase 5B workflow surfaces", () => {
  test("startup does not force focus into the workflow", async () => {
    await renderReadyApp();
    expect(document.activeElement).toBe(document.body);
  });

  test("shows backend-approved applicable alternatives alongside an exact match", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    mockApi.matchDevice.mockResolvedValue({
      ...supportedMatch,
      candidates: [
        { ...safePlan, planId: "plan.supported", name: "Supported setup", confidence: "exact" },
      ],
      safeGenericPlans: [
        { ...safePlan, planId: "generic.safe", name: "Conservative generic setup" },
      ],
      blankSetupPlans: [{
        ...safePlan,
        planId: "plan.supported",
        name: "Start from scratch",
        description: "Choose recipes manually.",
        selectionMode: "blank",
      }],
    });
    await renderReadyApp();

    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    expect(await screen.findByRole("radio", { name: /Supported setup/ })).toBeTruthy();
    expect(screen.getByRole("radio", { name: /Conservative generic setup/ })).toBeTruthy();
    const blank = screen.getByRole("radio", { name: /Start from scratch/ });
    await user.click(blank);
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(mockApi.describeConfiguration).toHaveBeenCalledWith(expect.objectContaining({
      devicePlan: "plan.supported",
      selectedRecipes: [],
    }));
  });

  test("searches and filters backend-authored recipes while summarizing selection", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    mockApi.describeConfiguration.mockResolvedValue({
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.retroarch"],
      expandedRecipes: ["recipe.retroarch"],
      recipeOptions: [
        {
          id: "recipe.retroarch",
          name: "RetroArch",
          description: "Install and configure the frontend.",
          selected: true,
          recommended: true,
          dependencyRequired: false,
          available: true,
          recipeDependencies: ["recipe.copy-bios"],
          contentRequirements: ["network_download"],
          requiredCapabilities: ["adb_available", "apk_install"],
          unavailableCapabilities: [],
        },
        {
          id: "recipe.copy-bios",
          name: "Copy BIOS files",
          description: "Copy user-provided firmware files.",
          contentRequirements: ["bios_files"],
          selected: false,
          recommended: false,
          dependencyRequired: false,
          available: true,
          unavailableCapabilities: [],
        },
        {
          id: "recipe.root-tools",
          name: "Root tools",
          description: "Configure features that require root access.",
          contentRequirements: ["apk_file", "rom_content"],
          selected: false,
          recommended: false,
          dependencyRequired: false,
          available: false,
          requiredCapabilities: ["root_shell"],
          unavailableCapabilities: ["root_shell"],
        },
      ],
      inputs: [],
      diagnostics: [],
    });
    await renderReadyApp();

    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    await user.click(await screen.findByRole("button", { name: "Continue" }));
    await screen.findByRole("heading", { name: "Choose what to install" });

    expect(screen.getByRole("heading", { name: "1 selected" })).toBeTruthy();
    expect(screen.getByText("RetroArch", { selector: ".recipe-selection-summary p" })).toBeTruthy();
    expect(screen.getByText("Recipes · 3 of 3 shown")).toBeTruthy();
    expect(screen.getByText("Device connection")).toBeTruthy();
    expect(screen.getByText("App installation")).toBeTruthy();
    expect(screen.getByText("Also includes: Copy BIOS files")).toBeTruthy();
    expect(screen.getByText("Downloads files")).toBeTruthy();

    const search = screen.getByRole("searchbox", { name: "Search recipes" });
    await user.type(search, "firmware");
    expect(screen.getByRole("checkbox", { name: /Copy BIOS files/ })).toBeTruthy();
    expect(screen.getByText("BIOS files required")).toBeTruthy();
    expect(screen.queryByRole("checkbox", { name: /RetroArch/ })).toBeNull();
    expect(screen.getByText("Recipes · 1 of 3 shown")).toBeTruthy();

    await user.clear(search);
    await user.click(screen.getByRole("radio", { name: "Unavailable" }));
    expect(screen.getByRole("checkbox", { name: /Root tools/ })).toBeTruthy();
    expect(screen.getByText("APK file required")).toBeTruthy();
    expect(screen.getByText("ROM or content folder required")).toBeTruthy();
    expect(screen.getByText("Root access unavailable")).toBeTruthy();
    expect(screen.queryByText("root_shell")).toBeNull();
    expect(screen.queryByRole("checkbox", { name: /Copy BIOS files/ })).toBeNull();

    await user.click(screen.getByRole("radio", { name: "Selected" }));
    const selectedRecipe = screen.getByRole("checkbox", { name: /RetroArch/ });
    await user.click(selectedRecipe);
    expect(screen.getByRole("heading", { name: "0 selected" })).toBeTruthy();
    expect(screen.getByText("No recipes selected.")).toBeTruthy();
    expect(screen.getByText("No recipes match the current search and filter.")).toBeTruthy();
  });

  test("selects only available backend-recommended recipes and revalidates them", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    const initialDescription = {
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.custom"],
      expandedRecipes: ["recipe.custom"],
      recipeOptions: [
        {
          id: "recipe.frontend",
          name: "Frontend",
          description: "Install the recommended frontend.",
          selected: false,
          recommended: true,
          dependencyRequired: false,
          available: true,
          unavailableCapabilities: [],
        },
        {
          id: "recipe.shader-pack",
          name: "Shader pack",
          description: "Install recommended visual presets.",
          selected: false,
          recommended: true,
          dependencyRequired: false,
          available: true,
          unavailableCapabilities: [],
        },
        {
          id: "recipe.custom",
          name: "Custom tools",
          description: "Optional custom tools.",
          selected: true,
          recommended: false,
          dependencyRequired: false,
          available: true,
          unavailableCapabilities: [],
        },
        {
          id: "recipe.dependency",
          name: "Shared dependency",
          description: "Added automatically by the backend.",
          selected: false,
          recommended: true,
          dependencyRequired: true,
          available: true,
          unavailableCapabilities: [],
        },
        {
          id: "recipe.root-recommended",
          name: "Root enhancement",
          description: "Recommended only when root is available.",
          selected: false,
          recommended: true,
          dependencyRequired: false,
          available: false,
          unavailableCapabilities: ["root"],
        },
      ],
      inputs: [],
      diagnostics: [],
    };
    mockApi.describeConfiguration
      .mockResolvedValueOnce(initialDescription)
      .mockResolvedValueOnce({
        ...initialDescription,
        selectedRecipes: ["recipe.frontend", "recipe.shader-pack", "recipe.dependency"],
        expandedRecipes: ["recipe.frontend", "recipe.shader-pack", "recipe.dependency"],
        recipeOptions: initialDescription.recipeOptions.map((recipe) => ({
          ...recipe,
          selected: ["recipe.frontend", "recipe.shader-pack", "recipe.dependency"].includes(recipe.id),
        })),
      });
    await renderReadyApp();

    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    await user.click(await screen.findByRole("button", { name: "Continue" }));
    await screen.findByRole("heading", { name: "Choose what to install" });

    const recommended = screen.getByRole("button", { name: "Select recommended setup" });
    expect((recommended as HTMLButtonElement).disabled).toBe(false);
    await user.click(recommended);

    await vi.waitFor(() => {
      expect(mockApi.describeConfiguration).toHaveBeenLastCalledWith(expect.objectContaining({
        devicePlan: "plan.supported",
        selectedRecipes: ["recipe.frontend", "recipe.shader-pack"],
      }));
    });

    await vi.waitFor(() => {
      expect((screen.getByRole("checkbox", { name: /Frontend/ }) as HTMLInputElement).checked).toBe(true);
      expect((screen.getByRole("checkbox", { name: /Shader pack/ }) as HTMLInputElement).checked).toBe(true);
      expect((screen.getByRole("checkbox", { name: /Custom tools/ }) as HTMLInputElement).checked).toBe(false);
      expect((screen.getByRole("checkbox", { name: /Shared dependency/ }) as HTMLInputElement).checked).toBe(true);
      expect(screen.getByText("Added automatically because another selected recipe requires it.")).toBeTruthy();
      expect((screen.getByRole("checkbox", { name: /Root enhancement/ }) as HTMLInputElement).checked).toBe(false);
      expect((screen.getByRole("button", { name: "Recommended setup selected" }) as HTMLButtonElement).disabled).toBe(true);
    });
  });

  test("distinguishes unauthorized devices and deliberately gates backend safe generic plans", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([
      {
        deviceHandle: "unauthorized-opaque",
        state: "unauthorized",
        displayName: "Authorization pending",
        maskedSerial: "••••0001",
      },
      availableDevice,
    ]);
    mockApi.matchDevice.mockResolvedValue({
      confidence: "none",
      recommendedPlanId: null,
      requiresExplicitChoice: true,
      candidates: [],
      safeGenericPlans: [safePlan],
      blocked: false,
      blockReason: null,
    });
    await renderReadyApp();

    const unauthorized = await screen.findByRole("button", { name: /Authorization pending.*Authorization required/ });
    expect((unauthorized as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/accept the USB debugging prompt/i)).toBeTruthy();
    expect(screen.getByText("Customize")).toBeTruthy();

    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    await user.click(screen.getByRole("button", { name: "Refresh devices" }));

    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    await screen.findByRole("heading", { name: "This device is not officially supported" });
    expect(screen.queryByText(safePlan.name)).toBeNull();

    const continueButton = screen.getByRole("button", { name: "Show safe generic setups" });
    expect((continueButton as HTMLButtonElement).disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /not officially supported.*not device-specific/i }));
    expect(screen.queryByText(safePlan.name)).toBeNull();
    await user.click(continueButton);

    const planRadio = await screen.findByRole("radio", { name: /Backend Safe Generic/ });
    expect((planRadio as HTMLInputElement).checked).toBe(false);
  });

  test("separates Platform-Tools picker and installation progress", async () => {
    const user = userEvent.setup();
    const picker = deferred<{ outcome: "selected"; selectionHandle: string }>();
    const installation = deferred<typeof readyAdb>();
    mockApi.adbStatus.mockResolvedValue(missingAdb);
    mockApi.pickPlatformToolsZip.mockReturnValue(picker.promise);
    mockApi.installPlatformToolsSelection.mockReturnValue(installation.promise);
    render(<App />);
    await screen.findByRole("heading", { name: "Set up Android Platform-Tools" });

    const importButton = screen.getByRole("button", { name: "Select Platform-Tools ZIP…" });
    await user.click(importButton);
    expect(screen.getByRole("button", { name: "Choosing ZIP…" })).toBeTruthy();
    expect((screen.getByRole("button", { name: "Choosing ZIP…" }) as HTMLButtonElement).disabled).toBe(true);

    await act(async () => picker.resolve({ outcome: "selected", selectionHandle: "selection-opaque" }));
    expect(await screen.findByRole("button", { name: "Checking and installing…" })).toBeTruthy();
    expect(mockApi.installPlatformToolsSelection).toHaveBeenCalledWith("selection-opaque", undefined);

    await act(async () => installation.resolve(readyAdb));
    await screen.findByRole("heading", { name: "Choose an Android device" });
    expect(mockApi.executionCapabilities).toHaveBeenCalledTimes(2);
  });

  test("requires removal confirmation and cancellation has no side effect", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(await openSupportAction(user, "Remove managed Platform-Tools", supportSnapshot({ platformActions: true })));
    const firstDialog = screen.getByRole("alertdialog", { name: "Remove Platform-Tools?" });
    await user.click(within(firstDialog).getByRole("button", { name: "Cancel" }));
    expect(mockApi.removePlatformTools).not.toHaveBeenCalled();

    await user.click(await openSupportAction(user, "Remove managed Platform-Tools", supportSnapshot({ platformActions: true })));
    const secondDialog = screen.getByRole("alertdialog", { name: "Remove Platform-Tools?" });
    expect(secondDialog.textContent).not.toMatch(/authority|handle/i);
    await user.click(within(secondDialog).getByRole("button", { name: "Remove" }));
    expect(mockApi.removePlatformTools).toHaveBeenCalledWith(7);
    await screen.findByRole("heading", { name: "Set up Android Platform-Tools" });
    expect(mockApi.executionCapabilities).toHaveBeenCalledTimes(2);
  });

  test("refresh is single-flight and completion feedback expires after five seconds", async () => {
    mockApi.pollDevices.mockResolvedValueOnce([]);
    const refresh = deferred<Array<typeof availableDevice>>();
    mockApi.pollDevices.mockReturnValue(refresh.promise);
    await renderReadyApp();

    vi.useFakeTimers();
    const refreshButton = screen.getByRole("button", { name: "Refresh devices" });
    fireEvent.click(refreshButton);
    expect((screen.getByRole("button", { name: "Refreshing…" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Refreshing…" }));
    expect(mockApi.pollDevices).toHaveBeenCalledTimes(2);

    await act(async () => refresh.resolve([availableDevice]));
    expect(screen.getAllByText("Refresh complete. 1 device found.")).toHaveLength(2);
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.getAllByText("Refresh complete. 1 device found.")).toHaveLength(1);
    vi.useRealTimers();
  });

  test("replacement reconciliation reports rediscovery without a disconnect warning", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    await renderReadyApp();
    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    await screen.findByRole("heading", { name: "Example Handheld" });

    await user.click(await openSupportAction(user, "Replace managed Platform-Tools", supportSnapshot({ platformActions: true })));
    expect(await screen.findByText(/Your device was rediscovered/)).toBeTruthy();
    expect(mockApi.executionCapabilities).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(/selected device disconnected/i)).toBeNull();
  });

  test("clean idle restart proceeds without a recovery prompt", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true })));

    await waitFor(() => expect(mockApi.restartRuntime).toHaveBeenCalledTimes(1));
    expect(mockApi.executionCapabilities).toHaveBeenCalledTimes(2);
    expect(mockApi.stageRecoveryDraft).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "Restart the app service?" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Choose an Android device" })).toBeTruthy();
  });

  test("rejects a device refresh response from before a runtime restart", async () => {
    const user = userEvent.setup();
    const staleRefresh = deferred<Array<typeof availableDevice>>();
    mockApi.pollDevices
      .mockResolvedValueOnce([])
      .mockReturnValueOnce(staleRefresh.promise)
      .mockResolvedValue([]);
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Refresh devices" }));
    await user.click(await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true })));
    await screen.findByRole("heading", { name: "Choose an Android device" });
    await act(async () => staleRefresh.resolve([availableDevice]));

    expect(screen.queryByRole("button", { name: /Supported Handheld.*Connected/ })).toBeNull();
    expect(screen.queryByText("Refresh complete. 1 device found.")).toBeNull();
  });

  test("restart confirmation exposes only friendly labels for backend-omitted values", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/token",
      inputId: "token",
      label: "Account token",
      required: true,
      sensitive: true,
    }));
    mockApi.stageRecoveryDraft.mockImplementation(async (request: { requestGeneration: number; draftGeneration: number }) => ({
      requestGeneration: request.requestGeneration,
      draftGeneration: request.draftGeneration,
      recordGeneration: request.draftGeneration,
      omittedBindings: ["recipe.one/token"],
    }));
    mockApi.restoreRecoveryDraft.mockImplementation(async (
      _sessionGeneration: number,
      draftGeneration: number,
      requestGeneration: number,
    ) => ({
      requestGeneration,
      draftGeneration,
      displayName: null,
      sourceStatus: "unsaved",
      document: null,
      intent: {
        dirty: true,
        devicePlan: "plan.supported",
        selectedRecipes: ["recipe.one"],
        bindings: {},
        requiredReentryBindings: ["recipe.one/token"],
      },
    }));
    await advanceToInputs(user);
    expect(screen.getByRole("heading", { name: "Other" })).toBeTruthy();
    expect(screen.getByText("Required")).toBeTruthy();
    expect(screen.getByText("Text")).toBeTruthy();
    expect(screen.getByText("Not saved")).toBeTruthy();
    await user.type(screen.getByLabelText("Account token"), "do-not-display");

    const restartButton = await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true }));
    await user.click(restartButton);
    const dialog = await screen.findByRole("alertdialog", { name: "Restart the app service?" });
    expect(dialog.textContent).toMatch(/Affected fields: Account token/);
    expect(dialog.textContent).not.toMatch(/do-not-display|recipe\.one\/token/);
    await user.click(within(dialog).getByRole("button", { name: "Restart app service" }));

    expect(mockApi.restartRuntime).toHaveBeenCalledTimes(1);
    await screen.findByRole("heading", { name: "Choose an Android device" });
    expect(await screen.findByText("Setup restored after restart")).toBeTruthy();
    expect(await screen.findAllByText(/Your setup choices were restored, but they have not been saved/i)).toHaveLength(2);
    expect(screen.queryByText(/recovered intent|source until you save/i)).toBeNull();
    expect(await screen.findAllByText(/Re-enter 1 sensitive input/)).toHaveLength(2);
    expect(mockApi.stageRecoveryDraft).toHaveBeenCalledWith(expect.objectContaining({
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.one"],
      bindings: { "recipe.one/token": "do-not-display" },
      dirty: true,
    }));
  });

  test("cancelling a restart with omitted values preserves dirty workflow state and focus", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/token",
      inputId: "token",
      label: "Account token",
      required: true,
      sensitive: true,
    }));
    mockApi.stageRecoveryDraft.mockImplementation(async (request: { requestGeneration: number; draftGeneration: number }) => ({
      requestGeneration: request.requestGeneration,
      draftGeneration: request.draftGeneration,
      recordGeneration: request.draftGeneration,
      omittedBindings: ["recipe.one/token"],
    }));
    await advanceToInputs(user);
    const input = screen.getByLabelText("Account token") as HTMLInputElement;
    await user.type(input, "keep-current-value");
    const supportButton = screen.getByRole("button", { name: "Troubleshooting" });
    vi.spyOn(supportButton, "getClientRects").mockReturnValue([{}] as unknown as DOMRectList);
    const restartButton = await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true }));

    await user.click(restartButton);
    const dialog = await screen.findByRole("alertdialog", { name: "Restart the app service?" });
    expect(dialog.textContent).not.toMatch(/keep-current-value|recipe\.one\/token/);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(mockApi.restartRuntime).not.toHaveBeenCalled();
    expect(input.value).toBe("keep-current-value");
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(supportButton));
  });

  test("successful restart restores nonsensitive portable intent without prompting", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/theme",
      inputId: "theme",
      label: "Theme",
      required: false,
      sensitive: false,
    }));
    mockApi.restoreRecoveryDraft.mockImplementation(async (
      _sessionGeneration: number,
      draftGeneration: number,
      requestGeneration: number,
    ) => ({
      requestGeneration,
      draftGeneration,
      displayName: null,
      sourceStatus: "unsaved",
      document: null,
      intent: {
        dirty: true,
        devicePlan: "plan.supported",
        selectedRecipes: ["recipe.one"],
        bindings: { "recipe.one/theme": "dark" },
        requiredReentryBindings: [],
      },
    }));
    await advanceToInputs(user);
    await user.type(screen.getByLabelText("Theme"), "dark");

    await user.click(await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true })));

    expect(screen.queryByRole("alertdialog", { name: "Restart the app service?" })).toBeNull();
    expect(await screen.findByText("Setup restored after restart")).toBeTruthy();
    expect(mockApi.stageRecoveryDraft).toHaveBeenCalledWith(expect.objectContaining({
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.one"],
      bindings: { "recipe.one/theme": "dark" },
      dirty: true,
    }));
    expect(mockApi.restoreRecoveryDraft).toHaveBeenCalledTimes(1);
  });

  test("restart failure preserves the active portable workflow and settles its guard", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/theme",
      inputId: "theme",
      label: "Theme",
      required: false,
      sensitive: false,
    }));
    mockApi.restartRuntime.mockRejectedValue(JSON.stringify({
      code: "runtime_restart_failed",
      message: "The app service could not restart. Try again.",
    }));
    await advanceToInputs(user);
    const input = screen.getByLabelText("Theme") as HTMLInputElement;
    await user.type(input, "dark");

    await user.click(await openSupportAction(user, "Restart app service", supportSnapshot({ serviceFailure: true })));

    expect(await screen.findAllByText("The app service could not restart. Try again.")).not.toHaveLength(0);
    expect(input.value).toBe("dark");
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
    await waitFor(() => expect(
      (screen.getByRole("button", { name: "Troubleshooting" }) as HTMLButtonElement).disabled,
    ).toBe(false));
    expect(document.body.textContent).not.toMatch(/runtime_restart_failed/);
  });
});

describe("Phase 5D input collection and repair", () => {
  test("new required inputs stay neutral until validation is requested", async () => {
    const user = userEvent.setup();
    const description = descriptionWithTextInput({
      key: "recipe.one/account",
      inputId: "account",
      label: "Account name",
      required: true,
      sensitive: false,
    });
    description.inputs[0].description = "Enter the account name used by this setup.";
    description.inputs[0].diagnostics = [{
      key: "recipe.one/account",
      code: "binding_missing",
      message: "Account name is required.",
      severity: "error",
    }];
    mockApi.describeConfiguration.mockResolvedValue(description);
    await advanceToInputs(user);

    expect(screen.queryByRole("heading", { name: /Resolve .* configuration error/ })).toBeNull();
    expect(screen.getByText("Enter the account name used by this setup.")).toBeTruthy();
    expect(screen.getByLabelText("Account name requirements").textContent).toContain("Required");

    await user.click(screen.getByRole("button", { name: "Review plan" }));
    const summaryHeading = await screen.findByRole("heading", {
      name: "Resolve 1 configuration error",
    });
    expect(summaryHeading.parentElement?.textContent).toContain("Account name is required.");
    expect(document.body.textContent).not.toContain("binding_missing");
  });

  test("ordinary validation renders label-based guidance without binding keys or codes", async () => {
    const user = userEvent.setup();
    const review = deferred<unknown>();
    const bindingKey = "app.xaniteog.install/xaniteog_apk";
    const diagnosticCode = "binding_path_missing";
    const message = "XaniteOG APK could not be found. Select it again.";
    const diagnostic = {
      key: bindingKey,
      code: diagnosticCode,
      message,
      severity: "error" as const,
      entryIndex: 0,
    };
    mockApi.describeConfiguration.mockResolvedValue({
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.one"],
      expandedRecipes: ["recipe.one"],
      recipeOptions: [{
        id: "recipe.one", name: "Recipe One", description: null, selected: true,
        recommended: true, dependencyRequired: false, available: true,
        unavailableCapabilities: [],
      }],
      inputs: [{
        key: bindingKey,
        recipeId: "app.xaniteog.install",
        inputId: "xaniteog_apk",
        type: "file",
        label: "XaniteOG APK",
        description: "Choose the XaniteOG Android application package.",
        required: true,
        multiple: false,
        sensitive: false,
        pathKind: "file",
        acceptedExtensions: ["apk"],
        presentationCategory: "Applications",
        presentationKind: "Single file",
        value: "/selected/missing-xaniteog.apk",
        valueSource: "explicit",
        entries: [{
          index: 0,
          displayName: "missing-xaniteog.apk",
          displayPath: "/selected/missing-xaniteog.apk",
          state: "error",
          diagnostics: [diagnostic],
        }],
        diagnostics: [diagnostic],
      }],
      diagnostics: [],
    });
    mockApi.createReview.mockReturnValue(review.promise);
    await advanceToInputs(user);
    await user.click(screen.getByRole("button", { name: "Review plan" }));

    const field = screen.getByRole("group", { name: "XaniteOG APK" });
    const summaryHeading = await screen.findByRole("heading", {
      name: "Resolve 1 configuration error",
    });
    const summary = summaryHeading.parentElement;
    expect(within(field).getByText(`Error: ${message}`)).toBeTruthy();
    expect(within(summary!).getByText(message)).toBeTruthy();
    for (const ordinarySurface of [field, summary]) {
      expect(ordinarySurface?.textContent).not.toContain(bindingKey);
      expect(ordinarySurface?.textContent).not.toContain(diagnosticCode);
    }
  });

  test("renders multi-file entries with per-entry repair and sanitized diagnostics", async () => {
    const user = userEvent.setup();
    const description = {
      devicePlan: "plan.supported",
      selectedRecipes: ["recipe.one"],
      expandedRecipes: ["recipe.one"],
      recipeOptions: [{
        id: "recipe.one", name: "Recipe One", description: null, selected: true,
        recommended: true, dependencyRequired: false, available: true,
        contentRequirements: ["rom_content"], unavailableCapabilities: [],
      }],
      inputs: [{
        key: "recipe.one/files", recipeId: "recipe.one", inputId: "files", type: "file",
        label: "Game files", description: "Choose game files to copy.", required: true,
        multiple: true, sensitive: false, pathKind: "file", acceptedExtensions: ["rom"],
        presentationCategory: "Games and content", presentationKind: "Multiple files",
        value: ["/selected/one.rom", "/moved/two.rom"], valueSource: "explicit",
        entries: [
          { index: 0, displayName: "one.rom", displayPath: "/selected/one.rom", state: "valid", diagnostics: [] },
          {
            index: 1, displayName: "two.rom", displayPath: "/moved/two.rom", state: "error",
            diagnostics: [{
              key: "recipe.one/files", code: "binding_path_missing",
              message: "Game files could not be found. Select it again.", severity: "error", entryIndex: 1,
            }],
          },
        ],
        diagnostics: [{
          key: "recipe.one/files", code: "binding_path_missing",
          message: "Game files could not be found. Select it again.", severity: "error", entryIndex: 1,
        }],
      }],
      diagnostics: [],
    };
    mockApi.describeConfiguration.mockResolvedValue(description);
    mockApi.pickInputPath.mockResolvedValue([
      "/selected/one.rom", "/moved/two.rom", "/selected/three.rom",
    ]);
    await advanceToInputs(user);

    expect(screen.getByText("Accepted file types: rom")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Relink…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add files…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove one.rom from Game files" })).toBeTruthy();
    expect(document.body.textContent).not.toContain("binding_path_missing");

    await user.click(screen.getByRole("button", { name: "Add files…" }));
    expect(mockApi.pickInputPath).toHaveBeenCalledWith(expect.objectContaining({
      inputKey: "recipe.one/files",
      mode: "append",
      entryIndex: null,
    }));
    expect((await screen.findAllByText("/selected/three.rom")).length).toBeGreaterThan(0);
  });

  test("picker cancellation settles and a response after leaving Inputs is ignored", async () => {
    const user = userEvent.setup();
    const picker = deferred<unknown | null>();
    mockApi.describeConfiguration.mockResolvedValue({
      devicePlan: "plan.supported", selectedRecipes: ["recipe.one"], expandedRecipes: ["recipe.one"],
      recipeOptions: [{
        id: "recipe.one", name: "Recipe One", description: null, selected: true,
        recommended: true, dependencyRequired: false, available: true, unavailableCapabilities: [],
      }],
      inputs: [{
        key: "recipe.one/file", recipeId: "recipe.one", inputId: "file", type: "file",
        label: "Setup file", description: "Choose a setup file.", required: false, multiple: false,
        sensitive: false, pathKind: "file", acceptedExtensions: ["cfg"],
        presentationCategory: "Other", presentationKind: "Single file", value: null,
        valueSource: null, entries: [], diagnostics: [],
      }], diagnostics: [],
    });
    mockApi.pickInputPath.mockReturnValue(picker.promise);
    await advanceToInputs(user);
    await user.click(screen.getByRole("button", { name: "Choose file…" }));
    expect((screen.getByRole("button", { name: "Choose file…" }) as HTMLButtonElement).disabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "Back" }));
    await screen.findByRole("heading", { name: "Choose what to install" });
    await act(async () => picker.resolve("/selected/stale.cfg"));
    expect(screen.queryByText("/selected/stale.cfg")).toBeNull();
    expect(await screen.findAllByText("An outdated file selection was ignored.")).not.toHaveLength(0);
  });

  test("device destinations stay textual and sensitive values use concealed controls", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue({
      devicePlan: "plan.supported", selectedRecipes: ["recipe.one"], expandedRecipes: ["recipe.one"],
      recipeOptions: [{
        id: "recipe.one", name: "Recipe One", description: null, selected: true,
        recommended: true, dependencyRequired: false, available: true, unavailableCapabilities: [],
      }],
      inputs: [
        {
          key: "recipe.one/destination", recipeId: "recipe.one", inputId: "destination",
          type: "device_path", label: "Device folder", description: "Folder on the Android device.",
          required: true, multiple: false, sensitive: false, presentationCategory: "Destination",
          presentationKind: "Device folder", value: "/sdcard/Games", valueSource: "recipe_default", diagnostics: [],
        },
        {
          key: "recipe.one/token", recipeId: "recipe.one", inputId: "token", type: "string",
          label: "Account token", description: "Token used for this setup.", required: true,
          multiple: false, sensitive: true, presentationCategory: "Other", presentationKind: "Text",
          value: null, valueSource: null, diagnostics: [],
        },
      ], diagnostics: [],
    });
    await advanceToInputs(user);
    expect((screen.getByLabelText("Device folder") as HTMLInputElement).value).toBe("/sdcard/Games");
    expect(screen.queryByRole("button", { name: /Choose folder|Browse/i })).toBeNull();
    expect((screen.getByLabelText("Account token") as HTMLInputElement).type).toBe("password");
    expect(screen.getByText("Not saved")).toBeTruthy();
  });
});

describe("Phase 5G troubleshooting and local-state maintenance", () => {
  test("keeps persistent maintenance controls out of the primary workflow", async () => {
    await renderReadyApp();

    expect(screen.queryByRole("button", { name: "Restart app service" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Replace managed Platform-Tools" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Remove managed Platform-Tools" })).toBeNull();
    expect(screen.getByRole("button", { name: "Troubleshooting" })).toBeTruthy();
  });

  test("renders a concise healthy summary and keeps subsystem detail collapsed", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    const dialog = await screen.findByRole("dialog", { name: "Troubleshooting and app storage" });
    expect(within(dialog).getByText("EmuChef is ready. No troubleshooting issues were found.")).toBeTruthy();
    expect(dialog.querySelectorAll("#troubleshooting-heading ~ article")).toHaveLength(0);
    const details = within(dialog).getByText("View all troubleshooting status and maintenance actions").closest("details");
    expect(details?.open).toBe(false);
  });

  test("copies only the stable public support code for an affected subsystem", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    mockApi.supportSnapshot.mockResolvedValue(supportSnapshot({ serviceFailure: true }));
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    const dialog = await screen.findByRole("dialog", { name: "Troubleshooting and app storage" });
    expect(within(dialog).getAllByText("The local app service could not start.").length).toBeGreaterThan(0);
    expect(dialog.textContent).not.toMatch(/sidecar|runtime handle|internal_/i);
    await user.click(within(dialog).getByRole("button", { name: "Copy support code" }));

    expect(writeText).toHaveBeenCalledWith("EMUCHEF-SERVICE-START-FAILED");
    expect(await screen.findByText("Support code EMUCHEF-SERVICE-START-FAILED copied.")).toBeTruthy();
  });

  test("discloses local-only diagnostics and disables empty cache cleanup", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    const dialog = await screen.findByRole("dialog", { name: "Troubleshooting and app storage" });
    expect(within(dialog).getByText(/stays on this Mac until you choose to share it/i)).toBeTruthy();
    expect(within(dialog).getByText(/exporting never uploads anything/i)).toBeTruthy();
    expect(within(dialog).getByText(/The app-owned cache is empty/)).toBeTruthy();
    expect(within(dialog).getByText("There are no unused removable cache entries.")).toBeTruthy();
    expect((within(dialog).getByRole("button", { name: "Clear unused" }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(dialog).getByRole("button", { name: "Clear all removable" }) as HTMLButtonElement).disabled).toBe(true);
  });

  test("requires category-specific confirmation before resetting Recents", async () => {
    const user = userEvent.setup();
    const snapshot = supportSnapshot();
    snapshot.resetCategories = [{
      resetHandle: "reset-recents-opaque",
      id: "recents",
      label: "Clear Recents",
      description: "Remove the app-owned list of recently opened setup files.",
      consequence: "Saved setup files, the active setup, and portable intent remain unchanged.",
      affectedScope: "Recent setup index only",
      available: true,
      unavailableReason: null,
      confirmationRequired: true,
      restartRequired: false,
      itemCount: 2,
      totalSizeBytes: null,
    }];
    mockApi.supportSnapshot.mockResolvedValue(snapshot);
    mockApi.resetLocalAppState.mockResolvedValue({
      outcome: { summary: "Recent setup history was cleared. Saved setup files were preserved." },
      snapshot: { ...snapshot, resetCategories: [] },
    });
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    await user.click(await screen.findByRole("button", { name: "Clear Recents" }));
    const confirmation = screen.getByRole("alertdialog", { name: "Clear Recents?" });
    expect(confirmation.textContent).toContain("Saved setup files, the active setup, and portable intent remain unchanged.");
    expect(confirmation.textContent).toContain("Recent setup index only");
    expect(mockApi.resetLocalAppState).not.toHaveBeenCalled();
    await user.click(within(confirmation).getByRole("button", { name: "Confirm reset" }));

    await waitFor(() => expect(mockApi.resetLocalAppState).toHaveBeenCalledWith("reset-recents-opaque"));
  });

  test("fails closed when an unknown corrective-action variant reaches dispatch", async () => {
    const user = userEvent.setup();
    const snapshot = supportSnapshot({ serviceFailure: true });
    snapshot.subsystems[0].actions[0].action = { kind: "future_untrusted_action" } as never;
    mockApi.supportSnapshot.mockResolvedValue(snapshot);
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Troubleshooting" }));
    await user.click(await screen.findByRole("button", { name: "Restart app service" }));

    expect(await screen.findByText("This troubleshooting action is not supported by this version of EmuChef.")).toBeTruthy();
    expect(mockApi.restartRuntime).not.toHaveBeenCalled();
  });
});

describe("Phase 5F saved setup management", () => {
  test("reviews a sanitized V1 compatibility summary before opening", async () => {
    const user = userEvent.setup();
    mockApi.previewSavedConfiguration.mockResolvedValue({
      outcome: "previewed",
      previewHandle: "preview-opaque",
      name: "Travel setup",
      fileLabel: "travel.yaml",
      schemaVersion: 1,
      lastModifiedEpochMs: null,
      setupLabel: "Supported setup",
      featureLabels: ["Recipe One"],
      savedInputCount: 1,
      omittedInputCount: 1,
      compatibility: {
        state: "migrated_baseline_pending",
        baselineState: "pending_first_v2_save",
        requiresRepair: false,
        message: "This older setup is valid against the current catalog.",
      },
      repairActions: [],
    });
    mockApi.compareSavedConfigurationPreview.mockResolvedValue({
      state: "no_current_intent",
      message: "There are no current setup choices to compare.",
    });
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Manage saved setups…" }));
    expect(screen.getByRole("heading", { name: "Saved setups" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "New" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save As…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Import…" })).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Open…" }));
    expect(await screen.findByRole("heading", { name: "Review Travel setup" })).toBeTruthy();
    expect(screen.getByText("Supported setup")).toBeTruthy();
    expect(screen.getByText(/next explicit save will record current compatibility information/)).toBeTruthy();
    expect(screen.getByText("No setup is currently in progress.")).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/Schema 1|current catalog|pending_first_v2_save/);
    expect(document.body.textContent).not.toContain("/Users/");
    expect(mockApi.confirmSavedConfigurationPreview).not.toHaveBeenCalled();
  });

  test("uses concrete save disclosure and preserves the Inputs stage after Save", async () => {
    const user = userEvent.setup();
    mockApi.describeConfiguration.mockResolvedValue(descriptionWithTextInput({
      key: "recipe.one/option",
      inputId: "option",
      label: "Optional setting",
      required: false,
      sensitive: false,
    }));
    await advanceToInputs(user);
    await user.type(screen.getByLabelText("Optional setting"), "chosen");
    mockApi.createSavedConfiguration.mockResolvedValue({
      outcome: "saved",
      configurationHandle: "configuration-opaque",
      name: "My EmuChef setup",
      schemaVersion: 2,
      dirty: false,
      revision: 0,
      devicePlan: "plan.supported",
      selectedRecipes: [],
      bindings: { "recipe.one/option": "chosen" },
      pendingSanitationCount: 0,
      compatibility: { baselineState: "unchanged" },
      validation: { state: "valid", diagnostics: [] },
    });

    await user.click(screen.getByRole("button", { name: "Manage saved setups…" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("heading", { name: "Name this setup" })).toBeTruthy();
    expect(screen.getByText(/selected setup, features, and reusable input references/)).toBeTruthy();
    expect(screen.getByText(/does not save the connected device, generated plan, execution progress, or results/)).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(await screen.findByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
    expect(mockApi.createSavedConfiguration).toHaveBeenCalledWith(expect.objectContaining({
      devicePlan: "plan.supported",
    }));

    mockApi.duplicateSavedConfiguration.mockResolvedValue({
      outcome: "saved",
      name: "My EmuChef setup copy",
      fileLabel: "My-EmuChef-setup-copy.yaml",
    });
    await user.click(screen.getByRole("button", { name: "Manage saved setups…" }));
    await user.click(screen.getByRole("button", { name: "Duplicate…" }));
    expect(await screen.findByRole("heading", { name: "Name the duplicate setup" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(mockApi.duplicateSavedConfiguration).toHaveBeenCalled());
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();

    mockApi.exportSavedConfiguration.mockResolvedValue({
      outcome: "saved",
      name: "My EmuChef setup export",
      fileLabel: "My-EmuChef-setup-export.yaml",
    });
    await user.click(screen.getByRole("button", { name: "Export…" }));
    expect(await screen.findByRole("heading", { name: "Name the exported setup" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(mockApi.exportSavedConfiguration).toHaveBeenCalled());
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
  });

  test("ignores a preview response after the management surface is cancelled", async () => {
    const user = userEvent.setup();
    const preview = deferred<unknown>();
    mockApi.previewSavedConfiguration.mockReturnValue(preview.promise);
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Manage saved setups…" }));
    await user.click(screen.getByRole("button", { name: "Open…" }));
    await user.click(screen.getByRole("button", { name: "Close" }));
    preview.resolve({
      outcome: "previewed",
      previewHandle: "stale-preview",
      name: "Stale setup",
      fileLabel: "stale.yaml",
      schemaVersion: 2,
      lastModifiedEpochMs: null,
      setupLabel: "Stale setup",
      featureLabels: [],
      savedInputCount: 0,
      omittedInputCount: 0,
      compatibility: {
        state: "compatible",
        baselineState: "unchanged",
        requiresRepair: false,
        message: "Compatible.",
      },
      repairActions: [],
    });
    await act(async () => { await preview.promise; });

    expect(screen.queryByRole("heading", { name: "Review Stale setup" })).toBeNull();
    expect(mockApi.compareSavedConfigurationPreview).not.toHaveBeenCalled();
  });
});

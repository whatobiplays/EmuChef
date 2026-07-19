import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mockApi = vi.hoisted(() => ({
  adbStatus: vi.fn(),
  beginAppSession: vi.fn(),
  beginUpdateInteractionSession: vi.fn(),
  catalog: vi.fn(),
  describeConfiguration: vi.fn(),
  endUpdateInteractionSession: vi.fn(),
  finishAppSession: vi.fn(),
  installPlatformToolsSelection: vi.fn(),
  listRecentConfigurations: vi.fn(),
  matchDevice: vi.fn(),
  pickPlatformToolsZip: vi.fn(),
  pollDevices: vi.fn(),
  probeDevice: vi.fn(),
  realExecutionAvailability: vi.fn(),
  removePlatformTools: vi.fn(),
  restartRuntime: vi.fn(),
  runtimeStatus: vi.fn(),
  setUpdateInteractionState: vi.fn(),
  stageRecoveryDraft: vi.fn(),
  restoreRecoveryDraft: vi.fn(),
}));

vi.mock("../src/api", () => ({ api: mockApi }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    close: vi.fn(),
    onCloseRequested: vi.fn(async () => () => undefined),
  }),
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
  mockApi.realExecutionAvailability.mockResolvedValue({ enabled: false });
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
  mockApi.finishAppSession.mockResolvedValue(undefined);
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

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolver) => { resolve = resolver; });
  return { promise, resolve };
}

beforeEach(() => resetApi());

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
          unavailableCapabilities: [],
        },
        {
          id: "recipe.copy-bios",
          name: "Copy BIOS files",
          description: "Copy user-provided firmware files.",
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
          selected: false,
          recommended: false,
          dependencyRequired: false,
          available: false,
          unavailableCapabilities: ["root"],
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

    const search = screen.getByRole("searchbox", { name: "Search recipes" });
    await user.type(search, "firmware");
    expect(screen.getByRole("checkbox", { name: /Copy BIOS files/ })).toBeTruthy();
    expect(screen.queryByRole("checkbox", { name: /RetroArch/ })).toBeNull();
    expect(screen.getByText("Recipes · 1 of 3 shown")).toBeTruthy();

    await user.clear(search);
    await user.click(screen.getByRole("radio", { name: "Unavailable" }));
    expect(screen.getByRole("checkbox", { name: /Root tools/ })).toBeTruthy();
    expect(screen.queryByRole("checkbox", { name: /Copy BIOS files/ })).toBeNull();

    await user.click(screen.getByRole("radio", { name: "Selected" }));
    const selectedRecipe = screen.getByRole("checkbox", { name: /RetroArch/ });
    await user.click(selectedRecipe);
    expect(screen.getByRole("heading", { name: "0 selected" })).toBeTruthy();
    expect(screen.getByText("No recipes selected.")).toBeTruthy();
    expect(screen.getByText("No recipes match the current search and filter.")).toBeTruthy();
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
    await screen.findByRole("heading", { name: "Android SDK Platform-Tools is required" });

    const importButton = screen.getByRole("button", { name: "Import Platform-Tools ZIP" });
    await user.click(importButton);
    expect(screen.getByRole("button", { name: "Choosing ZIP…" })).toBeTruthy();
    expect((screen.getByRole("button", { name: "Choosing ZIP…" }) as HTMLButtonElement).disabled).toBe(true);

    await act(async () => picker.resolve({ outcome: "selected", selectionHandle: "selection-opaque" }));
    expect(await screen.findByRole("button", { name: "Validating and installing…" })).toBeTruthy();
    expect(mockApi.installPlatformToolsSelection).toHaveBeenCalledWith("selection-opaque");

    await act(async () => installation.resolve(readyAdb));
    await screen.findByRole("heading", { name: "Choose an Android device" });
  });

  test("requires removal confirmation and cancellation has no side effect", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Remove Platform-Tools" }));
    const firstDialog = screen.getByRole("alertdialog", { name: "Remove Platform-Tools?" });
    await user.click(within(firstDialog).getByRole("button", { name: "Cancel" }));
    expect(mockApi.removePlatformTools).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Remove Platform-Tools" }));
    const secondDialog = screen.getByRole("alertdialog", { name: "Remove Platform-Tools?" });
    expect(secondDialog.textContent).not.toMatch(/authority|handle/i);
    await user.click(within(secondDialog).getByRole("button", { name: "Remove" }));
    expect(mockApi.removePlatformTools).toHaveBeenCalledTimes(1);
    await screen.findByRole("heading", { name: "Android SDK Platform-Tools is required" });
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

    await user.click(screen.getByRole("button", { name: "Replace Platform-Tools" }));
    expect(await screen.findByText(/Your device was rediscovered/)).toBeTruthy();
    expect(screen.queryByText(/selected device disconnected/i)).toBeNull();
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
    await user.click(screen.getByRole("button", { name: "Restart runtime" }));
    await screen.findByRole("heading", { name: "Choose an Android device" });
    await act(async () => staleRefresh.resolve([availableDevice]));

    expect(screen.queryByRole("button", { name: /Supported Handheld.*Connected/ })).toBeNull();
    expect(screen.queryByText("Refresh complete. 1 device found.")).toBeNull();
  });

  test("restart confirmation exposes only friendly labels for backend-omitted values", async () => {
    const user = userEvent.setup();
    mockApi.pollDevices.mockResolvedValue([availableDevice]);
    mockApi.describeConfiguration.mockResolvedValue({
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
        key: "recipe.one/token",
        recipeId: "recipe.one",
        inputId: "token",
        type: "string",
        label: "Account token",
        description: null,
        required: true,
        sensitive: true,
        value: null,
        valueSource: null,
        diagnostics: [],
      }],
      diagnostics: [],
    });
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
        selectedRecipes: [],
        bindings: {},
        requiredReentryBindings: ["recipe.one/token"],
      },
    }));
    await renderReadyApp();
    await user.click(await screen.findByRole("button", { name: /Supported Handheld.*Connected/ }));
    await screen.findByRole("heading", { name: "Example Handheld" });
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByRole("heading", { name: "Choose what to install" });
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByRole("heading", { name: "Provide required files and options" });
    await user.type(screen.getByLabelText("Account token (required)"), "do-not-display");

    await user.click(screen.getByRole("button", { name: "Restart runtime" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Restart the runtime?" });
    expect(dialog.textContent).toMatch(/Affected fields: Account token/);
    expect(dialog.textContent).not.toMatch(/do-not-display|recipe\.one\/token/);
    await user.click(within(dialog).getByRole("button", { name: "Restart" }));

    expect(mockApi.restartRuntime).toHaveBeenCalledTimes(1);
    await screen.findByRole("heading", { name: "Choose an Android device" });
    expect(await screen.findByText("Setup restored after restart")).toBeTruthy();
    expect(await screen.findAllByText(/Your setup choices were restored, but they have not been saved/i)).toHaveLength(2);
    expect(screen.queryByText(/recovered intent|source until you save/i)).toBeNull();
    expect(await screen.findAllByText(/Re-enter 1 sensitive input/)).toHaveLength(2);
  });
});

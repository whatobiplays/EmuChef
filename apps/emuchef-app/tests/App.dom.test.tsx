import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

function descriptionWithTextInput(input: {
  key: string;
  inputId: string;
  label: string;
  required: boolean;
  sensitive: boolean;
}) {
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
    expect(screen.getByText("ADB connection")).toBeTruthy();
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

  test("clean idle restart proceeds without a recovery prompt", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: "Restart runtime" }));

    await waitFor(() => expect(mockApi.restartRuntime).toHaveBeenCalledTimes(1));
    expect(mockApi.stageRecoveryDraft).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog", { name: "Restart the runtime?" })).toBeNull();
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
    await user.click(screen.getByRole("button", { name: "Restart runtime" }));
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

    const restartButton = screen.getByRole("button", { name: "Restart runtime" });
    await user.click(restartButton);
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
    const restartButton = screen.getByRole("button", { name: "Restart runtime" });
    vi.spyOn(restartButton, "getClientRects").mockReturnValue([{}] as unknown as DOMRectList);

    await user.click(restartButton);
    const dialog = await screen.findByRole("alertdialog", { name: "Restart the runtime?" });
    expect(dialog.textContent).not.toMatch(/keep-current-value|recipe\.one\/token/);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(mockApi.restartRuntime).not.toHaveBeenCalled();
    expect(input.value).toBe("keep-current-value");
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(restartButton));
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

    await user.click(screen.getByRole("button", { name: "Restart runtime" }));

    expect(screen.queryByRole("alertdialog", { name: "Restart the runtime?" })).toBeNull();
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
      message: "The Rust runtime could not restart. Try again.",
    }));
    await advanceToInputs(user);
    const input = screen.getByLabelText("Theme") as HTMLInputElement;
    await user.type(input, "dark");

    await user.click(screen.getByRole("button", { name: "Restart runtime" }));

    expect(await screen.findAllByText("The Rust runtime could not restart. Try again.")).not.toHaveLength(0);
    expect(input.value).toBe("dark");
    expect(screen.getByRole("heading", { name: "Provide required files and options" })).toBeTruthy();
    await waitFor(() => expect(
      (screen.getByRole("button", { name: "Restart runtime" }) as HTMLButtonElement).disabled,
    ).toBe(false));
    expect(document.body.textContent).not.toMatch(/runtime_restart_failed/);
  });
});

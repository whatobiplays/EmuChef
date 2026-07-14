import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { api } from "./api";
import { SupportPanel } from "./SupportPanel";
import { AccessibleDialog } from "./AccessibleDialog";
import {
  DialogController,
  claimFocusTransition,
  describedBy,
  executionAnnouncement,
  lifecycleBoundResult,
  restoreAccessibleFocus,
  stableDomId,
  type DialogSnapshot,
} from "./accessibility";
import type {
  AdbSetupStatus,
  AnyExecutionSnapshot,
  CatalogSummary,
  ConfigurationDescription,
  DeviceSummary,
  InputDescriptor,
  RecentConfiguration,
  RuntimeStatus,
  SavedConfigurationDocument,
  SavedConfigurationMutation,
  CacheCleanupMode,
} from "./types";
import {
  formatLastOpened,
  resolveUnsavedDecision,
  savedDevicePlanAvailable,
} from "./savedConfigurations";
import {
  emptyRealExecutionConfirmation,
  realExecutionConfirmationComplete,
} from "./realExecution";
import {
  initialWorkflowState,
  filterRepairBindings,
  inputDiagnosticsForDisplay,
  pageDiagnosticsForDisplay,
  recipeSelectionDisabled,
  reviewReady,
  runBusyAction,
  updateRecipeSelection,
  workflowReducer,
} from "./workflow";
import {
  cleanupConfirmation,
  entriesForCleanup,
  initialSupportState,
  supportReducer,
} from "./support";

const WORKFLOW_STEPS = [
  { step: "connect", label: "Connect" },
  { step: "device", label: "Device" },
  { step: "setup", label: "Setup" },
  { step: "inputs", label: "Inputs" },
  { step: "review", label: "Review" },
  { step: "execution", label: "Simulated Run" },
] as const;

type UnsavedDecision = "save" | "discard" | "cancel";

type AppDialogPayload =
  | {
      kind: "unsaved";
      invoker: HTMLElement | null;
    }
  | {
      kind: "name";
      title: string;
      initialValue: string;
      invoker: HTMLElement | null;
    }
  | {
      kind: "real-execution";
      invoker: HTMLElement | null;
    };

type AppDialogResult = UnsavedDecision | string | boolean | null;
export type AppDialogController = DialogController<AppDialogPayload, AppDialogResult>;

export function createAppDialogController(): AppDialogController {
  return new DialogController<AppDialogPayload, AppDialogResult>();
}

interface AppProps {
  dialogController?: AppDialogController;
}

function errorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { message?: unknown };
    return typeof parsed.message === "string" ? parsed.message : raw;
  } catch {
    return raw;
  }
}

function errorCode(error: unknown): string | null {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { code?: unknown };
    return typeof parsed.code === "string" ? parsed.code : null;
  } catch {
    return null;
  }
}

function executionDuration(snapshot: AnyExecutionSnapshot): string | null {
  if (!snapshot.startedAt) return null;
  const start = Date.parse(snapshot.startedAt);
  const finish = snapshot.finishedAt ? Date.parse(snapshot.finishedAt) : Date.now();
  if (!Number.isFinite(start) || !Number.isFinite(finish) || finish < start) return null;
  return `${Math.max(0, Math.round((finish - start) / 1000))}s`;
}

export function App({ dialogController: suppliedDialogController }: AppProps = {}) {
  const ownedDialogControllerRef = useRef<AppDialogController | null>(null);
  if (!ownedDialogControllerRef.current) {
    ownedDialogControllerRef.current = suppliedDialogController ?? createAppDialogController();
  }
  const dialogController = ownedDialogControllerRef.current;
  const [runtime, setRuntime] = useState<RuntimeStatus>({ status: "starting" });
  const [catalog, setCatalog] = useState<CatalogSummary | null>(null);
  const [adb, setAdb] = useState<AdbSetupStatus | null>(null);
  const [devices, setDevices] = useState<DeviceSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [realExecutionEnabled, setRealExecutionEnabled] = useState(false);
  const [realConfirmation, setRealConfirmation] = useState(emptyRealExecutionConfirmation);
  const [reportState, setReportState] = useState<"idle" | "exporting" | "saved" | "failed">("idle");
  const [launchState, setLaunchState] = useState<"idle" | "launching" | "launched" | "failed">("idle");
  const [repairPreparing, setRepairPreparing] = useState(false);
  const [savedConfiguration, setSavedConfiguration] = useState<SavedConfigurationDocument | null>(null);
  const [recentConfigurations, setRecentConfigurations] = useState<RecentConfiguration[]>([]);
  const [workflow, dispatch] = useReducer(workflowReducer, initialWorkflowState);
  const [support, supportDispatch] = useReducer(supportReducer, initialSupportState);
  const [activeDialog, setActiveDialog] = useState<DialogSnapshot<AppDialogPayload> | null>(
    dialogController.snapshot,
  );
  const [namePromptValue, setNamePromptValue] = useState("");
  const [politeAnnouncement, setPoliteAnnouncement] = useState({ id: 0, text: "" });
  const [assertiveAnnouncement, setAssertiveAnnouncement] = useState({ id: 0, text: "" });
  const mainRef = useRef<HTMLElement>(null);
  const supportInvokerRef = useRef<HTMLElement | null>(null);
  const validationSummaryRef = useRef<HTMLElement>(null);
  const executionAnnouncementKeyRef = useRef<string | null>(null);
  const appLifecycleGenerationRef = useRef(0);
  const savedConfigurationRef = useRef<SavedConfigurationDocument | null>(null);
  const workflowRef = useRef(workflow);
  const configurationMutationQueue = useRef<Promise<void>>(Promise.resolve());
  const sessionInitializedRef = useRef(false);
  const supportGenerationRef = useRef(0);
  const allowWindowCloseRef = useRef(false);

  const announce = useCallback((text: string, assertive = false) => {
    const update = (previous: { id: number; text: string }) => ({ id: previous.id + 1, text });
    if (assertive) setAssertiveAnnouncement(update);
    else setPoliteAnnouncement(update);
  }, []);

  useEffect(() => {
    const unsubscribe = dialogController.subscribe((snapshot) => {
      setActiveDialog(snapshot);
      if (snapshot?.payload.kind === "name") setNamePromptValue(snapshot.payload.initialValue);
    });
    return () => {
      appLifecycleGenerationRef.current += 1;
      unsubscribe();
      dialogController.cancelActive();
    };
  }, [dialogController]);

  const cancelPendingDialog = useCallback(() => {
    appLifecycleGenerationRef.current += 1;
    dialogController.cancelActive();
    setRealConfirmation(emptyRealExecutionConfirmation);
  }, [dialogController]);

  const requestAppDialog = useCallback(
    async (payload: AppDialogPayload, safeResult: AppDialogResult): Promise<AppDialogResult> => {
      const lifecycleGeneration = appLifecycleGenerationRef.current;
      const request = dialogController.request(payload, safeResult);
      if (!request.accepted) {
        announce("Another confirmation is already open. The new request was cancelled.");
      }
      return lifecycleBoundResult(
        request.result,
        safeResult,
        lifecycleGeneration,
        () => appLifecycleGenerationRef.current,
      );
    },
    [announce, dialogController],
  );

  const withNativeDialogFocus = useCallback(async <Result,>(
    action: () => Promise<Result>,
    preferred: Array<HTMLElement | null | undefined> = [],
  ): Promise<Result> => {
    const invoker = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const generation = claimFocusTransition();
    try {
      return await action();
    } finally {
      queueMicrotask(() => restoreAccessibleFocus({ invoker, preferred, generation }));
    }
  }, []);

  const initialize = useCallback(async () => {
    const [runtimeStatus, adbStatus, realAvailability] = await Promise.all([
      api.runtimeStatus(),
      api.adbStatus(),
      api.realExecutionAvailability().catch(() => ({ enabled: false })),
    ]);
    setRuntime(runtimeStatus);
    setAdb(adbStatus);
    setRealExecutionEnabled(realAvailability.enabled);
    if (runtimeStatus.status === "ready") {
      const [nextCatalog, recents] = await Promise.all([
        api.catalog(),
        api.listRecentConfigurations(),
      ]);
      setCatalog(nextCatalog);
      setRecentConfigurations(recents);
    }
  }, []);

  useEffect(() => {
    if (sessionInitializedRef.current) return;
    sessionInitializedRef.current = true;
    api.beginAppSession().then(initialize).catch((error) => {
      setRuntime({
        status: "failed",
        error: { code: "runtime_start_failed", message: errorMessage(error), actions: ["retry"] },
      });
    });
  }, [initialize]);

  useEffect(() => {
    savedConfigurationRef.current = savedConfiguration;
  }, [savedConfiguration]);

  useEffect(() => {
    workflowRef.current = workflow;
  }, [workflow]);

  useEffect(() => {
    const generation = claimFocusTransition();
    queueMicrotask(() => {
      const destination = mainRef.current?.querySelector<HTMLElement>("[data-step-heading]");
      restoreAccessibleFocus({
        preferred: [destination, mainRef.current],
        generation,
      });
    });
  }, [adb?.status, runtime.status, workflow.step]);

  useEffect(() => {
    if (!notice) return;
    announce(notice, /failed|error|could not|unavailable|disconnected/i.test(notice));
  }, [announce, notice]);

  useEffect(() => {
    if (activeDialog?.payload.kind === "real-execution" && !workflow.review) {
      dialogController.cancelActive();
    }
  }, [activeDialog?.id, activeDialog?.payload.kind, dialogController, workflow.review]);

  useEffect(() => {
    if (!savedConfiguration?.configurationHandle) return;
    const generation = claimFocusTransition();
    queueMicrotask(() => restoreAccessibleFocus({
      preferred: [mainRef.current?.querySelector<HTMLElement>("[data-step-heading]")],
      generation,
    }));
  }, [savedConfiguration?.configurationHandle]);

  const refreshRecents = async () => {
    setRecentConfigurations(await api.listRecentConfigurations());
  };

  const applySavedDocument = async (document: SavedConfigurationDocument) => {
    cancelPendingDialog();
    const previous = savedConfigurationRef.current;
    if (previous && previous.configurationHandle !== document.configurationHandle) {
      await api.closeSavedConfiguration(previous.configurationHandle).catch(() => undefined);
    }
    savedConfigurationRef.current = document;
    setSavedConfiguration(document);
    dispatch({
      type: "load-portable-intent",
      devicePlan: document.devicePlan,
      selectedRecipes: document.selectedRecipes,
      bindings: document.bindings,
      dirty: document.dirty,
    });
    await refreshRecents();
    setNotice(`Opened ${document.name}. Connect and select the current device to validate it.`);
  };

  const createFromCurrentIntent = async (): Promise<boolean> => {
    const current = workflowRef.current;
    if (!current.devicePlan) {
      setNotice("Choose a device plan reference before saving this portable configuration.");
      return false;
    }
    const nameResult = await requestAppDialog({
      kind: "name",
      title: "Name this portable configuration",
      initialValue: "My EmuChef setup",
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, null);
    const name = typeof nameResult === "string" ? nameResult.trim() : "";
    if (!name) return false;
    try {
      const result = await withNativeDialogFocus(() => api.createSavedConfiguration({
          name,
          devicePlan: current.devicePlan!,
          selectedRecipes: current.selectedRecipes ?? [],
          bindings: current.bindings,
        }));
      if (result.outcome === "cancelled") return false;
      await applySavedDocument(result);
      dispatch({ type: "portable-intent-saved" });
      return true;
    } catch (error) {
      setNotice(errorMessage(error));
      return false;
    }
  };

  const saveCurrentConfiguration = async (): Promise<boolean> => {
    await configurationMutationQueue.current;
    const current = savedConfigurationRef.current;
    if (!current) return createFromCurrentIntent();
    try {
      const saved = await api.saveSavedConfiguration(current.configurationHandle);
      savedConfigurationRef.current = saved;
      setSavedConfiguration(saved);
      dispatch({ type: "portable-intent-saved" });
      await refreshRecents();
      setNotice(`Saved ${saved.name}.`);
      return true;
    } catch (error) {
      setNotice(errorMessage(error));
      return false;
    }
  };

  const dirtyDecision = async (): Promise<"save" | "discard" | "cancel"> => {
    const dirty = workflowRef.current.portableIntentDirty || Boolean(savedConfigurationRef.current?.dirty);
    if (!dirty) return "discard";
    const result = await requestAppDialog({
      kind: "unsaved",
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, "cancel");
    if (result !== "save" && result !== "discard") return "cancel";
    return resolveUnsavedDecision(dirty, result === "save", result === "discard");
  };

  const runProtectedTransition = async (transition: () => Promise<void>) => {
    const decision = await dirtyDecision();
    if (decision === "cancel") return;
    if (decision === "save" && !(await saveCurrentConfiguration())) return;
    await configurationMutationQueue.current;
    await transition();
  };

  const startNewConfiguration = async () => {
    await runProtectedTransition(async () => {
      cancelPendingDialog();
      const current = savedConfigurationRef.current;
      if (current) await api.closeSavedConfiguration(current.configurationHandle).catch(() => undefined);
      savedConfigurationRef.current = null;
      setSavedConfiguration(null);
      dispatch({ type: "runtime-invalidated" });
      setNotice("Started a new portable configuration.");
    });
  };

  const openConfiguration = async () => {
    await runProtectedTransition(async () => {
      try {
        const result = await withNativeDialogFocus(api.openSavedConfiguration);
        if (result.outcome !== "cancelled") await applySavedDocument(result);
      } catch (error) {
        setNotice(errorMessage(error));
      }
    });
  };

  const openRecentConfiguration = async (recentHandle: string) => {
    await runProtectedTransition(async () => {
      try {
        await applySavedDocument(await api.openRecentConfiguration(recentHandle));
      } catch (error) {
        setNotice(errorMessage(error));
        await refreshRecents();
      }
    });
  };

  const relinkRecentConfiguration = async (recentHandle: string) => {
    await runProtectedTransition(async () => {
      try {
        const result = await withNativeDialogFocus(() => api.relinkRecentConfiguration(recentHandle));
        if (result.outcome !== "cancelled") await applySavedDocument(result);
      } catch (error) {
        setNotice(errorMessage(error));
      } finally {
        await refreshRecents();
      }
    });
  };

  const saveConfigurationAs = async () => {
    await configurationMutationQueue.current;
    const current = savedConfigurationRef.current;
    if (!current) {
      await createFromCurrentIntent();
      return;
    }
    const nameResult = await requestAppDialog({
      kind: "name",
      title: "Name the new portable configuration",
      initialValue: current.name,
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, null);
    const name = typeof nameResult === "string" ? nameResult.trim() : "";
    if (!name) return;
    try {
      const result = await withNativeDialogFocus(
        () => api.saveSavedConfigurationAs(current.configurationHandle, name),
      );
      if (result.outcome === "cancelled") return;
      savedConfigurationRef.current = result;
      setSavedConfiguration(result);
      dispatch({ type: "portable-intent-saved" });
      await refreshRecents();
      setNotice(`Saved the new portable configuration ${result.name}.`);
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  const queueSavedMutation = (mutation: SavedConfigurationMutation) => {
    if (!savedConfigurationRef.current) return;
    configurationMutationQueue.current = configurationMutationQueue.current
      .then(async () => {
        const current = savedConfigurationRef.current;
        if (!current) return;
        const updated = await api.updateSavedConfiguration(
          current.configurationHandle,
          current.revision,
          mutation,
        );
        if (savedConfigurationRef.current?.configurationHandle !== updated.configurationHandle) return;
        savedConfigurationRef.current = updated;
        setSavedConfiguration(updated);
      })
      .catch((error) => setNotice(errorMessage(error)));
  };

  const updateDevicePlanIntent = (devicePlan: string) => {
    dispatch({ type: "select-plan", devicePlan });
    queueSavedMutation({ kind: "device_plan", value: devicePlan });
  };

  const updateRecipeIntent = (selectedRecipes: string[]) => {
    dispatch({ type: "set-recipes", selectedRecipes });
    queueSavedMutation({ kind: "selected_recipes", value: selectedRecipes });
  };

  const updateBindingIntent = (key: string, value: unknown) => {
    dispatch({ type: "set-binding", key, value });
    queueSavedMutation({ kind: "binding", key, value });
  };

  const applyDescriptionValidation = (description: ConfigurationDescription) => {
    const current = savedConfigurationRef.current;
    if (!current) return;
    const diagnostics = [
      ...description.diagnostics,
      ...description.inputs.flatMap((input) => input.diagnostics),
    ];
    const state = diagnostics.some((diagnostic) => diagnostic.severity === "error")
      ? "requires_attention"
      : diagnostics.length > 0
        ? "valid_with_warnings"
        : "valid";
    const updated: SavedConfigurationDocument = {
      ...current,
      validation: { state, diagnostics },
    };
    savedConfigurationRef.current = updated;
    setSavedConfiguration(updated);
  };

  const focusValidationSummary = (description: ConfigurationDescription) => {
    const hasErrors = [
      ...description.diagnostics,
      ...description.inputs.flatMap((input) => input.diagnostics),
    ].some((diagnostic) => diagnostic.severity === "error");
    if (!hasErrors) return;
    const generation = claimFocusTransition();
    queueMicrotask(() => restoreAccessibleFocus({
      preferred: [validationSummaryRef.current],
      generation,
    }));
  };

  const syncSavedIntent = (
    devicePlan: string,
    selectedRecipes: string[],
    bindings: Record<string, unknown>,
  ) => {
    const current = savedConfigurationRef.current;
    if (!current) return;
    if (current.devicePlan !== devicePlan) {
      queueSavedMutation({ kind: "device_plan", value: devicePlan });
    }
    if (JSON.stringify(current.selectedRecipes) !== JSON.stringify(selectedRecipes)) {
      queueSavedMutation({ kind: "selected_recipes", value: selectedRecipes });
    }
    for (const key of Object.keys(current.bindings)) {
      if (!Object.hasOwn(bindings, key)) {
        queueSavedMutation({ kind: "remove_binding", key });
      }
    }
    for (const [key, value] of Object.entries(bindings)) {
      if (!Object.hasOwn(current.bindings, key) || current.bindings[key] !== value) {
        queueSavedMutation({ kind: "binding", key, value });
      }
    }
  };

  const restartRuntime = async () => {
    await runProtectedTransition(async () => {
      try {
        cancelPendingDialog();
        announce("Restarting the Rust runtime.");
        const status = await api.restartRuntime();
        savedConfigurationRef.current = null;
        setSavedConfiguration(null);
        dispatch({ type: "runtime-invalidated" });
        setRuntime(status);
        supportDispatch({ type: "runtime-restarted" });
        await initialize();
        if (support.open) await refreshSupportInventory();
        setNotice("Rust runtime restarted. Reopen a portable configuration before continuing.");
      } catch (error) {
        setNotice(errorMessage(error));
      }
    });
  };

  const refreshSupportInventory = async () => {
    const generation = ++supportGenerationRef.current;
    supportDispatch({ type: "inventory-requested", generation });
    announce("Refreshing the app-owned artifact cache inventory.");
    try {
      const inventory = await api.cacheInventory();
      supportDispatch({ type: "inventory-loaded", generation, inventory });
      announce(`Cache inventory refreshed. ${inventory.summary.entryCount} entries available.`);
    } catch (error) {
      supportDispatch({ type: "inventory-failed", generation, message: errorMessage(error) });
    }
  };

  const openSupport = (invoker: HTMLElement) => {
    if (dialogController.activeId !== null) {
      announce("Close the current confirmation before opening Support and Storage.");
      return;
    }
    supportInvokerRef.current = invoker;
    supportDispatch({ type: "open" });
    void refreshSupportInventory();
  };

  const prepareSupportCleanup = (mode: CacheCleanupMode) => {
    if (!support.inventory) return null;
    const entries = entriesForCleanup(support.inventory, mode, support.selectedHandles);
    const confirmation = cleanupConfirmation(entries);
    if (confirmation.entryCount === 0) {
      supportDispatch({
        type: "cleanup-failed",
        message: "No removable cache entries match this action.",
      });
      announce("No removable cache entries match this action.", true);
      return null;
    }
    return confirmation;
  };

  const cleanupSupportCache = async (mode: CacheCleanupMode) => {
    if (!support.inventory) return;
    const entries = entriesForCleanup(support.inventory, mode, support.selectedHandles);
    const confirmation = cleanupConfirmation(entries);
    if (confirmation.entryCount === 0) return;
    const generation = ++supportGenerationRef.current;
    supportDispatch({ type: "cleanup-started", generation });
    announce(`Removing ${confirmation.entryCount} confirmed cache ${confirmation.entryCount === 1 ? "entry" : "entries"}.`);
    try {
      const result = await api.cleanupCache({
        mode,
        inventoryGeneration: support.inventory.generation,
        entryHandles: mode === "selected" ? support.selectedHandles : [],
        confirmation: { confirmed: true, ...confirmation },
      });
      supportDispatch({
        type: "cleanup-finished",
        generation,
        inventory: result.inventory,
        outcomes: result.outcomes,
      });
      announce(`Cache cleanup finished. ${result.outcomes.length} outcomes are available.`);
    } catch (error) {
      supportDispatch({ type: "cleanup-failed", message: errorMessage(error) });
    }
  };

  const exportSupportDiagnostics = async () => {
    const generation = ++supportGenerationRef.current;
    supportDispatch({ type: "export-started", generation });
    announce("Preparing a sanitized diagnostics export.");
    try {
      const result = await withNativeDialogFocus(api.exportSupportDiagnostics);
      supportDispatch({ type: "export-finished", generation, outcome: result.outcome });
      announce(result.outcome === "saved" ? "Sanitized diagnostics saved." : "Diagnostics export cancelled.");
    } catch (error) {
      supportDispatch({ type: "export-failed", generation, message: errorMessage(error) });
    }
  };

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;
    const windowHandle = getCurrentWindow();
    void windowHandle.onCloseRequested(async (event) => {
      if (allowWindowCloseRef.current) {
        allowWindowCloseRef.current = false;
        return;
      }
      const dirty = workflowRef.current.portableIntentDirty
        || Boolean(savedConfigurationRef.current?.dirty);
      if (!dirty) return;
      event.preventDefault();
      const decision = await dirtyDecision();
      if (decision === "cancel") return;
      if (decision === "save" && !(await saveCurrentConfiguration())) return;
      allowWindowCloseRef.current = true;
      await windowHandle.close();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else cleanup = unlisten;
    });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  const pollDevices = useCallback(async () => {
    if (adb?.status !== "ready" || runtime.status !== "ready") return;
    try {
      const next = await api.pollDevices();
      setDevices(next);
      if (workflow.deviceHandle && !next.some((device) => device.deviceHandle === workflow.deviceHandle)) {
        dispatch({ type: "device-disappeared" });
        setNotice("The selected device disconnected. Connect it again to continue.");
      }
    } catch (error) {
      setNotice(errorMessage(error));
    }
  }, [adb?.status, runtime.status, workflow.deviceHandle]);

  useEffect(() => {
    pollDevices();
    const timer = window.setInterval(pollDevices, 2500);
    return () => window.clearInterval(timer);
  }, [pollDevices]);

  const importPlatformTools = async () => {
    setNotice(null);
    await runBusyAction({
      setBusy,
      action: () => withNativeDialogFocus(api.importPlatformTools),
      onSuccess: (status) => {
        setAdb(status);
        if (workflowRef.current.deviceHandle || workflowRef.current.review) {
          setDevices([]);
          dispatch({ type: "infrastructure-invalidated" });
          setNotice(savedConfigurationRef.current?.dirty
            ? "Platform-Tools replaced. Unsaved configuration edits remain open; select and validate a device again."
            : "Platform-Tools replaced. Device, review, and execution authority were invalidated.");
        }
      },
      onError: async (error) => {
        setNotice(errorMessage(error));
        setAdb(await api.adbStatus());
      },
    });
  };

  const openPlatformToolsPage = async () => {
    setNotice(null);
    try {
      await api.openPlatformToolsPage();
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  const removePlatformTools = async () => {
    setBusy(true);
    try {
      setAdb(await api.removePlatformTools());
      setDevices([]);
      dispatch({ type: "infrastructure-invalidated" });
      setNotice(savedConfigurationRef.current?.dirty
        ? "Platform-Tools removed. Unsaved configuration edits remain open; reconnect after reinstalling Platform-Tools."
        : "Platform-Tools removed. Device, review, and execution authority were invalidated.");
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const selectDevice = async (deviceHandle: string) => {
    dispatch({ type: "select-device", deviceHandle });
    setBusy(true);
    setNotice(null);
    announce("Reading the selected device properties.");
    try {
      const facts = await api.probeDevice(deviceHandle);
      const match = await api.matchDevice(deviceHandle);
      dispatch({ type: "device-probed", facts, match });
      announce("Device properties loaded. Confirm the matched setup.");
      const currentSaved = savedConfigurationRef.current;
      if (currentSaved && !savedDevicePlanAvailable(currentSaved, match)) {
        const updated: SavedConfigurationDocument = {
          ...currentSaved,
          validation: {
            state: "cannot_use",
            diagnostics: [{
              code: "saved_device_plan_incompatible",
              message: "The saved device plan reference is unavailable or incompatible with the current device.",
              severity: "error",
              key: null,
            }, ...currentSaved.validation.diagnostics],
          },
        };
        savedConfigurationRef.current = updated;
        setSavedConfiguration(updated);
      }
    } catch (error) {
      setNotice(errorMessage(error));
      dispatch({ type: "device-disappeared" });
    } finally {
      setBusy(false);
    }
  };

  const describe = async () => {
    if (!workflow.deviceHandle || !workflow.devicePlan) return;
    setBusy(true);
    setNotice(null);
    setOperationError(null);
    try {
      const generation = workflow.requestGeneration;
      const description = await api.describeConfiguration({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
      });
      dispatch({ type: "description", description, generation });
      if (workflowRef.current.requestGeneration === generation) {
        applyDescriptionValidation(description);
        focusValidationSummary(description);
        const errorCount = [
          ...description.diagnostics,
          ...description.inputs.flatMap((input) => input.diagnostics),
        ].filter((item) => item.severity === "error").length;
        announce(errorCount > 0
          ? `Validation needs attention. ${errorCount} ${errorCount === 1 ? "error" : "errors"} found.`
          : "Validation complete. The configuration is ready for review.", errorCount > 0);
      } else {
        announce("An outdated validation response was ignored.");
      }
    } catch (error) {
      const message = errorMessage(error);
      setNotice(message);
      setOperationError(message);
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({ preferred: [validationSummaryRef.current], generation }));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (
      workflow.step !== "inputs" ||
      !workflow.descriptionDirty ||
      !workflow.deviceHandle ||
      !workflow.devicePlan
    ) return;
    const generation = workflow.requestGeneration;
    const timer = window.setTimeout(() => {
      api.describeConfiguration({
        deviceHandle: workflow.deviceHandle!,
        devicePlan: workflow.devicePlan!,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
      }).then((description) => {
        dispatch({ type: "description", description, generation });
        if (workflowRef.current.requestGeneration === generation) {
          applyDescriptionValidation(description);
        } else {
          announce("An outdated validation response was ignored.");
        }
      }).catch((error) => setNotice(errorMessage(error)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [
    workflow.bindings,
    workflow.descriptionDirty,
    workflow.deviceHandle,
    workflow.devicePlan,
    workflow.requestGeneration,
    workflow.selectedRecipes,
    workflow.step,
  ]);

  const generateReview = async () => {
    if (!workflow.deviceHandle || !workflow.devicePlan) return;
    setBusy(true);
    setNotice(null);
    setOperationError(null);
    announce("Creating a fresh reviewed plan.");
    try {
      const review = await api.createReview({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
      });
      dispatch({ type: "review", review });
      announce("The reviewed plan is ready.");
    } catch (error) {
      const message = errorMessage(error);
      setNotice(message);
      setOperationError(message);
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({
        preferred: [validationSummaryRef.current],
        generation,
      }));
    } finally {
      setBusy(false);
    }
  };

  const startSimulation = async () => {
    if (!workflow.review || workflow.execution.kind === "starting") return;
    const generation = workflow.executionGeneration + 1;
    dispatch({ type: "execution-starting", generation });
    setBusy(true);
    setNotice(null);
    announce("Starting the simulated dry run.");
    try {
      const snapshot = await api.startSimulatedExecution(workflow.review.reviewHandle);
      dispatch({ type: "execution-started", generation, snapshot });
    } catch (error) {
      dispatch({ type: "execution-start-failed", generation });
      setNotice(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const startRealExecution = async () => {
    if (!realExecutionEnabled || !workflow.review || workflow.execution.kind === "starting") return;
    const generation = workflow.executionGeneration + 1;
    const confirmation = realConfirmation;
    dispatch({ type: "execution-starting", generation, mode: "real" });
    setBusy(true);
    setNotice(null);
    setRealConfirmation(emptyRealExecutionConfirmation);
    try {
      const snapshot = await api.startRealExecution(workflow.review.reviewHandle, confirmation);
      dispatch({ type: "execution-started", generation, snapshot });
    } catch (error) {
      dispatch({ type: "execution-start-failed", generation });
      setNotice(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const requestRealExecution = async (invoker: HTMLElement) => {
    setRealConfirmation(emptyRealExecutionConfirmation);
    const result = await requestAppDialog({
      kind: "real-execution",
      invoker,
    }, false);
    if (result === true) await startRealExecution();
  };

  const activeExecution =
    workflow.execution.kind === "active" ? workflow.execution : null;

  useEffect(() => {
    if (workflow.execution.kind !== "active" && workflow.execution.kind !== "terminal") return;
    const next = executionAnnouncement(
      workflow.execution.snapshot,
      executionAnnouncementKeyRef.current,
    );
    if (!next) return;
    executionAnnouncementKeyRef.current = next.key;
    announce(next.message, next.assertive);
    if (workflow.execution.kind === "terminal") {
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({
        preferred: [mainRef.current?.querySelector<HTMLElement>("[data-step-heading]")],
        generation,
      }));
    }
  }, [announce, workflow.execution]);

  useEffect(() => {
    if (!activeExecution) return;
    let disposed = false;
    let timer: number | null = null;
    const { generation, snapshot, mode } = activeExecution;
    const executionHandle = snapshot.executionHandle;
    let eventCursor = activeExecution.eventCursor;

    async function pollExecution() {
      try {
        const nextSnapshot = mode === "real"
          ? await api.getRealExecution(executionHandle)
          : await api.getSimulatedExecution(executionHandle);
        if (disposed) return;
        const currentExecution = workflowRef.current.execution;
        if (
          workflowRef.current.executionGeneration !== generation
          || (currentExecution.kind !== "active" && currentExecution.kind !== "terminal")
          || currentExecution.snapshot.executionHandle !== executionHandle
        ) {
          announce("An outdated execution response was ignored.");
          return;
        }
        dispatch({ type: "execution-snapshot", generation, snapshot: nextSnapshot });
        eventCursor = Math.max(eventCursor, nextSnapshot.latestSequence);
        if (nextSnapshot.terminal) return;

        const batch = mode === "real"
          ? await api.getRealExecutionEvents(executionHandle, eventCursor)
          : await api.getSimulatedExecutionEvents(executionHandle, eventCursor);
        if (disposed) return;
        dispatch({ type: "execution-events", generation, batch });
        for (const event of batch.events) eventCursor = Math.max(eventCursor, event.sequence);
        timer = window.setTimeout(pollExecution, 500);
      } catch (error) {
        if (disposed) return;
        if (errorCode(error) === "execution_unavailable") {
          dispatch({
            type: "execution-unavailable",
            generation,
            executionHandle,
            message: errorMessage(error),
          });
          return;
        }
        setNotice(errorMessage(error));
        timer = window.setTimeout(pollExecution, 1000);
      }
    }

    void pollExecution();
    return () => {
      disposed = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [activeExecution?.generation, activeExecution?.snapshot.executionHandle, activeExecution?.mode]);

  const cancelExecution = async () => {
    if (workflow.execution.kind !== "active" || workflow.execution.cancellationRequested) return;
    const { generation, snapshot } = workflow.execution;
    try {
      const cancellation = workflow.execution.mode === "real"
        ? await api.cancelRealExecution(snapshot.executionHandle)
        : await api.cancelSimulatedExecution(snapshot.executionHandle);
      if (cancellation.accepted) {
        dispatch({ type: "execution-cancellation-requested", generation });
      }
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  const exportExecutionReport = async () => {
    if (workflow.execution.kind !== "terminal") return;
    const executionHandle = workflow.execution.snapshot.executionHandle;
    setReportState("exporting");
    setNotice(null);
    try {
      const result = await withNativeDialogFocus(
        () => api.exportExecutionReport(executionHandle),
      );
      setReportState(result.outcome === "saved" ? "saved" : "idle");
    } catch (error) {
      setReportState("failed");
      setNotice(errorMessage(error));
    }
  };

  const launchConfiguredApp = async () => {
    if (
      workflow.execution.kind !== "terminal"
      || workflow.execution.snapshot.simulated
      || !workflow.execution.snapshot.launchAction
    ) return;
    const { generation, snapshot } = workflow.execution;
    const launchAction = snapshot.launchAction;
    if (!launchAction) return;
    const consumedHandle = launchAction.handle;
    setLaunchState("launching");
    setNotice(null);
    try {
      const result = await api.launchConfiguredApp(consumedHandle);
      setLaunchState("launched");
      setNotice(result.message);
    } catch (error) {
      setLaunchState("failed");
      setNotice(errorMessage(error));
      try {
        const refreshed = await api.getRealExecution(snapshot.executionHandle);
        dispatch({ type: "execution-snapshot", generation, snapshot: refreshed });
      } catch {
        // The original sanitized launch error remains the useful result. A lost
        // execution cannot mint another action and is handled by normal polling.
      }
    }
  };

  const prepareRepair = async () => {
    if (repairPreparing) return;
    const prior = workflow;
    setRepairPreparing(true);
    setReportState("idle");
    setLaunchState("idle");
    cancelPendingDialog();
    setRealConfirmation(emptyRealExecutionConfirmation);
    dispatch({ type: "prepare-repair" });
    setNotice(null);
    try {
      if (prior.review) {
        await api.discardReview(prior.review.reviewHandle).catch(() => undefined);
      }
      const [freshCatalog, freshDevices] = await Promise.all([api.catalog(), api.pollDevices()]);
      setCatalog(freshCatalog);
      setDevices(freshDevices);
      if (!prior.deviceHandle || !freshDevices.some((device) => device.deviceHandle === prior.deviceHandle)) {
        setNotice("Reconnect the intended device to continue the fresh repair flow.");
        return;
      }
      const [facts, match] = await Promise.all([
        api.probeDevice(prior.deviceHandle),
        api.matchDevice(prior.deviceHandle),
      ]);
      const plans = [...match.candidates, ...match.safeGenericPlans];
      const devicePlan = prior.devicePlan && plans.some((plan) => plan.planId === prior.devicePlan)
        ? prior.devicePlan
        : match.recommendedPlanId;
      if (!devicePlan) {
        dispatch({ type: "select-device", deviceHandle: prior.deviceHandle });
        dispatch({ type: "device-probed", facts, match });
        setNotice("Choose a current device plan before regenerating this configuration.");
        return;
      }
      const catalogRecipes = new Set(freshCatalog.recipes.map((recipe) => recipe.id));
      const selectedRecipes = (prior.selectedRecipes ?? []).filter((recipe) => catalogRecipes.has(recipe));
      const baseline = await api.describeConfiguration({
        deviceHandle: prior.deviceHandle,
        devicePlan,
        selectedRecipes,
        bindings: {},
      });
      const bindings = filterRepairBindings(prior.description, baseline, prior.bindings);
      const description = await api.describeConfiguration({
        deviceHandle: prior.deviceHandle,
        devicePlan,
        selectedRecipes,
        bindings,
      });
      dispatch({
        type: "repair-ready",
        facts,
        match,
        devicePlan,
        description,
        selectedRecipes: description.selectedRecipes,
        bindings,
      });
      syncSavedIntent(devicePlan, description.selectedRecipes, bindings);
      setNotice("Configuration refreshed. Resolve any diagnostics, then create and review a new plan.");
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setRepairPreparing(false);
    }
  };

  const pickInputValue = async (input: InputDescriptor) => {
    if (!input.pathKind) return;
    setNotice(null);
    await runBusyAction({
      setBusy,
      action: () => withNativeDialogFocus(
        () => api.pickInputPath(input.pathKind!, Boolean(input.multiple)),
        [document.getElementById(stableDomId("input", input.key))],
      ),
      onSuccess: (values) => {
        if (values) {
          updateBindingIntent(input.key, input.multiple ? values : values[0]);
        }
      },
      onError: (error) => setNotice(errorMessage(error)),
    });
  };

  const stepIndex = WORKFLOW_STEPS.findIndex((item) => item.step === workflow.step);
  const planOptions = useMemo(
    () => [...(workflow.match?.candidates ?? []), ...(workflow.match?.safeGenericPlans ?? [])],
    [workflow.match],
  );
  const savedPlanUnavailable = Boolean(
    savedConfiguration
      && workflow.match
      && (!workflow.devicePlan
        || !planOptions.some((candidate) => candidate.planId === workflow.devicePlan)),
  );
  const validationErrors = workflow.description
    ? [
        ...workflow.description.inputs.flatMap((input) => inputDiagnosticsForDisplay(input).map((diagnostic) => ({
          ...diagnostic,
          targetId: stableDomId("input", input.key),
        }))),
        ...pageDiagnosticsForDisplay(workflow.description).map((diagnostic) => ({
          ...diagnostic,
          targetId: null,
        })),
      ].filter((diagnostic) => diagnostic.severity === "error")
    : [];
  const configurationActionsLocked = busy
    || workflow.execution.kind === "active"
    || workflow.execution.kind === "starting";
  const saveDisabled = busy
    || !workflow.devicePlan
    || (!workflow.portableIntentDirty && !savedConfiguration?.dirty);

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">Skip to main content</a>
      <div className="live-regions" aria-label="Application notifications">
        <p aria-atomic="true" aria-live="polite" className="visually-hidden" key={`polite-${politeAnnouncement.id}`} role="status">
          {politeAnnouncement.text}
        </p>
        <p aria-atomic="true" aria-live="assertive" className="visually-hidden" key={`assertive-${assertiveAnnouncement.id}`} role="alert">
          {assertiveAnnouncement.text}
        </p>
      </div>
      <header className="app-header">
        <div className="brand-mark" aria-hidden="true">E</div>
        <div>
          <p className="eyebrow">EMUCHEF</p>
          <h1>Prepare your Android handheld</h1>
        </div>
        <div className="runtime-chip" aria-live="polite">
          {runtime.status === "ready" ? `Runtime ready · ${catalog?.catalog.version ?? "catalog"}` : runtime.status}
        </div>
        <button
          className="secondary"
          data-focus-fallback="header"
          onClick={(event) => openSupport(event.currentTarget)}
        >Support & Storage</button>
      </header>

      <SupportPanel
        state={support}
        returnFocus={supportInvokerRef.current}
        onClose={() => supportDispatch({ type: "close" })}
        onRefresh={() => void refreshSupportInventory()}
        onToggleSelection={(handle) => supportDispatch({ type: "toggle-selection", handle })}
        onPrepareCleanup={prepareSupportCleanup}
        onCleanup={(mode) => void cleanupSupportCache(mode)}
        onExport={() => void exportSupportDiagnostics()}
        onAnnounce={announce}
      />

      {runtime.status === "ready" && (
        <section className="configuration-bar" aria-label="Saved configurations">
          <div>
            <strong>{savedConfiguration?.name ?? "Unsaved configuration"}</strong>
            <small>
              {savedConfiguration
                ? `${savedConfiguration.validation.state.replaceAll("_", " ")}${savedConfiguration.dirty ? " · unsaved edits" : ""}`
                : "Portable intent only; generated plans and device authority are never saved"}
            </small>
          </div>
          <div className="button-row">
            <button aria-describedby={configurationActionsLocked ? "configuration-actions-reason" : undefined} className="secondary" onClick={startNewConfiguration} disabled={configurationActionsLocked}>New</button>
            <button aria-describedby={configurationActionsLocked ? "configuration-actions-reason" : undefined} className="secondary" onClick={openConfiguration} disabled={configurationActionsLocked}>Open…</button>
            <button aria-describedby={saveDisabled ? "save-configuration-reason" : undefined} onClick={saveCurrentConfiguration} disabled={saveDisabled}>Save</button>
            <button aria-describedby={busy || !workflow.devicePlan ? "save-as-reason" : undefined} className="secondary" onClick={saveConfigurationAs} disabled={busy || !workflow.devicePlan}>Save As…</button>
            <button aria-describedby={configurationActionsLocked ? "configuration-actions-reason" : undefined} className="text-button" onClick={restartRuntime} disabled={configurationActionsLocked}>Restart runtime</button>
          </div>
          {configurationActionsLocked && <p className="disabled-reason" id="configuration-actions-reason">Configuration replacement and runtime restart are unavailable while another operation or execution is active.</p>}
          {saveDisabled && <p className="disabled-reason" id="save-configuration-reason">Save requires a selected device plan and unsaved portable changes.</p>}
          {(busy || !workflow.devicePlan) && <p className="disabled-reason" id="save-as-reason">Save As requires a selected device plan and no other active operation.</p>}
        </section>
      )}

      {runtime.status === "unsupported" || runtime.status === "failed" ? (
        <main className="blocking-card" data-focus-fallback="main" id="main-content" role="alert" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">RUNTIME UNAVAILABLE</p>
          <h2 data-step-heading tabIndex={-1}>EmuChef could not start its Rust runtime</h2>
          <p>{runtime.error.message}</p>
          <button onClick={restartRuntime}>Retry runtime startup</button>
        </main>
      ) : adb?.status !== "ready" ? (
        <main className="blocking-card" aria-labelledby="adb-heading" data-focus-fallback="main" id="main-content" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">ONE-TIME SETUP</p>
          <h2 data-step-heading id="adb-heading" tabIndex={-1}>Android SDK Platform-Tools is required</h2>
          <p>
            EmuChef does not include or download ADB. Download the macOS Platform-Tools ZIP directly
            from Google, then import it here for local validation and managed installation.
          </p>
          {adb?.warning && <p className="warning">{adb.warning}</p>}
          {(adb?.error || notice) && (
            <p className="error" role="alert">{adb?.error?.message ?? notice}</p>
          )}
          <div className="button-row">
            <button className="secondary" onClick={openPlatformToolsPage}>
              Open Android Platform-Tools Download Page
            </button>
            <button aria-describedby={busy ? "platform-tools-busy" : undefined} onClick={importPlatformTools} disabled={busy}>
              {busy ? "Validating…" : "Import Platform-Tools ZIP"}
            </button>
            {adb?.canRemove && <button className="danger" onClick={removePlatformTools}>Remove</button>}
          </div>
          {busy && <p className="disabled-reason" id="platform-tools-busy" role="status">Platform-Tools validation is in progress.</p>}
          <p className="fine-print">
            EmuChef keeps only adb, NOTICE.txt, and source.properties in its application data. The
            selected ZIP remains yours and is never copied into the app bundle or repository.
          </p>
        </main>
      ) : (
        <main className="workflow-layout" data-focus-fallback="main" id="main-content" ref={mainRef} tabIndex={-1}>
          <nav aria-label="Setup progress" className="progress-nav">
            <ol>
              {WORKFLOW_STEPS.map((item, index) => (
                <li
                  aria-current={index === stepIndex ? "step" : undefined}
                  key={item.step}
                  className={index === stepIndex ? "current" : index < stepIndex ? "complete" : ""}
                >
                  <span aria-hidden="true">{index + 1}</span>
                  <span className="visually-hidden">{index < stepIndex ? "Completed: " : index === stepIndex ? "Current step: " : "Upcoming: "}</span>
                  {item.label}
                </li>
              ))}
            </ol>
          </nav>

          <section className="workflow-card" aria-busy={busy}>
            {notice && !operationError && <p className="warning" role="status">Attention: {notice}</p>}
            {operationError && (
              <section className="error error-summary" ref={validationSummaryRef} role="alert" tabIndex={-1}>
                <h2>Action could not be completed</h2>
                <p>Error: {operationError}</p>
                <p>Review the current selections and try the action again.</p>
              </section>
            )}

            {workflow.step === "connect" && (
              <>
                <p className="eyebrow">CONNECT DEVICE</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Choose an Android device</h2>
                <p>Connect with USB debugging enabled. EmuChef only reads device information in this phase.</p>
                {recentConfigurations.length > 0 && (
                  <section className="recent-configurations" aria-labelledby="recent-configurations-heading">
                    <h3 id="recent-configurations-heading">Recent configurations</h3>
                    <ul>
                    {recentConfigurations.map((recent) => (
                      <li key={recent.recentHandle}>
                        <div>
                          <strong>{recent.name}</strong>
                          <small>{formatLastOpened(recent.lastOpenedEpochMs)}</small>
                        </div>
                        {recent.availability === "available" ? (
                          <button className="secondary" onClick={() => openRecentConfiguration(recent.recentHandle)}>Open</button>
                        ) : (
                          <>
                            <span className="error">Unavailable: file missing</span>
                            <button className="secondary" onClick={() => relinkRecentConfiguration(recent.recentHandle)}>Relink…</button>
                            <button
                              className="text-button danger-text"
                              onClick={async () => {
                                await api.removeRecentConfiguration(recent.recentHandle);
                                await refreshRecents();
                              }}
                            >Remove</button>
                          </>
                        )}
                      </li>
                    ))}
                    </ul>
                  </section>
                )}
                {savedConfiguration && (
                  <section className={`configuration-validation ${savedConfiguration.validation.state}`}>
                    <strong>{savedConfiguration.name}</strong>
                    <span>{savedConfiguration.validation.state.replaceAll("_", " ")}</span>
                    {savedConfiguration.validation.diagnostics.map((diagnostic) => (
                      <details key={`${diagnostic.key ?? "configuration"}-${diagnostic.code}`}>
                        <summary>{diagnostic.message}</summary>
                        <code>{diagnostic.code}{diagnostic.key ? ` · ${diagnostic.key}` : ""}</code>
                      </details>
                    ))}
                  </section>
                )}
                <div className="device-list" aria-busy={busy} role="region" aria-label="Detected Android devices">
                  {devices.length === 0 && <div className="empty-state" role="status">No ADB devices detected yet. Refresh after connecting a device.</div>}
                  <ul>
                  {devices.map((device) => (
                    <li key={device.deviceHandle}>
                      <button
                        aria-describedby={device.state !== "available" ? stableDomId("device-reason", device.deviceHandle) : undefined}
                        className="device-row"
                        disabled={device.state !== "available" || busy}
                        onClick={() => selectDevice(device.deviceHandle)}
                      >
                        <span><strong>{device.displayName}</strong><small>{device.maskedSerial}</small></span>
                        <span className={`status ${device.state}`}>Status: {device.state}</span>
                      </button>
                      {device.state !== "available" && (
                        <small className="disabled-reason" id={stableDomId("device-reason", device.deviceHandle)}>
                          This device cannot be selected until its status is available.
                        </small>
                      )}
                    </li>
                  ))}
                  </ul>
                </div>
                <button className="text-button" onClick={pollDevices}>Refresh devices</button>
              </>
            )}

            {workflow.step === "device" && <div className="empty-state" role="status"><h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Reading device properties</h2><p>Keep the selected device connected.</p></div>}

            {workflow.step === "setup" && workflow.facts && workflow.match && (
              <>
                <p className="eyebrow">CONFIRM DEVICE</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>{workflow.facts.manufacturer ?? "Android"} {workflow.facts.model ?? "device"}</h2>
                <p>Android {workflow.facts.androidVersion ?? "unknown"} · API {workflow.facts.androidApiLevel ?? "unknown"}</p>
                <div className="confidence">Match confidence: <strong>{workflow.match.confidence}</strong></div>
                {savedPlanUnavailable && (
                  <p className="error">
                    The saved device plan reference is unavailable or incompatible with this current device.
                    Choose an offered device plan explicitly; EmuChef will not substitute one automatically.
                  </p>
                )}
                {workflow.match.blocked ? (
                  <p className="error">{workflow.match.blockReason}</p>
                ) : (
                  <fieldset className="plan-options">
                    <legend>Choose a safe setup</legend>
                    {planOptions.map((plan) => (
                      <label key={plan.planId}>
                        <input
                          type="radio"
                          name="device-plan"
                          checked={workflow.devicePlan === plan.planId}
                          onChange={() => updateDevicePlanIntent(plan.planId)}
                        />
                        <span><strong>{plan.name}</strong><small>{plan.description}</small></span>
                      </label>
                    ))}
                  </fieldset>
                )}
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button aria-describedby={!workflow.devicePlan || savedPlanUnavailable || busy ? "setup-continue-reason" : undefined} disabled={!workflow.devicePlan || savedPlanUnavailable || busy} onClick={describe}>Continue</button>
                </div>
                {(!workflow.devicePlan || savedPlanUnavailable || busy) && (
                  <p className="disabled-reason" id="setup-continue-reason">
                    {busy ? "Device validation is in progress." : savedPlanUnavailable ? "Choose an available setup explicitly." : "Choose a safe setup first."}
                  </p>
                )}
              </>
            )}

            {workflow.step === "inputs" && workflow.description && (
              <>
                <p className="eyebrow">CHOOSE SETUP</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Customize your setup</h2>
                {validationErrors.length > 0 && (
                  <section
                    aria-labelledby="validation-summary-heading"
                    className="error error-summary"
                    ref={validationSummaryRef}
                    role="alert"
                    tabIndex={-1}
                  >
                    <h3 id="validation-summary-heading">Resolve {validationErrors.length} configuration {validationErrors.length === 1 ? "error" : "errors"}</h3>
                    <ul>
                      {validationErrors.map((item, index) => (
                        <li key={`${item.key ?? "global"}-${item.code}-${index}`}>
                          {item.targetId ? <a href={`#${item.targetId}`}>{item.message}</a> : item.message}
                          <details><summary>Technical details</summary><code>{item.code}</code></details>
                        </li>
                      ))}
                    </ul>
                  </section>
                )}
                <fieldset className="recipe-list">
                  <legend>Choose recipes</legend>
                  {workflow.description.recipeOptions.map((recipe) => (
                    <label key={recipe.id} className={!recipe.available ? "unavailable" : ""}>
                      <input
                        type="checkbox"
                        disabled={recipeSelectionDisabled(recipe)}
                        checked={(workflow.selectedRecipes ?? []).includes(recipe.id) || recipe.dependencyRequired}
                        onChange={(event) => {
                          const selected = updateRecipeSelection(
                            workflow.selectedRecipes ?? [],
                            recipe,
                            event.target.checked,
                          );
                          updateRecipeIntent(selected);
                        }}
                      />
                      <span>
                        <strong>{recipe.name}</strong>
                        <small>
                          {recipe.dependencyRequired
                            ? "Required dependency"
                            : !recipe.available
                              ? `Unavailable: ${recipe.unavailableCapabilities.join(", ")}`
                              : recipe.recommended
                                ? `Recommended for this device${recipe.description ? ` · ${recipe.description}` : ""}`
                                : recipe.description ?? "Optional"}
                        </small>
                      </span>
                    </label>
                  ))}
                </fieldset>

                {workflow.description.inputs.map((input) => {
                  const inputId = stableDomId("input", input.key);
                  const descriptionId = `${inputId}-description`;
                  const extensionsId = `${inputId}-extensions`;
                  const sourceId = `${inputId}-source`;
                  const diagnostics = inputDiagnosticsForDisplay(input);
                  const diagnosticIds = diagnostics.map((_, index) => `${inputId}-error-${index}`);
                  return (
                  <div className="input-field" key={input.key}>
                    <label htmlFor={inputId}>{input.label}{input.required ? " (required)" : ""}</label>
                    {input.type === "boolean" ? (
                      <input
                        aria-describedby={describedBy(input.description && descriptionId, ...diagnosticIds)}
                        aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                        id={inputId}
                        type="checkbox"
                        checked={Boolean(workflow.bindings[input.key] ?? input.value)}
                        onChange={(event) => updateBindingIntent(input.key, event.target.checked)}
                      />
                    ) : input.options?.length ? (
                      <select
                        aria-describedby={describedBy(input.description && descriptionId, input.valueSource && sourceId, ...diagnosticIds)}
                        aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                        id={inputId}
                        value={String(workflow.bindings[input.key] ?? input.value ?? "")}
                        onChange={(event) => updateBindingIntent(input.key, event.target.value)}
                      >
                        <option value="">Choose…</option>
                        {input.options.map((option) => <option key={option}>{option}</option>)}
                      </select>
                    ) : input.pathKind ? (
                      <div className="path-picker">
                        <input
                          aria-describedby={describedBy(input.description && descriptionId, Boolean(input.acceptedExtensions?.length) && extensionsId, input.valueSource && sourceId, ...diagnosticIds)}
                          aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                          id={inputId}
                          readOnly
                          value={String(workflow.bindings[input.key] ?? input.value ?? "")}
                        />
                        <button
                          aria-describedby={busy ? `${inputId}-browse-reason` : undefined}
                          className="secondary"
                          disabled={busy}
                          onClick={() => pickInputValue(input)}
                        >Browse…</button>
                        {busy && <small className="disabled-reason" id={`${inputId}-browse-reason`}>A file or validation operation is already in progress.</small>}
                      </div>
                    ) : (
                      <input
                        aria-describedby={describedBy(input.description && descriptionId, input.valueSource && sourceId, ...diagnosticIds)}
                        aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                        id={inputId}
                        value={String(workflow.bindings[input.key] ?? input.value ?? "")}
                        onChange={(event) => updateBindingIntent(input.key, event.target.value)}
                      />
                    )}
                    {input.description && <small id={descriptionId}>{input.description}</small>}
                    {input.acceptedExtensions?.length ? (
                      <small id={extensionsId}>Accepted file types: {input.acceptedExtensions.join(", ")}</small>
                    ) : null}
                    {input.valueSource ? (
                      <small id={sourceId}>Value source: {input.valueSource.replaceAll("_", " ")}</small>
                    ) : null}
                    {diagnostics.map((item, index) => (
                      <small className="error" id={diagnosticIds[index]} key={`${item.key ?? input.key}-${item.code}`}>Error: {item.message}</small>
                    ))}
                  </div>
                  );
                })}

                {pageDiagnosticsForDisplay(workflow.description).map((item) => (
                  <p
                    className={item.severity === "error" ? "error" : "warning"}
                    key={`${item.key ?? "global"}-${item.code}-${item.message}`}
                  >{item.severity === "error" ? "Error: " : "Warning: "}{item.message}</p>
                ))}
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button className="secondary" onClick={describe} disabled={busy}>Refresh validation</button>
                  <button aria-describedby={!reviewReady(workflow) || busy ? "review-disabled-reason" : undefined} onClick={generateReview} disabled={!reviewReady(workflow) || busy}>Review plan</button>
                </div>
                {(!reviewReady(workflow) || busy) && (
                  <p className="disabled-reason" id="review-disabled-reason">
                    {busy ? "Validation is in progress." : "Resolve required values and validation errors before review."}
                  </p>
                )}
              </>
            )}

            {workflow.step === "review" && workflow.review && (
              <>
                <p className="eyebrow">REVIEW PLAN</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Ready for a simulated dry run</h2>
                <p className="simulation-banner">
                  Simulated / Dry Run only. This does not change or verify the real device.
                </p>
                <p>
                  Target: {workflow.review.target.manufacturer ?? "Android"} {workflow.review.target.model ?? "device"}
                  · Android {workflow.review.target.androidVersion ?? "unknown"}
                </p>
                {workflow.review.selectedInputs.length > 0 && (
                  <section className="review-inputs" aria-labelledby="selected-options-heading">
                    <h3 id="selected-options-heading">Selected options</h3>
                    <dl>
                      {workflow.review.selectedInputs.map((input) => (
                        <div key={input.key}><dt>{input.key}</dt><dd>{input.value}</dd></div>
                      ))}
                    </dl>
                  </section>
                )}
                {workflow.review.groups.map((group) => (
                  <article className="review-group" key={group.recipeId}>
                    <h3>{group.recipeName}</h3>
                    {group.recipeDescription && <p>{group.recipeDescription}</p>}
                    <ol>
                      {group.steps.map((step) => (
                        <li key={step.technicalId}>
                          <strong>{step.name}</strong>{step.note && <span>{step.note}</span>}
                            <span>{step.kindLabel}</span>
                            {step.elevated && <em>Elevated access</em>}
                            {step.requirements.length > 0 && (
                              <small>Requires: {step.requirements.join(", ")}</small>
                            )}
                          <details><summary>Technical details</summary><code>{step.technicalId} · {step.technicalType}</code></details>
                        </li>
                      ))}
                    </ol>
                  </article>
                ))}
                <p className="digest">Plan digest: {workflow.review.planDigest}</p>
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button aria-describedby={busy || workflow.execution.kind === "starting" ? "execution-start-reason" : undefined} onClick={startSimulation} disabled={busy || workflow.execution.kind === "starting"}>
                    {workflow.execution.kind === "starting" ? "Starting simulated run…" : "Start Simulated Dry Run"}
                  </button>
                  {realExecutionEnabled && (
                    <button
                      className="danger"
                      onClick={(event) => void requestRealExecution(event.currentTarget)}
                      disabled={busy || workflow.execution.kind === "starting"}
                    >
                      Apply to Device
                    </button>
                  )}
                </div>
                {(busy || workflow.execution.kind === "starting") && <p className="disabled-reason" id="execution-start-reason">Execution start is already being prepared.</p>}
              </>
            )}

            {workflow.step === "execution" &&
              (workflow.execution.kind === "active" || workflow.execution.kind === "terminal") && (
                <>
                  <p className="eyebrow">
                    {workflow.execution.mode === "real" ? "REAL DEVICE" : "SIMULATED / DRY RUN"}
                  </p>
                  <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
                    {workflow.execution.kind === "terminal"
                      ? `${workflow.execution.mode === "real" ? "Real-device execution" : "Simulation"} ${workflow.execution.snapshot.status.replaceAll("_", " ")}`
                      : workflow.execution.mode === "real"
                        ? "Applying the reviewed setup"
                        : "Simulating the reviewed setup"}
                  </h2>
                  {workflow.execution.snapshot.completion.counts.total > 0 ? (
                    <div className="execution-progress">
                      <label htmlFor="execution-progress">
                        Execution progress: {Math.round((
                          (workflow.execution.snapshot.completion.counts.completed
                            + workflow.execution.snapshot.completion.counts.skipped
                            + workflow.execution.snapshot.completion.counts.blocked
                            + workflow.execution.snapshot.completion.counts.failed
                            + workflow.execution.snapshot.completion.counts.cancelled)
                          / workflow.execution.snapshot.completion.counts.total
                        ) * 100)}%
                      </label>
                      <progress
                        id="execution-progress"
                        max={workflow.execution.snapshot.completion.counts.total}
                        value={workflow.execution.snapshot.completion.counts.completed
                          + workflow.execution.snapshot.completion.counts.skipped
                          + workflow.execution.snapshot.completion.counts.blocked
                          + workflow.execution.snapshot.completion.counts.failed
                          + workflow.execution.snapshot.completion.counts.cancelled}
                      />
                    </div>
                  ) : (
                    <p aria-busy="true" role="status">Execution progress is starting; the total step count is not available yet.</p>
                  )}
                  {workflow.execution.mode === "real" ? (
                    <p className="warning">
                      Keep the device connected. Failure or cancellation may leave completed changes on the device;
                      there is no rollback, restore, or automatic recovery.
                    </p>
                  ) : (
                    <p className="simulation-banner">
                      No real device changes are made. This report is simulated evidence only.
                    </p>
                  )}
                  <dl className="execution-summary">
                    <div><dt>Status</dt><dd>{workflow.execution.snapshot.status.replaceAll("_", " ")}</dd></div>
                    <div><dt>Started</dt><dd>{workflow.execution.snapshot.startedAt ?? "Starting"}</dd></div>
                    {executionDuration(workflow.execution.snapshot) && (
                      <div><dt>Duration</dt><dd>{executionDuration(workflow.execution.snapshot)}</dd></div>
                    )}
                    <div><dt>Completed</dt><dd>{workflow.execution.snapshot.completion.counts.completed}</dd></div>
                    <div><dt>Skipped</dt><dd>{workflow.execution.snapshot.completion.counts.skipped}</dd></div>
                    <div><dt>Blocked</dt><dd>{workflow.execution.snapshot.completion.counts.blocked}</dd></div>
                    <div><dt>Failed</dt><dd>{workflow.execution.snapshot.completion.counts.failed}</dd></div>
                  </dl>
                  {workflow.execution.snapshot.completion.partialChangesPossible && (
                    <p className="warning">
                      Some device changes completed before this {workflow.execution.snapshot.status} result.
                      The result remains {workflow.execution.snapshot.status}; EmuChef does not infer partial success or rollback completed work.
                    </p>
                  )}
                  {workflow.execution.snapshot.warnings.map((issue) => (
                    <div className="warning" key={`warning-${issue.code}-${issue.stepId ?? "run"}`}>
                      <p>{issue.message}</p><small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
                    </div>
                  ))}
                  {workflow.execution.snapshot.errors.map((issue) => (
                    <div className="error" key={`error-${issue.code}-${issue.stepId ?? "run"}`}>
                      <p>{issue.message}</p><small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
                    </div>
                  ))}
                  {workflow.execution.snapshot.recipes.map((recipe) => (
                    <article className={`execution-group status-${recipe.status}`} key={recipe.recipeId}>
                      <div className="execution-heading">
                        <div><h3>{recipe.name}</h3>{recipe.description && <p>{recipe.description}</p>}</div>
                        <span className="execution-status">{recipe.status.replaceAll("_", " ")}</span>
                      </div>
                      <ol>
                        {recipe.steps.map((step) => (
                          <li key={step.stepId} className={`step-${step.status}`}>
                            <strong>{step.note ?? step.name}</strong>
                            <span>{step.status.replaceAll("_", " ")}</span>
                            {step.note && step.note !== step.name && <small>{step.name}</small>}
                            {step.message && <small>{step.message}</small>}
                          </li>
                        ))}
                      </ol>
                    </article>
                  ))}
                  {workflow.execution.events.length > 0 && (
                    <details className="execution-events">
                      <summary>Incremental {workflow.execution.mode === "real" ? "real-device" : "simulated"} event log</summary>
                      <ol>
                        {workflow.execution.events.map((event) => (
                          <li key={event.sequence}>
                            <time>{event.timestamp}</time> {event.note ?? event.message ?? event.eventType}
                            {event.phase && ` · ${event.phase.replaceAll("_", " ")}`}
                            {event.status && ` · ${event.status.replaceAll("_", " ")}`}
                          </li>
                        ))}
                      </ol>
                    </details>
                  )}
                  {workflow.execution.kind === "active" ? (
                    <>
                      {workflow.execution.cancellationRequested && (
                        <p className="warning">
                          {workflow.execution.mode === "real"
                            ? "Cancellation requested. The current atomic operation may finish; completed device changes are not reversed, and no new work starts after cancellation is observed."
                            : "Cancellation requested. Completed simulated steps remain visible in this report. No new simulated steps start, the current simulated atomic step may finish, and no real device changes or rollback exist."}
                        </p>
                      )}
                      <button
                        aria-describedby={workflow.execution.cancellationRequested ? "cancellation-requested-reason" : undefined}
                        className="danger"
                        onClick={cancelExecution}
                        disabled={workflow.execution.cancellationRequested}
                      >
                        {workflow.execution.cancellationRequested
                          ? "Cancellation requested"
                          : workflow.execution.mode === "real"
                            ? "Request cancellation"
                            : "Cancel simulated run"}
                      </button>
                      {workflow.execution.cancellationRequested && <p className="disabled-reason" id="cancellation-requested-reason">A cancellation request is already pending; the current atomic operation may still finish.</p>}
                    </>
                  ) : (
                    <div className="button-row">
                      <button
                        className="secondary"
                        onClick={exportExecutionReport}
                        disabled={reportState === "exporting"}
                      >
                        {reportState === "exporting" ? "Exporting…" : reportState === "saved" ? "Report saved" : "Export report"}
                      </button>
                      {workflow.execution.snapshot.status !== "succeeded" && (
                        <button onClick={prepareRepair} disabled={repairPreparing}>
                          {repairPreparing
                            ? "Preparing fresh plan…"
                            : workflow.execution.snapshot.status === "succeeded_with_warnings"
                              ? "Repair configuration"
                              : "Retry failed work"}
                        </button>
                      )}
                      {!workflow.execution.snapshot.simulated && workflow.execution.snapshot.launchAction && (
                        <button
                          onClick={launchConfiguredApp}
                          disabled={launchState === "launching" || launchState === "launched"}
                        >
                          {launchState === "launching"
                            ? "Launching…"
                            : launchState === "launched"
                              ? "App launched"
                              : workflow.execution.snapshot.launchAction.label}
                        </button>
                      )}
                      <button
                        className="secondary"
                        onClick={() => {
                          if (workflow.execution.kind !== "terminal") return;
                          dispatch({
                            type: workflow.execution.mode === "real" ? "runtime-invalidated" : "return-to-review",
                          });
                        }}
                      >
                        {workflow.execution.mode === "real" ? "Start a fresh workflow" : "Return to Review"}
                      </button>
                    </div>
                  )}
                </>
              )}

            {workflow.step === "execution" && workflow.execution.kind === "unavailable" && (
              <>
                <p className="eyebrow">
                  {workflow.execution.mode === "real" ? "REAL-DEVICE OUTCOME UNKNOWN" : "SIMULATED RUN UNAVAILABLE"}
                </p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
                  {workflow.execution.mode === "real"
                    ? "The device may have been partially changed"
                    : "This in-memory simulation cannot be resumed"}
                </h2>
                <p className="warning">{workflow.execution.message}</p>
                <p>
                  {workflow.execution.mode === "real"
                    ? "The outcome cannot be inferred. Reconnect and create a fresh review; this execution cannot be resumed, retried in place, restored, or rolled back."
                    : "No execution history is persisted across an app or sidecar restart."}
                </p>
                <button onClick={prepareRepair} disabled={repairPreparing}>
                  {repairPreparing ? "Preparing fresh plan…" : "Repair configuration"}
                </button>
                <button
                  className="secondary"
                  onClick={() => {
                    if (workflow.execution.kind !== "unavailable") return;
                    dispatch({
                      type: workflow.execution.mode === "real" ? "runtime-invalidated" : "return-to-review",
                    });
                  }}
                >
                  {workflow.execution.mode === "real" ? "Start a fresh workflow" : "Return to Review"}
                </button>
              </>
            )}
          </section>

          <aside className="status-panel">
            <p className="eyebrow">SYSTEM STATUS</p>
            <dl>
              <div><dt>Rust runtime</dt><dd>Ready</dd></div>
              <div><dt>Platform-Tools</dt><dd>{adb.version}</dd></div>
              <div><dt>Catalog</dt><dd>{catalog?.catalog.version ?? "Ready"}</dd></div>
              <div><dt>Mode</dt><dd>{realExecutionEnabled ? "Simulation and guarded real execution" : "Simulation only"}</dd></div>
            </dl>
            {adb.warning && <p className="warning">{adb.warning}</p>}
            <button className="text-button" onClick={importPlatformTools} disabled={busy || workflow.step === "execution"}>Replace Platform-Tools</button>
            <button className="text-button danger-text" onClick={removePlatformTools} disabled={busy || workflow.step === "execution"}>Remove Platform-Tools</button>
            {(busy || workflow.step === "execution") && (
              <p className="disabled-reason">Platform-Tools changes are unavailable during another operation or execution.</p>
            )}
          </aside>
        </main>
      )}

      {activeDialog?.payload.kind === "unsaved" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="unsaved-dialog-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, "cancel")}
          returnFocus={activeDialog.payload.invoker}
          role="alertdialog"
          titleId="unsaved-dialog-title"
        >
          <p className="eyebrow">UNSAVED CONFIGURATION</p>
          <h2 id="unsaved-dialog-title">Save edits before continuing?</h2>
          <p id="unsaved-dialog-description">
            Save preserves the portable configuration edits. Discard permanently abandons the unsaved edits. Cancel keeps the current configuration open.
          </p>
          <div className="button-row">
            <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, "cancel")}>Cancel</button>
            <button onClick={() => dialogController.settle(activeDialog.id, "save")}>Save</button>
            <button className="danger" onClick={() => dialogController.settle(activeDialog.id, "discard")}>Discard edits</button>
          </div>
        </AccessibleDialog>
      )}

      {activeDialog?.payload.kind === "name" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="name-dialog-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, null)}
          returnFocus={activeDialog.payload.invoker}
          titleId="name-dialog-title"
        >
          <form onSubmit={(event) => {
            event.preventDefault();
            const value = namePromptValue.trim();
            if (value) dialogController.settle(activeDialog.id, value);
          }}>
            <h2 id="name-dialog-title">{activeDialog.payload.title}</h2>
            <p id="name-dialog-description">The name identifies this portable configuration. Runtime authority and device details are not saved.</p>
            <label className="input-field" htmlFor="configuration-name">Configuration name</label>
            <input
              autoComplete="off"
              id="configuration-name"
              onChange={(event) => setNamePromptValue(event.target.value)}
              required
              value={namePromptValue}
            />
            <div className="button-row">
              <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, null)} type="button">Cancel</button>
              <button disabled={!namePromptValue.trim()} type="submit">Continue</button>
            </div>
            {!namePromptValue.trim() && <p className="disabled-reason">Enter a configuration name to continue.</p>}
          </form>
        </AccessibleDialog>
      )}

      {activeDialog?.payload.kind === "real-execution" && workflow.review && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="real-confirmation-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, false)}
          returnFocus={activeDialog.payload.invoker}
          role="alertdialog"
          titleId="real-confirmation-heading"
        >
          <p className="eyebrow">REAL DEVICE</p>
          <h2 id="real-confirmation-heading">Confirm irreversible device changes</h2>
          <div id="real-confirmation-description">
            <p>
              Connected Android device · {workflow.review.target.manufacturer ?? "Android"}
              {` ${workflow.review.target.model ?? "device"}`} · API {workflow.review.target.androidApiLevel ?? "unknown"}
            </p>
            <p className="error">Danger: this can install software, transfer or replace files, change permissions and app operations, and launch or stop applications.</p>
            <p className="warning">No rollback, restore, automatic backup, or prior-state recovery exists. Cancellation is cooperative and does not undo completed changes.</p>
          </div>
          <label className="input-field" htmlFor="real-confirmation-phrase">Type APPLY TO DEVICE</label>
          <input
            autoComplete="off"
            id="real-confirmation-phrase"
            value={realConfirmation.phrase}
            onChange={(event) => setRealConfirmation({ ...realConfirmation, phrase: event.target.value })}
          />
          <label><input type="checkbox" checked={realConfirmation.irreversibleChangesAcknowledged} onChange={(event) => setRealConfirmation({ ...realConfirmation, irreversibleChangesAcknowledged: event.target.checked })} /> I understand this can irreversibly change the device.</label>
          <label><input type="checkbox" checked={realConfirmation.noRollbackAcknowledged} onChange={(event) => setRealConfirmation({ ...realConfirmation, noRollbackAcknowledged: event.target.checked })} /> I understand there is no rollback, restore, or backup recovery.</label>
          <label><input type="checkbox" checked={realConfirmation.keepDeviceConnectedAcknowledged} onChange={(event) => setRealConfirmation({ ...realConfirmation, keepDeviceConnectedAcknowledged: event.target.checked })} /> I will keep the intended device connected and stable.</label>
          <div className="button-row">
            <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, false)}>Cancel</button>
            <button
              aria-describedby={!realExecutionConfirmationComplete(realConfirmation) ? "real-confirmation-reason" : undefined}
              className="danger"
              disabled={!realExecutionConfirmationComplete(realConfirmation)}
              onClick={() => dialogController.settle(activeDialog.id, true)}
            >Start Real-Device Execution</button>
          </div>
          {!realExecutionConfirmationComplete(realConfirmation) && <p className="disabled-reason" id="real-confirmation-reason">Enter the exact phrase and acknowledge all three safety statements.</p>}
        </AccessibleDialog>
      )}
    </div>
  );
}

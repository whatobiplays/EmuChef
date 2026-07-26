import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { api } from "./api";
import {
  createAppDialogController,
  type AppDialogController,
  type AppDialogPayload,
  type AppDialogResult,
  type UnsavedDecision,
} from "./app-dialogs";
import {
  diagnosticIsBlocking,
  errorMessage,
} from "./app-helpers";
import { ExecutionStep } from "./ExecutionStep";
import { InputsStep } from "./InputsStep";
import { ReviewStep } from "./ReviewStep";
import { SupportPanel } from "./SupportPanel";
import { UpdatesPanel } from "./UpdatesPanel";
import { SavedConfigurationManager } from "./SavedConfigurationManager";
import {
  SavedConfigurationMenuBridge,
  type SavedConfigurationMenuAction,
} from "./SavedConfigurationMenuBridge";
import { useExecution } from "./useExecution";
import { AccessibleDialog } from "./AccessibleDialog";
import {
  claimFocusTransition,
  lifecycleBoundResult,
  restoreAccessibleFocus,
  stableDomId,
  type DialogSnapshot,
} from "./accessibility";
import type {
  AdbSetupStatus,
  CatalogSummary,
  ConfigurationDescription,
  DeviceSummary,
  ExecutionCapabilities,
  DeviceQualificationSnapshot,
  RootQualification,
  InputDescriptor,
  RecentConfiguration,
  RuntimeStatus,
  SavedConfigurationDocument,
  SavedConfigurationMutation,
  SavedConfigurationPreview,
  CacheCleanupMode,
  RecoveryDraftAvailable,
  RecoveryRestoreResult,
  RecoveryWriteAck,
  UpdateInteractionSession,
  CorrectiveAction,
  ResetLocalStateCategory,
} from "./types";
import {
  resolveUnsavedDecision,
  savedConfigurationBlocksProgress,
  saveConfigurationDisabledReason,
  savedConfigurationDiagnosticSummary,
  savedConfigurationValidationLabel,
  savedDevicePlanAvailable,
} from "./savedConfigurations";
import {
  emptyRealExecutionConfirmation,
  realExecutionConfirmationComplete,
} from "./realExecution";
import {
  initialWorkflowState,
  filterRepairBindings,
  deviceIsUnsupported,
  inputDiagnosticsForDisplay,
  pageDiagnosticsForDisplay,
  recipeSelectionDisabled,
  portableBindingsForTransition,
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
import { portableIntentSignature, recoveryResultIsCurrent } from "./recovery";
import {
  initialUpdatePanelState,
  nextInteractionGeneration,
  updateNavigationBlocked,
} from "./update-policy";

const WORKFLOW_STEPS = [
  { step: "connect", label: "Connect" },
  { step: "device", label: "Device" },
  { step: "setup", label: "Setup" },
  { step: "inputs", label: "Customize" },
  { step: "review", label: "Review" },
  { step: "execution", label: "Simulated Run" },
] as const;

const platformToolsStatusLabels: Record<ExecutionCapabilities["platformToolsStatus"], string> = {
  notApplicable: "Not applicable",
  ready: "Ready",
  notFound: "Not found",
  invalid: "Invalid",
  checkFailed: "Check failed",
};

const executorReadinessLabels: Record<ExecutionCapabilities["executorReadiness"], string> = {
  notCompiled: "Not compiled",
  ready: "Ready",
  blocked: "Blocked",
  unknown: "Unknown",
};

const deviceQualificationStateLabels: Record<DeviceQualificationSnapshot["state"], string> = {
  notApplicable: "Not applicable",
  noDevice: "No device",
  unauthorized: "Authorization required",
  offline: "Offline",
  insufficientlyQualified: "Qualification incomplete",
  unsupported: "Unsupported",
  supported: "Supported",
};

const deviceAbiLabels: Record<NonNullable<DeviceQualificationSnapshot["abiClass"]>, string> = {
  arm64: "64-bit ARM",
  arm32: "32-bit ARM",
  x86_64: "64-bit x86",
};

const capabilityAvailabilityLabels: Record<DeviceQualificationSnapshot["storage"], string> = {
  available: "Available",
  unavailable: "Unavailable",
  unknown: "Unknown",
};

function DeviceQualificationDetails({
  qualification,
  rootCheckPhase,
  onCheckRoot,
}: {
  qualification: DeviceQualificationSnapshot | null;
  rootCheckPhase: "idle" | "checking";
  onCheckRoot: () => void;
}) {
  if (!qualification) return null;

  const rootLabel = (root: RootQualification | null) => {
    if (!root) return "Not checked";
    if (root.status === "granted") return "Granted";
    if (root.status === "denied") return "Denied";
    if (root.status === "unavailable") return "Unavailable";
    return root.message;
  };

  return (
    <section className="device-qualification" aria-labelledby="device-qualification-heading">
      <div className="device-qualification-heading">
        <div>
          <p className="eyebrow">Compatibility check</p>
          <h3 id="device-qualification-heading">Device qualification</h3>
        </div>
        <span className={`status qualification-${qualification.state}`}>
          {deviceQualificationStateLabels[qualification.state]}
        </span>
      </div>
      <p>{qualification.summary}</p>
      <dl className="device-qualification-facts">
        <div><dt>Android version</dt><dd>{qualification.androidMajor ?? "Unknown"}</dd></div>
        <div><dt>API level</dt><dd>{qualification.androidApiLevel ?? "Unknown"}</dd></div>
        <div><dt>Processor architecture</dt><dd>{qualification.abiClass ? deviceAbiLabels[qualification.abiClass] : "Unknown"}</dd></div>
        <div><dt>Shared storage</dt><dd>{capabilityAvailabilityLabels[qualification.storage]}</dd></div>
        <div><dt>Package management</dt><dd>{capabilityAvailabilityLabels[qualification.packageManager]}</dd></div>
        <div><dt>Activity management</dt><dd>{capabilityAvailabilityLabels[qualification.activityManager]}</dd></div>
        <div><dt>Root access</dt><dd>{rootLabel(qualification.root)}</dd></div>
      </dl>
      {qualification.state === "supported" && (
        <div className="device-qualification-root-check">
          <p>
            {rootCheckPhase === "checking"
              ? "Waiting for root authorization on the device. Approve the prompt from Magisk, KernelSU, APatch, or your root manager."
              : "EmuChef will check root access only when you request it."}
          </p>
          <button type="button" onClick={onCheckRoot} disabled={rootCheckPhase === "checking"}>
            {rootCheckPhase === "checking"
              ? "Checking root access…"
              : qualification.root && qualification.root.status !== "granted" ? "Check again" : "Check root access"}
          </button>
        </div>
      )}
      {qualification.limitations.length > 0 && (
        <div className="device-qualification-limitations">
          <h4>Limitations and next steps</h4>
          <ul>
            {qualification.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}
          </ul>
        </div>
      )}
    </section>
  );
}

export { createAppDialogController } from "./app-dialogs";

interface AppProps {
  dialogController?: AppDialogController;
}

export function App({ dialogController: suppliedDialogController }: AppProps = {}) {
  const ownedDialogControllerRef = useRef<AppDialogController | null>(null);
  if (!ownedDialogControllerRef.current) {
    ownedDialogControllerRef.current = suppliedDialogController ?? createAppDialogController();
  }
  const dialogController = ownedDialogControllerRef.current;
  const [runtime, setRuntime] = useState<RuntimeStatus>({ status: "starting" });
  const [, setCatalog] = useState<CatalogSummary | null>(null);
  const [adb, setAdb] = useState<AdbSetupStatus | null>(null);
  const [devices, setDevices] = useState<DeviceSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [platformToolsOperation, setPlatformToolsOperation] = useState<{
    phase: "idle" | "picker" | "processing";
    kind: "import" | "replace" | "remove";
  }>({ phase: "idle", kind: "import" });
  const [deviceRefresh, setDeviceRefresh] = useState<{
    phase: "idle" | "refreshing" | "complete";
    generation: number;
    message: string | null;
  }>({ phase: "idle", generation: 0, message: null });
  const [notice, setNotice] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [executionCapabilities, setExecutionCapabilities] = useState<ExecutionCapabilities | null>(null);
  const [deviceQualification, setDeviceQualification] = useState<DeviceQualificationSnapshot | null>(null);
  const [rootCheckPhase, setRootCheckPhase] = useState<"idle" | "checking">("idle");
  const [executionCapabilitiesRefresh, setExecutionCapabilitiesRefresh] = useState<
    "idle" | "refreshing" | "failed"
  >("idle");
  const [realConfirmation, setRealConfirmation] = useState(emptyRealExecutionConfirmation);
  const [repairPreparing, setRepairPreparing] = useState(false);
  const [startupReady, setStartupReady] = useState(false);
  const [recoveredName, setRecoveredName] = useState<string | null>(null);
  const [savedConfiguration, setSavedConfiguration] = useState<SavedConfigurationDocument | null>(null);
  const [recentConfigurations, setRecentConfigurations] = useState<RecentConfiguration[]>([]);
  const [configurationManagerOpen, setConfigurationManagerOpen] = useState(false);
  const [configurationManagerBusy, setConfigurationManagerBusy] = useState(false);
  const [configurationPreview, setConfigurationPreview] = useState<SavedConfigurationPreview | null>(null);
  const [configurationPreviewMode, setConfigurationPreviewMode] = useState<"open" | "import">("open");
  const [workflow, dispatch] = useReducer(workflowReducer, initialWorkflowState);
  const [touchedInputKeys, setTouchedInputKeys] = useState<Set<string>>(() => new Set());
  const [validationRequested, setValidationRequested] = useState(false);
  const [support, supportDispatch] = useReducer(supportReducer, initialSupportState);
  const [updates, setUpdates] = useState(initialUpdatePanelState);
  const [activeDialog, setActiveDialog] = useState<DialogSnapshot<AppDialogPayload> | null>(
    dialogController.snapshot,
  );
  const [namePromptValue, setNamePromptValue] = useState("");
  const [politeAnnouncement, setPoliteAnnouncement] = useState({ id: 0, text: "" });
  const [assertiveAnnouncement, setAssertiveAnnouncement] = useState({ id: 0, text: "" });
  const mainRef = useRef<HTMLElement>(null);
  const supportInvokerRef = useRef<HTMLElement | null>(null);
  const updatesInvokerRef = useRef<HTMLElement | null>(null);
  const configurationManagerInvokerRef = useRef<HTMLElement | null>(null);
  const configurationPreviewGenerationRef = useRef(0);
  const validationSummaryRef = useRef<HTMLElement>(null);
  const realConfirmationRef = useRef(realConfirmation);
  const appLifecycleGenerationRef = useRef(0);
  const runtimeGenerationRef = useRef(0);
  const platformToolsGenerationRef = useRef(0);
  const executionCapabilitiesGenerationRef = useRef(0);
  const executionCapabilitiesRef = useRef<ExecutionCapabilities | null>(null);
  const devicePollGenerationRef = useRef(0);
  const deviceSelectionGenerationRef = useRef(0);
  const rootCheckGenerationRef = useRef(0);
  const deviceRefreshTimerRef = useRef<number | null>(null);
  const manualDeviceRefreshRef = useRef(false);
  const initialWorkflowPresentationRef = useRef(false);
  const savedConfigurationRef = useRef<SavedConfigurationDocument | null>(null);
  const workflowRef = useRef(workflow);
  const configurationMutationQueue = useRef<Promise<void>>(Promise.resolve());
  const sessionInitializedRef = useRef(false);
  const supportGenerationRef = useRef(0);
  const allowWindowCloseRef = useRef(false);
  const recoveryNotNowRef = useRef<HTMLButtonElement>(null);
  const recoverySessionGenerationRef = useRef(0);
  const recoveryRecordGenerationRef = useRef<number | null>(null);
  const recoveryRequestGenerationRef = useRef(0);
  const inputOperationGenerationRef = useRef(0);
  const recoveryDraftGenerationRef = useRef(0);
  const lastRecoverySignatureRef = useRef<string | null>(null);
  const updateInteractionSessionRef = useRef<UpdateInteractionSession | null>(null);
  const updateInteractionQueueRef = useRef<Promise<void>>(Promise.resolve());
  const [updateInteractionRevision, setUpdateInteractionRevision] = useState(0);

  const announce = useCallback((text: string, assertive = false) => {
    const update = (previous: { id: number; text: string }) => ({ id: previous.id + 1, text });
    if (assertive) setAssertiveAnnouncement(update);
    else setPoliteAnnouncement(update);
  }, []);

  const refreshExecutionCapabilities = useCallback(async (required = false) => {
    const generation = ++executionCapabilitiesGenerationRef.current;
    if (executionCapabilitiesRef.current) {
      setExecutionCapabilitiesRefresh("refreshing");
    }
    try {
      const capabilities = await api.executionCapabilities();
      if (executionCapabilitiesGenerationRef.current !== generation) return null;
      executionCapabilitiesRef.current = capabilities;
      setExecutionCapabilities(capabilities);
      setExecutionCapabilitiesRefresh("idle");
      return capabilities;
    } catch (error) {
      if (executionCapabilitiesGenerationRef.current !== generation) return null;
      setExecutionCapabilitiesRefresh("failed");
      if (required) throw error;
      return null;
    }
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

  const realExecutionCompiled = executionCapabilities?.realExecutionCompiled === true;

  const {
    cancelExecution,
    exportExecutionReport,
    launchConfiguredApp,
    launchState,
    reportState,
    resetExecutionPresentation,
    startRealExecution,
    startSimulation,
  } = useExecution({
    announce,
    dispatch,
    mainRef,
    qualification: deviceQualification,
    realExecutionCompiled,
    runtimeGenerationRef,
    setBusy,
    setNotice,
    withNativeDialogFocus,
    workflow,
    workflowRef,
  });

  const navigationBlocked = updateNavigationBlocked({
    startupReady,
    busy,
    executionKind: workflow.execution.kind,
    appDialogOpen: activeDialog !== null,
    supportOpen: support.open,
    updatePanelOpen: updates.open,
    updateChecking: updates.checking,
    updateOpening: updates.opening,
  });

  useEffect(() => {
    let disposed = false;
    void api.beginUpdateInteractionSession().then((session) => {
      if (disposed) {
        void api.endUpdateInteractionSession(session.sessionId).catch(() => undefined);
        return;
      }
      updateInteractionSessionRef.current = session;
      setUpdateInteractionRevision((value) => value + 1);
    }).catch(() => {
      updateInteractionSessionRef.current = null;
    });
    return () => {
      disposed = true;
      const session = updateInteractionSessionRef.current;
      updateInteractionSessionRef.current = null;
      if (session) void api.endUpdateInteractionSession(session.sessionId).catch(() => undefined);
    };
  }, []);

  useEffect(() => {
    const blocked = navigationBlocked;
    updateInteractionQueueRef.current = updateInteractionQueueRef.current.then(async () => {
      const session = updateInteractionSessionRef.current;
      if (!session) return;
      const generation = nextInteractionGeneration(session.generation);
      if (generation === null) {
        updateInteractionSessionRef.current = await api.beginUpdateInteractionSession();
        setUpdateInteractionRevision((value) => value + 1);
        return;
      }
      session.generation = generation;
      try {
        await api.setUpdateInteractionState({
          sessionId: session.sessionId,
          generation,
          blocked,
        });
      } catch {
        // Rotating the session returns Rust to blocked even if the page reloads,
        // teardown races, or a stale release update is rejected.
        updateInteractionSessionRef.current = await api.beginUpdateInteractionSession().catch(() => null);
        setUpdateInteractionRevision((value) => value + 1);
      }
    });
  }, [navigationBlocked, updateInteractionRevision]);

  const initialize = useCallback(async (runtimeGeneration = runtimeGenerationRef.current) => {
    const [runtimeStatus, adbStatus, , qualification] = await Promise.all([
      api.runtimeStatus(),
      api.adbStatus(),
      refreshExecutionCapabilities(true),
      api.deviceQualification(null),
    ]);
    if (runtimeGenerationRef.current !== runtimeGeneration) return;
    setRuntime(runtimeStatus);
    setAdb(adbStatus);
    setDeviceQualification(qualification);
    if (runtimeStatus.status === "ready") {
      const [nextCatalog, recents] = await Promise.all([
        api.catalog(),
        api.listRecentConfigurations(),
      ]);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      setCatalog(nextCatalog);
      setRecentConfigurations(recents);
    }
  }, [refreshExecutionCapabilities]);

  useEffect(() => {
    savedConfigurationRef.current = savedConfiguration;
  }, [savedConfiguration]);

  useEffect(() => {
    workflowRef.current = workflow;
  }, [workflow]);

  useEffect(() => {
    realConfirmationRef.current = realConfirmation;
  }, [realConfirmation]);

  useEffect(() => {
    if (!startupReady) return;
    if (!initialWorkflowPresentationRef.current) {
      initialWorkflowPresentationRef.current = true;
      return;
    }
    const generation = claimFocusTransition();
    queueMicrotask(() => {
      const destination = mainRef.current?.querySelector<HTMLElement>("[data-step-heading]");
      restoreAccessibleFocus({
        preferred: [destination, mainRef.current],
        generation,
      });
    });
  }, [adb?.status, runtime.status, startupReady, workflow.step]);

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
    const runtimeGeneration = runtimeGenerationRef.current;
    const recents = await api.listRecentConfigurations();
    if (runtimeGenerationRef.current === runtimeGeneration) setRecentConfigurations(recents);
  };

  const persistRecoveryNow = async (
    force = false,
    includeClean = false,
  ): Promise<RecoveryWriteAck | null> => {
    const current = workflowRef.current;
    const sessionGeneration = recoverySessionGenerationRef.current;
    const dirty = current.portableIntentDirty || Boolean(savedConfigurationRef.current?.dirty);
    if (!sessionGeneration || (!dirty && !includeClean) || !current.devicePlan) return null;
    const signature = portableIntentSignature(current);
    if (!force && signature === lastRecoverySignatureRef.current) {
      const recordGeneration = recoveryRecordGenerationRef.current;
      return recordGeneration === null ? null : {
        requestGeneration: recoveryRequestGenerationRef.current,
        draftGeneration: recoveryDraftGenerationRef.current,
        recordGeneration,
        omittedBindings: [],
      };
    }
    const requestGeneration = ++recoveryRequestGenerationRef.current;
    const draftGeneration = ++recoveryDraftGenerationRef.current;
    const result = await api.stageRecoveryDraft({
      sessionGeneration,
      requestGeneration,
      draftGeneration,
      displayName: savedConfigurationRef.current?.name ?? recoveredName,
      sourceConfigurationHandle: savedConfigurationRef.current?.configurationHandle ?? null,
      dirty,
      devicePlan: current.devicePlan,
      selectedRecipes: current.selectedRecipes ?? [],
      bindings: current.bindings,
    });
    if (!recoveryResultIsCurrent(result, requestGeneration, draftGeneration)) return null;
    recoveryRecordGenerationRef.current = result.recordGeneration;
    lastRecoverySignatureRef.current = signature;
    return result;
  };

  const discardCurrentRecovery = async () => {
    const recordGeneration = recoveryRecordGenerationRef.current;
    if (!recordGeneration) return;
    await api.discardRecoveryDraft(
      recoverySessionGenerationRef.current,
      recordGeneration,
    );
    recoveryRecordGenerationRef.current = null;
    lastRecoverySignatureRef.current = null;
    setRecoveredName(null);
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
      requiredReentryBindings: [],
    });
    await refreshRecents();
    const omitted = document.pendingSanitationCount ?? 0;
    setNotice(omitted > 0
      ? `Opened ${document.name}. ${omitted} stored input ${omitted === 1 ? "value was" : "values were"} not loaded because ${omitted === 1 ? "it is" : "they are"} sensitive or no longer active. Save to remove ${omitted === 1 ? "it" : "them"} from this file.`
      : `Opened ${document.name}. Connect and select the current device to validate it.`);
  };

  const applyRecoveryResult = async (result: RecoveryRestoreResult) => {
    const previous = savedConfigurationRef.current;
    if (previous && previous.configurationHandle !== result.document?.configurationHandle) {
      await api.closeSavedConfiguration(previous.configurationHandle).catch(() => undefined);
    }
    savedConfigurationRef.current = result.document;
    setSavedConfiguration(result.document);
    setRecoveredName(result.intent.dirty
      ? result.document?.name
        ? `${result.document.name} reopened after restart`
        : "Setup restored after restart"
      : null);
    dispatch({
      type: "load-portable-intent",
      devicePlan: result.intent.devicePlan,
      selectedRecipes: result.intent.selectedRecipes,
      bindings: result.intent.bindings,
      dirty: result.intent.dirty,
      requiredReentryBindings: result.intent.requiredReentryBindings,
    });
    lastRecoverySignatureRef.current = portableIntentSignature({
      devicePlan: result.intent.devicePlan,
      selectedRecipes: result.intent.selectedRecipes,
      bindings: result.intent.bindings,
    });
    await refreshRecents();
    const reentry = result.intent.requiredReentryBindings.length;
    const restoredSetupNotice = !result.intent.dirty && result.document
      ? `${result.document.name} reopened after restart. Select your device and review the setup again before continuing.`
      : result.sourceStatus === "missing"
        ? "Your setup choices were restored, but the original saved file could not be found. Save this setup to choose where it should be stored. Select your device and review the setup again before continuing."
        : "Your setup choices were restored, but they have not been saved. Select your device and review the setup again before continuing.";
    setNotice(`${restoredSetupNotice}${
      reentry > 0 ? ` Re-enter ${reentry} sensitive input${reentry === 1 ? "" : "s"}.` : ""
    }`);
  };

  const restoreRecovery = async (draftGeneration: number): Promise<boolean> => {
    const requestGeneration = ++recoveryRequestGenerationRef.current;
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const result = await api.restoreRecoveryDraft(
        recoverySessionGenerationRef.current,
        draftGeneration,
        requestGeneration,
      );
      if (
        runtimeGenerationRef.current !== runtimeGeneration
        || !recoveryResultIsCurrent(result, requestGeneration, draftGeneration)
      ) return false;
      recoveryRecordGenerationRef.current = result.draftGeneration;
      await applyRecoveryResult(result);
      return true;
    } catch (error) {
      if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
      return false;
    }
  };

  const offerRecovery = async (draft: RecoveryDraftAvailable) => {
    recoveryRecordGenerationRef.current = draft.draftGeneration;
    const decision = await requestAppDialog({ kind: "recovery", draft }, "not-now");
    if (decision === "restore" && await restoreRecovery(draft.draftGeneration)) return;
    try {
      if (decision === "discard-recovery") {
        await discardCurrentRecovery();
        setNotice("The recovery draft was discarded.");
      } else {
        await api.deferRecoveryDraft(
          recoverySessionGenerationRef.current,
          draft.draftGeneration,
        );
        setNotice("Recovery was deferred. The same draft will be offered next launch unless newer edits supersede it.");
      }
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  useEffect(() => {
    if (sessionInitializedRef.current) return;
    sessionInitializedRef.current = true;
    void (async () => {
      try {
        const session = await api.beginAppSession();
        recoverySessionGenerationRef.current = session.sessionGeneration;
        await initialize();
        if (session.recovery.state === "available") {
          await offerRecovery(session.recovery);
          if (session.interruptedSession) {
            setNotice((current) => `${current ? `${current} ` : ""}The previous session ended unexpectedly. Execution was not resumed.`);
          }
        } else if (session.recovery.state === "invalid_removed") {
          setNotice("An invalid recovery draft was removed. Start a new setup or open a saved one.");
        } else if (session.interruptedSession) {
          setNotice("The previous session ended unexpectedly. Execution was not resumed.");
        }
        setStartupReady(true);
      } catch (error) {
        setStartupReady(true);
        setRuntime({
          status: "failed",
          error: { code: "runtime_start_failed", message: errorMessage(error), actions: ["retry"] },
        });
      }
    })();
  }, [initialize]);

  useEffect(() => {
    if (!startupReady || !workflow.portableIntentDirty || !workflow.devicePlan) return;
    const timer = window.setTimeout(() => {
      void persistRecoveryNow().catch((error) => setNotice(errorMessage(error)));
    }, 350);
    return () => window.clearTimeout(timer);
  }, [
    startupReady,
    workflow.portableIntentDirty,
    workflow.devicePlan,
    workflow.selectedRecipes,
    workflow.bindings,
  ]);

  const createFromCurrentIntent = async (): Promise<boolean> => {
    const current = workflowRef.current;
    if (!current.devicePlan) {
      setNotice("Choose a setup before saving.");
      return false;
    }
    const nameResult = await requestAppDialog({
      kind: "name",
      title: "Name this setup",
      initialValue: "My EmuChef setup",
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, null);
    const name = typeof nameResult === "string" ? nameResult.trim() : "";
    if (!name) return false;
    try {
      await persistRecoveryNow(true);
      const result = await withNativeDialogFocus(() => api.createSavedConfiguration({
          name,
          devicePlan: current.devicePlan!,
          selectedRecipes: current.selectedRecipes ?? [],
          bindings: current.bindings,
        }));
      if (result.outcome === "cancelled") return false;
      savedConfigurationRef.current = result;
      setSavedConfiguration(result);
      dispatch({ type: "portable-intent-saved" });
      await refreshRecents();
      setNotice(`Saved ${result.name}.`);
      recoveryRecordGenerationRef.current = null;
      lastRecoverySignatureRef.current = null;
      setRecoveredName(null);
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
      await persistRecoveryNow(true);
      const saved = await api.saveSavedConfiguration(current.configurationHandle);
      savedConfigurationRef.current = saved;
      setSavedConfiguration(saved);
      dispatch({ type: "portable-intent-saved" });
      recoveryRecordGenerationRef.current = null;
      lastRecoverySignatureRef.current = null;
      setRecoveredName(null);
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

  const runProtectedTransition = async (transition: () => Promise<void>): Promise<boolean> => {
    const hadDirtyIntent = workflowRef.current.portableIntentDirty
      || Boolean(savedConfigurationRef.current?.dirty);
    const decision = await dirtyDecision();
    if (decision === "cancel") return false;
    if (decision === "save" && !(await saveCurrentConfiguration())) return false;
    if (decision === "discard" && hadDirtyIntent) {
      try {
        await discardCurrentRecovery();
      } catch (error) {
        setNotice(errorMessage(error));
        return false;
      }
    }
    await configurationMutationQueue.current;
    await transition();
    return true;
  };

  const startNewConfiguration = async () => {
    await runProtectedTransition(async () => {
      cancelPendingDialog();
      const current = savedConfigurationRef.current;
      if (current) await api.closeSavedConfiguration(current.configurationHandle).catch(() => undefined);
      savedConfigurationRef.current = null;
      setSavedConfiguration(null);
      dispatch({ type: "runtime-invalidated" });
      setNotice("Started a new setup.");
    });
  };

  const beginConfigurationPreview = async (
    mode: "open" | "import",
    recentHandle?: string,
  ) => {
    const generation = ++configurationPreviewGenerationRef.current;
    setConfigurationManagerOpen(true);
    setConfigurationManagerBusy(true);
    try {
      const result = await withNativeDialogFocus(() => recentHandle
        ? api.previewRecentConfiguration(recentHandle)
        : api.previewSavedConfiguration());
      if (configurationPreviewGenerationRef.current !== generation) return;
      if (result.outcome === "cancelled") return;
      const current = workflowRef.current;
      const comparison = await api.compareSavedConfigurationPreview({
        previewHandle: result.previewHandle,
        devicePlan: current.devicePlan,
        selectedRecipes: current.selectedRecipes ?? [],
        bindings: current.bindings,
      });
      if (configurationPreviewGenerationRef.current !== generation) return;
      setConfigurationPreviewMode(mode);
      setConfigurationPreview({ ...result, comparison });
    } catch (error) {
      if (configurationPreviewGenerationRef.current === generation) setNotice(errorMessage(error));
    } finally {
      if (configurationPreviewGenerationRef.current === generation) setConfigurationManagerBusy(false);
    }
  };

  const openConfiguration = async () => beginConfigurationPreview("open");

  const openRecentConfiguration = async (recentHandle: string) => {
    await beginConfigurationPreview("open", recentHandle);
  };

  const relinkRecentConfiguration = async (recentHandle: string) => {
    setConfigurationManagerBusy(true);
    try {
      await withNativeDialogFocus(() => api.relinkRecentConfiguration(recentHandle));
      setNotice("The Recent entry now points to the selected setup file.");
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      await refreshRecents();
      setConfigurationManagerBusy(false);
    }
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
      title: "Name the new setup",
      initialValue: current.name,
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, null);
    const name = typeof nameResult === "string" ? nameResult.trim() : "";
    if (!name) return;
    try {
      await persistRecoveryNow(true);
      const result = await withNativeDialogFocus(
        () => api.saveSavedConfigurationAs(current.configurationHandle, name),
      );
      if (result.outcome === "cancelled") return;
      savedConfigurationRef.current = result;
      setSavedConfiguration(result);
      dispatch({ type: "portable-intent-saved" });
      recoveryRecordGenerationRef.current = null;
      lastRecoverySignatureRef.current = null;
      setRecoveredName(null);
      await refreshRecents();
      setNotice(`Saved the new setup ${result.name}.`);
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  const requestSetupName = async (title: string, initialValue: string): Promise<string | null> => {
    const result = await requestAppDialog({
      kind: "name",
      title,
      initialValue,
      invoker: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    }, null);
    const name = typeof result === "string" ? result.trim() : "";
    return name || null;
  };

  const cancelConfigurationPreview = async () => {
    const preview = configurationPreview;
    configurationPreviewGenerationRef.current += 1;
    setConfigurationPreview(null);
    if (preview) await api.cancelSavedConfigurationPreview(preview.previewHandle).catch(() => undefined);
  };

  const applyConfigurationPreviewRepair = async (repairHandle: string) => {
    const preview = configurationPreview;
    if (!preview) return;
    const generation = ++configurationPreviewGenerationRef.current;
    setConfigurationManagerBusy(true);
    try {
      const result = await withNativeDialogFocus(
        () => api.applySavedConfigurationPreviewRepair(preview.previewHandle, repairHandle),
      );
      if (configurationPreviewGenerationRef.current !== generation || result.outcome === "cancelled") return;
      const current = workflowRef.current;
      const comparison = await api.compareSavedConfigurationPreview({
        previewHandle: result.previewHandle,
        devicePlan: current.devicePlan,
        selectedRecipes: current.selectedRecipes ?? [],
        bindings: current.bindings,
      });
      if (configurationPreviewGenerationRef.current !== generation) return;
      setConfigurationPreview({ ...result, comparison });
      setNotice("The repair was applied in memory. Save explicitly after opening to update the file.");
    } catch (error) {
      if (configurationPreviewGenerationRef.current === generation) setNotice(errorMessage(error));
    } finally {
      if (configurationPreviewGenerationRef.current === generation) setConfigurationManagerBusy(false);
    }
  };

  const confirmConfigurationPreview = async () => {
    const preview = configurationPreview;
    if (!preview || preview.compatibility.requiresRepair) return;
    setConfigurationManagerOpen(false);
    let importName: string | null = null;
    if (configurationPreviewMode === "import") {
      importName = await requestSetupName("Name the imported setup", preview.name);
      if (!importName) {
        setConfigurationManagerOpen(true);
        return;
      }
    }
    setConfigurationManagerBusy(true);
    const completed = await runProtectedTransition(async () => {
      if (configurationPreviewMode === "import") {
        const result = await withNativeDialogFocus(
          () => api.importSavedConfiguration(preview.previewHandle, importName!),
        );
        if (result.outcome !== "cancelled") await applySavedDocument(result);
      } else {
        await applySavedDocument(
          await api.confirmSavedConfigurationPreview(preview.previewHandle),
        );
      }
    }).catch((error) => {
      setNotice(errorMessage(error));
      return false;
    });
    setConfigurationManagerBusy(false);
    if (completed) {
      setConfigurationPreview(null);
    } else {
      setConfigurationManagerOpen(true);
    }
  };

  const renameCurrentConfiguration = async () => {
    const current = savedConfigurationRef.current;
    if (!current) return;
    setConfigurationManagerOpen(false);
    const name = await requestSetupName("Rename this setup", current.name);
    if (!name) {
      setConfigurationManagerOpen(true);
      return;
    }
    setConfigurationManagerBusy(true);
    try {
      await configurationMutationQueue.current;
      const renamed = await api.renameSavedConfiguration(current.configurationHandle, name);
      savedConfigurationRef.current = renamed;
      setSavedConfiguration(renamed);
      await refreshRecents();
      setNotice(`Renamed the setup to ${renamed.name}.`);
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setConfigurationManagerBusy(false);
      setConfigurationManagerOpen(true);
    }
  };

  const duplicateCurrentConfiguration = async () => {
    const current = savedConfigurationRef.current;
    if (!current) return;
    setConfigurationManagerOpen(false);
    const name = await requestSetupName("Name the duplicate setup", `${current.name} copy`);
    if (!name) {
      setConfigurationManagerOpen(true);
      return;
    }
    setConfigurationManagerBusy(true);
    try {
      await configurationMutationQueue.current;
      const result = await withNativeDialogFocus(
        () => api.duplicateSavedConfiguration(current.configurationHandle, name),
      );
      if (result.outcome === "saved") {
        await refreshRecents();
        setNotice(`Duplicated ${current.name} as ${result.name}.`);
      }
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setConfigurationManagerBusy(false);
      setConfigurationManagerOpen(true);
    }
  };

  const exportCurrentConfiguration = async () => {
    const current = savedConfigurationRef.current;
    if (!current) return;
    setConfigurationManagerOpen(false);
    const name = await requestSetupName("Name the exported setup", `${current.name} export`);
    if (!name) {
      setConfigurationManagerOpen(true);
      return;
    }
    setConfigurationManagerBusy(true);
    try {
      await configurationMutationQueue.current;
      const result = await withNativeDialogFocus(
        () => api.exportSavedConfiguration(current.configurationHandle, name),
      );
      if (result.outcome === "saved") setNotice(`Exported ${result.name}.`);
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setConfigurationManagerBusy(false);
      setConfigurationManagerOpen(true);
    }
  };

  const removeRecentConfiguration = async (recentHandle: string) => {
    setConfigurationManagerBusy(true);
    try {
      await api.removeRecentConfiguration(recentHandle);
      await refreshRecents();
      setNotice("Removed the setup from Recents. The setup file was not deleted.");
    } catch (error) {
      setNotice(errorMessage(error));
    } finally {
      setConfigurationManagerBusy(false);
    }
  };

  const queueSavedMutation = (mutation: SavedConfigurationMutation) => {
    if (!savedConfigurationRef.current) return;
    const runtimeGeneration = runtimeGenerationRef.current;
    configurationMutationQueue.current = configurationMutationQueue.current
      .then(async () => {
        if (runtimeGenerationRef.current !== runtimeGeneration) return;
        const current = savedConfigurationRef.current;
        if (!current) return;
        const updated = await api.updateSavedConfiguration(
          current.configurationHandle,
          current.revision,
          mutation,
        );
        if (runtimeGenerationRef.current !== runtimeGeneration) return;
        if (savedConfigurationRef.current?.configurationHandle !== updated.configurationHandle) return;
        savedConfigurationRef.current = updated;
        setSavedConfiguration(updated);
      })
      .catch((error) => {
        if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
      });
  };

  const updateDevicePlanIntent = (
    devicePlan: string,
    recipeSelection: "defaults" | "blank" = "defaults",
  ) => {
    dispatch({ type: "select-plan", devicePlan, recipeSelection });
    queueSavedMutation({ kind: "device_plan", value: devicePlan });
    if (recipeSelection === "blank") {
      queueSavedMutation({ kind: "selected_recipes", value: [] });
    }
  };

  const updateRecipeIntent = (selectedRecipes: string[]) => {
    setTouchedInputKeys(new Set());
    setValidationRequested(false);
    dispatch({ type: "set-recipes", selectedRecipes });
    queueSavedMutation({ kind: "selected_recipes", value: selectedRecipes });
  };

  const updateBindingIntent = (key: string, value: unknown) => {
    setTouchedInputKeys((current) => {
      if (current.has(key)) return current;
      const next = new Set(current);
      next.add(key);
      return next;
    });
    dispatch({ type: "set-binding", key, value });
    queueSavedMutation({ kind: "binding", key, value });
  };

  const clearBindingIntent = (key: string) => {
    setTouchedInputKeys((current) => {
      if (current.has(key)) return current;
      const next = new Set(current);
      next.add(key);
      return next;
    });
    dispatch({ type: "remove-binding", key });
    queueSavedMutation({ kind: "remove_binding", key });
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

  const restartRuntime = async (
    invoker: HTMLElement | null = null,
    expectedGeneration?: number,
  ) => {
    const current = workflowRef.current;
    let recoveryAck: RecoveryWriteAck | null = null;
    try {
      recoveryAck = await persistRecoveryNow(true, true);
    } catch (error) {
      const confirmed = await requestAppDialog({
        kind: "restart-loss",
        invoker,
        labels: [],
        omittedCount: 0,
        totalLoss: true,
      }, false);
      if (confirmed !== true) {
        setNotice(errorMessage(error));
        return;
      }
    }
    if (recoveryAck?.omittedBindings.length) {
      const omitted = new Set(recoveryAck.omittedBindings);
      const labels = (current.description?.inputs ?? [])
        .filter((input) => omitted.has(input.key))
        .map((input) => input.label)
        .filter((label, index, all) => all.indexOf(label) === index)
        .sort();
      const confirmed = await requestAppDialog({
        kind: "restart-loss",
        invoker,
        labels,
        omittedCount: recoveryAck.omittedBindings.length,
        totalLoss: false,
      }, false);
      if (confirmed !== true) return;
    }
    const runtimeGeneration = ++runtimeGenerationRef.current;
    platformToolsGenerationRef.current += 1;
    executionCapabilitiesGenerationRef.current += 1;
    devicePollGenerationRef.current += 1;
    deviceSelectionGenerationRef.current += 1;
    rootCheckGenerationRef.current += 1;
    setRootCheckPhase("idle");
    supportGenerationRef.current += 1;
    if (deviceRefreshTimerRef.current !== null) {
      window.clearTimeout(deviceRefreshTimerRef.current);
      deviceRefreshTimerRef.current = null;
    }
    setPlatformToolsOperation({ phase: "idle", kind: "import" });
    setRepairPreparing(false);
    resetExecutionPresentation();
    setRealConfirmation(emptyRealExecutionConfirmation);
    manualDeviceRefreshRef.current = false;
    setDeviceRefresh({ phase: "idle", generation: 0, message: null });
    setBusy(true);
    try {
      cancelPendingDialog();
      announce("Restarting the local app service.");
      const status = await api.restartRuntime(expectedGeneration);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      savedConfigurationRef.current = null;
      setSavedConfiguration(null);
      dispatch({ type: "runtime-invalidated" });
      setRuntime(status);
      supportDispatch({ type: "runtime-restarted" });
      await initialize(runtimeGeneration);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      if (recoveryAck) {
        await restoreRecovery(recoveryAck.recordGeneration);
      } else {
        setNotice("App service restarted. Select and validate a device before continuing.");
      }
      if (support.open) await refreshSupportInventory();
    } catch (error) {
      if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
    } finally {
      if (runtimeGenerationRef.current === runtimeGeneration) setBusy(false);
    }
  };

  const refreshSupportInventory = async () => {
    const generation = ++supportGenerationRef.current;
    supportDispatch({ type: "inventory-requested", generation });
    announce("Refreshing troubleshooting and app-owned storage status.");
    try {
      const snapshot = await api.supportSnapshot();
      supportDispatch({ type: "snapshot-loaded", generation, snapshot });
      announce(snapshot.overallSummary);
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

  const openUpdates = async (invoker: HTMLElement) => {
    if (dialogController.activeId !== null || support.open) {
      announce("Close the current panel or confirmation before opening Updates.");
      return;
    }
    updatesInvokerRef.current = invoker;
    setUpdates((current) => ({ ...current, open: true }));
    try {
      const status = await api.getUpdateStatus();
      setUpdates((current) => ({ ...current, status }));
    } catch (error) {
      announce(errorMessage(error), true);
    }
  };

  const checkForUpdates = async () => {
    setUpdates((current) => ({ ...current, checking: true }));
    try {
      const status = await api.checkForUpdates();
      setUpdates((current) => ({ ...current, checking: false, status }));
    } catch (error) {
      setUpdates((current) => ({ ...current, checking: false }));
      announce(errorMessage(error), true);
    }
  };

  const openUpdateDownload = async () => {
    setUpdates((current) => ({ ...current, opening: true }));
    try {
      await api.openUpdateDownload();
      announce("The validated DMG address was opened in your default browser.");
    } catch (error) {
      announce(errorMessage(error), true);
    } finally {
      setUpdates((current) => ({ ...current, opening: false }));
    }
  };

  const runSupportCorrectiveAction = async (
    action: CorrectiveAction,
    _invoker: HTMLElement,
  ) => {
    switch (action.kind) {
      case "restart_service":
        supportDispatch({ type: "close" });
        await restartRuntime(supportInvokerRef.current, action.serviceGeneration);
        return;
      case "import_managed_platform_tools":
      case "replace_managed_platform_tools":
        supportDispatch({ type: "close" });
        await importPlatformTools(action.platformToolsRevision);
        return;
      case "remove_managed_platform_tools":
        supportDispatch({ type: "close" });
        await removePlatformTools(supportInvokerRef.current, action.platformToolsRevision);
        return;
      case "refresh_devices":
        await pollDevices(true, false, action.deviceGeneration);
        await refreshSupportInventory();
        return;
      case "refresh_cache":
        await refreshSupportInventory();
        return;
      case "open_updates":
        supportDispatch({ type: "close" });
        updatesInvokerRef.current = supportInvokerRef.current;
        setUpdates((current) => ({ ...current, open: true }));
        try {
          const status = await api.getUpdateStatus();
          setUpdates((current) => ({ ...current, status }));
        } catch (error) {
          announce(errorMessage(error), true);
        }
        return;
      case "open_saved_setup_repair":
        supportDispatch({ type: "close" });
        openConfigurationManager(supportInvokerRef.current);
        return;
      default:
        announce("This troubleshooting action is not supported by this version of EmuChef.", true);
    }
  };

  const resetLocalAppState = async (category: ResetLocalStateCategory) => {
    if (!category.resetHandle) return;
    const generation = ++supportGenerationRef.current;
    supportDispatch({ type: "cleanup-started", generation });
    announce(`${category.label} started.`);
    try {
      const result = await api.resetLocalAppState(category.resetHandle);
      supportDispatch({ type: "snapshot-loaded", generation, snapshot: result.snapshot });
      if (category.id === "recents") await refreshRecents();
      announce(result.outcome.summary);
    } catch (error) {
      supportDispatch({ type: "cleanup-failed", generation, message: errorMessage(error) });
      announce(errorMessage(error), true);
    }
  };

  const prepareSupportCleanup = (mode: CacheCleanupMode) => {
    if (!support.inventory) return null;
    const entries = entriesForCleanup(support.inventory, mode, support.selectedHandles);
    const confirmation = cleanupConfirmation(entries);
    if (confirmation.entryCount === 0) {
      supportDispatch({
        type: "cleanup-failed",
        generation: support.requestGeneration,
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
      supportDispatch({ type: "cleanup-failed", generation, message: errorMessage(error) });
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
      event.preventDefault();
      const dirty = workflowRef.current.portableIntentDirty
        || Boolean(savedConfigurationRef.current?.dirty);
      if (dirty) {
        try {
          await persistRecoveryNow(true);
        } catch (error) {
          setNotice(errorMessage(error));
          return;
        }
        const decision = await dirtyDecision();
        if (decision === "cancel") return;
        if (decision === "save" && !(await saveCurrentConfiguration())) return;
        if (decision === "discard") {
          try {
            await discardCurrentRecovery();
          } catch (error) {
            setNotice(errorMessage(error));
            return;
          }
        }
      }
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

  const pollDevices = useCallback(async (
    manual = false,
    suppressSelectedDeviceLoss = false,
    expectedSupportGeneration?: number,
  ): Promise<DeviceSummary[] | null> => {
    if (adb?.status !== "ready" || runtime.status !== "ready") return null;
    if (manualDeviceRefreshRef.current) return null;
    const generation = ++devicePollGenerationRef.current;
    const runtimeGeneration = runtimeGenerationRef.current;
    if (manual) {
      manualDeviceRefreshRef.current = true;
      setDeviceRefresh({ phase: "refreshing", generation, message: null });
      announce("Refreshing connected devices.");
    }
    try {
      const next = await api.pollDevices(expectedSupportGeneration);
      const qualification = await api.deviceQualification(
        next.length === 1 ? next[0].deviceHandle : null,
      );
      if (
        devicePollGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return null;
      setDevices(next);
      setDeviceQualification(qualification);
      rootCheckGenerationRef.current += 1;
      setRootCheckPhase("idle");
      const current = workflowRef.current;
      const selected = current.deviceHandle
        ? next.find((device) => device.deviceHandle === current.deviceHandle)
        : null;
      if (current.deviceHandle && !suppressSelectedDeviceLoss && selected?.state !== "available") {
        const portable = portableBindingsForTransition(current.description, current.bindings);
        dispatch({
          type: "device-disappeared",
          bindings: portable.bindings,
          requiredReentryBindings: portable.requiredReentryBindings,
        });
        setNotice(selected?.state === "unauthorized"
          ? "The selected device needs authorization. Unlock it, accept the USB debugging prompt, and refresh."
          : selected?.state === "offline"
            ? "The selected device is offline. Reconnect it and refresh to continue."
            : "The selected device disconnected. Connect the same device to restore your setup choices.");
      }
      if (manual) {
        const message = next.length === 0
          ? "Refresh complete. No devices found."
          : `Refresh complete. ${next.length} device${next.length === 1 ? "" : "s"} found.`;
        setDeviceRefresh({ phase: "complete", generation, message });
        announce(message);
        if (deviceRefreshTimerRef.current !== null) {
          window.clearTimeout(deviceRefreshTimerRef.current);
        }
        deviceRefreshTimerRef.current = window.setTimeout(() => {
          setDeviceRefresh((state) => state.generation === generation
            ? { phase: "idle", generation, message: null }
            : state);
          deviceRefreshTimerRef.current = null;
        }, 5000);
      }
      return next;
    } catch (error) {
      if (
        devicePollGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) {
        setNotice(errorMessage(error));
        if (manual) setDeviceRefresh({ phase: "idle", generation, message: null });
      }
      return null;
    } finally {
      if (
        manual
        && devicePollGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) manualDeviceRefreshRef.current = false;
    }
  }, [adb?.status, announce, runtime.status]);

  const checkDeviceRoot = useCallback(async () => {
    const candidate = devices.length === 1 && devices[0].state === "available"
      ? devices[0].deviceHandle
      : null;
    if (!candidate || deviceQualification?.state !== "supported") return;
    const checkGeneration = ++rootCheckGenerationRef.current;
    const runtimeGeneration = runtimeGenerationRef.current;
    const pollGeneration = devicePollGenerationRef.current;
    setRootCheckPhase("checking");
    try {
      const result = await api.checkDeviceRoot(candidate);
      if (
        rootCheckGenerationRef.current !== checkGeneration
        || devicePollGenerationRef.current !== pollGeneration
        || runtimeGenerationRef.current !== runtimeGeneration
        || result.deviceIdentity !== candidate
      ) return;
      setDeviceQualification((current) => current && current.deviceIdentity === candidate
        ? { ...current, root: result.qualification }
        : current);
    } catch (error) {
      if (
        rootCheckGenerationRef.current === checkGeneration
        && devicePollGenerationRef.current === pollGeneration
        && runtimeGenerationRef.current === runtimeGeneration
      ) setNotice(errorMessage(error));
    } finally {
      if (rootCheckGenerationRef.current === checkGeneration) setRootCheckPhase("idle");
    }
  }, [deviceQualification?.state, devices]);

  useEffect(() => {
    void pollDevices();
    const timer = window.setInterval(() => void pollDevices(), 2500);
    return () => window.clearInterval(timer);
  }, [pollDevices]);

  const importPlatformTools = async (expectedRevision?: number) => {
    if (platformToolsOperation.phase !== "idle") return;
    const kind = adb?.status === "ready" ? "replace" : "import";
    const generation = ++platformToolsGenerationRef.current;
    const runtimeGeneration = runtimeGenerationRef.current;
    const prior = workflowRef.current;
    setNotice(null);
    setPlatformToolsOperation({ phase: "picker", kind });
    try {
      const picked = await withNativeDialogFocus(api.pickPlatformToolsZip);
      if (
        platformToolsGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      if (picked.outcome === "cancelled") return;
      setPlatformToolsOperation({ phase: "processing", kind });
      const status = await api.installPlatformToolsSelection(picked.selectionHandle, expectedRevision);
      if (
        platformToolsGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      devicePollGenerationRef.current += 1;
      deviceSelectionGenerationRef.current += 1;
      rootCheckGenerationRef.current += 1;
      setRootCheckPhase("idle");
      manualDeviceRefreshRef.current = false;
      setDeviceRefresh({ phase: "idle", generation: 0, message: null });
      setAdb(status);
      await refreshExecutionCapabilities();
      if (
        platformToolsGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      if (kind === "replace") {
        const portable = portableBindingsForTransition(prior.description, prior.bindings);
        setDevices([]);
        dispatch({
          type: "device-disappeared",
          bindings: portable.bindings,
          requiredReentryBindings: portable.requiredReentryBindings,
        });
        const refreshed = await pollDevices(false, true);
        if (
          platformToolsGenerationRef.current !== generation
          || runtimeGenerationRef.current !== runtimeGeneration
        ) return;
        const sameAvailable = Boolean(prior.deviceHandle && refreshed?.some((device) => (
          device.deviceHandle === prior.deviceHandle && device.state === "available"
        )));
        setNotice(sameAvailable
          ? "Platform-Tools replaced. Your device was rediscovered; select it to validate the preserved setup again."
          : "Platform-Tools replaced. Device detection was refreshed; connect and select the intended device to continue.");
      } else {
        setNotice("Platform-Tools installed. Connect a device to continue.");
      }
      if (support.open) await refreshSupportInventory();
    } catch (error) {
      if (
        platformToolsGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) {
        setNotice(errorMessage(error));
        const status = await api.adbStatus().catch(() => null);
        if (
          status
          && platformToolsGenerationRef.current === generation
          && runtimeGenerationRef.current === runtimeGeneration
        ) setAdb(status);
      }
    } finally {
      if (
        platformToolsGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) setPlatformToolsOperation({ phase: "idle", kind });
    }
  };

  const openPlatformToolsPage = async () => {
    setNotice(null);
    try {
      await api.openPlatformToolsPage();
    } catch (error) {
      setNotice(errorMessage(error));
    }
  };

  const removePlatformTools = async (invoker: HTMLElement | null, expectedRevision?: number) => {
    if (platformToolsOperation.phase !== "idle") return;
    const confirmed = await requestAppDialog({
      kind: "remove-platform-tools",
      invoker,
    }, false);
    if (confirmed !== true) return;
    const generation = ++platformToolsGenerationRef.current;
    const runtimeGeneration = runtimeGenerationRef.current;
    const current = workflowRef.current;
    setPlatformToolsOperation({ phase: "processing", kind: "remove" });
    try {
      const status = await api.removePlatformTools(expectedRevision);
      if (
        platformToolsGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      devicePollGenerationRef.current += 1;
      deviceSelectionGenerationRef.current += 1;
      rootCheckGenerationRef.current += 1;
      setRootCheckPhase("idle");
      manualDeviceRefreshRef.current = false;
      setDeviceRefresh({ phase: "idle", generation: 0, message: null });
      setAdb(status);
      await refreshExecutionCapabilities();
      if (
        platformToolsGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      setDevices([]);
      const portable = portableBindingsForTransition(current.description, current.bindings);
      dispatch({
        type: "device-disappeared",
        bindings: portable.bindings,
        requiredReentryBindings: portable.requiredReentryBindings,
      });
      setNotice("Platform-Tools removed. Device detection is unavailable until you install it again; then select your device and review a fresh plan.");
      if (support.open) await refreshSupportInventory();
    } catch (error) {
      if (
        platformToolsGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) setNotice(errorMessage(error));
    } finally {
      if (
        platformToolsGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) setPlatformToolsOperation({ phase: "idle", kind: "remove" });
    }
  };

  const selectDevice = async (deviceHandle: string, invoker: HTMLElement | null = null) => {
    if (savedConfigurationBlocksProgress(savedConfigurationRef.current)) {
      setNotice("Repair or replace the incompatible saved setup before selecting a device.");
      announce("This saved setup must be repaired before continuing.", true);
      return;
    }
    const before = workflowRef.current;
    const sameReconnect = before.reconnectDeviceHandle === deviceHandle;
    if (before.reconnectDeviceHandle && !sameReconnect) {
      const confirmed = await requestAppDialog({ kind: "different-device", invoker }, false);
      if (confirmed !== true) return;
    }
    const generation = ++deviceSelectionGenerationRef.current;
    rootCheckGenerationRef.current += 1;
    setRootCheckPhase("idle");
    const runtimeGeneration = runtimeGenerationRef.current;
    dispatch({ type: "select-device", deviceHandle, preserveIntent: sameReconnect });
    setBusy(true);
    setNotice(null);
    announce("Reading the selected device properties.");
    try {
      const [facts, match] = await Promise.all([
        api.probeDevice(deviceHandle),
        api.matchDevice(deviceHandle),
      ]);
      if (
        deviceSelectionGenerationRef.current !== generation
        || runtimeGenerationRef.current !== runtimeGeneration
      ) return;
      dispatch({ type: "device-probed", facts, match });
      announce(deviceIsUnsupported(match)
        ? "The connected device is not officially supported. Review the available safe generic options."
        : "Device properties loaded. Confirm the matched setup.");
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
      if (
        deviceSelectionGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) {
        setNotice(errorMessage(error));
        const portable = portableBindingsForTransition(before.description, before.bindings);
        dispatch({
          type: "device-disappeared",
          bindings: portable.bindings,
          requiredReentryBindings: portable.requiredReentryBindings,
        });
      }
    } finally {
      if (
        deviceSelectionGenerationRef.current === generation
        && runtimeGenerationRef.current === runtimeGeneration
      ) setBusy(false);
    }
  };

  const describe = async () => {
    if (!workflow.deviceHandle || !workflow.devicePlan) return;
    setValidationRequested(true);
    setBusy(true);
    setNotice(null);
    setOperationError(null);
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const generation = workflow.requestGeneration;
      const description = await api.describeConfiguration({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
        requestGeneration: generation,
      });
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      dispatch({ type: "description", description, generation });
      if (workflowRef.current.requestGeneration === generation) {
        if (workflow.selectedRecipes === null) {
          queueSavedMutation({ kind: "selected_recipes", value: description.selectedRecipes });
        }
        applyDescriptionValidation(description);
        focusValidationSummary(description);
        const errorCount = [
          ...description.diagnostics,
          ...description.inputs.flatMap((input) => input.diagnostics),
        ].filter((item) => item.severity === "error").length;
        announce(errorCount > 0
          ? `Validation needs attention. ${errorCount} ${errorCount === 1 ? "error" : "errors"} found.`
          : "Validation complete. The setup is ready for review.", errorCount > 0);
      } else {
        announce("An outdated validation response was ignored.");
      }
    } catch (error) {
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      const message = errorMessage(error);
      setNotice(message);
      setOperationError(message);
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({ preferred: [validationSummaryRef.current], generation }));
    } finally {
      if (runtimeGenerationRef.current === runtimeGeneration) setBusy(false);
    }
  };

  const continueToInputs = async () => {
    if (!workflow.deviceHandle || !workflow.devicePlan || !(workflow.selectedRecipes?.length)) return;
    setValidationRequested(false);
    setBusy(true);
    setNotice(null);
    setOperationError(null);
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const generation = workflow.requestGeneration;
      const description = await api.describeConfiguration({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
        requestGeneration: generation,
      });
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      dispatch({ type: "description", description, generation });
      if (workflowRef.current.requestGeneration !== generation) {
        announce("An outdated validation response was ignored.");
        return;
      }
      applyDescriptionValidation(description);
      if (description.selectedRecipes.length === 0) {
        announce("Choose at least one recipe before continuing.", true);
        return;
      }
      dispatch({ type: "continue-to-inputs" });
      announce(description.inputs.length > 0
        ? "Recipe selection validated. Continue with setup inputs."
        : "Recipe selection validated. No additional setup inputs are required.");
    } catch (error) {
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      const message = errorMessage(error);
      setNotice(message);
      setOperationError(message);
    } finally {
      if (runtimeGenerationRef.current === runtimeGeneration) setBusy(false);
    }
  };

  useEffect(() => {
    if (
      (workflow.step !== "recipes" && workflow.step !== "inputs") ||
      !workflow.descriptionDirty ||
      !workflow.deviceHandle ||
      !workflow.devicePlan
    ) return;
    const generation = workflow.requestGeneration;
    const runtimeGeneration = runtimeGenerationRef.current;
    const timer = window.setTimeout(() => {
      api.describeConfiguration({
        deviceHandle: workflow.deviceHandle!,
        devicePlan: workflow.devicePlan!,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
        requestGeneration: generation,
      }).then((description) => {
        if (runtimeGenerationRef.current !== runtimeGeneration) return;
        dispatch({ type: "description", description, generation });
        if (workflowRef.current.requestGeneration === generation) {
          applyDescriptionValidation(description);
        } else {
          announce("An outdated validation response was ignored.");
        }
      }).catch((error) => {
        if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
      });
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
    setValidationRequested(true);
    if (!reviewReady(workflow)) {
      if (workflow.description) focusValidationSummary(workflow.description);
      announce(
        "Resolve required values and validation errors before review.",
        true,
      );
      return;
    }
    setBusy(true);
    setNotice(null);
    setOperationError(null);
    announce("Creating a fresh reviewed plan.");
    const requestGeneration = workflow.requestGeneration;
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const review = await api.createReview({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
        requestGeneration,
      });
      if (
        runtimeGenerationRef.current !== runtimeGeneration
        || workflowRef.current.requestGeneration !== requestGeneration
      ) return;
      dispatch({ type: "review", review });
      announce("The reviewed plan is ready.");
    } catch (error) {
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      const message = errorMessage(error);
      setNotice(message);
      setOperationError(message);
      const generation = claimFocusTransition();
      queueMicrotask(() => restoreAccessibleFocus({
        preferred: [validationSummaryRef.current],
        generation,
      }));
    } finally {
      if (runtimeGenerationRef.current === runtimeGeneration) setBusy(false);
    }
  };

  const requestRealExecution = async (invoker: HTMLElement) => {
    setRealConfirmation(emptyRealExecutionConfirmation);
    const result = await requestAppDialog({
      kind: "real-execution",
      invoker,
    }, false);
    if (result === true) {
      const confirmation = realConfirmationRef.current;
      realConfirmationRef.current = emptyRealExecutionConfirmation;
      setRealConfirmation(emptyRealExecutionConfirmation);
      await startRealExecution(confirmation);
    }
  };

  const prepareRepair = async () => {
    if (repairPreparing) return;
    const prior = workflow;
    const runtimeGeneration = runtimeGenerationRef.current;
    setRepairPreparing(true);
    resetExecutionPresentation();
    cancelPendingDialog();
    setRealConfirmation(emptyRealExecutionConfirmation);
    dispatch({ type: "prepare-repair" });
    setNotice(null);
    try {
      if (prior.review) {
        await api.discardReview(prior.review.reviewHandle).catch(() => undefined);
      }
      const [freshCatalog, freshDevices] = await Promise.all([api.catalog(), api.pollDevices()]);
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
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
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      if (deviceIsUnsupported(match)) {
        dispatch({ type: "select-device", deviceHandle: prior.deviceHandle, preserveIntent: true });
        dispatch({ type: "device-probed", facts, match });
        setNotice("This device is not officially supported. Acknowledge that any offered generic setup is not device-specific before choosing one.");
        return;
      }
      const devicePlan = prior.devicePlan && match.candidates.some((plan) => plan.planId === prior.devicePlan)
        ? prior.devicePlan
        : match.recommendedPlanId;
      if (!devicePlan) {
        dispatch({ type: "select-device", deviceHandle: prior.deviceHandle });
        dispatch({ type: "device-probed", facts, match });
        setNotice("Choose a current device setup before preparing this setup again.");
        return;
      }
      const catalogRecipes = new Set(freshCatalog.recipes.map((recipe) => recipe.id));
      const selectedRecipes = (prior.selectedRecipes ?? []).filter((recipe) => catalogRecipes.has(recipe));
      const baseline = await api.describeConfiguration({
        deviceHandle: prior.deviceHandle,
        devicePlan,
        selectedRecipes,
        bindings: {},
        requestGeneration: prior.requestGeneration,
      });
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
      const bindings = filterRepairBindings(prior.description, baseline, prior.bindings);
      const description = await api.describeConfiguration({
        deviceHandle: prior.deviceHandle,
        devicePlan,
        selectedRecipes,
        bindings,
        requestGeneration: prior.requestGeneration,
      });
      if (runtimeGenerationRef.current !== runtimeGeneration) return;
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
      setNotice("Setup refreshed. Resolve any remaining issues, then create and review a new plan.");
    } catch (error) {
      if (runtimeGenerationRef.current === runtimeGeneration) setNotice(errorMessage(error));
    } finally {
      if (runtimeGenerationRef.current === runtimeGeneration) setRepairPreparing(false);
    }
  };

  const pickInputValue = async (
    input: InputDescriptor,
    mode: "replace_all" | "append" | "replace_entry" = "replace_all",
    entryIndex: number | null = null,
  ) => {
    if (!input.pathKind) return;
    setNotice(null);
    setBusy(true);
    const operationGeneration = ++inputOperationGenerationRef.current;
    const requestGeneration = workflowRef.current.requestGeneration;
    const runtimeGeneration = runtimeGenerationRef.current;
    try {
      const value = await withNativeDialogFocus(
        () => api.pickInputPath({
          inputKey: input.key,
          requestGeneration,
          mode,
          currentValue: workflowRef.current.bindings[input.key] ?? input.value ?? null,
          entryIndex,
        }),
        [document.getElementById(stableDomId("input", input.key))],
      );
      const current = workflowRef.current;
      if (
        runtimeGenerationRef.current !== runtimeGeneration
        || current.requestGeneration !== requestGeneration
        || current.step !== "inputs"
        || !current.description?.inputs.some((candidate) => candidate.key === input.key)
      ) {
        announce("An outdated file selection was ignored.");
        return;
      }
      if (value !== null) updateBindingIntent(input.key, value);
    } catch (error) {
      if (
        runtimeGenerationRef.current === runtimeGeneration
        && workflowRef.current.requestGeneration === requestGeneration
      ) setNotice(errorMessage(error));
    } finally {
      if (inputOperationGenerationRef.current === operationGeneration) setBusy(false);
    }
  };

  const removeInputEntry = (input: InputDescriptor, entryIndex: number) => {
    const value = workflowRef.current.bindings[input.key] ?? input.value;
    if (!Array.isArray(value)) {
      clearBindingIntent(input.key);
      return;
    }
    const next = value.filter((_, index) => index !== entryIndex);
    if (next.length === 0) clearBindingIntent(input.key);
    else updateBindingIntent(input.key, next);
  };

  const stepIndex = WORKFLOW_STEPS.findIndex((item) => item.step === workflow.step);
  const multipleDevicesConnected = devices.length > 1;
  const [recipeSearch, setRecipeSearch] = useState("");
  const [recipeFilter, setRecipeFilter] = useState<"all" | "available" | "selected" | "unavailable">("all");
  const selectedRecipeIds = useMemo(
    () => new Set(workflow.selectedRecipes ?? []),
    [workflow.selectedRecipes],
  );
  const filteredRecipeOptions = useMemo(() => {
    const query = recipeSearch.trim().toLocaleLowerCase();
    return (workflow.description?.recipeOptions ?? []).filter((recipe) => {
      const selected = selectedRecipeIds.has(recipe.id) || recipe.dependencyRequired;
      const matchesFilter = recipeFilter === "all"
        || (recipeFilter === "available" && recipe.available)
        || (recipeFilter === "selected" && selected)
        || (recipeFilter === "unavailable" && !recipe.available);
      if (!matchesFilter) return false;
      if (!query) return true;
      return [recipe.name, recipe.description ?? "", recipe.id]
        .some((value) => value.toLocaleLowerCase().includes(query));
    });
  }, [recipeFilter, recipeSearch, selectedRecipeIds, workflow.description]);
  const selectedRecipeOptions = useMemo(
    () => (workflow.description?.recipeOptions ?? []).filter(
      (recipe) => selectedRecipeIds.has(recipe.id) || recipe.dependencyRequired,
    ),
    [selectedRecipeIds, workflow.description],
  );
  const recommendedRecipeIds = useMemo(
    () => (workflow.description?.recipeOptions ?? [])
      .filter((recipe) => recipe.recommended && recipe.available && !recipe.dependencyRequired)
      .map((recipe) => recipe.id),
    [workflow.description],
  );
  const selectedOptionalRecipeIds = useMemo(
    () => (workflow.description?.recipeOptions ?? [])
      .filter((recipe) => !recipe.dependencyRequired && selectedRecipeIds.has(recipe.id))
      .map((recipe) => recipe.id),
    [selectedRecipeIds, workflow.description],
  );
  const recommendedSelectionActive = recommendedRecipeIds.length > 0
    && recommendedRecipeIds.length === selectedOptionalRecipeIds.length
    && recommendedRecipeIds.every((recipeId) => selectedRecipeIds.has(recipeId));
  const planOptions = useMemo(() => {
    if (!workflow.match) return [];
    if (deviceIsUnsupported(workflow.match)) {
      return workflow.unsupportedAcknowledged ? workflow.match.safeGenericPlans : [];
    }
    const byPlanId = new Map(
      [...workflow.match.candidates, ...workflow.match.safeGenericPlans]
        .map((plan) => [plan.planId, plan] as const),
    );
    return [...byPlanId.values()];
  }, [workflow.match, workflow.unsupportedAcknowledged]);
  const platformToolsBusy = platformToolsOperation.phase !== "idle";
  const blankPlanOptions = useMemo(() => {
    if (!workflow.match?.blankSetupPlans?.length) return [];
    const visiblePlanIds = new Set(planOptions.map((plan) => plan.planId));
    return workflow.match.blankSetupPlans.filter((plan) => visiblePlanIds.has(plan.planId));
  }, [planOptions, workflow.match]);
  const savedPlanUnavailable = Boolean(
    workflow.savedIntentLoaded
      && workflow.match
      && (!workflow.devicePlan
        || !planOptions.some((candidate) => candidate.planId === workflow.devicePlan)),
  );
  const validationErrors = workflow.description
    ? [
        ...workflow.description.inputs.flatMap((input) => inputDiagnosticsForDisplay(input)
          .filter((diagnostic) => diagnosticIsBlocking(
            diagnostic.code,
            diagnostic.key ?? input.key,
            validationRequested,
            touchedInputKeys,
          ))
          .map((diagnostic) => ({
            ...diagnostic,
            targetId: stableDomId("input", input.key),
          }))),
        ...pageDiagnosticsForDisplay(workflow.description)
          .filter((diagnostic) => diagnosticIsBlocking(
            diagnostic.code,
            diagnostic.key,
            validationRequested,
            touchedInputKeys,
          ))
          .map((diagnostic) => ({
            ...diagnostic,
            targetId: null,
          })),
      ].filter((diagnostic) => diagnostic.severity === "error")
    : [];
  const configurationActionsLocked = !startupReady
    || busy
    || repairPreparing
    || platformToolsBusy
    || workflow.execution.kind === "active"
    || workflow.execution.kind === "starting";
  const saveDisabled = busy
    || platformToolsBusy
    || !workflow.devicePlan
    || (!workflow.portableIntentDirty
      && !savedConfiguration?.dirty
      && !(savedConfiguration?.pendingSanitationCount));
  const savedConfigurationBlocked = savedConfigurationBlocksProgress(savedConfiguration);
  const saveDisabledReason = saveConfigurationDisabledReason(
    savedConfiguration,
    Boolean(workflow.devicePlan),
    workflow.portableIntentDirty
      || Boolean(savedConfiguration?.dirty)
      || Boolean(savedConfiguration?.pendingSanitationCount),
  );

  useEffect(() => {
    void api.updateSavedConfigurationMenu({
      runtimeReady: startupReady && runtime.status === "ready",
      commandBlocked: configurationActionsLocked || configurationManagerBusy,
      hasDocument: savedConfiguration !== null,
      dirty: workflow.portableIntentDirty
        || Boolean(savedConfiguration?.dirty)
        || Boolean(savedConfiguration?.pendingSanitationCount),
      hasPortableIntent: Boolean(workflow.devicePlan),
    }).catch(() => undefined);
  }, [
    configurationActionsLocked,
    configurationManagerBusy,
    recentConfigurations,
    runtime.status,
    savedConfiguration,
    startupReady,
    workflow.devicePlan,
    workflow.portableIntentDirty,
  ]);

  const openConfigurationManager = (invoker?: HTMLElement | null) => {
    configurationManagerInvokerRef.current = invoker
      ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setConfigurationManagerOpen(true);
  };

  const menuHandlers: Record<
    SavedConfigurationMenuAction,
    (recentHandle: string | null) => void
  > = {
    newConfiguration: () => void startNewConfiguration(),
    openConfiguration: () => void openConfiguration(),
    openRecentConfiguration: (recentHandle) => {
      if (recentHandle) void openRecentConfiguration(recentHandle);
    },
    saveConfiguration: () => void saveCurrentConfiguration(),
    saveConfigurationAs: () => void saveConfigurationAs(),
    importConfiguration: () => void beginConfigurationPreview("import"),
    exportConfiguration: () => void exportCurrentConfiguration(),
    manageConfigurations: () => openConfigurationManager(),
  };

  return (
    <div className="app-shell">
      <SavedConfigurationMenuBridge handlers={menuHandlers} />
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
          {runtime.status === "ready"
            ? "Ready"
            : runtime.status === "starting"
              ? "Starting"
              : "App service unavailable"}
        </div>
        <div className="button-row header-actions">
          <button
            className="secondary"
            data-focus-fallback="header"
            onClick={(event) => void openUpdates(event.currentTarget)}
          >Updates</button>
          <button
            className="secondary"
            onClick={(event) => openSupport(event.currentTarget)}
          >Troubleshooting</button>
        </div>
      </header>

      <UpdatesPanel
        state={updates}
        returnFocus={updatesInvokerRef.current}
        navigationBlocked={navigationBlocked}
        onClose={() => {
          if (!updates.checking && !updates.opening) {
            setUpdates((current) => ({ ...current, open: false }));
          }
        }}
        onCheck={() => void checkForUpdates()}
        onOpenDownload={() => void openUpdateDownload()}
        onAnnounce={announce}
      />

      <SupportPanel
        state={support}
        returnFocus={supportInvokerRef.current}
        onClose={() => supportDispatch({ type: "close" })}
        onRefresh={() => void refreshSupportInventory()}
        onToggleSelection={(handle) => supportDispatch({ type: "toggle-selection", handle })}
        onPrepareCleanup={prepareSupportCleanup}
        onCleanup={(mode) => void cleanupSupportCache(mode)}
        onExport={() => void exportSupportDiagnostics()}
        onCorrectiveAction={(action, invoker) => void runSupportCorrectiveAction(action, invoker)}
        onReset={(category) => void resetLocalAppState(category)}
        onAnnounce={announce}
      />

      {configurationManagerOpen && (
        <SavedConfigurationManager
          active={savedConfiguration}
          busy={configurationManagerBusy || configurationActionsLocked}
          canSave={!saveDisabled}
          canSaveAs={Boolean(workflow.devicePlan)}
          preview={configurationPreview}
          previewMode={configurationPreviewMode}
          recents={recentConfigurations}
          returnFocus={configurationManagerInvokerRef.current}
          onClose={() => {
            void cancelConfigurationPreview();
            setConfigurationManagerOpen(false);
          }}
          onNew={() => {
            setConfigurationManagerOpen(false);
            void startNewConfiguration();
          }}
          onSave={() => {
            setConfigurationManagerOpen(false);
            void saveCurrentConfiguration();
          }}
          onSaveAs={() => {
            setConfigurationManagerOpen(false);
            void saveConfigurationAs();
          }}
          onOpenPicker={() => void beginConfigurationPreview("open")}
          onImportPicker={() => void beginConfigurationPreview("import")}
          onConfirmPreview={() => void confirmConfigurationPreview()}
          onRepairPreview={(handle) => void applyConfigurationPreviewRepair(handle)}
          onCancelPreview={() => void cancelConfigurationPreview()}
          onOpenRecent={(handle) => void openRecentConfiguration(handle)}
          onRelinkRecent={(handle) => void relinkRecentConfiguration(handle)}
          onRemoveRecent={(handle) => void removeRecentConfiguration(handle)}
          onRename={() => void renameCurrentConfiguration()}
          onDuplicate={() => void duplicateCurrentConfiguration()}
          onExport={() => void exportCurrentConfiguration()}
        />
      )}

      {runtime.status === "ready" && (
        <section className="configuration-bar" aria-label="Saved setups">
          <div className="configuration-summary-text">
            <strong>{savedConfiguration?.name ?? recoveredName ?? "Unsaved setup"}</strong>
            <small>
              {savedConfiguration
                ? `${savedConfigurationValidationLabel(savedConfiguration)}${savedConfiguration.dirty ? " · unsaved edits" : ""}`
                : recoveredName
                  ? "Recovered unsaved intent; reconnect and validate before creating a fresh review"
                  : "Selected setup, features, and reusable input references can be saved"}
            </small>
          </div>
          <div className="button-row">
            <button
              className="secondary"
              disabled={configurationActionsLocked}
              onClick={(event) => openConfigurationManager(event.currentTarget)}
            >Manage saved setups…</button>
          </div>
          {configurationActionsLocked && <p className="disabled-reason">Saved-setup management is unavailable while another operation or execution is active.</p>}
          {savedConfiguration && saveDisabled && <p className="disabled-reason">{saveDisabledReason}</p>}
        </section>
      )}

      {!startupReady ? (
        <main className="blocking-card" data-focus-fallback="main" id="main-content" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">Recovery check</p>
          <h2 data-step-heading tabIndex={-1}>Checking for recoverable work</h2>
          <p>EmuChef is checking for a safe recovery draft before setup begins.</p>
        </main>
      ) : runtime.status === "unsupported" || runtime.status === "failed" ? (
        <main className="blocking-card" data-focus-fallback="main" id="main-content" role="alert" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">App service unavailable</p>
          <h2 data-step-heading tabIndex={-1}>EmuChef could not start its app service</h2>
          <p>{runtime.error.message}</p>
          <button onClick={(event) => void restartRuntime(event.currentTarget)}>Try starting the app service again</button>
        </main>
      ) : adb?.status !== "ready" ? (
        <main className="blocking-card" aria-labelledby="adb-heading" data-focus-fallback="main" id="main-content" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">One-time setup</p>
          <h2 data-step-heading id="adb-heading" tabIndex={-1}>Set up Android Platform-Tools</h2>
          <p>
            Platform-Tools lets EmuChef find and communicate with your Android device. Download the
            macOS Platform-Tools ZIP from Google, then select it here. EmuChef installs and manages
            the files it needs.
          </p>
          {adb?.warning && <p className="warning">{adb.warning}</p>}
          {(adb?.error || notice) && (
            <p className="error" role="alert">{adb?.error?.message ?? notice}</p>
          )}
          <div className="button-row">
            <button className="secondary" onClick={openPlatformToolsPage}>
              Open Google download page
            </button>
            <button aria-describedby={platformToolsBusy ? "platform-tools-busy" : undefined} onClick={() => void importPlatformTools()} disabled={platformToolsBusy}>
              {platformToolsOperation.phase === "picker"
                ? "Choosing ZIP…"
                : platformToolsOperation.phase === "processing"
                  ? "Checking and installing…"
                  : "Select Platform-Tools ZIP…"}
            </button>
            {adb?.canRemove && <button className="danger" onClick={(event) => void removePlatformTools(event.currentTarget)} disabled={platformToolsBusy}>Remove</button>}
          </div>
          {platformToolsBusy && (
            <p className="disabled-reason" id="platform-tools-busy" role="status">
              {platformToolsOperation.phase === "picker"
                ? "Choose a ZIP in the file picker or cancel to return."
                : "Platform-Tools setup is in progress."}
            </p>
          )}
          <p className="fine-print">
            Setup is normally needed only once. You can replace or remove Platform-Tools later from Troubleshooting.
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

          <section className="workflow-card" aria-busy={busy || platformToolsBusy}>
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
                <p className="eyebrow">Connect device</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Choose an Android device</h2>
                <p>Connect with USB debugging enabled. EmuChef only reads device information in this phase.</p>
                <details className="connection-help">
                  <summary>How to connect a device</summary>
                  <ol>
                    <li>Enable Developer Options and USB debugging on the Android device.</li>
                    <li>Connect it with a data-capable USB cable and keep the device unlocked.</li>
                    <li>Accept the USB debugging authorization prompt on the device.</li>
                    <li>If it is still missing, check the cable and USB mode, then refresh.</li>
                  </ol>
                </details>
                {savedConfiguration && (
                  <section className={`configuration-validation ${savedConfiguration.validation.state}`}>
                    <strong>{savedConfiguration.name}</strong>
                    <span>{savedConfigurationValidationLabel(savedConfiguration)}</span>
                    {savedConfigurationBlocked && (
                      <p>Open another saved setup or repair this file before continuing.</p>
                    )}
                    {savedConfiguration.validation.diagnostics.map((diagnostic, index) => (
                      <div key={`configuration-diagnostic-${index}`}>
                        <p>{savedConfigurationDiagnosticSummary(diagnostic)}</p>
                        <details><summary>Technical details</summary><code>{diagnostic.code}</code></details>
                      </div>
                    ))}
                  </section>
                )}
                <div className="device-list" aria-busy={busy} role="region" aria-label="Detected Android devices">
                  {devices.length === 0 && <div className="empty-state" role="status">No devices found. Connect an unlocked device, enable USB debugging, accept its authorization prompt, then refresh.</div>}
                  {multipleDevicesConnected && (
                    <div className="warning multiple-device-warning" role="status">
                      <strong>No device can be selected while multiple Android devices are connected.</strong>
                      <span>Disconnect all but one device, then refresh discovery.</span>
                    </div>
                  )}
                  <ul>
                  {devices.map((device) => (
                    <li key={device.deviceHandle}>
                      <button
                        aria-describedby={device.state !== "available" || savedConfigurationBlocked || multipleDevicesConnected
                          ? stableDomId("device-reason", device.deviceHandle)
                          : undefined}
                        className="device-row"
                        disabled={device.state !== "available" || busy || savedConfigurationBlocked || multipleDevicesConnected}
                        onClick={(event) => void selectDevice(device.deviceHandle, event.currentTarget)}
                      >
                        <span><strong>{device.displayName}</strong><small>{device.maskedSerial}</small></span>
                        <span className={`status ${device.state}`}>
                          {device.state === "available"
                              ? "Connected"
                              : device.state === "unauthorized"
                                ? "Authorization required"
                                : "Offline"}
                        </span>
                      </button>
                      {(device.state !== "available" || savedConfigurationBlocked || multipleDevicesConnected) && (
                        <small className="disabled-reason" id={stableDomId("device-reason", device.deviceHandle)}>
                          {savedConfigurationBlocked
                            ? "Repair or replace the incompatible saved setup before selecting a device."
                            : device.state === "unauthorized"
                              ? "Unlock the device, accept the USB debugging prompt, then refresh."
                              : device.state === "offline"
                                ? "Reconnect the device and refresh before selecting it."
                                : "Disconnect the other Android devices and refresh before selecting this device."}
                        </small>
                      )}
                    </li>
                  ))}
                  </ul>
                </div>
                <button className="text-button" disabled={deviceRefresh.phase === "refreshing"} onClick={() => void pollDevices(true)}>
                  {deviceRefresh.phase === "refreshing" ? "Refreshing…" : "Refresh devices"}
                </button>
                {deviceRefresh.message && <p className="disabled-reason" role="status">{deviceRefresh.message}</p>}
              </>
            )}

            {workflow.step === "device" && (!workflow.facts || !workflow.match) && (
              <div className="empty-state" role="status"><h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Reading device properties</h2><p>Keep the selected device connected.</p></div>
            )}

            {workflow.step === "device" && workflow.facts && workflow.match && deviceIsUnsupported(workflow.match) && (
              <>
                <p className="eyebrow">Unsupported device</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>This device is not officially supported</h2>
                <p>
                  EmuChef detected {workflow.facts.manufacturer ?? "an Android"} {workflow.facts.model ?? "device"},
                  but no supported device-specific setup matches it.
                </p>
                <DeviceQualificationDetails
                  qualification={deviceQualification}
                  rootCheckPhase={rootCheckPhase}
                  onCheckRoot={() => void checkDeviceRoot()}
                />
                {workflow.match.safeGenericPlans.length > 0 ? (
                  <>
                    <label className="acknowledgment">
                      <input
                        type="checkbox"
                        checked={workflow.unsupportedAcknowledged}
                        onChange={(event) => dispatch({
                          type: "set-unsupported-acknowledgment",
                          acknowledged: event.currentTarget.checked,
                        })}
                      />
                      I understand this device is not officially supported and any offered generic setup is not device-specific.
                    </label>
                    <div className="button-row">
                      <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                      <button
                        disabled={!workflow.unsupportedAcknowledged}
                        onClick={() => dispatch({ type: "continue-unsupported" })}
                      >Show safe generic setups</button>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="error">No safe generic setup is available for this device.</p>
                    <button className="secondary" onClick={() => dispatch({ type: "back" })}>Choose another device</button>
                  </>
                )}
              </>
            )}

            {workflow.step === "setup" && workflow.facts && workflow.match && (
              <>
                <p className="eyebrow">Confirm device</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>{workflow.facts.manufacturer ?? "Android"} {workflow.facts.model ?? "device"}</h2>
                <p>Android {workflow.facts.androidVersion ?? "unknown"} · API {workflow.facts.androidApiLevel ?? "unknown"}</p>
                <p className={deviceIsUnsupported(workflow.match) ? "warning" : "success"}>
                  {deviceIsUnsupported(workflow.match)
                    ? "Unsupported device: choose one offered generic setup explicitly."
                    : "A supported device setup is available."}
                </p>
                <DeviceQualificationDetails
                  qualification={deviceQualification}
                  rootCheckPhase={rootCheckPhase}
                  onCheckRoot={() => void checkDeviceRoot()}
                />
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
                    <legend>{deviceIsUnsupported(workflow.match) ? "Choose a generic setup" : "Choose a safe setup"}</legend>
                    {planOptions.map((plan) => (
                      <label key={`${plan.planId}:defaults`}>
                        <input
                          type="radio"
                          name="device-plan"
                          checked={workflow.devicePlan === plan.planId && workflow.selectedRecipes?.length !== 0}
                          onChange={() => updateDevicePlanIntent(plan.planId, "defaults")}
                        />
                        <span><strong>{plan.name}</strong><small>{plan.description}</small></span>
                      </label>
                    ))}
                    {blankPlanOptions.map((plan) => (
                      <label key={`${plan.planId}:blank`}>
                        <input
                          type="radio"
                          name="device-plan"
                          checked={workflow.devicePlan === plan.planId && workflow.selectedRecipes?.length === 0}
                          onChange={() => updateDevicePlanIntent(plan.planId, "blank")}
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

            {workflow.step === "recipes" && workflow.description && (
              <>
                <p className="eyebrow">Choose features</p>
                <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>Choose what to install</h2>
                <p>Select at least one recipe. Required dependencies remain selected automatically.</p>
                <div className="recipe-discovery" aria-label="Recipe discovery controls">
                  <label htmlFor="recipe-search">Search recipes</label>
                  <input
                    id="recipe-search"
                    type="search"
                    value={recipeSearch}
                    onChange={(event) => setRecipeSearch(event.target.value)}
                    placeholder="Search by name or description"
                  />
                  <fieldset className="recipe-filters">
                    <legend>Filter recipes</legend>
                    {([
                      ["all", "All"],
                      ["available", "Available"],
                      ["selected", "Selected"],
                      ["unavailable", "Unavailable"],
                    ] as const).map(([value, label]) => (
                      <label key={value}>
                        <input
                          type="radio"
                          name="recipe-filter"
                          value={value}
                          checked={recipeFilter === value}
                          onChange={() => setRecipeFilter(value)}
                        />
                        <span>{label}</span>
                      </label>
                    ))}
                  </fieldset>
                </div>
                <section className="recipe-selection-summary" aria-labelledby="recipe-selection-summary-heading">
                  <h3 id="recipe-selection-summary-heading">
                    {selectedRecipeOptions.length} selected
                  </h3>
                  {selectedRecipeOptions.length > 0 ? (
                    <p>{selectedRecipeOptions.map((recipe) => recipe.name).join(", ")}</p>
                  ) : (
                    <p>No recipes selected.</p>
                  )}
                  <button
                    className="secondary"
                    type="button"
                    disabled={recommendedRecipeIds.length === 0 || recommendedSelectionActive || busy}
                    onClick={() => updateRecipeIntent(recommendedRecipeIds)}
                  >
                    {recommendedSelectionActive ? "Recommended setup selected" : "Select recommended setup"}
                  </button>
                  {recommendedRecipeIds.length === 0 && (
                    <p className="disabled-reason">This device setup does not define a recommended recipe set.</p>
                  )}
                </section>
                <fieldset className="recipe-list">
                  <legend>
                    Recipes · {filteredRecipeOptions.length} of {workflow.description.recipeOptions.length} shown
                  </legend>
                  {filteredRecipeOptions.map((recipe) => (
                    <label key={recipe.id} className={!recipe.available ? "unavailable" : ""}>
                      <input
                        type="checkbox"
                        disabled={recipeSelectionDisabled(recipe)}
                        checked={selectedRecipeIds.has(recipe.id) || recipe.dependencyRequired}
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
                              ? `Unavailable on this device${recipe.description ? ` · ${recipe.description}` : ""}`
                              : recipe.recommended
                                ? `Recommended for this device${recipe.description ? ` · ${recipe.description}` : ""}`
                                : recipe.description ?? "Optional"}
                        </small>
                        {(recipe.recipeDependencies ?? []).length > 0 && (
                          <small className="recipe-dependencies">
                            Also includes: {(recipe.recipeDependencies ?? []).map((dependencyId) => (
                              workflow.description?.recipeOptions.find((option) => option.id === dependencyId)?.name
                                ?? dependencyId
                            )).join(", ")}
                          </small>
                        )}
                        {recipe.dependencyRequired && (
                          <small className="recipe-dependencies">Added automatically because another selected recipe requires it.</small>
                        )}
                        {(recipe.contentRequirements ?? []).length > 0 && (
                          <span className="recipe-requirements" aria-label="What you'll need">
                            {(recipe.contentRequirements ?? []).map((requirement) => (
                              <span key={requirement} className="recipe-requirement">
                                {{
                                  apk_file: "APK file required",
                                  bios_files: "BIOS files required",
                                  rom_content: "ROM or content folder required",
                                  network_download: "Downloads files",
                                }[requirement]}
                              </span>
                            ))}
                          </span>
                        )}
                        {(recipe.requiredCapabilities ?? []).length > 0 && (
                          <span className="recipe-requirements" aria-label="Requirements">
                            {(recipe.requiredCapabilities ?? []).map((capability) => {
                              const label = ({
                                adb_available: "Device connection",
                                apk_install: "App installation",
                                shared_storage_write: "Shared storage access",
                                app_launch: "App launch",
                                shell_command: "Device commands",
                                package_remove_for_user: "App removal",
                                root_shell: "Root access",
                                app_data_write: "App data access",
                              } as Record<string, string>)[capability] ?? "Additional device requirement";
                              const unavailable = recipe.unavailableCapabilities.includes(capability);
                              return (
                                <span
                                  key={capability}
                                  className={`recipe-requirement${unavailable ? " unavailable" : ""}`}
                                >
                                  {unavailable ? `${label} unavailable` : label}
                                </span>
                              );
                            })}
                          </span>
                        )}
                      </span>
                    </label>
                  ))}
                  {filteredRecipeOptions.length === 0 && (
                    <p className="empty-state">No recipes match the current search and filter.</p>
                  )}
                </fieldset>
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button
                    aria-describedby={!(workflow.selectedRecipes?.length) || busy ? "recipes-continue-reason" : undefined}
                    disabled={!(workflow.selectedRecipes?.length) || busy}
                    onClick={continueToInputs}
                  >Continue</button>
                </div>
                {(!(workflow.selectedRecipes?.length) || busy) && (
                  <p className="disabled-reason" id="recipes-continue-reason">
                    {busy ? "Recipe validation is in progress." : "Select at least one recipe to continue."}
                  </p>
                )}
              </>
            )}

            {workflow.step === "inputs" && workflow.description && (
              <InputsStep
                bindings={workflow.bindings}
                busy={busy || workflow.descriptionDirty}
                description={workflow.description}
                onBack={() => dispatch({ type: "back" })}
                onBindingChange={updateBindingIntent}
                onClearInput={(input) => clearBindingIntent(input.key)}
                onPickInput={pickInputValue}
                onRemoveInputEntry={removeInputEntry}
                onRefreshValidation={describe}
                onReview={generateReview}
                touchedInputKeys={touchedInputKeys}
                validationErrors={validationErrors}
                validationRequested={validationRequested}
                validationSummaryRef={validationSummaryRef}
              />
            )}

            {workflow.step === "review" && workflow.review && (
              <ReviewStep
                busy={busy}
                executionKind={workflow.execution.kind}
                onApplyToDevice={(invoker) => void requestRealExecution(invoker)}
                onBack={() => dispatch({ type: "back" })}
                onStartSimulation={startSimulation}
                realExecutionCompiled={realExecutionCompiled}
                qualification={deviceQualification}
                review={workflow.review}
                reviewStale={workflow.reviewStale}
              />
            )}

            {workflow.step === "execution" &&
              (workflow.execution.kind === "active"
                || workflow.execution.kind === "terminal"
                || workflow.execution.kind === "unavailable") && (
                <ExecutionStep
                  execution={workflow.execution}
                  launchState={launchState}
                  onCancel={cancelExecution}
                  onExportReport={exportExecutionReport}
                  onLaunchConfiguredApp={launchConfiguredApp}
                  onPrepareRepair={prepareRepair}
                  onReturn={() => {
                    const execution = workflow.execution;
                    if (execution.kind !== "terminal" && execution.kind !== "unavailable") return;
                    dispatch({
                      type: execution.mode === "real" ? "runtime-invalidated" : "return-to-review",
                    });
                  }}
                  repairPreparing={repairPreparing}
                  reportState={reportState}
                />
              )}
          </section>

          <aside className="status-panel">
            <p className="eyebrow">Current status</p>
            <dl>
              <div><dt>App service</dt><dd>Ready</dd></div>
              <div>
                <dt>Platform Tools</dt>
                <dd>
                  {executionCapabilitiesRefresh === "refreshing"
                    ? "Refreshing…"
                    : executionCapabilitiesRefresh === "failed"
                      ? "Status unavailable"
                      : executionCapabilities
                        ? platformToolsStatusLabels[executionCapabilities.platformToolsStatus]
                        : "Status unavailable"}
                </dd>
              </div>
              <div><dt>Setup catalog</dt><dd>Ready</dd></div>
              <div>
                <dt>Device qualification</dt>
                <dd>{deviceQualification ? deviceQualificationStateLabels[deviceQualification.state] : "Status unavailable"}</dd>
              </div>
              <div>
                <dt>Real-device execution</dt>
                <dd>{realExecutionCompiled ? "Compiled in" : "Not compiled"}</dd>
              </div>
              <div>
                <dt>Executor readiness</dt>
                <dd>
                  {executionCapabilitiesRefresh === "refreshing"
                    ? "Refreshing…"
                    : executionCapabilitiesRefresh === "failed"
                      ? "Status unavailable"
                      : executionCapabilities
                        ? executorReadinessLabels[executionCapabilities.executorReadiness]
                        : "Status unavailable"}
                </dd>
              </div>
            </dl>
            {adb.warning && <p className="warning">{adb.warning}</p>}
            {adb.warning && <p className="fine-print">Open Troubleshooting for Platform-Tools maintenance and device-connection help.</p>}
          </aside>
        </main>
      )}

      {activeDialog?.payload.kind === "remove-platform-tools" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="remove-platform-tools-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, false)}
          returnFocus={activeDialog.payload.invoker}
          role="alertdialog"
          titleId="remove-platform-tools-title"
        >
          <h2 id="remove-platform-tools-title">Remove Platform-Tools?</h2>
          <p id="remove-platform-tools-description">
            Device detection will stop. After reinstalling Platform-Tools, you must select your device,
            validate the setup, and create a fresh review before continuing.
          </p>
          <div className="button-row dialog-actions">
            <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, false)}>Cancel</button>
            <button className="danger" onClick={() => dialogController.settle(activeDialog.id, true)}>Remove</button>
          </div>
        </AccessibleDialog>
      )}

      {activeDialog?.payload.kind === "different-device" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="different-device-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, false)}
          returnFocus={activeDialog.payload.invoker}
          role="alertdialog"
          titleId="different-device-title"
        >
          <h2 id="different-device-title">Start with a different device?</h2>
          <p id="different-device-description">
            The preserved setup belongs to the device you were using. Starting with another device
            clears those held setup and input choices so they are not applied unsafely.
          </p>
          <div className="button-row dialog-actions">
            <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, false)}>Cancel</button>
            <button className="danger" onClick={() => dialogController.settle(activeDialog.id, true)}>Start fresh</button>
          </div>
        </AccessibleDialog>
      )}

      {activeDialog?.payload.kind === "restart-loss" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="restart-loss-description"
          dialogId={activeDialog.id}
          onDismiss={() => dialogController.settle(activeDialog.id, false)}
          returnFocus={activeDialog.payload.invoker}
          role="alertdialog"
          titleId="restart-loss-title"
        >
          <h2 id="restart-loss-title">Restart the app service?</h2>
          <div id="restart-loss-description">
            {activeDialog.payload.totalLoss ? (
              <p>EmuChef could not preserve the current setup choices safely. Restarting now clears them.</p>
            ) : (
              <>
                <p>
                  The setup and nonsensitive choices will be restored, but {activeDialog.payload.omittedCount}
                  {activeDialog.payload.omittedCount === 1 ? " value" : " values"} must be selected or entered again.
                </p>
                {activeDialog.payload.labels.length > 0 && (
                  <p>Affected fields: {activeDialog.payload.labels.join(", ")}.</p>
                )}
              </>
            )}
            <p>No generated plan or execution can be resumed after restart.</p>
          </div>
          <div className="button-row dialog-actions">
            <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, false)}>Cancel</button>
            <button className="danger" onClick={() => dialogController.settle(activeDialog.id, true)}>Restart app service</button>
          </div>
        </AccessibleDialog>
      )}

      {activeDialog?.payload.kind === "recovery" && (
        <AccessibleDialog
          currentDialogId={() => dialogController.activeId}
          descriptionId="recovery-dialog-description"
          dialogId={activeDialog.id}
          initialFocusRef={recoveryNotNowRef}
          onDismiss={() => dialogController.settle(activeDialog.id, "not-now")}
          returnFocus={null}
          role="alertdialog"
          titleId="recovery-dialog-title"
        >
          <p className="eyebrow">Recovery draft</p>
          <h2 id="recovery-dialog-title">Restore the unsaved setup?</h2>
          <div id="recovery-dialog-description">
            <p>
              {activeDialog.payload.draft.reason}
              {activeDialog.payload.draft.displayName ? ` Draft: ${activeDialog.payload.draft.displayName}.` : ""}
            </p>
            <p><strong>May restore:</strong> {activeDialog.payload.draft.restores}</p>
            <p><strong>Cannot restore:</strong> {activeDialog.payload.draft.doesNotRestore}</p>
            <p><strong>Restore:</strong> {activeDialog.payload.draft.restoreConsequence}</p>
            <p><strong>Discard:</strong> {activeDialog.payload.draft.discardConsequence}</p>
            <p>Not now keeps this draft for the next launch unless newer edits supersede it.</p>
          </div>
          <div className="button-row dialog-actions">
            <button onClick={() => dialogController.settle(activeDialog.id, "restore")}>Restore</button>
            <button className="danger" onClick={() => dialogController.settle(activeDialog.id, "discard-recovery")}>Discard</button>
            <button
              className="secondary"
              onClick={() => dialogController.settle(activeDialog.id, "not-now")}
              ref={recoveryNotNowRef}
            >Not now</button>
          </div>
        </AccessibleDialog>
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
          <p className="eyebrow">Unsaved setup</p>
          <h2 id="unsaved-dialog-title">Save edits before continuing?</h2>
          <p id="unsaved-dialog-description">
            Save preserves the current setup choices. Discard permanently abandons the unsaved edits. Cancel keeps the current setup open.
          </p>
          <div className="button-row dialog-actions">
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
              <p id="name-dialog-description">The file saves the selected setup, features, and reusable input references. It does not save the connected device, generated plan, execution progress, or results.</p>
            <label className="input-field" htmlFor="configuration-name">Setup name</label>
            <input
              autoComplete="off"
              id="configuration-name"
              onChange={(event) => setNamePromptValue(event.target.value)}
              required
              value={namePromptValue}
            />
            <div className="button-row dialog-actions">
              <button className="secondary" onClick={() => dialogController.settle(activeDialog.id, null)} type="button">Cancel</button>
              <button disabled={!namePromptValue.trim()} type="submit">Continue</button>
            </div>
            {!namePromptValue.trim() && <p className="disabled-reason">Enter a setup name to continue.</p>}
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
          <p className="eyebrow">Real device</p>
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
          <div className="button-row dialog-actions">
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

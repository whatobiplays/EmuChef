import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";

import { api } from "./api";
import type {
  AdbSetupStatus,
  AnyExecutionSnapshot,
  CatalogSummary,
  DeviceSummary,
  InputDescriptor,
  RuntimeStatus,
} from "./types";
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

const WORKFLOW_STEPS = [
  { step: "connect", label: "Connect" },
  { step: "device", label: "Device" },
  { step: "setup", label: "Setup" },
  { step: "inputs", label: "Inputs" },
  { step: "review", label: "Review" },
  { step: "execution", label: "Simulated Run" },
] as const;

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

export function App() {
  const [runtime, setRuntime] = useState<RuntimeStatus>({ status: "starting" });
  const [catalog, setCatalog] = useState<CatalogSummary | null>(null);
  const [adb, setAdb] = useState<AdbSetupStatus | null>(null);
  const [devices, setDevices] = useState<DeviceSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [realExecutionEnabled, setRealExecutionEnabled] = useState(false);
  const [realConfirmationOpen, setRealConfirmationOpen] = useState(false);
  const [realConfirmation, setRealConfirmation] = useState(emptyRealExecutionConfirmation);
  const [reportState, setReportState] = useState<"idle" | "exporting" | "saved" | "failed">("idle");
  const [launchState, setLaunchState] = useState<"idle" | "launching" | "launched" | "failed">("idle");
  const [repairPreparing, setRepairPreparing] = useState(false);
  const [workflow, dispatch] = useReducer(workflowReducer, initialWorkflowState);
  const mainRef = useRef<HTMLElement>(null);

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
      setCatalog(await api.catalog());
    }
  }, []);

  useEffect(() => {
    initialize().catch((error) => {
      setRuntime({
        status: "failed",
        error: { code: "runtime_start_failed", message: errorMessage(error), actions: ["retry"] },
      });
    });
  }, [initialize]);

  useEffect(() => {
    mainRef.current?.focus();
  }, [adb?.status, runtime.status, workflow.step]);

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
      action: api.importPlatformTools,
      onSuccess: setAdb,
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
      dispatch({ type: "runtime-invalidated" });
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
    try {
      const facts = await api.probeDevice(deviceHandle);
      const match = await api.matchDevice(deviceHandle);
      dispatch({ type: "device-probed", facts, match });
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
    try {
      const generation = workflow.requestGeneration;
      const description = await api.describeConfiguration({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
      });
      dispatch({ type: "description", description, generation });
    } catch (error) {
      setNotice(errorMessage(error));
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
    try {
      const review = await api.createReview({
        deviceHandle: workflow.deviceHandle,
        devicePlan: workflow.devicePlan,
        selectedRecipes: workflow.selectedRecipes,
        bindings: workflow.bindings,
      });
      dispatch({ type: "review", review });
    } catch (error) {
      setNotice(errorMessage(error));
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
    setRealConfirmationOpen(false);
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

  const activeExecution =
    workflow.execution.kind === "active" ? workflow.execution : null;

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
    setReportState("exporting");
    setNotice(null);
    try {
      const result = await api.exportExecutionReport(workflow.execution.snapshot.executionHandle);
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
    setRealConfirmationOpen(false);
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
      action: () => api.pickInputPath(input.pathKind!, Boolean(input.multiple)),
      onSuccess: (values) => {
        if (values) {
          dispatch({
            type: "set-binding",
            key: input.key,
            value: input.multiple ? values : values[0],
          });
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

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand-mark" aria-hidden="true">E</div>
        <div>
          <p className="eyebrow">EMUCHEF</p>
          <h1>Prepare your Android handheld</h1>
        </div>
        <div className="runtime-chip" aria-live="polite">
          {runtime.status === "ready" ? `Runtime ready · ${catalog?.catalog.version ?? "catalog"}` : runtime.status}
        </div>
      </header>

      {runtime.status === "unsupported" || runtime.status === "failed" ? (
        <main className="blocking-card" role="alert" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">RUNTIME UNAVAILABLE</p>
          <h2>EmuChef could not start its Rust runtime</h2>
          <p>{runtime.error.message}</p>
          <button onClick={initialize}>Retry runtime startup</button>
        </main>
      ) : adb?.status !== "ready" ? (
        <main className="blocking-card" aria-labelledby="adb-heading" ref={mainRef} tabIndex={-1}>
          <p className="eyebrow">ONE-TIME SETUP</p>
          <h2 id="adb-heading">Android SDK Platform-Tools is required</h2>
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
            <button onClick={importPlatformTools} disabled={busy}>
              {busy ? "Validating…" : "Import Platform-Tools ZIP"}
            </button>
            {adb?.canRemove && <button className="danger" onClick={removePlatformTools}>Remove</button>}
          </div>
          <p className="fine-print">
            EmuChef keeps only adb, NOTICE.txt, and source.properties in its application data. The
            selected ZIP remains yours and is never copied into the app bundle or repository.
          </p>
        </main>
      ) : (
        <main className="workflow-layout" ref={mainRef} tabIndex={-1}>
          <nav aria-label="Setup progress" className="progress-nav">
            <ol>
              {WORKFLOW_STEPS.map((item, index) => (
                <li key={item.step} className={index === stepIndex ? "current" : index < stepIndex ? "complete" : ""}>
                  <span>{index + 1}</span>{item.label}
                </li>
              ))}
            </ol>
          </nav>

          <section className="workflow-card" aria-busy={busy}>
            {notice && <p className="warning" role="status">{notice}</p>}

            {workflow.step === "connect" && (
              <>
                <p className="eyebrow">CONNECT DEVICE</p>
                <h2>Choose an Android device</h2>
                <p>Connect with USB debugging enabled. EmuChef only reads device information in this phase.</p>
                <div className="device-list">
                  {devices.length === 0 && <div className="empty-state">No ADB devices detected yet.</div>}
                  {devices.map((device) => (
                    <button
                      className="device-row"
                      key={device.deviceHandle}
                      disabled={device.state !== "available" || busy}
                      onClick={() => selectDevice(device.deviceHandle)}
                    >
                      <span><strong>{device.displayName}</strong><small>{device.maskedSerial}</small></span>
                      <span className={`status ${device.state}`}>{device.state}</span>
                    </button>
                  ))}
                </div>
                <button className="text-button" onClick={pollDevices}>Refresh devices</button>
              </>
            )}

            {workflow.step === "device" && <div className="empty-state">Reading device properties…</div>}

            {workflow.step === "setup" && workflow.facts && workflow.match && (
              <>
                <p className="eyebrow">CONFIRM DEVICE</p>
                <h2>{workflow.facts.manufacturer ?? "Android"} {workflow.facts.model ?? "device"}</h2>
                <p>Android {workflow.facts.androidVersion ?? "unknown"} · API {workflow.facts.androidApiLevel ?? "unknown"}</p>
                <div className="confidence">Match confidence: <strong>{workflow.match.confidence}</strong></div>
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
                          onChange={() => dispatch({ type: "select-plan", devicePlan: plan.planId })}
                        />
                        <span><strong>{plan.name}</strong><small>{plan.description}</small></span>
                      </label>
                    ))}
                  </fieldset>
                )}
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button disabled={!workflow.devicePlan || busy} onClick={describe}>Continue</button>
                </div>
              </>
            )}

            {workflow.step === "inputs" && workflow.description && (
              <>
                <p className="eyebrow">CHOOSE SETUP</p>
                <h2>Customize your setup</h2>
                <div className="recipe-list">
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
                          dispatch({ type: "set-recipes", selectedRecipes: selected });
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
                </div>

                {workflow.description.inputs.map((input) => (
                  <label className="input-field" key={input.key}>
                    <span>{input.label}{input.required ? " *" : ""}</span>
                    {input.type === "boolean" ? (
                      <input
                        type="checkbox"
                        checked={Boolean(workflow.bindings[input.key] ?? input.value)}
                        onChange={(event) => dispatch({ type: "set-binding", key: input.key, value: event.target.checked })}
                      />
                    ) : input.options?.length ? (
                      <select
                        value={String(workflow.bindings[input.key] ?? input.value ?? "")}
                        onChange={(event) => dispatch({ type: "set-binding", key: input.key, value: event.target.value })}
                      >
                        <option value="">Choose…</option>
                        {input.options.map((option) => <option key={option}>{option}</option>)}
                      </select>
                    ) : input.pathKind ? (
                      <div className="path-picker">
                        <input readOnly value={String(workflow.bindings[input.key] ?? input.value ?? "")} />
                        <button
                          className="secondary"
                          disabled={busy}
                          onClick={() => pickInputValue(input)}
                        >Browse…</button>
                      </div>
                    ) : (
                      <input
                        value={String(workflow.bindings[input.key] ?? input.value ?? "")}
                        onChange={(event) => dispatch({ type: "set-binding", key: input.key, value: event.target.value })}
                      />
                    )}
                    {input.description && <small>{input.description}</small>}
                    {input.acceptedExtensions?.length ? (
                      <small>Accepted file types: {input.acceptedExtensions.join(", ")}</small>
                    ) : null}
                    {input.valueSource ? (
                      <small>Value source: {input.valueSource.replaceAll("_", " ")}</small>
                    ) : null}
                    {inputDiagnosticsForDisplay(input).map((item) => (
                      <small className="error" key={`${item.key ?? input.key}-${item.code}`}>{item.message}</small>
                    ))}
                  </label>
                ))}

                {pageDiagnosticsForDisplay(workflow.description).map((item) => (
                  <p
                    className={item.severity === "error" ? "error" : "warning"}
                    key={`${item.key ?? "global"}-${item.code}-${item.message}`}
                  >{item.message}</p>
                ))}
                <div className="button-row">
                  <button className="secondary" onClick={() => dispatch({ type: "back" })}>Back</button>
                  <button className="secondary" onClick={describe} disabled={busy}>Refresh validation</button>
                  <button onClick={generateReview} disabled={!reviewReady(workflow) || busy}>Review plan</button>
                </div>
              </>
            )}

            {workflow.step === "review" && workflow.review && (
              <>
                <p className="eyebrow">REVIEW PLAN</p>
                <h2>Ready for a simulated dry run</h2>
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
                  <button onClick={startSimulation} disabled={busy || workflow.execution.kind === "starting"}>
                    {workflow.execution.kind === "starting" ? "Starting simulated run…" : "Start Simulated Dry Run"}
                  </button>
                  {realExecutionEnabled && (
                    <button
                      className="danger"
                      onClick={() => {
                        setRealConfirmation(emptyRealExecutionConfirmation);
                        setRealConfirmationOpen(true);
                      }}
                      disabled={busy || workflow.execution.kind === "starting"}
                    >
                      Apply to Device
                    </button>
                  )}
                </div>
                {realExecutionEnabled && realConfirmationOpen && (
                  <section className="real-confirmation" aria-labelledby="real-confirmation-heading">
                    <p className="eyebrow">REAL DEVICE</p>
                    <h3 id="real-confirmation-heading">Confirm irreversible device changes</h3>
                    <p>
                      Connected Android device · {workflow.review.target.manufacturer ?? "Android"}
                      {` ${workflow.review.target.model ?? "device"}`} · API {workflow.review.target.androidApiLevel ?? "unknown"}
                    </p>
                    <p className="error">
                      This can install software, transfer or replace files, change permissions and app operations,
                      and launch or stop applications on the connected device.
                    </p>
                    <p className="warning">
                      EmuChef provides no rollback, restore, automatic backup, or prior-state recovery. Cancellation
                      is cooperative: the current operation may finish, and completed changes are not undone.
                    </p>
                    <p className="warning">
                      Artifact transfer can fail after execution starts. Keep the intended device connected and stable
                      until a terminal result is shown.
                    </p>
                    <label className="input-field">
                      <span>Type APPLY TO DEVICE</span>
                      <input
                        value={realConfirmation.phrase}
                        autoComplete="off"
                        onChange={(event) => setRealConfirmation({ ...realConfirmation, phrase: event.target.value })}
                      />
                    </label>
                    <label>
                      <input
                        type="checkbox"
                        checked={realConfirmation.irreversibleChangesAcknowledged}
                        onChange={(event) => setRealConfirmation({
                          ...realConfirmation,
                          irreversibleChangesAcknowledged: event.target.checked,
                        })}
                      /> I understand this can irreversibly change the device.
                    </label>
                    <label>
                      <input
                        type="checkbox"
                        checked={realConfirmation.noRollbackAcknowledged}
                        onChange={(event) => setRealConfirmation({
                          ...realConfirmation,
                          noRollbackAcknowledged: event.target.checked,
                        })}
                      /> I understand there is no rollback, restore, or backup recovery.
                    </label>
                    <label>
                      <input
                        type="checkbox"
                        checked={realConfirmation.keepDeviceConnectedAcknowledged}
                        onChange={(event) => setRealConfirmation({
                          ...realConfirmation,
                          keepDeviceConnectedAcknowledged: event.target.checked,
                        })}
                      /> I will keep the intended device connected and stable.
                    </label>
                    <div className="button-row">
                      <button
                        className="secondary"
                        onClick={() => {
                          setRealConfirmationOpen(false);
                          setRealConfirmation(emptyRealExecutionConfirmation);
                        }}
                      >Cancel</button>
                      <button
                        className="danger"
                        onClick={startRealExecution}
                        disabled={busy || !realExecutionConfirmationComplete(realConfirmation)}
                      >Start Real-Device Execution</button>
                    </div>
                  </section>
                )}
              </>
            )}

            {workflow.step === "execution" &&
              (workflow.execution.kind === "active" || workflow.execution.kind === "terminal") && (
                <>
                  <p className="eyebrow">
                    {workflow.execution.mode === "real" ? "REAL DEVICE" : "SIMULATED / DRY RUN"}
                  </p>
                  <h2>
                    {workflow.execution.kind === "terminal"
                      ? `${workflow.execution.mode === "real" ? "Real-device execution" : "Simulation"} ${workflow.execution.snapshot.status.replaceAll("_", " ")}`
                      : workflow.execution.mode === "real"
                        ? "Applying the reviewed setup"
                        : "Simulating the reviewed setup"}
                  </h2>
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
                <h2>
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
          </aside>
        </main>
      )}
    </div>
  );
}

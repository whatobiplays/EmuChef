import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";

import { api } from "./api";
import type {
  AdbSetupStatus,
  CatalogSummary,
  DeviceSummary,
  ExecutionSnapshot,
  InputDescriptor,
  RuntimeStatus,
} from "./types";
import {
  initialWorkflowState,
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

function executionDuration(snapshot: ExecutionSnapshot): string | null {
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
  const [workflow, dispatch] = useReducer(workflowReducer, initialWorkflowState);
  const mainRef = useRef<HTMLElement>(null);

  const initialize = useCallback(async () => {
    const [runtimeStatus, adbStatus] = await Promise.all([api.runtimeStatus(), api.adbStatus()]);
    setRuntime(runtimeStatus);
    setAdb(adbStatus);
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

  const activeExecution =
    workflow.execution.kind === "active" ? workflow.execution : null;

  useEffect(() => {
    if (!activeExecution) return;
    let disposed = false;
    let timer: number | null = null;
    const { generation, snapshot } = activeExecution;
    const executionHandle = snapshot.executionHandle;
    let eventCursor = activeExecution.eventCursor;

    async function pollExecution() {
      try {
        const nextSnapshot = await api.getSimulatedExecution(executionHandle);
        if (disposed) return;
        dispatch({ type: "execution-snapshot", generation, snapshot: nextSnapshot });
        eventCursor = Math.max(eventCursor, nextSnapshot.latestSequence);
        if (nextSnapshot.terminal) return;

        const batch = await api.getSimulatedExecutionEvents(executionHandle, eventCursor);
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
  }, [activeExecution?.generation, activeExecution?.snapshot.executionHandle]);

  const cancelSimulation = async () => {
    if (workflow.execution.kind !== "active" || workflow.execution.cancellationRequested) return;
    const { generation, snapshot } = workflow.execution;
    try {
      const cancellation = await api.cancelSimulatedExecution(snapshot.executionHandle);
      if (cancellation.accepted) {
        dispatch({ type: "execution-cancellation-requested", generation });
      }
    } catch (error) {
      setNotice(errorMessage(error));
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
                </div>
              </>
            )}

            {workflow.step === "execution" &&
              (workflow.execution.kind === "active" || workflow.execution.kind === "terminal") && (
                <>
                  <p className="eyebrow">SIMULATED / DRY RUN</p>
                  <h2>
                    {workflow.execution.kind === "terminal"
                      ? `Simulation ${workflow.execution.snapshot.status.replaceAll("_", " ")}`
                      : "Simulating the reviewed setup"}
                  </h2>
                  <p className="simulation-banner">
                    No real device changes are made. This report is simulated evidence only.
                  </p>
                  <dl className="execution-summary">
                    <div><dt>Status</dt><dd>{workflow.execution.snapshot.status.replaceAll("_", " ")}</dd></div>
                    <div><dt>Started</dt><dd>{workflow.execution.snapshot.startedAt ?? "Starting"}</dd></div>
                    {executionDuration(workflow.execution.snapshot) && (
                      <div><dt>Duration</dt><dd>{executionDuration(workflow.execution.snapshot)}</dd></div>
                    )}
                  </dl>
                  {workflow.execution.snapshot.warnings.map((issue) => (
                    <p className="warning" key={`warning-${issue.code}-${issue.stepId ?? "run"}`}>{issue.message}</p>
                  ))}
                  {workflow.execution.snapshot.errors.map((issue) => (
                    <p className="error" key={`error-${issue.code}-${issue.stepId ?? "run"}`}>{issue.message}</p>
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
                      <summary>Incremental simulated event log</summary>
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
                          Cancellation requested. Completed simulated steps remain visible in this report. No new
                          simulated steps start, the current simulated atomic step may finish, and no real device
                          changes or rollback exist.
                        </p>
                      )}
                      <button
                        className="danger"
                        onClick={cancelSimulation}
                        disabled={workflow.execution.cancellationRequested}
                      >
                        {workflow.execution.cancellationRequested ? "Cancellation requested" : "Cancel simulated run"}
                      </button>
                    </>
                  ) : (
                    <div className="button-row">
                      <button className="secondary" onClick={() => dispatch({ type: "return-to-review" })}>
                        Return to Review
                      </button>
                    </div>
                  )}
                </>
              )}

            {workflow.step === "execution" && workflow.execution.kind === "unavailable" && (
              <>
                <p className="eyebrow">SIMULATED RUN UNAVAILABLE</p>
                <h2>This in-memory simulation cannot be resumed</h2>
                <p className="warning">{workflow.execution.message}</p>
                <p>No execution history is persisted across an app or sidecar restart.</p>
                <button className="secondary" onClick={() => dispatch({ type: "return-to-review" })}>
                  Return to Review
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
              <div><dt>Mode</dt><dd>Simulation only</dd></div>
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

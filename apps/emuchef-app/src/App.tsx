import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";

import { api } from "./api";
import type {
  AdbSetupStatus,
  CatalogSummary,
  DeviceSummary,
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

const STEP_LABELS = ["Connect", "Device", "Setup", "Inputs", "Review"];

function errorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { message?: unknown };
    return typeof parsed.message === "string" ? parsed.message : raw;
  } catch {
    return raw;
  }
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

  const stepIndex = STEP_LABELS.findIndex((label) => label.toLowerCase() === workflow.step);
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
              {STEP_LABELS.map((label, index) => (
                <li key={label} className={index === stepIndex ? "current" : index < stepIndex ? "complete" : ""}>
                  <span>{index + 1}</span>{label}
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
                <h2>Ready for a future configure step</h2>
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
                  <button disabled title="Device configuration begins in Phase 2">Configure Device — Phase 2</button>
                </div>
              </>
            )}
          </section>

          <aside className="status-panel">
            <p className="eyebrow">SYSTEM STATUS</p>
            <dl>
              <div><dt>Rust runtime</dt><dd>Ready</dd></div>
              <div><dt>Platform-Tools</dt><dd>{adb.version}</dd></div>
              <div><dt>Catalog</dt><dd>{catalog?.catalog.version ?? "Ready"}</dd></div>
              <div><dt>Mode</dt><dd>Read-only</dd></div>
            </dl>
            {adb.warning && <p className="warning">{adb.warning}</p>}
            <button className="text-button" onClick={importPlatformTools} disabled={busy}>Replace Platform-Tools</button>
            <button className="text-button danger-text" onClick={removePlatformTools} disabled={busy}>Remove Platform-Tools</button>
          </aside>
        </main>
      )}
    </div>
  );
}

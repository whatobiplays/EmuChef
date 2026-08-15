import { useState } from "react";

import { api } from "./api";
import { errorMessage } from "./app-helpers";
import { ExecutionStep } from "./ExecutionStep";
import type {
  Phase6d6LoadedProjection,
  Phase6d6UiCaptureResult,
  Phase6d6UiSmokeStatus,
  Phase6d6UiSmokeSubcase,
} from "./types";

const SUBJECT_LABELS: Record<Phase6d6UiSmokeSubcase, string> = {
  cancellation: "Cancellation",
  transport: "Transport",
  root: "Root",
  storage: "Storage",
  host_sleep: "Host sleep",
};

const SUBJECTS: Phase6d6UiSmokeSubcase[] = [
  "cancellation",
  "transport",
  "root",
  "storage",
  "host_sleep",
];

function subjectKey(subcase: Phase6d6UiSmokeSubcase): string {
  return subcase === "host_sleep" ? "host-sleep" : subcase;
}

interface Phase6d6UiSmokeProps {
  status: Phase6d6UiSmokeStatus;
}

/**
 * Development-only Phase 6D.6 UI-smoke qualification shell.
 *
 * This surface selects an accepted physical backend binding, loads its
 * projection through the production Tauri real-execution projection, renders
 * the normal terminal UI, and captures the canonical sanitized UI state. It
 * owns no live execution, cancellation, launch, or device authority.
 */
export function Phase6d6UiSmoke({ status }: Phase6d6UiSmokeProps) {
  const [selectedHandle, setSelectedHandle] = useState<string | null>(null);
  const [loaded, setLoaded] = useState<Phase6d6LoadedProjection | null>(null);
  const [uiRepetition, setUiRepetition] = useState<1 | 2>(1);
  const [busy, setBusy] = useState<"load" | "capture" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [capture, setCapture] = useState<Phase6d6UiCaptureResult | null>(null);

  const loadProjection = async () => {
    if (!selectedHandle || busy) return;
    setBusy("load");
    setError(null);
    setCapture(null);
    try {
      setLoaded(await api.phase6d6LoadProjection(selectedHandle));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const captureState = async () => {
    if (!loaded || busy) return;
    setBusy("capture");
    setError(null);
    try {
      setCapture(await api.phase6d6Capture(loaded.projectionHandle, uiRepetition));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!status.enabled) return null;

  return (
    <main className="phase6d6-ui-smoke" data-development-qualification="phase6d6-ui-smoke">
      <p className="eyebrow">Development qualification</p>
      <h1 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        Phase 6D.6 development UI-smoke qualification
      </h1>
      <p className="warning">
        This surface presents an accepted physical backend result through the production terminal
        UI. It is not a live execution: no device command runs, and nothing here can be resumed,
        retried, or rolled back.
      </p>
      {!status.ready && (
        <p role="alert" className="warning">
          {status.message}
        </p>
      )}
      {status.ready && (
        <section aria-label="Physical backend bindings">
          {SUBJECTS.map((subcase) => {
            const candidates = status.candidates.filter(
              (candidate) => candidate.subcase === subcase,
            );
            return (
              <fieldset key={subcase}>
                <legend>{SUBJECT_LABELS[subcase]}</legend>
                {candidates.length > 0 ? (
                  candidates.map((candidate) => (
                    <label key={candidate.handle}>
                      <input
                        type="radio"
                        name={`phase6d6-binding-${subcase}`}
                        value={candidate.handle}
                        checked={selectedHandle === candidate.handle}
                        onChange={() => {
                          setSelectedHandle(candidate.handle);
                          setLoaded(null);
                          setCapture(null);
                        }}
                      />
                      {candidate.label}
                    </label>
                  ))
                ) : (
                  <p>No accepted physical {subjectKey(subcase)} binding is available.</p>
                )}
              </fieldset>
            );
          })}
          <div className="button-row">
            <button
              type="button"
              onClick={() => void loadProjection()}
              disabled={!selectedHandle || busy !== null}
            >
              {busy === "load" ? "Loading projection…" : "Load projection"}
            </button>
          </div>
        </section>
      )}
      {loaded && (
        <>
          <ExecutionStep
            execution={{
              kind: "terminal",
              generation: 1,
              mode: "real",
              snapshot: loaded.snapshot,
              events: [],
              eventCursor: 0,
              cancellationRequested: false,
            }}
            launchState="idle"
            repairPreparing={false}
            reportState="idle"
            onCancel={() => undefined}
            onExportReport={() => undefined}
            onLaunchConfiguredApp={() => undefined}
            onPrepareRepair={() => undefined}
            onReturn={() => {
              setLoaded(null);
              setCapture(null);
              setSelectedHandle(null);
            }}
          />
          <section aria-label="UI-state capture">
            <h2>Capture UI state</h2>
            <label htmlFor="phase6d6-ui-repetition">UI-smoke repetition</label>
            <select
              id="phase6d6-ui-repetition"
              value={uiRepetition}
              onChange={(event) => setUiRepetition(event.target.value === "2" ? 2 : 1)}
            >
              <option value="1">1</option>
              <option value="2">2</option>
            </select>
            <button
              type="button"
              onClick={() => void captureState()}
              disabled={busy !== null}
            >
              {busy === "capture" ? "Capturing…" : "Capture UI state"}
            </button>
          </section>
        </>
      )}
      {capture && (
        <section aria-label="Capture result">
          <h2>UI-state capture written</h2>
          <dl className="execution-summary">
            <div><dt>Artifact</dt><dd>{capture.artifact.path}</dd></div>
            <div><dt>Digest</dt><dd>{capture.artifact.digest}</dd></div>
            <div><dt>Sub-run</dt><dd>{capture.subRunId}</dd></div>
            <div><dt>Backend run</dt><dd>{capture.backendRunId}</dd></div>
            <div><dt>Build identity</dt><dd>{capture.developmentBuild.identity}</dd></div>
          </dl>
          <p className="warning">
            The operator observation and final composite record remain manual evidence steps and
            are not created by this application.
          </p>
        </section>
      )}
      {error && (
        <p role="alert" className="warning">
          {error}
        </p>
      )}
    </main>
  );
}

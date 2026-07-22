import { executionDuration } from "./app-helpers";
import type { ExecutionWorkflowState } from "./workflow";

type PresentedExecution = Extract<
  ExecutionWorkflowState,
  { kind: "active" | "terminal" | "unavailable" }
>;

type ReportState = "idle" | "exporting" | "saved" | "failed";
type LaunchState = "idle" | "launching" | "launched" | "failed";

function localTimestamp(value: string | null): string {
  if (!value) return "Starting";
  const timestamp = new Date(value);
  if (Number.isNaN(timestamp.getTime())) return "Unavailable";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(timestamp);
}

interface ExecutionStepProps {
  execution: PresentedExecution;
  launchState: LaunchState;
  repairPreparing: boolean;
  reportState: ReportState;
  onCancel: () => void;
  onExportReport: () => void;
  onLaunchConfiguredApp: () => void;
  onPrepareRepair: () => void;
  onReturn: () => void;
}

export function ExecutionStep({
  execution,
  launchState,
  repairPreparing,
  reportState,
  onCancel,
  onExportReport,
  onLaunchConfiguredApp,
  onPrepareRepair,
  onReturn,
}: ExecutionStepProps) {
  if (execution.kind === "unavailable") {
    return (
      <>
        <p className="eyebrow">
          {execution.mode === "real" ? "REAL-DEVICE OUTCOME UNKNOWN" : "SIMULATED RUN UNAVAILABLE"}
        </p>
        <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
          {execution.mode === "real"
            ? "The device may have been partially changed"
            : "This in-memory simulation cannot be resumed"}
        </h2>
        <p className="warning">{execution.message}</p>
        <p>
          {execution.mode === "real"
            ? "The outcome cannot be inferred. Reconnect and create a fresh review; this execution cannot be resumed, retried in place, restored, or rolled back."
            : "No execution history is persisted across an app or sidecar restart."}
        </p>
        <button onClick={onPrepareRepair} disabled={repairPreparing}>
          {repairPreparing ? "Preparing fresh plan…" : "Repair configuration"}
        </button>
        <button className="secondary" onClick={onReturn}>
          {execution.mode === "real" ? "Start a fresh workflow" : "View previous review"}
        </button>
      </>
    );
  }

  const { snapshot } = execution;
  const counts = snapshot.completion.counts;
  const completedCount = counts.completed
    + counts.skipped
    + counts.blocked
    + counts.failed
    + counts.cancelled;
  const duration = executionDuration(snapshot);

  return (
    <>
      <p className="eyebrow">
        {execution.mode === "real" ? "REAL DEVICE" : "SIMULATED / DRY RUN"}
      </p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        {execution.kind === "terminal"
          ? `${execution.mode === "real" ? "Real-device execution" : "Simulation"} ${snapshot.status.replaceAll("_", " ")}`
          : execution.mode === "real"
            ? "Applying the reviewed setup"
            : "Simulating the reviewed setup"}
      </h2>
      {counts.total > 0 ? (
        <div className="execution-progress">
          <label htmlFor="execution-progress">
            Execution progress: {Math.round((completedCount / counts.total) * 100)}%
          </label>
          <progress id="execution-progress" max={counts.total} value={completedCount} />
        </div>
      ) : (
        <p aria-busy="true" role="status">
          Execution progress is starting; the total step count is not available yet.
        </p>
      )}
      {snapshot.progress.currentAction && (
        <p aria-live="polite" role="status">
          <strong>Current action:</strong> {snapshot.progress.currentAction}
          {snapshot.progress.currentFeature && ` · ${snapshot.progress.currentFeature}`}
        </p>
      )}
      {execution.mode === "real" ? (
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
        <div><dt>Status</dt><dd>{snapshot.status.replaceAll("_", " ")}</dd></div>
        <div><dt>Started</dt><dd>{localTimestamp(snapshot.startedAt)}</dd></div>
        {duration && <div><dt>Duration</dt><dd>{duration}</dd></div>}
        <div><dt>Completed</dt><dd>{counts.completed}</dd></div>
        <div><dt>Skipped</dt><dd>{counts.skipped}</dd></div>
        <div><dt>Blocked</dt><dd>{counts.blocked}</dd></div>
        <div><dt>Failed</dt><dd>{counts.failed}</dd></div>
      </dl>
      {snapshot.completion.partialChangesPossible && (
        <p className="warning">
          Some device changes completed before this {snapshot.status} result.
          The result remains {snapshot.status}; EmuChef does not infer partial success or rollback completed work.
        </p>
      )}
      {snapshot.warnings.map((issue, index) => (
        <div className="warning" key={`warning-${index}`}>
          <p>{issue.message}</p>
          <small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
        </div>
      ))}
      {snapshot.errors.map((issue, index) => (
        <div className="error" key={`error-${index}`}>
          <p>{issue.message}</p>
          <small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
        </div>
      ))}
      {snapshot.recipes.map((recipe, recipeIndex) => (
        <article className={`execution-group status-${recipe.status}`} key={`${recipe.name}-${recipeIndex}`}>
          <div className="execution-heading">
            <div>
              <h3>{recipe.name}</h3>
              {recipe.description && <p>{recipe.description}</p>}
            </div>
            <span className="execution-status">{recipe.status.replaceAll("_", " ")}</span>
          </div>
          <ol>
            {recipe.steps.map((step, stepIndex) => (
              <li key={`${step.name}-${stepIndex}`} className={`step-${step.status}`}>
                <strong>{step.note ?? step.name}</strong>
                <span>{step.status.replaceAll("_", " ")}</span>
                {step.note && step.note !== step.name && <small>{step.name}</small>}
                {step.message && <small>{step.message}</small>}
              </li>
            ))}
          </ol>
        </article>
      ))}
      {execution.events.length > 0 && (
        <details className="execution-events">
          <summary>
            Incremental {execution.mode === "real" ? "real-device" : "simulated"} event log
          </summary>
          <ol>
            {execution.events.map((event) => (
              <li key={event.sequence}>
                <time>{localTimestamp(event.timestamp)}</time> {event.label}
                {event.status && ` · ${event.status.replaceAll("_", " ")}`}
              </li>
            ))}
          </ol>
        </details>
      )}
      {execution.kind === "active" ? (
        <>
          {execution.cancellationRequested && (
            <p className="warning">
              {execution.mode === "real"
                ? "Cancellation requested. The current atomic operation may finish; completed device changes are not reversed, and no new work starts after cancellation is observed."
                : "Cancellation requested. Completed simulated steps remain visible in this report. No new simulated steps start, the current simulated atomic step may finish, and no real device changes or rollback exist."}
            </p>
          )}
          <button
            aria-describedby={execution.cancellationRequested ? "cancellation-requested-reason" : undefined}
            className="danger"
            disabled={execution.cancellationRequested}
            onClick={onCancel}
          >
            {execution.cancellationRequested
              ? "Cancellation requested"
              : execution.mode === "real"
                ? "Request cancellation"
                : "Cancel simulated run"}
          </button>
          {execution.cancellationRequested && (
            <p className="disabled-reason" id="cancellation-requested-reason">
              A cancellation request is already pending; the current atomic operation may still finish.
            </p>
          )}
        </>
      ) : (
        <div className="button-row">
          <button
            className="secondary"
            disabled={reportState === "exporting"}
            onClick={onExportReport}
          >
            {reportState === "exporting"
              ? "Exporting…"
              : reportState === "saved"
                ? "Report saved"
                : "Export report"}
          </button>
          {snapshot.status !== "succeeded" && (
            <div className="execution-repair-action">
              <button
                aria-describedby="execution-repair-explanation"
                onClick={onPrepareRepair}
                disabled={repairPreparing}
              >
                {repairPreparing
                  ? "Preparing fresh plan…"
                  : snapshot.status === "succeeded_with_warnings"
                    ? "Repair configuration"
                    : "Repair setup"}
              </button>
              <p className="disabled-reason" id="execution-repair-explanation">
                EmuChef will return to setup, preserve reusable choices, and require a fresh plan and review before another run.
                Completed steps remain report evidence and are not retried in place.
              </p>
            </div>
          )}
          {!snapshot.simulated && snapshot.launchAction && (
            <button
              disabled={launchState === "launching" || launchState === "launched"}
              onClick={onLaunchConfiguredApp}
            >
              {launchState === "launching"
                ? "Launching…"
                : launchState === "launched"
                  ? "App launched"
                  : snapshot.launchAction.label}
            </button>
          )}
          <button className="secondary" onClick={onReturn}>
            {execution.mode === "real"
              ? "Start a fresh workflow"
              : matchesFreshReviewRequirement(snapshot.status)
                ? "View previous review"
                : "Return to Review"}
          </button>
        </div>
      )}
    </>
  );
}

function matchesFreshReviewRequirement(status: string): boolean {
  return status === "failed" || status === "cancelled";
}

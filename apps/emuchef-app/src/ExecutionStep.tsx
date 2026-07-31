import { executionDuration } from "./app-helpers";
import type {
  ExecutionStatus,
  RecipeExecutionStatus,
  StepExecutionStatus,
} from "./types";
import type { ExecutionWorkflowState } from "./workflow";

type PresentedExecution = Extract<
  ExecutionWorkflowState,
  { kind: "active" | "terminal" | "unavailable" }
>;

type ReportState = "idle" | "exporting" | "saved" | "failed";
type LaunchState = "idle" | "launching" | "launched" | "failed";

const EXECUTION_STATUS_LABELS: Record<ExecutionStatus, string> = {
  queued: "Queued",
  running: "In progress",
  succeeded: "Completed",
  succeeded_with_warnings: "Completed with warnings",
  failed: "Failed",
  cancelled: "Cancelled",
};

const RECIPE_STATUS_LABELS: Record<RecipeExecutionStatus, string> = {
  pending: "Waiting",
  running: "In progress",
  succeeded: "Completed",
  succeeded_with_warnings: "Completed with warnings",
  failed: "Failed",
  blocked: "Blocked",
  cancelled: "Cancelled",
};

const STEP_STATUS_LABELS: Record<StepExecutionStatus, string> = {
  pending: "Waiting",
  running: "In progress",
  succeeded: "Completed",
  skipped: "Skipped",
  failed: "Failed",
  blocked: "Blocked",
  cancelled: "Cancelled",
};

function eventStatusLabel(status: string | null): string | null {
  if (!status) return null;
  return ({ ...EXECUTION_STATUS_LABELS, ...RECIPE_STATUS_LABELS, ...STEP_STATUS_LABELS } as Record<string, string>)[status]
    ?? "Updated";
}

function recipeStatusLabel(status: RecipeExecutionStatus, terminal: boolean): string {
  return terminal && status === "pending" ? "Not attempted" : RECIPE_STATUS_LABELS[status];
}

function stepStatusLabel(status: StepExecutionStatus, terminal: boolean): string {
  return terminal && status === "pending" ? "Not attempted" : STEP_STATUS_LABELS[status];
}

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
          {execution.mode === "real" ? "Real-device result unavailable" : "Simulation unavailable"}
        </p>
        <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
          {execution.mode === "real"
            ? "The device may have been partially changed"
            : "This in-memory simulation cannot be resumed"}
        </h2>
        <p className="warning">{execution.message}</p>
        <p>
          {execution.mode === "real"
            ? "The outcome cannot be inferred. Reconnect and create a fresh review; this installation cannot be resumed, retried in place, restored, or rolled back."
            : "Simulation history is not kept after the app service restarts."}
        </p>
        <button onClick={onPrepareRepair} disabled={repairPreparing}>
          {repairPreparing ? "Preparing fresh plan…" : "Repair setup"}
        </button>
        <button className="secondary" onClick={onReturn}>
          {execution.mode === "real" ? "Start a fresh workflow" : "View previous review"}
        </button>
      </>
    );
  }

  const { snapshot } = execution;
  const terminal = execution.kind === "terminal";
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
        {execution.mode === "real" ? "Real device" : "Simulation"}
      </p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        {execution.kind === "terminal"
          ? `${execution.mode === "real" ? "Real-device installation" : "Simulation"} ${EXECUTION_STATUS_LABELS[snapshot.status].toLowerCase()}`
          : execution.mode === "real"
            ? "Applying the reviewed setup"
            : "Simulating the reviewed setup"}
      </h2>
      {counts.total > 0 ? (
        <div className="execution-progress">
          <label htmlFor="execution-progress">
            {execution.mode === "real" ? "Installation" : "Simulation"} progress: {Math.round((completedCount / counts.total) * 100)}%
          </label>
          <progress id="execution-progress" max={counts.total} value={completedCount} />
        </div>
      ) : (
        <p aria-busy="true" role="status">
          {execution.mode === "real" ? "Installation" : "Simulation"} progress is starting; the total step count is not available yet.
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
        <div><dt>Status</dt><dd>{EXECUTION_STATUS_LABELS[snapshot.status]}</dd></div>
        <div><dt>Started</dt><dd>{localTimestamp(snapshot.startedAt)}</dd></div>
        {duration && <div><dt>Duration</dt><dd>{duration}</dd></div>}
        <div><dt>Completed</dt><dd>{counts.completed}</dd></div>
        <div><dt>Skipped</dt><dd>{counts.skipped}</dd></div>
        <div><dt>Blocked</dt><dd>{counts.blocked}</dd></div>
        <div><dt>Failed</dt><dd>{counts.failed}</dd></div>
        <div><dt>Cancelled</dt><dd>{counts.cancelled}</dd></div>
        <div><dt>{terminal ? "Not attempted" : "Waiting"}</dt><dd>{counts.pending}</dd></div>
      </dl>
      {snapshot.completion.partialChangesPossible && (
        <p className="warning">
          {counts.completed > 0
            ? `Some device changes completed before this ${EXECUTION_STATUS_LABELS[snapshot.status].toLowerCase()} result.`
            : `Device changes may have occurred before this ${EXECUTION_STATUS_LABELS[snapshot.status].toLowerCase()} result.`}
          {" "}The result remains {EXECUTION_STATUS_LABELS[snapshot.status].toLowerCase()}; EmuChef cannot determine whether a failed operation changed the device and does not infer rollback.
        </p>
      )}
      {(snapshot.warnings.length > 0 || snapshot.errors.length > 0) && (
        <section aria-label="Run notices" className="result-card-list">
          {snapshot.warnings.map((issue, index) => (
            <article className="result-card result-warning" key={`warning-${index}`}>
              <h3>Warning {index + 1}</h3>
              <p>{issue.message}</p>
              <small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
            </article>
          ))}
          {snapshot.errors.map((issue, index) => (
            <article className="result-card result-failed" key={`error-${index}`}>
              <h3>Problem {index + 1}</h3>
              <p>{issue.message}</p>
              <small><strong>{issue.remediation.title}:</strong> {issue.remediation.message}</small>
            </article>
          ))}
        </section>
      )}
      {snapshot.recipes.map((recipe, recipeIndex) => (
        <article className={`execution-group status-${recipe.status}`} key={`${recipe.name}-${recipeIndex}`}>
          <div className="execution-heading">
            <div>
              <h3>{recipe.name}</h3>
              {recipe.description && <p>{recipe.description}</p>}
            </div>
            <span className="execution-status">{recipeStatusLabel(recipe.status, terminal)}</span>
          </div>
          <ol>
            {recipe.steps.map((step, stepIndex) => (
              <li key={`${step.name}-${stepIndex}`} className={`step-${step.status}`}>
                <strong>{step.note ?? step.name}</strong>
                <span>{stepStatusLabel(step.status, terminal)}</span>
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
                {event.status && ` · ${eventStatusLabel(event.status)}`}
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
                    ? "Repair setup"
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

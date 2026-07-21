import type { WorkflowState } from "./workflow";

interface ReviewStepProps {
  busy: boolean;
  executionKind: WorkflowState["execution"]["kind"];
  realExecutionEnabled: boolean;
  review: NonNullable<WorkflowState["review"]>;
  reviewStale: boolean;
  onApplyToDevice: (invoker: HTMLElement) => void;
  onBack: () => void;
  onStartSimulation: () => void;
}

export function ReviewStep({
  busy,
  executionKind,
  realExecutionEnabled,
  review,
  reviewStale,
  onApplyToDevice,
  onBack,
  onStartSimulation,
}: ReviewStepProps) {
  const executionStarting = executionKind === "starting";
  const startDisabled = busy || executionStarting || reviewStale;

  return (
    <>
      <p className="eyebrow">REVIEW PLAN</p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        Ready for a simulated dry run
      </h2>
      <p className="simulation-banner">
        Simulated / Dry Run only. This does not change or verify the real device.
      </p>
      {reviewStale && (
        <section className="simulation-banner" role="alert" aria-labelledby="stale-review-heading">
          <h3 id="stale-review-heading">Review is out of date</h3>
          <p>
            This plan belongs to an earlier execution attempt and cannot be run again. Return to setup and repair the configuration to generate a fresh plan and review.
          </p>
        </section>
      )}
      <p>
        Target: {review.target.manufacturer ?? "Android"} {review.target.model ?? "device"}
        · Android {review.target.androidVersion ?? "unknown"}
      </p>
      {review.selectedInputs.length > 0 && (
        <section className="review-inputs" aria-labelledby="selected-options-heading">
          <h3 id="selected-options-heading">Selected options</h3>
          <dl>
            {review.selectedInputs.map((input) => (
              <div key={input.key}>
                <dt>{input.key}</dt>
                <dd>{input.value}</dd>
              </div>
            ))}
          </dl>
        </section>
      )}
      {review.groups.map((group) => (
        <article className="review-group" key={group.recipeId}>
          <h3>{group.recipeName}</h3>
          {group.recipeDescription && <p>{group.recipeDescription}</p>}
          <ol>
            {group.steps.map((step) => (
              <li key={step.technicalId}>
                <strong>{step.name}</strong>
                {step.note && <span>{step.note}</span>}
                <span>{step.kindLabel}</span>
                {step.elevated && <em>Elevated access</em>}
                {step.requirements.length > 0 && (
                  <small>Requires: {step.requirements.join(", ")}</small>
                )}
                <details>
                  <summary>Technical details</summary>
                  <code>{step.technicalId} · {step.technicalType}</code>
                </details>
              </li>
            ))}
          </ol>
        </article>
      ))}
      <p className="digest">Plan digest: {review.planDigest}</p>
      <div className="button-row">
        <button className="secondary" onClick={onBack}>Back</button>
        <button
          aria-describedby={startDisabled ? "execution-start-reason" : undefined}
          disabled={startDisabled}
          onClick={onStartSimulation}
        >
          {executionStarting ? "Starting simulated run…" : "Start Simulated Dry Run"}
        </button>
        {realExecutionEnabled && (
          <button
            className="danger"
            disabled={startDisabled}
            onClick={(event) => onApplyToDevice(event.currentTarget)}
          >
            Apply to Device
          </button>
        )}
      </div>
      {startDisabled && (
        <p className="disabled-reason" id="execution-start-reason">
          {reviewStale
            ? "This reviewed plan is stale. Repair the configuration and generate a fresh review before running again."
            : "Execution start is already being prepared."}
        </p>
      )}
    </>
  );
}

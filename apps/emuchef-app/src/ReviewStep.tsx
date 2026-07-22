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
  const startDisabled = busy || executionStarting || reviewStale || !review.canExecute;
  const targetIdentity = [review.target.manufacturer, review.target.model].filter(Boolean).join(" ");
  const androidDetails = [
    review.target.androidVersion === undefined ? null : `Android ${review.target.androidVersion}`,
    review.target.androidApiLevel === undefined ? null : `API ${review.target.androidApiLevel}`,
  ].filter(Boolean).join(" · ");

  return (
    <>
      <p className="eyebrow">Review</p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        Review this setup
      </h2>
      {!realExecutionEnabled && (
        <p className="simulation-banner">
          Simulation only. This does not change or verify the real device.
        </p>
      )}
      {reviewStale && (
        <section className="simulation-banner" role="alert" aria-labelledby="stale-review-heading">
          <h3 id="stale-review-heading">Review is out of date</h3>
          <p>
            This plan belongs to an earlier attempt and cannot be run again. Return to setup and repair the saved choices to generate a fresh plan and review.
          </p>
        </section>
      )}
      <section aria-labelledby="review-setup-heading">
        <h3 id="review-setup-heading">{review.setup.name}</h3>
        {review.setup.description && <p>{review.setup.description}</p>}
        <p>
          {review.target.label}: {targetIdentity || "Android device"}
          {androidDetails && <> · {androidDetails}</>}
        </p>
      </section>
      {review.inputs.length > 0 && (
        <section className="review-inputs" aria-labelledby="selected-options-heading">
          <h3 id="selected-options-heading">Selected options</h3>
          <dl>
            {review.inputs.map((input, index) => (
              <div key={`${input.label}-${index}`}>
                <dt>{input.label}</dt>
                <dd>{input.summary}{input.required ? " · Required" : ""}</dd>
              </div>
            ))}
          </dl>
        </section>
      )}
      {review.features.map((feature, featureIndex) => (
        <article className="review-group" key={`${feature.name}-${featureIndex}`}>
          <h3>
            {feature.name}
            {feature.automaticallyAdded && <small> · Added automatically</small>}
          </h3>
          {feature.description && <p>{feature.description}</p>}
          {feature.sections.map((section) => (
            <section key={section.kind} aria-labelledby={`feature-${featureIndex}-${section.kind}`}>
              <h4 id={`feature-${featureIndex}-${section.kind}`}>{section.label}</h4>
              <ol>
                {section.actions.map((action, actionIndex) => (
                  <li key={`${action.title}-${actionIndex}`}>
                    <strong>{action.title}</strong>
                    {action.description && <span>{action.description}</span>}
                    {action.requirement === "conditional" && <em>Conditional</em>}
                    {action.deviceLocation && <small>Destination: {action.deviceLocation}</small>}
                  </li>
                ))}
              </ol>
            </section>
          ))}
        </article>
      ))}
      {review.notices.map((notice, index) => (
        <section
          className="simulation-banner"
          role={notice.severity === "blocker" ? "alert" : "status"}
          key={`${notice.title}-${index}`}
        >
          <h3>{notice.title}</h3>
          <p>{notice.message}</p>
        </section>
      ))}
      <p>
        {review.work.actionCount} {review.work.actionCount === 1 ? "action" : "actions"}
        {review.work.knownWaitSeconds !== undefined && ` · ${review.work.knownWaitSeconds} seconds of known waits`}
      </p>
      <div className="button-row">
        <button className="secondary" onClick={onBack}>Back</button>
        <button
          aria-describedby={startDisabled ? "execution-start-reason" : undefined}
          disabled={startDisabled}
          onClick={onStartSimulation}
        >
          {executionStarting ? "Starting simulation…" : "Start simulation"}
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
            ? "This reviewed plan is out of date. Repair the setup and generate a fresh review before running again."
            : !review.canExecute
              ? "This plan contains work EmuChef cannot review safely. Update EmuChef or the setup catalog before continuing."
            : "The installation is already being prepared."}
        </p>
      )}
    </>
  );
}

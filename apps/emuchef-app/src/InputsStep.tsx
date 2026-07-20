import type { RefObject } from "react";
import { describedBy, stableDomId } from "./accessibility";
import { diagnosticIsBlocking, groupedInputs } from "./app-helpers";
import type {
  ConfigurationDescription,
  InputDescriptor,
  ValidationDiagnostic,
} from "./types";
import { inputDiagnosticsForDisplay, pageDiagnosticsForDisplay } from "./workflow";
type ValidationError = ValidationDiagnostic & {
  targetId: string | null;
};

interface InputsStepProps {
  description: ConfigurationDescription;
  bindings: Record<string, unknown>;
  busy: boolean;
  touchedInputKeys: Set<string>;
  validationRequested: boolean;
  validationErrors: ValidationError[];
  validationSummaryRef: RefObject<HTMLElement | null>;
  onBack: () => void;
  onBindingChange: (key: string, value: unknown) => void;
  onPickInput: (input: InputDescriptor) => void | Promise<void>;
  onRefreshValidation: () => void | Promise<void>;
  onReview: () => void | Promise<void>;
}

export function InputsStep({
  description,
  bindings,
  busy,
  touchedInputKeys,
  validationRequested,
  validationErrors,
  validationSummaryRef,
  onBack,
  onBindingChange,
  onPickInput,
  onRefreshValidation,
  onReview,
}: InputsStepProps) {
  return (
    <>
      <p className="eyebrow">CONFIGURE INPUTS</p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        Provide required files and options
      </h2>

      {validationErrors.length > 0 && (
        <section
          aria-labelledby="validation-summary-heading"
          className="error error-summary"
          ref={validationSummaryRef}
          role="alert"
          tabIndex={-1}
        >
          <h3 id="validation-summary-heading">
            Resolve {validationErrors.length} configuration{" "}
            {validationErrors.length === 1 ? "error" : "errors"}
          </h3>
          <ul>
            {validationErrors.map((item, index) => (
              <li key={`${item.key ?? "global"}-${item.code}-${index}`}>
                {item.targetId ? <a href={`#${item.targetId}`}>{item.message}</a> : item.message}
                <details>
                  <summary>Technical details</summary>
                  <code>{item.code}</code>
                </details>
              </li>
            ))}
          </ul>
        </section>
      )}

      {description.inputs.length === 0 && (
        <p className="success">The selected recipes do not require additional input.</p>
      )}

      {groupedInputs(description.inputs).map((group) => (
        <section
          aria-labelledby={stableDomId("input-group", group.category)}
          className="input-group"
          key={group.category}
        >
          <h3 id={stableDomId("input-group", group.category)}>{group.category}</h3>
          {group.inputs.map((input) => {
            const inputId = stableDomId("input", input.key);
            const descriptionId = `${inputId}-description`;
            const extensionsId = `${inputId}-extensions`;
            const sourceId = `${inputId}-source`;
            const diagnostics = inputDiagnosticsForDisplay(input).filter((diagnostic) =>
              diagnosticIsBlocking(
                diagnostic.code,
                diagnostic.key ?? input.key,
                validationRequested,
                touchedInputKeys,
              ),
            );
            const diagnosticIds = diagnostics.map((_, index) => `${inputId}-error-${index}`);

            return (
              <div className="input-field" key={input.key}>
                <div className="input-field-heading">
                  <label htmlFor={inputId}>{input.label}</label>
                  <span className="input-requirements" aria-label={`${input.label} requirements`}>
                    <span className={input.required ? "input-requirement required" : "input-requirement"}>
                      {input.required ? "Required" : "Optional"}
                    </span>
                    <span className="input-requirement">{input.presentationKind ?? "Text"}</span>
                    {input.sensitive && (
                      <span className="input-requirement sensitive">Not saved</span>
                    )}
                  </span>
                </div>

                {input.type === "boolean" ? (
                  <input
                    aria-describedby={describedBy(input.description && descriptionId, ...diagnosticIds)}
                    aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                    checked={Boolean(bindings[input.key] ?? input.value)}
                    id={inputId}
                    onChange={(event) => onBindingChange(input.key, event.target.checked)}
                    type="checkbox"
                  />
                ) : input.options?.length ? (
                  <select
                    aria-describedby={describedBy(
                      input.description && descriptionId,
                      input.valueSource && sourceId,
                      ...diagnosticIds,
                    )}
                    aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                    id={inputId}
                    onChange={(event) => onBindingChange(input.key, event.target.value)}
                    value={String(bindings[input.key] ?? input.value ?? "")}
                  >
                    <option value="">Choose…</option>
                    {input.options.map((option) => <option key={option}>{option}</option>)}
                  </select>
                ) : input.pathKind ? (
                  <div className="path-picker">
                    <input
                      aria-describedby={describedBy(
                        input.description && descriptionId,
                        Boolean(input.acceptedExtensions?.length) && extensionsId,
                        input.valueSource && sourceId,
                        ...diagnosticIds,
                      )}
                      aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                      id={inputId}
                      readOnly
                      value={String(bindings[input.key] ?? input.value ?? "")}
                    />
                    <button
                      aria-describedby={busy ? `${inputId}-browse-reason` : undefined}
                      className="secondary"
                      disabled={busy}
                      onClick={() => void onPickInput(input)}
                    >
                      Browse…
                    </button>
                    {busy && (
                      <small className="disabled-reason" id={`${inputId}-browse-reason`}>
                        A file or validation operation is already in progress.
                      </small>
                    )}
                  </div>
                ) : (
                  <input
                    aria-describedby={describedBy(
                      input.description && descriptionId,
                      input.valueSource && sourceId,
                      ...diagnosticIds,
                    )}
                    aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                    id={inputId}
                    onChange={(event) => onBindingChange(input.key, event.target.value)}
                    value={String(bindings[input.key] ?? input.value ?? "")}
                  />
                )}

                {input.description && <small id={descriptionId}>{input.description}</small>}
                {input.acceptedExtensions?.length ? (
                  <small id={extensionsId}>
                    Accepted file types: {input.acceptedExtensions.join(", ")}
                  </small>
                ) : null}
                {input.valueSource ? (
                  <small id={sourceId}>Value source: {input.valueSource.replaceAll("_", " ")}</small>
                ) : null}
                {diagnostics.map((item, index) => (
                  <small
                    className="error"
                    id={diagnosticIds[index]}
                    key={`${item.key ?? input.key}-${item.code}`}
                  >
                    Error: {item.message}
                  </small>
                ))}
              </div>
            );
          })}
        </section>
      ))}

      {pageDiagnosticsForDisplay(description)
        .filter((diagnostic) =>
          diagnosticIsBlocking(
            diagnostic.code,
            diagnostic.key,
            validationRequested,
            touchedInputKeys,
          ),
        )
        .map((item) => (
          <p
            className={item.severity === "error" ? "error" : "warning"}
            key={`${item.key ?? "global"}-${item.code}-${item.message}`}
          >
            {item.severity === "error" ? "Error: " : "Warning: "}
            {item.message}
          </p>
        ))}

      <div className="button-row">
        <button className="secondary" onClick={onBack}>Back</button>
        <button className="secondary" disabled={busy} onClick={() => void onRefreshValidation()}>
          Refresh validation
        </button>
        <button
          aria-describedby={busy ? "review-disabled-reason" : undefined}
          disabled={busy}
          onClick={() => void onReview()}
        >
          Review plan
        </button>
      </div>
      {busy && (
        <p className="disabled-reason" id="review-disabled-reason">
          Validation is in progress.
        </p>
      )}
    </>
  );
}

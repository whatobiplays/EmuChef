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
  onClearInput: (input: InputDescriptor) => void;
  onPickInput: (
    input: InputDescriptor,
    mode?: "replace_all" | "append" | "replace_entry",
    entryIndex?: number | null,
  ) => void | Promise<void>;
  onRemoveInputEntry: (input: InputDescriptor, entryIndex: number) => void;
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
  onClearInput,
  onPickInput,
  onRemoveInputEntry,
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
            const fieldDiagnostics = diagnostics.filter((diagnostic) => diagnostic.entryIndex == null);
            const diagnosticIds = diagnostics.map((_, index) => `${inputId}-error-${index}`);
            const currentValue = bindings[input.key] ?? input.value;
            const hasValue = currentValue !== null
              && currentValue !== undefined
              && currentValue !== ""
              && (!Array.isArray(currentValue) || currentValue.length > 0);
            const rawEntries = Array.isArray(currentValue)
              ? currentValue.filter((value): value is string => typeof value === "string")
              : typeof currentValue === "string" && currentValue ? [currentValue] : [];
            const entries = input.entries?.length === rawEntries.length
              ? input.entries
              : rawEntries.map((path, index) => ({
                  index,
                  displayName: path,
                  displayPath: path,
                  state: "valid" as const,
                  diagnostics: [],
                }));

            return (
              <div className="input-field" key={input.key}>
                <div className="input-field-heading">
                  <label htmlFor={input.pathKind ? undefined : inputId} id={`${inputId}-label`}>{input.label}</label>
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
                  <div aria-labelledby={`${inputId}-label`} className="path-picker" id={inputId} role="group">
                    {entries.length > 0 ? (
                      <ul className="input-entry-list" aria-label={`${input.label} selected items`}>
                        {entries.map((entry) => {
                          const entryDiagnostics = diagnostics.filter(
                            (diagnostic) => diagnostic.entryIndex === entry.index,
                          );
                          const needsRelink = entryDiagnostics.some((diagnostic) =>
                            [
                              "binding_path_missing",
                              "binding_path_inaccessible",
                              "binding_path_kind_mismatch",
                            ].includes(diagnostic.code),
                          );
                          return (
                            <li className={`input-entry ${entry.state}`} key={`${entry.index}-${entry.displayPath}`}>
                              <span>
                                <strong>{entry.displayName}</strong>
                                <small>{entry.displayPath}</small>
                              </span>
                              <span className="input-entry-actions">
                                <button
                                  className="secondary"
                                  disabled={busy}
                                  onClick={() => void onPickInput(
                                    input,
                                    input.multiple ? "replace_entry" : "replace_all",
                                    input.multiple ? entry.index : null,
                                  )}
                                  type="button"
                                >
                                  {needsRelink ? "Relink…" : "Replace…"}
                                </button>
                                {input.multiple && (
                                  <button
                                    aria-label={`Remove ${entry.displayName} from ${input.label}`}
                                    className="secondary"
                                    disabled={busy}
                                    onClick={() => onRemoveInputEntry(input, entry.index)}
                                    type="button"
                                  >
                                    Remove
                                  </button>
                                )}
                              </span>
                              {entryDiagnostics.map((diagnostic, index) => (
                                <small className={diagnostic.severity === "error" ? "error" : "warning"} key={`${diagnostic.code}-${index}`}>
                                  {diagnostic.severity === "error" ? "Error: " : "Warning: "}{diagnostic.message}
                                </small>
                              ))}
                            </li>
                          );
                        })}
                      </ul>
                    ) : (
                      <p className="empty-state">No {input.multiple ? "files" : input.pathKind === "directory" ? "folder" : "file"} selected.</p>
                    )}
                    <div className="button-row">
                      <button
                        aria-describedby={busy ? `${inputId}-browse-reason` : undefined}
                        className="secondary"
                        disabled={busy}
                        onClick={() => void onPickInput(input, input.multiple && hasValue ? "append" : "replace_all")}
                        type="button"
                      >
                        {input.multiple ? (hasValue ? "Add files…" : "Choose files…") : hasValue ? "Replace…" : input.pathKind === "directory" ? "Choose folder…" : "Choose file…"}
                      </button>
                      {hasValue && (
                        <button
                          aria-label={`Clear ${input.label}`}
                          className="secondary"
                          disabled={busy}
                          onClick={() => onClearInput(input)}
                          type="button"
                        >
                          Clear
                        </button>
                      )}
                    </div>
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
                    autoComplete={input.sensitive ? "off" : undefined}
                    id={inputId}
                    onChange={(event) => onBindingChange(input.key, event.target.value)}
                    type={input.sensitive ? "password" : "text"}
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
                {fieldDiagnostics.map((item, index) => (
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

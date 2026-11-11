import { useState, type RefObject } from "react";
import { describedBy, stableDomId } from "./accessibility";
import { diagnosticIsBlocking, groupedInputs } from "./app-helpers";
import type {
  ConfigurationDescription,
  InputDescriptor,
  SetupLocationProjection,
  ValidationDiagnostic,
} from "./types";
import { inputDiagnosticsForDisplay, pageDiagnosticsForDisplay } from "./workflow";
import type { ExecutionMode } from "./workflow";
type ValidationError = ValidationDiagnostic & {
  targetId: string | null;
};

function valueSourceLabel(source: NonNullable<InputDescriptor["valueSource"]>): string {
  switch (source) {
    case "explicit":
      return "Entered for this setup";
    case "user_configuration":
      return "Saved setup";
    case "device_plan":
      return "Device setup";
    case "recipe_default":
      return "Recommended default";
  }
}

interface InputsStepProps {
  description: ConfigurationDescription;
  setupLocations: SetupLocationProjection[];
  setupLocationListStatus: "loading" | "unavailable" | "ready";
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
  realExecutionCompiled?: boolean;
  onReview: (mode: ExecutionMode) => void | Promise<void>;
}

export function InputsStep({
  description,
  setupLocations,
  setupLocationListStatus,
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
  realExecutionCompiled = false,
  onReview,
}: InputsStepProps) {
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("simulated");
  const configurationSource = (recipeId: string) => {
    const sourceInput = description.inputs.find(
      (input) =>
        input.recipeId === recipeId && input.inputId === "retroarch_cfg_source",
    );
    if (!sourceInput) return undefined;
    if (
      touchedInputKeys.has(sourceInput.key) &&
      Object.prototype.hasOwnProperty.call(bindings, sourceInput.key)
    ) {
      return bindings[sourceInput.key];
    }
    return sourceInput.effectiveConfigurationSource ?? sourceInput.value;
  };

  const hasConfigurationSourceConflict = description.inputs.some(
    (input) =>
      input.inputId === "retroarch_cfg_source" &&
      configurationSource(input.recipeId) === "conflict",
  );

  const visibleInputs = description.inputs.filter((input) => {
    if (input.inputId !== "retroarch_video_threaded" && input.inputId !== "retroarch_cfg") {
      return true;
    }
    const source = configurationSource(input.recipeId);
    return input.inputId === "retroarch_video_threaded"
      ? source === "managed" || source === "conflict"
      : source === "uploaded" || source === "conflict";
  });

  const changeBinding = (key: string, value: unknown) => {
    const sourceInput = description.inputs.find(
      (input) => input.key === key && input.inputId === "retroarch_cfg_source",
    );
    onBindingChange(key, value);
    if (!sourceInput) return;

    const sourceInputs = description.inputs.filter(
      (input) =>
        input.recipeId === sourceInput.recipeId &&
        (input.inputId === "retroarch_video_threaded" || input.inputId === "retroarch_cfg"),
    );
    const selectedInputId =
      value === "managed"
        ? "retroarch_video_threaded"
        : value === "uploaded"
          ? "retroarch_cfg"
          : null;
    for (const input of sourceInputs) {
      if (input.inputId !== selectedInputId) onBindingChange(input.key, null);
    }

    if (value === "managed") {
      const managedInput = sourceInputs.find(
        (input) => input.inputId === "retroarch_video_threaded",
      );
      if (
        managedInput &&
        (!Object.prototype.hasOwnProperty.call(bindings, managedInput.key) ||
          bindings[managedInput.key] == null)
      ) {
        const defaultValue =
          managedInput.valueSource === "recipe_default" &&
          typeof managedInput.value === "boolean"
            ? managedInput.value
            : false;
        onBindingChange(managedInput.key, defaultValue);
      }
    }
  };

  return (
    <>
      <p className="eyebrow">Required files and choices</p>
      <h2 data-focus-fallback="workflow" data-step-heading tabIndex={-1}>
        Provide required files and options
      </h2>

      {hasConfigurationSourceConflict && (
        <p className="error" role="alert">
          Saved RetroArch configuration sources conflict. Choose a source to resolve the saved inputs.
        </p>
      )}

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

      {groupedInputs(visibleInputs).map((group) => (
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
            const setupLocationStatusId = `${inputId}-location-list-status`;
            const setupLocationStatusMessage = setupLocationListStatus === "loading"
              ? "Loading saved Setup Locations…"
              : setupLocationListStatus === "unavailable"
                ? "Saved Setup Locations could not be loaded. Refresh device storage to retry."
                : setupLocationListStatus === "ready" && setupLocations.length === 0
                  ? "No named Setup Locations are available for this device plan. Define one in device setup."
                  : null;
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
            const currentValue = input.inputId === "retroarch_cfg_source" &&
                !touchedInputKeys.has(input.key)
              ? input.effectiveConfigurationSource ?? input.value
              : Object.prototype.hasOwnProperty.call(bindings, input.key)
                ? bindings[input.key]
                : input.value;
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
                    checked={Boolean(currentValue)}
                    id={inputId}
                    onChange={(event) => onBindingChange(input.key, event.target.checked)}
                    type="checkbox"
                  />
                ) : input.type === "setup_location_ref" ? (
                  <>
                    <select
                      aria-describedby={describedBy(
                        input.description && descriptionId,
                        input.valueSource && sourceId,
                        setupLocationStatusMessage ? setupLocationStatusId : undefined,
                        ...diagnosticIds,
                      )}
                      aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                      id={inputId}
                      onChange={(event) => onBindingChange(input.key, event.target.value)}
                      value={String(currentValue ?? "")}
                    >
                      <option value="">Choose a Setup Location…</option>
                      {typeof currentValue === "string"
                        && currentValue
                        && !setupLocations.some((location) => location.id === currentValue) && (
                          <option disabled value={currentValue}>Unavailable Setup Location</option>
                        )}
                      {setupLocations.map((location) => (
                        <option disabled={!location.ready} key={location.id} value={location.id}>
                          {location.name} · {location.relativePath} · {location.volumeLabel}
                          {!location.ready ? ` · ${location.detail}` : ""}
                        </option>
                      ))}
                    </select>
                    {setupLocationStatusMessage && (
                      <small
                        aria-live="polite"
                        id={setupLocationStatusId}
                        role="status"
                      >
                        {setupLocationStatusMessage}
                      </small>
                    )}
                  </>
                ) : input.options?.length ? (
                  <select
                    aria-describedby={describedBy(
                      input.description && descriptionId,
                      input.valueSource && sourceId,
                      ...diagnosticIds,
                    )}
                    aria-invalid={diagnostics.some((item) => item.severity === "error") || undefined}
                    id={inputId}
                    onChange={(event) => changeBinding(input.key, event.target.value)}
                    value={String(currentValue ?? "")}
                  >
                    {input.inputId === "retroarch_cfg_source" && currentValue === "conflict" && (
                      <option disabled value="conflict">
                        Resolve saved source conflict
                      </option>
                    )}
                    <option value="">Choose…</option>
                    {input.options.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
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
                    value={String(currentValue ?? "")}
                  />
                )}

                {input.description && <small id={descriptionId}>{input.description}</small>}
                {input.acceptedExtensions?.length ? (
                  <small id={extensionsId}>
                    Accepted file types: {input.acceptedExtensions.join(", ")}
                  </small>
                ) : null}
                {input.valueSource ? (
                  <small id={sourceId}>Provided by: {valueSourceLabel(input.valueSource)}</small>
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
        {realExecutionCompiled && (
          <fieldset className="review-execution-mode">
            <legend>Execution mode for this review</legend>
            <label>
              <input
                checked={executionMode === "simulated"}
                name="review-execution-mode"
                onChange={() => setExecutionMode("simulated")}
                type="radio"
                value="simulated"
              />
              Simulation
            </label>
            <label>
              <input
                checked={executionMode === "real"}
                name="review-execution-mode"
                onChange={() => setExecutionMode("real")}
                type="radio"
                value="real"
              />
              Apply to device
            </label>
            <p>The review and its available action are tied to this mode.</p>
          </fieldset>
        )}
        <button
          aria-describedby={busy ? "review-disabled-reason" : undefined}
          disabled={busy}
          onClick={() => void onReview(executionMode)}
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

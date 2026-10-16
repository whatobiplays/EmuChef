import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type { EditorCommand } from "../api/commands";
import type {
  AppCatalogAppDto,
  AppCatalogDto,
  ArtifactDto,
  RecipeDto,
  RefIndexDto,
  StepDto,
  StepParamShapeDto,
  StepParamShapeFieldDto,
  StepSpecDto,
} from "../api/types";
import {
  addObjectListRow,
  addUniqueStringListValue,
  buildObjectListRowFieldUpdate,
  buildClearStepParamCommand,
  buildRefDependencyAction,
  buildRefPickerOptions,
  buildStepRefDependencyWarning,
  buildUpdateStepParamsCommand,
  displayValueForObjectField,
  isAppTargetValue,
  isAuthoredRefValue,
  literalSeedForParam,
  moveObjectListRow,
  moveStringListValue,
  objectListValue,
  objectValue,
  orderedParamNames,
  parseJsonParamDraft,
  parseNumberParamDraft,
  paramValuesEqual,
  removeObjectListRow,
  removeStringListValue,
  stringListValue,
  structuredParamEditorKind,
  updateObjectField,
  valueForObjectListRowFieldDraft,
  type RefPickerOption,
  type StepRefDependencyWarning,
} from "./stepParams.logic";
import {
  appTargetOptions,
  catalogApp,
  copyPolicyValue,
  installApkArtifactRefs,
  permissionSetChoices,
  refSourceValueType,
  type AppChoiceOption,
  type AppTargetOption,
} from "./appCatalog.logic";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

interface StepParamsEditorProps {
  readOnly?: boolean;
  recipe: RecipeDto;
  step: StepDto;
  stepSpec: StepSpecDto | null;
  refIndex: RefIndexDto;
  appCatalog: AppCatalogDto;
  onCommand: (command: EditorCommand) => Promise<boolean>;
  onUpdateDependencies: (dependencies: string[]) => Promise<boolean>;
}

export function StepParamsEditor({
  readOnly = false,
  recipe,
  step,
  stepSpec,
  refIndex,
  appCatalog,
  onCommand,
  onUpdateDependencies,
}: StepParamsEditorProps) {
  const paramNames = useMemo(() => orderedParamNames(step.params, stepSpec), [step.params, stepSpec]);

  async function updateParam(paramName: string, nextValue: unknown): Promise<boolean> {
    const command = buildUpdateStepParamsCommand(step.id, step.params, paramName, nextValue);
    if (command === null) {
      return true;
    }
    return onCommand(command);
  }

  async function clearParam(paramName: string): Promise<boolean> {
    const command = buildClearStepParamCommand(step.id, step.params, paramName);
    if (command === null) {
      return true;
    }
    return onCommand(command);
  }

  return (
    <div className="grid gap-3">
      <div>
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Params</h3>
      </div>
      {paramNames.length === 0 ? (
        <p className="text-sm text-slate-500">No params</p>
      ) : (
        <div className="divide-y divide-slate-200 rounded border border-slate-200">
          {paramNames.map((paramName) => (
            <StepParamRow
              key={paramName}
              paramName={paramName}
              readOnly={readOnly}
              recipe={recipe}
              refIndex={refIndex}
              step={step}
              stepSpec={stepSpec}
              appCatalog={appCatalog}
              value={Object.prototype.hasOwnProperty.call(step.params, paramName) ? step.params[paramName] : undefined}
              hasParam={Object.prototype.hasOwnProperty.call(step.params, paramName)}
              onClear={() => clearParam(paramName)}
              onUpdate={(nextValue) => updateParam(paramName, nextValue)}
              onUpdateDependencies={onUpdateDependencies}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function StepParamRow({
  paramName,
  value,
  hasParam,
  readOnly,
  recipe,
  step,
  stepSpec,
  refIndex,
  appCatalog,
  onUpdate,
  onClear,
  onUpdateDependencies,
}: {
  paramName: string;
  value: unknown;
  hasParam: boolean;
  readOnly: boolean;
  recipe: RecipeDto;
  step: StepDto;
  stepSpec: StepSpecDto | null;
  refIndex: RefIndexDto;
  appCatalog: AppCatalogDto;
  onUpdate: (nextValue: unknown) => Promise<boolean>;
  onClear: () => Promise<boolean>;
  onUpdateDependencies: (dependencies: string[]) => Promise<boolean>;
}) {
  const paramSpec = stepSpec?.params[paramName] ?? null;
  const shape = paramSpec?.shape;
  const structuredKind = structuredParamEditorKind(stepSpec, paramName, value);
  const isStructuredParam = structuredKind !== null;
  const enumValues =
    typeof value === "string" || value === undefined ? paramSpec?.enumValues ?? [] : [];
  const acceptedSources = paramSpec?.acceptedSources ?? [];
  const allowedRefSources = acceptedSources.filter((source) => source !== "literal" && source !== "app_target");
  const allowedValueTypes = paramSpec?.acceptedValueTypes ?? [];
  const selectedApp = catalogApp(appCatalog, step.appRef ?? null);
  const isPermissionSetParam = step.type === "grant_permissions" && paramName === "permission_set";
  const isAppTargetParam = acceptedSources.includes("app_target");
  const isInstallApkAppParam = step.type === "install_apk" && paramName === "app";
  // An app-aware APK install may only consume APK artifacts of its selected
  // App Definition. Legacy steps without app context keep every ref.
  const allowedInstallArtifactRefs =
    isInstallApkAppParam && step.appRef
      ? installApkArtifactRefs(appCatalog, recipe.artifacts, step.appRef)
      : null;
  const currentAppTargetId = isAppTargetValue(value) ? value.app_target : null;
  // App Target compatibility mirrors planning: the sibling `source` param
  // carries the copy origin type, and `copy_policy` decides whether a sync
  // copy needs a directory target. An absent policy falls back to the step
  // spec default and then to `merge`, matching the planner's fallback.
  const appTargetChoices = isAppTargetParam
    ? appTargetOptions(selectedApp, {
        sourceValueType: refSourceValueType(refIndex, step.params.source),
        copyPolicy: copyPolicyValue(step.params.copy_policy ?? stepSpec?.defaults.copy_policy ?? "merge"),
        currentTargetId: currentAppTargetId,
      })
    : [];
  // Seed a target the planner rules already accept, then fall back to a target
  // whose compatibility is unknown until validation. Incompatible targets and
  // retained values are never seeded.
  const selectableTargets = appTargetChoices.filter(
    (option) => !option.retained && option.compatibility !== "incompatible",
  );
  const appTargetSeed =
    (selectableTargets.find((option) => option.compatibility === "compatible") ?? selectableTargets[0])?.id ?? null;

  let control: ReactNode;
  if (isPermissionSetParam && (value === undefined || typeof value === "string")) {
    const options = permissionSetChoices(selectedApp, typeof value === "string" ? value : null);
    control = (
      <AppChoiceSelect
        disabled={readOnly}
        message={appOwnedChoiceMessage(step.appRef ?? null, selectedApp, options, {
          missingApp: "Select an App Definition to choose one of its defined permission sets.",
          none: "This App Definition defines no permission sets.",
        })}
        options={options}
        value={typeof value === "string" ? value : ""}
        onUpdate={onUpdate}
      />
    );
  } else if (isAppTargetParam && (isAppTargetValue(value) || value === undefined)) {
    control = (
      <AppTargetSelect
        appRef={step.appRef ?? null}
        disabled={readOnly}
        options={appTargetChoices}
        selectedApp={selectedApp}
        value={isAppTargetValue(value) ? value.app_target : ""}
        onUpdate={onUpdate}
      />
    );
  } else if (structuredKind === "artifact-id-list" && shape) {
    control = (
      <StringIdListParamInput
        addLabel="Add artifact"
        disabled={readOnly}
        emptyCatalogMessage="No artifacts exist in this recipe."
        emptyValueMessage="No artifact ids selected."
        options={artifactOptions(recipe.artifacts)}
        value={value}
        onUpdate={onUpdate}
      />
    );
  } else if (structuredKind === "artifact-group-id-list" && shape) {
    control = (
      <StringIdListParamInput
        addLabel="Add group"
        disabled={readOnly}
        emptyCatalogMessage="No artifact groups exist in this recipe."
        emptyValueMessage="No artifact group ids selected."
        options={artifactGroupOptions(recipe.artifactGroups)}
        value={value}
        onUpdate={onUpdate}
      />
    );
  } else if (structuredKind === "object-list" && shape) {
    control = <ObjectListParamInput disabled={readOnly} shape={shape} value={value} onUpdate={onUpdate} />;
  } else if (structuredKind === "object" && shape) {
    control = <ObjectParamInput disabled={readOnly} shape={shape} value={value} onUpdate={onUpdate} />;
  } else if (
    isAuthoredRefValue(value) ||
    (value === undefined && !acceptedSources.includes("literal") && allowedRefSources.length > 0)
  ) {
    // An absent ref parameter, such as the App Artifact of a newly added
    // install_apk step, still renders the catalog-backed ref picker so the
    // author never has to hand-author raw JSON to choose a value.
    const dependencyWarning = isAuthoredRefValue(value)
      ? buildStepRefDependencyWarning({
          refIndex,
          currentStepId: step.id,
          dependencyIds: step.dependencies,
          value,
        })
      : null;
    control = (
      <RefPicker
        allowedArtifactRefs={allowedInstallArtifactRefs}
        allowedSources={allowedRefSources}
        allowedValueTypes={allowedValueTypes}
        currentRef={isAuthoredRefValue(value) ? value.ref : ""}
        dependencyWarning={dependencyWarning}
        disabled={readOnly}
        refIndex={refIndex}
        onAddDependency={() => addRefDependency(step, dependencyWarning, onUpdateDependencies)}
        onUpdate={(ref) => onUpdate({ ref })}
      />
    );
  } else if (enumValues.length > 0) {
    control = <EnumParamInput disabled={readOnly} enumValues={enumValues} value={value} onUpdate={onUpdate} />;
  } else if (typeof value === "boolean" || (value === undefined && allowedValueTypes.includes("boolean"))) {
    control = <BooleanParamInput disabled={readOnly} value={value === true} onUpdate={onUpdate} />;
  } else if (typeof value === "number") {
    control = <NumberParamInput readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  } else if (typeof value === "string") {
    control = <StringParamInput readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  } else {
    control = <JsonValueEditor readOnly={readOnly} value={value} onUpdate={onUpdate} />;
  }

  const showValueSourceToggle =
    !isStructuredParam &&
    ((acceptedSources.includes("literal") && allowedRefSources.length > 0) || isAppTargetParam);
  if (showValueSourceToggle) {
    const compatibleRefs = buildRefPickerOptions(refIndex, {
      allowedSources: allowedRefSources,
      allowedValueTypes,
      currentRef: null,
      showAll: false,
    });
    const refSeed = compatibleRefs.find((option) => !option.missing && !option.incompatible)?.ref ?? null;
    control = (
      <div className="grid gap-2">
        <div className="flex gap-1" role="group" aria-label={`${paramName} value source`}>
          {acceptedSources.includes("literal") ? (
          // Switching to a literal value is an explicit author action. An
          // authored App Target or ref value stays untouched until this click,
          // and an absent value seeds a default literal so a required parameter
          // without App Target or ref choices remains authorable.
          <button
            className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
            disabled={readOnly || (value !== undefined && !isAuthoredRefValue(value) && !isAppTargetValue(value))}
            type="button"
            onClick={() =>
              void onUpdate(
                literalSeedForParam(allowedValueTypes, paramSpec?.enumValues ?? [], stepSpec?.defaults[paramName]),
              )
            }
          >
            Literal
          </button>
          ) : null}
          {allowedRefSources.length > 0 ? (
          <button
            className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
            disabled={readOnly || isAuthoredRefValue(value) || refSeed === null}
            type="button"
            onClick={() => {
              if (refSeed !== null) {
                void onUpdate({ ref: refSeed });
              }
            }}
          >
            Ref
          </button>
          ) : null}
          {isAppTargetParam ? (
            <button
              className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
              disabled={readOnly || isAppTargetValue(value) || appTargetSeed === null}
              type="button"
              onClick={() => {
                if (appTargetSeed !== null) {
                  void onUpdate({ app_target: appTargetSeed });
                }
              }}
            >
              App target
            </button>
          ) : null}
        </div>
        {control}
        {isAppTargetParam && !isAppTargetValue(value) && appTargetSeed === null ? (
          <p className="text-xs text-slate-500">
            {selectedApp === null
              ? "Select an App Definition to choose one of its App Targets."
              : selectedApp.targets.length === 0
                ? "This App Definition defines no App Targets."
                : "Every App Target of this App Definition is incompatible with the current copy source."}
          </p>
        ) : null}
      </div>
    );
  }

  if (isStructuredParam) {
    return (
      <div className="grid gap-3 bg-white p-3">
        <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
          <ParamNameBlock paramName={paramName} paramSpec={paramSpec} />
          {hasParam ? (
            <button
              className="h-9 rounded border border-slate-300 px-2 text-sm text-slate-700 hover:bg-slate-50 disabled:opacity-40"
              disabled={readOnly}
              type="button"
              onClick={() => void onClear()}
            >
              Clear
            </button>
          ) : null}
        </div>
        <div className="min-w-0">{control}</div>
      </div>
    );
  }

  return (
    <div className="grid gap-3 bg-white p-3 sm:grid-cols-[minmax(8rem,12rem)_minmax(0,1fr)_auto] sm:items-start">
      <ParamNameBlock paramName={paramName} paramSpec={paramSpec} />
      <div className="min-w-0">{control}</div>
      {hasParam ? (
        <button
          className="h-9 rounded border border-slate-300 px-2 text-sm text-slate-700 hover:bg-slate-50 disabled:opacity-40"
          disabled={readOnly}
          type="button"
          onClick={() => void onClear()}
        >
          Clear
        </button>
      ) : null}
    </div>
  );
}

function ParamNameBlock({
  paramName,
  paramSpec,
}: {
  paramName: string;
  paramSpec: StepSpecDto["params"][string] | null;
}) {
  return (
    <div className="min-w-0">
      <div className="break-words text-sm font-medium text-slate-950">{paramName}</div>
      {paramSpec ? (
        <div className="mt-1 text-xs text-slate-500">
          {paramSpec.required ? "Required" : "Optional"} · {paramSpec.acceptedSources.join(" / ")}
        </div>
      ) : null}
    </div>
  );
}

/**
 * Explains why a structured App-owned choice list is narrowed or empty.
 *
 * A catalog that does not define the selected App Definition cannot list its
 * choices, and a fully retained list means the App Definition defines no
 * choices of that kind. The message never replaces a retained value, which
 * stays rendered around it.
 */
function appOwnedChoiceMessage(
  appRef: string | null,
  selectedApp: AppCatalogAppDto | null,
  options: Array<{ retained: boolean }>,
  messages: { missingApp: string; none: string },
): string | null {
  if (selectedApp !== null) {
    return options.some((option) => !option.retained) ? null : messages.none;
  }
  if (appRef) {
    return `App Definition ${appRef} is not in the current catalog, so its choices cannot be listed.`;
  }
  return messages.missingApp;
}

/**
 * Structured select for one literal value an App Definition owns.
 *
 * `grant_permissions` selects a permission set this way: only sets the
 * selected App Definition defines are offered, and a retained current value
 * stays visible when the catalog does not define it.
 */
function AppChoiceSelect({
  disabled,
  message,
  options,
  value,
  onUpdate,
}: {
  disabled: boolean;
  message: string | null;
  options: AppChoiceOption[];
  value: string;
  onUpdate: (nextValue: string) => Promise<boolean>;
}) {
  if (options.length === 0) {
    return <p className="text-sm text-slate-500">{message ?? "No choices are available."}</p>;
  }
  return (
    <div className="grid gap-1">
      <select
        className="w-full border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
        disabled={disabled}
        value={value}
        onChange={(event) => {
          if (event.target.value !== value) {
            void onUpdate(event.target.value);
          }
        }}
      >
        {value === "" ? <option value="">Select a value</option> : null}
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      {message !== null ? <p className="text-sm text-slate-500">{message}</p> : null}
    </div>
  );
}

/**
 * Structured select for a `copy_files` destination owned by an App Target.
 *
 * Only targets of the selected App Definition are offered. Targets the
 * planner rules already reject for the current copy source or policy stay
 * listed with their reason and cannot be newly selected. A target whose
 * compatibility cannot be resolved yet is listed as unknown and stays
 * selectable, and a retained target the catalog does not define stays
 * selectable too, so authored values are never lost.
 */
function AppTargetSelect({
  appRef,
  disabled,
  options,
  selectedApp,
  value,
  onUpdate,
}: {
  appRef: string | null;
  disabled: boolean;
  options: AppTargetOption[];
  selectedApp: AppCatalogAppDto | null;
  value: string;
  onUpdate: (nextValue: { app_target: string }) => Promise<boolean>;
}) {
  const message = appOwnedChoiceMessage(appRef, selectedApp, options, {
    missingApp: "Select an App Definition to choose one of its App Targets.",
    none: "This App Definition defines no App Targets.",
  });
  const current = options.find((option) => option.id === value) ?? null;
  if (options.length === 0) {
    return <p className="text-sm text-slate-500">{message}</p>;
  }
  return (
    <div className="grid gap-1">
      <select
        className="w-full border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
        disabled={disabled}
        value={value}
        onChange={(event) => {
          if (event.target.value !== value) {
            void onUpdate({ app_target: event.target.value });
          }
        }}
      >
      {value === "" ? <option value="">Select an App Target</option> : null}
        {options.map((option) => (
          <option
            disabled={option.compatibility === "incompatible" && !option.retained}
            key={option.id}
            value={option.id}
          >
            {option.reason === null ? option.label : `${option.label} — ${option.reason}`}
          </option>
        ))}
      </select>
      {current !== null && current.compatibility === "incompatible" && current.reason !== null ? (
        <p className="text-xs text-amber-800">
          This App Target is not compatible with the current copy source: {current.reason}
        </p>
      ) : null}
      {current !== null && current.compatibility === "unknown" && current.reason !== null ? (
        <p className="text-xs text-slate-500">{current.reason}</p>
      ) : null}
      {message !== null ? <p className="text-sm text-slate-500">{message}</p> : null}
    </div>
  );
}

interface StringIdOption {
  id: string;
  secondary: string;
}

function StringIdListParamInput({
  value,
  options,
  addLabel,
  emptyValueMessage,
  emptyCatalogMessage,
  disabled = false,
  onUpdate,
}: {
  value: unknown;
  options: StringIdOption[];
  addLabel: string;
  emptyValueMessage: string;
  emptyCatalogMessage: string;
  disabled?: boolean;
  onUpdate: (nextValue: string[]) => Promise<boolean>;
}) {
  const ids = stringListValue(value);
  const optionById = useMemo(() => new Map(options.map((option) => [option.id, option])), [options]);
  const availableOptions = options.filter((option) => !ids.includes(option.id));
  const [selectedId, setSelectedId] = useState(availableOptions[0]?.id ?? "");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!availableOptions.some((option) => option.id === selectedId)) {
      setSelectedId(availableOptions[0]?.id ?? "");
    }
  }, [availableOptions, selectedId]);

  async function submit(nextValue: string[]) {
    if (paramValuesEqual(ids, nextValue)) {
      return;
    }
    const ok = await onUpdate(nextValue);
    if (ok) {
      setError(null);
    }
  }

  async function addSelected() {
    const result = addUniqueStringListValue(ids, selectedId);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    await submit(result.value);
  }

  return (
    <div className="grid gap-2">
      {ids.length === 0 ? <p className="text-sm text-slate-500">{emptyValueMessage}</p> : null}
      <div className="grid gap-1">
        {ids.map((id, index) => {
          const option = optionById.get(id);
          return (
            <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2" key={`${id}:${index}`}>
              <div className="min-w-0">
                <div className="break-words text-sm font-medium text-slate-900">{id}</div>
                <div className="break-words text-xs text-slate-500">{option?.secondary ?? "Missing"}</div>
              </div>
              <div className="flex flex-wrap gap-1">
                <button
                  className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
                  disabled={disabled || index === 0}
                  type="button"
                  onClick={() => void submit(moveStringListValue(ids, index, index - 1))}
                >
                  Up
                </button>
                <button
                  className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
                  disabled={disabled || index === ids.length - 1}
                  type="button"
                  onClick={() => void submit(moveStringListValue(ids, index, index + 1))}
                >
                  Down
                </button>
                <button
                  className="rounded border border-red-300 px-2 py-1 text-xs text-red-700 disabled:opacity-40"
                  disabled={disabled}
                  type="button"
                  onClick={() => void submit(removeStringListValue(ids, index))}
                >
                  Remove
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {options.length === 0 ? <p className="text-sm text-slate-500">{emptyCatalogMessage}</p> : null}
      {options.length > 0 ? (
        <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
          <select
            className="min-w-0 w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            disabled={disabled || availableOptions.length === 0}
            value={selectedId}
            onChange={(event) => {
              setSelectedId(event.target.value);
              setError(null);
            }}
          >
            {availableOptions.length === 0 ? (
              <option value="">No available ids</option>
            ) : (
              availableOptions.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.id} ({option.secondary})
                </option>
              ))
            )}
          </select>
          <button
            className="w-fit rounded border border-slate-900 bg-slate-900 px-3 py-2 text-sm text-white disabled:opacity-40 sm:w-auto"
            disabled={disabled || !selectedId}
            type="button"
            onClick={() => void addSelected()}
          >
            {addLabel}
          </button>
        </div>
      ) : null}
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function ObjectListParamInput({
  value,
  shape,
  disabled = false,
  onUpdate,
}: {
  value: unknown;
  shape: StepParamShapeDto;
  disabled?: boolean;
  onUpdate: (nextValue: Record<string, unknown>[]) => Promise<boolean>;
}) {
  const rows = objectListValue(value);
  const fieldEntries = useMemo(() => Object.entries(shape.fields), [shape.fields]);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState<Record<string, unknown>>(() => emptyObjectDraft(fieldEntries));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!adding) {
      setDraft(emptyObjectDraft(fieldEntries));
      setError(null);
    }
  }, [adding, fieldEntries]);

  async function submit(nextRows: Record<string, unknown>[]) {
    if (paramValuesEqual(rows, nextRows)) {
      return true;
    }
    const ok = await onUpdate(nextRows);
    if (ok) {
      setError(null);
    }
    return ok;
  }

  async function addDraftRow() {
    const validation = validateStructuredDraft(draft, fieldEntries);
    if (validation !== null) {
      setError(validation);
      return;
    }
    const result = addObjectListRow(rows, draft);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    const ok = await submit(result.value);
    if (ok) {
      setAdding(false);
      setDraft(emptyObjectDraft(fieldEntries));
    }
  }

  return (
    <div className="grid gap-2">
      {rows.length === 0 ? <p className="text-sm text-slate-500">No rows</p> : null}
      {rows.map((row, index) => (
        <ObjectListRowEditor
          disabled={disabled}
          fieldEntries={fieldEntries}
          index={index}
          key={index}
          row={row}
          rowCount={rows.length}
          onUpdateField={(fieldName, nextFieldValue) => {
            const result = buildObjectListRowFieldUpdate(rows, index, fieldName, nextFieldValue);
            if (result === null) {
              return Promise.resolve(true);
            }
            if (!result.ok) {
              return Promise.resolve(false);
            }
            return submit(result.value);
          }}
          onMoveDown={() => submit(moveObjectListRow(rows, index, index + 1))}
          onMoveUp={() => submit(moveObjectListRow(rows, index, index - 1))}
          onRemove={() => submit(removeObjectListRow(rows, index))}
        />
      ))}
      {adding ? (
        <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2">
          <StructuredDraftFields
            disabled={disabled}
            draft={draft}
            fieldEntries={fieldEntries}
            onDraftChange={(nextDraft) => {
              setDraft(nextDraft);
              setError(null);
            }}
            onEnter={() => void addDraftRow()}
          />
          <div className="flex flex-wrap items-center gap-2">
            <button
              className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
              disabled={disabled}
              type="button"
              onClick={() => void addDraftRow()}
            >
              Add row
            </button>
            <button
              className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
              disabled={disabled}
              type="button"
              onClick={() => setAdding(false)}
            >
              Cancel
            </button>
            {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
          </div>
        </div>
      ) : (
        <button
          className="w-fit rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
          disabled={disabled}
          type="button"
          onClick={() => setAdding(true)}
        >
          Add row
        </button>
      )}
    </div>
  );
}

function ObjectListRowEditor({
  row,
  index,
  rowCount,
  fieldEntries,
  disabled,
  onUpdateField,
  onMoveUp,
  onMoveDown,
  onRemove,
}: {
  row: Record<string, unknown>;
  index: number;
  rowCount: number;
  fieldEntries: Array<[string, StepParamShapeFieldDto]>;
  disabled: boolean;
  onUpdateField: (fieldName: string, nextFieldValue: unknown) => Promise<boolean>;
  onMoveUp: () => Promise<boolean>;
  onMoveDown: () => Promise<boolean>;
  onRemove: () => Promise<boolean>;
}) {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setError(null);
  }, [fieldEntries, row]);

  return (
    <div className="grid gap-2 rounded border border-slate-200 bg-slate-50 p-2">
      <div className="grid gap-2">
        {fieldEntries.map(([fieldName, field]) => (
          <StructuredRowField
            disabled={disabled}
            field={field}
            fieldName={fieldName}
            key={fieldName}
            row={row}
            onError={setError}
            onUpdateField={onUpdateField}
          />
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="rounded border border-slate-300 px-2 py-1.5 text-sm text-slate-700 disabled:opacity-40"
          disabled={disabled || index === 0}
          type="button"
          onClick={() => void onMoveUp()}
        >
          Up
        </button>
        <button
          className="rounded border border-slate-300 px-2 py-1.5 text-sm text-slate-700 disabled:opacity-40"
          disabled={disabled || index === rowCount - 1}
          type="button"
          onClick={() => void onMoveDown()}
        >
          Down
        </button>
        <button
          className="rounded border border-red-300 px-2 py-1.5 text-sm text-red-700 disabled:opacity-40"
          disabled={disabled}
          type="button"
          onClick={() => void onRemove()}
        >
          Remove
        </button>
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </div>
  );
}

function StructuredRowField({
  row,
  fieldName,
  field,
  disabled,
  onUpdateField,
  onError,
}: {
  row: Record<string, unknown>;
  fieldName: string;
  field: StepParamShapeFieldDto;
  disabled: boolean;
  onUpdateField: (fieldName: string, nextFieldValue: unknown) => Promise<boolean>;
  onError: (message: string | null) => void;
}) {
  const baseline = valueForObjectListRowFieldDraft(row, fieldName);
  const [draft, setDraft] = useState(baseline);
  const submittingRef = useRef(false);

  useEffect(() => {
    setDraft(baseline);
    onError(null);
  }, [baseline, onError]);

  async function commit() {
    if (submittingRef.current) {
      return;
    }
    if (field.required && !draft.trim()) {
      onError(`${fieldName} is required.`);
      return;
    }
    if (draft === baseline) {
      onError(null);
      return;
    }
    submittingRef.current = true;
    try {
      const ok = await onUpdateField(fieldName, draft);
      if (ok) {
        onError(null);
      }
    } finally {
      submittingRef.current = false;
    }
  }

  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{fieldName}</span>
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        disabled={disabled}
        value={draft}
        onBlur={() => void commit()}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          onError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void commit();
          }
          if (event.key === "Escape") {
            setDraft(baseline);
            onError(null);
          }
        }}
      />
    </label>
  );
}

function StructuredDraftFields({
  draft,
  fieldEntries,
  disabled,
  onDraftChange,
  onEnter,
}: {
  draft: Record<string, unknown>;
  fieldEntries: Array<[string, StepParamShapeFieldDto]>;
  disabled: boolean;
  onDraftChange: (draft: Record<string, unknown>) => void;
  onEnter: () => void;
}) {
  return (
    <div className="grid gap-2">
      {fieldEntries.map(([fieldName, field]) => (
        <label className="grid gap-1" key={fieldName}>
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{fieldName}</span>
          {field.kind === "boolean" ? (
            <input
              checked={Boolean(draft[fieldName])}
              className="h-4 w-4 rounded border-slate-300"
              disabled={disabled}
              type="checkbox"
              onChange={(event) => onDraftChange({ ...draft, [fieldName]: event.target.checked })}
            />
          ) : field.enumValues.length > 0 ? (
            <select
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
              disabled={disabled}
              value={String(draft[fieldName] ?? "")}
              onChange={(event) => onDraftChange({ ...draft, [fieldName]: event.target.value })}
            >
              {field.enumValues.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          ) : (
            <input
              {...textInputGuardProps}
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
              disabled={disabled}
              value={String(draft[fieldName] ?? "")}
              onChange={(event) => onDraftChange({ ...draft, [fieldName]: normalizeEditableText(event.target.value) })}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  onEnter();
                }
              }}
            />
          )}
        </label>
      ))}
    </div>
  );
}

function ObjectParamInput({
  value,
  shape,
  disabled = false,
  onUpdate,
}: {
  value: unknown;
  shape: StepParamShapeDto;
  disabled?: boolean;
  onUpdate: (nextValue: Record<string, unknown>) => Promise<boolean>;
}) {
  const current = objectValue(value);
  const fields = useMemo(() => Object.entries(shape.fields), [shape.fields]);

  async function commitField(fieldName: string, nextFieldValue: unknown) {
    const result = updateObjectField(current, fieldName, nextFieldValue);
    if (!result.ok || paramValuesEqual(current, result.value)) {
      return;
    }
    await onUpdate(result.value);
  }

  return (
    <div className="grid gap-3">
      {fields.map(([fieldName, field]) => (
        <ObjectFieldControl
          current={current}
          disabled={disabled}
          field={field}
          fieldName={fieldName}
          key={fieldName}
          onCommit={(nextValue) => commitField(fieldName, nextValue)}
        />
      ))}
    </div>
  );
}

function ObjectFieldControl({
  current,
  fieldName,
  field,
  disabled,
  onCommit,
}: {
  current: Record<string, unknown>;
  fieldName: string;
  field: StepParamShapeFieldDto;
  disabled: boolean;
  onCommit: (nextValue: unknown) => Promise<void>;
}) {
  const display = displayValueForObjectField(current, fieldName, field);
  if (field.kind === "boolean") {
    const checked = Boolean(display.value);
    return (
      <label className="grid gap-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{fieldName}</span>
        <span className="flex items-center gap-2 text-sm text-slate-700">
          <input
            checked={checked}
            className="h-4 w-4 rounded border-slate-300"
            disabled={disabled}
            type="checkbox"
            onChange={(event) => {
              if (event.target.checked !== checked) {
                void onCommit(event.target.checked);
              }
            }}
          />
          {display.defaulted ? "Default" : checked ? "True" : "False"}
        </span>
      </label>
    );
  }
  if (field.enumValues.length > 0) {
    const currentValue = typeof display.value === "string" ? display.value : String(display.value ?? "");
    const options = field.enumValues.includes(currentValue) || currentValue === "" ? field.enumValues : [currentValue, ...field.enumValues];
    return (
      <label className="grid gap-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{fieldName}</span>
        <select
          className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
          disabled={disabled}
          value={currentValue}
          onChange={(event) => {
            if (event.target.value !== currentValue) {
              void onCommit(event.target.value);
            }
          }}
        >
          {options.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
        {display.defaulted ? <span className="text-xs text-slate-500">Default</span> : null}
      </label>
    );
  }
  return <ObjectStringField current={current} disabled={disabled} field={field} fieldName={fieldName} onCommit={onCommit} />;
}

function ObjectStringField({
  current,
  fieldName,
  field,
  disabled,
  onCommit,
}: {
  current: Record<string, unknown>;
  fieldName: string;
  field: StepParamShapeFieldDto;
  disabled: boolean;
  onCommit: (nextValue: string) => Promise<void>;
}) {
  const display = displayValueForObjectField(current, fieldName, field);
  const value = typeof display.value === "string" ? display.value : "";
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(value);
    setError(null);
  }, [value]);

  async function commit() {
    if (field.required && !draft.trim()) {
      setError(`${fieldName} is required.`);
      return;
    }
    if (draft !== value) {
      await onCommit(draft);
    }
  }

  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{fieldName}</span>
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        disabled={disabled}
        value={draft}
        onBlur={() => void commit()}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void commit();
          }
          if (event.key === "Escape") {
            setDraft(value);
            setError(null);
          }
        }}
      />
      {display.defaulted ? <span className="text-xs text-slate-500">Default</span> : null}
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </label>
  );
}

function artifactOptions(artifacts: Record<string, ArtifactDto>): StringIdOption[] {
  return Object.entries(artifacts).map(([id, artifact]) => ({
    id,
    secondary: artifact.cache ? `${artifact.type} · ${artifact.cache}` : artifact.type,
  }));
}

function artifactGroupOptions(groups: Record<string, string[]>): StringIdOption[] {
  return Object.entries(groups).map(([id, members]) => ({
    id,
    secondary: `${members.length} member${members.length === 1 ? "" : "s"}`,
  }));
}

function emptyObjectDraft(fieldEntries: Array<[string, StepParamShapeFieldDto]>): Record<string, unknown> {
  return Object.fromEntries(
    fieldEntries.map(([fieldName, field]) => [
      fieldName,
      field.kind === "boolean" ? Boolean(field.default) : typeof field.default === "string" ? field.default : "",
    ]),
  );
}

function validateStructuredDraft(
  draft: Record<string, unknown>,
  fieldEntries: Array<[string, StepParamShapeFieldDto]>,
): string | null {
  for (const [fieldName, field] of fieldEntries) {
    const value = draft[fieldName];
    if (field.required && field.kind === "string" && (typeof value !== "string" || !value.trim())) {
      return `${fieldName} is required.`;
    }
  }
  return null;
}

function StringParamInput({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: string; onUpdate: (nextValue: string) => Promise<boolean> }) {
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(value);
    setError(null);
  }, [value]);

  async function commit() {
    if (draft === value || submitting) {
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(draft);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-1">
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        readOnly={readOnly}
        value={draft}
        onBlur={() => void commit()}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void commit();
          }
          if (event.key === "Escape") {
            setDraft(value);
            setError(null);
          }
        }}
      />
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function NumberParamInput({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: number; onUpdate: (nextValue: number) => Promise<boolean> }) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(String(value));
    setError(null);
  }, [value]);

  async function commit() {
    if (submitting) {
      return;
    }
    const parsed = parseNumberParamDraft(draft, value);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    if (paramValuesEqual(value, parsed.value)) {
      setError(null);
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(parsed.value as number);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-1">
      <input
        {...textInputGuardProps}
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        inputMode="decimal"
        readOnly={readOnly}
        value={draft}
        onBlur={() => void commit()}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void commit();
          }
          if (event.key === "Escape") {
            setDraft(String(value));
            setError(null);
          }
        }}
      />
      {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
    </div>
  );
}

function EnumParamInput({
  value,
  enumValues,
  disabled = false,
  onUpdate,
}: {
  value: unknown;
  enumValues: string[];
  disabled?: boolean;
  onUpdate: (nextValue: string) => Promise<boolean>;
}) {
  const stringValue = value === undefined ? "" : String(value);
  const options =
    stringValue === "" || enumValues.includes(stringValue) ? enumValues : [stringValue, ...enumValues];

  return (
    <select
      className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
      disabled={disabled}
      value={stringValue}
      onChange={(event) => {
        if (event.target.value !== stringValue) {
          void onUpdate(event.target.value);
        }
      }}
    >
      {stringValue === "" ? <option value="">Select a value</option> : null}
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

function BooleanParamInput({ disabled = false, value, onUpdate }: { disabled?: boolean; value: boolean; onUpdate: (nextValue: boolean) => Promise<boolean> }) {
  return (
    <label className="flex h-9 items-center gap-2 text-sm text-slate-700">
      <input
        checked={value}
        className="h-4 w-4 rounded border-slate-300"
        disabled={disabled}
        type="checkbox"
        onChange={(event) => {
          if (event.target.checked !== value) {
            void onUpdate(event.target.checked);
          }
        }}
      />
      Enabled
    </label>
  );
}

function RefPicker({
  allowedArtifactRefs = null,
  currentRef,
  allowedSources,
  allowedValueTypes,
  dependencyWarning,
  disabled = false,
  refIndex,
  onAddDependency,
  onUpdate,
}: {
  allowedArtifactRefs?: readonly string[] | null;
  currentRef: string;
  allowedSources: readonly string[];
  allowedValueTypes: readonly string[];
  dependencyWarning: StepRefDependencyWarning | null;
  disabled?: boolean;
  refIndex: RefIndexDto;
  onAddDependency: () => Promise<boolean>;
  onUpdate: (nextRef: string) => Promise<boolean>;
}) {
  const [showAll, setShowAll] = useState(false);
  const options = useMemo(
    () =>
      buildRefPickerOptions(refIndex, {
        allowedArtifactRefs,
        allowedSources,
        allowedValueTypes,
        currentRef,
        showAll,
      }),
    [allowedArtifactRefs, allowedSources, allowedValueTypes, currentRef, refIndex, showAll],
  );

  return (
    <div className="grid gap-2">
      <select
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
        disabled={disabled}
        value={currentRef}
        onChange={(event) => {
          if (event.target.value !== currentRef) {
            void onUpdate(event.target.value);
          }
        }}
      >
        {currentRef === "" ? <option value="">Select a ref</option> : null}
        {options.map((option) => (
          <option key={option.ref} value={option.ref}>
            {refOptionLabel(option)}
          </option>
        ))}
      </select>
      {allowedSources.length > 0 || allowedValueTypes.length > 0 ? (
        <button
          className="w-fit rounded border border-slate-300 px-2 py-1 text-xs text-slate-700 hover:bg-slate-50"
          disabled={disabled}
          type="button"
          onClick={() => setShowAll((current) => !current)}
        >
          {showAll ? "Filter refs" : "Show all refs"}
        </button>
      ) : null}
      {dependencyWarning ? (
        <div className="grid gap-2 rounded border border-amber-200 bg-amber-50 px-3 py-2">
          <p className="text-sm text-amber-900">{dependencyWarning.message}</p>
          <button
            className="w-fit rounded border border-amber-700 bg-white px-2 py-1 text-xs font-medium text-amber-900 hover:bg-amber-100 disabled:opacity-40"
            disabled={disabled}
            type="button"
            onClick={() => void onAddDependency()}
          >
            Add dependency
          </button>
        </div>
      ) : null}
    </div>
  );
}

async function addRefDependency(
  step: StepDto,
  dependencyWarning: StepRefDependencyWarning | null,
  onUpdateDependencies: (dependencies: string[]) => Promise<boolean>,
): Promise<boolean> {
  if (dependencyWarning === null) {
    return false;
  }
  const result = buildRefDependencyAction(step.dependencies, step.id, dependencyWarning.producerStepId);
  if (!result.ok) {
    return false;
  }
  return onUpdateDependencies(result.dependencies);
}

function JsonValueEditor({ readOnly = false, value, onUpdate }: { readOnly?: boolean; value: unknown; onUpdate: (nextValue: unknown) => Promise<boolean> }) {
  const formattedValue = useMemo(() => JSON.stringify(value, null, 2), [value]);
  const [draft, setDraft] = useState(formattedValue);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    setDraft(formattedValue);
    setError(null);
  }, [formattedValue]);

  async function apply() {
    if (submitting) {
      return;
    }
    const parsed = parseJsonParamDraft(draft);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    if (paramValuesEqual(value, parsed.value)) {
      setError(null);
      return;
    }
    setSubmitting(true);
    const ok = await onUpdate(parsed.value);
    setSubmitting(false);
    if (ok) {
      setError(null);
    }
  }

  return (
    <div className="grid gap-2">
      <textarea
        {...textInputGuardProps}
        className="min-h-24 w-full resize-y rounded border border-slate-300 bg-white px-3 py-2 font-mono text-xs text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
        readOnly={readOnly}
        value={draft}
        onChange={(event) => {
          setDraft(normalizeEditableText(event.target.value));
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setDraft(formattedValue);
            setError(null);
          }
        }}
      />
      <div className="flex items-center gap-2">
        <button
          className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
          disabled={readOnly || submitting}
          type="button"
          onClick={() => void apply()}
        >
          Apply
        </button>
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </div>
  );
}

function refOptionLabel(option: RefPickerOption): string {
  const suffixes: string[] = [];
  if (option.current) {
    suffixes.push("current");
  }
  if (option.missing) {
    suffixes.push("missing");
  }
  if (option.incompatible) {
    suffixes.push("incompatible");
  }
  const metadata = option.valueType ? `${option.label} · ${option.valueType}` : option.label;
  return suffixes.length > 0 ? `${metadata} (${suffixes.join(", ")})` : metadata;
}

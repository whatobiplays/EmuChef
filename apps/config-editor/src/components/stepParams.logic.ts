import type { EditorCommand } from "../api/commands.js";
import type { RefCandidateDto, RefIndexDto, StepParamShapeFieldDto, StepSpecDto } from "../api/types.js";

export interface AuthoredRefValue {
  ref: string;
}

export interface RefPickerOption {
  ref: string;
  label: string;
  valueType: string | null;
  sourceKind: string;
  sourceId: string;
  current: boolean;
  missing: boolean;
  incompatible: boolean;
}

export type ParseResult = { ok: true; value: unknown } | { ok: false; error: string };
export type StructuredParamEditorKind = "artifact-id-list" | "artifact-group-id-list" | "object-list" | "object";
export type StructuredValueResult<T> = { ok: true; value: T } | { ok: false; error: string };

export function isAuthoredRefValue(value: unknown): value is AuthoredRefValue {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const keys = Object.keys(value);
  return keys.length === 1 && keys[0] === "ref" && typeof (value as { ref?: unknown }).ref === "string";
}

export function orderedParamNames(params: Record<string, unknown>, stepSpec: StepSpecDto | null | undefined): string[] {
  const presentNames = new Set(Object.keys(params));
  const ordered: string[] = [];
  for (const name of stepSpec?.paramOrder ?? []) {
    if (presentNames.has(name) || structuredParamEditorKind(stepSpec, name, undefined) !== null) {
      ordered.push(name);
    }
  }
  for (const name of Object.keys(params)) {
    if (!ordered.includes(name)) {
      ordered.push(name);
    }
  }
  return ordered;
}

export function structuredParamEditorKind(
  stepSpec: StepSpecDto | null | undefined,
  paramName: string,
  value: unknown,
): StructuredParamEditorKind | null {
  const shape = stepSpec?.params[paramName]?.shape;
  if (!shape) {
    return null;
  }
  if (shape.kind === "list" && shape.itemKind === "string" && shape.target === "artifact") {
    return value === undefined || isStringList(value) ? "artifact-id-list" : null;
  }
  if (shape.kind === "list" && shape.itemKind === "string" && shape.target === "artifact_group") {
    return value === undefined || isStringList(value) ? "artifact-group-id-list" : null;
  }
  if (shape.kind === "list" && shape.itemKind === "object" && supportedStructuredFields(shape.fields)) {
    return value === undefined || isObjectList(value) ? "object-list" : null;
  }
  if (shape.kind === "object" && supportedStructuredFields(shape.fields)) {
    return value === undefined || isJsonObject(value) ? "object" : null;
  }
  return null;
}

export function stringListValue(value: unknown): string[] {
  return isStringList(value) ? [...value] : [];
}

export function objectListValue(value: unknown): Record<string, unknown>[] {
  return isObjectList(value) ? value.map((item) => ({ ...item })) : [];
}

export function objectValue(value: unknown): Record<string, unknown> {
  return isJsonObject(value) ? { ...value } : {};
}

export function addUniqueStringListValue(currentValue: unknown, nextId: string): StructuredValueResult<string[]> {
  if (!isStringList(currentValue)) {
    return { ok: false, error: "Existing value is not a list of strings." };
  }
  if (!nextId.trim()) {
    return { ok: false, error: "Choose an id to add." };
  }
  if (currentValue.includes(nextId)) {
    return { ok: false, error: "This id is already selected." };
  }
  return { ok: true, value: [...currentValue, nextId] };
}

export function removeStringListValue(currentValue: unknown, index: number): string[] {
  if (!isStringList(currentValue) || index < 0 || index >= currentValue.length) {
    return isStringList(currentValue) ? [...currentValue] : [];
  }
  return currentValue.filter((_, itemIndex) => itemIndex !== index);
}

export function moveStringListValue(currentValue: unknown, index: number, toIndex: number): string[] {
  if (!isStringList(currentValue) || index < 0 || index >= currentValue.length || toIndex < 0 || toIndex >= currentValue.length) {
    return isStringList(currentValue) ? [...currentValue] : [];
  }
  const next = [...currentValue];
  const [item] = next.splice(index, 1);
  next.splice(toIndex, 0, item);
  return next;
}

export function addObjectListRow(
  currentValue: unknown,
  row: Record<string, unknown>,
): StructuredValueResult<Record<string, unknown>[]> {
  if (!isObjectList(currentValue)) {
    return { ok: false, error: "Existing value is not a list of objects." };
  }
  return { ok: true, value: [...currentValue.map((item) => ({ ...item })), { ...row }] };
}

export function removeObjectListRow(currentValue: unknown, index: number): Record<string, unknown>[] {
  if (!isObjectList(currentValue) || index < 0 || index >= currentValue.length) {
    return isObjectList(currentValue) ? currentValue.map((item) => ({ ...item })) : [];
  }
  return currentValue.filter((_, itemIndex) => itemIndex !== index).map((item) => ({ ...item }));
}

export function moveObjectListRow(currentValue: unknown, index: number, toIndex: number): Record<string, unknown>[] {
  if (!isObjectList(currentValue) || index < 0 || index >= currentValue.length || toIndex < 0 || toIndex >= currentValue.length) {
    return isObjectList(currentValue) ? currentValue.map((item) => ({ ...item })) : [];
  }
  const next = currentValue.map((item) => ({ ...item }));
  const [item] = next.splice(index, 1);
  next.splice(toIndex, 0, item);
  return next;
}

export function updateObjectListRowField(
  currentValue: unknown,
  index: number,
  fieldName: string,
  nextFieldValue: unknown,
): StructuredValueResult<Record<string, unknown>[]> {
  if (!isObjectList(currentValue)) {
    return { ok: false, error: "Existing value is not a list of objects." };
  }
  if (index < 0 || index >= currentValue.length) {
    return { ok: false, error: "Row does not exist." };
  }
  const next = currentValue.map((item) => ({ ...item }));
  next[index] = { ...next[index], [fieldName]: nextFieldValue };
  return { ok: true, value: next };
}

export function buildObjectListRowFieldUpdate(
  currentValue: unknown,
  index: number,
  fieldName: string,
  nextFieldValue: unknown,
): StructuredValueResult<Record<string, unknown>[]> | null {
  if (!isObjectList(currentValue)) {
    return { ok: false, error: "Existing value is not a list of objects." };
  }
  if (index < 0 || index >= currentValue.length) {
    return { ok: false, error: "Row does not exist." };
  }
  if (paramValuesEqual(currentValue[index][fieldName], nextFieldValue)) {
    return null;
  }
  return updateObjectListRowField(currentValue, index, fieldName, nextFieldValue);
}

export function valueForObjectListRowFieldDraft(row: Record<string, unknown>, fieldName: string): string {
  const value = row[fieldName];
  return typeof value === "string" ? value : "";
}

export function updateObjectField(
  currentValue: unknown,
  fieldName: string,
  nextFieldValue: unknown,
): StructuredValueResult<Record<string, unknown>> {
  if (!isJsonObject(currentValue)) {
    return { ok: false, error: "Existing value is not an object." };
  }
  return { ok: true, value: { ...currentValue, [fieldName]: nextFieldValue } };
}

export function displayValueForObjectField(
  value: Record<string, unknown>,
  fieldName: string,
  field: StepParamShapeFieldDto | undefined,
): { value: unknown; defaulted: boolean } {
  if (!field) {
    return { value: undefined, defaulted: false };
  }
  if (Object.prototype.hasOwnProperty.call(value, fieldName)) {
    return { value: value[fieldName], defaulted: false };
  }
  if ("default" in field) {
    return { value: field.default, defaulted: true };
  }
  return { value: undefined, defaulted: false };
}

export function buildUpdateStepParamsCommand(
  stepId: string,
  currentParams: Record<string, unknown>,
  paramName: string,
  nextValue: unknown,
): Extract<EditorCommand, { type: "UpdateStepParams" }> | null {
  if (paramValuesEqual(currentParams[paramName], nextValue)) {
    return null;
  }
  return {
    type: "UpdateStepParams",
    stepId,
    params: {
      ...currentParams,
      [paramName]: nextValue,
    },
  };
}

export function buildClearStepParamCommand(
  stepId: string,
  currentParams: Record<string, unknown>,
  paramName: string,
): Extract<EditorCommand, { type: "UpdateStepParams" }> | null {
  if (!Object.prototype.hasOwnProperty.call(currentParams, paramName)) {
    return null;
  }
  const nextParams = { ...currentParams };
  delete nextParams[paramName];
  return {
    type: "UpdateStepParams",
    stepId,
    params: nextParams,
  };
}

export function paramValuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function parseNumberParamDraft(draft: string, currentValue: unknown): ParseResult {
  const trimmed = draft.trim();
  if (trimmed === "") {
    return { ok: false, error: "Enter a valid number." };
  }
  const value = Number(trimmed);
  if (!Number.isFinite(value)) {
    return { ok: false, error: "Enter a valid number." };
  }
  if (shouldPreserveInteger(currentValue) && !Number.isInteger(value)) {
    return { ok: false, error: "Enter a whole number." };
  }
  return { ok: true, value };
}

export function parseJsonParamDraft(draft: string): ParseResult {
  try {
    return { ok: true, value: JSON.parse(draft) };
  } catch {
    return { ok: false, error: "Enter valid JSON." };
  }
}

export function buildRefPickerOptions(
  refIndex: RefIndexDto,
  {
    allowedValueTypes,
    currentRef,
    showAll,
  }: {
    allowedValueTypes: readonly string[];
    currentRef: string | null;
    showAll: boolean;
  },
): RefPickerOption[] {
  const allowed = new Set(allowedValueTypes);
  const hasFilter = allowed.size > 0 && !showAll;
  const candidates = candidateOptions(refIndex.candidates);
  const candidateRefs = new Set(candidates.map((option) => option.ref));
  const fallbackOptions = refIndex.allRefs
    .filter((ref) => !candidateRefs.has(ref))
    .map((ref) => rawRefOption(ref));
  const allOptions = [...candidates, ...fallbackOptions];
  const filteredOptions = hasFilter
    ? allOptions.filter((option) => option.valueType !== null && allowed.has(option.valueType))
    : allOptions;
  const optionsByRef = new Map(filteredOptions.map((option) => [option.ref, option]));

  if (currentRef) {
    const currentFromAll = allOptions.find((option) => option.ref === currentRef) ?? rawRefOption(currentRef, true);
    optionsByRef.set(currentRef, {
      ...currentFromAll,
      current: true,
      missing: !allOptions.some((option) => option.ref === currentRef),
      incompatible:
        hasFilter && currentFromAll.valueType !== null && !allowed.has(currentFromAll.valueType),
    });
  }

  return Array.from(optionsByRef.values()).sort((left, right) => {
    if (left.current !== right.current) {
      return left.current ? -1 : 1;
    }
    return 0;
  });
}

function shouldPreserveInteger(currentValue: unknown): boolean {
  return typeof currentValue === "number" && Number.isInteger(currentValue);
}

function isStringList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isObjectList(value: unknown): value is Record<string, unknown>[] {
  return Array.isArray(value) && value.every((item) => isJsonObject(item));
}

function supportedStructuredFields(fields: Record<string, StepParamShapeFieldDto>): boolean {
  return Object.values(fields).every((field) => field.kind === "string" || field.kind === "boolean");
}

function candidateOptions(candidates: readonly RefCandidateDto[]): RefPickerOption[] {
  return candidates.map((candidate) => ({
    ref: candidate.ref,
    label: candidate.label,
    valueType: candidate.valueType,
    sourceKind: candidate.sourceKind,
    sourceId: candidate.sourceId,
    current: false,
    missing: false,
    incompatible: false,
  }));
}

function rawRefOption(ref: string, missing = false): RefPickerOption {
  return {
    ref,
    label: ref,
    valueType: null,
    sourceKind: missing ? "unknown" : "raw",
    sourceId: ref,
    current: false,
    missing,
    incompatible: false,
  };
}

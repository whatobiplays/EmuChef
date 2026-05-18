import type { EditorCommand } from "../api/commands.js";

export type AdvancedInternalsField = "constraints" | "skipIf" | "verify";

export type AdvancedParseResult = { ok: true; value: unknown } | { ok: false; error: string };
export type AdvancedConversionResult<T> = { ok: true; value: T } | { ok: false; error: string };
export interface SupportedConstraintsDto {
  capabilities: string[];
  conflictsWith: string[];
}
export type ConstraintsClassification =
  | { kind: "structured"; value: SupportedConstraintsDto }
  | {
      kind: "raw";
      authoredJsonValue: unknown;
      reason:
        | "not_object"
        | "unknown_top_level_key"
        | "non_string_capabilities"
        | "non_string_conflictsWith"
        | "unsupported_field_shape"
        | "ambiguous_conflict_fields";
    };
export type SupportedVerifyType = "path_exists" | "file_exists" | "package_installed";
export type VerifyClassification =
  | { kind: "structured"; type: SupportedVerifyType; fieldName: "path" | "package_name"; fieldValue: string }
  | { kind: "json" };
export type VerifyUpdateResult = { ok: true; value: unknown[] } | { ok: false; error: string };

const VERIFY_TYPE_FIELDS: Record<SupportedVerifyType, "path" | "package_name"> = {
  path_exists: "path",
  file_exists: "path",
  package_installed: "package_name",
};

export function formatJsonDraft(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function revertJsonDraft(value: unknown): string {
  return formatJsonDraft(value);
}

export function editorValueForAdvancedField(field: AdvancedInternalsField, value: unknown): unknown {
  if (field !== "constraints") {
    return value;
  }
  return toAuthoredConstraintsJsonValue(value);
}

export function parseAdvancedJsonDraft(field: AdvancedInternalsField, draft: string): AdvancedParseResult {
  let value: unknown;
  try {
    value = JSON.parse(draft);
  } catch {
    return { ok: false, error: "Enter valid JSON." };
  }

  if (field === "constraints" && !isJsonObject(value)) {
    return { ok: false, error: "Constraints must be a JSON object." };
  }
  if (field === "constraints") {
    const converted = constraintsCommandValue(value);
    if (!converted.ok) {
      return converted;
    }
    return { ok: true, value: converted.value };
  }
  if (field === "skipIf" && !Array.isArray(value)) {
    return { ok: false, error: "skip_if must be a JSON array." };
  }
  if (field === "verify" && !Array.isArray(value)) {
    return { ok: false, error: "Verify must be a JSON array." };
  }
  return { ok: true, value };
}

export function buildAdvancedInternalsCommand(
  field: AdvancedInternalsField,
  stepId: string,
  nextValue: unknown,
  currentValue: unknown,
): Extract<EditorCommand, { type: "UpdateStepConstraints" | "UpdateStepSkipIf" | "UpdateStepVerify" }> | null {
  if (jsonValuesEqual(nextValue, currentValue)) {
    return null;
  }
  if (field === "constraints") {
    const converted = constraintsCommandValue(nextValue);
    if (!converted.ok) {
      return null;
    }
    return {
      type: "UpdateStepConstraints",
      stepId,
      constraints: converted.value,
    };
  }
  if (field === "skipIf") {
    return {
      type: "UpdateStepSkipIf",
      stepId,
      skipIf: nextValue as unknown[],
    };
  }
  return {
    type: "UpdateStepVerify",
    stepId,
    verify: nextValue as unknown[],
  };
}

export function jsonValuesEqual(left: unknown, right: unknown): boolean {
  return stableJsonString(left) === stableJsonString(right);
}

export function verifyFieldForType(type: string): "path" | "package_name" | null {
  return Object.prototype.hasOwnProperty.call(VERIFY_TYPE_FIELDS, type)
    ? VERIFY_TYPE_FIELDS[type as SupportedVerifyType]
    : null;
}

export function classifyVerifyEntry(entry: unknown): VerifyClassification {
  if (!isJsonObject(entry) || hasUnsupportedKeys(entry, ["type", "params"])) {
    return { kind: "json" };
  }
  if (typeof entry.type !== "string" || !isJsonObject(entry.params)) {
    return { kind: "json" };
  }
  const fieldName = verifyFieldForType(entry.type);
  if (fieldName === null || typeof entry.params[fieldName] !== "string") {
    return { kind: "json" };
  }
  return {
    kind: "structured",
    type: entry.type as SupportedVerifyType,
    fieldName,
    fieldValue: entry.params[fieldName],
  };
}

export function classifyConstraintsDto(value: unknown): ConstraintsClassification {
  if (!isJsonObject(value)) {
    return { kind: "raw", authoredJsonValue: value, reason: "not_object" };
  }
  if ("conflicts_with" in value && "conflictsWith" in value) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "ambiguous_conflict_fields",
    };
  }
  if (hasUnsupportedKeys(value, ["capabilities", "conflictsWith"])) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "unknown_top_level_key",
    };
  }
  if ("capabilities" in value && !Array.isArray(value.capabilities)) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "unsupported_field_shape",
    };
  }
  if ("conflictsWith" in value && !Array.isArray(value.conflictsWith)) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "unsupported_field_shape",
    };
  }
  const capabilities = "capabilities" in value && Array.isArray(value.capabilities) ? value.capabilities : [];
  const conflictsWith = "conflictsWith" in value && Array.isArray(value.conflictsWith) ? value.conflictsWith : [];
  if (!capabilities.every((item: unknown) => typeof item === "string")) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "non_string_capabilities",
    };
  }
  if (!conflictsWith.every((item: unknown) => typeof item === "string")) {
    return {
      kind: "raw",
      authoredJsonValue: toAuthoredConstraintsJsonValue(value),
      reason: "non_string_conflictsWith",
    };
  }
  return {
    kind: "structured",
    value: {
      capabilities: [...capabilities],
      conflictsWith: [...conflictsWith],
    },
  };
}

export function toAuthoredConstraintsJsonValue(dtoValue: unknown): unknown {
  if (!isJsonObject(dtoValue)) {
    return dtoValue;
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(dtoValue)) {
    if (key === "conflictsWith" && !("conflicts_with" in dtoValue)) {
      result.conflicts_with = value;
    } else {
      result[key] = value;
    }
  }
  return result;
}

export function constraintsCommandValue(
  authoredJsonValue: unknown,
): AdvancedConversionResult<Record<string, unknown>> {
  if (!isJsonObject(authoredJsonValue)) {
    return { ok: false, error: "Constraints must be a JSON object." };
  }
  if ("conflicts_with" in authoredJsonValue && "conflictsWith" in authoredJsonValue) {
    return { ok: false, error: "Use either conflicts_with or conflictsWith, not both." };
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(authoredJsonValue)) {
    result[key === "conflicts_with" ? "conflictsWith" : key] = value;
  }
  return { ok: true, value: result };
}

export function buildAddConstraintValue(
  current: SupportedConstraintsDto,
  field: "capabilities" | "conflictsWith",
  value: string,
): SupportedConstraintsDto {
  return {
    capabilities: field === "capabilities" ? [...current.capabilities, value] : [...current.capabilities],
    conflictsWith: field === "conflictsWith" ? [...current.conflictsWith, value] : [...current.conflictsWith],
  };
}

export function buildUpdateConstraintValue(
  current: SupportedConstraintsDto,
  field: "capabilities" | "conflictsWith",
  index: number,
  value: string,
): SupportedConstraintsDto | null {
  const values = current[field];
  if (index < 0 || index >= values.length || values[index] === value) {
    return null;
  }
  const nextValues = [...values];
  nextValues[index] = value;
  return {
    capabilities: field === "capabilities" ? nextValues : [...current.capabilities],
    conflictsWith: field === "conflictsWith" ? nextValues : [...current.conflictsWith],
  };
}

export function removeConstraintValue(
  current: SupportedConstraintsDto,
  field: "capabilities" | "conflictsWith",
  index: number,
): SupportedConstraintsDto | null {
  const values = current[field];
  if (index < 0 || index >= values.length) {
    return null;
  }
  const nextValues = values.filter((_, itemIndex) => itemIndex !== index);
  return {
    capabilities: field === "capabilities" ? nextValues : [...current.capabilities],
    conflictsWith: field === "conflictsWith" ? nextValues : [...current.conflictsWith],
  };
}

export function buildAddVerifyEntry(
  currentVerify: unknown[],
  type: SupportedVerifyType,
  fieldValue: string,
): VerifyUpdateResult {
  const fieldName = VERIFY_TYPE_FIELDS[type];
  const validation = validateKnownVerifyField(fieldName, fieldValue);
  if (validation !== null) {
    return { ok: false, error: validation };
  }
  return { ok: true, value: [...currentVerify, { type, params: { [fieldName]: fieldValue } }] };
}

export function buildVerifyKnownFieldUpdate(
  currentVerify: unknown[],
  index: number,
  fieldValue: string,
): VerifyUpdateResult | null {
  if (index < 0 || index >= currentVerify.length) {
    return { ok: false, error: "Verify entry does not exist." };
  }
  const entry = currentVerify[index];
  const classification = classifyVerifyEntry(entry);
  if (classification.kind !== "structured" || !isJsonObject(entry) || !isJsonObject(entry.params)) {
    return { ok: false, error: "Verify entry is not a supported structured check." };
  }
  const validation = validateKnownVerifyField(classification.fieldName, fieldValue);
  if (validation !== null) {
    return { ok: false, error: validation };
  }
  const nextEntry = {
    type: classification.type,
    params: {
      ...entry.params,
      [classification.fieldName]: fieldValue,
    },
  };
  if (jsonValuesEqual(nextEntry, entry)) {
    return null;
  }
  const nextVerify = [...currentVerify];
  nextVerify[index] = nextEntry;
  return { ok: true, value: nextVerify };
}

export function buildVerifyEntryJsonUpdate(
  currentVerify: unknown[],
  index: number,
  draft: string,
): VerifyUpdateResult | null {
  if (index < 0 || index >= currentVerify.length) {
    return { ok: false, error: "Verify entry does not exist." };
  }
  let value: unknown;
  try {
    value = JSON.parse(draft);
  } catch {
    return { ok: false, error: "Enter valid JSON." };
  }
  const shapeError = validateVerifyEntryCommandShape(value);
  if (shapeError !== null) {
    return { ok: false, error: shapeError };
  }
  if (jsonValuesEqual(value, currentVerify[index])) {
    return null;
  }
  const nextVerify = [...currentVerify];
  nextVerify[index] = value;
  return { ok: true, value: nextVerify };
}

export function removeVerifyEntry(currentVerify: unknown[], index: number): unknown[] {
  if (index < 0 || index >= currentVerify.length) {
    return [...currentVerify];
  }
  return currentVerify.filter((_, itemIndex) => itemIndex !== index);
}

export function moveVerifyEntry(currentVerify: unknown[], index: number, toIndex: number): unknown[] {
  if (index < 0 || index >= currentVerify.length || toIndex < 0 || toIndex >= currentVerify.length) {
    return [...currentVerify];
  }
  const next = [...currentVerify];
  const [entry] = next.splice(index, 1);
  next.splice(toIndex, 0, entry);
  return next;
}

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasUnsupportedKeys(value: unknown, allowedKeys: readonly string[]): boolean {
  if (!isJsonObject(value)) {
    return false;
  }
  const allowed = new Set(allowedKeys);
  return Object.keys(value).some((key) => !allowed.has(key));
}

function validateKnownVerifyField(fieldName: "path" | "package_name", fieldValue: string): string | null {
  if (!fieldValue.trim()) {
    return `${fieldName} is required.`;
  }
  return null;
}

function validateVerifyEntryCommandShape(value: unknown): string | null {
  if (!isJsonObject(value)) {
    return "Verify entry must be a JSON object.";
  }
  if (hasUnsupportedKeys(value, ["type", "params"])) {
    return "Verify entry supports only type and params.";
  }
  if (typeof value.type !== "string") {
    return "Verify entry type must be a string.";
  }
  if ("params" in value && value.params !== null && !isJsonObject(value.params)) {
    return "Verify entry params must be a JSON object.";
  }
  return null;
}

function stableJsonString(value: unknown): string {
  return JSON.stringify(sortJsonValue(value)) ?? "undefined";
}

function sortJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => sortJsonValue(item));
  }
  if (isJsonObject(value)) {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortJsonValue(value[key])]),
    );
  }
  return value;
}

import type { EditorCommand } from "../api/commands.js";
import type { RefCandidateDto, RefIndexDto, StepSpecDto } from "../api/types.js";

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
    if (presentNames.has(name)) {
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

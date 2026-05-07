import type { EditorCommand } from "../api/commands.js";

export type AdvancedInternalsField = "constraints" | "skipIf" | "verify";

export type AdvancedParseResult = { ok: true; value: unknown } | { ok: false; error: string };

export function formatJsonDraft(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function revertJsonDraft(value: unknown): string {
  return formatJsonDraft(value);
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
    return {
      type: "UpdateStepConstraints",
      stepId,
      constraints: nextValue as Record<string, unknown>,
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

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
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

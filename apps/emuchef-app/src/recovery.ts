/**
 * Builds a deterministic identity for the portable intent covered by recovery.
 * Runtime-only state is intentionally excluded so device discovery, review, and
 * execution changes cannot create recovery writes.
 */
export function portableIntentSignature(intent: {
  devicePlan: string | null;
  selectedRecipes: string[] | null;
  bindings: Record<string, unknown>;
}): string {
  return JSON.stringify({
    devicePlan: intent.devicePlan,
    selectedRecipes: intent.selectedRecipes ?? [],
    bindings: normalizeValue(intent.bindings),
  });
}

/** Accepts an asynchronous recovery result only for the latest request and draft. */
export function recoveryResultIsCurrent(
  result: { requestGeneration: number; draftGeneration: number },
  expectedRequestGeneration: number,
  expectedDraftGeneration: number,
): boolean {
  return result.requestGeneration === expectedRequestGeneration
    && result.draftGeneration === expectedDraftGeneration;
}

function normalizeValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(normalizeValue);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, normalizeValue(nested)]),
    );
  }
  return value;
}

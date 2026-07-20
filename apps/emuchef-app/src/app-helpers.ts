import type { AnyExecutionSnapshot, InputDescriptor } from "./types";

export function errorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { message?: unknown };
    return typeof parsed.message === "string" ? parsed.message : raw;
  } catch {
    return raw;
  }
}

export function errorCode(error: unknown): string | null {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { code?: unknown };
    return typeof parsed.code === "string" ? parsed.code : null;
  } catch {
    return null;
  }
}

export function executionDuration(snapshot: AnyExecutionSnapshot): string | null {
  if (!snapshot.startedAt) return null;
  const start = Date.parse(snapshot.startedAt);
  const finish = snapshot.finishedAt ? Date.parse(snapshot.finishedAt) : Date.now();
  if (!Number.isFinite(start) || !Number.isFinite(finish) || finish < start) return null;
  return `${Math.max(0, Math.round((finish - start) / 1000))}s`;
}

export function groupedInputs(inputs: InputDescriptor[]): Array<{
  category: string;
  inputs: InputDescriptor[];
}> {
  const groups = new Map<string, InputDescriptor[]>();
  for (const input of inputs) {
    const category = input.presentationCategory ?? "Other";
    const group = groups.get(category);
    if (group) {
      group.push(input);
    } else {
      groups.set(category, [input]);
    }
  }
  return Array.from(groups, ([category, grouped]) => ({
    category,
    inputs: grouped,
  }));
}

export function diagnosticIsBlocking(
  code: string,
  key: string | null | undefined,
  validationRequested: boolean,
  touchedInputKeys: Set<string>,
): boolean {
  return code !== "binding_missing"
    || validationRequested
    || Boolean(key && touchedInputKeys.has(key));
}

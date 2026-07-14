import type { DeviceMatch, SavedConfigurationDocument } from "./types";

export type UnsavedDecision = "save" | "discard" | "cancel";

/** Resolve the three-way dirty prompt without treating infrastructure changes as edits. */
export function resolveUnsavedDecision(
  dirty: boolean,
  saveConfirmed: boolean,
  discardConfirmed: boolean,
): UnsavedDecision {
  if (!dirty) return "discard";
  if (saveConfirmed) return "save";
  return discardConfirmed ? "discard" : "cancel";
}

/** A saved device-plan reference is usable only when current matching offers it. */
export function savedDevicePlanAvailable(
  document: SavedConfigurationDocument | null,
  match: DeviceMatch | null,
): boolean {
  if (!document || !match || match.blocked) return false;
  return [...match.candidates, ...match.safeGenericPlans]
    .some((candidate) => candidate.planId === document.devicePlan);
}

export function formatLastOpened(epochMs: number): string {
  if (!Number.isFinite(epochMs) || epochMs <= 0) return "Last opened time unavailable";
  return `Last opened ${new Date(epochMs).toLocaleString()}`;
}

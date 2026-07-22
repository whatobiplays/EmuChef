import type {
  DeviceMatch,
  SavedConfigurationDocument,
  ValidationDiagnostic,
} from "./types";

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

export function savedConfigurationBlocksProgress(
  document: SavedConfigurationDocument | null,
): boolean {
  return document?.validation.state === "requires_attention"
    || document?.validation.state === "cannot_use";
}

export function savedConfigurationValidationLabel(
  document: SavedConfigurationDocument,
): string {
  switch (document.validation.state) {
    case "valid":
      return "Ready to use";
    case "valid_with_warnings":
      return "Ready with warnings";
    case "requires_attention":
      return "Needs repair before continuing";
    case "cannot_use":
      return "Cannot be used with the current setup catalog";
  }
}

export function savedConfigurationDiagnosticSummary(
  diagnostic: ValidationDiagnostic,
): string {
  switch (diagnostic.code) {
    case "unknown_recipe":
      return "A feature used by this saved setup is no longer available.";
    case "device_plan_not_found":
    case "unknown_device_plan":
      return "The saved device setup is no longer available.";
    case "unknown_binding":
      return "A saved input no longer matches the current recipe definition.";
    default:
      return diagnostic.severity === "error"
        ? "This saved setup contains an item that must be repaired."
        : "This saved setup contains a compatibility warning.";
  }
}

export function saveConfigurationDisabledReason(
  document: SavedConfigurationDocument | null,
  hasDevicePlan: boolean,
  hasPortableChanges: boolean,
): string {
  if (document && hasDevicePlan && !hasPortableChanges) {
    return "Setup saved. Save becomes available after another change.";
  }
  return "Save requires a selected device setup and unsaved setup changes.";
}

export function formatLastOpened(epochMs: number): string {
  if (!Number.isFinite(epochMs) || epochMs <= 0) return "Last opened time unavailable";
  return `Last opened ${new Date(epochMs).toLocaleString()}`;
}

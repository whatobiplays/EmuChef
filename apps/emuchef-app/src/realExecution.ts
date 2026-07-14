import type { RealExecutionConfirmation } from "./types";

export const emptyRealExecutionConfirmation: RealExecutionConfirmation = {
  phrase: "",
  irreversibleChangesAcknowledged: false,
  noRollbackAcknowledged: false,
  keepDeviceConnectedAcknowledged: false,
};

/** Mirrors Tauri's local eligibility check without replacing authoritative validation. */
export function realExecutionConfirmationComplete(
  confirmation: RealExecutionConfirmation,
): boolean {
  return confirmation.phrase.trim() === "APPLY TO DEVICE"
    && confirmation.irreversibleChangesAcknowledged
    && confirmation.noRollbackAcknowledged
    && confirmation.keepDeviceConnectedAcknowledged;
}

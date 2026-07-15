import type { UpdateStatus } from "./types";

export interface UpdatePanelState {
  open: boolean;
  checking: boolean;
  opening: boolean;
  status: UpdateStatus | null;
}

export const initialUpdatePanelState: UpdatePanelState = {
  open: false,
  checking: false,
  opening: false,
  status: null,
};

export function updateNavigationBlocked(input: {
  startupReady: boolean;
  busy: boolean;
  executionKind: string;
  appDialogOpen: boolean;
  supportOpen: boolean;
  updatePanelOpen: boolean;
  updateChecking: boolean;
  updateOpening: boolean;
}): boolean {
  return !input.startupReady
    || input.busy
    || input.executionKind === "starting"
    || input.executionKind === "active"
    || input.appDialogOpen
    || input.supportOpen
    || !input.updatePanelOpen
    || input.updateChecking;
}

export function nextInteractionGeneration(current: number): number | null {
  if (!Number.isSafeInteger(current) || current < 0 || current >= 1_000_000) return null;
  return current + 1;
}

export function formatUpdateBytes(bytes: number | null): string {
  if (bytes === null || !Number.isSafeInteger(bytes) || bytes <= 0) return "Unknown size";
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

import type {
  CacheCleanupMode,
  CacheCleanupOutcome,
  CacheEntry,
  CacheInventory,
} from "./types";

export interface SupportState {
  open: boolean;
  requestGeneration: number;
  exportGeneration: number;
  loading: boolean;
  cleaning: boolean;
  exporting: boolean;
  exportOutcome: "idle" | "saved" | "cancelled" | "failed";
  inventory: CacheInventory | null;
  selectedHandles: string[];
  outcomes: CacheCleanupOutcome[];
  error: string | null;
}

export const initialSupportState: SupportState = {
  open: false,
  requestGeneration: 0,
  exportGeneration: 0,
  loading: false,
  cleaning: false,
  exporting: false,
  exportOutcome: "idle",
  inventory: null,
  selectedHandles: [],
  outcomes: [],
  error: null,
};

export type SupportAction =
  | { type: "open" }
  | { type: "close" }
  | { type: "inventory-requested"; generation: number }
  | { type: "inventory-loaded"; generation: number; inventory: CacheInventory }
  | { type: "inventory-failed"; generation: number; message: string }
  | { type: "toggle-selection"; handle: string }
  | { type: "cleanup-started"; generation: number }
  | { type: "cleanup-finished"; generation: number; inventory: CacheInventory; outcomes: CacheCleanupOutcome[] }
  | { type: "cleanup-failed"; message: string }
  | { type: "export-started"; generation: number }
  | { type: "export-finished"; generation: number; outcome: "saved" | "cancelled" }
  | { type: "export-failed"; generation: number; message: string }
  | { type: "runtime-restarted" };

export function supportReducer(state: SupportState, action: SupportAction): SupportState {
  switch (action.type) {
    case "open":
      return {
        ...state,
        open: true,
        exportGeneration: state.exportGeneration + 1,
        exporting: false,
        exportOutcome: "idle",
        error: null,
      };
    case "close":
      return {
        ...state,
        open: false,
        exportGeneration: state.exportGeneration + 1,
        exporting: false,
        exportOutcome: "idle",
        error: null,
      };
    case "inventory-requested":
      return { ...state, requestGeneration: action.generation, loading: true, error: null };
    case "inventory-loaded":
      if (state.requestGeneration !== action.generation) return state;
      return { ...state, loading: false, inventory: action.inventory, selectedHandles: [], error: null };
    case "inventory-failed":
      if (state.requestGeneration !== action.generation) return state;
      return { ...state, loading: false, error: action.message };
    case "toggle-selection":
      return {
        ...state,
        selectedHandles: state.selectedHandles.includes(action.handle)
          ? state.selectedHandles.filter((handle) => handle !== action.handle)
          : [...state.selectedHandles, action.handle],
      };
    case "cleanup-started":
      return { ...state, requestGeneration: action.generation, cleaning: true, error: null, outcomes: [] };
    case "cleanup-finished":
      if (state.requestGeneration !== action.generation) return state;
      return {
        ...state,
        cleaning: false,
        requestGeneration: action.generation,
        inventory: action.inventory,
        selectedHandles: [],
        outcomes: action.outcomes,
      };
    case "cleanup-failed":
      return { ...state, cleaning: false, error: action.message };
    case "export-started":
      return { ...state, exportGeneration: action.generation, exporting: true, exportOutcome: "idle", error: null };
    case "export-finished":
      if (state.exportGeneration !== action.generation) return state;
      return { ...state, exporting: false, exportOutcome: action.outcome };
    case "export-failed":
      if (state.exportGeneration !== action.generation) return state;
      return { ...state, exporting: false, exportOutcome: "failed", error: action.message };
    case "runtime-restarted":
      return {
        ...initialSupportState,
        open: state.open,
        requestGeneration: state.requestGeneration + 1,
      };
  }
}

export function entriesForCleanup(
  inventory: CacheInventory,
  mode: CacheCleanupMode,
  selectedHandles: string[],
): CacheEntry[] {
  if (mode === "selected") {
    const selected = new Set(selectedHandles);
    return inventory.entries.filter((entry) => selected.has(entry.cacheEntryHandle));
  }
  if (mode === "unused") {
    return inventory.entries.filter((entry) => entry.removable && !entry.inUse);
  }
  return inventory.entries.filter((entry) => entry.removable);
}

export function cleanupConfirmation(entries: CacheEntry[]): { entryCount: number; totalSizeBytes: number } {
  return {
    entryCount: entries.length,
    totalSizeBytes: entries.reduce((total, entry) => total + entry.sizeBytes, 0),
  };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

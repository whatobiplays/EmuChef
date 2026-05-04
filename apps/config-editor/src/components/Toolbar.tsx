import type { SidecarStatusResult } from "../api/types";

interface ToolbarProps {
  canRedo: boolean;
  canUndo: boolean;
  currentPath: string | null;
  hasDocument: boolean;
  loadingLabel: string | null;
  sidecarStatus: SidecarStatusResult | null;
  stepSpecsCount: number | null;
  stepSpecsLoading: boolean;
  onApplyDebugRename: () => void;
  onOpenRecipe: () => void;
  onRedo: () => void;
  onRefreshDocument: () => void;
  onValidate: () => void;
  onRefreshYaml: () => void;
  onSave: () => void;
  onUndo: () => void;
}

export function Toolbar({
  canRedo,
  canUndo,
  currentPath,
  hasDocument,
  loadingLabel,
  sidecarStatus,
  stepSpecsCount,
  stepSpecsLoading,
  onApplyDebugRename,
  onOpenRecipe,
  onRedo,
  onRefreshDocument,
  onValidate,
  onRefreshYaml,
  onSave,
  onUndo,
}: ToolbarProps) {
  const busy = loadingLabel !== null;
  const sidecarLabel = formatSidecarStatus(sidecarStatus);

  return (
    <div className="flex min-h-14 flex-wrap items-center gap-2 px-4 py-2">
      <div className="mr-2 flex min-w-0 flex-col">
        <span className="text-sm font-semibold text-slate-950">EmuChef Config Editor</span>
        <span className="truncate text-xs text-slate-500">{currentPath ?? "No recipe open"}</span>
      </div>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={busy}
        type="button"
        onClick={onOpenRecipe}
      >
        Open Recipe
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || busy}
        type="button"
        onClick={onRefreshDocument}
      >
        Refresh Document
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || busy}
        type="button"
        onClick={onValidate}
      >
        Validate
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || busy}
        type="button"
        onClick={onRefreshYaml}
      >
        Refresh YAML
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || !canUndo || busy}
        type="button"
        onClick={onUndo}
      >
        Undo
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || !canRedo || busy}
        type="button"
        onClick={onRedo}
      >
        Redo
      </button>
      <button
        className="rounded border border-slate-300 bg-white px-3 py-1.5 text-sm font-medium text-slate-800 shadow-sm hover:bg-slate-50 disabled:opacity-50"
        disabled={!hasDocument || busy}
        type="button"
        onClick={onSave}
      >
        Save
      </button>
      <button
        className="rounded border border-amber-300 bg-amber-50 px-3 py-1.5 text-sm font-medium text-amber-900 shadow-sm hover:bg-amber-100 disabled:opacity-50"
        disabled={!hasDocument || busy}
        type="button"
        onClick={onApplyDebugRename}
      >
        Apply Debug Rename
      </button>
      <div className="ml-auto flex items-center gap-3 text-xs text-slate-500">
        {loadingLabel ? <span>{loadingLabel}</span> : null}
        <span>{sidecarLabel}</span>
        <span>
          Step specs: {stepSpecsLoading ? "loading" : stepSpecsCount === null ? "unavailable" : stepSpecsCount}
        </span>
      </div>
    </div>
  );
}

function formatSidecarStatus(status: SidecarStatusResult | null): string {
  if (status === null) {
    return "Sidecar: unknown";
  }
  if (status.running) {
    return `Sidecar: running${status.pid === null ? "" : ` pid ${status.pid}`}`;
  }
  if (status.state) {
    return `Sidecar: ${status.state}`;
  }
  return "Sidecar: stopped";
}

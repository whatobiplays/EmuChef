import type { SidecarStatusResult } from "../api/types";
import { formatSidecarStatusLabel } from "./phase5EditorState.logic";

interface ToolbarProps {
  currentPath: string | null;
  dirty: boolean;
  documentSessionValid: boolean;
  hasDocument: boolean;
  loadingLabel: string | null;
  sidecarStatus: SidecarStatusResult | null;
  stepSpecsCount: number | null;
  stepSpecsLoading: boolean;
}

export function Toolbar({
  currentPath,
  dirty,
  documentSessionValid,
  hasDocument,
  loadingLabel,
  sidecarStatus,
  stepSpecsCount,
  stepSpecsLoading,
}: ToolbarProps) {
  const sidecarLabel = formatSidecarStatusLabel(sidecarStatus);

  return (
    <div className="flex min-h-14 flex-wrap items-center gap-2 px-4 py-2">
      <div className="mr-2 flex min-w-0 flex-1 flex-col">
        <span className="text-sm font-semibold text-slate-950">EmuChef Config Editor</span>
        <span className="truncate text-xs text-slate-500">{currentPath ?? "No recipe open"}</span>
      </div>
      <div className="flex items-center gap-3 text-xs text-slate-500">
        {loadingLabel ? <span className="font-medium text-slate-700">{loadingLabel}</span> : null}
        <span className={dirty ? "font-semibold text-amber-700" : "text-slate-500"}>
          {hasDocument ? (documentSessionValid ? (dirty ? "Unsaved" : "Saved") : "Session invalid") : "No document"}
        </span>
        <span>{sidecarLabel}</span>
        <span>
          Step specs: {stepSpecsLoading ? "loading" : stepSpecsCount === null ? "unavailable" : stepSpecsCount}
        </span>
      </div>
    </div>
  );
}

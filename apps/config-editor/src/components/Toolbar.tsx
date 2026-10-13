import type { SidecarStatusResult } from "../api/types";
import { formatSidecarStatusLabel } from "./editorState.logic";

interface ToolbarProps {
  currentPath: string | null;
  documentAuthoredRoot: string | null;
  appDefinitionOwningRoot?: string | null;
  dirty: boolean;
  pendingAppDefinitionInput: boolean;
  canSave: boolean;
  canSaveAs: boolean;
  documentSessionValid: boolean;
  hasDocument: boolean;
  loadingLabel: string | null;
  onSave: () => void;
  onSaveAs: () => void;
  selectedAuthoredRoot: string | null;
  sidecarStatus: SidecarStatusResult | null;
  stepSpecsCount: number | null;
  stepSpecsLoading: boolean;
}

export function Toolbar({
  currentPath,
  documentAuthoredRoot,
  appDefinitionOwningRoot,
  dirty,
  pendingAppDefinitionInput,
  canSave,
  canSaveAs,
  documentSessionValid,
  hasDocument,
  loadingLabel,
  onSave,
  onSaveAs,
  selectedAuthoredRoot,
  sidecarStatus,
  stepSpecsCount,
  stepSpecsLoading,
}: ToolbarProps) {
  const sidecarLabel = formatSidecarStatusLabel(sidecarStatus);

  return (
    <div className="flex min-h-14 flex-wrap items-center gap-2 px-4 py-2">
      <div className="mr-2 flex min-w-0 flex-1 flex-col">
        <span className="text-sm font-semibold text-slate-950">EmuChef Config Editor</span>
        <span className="truncate text-xs text-slate-500">{currentPath ?? "No document open"}</span>
        <span className="truncate text-xs text-slate-500">
          {appDefinitionOwningRoot !== undefined ? (
            <>Selected authored root: {documentAuthoredRoot ?? "Not set"} | Owning authored root: {appDefinitionOwningRoot ?? "None (standalone)"}</>
          ) : (
            <>Root for new opens: {selectedAuthoredRoot ?? "Not set"} | Current document root: {hasDocument ? (documentAuthoredRoot ?? "None") : "None"}</>
          )}
        </span>
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
      <div className="flex items-center gap-2">
        <button
          className="rounded border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-800 hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
          type="button"
          disabled={!canSave}
          onClick={onSave}
        >
          Save
        </button>
        <button
          className="rounded border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-800 hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
          type="button"
          disabled={!canSaveAs}
          onClick={onSaveAs}
        >
          Save As
        </button>
      </div>
      {pendingAppDefinitionInput ? (
        <p className="w-full text-xs text-amber-800" role="status">
          Apply or discard pending App Definition form input before saving.
        </p>
      ) : null}
    </div>
  );
}

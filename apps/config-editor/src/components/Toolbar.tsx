interface ToolbarProps {
  currentPath: string | null;
  hasDocument: boolean;
  loadingLabel: string | null;
  stepSpecsCount: number | null;
  stepSpecsLoading: boolean;
  onOpenRecipe: () => void;
  onValidate: () => void;
  onRefreshYaml: () => void;
}

export function Toolbar({
  currentPath,
  hasDocument,
  loadingLabel,
  stepSpecsCount,
  stepSpecsLoading,
  onOpenRecipe,
  onValidate,
  onRefreshYaml,
}: ToolbarProps) {
  const busy = loadingLabel !== null;

  return (
    <div className="flex h-14 items-center gap-3 px-4">
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
      <div className="ml-auto flex items-center gap-3 text-xs text-slate-500">
        {loadingLabel ? <span>{loadingLabel}</span> : null}
        <span>
          Step specs: {stepSpecsLoading ? "loading" : stepSpecsCount === null ? "unavailable" : stepSpecsCount}
        </span>
      </div>
    </div>
  );
}

import type { RecipeDocumentDto } from "../api/types";

export type EditorView = "overview" | "inputs" | "artifacts" | "artifactGroups" | "steps" | "stepSpecs";

interface SidebarProps {
  activeView: EditorView;
  document: RecipeDocumentDto | null;
  stepSpecsCount: number | null;
  onSelectView: (view: EditorView) => void;
}

export function Sidebar({ activeView, document, stepSpecsCount, onSelectView }: SidebarProps) {
  const recipe = document?.recipe;
  const rows = [
    ["overview", "Overview", recipe ? 1 : 0],
    ["inputs", "Inputs", recipe ? Object.keys(recipe.inputs).length : 0],
    ["artifacts", "Artifacts", recipe ? Object.keys(recipe.artifacts).length : 0],
    ["artifactGroups", "Artifact Groups", recipe ? Object.keys(recipe.artifactGroups).length : 0],
    ["steps", "Steps", recipe ? recipe.steps.length : 0],
    ["stepSpecs", "Step Specs", stepSpecsCount ?? 0],
  ] as const;

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-slate-200 p-4">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Recipe</h2>
        <p className="mt-1 truncate text-sm font-medium text-slate-900">{recipe?.id ?? "None"}</p>
      </div>
      <nav className="min-h-0 flex-1 overflow-y-auto p-2">
        {rows.map(([view, label, count]) => (
          <button
            className={`flex w-full items-center justify-between rounded px-3 py-2 text-left text-sm ${
              activeView === view ? "bg-slate-900 text-white" : "text-slate-700 hover:bg-slate-100"
            }`}
            key={view}
            type="button"
            onClick={() => onSelectView(view)}
          >
            <span>{label}</span>
            <span
              className={`rounded px-2 py-0.5 text-xs ${
                activeView === view ? "bg-white/15 text-white" : "bg-slate-100 text-slate-600"
              }`}
            >
              {count}
            </span>
          </button>
        ))}
      </nav>
    </div>
  );
}

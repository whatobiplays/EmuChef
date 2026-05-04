import type { RecipeDocumentDto } from "../api/types";

interface SidebarProps {
  document: RecipeDocumentDto | null;
  stepSpecsCount: number | null;
}

export function Sidebar({ document, stepSpecsCount }: SidebarProps) {
  const recipe = document?.recipe;
  const rows = [
    ["Overview", recipe ? 1 : 0],
    ["Inputs", recipe ? Object.keys(recipe.inputs).length : 0],
    ["Artifacts", recipe ? Object.keys(recipe.artifacts).length : 0],
    ["Artifact Groups", recipe ? Object.keys(recipe.artifactGroups).length : 0],
    ["Steps", recipe ? recipe.steps.length : 0],
    ["Step Specs", stepSpecsCount ?? 0],
  ] as const;

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-slate-200 p-4">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-500">Recipe</h2>
        <p className="mt-1 truncate text-sm font-medium text-slate-900">{recipe?.id ?? "None"}</p>
      </div>
      <nav className="min-h-0 flex-1 overflow-y-auto p-2">
        {rows.map(([label, count]) => (
          <div
            className="flex items-center justify-between rounded px-3 py-2 text-sm text-slate-700"
            key={label}
          >
            <span>{label}</span>
            <span className="rounded bg-slate-100 px-2 py-0.5 text-xs text-slate-600">{count}</span>
          </div>
        ))}
      </nav>
    </div>
  );
}

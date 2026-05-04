import type { RecipeDocumentDto, StepDto } from "../api/types";

interface RecipeSummaryProps {
  document: RecipeDocumentDto;
}

export function RecipeSummary({ document }: RecipeSummaryProps) {
  const { recipe } = document;
  const counts = [
    ["Inputs", Object.keys(recipe.inputs).length],
    ["Artifacts", Object.keys(recipe.artifacts).length],
    ["Artifact Groups", Object.keys(recipe.artifactGroups).length],
    ["Steps", recipe.steps.length],
  ] as const;

  return (
    <div className="space-y-6 p-6">
      <section className="border-b border-slate-200 pb-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <h1 className="truncate text-2xl font-semibold text-slate-950">{recipe.name}</h1>
            <p className="mt-1 text-sm text-slate-600">{recipe.description || "No description"}</p>
          </div>
          <div className="grid grid-cols-2 gap-2 text-xs">
            <Badge label="ID" value={recipe.id} />
            <Badge label="Kind" value={recipe.kind} />
            <Badge label="Schema" value={String(recipe.schemaVersion)} />
            <Badge label="Dirty" value={document.dirty ? "yes" : "no"} />
          </div>
        </div>
      </section>

      <section className="grid grid-cols-4 gap-3">
        {counts.map(([label, value]) => (
          <div className="rounded border border-slate-200 bg-white p-4" key={label}>
            <div className="text-xs font-medium uppercase tracking-wide text-slate-500">{label}</div>
            <div className="mt-2 text-2xl font-semibold text-slate-950">{value}</div>
          </div>
        ))}
      </section>

      <section>
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Ordered Steps</h2>
        <div className="mt-3 space-y-2">
          {recipe.steps.length === 0 ? (
            <p className="text-sm text-slate-500">No steps</p>
          ) : (
            recipe.steps.map((step, index) => <StepRow index={index} key={step.id} step={step} />)
          )}
        </div>
      </section>
    </div>
  );
}

function Badge({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded border border-slate-200 bg-white px-3 py-2">
      <div className="text-slate-500">{label}</div>
      <div className="truncate font-medium text-slate-900">{value}</div>
    </div>
  );
}

function StepRow({ index, step }: { index: number; step: StepDto }) {
  return (
    <div className="grid grid-cols-[3rem_minmax(0,1fr)_12rem] items-center gap-3 rounded border border-slate-200 bg-white px-3 py-2">
      <span className="text-xs tabular-nums text-slate-500">{index + 1}</span>
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-slate-950">{step.name || step.id}</div>
        <div className="truncate text-xs text-slate-500">{step.id}</div>
      </div>
      <span className="truncate rounded bg-slate-100 px-2 py-1 text-xs text-slate-700">{step.type}</span>
    </div>
  );
}

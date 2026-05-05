import type { StepSpecDto } from "../api/types";

interface StepSpecsPanelProps {
  stepSpecs: StepSpecDto[];
}

export function StepSpecsPanel({ stepSpecs }: StepSpecsPanelProps) {
  return (
    <section className="h-full min-h-0 overflow-y-auto border-t border-slate-200 bg-white p-6">
      <details>
        <summary className="cursor-pointer text-sm font-semibold uppercase tracking-wide text-slate-500">
          Step Specs ({stepSpecs.length})
        </summary>
        <div className="mt-4 grid grid-cols-2 gap-2">
          {stepSpecs.map((spec) => (
            <div className="rounded border border-slate-200 p-3" key={spec.type}>
              <div className="truncate text-sm font-medium text-slate-950">{spec.label}</div>
              <div className="mt-1 truncate font-mono text-xs text-slate-500">{spec.type}</div>
              <div className="mt-2 text-xs text-slate-600">
                Primary output: {spec.primaryOutputName ?? "none"}
              </div>
            </div>
          ))}
        </div>
      </details>
    </section>
  );
}

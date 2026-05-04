import type { DiagnosticDto } from "../api/types";

interface DiagnosticsPanelProps {
  diagnostics: DiagnosticDto[];
}

export function DiagnosticsPanel({ diagnostics }: DiagnosticsPanelProps) {
  return (
    <section className="border-b border-slate-200">
      <div className="flex items-center justify-between border-b border-slate-200 px-4 py-3">
        <h2 className="text-sm font-semibold text-slate-950">Diagnostics</h2>
        <span className="rounded bg-slate-100 px-2 py-0.5 text-xs text-slate-600">{diagnostics.length}</span>
      </div>
      <div className="max-h-72 overflow-y-auto p-3">
        {diagnostics.length === 0 ? (
          <p className="px-1 py-2 text-sm text-slate-500">No diagnostics</p>
        ) : (
          <div className="space-y-2">
            {diagnostics.map((diagnostic, index) => (
              <DiagnosticItem diagnostic={diagnostic} index={index} key={`${diagnostic.code}-${index}`} />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function DiagnosticItem({ diagnostic, index }: { diagnostic: DiagnosticDto; index: number }) {
  return (
    <article className="rounded border border-slate-200 bg-white p-3 text-sm">
      <div className="flex items-center gap-2">
        <span className={severityClass(diagnostic.severity)}>{diagnostic.severity}</span>
        <span className="font-mono text-xs text-slate-500">{diagnostic.code || `diagnostic_${index}`}</span>
      </div>
      <p className="mt-2 text-slate-800">{diagnostic.message}</p>
      <p className="mt-2 text-xs text-slate-500">
        {[diagnostic.objectKind, diagnostic.objectId, diagnostic.field].filter(Boolean).join(" / ") ||
          diagnostic.file ||
          "No location"}
      </p>
    </article>
  );
}

function severityClass(severity: string) {
  if (severity === "error") {
    return "rounded bg-red-100 px-2 py-0.5 text-xs font-medium text-red-700";
  }
  if (severity === "warning") {
    return "rounded bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-700";
  }
  return "rounded bg-slate-100 px-2 py-0.5 text-xs font-medium text-slate-700";
}

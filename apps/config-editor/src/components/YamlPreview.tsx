interface YamlPreviewProps {
  yaml: string;
}

export function YamlPreview({ yaml }: YamlPreviewProps) {
  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-slate-200 px-4 py-3">
        <h2 className="text-sm font-semibold text-slate-950">Canonical YAML</h2>
      </div>
      <pre className="min-h-0 flex-1 overflow-auto whitespace-pre p-4 font-mono text-xs leading-5 text-slate-800">
        {yaml || "No YAML loaded"}
      </pre>
    </section>
  );
}

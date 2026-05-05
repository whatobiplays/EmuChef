interface ReadOnlyJsonProps {
  label: string;
  value: unknown;
}

export function ReadOnlyJson({ label, value }: ReadOnlyJsonProps) {
  const text = JSON.stringify(value, null, 2);

  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      <pre className="max-h-48 overflow-auto rounded border border-slate-200 bg-slate-50 p-3 text-xs text-slate-700">
        {text}
      </pre>
    </label>
  );
}

export function EmptyState() {
  return (
    <div className="flex min-h-[24rem] items-center justify-center p-6">
      <div className="max-w-md text-center">
        <h1 className="text-xl font-semibold text-slate-950">No recipe open</h1>
        <p className="mt-2 text-sm leading-6 text-slate-600">
          Open a YAML recipe to inspect its summary, diagnostics, canonical YAML, and available step specs.
        </p>
      </div>
    </div>
  );
}

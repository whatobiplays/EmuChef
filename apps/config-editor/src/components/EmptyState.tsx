interface EmptyStateProps {
  sidecarAvailable?: boolean;
  sidecarMessage?: string | null;
}

export function EmptyState({ sidecarAvailable = true, sidecarMessage = null }: EmptyStateProps) {
  return (
    <div className="flex min-h-[24rem] items-center justify-center p-6">
      <div className="max-w-md text-center">
        <h1 className="text-xl font-semibold text-slate-950">No recipe open</h1>
        <p className="mt-2 text-sm leading-6 text-slate-600">
          Use File &gt; Open Recipe to open a YAML recipe for authored-model editing, diagnostics, and read-only
          canonical YAML preview.
        </p>
        {sidecarAvailable ? null : (
          <p className="mt-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-sm leading-6 text-amber-900">
            {sidecarMessage ??
              "The backend sidecar is unavailable. Build the local Rust sidecar with cargo build --manifest-path crates/emuchef-rust-backend/Cargo.toml, then restart the Tauri app."}
          </p>
        )}
      </div>
    </div>
  );
}

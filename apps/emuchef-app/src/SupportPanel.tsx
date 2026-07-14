import type { CacheCleanupMode } from "./types";
import { formatBytes, type SupportState } from "./support";

interface SupportPanelProps {
  state: SupportState;
  onClose: () => void;
  onRefresh: () => void;
  onToggleSelection: (handle: string) => void;
  onCleanup: (mode: CacheCleanupMode) => void;
  onExport: () => void;
}

/** Support and app-owned artifact storage controls with no filesystem authority. */
export function SupportPanel({
  state,
  onClose,
  onRefresh,
  onToggleSelection,
  onCleanup,
  onExport,
}: SupportPanelProps) {
  if (!state.open) return null;
  const inventory = state.inventory;
  return (
    <div className="support-backdrop" role="presentation">
      <section aria-label="Support and storage" aria-modal="true" className="support-panel" role="dialog">
        <div className="support-heading">
          <div>
            <p className="eyebrow">SUPPORT & STORAGE</p>
            <h2>Diagnostics and artifact cache</h2>
          </div>
          <button className="secondary" onClick={onClose}>Close</button>
        </div>

        <section className="support-section">
          <h3>Support diagnostics</h3>
          <p>Export a sanitized local ZIP. It excludes paths, serials, credentials, raw logs, and configuration contents.</p>
          <button disabled={state.exporting} onClick={onExport}>
            {state.exporting ? "Exporting…" : "Export diagnostics…"}
          </button>
          {state.exportOutcome === "saved" && <p className="success">Diagnostics saved.</p>}
          {state.exportOutcome === "cancelled" && <p className="fine-print">Export cancelled.</p>}
        </section>

        <section className="support-section">
          <div className="support-heading">
            <div>
              <h3>Artifact cache</h3>
              <p className="fine-print">Only app-owned logical cache entries can be removed.</p>
            </div>
            <button className="secondary" disabled={state.loading || state.cleaning} onClick={onRefresh}>
              {state.loading ? "Refreshing…" : "Refresh"}
            </button>
          </div>
          {inventory && (
            <p>
              {inventory.summary.entryCount} entries · {formatBytes(inventory.summary.totalSizeBytes)}
              {inventory.summary.unmanagedCount > 0 ? ` · ${inventory.summary.unmanagedCount} unmanaged` : ""}
            </p>
          )}
          <div className="cache-list">
            {inventory?.entries.map((entry) => (
              <label className="cache-entry" key={entry.cacheEntryHandle}>
                <input
                  checked={state.selectedHandles.includes(entry.cacheEntryHandle)}
                  disabled={!entry.removable || entry.inUse || state.cleaning}
                  onChange={() => onToggleSelection(entry.cacheEntryHandle)}
                  type="checkbox"
                />
                <span>
                  <strong>{entry.artifactLabel}</strong>
                  <small>
                    {entry.category.replaceAll("_", " ")} · {entry.sourceKind.replaceAll("_", " ")} · {entry.integrityState.replaceAll("_", " ")} · {formatBytes(entry.sizeBytes)} · {entry.ageBucket.replaceAll("_", " ")}
                    {entry.inUse ? " · in use" : ""}
                  </small>
                </span>
              </label>
            ))}
            {inventory && inventory.entries.length === 0 && <p className="empty-state">The app-owned artifact cache is empty.</p>}
          </div>
          <div className="button-row">
            <button
              className="danger"
              disabled={state.cleaning || state.selectedHandles.length === 0}
              onClick={() => onCleanup("selected")}
            >Remove selected</button>
            <button className="danger" disabled={state.cleaning || !inventory} onClick={() => onCleanup("unused")}>Clear unused</button>
            <button className="danger" disabled={state.cleaning || !inventory} onClick={() => onCleanup("all_removable")}>Clear all removable</button>
          </div>
          {state.outcomes.map((outcome) => (
            <p className={outcome.outcome === "removed" ? "success" : "warning"} key={`${outcome.entryHandle}-${outcome.code}`}>
              {outcome.message} <code>{outcome.code}</code>
            </p>
          ))}
        </section>
        {state.error && <p className="error" role="alert">{state.error}</p>}
      </section>
    </div>
  );
}

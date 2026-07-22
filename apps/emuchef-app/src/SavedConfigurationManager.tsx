import { useRef } from "react";

import { AccessibleDialog } from "./AccessibleDialog";
import { formatLastOpened } from "./savedConfigurations";
import type {
  RecentConfiguration,
  SavedConfigurationDocument,
  SavedConfigurationPreview,
} from "./types";

interface Props {
  active: SavedConfigurationDocument | null;
  busy: boolean;
  canSave: boolean;
  canSaveAs: boolean;
  preview: SavedConfigurationPreview | null;
  previewMode: "open" | "import";
  recents: RecentConfiguration[];
  returnFocus: HTMLElement | null;
  onClose: () => void;
  onNew: () => void;
  onSave: () => void;
  onSaveAs: () => void;
  onOpenPicker: () => void;
  onImportPicker: () => void;
  onConfirmPreview: () => void;
  onRepairPreview: (repairHandle: string) => void;
  onCancelPreview: () => void;
  onOpenRecent: (handle: string) => void;
  onRelinkRecent: (handle: string) => void;
  onRemoveRecent: (handle: string) => void;
  onRename: () => void;
  onDuplicate: () => void;
  onExport: () => void;
}

function compatibilitySummary(preview: SavedConfigurationPreview): string {
  switch (preview.compatibility.state) {
    case "compatible":
      return "This saved setup is ready to open.";
    case "compatible_with_warnings":
      return "This saved setup can be opened, but some saved choices need attention.";
    case "migrated_baseline_pending":
      return "This older saved setup can be opened. Its next explicit save will record current compatibility information.";
    case "materially_changed":
      return "The available setup has changed since this file was saved. Review and repair the saved choices before opening it.";
    case "repair_required":
      return "This saved setup must be repaired before it can be opened.";
  }
}

function comparisonSummary(preview: SavedConfigurationPreview): string | null {
  switch (preview.comparison?.state) {
    case "matches":
      return "This saved setup matches the setup currently in progress.";
    case "differs":
      return "Opening this saved setup will replace the setup choices currently in progress.";
    case "requires_repair":
      return "Repair this saved setup before it can replace the setup currently in progress.";
    case "no_current_intent":
      return "No setup is currently in progress.";
    default:
      return null;
  }
}

/** Focused saved-setup maintenance surface; paths and compatibility authority stay native. */
export function SavedConfigurationManager({
  active,
  busy,
  canSave,
  canSaveAs,
  preview,
  previewMode,
  recents,
  returnFocus,
  onClose,
  onNew,
  onSave,
  onSaveAs,
  onOpenPicker,
  onImportPicker,
  onConfirmPreview,
  onRepairPreview,
  onCancelPreview,
  onOpenRecent,
  onRelinkRecent,
  onRemoveRecent,
  onRename,
  onDuplicate,
  onExport,
}: Props) {
  const closeRef = useRef<HTMLButtonElement>(null);
  return (
    <AccessibleDialog
      descriptionId="saved-setup-manager-description"
      initialFocusRef={closeRef}
      onDismiss={onClose}
      returnFocus={returnFocus}
      titleId="saved-setup-manager-title"
    >
      <div className="dialog-heading">
        <div>
          <h2 id="saved-setup-manager-title">Saved setups</h2>
          <p id="saved-setup-manager-description">Open, organize, import, or export reusable setup choices.</p>
        </div>
        <button className="secondary" onClick={onClose} ref={closeRef}>Close</button>
      </div>

      {preview ? (
        <section className="configuration-summary" aria-labelledby="configuration-summary-title">
          <h3 id="configuration-summary-title">Review {preview.name}</h3>
          <p><strong>{preview.setupLabel}</strong></p>
          <p>{compatibilitySummary(preview)}</p>
          <p>{preview.featureLabels.length > 0
            ? preview.featureLabels.join(", ")
            : "No optional features selected"}</p>
          <p>{preview.savedInputCount} reusable input {preview.savedInputCount === 1 ? "reference" : "references"}; {preview.omittedInputCount} omitted.</p>
          <p>{preview.fileLabel}{preview.lastModifiedEpochMs ? ` · ${formatLastOpened(preview.lastModifiedEpochMs).replace("Last opened", "Modified")}` : ""}</p>
          {comparisonSummary(preview) && <p>{comparisonSummary(preview)}</p>}
          {preview.repairActions.length > 0 && (
            <div className="button-row" aria-label="Available repairs">
              {preview.repairActions.map((repair) => (
                <button
                  aria-describedby={busy ? "saved-preview-busy-reason" : undefined}
                  className="secondary"
                  disabled={busy}
                  key={repair.repairHandle}
                  onClick={() => onRepairPreview(repair.repairHandle)}
                >{repair.label}</button>
              ))}
            </div>
          )}
          <div className="button-row dialog-actions">
            <button aria-describedby={busy ? "saved-preview-busy-reason" : preview.compatibility.requiresRepair ? "saved-preview-repair-reason" : undefined} disabled={busy || preview.compatibility.requiresRepair} onClick={onConfirmPreview}>
              {previewMode === "import" ? "Import a copy…" : "Open setup"}
            </button>
            <button aria-describedby={busy ? "saved-preview-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onCancelPreview}>Cancel</button>
          </div>
          {busy && <p className="disabled-reason" id="saved-preview-busy-reason" role="status">Saved-setup actions are unavailable while the current operation finishes.</p>}
          {preview.compatibility.requiresRepair && (
            <p className="disabled-reason" id="saved-preview-repair-reason">This setup must be repaired before it can replace the active workflow.</p>
          )}
        </section>
      ) : (
        <>
          <div className="button-row dialog-actions">
            <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onNew}>New</button>
            <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} disabled={busy} onClick={onOpenPicker}>Open…</button>
            <button aria-describedby={busy ? "saved-manager-busy-reason" : !canSave ? "saved-manager-save-reason" : undefined} disabled={busy || !canSave} onClick={onSave}>Save</button>
            <button aria-describedby={busy ? "saved-manager-busy-reason" : !canSaveAs ? "saved-manager-save-as-reason" : undefined} className="secondary" disabled={busy || !canSaveAs} onClick={onSaveAs}>Save As…</button>
            <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onImportPicker}>Import…</button>
            {active && <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onRename}>Rename…</button>}
            {active && <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onDuplicate}>Duplicate…</button>}
            {active && <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={onExport}>Export…</button>}
          </div>
          {busy && <p className="disabled-reason" id="saved-manager-busy-reason" role="status">Saved-setup actions are unavailable while the current operation finishes.</p>}
          {!busy && !canSave && <p className="disabled-reason" id="saved-manager-save-reason">Save becomes available after an opened setup has unsaved changes.</p>}
          {!busy && !canSaveAs && <p className="disabled-reason" id="saved-manager-save-as-reason">Choose a device setup before saving a copy.</p>}
          <section className="recent-configurations" aria-labelledby="manager-recents-title">
            <h3 id="manager-recents-title">Recent setups</h3>
            {recents.length === 0 ? <p className="empty-state">No saved setups have been opened yet. Choose Open or Import to add one.</p> : (
              <ul>
                {recents.map((recent) => (
                  <li key={recent.recentHandle}>
                    <div>
                      <strong>{recent.name}</strong>
                      <small>{recent.fileLabel} · {formatLastOpened(recent.lastOpenedEpochMs)}</small>
                      {recent.identityConflict && <small className="warning">Another file uses the same setup identity.</small>}
                    </div>
                    {recent.availability === "available" ? (
                      <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={() => onOpenRecent(recent.recentHandle)}>Review</button>
                    ) : (
                      <>
                        <span className="error">File missing</span>
                        <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="secondary" disabled={busy} onClick={() => onRelinkRecent(recent.recentHandle)}>Relink…</button>
                      </>
                    )}
                    <button aria-describedby={busy ? "saved-manager-busy-reason" : undefined} className="text-button danger-text" disabled={busy} onClick={() => onRemoveRecent(recent.recentHandle)}>Remove from Recent</button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </>
      )}
    </AccessibleDialog>
  );
}

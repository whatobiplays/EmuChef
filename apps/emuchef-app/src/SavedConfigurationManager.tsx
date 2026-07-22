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
      <div className="support-heading">
        <div>
          <h2 id="saved-setup-manager-title">Saved setups</h2>
          <p id="saved-setup-manager-description">Open, organize, import, or export reusable setup choices.</p>
        </div>
        <button className="secondary" onClick={onClose} ref={closeRef}>Close</button>
      </div>

      {preview ? (
        <section className="configuration-summary" aria-labelledby="configuration-summary-title">
          <h3 id="configuration-summary-title">Review {preview.name}</h3>
          <p><strong>{preview.setupLabel}</strong> · Schema {preview.schemaVersion}</p>
          <p>{preview.compatibility.message}</p>
          {preview.compatibility.state === "migrated_baseline_pending" && (
            <p>The current catalog validates this older file, but historical compatibility cannot be established until its first explicit save.</p>
          )}
          <p>{preview.featureLabels.length > 0
            ? preview.featureLabels.join(", ")
            : "No optional features selected"}</p>
          <p>{preview.savedInputCount} reusable input {preview.savedInputCount === 1 ? "reference" : "references"}; {preview.omittedInputCount} omitted.</p>
          <p>{preview.fileLabel}{preview.lastModifiedEpochMs ? ` · ${formatLastOpened(preview.lastModifiedEpochMs).replace("Last opened", "Modified")}` : ""}</p>
          {preview.comparison && <p>{preview.comparison.message}</p>}
          {preview.repairActions.length > 0 && (
            <div className="button-row" aria-label="Available repairs">
              {preview.repairActions.map((repair) => (
                <button
                  className="secondary"
                  disabled={busy}
                  key={repair.repairHandle}
                  onClick={() => onRepairPreview(repair.repairHandle)}
                >{repair.label}</button>
              ))}
            </div>
          )}
          <div className="button-row">
            <button disabled={busy || preview.compatibility.requiresRepair} onClick={onConfirmPreview}>
              {previewMode === "import" ? "Import a copy…" : "Open setup"}
            </button>
            <button className="secondary" disabled={busy} onClick={onCancelPreview}>Cancel</button>
          </div>
          {preview.compatibility.requiresRepair && (
            <p className="disabled-reason">This setup must be repaired before it can replace the active workflow.</p>
          )}
        </section>
      ) : (
        <>
          <div className="button-row">
            <button className="secondary" disabled={busy} onClick={onNew}>New</button>
            <button disabled={busy} onClick={onOpenPicker}>Open…</button>
            <button disabled={busy || !canSave} onClick={onSave}>Save</button>
            <button className="secondary" disabled={busy || !canSaveAs} onClick={onSaveAs}>Save As…</button>
            <button className="secondary" disabled={busy} onClick={onImportPicker}>Import…</button>
            {active && <button className="secondary" disabled={busy} onClick={onRename}>Rename…</button>}
            {active && <button className="secondary" disabled={busy} onClick={onDuplicate}>Duplicate…</button>}
            {active && <button className="secondary" disabled={busy} onClick={onExport}>Export…</button>}
          </div>
          <section className="recent-configurations" aria-labelledby="manager-recents-title">
            <h3 id="manager-recents-title">Recent setups</h3>
            {recents.length === 0 ? <p>No recent setup files.</p> : (
              <ul>
                {recents.map((recent) => (
                  <li key={recent.recentHandle}>
                    <div>
                      <strong>{recent.name}</strong>
                      <small>{recent.fileLabel} · {formatLastOpened(recent.lastOpenedEpochMs)}</small>
                      {recent.identityConflict && <small className="warning">Another file uses the same setup identity.</small>}
                    </div>
                    {recent.availability === "available" ? (
                      <button className="secondary" disabled={busy} onClick={() => onOpenRecent(recent.recentHandle)}>Review</button>
                    ) : (
                      <>
                        <span className="error">File missing</span>
                        <button className="secondary" disabled={busy} onClick={() => onRelinkRecent(recent.recentHandle)}>Relink…</button>
                      </>
                    )}
                    <button className="text-button danger-text" disabled={busy} onClick={() => onRemoveRecent(recent.recentHandle)}>Remove from Recent</button>
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

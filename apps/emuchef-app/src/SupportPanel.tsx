import { useEffect, useRef, useState } from "react";

import { AccessibleDialog } from "./AccessibleDialog";
import {
  DialogController,
  describedBy,
  lifecycleBoundResult,
  restoreAccessibleFocus,
  stableDomId,
  type DialogSnapshot,
} from "./accessibility";
import type { CacheCleanupMode } from "./types";
import { formatBytes, type SupportState } from "./support";

interface CleanupConfirmation {
  entryCount: number;
  totalSizeBytes: number;
}

interface CleanupDialogPayload {
  mode: CacheCleanupMode;
  confirmation: CleanupConfirmation;
  invoker: HTMLElement | null;
}

interface SupportPanelProps {
  state: SupportState;
  returnFocus: HTMLElement | null;
  onClose: () => void;
  onRefresh: () => void;
  onToggleSelection: (handle: string) => void;
  onPrepareCleanup: (mode: CacheCleanupMode) => CleanupConfirmation | null;
  onCleanup: (mode: CacheCleanupMode) => void;
  onExport: () => void;
  onAnnounce: (message: string, assertive?: boolean) => void;
}

/** Support and app-owned artifact storage controls with no filesystem authority. */
export function SupportPanel({
  state,
  returnFocus,
  onClose,
  onRefresh,
  onToggleSelection,
  onPrepareCleanup,
  onCleanup,
  onExport,
  onAnnounce,
}: SupportPanelProps) {
  const cleanupControllerRef = useRef(new DialogController<CleanupDialogPayload, boolean>());
  const cleanupController = cleanupControllerRef.current;
  const [cleanupDialog, setCleanupDialog] = useState<DialogSnapshot<CleanupDialogPayload> | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const cleanupCancelRef = useRef<HTMLButtonElement>(null);
  const lifecycleGenerationRef = useRef(0);

  useEffect(() => {
    const unsubscribe = cleanupController.subscribe(setCleanupDialog);
    return () => {
      lifecycleGenerationRef.current += 1;
      unsubscribe();
      cleanupController.dispose();
    };
  }, [cleanupController]);

  useEffect(() => {
    if (!state.open) cleanupController.cancelActive();
  }, [cleanupController, state.open]);

  useEffect(() => {
    if (!cleanupDialog) return;
    queueMicrotask(() => cleanupCancelRef.current?.focus({ preventScroll: true }));
  }, [cleanupDialog?.id]);

  if (!state.open) return null;
  const inventory = state.inventory;
  const closeBlocked = state.cleaning || state.exporting;

  const closePanel = () => {
    if (closeBlocked) {
      onAnnounce(
        state.cleaning
          ? "Support and Storage cannot close until the confirmed cache cleanup finishes."
          : "Support and Storage cannot close until the diagnostics export finishes.",
        true,
      );
      return;
    }
    cleanupController.cancelActive();
    lifecycleGenerationRef.current += 1;
    onClose();
  };

  const requestCleanup = async (mode: CacheCleanupMode, invoker: HTMLElement) => {
    const lifecycleGeneration = lifecycleGenerationRef.current;
    const confirmation = onPrepareCleanup(mode);
    if (!confirmation) return;
    const request = cleanupController.request({ mode, confirmation, invoker }, false);
    if (!request.accepted) {
      onAnnounce("A cache cleanup confirmation is already open.");
      return;
    }
    const confirmed = await lifecycleBoundResult(
      request.result,
      false,
      lifecycleGeneration,
      () => lifecycleGenerationRef.current,
    );
    if (confirmed) {
      onCleanup(mode);
    } else {
      queueMicrotask(() => restoreAccessibleFocus({ invoker }));
    }
  };

  const titleId = cleanupDialog ? "cache-cleanup-title" : "support-title";
  const descriptionId = cleanupDialog ? "cache-cleanup-description" : "support-description";

  return (
    <AccessibleDialog
      className="support-panel"
      currentDialogId={() => cleanupController.activeId}
      descriptionId={descriptionId}
      dialogId={cleanupDialog?.id}
      dismissible={!closeBlocked}
      initialFocusRef={closeRef}
      onDismiss={closePanel}
      onDismissBlocked={closePanel}
      returnFocus={returnFocus}
      role={cleanupDialog ? "alertdialog" : "dialog"}
      titleId={titleId}
    >
      {cleanupDialog ? (
        <div className="confirmation-content">
          <p className="eyebrow">CONFIRM CACHE CLEANUP</p>
          <h2 id="cache-cleanup-title">Remove app-owned cache entries?</h2>
          <p id="cache-cleanup-description">
            Remove {cleanupDialog.payload.confirmation.entryCount} cache {cleanupDialog.payload.confirmation.entryCount === 1 ? "entry" : "entries"}
            {" "}totaling {formatBytes(cleanupDialog.payload.confirmation.totalSizeBytes)}. In-use and non-removable entries remain protected.
          </p>
          <div className="button-row">
            <button
              className="secondary"
              onClick={() => cleanupController.settle(cleanupDialog.id, false)}
              ref={cleanupCancelRef}
            >Cancel</button>
            <button
              className="danger"
              onClick={() => cleanupController.settle(cleanupDialog.id, true)}
            >Remove confirmed entries</button>
          </div>
        </div>
      ) : (
        <>
          <div className="support-heading">
            <div>
              <p className="eyebrow">SUPPORT & STORAGE</p>
              <h2 id="support-title">Diagnostics and artifact cache</h2>
              <p id="support-description">Export sanitized support information and manage only app-owned cache entries.</p>
            </div>
            <button
              aria-describedby={closeBlocked ? "support-close-reason" : undefined}
              className="secondary"
              disabled={closeBlocked}
              onClick={closePanel}
              ref={closeRef}
            >Close</button>
          </div>
          {closeBlocked && (
            <p className="disabled-reason" id="support-close-reason" role="status">
              {state.cleaning ? "Close is unavailable while cleanup finishes." : "Close is unavailable while export finishes."}
            </p>
          )}

          <section className="support-section" aria-labelledby="support-diagnostics-heading">
            <h3 id="support-diagnostics-heading">Support diagnostics</h3>
            <p>Export a sanitized local ZIP. It excludes paths, serials, credentials, raw logs, and configuration contents.</p>
            <button
              aria-describedby={state.exporting ? "diagnostics-export-reason" : undefined}
              disabled={state.exporting}
              onClick={onExport}
            >
              {state.exporting ? "Exporting…" : "Export diagnostics…"}
            </button>
            {state.exporting && <p className="disabled-reason" id="diagnostics-export-reason">Export is already in progress.</p>}
            {state.exportOutcome === "saved" && <p className="success" role="status">Success: diagnostics saved.</p>}
            {state.exportOutcome === "cancelled" && <p className="fine-print" role="status">Diagnostics export cancelled.</p>}
          </section>

          <section className="support-section" aria-labelledby="artifact-cache-heading">
            <div className="support-heading">
              <div>
                <h3 id="artifact-cache-heading">Artifact cache</h3>
                <p className="fine-print">Only app-owned logical cache entries can be removed.</p>
              </div>
              <button
                aria-describedby={state.loading || state.cleaning ? "cache-refresh-reason" : undefined}
                className="secondary"
                disabled={state.loading || state.cleaning}
                onClick={onRefresh}
              >
                {state.loading ? "Refreshing…" : "Refresh"}
              </button>
            </div>
            {(state.loading || state.cleaning) && (
              <p className="disabled-reason" id="cache-refresh-reason" role="status">
                {state.cleaning ? "Refresh is unavailable while cleanup finishes." : "The cache inventory is loading."}
              </p>
            )}
            {inventory && (
              <p>
                {inventory.summary.entryCount} entries · {formatBytes(inventory.summary.totalSizeBytes)}
                {inventory.summary.unmanagedCount > 0 ? ` · ${inventory.summary.unmanagedCount} unmanaged` : ""}
              </p>
            )}
            <div className="cache-list" role="region" aria-label="App-owned artifact cache entries" tabIndex={0}>
              <ul>
                {inventory?.entries.map((entry, index) => {
                  const inputId = stableDomId("cache-entry", entry.artifactLabel, index);
                  const description = `${inputId}-description`;
                  const reason = `${inputId}-reason`;
                  const disabled = !entry.removable || entry.inUse || state.cleaning;
                  return (
                    <li className="cache-entry" key={entry.cacheEntryHandle}>
                      <input
                        aria-describedby={describedBy(description, disabled && reason)}
                        checked={state.selectedHandles.includes(entry.cacheEntryHandle)}
                        disabled={disabled}
                        id={inputId}
                        onChange={() => onToggleSelection(entry.cacheEntryHandle)}
                        type="checkbox"
                      />
                      <span>
                        <label htmlFor={inputId}><strong>{entry.artifactLabel}</strong></label>
                        <small id={description}>
                          {entry.category.replaceAll("_", " ")} · {entry.sourceKind.replaceAll("_", " ")} · {entry.integrityState.replaceAll("_", " ")} · {formatBytes(entry.sizeBytes)} · {entry.ageBucket.replaceAll("_", " ")}
                        </small>
                        {disabled && (
                          <small id={reason}>
                            {state.cleaning ? "Unavailable while cleanup finishes." : entry.inUse ? "In use and protected from removal." : "This entry is not removable."}
                          </small>
                        )}
                      </span>
                    </li>
                  );
                })}
              </ul>
              {inventory && inventory.entries.length === 0 && <p className="empty-state">The app-owned artifact cache is empty.</p>}
              {!inventory && !state.loading && <p className="empty-state">Cache inventory is unavailable. Refresh to try again.</p>}
            </div>
            <div className="button-row">
              <button
                aria-describedby={state.selectedHandles.length === 0 ? "remove-selected-reason" : undefined}
                className="danger"
                disabled={state.cleaning || state.selectedHandles.length === 0}
                onClick={(event) => void requestCleanup("selected", event.currentTarget)}
              >Remove selected</button>
              <button className="danger" disabled={state.cleaning || !inventory} onClick={(event) => void requestCleanup("unused", event.currentTarget)}>Clear unused</button>
              <button className="danger" disabled={state.cleaning || !inventory} onClick={(event) => void requestCleanup("all_removable", event.currentTarget)}>Clear all removable</button>
            </div>
            {state.selectedHandles.length === 0 && <p className="disabled-reason" id="remove-selected-reason">Select at least one removable entry first.</p>}
            {state.outcomes.length > 0 && (
              <ul aria-label="Cache cleanup outcomes" className="outcome-list">
                {state.outcomes.map((outcome) => (
                  <li className={outcome.outcome === "removed" ? "success" : "warning"} key={`${outcome.entryHandle}-${outcome.code}`}>
                    {outcome.outcome === "removed" ? "Success: " : "Attention: "}{outcome.message}
                    <details><summary>Technical details</summary><code>{outcome.code}</code></details>
                  </li>
                ))}
              </ul>
            )}
          </section>
          {state.error && <p className="error" role="alert">Error: {state.error}</p>}
        </>
      )}
    </AccessibleDialog>
  );
}

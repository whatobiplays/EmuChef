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
import type {
  CacheCleanupMode,
  CorrectiveAction,
  ResetLocalStateCategory,
} from "./types";
import { formatBytes, type SupportState } from "./support";

interface CleanupConfirmation {
  entryCount: number;
  totalSizeBytes: number;
}

type SupportConfirmationPayload =
  | {
      kind: "cache";
      mode: CacheCleanupMode;
      confirmation: CleanupConfirmation;
      invoker: HTMLElement | null;
    }
  | {
      kind: "reset";
      category: ResetLocalStateCategory;
      invoker: HTMLElement | null;
    };

interface SupportPanelProps {
  state: SupportState;
  returnFocus: HTMLElement | null;
  onClose: () => void;
  onRefresh: () => void;
  onToggleSelection: (handle: string) => void;
  onPrepareCleanup: (mode: CacheCleanupMode) => CleanupConfirmation | null;
  onCleanup: (mode: CacheCleanupMode) => void;
  onExport: () => void;
  onCorrectiveAction: (action: CorrectiveAction, invoker: HTMLElement) => void;
  onReset: (category: ResetLocalStateCategory) => void;
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
  onCorrectiveAction,
  onReset,
  onAnnounce,
}: SupportPanelProps) {
  const cleanupControllerRef = useRef(new DialogController<SupportConfirmationPayload, boolean>());
  const cleanupController = cleanupControllerRef.current;
  const [cleanupDialog, setCleanupDialog] = useState<DialogSnapshot<SupportConfirmationPayload> | null>(null);
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
    const request = cleanupController.request({ kind: "cache", mode, confirmation, invoker }, false);
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

  const requestReset = async (category: ResetLocalStateCategory, invoker: HTMLElement) => {
    if (!category.available || !category.resetHandle) return;
    const lifecycleGeneration = lifecycleGenerationRef.current;
    const request = cleanupController.request({ kind: "reset", category, invoker }, false);
    if (!request.accepted) {
      onAnnounce("Another support confirmation is already open.");
      return;
    }
    const confirmed = await lifecycleBoundResult(
      request.result,
      false,
      lifecycleGeneration,
      () => lifecycleGenerationRef.current,
    );
    if (confirmed) onReset(category);
    else queueMicrotask(() => restoreAccessibleFocus({ invoker }));
  };

  const copySupportCode = async (code: string) => {
    try {
      await navigator.clipboard.writeText(code);
      onAnnounce(`Support code ${code} copied.`);
    } catch {
      onAnnounce("The support code could not be copied. Select the code and copy it manually.", true);
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
          <p className="eyebrow">Confirm local data change</p>
          <h2 id="cache-cleanup-title">
            {cleanupDialog.payload.kind === "cache" ? "Remove app-owned cache entries?" : `${cleanupDialog.payload.category.label}?`}
          </h2>
          {cleanupDialog.payload.kind === "cache" ? (
            <p id="cache-cleanup-description">
              Remove {cleanupDialog.payload.confirmation.entryCount} cache {cleanupDialog.payload.confirmation.entryCount === 1 ? "entry" : "entries"}
              {" "}totaling {formatBytes(cleanupDialog.payload.confirmation.totalSizeBytes)}. In-use and non-removable entries remain protected.
            </p>
          ) : (
            <div id="cache-cleanup-description">
              <p>{cleanupDialog.payload.category.consequence}</p>
              <p><strong>Affected scope:</strong> {cleanupDialog.payload.category.affectedScope}</p>
            </div>
          )}
          <div className="button-row dialog-actions">
            <button
              className="secondary"
              onClick={() => cleanupController.settle(cleanupDialog.id, false)}
              ref={cleanupCancelRef}
            >Cancel</button>
            <button
              className="danger"
              onClick={() => cleanupController.settle(cleanupDialog.id, true)}
            >{cleanupDialog.payload.kind === "cache" ? "Remove confirmed entries" : "Confirm reset"}</button>
          </div>
        </div>
      ) : (
        <>
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">Troubleshooting</p>
              <h2 id="support-title">Troubleshooting and app storage</h2>
              <p id="support-description">Review current issues, export safe support information, and manage app-owned cache entries.</p>
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

          <section className="support-section" aria-labelledby="troubleshooting-heading">
            <h3 id="troubleshooting-heading">Troubleshooting</h3>
            {state.snapshot ? (
              <>
                <p className={state.snapshot.overallSeverity === "healthy" ? "success" : "warning"} role="status">
                  {state.snapshot.overallSummary}
                </p>
                {state.snapshot.subsystems
                  .filter((subsystem) => subsystem.severity === "warning" || subsystem.severity === "failure")
                  .map((subsystem) => (
                    <article className="cache-entry" key={subsystem.id}>
                      <h4>{subsystem.label}</h4>
                      <p>{subsystem.summary}</p>
                      <p className="fine-print">{subsystem.consequence}</p>
                      {subsystem.supportCode && (
                        <div className="button-row">
                          <code>{subsystem.supportCode}</code>
                          <button className="secondary" onClick={() => void copySupportCode(subsystem.supportCode!)}>
                            Copy support code
                          </button>
                        </div>
                      )}
                      {subsystem.actions.map((entry) => (
                        <div key={`${subsystem.id}-${entry.action.kind}`}>
                          <button
                            aria-describedby={!entry.available ? `${subsystem.id}-${entry.action.kind}-reason` : undefined}
                            className={entry.destructive ? "danger" : "secondary"}
                            disabled={!entry.available}
                            onClick={(event) => onCorrectiveAction(entry.action, event.currentTarget)}
                          >{entry.label}</button>
                          <p className="fine-print">{entry.consequence}</p>
                          {!entry.available && entry.unavailableReason && (
                            <p className="disabled-reason" id={`${subsystem.id}-${entry.action.kind}-reason`}>{entry.unavailableReason}</p>
                          )}
                        </div>
                      ))}
                    </article>
                  ))}
                <details>
                  <summary>View all troubleshooting status and maintenance actions</summary>
                  <ul className="outcome-list">
                    {state.snapshot.subsystems.map((subsystem) => (
                      <li key={`all-${subsystem.id}`}>
                        <strong>{subsystem.label}:</strong> {subsystem.summary}
                        <p className="fine-print">{subsystem.consequence}</p>
                        {(subsystem.severity === "healthy" || subsystem.severity === "neutral") &&
                          subsystem.actions.map((entry) => (
                            <div key={`all-${subsystem.id}-${entry.action.kind}`}>
                              <button
                                className={entry.destructive ? "danger" : "secondary"}
                                disabled={!entry.available}
                                onClick={(event) => onCorrectiveAction(entry.action, event.currentTarget)}
                              >{entry.label}</button>
                              {!entry.available && entry.unavailableReason && <p className="disabled-reason">{entry.unavailableReason}</p>}
                            </div>
                          ))}
                      </li>
                    ))}
                  </ul>
                </details>
              </>
            ) : state.loading ? (
              <p>Checking current troubleshooting status…</p>
            ) : (
              <p className="warning">Troubleshooting status is unavailable. Refresh to try again.</p>
            )}
          </section>

          <section className="support-section" aria-labelledby="support-diagnostics-heading">
            <h3 id="support-diagnostics-heading">Support diagnostics</h3>
            {state.snapshot ? (
              <>
                <p>
                  Export a sanitized local ZIP no larger than {formatBytes(state.snapshot.diagnosticsDisclosure.maximumSizeBytes)}.
                  It stays on this Mac until you choose to share it, and exporting never uploads anything.
                </p>
                <details>
                  <summary>What the archive includes and excludes</summary>
                  <h4>Included</h4>
                  <ul>{state.snapshot.diagnosticsDisclosure.includedCategories.map((item) => <li key={item}>{item}</li>)}</ul>
                  <h4>Excluded</h4>
                  <ul>{state.snapshot.diagnosticsDisclosure.excludedCategories.map((item) => <li key={item}>{item}</li>)}</ul>
                </details>
              </>
            ) : (
              <p>Export a sanitized local ZIP. It excludes paths, serials, credentials, raw logs, and saved-setup contents.</p>
            )}
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
                <h3 id="artifact-cache-heading">App-owned cache</h3>
                <p className="fine-print">Only entries managed by EmuChef can be removed here.</p>
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
              <>
                <p>
                  {inventory.summary.entryCount} entries · {formatBytes(inventory.summary.totalSizeBytes)}
                  {inventory.summary.unmanagedCount > 0 ? ` · ${inventory.summary.unmanagedCount} unmanaged and protected` : ""}
                </p>
                <details>
                  <summary>About cache categories</summary>
                  {inventory.categories.map((category) => (
                    <div key={category.id}>
                      <strong>{category.label}</strong>
                      <p>{category.description} {category.deletionConsequence}</p>
                    </div>
                  ))}
                </details>
              </>
            )}
            <div className="cache-list" role="region" aria-label="App-owned cache entries" tabIndex={0}>
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
                          {entry.categoryLabel} · {formatBytes(entry.sizeBytes)}. {entry.description} {entry.deletionConsequence}
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
              {inventory && inventory.entries.length === 0 && <p className="empty-state">The app-owned cache is empty. Downloaded setup files will appear here when EmuChef needs them.</p>}
              {!inventory && !state.loading && <p className="empty-state">Cache inventory is unavailable. Refresh to try again.</p>}
            </div>
            <div className="button-row">
              <button
                aria-describedby={state.selectedHandles.length === 0 ? "remove-selected-reason" : undefined}
                className="danger"
                disabled={state.cleaning || state.selectedHandles.length === 0}
                onClick={(event) => void requestCleanup("selected", event.currentTarget)}
              >Remove selected</button>
              <button
                aria-describedby={inventory?.summary.unusedRemovableCount === 0 ? "clear-unused-reason" : undefined}
                className="danger"
                disabled={state.cleaning || !inventory || inventory.summary.unusedRemovableCount === 0}
                onClick={(event) => void requestCleanup("unused", event.currentTarget)}
              >Clear unused</button>
              <button
                aria-describedby={inventory?.summary.removableCount === 0 ? "clear-removable-reason" : undefined}
                className="danger"
                disabled={state.cleaning || !inventory || inventory.summary.removableCount === 0}
                onClick={(event) => void requestCleanup("all_removable", event.currentTarget)}
              >Clear all removable</button>
            </div>
            {state.selectedHandles.length === 0 && <p className="disabled-reason" id="remove-selected-reason">Select at least one removable entry first.</p>}
            {inventory?.summary.unusedRemovableCount === 0 && <p className="disabled-reason" id="clear-unused-reason">There are no unused removable cache entries.</p>}
            {inventory?.summary.removableCount === 0 && <p className="disabled-reason" id="clear-removable-reason">There are no removable cache entries.</p>}
            {state.outcomes.length > 0 && (
              <ul aria-label="Cache cleanup outcomes" className="outcome-list">
                {state.outcomes.map((outcome, index) => (
                  <li className={outcome.outcome === "removed" || outcome.outcome === "already_missing" ? "success" : "warning"} key={`${outcome.entryCategory}-${index}`}>
                    {outcome.outcome === "removed" ? "Success: " : "Attention: "}{outcome.message}
                    {outcome.supportCode && (
                      <div className="button-row">
                        <code>{outcome.supportCode}</code>
                        <button className="secondary" onClick={() => void copySupportCode(outcome.supportCode!)}>Copy support code</button>
                      </div>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </section>
          <section className="support-section" aria-labelledby="reset-local-state-heading">
            <h3 id="reset-local-state-heading">Reset local app data</h3>
            <p>Reset one visible app-owned category at a time. Saved setup files, external content, Platform-Tools, and exported files are not included.</p>
            {state.snapshot?.resetCategories.map((category) => (
              <article className="cache-entry" key={category.id}>
                <h4>{category.label}</h4>
                <p>{category.description}</p>
                <p className="fine-print">{category.consequence}</p>
                <p className="fine-print"><strong>Affected scope:</strong> {category.affectedScope}</p>
                <button
                  aria-describedby={!category.available ? `reset-${category.id}-reason` : undefined}
                  className="danger"
                  disabled={!category.available || state.cleaning || state.exporting}
                  onClick={(event) => void requestReset(category, event.currentTarget)}
                >{category.label}</button>
                {!category.available && category.unavailableReason && (
                  <p className="disabled-reason" id={`reset-${category.id}-reason`}>{category.unavailableReason}</p>
                )}
              </article>
            ))}
          </section>
          {state.error && <p className="error" role="alert">Error: {state.error}</p>}
        </>
      )}
    </AccessibleDialog>
  );
}

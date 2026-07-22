import { useEffect, useRef } from "react";

import { AccessibleDialog } from "./AccessibleDialog";
import { formatUpdateBytes, type UpdatePanelState } from "./update-policy";

interface UpdatesPanelProps {
  state: UpdatePanelState;
  returnFocus: HTMLElement | null;
  navigationBlocked: boolean;
  onClose: () => void;
  onCheck: () => void;
  onOpenDownload: () => void;
  onAnnounce: (message: string, assertive?: boolean) => void;
}

/** Accessible, display-only update UI with no external-navigation parameters. */
export function UpdatesPanel({
  state,
  returnFocus,
  navigationBlocked,
  onClose,
  onCheck,
  onOpenDownload,
  onAnnounce,
}: UpdatesPanelProps) {
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (state.status?.message) {
      onAnnounce(state.status.message, state.status.state === "failed");
    }
  }, [onAnnounce, state.status?.message, state.status?.state]);

  if (!state.open) return null;
  const status = state.status;
  const available = status?.state === "update_available";
  const openDisabled = !available || !status.canOpenDownload || navigationBlocked || state.opening;
  const closeBlocked = state.checking || state.opening;

  return (
    <AccessibleDialog
      titleId="updates-title"
      descriptionId="updates-description"
      initialFocusRef={closeRef}
      returnFocus={returnFocus}
      dismissible={!closeBlocked}
      onDismiss={onClose}
      onDismissBlocked={() => onAnnounce("Updates cannot close until the current update action finishes.", true)}
    >
      <header className="dialog-heading">
        <div>
          <p className="eyebrow">Update status</p>
          <h2 id="updates-title">EmuChef updates</h2>
        </div>
        <button
          aria-describedby={closeBlocked ? "updates-close-reason" : undefined}
          className="secondary"
          disabled={closeBlocked}
          onClick={onClose}
          ref={closeRef}
        >Close</button>
      </header>
      <p id="updates-description">
        EmuChef checks verified release information only when you ask. It never downloads, installs,
        replaces, or restarts the app.
      </p>
      {closeBlocked && <p className="disabled-reason" id="updates-close-reason" role="status">Close is unavailable while the current update action finishes.</p>}
      <dl className="update-summary">
        <div><dt>Current version</dt><dd>{status?.currentVersion ?? "Loading…"}</dd></div>
        {status?.latestVersion && <div><dt>Latest version</dt><dd>{status.latestVersion}</dd></div>}
        {status?.dmgSizeBytes && <div><dt>Download size</dt><dd>{formatUpdateBytes(status.dmgSizeBytes)}</dd></div>}
      </dl>
      {status?.notes && <section aria-labelledby="update-notes-title"><h3 id="update-notes-title">Release notes</h3><p className="release-notes">{status.notes}</p></section>}
      {status?.minimumMacosVersion && (
        <p className="warning">Signed metadata lists macOS {status.minimumMacosVersion} or newer. This is informational; EmuChef does not inspect the local macOS version.</p>
      )}
      {status?.message && <p className={status.state === "failed" ? "error" : "fine-print"} role={status.state === "failed" ? "alert" : "status"}>{status.message}</p>}
      <section aria-labelledby="manual-install-title">
        <h3 id="manual-install-title">Install the update manually</h3>
        <ol>
          <li>Open the verified download page in your default browser.</li>
          <li>After the browser downloads the disk image, open it and drag EmuChef to Applications.</li>
          <li>Replace the existing copy, then relaunch EmuChef manually.</li>
        </ol>
        <p className="warning">
          EmuChef verifies the release information before opening the download page. Your browser
          downloads the installer, and macOS checks the app when you open it.
        </p>
      </section>
      <div className="button-row dialog-actions">
        <button className="secondary" disabled={state.checking || state.opening} onClick={onCheck}>
          {state.checking ? "Checking…" : "Check for updates"}
        </button>
        <button disabled={openDisabled} onClick={onOpenDownload}>
          {state.opening ? "Opening…" : "Open download page"}
        </button>
      </div>
      {openDisabled && available && <p className="disabled-reason">Close other dialogs and wait for current work to finish before opening the browser.</p>}
    </AccessibleDialog>
  );
}

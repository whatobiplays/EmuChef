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

  return (
    <AccessibleDialog
      titleId="updates-title"
      descriptionId="updates-description"
      initialFocusRef={closeRef}
      returnFocus={returnFocus}
      onDismiss={onClose}
    >
      <header className="modal-header">
        <div>
          <p className="eyebrow">MANUAL UPDATE</p>
          <h2 id="updates-title">EmuChef Updates</h2>
        </div>
        <button className="text-button" onClick={onClose} ref={closeRef}>Close</button>
      </header>
      <p id="updates-description">
        EmuChef checks signed release metadata only when you ask. It never downloads, installs,
        replaces, or restarts the application.
      </p>
      <dl className="update-summary">
        <div><dt>Current version</dt><dd>{status?.currentVersion ?? "Loading…"}</dd></div>
        {status?.latestVersion && <div><dt>Latest version</dt><dd>{status.latestVersion}</dd></div>}
        {status?.dmgSizeBytes && <div><dt>DMG size</dt><dd>{formatUpdateBytes(status.dmgSizeBytes)}</dd></div>}
        {status?.dmgSha256 && <div><dt>Release SHA-256</dt><dd><code>{status.dmgSha256}</code></dd></div>}
      </dl>
      {status?.notes && <section aria-labelledby="update-notes-title"><h3 id="update-notes-title">Release notes</h3><p className="release-notes">{status.notes}</p></section>}
      {status?.minimumMacosVersion && (
        <p className="warning">Signed metadata lists macOS {status.minimumMacosVersion} or newer. This is informational; EmuChef does not inspect the local macOS version.</p>
      )}
      {status?.message && <p className={status.state === "failed" ? "error" : "fine-print"} role={status.state === "failed" ? "alert" : "status"}>{status.message}</p>}
      <section aria-labelledby="manual-install-title">
        <h3 id="manual-install-title">Manual replacement</h3>
        <ol>
          <li>Open the validated DMG address in your default browser.</li>
          <li>After the browser downloads it, open the DMG and drag EmuChef.app to Applications.</li>
          <li>Replace the existing copy, then relaunch EmuChef manually.</li>
        </ol>
        <p className="warning">
          EmuChef verified the signed release metadata. The browser performs the download, and
          EmuChef does not inspect or verify the local DMG. Developer ID signing, notarization,
          stapling, and Gatekeeper remain the executable trust controls when you open it.
        </p>
      </section>
      <div className="button-row">
        <button className="secondary" disabled={state.checking || state.opening} onClick={onCheck}>
          {state.checking ? "Checking…" : "Check for Updates"}
        </button>
        <button disabled={openDisabled} onClick={onOpenDownload}>
          {state.opening ? "Opening…" : "Open DMG Download in Browser"}
        </button>
      </div>
      {openDisabled && available && <p className="disabled-reason">Close other dialogs and wait for current work to finish before opening the browser.</p>}
    </AccessibleDialog>
  );
}

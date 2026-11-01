import { useEffect, useId, useRef } from "react";

import { describedBy } from "./accessibility";
import type {
  DeviceVolumeChoice,
  DeviceVolumeDiscoveryStatus,
  DeviceVolumesProjection,
} from "./types";

/**
 * Storage option presentation for the selected-device setup step.
 *
 * The component renders only what the trusted projection contains. It never
 * derives availability, never selects a destination on its own, and never
 * implies write access, application-side access, or execution eligibility.
 * Mounted storage is reported as mounted; EmuChef verifies nothing beyond the
 * passive device report.
 *
 * Radio accessible names contain only the option label. Mount and status
 * details are exposed through aria-describedby. Focus tracking follows actual
 * radio focus, including keyboard focus before selection, and focus restoration
 * runs only when the same option remains an appropriate target.
 */
export interface DeviceStorageChoicesProps {
  /** Trusted projection, or null before the first observation completes. */
  projection: DeviceVolumesProjection | null;
  /** True while a fresh observation is in progress. */
  loading: boolean;
  /** Fixed product error text from the trusted bridge, if any. */
  error: string | null;
  /** Request a fresh observation, bypassing the short-lived cache. */
  onRefresh: () => void;
  /** Record the user's explicit choice with the displayed observation token. */
  onSelect: (volumeHandle: string, observationToken: string) => void;
  /** Announce a focused-option loss through the app shell live region. */
  announce: (message: string, assertive?: boolean) => void;
}

function discoveryMessage(status: DeviceVolumeDiscoveryStatus): string | null {
  switch (status) {
    case "noDevice":
      return "No connected device is available for storage discovery.";
    case "multipleDevices":
      return "Connect only the intended device before reading storage options.";
    case "unauthorized":
      return "Accept the USB debugging prompt on the device, then refresh storage options.";
    case "offline":
      return "This device is offline. Reconnect it, then refresh storage options.";
    case "refreshing":
      return "Reading storage options from the device.";
    case "unavailable":
      return "Storage options could not be read. Refresh storage options to try again.";
    case "unknown":
      return "Storage options could not be read. Refresh to try again.";
    case "observed":
      return null;
  }
}

function evidenceText(projection: DeviceVolumesProjection | null): string {
  if (!projection) return "Storage options have not been read yet.";
  if (projection.discoveryStatus === "refreshing" && projection.evidence === "unavailable") {
    return "Storage options have not been read yet.";
  }
  if (projection.discoveryStatus === "unavailable" || projection.evidence === "unavailable") {
    return "Storage options could not be read. Refresh storage options to try again.";
  }
  return projection.evidence === "live"
    ? "Showing a storage report read from the device just now."
    : "Showing a recent storage report. Refresh to read the device again.";
}

/**
 * Render storage choices for one selected device.
 *
 * Only currently available internal or portable storage is offered as a radio
 * control. Every other observed volume is listed with its explanation so an
 * unsupported or unavailable destination is visible without becoming
 * selectable.
 */
export function DeviceStorageChoices({
  projection,
  loading,
  error,
  onRefresh,
  onSelect,
  announce,
}: DeviceStorageChoicesProps) {
  const baseId = useId();
  const optionListId = baseId + "-options";
  const evidenceId = baseId + "-evidence";
  const unavailableId = baseId + "-unavailable";
  const loadingId = baseId + "-loading";
  const noticeId = baseId + "-notice";
  const errorId = baseId + "-error";
  const groupName = baseId + "-storage";

  const radioRefs = useRef(new Map<string, HTMLInputElement>());
  const focusedHandleRef = useRef<string | null>(null);
  const shouldRestoreFocusRef = useRef(false);
  const refreshButtonRef = useRef<HTMLButtonElement | null>(null);
  const wasLoadingRef = useRef(loading);

  const notice = projection?.selectionNotice ?? null;

  useEffect(() => {
    const wasLoading = wasLoadingRef.current;
    wasLoadingRef.current = loading;
    if (!wasLoading && loading) {
      const handle = focusedHandleRef.current;
      const element = handle ? radioRefs.current.get(handle) : undefined;
      shouldRestoreFocusRef.current = Boolean(element && document.activeElement === element);
      return;
    }
    if (!wasLoading || loading) return;
    const hadFocus = shouldRestoreFocusRef.current;
    shouldRestoreFocusRef.current = false;
    const handle = focusedHandleRef.current;
    if (!handle || !hadFocus) return;
    const element = radioRefs.current.get(handle);
    const active = document.activeElement;
    if (element && element.isConnected) {
      if (active === null || active === document.body || active === element) element.focus();
      return;
    }
    focusedHandleRef.current = null;
    if (active !== null && active !== document.body && active !== refreshButtonRef.current) {
      return;
    }
    refreshButtonRef.current?.focus();
    if (!error && !notice) {
      announce(
        "That storage option is no longer available after refresh. Choose another option or refresh again.",
        true,
      );
    }
  }, [announce, error, loading, notice]);

  const selectable = projection?.volumes.filter((volume) => volume.selectable) ?? [];
  const unavailable = projection?.volumes.filter((volume) => !volume.selectable) ?? [];
  const statusMessage = projection ? discoveryMessage(projection.discoveryStatus) : null;
  const observationToken = projection?.observationToken ?? "";
  const descriptionIds = describedBy(
    evidenceId,
    loading ? loadingId : undefined,
    notice && !error ? noticeId : undefined,
    error ? errorId : undefined,
  );

  return (
    <fieldset className="storage-options" aria-describedby={descriptionIds}>
      <legend>Storage options</legend>
      <p className="eyebrow">Where EmuChef may place content later</p>
      <p id={evidenceId} className="storage-evidence">
        {evidenceText(projection)}
      </p>
      {error ? (
        <p id={errorId} className="error" role="alert">
          {error}
        </p>
      ) : (
        statusMessage && (
          <p className="warning" role="status">
            {statusMessage}
          </p>
        )
      )}
      {loading && (
        <p id={loadingId} className="disabled-reason">
          Refreshing storage options…
        </p>
      )}
      {projection?.discoveryStatus === "observed" && selectable.length === 0 && (
        <p className="disabled-reason">
          No selectable storage is available right now. EmuChef has not chosen a storage option for you.
        </p>
      )}
      {selectable.length > 0 && (
        <div id={optionListId} className="storage-option-list">
          {selectable.map((volume) => {
            const detailId = baseId + "-" + volume.presentationKey;
            return (
              <StorageRadio
                key={volume.presentationKey}
                volume={volume}
                groupName={groupName}
                checked={projection?.selectedVolumeHandle === volume.volumeHandle}
                describedBy={describedBy(detailId, notice && !error ? noticeId : undefined)}
                detailId={detailId}
                onFocus={() => {
                  focusedHandleRef.current = volume.volumeHandle;
                  if (loading) shouldRestoreFocusRef.current = true;
                }}
                onSelect={() => onSelect(volume.volumeHandle, observationToken)}
                register={(element) => {
                  if (element) radioRefs.current.set(volume.volumeHandle, element);
                  else radioRefs.current.delete(volume.volumeHandle);
                }}
              />
            );
          })}
        </div>
      )}
      {unavailable.length > 0 && (
        <div className="storage-unavailable">
          <p id={unavailableId} className="storage-unavailable-heading">
            Not offered as a destination
          </p>
          <ul aria-labelledby={unavailableId}>
            {unavailable.map((volume) => (
              <li key={volume.presentationKey} className="unavailable">
                <strong>{volume.label}</strong>
                <small>{volume.detail}</small>
              </li>
            ))}
          </ul>
        </div>
      )}
      {notice && !error && (
        <p id={noticeId} className="warning" role="status">
          {notice}
        </p>
      )}
      <button
        ref={refreshButtonRef}
        type="button"
        className="text-button"
        disabled={loading}
        aria-describedby={loading ? loadingId : undefined}
        onClick={onRefresh}
      >
        {loading ? "Refreshing storage…" : "Refresh storage"}
      </button>
      <p className="disabled-reason">
        Storage options are read-only evidence. A mount does not prove EmuChef can write to the storage, and a
        selection here does not authorize execution.
      </p>
    </fieldset>
  );
}

function StorageRadio({
  volume,
  groupName,
  checked,
  describedBy,
  onSelect,
  onFocus,
  register,
  detailId,
}: {
  volume: DeviceVolumeChoice;
  groupName: string;
  checked: boolean;
  describedBy: string | undefined;
  onSelect: () => void;
  onFocus: () => void;
  register: (element: HTMLInputElement | null) => void;
  detailId: string;
}) {
  return (
    <label>
      <input
        ref={register}
        type="radio"
        name={groupName}
        value={volume.volumeHandle}
        checked={checked}
        aria-label={volume.label}
        aria-describedby={describedBy}
        onFocus={onFocus}
        onChange={onSelect}
      />
      <span>
        <strong>{volume.label}</strong>
        <small id={detailId}>{volume.detail}</small>
      </span>
    </label>
  );
}

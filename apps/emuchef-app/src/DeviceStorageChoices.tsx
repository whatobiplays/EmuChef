import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { describedBy } from "./accessibility";
import type {
  DeviceVolumeChoice,
  DeviceVolumeDiscoveryStatus,
  DeviceVolumesProjection,
  SetupLocationProjection,
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
  /** True while a storage read or selection request is in progress. */
  loading: boolean;
  /** The storage operation currently keeping the panel busy. */
  loadingKind?: DeviceStorageLoadingKind;
  /** Fixed product error text from the trusted bridge, if any. */
  error: string | null;
  /** Request a fresh observation, bypassing the short-lived cache. */
  onRefresh: () => void;
  /** Record the user's explicit choice with the displayed observation token. */
  onSelect: (volumeHandle: string, observationToken: string) => void;
  /** Announce a focused-option loss through the app shell live region. */
  announce: (message: string, assertive?: boolean) => void;
  /** Current device-plan context for its independent named locations. */
  devicePlan?: string | null;
  /** Sanitized named locations for the current device-plan context. */
  setupLocations?: SetupLocationProjection[];
  /** True while listing or mutating named locations. */
  setupLocationLoading?: boolean;
  /** Fixed product error text from the trusted bridge, if any. */
  setupLocationError?: string | null;
  /** Save a location using a volume and observation from the current projection. */
  onSaveSetupLocation?: (
    volumeHandle: string,
    observationToken: string,
    existingId: string | undefined,
    name: string,
    relativePath: string,
  ) => Promise<boolean>;
  /** Remove a named location from the current device-plan context. */
  onRemoveSetupLocation?: (id: string) => void | Promise<void>;
}

export type DeviceStorageLoadingKind = "refresh" | "selection";

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
  if (
    projection.discoveryStatus === "refreshing" &&
    projection.evidence === "unavailable"
  ) {
    return "Storage options have not been read yet.";
  }
  if (
    projection.discoveryStatus === "unavailable" ||
    projection.evidence === "unavailable"
  ) {
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
  loadingKind = "refresh",
  error,
  onRefresh,
  onSelect,
  announce,
  devicePlan = null,
  setupLocations = [],
  setupLocationLoading = false,
  setupLocationError = null,
  onSaveSetupLocation,
  onRemoveSetupLocation,
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
  const [setupLocationName, setSetupLocationName] = useState("");
  const [setupLocationPath, setSetupLocationPath] = useState("");
  const [setupLocationVolumeHandle, setSetupLocationVolumeHandle] =
    useState("");
  const [editingSetupLocationId, setEditingSetupLocationId] = useState<
    string | undefined
  >();
  const wasLoadingRef = useRef(loading);
  const setupLocationContextRef = useRef({
    devicePlan,
    deviceHandle: projection?.deviceHandle ?? null,
    sessionEpoch: projection?.sessionEpoch ?? null,
    hasFreshObservation: projection?.discoveryStatus === "observed"
      && projection.evidence === "live"
      && Boolean(projection.observationToken),
  });

  const notice = projection?.selectionNotice ?? null;

  useEffect(() => {
    const previous = setupLocationContextRef.current;
    const currentDeviceHandle = projection?.deviceHandle ?? previous.deviceHandle;
    const currentSessionEpoch = projection?.sessionEpoch ?? previous.sessionEpoch;
    const currentHasFreshObservation = projection?.discoveryStatus === "observed"
      && projection.evidence === "live"
      && Boolean(projection.observationToken);
    const deviceContextChanged = previous.deviceHandle !== null
      && (currentDeviceHandle !== previous.deviceHandle
        || currentSessionEpoch !== previous.sessionEpoch);
    const observationLost = previous.deviceHandle !== null
      && (projection?.deviceHandle == null
        || (previous.hasFreshObservation && !currentHasFreshObservation));

    if (previous.devicePlan !== devicePlan || deviceContextChanged || observationLost) {
      setSetupLocationName("");
      setSetupLocationPath("");
      setSetupLocationVolumeHandle("");
      setEditingSetupLocationId(undefined);
    }

    setupLocationContextRef.current = {
      devicePlan,
      deviceHandle: currentDeviceHandle,
      sessionEpoch: currentSessionEpoch,
      hasFreshObservation: currentHasFreshObservation,
    };
  }, [
    devicePlan,
    projection?.deviceHandle,
    projection?.sessionEpoch,
    projection?.discoveryStatus,
    projection?.evidence,
    projection?.observationToken,
  ]);

  useEffect(() => {
    const wasLoading = wasLoadingRef.current;
    wasLoadingRef.current = loading;
    if (!wasLoading && loading) {
      const handle = focusedHandleRef.current;
      const element = handle ? radioRefs.current.get(handle) : undefined;
      shouldRestoreFocusRef.current = Boolean(
        element && document.activeElement === element,
      );
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
      if (active === null || active === document.body || active === element)
        element.focus();
      return;
    }
    focusedHandleRef.current = null;
    if (
      active !== null &&
      active !== document.body &&
      active !== refreshButtonRef.current
    ) {
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

  const selectable =
    projection?.volumes.filter((volume) => volume.selectable) ?? [];
  const freshObservation =
    projection?.discoveryStatus === "observed" &&
    projection.evidence === "live";
  const setupLocationVolume = freshObservation
    ? selectable.find(
        (volume) => volume.volumeHandle === setupLocationVolumeHandle,
      )
    : undefined;
  const canSaveSetupLocation = Boolean(
    devicePlan &&
      freshObservation &&
      setupLocationVolume &&
      setupLocationName.trim() &&
      setupLocationPath.trim() &&
      !loading &&
      !setupLocationLoading &&
      onSaveSetupLocation,
  );
  const unavailable =
    projection?.volumes.filter((volume) => !volume.selectable) ?? [];
  const statusMessage = projection
    ? discoveryMessage(projection.discoveryStatus)
    : null;
  const observationToken = projection?.observationToken ?? "";
  const loadingMessage =
    loadingKind === "selection"
      ? "Selecting storage…"
      : "Refreshing storage options…";
  const refreshButtonText = loading
    ? loadingKind === "selection"
      ? "Selecting storage…"
      : "Refreshing storage…"
    : "Refresh storage";
  const descriptionIds = describedBy(
    evidenceId,
    loading ? loadingId : undefined,
    notice && !error ? noticeId : undefined,
    error ? errorId : undefined,
  );
  const resetSetupLocationForm = () => {
    setSetupLocationName("");
    setSetupLocationPath("");
    setSetupLocationVolumeHandle("");
    setEditingSetupLocationId(undefined);
  };
  useEffect(() => {
    if (
      editingSetupLocationId &&
      !setupLocations.some((location) => location.id === editingSetupLocationId)
    ) {
      setSetupLocationName("");
      setSetupLocationPath("");
      setSetupLocationVolumeHandle("");
      setEditingSetupLocationId(undefined);
    }
  }, [editingSetupLocationId, setupLocations]);

  const submitSetupLocation = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (
      !canSaveSetupLocation ||
      !projection ||
      !setupLocationVolume ||
      !onSaveSetupLocation
    )
      return;
    const saved = await onSaveSetupLocation(
      setupLocationVolume.volumeHandle,
      projection.observationToken,
      editingSetupLocationId,
      setupLocationName.trim(),
      setupLocationPath.trim(),
    );
    if (saved) resetSetupLocationForm();
  };

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
          {loadingMessage}
        </p>
      )}
      {projection?.discoveryStatus === "observed" &&
        selectable.length === 0 && (
          <p className="disabled-reason">
            No selectable storage is available right now. EmuChef has not chosen
            a storage option for you.
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
                checked={
                  projection?.selectedVolumeHandle === volume.volumeHandle
                }
                submissionBlocked={loading}
                describedBy={describedBy(
                  detailId,
                  notice && !error ? noticeId : undefined,
                )}
                detailId={detailId}
                onFocus={() => {
                  focusedHandleRef.current = volume.volumeHandle;
                  if (loading) shouldRestoreFocusRef.current = true;
                }}
                onSelect={() => onSelect(volume.volumeHandle, observationToken)}
                register={(element) => {
                  if (element)
                    radioRefs.current.set(volume.volumeHandle, element);
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
        {refreshButtonText}
      </button>
      <p className="disabled-reason">
        Storage options are read-only evidence. A mount does not prove EmuChef
        can write to the storage, and a selection here does not authorize
        execution.
      </p>
      <fieldset className="setup-locations">
        <legend>Named Setup Locations</legend>
        <p>Save a named folder destination for recipes in this device plan.</p>
        {setupLocationError && (
          <p className="error" role="alert">
            {setupLocationError}
          </p>
        )}
        {!devicePlan && (
          <p className="disabled-reason">
            Choose a device plan before saving Setup Locations.
          </p>
        )}
        {devicePlan && !freshObservation && (
          <p className="disabled-reason">
            Refresh storage options before adding or updating a Setup Location.
          </p>
        )}
        {devicePlan && freshObservation && selectable.length === 0 && (
          <p className="disabled-reason">
            No selectable storage volume is available in this report.
          </p>
        )}
        {setupLocations.length > 0 && (
          <ul
            aria-label="Saved Setup Locations"
            className="setup-location-list"
          >
            {setupLocations.map((location) => (
              <li key={location.id}>
                <span>
                  <strong>{location.name}</strong>
                  <small>
                    {location.relativePath} · {location.volumeLabel}
                  </small>
                  <small className={location.ready ? undefined : "warning"}>
                    {location.ready
                      ? "Ready for recipe selection."
                      : location.detail}
                  </small>
                </span>
                <span className="input-entry-actions">
                  <button
                    className="secondary"
                    disabled={setupLocationLoading || !onSaveSetupLocation}
                    onClick={() => {
                      setEditingSetupLocationId(location.id);
                      setSetupLocationName(location.name);
                      setSetupLocationPath(location.relativePath);
                      setSetupLocationVolumeHandle("");
                    }}
                    type="button"
                  >
                    Edit {location.name}
                  </button>
                  <button
                    className="secondary"
                    disabled={setupLocationLoading || !onRemoveSetupLocation}
                    onClick={() => void onRemoveSetupLocation?.(location.id)}
                    type="button"
                  >
                    Remove {location.name}
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}
        <form onSubmit={(event) => void submitSetupLocation(event)}>
          <label className="input-field">
            Storage volume for Setup Location
            <select
              disabled={
                !devicePlan ||
                !freshObservation ||
                loading ||
                setupLocationLoading
              }
              onChange={(event) =>
                setSetupLocationVolumeHandle(event.target.value)
              }
              value={setupLocationVolumeHandle}
            >
              <option value="">Choose storage volume…</option>
              {selectable.map((volume) => (
                <option
                  key={volume.presentationKey}
                  value={volume.volumeHandle}
                >
                  {volume.label}
                </option>
              ))}
            </select>
          </label>
          <label className="input-field">
            Setup Location name
            <input
              autoComplete="off"
              disabled={!devicePlan || setupLocationLoading}
              onChange={(event) => setSetupLocationName(event.target.value)}
              value={setupLocationName}
            />
          </label>
          <label className="input-field">
            Relative folder
            <input
              autoComplete="off"
              disabled={!devicePlan || setupLocationLoading}
              onChange={(event) => setSetupLocationPath(event.target.value)}
              value={setupLocationPath}
            />
          </label>
          <div className="button-row">
            <button disabled={!canSaveSetupLocation} type="submit">
              {editingSetupLocationId
                ? "Update Setup Location"
                : "Save Setup Location"}
            </button>
            {editingSetupLocationId && (
              <button
                className="secondary"
                onClick={resetSetupLocationForm}
                type="button"
              >
                Cancel edit
              </button>
            )}
          </div>
        </form>
      </fieldset>
    </fieldset>
  );
}

function StorageRadio({
  volume,
  groupName,
  checked,
  submissionBlocked,
  describedBy,
  onSelect,
  onFocus,
  register,
  detailId,
}: {
  volume: DeviceVolumeChoice;
  groupName: string;
  checked: boolean;
  submissionBlocked: boolean;
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
        aria-disabled={submissionBlocked || undefined}
        aria-label={volume.label}
        aria-describedby={describedBy}
        onFocus={onFocus}
        onChange={() => {
          if (!submissionBlocked) onSelect();
        }}
      />
      <span>
        <strong>{volume.label}</strong>
        <small id={detailId}>{volume.detail}</small>
      </span>
    </label>
  );
}

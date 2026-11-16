import { useState } from "react";
import type { DeviceFacts, DeviceMatch } from "./types";

/** A candidate button confirms a profile. Tabbing only moves focus. */
export function DeviceProfileSelection({ match, facts, busy, onSelect, onRetry, onChooseDevice }: {
  match: DeviceMatch; facts: DeviceFacts; busy: boolean;
  onRetry?: () => void;
  onChooseDevice?: () => void;
  onSelect: (profileId: string, genericAcknowledged: boolean) => void;
}) {
  const [genericAcknowledged, setGenericAcknowledged] = useState(false);
  const eligible = (match.profiles ?? []).filter((profile) => profile.eligibility === "eligible");
  const incomplete = match.resolutionStatus === "incomplete";
  return <section aria-labelledby="profile-heading">
    <h2 id="profile-heading" data-step-heading tabIndex={-1}>Choose Device Profile</h2>
    <p>{facts.manufacturer ?? "Unknown manufacturer"} · {facts.model ?? "Unknown model"} · Android {facts.androidVersion ?? "unknown"}</p>
    <p>Android reports compatibility facts that can be shared by different products. Choose the profile for the device you connected.</p>
    {match.profileSelectionNotice && <p role="status">{match.profileSelectionNotice}</p>}
    {incomplete && <p role="status">Device identification is incomplete. Read device information again before choosing a model profile.</p>}
    {onChooseDevice && <button type="button" disabled={busy || match.profileChangeAllowed !== true} onClick={onChooseDevice}>Choose another device</button>}
    {onRetry && <button type="button" disabled={busy || match.profileChangeAllowed !== true} onClick={onRetry}>Read device information again</button>}
    {match.profileChangeAllowed !== true && <p role="status">{match.profileChangeReason ?? "Profile selection is unavailable while execution state is unresolved."}</p>}
    {eligible.some((profile) => profile.generic) && <label><input type="checkbox" checked={genericAcknowledged} onChange={(event) => setGenericAcknowledged(event.target.checked)} />I understand a generic profile provides only the capabilities independently established for this device.</label>}
    {eligible.map((profile) => <div key={profile.profileId}>
      <button type="button" disabled={busy || !match.resolutionHandle || match.profileChangeAllowed !== true || (profile.generic ? !genericAcknowledged : incomplete)} onClick={() => onSelect(profile.profileId, profile.generic && genericAcknowledged)}>{profile.profileName}</button>
      {profile.profileDescription && <p>{profile.profileDescription}</p>}
      <p>{profile.reasons.join(" ")}</p>
      {profile.plans.length === 0 && <p>No setup is available for this profile.</p>}
    </div>)}
  </section>;
}

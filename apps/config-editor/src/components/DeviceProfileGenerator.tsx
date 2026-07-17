import { useEffect, useReducer, useRef } from "react";

import {
  beginDeviceProfileGenerator,
  cancelDeviceProfileGenerator,
  checkDeviceProfileCollisions,
  chooseDeviceProfileAuthoredRoot,
  setDeviceProfileAuthoredRoot,
  generateDeviceProfileDraft,
  listDeviceProfileGeneratorDevices,
  probeDeviceProfileGeneratorDevice,
  saveGeneratedDeviceProfile,
  type EditorApiResult,
} from "../api/editorApi";
import type {
  DeviceProfileDraftResult,
  DeviceProfileEvidenceState,
  DeviceProfileFieldEvidenceDto,
  SafeDetectedDeviceFactsDto,
} from "../api/types";
import {
  formToProfile,
  initialDeviceProfileGeneratorState,
  reduceDeviceProfileGenerator,
  type DeviceProfileFormState,
} from "./deviceProfileGenerator.logic";

interface DeviceProfileGeneratorProps {
  initialAuthoredRoot: string | null;
  onAuthoredRootSelected: (path: string) => void | Promise<void>;
  onClose: () => void;
  onSaved: (displayPath: string) => void;
}

const capabilityLabels: Array<[
  keyof DeviceProfileFormState["capabilities"],
  string,
]> = [
  ["adb_available", "ADB available"],
  ["apk_install", "APK install"],
  ["shared_storage_write", "Shared-storage write"],
  ["app_launch", "App launch"],
  ["shell_command", "Shell command"],
  ["package_remove_for_user", "Remove package for user"],
  ["root_shell", "Root shell"],
  ["app_data_write", "App-data write"],
];

function isSelectableDeviceState(state: string): boolean {
  return state === "available" || state === "device";
}

/** Focused ephemeral wizard for generating one new authored device profile. */
export function DeviceProfileGenerator({ initialAuthoredRoot, onAuthoredRootSelected, onClose, onSaved }: DeviceProfileGeneratorProps) {
  const [state, dispatch] = useReducer(
    reduceDeviceProfileGenerator,
    initialDeviceProfileGeneratorState,
  );
  const sessionRef = useRef<string | null>(null);
  const initialAuthoredRootRef = useRef(initialAuthoredRoot);

  useEffect(() => {
    let disposed = false;
    void beginDeviceProfileGenerator().then(async (response) => {
      if (response.kind !== "success") {
        if (!disposed) dispatch({ type: "failed", message: apiFailure(response) });
        return;
      }
      const handle = response.result.sessionHandle;
      if (disposed) {
        await cancelDeviceProfileGenerator(handle);
        return;
      }
      sessionRef.current = handle;
      dispatch({ type: "sessionStarted", sessionHandle: handle });
      if (initialAuthoredRootRef.current) {
        const root = await setDeviceProfileAuthoredRoot(handle, initialAuthoredRootRef.current);
        if (disposed) return;
        if (root.kind === "success" && root.result.rootHandle && root.result.label) {
          dispatch({ type: "rootSelected", rootHandle: root.result.rootHandle, rootLabel: root.result.label });
        }
      }
      dispatch({ type: "devicesLoading" });
      const devices = await listDeviceProfileGeneratorDevices(handle);
      if (disposed) return;
      if (devices.kind === "success") {
        dispatch({ type: "devicesLoaded", devices: devices.result.devices });
      } else {
        dispatch({ type: "failed", message: apiFailure(devices) });
      }
    });
    return () => {
      disposed = true;
      const handle = sessionRef.current;
      sessionRef.current = null;
      if (handle) void cancelDeviceProfileGenerator(handle);
    };
  }, []);

  async function refreshDevices(sessionHandle = state.sessionHandle) {
    if (!sessionHandle) return;
    dispatch({ type: "devicesLoading" });
    const response = await listDeviceProfileGeneratorDevices(sessionHandle);
    if (response.kind === "success") {
      dispatch({ type: "devicesLoaded", devices: response.result.devices });
    } else {
      dispatch({ type: "failed", message: apiFailure(response) });
    }
  }

  async function probeSelectedDevice() {
    if (!state.sessionHandle || !state.selectedDeviceHandle) return;
    dispatch({ type: "saveStarted" });
    const probed = await probeDeviceProfileGeneratorDevice(
      state.sessionHandle,
      state.selectedDeviceHandle,
    );
    if (probed.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(probed) });
      return;
    }
    const generated = await generateDeviceProfileDraft(
      state.sessionHandle,
      state.selectedDeviceHandle,
    );
    if (generated.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(generated) });
      return;
    }
    dispatch({
      type: "probeLoaded",
      facts: probed.result.facts,
      draft: generated.result,
    });
  }

  async function selectRoot(): Promise<string | null> {
    if (!state.sessionHandle) return null;
    const response = await chooseDeviceProfileAuthoredRoot(state.sessionHandle);
    if (response.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(response) });
      return null;
    }
    if (response.result.cancelled) return null;
    const { rootHandle, label } = response.result;
    if (!rootHandle || !label) {
      dispatch({ type: "failed", message: "Authored-root selection returned incomplete data." });
      return null;
    }
    dispatch({ type: "rootSelected", rootHandle, rootLabel: label });
    if (response.result.path) await onAuthoredRootSelected(response.result.path);
    return rootHandle;
  }

  async function reviewProfile() {
    if (!state.sessionHandle || !state.selectedDeviceHandle || !state.form) return;
    const converted = formToProfile(state.form);
    if (!converted.ok) {
      dispatch({ type: "failed", message: converted.message });
      return;
    }
    dispatch({ type: "saveStarted" });
    const generated = await generateDeviceProfileDraft(
      state.sessionHandle,
      state.selectedDeviceHandle,
      converted.profile,
    );
    if (generated.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(generated) });
      return;
    }
    if (!generated.result.canonicalYaml) {
      dispatch({
        type: "draftInvalid",
        draft: generated.result,
        message: "Resolve the validation errors before reviewing the profile.",
      });
      return;
    }
    const rootHandle = state.rootHandle ?? (await selectRoot());
    if (!rootHandle) {
      dispatch({ type: "failed", message: "Select an authored root before reviewing collisions." });
      return;
    }
    const collisions = await checkDeviceProfileCollisions(
      state.sessionHandle,
      state.selectedDeviceHandle,
      rootHandle,
      generated.result.profile,
    );
    if (collisions.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(collisions) });
      return;
    }
    dispatch({ type: "reviewLoaded", draft: generated.result, collisions: collisions.result });
  }

  async function saveProfile() {
    if (
      !state.sessionHandle ||
      !state.selectedDeviceHandle ||
      !state.rootHandle ||
      !state.draft
    ) {
      return;
    }
    dispatch({ type: "saveStarted" });
    const response = await saveGeneratedDeviceProfile(
      state.sessionHandle,
      state.selectedDeviceHandle,
      state.rootHandle,
      state.draft.profile,
    );
    if (response.kind !== "success") {
      dispatch({ type: "failed", message: apiFailure(response) });
      return;
    }
    dispatch({ type: "saveSucceeded", saved: response.result });
    onSaved(response.result.displayPath);
  }

  async function closeWizard() {
    const handle = sessionRef.current;
    sessionRef.current = null;
    if (handle) await cancelDeviceProfileGenerator(handle);
    dispatch({ type: "cancelled" });
    onClose();
  }

  function changeForm(update: Partial<DeviceProfileFormState>) {
    if (!state.form) return;
    dispatch({ type: "formChanged", form: { ...state.form, ...update } });
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/40 p-4" role="presentation">
      <section
        aria-labelledby="device-profile-generator-title"
        aria-modal="true"
        className="flex max-h-[94vh] w-full max-w-6xl flex-col overflow-hidden rounded-xl border border-slate-200 bg-white shadow-2xl"
        role="dialog"
      >
        <header className="flex items-center justify-between border-b border-slate-200 px-6 py-4">
          <div>
            <h2 className="text-lg font-semibold text-slate-950" id="device-profile-generator-title">
              Generate Device Profile
            </h2>
            <p className="text-sm text-slate-500">Read-only ADB capture · explicit review · new files only</p>
          </div>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" type="button" onClick={() => void closeWizard()}>
            Cancel
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-6">
          {state.error ? (
            <div className="mb-4 rounded border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-800">
              {state.error}
            </div>
          ) : null}
          {state.phase === "starting" ? <p className="text-sm text-slate-600">Starting generator…</p> : null}
          {state.phase === "devices" ? renderDevices() : null}
          {state.phase === "facts" && state.facts ? renderFacts(state.facts) : null}
          {state.phase === "edit" && state.form && state.draft ? renderEditor() : null}
          {state.phase === "review" && state.draft && state.collisions ? renderReview(state.draft) : null}
          {state.phase === "saved" && state.saved ? renderSaved() : null}
        </div>
      </section>
    </div>
  );

  function renderDevices() {
    return (
      <div className="grid gap-5">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-semibold text-slate-900">1. Select a connected device</h3>
            <p className="text-sm text-slate-500">Device serials remain in trusted Tauri memory.</p>
          </div>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" disabled={state.busy} type="button" onClick={() => void refreshDevices()}>
            Refresh
          </button>
        </div>
        <div className="grid gap-2">
          {state.devices.length === 0 && !state.busy ? <p className="text-sm text-slate-600">No ADB devices were found.</p> : null}
          {state.devices.map((device) => (
            <label className="flex items-center gap-3 rounded border border-slate-200 p-3" key={device.deviceHandle}>
              <input
                checked={state.selectedDeviceHandle === device.deviceHandle}
                disabled={!isSelectableDeviceState(device.state)}
                name="generator-device"
                type="radio"
                onChange={() => dispatch({ type: "deviceSelected", deviceHandle: device.deviceHandle })}
              />
              <span className="font-medium text-slate-900">{device.model ?? "Unknown model"}</span>
              <span className="text-xs uppercase tracking-wide text-slate-500">{device.state}</span>
            </label>
          ))}
        </div>
        <div className="flex justify-end">
          <button className="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-40" disabled={!state.selectedDeviceHandle || state.busy} type="button" onClick={() => void probeSelectedDevice()}>
            Read Device Information
          </button>
        </div>
      </div>
    );
  }

  function renderFacts(facts: SafeDetectedDeviceFactsDto) {
    const values: Array<[string, string, string | number | string[] | null]> = [
      ["Manufacturer", "facts.manufacturer", facts.manufacturer],
      ["Brand", "facts.brand", facts.brand],
      ["Model", "facts.model", facts.model],
      ["Product", "facts.product", facts.product],
      ["Device", "facts.device", facts.device],
      ["Board", "facts.board", facts.board],
      ["Hardware", "facts.hardware", facts.hardware],
      ["ABIs", "facts.abis", facts.abis],
      ["Android version", "facts.androidVersion", facts.androidVersion],
      ["Android API level", "facts.androidApiLevel", facts.androidApiLevel],
    ];
    return (
      <div className="grid gap-5">
        <h3 className="font-semibold text-slate-900">2. Review detected facts</h3>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          {values.map(([label, field, value]) => (
            <div className="rounded border border-slate-200 p-3" key={field}>
              <div className="mb-1 flex items-center justify-between gap-2">
                <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
                <EvidenceBadge evidence={evidenceFor(state.draft, field)} />
              </div>
              <p className="break-words text-sm text-slate-900">{Array.isArray(value) ? value.join(", ") || "Missing" : value ?? "Missing"}</p>
            </div>
          ))}
        </div>
        <div className="flex justify-between">
          <button className="rounded border border-slate-300 px-4 py-2 text-sm" type="button" onClick={() => void refreshDevices()}>
            Choose Another Device
          </button>
          <button className="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white" type="button" onClick={() => dispatch({ type: "editStarted" })}>
            Configure Profile
          </button>
        </div>
      </div>
    );
  }

  function renderEditor() {
    const form = state.form!;
    return (
      <div className="grid gap-6">
        <div>
          <h3 className="font-semibold text-slate-900">3. Configure the profile</h3>
          <p className="text-sm text-slate-500">Every proposed authored value is editable. Schema identity remains fixed.</p>
        </div>
        <div className="grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-2">
          <FixedValue label="Schema version" value="1" />
          <FixedValue label="Kind" value="device_profile" />
          <TextField label="ID" value={form.id} evidence={evidenceFor(state.draft, "id")} onChange={(id) => changeForm({ id })} />
          <TextField label="Name" value={form.name} evidence={evidenceFor(state.draft, "name")} onChange={(name) => changeForm({ name })} />
          <TextField label="Description" value={form.description} evidence={evidenceFor(state.draft, "description")} onChange={(description) => changeForm({ description })} />
          <TextField label="Android minimum" inputMode="numeric" value={form.androidMinimum} evidence={evidenceFor(state.draft, "match.android_version")} onChange={(androidMinimum) => changeForm({ androidMinimum })} />
        </div>
        <div className="grid gap-4 rounded border border-slate-200 p-4 md:grid-cols-3">
          <TextArea label="Manufacturer contains" value={form.manufacturers} evidence={evidenceFor(state.draft, "match.manufacturer_contains")} onChange={(manufacturers) => changeForm({ manufacturers })} />
          <TextArea label="Brand contains" value={form.brands} evidence={evidenceFor(state.draft, "match.brand_contains")} onChange={(brands) => changeForm({ brands })} />
          <TextArea label="Model patterns" value={form.modelPatterns} evidence={evidenceFor(state.draft, "match.model_patterns")} onChange={(modelPatterns) => changeForm({ modelPatterns })} />
        </div>
        <div className="rounded border border-slate-200 p-4">
          <h4 className="mb-3 text-sm font-semibold text-slate-900">Capability defaults</h4>
          <div className="grid gap-3 md:grid-cols-2">
            {capabilityLabels.map(([field, label]) => (
              <label className="flex items-center justify-between gap-3 rounded bg-slate-50 px-3 py-2" key={field}>
                <span className="flex items-center gap-2 text-sm text-slate-800">
                  {label}
                  <EvidenceBadge evidence={evidenceFor(state.draft, `capability_defaults.${field}`)} />
                </span>
                <input checked={form.capabilities[field]} type="checkbox" onChange={(event) => changeForm({ capabilities: { ...form.capabilities, [field]: event.target.checked } })} />
              </label>
            ))}
          </div>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <TextArea label="Device tags" value={form.deviceTags} evidence={evidenceFor(state.draft, "device_tags")} onChange={(deviceTags) => changeForm({ deviceTags })} />
          <TextArea label="Metadata JSON object" rows={8} value={form.metadata} evidence={evidenceFor(state.draft, "metadata")} onChange={(metadata) => changeForm({ metadata })} />
        </div>
        <Diagnostics draft={state.draft!} />
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="text-sm text-slate-600">
            Authored root: <span className="font-medium">{state.rootLabel ?? "Not selected"}</span>
            <button className="ml-3 rounded border border-slate-300 px-2 py-1 text-xs" type="button" onClick={() => void selectRoot()}>
              {state.rootHandle ? "Change" : "Select"}
            </button>
          </div>
          <button className="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-40" disabled={state.busy} type="button" onClick={() => void reviewProfile()}>
            Validate and Review
          </button>
        </div>
      </div>
    );
  }

  function renderReview(draft: DeviceProfileDraftResult) {
    return (
      <div className="grid gap-5">
        <div>
          <h3 className="font-semibold text-slate-900">4. Review YAML and collisions</h3>
          <p className="text-sm text-slate-500">Destination: {draft.destination.relativePath}</p>
        </div>
        <div className="grid min-h-[24rem] gap-4 lg:grid-cols-2">
          <pre className="overflow-auto rounded bg-slate-950 p-4 text-xs text-slate-100">{draft.canonicalYaml}</pre>
          <div className="grid content-start gap-2 rounded border border-slate-200 p-4">
            <h4 className="font-semibold text-slate-900">Collision analysis</h4>
            {state.collisions!.collisions.length === 0 ? <p className="text-sm text-emerald-700">No collisions found.</p> : null}
            {state.collisions!.collisions.map((collision, index) => (
              <div className={`rounded border px-3 py-2 text-sm ${collision.severity === "blocking" ? "border-red-200 bg-red-50 text-red-800" : "border-amber-200 bg-amber-50 text-amber-900"}`} key={`${collision.code}-${collision.existingProfileId ?? index}`}>
                <span className="font-semibold capitalize">{collision.severity}: </span>{collision.message}
                {collision.existingProfileId ? <span> ({collision.existingProfileId})</span> : null}
              </div>
            ))}
          </div>
        </div>
        <div className="flex justify-between">
          <button className="rounded border border-slate-300 px-4 py-2 text-sm" type="button" onClick={() => dispatch({ type: "editResumed" })}>
            Edit Profile
          </button>
          <button className="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white disabled:opacity-40" disabled={state.busy || state.collisions!.blocking} type="button" onClick={() => void saveProfile()}>
            Save New Profile
          </button>
        </div>
      </div>
    );
  }

  function renderSaved() {
    return (
      <div className="mx-auto grid max-w-xl gap-4 py-12 text-center">
        <h3 className="text-xl font-semibold text-emerald-800">Device profile saved</h3>
        <p className="text-sm text-slate-600">{state.saved!.displayPath}</p>
        <button className="mx-auto rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white" type="button" onClick={() => void closeWizard()}>
          Done
        </button>
      </div>
    );
  }
}

function evidenceFor(draft: DeviceProfileDraftResult | null, field: string) {
  return draft?.evidence.find((evidence) => evidence.field === field) ?? null;
}

function EvidenceBadge({ evidence }: { evidence: DeviceProfileFieldEvidenceDto | null }) {
  if (!evidence) return null;
  const styles: Record<DeviceProfileEvidenceState, string> = {
    verified: "bg-emerald-100 text-emerald-800",
    derived: "bg-blue-100 text-blue-800",
    suggested: "bg-amber-100 text-amber-900",
    missing: "bg-slate-200 text-slate-700",
  };
  return (
    <span className={`rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${styles[evidence.state]}`} title={evidence.source}>
      {evidence.state}{evidence.editedFromProposal ? " · edited" : ""}
    </span>
  );
}

function FixedValue({ label, value }: { label: string; value: string }) {
  return <div className="grid gap-1"><span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span><div className="rounded border border-slate-200 bg-slate-100 px-3 py-2 text-sm text-slate-700">{value}</div></div>;
}

function TextField({ label, value, evidence, inputMode, onChange }: { label: string; value: string; evidence: DeviceProfileFieldEvidenceDto | null; inputMode?: "numeric"; onChange: (value: string) => void }) {
  return <label className="grid gap-1"><span className="flex items-center justify-between gap-2 text-xs font-semibold uppercase tracking-wide text-slate-500">{label}<EvidenceBadge evidence={evidence} /></span><input className="rounded border border-slate-300 px-3 py-2 text-sm" inputMode={inputMode} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function TextArea({ label, value, evidence, rows = 5, onChange }: { label: string; value: string; evidence: DeviceProfileFieldEvidenceDto | null; rows?: number; onChange: (value: string) => void }) {
  return <label className="grid gap-1"><span className="flex items-center justify-between gap-2 text-xs font-semibold uppercase tracking-wide text-slate-500">{label}<EvidenceBadge evidence={evidence} /></span><textarea className="rounded border border-slate-300 px-3 py-2 font-mono text-xs" rows={rows} value={value} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Diagnostics({ draft }: { draft: DeviceProfileDraftResult }) {
  if (draft.diagnostics.length === 0) return null;
  return <div className="grid gap-2">{draft.diagnostics.map((diagnostic, index) => <div className={`rounded border px-3 py-2 text-sm ${diagnostic.severity === "error" ? "border-red-200 bg-red-50 text-red-800" : "border-amber-200 bg-amber-50 text-amber-900"}`} key={`${diagnostic.code}-${diagnostic.field}-${index}`}><span className="font-semibold">{diagnostic.field}: </span>{diagnostic.message}</div>)}</div>;
}

function apiFailure<T>(response: Exclude<EditorApiResult<T>, { kind: "success" }>): string {
  return response.kind === "api-error" ? response.error.message : response.message;
}

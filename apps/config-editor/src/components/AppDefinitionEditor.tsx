import { useState } from "react";
import type { ReactNode } from "react";

import type {
  AppArtifactKindDto,
  AppArtifactStrategyDto,
  AppDefinitionV1Dto,
  AppReleaseProviderDto,
  AppTargetKindDto,
  AppTargetLocationDto,
} from "../api/types";
import {
  appArtifactFromDraft,
  appPermissionSetFromDraft,
  appTargetFromDraft,
  createAppDefinitionEditorDraft,
  createEmptyArtifactEditorDraft,
  createEmptyMetadataEditorDraft,
  createEmptyPermissionActionEditorDraft,
  createEmptyTargetEditorDraft,
  isAppDefinitionEditorDraftDirty,
  isReservedAppMetadataKey,
  parseAppMetadataValue,
  reconcileAppDefinitionEditorDraft,
  requireAppDefinitionId,
  type AppDefinitionCommand,
  type AppDefinitionEditorDraft,
  type AppMetadataEditorDraft,
  type AppPermissionSetEditorDraft,
  type AppTargetEditorDraft,
} from "./appDefinitionEditor.logic";

/** Properties required to connect the standalone forms to an App Definition document session. */
export interface AppDefinitionEditorProps {
  /** The latest accepted schema-v1 App Definition returned by the sidecar. */
  readonly appDefinition: AppDefinitionV1Dto;
  /** Serializable form values, including incomplete local input not yet submitted. */
  readonly draft: AppDefinitionEditorDraft;
  /** Whether this document may change its App Definition ID and package identity. */
  readonly identityEditable: boolean;
  /** Disables all controls when the document session is read-only or applying a command. */
  readonly disabled: boolean;
  /** Receives every local form edit and whether form values differ from the accepted document. */
  readonly onDraftChange: (draft: AppDefinitionEditorDraft, hasPendingChanges: boolean) => void;
  /** Applies one exact tagged App Definition command and resolves with the accepted DTO. */
  readonly onApply: (command: AppDefinitionCommand) => Promise<AppDefinitionV1Dto>;
  /** Installs an accepted command result in the parent document session. */
  readonly onDocumentAccepted: (appDefinition: AppDefinitionV1Dto) => void;
}

/** Accessible schema-v1 forms that apply one typed command at a time. */
export function AppDefinitionEditor({
  appDefinition,
  draft,
  identityEditable,
  disabled,
  onDraftChange,
  onApply,
  onDocumentAccepted,
}: AppDefinitionEditorProps) {
  const [applying, setApplying] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const controlDisabled = disabled || applying;

  const clearError = (key: string) => {
    setErrors((current) => {
      if (current[key] === undefined) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  };

  const showError = (key: string, message: string) => {
    setErrors((current) => ({ ...current, [key]: message }));
  };

  const changeDraft = (nextDraft: AppDefinitionEditorDraft, key?: string) => {
    onDraftChange(nextDraft, isAppDefinitionEditorDraftDirty(nextDraft, appDefinition));
    if (key !== undefined) clearError(key);
  };

  const discardPendingDraft = () => {
    onDraftChange(createAppDefinitionEditorDraft(appDefinition), false);
    setErrors({});
  };

  const applyCommand = async (
    key: string,
    command: AppDefinitionCommand,
    submittedDraft: AppDefinitionEditorDraft = draft,
    submittedRowLocalId?: string,
  ) => {
    if (controlDisabled) return;
    clearError(key);
    setApplying(true);
    try {
      const accepted = await onApply(command);
      const nextDraft = reconcileAppDefinitionEditorDraft(
        submittedDraft,
        accepted,
        command,
        submittedRowLocalId,
      );
      onDocumentAccepted(accepted);
      onDraftChange(nextDraft, isAppDefinitionEditorDraftDirty(nextDraft, accepted));
    } catch (error) {
      showError(key, error instanceof Error ? error.message : "The App Definition change was rejected.");
    } finally {
      setApplying(false);
    }
  };

  const applyTextField = (
    field: "id" | "packageId" | "name" | "description" | "category" | "launcherActivity",
    value: string | undefined,
  ) => {
    const key = field;
    if (value === undefined) {
      const command: AppDefinitionCommand = field === "description"
        ? { type: "SetDescription", description: null }
        : field === "category"
          ? { type: "SetCategory", category: null }
          : { type: "SetLauncherActivity", launcherActivity: null };
      void applyCommand(key, command);
      return;
    }
    if (!value.trim()) {
      showError(key, `${fieldLabel(field)} must not be empty. Use the remove action for optional fields.`);
      return;
    }
    if (field === "id") {
      try {
        requireAppDefinitionId(value, "App ID");
      } catch (error) {
        showError(key, error instanceof Error ? error.message : "App ID is invalid.");
        return;
      }
      void applyCommand(key, { type: "SetId", id: value });
    } else if (field === "packageId") {
      void applyCommand(key, { type: "SetPackageId", packageId: value });
    } else if (field === "name") {
      void applyCommand(key, { type: "SetName", name: value });
    } else if (field === "description") {
      void applyCommand(key, { type: "SetDescription", description: value });
    } else if (field === "category") {
      void applyCommand(key, { type: "SetCategory", category: value });
    } else {
      void applyCommand(key, { type: "SetLauncherActivity", launcherActivity: value });
    }
  };

  const updateIdentity = (field: "id" | "packageId", value: string) => {
    changeDraft({ ...draft, identity: { ...draft.identity, [field]: value } }, field);
  };

  const updatePresentation = (
    field: "name" | "description" | "category" | "launcherActivity",
    value: string,
  ) => {
    changeDraft({ ...draft, presentation: { ...draft.presentation, [field]: value } }, field);
  };

  return (
    <div className="space-y-5" aria-label="App Definition editor">
      <section className="rounded border border-slate-200 p-4" aria-labelledby="app-overview-heading">
        <h2 id="app-overview-heading" className="text-lg font-semibold">Overview</h2>
        <p className="mt-1 text-sm text-slate-600">
          Apply each field independently. Existing diagnostics in other fields do not prevent a correction.
        </p>
        {isAppDefinitionEditorDraftDirty(draft, appDefinition) ? (
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950" role="status">
            <span>Form input is pending. Apply individual fields or discard it before saving.</span>
            <button
              className="rounded border border-amber-500 bg-white px-3 py-1.5 font-medium hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-40"
              type="button"
              disabled={controlDisabled}
              onClick={discardPendingDraft}
            >
              Discard pending form input
            </button>
          </div>
        ) : null}
        <div className="mt-3 grid gap-3 md:grid-cols-2">
          <EditorField
            label="App ID"
            value={draft.identity.id}
            disabled={controlDisabled || !identityEditable}
            onChange={(value) => updateIdentity("id", value)}
            help={identityEditable ? undefined : "Catalog identity is fixed for this document."}
            error={errors.id}
            submitLabel="Apply app ID"
            onSubmit={() => applyTextField("id", draft.identity.id)}
          />
          <EditorField
            label="Package ID"
            value={draft.identity.packageId}
            disabled={controlDisabled || !identityEditable}
            onChange={(value) => updateIdentity("packageId", value)}
            help={identityEditable ? "Android application ID declared by the app." : "Catalog identity is fixed for this document."}
            error={errors.packageId}
            submitLabel="Apply package ID"
            onSubmit={() => applyTextField("packageId", draft.identity.packageId)}
          />
          <EditorField
            label="Name"
            value={draft.presentation.name}
            disabled={controlDisabled}
            onChange={(value) => updatePresentation("name", value)}
            error={errors.name}
            submitLabel="Apply name"
            onSubmit={() => applyTextField("name", draft.presentation.name)}
          />
          <OptionalEditorField
            label="Description"
            value={draft.presentation.description}
            disabled={controlDisabled}
            error={errors.description}
            onChange={(value) => updatePresentation("description", value)}
            onSubmit={() => applyTextField("description", draft.presentation.description)}
            onRemove={() => {
              const next = { ...draft, presentation: { ...draft.presentation, description: undefined } };
              changeDraft(next, "description");
              void applyCommand("description", { type: "SetDescription", description: null }, next);
            }}
          />
          <OptionalEditorField
            label="Category"
            value={draft.presentation.category}
            disabled={controlDisabled}
            error={errors.category}
            onChange={(value) => updatePresentation("category", value)}
            onSubmit={() => applyTextField("category", draft.presentation.category)}
            onRemove={() => {
              const next = { ...draft, presentation: { ...draft.presentation, category: undefined } };
              changeDraft(next, "category");
              void applyCommand("category", { type: "SetCategory", category: null }, next);
            }}
          />
          <OptionalEditorField
            label="Launcher activity"
            value={draft.presentation.launcherActivity}
            disabled={controlDisabled}
            error={errors.launcherActivity}
            onChange={(value) => updatePresentation("launcherActivity", value)}
            onSubmit={() => applyTextField("launcherActivity", draft.presentation.launcherActivity)}
            onRemove={() => {
              const next = { ...draft, presentation: { ...draft.presentation, launcherActivity: undefined } };
              changeDraft(next, "launcherActivity");
              void applyCommand("launcherActivity", { type: "SetLauncherActivity", launcherActivity: null }, next);
            }}
          />
        </div>
        {!identityEditable ? (
          <p className="mt-3 text-sm text-slate-600">
            This catalog App Definition keeps its ID and package identity. Create a separate definition or use the catalog refactor workflow to change identity.
          </p>
        ) : null}
      </section>

      <ArtifactSection
        draft={draft}
        disabled={controlDisabled}
        errorFor={(key) => errors[key]}
        changeDraft={changeDraft}
        showError={showError}
        applyCommand={applyCommand}
      />
      <PermissionSection
        draft={draft}
        disabled={controlDisabled}
        errorFor={(key) => errors[key]}
        changeDraft={changeDraft}
        showError={showError}
        applyCommand={applyCommand}
      />
      <TargetSection
        draft={draft}
        disabled={controlDisabled}
        errorFor={(key) => errors[key]}
        changeDraft={changeDraft}
        showError={showError}
        applyCommand={applyCommand}
      />
      <MetadataSection
        draft={draft}
        disabled={controlDisabled}
        errorFor={(key) => errors[key]}
        changeDraft={changeDraft}
        showError={showError}
        applyCommand={applyCommand}
      />
    </div>
  );
}

interface SectionProps {
  readonly draft: AppDefinitionEditorDraft;
  readonly disabled: boolean;
  readonly errorFor: (key: string) => string | undefined;
  readonly changeDraft: (draft: AppDefinitionEditorDraft, key?: string) => void;
  readonly showError: (key: string, message: string) => void;
  readonly applyCommand: (
    key: string,
    command: AppDefinitionCommand,
    submittedDraft?: AppDefinitionEditorDraft,
    submittedRowLocalId?: string,
  ) => Promise<void>;
}

function fieldLabel(field: "id" | "packageId" | "name" | "description" | "category" | "launcherActivity"): string {
  const labels = {
    id: "App ID",
    packageId: "Package ID",
    name: "Name",
    description: "Description",
    category: "Category",
    launcherActivity: "Launcher activity",
  };
  return labels[field];
}

function EditorField({
  label,
  value,
  disabled,
  onChange,
  help,
  error,
  submitLabel,
  onSubmit,
}: {
  readonly label: string;
  readonly value: string;
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
  readonly help?: string;
  readonly error?: string;
  readonly submitLabel: string;
  readonly onSubmit: () => void;
}) {
  return (
    <form className="rounded border border-slate-200 p-3" onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      <label className="block text-sm font-medium">
        {label}
        <input
          className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
          type="text"
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.currentTarget.value)}
        />
      </label>
      {help ? <p className="mt-1 text-xs text-slate-500">{help}</p> : null}
      <InlineError message={error} />
      <button className="mt-2 rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="submit">
        {submitLabel}
      </button>
    </form>
  );
}

function OptionalEditorField({
  label,
  value,
  disabled,
  error,
  onChange,
  onSubmit,
  onRemove,
}: {
  readonly label: string;
  readonly value?: string;
  readonly disabled: boolean;
  readonly error?: string;
  readonly onChange: (value: string) => void;
  readonly onSubmit: () => void;
  readonly onRemove: () => void;
}) {
  return (
    <form className="rounded border border-slate-200 p-3" onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      <label className="block text-sm font-medium">
        {label}
        <input
          className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
          type="text"
          value={value ?? ""}
          disabled={disabled}
          onChange={(event) => onChange(event.currentTarget.value)}
        />
      </label>
      <InlineError message={error} />
      <div className="mt-2 flex gap-2">
        <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="submit">
          Apply {label.toLowerCase()}
        </button>
        <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled || value === undefined} type="button" onClick={onRemove}>
          Remove {label.toLowerCase()}
        </button>
      </div>
    </form>
  );
}

function TextInput({
  label,
  value,
  disabled,
  onChange,
  help,
  readOnly = false,
}: {
  readonly label: string;
  readonly value?: string;
  readonly disabled: boolean;
  readonly onChange?: (value: string) => void;
  readonly help?: string;
  readonly readOnly?: boolean;
}) {
  return (
    <label className="block text-sm font-medium">
      {label}
      <input
        className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
        type="text"
        value={value ?? ""}
        disabled={disabled}
        readOnly={readOnly}
        onChange={(event) => onChange?.(event.currentTarget.value)}
      />
      {help ? <span className="mt-1 block text-xs font-normal text-slate-500">{help}</span> : null}
    </label>
  );
}

function TextAreaInput({
  label,
  value,
  disabled,
  onChange,
}: {
  readonly label: string;
  readonly value: string;
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
}) {
  return (
    <label className="block text-sm font-medium">
      {label}
      <textarea
        className="mt-1 min-h-28 w-full rounded border border-slate-300 px-3 py-2 font-mono text-sm"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </label>
  );
}

function SelectInput<T extends string>({
  label,
  value,
  disabled,
  onChange,
  options,
}: {
  readonly label: string;
  readonly value: T;
  readonly disabled: boolean;
  readonly onChange: (value: T) => void;
  readonly options: readonly { readonly value: T; readonly label: string }[];
}) {
  return (
    <label className="block text-sm font-medium">
      {label}
      <select
        className="mt-1 w-full rounded border border-slate-300 px-3 py-2"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value as T)}
      >
        {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
      </select>
    </label>
  );
}

function CheckInput({
  label,
  checked,
  disabled,
  onChange,
}: {
  readonly label: string;
  readonly checked: boolean;
  readonly disabled: boolean;
  readonly onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.currentTarget.checked)} />
      {label}
    </label>
  );
}

function InlineError({ message }: { readonly message?: string }) {
  return message ? <p className="mt-2 text-sm text-red-700" role="alert">{message}</p> : null;
}

function SectionHeading({ id, children }: { readonly id: string; readonly children: ReactNode }) {
  return <h2 id={id} className="text-lg font-semibold">{children}</h2>;
}

function ArtifactSection({ draft, disabled, errorFor, changeDraft, showError, applyCommand }: SectionProps) {
  const addArtifact = () => changeDraft({ ...draft, artifacts: [...draft.artifacts, createEmptyArtifactEditorDraft()] });

  const updateArtifact = (localId: string, update: (row: AppDefinitionEditorDraft["artifacts"][number]) => AppDefinitionEditorDraft["artifacts"][number]) => {
    changeDraft({ ...draft, artifacts: draft.artifacts.map((row) => row.localId === localId ? update(row) : row) }, `artifact:${localId}`);
  };

  const submitArtifact = async (row: AppDefinitionEditorDraft["artifacts"][number]) => {
    const errorKey = `artifact:${row.localId}`;
    try {
      const artifact = appArtifactFromDraft(row);
      const command: AppDefinitionCommand = row.originalId === undefined
        ? { type: "AddArtifact", artifactId: row.id, artifact }
        : { type: "ReplaceArtifact", artifactId: row.originalId, artifact };
      await applyCommand(errorKey, command);
    } catch (error) {
      showError(errorKey, error instanceof Error ? error.message : "Artifact fields are invalid.");
    }
  };

  const removeArtifact = (row: AppDefinitionDraftArtifact) => {
    if (row.originalId === undefined) {
      changeDraft({ ...draft, artifacts: draft.artifacts.filter((item) => item.localId !== row.localId) });
      return;
    }
    void applyCommand(`artifact:${row.localId}`, { type: "RemoveArtifact", artifactId: row.originalId }, draft);
  };

  return (
    <section className="space-y-3 rounded border border-slate-200 p-4" aria-labelledby="app-artifacts-heading">
      <SectionHeading id="app-artifacts-heading">Artifacts</SectionHeading>
      <p className="text-sm text-slate-600">Artifact IDs identify authored entries. Existing IDs stay fixed while their source policy is edited.</p>
      {draft.artifacts.map((row, index) => {
        const errorKey = `artifact:${row.localId}`;
        const rowName = row.originalId ?? "new";
        const strategyOptions: readonly { value: AppArtifactStrategyDto; label: string }[] = [
          { value: "user_provided", label: "Provided by the user" },
          { value: "direct_url", label: "Direct HTTPS URL" },
          { value: "latest_release", label: "Latest release asset" },
        ];
        const kindOptions: readonly { value: AppArtifactKindDto; label: string }[] = [
          { value: "apk", label: "APK" },
          { value: "file", label: "File" },
        ];
        const providerOptions: readonly { value: AppReleaseProviderDto; label: string }[] = [
          { value: "github", label: "GitHub" },
          { value: "gitlab", label: "GitLab" },
          { value: "forgejo", label: "Forgejo" },
        ];
        const allPersisted = draft.artifacts.every((item) => item.originalId !== undefined);
        return (
          <fieldset key={row.localId} className="space-y-3 rounded border border-slate-200 p-3">
            <legend className="px-1 font-medium">Artifact {rowName}</legend>
            <div className="grid gap-3 md:grid-cols-2">
              <TextInput label={`Artifact ID ${rowName}`} value={row.id} disabled={disabled || row.originalId !== undefined} readOnly={row.originalId !== undefined} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, id: value }))} />
              <SelectInput label={`Artifact kind ${rowName}`} value={row.kind} disabled={disabled} options={kindOptions} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, kind: value }))} />
              <OptionalRowInput label={`Artifact name ${rowName}`} value={row.name} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, name: value }))} onRemove={() => updateArtifact(row.localId, (item) => ({ ...item, name: undefined }))} />
              <OptionalRowInput label={`Artifact description ${rowName}`} value={row.description} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, description: value }))} onRemove={() => updateArtifact(row.localId, (item) => ({ ...item, description: undefined }))} />
              <SelectInput label={`Artifact source policy ${rowName}`} value={row.sourceStrategy} disabled={disabled} options={strategyOptions} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, sourceStrategy: value }))} />
              {row.sourceStrategy === "direct_url" ? (
                <>
                  <TextInput label="Direct URL" value={row.url} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, url: value }))} />
                  <OptionalRowInput label="SHA-256" value={row.sha256} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, sha256: value }))} onRemove={() => updateArtifact(row.localId, (item) => ({ ...item, sha256: undefined }))} />
                </>
              ) : null}
              {row.sourceStrategy === "latest_release" ? (
                <>
                  <SelectInput label="Release provider" value={row.provider} disabled={disabled} options={providerOptions} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, provider: value }))} />
                  <TextInput label="Release service origin" value={row.baseUrl} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, baseUrl: value }))} />
                  <TextInput label="Release repository" value={row.repository} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, repository: value }))} />
                  <OptionalRowInput label="Asset pattern" value={row.assetPattern} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, assetPattern: value }))} onRemove={() => updateArtifact(row.localId, (item) => ({ ...item, assetPattern: undefined }))} />
                  <CheckInput label="Invert asset pattern matches" checked={row.invertAssetPattern ?? false} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, invertAssetPattern: value }))} />
                  <CheckInput label="Allow prereleases" checked={row.prerelease} disabled={disabled} onChange={(value) => updateArtifact(row.localId, (item) => ({ ...item, prerelease: value }))} />
                </>
              ) : null}
            </div>
            <InlineError message={errorFor(errorKey)} />
            <div className="flex flex-wrap gap-2">
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void submitArtifact(row); }}>
                {row.originalId === undefined ? `Add artifact ${row.id || "new"}` : `Apply artifact ${row.originalId}`}
              </button>
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => removeArtifact(row)}>
                {row.originalId === undefined ? "Discard artifact" : `Remove artifact ${row.originalId}`}
              </button>
              {row.originalId !== undefined && allPersisted && index > 0 ? (
                <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void applyCommand(errorKey, { type: "ReorderArtifact", artifactId: row.originalId!, toIndex: index - 1 }); }}>
                  Move artifact {row.originalId} up
                </button>
              ) : null}
              {row.originalId !== undefined && allPersisted && index < draft.artifacts.length - 1 ? (
                <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void applyCommand(errorKey, { type: "ReorderArtifact", artifactId: row.originalId!, toIndex: index + 1 }); }}>
                  Move artifact {row.originalId} down
                </button>
              ) : null}
            </div>
          </fieldset>
        );
      })}
      <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={addArtifact}>Add artifact</button>
    </section>
  );
}

type AppDefinitionDraftArtifact = AppDefinitionEditorDraft["artifacts"][number];

function OptionalRowInput({
  label,
  value,
  disabled,
  onChange,
  onRemove,
}: {
  readonly label: string;
  readonly value?: string;
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
  readonly onRemove: () => void;
}) {
  return (
    <div className="space-y-1">
      <TextInput label={label} value={value} disabled={disabled} onChange={onChange} />
      <button className="text-xs text-slate-600 underline" disabled={disabled || value === undefined} type="button" onClick={onRemove}>
        Remove {label.toLowerCase()}
      </button>
    </div>
  );
}

function PermissionSection({ draft, disabled, errorFor, changeDraft, showError, applyCommand }: SectionProps) {
  const updateSet = (
    name: "baseline" | "elevated",
    update: (value: AppPermissionSetEditorDraft) => AppPermissionSetEditorDraft,
  ) => {
    const current = draft.permissionSets[name] ?? { runtime: [], appOps: [] };
    changeDraft({ ...draft, permissionSets: { ...draft.permissionSets, [name]: update(current) } }, `permission:${name}`);
  };

  const applySet = (name: "baseline" | "elevated") => {
    const key = `permission:${name}`;
    const value = draft.permissionSets[name];
    if (value === undefined) return;
    try {
      void applyCommand(key, { type: "SetPermissionSet", permissionSet: name, value: appPermissionSetFromDraft(value) });
    } catch (error) {
      showError(key, error instanceof Error ? error.message : "Permission set fields are invalid.");
    }
  };

  const renderSet = (name: "baseline" | "elevated") => {
    const value = draft.permissionSets[name];
    const label = name === "baseline" ? "Baseline" : "Elevated";
    const set = value ?? { runtime: [], appOps: [] };
    return (
      <fieldset key={name} className="space-y-3 rounded border border-slate-200 p-3">
        <legend className="px-1 font-medium">{label} permission set</legend>
        {value === undefined ? <p className="text-sm text-slate-600">This optional permission set is not authored.</p> : null}
        <div className="space-y-2">
          {(set.runtime ?? []).map((action, index) => (
            <div key={action.localId} className="grid gap-2 rounded border border-slate-100 p-2 md:grid-cols-4">
              <TextInput label={`Runtime permission ${index + 1}`} value={action.permission} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, runtime: current.runtime?.map((row) => row.localId === action.localId ? { ...row, permission: text } : row) }))} />
              <TextInput label={`Runtime permission minimum Android API ${index + 1}`} value={action.androidApiMin} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, runtime: current.runtime?.map((row) => row.localId === action.localId ? { ...row, androidApiMin: text } : row) }))} />
              <TextInput label={`Runtime permission maximum Android API ${index + 1}`} value={action.androidApiMax} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, runtime: current.runtime?.map((row) => row.localId === action.localId ? { ...row, androidApiMax: text } : row) }))} />
              <button className="self-end rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => updateSet(name, (current) => ({ ...current, runtime: (current.runtime ?? []).filter((row) => row.localId !== action.localId) }))}>Remove runtime permission {index + 1}</button>
            </div>
          ))}
          {(set.appOps ?? []).map((action, index) => (
            <div key={action.localId} className="grid gap-2 rounded border border-slate-100 p-2 md:grid-cols-4">
              <TextInput label={`App-op action ${index + 1}`} value={action.op} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, appOps: current.appOps?.map((row) => row.localId === action.localId ? { ...row, op: text } : row) }))} />
              <TextInput label={`App-op minimum Android API ${index + 1}`} value={action.androidApiMin} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, appOps: current.appOps?.map((row) => row.localId === action.localId ? { ...row, androidApiMin: text } : row) }))} />
              <TextInput label={`App-op maximum Android API ${index + 1}`} value={action.androidApiMax} disabled={disabled} onChange={(text) => updateSet(name, (current) => ({ ...current, appOps: current.appOps?.map((row) => row.localId === action.localId ? { ...row, androidApiMax: text } : row) }))} />
              <button className="self-end rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => updateSet(name, (current) => ({ ...current, appOps: (current.appOps ?? []).filter((row) => row.localId !== action.localId) }))}>Remove app-op action {index + 1}</button>
            </div>
          ))}
        </div>
        <div className="flex flex-wrap gap-2">
          <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => updateSet(name, (current) => ({ ...current, runtime: [...(current.runtime ?? []), createEmptyPermissionActionEditorDraft("runtime")] }))}>Add runtime permission</button>
          <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => updateSet(name, (current) => ({ ...current, appOps: [...(current.appOps ?? []), createEmptyPermissionActionEditorDraft("app_op")] }))}>Add app-op action</button>
          <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled || value === undefined} type="button" onClick={() => applySet(name)}>Apply {name} permissions</button>
          {value === undefined ? (
            <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => changeDraft({ ...draft, permissionSets: { ...draft.permissionSets, [name]: { runtime: [], appOps: [] } } })}>Create {name} permission set</button>
          ) : (
            <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void applyCommand(`permission:${name}`, { type: "SetPermissionSet", permissionSet: name, value: null }); }}>Remove {name} permission set</button>
          )}
        </div>
        <InlineError message={errorFor(`permission:${name}`)} />
      </fieldset>
    );
  };

  return (
    <section className="space-y-3 rounded border border-slate-200 p-4" aria-labelledby="app-permissions-heading">
      <SectionHeading id="app-permissions-heading">Permissions</SectionHeading>
      <p className="text-sm text-slate-600">Each named permission set is applied atomically. Other document diagnostics remain available for separate corrections.</p>
      {renderSet("baseline")}
      {renderSet("elevated")}
    </section>
  );
}

function TargetSection({ draft, disabled, errorFor, changeDraft, showError, applyCommand }: SectionProps) {
  const updateTarget = (localId: string, update: (target: AppTargetEditorDraft) => AppTargetEditorDraft) => {
    changeDraft({ ...draft, targets: draft.targets.map((row) => row.localId === localId ? update(row) : row) }, `target:${localId}`);
  };

  const submitTarget = (row: AppTargetEditorDraft) => {
    const key = `target:${row.localId}`;
    try {
      const target = appTargetFromDraft(row);
      const command: AppDefinitionCommand = row.originalId === undefined
        ? { type: "AddTarget", targetId: row.id, target }
        : { type: "ReplaceTarget", targetId: row.originalId, target };
      void applyCommand(key, command);
    } catch (error) {
      showError(key, error instanceof Error ? error.message : "Target fields are invalid.");
    }
  };

  const addTarget = () => changeDraft({ ...draft, targets: [...draft.targets, createEmptyTargetEditorDraft()] });

  return (
    <section className="space-y-3 rounded border border-slate-200 p-4" aria-labelledby="app-targets-heading">
      <SectionHeading id="app-targets-heading">Targets</SectionHeading>
      <p className="text-sm text-slate-600">Target IDs stay fixed after creation. A target describes one authored installation destination.</p>
      {draft.targets.map((row, index) => {
        const rowName = row.originalId ?? "new";
        const errorKey = `target:${row.localId}`;
        const kindOptions: readonly { value: AppTargetKindDto; label: string }[] = [
          { value: "file", label: "File" },
          { value: "directory", label: "Directory" },
        ];
        const locationOptions: readonly { value: AppTargetLocationDto; label: string }[] = [
          { value: "app_data", label: "App data" },
          { value: "external_app_data", label: "External app data" },
          { value: "shared_storage", label: "Shared storage" },
          { value: "absolute_device_path", label: "Absolute device path" },
        ];
        const allPersisted = draft.targets.every((item) => item.originalId !== undefined);
        return (
          <fieldset key={row.localId} className="space-y-3 rounded border border-slate-200 p-3">
            <legend className="px-1 font-medium">Target {rowName}</legend>
            <div className="grid gap-3 md:grid-cols-2">
              <TextInput label={`Target ID ${rowName}`} value={row.id} disabled={disabled || row.originalId !== undefined} readOnly={row.originalId !== undefined} onChange={(value) => updateTarget(row.localId, (item) => ({ ...item, id: value }))} />
              <SelectInput label={`Target kind ${rowName}`} value={row.kind} disabled={disabled} options={kindOptions} onChange={(value) => updateTarget(row.localId, (item) => ({ ...item, kind: value }))} />
              <SelectInput label={`Target location ${rowName}`} value={row.location} disabled={disabled} options={locationOptions} onChange={(value) => updateTarget(row.localId, (item) => ({ ...item, location: value }))} />
              <TextInput label={`Target path ${rowName}`} value={row.path} disabled={disabled} onChange={(value) => updateTarget(row.localId, (item) => ({ ...item, path: value }))} />
            </div>
            <InlineError message={errorFor(errorKey)} />
            <div className="flex flex-wrap gap-2">
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => submitTarget(row)}>
                {row.originalId === undefined ? `Add target ${row.id || "new"}` : `Apply target ${row.originalId}`}
              </button>
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => row.originalId === undefined
                ? changeDraft({ ...draft, targets: draft.targets.filter((item) => item.localId !== row.localId) })
                : void applyCommand(errorKey, { type: "RemoveTarget", targetId: row.originalId })}>
                {row.originalId === undefined ? "Discard target" : `Remove target ${row.originalId}`}
              </button>
              {row.originalId !== undefined && allPersisted && index > 0 ? (
                <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void applyCommand(errorKey, { type: "ReorderTarget", targetId: row.originalId!, toIndex: index - 1 }); }}>Move target {row.originalId} up</button>
              ) : null}
              {row.originalId !== undefined && allPersisted && index < draft.targets.length - 1 ? (
                <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => { void applyCommand(errorKey, { type: "ReorderTarget", targetId: row.originalId!, toIndex: index + 1 }); }}>Move target {row.originalId} down</button>
              ) : null}
            </div>
          </fieldset>
        );
      })}
      <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={addTarget}>Add target</button>
    </section>
  );
}

function MetadataSection({ draft, disabled, errorFor, changeDraft, showError, applyCommand }: SectionProps) {
  const updateMetadata = (localId: string, update: (row: AppMetadataEditorDraft) => AppMetadataEditorDraft) => {
    changeDraft({ ...draft, metadata: draft.metadata.map((row) => row.localId === localId ? update(row) : row) }, `metadata:${localId}`);
  };

  const submitMetadata = (row: AppMetadataEditorDraft) => {
    const errorKey = `metadata:${row.localId}`;
    if (!row.key.trim()) {
      showError(errorKey, "Metadata key must not be empty.");
      return;
    }
    if (isReservedAppMetadataKey(row.key)) {
      showError(errorKey, `Metadata key ${row.key} is reserved for an App Definition field.`);
      return;
    }
    if (draft.metadata.some((item) => item.localId !== row.localId && item.key === row.key)) {
      showError(errorKey, "Metadata key already exists.");
      return;
    }
    try {
      const value = parseAppMetadataValue(row.valueJson);
      void applyCommand(errorKey, { type: "SetMetadataEntry", key: row.key, value }, draft, row.localId);
    } catch (error) {
      showError(errorKey, error instanceof Error ? error.message : "Metadata value is invalid.");
    }
  };

  return (
    <section className="space-y-3 rounded border border-slate-200 p-4" aria-labelledby="app-metadata-heading">
      <SectionHeading id="app-metadata-heading">Metadata</SectionHeading>
      <p className="text-sm text-slate-600">Metadata is descriptive data. Reserved schema keys cannot be authored through this section.</p>
      {draft.metadata.map((row) => {
        const rowName = row.originalKey ?? "new";
        const errorKey = `metadata:${row.localId}`;
        return (
          <fieldset key={row.localId} className="space-y-3 rounded border border-slate-200 p-3">
            <legend className="px-1 font-medium">Metadata {rowName}</legend>
            <TextInput label={`Metadata key ${rowName}`} value={row.key} disabled={disabled || row.originalKey !== undefined} readOnly={row.originalKey !== undefined} onChange={(value) => updateMetadata(row.localId, (item) => ({ ...item, key: value }))} />
            <TextAreaInput label={`Metadata value JSON ${rowName}`} value={row.valueJson} disabled={disabled} onChange={(value) => updateMetadata(row.localId, (item) => ({ ...item, valueJson: value }))} />
            <InlineError message={errorFor(errorKey)} />
            <div className="flex flex-wrap gap-2">
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => submitMetadata(row)}>
                {row.originalKey === undefined ? `Add metadata ${row.key || "new"}` : `Apply metadata ${row.originalKey}`}
              </button>
              <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => row.originalKey === undefined
                ? changeDraft({ ...draft, metadata: draft.metadata.filter((item) => item.localId !== row.localId) })
                : void applyCommand(errorKey, { type: "RemoveMetadataEntry", key: row.originalKey })}>
                {row.originalKey === undefined ? "Discard metadata entry" : `Remove metadata ${row.originalKey}`}
              </button>
            </div>
          </fieldset>
        );
      })}
      <button className="rounded border border-slate-300 px-3 py-1 text-sm" disabled={disabled} type="button" onClick={() => changeDraft({ ...draft, metadata: [...draft.metadata, createEmptyMetadataEditorDraft()] })}>Add metadata entry</button>
    </section>
  );
}

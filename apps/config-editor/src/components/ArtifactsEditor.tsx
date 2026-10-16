import { useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { AppCatalogDto, ArtifactDto, InputDto, RecipeDocumentDto } from "../api/types";
import {
  appArtifactOptions,
  appRefOptions,
  catalogApp,
  catalogApps,
  inputBindingOptions,
  uniqueArtifactId,
} from "./appCatalog.logic";
import { EditableTextField } from "./EditableTextField";
import { ResizableEditorLayout } from "./ResizableEditorLayout";
import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

const ARTIFACT_CACHE_VALUES = ["default", "none"] as const;

interface ArtifactsEditorProps {
  document: RecipeDocumentDto;
  promptForId: (title: string, initialValue: string) => Promise<string | null>;
  promptForRequiredText: (title: string, initialValue: string, label: string) => Promise<string | null>;
  confirmAction: (
    title: string,
    message: string,
    options?: { confirmLabel?: string; destructive?: boolean },
  ) => Promise<boolean>;
  readOnly?: boolean;
  onCommand: (command: EditorCommand) => Promise<boolean>;
}

export function ArtifactsEditor({
  document,
  promptForId,
  promptForRequiredText,
  confirmAction,
  readOnly = false,
  onCommand,
}: ArtifactsEditorProps) {
  const artifacts = document.recipe.artifacts;
  const artifactIds = Object.keys(artifacts).sort();
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const [addAppArtifact, setAddAppArtifact] = useState(false);
  const selectedId = selectedArtifactId && selectedArtifactId in artifacts ? selectedArtifactId : artifactIds[0] ?? null;
  const selectedArtifact = selectedId ? artifacts[selectedId] : null;

  async function addArtifact() {
    let attemptedId = "new_artifact";
    let attemptedUrl = "";
    while (true) {
      const artifactId = await promptForId("Add artifact id", attemptedId);
      if (artifactId === null) {
        return;
      }
      const url = await promptForRequiredText(`URL for artifact ${artifactId}`, attemptedUrl, "Artifact URL");
      if (url === null) {
        return;
      }
      const ok = await onCommand({ type: "AddArtifact", artifactId, url });
      if (ok) {
        setSelectedArtifactId(artifactId);
        return;
      }
      attemptedId = artifactId;
      attemptedUrl = url;
    }
  }

  async function renameArtifact(artifactId: string) {
    let attempted = artifactId;
    while (true) {
      const newArtifactId = await promptForId(`Rename artifact ${artifactId}`, attempted);
      if (newArtifactId === null || newArtifactId === artifactId) {
        return;
      }
      const ok = await onCommand({ type: "RenameArtifact", artifactId, newArtifactId });
      if (ok) {
        setSelectedArtifactId(newArtifactId);
        return;
      }
      attempted = newArtifactId;
    }
  }

  async function duplicateArtifact(artifactId: string) {
    let attempted = `${artifactId}_copy`;
    while (true) {
      const newArtifactId = await promptForId(`Duplicate artifact ${artifactId}`, attempted);
      if (newArtifactId === null) {
        return;
      }
      const ok = await onCommand({ type: "DuplicateArtifact", sourceArtifactId: artifactId, newArtifactId });
      if (ok) {
        setSelectedArtifactId(newArtifactId);
        return;
      }
      attempted = newArtifactId;
    }
  }

  async function deleteArtifact(artifactId: string) {
    const confirmed = await confirmAction("Delete artifact", `Delete artifact ${artifactId}?`, {
      confirmLabel: "Delete",
      destructive: true,
    });
    if (!confirmed) {
      return;
    }
    const currentIndex = artifactIds.indexOf(artifactId);
    const nextId = artifactIds.filter((id) => id !== artifactId)[Math.max(0, currentIndex - 1)] ?? null;
    const ok = await onCommand({ type: "DeleteArtifact", artifactId });
    if (ok) {
      setSelectedArtifactId(nextId);
    }
  }

  return (
    <ResizableEditorLayout
      resizeLabel="Resize artifacts list"
      sidebarBody={
        <div className="space-y-1">
          {artifactIds.length === 0 ? <p className="text-sm text-slate-500">No artifacts</p> : null}
          {artifactIds.map((artifactId) => (
            <button
              className={`block w-full rounded px-3 py-2 text-left text-sm ${
                artifactId === selectedId ? "bg-slate-900 text-white" : "text-slate-700 hover:bg-slate-100"
              }`}
              key={artifactId}
              type="button"
              onClick={() => setSelectedArtifactId(artifactId)}
            >
              {artifactId}
            </button>
          ))}
        </div>
      }
      sidebarHeader={
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Artifacts</h1>
          <div className="flex gap-1">
          <button
            className="rounded border border-slate-300 px-2 py-1 text-sm disabled:opacity-40"
            disabled={readOnly}
            type="button"
            onClick={addArtifact}
          >
            Add
          </button>
          <button
            className="rounded border border-slate-300 px-2 py-1 text-sm disabled:opacity-40"
            disabled={readOnly || catalogApps(document.appCatalog).length === 0}
            type="button"
            onClick={() => setAddAppArtifact(true)}
          >
            Add app
          </button>
          </div>
        </div>
      }
      storageKey="emuchef.configEditor.artifacts.sidebarWidth"
    >
      {addAppArtifact ? (
        <AddAppArtifactForm
          appCatalog={document.appCatalog}
          artifacts={artifacts}
          inputs={document.recipe.inputs}
          readOnly={readOnly}
          onCancel={() => setAddAppArtifact(false)}
          onAddAppArtifact={async (command) => {
            const ok = await onCommand(command);
            if (ok) {
              setSelectedArtifactId(command.artifactId);
              setAddAppArtifact(false);
            }
            return ok;
          }}
        />
      ) : selectedId && selectedArtifact ? (
        <ArtifactDetail
          appCatalog={document.appCatalog}
          artifact={selectedArtifact}
          artifactId={selectedId}
          inputs={document.recipe.inputs}
          readOnly={readOnly}
          onDelete={() => void deleteArtifact(selectedId)}
          onDuplicate={() => void duplicateArtifact(selectedId)}
          onRename={() => void renameArtifact(selectedId)}
          onUpdateField={(field, value) =>
            onCommand({ type: "UpdateArtifactField", artifactId: selectedId, field, value })
          }
        />
      ) : (
        <p className="text-sm text-slate-500">Select or add an artifact.</p>
      )}
    </ResizableEditorLayout>
  );
}

interface ArtifactDetailProps {
  appCatalog: AppCatalogDto;
  artifact: ArtifactDto;
  artifactId: string;
  inputs: Record<string, InputDto>;
  readOnly: boolean;
  onRename: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onUpdateField: (field: "url" | "cache" | "appRef" | "artifact" | "input", value: unknown) => Promise<boolean>;
}

function ArtifactDetail({
  appCatalog,
  artifact,
  artifactId,
  inputs,
  readOnly,
  onRename,
  onDelete,
  onDuplicate,
  onUpdateField,
}: ArtifactDetailProps) {
  const isRemoteFile = artifact.type === "remote_file";
  const selectedApp = catalogApp(appCatalog, artifact.appRef ?? null);
  const selectedCatalogArtifact =
    selectedApp?.artifacts.find((catalogArtifact) => catalogArtifact.id === artifact.artifact) ?? null;
  // Whether the projected App Definition artifact sources its bytes from a
  // user-provided file. Null means the projection cannot answer because the
  // App Definition or the artifact is not in the catalog; binding edits then
  // fail closed while any authored binding stays visible.
  const requiresInput = selectedCatalogArtifact?.requiresInput ?? null;
  const hasBinding = artifact.input !== undefined && artifact.input !== null;
  const bindingOptions = inputBindingOptions(inputs, artifact.input ?? null);
  const bindingIsCompatible =
    !hasBinding || bindingOptions.some((option) => option.value === artifact.input && !option.retained);

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-xl font-semibold text-slate-950">{artifactId}</h2>
          <p className="text-sm text-slate-500">Artifact id is changed with Rename only.</p>
        </div>
        <div className="flex gap-2">
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm disabled:opacity-40" disabled={readOnly} type="button" onClick={onRename}>
            Rename
          </button>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm disabled:opacity-40" disabled={readOnly} type="button" onClick={onDuplicate}>
            Duplicate
          </button>
          <button className="rounded border border-red-300 px-3 py-1.5 text-sm text-red-700 disabled:opacity-40" disabled={readOnly} type="button" onClick={onDelete}>
            Delete
          </button>
        </div>
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <ReadonlyText label="ID" value={artifact.id} />
        <ReadonlyText label="Type" value={artifact.type} />
        {isRemoteFile ? (
          <>
            <EditableTextField label="URL" readOnly={readOnly} value={artifact.url ?? ""} onCommit={(value) => onUpdateField("url", value)} />
          </>
        ) : (
          <>
            <label className="grid gap-1">
              <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">App Definition</span>
              <select
                className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
                disabled={readOnly}
                value={artifact.appRef ?? ""}
                onChange={(event) => {
                  if (event.target.value !== artifact.appRef) {
                    void onUpdateField("appRef", event.target.value);
                  }
                }}
              >
                {appRefOptions(appCatalog, artifact.appRef ?? null).map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1">
              <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">App artifact</span>
              <select
                className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
                disabled={readOnly}
                value={artifact.artifact ?? ""}
                onChange={(event) => {
                  if (event.target.value !== artifact.artifact) {
                    void onUpdateField("artifact", event.target.value);
                  }
                }}
              >
                {appArtifactOptions(selectedApp, artifact.artifact ?? null).map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            {requiresInput === false ? (
              hasBinding ? (
                <>
                  <label className="grid gap-1">
                    <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">File input</span>
                    <select
                      className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
                      disabled={readOnly}
                      value={artifact.input ?? ""}
                      onChange={(event) => {
                        const nextInput = event.target.value === "" ? null : event.target.value;
                        if (nextInput !== (artifact.input ?? null)) {
                          void onUpdateField("input", nextInput);
                        }
                      }}
                    >
                      <option value="">No input binding</option>
                      {bindingOptions
                        .filter((option) => option.value === artifact.input)
                        .map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.retained
                              ? option.label
                              : `${option.label} (invalid: only user-provided App Artifacts bind inputs)`}
                          </option>
                        ))}
                    </select>
                  </label>
                  <p className="text-xs text-amber-800">
                    Only a user-provided App Artifact can bind a Recipe file input. Set this binding to
                    &quot;No input binding&quot; to correct it.
                  </p>
                </>
              ) : (
                <p className="text-sm text-slate-500">
                  This App Definition artifact is not user-provided, so it cannot bind a Recipe file input.
                </p>
              )
            ) : requiresInput === true ? (
              <>
                <label className="grid gap-1">
                  <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">File input</span>
                  <select
                    className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
                    disabled={readOnly}
                    value={artifact.input ?? ""}
                    onChange={(event) => {
                      const nextInput = event.target.value;
                      if (nextInput !== "" && nextInput !== (artifact.input ?? null)) {
                        void onUpdateField("input", nextInput);
                      }
                    }}
                  >
                    {hasBinding ? null : (
                      <option value="" disabled>
                        Choose a Recipe file input
                      </option>
                    )}
                    {bindingOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>
                {!hasBinding ? (
                  <p className="text-xs text-amber-800">
                    A user-provided App Artifact must bind one single-file Recipe input. Choose one of the
                    Recipe file inputs.
                  </p>
                ) : null}
                {hasBinding && !bindingIsCompatible ? (
                  <p className="text-xs text-amber-800">
                    This binding is not a single-file input declared by this Recipe. Choose a compatible
                    file input; a required user-provided binding cannot be dropped.
                  </p>
                ) : null}
              </>
            ) : (
              <>
                {hasBinding ? (
                  <label className="grid gap-1">
                    <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">File input</span>
                    <select
                      className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
                      disabled={true}
                      value={artifact.input ?? ""}
                    >
                      {bindingOptions
                        .filter((option) => option.value === artifact.input)
                        .map((option) => (
                          <option key={option.value} value={option.value}>
                            {option.label}
                          </option>
                        ))}
                    </select>
                  </label>
                ) : null}
                <p className="text-xs text-amber-800">
                  {hasBinding
                    ? "The catalog projection does not answer whether this artifact requires a Recipe file input. The current binding stays as authored; binding changes stay disabled."
                    : "The catalog projection does not answer whether this artifact requires a Recipe file input, so no binding can be authored yet."}
                </p>
              </>
            )}
            <p className="text-sm text-slate-600">
              Source and package identity come from the App Definition. The Recipe owns the reference, its optional
              file input, and cache behavior.
            </p>
          </>
        )}
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Cache</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            disabled={readOnly}
            value={artifact.cache ?? "default"}
            onChange={(event) => {
              const value = event.target.value;
              if (value !== artifact.cache) {
                void onUpdateField("cache", value);
              }
            }}
          >
            {ARTIFACT_CACHE_VALUES.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
      </div>
    </div>
  );
}

function ReadonlyText({ label, value }: { label: string; value: string }) {
  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      <input className="w-full rounded border border-slate-200 bg-slate-100 px-3 py-2 text-sm text-slate-600" readOnly value={value} />
    </label>
  );
}

interface AddAppArtifactFormProps {
  appCatalog: AppCatalogDto;
  artifacts: Record<string, ArtifactDto>;
  inputs: Record<string, InputDto>;
  readOnly: boolean;
  onCancel: () => void;
  onAddAppArtifact: (
    command: Extract<EditorCommand, { type: "AddAppArtifact" }>,
  ) => Promise<boolean>;
}

/**
 * Inline form that authors one Recipe App Artifact from catalog choices.
 *
 * Every app-owned choice comes from the document's App Definition projection:
 * the App Definition, its artifact, and the optional Recipe file input that
 * supplies a user-provided artifact. The form never creates inputs or copies
 * package, launcher, source, or target facts into the Recipe; the seeded
 * Recipe artifact id is the only value the author edits freely. Submission
 * re-checks the live projection on every render, so a refreshed catalog that
 * no longer projects the draft App Definition or artifact keeps the draft
 * visible with an explanatory diagnostic and disables the submit button.
 *
 * The Recipe file input control follows the projected artifact: a
 * user-provided artifact must bind exactly one single-file Recipe input, while
 * any other artifact source cannot bind one. The form never creates the input.
 */
function AddAppArtifactForm({
  appCatalog,
  artifacts,
  inputs,
  readOnly,
  onCancel,
  onAddAppArtifact,
}: AddAppArtifactFormProps) {
  const apps = catalogApps(appCatalog);
  const firstApp = apps[0] ?? null;
  const [appRef, setAppRef] = useState(firstApp?.id ?? "");
  const [appArtifactId, setAppArtifactId] = useState(() => appArtifactOptions(firstApp, null)[0]?.value ?? "");
  const [recipeArtifactId, setRecipeArtifactId] = useState(() =>
    uniqueArtifactId(appArtifactOptions(firstApp, null)[0]?.value ?? "artifact", Object.keys(artifacts)),
  );
  const [inputId, setInputId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const selectedApp = catalogApp(appCatalog, appRef);
  const selectedCatalogArtifact =
    selectedApp?.artifacts.find((artifact) => artifact.id === appArtifactId) ?? null;
  // Whether the selected App Definition artifact sources its bytes from a
  // user-provided file. Null means the projection cannot answer; membership
  // validation already blocks submission in that case.
  const requiresInput = selectedCatalogArtifact?.requiresInput ?? null;
  const membershipIssue = describeMembershipIssue();

  /**
   * Explains why the current draft cannot be submitted, or null when it can.
   *
   * The App Definition projection is re-read on every render and again when
   * the author submits, so a refreshed catalog never turns a retained draft
   * into a stale-but-accepted App Artifact.
   */
  function describeMembershipIssue(): string | null {
    if (appCatalog.status !== "ready") {
      return "The App Definition catalog is not available; adding App Artifacts is disabled.";
    }
    if (appRef === "") {
      return "Select an App Definition.";
    }
    if (selectedApp === null) {
      return `App Definition ${appRef} is not in the current catalog.`;
    }
    if (appArtifactId === "") {
      return "Select an App Definition artifact.";
    }
    if (selectedCatalogArtifact === null) {
      return `App Definition ${appRef} has no artifact ${appArtifactId}.`;
    }
    return null;
  }

  /**
   * Explains why the draft still needs a Recipe input binding, or null when
   * the selected artifact already satisfies the binding rules.
   *
   * A user-provided App Artifact must bind exactly one single-file Recipe
   * input. The form never creates that input itself; when the Recipe declares
   * no compatible input, the message says so instead of offering a seed.
   */
  function describeInputBindingIssue(): string | null {
    if (requiresInput !== true || inputId !== "") {
      return null;
    }
    const compatibleInputs = inputBindingOptions(inputs, null).filter((option) => !option.retained);
    if (compatibleInputs.length === 0) {
      return "A user-provided App Artifact must bind one single-file Recipe input. Add a single-file Recipe input first.";
    }
    return "A user-provided App Artifact must bind one single-file Recipe input. Choose one of the Recipe file inputs.";
  }

  function changeApp(nextAppRef: string) {
    setAppRef(nextAppRef);
    const nextApp = catalogApp(appCatalog, nextAppRef);
    const nextArtifactId = appArtifactOptions(nextApp, null)[0]?.value ?? "";
    setAppArtifactId(nextArtifactId);
    setRecipeArtifactId(uniqueArtifactId(nextArtifactId || "artifact", Object.keys(artifacts)));
    setInputId("");
    setError(null);
  }

  async function submit() {
    const trimmedId = recipeArtifactId.trim();
    if (membershipIssue !== null) {
      setError(membershipIssue);
      return;
    }
    if (!trimmedId) {
      setError("Enter a Recipe artifact id.");
      return;
    }
    if (Object.prototype.hasOwnProperty.call(artifacts, trimmedId)) {
      setError(`Artifact ${trimmedId} already exists.`);
      return;
    }
    const bindingIssue = describeInputBindingIssue();
    if (bindingIssue !== null) {
      setError(bindingIssue);
      return;
    }
    setSubmitting(true);
    const ok = await onAddAppArtifact({
      type: "AddAppArtifact",
      artifactId: trimmedId,
      appRef,
      artifact: appArtifactId,
      input: requiresInput === true && inputId !== "" ? inputId : null,
    });
    setSubmitting(false);
    if (!ok) {
      setError("Adding the App Artifact failed.");
    }
  }

  return (
    <div className="space-y-5">
      <div className="min-w-0">
        <h2 className="truncate text-xl font-semibold text-slate-950">Add App Artifact</h2>
        <p className="text-sm text-slate-500">
          The Recipe owns the reference and its optional file input; source and package facts stay with the App
          Definition.
        </p>
      </div>
      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">App Definition</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            disabled={readOnly || submitting}
            value={appRef}
            onChange={(event) => changeApp(event.target.value)}
          >
            {appRefOptions(appCatalog, appRef).map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">App artifact</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            disabled={readOnly || submitting}
            value={appArtifactId}
            onChange={(event) => {
              setAppArtifactId(event.target.value);
              setError(null);
            }}
          >
            {appArtifactOptions(selectedApp, appArtifactId).map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        {requiresInput === true ? (
          <p className="text-sm text-slate-600">
            This App Definition sources the artifact from a user-provided file, so the Recipe must bind one of
            its single-file inputs.
          </p>
        ) : null}
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Artifact id</span>
          <input
            {...textInputGuardProps}
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
            disabled={readOnly || submitting}
            value={recipeArtifactId}
            onChange={(event) => {
              setRecipeArtifactId(normalizeEditableText(event.target.value));
              setError(null);
            }}
          />
        </label>
        {requiresInput === true ? (
          <label className="grid gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">File input</span>
            <select
              className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
              disabled={readOnly || submitting}
              value={inputId}
              onChange={(event) => {
                setInputId(event.target.value);
                setError(null);
              }}
            >
              <option value="">No input binding</option>
              {inputBindingOptions(inputs, inputId).map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
        ) : requiresInput === false ? (
          <p className="text-sm text-slate-500">
            This App Definition artifact is not user-provided, so it cannot bind a Recipe file input.
          </p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <button
            className="rounded border border-slate-900 bg-slate-900 px-3 py-1.5 text-sm text-white disabled:opacity-40"
            disabled={readOnly || submitting || membershipIssue !== null}
            type="button"
            onClick={() => void submit()}
          >
            Add app artifact
          </button>
          <button
            className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
            disabled={submitting}
            type="button"
            onClick={onCancel}
          >
            Cancel
          </button>
        </div>
        {membershipIssue !== null ? (
          <p className="text-sm font-medium text-amber-700">{membershipIssue}</p>
        ) : null}
        {error ? <p className="text-sm font-medium text-red-700">{error}</p> : null}
      </div>
    </div>
  );
}

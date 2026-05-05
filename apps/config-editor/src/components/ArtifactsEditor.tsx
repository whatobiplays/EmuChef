import { useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { ArtifactDto, RecipeDocumentDto } from "../api/types";
import { EditableTextField } from "./EditableTextField";
import { ResizableEditorLayout } from "./ResizableEditorLayout";

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
  onCommand: (command: EditorCommand) => Promise<boolean>;
}

export function ArtifactsEditor({
  document,
  promptForId,
  promptForRequiredText,
  confirmAction,
  onCommand,
}: ArtifactsEditorProps) {
  const artifacts = document.recipe.artifacts;
  const artifactIds = Object.keys(artifacts).sort();
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
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
          <button className="rounded border border-slate-300 px-2 py-1 text-sm" type="button" onClick={addArtifact}>
            Add
          </button>
        </div>
      }
      storageKey="emuchef.configEditor.artifacts.sidebarWidth"
    >
      {selectedId && selectedArtifact ? (
        <ArtifactDetail
          artifact={selectedArtifact}
          artifactId={selectedId}
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
  artifact: ArtifactDto;
  artifactId: string;
  onRename: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onUpdateField: (field: "url" | "cache", value: unknown) => Promise<boolean>;
}

function ArtifactDetail({ artifact, artifactId, onRename, onDelete, onDuplicate, onUpdateField }: ArtifactDetailProps) {
  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-xl font-semibold text-slate-950">{artifactId}</h2>
          <p className="text-sm text-slate-500">Artifact id is changed with Rename only.</p>
        </div>
        <div className="flex gap-2">
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" type="button" onClick={onRename}>
            Rename
          </button>
          <button className="rounded border border-slate-300 px-3 py-1.5 text-sm" type="button" onClick={onDuplicate}>
            Duplicate
          </button>
          <button className="rounded border border-red-300 px-3 py-1.5 text-sm text-red-700" type="button" onClick={onDelete}>
            Delete
          </button>
        </div>
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <ReadonlyText label="ID" value={artifact.id} />
        <ReadonlyText label="Type" value={artifact.type} />
        <EditableTextField label="URL" value={artifact.url} onCommit={(value) => onUpdateField("url", value)} />
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Cache</span>
          <select
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
            value={artifact.cache}
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

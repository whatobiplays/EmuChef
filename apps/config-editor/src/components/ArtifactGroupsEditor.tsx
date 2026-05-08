import { useMemo, useState } from "react";

import type { EditorCommand } from "../api/commands";
import type { RecipeDocumentDto } from "../api/types";
import { ResizableEditorLayout } from "./ResizableEditorLayout";

interface ArtifactGroupsEditorProps {
  document: RecipeDocumentDto;
  promptForId: (title: string, initialValue: string) => Promise<string | null>;
  confirmAction: (
    title: string,
    message: string,
    options?: { confirmLabel?: string; destructive?: boolean },
  ) => Promise<boolean>;
  readOnly?: boolean;
  onCommand: (command: EditorCommand) => Promise<boolean>;
}

export function ArtifactGroupsEditor({ document, promptForId, confirmAction, readOnly = false, onCommand }: ArtifactGroupsEditorProps) {
  const groups = document.recipe.artifactGroups;
  const groupIds = Object.keys(groups);
  const artifactIds = useMemo(() => Object.keys(document.recipe.artifacts).sort(), [document.recipe.artifacts]);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const selectedId = selectedGroupId && selectedGroupId in groups ? selectedGroupId : groupIds[0] ?? null;
  const members = selectedId ? groups[selectedId] : [];
  const availableMembers = selectedId ? artifactIds.filter((artifactId) => !members.includes(artifactId)) : [];

  async function addGroup() {
    let attempted = "new_group";
    while (true) {
      const groupId = await promptForId("Add artifact group id", attempted);
      if (groupId === null) {
        return;
      }
      const ok = await onCommand({ type: "AddArtifactGroup", groupId });
      if (ok) {
        setSelectedGroupId(groupId);
        return;
      }
      attempted = groupId;
    }
  }

  async function renameGroup(groupId: string) {
    let attempted = groupId;
    while (true) {
      const newGroupId = await promptForId(`Rename artifact group ${groupId}`, attempted);
      if (newGroupId === null || newGroupId === groupId) {
        return;
      }
      const ok = await onCommand({ type: "RenameArtifactGroup", groupId, newGroupId });
      if (ok) {
        setSelectedGroupId(newGroupId);
        return;
      }
      attempted = newGroupId;
    }
  }

  async function duplicateGroup(groupId: string) {
    let attempted = `${groupId}_copy`;
    while (true) {
      const newGroupId = await promptForId(`Duplicate artifact group ${groupId}`, attempted);
      if (newGroupId === null) {
        return;
      }
      const ok = await onCommand({ type: "DuplicateArtifactGroup", sourceGroupId: groupId, newGroupId });
      if (ok) {
        setSelectedGroupId(newGroupId);
        return;
      }
      attempted = newGroupId;
    }
  }

  async function deleteGroup(groupId: string) {
    const confirmed = await confirmAction("Delete artifact group", `Delete artifact group ${groupId}?`, {
      confirmLabel: "Delete",
      destructive: true,
    });
    if (!confirmed) {
      return;
    }
    const currentIndex = groupIds.indexOf(groupId);
    const nextId = groupIds.filter((id) => id !== groupId)[Math.max(0, currentIndex - 1)] ?? null;
    const ok = await onCommand({ type: "DeleteArtifactGroup", groupId });
    if (ok) {
      setSelectedGroupId(nextId);
    }
  }

  async function moveGroup(groupId: string, toIndex: number) {
    await onCommand({ type: "ReorderArtifactGroup", groupId, toIndex });
  }

  async function addMember(groupId: string, artifactId: string) {
    if (!artifactId || members.includes(artifactId)) {
      return;
    }
    await onCommand({ type: "AddArtifactGroupMember", groupId, artifactId, index: members.length });
  }

  async function removeMember(groupId: string, artifactId: string, index: number) {
    const confirmed = await confirmAction(
      "Remove group member",
      `Remove artifact ${artifactId} from group ${groupId}?`,
      { confirmLabel: "Remove", destructive: true },
    );
    if (!confirmed) {
      return;
    }
    await onCommand({ type: "RemoveArtifactGroupMember", groupId, index });
  }

  async function moveMember(groupId: string, index: number, toIndex: number) {
    await onCommand({ type: "ReorderArtifactGroupMember", groupId, index, toIndex });
  }

  return (
    <ResizableEditorLayout
      minSidebarWidth={288}
      resizeLabel="Resize artifact groups list"
      sidebarBody={
        <div className="space-y-1">
          {groupIds.length === 0 ? <p className="text-sm text-slate-500">No artifact groups</p> : null}
          {groupIds.map((groupId, index) => (
            <div className="flex gap-1" key={groupId}>
              <button
                className={`min-w-0 flex-1 rounded px-3 py-2 text-left text-sm ${
                  groupId === selectedId ? "bg-slate-900 text-white" : "text-slate-700 hover:bg-slate-100"
                }`}
                type="button"
                onClick={() => setSelectedGroupId(groupId)}
              >
                {groupId}
              </button>
              <button
                className="rounded border border-slate-300 px-2 text-xs disabled:opacity-40"
                disabled={readOnly || index === 0}
                title="Move group up"
                type="button"
                onClick={() => void moveGroup(groupId, index - 1)}
              >
                Up
              </button>
              <button
                className="rounded border border-slate-300 px-2 text-xs disabled:opacity-40"
                disabled={readOnly || index === groupIds.length - 1}
                title="Move group down"
                type="button"
                onClick={() => void moveGroup(groupId, index + 1)}
              >
                Down
              </button>
            </div>
          ))}
        </div>
      }
      sidebarHeader={
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Artifact Groups</h1>
          <button
            className="rounded border border-slate-300 px-2 py-1 text-sm disabled:opacity-40"
            disabled={readOnly}
            type="button"
            onClick={addGroup}
          >
            Add
          </button>
        </div>
      }
      storageKey="emuchef.configEditor.artifactGroups.sidebarWidth"
    >
      {selectedId ? (
        <GroupDetail
          availableMembers={availableMembers}
          groupId={selectedId}
          members={members}
          readOnly={readOnly}
          onAddMember={(artifactId) => addMember(selectedId, artifactId)}
          onDelete={() => void deleteGroup(selectedId)}
          onDuplicate={() => void duplicateGroup(selectedId)}
          onMoveMember={(index, toIndex) => moveMember(selectedId, index, toIndex)}
          onRemoveMember={(artifactId, index) => removeMember(selectedId, artifactId, index)}
          onRename={() => void renameGroup(selectedId)}
        />
      ) : (
        <p className="text-sm text-slate-500">Select or add an artifact group.</p>
      )}
    </ResizableEditorLayout>
  );
}

interface GroupDetailProps {
  groupId: string;
  members: string[];
  availableMembers: string[];
  readOnly: boolean;
  onRename: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onAddMember: (artifactId: string) => Promise<void>;
  onRemoveMember: (artifactId: string, index: number) => Promise<void>;
  onMoveMember: (index: number, toIndex: number) => Promise<void>;
}

function GroupDetail({
  groupId,
  members,
  availableMembers,
  readOnly,
  onRename,
  onDelete,
  onDuplicate,
  onAddMember,
  onRemoveMember,
  onMoveMember,
}: GroupDetailProps) {
  const [memberToAdd, setMemberToAdd] = useState(availableMembers[0] ?? "");
  const selectedMember = availableMembers.includes(memberToAdd) ? memberToAdd : availableMembers[0] ?? "";

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-xl font-semibold text-slate-950">{groupId}</h2>
          <p className="text-sm text-slate-500">Group id is changed with Rename only.</p>
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
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Group ID</span>
          <input className="w-full rounded border border-slate-200 bg-slate-100 px-3 py-2 text-sm text-slate-600" readOnly value={groupId} />
        </label>

        <div className="flex flex-wrap items-end gap-2">
          <label className="min-w-64 flex-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">Add Member</span>
            <select
              className="mt-1 w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
              disabled={readOnly || availableMembers.length === 0}
              value={selectedMember}
              onChange={(event) => setMemberToAdd(event.target.value)}
            >
              {availableMembers.length === 0 ? <option value="">No available artifacts</option> : null}
              {availableMembers.map((artifactId) => (
                <option key={artifactId} value={artifactId}>
                  {artifactId}
                </option>
              ))}
            </select>
          </label>
          <button
            className="rounded border border-slate-300 px-3 py-2 text-sm disabled:opacity-40"
            disabled={readOnly || !selectedMember}
            type="button"
            onClick={() => void onAddMember(selectedMember)}
          >
            Add
          </button>
        </div>
      </div>

      <div className="space-y-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Members</h3>
        {members.length === 0 ? <p className="text-sm text-slate-500">No members</p> : null}
        {members.map((artifactId, index) => (
          <div
            className="grid grid-cols-[minmax(0,1fr)_auto_auto_auto] items-center gap-2 rounded border border-slate-200 bg-white px-3 py-2"
            key={`${artifactId}-${index}`}
          >
            <span className="truncate text-sm text-slate-900">{artifactId}</span>
            <button
              className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
              disabled={readOnly || index === 0}
              type="button"
              onClick={() => void onMoveMember(index, index - 1)}
            >
              Up
            </button>
            <button
              className="rounded border border-slate-300 px-2 py-1 text-xs disabled:opacity-40"
              disabled={readOnly || index === members.length - 1}
              type="button"
              onClick={() => void onMoveMember(index, index + 1)}
            >
              Down
            </button>
            <button
              className="rounded border border-red-300 px-2 py-1 text-xs text-red-700 disabled:opacity-40"
              disabled={readOnly}
              type="button"
              onClick={() => void onRemoveMember(artifactId, index)}
            >
              Remove
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

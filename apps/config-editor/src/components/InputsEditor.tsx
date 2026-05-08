import { useState } from "react";

import type { EditorCommand, InputEditableField } from "../api/commands";
import type { InputDto, RecipeDocumentDto } from "../api/types";
import { EditableTextField } from "./EditableTextField";
import { ReadOnlyJson } from "./ReadOnlyJson";
import { ResizableEditorLayout } from "./ResizableEditorLayout";

const INPUT_TYPES = ["file", "directory"] as const;
const INPUT_ROLES = ["apk", "bios", "roms", "config_bundle", "generic"] as const;
const PATH_KIND_VALUES = ["", "file", "directory"] as const;

interface InputsEditorProps {
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

export function InputsEditor({ document, promptForId, confirmAction, readOnly = false, onCommand }: InputsEditorProps) {
  const inputs = document.recipe.inputs;
  const inputIds = Object.keys(inputs).sort();
  const [selectedInputId, setSelectedInputId] = useState<string | null>(null);
  const selectedId = selectedInputId && selectedInputId in inputs ? selectedInputId : inputIds[0] ?? null;
  const selectedInput = selectedId ? inputs[selectedId] : null;

  async function addInput() {
    let attempted = "new_input";
    while (true) {
      const inputId = await promptForId("Add input id", attempted);
      if (inputId === null) {
        return;
      }
      const ok = await onCommand({ type: "AddInput", inputId });
      if (ok) {
        setSelectedInputId(inputId);
        return;
      }
      attempted = inputId;
    }
  }

  async function renameInput(inputId: string) {
    let attempted = inputId;
    while (true) {
      const newInputId = await promptForId(`Rename input ${inputId}`, attempted);
      if (newInputId === null || newInputId === inputId) {
        return;
      }
      const ok = await onCommand({ type: "RenameInput", inputId, newInputId });
      if (ok) {
        setSelectedInputId(newInputId);
        return;
      }
      attempted = newInputId;
    }
  }

  async function duplicateInput(inputId: string) {
    let attempted = `${inputId}_copy`;
    while (true) {
      const newInputId = await promptForId(`Duplicate input ${inputId}`, attempted);
      if (newInputId === null) {
        return;
      }
      const ok = await onCommand({ type: "DuplicateInput", sourceInputId: inputId, newInputId });
      if (ok) {
        setSelectedInputId(newInputId);
        return;
      }
      attempted = newInputId;
    }
  }

  async function deleteInput(inputId: string) {
    const confirmed = await confirmAction("Delete input", `Delete input ${inputId}?`, {
      confirmLabel: "Delete",
      destructive: true,
    });
    if (!confirmed) {
      return;
    }
    const currentIndex = inputIds.indexOf(inputId);
    const nextId = inputIds.filter((id) => id !== inputId)[Math.max(0, currentIndex - 1)] ?? null;
    const ok = await onCommand({ type: "DeleteInput", inputId });
    if (ok) {
      setSelectedInputId(nextId);
    }
  }

  async function updateField(inputId: string, field: InputEditableField, value: unknown) {
    return onCommand({ type: "UpdateInputField", inputId, field, value });
  }

  return (
    <ResizableEditorLayout
      resizeLabel="Resize inputs list"
      sidebarBody={
        <div className="space-y-1">
          {inputIds.length === 0 ? <p className="text-sm text-slate-500">No inputs</p> : null}
          {inputIds.map((inputId) => (
            <button
              className={`block w-full rounded px-3 py-2 text-left text-sm ${
                inputId === selectedId ? "bg-slate-900 text-white" : "text-slate-700 hover:bg-slate-100"
              }`}
              key={inputId}
              type="button"
              onClick={() => setSelectedInputId(inputId)}
            >
              {inputId}
            </button>
          ))}
        </div>
      }
      sidebarHeader={
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Inputs</h1>
          <button
            className="rounded border border-slate-300 px-2 py-1 text-sm disabled:opacity-40"
            disabled={readOnly}
            type="button"
            onClick={addInput}
          >
            Add
          </button>
        </div>
      }
      storageKey="emuchef.configEditor.inputs.sidebarWidth"
    >
      {selectedId && selectedInput ? (
        <InputDetail
          input={selectedInput}
          inputId={selectedId}
          readOnly={readOnly}
          onDelete={() => void deleteInput(selectedId)}
          onDuplicate={() => void duplicateInput(selectedId)}
          onRename={() => void renameInput(selectedId)}
          onUpdateField={(field, value) => updateField(selectedId, field, value)}
        />
      ) : (
        <p className="text-sm text-slate-500">Select or add an input.</p>
      )}
    </ResizableEditorLayout>
  );
}

interface InputDetailProps {
  input: InputDto;
  inputId: string;
  readOnly: boolean;
  onRename: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onUpdateField: (field: InputEditableField, value: unknown) => Promise<boolean>;
}

function InputDetail({ input, inputId, readOnly, onRename, onDelete, onDuplicate, onUpdateField }: InputDetailProps) {
  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate text-xl font-semibold text-slate-950">{inputId}</h2>
          <p className="text-sm text-slate-500">Input id is changed with Rename only.</p>
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
        <ReadonlyText label="ID" value={input.id} />
        <SelectField
          label="Type"
          disabled={readOnly}
          value={input.type}
          values={INPUT_TYPES}
          onChange={(value) => value !== input.type && onUpdateField("type", value)}
        />
        <SelectField
          label="Role"
          disabled={readOnly}
          value={input.role}
          values={INPUT_ROLES}
          onChange={(value) => value !== input.role && onUpdateField("role", value)}
        />
        <EditableTextField label="Label" readOnly={readOnly} value={input.label} onCommit={(value) => onUpdateField("label", value)} />
        <EditableTextField
          label="Description"
          multiline
          readOnly={readOnly}
          value={input.description}
          onCommit={(value) => onUpdateField("description", value)}
        />
        <CheckboxField
          checked={input.required}
          disabled={readOnly}
          label="Required"
          onChange={(value) => value !== input.required && onUpdateField("required", value)}
        />
        <CheckboxField
          checked={input.multiple}
          disabled={readOnly}
          label="Multiple"
          onChange={(value) => value !== input.multiple && onUpdateField("multiple", value)}
        />
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500">Validation</h3>
        <CheckboxField
          checked={input.validation.mustExist}
          disabled={readOnly}
          label="Must Exist"
          onChange={(value) => value !== input.validation.mustExist && onUpdateField("validation.must_exist", value)}
        />
        <EditableTextField
          label="Allowed Extensions"
          readOnly={readOnly}
          value={input.validation.allowedExtensions.join(", ")}
          onCommit={(value) => onUpdateField("validation.allowed_extensions", value)}
        />
        <SelectField
          label="Path Kind"
          disabled={readOnly}
          value={input.validation.pathKind ?? ""}
          values={PATH_KIND_VALUES}
          onChange={(value) =>
            value !== (input.validation.pathKind ?? "") && onUpdateField("validation.path_kind", value || null)
          }
        />
      </div>

      <div className="grid gap-4 rounded border border-slate-200 bg-white p-4">
        <ReadOnlyJson label="Default" value={input.default} />
        <ReadOnlyJson label="Metadata" value={input.metadata} />
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

function SelectField<T extends string>({
  label,
  value,
  values,
  disabled = false,
  onChange,
}: {
  label: string;
  value: T;
  values: readonly T[];
  disabled?: boolean;
  onChange: (value: T) => boolean | void | Promise<boolean>;
}) {
  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      <select
        className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm"
        disabled={disabled}
        value={value}
        onChange={(event) => void onChange(event.target.value as T)}
      >
        {values.map((item) => (
          <option key={item || "(none)"} value={item}>
            {item || "(none)"}
          </option>
        ))}
      </select>
    </label>
  );
}

function CheckboxField({
  checked,
  disabled = false,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (value: boolean) => boolean | void | Promise<boolean>;
}) {
  return (
    <label className="flex items-center gap-2 text-sm text-slate-800">
      <input checked={checked} disabled={disabled} type="checkbox" onChange={(event) => void onChange(event.target.checked)} />
      <span>{label}</span>
    </label>
  );
}

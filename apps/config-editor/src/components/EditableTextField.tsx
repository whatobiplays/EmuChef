import { KeyboardEvent, useEffect, useState } from "react";

interface EditableTextFieldProps {
  label: string;
  value: string;
  readOnly?: boolean;
  multiline?: boolean;
  placeholder?: string;
  onCommit?: (value: string) => Promise<boolean> | boolean;
}

export function EditableTextField({
  label,
  value,
  readOnly = false,
  multiline = false,
  placeholder,
  onCommit,
}: EditableTextFieldProps) {
  const [draft, setDraft] = useState(value);
  const [committing, setCommitting] = useState(false);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  async function commit() {
    if (readOnly || onCommit === undefined || draft === value) {
      return;
    }
    setCommitting(true);
    try {
      const changed = await onCommit(draft);
      if (!changed) {
        return;
      }
    } finally {
      setCommitting(false);
    }
  }

  function revert() {
    setDraft(value);
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      revert();
      event.currentTarget.blur();
      return;
    }
    if (!multiline && event.key === "Enter") {
      event.preventDefault();
      void commit();
    }
  }

  const controlClass =
    "w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm disabled:bg-slate-100 disabled:text-slate-500";

  return (
    <label className="grid gap-1">
      <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
      {multiline ? (
        <textarea
          className={`${controlClass} min-h-24 resize-y`}
          disabled={committing}
          placeholder={placeholder}
          readOnly={readOnly}
          value={draft}
          onBlur={() => void commit()}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
        />
      ) : (
        <input
          className={controlClass}
          disabled={committing}
          placeholder={placeholder}
          readOnly={readOnly}
          type="text"
          value={draft}
          onBlur={() => void commit()}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
        />
      )}
    </label>
  );
}

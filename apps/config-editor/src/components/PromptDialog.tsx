import { useEffect, useState, type ReactNode } from "react";

import { normalizeEditableText, textInputGuardProps } from "./textInputGuards.logic";

interface TextPromptDialogProps {
  title: string;
  label: string;
  initialValue: string;
  requiredMessage: string;
  confirmLabel?: string;
  trimResult: boolean;
  onCancel: () => void;
  onSubmit: (value: string) => void;
}

interface ConfirmDialogProps {
  title: string;
  message: string;
  confirmLabel?: string;
  destructive?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

export function TextPromptDialog({
  title,
  label,
  initialValue,
  requiredMessage,
  confirmLabel = "OK",
  trimResult,
  onCancel,
  onSubmit,
}: TextPromptDialogProps) {
  const [value, setValue] = useState(initialValue);
  const [validationMessage, setValidationMessage] = useState<string | null>(null);

  useEffect(() => {
    setValue(initialValue);
    setValidationMessage(null);
  }, [initialValue, title]);

  function submit() {
    if (!value.trim()) {
      setValidationMessage(requiredMessage);
      return;
    }
    onSubmit(trimResult ? value.trim() : value);
  }

  return (
    <ModalFrame title={title} onCancel={onCancel}>
      <form
        className="grid gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="grid gap-1">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</span>
          <input
            {...textInputGuardProps}
            autoFocus
            className="w-full rounded border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 shadow-sm outline-none focus:border-slate-500 focus:ring-2 focus:ring-slate-200"
            value={value}
            onChange={(event) => {
              setValue(normalizeEditableText(event.target.value));
              setValidationMessage(null);
            }}
          />
        </label>
        {validationMessage ? <p className="text-sm font-medium text-red-700">{validationMessage}</p> : null}
        <DialogActions
          confirmLabel={confirmLabel}
          onCancel={onCancel}
        />
      </form>
    </ModalFrame>
  );
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel = "Confirm",
  destructive = false,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <ModalFrame title={title} onCancel={onCancel}>
      <div className="grid gap-4">
        <p className="text-sm leading-6 text-slate-700">{message}</p>
        <DialogActions
          confirmLabel={confirmLabel}
          destructive={destructive}
          onCancel={onCancel}
          onConfirm={onConfirm}
        />
      </div>
    </ModalFrame>
  );
}

function ModalFrame({ title, children, onCancel }: { title: string; children: ReactNode; onCancel: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 px-4" role="presentation">
      <div
        aria-modal="true"
        className="w-full max-w-md rounded-lg border border-slate-200 bg-white p-5 shadow-xl"
        role="dialog"
      >
        <div className="mb-4">
          <h2 className="text-base font-semibold text-slate-950">{title}</h2>
        </div>
        {children}
      </div>
    </div>
  );
}

function DialogActions({
  confirmLabel,
  destructive = false,
  onCancel,
  onConfirm,
}: {
  confirmLabel: string;
  destructive?: boolean;
  onCancel: () => void;
  onConfirm?: () => void;
}) {
  const confirmClass = destructive
    ? "border-red-700 bg-red-700 text-white hover:bg-red-800"
    : "border-slate-900 bg-slate-900 text-white hover:bg-slate-800";

  return (
    <div className="flex justify-end gap-2">
      <button
        className="rounded border border-slate-300 px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-50"
        type="button"
        onClick={onCancel}
      >
        Cancel
      </button>
      <button
        className={`rounded border px-3 py-1.5 text-sm ${confirmClass}`}
        type={onConfirm ? "button" : "submit"}
        onClick={onConfirm}
      >
        {confirmLabel}
      </button>
    </div>
  );
}

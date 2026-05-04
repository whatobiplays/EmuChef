interface ErrorBannerProps {
  message: string;
  onDismiss: () => void;
}

export function ErrorBanner({ message, onDismiss }: ErrorBannerProps) {
  return (
    <div className="border-b border-red-200 bg-red-50 px-4 py-3 text-sm text-red-800">
      <div className="flex items-start gap-3">
        <p className="min-w-0 flex-1">{message}</p>
        <button className="shrink-0 rounded px-2 py-0.5 text-red-700 hover:bg-red-100" type="button" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

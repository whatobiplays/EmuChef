interface LoadingStateProps {
  label: string;
}

export function LoadingState({ label }: LoadingStateProps) {
  return (
    <div className="border-b border-slate-200 bg-slate-50 px-4 py-2 text-xs font-medium text-slate-600">
      {label}
    </div>
  );
}

import type { ReactNode } from "react";

interface AppShellProps {
  toolbar: ReactNode;
  sidebar: ReactNode;
  children: ReactNode;
  rightPanel: ReactNode;
}

export function AppShell({ toolbar, sidebar, children, rightPanel }: AppShellProps) {
  return (
    <div className="flex h-full flex-col bg-slate-50 text-slate-900">
      <header className="shrink-0 border-b border-slate-200 bg-white">{toolbar}</header>
      <main className="grid min-h-0 flex-1 grid-cols-[16rem_minmax(0,1fr)_26rem]">
        <aside className="min-h-0 border-r border-slate-200 bg-white">{sidebar}</aside>
        <section className="min-h-0 overflow-y-auto">{children}</section>
        <aside className="min-h-0 border-l border-slate-200 bg-white">{rightPanel}</aside>
      </main>
    </div>
  );
}

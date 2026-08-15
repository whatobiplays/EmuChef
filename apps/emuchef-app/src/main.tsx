import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";

import { api } from "./api";
import { App, createAppDialogController } from "./App";
import { FrontendErrorBoundary } from "./ErrorBoundary";
import { Phase6d6UiSmoke } from "./Phase6d6UiSmoke";
import type { Phase6d6UiSmokeStatus } from "./types";
import "./styles.css";

const dialogController = createAppDialogController();

type StartupState =
  | { kind: "loading" }
  | { kind: "app" }
  | { kind: "qualification"; status: Phase6d6UiSmokeStatus }
  | { kind: "error" };

/**
 * Startup gate for the development-only Phase 6D.6 UI-smoke qualification
 * surface.
 *
 * The normal app mounts only after the qualification status command
 * explicitly reports `enabled: false`. A failed status invocation renders a
 * blocking sanitized error instead of either workflow, so a broken
 * qualification build can never silently fall back to the normal device/ADB
 * workflow.
 */
function Startup() {
  const [state, setState] = useState<StartupState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    api
      .phase6d6UiSmokeStatus()
      .then((status) => {
        if (cancelled) return;
        setState(status.enabled ? { kind: "qualification", status } : { kind: "app" });
      })
      .catch(() => {
        if (!cancelled) setState({ kind: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (state.kind === "loading") {
    return (
      <main className="startup" role="status">
        <p className="eyebrow">EmuChef</p>
        <h1>Loading…</h1>
      </main>
    );
  }
  if (state.kind === "error") {
    return (
      <main className="startup" role="alert">
        <p className="eyebrow">EmuChef</p>
        <h1>EmuChef could not start safely</h1>
        <p className="warning">
          Startup mode could not be verified. Restart the app; if this persists, contact support
          with the current build version.
        </p>
      </main>
    );
  }
  if (state.kind === "qualification") {
    return <Phase6d6UiSmoke status={state.status} />;
  }
  return (
    <FrontendErrorBoundary onActivate={() => dialogController.cancelActive()}>
      <App dialogController={dialogController} />
    </FrontendErrorBoundary>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Startup />
  </StrictMode>,
);

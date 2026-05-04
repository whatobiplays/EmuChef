import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";

import {
  sidecarApplyRecipeCommand,
  sidecarEmitYaml,
  sidecarGetDocument,
  sidecarListStepSpecs,
  sidecarOpenRecipe,
  sidecarRedo,
  sidecarSaveRecipe,
  sidecarStatus,
  sidecarUndo,
  sidecarValidate,
  type EditorApiResult,
} from "./api/editorApi";
import type {
  DiagnosticDto,
  RecipeDocumentDto,
  SidecarStatusResult,
  StepSpecDto,
} from "./api/types";
import { AppShell } from "./components/AppShell";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { EmptyState } from "./components/EmptyState";
import { ErrorBanner } from "./components/ErrorBanner";
import { LoadingState } from "./components/LoadingState";
import { RecipeSummary } from "./components/RecipeSummary";
import { Sidebar } from "./components/Sidebar";
import { StepSpecsPanel } from "./components/StepSpecsPanel";
import { Toolbar } from "./components/Toolbar";
import { YamlPreview } from "./components/YamlPreview";

export default function App() {
  const [stepSpecs, setStepSpecs] = useState<StepSpecDto[]>([]);
  const [stepSpecsLoaded, setStepSpecsLoaded] = useState(false);
  const [stepSpecsLoading, setStepSpecsLoading] = useState(true);
  const [currentDocument, setCurrentDocument] = useState<RecipeDocumentDto | null>(null);
  const [currentPath, setCurrentPath] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticDto[]>([]);
  const [yaml, setYaml] = useState("");
  const [sidecarState, setSidecarState] = useState<SidecarStatusResult | null>(null);
  const [loadingLabel, setLoadingLabel] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadSpecs() {
      setStepSpecsLoading(true);
      const initialStatus = await sidecarStatus();
      if (!cancelled) {
        handleStatusResponse(initialStatus);
      }
      const response = await sidecarListStepSpecs();
      if (cancelled) {
        return;
      }
      if (response.kind === "success") {
        setStepSpecs(response.result.stepSpecs);
        setStepSpecsLoaded(true);
      } else {
        setErrorMessage(resultMessage(response, "Step specs failed to load."));
      }
      const finalStatus = await sidecarStatus();
      if (!cancelled) {
        handleStatusResponse(finalStatus);
      }
      setStepSpecsLoading(false);
    }

    void loadSpecs();

    return () => {
      cancelled = true;
    };
  }, []);

  const stepSpecsCount = useMemo(() => {
    if (!stepSpecsLoaded) {
      return null;
    }
    return stepSpecs.length;
  }, [stepSpecs.length, stepSpecsLoaded]);

  async function handleOpenRecipe() {
    let selected: string | string[] | null;
    try {
      selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "YAML recipes", extensions: ["yaml", "yml"] }],
      });
    } catch (error) {
      setErrorMessage(`File dialog failed: ${errorMessageFromUnknown(error)}`);
      return;
    }

    if (selected === null) {
      return;
    }

    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) {
      return;
    }

    setLoadingLabel("Opening recipe");
    const response = await sidecarOpenRecipe(path, null);
    if (response.kind === "success") {
      applyDocument(response.result.document, path);
      setErrorMessage(null);
      setStatusMessage(null);
    } else {
      setErrorMessage(resultMessage(response, "Recipe failed to open."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleRefreshDocument() {
    if (currentDocument === null) {
      return;
    }

    setLoadingLabel("Refreshing document");
    const response = await sidecarGetDocument(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage("Document refreshed from the sidecar session.");
    } else {
      setErrorMessage(resultMessage(response, "Document refresh failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleValidate() {
    if (currentDocument === null) {
      return;
    }

    setLoadingLabel("Validating recipe");
    const response = await sidecarValidate(currentDocument.documentId);
    if (response.kind === "success") {
      setDiagnostics(response.result.diagnostics);
      setErrorMessage(null);
      setStatusMessage("Validation refreshed from the sidecar session.");
    } else {
      setErrorMessage(resultMessage(response, "Validation failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleRefreshYaml() {
    if (currentDocument === null) {
      return;
    }

    setLoadingLabel("Refreshing YAML");
    const response = await sidecarEmitYaml(currentDocument.documentId);
    if (response.kind === "success") {
      setYaml(response.result.yaml);
      setErrorMessage(null);
      setStatusMessage("YAML refreshed from the sidecar session.");
    } else {
      setErrorMessage(resultMessage(response, "YAML refresh failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleUndo() {
    if (currentDocument === null) {
      return;
    }

    setLoadingLabel("Undoing change");
    const response = await sidecarUndo(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage(response.result.commandResult.changed ? "Undo applied." : "Nothing to undo.");
    } else {
      setErrorMessage(resultMessage(response, "Undo failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleRedo() {
    if (currentDocument === null) {
      return;
    }

    setLoadingLabel("Redoing change");
    const response = await sidecarRedo(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage(response.result.commandResult.changed ? "Redo applied." : "Nothing to redo.");
    } else {
      setErrorMessage(resultMessage(response, "Redo failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleSave() {
    if (currentDocument === null) {
      return;
    }
    const confirmed = window.confirm(
      "Phase 3A Save writes canonical YAML to the currently open file. Use only with a safe or temporary recipe copy. Continue?",
    );
    if (!confirmed) {
      return;
    }

    setLoadingLabel("Saving recipe");
    const response = await sidecarSaveRecipe(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage("Saved the current sidecar document.");
    } else {
      setErrorMessage(resultMessage(response, "Save failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  async function handleApplyDebugRename() {
    if (currentDocument === null) {
      return;
    }
    const confirmed = window.confirm(
      "Debug-only rename changes the in-memory recipe name and does not save. Use a safe or temporary recipe copy if you plan to save. Continue?",
    );
    if (!confirmed) {
      return;
    }

    setLoadingLabel("Applying debug rename");
    const response = await sidecarApplyRecipeCommand(currentDocument.documentId, {
      type: "SetOverviewField",
      field: "name",
      value: `DEBUG Sidecar Rename ${new Date().toISOString()}`,
    });
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage("Debug rename applied in memory. Save was not run.");
    } else {
      setErrorMessage(resultMessage(response, "Debug rename failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
    }
    setLoadingLabel(null);
  }

  function applyDocument(document: RecipeDocumentDto, fallbackPath: string | null = null) {
    setCurrentDocument(document);
    setCurrentPath(document.path || fallbackPath || currentPath);
    setDiagnostics(document.diagnostics);
    setYaml(document.yaml);
  }

  async function refreshSidecarStatus() {
    const response = await sidecarStatus();
    handleStatusResponse(response);
  }

  function handleStatusResponse(response: EditorApiResult<SidecarStatusResult>) {
    if (response.kind === "success") {
      setSidecarState(response.result);
    } else {
      setErrorMessage(resultMessage(response, "Sidecar status unavailable."));
    }
  }

  return (
    <AppShell
      toolbar={
        <>
          <Toolbar
            canRedo={currentDocument?.canRedo ?? false}
            canUndo={currentDocument?.canUndo ?? false}
            currentPath={currentPath}
            hasDocument={currentDocument !== null}
            loadingLabel={loadingLabel}
            sidecarStatus={sidecarState}
            stepSpecsCount={stepSpecsCount}
            stepSpecsLoading={stepSpecsLoading}
            onApplyDebugRename={handleApplyDebugRename}
            onOpenRecipe={handleOpenRecipe}
            onRedo={handleRedo}
            onRefreshDocument={handleRefreshDocument}
            onRefreshYaml={handleRefreshYaml}
            onSave={handleSave}
            onUndo={handleUndo}
            onValidate={handleValidate}
          />
          {errorMessage ? <ErrorBanner message={errorMessage} onDismiss={() => setErrorMessage(null)} /> : null}
          {statusMessage ? (
            <div className="border-b border-emerald-200 bg-emerald-50 px-4 py-2 text-sm text-emerald-800">
              {statusMessage}
            </div>
          ) : null}
          {loadingLabel ? <LoadingState label={loadingLabel} /> : null}
        </>
      }
      sidebar={<Sidebar document={currentDocument} stepSpecsCount={stepSpecsCount} />}
      rightPanel={
        <div className="flex h-full min-h-0 flex-col">
          <DiagnosticsPanel diagnostics={diagnostics} />
          <YamlPreview yaml={yaml} />
        </div>
      }
    >
      {currentDocument ? <RecipeSummary document={currentDocument} /> : <EmptyState />}
      <StepSpecsPanel stepSpecs={stepSpecs} />
    </AppShell>
  );
}

function resultMessage<T>(result: Exclude<EditorApiResult<T>, { kind: "success" }>, fallback: string) {
  if (result.kind === "api-error") {
    return `${fallback} ${result.error.code}: ${result.error.message}`;
  }
  return `${fallback} ${result.message}`;
}

function errorMessageFromUnknown(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}

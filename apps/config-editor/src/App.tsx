import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";

import {
  emitRecipeYamlFromPath,
  listStepSpecs,
  openRecipe,
  validateRecipePath,
  type EditorApiResult,
} from "./api/editorApi";
import type {
  DiagnosticDto,
  RecipeDocumentDto,
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
  const [loadingLabel, setLoadingLabel] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadSpecs() {
      setStepSpecsLoading(true);
      const response = await listStepSpecs();
      if (cancelled) {
        return;
      }
      if (response.kind === "success") {
        setStepSpecs(response.result.stepSpecs);
        setStepSpecsLoaded(true);
      } else {
        setErrorMessage(resultMessage(response, "Step specs failed to load."));
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
    const response = await openRecipe(path, null);
    if (response.kind === "success") {
      const document = response.result.document;
      setCurrentDocument(document);
      setCurrentPath(document.path || path);
      setDiagnostics(document.diagnostics);
      setYaml(document.yaml);
      setErrorMessage(null);
    } else {
      setErrorMessage(resultMessage(response, "Recipe failed to open."));
    }
    setLoadingLabel(null);
  }

  async function handleValidate() {
    if (currentPath === null) {
      return;
    }

    setLoadingLabel("Validating recipe");
    const response = await validateRecipePath(currentPath, null);
    if (response.kind === "success") {
      setDiagnostics(response.result.diagnostics);
      setErrorMessage(null);
    } else {
      setErrorMessage(resultMessage(response, "Validation failed."));
    }
    setLoadingLabel(null);
  }

  async function handleRefreshYaml() {
    if (currentPath === null) {
      return;
    }

    setLoadingLabel("Refreshing YAML");
    const response = await emitRecipeYamlFromPath(currentPath, null);
    if (response.kind === "success") {
      setYaml(response.result.yaml);
      setErrorMessage(null);
    } else {
      setErrorMessage(resultMessage(response, "YAML refresh failed."));
    }
    setLoadingLabel(null);
  }

  return (
    <AppShell
      toolbar={
        <>
          <Toolbar
            currentPath={currentPath}
            hasDocument={currentDocument !== null}
            loadingLabel={loadingLabel}
            stepSpecsCount={stepSpecsCount}
            stepSpecsLoading={stepSpecsLoading}
            onOpenRecipe={handleOpenRecipe}
            onRefreshYaml={handleRefreshYaml}
            onValidate={handleValidate}
          />
          {errorMessage ? <ErrorBanner message={errorMessage} onDismiss={() => setErrorMessage(null)} /> : null}
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

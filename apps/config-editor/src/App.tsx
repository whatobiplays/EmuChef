import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";

import type { EditorCommand } from "./api/commands";
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
  updateMenuState,
  type EditorApiResult,
} from "./api/editorApi";
import type {
  DiagnosticDto,
  RecipeDocumentDto,
  SidecarStatusResult,
  StepSpecDto,
} from "./api/types";
import { AppShell } from "./components/AppShell";
import { ArtifactGroupsEditor } from "./components/ArtifactGroupsEditor";
import { ArtifactsEditor } from "./components/ArtifactsEditor";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { EmptyState } from "./components/EmptyState";
import { ErrorBanner } from "./components/ErrorBanner";
import { InputsEditor } from "./components/InputsEditor";
import { MenuEventBridge, type MenuAction } from "./components/MenuEventBridge";
import { OverviewEditor } from "./components/OverviewEditor";
import { ConfirmDialog, TextPromptDialog } from "./components/PromptDialog";
import { Sidebar, type EditorView } from "./components/Sidebar";
import { StepSpecsPanel } from "./components/StepSpecsPanel";
import { StepsEditor } from "./components/StepsEditor";
import { Toolbar } from "./components/Toolbar";
import { YamlPreview } from "./components/YamlPreview";

interface TextPromptRequest {
  title: string;
  label: string;
  initialValue: string;
  requiredMessage: string;
  confirmLabel?: string;
  trimResult: boolean;
  resolve: (value: string | null) => void;
}

interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel?: string;
  destructive?: boolean;
  resolve: (confirmed: boolean) => void;
}

interface ConfirmActionOptions {
  confirmLabel?: string;
  destructive?: boolean;
}

export default function App() {
  const [stepSpecs, setStepSpecs] = useState<StepSpecDto[]>([]);
  const [stepSpecsLoaded, setStepSpecsLoaded] = useState(false);
  const [stepSpecsLoading, setStepSpecsLoading] = useState(true);
  const [currentDocument, setCurrentDocument] = useState<RecipeDocumentDto | null>(null);
  const [currentPath, setCurrentPath] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticDto[]>([]);
  const [yaml, setYaml] = useState("");
  const [sidecarState, setSidecarState] = useState<SidecarStatusResult | null>(null);
  const [activeView, setActiveView] = useState<EditorView>("overview");
  const [loadingLabel, setLoadingLabel] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [textPrompt, setTextPrompt] = useState<TextPromptRequest | null>(null);
  const [confirmPrompt, setConfirmPrompt] = useState<ConfirmRequest | null>(null);

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

  useEffect(() => {
    void syncMenuState(currentDocument);
  }, [currentDocument]);

  const stepSpecsCount = useMemo(() => {
    if (!stepSpecsLoaded) {
      return null;
    }
    return stepSpecs.length;
  }, [stepSpecs.length, stepSpecsLoaded]);

  const menuHandlers: Record<MenuAction, () => void> = {
    openRecipe: () => void openRecipe(),
    saveRecipe: () => void saveRecipe(),
    undo: () => void undo(),
    redo: () => void redo(),
    validate: () => void validate(),
    refreshYaml: () => void refreshYaml(),
    refreshDocument: () => void refreshDocument(),
    applyDebugRename: () => void applyDebugRename(),
  };

  async function openRecipe() {
    if (currentDocument?.dirty) {
      const confirmed = await confirmAction(
        "Discard unsaved changes",
        `Discard unsaved changes to ${currentDocument.recipe.id}?`,
        { confirmLabel: "Discard", destructive: true },
      );
      if (!confirmed) {
        await syncMenuState(currentDocument);
        return;
      }
    }

    let selected: string | string[] | null;
    try {
      selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "YAML recipes", extensions: ["yaml", "yml"] }],
      });
    } catch (error) {
      setErrorMessage(`File dialog failed: ${errorMessageFromUnknown(error)}`);
      setStatusMessage(null);
      await syncMenuState(currentDocument);
      return;
    }

    if (selected === null) {
      await syncMenuState(currentDocument);
      return;
    }

    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) {
      await syncMenuState(currentDocument);
      return;
    }

    setLoadingLabel("Opening recipe");
    const response = await sidecarOpenRecipe(path, null);
    if (response.kind === "success") {
      applyDocument(response.result.document, path);
      setActiveView("overview");
      setErrorMessage(null);
      setStatusMessage(null);
      await syncMenuState(response.result.document);
    } else {
      setErrorMessage(resultMessage(response, "Recipe failed to open."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function refreshDocument() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Refreshing document");
    const response = await sidecarGetDocument(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage("Document refreshed from the sidecar session.");
      await syncMenuState(response.result.document);
    } else {
      setErrorMessage(resultMessage(response, "Document refresh failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function validate() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Validating recipe");
    const response = await sidecarValidate(currentDocument.documentId);
    if (response.kind === "success") {
      setDiagnostics(response.result.diagnostics);
      setErrorMessage(null);
      setStatusMessage("Validation refreshed from the sidecar session.");
      await syncMenuState(currentDocument);
    } else {
      setErrorMessage(resultMessage(response, "Validation failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function refreshYaml() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Refreshing YAML");
    const response = await sidecarEmitYaml(currentDocument.documentId);
    if (response.kind === "success") {
      setYaml(response.result.yaml);
      setErrorMessage(null);
      setStatusMessage("YAML refreshed from the sidecar session.");
      await syncMenuState(currentDocument);
    } else {
      setErrorMessage(resultMessage(response, "YAML refresh failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function undo() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Undoing change");
    const response = await sidecarUndo(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage(response.result.commandResult.changed ? "Undo applied." : "Nothing to undo.");
      await syncMenuState(response.result.document);
    } else {
      setErrorMessage(resultMessage(response, "Undo failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function redo() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Redoing change");
    const response = await sidecarRedo(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage(response.result.commandResult.changed ? "Redo applied." : "Nothing to redo.");
      await syncMenuState(response.result.document);
    } else {
      setErrorMessage(resultMessage(response, "Redo failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function saveRecipe() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }

    setLoadingLabel("Saving recipe");
    const response = await sidecarSaveRecipe(currentDocument.documentId);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      setStatusMessage("Saved the current sidecar document.");
      await syncMenuState(response.result.document);
    } else {
      setErrorMessage(resultMessage(response, "Save failed."));
      setStatusMessage(null);
      await refreshSidecarStatus();
      await syncMenuState(currentDocument);
    }
    setLoadingLabel(null);
  }

  async function applyDebugRename() {
    if (currentDocument === null) {
      await syncMenuState(null);
      return;
    }
    const confirmed = await confirmAction(
      "Debug rename",
      "Debug-only rename changes the in-memory recipe name and does not save. Continue?",
      { confirmLabel: "Continue" },
    );
    if (!confirmed) {
      await syncMenuState(currentDocument);
      return;
    }

    await applyCommand({
      type: "SetOverviewField",
      field: "name",
      value: `DEBUG Sidecar Rename ${new Date().toISOString()}`,
    });
  }

  async function applyCommand(command: EditorCommand): Promise<boolean> {
    if (currentDocument === null) {
      await syncMenuState(null);
      return false;
    }

    const response = await sidecarApplyRecipeCommand(currentDocument.documentId, command);
    if (response.kind === "success") {
      applyDocument(response.result.document);
      setErrorMessage(null);
      await syncMenuState(response.result.document);
      return true;
    }

    setErrorMessage(resultMessage(response, "Edit failed."));
    setStatusMessage(null);
    await refreshSidecarStatus();
    await syncMenuState(currentDocument);
    return false;
  }

  function applyDocument(document: RecipeDocumentDto, fallbackPath: string | null = null) {
    setCurrentDocument(document);
    setCurrentPath(document.path || fallbackPath || null);
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

  async function syncMenuState(document: RecipeDocumentDto | null) {
    try {
      await updateMenuState({
        hasDocument: document !== null,
        dirty: document?.dirty ?? false,
        canUndo: document?.canUndo ?? false,
        canRedo: document?.canRedo ?? false,
      });
    } catch (error) {
      setErrorMessage(`Menu state update failed: ${errorMessageFromUnknown(error)}`);
    }
  }

  function promptForId(title: string, initialValue: string): Promise<string | null> {
    return promptForText({
      title,
      label: "ID",
      initialValue,
      requiredMessage: "ID must not be empty.",
      trimResult: true,
    });
  }

  function promptForRequiredText(title: string, initialValue: string, label: string): Promise<string | null> {
    return promptForText({
      title,
      label,
      initialValue,
      requiredMessage: `${label} must not be empty.`,
      trimResult: false,
    });
  }

  function promptForText(options: Omit<TextPromptRequest, "resolve">): Promise<string | null> {
    return new Promise((resolve) => {
      setTextPrompt({ ...options, resolve });
    });
  }

  function confirmAction(title: string, message: string, options: ConfirmActionOptions = {}): Promise<boolean> {
    return new Promise((resolve) => {
      setConfirmPrompt({
        title,
        message,
        confirmLabel: options.confirmLabel,
        destructive: options.destructive,
        resolve,
      });
    });
  }

  function resolveTextPrompt(value: string | null) {
    if (textPrompt === null) {
      return;
    }
    const resolver = textPrompt.resolve;
    setTextPrompt(null);
    resolver(value);
  }

  function resolveConfirmPrompt(confirmed: boolean) {
    if (confirmPrompt === null) {
      return;
    }
    const resolver = confirmPrompt.resolve;
    setConfirmPrompt(null);
    resolver(confirmed);
  }

  function renderMainContent() {
    if (activeView === "stepSpecs") {
      return <StepSpecsPanel stepSpecs={stepSpecs} />;
    }
    if (currentDocument === null) {
      return <EmptyState />;
    }
    switch (activeView) {
      case "overview":
        return <OverviewEditor document={currentDocument} onCommand={applyCommand} />;
      case "inputs":
        return (
          <InputsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            onCommand={applyCommand}
          />
        );
      case "artifacts":
        return (
          <ArtifactsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            promptForRequiredText={promptForRequiredText}
            onCommand={applyCommand}
          />
        );
      case "artifactGroups":
        return (
          <ArtifactGroupsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            onCommand={applyCommand}
          />
        );
      case "steps":
        return (
          <StepsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            stepSpecs={stepSpecs}
            onCommand={applyCommand}
          />
        );
    }
  }

  return (
    <>
      {textPrompt ? (
        <TextPromptDialog
          confirmLabel={textPrompt.confirmLabel}
          initialValue={textPrompt.initialValue}
          label={textPrompt.label}
          requiredMessage={textPrompt.requiredMessage}
          title={textPrompt.title}
          trimResult={textPrompt.trimResult}
          onCancel={() => resolveTextPrompt(null)}
          onSubmit={resolveTextPrompt}
        />
      ) : null}
      {confirmPrompt ? (
        <ConfirmDialog
          confirmLabel={confirmPrompt.confirmLabel}
          destructive={confirmPrompt.destructive}
          message={confirmPrompt.message}
          title={confirmPrompt.title}
          onCancel={() => resolveConfirmPrompt(false)}
          onConfirm={() => resolveConfirmPrompt(true)}
        />
      ) : null}
      <MenuEventBridge handlers={menuHandlers} />
      <AppShell
        toolbar={
          <>
            <Toolbar
              currentPath={currentPath}
              dirty={currentDocument?.dirty ?? false}
              hasDocument={currentDocument !== null}
              loadingLabel={loadingLabel}
              sidecarStatus={sidecarState}
              stepSpecsCount={stepSpecsCount}
              stepSpecsLoading={stepSpecsLoading}
            />
            {errorMessage ? <ErrorBanner message={errorMessage} onDismiss={() => setErrorMessage(null)} /> : null}
            {statusMessage ? (
              <div className="border-b border-emerald-200 bg-emerald-50 px-4 py-2 text-sm text-emerald-800">
                {statusMessage}
              </div>
            ) : null}
          </>
        }
        sidebar={
          <Sidebar
            activeView={activeView}
            document={currentDocument}
            stepSpecsCount={stepSpecsCount}
            onSelectView={setActiveView}
          />
        }
        rightPanel={
          <div className="flex h-full min-h-0 flex-col">
            <DiagnosticsPanel diagnostics={diagnostics} />
            <YamlPreview yaml={yaml} />
          </div>
        }
      >
        {renderMainContent()}
      </AppShell>
    </>
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

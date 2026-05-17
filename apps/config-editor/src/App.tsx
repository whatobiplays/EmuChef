import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm as nativeConfirm, open, save as saveFile } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";

import type { EditorCommand } from "./api/commands";
import {
  sidecarApplyRecipeCommand,
  sidecarEmitYaml,
  sidecarListStepSpecs,
  sidecarOpenRecipe,
  sidecarRedo,
  sidecarSaveRecipe,
  sidecarSaveRecipeAs,
  sidecarSetDocumentAuthoredRoot,
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
import {
  beginCommand,
  buildCloseConfirmationCopy,
  buildActionAvailability,
  classifySidecarStatus,
  classifyOperationFailure,
  decideCloseRequest,
  invalidSessionMessage,
  resolveAuthoredRootSelectionAttempt,
  resolveClosePromptResult,
  resolveOpenAttempt,
  type CommandName,
} from "./components/phase5EditorState.logic";
import { TextPromptDialog } from "./components/PromptDialog";
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

interface ConfirmActionOptions {
  confirmLabel?: string;
  destructive?: boolean;
}

interface CommandApplyOutcome {
  ok: boolean;
  changed: boolean;
}

export default function App() {
  const [stepSpecs, setStepSpecs] = useState<StepSpecDto[]>([]);
  const [stepSpecsLoaded, setStepSpecsLoaded] = useState(false);
  const [stepSpecsLoading, setStepSpecsLoading] = useState(true);
  const [currentDocument, setCurrentDocument] = useState<RecipeDocumentDto | null>(null);
  const [currentPath, setCurrentPath] = useState<string | null>(null);
  const [selectedAuthoredRoot, setSelectedAuthoredRoot] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticDto[]>([]);
  const [yaml, setYaml] = useState("");
  const [sidecarState, setSidecarState] = useState<SidecarStatusResult | null>(null);
  const [documentSessionValid, setDocumentSessionValid] = useState(true);
  const [sessionInvalidReason, setSessionInvalidReason] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<EditorView>("overview");
  const [commandInFlight, setCommandInFlight] = useState<CommandName | null>(null);
  const [loadingLabel, setLoadingLabel] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [textPrompt, setTextPrompt] = useState<TextPromptRequest | null>(null);

  const currentDocumentRef = useRef<RecipeDocumentDto | null>(null);
  const selectedAuthoredRootRef = useRef<string | null>(null);
  const commandInFlightRef = useRef<CommandName | null>(null);
  const documentSessionValidRef = useRef(true);
  const sessionInvalidReasonRef = useRef<string | null>(null);
  const promptActiveRef = useRef(false);
  // Confirmed window closes are reissued through Tauri, which emits one more close-request event.
  // This guard lets only that intentional second event pass without reopening the confirmation.
  const allowCloseRef = useRef(false);
  const closePromptOpenRef = useRef(false);

  const actionAvailability = useMemo(
    () =>
      buildActionAvailability({
        hasDocument: currentDocument !== null,
        hasSelectedAuthoredRoot: selectedAuthoredRoot !== null,
        hasDocumentAuthoredRoot: currentDocument?.authoredRoot !== null && currentDocument?.authoredRoot !== undefined,
        dirty: currentDocument?.dirty ?? false,
        canUndo: currentDocument?.canUndo ?? false,
        canRedo: currentDocument?.canRedo ?? false,
        commandInFlight,
        documentSessionValid,
        backendCompatible: sidecarState?.compatible ?? null,
      }),
    [commandInFlight, currentDocument, documentSessionValid, selectedAuthoredRoot, sidecarState?.compatible],
  );

  const stepSpecsCount = useMemo(() => {
    if (!stepSpecsLoaded) {
      return null;
    }
    return stepSpecs.length;
  }, [stepSpecs.length, stepSpecsLoaded]);

  useEffect(() => {
    currentDocumentRef.current = currentDocument;
  }, [currentDocument]);

  useEffect(() => {
    selectedAuthoredRootRef.current = selectedAuthoredRoot;
  }, [selectedAuthoredRoot]);

  useEffect(() => {
    commandInFlightRef.current = commandInFlight;
  }, [commandInFlight]);

  useEffect(() => {
    documentSessionValidRef.current = documentSessionValid;
  }, [documentSessionValid]);

  useEffect(() => {
    sessionInvalidReasonRef.current = sessionInvalidReason;
  }, [sessionInvalidReason]);

  useEffect(() => {
    void syncMenuState(currentDocument, commandInFlight, documentSessionValid);
  }, [commandInFlight, currentDocument, documentSessionValid, selectedAuthoredRoot, sidecarState?.compatible]);

  useEffect(() => {
    if (!statusMessage) {
      return;
    }
    const timer = window.setTimeout(() => setStatusMessage(null), 4000);
    return () => window.clearTimeout(timer);
  }, [statusMessage]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;
    const windowHandle = getCurrentWindow();

    void windowHandle.onCloseRequested(async (event) => {
      if (allowCloseRef.current) {
        allowCloseRef.current = false;
        return;
      }

      const document = currentDocumentRef.current;
      const decision = decideCloseRequest({
        commandInFlight: commandInFlightRef.current,
        dirty: document?.dirty ?? false,
        promptInFlight: closePromptOpenRef.current || promptActiveRef.current,
      });

      if (decision.kind === "allow") {
        return;
      }

      event.preventDefault();

      if (decision.kind === "prevent") {
        return;
      }

      closePromptOpenRef.current = true;
      try {
        const copy = buildCloseConfirmationCopy(decision.reason, document?.recipe.id ?? null);
        const confirmed = await confirmAction(copy.title, copy.message, {
          destructive: decision.reason !== "command-in-flight",
        });
        if (resolveClosePromptResult(confirmed).kind === "allow") {
          allowCloseRef.current = true;
          try {
            await windowHandle.close();
            // If Tauri accepts the close command but the follow-up close event never arrives,
            // clear the guard so a later unrelated close request cannot bypass confirmation.
            window.setTimeout(() => {
              allowCloseRef.current = false;
            }, 1000);
          } catch (error) {
            allowCloseRef.current = false;
            setErrorMessage(`Window close failed: ${errorMessageFromUnknown(error)}`);
            setStatusMessage(null);
          }
        }
      } finally {
        closePromptOpenRef.current = false;
      }
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      cleanup = unlisten;
    });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

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
        handleOperationFailure(response, "Step specs failed to load.");
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

  const menuHandlers: Record<MenuAction, () => void> = {
    openRecipe: () => void openRecipe(),
    saveRecipe: () => void saveRecipe(),
    saveRecipeAs: () => void saveRecipeAs(),
    undo: () => void undo(),
    redo: () => void redo(),
    validate: () => void validate(),
    refreshYaml: () => void refreshYaml(),
    setAuthoredRoot: () => void setAuthoredRootFromDialog(),
    clearAuthoredRoot: () => void clearAuthoredRoot(),
  };

  async function openRecipe() {
    if (!documentSessionValidRef.current) {
      showInvalidSessionMessage();
      await syncMenuState(currentDocumentRef.current);
      return;
    }
    if (commandInFlightRef.current !== null) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const current = currentDocumentRef.current;
    if (current?.dirty) {
      const confirmed = await confirmAction(
        "Discard unsaved changes",
        `Discard unsaved changes to ${current.recipe.id}?`,
        { confirmLabel: "Discard", destructive: true },
      );
      if (!confirmed) {
        await syncMenuState(currentDocumentRef.current);
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
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) {
      const resolution = resolveOpenAttempt(currentDocumentRef.current, { kind: "picker-cancelled" });
      setCurrentDocument(resolution.document);
      await syncMenuState(resolution.document);
      return;
    }

    if (!beginAppCommand("openRecipe", "Opening recipe")) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }
    try {
      const response = await sidecarOpenRecipe(path, selectedAuthoredRootRef.current);
      if (response.kind === "success") {
        applyDocument(response.result.document, path);
        setDocumentSessionValid(true);
        setSessionInvalidReason(null);
        setActiveView("overview");
        setErrorMessage(null);
        setStatusMessage(null);
        await syncMenuState(response.result.document, commandInFlightRef.current, true);
      } else {
        const classification = handleOperationFailure(response, "Recipe failed to open.");
        const resolution = resolveOpenAttempt(currentDocumentRef.current, {
          kind: "open-failed",
          sessionInvalid: classification.sessionInvalid,
        });
        if (!resolution.sessionValid) {
          markDocumentSessionInvalid(classification.message);
        }
        await refreshSidecarStatus();
        await syncMenuState(resolution.document, commandInFlightRef.current, resolution.sessionValid);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function setAuthoredRootFromDialog() {
    if (!actionAvailability.setAuthoredRoot) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    let selected: string | string[] | null;
    try {
      selected = await open({
        multiple: false,
        directory: true,
      });
    } catch (error) {
      setErrorMessage(`Directory dialog failed: ${errorMessageFromUnknown(error)}`);
      setStatusMessage(null);
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    await updateAuthoredRootSelection(path);
  }

  async function clearAuthoredRoot() {
    if (!actionAvailability.clearAuthoredRoot) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    await updateAuthoredRootSelection(null);
  }

  async function updateAuthoredRootSelection(nextAuthoredRoot: string | null) {
    const document = currentDocumentRef.current;
    if (document === null) {
      const resolution = resolveAuthoredRootSelectionAttempt(selectedAuthoredRootRef.current, null, {
        kind: "no-document",
        authoredRoot: nextAuthoredRoot,
      });
      selectedAuthoredRootRef.current = resolution.selectedAuthoredRoot;
      setSelectedAuthoredRoot(resolution.selectedAuthoredRoot);
      setErrorMessage(null);
      setStatusMessage(nextAuthoredRoot === null ? "Authored root selection cleared." : "Authored root selected.");
      await syncMenuState(null);
      return;
    }

    if (!documentSessionValidRef.current) {
      showInvalidSessionMessage();
      await syncMenuState(document);
      return;
    }

    if (!beginAppCommand("setAuthoredRoot", "Updating authored root")) {
      await syncMenuState(document);
      return;
    }
    try {
      const response = await sidecarSetDocumentAuthoredRoot(document.documentId, nextAuthoredRoot);
      if (response.kind === "success") {
        const resolution = resolveAuthoredRootSelectionAttempt(selectedAuthoredRootRef.current, document, {
          kind: "updated",
          authoredRoot: nextAuthoredRoot,
          document: response.result.document,
        });
        selectedAuthoredRootRef.current = resolution.selectedAuthoredRoot;
        setSelectedAuthoredRoot(resolution.selectedAuthoredRoot);
        if (resolution.document !== null) {
          applyDocument(resolution.document);
        }
        setErrorMessage(null);
        setStatusMessage(nextAuthoredRoot === null ? "Authored root cleared." : "Authored root updated.");
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Authored root update failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function validate() {
    const document = beginDocumentCommand("validate", "Validating recipe", "validate");
    if (document === null) {
      return;
    }
    try {
      const response = await sidecarValidate(document.documentId);
      if (response.kind === "success") {
        setDiagnostics(response.result.diagnostics);
        setErrorMessage(null);
        setStatusMessage("Validation refreshed from the sidecar session.");
        await syncMenuState(currentDocumentRef.current);
      } else {
        handleOperationFailure(response, "Validation failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function refreshYaml() {
    const document = beginDocumentCommand("refreshYaml", "Refreshing YAML", "refreshYaml");
    if (document === null) {
      return;
    }
    try {
      const response = await sidecarEmitYaml(document.documentId);
      if (response.kind === "success") {
        setYaml(response.result.yaml);
        setErrorMessage(null);
        setStatusMessage("YAML refreshed from the sidecar session.");
        await syncMenuState(currentDocumentRef.current);
      } else {
        handleOperationFailure(response, "YAML refresh failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function undo() {
    const document = beginDocumentCommand("undo", "Undoing change", "undo");
    if (document === null) {
      return;
    }
    try {
      const response = await sidecarUndo(document.documentId);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        setStatusMessage(response.result.commandResult.changed ? "Undo applied." : "Nothing to undo.");
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Undo failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function redo() {
    const document = beginDocumentCommand("redo", "Redoing change", "redo");
    if (document === null) {
      return;
    }
    try {
      const response = await sidecarRedo(document.documentId);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        setStatusMessage(response.result.commandResult.changed ? "Redo applied." : "Nothing to redo.");
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Redo failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function saveRecipe() {
    const document = beginDocumentCommand("saveRecipe", "Saving recipe", "saveRecipe");
    if (document === null) {
      return;
    }
    try {
      const response = await sidecarSaveRecipe(document.documentId);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        setStatusMessage("Saved the current sidecar document.");
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Save failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function saveRecipeAs() {
    const document = currentDocumentRef.current;
    if (document === null) {
      await syncMenuState(null);
      return;
    }
    if (!documentSessionValidRef.current) {
      showInvalidSessionMessage();
      await syncMenuState(document);
      return;
    }

    const availability = buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: selectedAuthoredRootRef.current !== null,
      hasDocumentAuthoredRoot: document.authoredRoot !== null,
      dirty: document.dirty,
      canUndo: document.canUndo,
      canRedo: document.canRedo,
      commandInFlight: commandInFlightRef.current,
      documentSessionValid: documentSessionValidRef.current,
      backendCompatible: sidecarState?.compatible ?? null,
    });
    if (!availability.saveRecipeAs) {
      await syncMenuState(document);
      return;
    }

    let selectedPath: string | null;
    try {
      selectedPath = await saveFile({
        defaultPath: document.path || `${document.recipe.id || "recipe"}.yaml`,
        filters: [{ name: "YAML recipes", extensions: ["yaml", "yml"] }],
      });
    } catch (error) {
      setErrorMessage(`File dialog failed: ${errorMessageFromUnknown(error)}`);
      setStatusMessage(null);
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    if (!selectedPath) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    if (!beginAppCommand("saveRecipeAs", "Saving recipe as")) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }
    try {
      const response = await sidecarSaveRecipeAs(document.documentId, selectedPath);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        setStatusMessage(null);
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Save As failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        await refreshSidecarStatus();
        await syncMenuState(currentDocumentRef.current);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function applyCommand(command: EditorCommand): Promise<boolean> {
    const outcome = await applyCommandDetailed(command);
    return outcome.ok;
  }

  async function applyCommandDetailed(command: EditorCommand): Promise<CommandApplyOutcome> {
    const document = beginDocumentCommand("mutation", "Applying edit", "editDocument");
    if (document === null) {
      return { ok: false, changed: false };
    }
    try {
      const response = await sidecarApplyRecipeCommand(document.documentId, command);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        await syncMenuState(response.result.document);
        return { ok: true, changed: response.result.commandResult.changed };
      }

      handleOperationFailure(response, "Edit failed.", {
        commandDocumentId: document.documentId,
        currentDocumentId: currentDocumentRef.current?.documentId ?? null,
      });
      await refreshSidecarStatus();
      await syncMenuState(currentDocumentRef.current);
      return { ok: false, changed: false };
    } finally {
      finishAppCommand();
    }
  }

  function beginDocumentCommand(
    command: CommandName,
    label: string,
    action: keyof ReturnType<typeof buildActionAvailability>,
  ): RecipeDocumentDto | null {
    const document = currentDocumentRef.current;
    if (document === null) {
      void syncMenuState(null);
      return null;
    }
    if (!documentSessionValidRef.current) {
      showInvalidSessionMessage();
      void syncMenuState(document);
      return null;
    }
    const availability = buildActionAvailability({
      hasDocument: true,
      hasSelectedAuthoredRoot: selectedAuthoredRootRef.current !== null,
      hasDocumentAuthoredRoot: document.authoredRoot !== null,
      dirty: document.dirty,
      canUndo: document.canUndo,
      canRedo: document.canRedo,
      commandInFlight: commandInFlightRef.current,
      documentSessionValid: documentSessionValidRef.current,
      backendCompatible: sidecarState?.compatible ?? null,
    });
    if (!availability[action]) {
      void syncMenuState(document);
      return null;
    }
    if (!beginAppCommand(command, label)) {
      void syncMenuState(document);
      return null;
    }
    return document;
  }

  function beginAppCommand(command: CommandName, label: string): boolean {
    const result = beginCommand(commandInFlightRef.current, command);
    if (!result.started) {
      return false;
    }
    commandInFlightRef.current = result.commandInFlight;
    setCommandInFlight(result.commandInFlight);
    setLoadingLabel(label);
    setStatusMessage(null);
    return true;
  }

  function finishAppCommand() {
    commandInFlightRef.current = null;
    setCommandInFlight(null);
    setLoadingLabel(null);
  }

  function applyDocument(document: RecipeDocumentDto, fallbackPath: string | null = null) {
    currentDocumentRef.current = document;
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
      const classification = classifySidecarStatus(response.result);
      if (classification.sessionInvalid && classification.message !== null) {
        markDocumentSessionInvalid(classification.message);
        setErrorMessage(classification.message);
        setStatusMessage(null);
      }
      return;
    }

    const classification = handleOperationFailure(response, "Sidecar status unavailable.");
    if (classification.sessionInvalid) {
      markDocumentSessionInvalid(classification.message);
    }
  }

  function handleOperationFailure<T>(
    response: Exclude<EditorApiResult<T>, { kind: "success" }>,
    fallback: string,
    context: Parameters<typeof classifyOperationFailure>[2] = {},
  ) {
    const classification = classifyOperationFailure(response, fallback, context);
    setErrorMessage(classification.message);
    setStatusMessage(null);
    if (classification.sessionInvalid) {
      markDocumentSessionInvalid(classification.message);
    }
    return classification;
  }

  function markDocumentSessionInvalid(message: string) {
    documentSessionValidRef.current = false;
    sessionInvalidReasonRef.current = message;
    setDocumentSessionValid(false);
    setSessionInvalidReason(message);
  }

  function showInvalidSessionMessage() {
    setErrorMessage(sessionInvalidReasonRef.current ?? invalidSessionMessage());
    setStatusMessage(null);
  }

  async function syncMenuState(
    document: RecipeDocumentDto | null,
    menuCommandInFlight: CommandName | null = commandInFlightRef.current,
    menuDocumentSessionValid: boolean = documentSessionValidRef.current,
  ) {
    try {
      await updateMenuState({
        hasDocument: document !== null,
        hasSelectedAuthoredRoot: selectedAuthoredRootRef.current !== null,
        hasDocumentAuthoredRoot: document?.authoredRoot !== null && document?.authoredRoot !== undefined,
        dirty: document?.dirty ?? false,
        canUndo: document?.canUndo ?? false,
        canRedo: document?.canRedo ?? false,
        commandInFlight: menuCommandInFlight !== null,
        documentSessionValid: menuDocumentSessionValid,
        backendCompatible: sidecarState?.compatible ?? null,
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
    if (!actionAvailability.editDocument) {
      showInvalidSessionMessage();
      return Promise.resolve(null);
    }
    if (promptActiveRef.current) {
      return Promise.resolve(null);
    }
    return new Promise((resolve) => {
      promptActiveRef.current = true;
      setTextPrompt({ ...options, resolve });
    });
  }

  function confirmAction(title: string, message: string, options: ConfirmActionOptions = {}): Promise<boolean> {
    if (promptActiveRef.current) {
      return Promise.resolve(false);
    }
    return confirmNativeAction(title, message, options);
  }

  async function confirmNativeAction(
    title: string,
    message: string,
    options: ConfirmActionOptions = {},
  ): Promise<boolean> {
    promptActiveRef.current = true;
    try {
      return await nativeConfirm(message, {
        title,
        kind: options.destructive ? "warning" : "info",
      });
    } catch {
      return window.confirm(`${title}\n\n${message}`);
    } finally {
      promptActiveRef.current = false;
    }
  }

  function resolveTextPrompt(value: string | null) {
    if (textPrompt === null) {
      return;
    }
    const resolver = textPrompt.resolve;
    setTextPrompt(null);
    promptActiveRef.current = false;
    resolver(value);
  }

  function renderMainContent() {
    if (activeView === "stepSpecs") {
      return <StepSpecsPanel stepSpecs={stepSpecs} />;
    }
    if (currentDocument === null) {
      return (
        <EmptyState
          sidecarAvailable={documentSessionValid}
          sidecarMessage={sessionInvalidReason}
        />
      );
    }
    switch (activeView) {
      case "overview":
        return <OverviewEditor document={currentDocument} readOnly={!actionAvailability.editDocument} onCommand={applyCommand} />;
      case "inputs":
        return (
          <InputsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            readOnly={!actionAvailability.editDocument}
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
            readOnly={!actionAvailability.editDocument}
            onCommand={applyCommand}
          />
        );
      case "artifactGroups":
        return (
          <ArtifactGroupsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            readOnly={!actionAvailability.editDocument}
            onCommand={applyCommand}
          />
        );
      case "steps":
        return (
          <StepsEditor
            confirmAction={confirmAction}
            document={currentDocument}
            promptForId={promptForId}
            readOnly={!actionAvailability.editDocument}
            stepSpecs={stepSpecs}
            onCommand={applyCommand}
            onAdvancedCommand={applyCommandDetailed}
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
      <MenuEventBridge handlers={menuHandlers} />
      <AppShell
        toolbar={
          <>
            <Toolbar
              currentPath={currentPath}
              documentAuthoredRoot={currentDocument?.authoredRoot ?? null}
              dirty={currentDocument?.dirty ?? false}
              documentSessionValid={documentSessionValid}
              hasDocument={currentDocument !== null}
              loadingLabel={loadingLabel}
              selectedAuthoredRoot={selectedAuthoredRoot}
              sidecarStatus={sidecarState}
              stepSpecsCount={stepSpecsCount}
              stepSpecsLoading={stepSpecsLoading}
            />
            {!documentSessionValid && currentDocument !== null ? (
              <div className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-900">
                Editor session invalid. The open recipe is read-only for reference; restart the Tauri app and reopen it.
              </div>
            ) : null}
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

function errorMessageFromUnknown(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return JSON.stringify(error);
}

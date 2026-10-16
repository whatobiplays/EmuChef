import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm as nativeConfirm, open, save as saveFile } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AppDefinitionCommand, EditorCommand } from "./api/commands";
import {
  applyAppDefinitionCommand as applyAppDefinitionCommandApi,
  applyRecipeCommand,
  emitYaml,
  getConfigEditorAuthoredRoot,
  getDocument,
  openUserConfiguration as openUserConfigurationDocument,
  listStepSpecs,
  openAppDefinition as openAppDefinitionDocument,
  openRecipe as openRecipeDocument,
  redo as redoDocument,
  sidecarRestart,
  saveAppDefinition as saveAppDefinitionDocument,
  saveAppDefinitionAs as saveAppDefinitionDocumentAs,
  saveRecipe as saveRecipeDocument,
  saveRecipeAs as saveRecipeDocumentAs,
  setConfigEditorAuthoredRoot,
  setDocumentAuthoredRoot,
  sidecarStatus,
  undo as undoDocument,
  validate as validateDocument,
  updateMenuState,
  type EditorApiResult,
} from "./api/editorApi";
import type {
  DiagnosticDto,
  AppDefinitionV1Dto,
  AppGeneratorPublicationOutcomeDto,
  EditorDocumentDto,
  SidecarStatusResult,
  StepSpecDto,
  UserConfigurationDocumentDto,
} from "./api/types";
import { isAppDefinitionDocument } from "./api/types";
import { AppShell } from "./components/AppShell";
import { AppGenerator } from "./components/AppGenerator";
import { AppDefinitionEditor } from "./components/AppDefinitionEditor";
import {
  createAppDefinitionEditorDraft,
  isAppDefinitionEditorDraftDirty,
  rebasePendingAppDefinitionEditorDraft,
  type AppDefinitionEditorDraft,
} from "./components/appDefinitionEditor.logic";
import { ArtifactGroupsEditor } from "./components/ArtifactGroupsEditor";
import { ArtifactsEditor } from "./components/ArtifactsEditor";
import { DiagnosticsPanel } from "./components/DiagnosticsPanel";
import { DeviceProfileGenerator } from "./components/DeviceProfileGenerator";
import { EmptyState } from "./components/EmptyState";
import { ErrorBanner } from "./components/ErrorBanner";
import { InputsEditor } from "./components/InputsEditor";
import { MenuEventBridge, type MenuAction } from "./components/MenuEventBridge";
import { OverviewEditor } from "./components/OverviewEditor";
import {
  beginCommand,
  buildCloseConfirmationCopy,
  buildActionAvailability,
  buildInvalidSessionRecovery,
  classifySidecarStatus,
  classifyOperationFailure,
  classifyRestartFailure,
  decideCloseRequest,
  invalidSessionMessage,
  resolveAuthoredRootSelectionAttempt,
  resolveClosePromptResult,
  resolveOpenAttempt,
  resolveReopenAttempt,
  resolveRestartSuccess,
  saveFailureDiagnosticsForActiveDocument,
  type CommandName,
} from "./components/editorState.logic";
import { TextPromptDialog } from "./components/PromptDialog";
import { Sidebar, type EditorView } from "./components/Sidebar";
import { StepSpecsPanel } from "./components/StepSpecsPanel";
import { StepsEditor } from "./components/StepsEditor";
import { Toolbar } from "./components/Toolbar";
import { UserConfigurationEditor } from "./components/UserConfigurationEditor";
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

function documentName(document: EditorDocumentDto): string {
  return isAppDefinitionDocument(document) ? document.appDefinition.id : document.recipe.id;
}

function documentCanSaveAs(document: EditorDocumentDto | null): boolean {
  return document !== null && (!isAppDefinitionDocument(document) || document.editRestrictions.canSaveAs);
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
  const [currentDocument, setCurrentDocument] = useState<EditorDocumentDto | null>(null);
  const [appDefinitionEditorDraft, setAppDefinitionEditorDraft] = useState<AppDefinitionEditorDraft | null>(null);
  const [appDefinitionDraftDirty, setAppDefinitionDraftDirty] = useState(false);
  const [userConfigurationDocument, setUserConfigurationDocument] = useState<UserConfigurationDocumentDto | null>(null);
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
  const [deviceProfileGeneratorOpen, setDeviceProfileGeneratorOpen] = useState(false);
  const [appGeneratorOpen, setAppGeneratorOpen] = useState(false);

  const currentDocumentRef = useRef<EditorDocumentDto | null>(null);
  const userConfigurationDocumentRef = useRef<UserConfigurationDocumentDto | null>(null);
  const userConfigurationOpenRequestRef = useRef(0);
  const appDefinitionEditorDraftRef = useRef<AppDefinitionEditorDraft | null>(null);
  const appDefinitionDraftDirtyRef = useRef(false);
  const selectedAuthoredRootRef = useRef<string | null>(null);
  const sidecarStateRef = useRef<SidecarStatusResult | null>(null);
  const commandInFlightRef = useRef<CommandName | null>(null);
  const documentSessionValidRef = useRef(true);
  const sessionInvalidReasonRef = useRef<string | null>(null);
  const promptActiveRef = useRef(false);
  const deviceProfileGeneratorOpenRef = useRef(false);
  const appGeneratorOpenRef = useRef(false);
  // Confirmed window closes are reissued through Tauri, which emits one more close-request event.
  // This guard lets only that intentional second event pass without reopening the confirmation.
  const allowCloseRef = useRef(false);
  const closePromptOpenRef = useRef(false);

  const localDraftIsDirty = useCallback(
    (document: EditorDocumentDto | null): boolean =>
      document !== null &&
      isAppDefinitionDocument(document) &&
      currentDocumentRef.current?.documentId === document.documentId &&
      appDefinitionDraftDirtyRef.current,
    [],
  );

  const documentHasUnsavedChanges = useCallback(
    (document: EditorDocumentDto | null): boolean =>
      (document?.dirty ?? false) || localDraftIsDirty(document),
    [localDraftIsDirty],
  );

  const currentActionAvailability = useCallback(
    (
      document: EditorDocumentDto | null,
      sessionValid: boolean = documentSessionValidRef.current,
      command: CommandName | null = commandInFlightRef.current,
    ) => {
      const status = sidecarStateRef.current;
      return buildActionAvailability({
        hasDocument: document !== null,
        hasSelectedAuthoredRoot: selectedAuthoredRootRef.current !== null,
        hasDocumentAuthoredRoot: document?.authoredRoot !== null && document?.authoredRoot !== undefined,
        hasDocumentPath: Boolean(document?.path),
        dirty: documentHasUnsavedChanges(document),
        pendingAppDefinitionInput: localDraftIsDirty(document),
        canSaveAs: documentCanSaveAs(document),
        canUndo: document?.canUndo ?? false,
        canRedo: document?.canRedo ?? false,
        commandInFlight: command,
        documentSessionValid: sessionValid,
        backendCompatible: status?.compatible ?? null,
        sidecarRunning: status?.running ?? null,
      });
    },
    [documentHasUnsavedChanges, localDraftIsDirty],
  );

  const syncMenuState = useCallback(
    async (
      document: EditorDocumentDto | null,
      menuCommandInFlight: CommandName | null = commandInFlightRef.current,
      menuDocumentSessionValid: boolean = documentSessionValidRef.current,
    ) => {
      try {
        const status = sidecarStateRef.current;
        const availability = currentActionAvailability(
          document,
          menuDocumentSessionValid,
          menuCommandInFlight,
        );
        await updateMenuState({
          documentKind: document === null ? null : isAppDefinitionDocument(document) ? "app_definition" : "recipe",
          hasDocument: document !== null,
          hasSelectedAuthoredRoot: selectedAuthoredRootRef.current !== null,
          hasDocumentAuthoredRoot: document?.authoredRoot !== null && document?.authoredRoot !== undefined,
          hasDocumentPath: Boolean(document?.path),
          dirty: documentHasUnsavedChanges(document),
          canSave: availability.saveRecipe,
          canSaveAs: availability.saveRecipeAs,
          pendingAppDefinitionInput: localDraftIsDirty(document),
          canUndo: document?.canUndo ?? false,
          canRedo: document?.canRedo ?? false,
          commandInFlight: menuCommandInFlight !== null,
          documentSessionValid: menuDocumentSessionValid,
          backendCompatible: status?.compatible ?? null,
          sidecarRunning: status?.running ?? null,
          generatorActive: deviceProfileGeneratorOpenRef.current || appGeneratorOpenRef.current,
        });
      } catch (error) {
        setErrorMessage(`Menu state update failed: ${errorMessageFromUnknown(error)}`);
      }
    },
    [currentActionAvailability, documentHasUnsavedChanges, localDraftIsDirty],
  );

  function installAppDefinitionDraft(draft: AppDefinitionEditorDraft | null, dirty = false) {
    appDefinitionEditorDraftRef.current = draft;
    appDefinitionDraftDirtyRef.current = dirty;
    setAppDefinitionEditorDraft(draft);
    setAppDefinitionDraftDirty(dirty);
  }

  const markDocumentSessionInvalid = useCallback((message: string) => {
    documentSessionValidRef.current = false;
    sessionInvalidReasonRef.current = message;
    setDocumentSessionValid(false);
    setSessionInvalidReason(message);
  }, []);

  const handleOperationFailure = useCallback(
    <T,>(
      response: Exclude<EditorApiResult<T>, { kind: "success" }>,
      fallback: string,
      context: Parameters<typeof classifyOperationFailure>[2] = {},
    ) => {
      const classification = classifyOperationFailure(response, fallback, context);
      setErrorMessage(classification.message);
      setStatusMessage(null);
      if (classification.sessionInvalid) {
        markDocumentSessionInvalid(classification.message);
      }
      return classification;
    },
    [markDocumentSessionInvalid],
  );

  const handleStatusResponse = useCallback(
    (response: EditorApiResult<SidecarStatusResult>) => {
      if (response.kind === "success") {
        sidecarStateRef.current = response.result;
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
    },
    [handleOperationFailure, markDocumentSessionInvalid],
  );

  const confirmNativeAction = useCallback(
    async (title: string, message: string, options: ConfirmActionOptions = {}) => {
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
    },
    [],
  );

  const confirmAction = useCallback(
    (title: string, message: string, options: ConfirmActionOptions = {}): Promise<boolean> => {
      if (promptActiveRef.current) {
        return Promise.resolve(false);
      }
      return confirmNativeAction(title, message, options);
    },
    [confirmNativeAction],
  );

  function currentViewHasUnsavedChanges(): boolean {
    return documentHasUnsavedChanges(currentDocumentRef.current) ||
      userConfigurationDocumentRef.current?.dirty === true;
  }

  function currentViewName(): string {
    const document = currentDocumentRef.current;
    if (document !== null) return documentName(document);
    return userConfigurationDocumentRef.current?.configuration.name ?? "the open document";
  }

  async function confirmDiscardCurrentViewChanges(message: string): Promise<boolean> {
    if (!currentViewHasUnsavedChanges()) return true;
    return confirmAction("Discard unsaved changes", message, {
      confirmLabel: "Discard",
      destructive: true,
    });
  }

  function installUserConfigurationDocument(document: UserConfigurationDocumentDto) {
    userConfigurationDocumentRef.current = document;
    setUserConfigurationDocument(document);
  }

  function clearUserConfigurationDocument() {
    userConfigurationOpenRequestRef.current += 1;
    userConfigurationDocumentRef.current = null;
    setUserConfigurationDocument(null);
  }

  function updateUserConfigurationDocument(document: UserConfigurationDocumentDto) {
    if (
      currentDocumentRef.current !== null ||
      userConfigurationDocumentRef.current?.documentId !== document.documentId
    ) {
      return;
    }
    installUserConfigurationDocument(document);
  }

  const pendingAppDefinitionInput =
    currentDocument !== null && isAppDefinitionDocument(currentDocument) && appDefinitionDraftDirty;

  const actionAvailability = useMemo(
    () =>
      buildActionAvailability({
        hasDocument: currentDocument !== null,
        hasSelectedAuthoredRoot: selectedAuthoredRoot !== null,
        hasDocumentAuthoredRoot: currentDocument?.authoredRoot !== null && currentDocument?.authoredRoot !== undefined,
        hasDocumentPath: Boolean(currentDocument?.path),
        dirty: (currentDocument?.dirty ?? false) || pendingAppDefinitionInput,
        pendingAppDefinitionInput,
        canSaveAs: documentCanSaveAs(currentDocument),
        canUndo: currentDocument?.canUndo ?? false,
        canRedo: currentDocument?.canRedo ?? false,
        commandInFlight,
        documentSessionValid,
        backendCompatible: sidecarState?.compatible ?? null,
        sidecarRunning: sidecarState?.running ?? null,
      }),
    [
      commandInFlight,
      currentDocument,
      documentSessionValid,
      pendingAppDefinitionInput,
      selectedAuthoredRoot,
      sidecarState?.compatible,
      sidecarState?.running,
    ],
  );

  const invalidSessionRecovery = useMemo(
    () =>
      buildInvalidSessionRecovery({
        hasDocument: currentDocument !== null,
        hasSelectedAuthoredRoot: selectedAuthoredRoot !== null,
        hasDocumentAuthoredRoot: currentDocument?.authoredRoot !== null && currentDocument?.authoredRoot !== undefined,
        hasDocumentPath: Boolean(currentDocument?.path),
        dirty: (currentDocument?.dirty ?? false) || pendingAppDefinitionInput,
        pendingAppDefinitionInput,
        canSaveAs: documentCanSaveAs(currentDocument),
        canUndo: currentDocument?.canUndo ?? false,
        canRedo: currentDocument?.canRedo ?? false,
        commandInFlight,
        documentSessionValid,
        backendCompatible: sidecarState?.compatible ?? null,
        sidecarRunning: sidecarState?.running ?? null,
      }),
    [
      commandInFlight,
      currentDocument,
      documentSessionValid,
      pendingAppDefinitionInput,
      selectedAuthoredRoot,
      sidecarState?.compatible,
      sidecarState?.running,
    ],
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
    let active = true;
    void getConfigEditorAuthoredRoot().then((response) => {
      if (!active || response.kind !== "success") return;
      selectedAuthoredRootRef.current = response.result.authoredRoot;
      setSelectedAuthoredRoot(response.result.authoredRoot);
    });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    sidecarStateRef.current = sidecarState;
  }, [sidecarState]);

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
    deviceProfileGeneratorOpenRef.current = deviceProfileGeneratorOpen;
    void syncMenuState(currentDocumentRef.current);
  }, [deviceProfileGeneratorOpen, syncMenuState]);

  useEffect(() => {
    appGeneratorOpenRef.current = appGeneratorOpen;
    void syncMenuState(currentDocumentRef.current);
  }, [appGeneratorOpen, syncMenuState]);

  useEffect(() => {
    void syncMenuState(currentDocument, commandInFlight, documentSessionValid);
  }, [
    appDefinitionDraftDirty,
    commandInFlight,
    currentDocument,
    documentSessionValid,
    selectedAuthoredRoot,
    sidecarState?.compatible,
    sidecarState?.running,
    syncMenuState,
  ]);

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
        dirty: documentHasUnsavedChanges(document),
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
        const copy = buildCloseConfirmationCopy(
          decision.reason,
          document === null ? null : documentName(document),
        );
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
  }, [confirmAction, documentHasUnsavedChanges]);

  useEffect(() => {
    let cancelled = false;

    async function loadSpecs() {
      setStepSpecsLoading(true);
      const initialStatus = await sidecarStatus();
      if (!cancelled) {
        handleStatusResponse(initialStatus);
      }
      const response = await listStepSpecs();
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
  }, [handleOperationFailure, handleStatusResponse]);

  const menuHandlers: Record<MenuAction, () => void> = {
    openRecipe: () => void openRecipe(),
    openAppDefinition: () => void openAppDefinition(),
    openUserConfiguration: () => void openUserConfiguration(),
    generateAppRecipe: () => void openAppGenerator(),
    generateDeviceProfile: () => {
      const status = sidecarStateRef.current;
      if (
        commandInFlightRef.current === null &&
        !appGeneratorOpenRef.current &&
        !deviceProfileGeneratorOpenRef.current &&
        status?.compatible === true &&
        status.running === true
      ) {
        setDeviceProfileGeneratorOpen(true);
      }
    },
    saveRecipe: () => void saveRecipe(),
    saveRecipeAs: () => void saveRecipeAs(),
    restartSidecar: () => void restartSidecar(),
    undo: () => void undo(),
    redo: () => void redo(),
    validate: () => void validate(),
    refreshYaml: () => void refreshYaml(),
    setAuthoredRoot: () => void setAuthoredRootFromDialog(),
    clearAuthoredRoot: () => void clearAuthoredRoot(),
  };

  async function openAppGenerator() {
    const status = sidecarStateRef.current;
    if (
      commandInFlightRef.current !== null ||
      appGeneratorOpenRef.current ||
      deviceProfileGeneratorOpenRef.current ||
      status?.compatible !== true ||
      status.running !== true
    ) {
      return;
    }
    if (!await confirmDiscardCurrentViewChanges(
      `Generating and opening a recipe will replace the unsaved ${currentViewName()}. Discard those changes?`,
    )) {
      return;
    }
    userConfigurationOpenRequestRef.current += 1;
    setAppGeneratorOpen(true);
  }

  async function handleGeneratedAppRecipeSaved(result: AppGeneratorPublicationOutcomeDto) {
    if (result.outcome !== "saved") return;
    setAppGeneratorOpen(false);
    setStatusMessage("Saved " + result.app.appPath + " and " + result.recipe.recipePath + ".");
    setErrorMessage(null);
    if (result.recipe.documentId === null) {
      // The catalog writes committed; only opening the new document failed. The
      // author opens the saved file manually instead of another automatic try.
      const openDetail = result.recipe.openError?.message;
      const openFailure = openDetail
        ? ": " + (openDetail.endsWith(".") ? openDetail : openDetail + ".")
        : ".";
      setStatusMessage(
        "Saved " + result.app.appPath + " and " + result.recipe.recipePath
        + ". The editor could not open the saved Recipe automatically"
        + openFailure
        + " Open " + result.recipe.recipePath + " manually to review it.",
      );
      return;
    }
    const opened = await getDocument(result.recipe.documentId);
    if (opened.kind !== "success") {
      setErrorMessage(
        "Saved " + result.recipe.recipePath + ", but opening the saved Recipe failed: "
        + (opened.kind === "api-error" ? opened.error.message : opened.message),
      );
      return;
    }
    applyDocument(opened.result.document);
    documentSessionValidRef.current = true;
    setDocumentSessionValid(true);
    sessionInvalidReasonRef.current = null;
    setSessionInvalidReason(null);
    setActiveView("overview");
    void syncMenuState(opened.result.document, null, true);
  }

  async function openUserConfiguration() {
    if (commandInFlightRef.current !== null) {
      return;
    }
    const requestId = ++userConfigurationOpenRequestRef.current;
    if (!await confirmDiscardCurrentViewChanges(`Discard unsaved changes to ${currentViewName()}?`)) {
      return;
    }
    let selected: string | string[] | null;
    try {
      selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "YAML user configurations", extensions: ["yaml", "yml"] }],
      });
    } catch (error) {
      setErrorMessage(`File dialog failed: ${errorMessageFromUnknown(error)}`);
      return;
    }
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path || requestId !== userConfigurationOpenRequestRef.current) {
      return;
    }
    setLoadingLabel("Opening user configuration");
    try {
      const response = await openUserConfigurationDocument(path, selectedAuthoredRootRef.current);
      if (requestId !== userConfigurationOpenRequestRef.current || currentDocumentRef.current !== null) {
        return;
      }
      if (response.kind === "success") {
        currentDocumentRef.current = null;
        setCurrentDocument(null);
        installAppDefinitionDraft(null);
        installUserConfigurationDocument(response.result.document);
        setErrorMessage(null);
      } else {
        handleOperationFailure(response, "User configuration failed to open.");
      }
    } finally {
      if (requestId === userConfigurationOpenRequestRef.current) {
        setLoadingLabel(null);
      }
    }
  }

  async function openRecipe() {
    if (commandInFlightRef.current !== null) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const current = currentDocumentRef.current;
    if (!currentActionAvailability(current).openRecipe) {
      if (!documentSessionValidRef.current) {
        showInvalidSessionMessage();
      }
      await syncMenuState(current);
      return;
    }

    if (!await confirmDiscardCurrentViewChanges(`Discard unsaved changes to ${currentViewName()}?`)) {
      await syncMenuState(currentDocumentRef.current);
      return;
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

    await openRecipeAtPath(path);
  }

  async function openAppDefinition() {
    if (commandInFlightRef.current !== null) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const current = currentDocumentRef.current;
    if (!currentActionAvailability(current).openRecipe) {
      if (!documentSessionValidRef.current) {
        showInvalidSessionMessage();
      }
      await syncMenuState(current);
      return;
    }

    if (!await confirmDiscardCurrentViewChanges(`Discard unsaved changes to ${currentViewName()}?`)) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    let selected: string | string[] | null;
    try {
      selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "YAML App Definitions", extensions: ["yaml", "yml"] }],
      });
    } catch (error) {
      setErrorMessage(`File dialog failed: ${errorMessageFromUnknown(error)}`);
      setStatusMessage(null);
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    const path = Array.isArray(selected) ? selected[0] : selected;
    if (path) {
      await openAppDefinitionAtPath(path);
    } else {
      await syncMenuState(currentDocumentRef.current);
    }
  }

  async function openAppDefinitionAtPath(path: string): Promise<boolean> {
    if (!beginAppCommand("openAppDefinition", "Opening App Definition")) {
      await syncMenuState(currentDocumentRef.current);
      return false;
    }
    userConfigurationOpenRequestRef.current += 1;
    try {
      const response = await openAppDefinitionDocument(path, selectedAuthoredRootRef.current);
      if (response.kind === "success") {
        applyDocument(response.result.document, path);
        documentSessionValidRef.current = true;
        setDocumentSessionValid(true);
        sessionInvalidReasonRef.current = null;
        setSessionInvalidReason(null);
        setActiveView("overview");
        setErrorMessage(null);
        setStatusMessage(null);
        await syncMenuState(response.result.document, commandInFlightRef.current, true);
        return true;
      }

      const classification = handleOperationFailure(response, "App Definition failed to open.");
      const resolution = resolveOpenAttempt(currentDocumentRef.current, {
        kind: "open-failed",
        sessionInvalid: classification.sessionInvalid,
      });
      if (!resolution.sessionValid) {
        markDocumentSessionInvalid(classification.message);
      }
      await refreshSidecarStatus();
      await syncMenuState(
        resolution.document,
        commandInFlightRef.current,
        documentSessionValidRef.current && resolution.sessionValid,
      );
      return false;
    } finally {
      finishAppCommand();
    }
  }

  async function openRecipeAtPath(path: string): Promise<boolean> {
    if (!beginAppCommand("openRecipe", "Opening recipe")) {
      await syncMenuState(currentDocumentRef.current);
      return false;
    }
    userConfigurationOpenRequestRef.current += 1;
    try {
      const response = await openRecipeDocument(path, selectedAuthoredRootRef.current);
      if (response.kind === "success") {
        applyDocument(response.result.document, path);
        documentSessionValidRef.current = true;
        setDocumentSessionValid(true);
        sessionInvalidReasonRef.current = null;
        setSessionInvalidReason(null);
        setActiveView("overview");
        setErrorMessage(null);
        setStatusMessage(null);
        await syncMenuState(response.result.document, commandInFlightRef.current, true);
        return true;
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
        await syncMenuState(resolution.document, commandInFlightRef.current, documentSessionValidRef.current && resolution.sessionValid);
        return false;
      }
    } finally {
      finishAppCommand();
    }
  }

  async function reopenFromDisk() {
    const document = currentDocumentRef.current;
    const path = document?.path;
    const availability = currentActionAvailability(document).reopenFromDisk;
    if (document === null || !path || !availability) {
      await syncMenuState(document);
      return;
    }

    if (documentHasUnsavedChanges(document)) {
      const confirmed = await confirmAction(
        "Reopen saved file",
        `Reopen ${documentName(document)} from disk? The stale in-memory edits cannot be saved through the invalid sidecar session and will not be replayed.`,
        { confirmLabel: "Reopen", destructive: true },
      );
      if (!confirmed) {
        await syncMenuState(currentDocumentRef.current);
        return;
      }
    }

    const appDefinitionDocument = isAppDefinitionDocument(document);
    if (!beginAppCommand(appDefinitionDocument ? "openAppDefinition" : "openRecipe", "Reopening document")) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }
    try {
      const response = appDefinitionDocument
        ? await openAppDefinitionDocument(path, selectedAuthoredRootRef.current)
        : await openRecipeDocument(path, selectedAuthoredRootRef.current);
      if (response.kind === "success") {
        const resolution = resolveReopenAttempt(document, { kind: "opened", document: response.result.document });
        applyDocument(response.result.document, path);
        setDocumentSessionValid(resolution.sessionValid);
        documentSessionValidRef.current = resolution.sessionValid;
        setSessionInvalidReason(null);
        sessionInvalidReasonRef.current = null;
        setActiveView("overview");
        setErrorMessage(null);
        setStatusMessage(null);
        await syncMenuState(response.result.document, commandInFlightRef.current, resolution.sessionValid);
      } else {
        const classification = handleOperationFailure(
          response,
          appDefinitionDocument ? "App Definition failed to reopen." : "Recipe failed to reopen.",
        );
        const resolution = resolveReopenAttempt(document, {
          kind: "open-failed",
          sessionInvalid: classification.sessionInvalid,
        });
        if (!resolution.sessionValid) {
          markDocumentSessionInvalid(sessionInvalidReasonRef.current ?? classification.message);
        }
        await refreshSidecarStatus();
        await syncMenuState(resolution.document, commandInFlightRef.current, false);
      }
    } finally {
      finishAppCommand();
    }
  }

  async function restartSidecar() {
    if (!currentActionAvailability(currentDocumentRef.current).restartSidecar) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }
    if (!beginAppCommand("restartSidecar", "Restarting sidecar")) {
      await syncMenuState(currentDocumentRef.current);
      return;
    }

    try {
      const response = await sidecarRestart();
      if (response.kind !== "success") {
        const classification = classifyRestartFailure(response, "Sidecar restart failed.");
        setErrorMessage(classification.message);
        setStatusMessage(null);
        await syncMenuState(currentDocumentRef.current);
        return;
      }

      const status = response.result.status;
      setAppGeneratorOpen(false);
      setDeviceProfileGeneratorOpen(false);
      sidecarStateRef.current = status;
      setSidecarState(status);
      const document = currentDocumentRef.current;
      const restartResolution = resolveRestartSuccess(status, document !== null);

      if (document !== null) {
        const message = restartResolution.message ?? invalidSessionMessage();
        markDocumentSessionInvalid(message);
      } else {
        documentSessionValidRef.current = restartResolution.sessionValid;
        setDocumentSessionValid(restartResolution.sessionValid);
        sessionInvalidReasonRef.current = restartResolution.message;
        setSessionInvalidReason(restartResolution.message);
      }

      if (!restartResolution.sidecarUsable) {
        setErrorMessage(restartResolution.message);
        setStatusMessage(null);
        await syncMenuState(document, commandInFlightRef.current, restartResolution.sessionValid);
        return;
      }

      const specsRefreshed = await refreshStepSpecsAfterRestart();
      if (specsRefreshed) {
        setErrorMessage(null);
        setStatusMessage(document === null ? "Sidecar restarted." : "Sidecar restarted. Reopen the stale document from disk to edit it.");
      }
      await syncMenuState(document, commandInFlightRef.current, restartResolution.sessionValid);
    } finally {
      finishAppCommand();
    }
  }

  async function refreshStepSpecsAfterRestart(): Promise<boolean> {
    setStepSpecsLoading(true);
    const response = await listStepSpecs();
    if (response.kind === "success") {
      setStepSpecs(response.result.stepSpecs);
      setStepSpecsLoaded(true);
      setStepSpecsLoading(false);
      return true;
    }

    handleOperationFailure(response, "Step specs failed to refresh after sidecar restart.");
    setStepSpecsLoading(false);
    return false;
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
      const persisted = await setConfigEditorAuthoredRoot(resolution.selectedAuthoredRoot);
      if (persisted.kind !== "success") {
        setErrorMessage("The authored root could not be saved as the app-wide selection.");
        return;
      }
      selectedAuthoredRootRef.current = persisted.result.authoredRoot;
      setSelectedAuthoredRoot(persisted.result.authoredRoot);
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
      const response = await setDocumentAuthoredRoot(document.documentId, nextAuthoredRoot);
      if (response.kind === "success") {
        const resolution = resolveAuthoredRootSelectionAttempt(selectedAuthoredRootRef.current, document, {
          kind: "updated",
          authoredRoot: nextAuthoredRoot,
          document: response.result.document,
        });
        const persisted = await setConfigEditorAuthoredRoot(resolution.selectedAuthoredRoot);
        if (persisted.kind !== "success") {
          setErrorMessage("The authored root could not be saved as the app-wide selection.");
          return;
        }
        selectedAuthoredRootRef.current = persisted.result.authoredRoot;
        setSelectedAuthoredRoot(persisted.result.authoredRoot);
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
      const response = await validateDocument(document.documentId);
      if (response.kind === "success") {
        if (response.result.document !== undefined) {
          applyDocument(response.result.document);
        } else {
          setDiagnostics(response.result.diagnostics);
        }
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
      const response = await emitYaml(document.documentId);
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
      const response = await undoDocument(document.documentId);
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
      const response = await redoDocument(document.documentId);
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
    if (localDraftIsDirty(currentDocumentRef.current)) {
      setErrorMessage("Apply or discard the pending App Definition form input before saving the document.");
      setStatusMessage(null);
      return;
    }
    const document = beginDocumentCommand("saveRecipe", "Saving document", "saveRecipe");
    if (document === null) {
      return;
    }
    try {
      const response = isAppDefinitionDocument(document)
        ? await saveAppDefinitionDocument(document.documentId)
        : await saveRecipeDocument(document.documentId);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        setStatusMessage(
          isAppDefinitionDocument(response.result.document) ? "Saved the current App Definition." : "Saved the current recipe.",
        );
        await syncMenuState(response.result.document);
      } else {
        handleOperationFailure(response, "Save failed.", {
          commandDocumentId: document.documentId,
          currentDocumentId: currentDocumentRef.current?.documentId ?? null,
        });
        applySaveFailureDiagnostics(document.documentId, response);
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

    if (localDraftIsDirty(document)) {
      setErrorMessage("Apply or discard the pending App Definition form input before saving the document.");
      setStatusMessage(null);
      await syncMenuState(document);
      return;
    }

    // A standalone App Definition is published into the selected catalog, never beside its source file.
    if (isAppDefinitionDocument(document) && document.owningAuthoredRoot === null && (
      !document.editRestrictions.canSaveAs || !document.authoredRoot || !document.authoredRoot.startsWith("/")
    )) {
      setErrorMessage("Select an authored root before saving this standalone App Definition. Then choose Save As again.");
      setStatusMessage(null);
      await setAuthoredRootFromDialog();
      return;
    }

    const availability = currentActionAvailability(document);
    if (!availability.saveRecipeAs) {
      if (isAppDefinitionDocument(document) && !document.editRestrictions.canSaveAs) {
        setErrorMessage("Save As is not available for this App Definition document.");
        setStatusMessage(null);
      }
      await syncMenuState(document);
      return;
    }

    let selectedPath: string | null;
    try {
      selectedPath = await saveFile({
        defaultPath: isAppDefinitionDocument(document) && document.owningAuthoredRoot === null
          ? `${document.authoredRoot?.replace(/\/$/, "")}/apps/${document.appDefinition.id}.yaml`
          : document.path || `${documentName(document) || "document"}.yaml`,
        filters: [
          {
            name: isAppDefinitionDocument(document) ? "YAML App Definitions" : "YAML recipes",
            extensions: ["yaml", "yml"],
          },
        ],
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
      const response = isAppDefinitionDocument(document)
        ? await saveAppDefinitionDocumentAs(document.documentId, selectedPath)
        : await saveRecipeDocumentAs(document.documentId, selectedPath);
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
        applySaveFailureDiagnostics(document.documentId, response);
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
    if (isAppDefinitionDocument(document)) {
      finishAppCommand();
      return { ok: false, changed: false };
    }
    try {
      const response = await applyRecipeCommand(document.documentId, command);
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

  async function applyAppDefinitionEditorCommand(command: AppDefinitionCommand): Promise<AppDefinitionV1Dto> {
    const document = beginDocumentCommand("mutation", "Applying App Definition edit", "editDocument");
    if (document === null) {
      throw new Error("No editable App Definition document is open.");
    }
    if (!isAppDefinitionDocument(document)) {
      finishAppCommand();
      throw new Error("The open document is not an App Definition.");
    }

    try {
      const response = await applyAppDefinitionCommandApi(document.documentId, command);
      if (response.kind === "success") {
        applyDocument(response.result.document);
        setErrorMessage(null);
        await syncMenuState(response.result.document);
        return response.result.document.appDefinition;
      }

      const classification = handleOperationFailure(response, "App Definition edit failed.", {
        commandDocumentId: document.documentId,
        currentDocumentId: currentDocumentRef.current?.documentId ?? null,
      });
      await refreshSidecarStatus();
      await syncMenuState(currentDocumentRef.current);
      throw new Error(classification.message);
    } finally {
      finishAppCommand();
    }
  }

  function acceptAppDefinitionEditorDocument(appDefinition: AppDefinitionV1Dto) {
    const document = currentDocumentRef.current;
    if (document === null || !isAppDefinitionDocument(document)) {
      return;
    }
    const next = { ...document, appDefinition };
    currentDocumentRef.current = next;
    setCurrentDocument(next);
    installAppDefinitionDraft(
      appDefinitionEditorDraftRef.current,
      appDefinitionEditorDraftRef.current !== null &&
        isAppDefinitionEditorDraftDirty(appDefinitionEditorDraftRef.current, appDefinition),
    );
  }

  function beginDocumentCommand(
    command: CommandName,
    label: string,
    action: keyof ReturnType<typeof buildActionAvailability>,
  ): EditorDocumentDto | null {
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
    const availability = currentActionAvailability(document);
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

  function applyDocument(document: EditorDocumentDto, fallbackPath: string | null = null) {
    const previousDocument = currentDocumentRef.current;
    clearUserConfigurationDocument();

    if (isAppDefinitionDocument(document)) {
      const previousAppDefinition =
        previousDocument !== null &&
        isAppDefinitionDocument(previousDocument) &&
        previousDocument.documentId === document.documentId
          ? previousDocument.appDefinition
          : null;
      if (appDefinitionDraftDirtyRef.current && appDefinitionEditorDraftRef.current !== null && previousAppDefinition !== null) {
        const rebasedDraft = rebasePendingAppDefinitionEditorDraft(
          appDefinitionEditorDraftRef.current,
          previousAppDefinition,
          document.appDefinition,
        );
        installAppDefinitionDraft(rebasedDraft, isAppDefinitionEditorDraftDirty(rebasedDraft, document.appDefinition));
      } else {
        installAppDefinitionDraft(createAppDefinitionEditorDraft(document.appDefinition));
      }
    } else {
      installAppDefinitionDraft(null);
    }
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

  function showInvalidSessionMessage() {
    setErrorMessage(sessionInvalidReasonRef.current ?? invalidSessionMessage());
    setStatusMessage(null);
  }

  function applySaveFailureDiagnostics<T>(
    requestDocumentId: string,
    response: Exclude<EditorApiResult<T>, { kind: "success" }>,
  ) {
    if (response.kind !== "api-error") {
      return;
    }
    const diagnostics = saveFailureDiagnosticsForActiveDocument(
      response.error.details,
      requestDocumentId,
      currentDocumentRef.current?.documentId ?? null,
    );
    if (diagnostics !== null) {
      setDiagnostics(diagnostics);
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
    if (isAppDefinitionDocument(currentDocument)) {
      return (
        <AppDefinitionEditor
          key={currentDocument.documentId}
          appDefinition={currentDocument.appDefinition}
          draft={appDefinitionEditorDraft ?? createAppDefinitionEditorDraft(currentDocument.appDefinition)}
          identityEditable={
            currentDocument.editRestrictions.canEditIdentity &&
            !currentDocument.editRestrictions.identityLocked
          }
          disabled={!actionAvailability.editDocument}
          onDraftChange={installAppDefinitionDraft}
          onApply={applyAppDefinitionEditorCommand}
          onDocumentAccepted={acceptAppDefinitionEditorDocument}
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

  if (userConfigurationDocument !== null) {
    return (
      <>
        <MenuEventBridge handlers={menuHandlers} />
        {appGeneratorOpen ? (
          <AppGenerator
            initialAuthoredRoot={selectedAuthoredRoot}
            onAuthoredRootSelected={updateAuthoredRootSelection}
            onClose={() => setAppGeneratorOpen(false)}
            onSaved={handleGeneratedAppRecipeSaved}
          />
        ) : null}
        {deviceProfileGeneratorOpen ? (
          <DeviceProfileGenerator
            initialAuthoredRoot={selectedAuthoredRoot}
            onAuthoredRootSelected={updateAuthoredRootSelection}
            onClose={() => setDeviceProfileGeneratorOpen(false)}
            onSaved={(path) => setStatusMessage(`Saved ${path}.`)}
          />
        ) : null}
        {errorMessage ? <ErrorBanner message={errorMessage} onDismiss={() => setErrorMessage(null)} /> : null}
        <UserConfigurationEditor
          document={userConfigurationDocument}
          onClose={clearUserConfigurationDocument}
          onDocument={updateUserConfigurationDocument}
          onError={(message) => {
            if (
              currentDocumentRef.current === null &&
              userConfigurationDocumentRef.current?.documentId === userConfigurationDocument.documentId
            ) {
              setErrorMessage(message);
            }
          }}
        />
      </>
    );
  }

  return (
    <>
      {appGeneratorOpen ? (
        <AppGenerator
          initialAuthoredRoot={selectedAuthoredRoot}
          onAuthoredRootSelected={updateAuthoredRootSelection}
          onClose={() => setAppGeneratorOpen(false)}
          onSaved={handleGeneratedAppRecipeSaved}
        />
      ) : null}
      {deviceProfileGeneratorOpen ? (
        <DeviceProfileGenerator
          initialAuthoredRoot={selectedAuthoredRoot}
          onAuthoredRootSelected={updateAuthoredRootSelection}
          onClose={() => setDeviceProfileGeneratorOpen(false)}
          onSaved={(path) => setStatusMessage(`Saved ${path}.`)}
        />
      ) : null}
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
              appDefinitionOwningRoot={currentDocument && isAppDefinitionDocument(currentDocument) ? currentDocument.owningAuthoredRoot : undefined}
              dirty={documentHasUnsavedChanges(currentDocument)}
              pendingAppDefinitionInput={pendingAppDefinitionInput}
              canSave={actionAvailability.saveRecipe}
              canSaveAs={actionAvailability.saveRecipeAs}
              documentSessionValid={documentSessionValid}
              hasDocument={currentDocument !== null}
              loadingLabel={loadingLabel}
              onSave={() => void saveRecipe()}
              onSaveAs={() => void saveRecipeAs()}
              selectedAuthoredRoot={selectedAuthoredRoot}
              sidecarStatus={sidecarState}
              stepSpecsCount={stepSpecsCount}
              stepSpecsLoading={stepSpecsLoading}
            />
            {!documentSessionValid && currentDocument !== null ? (
              <div className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-900">
                <div className="flex flex-wrap items-center gap-3">
                  <p className="min-w-0 flex-1">
                    {invalidSessionRecovery.message ??
                      "The displayed document is a stale read-only reference. Restart the sidecar, then reopen it from disk to create a new document session."}
                  </p>
                  {invalidSessionRecovery.reopenFromDisk ? (
                    <button
                      className="rounded border border-amber-300 bg-white px-3 py-1 text-sm font-medium text-amber-950 hover:bg-amber-100 disabled:opacity-40"
                      type="button"
                      onClick={() => void reopenFromDisk()}
                    >
                      Reopen from Disk
                    </button>
                  ) : null}
                </div>
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

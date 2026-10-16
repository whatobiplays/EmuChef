import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AppDefinitionDocumentDto,
  AppDefinitionV1Dto,
  RecipeDocumentDto,
  UserConfigurationDocumentDto,
} from "../src/api/types";

const mocks = vi.hoisted(() => ({
  openDialog: vi.fn(),
  saveDialog: vi.fn(),
  nativeConfirm: vi.fn(),
  menuListener: vi.fn(),
  menuActionHandler: null as unknown,
  closeRequestHandler: null as unknown,
  closeWindow: vi.fn(),
  api: {
    applyAppDefinitionCommand: vi.fn(),
    applyRecipeCommand: vi.fn(),
    emitYaml: vi.fn(),
    getConfigEditorAuthoredRoot: vi.fn(),
    listStepSpecs: vi.fn(),
    openAppDefinition: vi.fn(),
    openRecipe: vi.fn(),
    openUserConfiguration: vi.fn(),
    redo: vi.fn(),
    saveAppDefinition: vi.fn(),
    saveAppDefinitionAs: vi.fn(),
    saveRecipe: vi.fn(),
    saveRecipeAs: vi.fn(),
    setConfigEditorAuthoredRoot: vi.fn(),
    setDocumentAuthoredRoot: vi.fn(),
    sidecarRestart: vi.fn(),
    sidecarStatus: vi.fn(),
    undo: vi.fn(),
    updateMenuState: vi.fn(),
    validate: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.menuListener,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: (handler: unknown) => {
      mocks.closeRequestHandler = handler;
      return Promise.resolve(() => undefined);
    },
    close: mocks.closeWindow,
  }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: mocks.nativeConfirm,
  open: mocks.openDialog,
  save: mocks.saveDialog,
}));

vi.mock("../src/api/editorApi", () => mocks.api);

import App from "../src/App";

const apiSuccess = <T,>(result: T) => ({ kind: "success" as const, result });
const apiFailure = (message: string, details: Record<string, unknown> = {}) => ({
  kind: "api-error" as const,
  error: { code: "invalid_request", message, details },
});

function appDefinition(overrides: Partial<AppDefinitionV1Dto> = {}): AppDefinitionV1Dto {
  return {
    schema_version: 1,
    kind: "app_definition",
    id: "invalid app id",
    name: "",
    package_id: "",
    ...overrides,
  };
}

function appDocument(
  documentId = "app-session-1",
  overrides: Partial<AppDefinitionDocumentDto> = {},
): AppDefinitionDocumentDto {
  return {
    documentKind: "app_definition",
    documentId,
    path: "/catalog/apps/invalid-app.yaml",
    authoredRoot: "/catalog",
    owningAuthoredRoot: null,
    dirty: false,
    canUndo: false,
    canRedo: false,
    appDefinition: appDefinition(),
    yaml: "kind: app_definition\nid: invalid app id\n",
    diagnostics: [
      {
        severity: "error",
        code: "APP_SEMANTIC_ERROR",
        message: "Existing semantic error remains visible while another field is repaired.",
        file: "invalid-app.yaml",
        objectKind: "app_definition",
        objectId: "invalid app id",
        field: "package_id",
      },
    ],
    editRestrictions: { identityLocked: false, canEditIdentity: true, canSaveAs: true },
    ...overrides,
  };
}

function recipeDocument(
  documentId = "recipe-session-1",
  overrides: Partial<RecipeDocumentDto> = {},
): RecipeDocumentDto {
  return {
    documentId,
    path: "/catalog/recipes/demo.yaml",
    authoredRoot: "/catalog",
    dirty: false,
    canUndo: false,
    canRedo: false,
    recipe: {
      schemaVersion: 1,
      kind: "recipe",
      id: "demo-recipe",
      name: "Demo Recipe",
      description: "",
      recipeDependencies: [],
      provides: { features: [] },
      inputs: {},
      artifacts: {},
      artifactGroups: {},
      steps: [],
    },
    yaml: "kind: recipe\nid: demo-recipe\n",
    diagnostics: [],
    refIndex: {
      inputRefs: [],
      artifactRefs: [],
      stepRefs: [],
      stepOutputRefs: [],
      allRefs: [],
      candidates: [],
    },
    appCatalog: { status: "unbound", apps: [], diagnostics: [] },
    ...overrides,
  };
}

function userConfigurationDocument(): UserConfigurationDocumentDto {
  return {
    documentId: "user-configuration-session-1",
    path: "/catalog/user-configurations/demo.yaml",
    authoredRoot: null,
    dirty: false,
    configuration: {
      schemaVersion: 1,
      kind: "user_configuration",
      id: "demo-user-configuration",
      name: "Demo User Configuration",
      devicePlan: "demo-device-plan",
      selectedRecipes: [],
      bindings: {},
      extensions: {},
    },
    yaml: "kind: user_configuration\nid: demo-user-configuration\n",
    diagnostics: [],
  };
}

function dispatchMenu(action: string) {
  const handler = mocks.menuActionHandler as ((event: { payload: { action: string } }) => void) | null;
  if (handler === null) throw new Error("The native menu listener has not been installed.");
  act(() => handler({ payload: { action } }));
}

async function mountEditor() {
  render(<App />);
  await waitFor(() => expect(mocks.menuActionHandler).not.toBeNull());
  await waitFor(() => expect(mocks.api.listStepSpecs).toHaveBeenCalled());
}

async function openAppDefinitionDocument(document: AppDefinitionDocumentDto) {
  mocks.openDialog.mockResolvedValueOnce(document.path);
  mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document }));
  dispatchMenu("openAppDefinition");
  await waitFor(() => {
    expect(screen.getByLabelText("App Definition editor")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value)
      .toBe(document.appDefinition.id);
  });
  await waitFor(() => expect(mocks.api.openAppDefinition).toHaveBeenCalled());
}

describe("App Definition owning roots and Save As destinations", () => {
  it("uses the selected root for standalone Save As and preserves canceled/rejected drafts", async () => {
    await mountEditor();
    const document = appDocument("standalone", {
      path: "/downloads/demo.yaml", authoredRoot: "/selected", owningAuthoredRoot: null,
      dirty: true, canUndo: true, appDefinition: appDefinition({ id: "demo", name: "Draft" }),
    });
    await openAppDefinitionDocument(document);
    expect(screen.getByText(/Owning authored root: None \(standalone\)/)).toBeTruthy();
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.saveDialog).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: "/selected/apps/demo.yaml",
    })));
    expect(mocks.api.saveAppDefinitionAs).not.toHaveBeenCalled();
    mocks.saveDialog.mockResolvedValueOnce("/selected/apps/demo.yaml");
    mocks.api.saveAppDefinitionAs.mockResolvedValueOnce(apiFailure("Destination already exists."));
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(screen.getAllByText(/Destination already exists\./).length).toBeGreaterThan(0));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Draft");
    expect(screen.getByText("Unsaved")).toBeTruthy();
    expect(mocks.api.saveAppDefinitionAs).toHaveBeenCalledWith("standalone", "/selected/apps/demo.yaml");
    mocks.openDialog.mockResolvedValueOnce("/next");
    mocks.api.setDocumentAuthoredRoot.mockResolvedValueOnce(apiSuccess({ document: { ...document, authoredRoot: "/next" } }));
    mocks.api.setConfigEditorAuthoredRoot.mockResolvedValueOnce(apiSuccess({ authoredRoot: "/next" }));
    dispatchMenu("setAuthoredRoot");
    await waitFor(() => expect(screen.getByText(/Selected authored root: \/next/)).toBeTruthy());
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.saveDialog).toHaveBeenLastCalledWith(expect.objectContaining({ defaultPath: "/next/apps/demo.yaml" })));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Draft");
  });

  it("requires a validated root selection before opening a standalone save dialog", async () => {
    await mountEditor();
    const document = appDocument("no-root", {
      authoredRoot: null, appDefinition: appDefinition({ id: "demo" }),
      editRestrictions: { identityLocked: false, canEditIdentity: true, canSaveAs: false },
    });
    await openAppDefinitionDocument(document);
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.openDialog).toHaveBeenLastCalledWith({ multiple: false, directory: true }));
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    mocks.openDialog.mockResolvedValueOnce("/invalid");
    mocks.api.setDocumentAuthoredRoot.mockResolvedValueOnce(apiFailure("Selected directory is not an authored root."));
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(screen.getAllByText(/Selected directory is not an authored root\./).length).toBeGreaterThan(0));
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    mocks.openDialog.mockResolvedValueOnce("/selected");
    mocks.api.setDocumentAuthoredRoot.mockResolvedValueOnce(apiSuccess({ document: {
      ...document, authoredRoot: "/selected", editRestrictions: { ...document.editRestrictions, canSaveAs: true },
    } }));
    mocks.api.setConfigEditorAuthoredRoot.mockResolvedValueOnce(apiSuccess({ authoredRoot: "/selected" }));
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(screen.getByText(/Selected authored root: \/selected/)).toBeTruthy());
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.saveDialog).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "/selected/apps/demo.yaml" })));
  });

  it("keeps catalog ownership and Save As restrictions when the selected root changes", async () => {
    await mountEditor();
    const document = appDocument("catalog", {
      authoredRoot: "/B", owningAuthoredRoot: "/A", dirty: true,
      editRestrictions: { identityLocked: true, canEditIdentity: false, canSaveAs: false },
    });
    await openAppDefinitionDocument(document);
    expect(screen.getByText(/Selected authored root: \/B \| Owning authored root: \/A/)).toBeTruthy();
    mocks.openDialog.mockResolvedValueOnce("/C");
    mocks.api.setDocumentAuthoredRoot.mockResolvedValueOnce(apiSuccess({ document: { ...document, authoredRoot: "/C" } }));
    mocks.api.setConfigEditorAuthoredRoot.mockResolvedValueOnce(apiSuccess({ authoredRoot: "/C" }));
    dispatchMenu("setAuthoredRoot");
    await waitFor(() => expect(screen.getByText(/Selected authored root: \/C \| Owning authored root: \/A/)).toBeTruthy());
    const rootDialogCalls = mocks.openDialog.mock.calls.length;
    dispatchMenu("saveRecipeAs");
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    expect(mocks.openDialog).toHaveBeenCalledTimes(rootDialogCalls);
    expect((screen.getByRole("button", { name: "Save As" }) as HTMLButtonElement).disabled).toBe(true);
    mocks.api.saveAppDefinition.mockResolvedValueOnce(apiSuccess({ document: { ...document, authoredRoot: "/C", dirty: false } }));
    dispatchMenu("saveRecipe");
    await waitFor(() => expect(mocks.api.saveAppDefinition).toHaveBeenCalledWith("catalog"));
    expect(screen.getByText(/Owning authored root: \/A/)).toBeTruthy();
  });

  it("does not suggest a relative selected root as a native Save As destination", async () => {
    await mountEditor();
    await openAppDefinitionDocument(appDocument("relative-root", { authoredRoot: "relative", appDefinition: appDefinition({ id: "demo" }) }));
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.openDialog).toHaveBeenLastCalledWith({ multiple: false, directory: true }));
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    expect(mocks.api.saveAppDefinitionAs).not.toHaveBeenCalled();
  });

  it("requests root selection for an ineligible absolute root and honors the returned eligibility", async () => {
    await mountEditor();
    const document = appDocument("invalid-absolute-root", {
      authoredRoot: "/invalid", dirty: true, canUndo: true,
      appDefinition: appDefinition({ id: "demo", name: "Accepted draft" }),
      editRestrictions: { identityLocked: false, canEditIdentity: true, canSaveAs: false },
    });
    await openAppDefinitionDocument(document);
    mocks.openDialog.mockResolvedValueOnce("/still-invalid");
    mocks.api.setDocumentAuthoredRoot.mockResolvedValueOnce(apiSuccess({ document: { ...document, authoredRoot: "/still-invalid" } }));
    mocks.api.setConfigEditorAuthoredRoot.mockResolvedValueOnce(apiSuccess({ authoredRoot: "/still-invalid" }));
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(screen.getByText(/Selected authored root: \/still-invalid/)).toBeTruthy());
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    expect(mocks.api.saveAppDefinitionAs).not.toHaveBeenCalled();
    const rootDialogCalls = mocks.openDialog.mock.calls.length;
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.openDialog).toHaveBeenCalledTimes(rootDialogCalls + 1));
    await waitFor(() => expect(mocks.openDialog).toHaveBeenLastCalledWith({ multiple: false, directory: true }));
    expect(mocks.saveDialog).not.toHaveBeenCalled();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Accepted draft");
    expect(screen.getByText("Unsaved")).toBeTruthy();
  });
});

beforeEach(() => {
  mocks.openDialog.mockReset();
  mocks.saveDialog.mockReset();
  mocks.nativeConfirm.mockReset();
  mocks.menuListener.mockReset();
  mocks.closeWindow.mockReset();
  mocks.api.applyAppDefinitionCommand.mockReset();
  mocks.api.applyRecipeCommand.mockReset();
  mocks.api.emitYaml.mockReset();
  mocks.api.getConfigEditorAuthoredRoot.mockReset();
  mocks.api.listStepSpecs.mockReset();
  mocks.api.openAppDefinition.mockReset();
  mocks.api.openRecipe.mockReset();
  mocks.api.openUserConfiguration.mockReset();
  mocks.api.redo.mockReset();
  mocks.api.saveAppDefinition.mockReset();
  mocks.api.saveAppDefinitionAs.mockReset();
  mocks.api.saveRecipe.mockReset();
  mocks.api.saveRecipeAs.mockReset();
  mocks.api.setConfigEditorAuthoredRoot.mockReset();
  mocks.api.setDocumentAuthoredRoot.mockReset();
  mocks.api.sidecarRestart.mockReset();
  mocks.api.sidecarStatus.mockReset();
  mocks.api.undo.mockReset();
  mocks.api.updateMenuState.mockReset();
  mocks.api.validate.mockReset();
  mocks.menuActionHandler = null;
  mocks.closeRequestHandler = null;

  mocks.openDialog.mockResolvedValue(null);
  mocks.saveDialog.mockResolvedValue(null);
  mocks.nativeConfirm.mockResolvedValue(true);
  mocks.menuListener.mockImplementation((_name: string, handler: unknown) => {
    mocks.menuActionHandler = handler;
    return Promise.resolve(() => undefined);
  });
  mocks.api.getConfigEditorAuthoredRoot.mockResolvedValue(apiSuccess({ authoredRoot: null }));
  mocks.api.listStepSpecs.mockResolvedValue(apiSuccess({ stepSpecs: [] }));
  mocks.api.sidecarStatus.mockResolvedValue(
    apiSuccess({ running: true, pid: 123, compatible: true, protocolVersion: 1 }),
  );
  mocks.api.updateMenuState.mockResolvedValue(apiSuccess(undefined));
});

describe("Config Editor App Definition document integration", () => {
  it("confirms dirty User Configuration before App, Recipe, and generator transitions", async () => {
    await mountEditor();
    await waitFor(() => expect(mocks.api.sidecarStatus).toHaveBeenCalled());

    const configuration = { ...userConfigurationDocument(), dirty: true };
    mocks.openDialog.mockResolvedValueOnce(configuration.path);
    mocks.api.openUserConfiguration.mockResolvedValueOnce(apiSuccess({ document: configuration }));
    dispatchMenu("openUserConfiguration");
    await screen.findByRole("heading", { name: configuration.configuration.name });

    const app = appDocument("app-after-user-configuration", {
      path: "/catalog/apps/after-user-configuration.yaml",
      appDefinition: appDefinition({ id: "after-user-configuration", name: "Opened App" }),
    });
    mocks.nativeConfirm.mockResolvedValueOnce(false);
    dispatchMenu("openAppDefinition");
    await waitFor(() => expect(mocks.nativeConfirm).toHaveBeenCalledTimes(1));
    expect(mocks.nativeConfirm).toHaveBeenLastCalledWith(
      expect.stringContaining(configuration.configuration.name),
      expect.objectContaining({ title: "Discard unsaved changes" }),
    );
    expect(mocks.openDialog).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("heading", { name: configuration.configuration.name })).toBeTruthy();

    mocks.nativeConfirm.mockResolvedValueOnce(false);
    dispatchMenu("openRecipe");
    await waitFor(() => expect(mocks.nativeConfirm).toHaveBeenCalledTimes(2));
    expect(mocks.openDialog).toHaveBeenCalledTimes(1);

    mocks.nativeConfirm.mockResolvedValueOnce(false);
    dispatchMenu("generateAppRecipe");
    await waitFor(() => expect(mocks.nativeConfirm).toHaveBeenCalledTimes(3));
    expect(mocks.openDialog).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("heading", { name: configuration.configuration.name })).toBeTruthy();

    mocks.nativeConfirm.mockResolvedValueOnce(true);
    mocks.openDialog.mockResolvedValueOnce(app.path);
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document: app }));
    dispatchMenu("openAppDefinition");
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value)
        .toBe(app.appDefinition.id);
    });
    expect(screen.queryByRole("heading", { name: configuration.configuration.name })).toBeNull();
  });

  it("ignores a late User Configuration response after an App Definition opens", async () => {
    await mountEditor();

    const configuration = userConfigurationDocument();
    let resolveConfiguration!: (value: { kind: "success"; result: { document: UserConfigurationDocumentDto } }) => void;
    const pendingConfiguration = new Promise<{
      kind: "success";
      result: { document: UserConfigurationDocumentDto };
    }>((resolve) => {
      resolveConfiguration = resolve;
    });
    mocks.openDialog.mockResolvedValueOnce(configuration.path);
    mocks.api.openUserConfiguration.mockReturnValueOnce(pendingConfiguration);
    dispatchMenu("openUserConfiguration");
    await waitFor(() => expect(mocks.api.openUserConfiguration).toHaveBeenCalledWith(configuration.path, null));

    const app = appDocument("app-before-late-user-configuration", {
      path: "/catalog/apps/before-late-user-configuration.yaml",
      appDefinition: appDefinition({ id: "before-late-user-configuration", name: "Current App" }),
    });
    mocks.openDialog.mockResolvedValueOnce(app.path);
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document: app }));
    dispatchMenu("openAppDefinition");
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value)
        .toBe(app.appDefinition.id);
    });

    await act(async () => {
      resolveConfiguration(apiSuccess({ document: configuration }));
      await pendingConfiguration;
    });

    expect(screen.getByLabelText("App Definition editor")).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "App ID" })).toHaveProperty("value", app.appDefinition.id);
    expect(screen.queryByRole("heading", { name: configuration.configuration.name })).toBeNull();
  });

  it("clears App Definition form errors when a different document session opens", async () => {
    const user = userEvent.setup();
    await mountEditor();
    await openAppDefinitionDocument(appDocument("app-error-session", {
      path: "/catalog/apps/error-session.yaml",
      appDefinition: appDefinition({ id: "error-session", name: "" }),
    }));

    await user.click(screen.getByRole("button", { name: "Apply name" }));
    expect(screen.getByText("Name must not be empty. Use the remove action for optional fields.")).toBeTruthy();

    const nextDocument = appDocument("app-clean-session", {
      path: "/catalog/apps/clean-session.yaml",
      appDefinition: appDefinition({ id: "clean-session", name: "Clean Session" }),
    });
    await openAppDefinitionDocument(nextDocument);
    expect(screen.queryByText("Name must not be empty. Use the remove action for optional fields.")).toBeNull();
  });

  it("switches from User Configuration to the opened App Definition document", async () => {
    await mountEditor();
    const userConfiguration = userConfigurationDocument();
    mocks.openDialog.mockResolvedValueOnce(userConfiguration.path);
    mocks.api.openUserConfiguration.mockResolvedValueOnce(apiSuccess({ document: userConfiguration }));
    dispatchMenu("openUserConfiguration");
    await screen.findByRole("heading", { name: "Demo User Configuration" });

    const app = appDocument("app-session-2", {
      path: "/catalog/apps/opened-app.yaml",
      appDefinition: appDefinition({ id: "opened-app", name: "Opened App", package_id: "com.example.opened" }),
    });
    mocks.openDialog.mockResolvedValueOnce(app.path);
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document: app }));
    dispatchMenu("openAppDefinition");

    await waitFor(() => expect(screen.getByLabelText("App Definition editor")).toBeTruthy());
    expect(screen.queryByRole("heading", { name: "Demo User Configuration" })).toBeNull();
    expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value).toBe("opened-app");
  });

  it("routes an invalid App Definition into the typed editor and applies one field independently", async () => {
    const user = userEvent.setup();
    await mountEditor();
    await openAppDefinitionDocument(appDocument());

    expect(screen.getByLabelText("App Definition editor")).toBeTruthy();
    expect(screen.getByText(/Existing semantic error remains visible/)).toBeTruthy();
    expect(screen.getByText(/id: invalid app id/)).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("");

    await user.type(screen.getByRole("textbox", { name: "Name" }), "Fixed Name");
    expect(screen.getByText("Unsaved")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Save As" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/Apply or discard pending App Definition form input before saving/)).toBeTruthy();
    expect(
      mocks.api.updateMenuState.mock.calls.some(([state]) => {
        const menuState = state as {
          documentKind?: string;
          dirty?: boolean;
          canSaveAs?: boolean;
          pendingAppDefinitionInput?: boolean;
          canSave?: boolean;
        };
        return menuState.documentKind === "app_definition" &&
          menuState.dirty === true &&
          menuState.canSaveAs === false &&
          menuState.pendingAppDefinitionInput === true &&
          menuState.canSave === false;
      }),
    ).toBe(true);

    const accepted = appDocument("app-session-1", {
      dirty: true,
      canUndo: true,
      appDefinition: appDefinition({ name: "Fixed Name" }),
    });
    mocks.api.applyAppDefinitionCommand.mockResolvedValueOnce(
      apiSuccess({ commandResult: { changed: true }, document: accepted }),
    );
    await user.click(screen.getByRole("button", { name: "Apply name" }));

    await waitFor(() => expect(mocks.api.applyAppDefinitionCommand).toHaveBeenCalledWith("app-session-1", {
      type: "SetName",
      name: "Fixed Name",
    }));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Fixed Name");
    expect(screen.getByText(/Existing semantic error remains visible/)).toBeTruthy();
  });

  it("preserves backend and local drafts after a cancelled or failed document open", async () => {
    const user = userEvent.setup();
    const initial = appDocument();
    await mountEditor();
    await openAppDefinitionDocument(initial);
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Local unsaved value");

    mocks.nativeConfirm.mockResolvedValue(true);
    mocks.openDialog.mockResolvedValueOnce(null);
    dispatchMenu("openAppDefinition");
    await waitFor(() => expect(mocks.openDialog).toHaveBeenCalledTimes(2));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local unsaved value");
    expect(screen.getByText("invalid app id")).toBeTruthy();

    mocks.openDialog.mockResolvedValueOnce("/catalog/apps/other.yaml");
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiFailure("The other App Definition could not be opened."));
    dispatchMenu("openAppDefinition");
    await screen.findByText(/The other App Definition could not be opened/);

    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local unsaved value");
    expect(screen.getByText("invalid app id")).toBeTruthy();
    expect(mocks.api.openAppDefinition).toHaveBeenCalledTimes(2);
  });

  it("rejects Save while local input is pending, offers discard, and honors switch cancellation", async () => {
    const user = userEvent.setup();
    await mountEditor();
    await openAppDefinitionDocument(appDocument());
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Unapplied name");

    dispatchMenu("saveRecipe");
    expect(await screen.findByText(/Apply or discard the pending App Definition form input/)).toBeTruthy();
    expect(mocks.api.saveAppDefinition).not.toHaveBeenCalled();
    expect(mocks.api.saveRecipe).not.toHaveBeenCalled();

    dispatchMenu("saveRecipeAs");
    expect(await screen.findByText(/Apply or discard the pending App Definition form input/)).toBeTruthy();
    expect(mocks.saveDialog).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Discard pending form input" }));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("");
    expect(screen.queryByText(/Form input is pending/)).toBeNull();
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Save As" }) as HTMLButtonElement).disabled).toBe(false);
    expect(
      mocks.api.updateMenuState.mock.calls.some(([state]) => {
        const menuState = state as { pendingAppDefinitionInput?: boolean; canSaveAs?: boolean };
        return menuState.pendingAppDefinitionInput === false && menuState.canSaveAs === true;
      }),
    ).toBe(true);

    await user.type(screen.getByRole("textbox", { name: "Name" }), "Unapplied name again");

    mocks.nativeConfirm.mockResolvedValueOnce(false);
    dispatchMenu("openRecipe");
    await waitFor(() => expect(mocks.nativeConfirm).toHaveBeenCalled());
    expect(mocks.openDialog).toHaveBeenCalledTimes(1);
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Unapplied name again");

    const recipe = recipeDocument();
    mocks.nativeConfirm.mockResolvedValueOnce(true);
    mocks.openDialog.mockResolvedValueOnce(recipe.path);
    mocks.api.openRecipe.mockResolvedValueOnce(apiSuccess({ document: recipe }));
    dispatchMenu("openRecipe");
    await screen.findByRole("heading", { name: "Recipe" });
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Demo Recipe"));
    expect(mocks.api.openRecipe).toHaveBeenCalledWith(recipe.path, null);
    expect(screen.queryByLabelText("App Definition editor")).toBeNull();
  });

  it("dispatches App Definition Save and Save As through the document API and blocks forbidden Save As", async () => {
    await mountEditor();
    const dirtyApp = appDocument("app-session-1", { dirty: true, editRestrictions: {
      identityLocked: false,
      canEditIdentity: true,
      canSaveAs: true,
    } });
    await openAppDefinitionDocument(dirtyApp);

    mocks.api.saveAppDefinition.mockResolvedValueOnce(
      apiSuccess({ document: appDocument("app-session-1", { appDefinition: appDefinition({ name: "Saved" }) }) }),
    );
    dispatchMenu("saveRecipe");
    await waitFor(() => expect(mocks.api.saveAppDefinition).toHaveBeenCalledWith("app-session-1"));
    expect(mocks.api.saveRecipe).not.toHaveBeenCalled();

    mocks.saveDialog.mockResolvedValueOnce("/exports/app.yaml");
    mocks.api.saveAppDefinitionAs.mockResolvedValueOnce(
      apiSuccess({ document: appDocument("app-session-1", { path: "/exports/app.yaml" }) }),
    );
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.api.saveAppDefinitionAs).toHaveBeenCalledWith("app-session-1", "/exports/app.yaml"));
    expect(mocks.api.saveRecipeAs).not.toHaveBeenCalled();

    const restricted = appDocument("app-session-2", {
      owningAuthoredRoot: "/catalog",
      editRestrictions: { identityLocked: true, canEditIdentity: false, canSaveAs: false },
    });
    const restrictedOpen = appDocument("app-session-2", {
      ...restricted,
      path: "/catalog/apps/restricted.yaml",
      appDefinition: appDefinition({
        id: "restricted-app",
        name: "Restricted App",
        package_id: "com.example.restricted",
      }),
    });
    mocks.openDialog.mockResolvedValueOnce(restrictedOpen.path);
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document: restrictedOpen }));
    dispatchMenu("openAppDefinition");
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: "App ID" }) as HTMLInputElement).value).toBe("restricted-app");
    });

    expect((screen.getByRole("button", { name: "Save As" }) as HTMLButtonElement).disabled).toBe(true);
    expect(
      mocks.api.updateMenuState.mock.calls.some(([state]) => {
        const menuState = state as {
          documentKind?: string;
          canSaveAs?: boolean;
          pendingAppDefinitionInput?: boolean;
          canSave?: boolean;
        };
        return menuState.documentKind === "app_definition" &&
          menuState.canSaveAs === false &&
          menuState.pendingAppDefinitionInput === false &&
          menuState.canSave === false;
      }),
    ).toBe(true);

    dispatchMenu("saveRecipeAs");
    expect(await screen.findByText("Save As is not available for this App Definition document.")).toBeTruthy();
    expect(mocks.saveDialog).toHaveBeenCalledTimes(1);
    expect(mocks.api.saveAppDefinitionAs).toHaveBeenCalledTimes(1);
  });

  it("shows App Definition Save admission diagnostics without replacing the active draft", async () => {
    await mountEditor();
    const draft = appDocument("app-session-1", {
      dirty: true,
      canUndo: true,
      appDefinition: appDefinition({ id: "demo-app", name: "Backend draft" }),
      diagnostics: [],
    });
    await openAppDefinitionDocument(draft);

    mocks.api.saveAppDefinition.mockResolvedValueOnce(apiFailure(
      "App Definition save is blocked; review the document diagnostics.",
      {
        documentId: draft.documentId,
        diagnostics: [{
          severity: "error",
          code: "app_definition_recipe_reference",
          message: "An affected Recipe still refers to this App Definition.",
          file: "authored/recipes/affected-recipe.yaml",
          object_kind: "recipe",
          object_id: "affectedRecipe",
          field: "app_ref",
        }],
      },
    ));

    dispatchMenu("saveRecipe");

    expect(await screen.findByText("An affected Recipe still refers to this App Definition.")).toBeTruthy();
    expect(screen.getByText("recipe / affectedRecipe / app_ref")).toBeTruthy();
    expect(screen.getByText("File: authored/recipes/affected-recipe.yaml")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Backend draft");
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(false);
    expect(screen.getByText("Unsaved")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("shows App Definition Save As file diagnostics while preserving path, draft, and history", async () => {
    await mountEditor();
    const draft = appDocument("app-session-1", {
      dirty: true,
      canUndo: true,
      appDefinition: appDefinition({ id: "demo-app", name: "Save As draft" }),
      diagnostics: [],
    });
    await openAppDefinitionDocument(draft);
    mocks.saveDialog.mockResolvedValueOnce("/exports/demo-app.yaml");
    mocks.api.saveAppDefinitionAs.mockResolvedValueOnce(apiFailure(
      "App Definition Save As is blocked; review the document diagnostics.",
      {
        documentId: draft.documentId,
        path: "/exports/demo-app.yaml",
        diagnostics: [
          {
            severity: "error",
            code: "app_definition_recipe_reference",
            message: "An affected Recipe still refers to this App Definition.",
            file: "authored/recipes/affected-recipe.yaml",
            object_kind: "recipe",
            object_id: "affectedRecipe",
            field: "app_ref",
          },
          {
            severity: "error",
            code: "app_definition_destination_conflict",
            message: "The requested App Definition destination is unavailable.",
            file: "/exports/demo-app.yaml",
            object_kind: "file",
            object_id: null,
            field: "path",
          },
        ],
      },
    ));

    dispatchMenu("saveRecipeAs");

    expect(await screen.findByText("The requested App Definition destination is unavailable.")).toBeTruthy();
    expect(screen.getByText("recipe / affectedRecipe / app_ref")).toBeTruthy();
    expect(screen.getByText("File: authored/recipes/affected-recipe.yaml")).toBeTruthy();
    expect(screen.getByText("file / path")).toBeTruthy();
    expect(screen.getByText("File: /exports/demo-app.yaml")).toBeTruthy();
    expect(screen.getByText("/catalog/apps/invalid-app.yaml")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Save As draft");
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
    expect(mocks.api.saveAppDefinitionAs).toHaveBeenCalledWith(draft.documentId, "/exports/demo-app.yaml");
    expect(
      mocks.api.updateMenuState.mock.calls.some(([state]) => {
        const menuState = state as { canUndo?: boolean; documentKind?: string };
        return menuState.documentKind === "app_definition" && menuState.canUndo === true;
      }),
    ).toBe(true);
  });

  it("preserves pending form fields while undo and redo refresh accepted non-pending fields", async () => {
    const user = userEvent.setup();
    await mountEditor();
    await openAppDefinitionDocument(appDocument("app-session-1", {
      canUndo: true,
      appDefinition: appDefinition({ id: "demo-app", name: "Initial", package_id: "com.example.initial" }),
    }));
    await user.clear(screen.getByRole("textbox", { name: "Name" }));
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Pending local name");

    const undone = appDocument("app-session-1", {
      appDefinition: appDefinition({ id: "demo-app", name: "Undone", package_id: "com.example.undo" }),
      canUndo: false,
      canRedo: true,
    });
    mocks.api.undo.mockResolvedValueOnce(apiSuccess({ commandResult: { changed: true }, document: undone }));
    dispatchMenu("undo");
    await waitFor(() => expect(mocks.api.undo).toHaveBeenCalledWith("app-session-1"));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Package ID" }) as HTMLInputElement).value).toBe("com.example.undo"));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Pending local name");

    const redone = appDocument("app-session-1", {
      appDefinition: appDefinition({ id: "demo-app", name: "Redone", package_id: "com.example.redo" }),
      canUndo: true,
      canRedo: false,
    });
    mocks.api.redo.mockResolvedValueOnce(apiSuccess({ commandResult: { changed: true }, document: redone }));
    dispatchMenu("redo");
    await waitFor(() => expect(mocks.api.redo).toHaveBeenCalledWith("app-session-1"));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Package ID" }) as HTMLInputElement).value).toBe("com.example.redo"));
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Pending local name");
  });

  it("counts local input in dirty-close prompts and preserves it through restart until disk reopen", async () => {
    const user = userEvent.setup();
    await mountEditor();
    const original = appDocument();
    await openAppDefinitionDocument(original);
    await user.type(screen.getByRole("textbox", { name: "Name" }), "Local value before restart");

    const closeEvent = { preventDefault: vi.fn() };
    const closeHandler = mocks.closeRequestHandler as ((event: typeof closeEvent) => Promise<void>) | null;
    if (closeHandler === null) throw new Error("The window close handler has not been installed.");
    mocks.nativeConfirm.mockResolvedValueOnce(false);
    await act(async () => closeHandler(closeEvent));
    expect(closeEvent.preventDefault).toHaveBeenCalled();
    expect(mocks.nativeConfirm).toHaveBeenCalledWith(
      expect.stringContaining("invalid app id"),
      expect.objectContaining({ title: "Discard unsaved changes" }),
    );

    mocks.api.sidecarRestart.mockResolvedValueOnce(
      apiSuccess({ status: { running: true, pid: 124, compatible: true, protocolVersion: 1 }, documentSessionsPreserved: false }),
    );
    dispatchMenu("restartSidecar");
    expect(await screen.findByText("Session invalid")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Local value before restart");

    const reopened = appDocument("app-session-2", { appDefinition: appDefinition({ name: "Disk value" }) });
    mocks.nativeConfirm.mockResolvedValueOnce(true);
    mocks.api.openAppDefinition.mockResolvedValueOnce(apiSuccess({ document: reopened }));
    await user.click(screen.getByRole("button", { name: "Reopen from Disk" }));
    await waitFor(() => expect(mocks.api.openAppDefinition).toHaveBeenLastCalledWith(original.path, null));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Disk value"));
    expect(screen.getByText("Saved")).toBeTruthy();
  });

  it("keeps normal Recipe open, Save, and Save As routing unchanged", async () => {
    await mountEditor();
    const recipe = recipeDocument("recipe-session-1", { dirty: true });
    mocks.openDialog.mockResolvedValueOnce(recipe.path);
    mocks.api.openRecipe.mockResolvedValueOnce(apiSuccess({ document: recipe }));
    dispatchMenu("openRecipe");
    await screen.findByRole("heading", { name: "Recipe" });
    expect((screen.getByRole("button", { name: "Save As" }) as HTMLButtonElement).disabled).toBe(false);
    await waitFor(() => {
      expect(
        mocks.api.updateMenuState.mock.calls.some(([state]) => {
          const menuState = state as { documentKind?: string; canSave?: boolean; canSaveAs?: boolean };
          return menuState.documentKind === "recipe" &&
            menuState.canSave === true &&
            menuState.canSaveAs === true;
        }),
      ).toBe(true);
    });

    mocks.api.saveRecipe.mockResolvedValueOnce(apiSuccess({ document: recipeDocument() }));
    dispatchMenu("saveRecipe");
    await waitFor(() => expect(mocks.api.saveRecipe).toHaveBeenCalledWith("recipe-session-1"));
    expect(mocks.api.saveAppDefinition).not.toHaveBeenCalled();

    mocks.saveDialog.mockResolvedValueOnce("/exports/recipe.yaml");
    mocks.api.saveRecipeAs.mockResolvedValueOnce(
      apiSuccess({ document: recipeDocument("recipe-session-1", { path: "/exports/recipe.yaml" }) }),
    );
    dispatchMenu("saveRecipeAs");
    await waitFor(() => expect(mocks.api.saveRecipeAs).toHaveBeenCalledWith("recipe-session-1", "/exports/recipe.yaml"));
    expect(mocks.saveDialog).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: recipe.path }));
    expect(mocks.api.saveAppDefinitionAs).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("App Definition editor")).toBeNull();
  });
});

describe("Recipe validation refresh", () => {
  it("applies the full refreshed Recipe document returned by Validate", async () => {
    await mountEditor();
    const recipe = recipeDocument();
    mocks.openDialog.mockResolvedValueOnce(recipe.path);
    mocks.api.openRecipe.mockResolvedValueOnce(apiSuccess({ document: recipe }));
    dispatchMenu("openRecipe");
    await screen.findByRole("heading", { name: "Recipe" });
    await waitFor(() =>
      expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe(
        "Demo Recipe",
      ),
    );

    const refreshed = recipeDocument("recipe-session-1", {
      dirty: true,
      canUndo: true,
      recipe: { ...recipe.recipe, name: "Refreshed Recipe" },
    });
    mocks.api.validate.mockResolvedValueOnce(
      apiSuccess({ diagnostics: refreshed.diagnostics, document: refreshed }),
    );

    dispatchMenu("validate");
    await waitFor(() => expect(mocks.api.validate).toHaveBeenCalledWith("recipe-session-1"));
    await waitFor(() =>
      expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe(
        "Refreshed Recipe",
      ),
    );

    // The refreshed projection drives menu state, proving the applied document
    // is the one the sidecar returned rather than a diagnostics-only update.
    expect(
      mocks.api.updateMenuState.mock.calls.some(([state]) => {
        const menuState = state as { canUndo?: boolean };
        return menuState.canUndo === true;
      }),
    ).toBe(true);
  });
});

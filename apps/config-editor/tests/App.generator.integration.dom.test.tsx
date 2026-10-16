import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RecipeDocumentDto } from "../src/api/types";

const mocks = vi.hoisted(() => ({
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
    getDocument: vi.fn(),
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
  open: vi.fn(),
  save: vi.fn(),
}));

vi.mock("../src/api/editorApi", () => mocks.api);

// The generator's authoring flows are covered by the AppGenerator suite; this
// boundary stub exercises how the shell consumes a publication outcome.
vi.mock("../src/components/AppGenerator", () => {
  const savedOutcome = (documentId: string | null) => ({
    outcome: "saved" as const,
    operationId: "operation-1",
    app: { appPath: "apps/example.yaml" },
    recipe: {
      recipePath: "recipes/app.example.install.yaml",
      documentId,
      openError:
        documentId === null
          ? { code: "document_open_failed", message: "Locked by another session" }
          : null,
    },
    createdFiles: ["apps/example.yaml", "recipes/app.example.install.yaml"],
    replacedFiles: [],
  });
  return {
    AppGenerator: ({ onClose, onSaved }: {
      initialAuthoredRoot: string | null;
      onAuthoredRootSelected: (path: string | null) => void | Promise<void>;
      onClose: () => void;
      onSaved: (result: ReturnType<typeof savedOutcome>) => void;
    }) => (
      <div>
        <button onClick={() => onSaved(savedOutcome("document-1"))}>stub save with opened Recipe</button>
        <button onClick={() => onSaved(savedOutcome(null))}>stub save with unopened Recipe</button>
        <button onClick={onClose}>stub close</button>
      </div>
    ),
  };
});

import App from "../src/App";

const apiSuccess = <T,>(result: T) => ({ kind: "success" as const, result });

function recipeDocument(): RecipeDocumentDto {
  return {
    documentId: "document-1",
    path: "/catalog/recipes/app.example.install.yaml",
    authoredRoot: "/catalog",
    dirty: false,
    canUndo: false,
    canRedo: false,
    recipe: {
      schemaVersion: 1,
      kind: "recipe",
      id: "app.example.install",
      name: "Install Example",
      description: "",
      recipeDependencies: [],
      provides: { features: [] },
      inputs: {},
      artifacts: {},
      artifactGroups: {},
      steps: [],
    },
    yaml: "kind: recipe\nid: app.example.install\n",
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
  };
}

function dispatchMenu(action: string) {
  const handler = mocks.menuActionHandler as ((event: { payload: { action: string } }) => void) | null;
  if (handler === null) throw new Error("The native menu listener has not been installed.");
  handler({ payload: { action } });
}

async function mountEditor() {
  render(<App />);
  await waitFor(() => expect(mocks.menuActionHandler).not.toBeNull());
  await waitFor(() => expect(mocks.api.listStepSpecs).toHaveBeenCalled());
}

async function openStubGenerator() {
  await waitFor(() => {
    dispatchMenu("generateAppRecipe");
    expect(screen.queryByRole("button", { name: "stub save with opened Recipe" })).not.toBeNull();
  });
}

beforeEach(() => {
  mocks.nativeConfirm.mockReset();
  mocks.menuListener.mockReset();
  mocks.closeWindow.mockReset();
  for (const mock of Object.values(mocks.api)) {
    mock.mockReset();
  }
  mocks.menuActionHandler = null;
  mocks.closeRequestHandler = null;

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

describe("App generator publication wiring", () => {
  it("reports the saved paths with manual-open recovery when the Recipe document did not open", async () => {
    await mountEditor();
    await openStubGenerator();

    fireEvent.click(screen.getByRole("button", { name: "stub save with unopened Recipe" }));

    await waitFor(() =>
      expect(
        screen.getByText(
          "Saved apps/example.yaml and recipes/app.example.install.yaml. The editor could not open the saved Recipe automatically: Locked by another session. Open recipes/app.example.install.yaml manually to review it.",
        ),
      ).toBeTruthy(),
    );
    expect(mocks.api.getDocument).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "stub save with unopened Recipe" })).toBeNull();
  });

  it("opens the saved Recipe document when the publication returns a document id", async () => {
    await mountEditor();
    await openStubGenerator();

    mocks.api.getDocument.mockResolvedValue(apiSuccess({ document: recipeDocument() }));
    fireEvent.click(screen.getByRole("button", { name: "stub save with opened Recipe" }));

    await waitFor(() => expect(mocks.api.getDocument).toHaveBeenCalledWith("document-1"));
    expect(await screen.findByRole("heading", { name: "Recipe" })).toBeTruthy();
    expect(
      screen.getByText("Saved apps/example.yaml and recipes/app.example.install.yaml."),
    ).toBeTruthy();
  });
});

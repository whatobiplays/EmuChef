import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EditorCommand } from "../src/api/commands";
import type { AppCatalogDto, ArtifactDto, InputDto, RecipeDocumentDto } from "../src/api/types";
import { ArtifactsEditor } from "../src/components/ArtifactsEditor";

const catalog: AppCatalogDto = {
  status: "ready",
  apps: [
    {
      id: "demo_app",
      name: "Demo App",
      artifacts: [
        { id: "apk", kind: "apk", requiresInput: false },
        { id: "config", kind: "file", requiresInput: true },
      ],
      permissionSets: ["baseline"],
      targets: [{ id: "system", kind: "directory", location: "shared_storage", path: "Demo/system" }],
      launcherActivity: "com.example.demo.MainActivity",
    },
    {
      id: "second_app",
      name: "Second App",
      artifacts: [{ id: "apk", kind: "apk", requiresInput: false }],
      permissionSets: [],
      targets: [],
      launcherActivity: null,
    },
  ],
  diagnostics: [],
};

function inputDto(id: string, label: string): InputDto {
  return {
    id,
    recipeId: "demo-recipe",
    inputId: id,
    key: id,
    type: "file",
    role: "source",
    label,
    description: "",
    required: true,
    multiple: false,
    validation: { mustExist: true, allowedExtensions: [], pathKind: "file", allowedPrefixes: [] },
    default: null,
    options: [],
    sensitive: false,
    advanced: false,
    metadata: {},
  };
}

function recipeDocument({
  artifacts = {},
  inputs = { apk_file: inputDto("apk_file", "APK file") },
  appCatalog = catalog,
}: {
  artifacts?: Record<string, ArtifactDto>;
  inputs?: Record<string, InputDto>;
  appCatalog?: AppCatalogDto;
} = {}): RecipeDocumentDto {
  return {
    documentId: "recipe-session-1",
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
      inputs,
      artifacts,
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
    appCatalog,
  };
}

function applyArtifactCommand(document: RecipeDocumentDto, command: EditorCommand): RecipeDocumentDto {
  if (command.type === "AddAppArtifact") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        artifacts: {
          ...document.recipe.artifacts,
          [command.artifactId]: {
            id: command.artifactId,
            type: "app_artifact",
            appRef: command.appRef,
            artifact: command.artifact,
            input: command.input ?? undefined,
            cache: "default",
          },
        },
      },
    };
  }
  if (command.type === "AddArtifact") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        artifacts: {
          ...document.recipe.artifacts,
          [command.artifactId]: {
            id: command.artifactId,
            type: "remote_file",
            url: command.url,
            cache: "default",
          },
        },
      },
    };
  }
  if (command.type === "UpdateArtifactField") {
    const artifact = document.recipe.artifacts[command.artifactId];
    if (!artifact) {
      return document;
    }
    return {
      ...document,
      recipe: {
        ...document.recipe,
        artifacts: {
          ...document.recipe.artifacts,
          [command.artifactId]: { ...artifact, [command.field]: command.value ?? undefined },
        },
      },
    };
  }
  return document;
}

function ArtifactsHarness({
  initialDocument,
  onCommand,
}: {
  initialDocument: RecipeDocumentDto;
  onCommand: (command: EditorCommand) => Promise<boolean>;
}) {
  const [document, setDocument] = useState(initialDocument);
  return (
    <ArtifactsEditor
      confirmAction={async () => true}
      document={document}
      promptForId={async () => "new_artifact"}
      promptForRequiredText={async () => "https://example.com/app.apk"}
      onCommand={async (command) => {
        const ok = await onCommand(command);
        if (ok) {
          setDocument((current) => applyArtifactCommand(current, command));
        }
        return ok;
      }}
    />
  );
}

describe("ArtifactsEditor catalog-backed App Artifacts", () => {
  it("adds an App Artifact authored only from catalog choices", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(<ArtifactsHarness initialDocument={recipeDocument()} onCommand={onCommand} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));

    expect((screen.getByRole("combobox", { name: "App Definition" }) as HTMLSelectElement).value).toBe(
      "demo_app",
    );
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("apk");
    expect((screen.getByRole("textbox", { name: "Artifact id" }) as HTMLInputElement).value).toBe("apk");

    await user.selectOptions(screen.getByRole("combobox", { name: "App artifact" }), "config");
    await user.clear(screen.getByRole("textbox", { name: "Artifact id" }));
    await user.type(screen.getByRole("textbox", { name: "Artifact id" }), "app_config");
    await user.selectOptions(screen.getByRole("combobox", { name: "File input" }), "apk_file");
    await user.click(screen.getByRole("button", { name: "Add app artifact" }));

    expect(onCommand).toHaveBeenCalledWith({
      type: "AddAppArtifact",
      artifactId: "app_config",
      appRef: "demo_app",
      artifact: "config",
      input: "apk_file",
    });
    expect(screen.queryByRole("textbox", { name: "Artifact id" })).toBeNull();
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("config");
  });

  it("lets the catalog define the artifact seed while the author edits the Recipe id", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            apk: { id: "apk", type: "app_artifact", appRef: "demo_app", artifact: "apk", cache: "default" },
          },
        })}
        onCommand={onCommand}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Add app" }));
    expect((screen.getByRole("textbox", { name: "Artifact id" }) as HTMLInputElement).value).toBe("apk_2");

    await user.selectOptions(screen.getByRole("combobox", { name: "App Definition" }), "second_app");
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("apk");
    expect((screen.getByRole("textbox", { name: "Artifact id" }) as HTMLInputElement).value).toBe("apk_2");

    await user.clear(screen.getByRole("textbox", { name: "Artifact id" }));
    await user.type(screen.getByRole("textbox", { name: "Artifact id" }), "second_apk");
    await user.click(screen.getByRole("button", { name: "Add app artifact" }));

    expect(onCommand).toHaveBeenCalledWith({
      type: "AddAppArtifact",
      artifactId: "second_apk",
      appRef: "second_app",
      artifact: "apk",
      input: null,
    });
  });

  it("edits one App Artifact reference field at a time without rewriting dependent values", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_config: {
              id: "demo_config",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "config",
              cache: "default",
            },
          },
        })}
        onCommand={onCommand}
      />,
    );

    await user.selectOptions(screen.getByRole("combobox", { name: "File input" }), "apk_file");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateArtifactField",
      artifactId: "demo_config",
      field: "input",
      value: "apk_file",
    });

    // A required user-provided binding cannot transition to null, so the
    // control offers no "No input binding" option.
    expect(screen.queryByRole("option", { name: "No input binding" })).toBeNull();

    await user.selectOptions(screen.getByRole("combobox", { name: "App Definition" }), "second_app");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateArtifactField",
      artifactId: "demo_config",
      field: "appRef",
      value: "second_app",
    });
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("config");
  });

  it("keeps retained App Artifact references visible when the catalog does not define them", () => {
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            ghost_apk: {
              id: "ghost_apk",
              type: "app_artifact",
              appRef: "ghost_app",
              artifact: "ghost",
              input: "ghost_input",
              cache: "default",
            },
          },
        })}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    expect((screen.getByRole("combobox", { name: "App Definition" }) as HTMLSelectElement).value).toBe(
      "ghost_app",
    );
    expect(screen.getByRole("option", { name: "ghost_app (not in this catalog)" })).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("ghost");
    expect(screen.getByRole("option", { name: "ghost (not defined by this App Definition)" })).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "File input" }) as HTMLSelectElement).value).toBe(
      "ghost_input",
    );
    expect(screen.getByRole("option", { name: "ghost_input (not defined in this Recipe)" })).toBeTruthy();
  });

  it("fails closed for App Artifact authoring while the catalog is unavailable", () => {
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({ appCatalog: { status: "unbound", apps: [], diagnostics: [] } })}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );
    expect((screen.getByRole("button", { name: "Add app" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("keeps the remote file artifact flow unchanged", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(<ArtifactsHarness initialDocument={recipeDocument()} onCommand={onCommand} />);

    await user.click(screen.getByRole("button", { name: "Add" }));

    expect(onCommand).toHaveBeenCalledWith({
      type: "AddArtifact",
      artifactId: "new_artifact",
      url: "https://example.com/app.apk",
    });
  });

  it("disables submission and keeps the draft when a refresh removes the catalog", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const editorProps = (activeDocument: RecipeDocumentDto) => ({
      confirmAction: async () => true,
      document: activeDocument,
      onCommand,
      promptForId: async () => "new_artifact",
      promptForRequiredText: async () => "https://example.com/app.apk",
    });
    const { rerender } = render(<ArtifactsEditor {...editorProps(recipeDocument())} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    expect((screen.getByRole("button", { name: "Add app artifact" }) as HTMLButtonElement).disabled).toBe(false);

    rerender(
      <ArtifactsEditor
        {...editorProps(recipeDocument({ appCatalog: { status: "invalid", apps: [], diagnostics: [] } }))}
      />,
    );
    expect((screen.getByRole("button", { name: "Add app artifact" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/catalog is not available/)).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "App Definition" }) as HTMLSelectElement).value).toBe("demo_app");
  });

  it("disables submission when a refreshed catalog no longer projects the draft App Definition", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const editorProps = (activeDocument: RecipeDocumentDto) => ({
      confirmAction: async () => true,
      document: activeDocument,
      onCommand,
      promptForId: async () => "new_artifact",
      promptForRequiredText: async () => "https://example.com/app.apk",
    });
    const { rerender } = render(<ArtifactsEditor {...editorProps(recipeDocument())} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    rerender(
      <ArtifactsEditor
        {...editorProps(
          recipeDocument({
            appCatalog: {
              status: "ready",
              apps: [
                {
                  id: "other_app",
                  name: "Other App",
                  artifacts: [{ id: "apk", kind: "apk", requiresInput: false }],
                  permissionSets: [],
                  targets: [],
                  launcherActivity: null,
                },
              ],
              diagnostics: [],
            },
          }),
        )}
      />,
    );

    expect((screen.getByRole("button", { name: "Add app artifact" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("App Definition demo_app is not in the current catalog.")).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "App Definition" }) as HTMLSelectElement).value).toBe("demo_app");
    expect(screen.getByRole("option", { name: "demo_app (not in this catalog)" })).toBeTruthy();
  });

  it("disables submission when a refreshed catalog drops the selected App artifact", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const editorProps = (activeDocument: RecipeDocumentDto) => ({
      confirmAction: async () => true,
      document: activeDocument,
      onCommand,
      promptForId: async () => "new_artifact",
      promptForRequiredText: async () => "https://example.com/app.apk",
    });
    const { rerender } = render(<ArtifactsEditor {...editorProps(recipeDocument())} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    const demoApp = catalog.apps[0];
    rerender(
      <ArtifactsEditor
        {...editorProps(
          recipeDocument({
            appCatalog: {
              status: "ready",
              apps: [{ ...demoApp, artifacts: [{ id: "config", kind: "file", requiresInput: true }] }],
              diagnostics: [],
            },
          }),
        )}
      />,
    );

    expect((screen.getByRole("button", { name: "Add app artifact" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("App Definition demo_app has no artifact apk.")).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement).value).toBe("apk");
    expect(screen.getByRole("option", { name: "apk (not defined by this App Definition)" })).toBeTruthy();
  });

  it("blocks binding a Recipe input to a non-user-provided App Artifact", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(<ArtifactsHarness initialDocument={recipeDocument()} onCommand={onCommand} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    expect(screen.queryByRole("combobox", { name: "File input" })).toBeNull();
    expect(screen.getByText(/is not user-provided, so it cannot bind a Recipe file input/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Add app artifact" }));
    expect(onCommand).toHaveBeenCalledWith({
      type: "AddAppArtifact",
      artifactId: "apk",
      appRef: "demo_app",
      artifact: "apk",
      input: null,
    });
  });

  it("requires a single-file Recipe input for a user-provided App Artifact", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(<ArtifactsHarness initialDocument={recipeDocument()} onCommand={onCommand} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "App artifact" }), "config");
    await user.clear(screen.getByRole("textbox", { name: "Artifact id" }));
    await user.type(screen.getByRole("textbox", { name: "Artifact id" }), "app_config");

    await user.click(screen.getByRole("button", { name: "Add app artifact" }));
    expect(onCommand).not.toHaveBeenCalled();
    expect(screen.getByText(/must bind one single-file Recipe input/)).toBeTruthy();
  });

  it("asks for a single-file Recipe input when the Recipe declares none", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(<ArtifactsHarness initialDocument={recipeDocument({ inputs: {} })} onCommand={onCommand} />);

    await user.click(screen.getByRole("button", { name: "Add app" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "App artifact" }), "config");
    await user.click(screen.getByRole("button", { name: "Add app artifact" }));

    expect(onCommand).not.toHaveBeenCalled();
    expect(screen.getByText(/Add a single-file Recipe input first/)).toBeTruthy();
  });

  it("flags a user-provided App Artifact that has no input binding", () => {
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_config: {
              id: "demo_config",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "config",
              cache: "default",
            },
          },
        })}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    expect(screen.getByRole("combobox", { name: "File input" })).toBeTruthy();
    expect(screen.getByText(/must bind one single-file Recipe input/)).toBeTruthy();
  });

  it("retains an invalid binding on a non-user-provided App Artifact with correction guidance", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_apk: {
              id: "demo_apk",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "apk",
              input: "apk_file",
              cache: "default",
            },
          },
        })}
        onCommand={onCommand}
      />,
    );

    const inputSelect = screen.getByRole("combobox", { name: "File input" }) as HTMLSelectElement;
    expect(inputSelect.value).toBe("apk_file");
    expect(screen.getByText(/Only a user-provided App Artifact can bind a Recipe file input/)).toBeTruthy();

    await user.selectOptions(inputSelect, "");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateArtifactField",
      artifactId: "demo_apk",
      field: "input",
      value: null,
    });
  });

  it("keeps a required user-provided binding and offers no null binding transition", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_config: {
              id: "demo_config",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "config",
              input: "apk_file",
              cache: "default",
            },
          },
        })}
        onCommand={onCommand}
      />,
    );

    const inputSelect = screen.getByRole("combobox", { name: "File input" }) as HTMLSelectElement;
    expect(inputSelect.value).toBe("apk_file");
    expect(screen.queryByRole("option", { name: "No input binding" })).toBeNull();
    expect(Array.from(inputSelect.options).map((option) => option.value)).not.toContain("");

    fireEvent.change(inputSelect, { target: { value: "" } });
    expect(onCommand).not.toHaveBeenCalled();

    await user.selectOptions(inputSelect, "apk_file");
    expect(onCommand).not.toHaveBeenCalled();
  });

  it("keeps the authored binding visible and disables binding edits while the catalog does not resolve the input policy", () => {
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_payload: {
              id: "demo_payload",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "missing_payload",
              input: "apk_file",
              cache: "default",
            },
          },
        })}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const inputSelect = screen.getByRole("combobox", { name: "File input" }) as HTMLSelectElement;
    expect(inputSelect.value).toBe("apk_file");
    expect(inputSelect.disabled).toBe(true);
    expect(screen.getByText(/does not answer whether this artifact requires a Recipe file input/)).toBeTruthy();
    expect(screen.getByText(/current binding stays as authored/)).toBeTruthy();
  });

  it("does not allow authoring a binding while the catalog does not resolve the input policy", () => {
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_payload: {
              id: "demo_payload",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "missing_payload",
              cache: "default",
            },
          },
        })}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    expect(screen.queryByRole("combobox", { name: "File input" })).toBeNull();
    expect(screen.getByText(/no binding can be authored yet/)).toBeTruthy();
  });

  it("never clears a binding when the App artifact changes to a non-user-provided artifact", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_config: {
              id: "demo_config",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "config",
              input: "apk_file",
              cache: "default",
            },
          },
        })}
        onCommand={onCommand}
      />,
    );

    const appArtifactSelect = screen.getByRole("combobox", { name: "App artifact" }) as HTMLSelectElement;
    expect(appArtifactSelect.value).toBe("config");
    await user.selectOptions(appArtifactSelect, "apk");

    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateArtifactField",
      artifactId: "demo_config",
      field: "artifact",
      value: "apk",
    });
    expect(onCommand).not.toHaveBeenCalledWith(expect.objectContaining({ field: "input" }));
    expect((screen.getByRole("combobox", { name: "File input" }) as HTMLSelectElement).value).toBe("apk_file");
    expect(screen.getByText(/Only a user-provided App Artifact can bind a Recipe file input/)).toBeTruthy();
  });

  it("offers Rename, Duplicate, and explicit Delete for App Artifacts", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const appArtifact = render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            demo_apk: {
              id: "demo_apk",
              type: "app_artifact",
              appRef: "demo_app",
              artifact: "apk",
              cache: "default",
            },
          },
        })}
        onCommand={onCommand}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Rename" }));
    expect(onCommand).toHaveBeenCalledWith({
      type: "RenameArtifact",
      artifactId: "demo_apk",
      newArtifactId: "new_artifact",
    });

    await user.click(screen.getByRole("button", { name: "Duplicate" }));
    expect(onCommand).toHaveBeenCalledWith({
      type: "DuplicateArtifact",
      sourceArtifactId: "demo_apk",
      newArtifactId: "new_artifact",
    });

    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onCommand).toHaveBeenCalledWith({ type: "DeleteArtifact", artifactId: "demo_apk" });

    appArtifact.unmount();
    render(
      <ArtifactsHarness
        initialDocument={recipeDocument({
          artifacts: {
            remote: { id: "remote", type: "remote_file", url: "https://example.com/a.apk", cache: "default" },
          },
        })}
        onCommand={onCommand}
      />,
    );
    expect(screen.getByRole("button", { name: "Rename" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete" })).toBeTruthy();
  });
});

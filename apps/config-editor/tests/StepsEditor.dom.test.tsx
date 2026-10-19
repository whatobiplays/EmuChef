import { useState } from "react";

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EditorCommand } from "../src/api/commands";
import type {
  AppCatalogDto,
  RefCandidateDto,
  RecipeDocumentDto,
  StepDto,
  StepSpecDto,
} from "../src/api/types";
import type { AdvancedCommandResult } from "../src/components/AdvancedStepInternalsEditor";
import { StepsEditor } from "../src/components/StepsEditor";

/**
 * DOM interaction coverage for catalog-backed App-aware Recipe editing.
 *
 * The step spec fixtures mirror the Rust step registry: `install_apk`,
 * `grant_permissions`, and `launch_app` require App Definition context,
 * `copy_files` accepts optional context, and `wait` consumes none. Every
 * scenario drives the structured controls the way an author does and asserts
 * that dependent controls update from the projected catalog, with no
 * free-form identifier entry anywhere.
 */

const readyCatalog: AppCatalogDto = {
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
      targets: [
        { id: "system", kind: "directory", location: "shared_storage", path: "Demo/system" },
        { id: "config_file", kind: "file", location: "external_app_data", path: "demo.cfg" },
      ],
      launcherActivity: "com.example.demo.MainActivity",
    },
    {
      id: "plain_app",
      name: "Plain App",
      artifacts: [],
      permissionSets: ["elevated"],
      targets: [],
      launcherActivity: null,
    },
  ],
  diagnostics: [],
};

const stepSpecs: StepSpecDto[] = [
  {
    type: "install_apk",
    label: "Install APK",
    supported: true,
    appContext: "required",
    primaryOutputName: null,
    outputs: [],
    paramOrder: ["app", "replace_existing"],
    params: {
      app: {
        acceptedSources: ["input_ref", "artifact_ref", "step_output_ref"],
        acceptedValueTypes: ["file_path"],
        required: true,
        enumValues: [],
      },
      replace_existing: {
        acceptedSources: ["literal"],
        acceptedValueTypes: ["boolean"],
        required: false,
        enumValues: [],
      },
    },
    defaults: { replace_existing: false },
  },
  {
    type: "grant_permissions",
    label: "Grant Permissions",
    supported: true,
    appContext: "required",
    primaryOutputName: null,
    outputs: [],
    paramOrder: ["permission_set"],
    params: {
      permission_set: {
        acceptedSources: ["literal"],
        acceptedValueTypes: ["string"],
        required: true,
        enumValues: ["baseline", "elevated"],
      },
    },
    defaults: {},
  },
  {
    type: "copy_files",
    label: "Copy Files",
    supported: true,
    appContext: "optional",
    primaryOutputName: "copied_paths",
    outputs: [{ name: "copied_paths", valueType: "path_list", primary: true }],
    paramOrder: ["source", "dest", "copy_policy"],
    params: {
      source: {
        acceptedSources: ["input_ref", "artifact_ref", "step_output_ref"],
        acceptedValueTypes: ["file_path", "directory_path", "path_list"],
        required: true,
        enumValues: [],
      },
      dest: {
        acceptedSources: ["literal", "input_ref", "app_target"],
        acceptedValueTypes: ["device_path"],
        required: true,
        enumValues: [],
      },
      copy_policy: {
        acceptedSources: ["literal", "input_ref"],
        acceptedValueTypes: ["string"],
        required: false,
        enumValues: ["merge", "sync", "replace"],
      },
    },
    defaults: { copy_policy: "merge" },
  },
  {
    type: "launch_app",
    label: "Launch App",
    supported: true,
    appContext: "required",
    primaryOutputName: null,
    outputs: [],
    paramOrder: ["package_name", "activity"],
    params: {
      package_name: { acceptedSources: ["literal"], acceptedValueTypes: ["string"], required: false, enumValues: [] },
      activity: { acceptedSources: ["literal"], acceptedValueTypes: ["string"], required: false, enumValues: [] },
    },
    defaults: {},
  },
  {
    type: "wait",
    label: "Wait",
    supported: true,
    appContext: "none",
    primaryOutputName: null,
    outputs: [],
    paramOrder: [],
    params: {},
    defaults: {},
  },
];

const refCandidates: RefCandidateDto[] = [
  { ref: "inputs.payload", label: "Payload file", valueType: "file_path", sourceKind: "input_ref", sourceId: "payload" },
  { ref: "artifacts.apk.local_path", label: "Demo APK", valueType: "file_path", sourceKind: "artifact_ref", sourceId: "apk" },
  { ref: "artifacts.config.local_path", label: "Config file", valueType: "file_path", sourceKind: "artifact_ref", sourceId: "config" },
  { ref: "artifacts.other_apk.local_path", label: "Other APK", valueType: "file_path", sourceKind: "artifact_ref", sourceId: "other_apk" },
];

function buildStep(overrides: Partial<StepDto> & Pick<StepDto, "id" | "type">): StepDto {
  const { id, type, ...rest } = overrides;
  return {
    id,
    type,
    name: id,
    description: "",
    userToggleable: true,
    dependencies: [],
    constraints: { capabilities: [], conflictsWith: [] },
    skipIf: [],
    params: {},
    verify: [],
    ...rest,
  };
}

function recipeDocument(steps: StepDto[], appCatalog: AppCatalogDto = readyCatalog): RecipeDocumentDto {
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
      recipeDependencies: [],
      provides: { features: [] },
      inputs: {},
      artifacts: {
        apk: { id: "apk", type: "app_artifact", appRef: "demo_app", artifact: "apk", cache: "default" },
        other_apk: { id: "other_apk", type: "app_artifact", appRef: "other_app", artifact: "apk", cache: "default" },
      },
      artifactGroups: {},
      steps,
    },
    yaml: "",
    diagnostics: [],
    refIndex: {
      inputRefs: ["inputs.payload"],
      artifactRefs: refCandidates.filter((candidate) => candidate.sourceKind === "artifact_ref").map((candidate) => candidate.ref),
      stepRefs: [],
      stepOutputRefs: [],
      allRefs: refCandidates.map((candidate) => candidate.ref),
      candidates: refCandidates,
    },
    appCatalog,
  };
}

function applyStepCommand(document: RecipeDocumentDto, command: EditorCommand): RecipeDocumentDto {
  if (command.type === "SetStepAppRef") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        steps: document.recipe.steps.map((step) =>
          step.id === command.stepId ? { ...step, appRef: command.appRef } : step,
        ),
      },
    };
  }
  if (command.type === "UpdateStepParams") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        steps: document.recipe.steps.map((step) =>
          step.id === command.stepId ? { ...step, params: command.params } : step,
        ),
      },
    };
  }
  return document;
}

/**
 * Applies the advanced internals commands the document session accepts.
 *
 * Only the step sidecar fields the Advanced editor owns are rewritten; other
 * commands fall through unchanged so tests fail loudly on unexpected writes.
 */
function applyAdvancedCommand(document: RecipeDocumentDto, command: EditorCommand): RecipeDocumentDto {
  if (command.type === "UpdateStepVerify") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        steps: document.recipe.steps.map((step) =>
          step.id === command.stepId ? { ...step, verify: command.verify } : step,
        ),
      },
    };
  }
  if (command.type === "UpdateStepSkipIf") {
    return {
      ...document,
      recipe: {
        ...document.recipe,
        steps: document.recipe.steps.map((step) =>
          step.id === command.stepId ? { ...step, skipIf: command.skipIf } : step,
        ),
      },
    };
  }
  return document;
}

function StepsHarness({
  initialDocument,
  onCommand,
  onAdvancedCommand,
}: {
  initialDocument: RecipeDocumentDto;
  onCommand: (command: EditorCommand) => Promise<boolean>;
  onAdvancedCommand?: (command: EditorCommand) => Promise<AdvancedCommandResult>;
}) {
  const [document, setDocument] = useState(initialDocument);
  return (
    <StepsEditor
      confirmAction={async () => true}
      document={document}
      promptForId={async () => "new_step"}
      stepSpecs={stepSpecs}
      onAdvancedCommand={async (command) => {
        const result = onAdvancedCommand ? await onAdvancedCommand(command) : { ok: false, changed: false };
        if (result.ok && result.changed) {
          setDocument((current) => applyAdvancedCommand(current, command));
        }
        return result;
      }}
      onCommand={async (command) => {
        const ok = await onCommand(command);
        if (ok) {
          setDocument((current) => applyStepCommand(current, command));
        }
        return ok;
      }}
    />
  );
}

function paramRow(paramName: string): HTMLElement {
  const nameBlock = screen.getByText(paramName, { selector: "div.break-words" });
  const row = nameBlock.closest("div[class*='sm:grid-cols']");
  if (!(row instanceof HTMLElement)) {
    throw new Error("Param row not found for " + paramName);
  }
  return row;
}

function selectValues(select: HTMLSelectElement): string[] {
  return Array.from(select.options).map((option) => option.value);
}

/**
 * The Advanced internals Verify section.
 *
 * The add-entry Type select is scoped through this helper because the step
 * detail page also renders parameter and App Definition selects.
 */
function verifySectionElement(): HTMLElement {
  const section = screen.getByText("Verify").closest("section");
  if (!(section instanceof HTMLElement)) {
    throw new Error("Verify section not found");
  }
  return section;
}

/**
 * The Advanced internals skip_if section.
 *
 * Scoped the same way as verifySectionElement so assertions cannot match
 * controls rendered by other sections or by the step parameter form.
 */
function skipIfSectionElement(): HTMLElement {
  const section = screen.getByText("skip_if").closest("section");
  if (!(section instanceof HTMLElement)) {
    throw new Error("skip_if section not found");
  }
  return section;
}

/**
 * The App Definition select control.
 *
 * The wrapping label also carries launcher and unavailable-catalog
 * diagnostics text, so the accessible name matches on the label prefix.
 */
function appDefinitionSelect(): HTMLSelectElement {
  return screen.getByRole("combobox", { name: /App Definition/ }) as HTMLSelectElement;
}

function optionByValue(select: HTMLSelectElement, value: string): HTMLOptionElement {
  const option = Array.from(select.options).find((candidate) => candidate.value === value);
  if (option === undefined) {
    throw new Error("Option not found for value " + value);
  }
  return option;
}

describe("StepsEditor catalog-backed App-aware controls", () => {
  it("offers catalog App Definitions for required app context and dispatches SetStepAppRef", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "install", type: "install_apk", params: { app: { ref: "inputs.payload" } } }),
        ])}
        onCommand={onCommand}
      />,
    );

    const select = appDefinitionSelect();
    expect(select.value).toBe("");
    expect(within(select).getByRole("option", { name: "Select an App Definition" })).toBeTruthy();
    expect(within(select).getByRole("option", { name: "Demo App (demo_app)" })).toBeTruthy();
    expect(within(select).getByRole("option", { name: "Plain App (plain_app)" })).toBeTruthy();

    await user.selectOptions(select, "demo_app");
    expect(onCommand).toHaveBeenCalledWith({ type: "SetStepAppRef", stepId: "install", appRef: "demo_app" });
    expect(appDefinitionSelect().value).toBe("demo_app");
  });

  it("restricts install_apk artifact choices to APK artifacts of the selected App Definition", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "install", type: "install_apk", params: { app: { ref: "inputs.payload" } } }),
        ])}
        onCommand={onCommand}
      />,
    );

    const appParamSelect = () => within(paramRow("app")).getByRole("combobox") as HTMLSelectElement;
    expect(selectValues(appParamSelect()).sort()).toEqual(
      ["artifacts.apk.local_path", "artifacts.config.local_path", "artifacts.other_apk.local_path", "inputs.payload"].sort(),
    );

    await user.selectOptions(appDefinitionSelect(), "demo_app");
    expect(selectValues(appParamSelect()).sort()).toEqual(["artifacts.apk.local_path", "inputs.payload"].sort());

    await user.click(within(paramRow("app")).getByRole("button", { name: "Show all refs" }));
    expect(selectValues(appParamSelect()).sort()).toEqual(["artifacts.apk.local_path", "inputs.payload"].sort());
  });

  it("exposes the catalog-backed App Artifact picker for a newly added install_apk step", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "install", type: "install_apk", appRef: "demo_app" }),
        ])}
        onCommand={onCommand}
      />,
    );

    const appRow = paramRow("app");
    expect(within(appRow).queryByRole("textbox")).toBeNull();
    const appSelect = within(appRow).getByRole("combobox") as HTMLSelectElement;
    expect(selectValues(appSelect)).toEqual(["", "artifacts.apk.local_path"]);

    await user.selectOptions(appSelect, "artifacts.apk.local_path");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateStepParams",
      stepId: "install",
      params: { app: { ref: "artifacts.apk.local_path" } },
    });
  });

  it("exposes required copy_files controls for a newly added step", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "copy", type: "copy_files", appRef: "demo_app" }),
        ])}
        onCommand={onCommand}
      />,
    );

    const sourceSelect = within(paramRow("source")).getByRole("combobox") as HTMLSelectElement;
    expect(selectValues(sourceSelect)).toContain("inputs.payload");

    const destSelect = within(paramRow("dest")).getByRole("combobox") as HTMLSelectElement;
    expect(destSelect.value).toBe("");
    expect(within(destSelect).getByRole("option", { name: "Select an App Target" })).toBeTruthy();

    await user.selectOptions(destSelect, "system");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateStepParams",
      stepId: "copy",
      params: { dest: { app_target: "system" } },
    });

    const policySelect = within(paramRow("copy_policy")).getByRole("combobox") as HTMLSelectElement;
    expect(selectValues(policySelect)).toEqual(["", "merge", "sync", "replace"]);
  });

  it("offers only permission sets the selected App Definition defines", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "grant", type: "grant_permissions", appRef: "demo_app", params: { permission_set: "baseline" } }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = within(paramRow("permission_set")).getByRole("combobox") as HTMLSelectElement;
    expect(select.value).toBe("baseline");
    expect(selectValues(select)).toEqual(["baseline"]);
    expect(screen.queryByRole("option", { name: "elevated" })).toBeNull();
  });

  it("keeps an undefined permission set visible for the author", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({ id: "grant", type: "grant_permissions", appRef: "demo_app", params: { permission_set: "legacy_set" } }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = within(paramRow("permission_set")).getByRole("combobox") as HTMLSelectElement;
    expect(select.value).toBe("legacy_set");
    expect(within(select).getByRole("option", { name: "legacy_set (not defined by this App Definition)" })).toBeTruthy();
  });

  it("lists App Targets with planner compatibility and re-evaluates them when the copy policy changes", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            params: {
              source: { ref: "inputs.payload" },
              dest: { app_target: "config_file" },
              copy_policy: "sync",
            },
          }),
        ])}
        onCommand={onCommand}
      />,
    );

    const destSelect = () => within(paramRow("dest")).getByRole("combobox") as HTMLSelectElement;
    expect(optionByValue(destSelect(), "system").disabled).toBe(false);
    const syncTarget = optionByValue(destSelect(), "config_file");
    expect(syncTarget.disabled).toBe(true);
    expect(syncTarget.textContent ?? "").toContain("A sync copy needs a directory target.");

    await user.selectOptions(within(paramRow("copy_policy")).getByRole("combobox"), "merge");
    expect(destSelect().value).toBe("config_file");
    expect(optionByValue(destSelect(), "config_file").disabled).toBe(false);
  });

  it("keeps App Targets with unknown compatibility selectable and labeled", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            params: { source: { ref: "inputs.unresolved" }, dest: { app_target: "system" } },
          }),
        ])}
        onCommand={onCommand}
      />,
    );

    const destSelect = () => within(paramRow("dest")).getByRole("combobox") as HTMLSelectElement;
    const unknownTarget = optionByValue(destSelect(), "config_file");
    expect(unknownTarget.disabled).toBe(false);
    expect(unknownTarget.textContent ?? "").toContain("compatibility is unknown");

    await user.selectOptions(destSelect(), "config_file");
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateStepParams",
      stepId: "copy",
      params: { source: { ref: "inputs.unresolved" }, dest: { app_target: "config_file" } },
    });
  });

  it("switches an App Target destination back to a literal value for the author", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            params: { source: { ref: "inputs.payload" }, dest: { app_target: "system" } },
          }),
        ])}
        onCommand={onCommand}
      />,
    );

    expect(onCommand).not.toHaveBeenCalled();
    await user.click(within(paramRow("dest")).getByRole("button", { name: "Literal" }));
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateStepParams",
      stepId: "copy",
      params: { source: { ref: "inputs.payload" }, dest: "" },
    });
  });

  it("seeds a literal destination for a generic copy_files step without App Target or ref choices", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "copy", type: "copy_files" })])}
        onCommand={onCommand}
      />,
    );

    const destRow = paramRow("dest");
    const literalButton = within(destRow).getByRole("button", { name: "Literal" }) as HTMLButtonElement;
    expect(literalButton.disabled).toBe(false);
    expect(within(destRow).queryByRole("textbox")).toBeNull();

    await user.click(literalButton);
    expect(onCommand).toHaveBeenCalledWith({
      type: "UpdateStepParams",
      stepId: "copy",
      params: { dest: "" },
    });

    // The accepted command round-trips through the harness, so the literal
    // editor becomes available and the required destination stays authorable.
    const destInput = (await within(paramRow("dest")).findByRole("textbox")) as HTMLInputElement;
    await user.type(destInput, "/sdcard/Demo");
    await user.keyboard("{Enter}");
    expect(onCommand).toHaveBeenLastCalledWith({
      type: "UpdateStepParams",
      stepId: "copy",
      params: { dest: "/sdcard/Demo" },
    });
  });

  it("surfaces a missing canonical launcher activity for launch_app", () => {
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const { unmount } = render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "launch", type: "launch_app", appRef: "plain_app" })])}
        onCommand={onCommand}
      />,
    );
    expect(appDefinitionSelect().value).toBe("plain_app");
    expect(screen.getByText(/defines no launcher activity/)).toBeTruthy();

    unmount();
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "launch", type: "launch_app", appRef: "demo_app" })])}
        onCommand={onCommand}
      />,
    );
    expect(appDefinitionSelect().value).toBe("demo_app");
    expect(screen.queryByText(/defines no launcher activity/)).toBeNull();
  });

  it("keeps a retained App Definition reference visible when the catalog does not define it", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "install",
            type: "install_apk",
            appRef: "ghost_app",
            params: { app: { ref: "artifacts.other_apk.local_path" } },
          }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = appDefinitionSelect();
    expect(select.value).toBe("ghost_app");
    expect(within(select).getByRole("option", { name: "ghost_app (not in this catalog)" })).toBeTruthy();

    const appParamSelect = within(paramRow("app")).getByRole("combobox") as HTMLSelectElement;
    expect(appParamSelect.value).toBe("artifacts.other_apk.local_path");
    const retained = Array.from(appParamSelect.options).find(
      (option) => option.value === "artifacts.other_apk.local_path",
    );
    expect(retained?.textContent ?? "").toContain("incompatible");
  });

  it("hides the App Definition control for app-context-free steps", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "wait", type: "wait" })])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );
    expect(screen.queryByRole("combobox", { name: "App Definition" })).toBeNull();
  });

  it("keeps a retained App Definition reference visible and clearable on an app-context-free step", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "wait", type: "wait", appRef: "demo_app" })])}
        onCommand={onCommand}
      />,
    );

    expect(screen.queryByRole("combobox", { name: "App Definition" })).toBeNull();
    expect(screen.getByText("App Definition context")).toBeTruthy();
    expect(screen.getByDisplayValue("demo_app")).toBeTruthy();
    expect(screen.getByText(/does not use an App Definition/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Clear App Definition reference" }));
    expect(onCommand).toHaveBeenCalledWith({ type: "SetStepAppRef", stepId: "wait", appRef: null });
    expect(screen.queryByDisplayValue("demo_app")).toBeNull();
  });

  it("fails closed while the catalog is unbound or invalid", () => {
    const onCommand = vi.fn(async (_command: EditorCommand) => true);
    const step = buildStep({ id: "install", type: "install_apk" });
    const { unmount } = render(
      <StepsHarness
        initialDocument={recipeDocument([step], { status: "unbound", apps: [], diagnostics: [] })}
        onCommand={onCommand}
      />,
    );
    expect(selectValues(appDefinitionSelect())).toEqual([""]);
    expect(screen.getByText(/Bind an authored catalog root/)).toBeTruthy();

    unmount();
    render(
      <StepsHarness
        initialDocument={recipeDocument([step], {
          status: "invalid",
          apps: [],
          diagnostics: [
            {
              severity: "error",
              code: "catalog_invalid",
              message: "catalog file unreadable",
              file: null,
              objectKind: null,
              objectId: null,
              field: null,
            },
          ],
        })}
        onCommand={onCommand}
      />,
    );
    expect(selectValues(appDefinitionSelect())).toEqual([""]);
    expect(screen.getByText(/until catalog diagnostics are resolved/)).toBeTruthy();
  });

  it("keeps legacy package_name rows inspectable as JSON and hides that authoring option", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "install",
            type: "install_apk",
            appRef: "demo_app",
            verify: [{ type: "package_installed", params: { package_name: "com.example.legacy" } }],
          }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const verifySection = verifySectionElement();
    const addTypeSelect = within(verifySection).getByRole("combobox", { name: "Type" }) as HTMLSelectElement;
    expect(selectValues(addTypeSelect)).toEqual([
      "path_exists",
      "file_exists",
      "app_target_path_exists",
      "app_target_file_exists",
      "app_package_installed",
    ]);
    expect(within(verifySection).queryByRole("option", { name: "Package installed" })).toBeNull();
    expect(within(verifySection).getByRole("option", { name: "Package installed (App Definition)" })).toBeTruthy();
    expect(within(verifySection).getByText("JSON check")).toBeTruthy();
    expect(within(verifySection).queryByRole("textbox", { name: "Package name" })).toBeNull();
    expect((verifySection.querySelector("textarea") as HTMLTextAreaElement).value).toContain(
      '"package_name": "com.example.legacy"',
    );
  });

  it("does not offer a package_name condition for authoring without app context", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "wait", type: "wait" })])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const addTypeSelect = within(verifySectionElement()).getByRole("combobox", { name: "Type" }) as HTMLSelectElement;
    expect(selectValues(addTypeSelect)).toEqual(["path_exists", "file_exists"]);
  });

  it("offers only the parameterless package condition after the step gains App Definition context", async () => {
    const user = userEvent.setup();
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "copy", type: "copy_files" })])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const verifySection = verifySectionElement();
    const addTypeSelect = within(verifySection).getByRole("combobox", { name: "Type" }) as HTMLSelectElement;
    expect(selectValues(addTypeSelect)).toEqual(["path_exists", "file_exists"]);

    await user.selectOptions(appDefinitionSelect(), "demo_app");
    expect(selectValues(addTypeSelect)).toEqual([
      "path_exists",
      "file_exists",
      "app_target_path_exists",
      "app_target_file_exists",
      "app_package_installed",
    ]);
    expect(within(verifySection).getByRole("option", { name: "Package installed (App Definition)" })).toBeTruthy();
  });

  it("edits an App Target path check through the step App Definition", async () => {
    const user = userEvent.setup();
    const onAdvancedCommand = vi.fn(async (_command: EditorCommand) => ({ ok: true, changed: true }));
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            verify: [{ type: "path_exists", params: { path: { app_target: "config_file" } } }],
          }),
        ])}
        onAdvancedCommand={onAdvancedCommand}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = within(verifySectionElement()).getByRole("combobox", { name: "App Target" }) as HTMLSelectElement;
    expect(select.value).toBe("config_file");

    await user.selectOptions(select, "system");
    expect(onAdvancedCommand).toHaveBeenCalledWith({
      type: "UpdateStepVerify",
      stepId: "copy",
      verify: [{ type: "path_exists", params: { path: { app_target: "system" } } }],
    });
  });

  it("marks directory App Targets incompatible for a file_exists check", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            verify: [{ type: "file_exists", params: { path: { app_target: "config_file" } } }],
          }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = within(verifySectionElement()).getByRole("combobox", { name: "App Target" }) as HTMLSelectElement;
    expect(select.value).toBe("config_file");
    expect(optionByValue(select, "config_file").disabled).toBe(false);
    const directoryTarget = optionByValue(select, "system");
    expect(directoryTarget.disabled).toBe(true);
    expect(directoryTarget.textContent ?? "").toContain("A file_exists check needs a file target.");
  });

  it("edits an App Target skip_if check through the step App Definition", async () => {
    const user = userEvent.setup();
    const onAdvancedCommand = vi.fn(async (_command: EditorCommand) => ({ ok: true, changed: true }));
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            skipIf: [{ type: "path_exists", params: { path: { app_target: "config_file" } } }],
          }),
        ])}
        onAdvancedCommand={onAdvancedCommand}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const select = within(skipIfSectionElement()).getByRole("combobox", { name: "App Target" }) as HTMLSelectElement;
    await user.selectOptions(select, "system");
    expect(onAdvancedCommand).toHaveBeenCalledWith({
      type: "UpdateStepSkipIf",
      stepId: "copy",
      skipIf: [{ type: "path_exists", params: { path: { app_target: "system" } } }],
    });
  });

  it("keeps an App Target check visible without step App Definition context", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "wait",
            type: "wait",
            verify: [{ type: "file_exists", params: { path: { app_target: "ghost_target" } } }],
          }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const verifySection = verifySectionElement();
    const select = within(verifySection).getByRole("combobox", { name: "App Target" }) as HTMLSelectElement;
    expect(select.value).toBe("ghost_target");
    expect(within(select).getByRole("option", { name: "ghost_target (not defined by this App Definition)" })).toBeTruthy();
    expect(
      within(verifySection).getByText("Select an App Definition on the step to choose one of its App Targets."),
    ).toBeTruthy();
  });

  it("switches an App Target check back to a literal path for the author", async () => {
    const user = userEvent.setup();
    const onAdvancedCommand = vi.fn(async (_command: EditorCommand) => ({ ok: true, changed: true }));
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            verify: [{ type: "file_exists", params: { path: { app_target: "config_file" } } }],
          }),
        ])}
        onAdvancedCommand={onAdvancedCommand}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    await user.click(within(verifySectionElement()).getByRole("button", { name: "Literal path" }));
    expect(onAdvancedCommand).toHaveBeenCalledWith({
      type: "UpdateStepVerify",
      stepId: "copy",
      verify: [{ type: "file_exists", params: { path: "" } }],
    });
  });

  it("switches a literal path check to an App Target for the author", async () => {
    const user = userEvent.setup();
    const onAdvancedCommand = vi.fn(async (_command: EditorCommand) => ({ ok: true, changed: true }));
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            verify: [{ type: "path_exists", params: { path: "/sdcard/RetroArch" } }],
          }),
        ])}
        onAdvancedCommand={onAdvancedCommand}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    await user.click(within(verifySectionElement()).getByRole("button", { name: "App Target" }));
    expect(onAdvancedCommand).toHaveBeenCalledWith({
      type: "UpdateStepVerify",
      stepId: "copy",
      verify: [{ type: "path_exists", params: { path: { app_target: "config_file" } } }],
    });
  });

  it("keeps unsupported App Target condition shapes as JSON checks", () => {
    render(
      <StepsHarness
        initialDocument={recipeDocument([
          buildStep({
            id: "copy",
            type: "copy_files",
            appRef: "demo_app",
            verify: [{ type: "path_exists", params: { path: { app_target: "system" }, future: true } }],
          }),
        ])}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const verifySection = verifySectionElement();
    expect(within(verifySection).queryByRole("combobox", { name: "App Target" })).toBeNull();
    expect(within(verifySection).getByText("JSON check")).toBeTruthy();
  });

  it("adds an App Target file_exists check from the add controls", async () => {
    const user = userEvent.setup();
    const onAdvancedCommand = vi.fn(async (_command: EditorCommand) => ({ ok: true, changed: true }));
    render(
      <StepsHarness
        initialDocument={recipeDocument([buildStep({ id: "copy", type: "copy_files", appRef: "demo_app" })])}
        onAdvancedCommand={onAdvancedCommand}
        onCommand={vi.fn(async (_command: EditorCommand) => true)}
      />,
    );

    const verifySection = verifySectionElement();
    await user.selectOptions(
      within(verifySection).getByRole("combobox", { name: "Type" }),
      "app_target_file_exists",
    );
    const addTarget = within(verifySection).getByRole("combobox", { name: "App Target" }) as HTMLSelectElement;
    expect(addTarget.value).toBe("config_file");

    await user.click(within(verifySection).getByRole("button", { name: "Add check" }));
    expect(onAdvancedCommand).toHaveBeenCalledWith({
      type: "UpdateStepVerify",
      stepId: "copy",
      verify: [{ type: "file_exists", params: { path: { app_target: "config_file" } } }],
    });
  });
});

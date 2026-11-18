import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { EditorCommand } from "../src/api/commands";
import type { InputDto, RecipeDocumentDto } from "../src/api/types";
import { InputsEditor } from "../src/components/InputsEditor";

const destinationInput: InputDto = {
  id: "destination",
  recipeId: "feature.copy_roms",
  inputId: "destination",
  key: "feature.copy_roms/destination",
  type: "setup_location_ref",
  role: "rom_destination",
  label: "Device ROM folder",
  description: "",
  required: true,
  multiple: false,
  validation: { mustExist: false, allowedExtensions: [], pathKind: null, allowedPrefixes: [] },
  default: null,
  options: [],
  sensitive: false,
  advanced: false,
  metadata: {},
};

const document: RecipeDocumentDto = {
  documentId: "recipe-session-1",
  path: "/catalog/recipes/feature.copy_roms.yaml",
  authoredRoot: "/catalog",
  dirty: false,
  canUndo: false,
  canRedo: false,
  recipe: {
    schemaVersion: 1,
    kind: "recipe",
    id: "feature.copy_roms",
    name: "Copy ROM library",
    description: "",
    recipeDependencies: [],
    provides: { features: ["rom_library"] },
    inputs: { destination: destinationInput },
    artifacts: {},
    artifactGroups: {},
    steps: [],
  },
  yaml: "kind: recipe\n",
  diagnostics: [],
  refIndex: { inputRefs: [], artifactRefs: [], stepRefs: [], stepOutputRefs: [], allRefs: [], candidates: [] },
  appCatalog: { status: "ready", apps: [], diagnostics: [] },
};

describe("InputsEditor runtime-only input types", () => {
  it("keeps Setup Location refs outside type authoring and changes only the edited field", async () => {
    const user = userEvent.setup();
    const onCommand = vi.fn(async (_command: EditorCommand) => true);

    render(
      <InputsEditor
        document={document}
        promptForId={async () => null}
        confirmAction={async () => true}
        onCommand={onCommand}
      />,
    );

    const typeSelect = screen.getByLabelText("Type") as HTMLSelectElement;
    expect(Array.from(typeSelect.options, (option) => option.value)).not.toContain("setup_location_ref");

    const label = screen.getByLabelText("Label");
    await user.clear(label);
    await user.type(label, "Selected ROM destination");
    await user.tab();

    await waitFor(() =>
      expect(onCommand).toHaveBeenCalledWith({
        type: "UpdateInputField",
        inputId: "destination",
        field: "label",
        value: "Selected ROM destination",
      }),
    );
    expect(onCommand.mock.calls.some(([command]) => command.type === "UpdateInputField" && command.field === "type")).toBe(
      false,
    );
  });
});

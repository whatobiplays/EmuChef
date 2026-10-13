import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { saveAppDefinition, saveAppDefinitionAs, saveRecipe, saveRecipeAs } from "../src/api/editorApi";

describe("document save API routes", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  it("keeps Recipe routes and dispatches App Definitions through their typed routes", async () => {
    await saveRecipe("recipe-1");
    await saveRecipeAs("recipe-1", "/tmp/recipe.yaml");
    await saveAppDefinition("app-1");
    await saveAppDefinitionAs("app-1", "/tmp/app.yaml");

    expect(mocks.invoke.mock.calls).toEqual([
      ["save_recipe", { documentId: "recipe-1" }],
      ["save_recipe_as", { documentId: "recipe-1", path: "/tmp/recipe.yaml" }],
      ["save_app_definition", { documentId: "app-1" }],
      ["save_app_definition_as", { documentId: "app-1", path: "/tmp/app.yaml" }],
    ]);
  });
});

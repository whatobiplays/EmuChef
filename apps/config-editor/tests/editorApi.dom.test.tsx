import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AppDefinitionV1Dto,
  AppGeneratorRequestContext,
  AppRecipeEditsDto,
  AppMappingEditsDto,
} from "../src/api/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import {
  analyzeAppGeneratorSource,
  appGeneratorPublicationStatus,
  beginAppGenerator,
  chooseAppGeneratorApk,
  chooseAppGeneratorAuthoredRoot,
  downloadAppGeneratorRemoteApk,
  generateAppRecipeDraft,
  generateRemoteAppRecipeDraft,
  inspectAppGeneratorApk,
  listAppGeneratorExistingApps,
  reviewAppGeneratorDraft,
  reviewRemoteAppGeneratorDraft,
  saveAppGeneratorReview,
  saveAppDefinition,
  saveAppDefinitionAs,
  saveRecipe,
  saveRecipeAs,
  selectAppGeneratorExistingApp,
  setAppGeneratorAuthoredRoot,
  previewCatalogChange,
  applyCatalogChange,
applyCatalogChangeWithStatusRecovery,
  catalogChangeStatus,
  listCatalogChangeRecoveries,
  retryCatalogChangeRecovery,
} from "../src/api/editorApi";

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

describe("catalog change API routes", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({ ok: true, result: {} });
  });

  it("forwards preview, apply, durable status, discovery, and retry arguments exactly", async () => {
    await previewCatalogChange("app-1", { kind: "renameApp", appId: "old", newId: "new" });
    await applyCatalogChange("operation-1", "token-1");
    await catalogChangeStatus("/catalog", "operation-1");
    await listCatalogChangeRecoveries("/catalog");
    await retryCatalogChangeRecovery("/catalog", "operation-1");

    expect(mocks.invoke.mock.calls).toEqual([
      ["preview_catalog_change", { documentId: "app-1", intent: { kind: "renameApp", appId: "old", newId: "new" } }],
      ["apply_catalog_change", { operationId: "operation-1", previewToken: "token-1" }],
      ["catalog_change_status", { authoredRoot: "/catalog", operationId: "operation-1" }],
      ["list_catalog_change_recoveries", { authoredRoot: "/catalog" }],
      ["retry_catalog_change_recovery", { authoredRoot: "/catalog", operationId: "operation-1" }],
    ]);
  });

  it("recovers a lost apply response from durable operation status", async () => {
    const completed = { operationId: "operation-1", state: "completed", changedFiles: [] } as const;
    mocks.invoke
      .mockRejectedValueOnce(new Error("sidecar response lost"))
      .mockResolvedValueOnce({ ok: true, result: { operation: completed } });

    const result = await applyCatalogChangeWithStatusRecovery("/catalog", "operation-1", "token-1");

    expect(result).toEqual({ kind: "success", result: completed });
    expect(mocks.invoke.mock.calls).toEqual([
      ["apply_catalog_change", { operationId: "operation-1", previewToken: "token-1" }],
      ["catalog_change_status", { authoredRoot: "/catalog", operationId: "operation-1" }],
    ]);
  });

  it("preserves the apply transport error when durable status has no operation", async () => {
    mocks.invoke
      .mockRejectedValueOnce(new Error("sidecar response lost"))
      .mockResolvedValueOnce({ ok: true, result: { operation: null } });

    const result = await applyCatalogChangeWithStatusRecovery("/catalog", "operation-1", "token-1");

    expect(result).toEqual({ kind: "transport-error", message: "sidecar response lost" });
  });
});

const appDto: AppDefinitionV1Dto = {
  schema_version: 1,
  kind: "app_definition",
  id: "example",
  name: "Example",
  package_id: "com.example.app",
};

const recipeEdits: AppRecipeEditsDto = {
  ids: null,
  name: "Install Example",
  description: "Install Example from a provided APK.",
  inputLabel: "Example APK",
  inputDescription: "Local Example APK to install.",
  replaceExisting: false,
  launchEnabled: false,
  launcherActivity: null,
};

const mappingEdits: AppMappingEditsDto = {
  artifacts: [
    {
      id: "bundle",
      kind: "apk",
      source: { strategy: "user_provided" },
    },
  ],
  targets: [],
  metadata: "{}",
};

describe("app generator API routes", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({ ok: true, result: {} });
  });

  it("routes session, source, and inspection commands with their exact payloads", async () => {
    await beginAppGenerator();
    await chooseAppGeneratorApk("session-1");
    await chooseAppGeneratorAuthoredRoot("session-1");
    await setAppGeneratorAuthoredRoot("session-1", "/catalog");
    await analyzeAppGeneratorSource(
      "session-1",
      "github_release",
      "https://github.com/example/project/releases/tag/v1",
      true,
    );
    await downloadAppGeneratorRemoteApk("session-1", "asset-1");
    await inspectAppGeneratorApk("session-1", "apk-1");
    await listAppGeneratorExistingApps("session-1", "root-1");
    await listAppGeneratorExistingApps("session-1");
    await selectAppGeneratorExistingApp("session-1", "apps/example.yaml");

    expect(mocks.invoke.mock.calls).toEqual([
      ["begin_app_generator", undefined],
      ["choose_app_generator_apk", { sessionHandle: "session-1" }],
      ["choose_app_generator_authored_root", { sessionHandle: "session-1" }],
      ["set_app_generator_authored_root", { sessionHandle: "session-1", authoredRoot: "/catalog" }],
      [
        "analyze_app_generator_source",
        {
          sessionHandle: "session-1",
          mode: "github_release",
          sourceUrl: "https://github.com/example/project/releases/tag/v1",
          includePrereleases: true,
        },
      ],
      ["download_app_generator_remote_apk", { sessionHandle: "session-1", assetHandle: "asset-1" }],
      ["inspect_app_generator_apk", { sessionHandle: "session-1", apkHandle: "apk-1" }],
      ["list_app_generator_existing_apps", { sessionHandle: "session-1", rootHandle: "root-1" }],
      ["list_app_generator_existing_apps", { sessionHandle: "session-1", rootHandle: null }],
      [
        "select_app_generator_existing_app",
        { sessionHandle: "session-1", appPath: "apps/example.yaml" },
      ],
    ]);
  });

  it("routes review, save, and status commands with the author-controlled identity fields", async () => {
    const minimalContext: AppGeneratorRequestContext = {
      requestId: 5,
      existingAppHandle: null,
      appArtifactId: "bundle",
      recipeId: "app.example.bundle.install",
      artifactName: null,
    };
    await reviewAppGeneratorDraft({
      sessionHandle: "session-1",
      apkHandle: "apk-1",
      rootHandle: "root-1",
      context: minimalContext,
      app: null,
      recipe: null,
      mappings: null,
      permissionSelection: null,
      regenerateIdentifiers: false,
    });

    const fullContext: AppGeneratorRequestContext = {
      requestId: 6,
      existingAppHandle: "app-handle-1",
      appArtifactId: "bundle",
      recipeId: "app.example.bundle.install",
      artifactName: "Bundle APK",
      permissionAutomationEnabled: true,
    };
    await reviewAppGeneratorDraft({
      sessionHandle: "session-1",
      apkHandle: "apk-1",
      rootHandle: "root-1",
      context: fullContext,
      app: appDto,
      recipe: recipeEdits,
      mappings: mappingEdits,
      permissionSelection: {
        inspectionHandle: "apk-inspection-1",
        runtimePermissions: [{ permissionName: "android.permission.CAMERA" }],
        appOps: [],
      },
      regenerateIdentifiers: true,
    });

    await reviewAppGeneratorDraft({
      sessionHandle: "session-1",
      apkHandle: "apk-1",
      rootHandle: "root-1",
      context: fullContext,
      app: null,
      recipe: recipeEdits,
      mappings: null,
      permissionSelection: null,
      regenerateIdentifiers: false,
    });

    await reviewRemoteAppGeneratorDraft({
      sessionHandle: "session-1",
      apkHandle: "apk-1",
      rootHandle: "root-1",
      context: minimalContext,
      app: null,
      recipe: null,
      mappings: null,
      permissionSelection: null,
      regenerateIdentifiers: false,
      assetHandle: "asset-1",
      strategy: "pinned_remote_asset",
      assetPattern: "^app-.*\\.apk$",
      includePrereleases: true,
      trustedSha256: "ABCD",
    });

    await saveAppGeneratorReview("session-1", "review-handle-1", 12);
    await appGeneratorPublicationStatus("operation-1");

    expect(mocks.invoke.mock.calls).toEqual([
      [
        "review_app_generator_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          rootHandle: "root-1",
          permissionSelection: null,
          regenerateIdentifiers: false,
          requestId: 5,
          appArtifactId: "bundle",
          recipeId: "app.example.bundle.install",
          permissionAutomationEnabled: false,
        },
      ],
      [
        "review_app_generator_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          rootHandle: "root-1",
          permissionSelection: {
            inspectionHandle: "apk-inspection-1",
            runtimePermissions: [{ permissionName: "android.permission.CAMERA" }],
            appOps: [],
          },
          regenerateIdentifiers: true,
          app: appDto,
          recipe: recipeEdits,
          mappings: mappingEdits,
          requestId: 6,
          existingAppHandle: "app-handle-1",
          appArtifactId: "bundle",
          recipeId: "app.example.bundle.install",
          artifactName: "Bundle APK",
          permissionAutomationEnabled: true,
        },
      ],
      [
        "review_app_generator_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          rootHandle: "root-1",
          permissionSelection: null,
          regenerateIdentifiers: false,
          recipe: recipeEdits,
          requestId: 6,
          existingAppHandle: "app-handle-1",
          appArtifactId: "bundle",
          recipeId: "app.example.bundle.install",
          artifactName: "Bundle APK",
          permissionAutomationEnabled: true,
        },
      ],
      [
        "review_remote_app_generator_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          rootHandle: "root-1",
          assetHandle: "asset-1",
          strategy: "pinned_remote_asset",
          assetPattern: "^app-.*\\.apk$",
          includePrereleases: true,
          trustedSha256: "ABCD",
          permissionSelection: null,
          regenerateIdentifiers: false,
          requestId: 5,
          appArtifactId: "bundle",
          recipeId: "app.example.bundle.install",
          permissionAutomationEnabled: false,
        },
      ],
      ["save_app_generator_review", { sessionHandle: "session-1", reviewHandle: "review-handle-1", requestId: 12 }],
      ["app_generator_publication_status", { operationId: "operation-1" }],
    ]);
  });

  it("forwards permission automation intent for local and remote drafts and defaults absent to disabled", async () => {
    await generateAppRecipeDraft(
      "session-1",
      "apk-1",
      null,
      null,
      null,
      null,
      false,
      null,
      { requestId: 7, permissionAutomationEnabled: true },
    );
    await generateAppRecipeDraft(
      "session-1",
      "apk-1",
      null,
      null,
      null,
      null,
      false,
      null,
      { requestId: 8, permissionAutomationEnabled: false },
    );
    await generateRemoteAppRecipeDraft(
      "session-1",
      "apk-1",
      "asset-1",
      "pinned_remote_asset",
      null,
      false,
      null,
      null,
      null,
      null,
      null,
      false,
      null,
      { requestId: 9 },
    );

    expect(mocks.invoke.mock.calls).toEqual([
      [
        "generate_app_recipe_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          permissionSelection: null,
          regenerateIdentifiers: false,
          rootHandle: null,
          requestId: 7,
          permissionAutomationEnabled: true,
        },
      ],
      [
        "generate_app_recipe_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          permissionSelection: null,
          regenerateIdentifiers: false,
          rootHandle: null,
          requestId: 8,
          permissionAutomationEnabled: false,
        },
      ],
      [
        "generate_remote_app_recipe_draft",
        {
          sessionHandle: "session-1",
          apkHandle: "apk-1",
          assetHandle: "asset-1",
          strategy: "pinned_remote_asset",
          assetPattern: null,
          includePrereleases: false,
          trustedSha256: null,
          permissionSelection: null,
          regenerateIdentifiers: false,
          rootHandle: null,
          requestId: 9,
          permissionAutomationEnabled: false,
        },
      ],
    ]);
  });
});

describe("app generator API result envelopes", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  it("returns success results unchanged", async () => {
    mocks.invoke.mockResolvedValue({ ok: true, result: { sessionHandle: "session-1" } });

    expect(await beginAppGenerator()).toEqual({
      kind: "success",
      result: { sessionHandle: "session-1" },
    });
  });

  it("maps structured envelope errors to api-error results", async () => {
    mocks.invoke.mockResolvedValue({
      ok: false,
      error: { code: "review_required", message: "A review handle is required.", details: {} },
    });

    expect(await saveAppGeneratorReview("session-1", "review-handle-1", 12)).toEqual({
      kind: "api-error",
      error: { code: "review_required", message: "A review handle is required.", details: {} },
      debug: undefined,
    });
  });

  it("reports an invalid envelope as a transport error for the command", async () => {
    mocks.invoke.mockResolvedValue({ ok: true });

    expect(await beginAppGenerator()).toEqual({
      kind: "transport-error",
      message: "Command begin_app_generator returned an invalid API envelope.",
    });
  });

  it("reports a rejected invoke call as a transport error", async () => {
    mocks.invoke.mockRejectedValue(new Error("sidecar offline"));

    expect(await appGeneratorPublicationStatus("operation-1")).toEqual({
      kind: "transport-error",
      message: "sidecar offline",
    });
  });

  it("keeps an unknown publication outcome as a successful result", async () => {
    const result = {
      outcome: "unknown",
      operationId: "operation-unknown",
      message: "The sidecar response was lost after submission.",
    };
    mocks.invoke.mockResolvedValue({ ok: true, result });

    expect(await saveAppGeneratorReview("session-1", "review-handle-1", 12)).toEqual({
      kind: "success",
      result,
    });
  });

  it("keeps a committed save whose Recipe open failed, including the structured open error", async () => {
    const result = {
      outcome: "saved",
      operationId: "operation-saved",
      app: { appPath: "apps/example.yaml" },
      recipe: {
        recipePath: "recipes/app.example.bundle.install.yaml",
        documentId: null,
        openError: { code: "document_open_failed", message: "The editor could not read the file." },
      },
      createdFiles: ["apps/example.yaml", "recipes/app.example.bundle.install.yaml"],
      replacedFiles: [],
    };
    mocks.invoke.mockResolvedValue({ ok: true, result });

    expect(await saveAppGeneratorReview("session-1", "review-handle-1", 12)).toEqual({
      kind: "success",
      result,
    });
  });
});

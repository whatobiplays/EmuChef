import assert from "node:assert/strict";
import test from "node:test";

import type {
  ApkInspectionResult,
  AppGeneratorPublicationOutcomeDto,
  AppGeneratorPublicationStatusResult,
  AppGeneratorReviewResult,
  AppRecipeDraftResult,
  ExistingAppSelectionResult,
  ExistingAppSummaryDto,
  RemoteSourceAnalysisResult,
} from "../src/api/types.js";
import type {
  AppArtifactRowEdits,
  AppGeneratorAction,
  AppGeneratorState,
} from "../src/components/appGenerator.logic.js";
import {
  analysisForPrereleasePolicy,
  artifactEditForRow,
  artifactRowForDefinition,
  artifactSourceForRow,
  assetPatternError,
  buildReleasePatternPreview,
  diagnosticDisplayTitle,
  draftToForm,
  effectiveAppId,
  effectiveArtifactId,
  effectiveRecipeId,
  eligibleApkAssets,
  emptyArtifactRow,
  emptyTargetRow,
  formToRequest,
  generatorRequestContext,
  globalInspectionWarnings,
  initialAppGeneratorState,
  inspectionPartition,
  mappingEditsFromRows,
  matchingAssetNames,
  mirroredTrustedSha256,
  otherRequestedPermissions,
  parseTrustedSha256,
  permissionApplicabilityLabel,
  permissionApplicabilityReasonLabel,
  permissionSelectionForInspection,
  permissionWarnings,
  proposedRecipeId,
  publicationFailureRecovery,
  publicationUnresolved,
  readableNameFromPackage,
  reduceAppGenerator,
  releaseAnalysisDescribesReviewedPolicy,
  selectedPackageMismatch,
  suggestAssetPattern,
  targetEditForRow,
  targetRowForDefinition,
  visibleDraftDiagnostics,
} from "../src/components/appGenerator.logic.js";

function remoteAsset(
  assetHandle: string,
  fileName: string,
  releaseTag: string,
  prerelease: boolean,
) {
  return {
    assetHandle,
    fileName,
    size: 10,
    contentType: "application/vnd.android.package-archive",
    releaseTag,
    releaseName: null,
    publishedAt: null,
    prerelease,
  };
}

function remoteAnalysis(): RemoteSourceAnalysisResult {
  const current = remoteAsset("current", "app-v3-arm64.apk", "v3", false);
  const olderOther = remoteAsset("older-other", "other.apk", "v2", false);
  const olderOne = remoteAsset("older-one", "app-v1-arm64.apk", "v1", false);
  const olderTwo = remoteAsset("older-two", "app-v2-arm64.apk", "v1", false);
  const pre = remoteAsset("prerelease", "app-v4-arm64.apk", "v4-beta", true);
  return {
    sourceHandle: "source",
    mode: "github_repository",
    normalizedUrl: "https://github.com/example/project",
    capabilities: {
      pinnedArtifact: true,
      latestRelease: true,
      prereleaseFiltering: true,
      deterministicAssetFiltering: true,
    },
    repository: {
      fullName: "example/project",
      name: "project",
      description: null,
      htmlUrl: "https://github.com/example/project",
    },
    releases: [
      { tag: "v4-beta", name: null, prerelease: true, publishedAt: null, assets: [pre] },
      { tag: "v3", name: null, prerelease: false, publishedAt: null, assets: [current] },
      { tag: "v2", name: null, prerelease: false, publishedAt: null, assets: [olderOther] },
      { tag: "v1", name: null, prerelease: false, publishedAt: null, assets: [olderTwo, olderOne] },
    ],
    assets: [pre, current, olderOther, olderTwo, olderOne],
    preselectedAssetHandle: null,
  };
}

function applicable() {
  return {
    status: "applicable" as const,
    reason: null,
    maximumSdkVersion: null,
    introductionApi: null,
    minimumDeviceApi: null,
    minimumTargetSdk: null,
    maximumTargetSdk: null,
    actualTargetSdk: null,
    targetSdkState: null,
  };
}

function inspection(): ApkInspectionResult {
  return {
    inspectionHandle: "apk-inspection-1",
    manifest: {
      packageName: "com.example.app",
      versionCode: "42",
      versionName: "1.2",
      minSdkVersion: "23",
      targetSdkVersion: "35",
    },
    permissions: [
      {
        name: "android.permission.CAMERA",
        declarationKind: "uses_permission",
        maxSdkVersion: null,
        classification: "runtime_grantable",
        applicability: applicable(),
      },
      {
        name: "android.permission.MANAGE_EXTERNAL_STORAGE",
        declarationKind: "uses_permission",
        maxSdkVersion: null,
        classification: "app_op_grantable",
        applicability: applicable(),
      },
      {
        name: "android.permission.UNKNOWN",
        declarationKind: "uses_permission",
        maxSdkVersion: "28",
        classification: "unknown",
        applicability: applicable(),
      },
      {
        name: "android.permission.WRITE_EXTERNAL_STORAGE",
        declarationKind: "uses_permission",
        maxSdkVersion: null,
        classification: "runtime_grantable",
        applicability: {
          ...applicable(),
          status: "not_applicable",
          reason: "target_sdk_above_maximum",
          maximumTargetSdk: 29,
          actualTargetSdk: 35,
        },
      },
      {
        name: "android.permission.MAYBE",
        declarationKind: "uses_permission_sdk_23",
        maxSdkVersion: "preview",
        classification: "unknown",
        applicability: {
          ...applicable(),
          status: "indeterminate",
          reason: "invalid_max_sdk_version",
        },
      },
    ],
    runtimeGrantCandidates: [
      {
        permissionName: "android.permission.CAMERA",
        requiresRoot: false,
        androidApiMin: 23,
        androidApiMax: null,
        selected: false,
      },
    ],
    appOpCandidates: [
      {
        permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE",
        operationName: "MANAGE_EXTERNAL_STORAGE",
        mode: "allow",
        requiresRoot: true,
        androidApiMin: 30,
        androidApiMax: null,
        selected: false,
      },
    ],
    warnings: [
      {
        code: "apk_permission_unknown",
        message: "Review this permission.",
        permissionName: "android.permission.UNKNOWN",
        applicabilityReason: null,
      },
      {
        code: "apk_permission_signature_or_privileged",
        message: "This inspection is limited to one declared permission set.",
        permissionName: null,
        applicabilityReason: null,
      },
    ],
    calculatedSha256: "ABCD",
    checksumStatus: "not_compared",
    signatureVerification: "not_performed",
  };
}

function draft(): AppRecipeDraftResult {
  return {
    app: {
      schema_version: 1,
      kind: "app_definition",
      id: "example",
      name: "Example",
      category: "utility",
      package_id: "com.example.app",
      artifacts: {
        apk: {
          kind: "apk",
          name: "Example APK",
          source: { strategy: "user_provided" },
        },
      },
      targets: {
        data: { kind: "directory", location: "app_data", path: "com.example.app" },
      },
      metadata: { homepage: "https://example.com" },
    },
    apkInspection: { inspectionHandle: "apk-inspection-1" },
    recipe: {
      schemaVersion: 1,
      kind: "recipe",
      id: "app.example.install",
      name: "Install Example",
      description: "Install Example from a provided APK.",
      recipeDependencies: [],
      provides: { features: ["example_install"] },
      inputs: {
        example_apk: {
          id: "example_apk",
          recipeId: "app.example.install",
          inputId: "example_apk",
          key: "example_apk",
          type: "file",
          role: "artifact",
          label: "Example APK",
          description: "Local Example APK to install.",
          required: true,
          multiple: false,
          validation: {
            mustExist: true,
            allowedExtensions: [".apk"],
            pathKind: "file",
            allowedPrefixes: [],
          },
          default: null,
          options: [],
          sensitive: false,
          advanced: false,
          metadata: {},
        },
      },
      artifacts: {
        apk: { id: "apk", type: "apk", appRef: "example", artifact: "apk", input: "example_apk" },
      },
      artifactGroups: {},
      steps: [
        {
          id: "install_example",
          type: "install_app_apk",
          name: "Install Example",
          appRef: "example",
          description: "",
          userToggleable: false,
          dependencies: [],
          constraints: { capabilities: [], conflictsWith: [] },
          skipIf: [],
          params: {},
          verify: [],
        },
        {
          id: "grant_permissions_example",
          type: "grant_app_permissions",
          name: "Grant Example permissions",
          appRef: "example",
          description: "",
          userToggleable: false,
          dependencies: ["install_example"],
          constraints: { capabilities: [], conflictsWith: [] },
          skipIf: [],
          params: {},
          verify: [],
        },
        {
          id: "launch_example",
          type: "launch_app",
          name: "Launch Example",
          appRef: "example",
          description: "",
          userToggleable: true,
          dependencies: ["grant_permissions_example"],
          constraints: { capabilities: [], conflictsWith: [] },
          skipIf: [],
          params: {},
          verify: [],
        },
      ],
    },
    recipeEdits: {
      ids: {
        recipeId: "app.example.install",
        inputId: "example_apk",
        featureId: "example_install",
        installStepId: "install_example",
        permissionStepId: "grant_permissions_example",
        launchStepId: "launch_example",
      },
      name: "Install Example",
      description: "Install Example from a provided APK.",
      inputLabel: "Example APK",
      inputDescription: "Local Example APK to install.",
      replaceExisting: false,
      launchEnabled: true,
      launcherActivity: "com.example.app/.MainActivity",
    },
    appCanonicalYaml: "app: example",
    recipeCanonicalYaml: "recipe: app.example.install",
    appDestination: { fileName: "example.yaml", relativePath: "apps/example.yaml" },
    recipeDestination: {
      fileName: "app.example.install.yaml",
      relativePath: "recipes/app.example.install.yaml",
    },
    evidence: [],
    diagnostics: [],
    blocking: false,
    collisions: null,
  };
}

function existingSummary(
  overrides: Partial<ExistingAppSummaryDto> = {},
): ExistingAppSummaryDto {
  return {
    appPath: "apps/example.yaml",
    appId: "example",
    name: "Example",
    packageId: "com.example.app",
    artifactIds: ["apk"],
    permissionSetSummary: { baseline: 1, elevated: 0 },
    hasLauncher: true,
    selectable: true,
    diagnostics: [],
    ...overrides,
  };
}

function existingSelection(): ExistingAppSelectionResult {
  return {
    appHandle: "app-handle-1",
    appPath: "apps/example.yaml",
    appId: "example",
    name: "Example",
    packageId: "com.example.app",
    artifactIds: ["apk"],
    permissionSetSummary: { baseline: 1, elevated: 0 },
    launcherActivity: "com.example.app/.MainActivity",
  };
}

function review(): AppGeneratorReviewResult {
  return {
    reviewHandle: "review-handle-1",
    requestId: 7,
    blocking: false,
    diagnostics: [],
    app: draft().app,
    appCanonicalYaml: "app: example",
    recipe: draft().recipe,
    recipeCanonicalYaml: "recipe: app.example.install",
    destinations: {
      app: { fileName: "example.yaml", relativePath: "apps/example.yaml" },
      recipe: {
        fileName: "app.example.install.yaml",
        relativePath: "recipes/app.example.install.yaml",
      },
    },
    target: {
      mode: "new_app",
      appId: "example",
      appPath: null,
      packageId: "com.example.app",
      packageLocked: false,
    },
    artifact: {
      appArtifactId: "apk",
      name: "Example APK",
      kind: "apk",
      source: { strategy: "user_provided" },
      change: "proposed",
    },
    permissionAutomation: {
      enabled: false,
      baseline: { actions: [] },
      elevated: { actions: [], rootRequired: true, optional: true },
    },
    changeImpact: {
      addedArtifactIds: ["apk"],
      preservedArtifactIds: [],
      addedPermissionActions: { baseline: [], elevated: [] },
      affectedRecipes: [],
      warnings: [],
    },
    apkInspection: null,
  };
}

function savedOutcome(): AppGeneratorPublicationOutcomeDto {
  return {
    outcome: "saved",
    operationId: "operation-saved",
    app: { appPath: "apps/example.yaml" },
    recipe: {
      recipePath: "recipes/app.example.install.yaml",
      documentId: "document-1",
    },
    createdFiles: ["apps/example.yaml", "recipes/app.example.install.yaml"],
    replacedFiles: [],
  };
}

function failedOutcome(): AppGeneratorPublicationOutcomeDto {
  return {
    outcome: "failed",
    operationId: "operation-failed",
    error: {
      code: "recipe_destination_exists",
      message: "The Recipe destination already exists.",
      details: {},
    },
    rollback: { recipeRemoved: true, recipeRecoveryRequired: false },
  };
}

function unknownOutcome(): AppGeneratorPublicationOutcomeDto {
  return {
    outcome: "unknown",
    operationId: "operation-unknown",
    message: "The sidecar response was lost after submission.",
  };
}

function recordedStatus(
  outcome: "saved" | "failed" | "unknown",
  detail: Record<string, unknown> | null = null,
): AppGeneratorPublicationStatusResult {
  return {
    operationId: "operation-unknown",
    outcome,
    confirmed: outcome !== "unknown",
    recordedAt: "2026-10-15T00:00:00Z",
    detail,
  };
}

function stateWith(overrides: Partial<AppGeneratorState>): AppGeneratorState {
  return { ...initialAppGeneratorState, ...overrides };
}

function reduce(
  state: AppGeneratorState,
  actions: AppGeneratorAction[],
): AppGeneratorState {
  return actions.reduce(
    (current, action) => reduceAppGenerator(current, action),
    state,
  );
}


function descriptorFor(releaseTag: string) {
  return {
    mode: "github_repository" as const,
    strategy: "pinned_remote_asset" as const,
    downloadUrl:
      "https://github.com/example/project/releases/download/" + releaseTag + "/app.apk",
    provider: "github" as const,
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag,
    assetName: "app.apk",
    assetPattern: null,
    includePrereleases: false,
  };
}

function apkRow(
  id: string,
  overrides: Partial<AppArtifactRowEdits> = {},
): AppArtifactRowEdits {
  return { ...emptyArtifactRow(id), ...overrides };
}

function formWithRows(rows: AppArtifactRowEdits[]) {
  const form = draftToForm(draft());
  form.mappings.artifacts = rows;
  return form;
}

function formForApp(appId: string) {
  const form = draftToForm(draft());
  form.app.id = appId;
  return form;
}

test("package fallback creates a readable App name and matching Recipe text", () => {
  assert.equal(readableNameFromPackage("dev.eden.eden_emulator"), "Eden Emulator");
  const packageDraft = draft();
  packageDraft.app.name = "dev.eden.eden_emulator";
  packageDraft.app.package_id = "dev.eden.eden_emulator";
  packageDraft.recipeEdits.name = "Install dev.eden.eden_emulator";
  packageDraft.recipeEdits.description = "Install a user-provided dev.eden.eden_emulator APK.";
  packageDraft.recipeEdits.inputLabel = "dev.eden.eden_emulator APK";
  packageDraft.recipeEdits.inputDescription = "Local dev.eden.eden_emulator APK to install.";
  const form = draftToForm(packageDraft);
  assert.equal(form.app.name, "Eden Emulator");
  assert.equal(form.recipe.name, "Install Eden Emulator");
  assert.equal(form.recipe.inputLabel, "Eden Emulator APK");
  assert.equal(form.recipe.inputDescription, "Local Eden Emulator APK to install.");
  assert.equal(form.recipe.description, "Install a user-provided Eden Emulator APK.");
});

test("explicit App Artifact and Recipe identities win over proposals", () => {
  const explicit = stateWith({
    appArtifactId: "  main-apk  ",
    recipeId: " custom.recipe.install ",
  });
  assert.equal(effectiveArtifactId(explicit), "main-apk");
  assert.equal(effectiveRecipeId(explicit), "custom.recipe.install");
  assert.equal(effectiveArtifactId(stateWith({})), "apk");
});

test("proposed Recipe id composes the App and App Artifact identities", () => {
  assert.equal(proposedRecipeId("example", "main"), "app.example.main.install");
  assert.equal(
    effectiveRecipeId(stateWith({ form: formForApp("example") })),
    "app.example.apk.install",
  );
  assert.equal(effectiveRecipeId(stateWith({})), "");
});

test("effective App id follows the locked selection then the previewed form", () => {
  assert.equal(effectiveAppId(stateWith({})), "");
  assert.equal(effectiveAppId(stateWith({ draft: draft() })), "example");
  assert.equal(
    effectiveAppId(stateWith({ draft: draft(), form: formForApp("form-app") })),
    "form-app",
  );
  assert.equal(
    effectiveAppId(stateWith({ existingApp: existingSelection(), form: formForApp("form-app") })),
    "example",
  );
});

test("generator request context carries explicit identities and omits unresolved ones", () => {
  assert.deepEqual(
    generatorRequestContext(stateWith({ artifactName: "  Main APK  " }), 12),
    {
      requestId: 12,
      existingAppHandle: null,
      appArtifactId: "apk",
      recipeId: null,
      artifactName: "Main APK",
      permissionAutomationEnabled: false,
    },
  );
  assert.deepEqual(
    generatorRequestContext(
      stateWith({
        targetMode: "existing_app",
        existingApp: existingSelection(),
        appArtifactId: "apk2",
        recipeId: "app.example.apk2.install",
      }),
      13,
    ),
    {
      requestId: 13,
      existingAppHandle: "app-handle-1",
      appArtifactId: "apk2",
      recipeId: "app.example.apk2.install",
      artifactName: null,
      permissionAutomationEnabled: false,
    },
  );
});

test("generator request context carries permission automation intent from the opt-in", () => {
  assert.equal(generatorRequestContext(stateWith({}), 14).permissionAutomationEnabled, false);
  assert.equal(
    generatorRequestContext(stateWith({ automationOptIn: true }), 15).permissionAutomationEnabled,
    true,
  );
});

test("identity edits invalidate the review and clear pending response slots", () => {
  let state = stateWith({
    review: review(),
    pendingPreviewRequestId: 3,
    pendingReviewRequestId: 4,
  });
  state = reduceAppGenerator(state, { type: "artifact-id", value: "main" });
  assert.equal(state.appArtifactId, "main");
  assert.equal(state.review, null);
  assert.equal(state.pendingPreviewRequestId, null);
  assert.equal(state.pendingReviewRequestId, null);
  state = reduceAppGenerator(state, { type: "recipe-id", value: "app.example.main.install" });
  assert.equal(state.recipeId, "app.example.main.install");
  state = reduceAppGenerator(state, { type: "artifact-name", value: "Main APK" });
  assert.equal(state.artifactName, "Main APK");
});

test("identity reset restores the proposed defaults without touching other fields", () => {
  const state = reduceAppGenerator(
    stateWith({
      appArtifactId: "main",
      recipeId: "app.example.main.install",
      artifactName: "Main",
      inspection: inspection(),
      review: review(),
    }),
    { type: "identity-reset" },
  );
  assert.equal(state.appArtifactId, null);
  assert.equal(state.recipeId, null);
  assert.equal(state.artifactName, "");
  assert.equal(state.review, null);
  assert.equal(state.inspection?.inspectionHandle, "apk-inspection-1");
  assert.equal(effectiveArtifactId(state), "apk");
  assert.equal(effectiveRecipeId(state), "");
});

test("target mode switch clears target-bound draft state and invalidates the review", () => {
  const state = reduceAppGenerator(
    stateWith({
      phase: "reviewing",
      targetMode: "new_app",
      draft: draft(),
      form: draftToForm(draft()),
      collisions: { collisions: [], blocking: false },
      review: review(),
      inspection: inspection(),
      apkHandle: "apk-handle-1",
    }),
    { type: "target-mode", mode: "existing_app" },
  );
  assert.equal(state.targetMode, "existing_app");
  assert.equal(state.phase, "editing");
  assert.equal(state.draft, null);
  assert.equal(state.form, null);
  assert.equal(state.collisions, null);
  assert.equal(state.review, null);
  assert.equal(state.existingApp, null);
  assert.equal(state.inspection?.inspectionHandle, "apk-inspection-1");
  assert.equal(state.apkHandle, "apk-handle-1");
});

test("existing App listing records inventory and ignores a stale response", () => {
  const requested = reduceAppGenerator(stateWith({}), {
    type: "existing-apps-request",
    requestId: 2,
  });
  assert.equal(requested.existingAppsPending, true);
  assert.equal(requested.pendingExistingAppsRequestId, 2);
  assert.equal(requested.lastIssuedRequestId, 2);

  const stale = reduceAppGenerator(requested, {
    type: "existing-apps-response",
    requestId: 1,
    result: {
      apps: [existingSummary()],
      inventory: { state: "complete", diagnostics: [] },
    },
  });
  assert.equal(stale, requested);

  const applied = reduceAppGenerator(requested, {
    type: "existing-apps-response",
    requestId: 2,
    result: {
      apps: [existingSummary()],
      inventory: {
        state: "incomplete",
        diagnostics: [{ code: "app_catalog_unreadable", message: "One App file could not be read." }],
      },
    },
  });
  assert.equal(applied.existingAppsPending, false);
  assert.equal(applied.pendingExistingAppsRequestId, null);
  assert.equal(applied.existingApps?.length, 1);
  assert.equal(applied.existingAppsInventory?.state, "incomplete");
});

test("existing App selection locks the target and clears target-bound draft state", () => {
  const requested = reduceAppGenerator(
    stateWith({ targetMode: "existing_app", phase: "editing" }),
    { type: "existing-app-select-request", requestId: 5 },
  );
  assert.equal(requested.existingAppSelectPending, true);
  assert.equal(requested.pendingExistingAppSelectRequestId, 5);

  const applied = reduceAppGenerator(requested, {
    type: "existing-app-select-response",
    requestId: 5,
    result: existingSelection(),
  });
  assert.equal(applied.existingApp?.appHandle, "app-handle-1");
  assert.equal(applied.existingAppSelectPending, false);
  assert.equal(applied.draft, null);
  assert.equal(applied.form, null);
  assert.equal(applied.collisions, null);
  assert.equal(applied.review, null);

  const ignored = reduceAppGenerator(applied, {
    type: "existing-app-select-response",
    requestId: 4,
    result: existingSelection(),
  });
  assert.equal(ignored, applied);
});

test("an inspected package mismatch reports both identities for an actionable error", () => {
  assert.equal(selectedPackageMismatch(stateWith({})), null);
  assert.equal(
    selectedPackageMismatch(
      stateWith({ targetMode: "existing_app", existingApp: existingSelection() }),
    ),
    null,
  );
  assert.deepEqual(
    selectedPackageMismatch(
      stateWith({
        targetMode: "existing_app",
        existingApp: { ...existingSelection(), packageId: "com.other.app" },
        inspection: inspection(),
      }),
    ),
    { inspected: "com.example.app", selected: "com.other.app" },
  );
  assert.equal(
    selectedPackageMismatch(
      stateWith({
        targetMode: "existing_app",
        existingApp: existingSelection(),
        inspection: inspection(),
      }),
    ),
    null,
  );
});

test("local APK selection clears downloaded and inspected evidence", () => {
  const state = reduceAppGenerator(
    stateWith({
      sourceMode: "local_apk",
      inspection: inspection(),
      draft: draft(),
      form: draftToForm(draft()),
      remoteSource: descriptorFor("v3"),
      trustedSha256: "AB",
      review: review(),
    }),
    { type: "apk-selected", apkHandle: "apk-handle-2", label: "example.apk" },
  );
  assert.equal(state.apkHandle, "apk-handle-2");
  assert.equal(state.apkLabel, "example.apk");
  assert.equal(state.inspection, null);
  assert.equal(state.draft, null);
  assert.equal(state.form, null);
  assert.equal(state.trustedSha256, "");
  assert.equal(state.review, null);
});

test("source mode and URL edits reset retained analysis and downloaded evidence", () => {
  const withAnalysis = stateWith({
    sourceMode: "github_repository",
    sourceUrl: "https://github.com/example/project",
    sourceAnalysis: remoteAnalysis(),
    selectedAssetHandle: "current",
    trustedSha256: "AB",
    apkHandle: "apk-handle-1",
    apkLabel: "app.apk",
    inspection: inspection(),
    draft: draft(),
    remoteSource: descriptorFor("v3"),
    review: review(),
  });
  const switched = reduceAppGenerator(withAnalysis, { type: "source-mode", mode: "direct_apk" });
  assert.equal(switched.sourceMode, "direct_apk");
  assert.equal(switched.phase, "selecting");
  assert.equal(switched.sourceUrl, "");
  assert.equal(switched.sourceAnalysis, null);
  assert.equal(switched.selectedAssetHandle, null);
  assert.equal(switched.trustedSha256, "");
  assert.equal(switched.apkHandle, null);
  assert.equal(switched.inspection, null);
  assert.equal(switched.draft, null);
  assert.equal(switched.form, null);
  assert.equal(switched.review, null);
  assert.equal(switched.installStrategy, withAnalysis.installStrategy);

  const retyped = reduceAppGenerator(switched, {
    type: "source-url",
    value: "https://example.com/app.apk",
  });
  assert.equal(retyped.sourceUrl, "https://example.com/app.apk");
  assert.equal(retyped.sourceAnalysis, null);
  assert.equal(retyped.review, null);
});

test("source analysis stores the preselection and clears downloaded evidence", () => {
  const analysis = remoteAnalysis();
  analysis.preselectedAssetHandle = "current";
  const state = reduceAppGenerator(
    stateWith({
      sourceMode: "github_repository",
      sourceUrl: "https://github.com/example/project",
      apkHandle: "apk-handle-1",
      inspection: inspection(),
      draft: draft(),
      review: review(),
    }),
    { type: "source-analyzed", analysis },
  );
  assert.equal(state.sourceAnalysis, analysis);
  assert.equal(state.phase, "selecting");
  assert.equal(state.selectedAssetHandle, "current");
  assert.equal(state.apkHandle, null);
  assert.equal(state.inspection, null);
  assert.equal(state.draft, null);
  assert.equal(state.form, null);
  assert.equal(state.review, null);
  assert.equal(state.assetPattern, suggestAssetPattern("app-v3-arm64.apk", ["app-v3-arm64.apk"]));
});

test("asset selection seeds a version-tolerant filename pattern", () => {
  const state = reduceAppGenerator(
    stateWith({
      sourceMode: "github_repository",
      sourceAnalysis: remoteAnalysis(),
      selectedAssetHandle: "current",
      apkHandle: "apk-handle-1",
      inspection: inspection(),
    }),
    { type: "asset-selected", assetHandle: "older-one" },
  );
  assert.equal(state.selectedAssetHandle, "older-one");
  assert.equal(state.apkHandle, null);
  assert.equal(state.inspection, null);
  assert.equal(state.review, null);
  assert.equal(state.assetPattern, "^app-v1-arm64\\.apk$");
});

test("remote download records the chosen strategy and latest-release policy", () => {
  const source = descriptorFor("v3");
  const latest = reduceAppGenerator(
    stateWith({
      installStrategy: "latest_compatible_release",
      assetPattern: "^app",
      includePrereleases: true,
    }),
    {
      type: "remote-downloaded",
      apkHandle: "apk-handle-3",
      label: "app-v3-arm64.apk",
      source,
    },
  );
  assert.equal(latest.apkHandle, "apk-handle-3");
  assert.equal(latest.remoteSource?.strategy, "latest_compatible_release");
  assert.equal(latest.remoteSource?.assetPattern, "^app");
  assert.equal(latest.remoteSource?.includePrereleases, true);
  assert.equal(latest.inspection, null);
  assert.equal(latest.review, null);

  const pinned = reduceAppGenerator(
    stateWith({
      installStrategy: "pinned_remote_asset",
      assetPattern: "^app",
      includePrereleases: true,
    }),
    { type: "remote-downloaded", apkHandle: "apk-handle-4", label: "app.apk", source },
  );
  assert.equal(pinned.remoteSource?.strategy, "pinned_remote_asset");
  assert.equal(pinned.remoteSource?.assetPattern, null);

  const downloading = reduceAppGenerator(stateWith({}), { type: "downloading" });
  assert.equal(downloading.phase, "downloading");
});

test("inspection transitions keep the reviewed checksum while a paired row exists", () => {
  const rowSha = "aa".repeat(32);
  const withRow = reduceAppGenerator(
    stateWith({ form: formWithRows([apkRow("apk", { sha256: rowSha })]), trustedSha256: "bb" }),
    { type: "inspecting" },
  );
  assert.equal(withRow.phase, "inspecting");
  assert.equal(withRow.trustedSha256, rowSha);

  const withoutRow = reduceAppGenerator(stateWith({}), { type: "inspecting" });
  assert.equal(withoutRow.trustedSha256, "");

  const inspected = reduceAppGenerator(withoutRow, {
    type: "inspected",
    inspection: inspection(),
  });
  assert.equal(inspected.phase, "editing");
  assert.equal(inspected.inspection?.inspectionHandle, "apk-inspection-1");
});

test("source-step pattern edits keep the reviewed artifact row aligned", () => {
  const row = apkRow("apk", {
    strategy: "latest_release",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    assetPattern: "",
  });
  const state = stateWith({
    remoteSource: {
      ...descriptorFor("v3"),
      strategy: "latest_compatible_release",
      assetPattern: "",
    },
    form: formWithRows([row, apkRow("extra", { strategy: "user_provided", assetPattern: "keep" })]),
    assetPattern: "",
    review: review(),
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "^app-v\\d+" });
  assert.equal(edited.assetPattern, "^app-v\\d+");
  assert.equal(edited.form?.mappings.artifacts[0]?.assetPattern, "^app-v\\d+");
  assert.equal(edited.form?.mappings.artifacts[1]?.assetPattern, "keep");
  assert.equal(edited.review, null);
  assert.equal(edited.draft, null);
});

test("source-step pattern edits never rewrite a customized artifact row", () => {
  const state = stateWith({
    form: formWithRows([apkRow("apk", { assetPattern: "custom" })]),
    assetPattern: "step",
    remoteSource: null,
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "next" });
  assert.equal(edited.assetPattern, "next");
  assert.equal(edited.form?.mappings.artifacts[0]?.assetPattern, "custom");
});

test("source-step checksum edits follow only rows still carrying the previous value", () => {
  const previous = "aa".repeat(32);
  const customized = "bb".repeat(32);
  const state = stateWith({
    form: formWithRows([
      apkRow("apk", { sha256: previous }),
      apkRow("apk2", { sha256: customized }),
    ]),
    trustedSha256: previous,
    appArtifactId: "apk",
  });
  const next = "cc".repeat(32);
  const edited = reduceAppGenerator(state, { type: "trusted-sha256", value: next });
  assert.equal(edited.trustedSha256, next);
  assert.equal(edited.form?.mappings.artifacts[0]?.sha256, next);
  assert.equal(edited.form?.mappings.artifacts[1]?.sha256, customized);
});

test("form edits mirror the paired row into the source-step controls", () => {
  const row = apkRow("apk", {
    strategy: "latest_release",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    assetPattern: "^row",
    prerelease: true,
    sha256: "dd".repeat(32),
  });
  const state = stateWith({
    remoteSource: {
      ...descriptorFor("v3"),
      strategy: "latest_compatible_release",
      assetPattern: "^row",
      includePrereleases: true,
    },
  });
  const edited = reduceAppGenerator(state, { type: "form", form: formWithRows([row]) });
  assert.equal(edited.assetPattern, "^row");
  assert.equal(edited.includePrereleases, true);
  assert.equal(edited.trustedSha256, "dd".repeat(32));
  assert.equal(edited.draft, null);
  assert.equal(edited.collisions, null);
});

test("prerelease policy keeps an eligible selection and clears an ineligible one", () => {
  const analysis = remoteAnalysis();
  const kept = reduceAppGenerator(
    stateWith({
      sourceMode: "github_repository",
      sourceAnalysis: analysis,
      selectedAssetHandle: "current",
    }),
    { type: "include-prereleases", value: true },
  );
  assert.equal(kept.includePrereleases, true);
  assert.equal(kept.selectedAssetHandle, "current");

  const cleared = reduceAppGenerator(
    stateWith({
      sourceMode: "github_repository",
      sourceAnalysis: analysis,
      selectedAssetHandle: "prerelease",
      apkHandle: "apk-handle-1",
      inspection: inspection(),
      form: formWithRows([apkRow("apk")]),
      review: review(),
    }),
    { type: "include-prereleases", value: false },
  );
  assert.equal(cleared.selectedAssetHandle, null);
  assert.equal(cleared.apkHandle, null);
  assert.equal(cleared.inspection, null);
  assert.equal(cleared.form, null);
  assert.equal(cleared.assetPattern, "");
  assert.equal(cleared.review, null);
});

test("artifact ids pair with the same trimming the payload uses", () => {
  const state = stateWith({
    form: formWithRows([apkRow("  apk  ", { assetPattern: "" })]),
    assetPattern: "",
    appArtifactId: "apk",
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "^app" });
  assert.equal(edited.form?.mappings.artifacts[0]?.assetPattern, "^app");
});

test("a file artifact never represents the reviewed remote source", () => {
  const state = stateWith({
    form: formWithRows([apkRow("apk", { kind: "file", assetPattern: "" })]),
    assetPattern: "",
    trustedSha256: "aa".repeat(32),
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "^app" });
  assert.equal(edited.assetPattern, "^app");
  assert.equal(edited.form?.mappings.artifacts[0]?.assetPattern, "");
});

test("several artifacts sharing the reviewed id leave pairing unresolved", () => {
  const state = stateWith({
    form: formWithRows([apkRow("apk", { assetPattern: "" }), apkRow("apk", { assetPattern: "" })]),
    assetPattern: "",
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "^app" });
  assert.equal(edited.form, state.form);
});

test("an explicit artifact id pairs independent of the recorded source strategy", () => {
  const reviewed = apkRow("main", {
    strategy: "latest_release",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    assetPattern: "",
  });
  const state = stateWith({
    appArtifactId: "main",
    remoteSource: {
      ...descriptorFor("v3"),
      strategy: "latest_compatible_release",
      assetPattern: "",
    },
    form: formWithRows([apkRow("apk", { assetPattern: "" }), reviewed]),
    assetPattern: "",
  });
  const edited = reduceAppGenerator(state, { type: "asset-pattern", value: "^main" });
  assert.equal(edited.form?.mappings.artifacts[0]?.assetPattern, "");
  assert.equal(edited.form?.mappings.artifacts[1]?.assetPattern, "^main");
});

test("retained analysis describes the reviewed latest-release policy", () => {
  const analysis = remoteAnalysis();
  assert.equal(releaseAnalysisDescribesReviewedPolicy(stateWith({})), false);
  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(stateWith({ sourceAnalysis: analysis })),
    true,
  );

  const matching = apkRow("apk", {
    strategy: "latest_release",
    provider: "github",
    baseUrl: "https://github.com/",
    repository: "Example/Project",
    assetPattern: "^app",
  });
  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(
      stateWith({ sourceAnalysis: analysis, form: formWithRows([matching]) }),
    ),
    true,
  );

  const retargeted = apkRow("apk", {
    strategy: "latest_release",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "other/project",
    assetPattern: "^app",
  });
  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(
      stateWith({ sourceAnalysis: analysis, form: formWithRows([retargeted]) }),
    ),
    false,
  );

  const gitlab = apkRow("apk", {
    strategy: "latest_release",
    provider: "gitlab",
    baseUrl: "https://gitlab.com",
    repository: "example/project",
    assetPattern: "^app",
  });
  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(
      stateWith({ sourceAnalysis: analysis, form: formWithRows([gitlab]) }),
    ),
    false,
  );

  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(
      stateWith({ sourceAnalysis: analysis, remoteSource: descriptorFor("v3") }),
    ),
    true,
  );
  const unrelated = descriptorFor("v3");
  unrelated.repository = "other/project";
  assert.equal(
    releaseAnalysisDescribesReviewedPolicy(
      stateWith({ sourceAnalysis: analysis, remoteSource: unrelated }),
    ),
    false,
  );
});

test("the source step shows the reviewed row checksum once a row exists", () => {
  const row = apkRow("apk", { sha256: "ee".repeat(32) });
  const form = formWithRows([row]);
  assert.equal(mirroredTrustedSha256(stateWith({ form }), form), "ee".repeat(32));
  assert.equal(mirroredTrustedSha256(stateWith({ trustedSha256: "11" }), null), "11");
});


test("a preview request supersedes older responses in both streams", () => {
  const previewDraft = draft();
  previewDraft.collisions = {
    collisions: [
      {
        severity: "blocking",
        code: "recipe_destination_exists",
        message: "The Recipe destination already exists.",
        existingId: null,
        relativePath: "recipes/app.example.install.yaml",
      },
    ],
    blocking: true,
  };
  const requested = reduceAppGenerator(
    stateWith({ pendingReviewRequestId: 4, lastIssuedRequestId: 4 }),
    { type: "preview-request", requestId: 6 },
  );
  assert.equal(requested.previewPending, true);
  assert.equal(requested.pendingPreviewRequestId, 6);
  assert.equal(requested.pendingReviewRequestId, null);
  assert.equal(requested.lastIssuedRequestId, 6);

  const late = reduceAppGenerator(requested, {
    type: "preview-response",
    requestId: 4,
    draft: previewDraft,
  });
  assert.equal(late, requested);

  const applied = reduceAppGenerator(requested, {
    type: "preview-response",
    requestId: 6,
    draft: previewDraft,
  });
  assert.equal(applied.previewPending, false);
  assert.equal(applied.pendingPreviewRequestId, null);
  assert.equal(applied.phase, "editing");
  assert.equal(applied.draft, previewDraft);
  assert.equal(applied.form?.app.id, "example");
  assert.equal(applied.collisions, previewDraft.collisions);
  assert.equal(applied.review, null);
});

test("a response for a superseded preview request never applies over newer intent", () => {
  const first = reduceAppGenerator(stateWith({}), { type: "preview-request", requestId: 1 });
  const second = reduceAppGenerator(first, { type: "preview-request", requestId: 2 });
  const older = draft();
  older.app.name = "Older";
  const applied = reduceAppGenerator(second, {
    type: "preview-response",
    requestId: 1,
    draft: older,
  });
  assert.equal(applied, second);
});

test("a review request and response track one pending review", () => {
  const requested = reduceAppGenerator(
    stateWith({ pendingPreviewRequestId: 8, lastIssuedRequestId: 8 }),
    { type: "review-request", requestId: 9 },
  );
  assert.equal(requested.reviewPending, true);
  assert.equal(requested.pendingReviewRequestId, 9);
  assert.equal(requested.pendingPreviewRequestId, null);

  const stale = reduceAppGenerator(requested, {
    type: "review-response",
    requestId: 8,
    review: review(),
  });
  assert.equal(stale, requested);

  const applied = reduceAppGenerator(requested, {
    type: "review-response",
    requestId: 9,
    review: review(),
  });
  assert.equal(applied.reviewPending, false);
  assert.equal(applied.pendingReviewRequestId, null);
  assert.equal(applied.review?.reviewHandle, "review-handle-1");
  assert.equal(applied.review?.artifact?.appArtifactId, "apk");
  assert.equal(applied.review?.target.packageLocked, false);
  assert.equal(applied.phase, "reviewing");
});

test("a save request keeps the review until the publication outcome arrives", () => {
  const requested = reduceAppGenerator(
    stateWith({
      review: review(),
      phase: "reviewing",
      pendingPreviewRequestId: 3,
      lastIssuedRequestId: 10,
    }),
    { type: "save-request", requestId: 11 },
  );
  assert.equal(requested.savePending, true);
  assert.equal(requested.pendingPreviewRequestId, null);
  assert.equal(requested.review?.reviewHandle, "review-handle-1");
  assert.equal(requested.lastIssuedRequestId, 11);

  const saved = reduceAppGenerator(requested, {
    type: "publication-response",
    result: savedOutcome(),
  });
  assert.equal(saved.savePending, false);
  assert.equal(saved.review, null);
  assert.equal(saved.pendingReviewRequestId, null);
  assert.equal(saved.publication?.outcome, "saved");
  assert.equal(saved.publicationStatus, null);
});

test("a failed publication reports failure without claiming success", () => {
  const failed = reduceAppGenerator(
    stateWith({ savePending: true, review: review() }),
    { type: "publication-response", result: failedOutcome() },
  );
  assert.equal(failed.savePending, false);
  assert.equal(failed.review, null);
  assert.equal(failed.publication?.outcome, "failed");
});

test("a failed publication status exposes its recorded Recipe recovery facts", () => {
  const failed = recordedStatus("failed", {
    outcome: "failed",
    rollback: {
      recipeRemoved: false,
      recipeRecoveryRequired: true,
      recipePath: "recipes/app.example.install.yaml",
    },
    error: {
      code: "recipe_commit_failed",
      message: "The Recipe file could not be committed completely.",
      details: { reason: "recipe_commit_failed" },
    },
  });
  assert.deepEqual(publicationFailureRecovery(failed), {
    message: "The Recipe file could not be committed completely.",
    recipePath: "recipes/app.example.install.yaml",
    recipeRecoveryRequired: true,
    recipeRemoved: false,
  });

  const removed = recordedStatus("failed", {
    rollback: { recipeRemoved: true, recipeRecoveryRequired: false },
    error: { code: "recipe_commit_failed", message: "   " },
  });
  assert.deepEqual(publicationFailureRecovery(removed), {
    message: null,
    recipePath: null,
    recipeRecoveryRequired: false,
    recipeRemoved: true,
  });

  const noRecord = recordedStatus("unknown", { reason: "no_record" });
  assert.deepEqual(publicationFailureRecovery(noRecord), {
    message: null,
    recipePath: null,
    recipeRecoveryRequired: false,
    recipeRemoved: false,
  });

  const malformed = recordedStatus("failed", { rollback: "removed", error: 12 });
  assert.deepEqual(publicationFailureRecovery(malformed), {
    message: null,
    recipePath: null,
    recipeRecoveryRequired: false,
    recipeRemoved: false,
  });
});

test("an unknown publication stays unresolved until a status lookup resolves it", () => {
  const unknown = reduceAppGenerator(
    stateWith({ savePending: true, review: review() }),
    { type: "publication-response", result: unknownOutcome() },
  );
  assert.equal(unknown.publication?.outcome, "unknown");
  assert.equal(unknown.review, null);
  assert.equal(publicationUnresolved(unknown), true);

  const requested = reduceAppGenerator(unknown, { type: "publication-status-request" });
  assert.equal(requested.publicationStatusPending, true);

  const stillUnknown = reduceAppGenerator(requested, {
    type: "publication-status-response",
    result: recordedStatus("unknown"),
  });
  assert.equal(stillUnknown.publicationStatusPending, false);
  assert.equal(stillUnknown.publicationStatus?.outcome, "unknown");
  assert.equal(publicationUnresolved(stillUnknown), true);

  const resolved = reduceAppGenerator(stillUnknown, {
    type: "publication-status-response",
    result: recordedStatus("saved"),
  });
  assert.equal(publicationUnresolved(resolved), false);

  assert.equal(publicationUnresolved(stateWith({ publication: savedOutcome() })), false);
  assert.equal(publicationUnresolved(stateWith({ publication: failedOutcome() })), false);
  assert.equal(publicationUnresolved(stateWith({})), false);
  assert.equal(
    publicationUnresolved(
      stateWith({ publication: unknownOutcome(), publicationStatus: recordedStatus("failed") }),
    ),
    false,
  );
});

test("an uncertain publication keeps the exact author identities for an explicit review", () => {
  const state = reduce(
    stateWith({
      appArtifactId: "main",
      recipeId: "app.example.main.install",
      publication: unknownOutcome(),
    }),
    [
      { type: "target-mode", mode: "existing_app" },
      { type: "target-mode", mode: "new_app" },
    ],
  );
  assert.equal(state.appArtifactId, "main");
  assert.equal(state.recipeId, "app.example.main.install");
  assert.equal(state.publication?.outcome, "unknown");
  assert.equal(publicationUnresolved(state), true);
});

test("a failure clears every pending flag and keeps the session", () => {
  const state = reduceAppGenerator(
    stateWith({
      phase: "reviewing",
      sessionHandle: "session-1",
      previewPending: true,
      reviewPending: true,
      savePending: true,
      existingAppsPending: true,
      existingAppSelectPending: true,
      publicationStatusPending: true,
      publication: unknownOutcome(),
    }),
    { type: "failure", message: "The sidecar is unavailable." },
  );
  assert.equal(state.error, "The sidecar is unavailable.");
  assert.equal(state.phase, "editing");
  assert.equal(state.previewPending, false);
  assert.equal(state.reviewPending, false);
  assert.equal(state.savePending, false);
  assert.equal(state.existingAppsPending, false);
  assert.equal(state.existingAppSelectPending, false);
  assert.equal(state.publicationStatusPending, false);
  assert.equal(state.sessionHandle, "session-1");
  assert.equal(publicationUnresolved(state), true);

  const starting = reduceAppGenerator(stateWith({ phase: "starting" }), {
    type: "failure",
    message: "No sidecar.",
  });
  assert.equal(starting.phase, "starting");
});

test("root selection records the trusted handle and clears collisions", () => {
  const state = reduceAppGenerator(
    stateWith({
      collisions: { collisions: [], blocking: false },
      review: review(),
      draft: draft(),
    }),
    { type: "root-selected", rootHandle: "root-1", label: "EmuChef authored" },
  );
  assert.equal(state.rootHandle, "root-1");
  assert.equal(state.rootLabel, "EmuChef authored");
  assert.equal(state.collisions, null);
  assert.equal(state.review, null);
  assert.equal(state.draft?.app.id, "example");
});

test("permission candidate selections update identity evidence and require an inspection", () => {
  const withoutInspection = reduceAppGenerator(stateWith({}), {
    type: "runtime-candidate-selected",
    index: 0,
    selected: true,
  });
  assert.equal(withoutInspection.inspection, null);

  const state = reduce(
    stateWith({ inspection: inspection(), draft: draft(), review: review() }),
    [
      { type: "runtime-candidate-selected", index: 0, selected: true },
      { type: "app-op-candidate-selected", index: 0, selected: true },
    ],
  );
  assert.equal(state.phase, "editing");
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, true);
  assert.equal(state.inspection?.appOpCandidates[0]?.selected, true);
  assert.equal(state.draft, null);
  assert.equal(state.review, null);
  assert.deepEqual(permissionSelectionForInspection(state.inspection), {
    inspectionHandle: "apk-inspection-1",
    runtimePermissions: [{ permissionName: "android.permission.CAMERA" }],
    appOps: [
      {
        permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE",
        operationName: "MANAGE_EXTERNAL_STORAGE",
        mode: "allow",
      },
    ],
  });
});

test("the automation opt-in records App policy intent and invalidates the review", () => {
  const state = reduceAppGenerator(
    stateWith({
      inspection: inspection(),
      review: review(),
      pendingPreviewRequestId: 3,
      pendingReviewRequestId: 4,
    }),
    { type: "automation-opt-in", value: true },
  );
  assert.equal(state.automationOptIn, true);
  assert.equal(state.review, null);
  assert.equal(state.pendingPreviewRequestId, null);
  assert.equal(state.pendingReviewRequestId, null);

  const optedOut = reduceAppGenerator(state, { type: "automation-opt-in", value: false });
  assert.equal(optedOut.automationOptIn, false);
});

test("install strategy changes reset the checksum and candidate selections", () => {
  const selected = inspection();
  selected.runtimeGrantCandidates[0].selected = true;
  const state = reduceAppGenerator(
    stateWith({
      installStrategy: "pinned_remote_asset",
      trustedSha256: "AB",
      inspection: selected,
      draft: draft(),
      form: draftToForm(draft()),
      collisions: { collisions: [], blocking: false },
    }),
    { type: "install-strategy", strategy: "latest_compatible_release" },
  );
  assert.equal(state.installStrategy, "latest_compatible_release");
  assert.equal(state.trustedSha256, "");
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, false);
  assert.equal(state.inspection?.inspectionHandle, "apk-inspection-1");
  assert.equal(state.draft, null);
  assert.equal(state.form, null);
  assert.equal(state.collisions, null);
});

test("trusted publisher checksum parsing accepts only plain hexadecimal digests", () => {
  assert.deepEqual(parseTrustedSha256(""), { ok: true, value: null });
  assert.deepEqual(parseTrustedSha256("  \t\n "), { ok: true, value: null });
  const digest = "ab".repeat(32);
  assert.deepEqual(parseTrustedSha256(" " + digest + " "), {
    ok: true,
    value: digest.toUpperCase(),
  });
  const spaced = digest.slice(0, 10) + " " + digest.slice(10);
  for (const input of [spaced, "abcd", digest.slice(0, 63), digest + "f"]) {
    assert.equal(parseTrustedSha256(input).ok, false);
  }
  const invalid = parseTrustedSha256("zz".repeat(32));
  assert.equal(invalid.ok, false);
  if (!invalid.ok) {
    assert.equal(
      invalid.message,
      "Trusted publisher SHA-256 must contain exactly 64 hexadecimal characters.",
    );
  }
});

test("inspected permission candidates partition into baseline and root-required groups", () => {
  const result = inspectionPartition(inspection());
  assert.deepEqual(result.runtime, { baseline: [0], elevated: [] });
  assert.deepEqual(result.appOps, { baseline: [], elevated: [0] });

  const expanded = inspection();
  expanded.runtimeGrantCandidates.push(
    {
      permissionName: "android.permission.LOCATION",
      requiresRoot: true,
      androidApiMin: 29,
      androidApiMax: null,
      selected: false,
    },
    {
      permissionName: "android.permission.CAMERA_EXTRA",
      requiresRoot: false,
      androidApiMin: 23,
      androidApiMax: null,
      selected: false,
    },
  );
  const partitioned = inspectionPartition(expanded);
  assert.deepEqual(partitioned.runtime.baseline, [0, 2]);
  assert.deepEqual(partitioned.runtime.elevated, [1]);
});

test("permission selections serialize identities with the opaque inspection binding", () => {
  assert.equal(permissionSelectionForInspection(null), null);
  assert.equal(permissionSelectionForInspection(inspection()), null);

  const selected = inspection();
  selected.runtimeGrantCandidates[0].selected = true;
  selected.appOpCandidates[0].selected = true;
  assert.deepEqual(permissionSelectionForInspection(selected), {
    inspectionHandle: "apk-inspection-1",
    runtimePermissions: [{ permissionName: "android.permission.CAMERA" }],
    appOps: [
      {
        permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE",
        operationName: "MANAGE_EXTERNAL_STORAGE",
        mode: "allow",
      },
    ],
  });
});

test("other requested permissions keep declarations outside the automation candidates", () => {
  const others = otherRequestedPermissions(inspection());
  assert.deepEqual(
    others.map((permission) => permission.name),
    [
      "android.permission.UNKNOWN",
      "android.permission.WRITE_EXTERNAL_STORAGE",
      "android.permission.MAYBE",
    ],
  );
});

test("inspection-wide warnings stay separate from declaration-specific outcomes", () => {
  const warnings = globalInspectionWarnings(inspection());
  assert.equal(warnings.length, 1);
  assert.equal(warnings[0]?.code, "apk_permission_signature_or_privileged");
});

test("declaration-specific warnings attach to their permission name", () => {
  const warnings = permissionWarnings(inspection(), "android.permission.UNKNOWN");
  assert.equal(warnings.length, 1);
  assert.equal(warnings[0]?.message, "Review this permission.");
  assert.deepEqual(permissionWarnings(inspection(), "android.permission.CAMERA"), []);
});

test("permission applicability labels describe status, reason, and context", () => {
  assert.equal(permissionApplicabilityLabel(null), "applicability unavailable");
  assert.equal(permissionApplicabilityLabel(applicable()), "applicable");
  assert.equal(
    permissionApplicabilityLabel({
      ...applicable(),
      status: "not_applicable",
      reason: "target_sdk_above_maximum",
      maximumTargetSdk: 29,
      actualTargetSdk: 35,
    }),
    "not applicable — app target SDK 35 exceeds maximum supported target SDK 29",
  );
  assert.equal(
    permissionApplicabilityLabel({
      ...applicable(),
      status: "indeterminate",
      reason: "invalid_max_sdk_version",
    }),
    "indeterminate — maximum SDK could not be interpreted",
  );
  assert.equal(
    permissionApplicabilityLabel({ ...applicable(), maximumSdkVersion: 28 }),
    "applicable (maximum SDK 28)",
  );
  assert.equal(
    permissionApplicabilityLabel({
      ...applicable(),
      status: "not_applicable",
      reason: "target_sdk_below_minimum",
      minimumTargetSdk: 30,
    }),
    "not applicable — application target SDK is below the required minimum; minimum target SDK 30",
  );
  assert.equal(
    permissionApplicabilityReasonLabel("target_sdk_above_maximum"),
    "application target SDK exceeds the supported maximum",
  );
});

test("artifact rows rebuild canonical sources and omit blank optional values", () => {
  assert.deepEqual(artifactSourceForRow(emptyArtifactRow()), { strategy: "user_provided" });

  const direct = apkRow("apk", {
    strategy: "direct_url",
    url: " https://example.com/app.apk ",
    sha256: " ABCD ",
  });
  assert.deepEqual(artifactSourceForRow(direct), {
    strategy: "direct_url",
    url: "https://example.com/app.apk",
    sha256: "abcd",
  });
  const directNoSha = apkRow("apk", {
    strategy: "direct_url",
    url: "https://example.com/app.apk",
    sha256: "  ",
  });
  assert.deepEqual(artifactSourceForRow(directNoSha), {
    strategy: "direct_url",
    url: "https://example.com/app.apk",
  });

  const latest = apkRow("apk", {
    strategy: "latest_release",
    provider: "gitlab",
    baseUrl: " https://gitlab.com ",
    repository: " example/project ",
    assetPattern: "",
    invertAssetPattern: true,
    prerelease: true,
  });
  assert.deepEqual(artifactSourceForRow(latest), {
    strategy: "latest_release",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    repository: "example/project",
    prerelease: true,
  });
  const patterned = { ...latest, assetPattern: "^app", invertAssetPattern: true };
  assert.deepEqual(artifactSourceForRow(patterned), {
    strategy: "latest_release",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    repository: "example/project",
    asset_pattern: "^app",
    invert_asset_pattern: true,
    prerelease: true,
  });
});

test("artifact edits trim identities and omit blank display text", () => {
  assert.deepEqual(
    artifactEditForRow(
      apkRow(" main ", { strategy: "user_provided", name: " Main APK ", description: "  " }),
    ),
    { id: "main", kind: "apk", name: "Main APK", source: { strategy: "user_provided" } },
  );
  assert.deepEqual(artifactEditForRow(emptyArtifactRow("")), {
    id: "",
    kind: "apk",
    source: { strategy: "user_provided" },
  });
});

test("target and mapping edits convert rows into the submitted payload", () => {
  assert.deepEqual(targetEditForRow(emptyTargetRow(" data ")), {
    id: "data",
    kind: "file",
    location: "app_data",
    path: "",
  });
  const mappings = mappingEditsFromRows({
    artifacts: [emptyArtifactRow("apk")],
    targets: [emptyTargetRow("logs")],
    metadata: "{}",
  });
  assert.deepEqual(mappings, {
    artifacts: [{ id: "apk", kind: "apk", source: { strategy: "user_provided" } }],
    targets: [{ id: "logs", kind: "file", location: "app_data", path: "" }],
    metadata: "{}",
  });
});

test("canonical artifact and target definitions round-trip through editable rows", () => {
  const row = artifactRowForDefinition("apk", {
    kind: "apk",
    name: "Example APK",
    source: {
      strategy: "latest_release",
      provider: "github",
      base_url: "https://github.com",
      repository: "example/project",
      asset_pattern: "^app",
      invert_asset_pattern: true,
      prerelease: true,
    },
  });
  assert.equal(row.id, "apk");
  assert.equal(row.strategy, "latest_release");
  assert.equal(row.assetPattern, "^app");
  assert.equal(row.invertAssetPattern, true);
  assert.equal(row.prerelease, true);
  assert.deepEqual(artifactSourceForRow(row), {
    strategy: "latest_release",
    provider: "github",
    base_url: "https://github.com",
    repository: "example/project",
    asset_pattern: "^app",
    invert_asset_pattern: true,
    prerelease: true,
  });

  const target = targetRowForDefinition("data", {
    kind: "directory",
    location: "app_data",
    path: "com.example.app",
  });
  assert.deepEqual(target, {
    id: "data",
    kind: "directory",
    location: "app_data",
    path: "com.example.app",
  });
});

test("draft conversion builds artifact, target, and metadata rows from the canonical App", () => {
  const form = draftToForm(draft());
  assert.equal(form.app.id, "example");
  assert.equal(form.mappings.artifacts.length, 1);
  assert.equal(form.mappings.artifacts[0]?.id, "apk");
  assert.equal(form.mappings.artifacts[0]?.strategy, "user_provided");
  assert.deepEqual(form.mappings.targets, [
    { id: "data", kind: "directory", location: "app_data", path: "com.example.app" },
  ]);
  assert.equal(
    form.mappings.metadata,
    JSON.stringify({ homepage: "https://example.com" }, null, 2),
  );
  assert.equal(form.recipe.ids?.recipeId, "app.example.install");
  assert.equal(form.recipe.launcherActivity, "com.example.app/.MainActivity");
});

test("form requests clone App and Recipe edits and parse metadata", () => {
  const form = draftToForm(draft());
  const result = formToRequest(form);
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.app.id, "example");
    assert.equal(result.mappings.artifacts[0]?.id, "apk");
    assert.equal(
      result.mappings.metadata,
      JSON.stringify({ homepage: "https://example.com" }, null, 2),
    );
    result.app.name = "Mutated";
    assert.equal(form.app.name, "Example");
  }
});

test("form requests reject metadata that is not a plain JSON object", () => {
  const form = draftToForm(draft());
  form.mappings.metadata = "{not json";
  const invalidJson = formToRequest(form);
  assert.equal(invalidJson.ok, false);
  if (!invalidJson.ok) {
    assert.match(invalidJson.message, /^Metadata: /u);
  }
  form.mappings.metadata = "{\"a\": 1, \"a\": 2}";
  assert.equal(formToRequest(form).ok, false);
});

test("root-backed reviews hide the pre-root validation notice", () => {
  const diagnostics = [
    {
      severity: "warning" as const,
      code: "validation_context_limited",
      message: "Catalog validation is limited.",
      field: "root",
    },
    {
      severity: "error" as const,
      code: "recipe_id_collision",
      message: "The Recipe id is already used.",
      field: "recipe.id",
    },
  ];
  assert.equal(visibleDraftDiagnostics(diagnostics, false).length, 2);
  const hidden = visibleDraftDiagnostics(diagnostics, true);
  assert.equal(hidden.length, 1);
  assert.equal(hidden[0]?.code, "recipe_id_collision");
});

test("diagnostic titles translate internal codes for display", () => {
  assert.equal(
    diagnosticDisplayTitle("validation_context_limited", "warning"),
    "Catalog validation not yet available",
  );
  assert.equal(
    diagnosticDisplayTitle("apk_certificate_missing", "warning"),
    "Signing certificate unavailable",
  );
  assert.equal(diagnosticDisplayTitle("apk_label_missing", "warning"), "Application name derived");
  assert.equal(diagnosticDisplayTitle("recipe_id_collision", "error"), "Action required");
  assert.equal(diagnosticDisplayTitle("recipe_id_collision", "warning"), "Review recommended");
});

test("asset helpers filter by APK kind, release, and filename pattern", () => {
  const analysis = remoteAnalysis();
  assert.equal(eligibleApkAssets(null).length, 0);
  assert.equal(eligibleApkAssets(analysis).length, 5);
  assert.deepEqual(
    eligibleApkAssets(analysis, "v1").map((asset) => asset.fileName),
    ["app-v2-arm64.apk", "app-v1-arm64.apk"],
  );
  assert.deepEqual(
    matchingAssetNames("v1", ["app-v1-arm64.apk", "app-v3-arm64.apk"]),
    ["app-v1-arm64.apk"],
  );
  assert.deepEqual(matchingAssetNames("[", ["app.apk"]), []);
});

test("release pattern preview counts a single eligible APK per release", () => {
  const preview = buildReleasePatternPreview(remoteAnalysis(), "", false);
  assert.equal(preview.blocking, false);
  assert.equal(preview.patternError, null);
  assert.equal(preview.eligibleReleaseCount, 3);
  assert.deepEqual(preview.releases.map((release) => release.releaseTag), ["v3", "v2", "v1"]);
  assert.deepEqual(preview.releases[2]?.matchingAssetNames, [
    "app-v1-arm64.apk",
    "app-v2-arm64.apk",
  ]);
  assert.equal(preview.releases[2]?.outcome, "multiple_matches");
  assert.equal(preview.uniqueMatchCount, 2);
  assert.equal(preview.noMatchCount, 0);
  assert.equal(preview.multipleMatchesCount, 1);

  const withPrereleases = buildReleasePatternPreview(remoteAnalysis(), "", true);
  assert.equal(withPrereleases.eligibleReleaseCount, 4);
  assert.equal(withPrereleases.uniqueMatchCount, 3);
});

test("release pattern preview blocks only when the newest eligible release cannot resolve", () => {
  const noneNewest = buildReleasePatternPreview(remoteAnalysis(), "^zz", false);
  assert.equal(noneNewest.blocking, true);
  assert.equal(
    noneNewest.blockingMessage,
    "The newest eligible analyzed release (v3) has no matching APK.",
  );
  assert.equal(noneNewest.patternError, null);
  assert.equal(noneNewest.noMatchCount, 3);

  const historical = buildReleasePatternPreview(remoteAnalysis(), "^app-v3", false);
  assert.equal(historical.blocking, false);
  assert.equal(historical.uniqueMatchCount, 1);
  assert.equal(historical.noMatchCount, 2);

  const multiple = remoteAnalysis();
  multiple.releases = [
    {
      tag: "v9",
      name: null,
      prerelease: false,
      publishedAt: null,
      assets: [
        remoteAsset("a1", "app-v9-universal.apk", "v9", false),
        remoteAsset("a2", "app-v9-arm64.apk", "v9", false),
      ],
    },
  ];
  const multiplePreview = buildReleasePatternPreview(multiple, "^app", false);
  assert.equal(multiplePreview.blocking, true);
  assert.equal(
    multiplePreview.blockingMessage,
    "The newest eligible analyzed release (v9) has multiple matching APKs.",
  );

  const invalid = buildReleasePatternPreview(remoteAnalysis(), "[", false);
  assert.equal(invalid.blocking, true);
  assert.equal(
    invalid.patternError,
    "Enter a valid regular expression. Rust performs final validation before generation.",
  );
  assert.equal(invalid.eligibleReleaseCount, 3);

  const noneEligible = remoteAnalysis();
  noneEligible.releases = [];
  const blocked = buildReleasePatternPreview(noneEligible, "", false);
  assert.equal(blocked.blocking, true);
  assert.equal(
    blocked.patternError,
    "No analyzed releases are eligible for this prerelease policy.",
  );
  assert.equal(blocked.eligibleReleaseCount, 0);
});

test("prerelease policy filtering retains the snapshot but hides prerelease picks", () => {
  const analysis = remoteAnalysis();
  assert.equal(analysisForPrereleasePolicy(analysis, true), analysis);

  const filtered = analysisForPrereleasePolicy(analysis, false);
  assert.equal(filtered.releases.length, 3);
  assert.equal(filtered.assets.length, 4);
  assert.equal(filtered.preselectedAssetHandle, null);
  assert.equal(analysis.releases.length, 4);

  const single = remoteAnalysis();
  single.assets = [remoteAsset("only", "app-v1-arm64.apk", "v1", false)];
  single.releases = [
    { tag: "v1", name: null, prerelease: false, publishedAt: null, assets: single.assets },
  ];
  assert.equal(analysisForPrereleasePolicy(single, false).preselectedAssetHandle, "only");

  const localMode = { ...analysis, mode: "direct_apk" as const };
  assert.equal(analysisForPrereleasePolicy(localMode, false), localMode);
});

test("asset pattern validation rejects invalid, empty, and ambiguous rules", () => {
  assert.equal(assetPatternError("", ["app-v1.apk"]), null);
  assert.equal(
    assetPatternError("", ["app.apk", "app-2.apk"]),
    "The selected release has multiple eligible APK assets. Add an APK filename pattern to select one.",
  );
  assert.equal(
    assetPatternError("", ["readme.txt"]),
    "The selected release has no eligible APK asset.",
  );
  assert.equal(assetPatternError("^app", ["app.apk"]), null);
  assert.equal(
    assetPatternError("^zz", ["app.apk"]),
    "The pattern does not match an APK in the selected release.",
  );
  assert.equal(
    assetPatternError("^app", ["app.apk", "app-2.apk"]),
    "The pattern matches multiple APKs. Make it more specific.",
  );
  assert.equal(assetPatternError("[", ["app.apk"]), "Enter a valid regular expression.");
});

test("asset pattern suggestions generalize the version while preserving the variant", () => {
  assert.equal(suggestAssetPattern("", ["app.apk"]), "");
  assert.equal(
    suggestAssetPattern("app-1.2.3.apk", ["app-1.2.3.apk"]),
    "^app-v?\\d+(?:[._-]\\d+){1,4}\\.apk$",
  );
  assert.equal(
    suggestAssetPattern("app-v3-arm64.apk", ["app-v3-arm64.apk"]),
    "^app-v3-arm64\\.apk$",
  );
  assert.equal(
    suggestAssetPattern("app-v1-arm64.apk", ["app-v2-arm64.apk", "app-v1-arm64.apk"]),
    "^app-v1-arm64\\.apk$",
  );
});


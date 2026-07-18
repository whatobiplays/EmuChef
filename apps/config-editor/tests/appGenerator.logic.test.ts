import assert from "node:assert/strict";
import test from "node:test";

import type {
  ApkInspectionResult,
  AppRecipeDraftResult,
  AppRecipeSaveResult,
} from "../src/api/types.js";
import type { AppGeneratorState } from "../src/components/appGenerator.logic.js";
import {
  assetPatternError,
  diagnosticDisplayTitle,
  draftToForm,
  formToRequest,
  initialAppGeneratorState,
  matchingAssetNames,
  otherRequestedPermissions,
  parseConnectedDeviceApi,
  parseTrustedSha256,
  readableNameFromPackage,
  reduceAppGenerator,
  suggestAssetPattern,
  visibleDraftDiagnostics,
} from "../src/components/appGenerator.logic.js";

function inspection(): ApkInspectionResult {
  const applicable = {
    status: "applicable" as const,
    reason: null,
    maximumSdkVersion: null,
    introductionApi: null,
    minimumDeviceApi: null,
    minimumTargetSdk: null,
    targetSdkState: null,
  };
  return {
    manifest: {
      packageName: "com.example.app",
      versionCode: "42",
      versionName: "1.2",
      minSdkVersion: "23",
      targetSdkVersion: "35",
    },
    permissions: [
      { name: "android.permission.CAMERA", declarationKind: "uses_permission", maxSdkVersion: null, classification: "runtime_grantable", applicability: applicable },
      { name: "android.permission.MANAGE_EXTERNAL_STORAGE", declarationKind: "uses_permission", maxSdkVersion: null, classification: "app_op_grantable", applicability: applicable },
      { name: "android.permission.UNKNOWN", declarationKind: "uses_permission", maxSdkVersion: null, classification: "unknown", applicability: applicable },
      {
        name: "android.permission.OLD",
        declarationKind: "uses_permission",
        maxSdkVersion: "28",
        classification: "runtime_grantable",
        applicability: { ...applicable, status: "not_applicable", reason: "max_sdk_version_exceeded", maximumSdkVersion: 28 },
      },
      {
        name: "android.permission.MAYBE",
        declarationKind: "uses_permission_sdk_23",
        maxSdkVersion: "preview",
        classification: "unknown",
        applicability: { ...applicable, status: "indeterminate", reason: "invalid_max_sdk_version" },
      },
    ],
    runtimeGrantCandidates: [{ permissionName: "android.permission.CAMERA", requiresRoot: false, selected: false }],
    appOpCandidates: [{ permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE", operationName: "MANAGE_EXTERNAL_STORAGE", mode: "allow", requiresRoot: true, selected: false }],
    warnings: [{ code: "apk_permission_unknown", message: "Review this permission.", permissionName: "android.permission.UNKNOWN", applicabilityReason: null }],
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
      package: { primary: "com.example.app", aliases: [] },
      install_source: {
        type: "user_provided_apk",
        resolver: "none",
        options: {},
      },
      tracking_source: { type: "local_apk" },
      artifacts: {
        apk: { required: false },
        shared_storage_config: { supported: false },
        app_data_config: { supported: false },
        byo_apk: { required: true },
      },
      provisioning: {
        launch_once_recommended: false,
        shared_storage_paths: [],
        app_data_paths: [],
        config_targets: [],
      },
      inputs: [],
      metadata: {},
    },
    recipe: {
      schemaVersion: 1,
      kind: "recipe",
      id: "app.example.install",
      name: "Install Example",
      description: "",
      recipeDependencies: [],
      provides: { features: ["example_install"] },
      inputs: {},
      artifacts: {},
      artifactGroups: {},
      steps: [],
    },
    recipeEdits: {
      ids: {
        recipeId: "app.example.install",
        inputId: "example_apk",
        featureId: "example_install",
        installStepId: "install_example",
        launchStepId: "launch_example",
      },
      name: "Install Example",
      description: "",
      inputLabel: "Example APK",
      inputDescription: "",
      replaceExisting: false,
      launchEnabled: false,
      launcherActivity: null,
    },
    appCanonicalYaml: "app",
    recipeCanonicalYaml: "recipe",
    appDestination: {
      fileName: "example.yaml",
      relativePath: "apps/example.yaml",
    },
    recipeDestination: {
      fileName: "app.example.install.yaml",
      relativePath: "recipes/app.example.install.yaml",
    },
    evidence: [],
    diagnostics: [],
    blocking: false,
  };
}

test("package fallback creates a readable app name and matching recipe text", () => {
  assert.equal(readableNameFromPackage("dev.eden.eden_emulator"), "Eden Emulator");
  const packageDraft = draft();
  packageDraft.app.name = "dev.eden.eden_emulator";
  packageDraft.app.package.primary = "dev.eden.eden_emulator";
  packageDraft.recipeEdits.name = "Install dev.eden.eden_emulator";
  packageDraft.recipeEdits.description = "Install a user-provided dev.eden.eden_emulator APK.";
  packageDraft.recipeEdits.inputLabel = "dev.eden.eden_emulator APK";
  packageDraft.recipeEdits.inputDescription = "Local dev.eden.eden_emulator APK to install.";
  const form = draftToForm(packageDraft);
  assert.equal(form.app.name, "Eden Emulator");
  assert.equal(form.recipe.name, "Install Eden Emulator");
  assert.equal(form.recipe.inputLabel, "Eden Emulator APK");
});

test("asset pattern generalizes version while preserving selected variant", () => {
  const files = [
    "Eden-0.0.4-android-arm64-v8a.apk",
    "Eden-0.0.4-android-x86_64.apk",
    "Eden-0.0.4-debug-arm64-v8a.apk",
  ];
  const pattern = suggestAssetPattern(files[0], files);
  assert.doesNotMatch(pattern, /0\\\.0\\\.4/u);
  assert.match(pattern, /arm64-v8a/u);
  assert.deepEqual(matchingAssetNames(pattern, files), [files[0]]);
  assert.equal(assetPatternError(pattern, files), null);
});

test("asset pattern validation rejects invalid zero-match and ambiguous rules", () => {
  const files = ["app-v1.2.3-arm64.apk", "app-v1.2.3-x86_64.apk"];
  assert.match(assetPatternError("[", files) ?? "", /valid regular expression/u);
  assert.match(assetPatternError("^missing", files) ?? "", /does not match/u);
  assert.match(assetPatternError("^app-.*\\.apk$", files) ?? "", /multiple APKs/u);
});

test("remote download has a distinct busy phase before inspection", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
  });
  state = reduceAppGenerator(state, { type: "downloading" });
  assert.equal(state.phase, "downloading");
  state = reduceAppGenerator(state, { type: "inspecting" });
  assert.equal(state.phase, "inspecting");
});

test("started session restores only the trusted root handle", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
    rootHandle: "root",
    rootLabel: "Selected authored root",
  });
  assert.equal(state.rootHandle, "root");
});

test("connected-device API validation accepts blank and positive u32 values", () => {
  assert.deepEqual(parseConnectedDeviceApi(""), { ok: true, value: null });
  assert.deepEqual(parseConnectedDeviceApi(" 35 "), { ok: true, value: 35 });
  assert.deepEqual(parseConnectedDeviceApi("4294967295"), { ok: true, value: 4_294_967_295 });
  for (const invalid of ["0", "-1", "1.5", "preview", "4294967296"]) {
    assert.equal(parseConnectedDeviceApi(invalid).ok, false, invalid);
  }
});

test("trusted publisher SHA-256 accepts only plain hexadecimal and normalizes uppercase", () => {
  assert.deepEqual(parseTrustedSha256(" \t\r\n"), { ok: true, value: null });
  assert.deepEqual(
    parseTrustedSha256(
      " \t0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n",
    ),
    {
      ok: true,
      value: "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
    },
  );
  for (const invalid of [
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "0123456789abcdef0123456789abcdef 0123456789abcdef0123456789abcdef",
    "0123456789abcdef0123456789abcdef-0123456789abcdef0123456789abcdef",
    "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "0123456789abcdef",
    "\u00A00123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\u00A0",
  ]) {
    assert.equal(parseTrustedSha256(invalid).ok, false, invalid);
  }
});

test("trusted checksum edits invalidate reviewed output without clearing editable fields", () => {
  const generated = draft();
  const collisions = { collisions: [], blocking: false };
  const form = draftToForm(generated);
  const state: AppGeneratorState = {
    ...initialAppGeneratorState,
    phase: "reviewing",
    draft: generated,
    form,
    collisions,
    saved: {} as AppRecipeSaveResult,
  };

  const next = reduceAppGenerator(state, {
    type: "trusted-sha256",
    value: "A".repeat(64),
  });

  assert.equal(next.trustedSha256, "A".repeat(64));
  assert.equal(next.form, form);
  assert.equal(next.draft, null);
  assert.equal(next.collisions, null);
  assert.equal(next.saved, null);
});

test("trusted checksum resets for strategy, APK, and inspection transitions", () => {
  const withChecksum = {
    ...initialAppGeneratorState,
    trustedSha256: "A".repeat(64),
  };
  assert.equal(
    reduceAppGenerator(withChecksum, {
      type: "install-strategy",
      strategy: "latest_compatible_release",
    }).trustedSha256,
    "",
  );
  assert.equal(
    reduceAppGenerator(withChecksum, {
      type: "apk-selected",
      apkHandle: "replacement",
      label: "Replacement APK",
    }).trustedSha256,
    "",
  );
  const inspecting = reduceAppGenerator(withChecksum, { type: "inspecting" });
  assert.equal(inspecting.trustedSha256, "");
  assert.equal(
    reduceAppGenerator(inspecting, { type: "inspected", inspection: inspection() })
      .trustedSha256,
    "",
    "calculatedSha256 must never initialize trustedSha256",
  );
});

test("permission review keeps candidates separate from all other declarations", () => {
  assert.deepEqual(
    otherRequestedPermissions(inspection()).map((permission) => permission.name),
    ["android.permission.UNKNOWN", "android.permission.OLD", "android.permission.MAYBE"],
  );
  const withoutContext = inspection();
  withoutContext.permissions = withoutContext.permissions.map((permission) => ({
    ...permission,
    classification: null,
    applicability: null,
  }));
  withoutContext.runtimeGrantCandidates = [];
  withoutContext.appOpCandidates = [];
  assert.equal(otherRequestedPermissions(withoutContext).length, withoutContext.permissions.length);
});

test("candidate review state resets on inspection and never invalidates generated work", () => {
  const generated = draft();
  const collisions = { collisions: [], blocking: false };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: generated });
  state = reduceAppGenerator(state, { type: "reviewed", draft: generated, collisions });
  state = reduceAppGenerator(state, { type: "inspected", inspection: inspection() });
  const form = state.form;
  state = reduceAppGenerator(state, { type: "runtime-candidate-selected", index: 0, selected: true });
  state = reduceAppGenerator(state, { type: "app-op-candidate-selected", index: 0, selected: true });
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, true);
  assert.equal(state.inspection?.appOpCandidates[0]?.selected, true);
  assert.equal(state.draft, generated);
  assert.equal(state.form, form);
  assert.equal(state.collisions, collisions);

  state = reduceAppGenerator(state, { type: "inspected", inspection: inspection() });
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, false);
  assert.equal(state.inspection?.appOpCandidates[0]?.selected, false);
  state = reduceAppGenerator(state, { type: "apk-selected", apkHandle: "new-apk", label: "New APK" });
  assert.equal(state.inspection, null);
});

test("reducer clears stale review when the form changes", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
  });
  state = reduceAppGenerator(state, { type: "drafted", draft: draft() });
  assert.equal(state.phase, "editing");
  assert.ok(state.form);
  state = reduceAppGenerator(state, { type: "form", form: state.form! });
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);
});

test("form conversion rejects duplicate keys at any nesting depth", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.metadata = '{"outer":{"same":1,"same":2}}';
  const result = formToRequest(form);
  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.message, /duplicate key/u);
});

test("form conversion retains ordered mapping source text for backend parsing", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.metadata = '{"z":1,"a":2}';
  form.aliases = ["com.example.old", "com.example.legacy"];
  const result = formToRequest(form);
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.mappings.metadata, '{"z":1,"a":2}');
    assert.deepEqual(result.app.package.aliases, [
      "com.example.old",
      "com.example.legacy",
    ]);
  }
});

test("form state preserves an empty package alias row", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.aliases = [""];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.aliases, [""]);
});

test("form state preserves empty shared-storage and app-data path rows", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.sharedStoragePaths = [""];
  form.appDataPaths = [""];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.sharedStoragePaths, [""]);
  assert.deepEqual(next.form?.appDataPaths, [""]);
});

test("root-backed review hides the pre-root validation warning", () => {
  const diagnostics = [
    { severity: "warning" as const, code: "validation_context_limited", message: "limited", field: "authored_root" },
    { severity: "warning" as const, code: "other_warning", message: "keep", field: "app" },
  ];
  assert.deepEqual(visibleDraftDiagnostics(diagnostics, false), diagnostics);
  assert.deepEqual(visibleDraftDiagnostics(diagnostics, true), [diagnostics[1]]);
});

test("diagnostic titles do not expose internal codes", () => {
  assert.equal(diagnosticDisplayTitle("validation_context_limited", "warning"), "Catalog validation not yet available");
  assert.equal(diagnosticDisplayTitle("unknown_internal_code", "error"), "Action required");
});

test("changing source mode clears stale analysis and APK state", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "started",
    sessionHandle: "session",
  });
  state = reduceAppGenerator(state, {
    type: "source-analyzed",
    analysis: {
      sourceHandle: "source",
      mode: "github_repository",
      normalizedUrl: "https://github.com/example/project",
      capabilities: {
        pinnedArtifact: true,
        latestRelease: true,
        prereleaseFiltering: true,
        deterministicAssetFiltering: true,
      },
      repository: { fullName: "example/project", name: "project", description: null, htmlUrl: "https://github.com/example/project" },
      releases: [],
      assets: [{ assetHandle: "asset", fileName: "app.apk", size: 10, contentType: "application/vnd.android.package-archive", releaseTag: "v1", releaseName: null, prerelease: false, publishedAt: null }],
      preselectedAssetHandle: "asset",
    },
  });
  state = reduceAppGenerator(state, { type: "source-mode", mode: "direct_apk" });
  assert.equal(state.sourceMode, "direct_apk");
  assert.equal(state.sourceAnalysis, null);
  assert.equal(state.selectedAssetHandle, null);
  assert.equal(state.apkHandle, null);
});

test("remote analysis preselects one asset and strategy changes invalidate drafts", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "source-analyzed",
    analysis: {
      sourceHandle: "source",
      mode: "github_release",
      normalizedUrl: "https://github.com/example/project/releases/tag/v1",
      capabilities: {
        pinnedArtifact: true,
        latestRelease: false,
        prereleaseFiltering: false,
        deterministicAssetFiltering: false,
      },
      repository: { fullName: "example/project", name: "project", description: null, htmlUrl: "https://github.com/example/project" },
      releases: [],
      assets: [{ assetHandle: "asset", fileName: "app.apk", size: 10, contentType: null, releaseTag: "v1", releaseName: null, prerelease: false, publishedAt: null }],
      preselectedAssetHandle: "asset",
    },
  });
  assert.equal(state.selectedAssetHandle, "asset");
  state = { ...state, draft: draft(), form: draftToForm(draft()) };
  state = reduceAppGenerator(state, { type: "install-strategy", strategy: "user_provided_apk" });
  assert.equal(state.installStrategy, "user_provided_apk");
  assert.equal(state.draft, null);
  assert.equal(state.form, null);
});

test("remote download preserves the selected strategy in trusted source state", () => {
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "install-strategy", strategy: "user_provided_apk" });
  state = reduceAppGenerator(state, {
    type: "remote-downloaded",
    apkHandle: "apk",
    label: "app.apk",
    source: {
      mode: "direct_apk",
      strategy: "pinned_remote_asset",
      downloadUrl: "https://example.com/app.apk",
      provider: null,
      baseUrl: null,
      repository: null,
      releaseTag: null,
      assetName: "app.apk",
      assetPattern: null,
      includePrereleases: false,
    },
  });
  assert.equal(state.apkHandle, "apk");
  assert.equal(state.remoteSource?.strategy, "user_provided_apk");
});

test("remote download preserves the selected latest-release asset policy", () => {
  let state: AppGeneratorState = {
    ...initialAppGeneratorState,
    installStrategy: "latest_compatible_release" as const,
    assetPattern: "^app-v.*-arm64\\.apk$",
    includePrereleases: true,
  };
  state = reduceAppGenerator(state, {
    type: "remote-downloaded",
    apkHandle: "apk",
    label: "app-v1-arm64.apk",
    source: {
      mode: "github_repository",
      strategy: "pinned_remote_asset",
      downloadUrl: "https://example.com/app-v1-arm64.apk",
      provider: "github",
      baseUrl: "https://github.com",
      repository: "example/project",
      releaseTag: "v1",
      assetName: "app-v1-arm64.apk",
      assetPattern: null,
      includePrereleases: false,
    },
  });
  assert.equal(state.remoteSource?.strategy, "latest_compatible_release");
  assert.equal(state.remoteSource?.assetPattern, "^app-v.*-arm64\\.apk$");
  assert.equal(state.remoteSource?.includePrereleases, true);
});

test("reducer records the opened recipe result after an explicit save", () => {
  const result = {
    appFileName: "example.yaml",
    appRelativePath: "apps/example.yaml",
    recipeFileName: "app.example.install.yaml",
    recipeRelativePath: "recipes/app.example.install.yaml",
    openedRecipe: {
      document: {
        documentId: "document",
        path: "recipes/app.example.install.yaml",
        authoredRoot: null,
        recipe: draft().recipe,
        diagnostics: [],
        yaml: "recipe",
        dirty: false,
        canUndo: false,
        canRedo: false,
        refIndex: {
          inputRefs: [],
          artifactRefs: [],
          stepRefs: [],
          stepOutputRefs: [],
          allRefs: [],
          candidates: [],
        },
      },
    },
  } satisfies AppRecipeSaveResult;
  let state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  state = reduceAppGenerator(state, { type: "saving" });
  assert.equal(state.phase, "saving");
  state = reduceAppGenerator(state, { type: "saved", result });
  assert.equal(state.phase, "saved");
  assert.equal(state.saved?.openedRecipe.document.documentId, "document");
});

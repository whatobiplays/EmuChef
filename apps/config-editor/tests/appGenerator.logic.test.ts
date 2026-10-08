import assert from "node:assert/strict";
import test from "node:test";

import type {
  ApkInspectionResult,
  AppRecipeDraftResult,
  AppRecipeSaveResult,
  RemoteSourceAnalysisResult,
  RemoteSourceDescriptorDto,
} from "../src/api/types.js";
import type {
  AppArtifactRowEdits,
  AppGeneratorState,
} from "../src/components/appGenerator.logic.js";
import {
  analysisForPrereleasePolicy,
  artifactSourceForRow,
  assetPatternError,
  buildReleasePatternPreview,
  diagnosticDisplayTitle,
  draftToForm,
  emptyArtifactRow,
  emptyTargetRow,
  formToRequest,
  globalInspectionWarnings,
  initialAppGeneratorState,
  matchingAssetNames,
  mirroredTrustedSha256,
  otherRequestedPermissions,
  parseTrustedSha256,
  permissionApplicabilityLabel,
  permissionAutomationEligible,
  permissionSelectionForInspection,
  readableNameFromPackage,
  reduceAppGenerator,
  releaseAnalysisDescribesReviewedPolicy,
  suggestAssetPattern,
  visibleDraftDiagnostics,
} from "../src/components/appGenerator.logic.js";

function remoteAnalysis(): RemoteSourceAnalysisResult {
  const asset = (
    assetHandle: string,
    fileName: string,
    releaseTag: string,
    prerelease: boolean,
  ) => ({
    assetHandle,
    fileName,
    size: 10,
    contentType: "application/vnd.android.package-archive",
    releaseTag,
    releaseName: null,
    prerelease,
    publishedAt: null,
  });
  const current = asset("current", "app-v3-arm64.apk", "v3", false);
  const olderOther = asset("older-other", "other.apk", "v2", false);
  const olderOne = asset("older-one", "app-v1-arm64.apk", "v1", false);
  const olderTwo = asset("older-two", "app-v2-arm64.apk", "v1", false);
  const prerelease = asset("prerelease", "app-v4-arm64.apk", "v4-beta", true);
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
      { tag: "v4-beta", name: null, prerelease: true, publishedAt: null, assets: [prerelease] },
      { tag: "v3", name: null, prerelease: false, publishedAt: null, assets: [current] },
      { tag: "v2", name: null, prerelease: false, publishedAt: null, assets: [olderOther] },
      { tag: "v1", name: null, prerelease: false, publishedAt: null, assets: [olderTwo, olderOne] },
    ],
    assets: [prerelease, current, olderOther, olderTwo, olderOne],
    preselectedAssetHandle: null,
  };
}

function inspection(): ApkInspectionResult {
  const applicable = {
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
      {
        name: "android.permission.WRITE_EXTERNAL_STORAGE",
        declarationKind: "uses_permission",
        maxSdkVersion: null,
        classification: "runtime_grantable",
        applicability: {
          ...applicable,
          status: "not_applicable",
          reason: "target_sdk_above_maximum",
          maximumTargetSdk: 29,
          actualTargetSdk: 35,
        },
      },
    ],
    runtimeGrantCandidates: [{ permissionName: "android.permission.CAMERA", requiresRoot: false, androidApiMin: 23, androidApiMax: null, selected: false }],
    appOpCandidates: [{ permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE", operationName: "MANAGE_EXTERNAL_STORAGE", mode: "allow", requiresRoot: true, androidApiMin: 30, androidApiMax: null, selected: false }],
    warnings: [
      { code: "apk_permission_unknown", message: "Review this permission.", permissionName: "android.permission.UNKNOWN", applicabilityReason: null },
      { code: "apk_permission_not_applicable", message: "This permission is not applicable.", permissionName: "android.permission.WRITE_EXTERNAL_STORAGE", applicabilityReason: "target_sdk_above_maximum" },
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
      metadata: {},
    },
    apkInspection: null,
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
        permissionStepId: "grant_permissions_example",
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
    collisions: null,
  };
}

test("package fallback creates a readable app name and matching recipe text", () => {
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

test("blank asset pattern is valid and selects by artifact kind alone", () => {
  assert.equal(assetPatternError("", ["app.apk"]), null);
  assert.equal(assetPatternError("   ", ["app.apk"]), null);
  assert.match(
    assetPatternError("", ["app.apk", "app-x86_64.apk"]) ?? "",
    /multiple eligible APK/u,
  );
  assert.match(assetPatternError("", ["release-notes.txt"]) ?? "", /no eligible APK/u);
});

test("blank asset pattern previews kind-only selection for every eligible release", () => {
  const preview = buildReleasePatternPreview(remoteAnalysis(), "   ", false);
  assert.equal(preview.patternError, null);
  assert.equal(preview.eligibleReleaseCount, 3);
  assert.equal(preview.uniqueMatchCount, 2);
  assert.equal(preview.noMatchCount, 0);
  assert.equal(preview.multipleMatchesCount, 1);
  assert.equal(preview.blocking, false);
  assert.deepEqual(preview.releases[0]?.matchingAssetNames, ["app-v3-arm64.apk"]);
  assert.deepEqual(preview.releases[1]?.matchingAssetNames, ["other.apk"]);
  assert.deepEqual(preview.releases[2]?.matchingAssetNames, [
    "app-v1-arm64.apk",
    "app-v2-arm64.apk",
  ]);
});

test("blank asset pattern preview blocks a newest release without an eligible APK", () => {
  const analysis = remoteAnalysis();
  analysis.releases[1] = {
    ...analysis.releases[1]!,
    assets: [
      {
        ...analysis.releases[1]!.assets[0]!,
        assetHandle: "notes",
        fileName: "release-notes.txt",
      },
    ],
  };
  const preview = buildReleasePatternPreview(analysis, "", false);
  assert.equal(preview.blocking, true);
  assert.match(preview.blockingMessage ?? "", /v3.*no eligible APK/u);
  assert.equal(preview.releases[0]?.outcome, "no_match");
});

test("artifact rows rebuild canonical sources and omit blank optional values", () => {
  const row: AppArtifactRowEdits = {
    ...emptyArtifactRow("apk"),
    strategy: "latest_release",
    provider: "gitlab",
    baseUrl: " https://gitlab.com ",
    repository: " group/subgroup/project ",
    assetPattern: "   ",
    invertAssetPattern: true,
    prerelease: true,
  };
  assert.deepEqual(artifactSourceForRow(row), {
    strategy: "latest_release",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    repository: "group/subgroup/project",
    prerelease: true,
  });
  assert.deepEqual(
    artifactSourceForRow({
      ...row,
      assetPattern: String.raw`^app-v\d+-arm64\.apk$`,
      invertAssetPattern: false,
    }),
    {
      strategy: "latest_release",
      provider: "gitlab",
      base_url: "https://gitlab.com",
      repository: "group/subgroup/project",
      asset_pattern: String.raw`^app-v\d+-arm64\.apk$`,
      prerelease: true,
    },
  );
  assert.deepEqual(
    artifactSourceForRow({ ...row, assetPattern: String.raw`^app-v\d+-arm64\.apk$` }),
    {
      strategy: "latest_release",
      provider: "gitlab",
      base_url: "https://gitlab.com",
      repository: "group/subgroup/project",
      asset_pattern: String.raw`^app-v\d+-arm64\.apk$`,
      invert_asset_pattern: true,
      prerelease: true,
    },
  );
  assert.deepEqual(artifactSourceForRow(emptyArtifactRow("apk")), {
    strategy: "user_provided",
  });
});

test("direct URL rows normalize a trusted checksum and omit an absent one", () => {
  const row: AppArtifactRowEdits = {
    ...emptyArtifactRow("apk"),
    strategy: "direct_url",
    url: " https://example.invalid/app.apk ",
    sha256: " ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789 ",
  };
  assert.deepEqual(artifactSourceForRow(row), {
    strategy: "direct_url",
    url: "https://example.invalid/app.apk",
    sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
  });
  assert.deepEqual(artifactSourceForRow({ ...row, sha256: "   " }), {
    strategy: "direct_url",
    url: "https://example.invalid/app.apk",
  });
});

test("draft rows round-trip canonical artifacts targets and metadata", () => {
  const generated = draft();
  generated.app.artifacts = {
    apk: {
      kind: "apk",
      name: "Example APK",
      source: {
        strategy: "latest_release",
        provider: "forgejo",
        base_url: "https://codeberg.org",
        repository: "team/project",
        asset_pattern: String.raw`^app-v\d+-arm64\.apk$`,
        invert_asset_pattern: true,
        prerelease: true,
      },
    },
    data: {
      kind: "file",
      source: { strategy: "direct_url", url: "https://example.invalid/data.zip" },
    },
  };
  generated.app.targets = {
    storage: { kind: "directory", location: "shared_storage", path: "Example" },
  };
  generated.app.metadata = { homepage: "https://example.invalid" };

  const form = draftToForm(generated);
  assert.deepEqual(
    form.mappings.artifacts.map((artifactRow) => artifactRow.id),
    ["apk", "data"],
  );
  assert.equal(form.mappings.artifacts[0]?.strategy, "latest_release");
  assert.equal(form.mappings.artifacts[0]?.assetPattern, String.raw`^app-v\d+-arm64\.apk$`);
  assert.equal(form.mappings.artifacts[0]?.invertAssetPattern, true);
  assert.equal(form.mappings.artifacts[1]?.strategy, "direct_url");
  assert.deepEqual(form.mappings.targets, [
    { id: "storage", kind: "directory", location: "shared_storage", path: "Example" },
  ]);

  const request = formToRequest(form);
  assert.equal(request.ok, true);
  if (request.ok) {
    assert.deepEqual(request.mappings.artifacts, [
      {
        id: "apk",
        kind: "apk",
        name: "Example APK",
        source: {
          strategy: "latest_release",
          provider: "forgejo",
          base_url: "https://codeberg.org",
          repository: "team/project",
          asset_pattern: String.raw`^app-v\d+-arm64\.apk$`,
          invert_asset_pattern: true,
          prerelease: true,
        },
      },
      {
        id: "data",
        kind: "file",
        source: { strategy: "direct_url", url: "https://example.invalid/data.zip" },
      },
    ]);
    assert.deepEqual(request.mappings.targets, [
      { id: "storage", kind: "directory", location: "shared_storage", path: "Example" },
    ]);
    assert.equal(request.app.package_id, "com.example.app");
  }
});

test("release pattern preview counts every eligible release and keeps historical mismatches non-blocking", () => {
  const preview = buildReleasePatternPreview(
    remoteAnalysis(),
    String.raw`^app-v\d+-arm64\.apk$`,
    false,
  );
  assert.equal(preview.blocking, false);
  assert.equal(preview.eligibleReleaseCount, 3);
  assert.equal(preview.uniqueMatchCount, 1);
  assert.equal(preview.noMatchCount, 1);
  assert.equal(preview.multipleMatchesCount, 1);
  assert.deepEqual(preview.releases[2]?.matchingAssetNames, [
    "app-v1-arm64.apk",
    "app-v2-arm64.apk",
  ]);
});

test("release pattern preview blocks invalid empty newest zero and newest multiple outcomes", () => {
  const invalid = buildReleasePatternPreview(remoteAnalysis(), "[", false);
  assert.equal(invalid.blocking, true);
  assert.match(invalid.patternError ?? "", /valid regular expression/u);

  const empty = buildReleasePatternPreview(
    { ...remoteAnalysis(), releases: [], assets: [] },
    String.raw`\.apk$`,
    false,
  );
  assert.equal(empty.blocking, true);
  assert.match(empty.blockingMessage ?? "", /No analyzed releases/u);

  const zero = buildReleasePatternPreview(remoteAnalysis(), String.raw`^missing\.apk$`, false);
  assert.equal(zero.blocking, true);
  assert.match(zero.blockingMessage ?? "", /v3.*no matching APK/u);

  const multipleAnalysis = remoteAnalysis();
  multipleAnalysis.releases[1]!.assets.push({
    ...multipleAnalysis.releases[1]!.assets[0]!,
    assetHandle: "current-two",
    fileName: "app-v3-x86.apk",
  });
  const multiple = buildReleasePatternPreview(
    multipleAnalysis,
    String.raw`^app-v3-.*\.apk$`,
    false,
  );
  assert.equal(multiple.blocking, true);
  assert.equal(multiple.releases[0]?.outcome, "multiple_matches");
});

test("release pattern preview applies prerelease filtering without reordering provider results", () => {
  const stableOnly = buildReleasePatternPreview(
    remoteAnalysis(),
    String.raw`^app-v\d+-arm64\.apk$`,
    false,
  );
  assert.equal(stableOnly.releases[0]?.releaseTag, "v3");
  const withPrereleases = buildReleasePatternPreview(
    remoteAnalysis(),
    String.raw`^app-v\d+-arm64\.apk$`,
    true,
  );
  assert.equal(withPrereleases.releases[0]?.releaseTag, "v4-beta");
  assert.equal(withPrereleases.eligibleReleaseCount, 4);
});

test("browser preview is advisory when JavaScript accepts Rust-incompatible lookahead", () => {
  const preview = buildReleasePatternPreview(
    remoteAnalysis(),
    String.raw`^(?=app-v3).*\.apk$`,
    false,
  );
  assert.equal(preview.patternError, null);
  assert.equal(preview.releases[0]?.outcome, "unique_match");
});

test("GitHub prerelease policy filters the picker while retaining full analysis", () => {
  const analysis = remoteAnalysis();
  const filtered = analysisForPrereleasePolicy(analysis, false);
  assert.equal(analysis.releases.length, 4);
  assert.equal(filtered.releases.length, 3);
  assert.equal(filtered.assets.some((asset) => asset.prerelease), false);
});

test("GitHub prerelease changes retain analysis and invalidate stale review state", () => {
  const analysis = remoteAnalysis();
  let state: AppGeneratorState = {
    ...initialAppGeneratorState,
    sourceMode: "github_repository",
    sourceAnalysis: analysis,
    selectedAssetHandle: "current",
    assetPattern: "pattern",
    apkHandle: "apk",
    apkLabel: "app.apk",
    inspection: inspection(),
    draft: draft(),
    form: draftToForm(draft()),
    collisions: { collisions: [], blocking: false },
  };
  state = reduceAppGenerator(state, { type: "include-prereleases", value: true });
  assert.equal(state.sourceAnalysis, analysis);
  assert.equal(state.selectedAssetHandle, "current");
  assert.equal(state.form !== null, true);
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);

  state = { ...state, selectedAssetHandle: "prerelease" };
  state = reduceAppGenerator(state, { type: "include-prereleases", value: false });
  assert.equal(state.sourceAnalysis, analysis);
  assert.equal(state.selectedAssetHandle, null);
  assert.equal(state.apkHandle, null);
  assert.equal(state.form, null);
});

test("pattern edits retain analyzed releases and editable form while invalidating review", () => {
  const analysis = remoteAnalysis();
  const generated = draft();
  const form = draftToForm(generated);
  const state = reduceAppGenerator(
    {
      ...initialAppGeneratorState,
      sourceAnalysis: analysis,
      draft: generated,
      form,
      collisions: { collisions: [], blocking: false },
    },
    { type: "asset-pattern", value: "new-pattern" },
  );
  assert.equal(state.sourceAnalysis, analysis);
  assert.equal(state.form, form);
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);
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
    [
      "android.permission.UNKNOWN",
      "android.permission.OLD",
      "android.permission.MAYBE",
      "android.permission.WRITE_EXTERNAL_STORAGE",
    ],
  );
});

test("permission-only warnings omit the inspection warning section", () => {
  assert.deepEqual(globalInspectionWarnings(inspection()), []);
});

test("mixed inspection warnings retain only global warnings", () => {
  const reviewed = inspection();
  reviewed.warnings.push({
    code: "apk_permission_applicability_indeterminate",
    message: "Target SDK context was unavailable.",
    permissionName: null,
    applicabilityReason: null,
  });
  assert.deepEqual(globalInspectionWarnings(reviewed), [reviewed.warnings[2]]);
});

test("unknown and maximum-target permissions remain visible exactly once", () => {
  const names = otherRequestedPermissions(inspection()).map((permission) => permission.name);
  assert.equal(names.filter((name) => name === "android.permission.UNKNOWN").length, 1);
  assert.equal(
    names.filter((name) => name === "android.permission.WRITE_EXTERNAL_STORAGE").length,
    1,
  );
});

test("maximum-target applicability uses exact structured display text", () => {
  const write = inspection().permissions.find(
    (permission) => permission.name === "android.permission.WRITE_EXTERNAL_STORAGE",
  );
  assert.equal(
    permissionApplicabilityLabel(write?.applicability ?? null),
    "not applicable — app target SDK 35 exceeds maximum supported target SDK 29",
  );
});

test("candidate selections invalidate reviewed work without clearing editable form state", () => {
  const generated = draft();
  const collisions = { collisions: [], blocking: false };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: generated });
  state = reduceAppGenerator(state, { type: "reviewed", draft: generated, collisions });
  state = reduceAppGenerator(state, { type: "inspected", inspection: inspection() });
  const form = state.form;
  state = reduceAppGenerator(state, { type: "runtime-candidate-selected", index: 0, selected: true });
  assert.equal(state.form, form);
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);
  state = reduceAppGenerator(state, { type: "app-op-candidate-selected", index: 0, selected: true });
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, true);
  assert.equal(state.inspection?.appOpCandidates[0]?.selected, true);
  assert.equal(state.form, form);
  assert.equal(state.draft, null);
  assert.equal(state.collisions, null);

  state = reduceAppGenerator(state, { type: "inspected", inspection: inspection() });
  assert.equal(state.inspection?.runtimeGrantCandidates[0]?.selected, false);
  assert.equal(state.inspection?.appOpCandidates[0]?.selected, false);
  state = reduceAppGenerator(state, { type: "apk-selected", apkHandle: "new-apk", label: "New APK" });
  assert.equal(state.inspection, null);
});

test("permission selection serialization includes identities only", () => {
  const reviewed = inspection();
  reviewed.runtimeGrantCandidates[0]!.selected = true;
  reviewed.appOpCandidates[0]!.selected = true;
  assert.deepEqual(permissionSelectionForInspection(reviewed), {
    inspectionHandle: "apk-inspection-1",
    runtimePermissions: [{ permissionName: "android.permission.CAMERA" }],
    appOps: [{
      permissionName: "android.permission.MANAGE_EXTERNAL_STORAGE",
      operationName: "MANAGE_EXTERNAL_STORAGE",
      mode: "allow",
    }],
  });
  assert.equal(permissionSelectionForInspection(inspection()), null);
  assert.equal(permissionSelectionForInspection(null), null);
});

test("permission automation eligibility is limited to package-enforced remote strategies", () => {
  assert.equal(permissionAutomationEligible("github_release", "pinned_remote_asset"), true);
  assert.equal(permissionAutomationEligible("github_repository", "latest_compatible_release"), true);
  assert.equal(permissionAutomationEligible("github_release", "user_provided_apk"), false);
  assert.equal(permissionAutomationEligible("local_apk", "pinned_remote_asset"), false);
});

test("install strategy transitions reset selected permission candidates", () => {
  const reviewed = inspection();
  reviewed.runtimeGrantCandidates[0]!.selected = true;
  reviewed.appOpCandidates[0]!.selected = true;
  const next = reduceAppGenerator(
    { ...initialAppGeneratorState, inspection: reviewed },
    { type: "install-strategy", strategy: "user_provided_apk" },
  );
  assert.equal(next.inspection?.runtimeGrantCandidates[0]?.selected, false);
  assert.equal(next.inspection?.appOpCandidates[0]?.selected, false);
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
  const result = formToRequest(form);
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.mappings.metadata, '{"z":1,"a":2}');
    assert.deepEqual(
      result.mappings.artifacts.map((artifact) => artifact.id),
      ["apk"],
    );
    assert.deepEqual(result.mappings.targets, []);
    assert.equal(result.app.package_id, "com.example.app");
  }
});

test("form state preserves an empty artifact row", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.artifacts = [emptyArtifactRow("")];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.mappings.artifacts, [emptyArtifactRow("")]);
});

test("form state preserves an empty App Target row", () => {
  const state = reduceAppGenerator(initialAppGeneratorState, {
    type: "drafted",
    draft: draft(),
  });
  const form = structuredClone(state.form!);
  form.mappings.targets = [emptyTargetRow("")];
  const next = reduceAppGenerator(state, { type: "form", form });
  assert.deepEqual(next.form?.mappings.targets, [emptyTargetRow("")]);
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

test("source step pattern edits keep the selected remote artifact row in step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      name: "Example APK",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^app-v.*-arm64\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v3/app-v3-arm64.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v3",
    assetName: "app-v3-arm64.apk",
    assetPattern: "^app-v.*-arm64\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: remoteDraft });
  state = { ...state, remoteSource };

  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^app-v3.*\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^app-v3.*\\.apk$");

  const customized = structuredClone(state.form!);
  customized.mappings.artifacts[0]!.assetPattern = "^mine\\.apk$";
  const next = reduceAppGenerator(
    { ...state, form: customized },
    { type: "asset-pattern", value: "^other\\.apk$" },
  );
  assert.equal(next.form?.mappings.artifacts[0]?.assetPattern, "^mine\\.apk$");
});

test("source step checksum edits keep the selected remote artifact row in step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: { strategy: "direct_url", url: "https://example.test/app.apk" },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_release",
    strategy: "pinned_remote_asset",
    downloadUrl: "https://example.test/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: null,
    includePrereleases: false,
  };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: remoteDraft });
  state = { ...state, remoteSource };

  const checksum = "A".repeat(64);
  state = reduceAppGenerator(state, { type: "trusted-sha256", value: checksum });
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, checksum);

  state = reduceAppGenerator(state, { type: "trusted-sha256", value: "" });
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "");
});

test("pre-release edits keep the selected remote artifact row in step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v3/app-v3-arm64.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v3",
    assetName: "app-v3-arm64.apk",
    assetPattern: null,
    includePrereleases: false,
  };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: remoteDraft });
  state = {
    ...state,
    remoteSource,
    sourceMode: "github_repository",
    sourceAnalysis: remoteAnalysis(),
    selectedAssetHandle: "current",
  };
  state = reduceAppGenerator(state, { type: "trusted-sha256", value: "B".repeat(64) });

  state = reduceAppGenerator(state, { type: "include-prereleases", value: true });
  assert.equal(state.form?.mappings.artifacts[0]?.prerelease, true);
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "");
});

test("drafting mirrors an existing artifact checksum into the source step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "direct_url",
        url: "https://example.test/app.apk",
        sha256: "A".repeat(64),
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_release",
    strategy: "pinned_remote_asset",
    downloadUrl: "https://example.test/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: null,
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // The draft persisted the publisher checksum, so the source step shows it
  // and the author can correct it without the row silently keeping the old
  // value.
  assert.equal(state.trustedSha256, "A".repeat(64));
  state = reduceAppGenerator(state, { type: "trusted-sha256", value: "C".repeat(64) });
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "C".repeat(64));
  state = reduceAppGenerator(state, { type: "trusted-sha256", value: "" });
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "");
});

test("artifact row checksum edits update the source step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "direct_url",
        url: "https://example.test/app.apk",
        sha256: "A".repeat(64),
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_release",
    strategy: "pinned_remote_asset",
    downloadUrl: "https://example.test/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: null,
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });
  const form = structuredClone(state.form!);
  form.mappings.artifacts[0]!.sha256 = "B".repeat(64);

  state = reduceAppGenerator(state, { type: "form", form });
  assert.equal(state.trustedSha256, "B".repeat(64));

  state = reduceAppGenerator(state, { type: "inspecting" });
  assert.equal(state.trustedSha256, "B".repeat(64));
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "B".repeat(64));
});

test("the drafted artifact receives source-step edits whatever origin it records", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "forgejo",
        base_url: "https://forgejo.first",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "forgejo_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://forgejo.second/example/project/releases/download/v1/app.apk",
    provider: "forgejo",
    baseUrl: "https://forgejo.second",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^second\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // Native generation pairs the artifact the generated draft proposes by kind
  // and source strategy alone, so a row that records another service origin
  // still receives the source-step policy the generated Recipe duplicates.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^edited\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^edited\\.apk$");
});


test("a retained artifact that matches the source does not capture the pairing", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/retargeted",
        asset_pattern: "^retargeted\\.apk$",
        prerelease: false,
      },
    },
    backup: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // The generated Recipe installs the artifact the draft proposes, so a
  // retained row that still matches the reviewed source must not take over the
  // source-step edits the author makes.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.id, "apk");
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^third\\.apk$");
  assert.equal(state.form?.mappings.artifacts[1]?.id, "backup");
  assert.equal(state.form?.mappings.artifacts[1]?.assetPattern, "^first\\.apk$");
});

test("the proposed artifact id decides the pairing in any row order", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    backup: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/retargeted",
        asset_pattern: "^retargeted\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // Native generation pairs its proposed artifact id rather than the first
  // returned artifact, so the author's renamed row keeps its own policy even
  // though it still records the reviewed source.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.id, "backup");
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^first\\.apk$");
  assert.equal(state.form?.mappings.artifacts[1]?.id, "apk");
  assert.equal(state.form?.mappings.artifacts[1]?.assetPattern, "^third\\.apk$");
});

test("a pinned artifact keeps the author's release-policy controls", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: { strategy: "direct_url", url: "https://example.com/app.apk" },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_release",
    strategy: "pinned_remote_asset",
    downloadUrl: "https://example.com/app.apk",
    provider: null,
    baseUrl: null,
    repository: null,
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: null,
    includePrereleases: true,
  };
  const state = reduceAppGenerator(
    {
      ...initialAppGeneratorState,
      remoteSource,
      includePrereleases: true,
      assetPattern: "^app\\.apk$",
    },
    { type: "drafted", draft: remoteDraft },
  );

  // A direct-url artifact carries no release policy, so drafting one must not
  // reset the source-step controls the author set for a pinned selection.
  assert.equal(state.form?.mappings.artifacts[0]?.id, "apk");
  assert.equal(state.includePrereleases, true);
  assert.equal(state.assetPattern, "^app\\.apk$");
});

test("a renamed artifact keeps the pairing while it stays the only candidate", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    main: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/retargeted",
        asset_pattern: "^retargeted\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // Native generation accepts the sole APK artifact with the expected source
  // strategy, so renaming and retargeting one still receives its policy edits.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.id, "main");
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^third\\.apk$");
});

test("artifact ids pair with the same trimming the payload uses", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    backup: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });
  const form = draftToForm(remoteDraft);
  form.mappings.artifacts[0]!.id = " apk ";
  state = reduceAppGenerator(state, { type: "form", form });

  // The submitted payload trims artifact ids, so the pairing trims them the
  // same way instead of falling back to an unresolved pairing.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^third\\.apk$");
  assert.equal(state.form?.mappings.artifacts[1]?.assetPattern, "^first\\.apk$");
});

test("an unrelated release artifact keeps the retained analysis current", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    notes: {
      kind: "file",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/other",
        asset_pattern: "^other\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });
  state = { ...state, sourceMode: "github_repository", sourceAnalysis: remoteAnalysis() };

  // Native generation validates the retained analysis against the paired
  // artifact, so an unrelated row cannot make a confirmed policy look stale.
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), true);
});
test("drafting mirrors the reviewed artifact release policy into the source step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^only\\.apk$",
        prerelease: true,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^stale\\.apk$",
    includePrereleases: false,
  };
  const state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // The reviewed artifact is authoritative, so the source step and the release
  // preview it feeds follow the row instead of the stale selection.
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^only\\.apk$");
  assert.equal(state.assetPattern, "^only\\.apk$");
  assert.equal(state.includePrereleases, true);
});

test("artifact row release-policy edits update the source step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    notes: {
      kind: "file",
      source: {
        strategy: "latest_release",
        provider: "forgejo",
        base_url: "https://forgejo.other",
        repository: "example/notes",
        asset_pattern: "^notes\\.txt$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  const form = structuredClone(state.form!);
  form.mappings.artifacts[0]!.assetPattern = "^second\\.apk$";
  form.mappings.artifacts[0]!.prerelease = true;
  state = reduceAppGenerator(state, { type: "form", form });
  assert.equal(state.assetPattern, "^second\\.apk$");
  assert.equal(state.includePrereleases, true);

  // A row that represents another service origin never drives the source step.
  const otherForm = structuredClone(state.form!);
  otherForm.mappings.artifacts[1]!.assetPattern = "^renamed\\.txt$";
  otherForm.mappings.artifacts[1]!.prerelease = false;
  state = reduceAppGenerator(state, { type: "form", form: otherForm });
  assert.equal(state.assetPattern, "^second\\.apk$");
  assert.equal(state.includePrereleases, true);
});

test("retargeting the reviewed artifact makes the retained release analysis stale", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^app\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^app\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator(
    {
      ...initialAppGeneratorState,
      sourceMode: "github_repository",
      sourceAnalysis: remoteAnalysis(),
      remoteSource,
    },
    { type: "drafted", draft: remoteDraft },
  );
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), true);

  // Another repository leaves the analyzed releases describing a source the
  // generated documents no longer resolve from.
  const retargetedRepository = structuredClone(state.form!);
  retargetedRepository.mappings.artifacts[0]!.repository = "example/other";
  state = reduceAppGenerator(state, { type: "form", form: retargetedRepository });
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), false);

  // Another provider cannot use the GitHub analysis either.
  const retargetedProvider = structuredClone(state.form!);
  retargetedProvider.mappings.artifacts[0]!.provider = "forgejo";
  retargetedProvider.mappings.artifacts[0]!.repository = "example/project";
  state = reduceAppGenerator(state, { type: "form", form: retargetedProvider });
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), false);

  // Returning to the analyzed source accepts the analysis again, and a
  // trailing slash on the service origin carries no meaning.
  const restored = structuredClone(state.form!);
  restored.mappings.artifacts[0]!.provider = "github";
  restored.mappings.artifacts[0]!.baseUrl = "https://github.com/";
  state = reduceAppGenerator(state, { type: "form", form: restored });
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), true);
});


test("service origin spelling differences still mirror the reviewed artifact policy", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com/",
        repository: "Example/Project",
        asset_pattern: "^only\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^stale\\.apk$",
    includePrereleases: false,
  };
  const state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // Letter case and a trailing slash describe the same service origin and
  // repository, so the reviewed row stays authoritative for the source step.
  assert.equal(state.assetPattern, "^only\\.apk$");
});

test("the generated artifact stays paired when another artifact shares the source", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    backup: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // Native generation pairs the artifact the generated draft proposes even when
  // another artifact shares the source, so the source-step edit rewrites that
  // artifact and leaves the independent authored policy alone.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.assetPattern, "^third\\.apk$");
  assert.equal(state.form?.mappings.artifacts[0]?.id, "apk");
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^third\\.apk$");
  assert.equal(state.form?.mappings.artifacts[1]?.id, "backup");
  assert.equal(state.form?.mappings.artifacts[1]?.assetPattern, "^first\\.apk$");
});

test("pre-release toggling keeps a retained artifact paired with the source step", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
    backup: {
      kind: "apk",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^first\\.apk$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^first\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });
  state = {
    ...state,
    sourceMode: "github_repository",
    sourceAnalysis: remoteAnalysis(),
    selectedAssetHandle: "current",
  };

  // Widening the pre-release policy keeps the selected asset, so the recorded
  // pairing survives and later source-step edits still rewrite that artifact.
  state = reduceAppGenerator(state, { type: "include-prereleases", value: true });
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^third\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.id, "apk");
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^third\\.apk$");
  assert.equal(state.form?.mappings.artifacts[1]?.id, "backup");
  assert.equal(state.form?.mappings.artifacts[1]?.assetPattern, "^first\\.apk$");
});

test("re-downloading an artifact clears the previous publisher checksum", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    apk: {
      kind: "apk",
      source: {
        strategy: "direct_url",
        url: "https://example.com/app-a.apk",
        sha256: "B".repeat(64),
      },
    },
  };
  let state = reduceAppGenerator(initialAppGeneratorState, { type: "drafted", draft: remoteDraft });
  state = reduceAppGenerator(state, { type: "trusted-sha256", value: "B".repeat(64) });
  assert.equal(state.form?.mappings.artifacts[0]?.sha256, "B".repeat(64));

  // The freshly downloaded artifact belongs to a new source selection, so the
  // previous publisher checksum must not be carried into its draft.
  state = reduceAppGenerator(state, {
    type: "remote-downloaded",
    apkHandle: "apk-2",
    label: "app-b.apk",
    source: {
      mode: "github_release",
      strategy: "pinned_remote_asset",
      downloadUrl: "https://example.com/app-b.apk",
      provider: null,
      baseUrl: null,
      repository: null,
      releaseTag: null,
      assetName: "app-b.apk",
      assetPattern: null,
      includePrereleases: false,
    },
  });
  assert.equal(state.trustedSha256, "");
  assert.equal(state.form, null);
  assert.equal(mirroredTrustedSha256(state, state.form), "");
});
test("a file artifact never represents the reviewed remote source", () => {
  const remoteDraft = draft();
  remoteDraft.app.artifacts = {
    notes: {
      kind: "file",
      source: {
        strategy: "latest_release",
        provider: "github",
        base_url: "https://github.com",
        repository: "example/project",
        asset_pattern: "^notes\\.txt$",
        prerelease: false,
      },
    },
  };
  const remoteSource: RemoteSourceDescriptorDto = {
    mode: "github_repository",
    strategy: "latest_compatible_release",
    downloadUrl: "https://github.com/example/project/releases/download/v1/app.apk",
    provider: "github",
    baseUrl: "https://github.com",
    repository: "example/project",
    releaseTag: "v1",
    assetName: "app.apk",
    assetPattern: "^app\\.apk$",
    includePrereleases: false,
  };
  let state = reduceAppGenerator({ ...initialAppGeneratorState, remoteSource }, {
    type: "drafted",
    draft: remoteDraft,
  });

  // The generated recipe installs an APK, so a generic file artifact cannot
  // stand in for the reviewed source: the source step keeps its own value and
  // native generation reports the missing pairing instead.
  state = reduceAppGenerator(state, { type: "asset-pattern", value: "^app\\.apk$" });
  assert.equal(state.form?.mappings.artifacts[0]?.assetPattern, "^notes\\.txt$");
  assert.equal(state.assetPattern, "^app\\.apk$");
});

test("retained analysis confirms the release policy before the APK download", () => {
  const state = {
    ...initialAppGeneratorState,
    sourceMode: "github_repository" as const,
    installStrategy: "latest_compatible_release" as const,
    sourceAnalysis: remoteAnalysis(),
  };

  // No artifact row and no downloaded source descriptor exist yet, so the
  // analysis still describes the selection the author is choosing.
  assert.equal(releaseAnalysisDescribesReviewedPolicy(state), true);
});

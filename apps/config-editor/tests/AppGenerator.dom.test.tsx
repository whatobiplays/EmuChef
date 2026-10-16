import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { EditorApiResult } from "../src/api/editorApi";
import type {
  ApkInspectionResult,
  AppGeneratorPublicationOutcomeDto,
  AppGeneratorPublicationStatusResult,
  AppGeneratorReviewResult,
  AppRecipeDraftResult,
  ExistingAppSelectionResult,
  ExistingAppSummaryDto,
  RemoteSourceAnalysisResult,
} from "../src/api/types";

const mocks = vi.hoisted(() => ({
  beginAppGenerator: vi.fn(),
  chooseAppGeneratorApk: vi.fn(),
  chooseAppGeneratorAuthoredRoot: vi.fn(),
  setAppGeneratorAuthoredRoot: vi.fn(),
  analyzeAppGeneratorSource: vi.fn(),
  downloadAppGeneratorRemoteApk: vi.fn(),
  inspectAppGeneratorApk: vi.fn(),
  listAppGeneratorExistingApps: vi.fn(),
  selectAppGeneratorExistingApp: vi.fn(),
  reviewAppGeneratorDraft: vi.fn(),
  reviewRemoteAppGeneratorDraft: vi.fn(),
  saveAppGeneratorReview: vi.fn(),
  appGeneratorPublicationStatus: vi.fn(),
  generateAppRecipeDraft: vi.fn(),
  generateRemoteAppRecipeDraft: vi.fn(),
  cancelAppGenerator: vi.fn(),
}));

vi.mock("../src/api/editorApi", () => mocks);

import { AppGenerator } from "../src/components/AppGenerator";

const apiSuccess = <T,>(result: T) => ({ kind: "success" as const, result });

const UNKNOWN_PUBLICATION_MESSAGE =
  "Publication status unknown — the catalog may already contain these files. Do not repeat the save.";

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

function inspection(packageName = "com.example.app"): ApkInspectionResult {
  return {
    inspectionHandle: "apk-inspection-1",
    manifest: {
      packageName,
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

function review(
  overrides: {
    targetMode?: "new_app" | "existing_app";
    appArtifactId?: string;
    recipePath?: string;
    requestId?: number;
    blocking?: boolean;
    diagnostics?: AppGeneratorReviewResult["diagnostics"];
    artifact?: AppGeneratorReviewResult["artifact"];
    permissionAutomation?: AppGeneratorReviewResult["permissionAutomation"];
    changeImpact?: AppGeneratorReviewResult["changeImpact"];
  } = {},
): AppGeneratorReviewResult {
  const artifactId = overrides.appArtifactId ?? "apk";
  const recipePath = overrides.recipePath ?? "recipes/app.example.install.yaml";
  const targetMode = overrides.targetMode ?? "new_app";
  return {
    reviewHandle: "review-handle-1",
    requestId: overrides.requestId ?? 7,
    blocking: overrides.blocking ?? false,
    diagnostics: overrides.diagnostics ?? [],
    app: draft().app,
    appCanonicalYaml: "app: example",
    recipe: draft().recipe,
    recipeCanonicalYaml: "recipe: app.example.install",
    destinations: {
      app: { fileName: "example.yaml", relativePath: "apps/example.yaml" },
      recipe: { fileName: "app.example.install.yaml", relativePath: recipePath },
    },
    target: {
      mode: targetMode,
      appId: "example",
      appPath: targetMode === "existing_app" ? "apps/example.yaml" : null,
      packageId: "com.example.app",
      packageLocked: targetMode === "existing_app",
    },
    artifact:
      overrides.artifact === undefined
        ? {
            appArtifactId: artifactId,
            name: "Example APK",
            kind: "apk",
            source: { strategy: "user_provided" },
            change: targetMode === "existing_app" ? "added" : "proposed",
          }
        : overrides.artifact,
    permissionAutomation: overrides.permissionAutomation ?? {
      enabled: false,
      baseline: { actions: [] },
      elevated: { actions: [], rootRequired: true, optional: true },
    },
    changeImpact: overrides.changeImpact ?? {
      addedArtifactIds: [artifactId],
      preservedArtifactIds: [],
      addedPermissionActions: { baseline: [], elevated: [] },
      affectedRecipes: [],
      warnings: [],
    },
    apkInspection: null,
  };
}

function savedOutcome(
  overrides: {
    documentId?: string | null;
    openError?: { code: string; message: string } | null;
  } = {},
): AppGeneratorPublicationOutcomeDto {
  const documentId = overrides.documentId === undefined ? "document-1" : overrides.documentId;
  return {
    outcome: "saved",
    operationId: "operation-saved",
    app: { appPath: "apps/example.yaml" },
    recipe: {
      recipePath: "recipes/app.example.install.yaml",
      documentId,
      ...(overrides.openError ? { openError: overrides.openError } : {}),
    },
    createdFiles: ["apps/example.yaml", "recipes/app.example.install.yaml"],
    replacedFiles: [],
  };
}

function unknownOutcome(): AppGeneratorPublicationOutcomeDto {
  return {
    outcome: "unknown",
    operationId: "operation-unknown",
    message: "The sidecar response was lost after submission.",
  };
}

function publicationStatus(
  outcome: "saved" | "failed" | "unknown",
  detail: Record<string, unknown> | null = null,
): AppGeneratorPublicationStatusResult {
  return {
    operationId: "operation-unknown",
    outcome,
    confirmed: outcome !== "unknown",
    recordedAt: null,
    detail,
  };
}

function remoteAsset(
  assetHandle: string,
  fileName: string,
  releaseTag: string | null,
  prerelease: boolean,
) {
  return {
    assetHandle,
    fileName,
    size: 1048576,
    contentType: "application/vnd.android.package-archive",
    releaseTag,
    releaseName: null,
    publishedAt: null,
    prerelease,
  };
}

function directAnalysis(): RemoteSourceAnalysisResult {
  const asset = remoteAsset("asset-1", "app.apk", null, false);
  return {
    sourceHandle: "source-1",
    mode: "direct_apk",
    normalizedUrl: "https://cdn.example.com/app.apk",
    capabilities: {
      pinnedArtifact: true,
      latestRelease: false,
      prereleaseFiltering: false,
      deterministicAssetFiltering: false,
    },
    repository: null,
    releases: [],
    assets: [asset],
    preselectedAssetHandle: "asset-1",
  };
}

function releaseAnalysis(): RemoteSourceAnalysisResult {
  const current = remoteAsset("asset-current", "app-v3-arm64.apk", "v3", false);
  const prerelease = remoteAsset("asset-prerelease", "app-v4-arm64.apk", "v4-beta", true);
  return {
    sourceHandle: "source-2",
    mode: "github_release",
    normalizedUrl: "https://github.com/example/project/releases/tag/v3",
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
    ],
    assets: [prerelease, current],
    preselectedAssetHandle: null,
  };
}

function existingSummary(overrides: Partial<ExistingAppSummaryDto> = {}): ExistingAppSummaryDto {
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

function renderGenerator(options: {
  initialAuthoredRoot?: string | null;
  onAuthoredRootSelected?: (path: string) => void | Promise<void>;
  onClose?: () => void;
  onSaved?: (result: AppGeneratorPublicationOutcomeDto) => void;
} = {}) {
  const onSaved = options.onSaved ?? vi.fn();
  const onAuthoredRootSelected = options.onAuthoredRootSelected ?? vi.fn();
  const onClose = options.onClose ?? vi.fn();
  render(
    <AppGenerator
      initialAuthoredRoot={options.initialAuthoredRoot ?? null}
      onAuthoredRootSelected={onAuthoredRootSelected}
      onClose={onClose}
      onSaved={onSaved}
    />,
  );
  return { onSaved, onAuthoredRootSelected, onClose };
}

async function waitForStarted(): Promise<void> {
  await waitFor(() => {
    const button = screen.getByRole("button", { name: "Choose and inspect APK" }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
  });
}

async function selectLocalApk(user: ReturnType<typeof userEvent.setup>, packagedName = "com.example.app") {
  mocks.chooseAppGeneratorApk.mockResolvedValue(
    apiSuccess({ cancelled: false, apkHandle: "apk-1", label: "example.apk" }),
  );
  mocks.inspectAppGeneratorApk.mockResolvedValue(apiSuccess(inspection(packagedName)));
  mocks.generateAppRecipeDraft.mockResolvedValue(apiSuccess(draft()));
  await user.click(screen.getByRole("button", { name: "Choose and inspect APK" }));
  await screen.findByRole("textbox", { name: /App Artifact ID/ });
}

async function chooseAuthoredRoot(user: ReturnType<typeof userEvent.setup>) {
  mocks.chooseAppGeneratorAuthoredRoot.mockResolvedValue(
    apiSuccess({ cancelled: false, rootHandle: "root-1", label: "catalog", path: "/catalog" }),
  );
  await user.click(screen.getByRole("button", { name: "Choose authored root..." }));
  await screen.findByText("Selected and ready");
}

async function buildReviewedOutput(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Build reviewed output" }));
  await screen.findByRole("heading", { name: "Reviewed output" });
}

describe("AppGenerator authoring flows", () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) {
      mock.mockReset();
    }
    mocks.beginAppGenerator.mockResolvedValue(apiSuccess({ sessionHandle: "session-1" }));
    mocks.cancelAppGenerator.mockResolvedValue(apiSuccess({}));
  });

  it("carries explicit artifact and Recipe ids through review and save for a local APK", async () => {
    const user = userEvent.setup();
    const { onSaved } = renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);

    expect(mocks.inspectAppGeneratorApk).toHaveBeenCalledWith("session-1", "apk-1");
    expect(mocks.generateAppRecipeDraft).toHaveBeenCalledWith(
      "session-1",
      "apk-1",
      null,
      null,
      null,
      null,
      false,
      null,
      expect.objectContaining({ appArtifactId: "apk" }),
    );

    const artifactIdInput = screen.getByRole("textbox", { name: /App Artifact ID/ }) as HTMLInputElement;
    expect(artifactIdInput.value).toBe("apk");
    expect((screen.getByRole("textbox", { name: /Recipe ID/ }) as HTMLInputElement).value).toBe("app.example.apk.install");

    fireEvent.change(artifactIdInput, { target: { value: "bundle" } });
    expect((screen.getByRole("textbox", { name: /Recipe ID/ }) as HTMLInputElement).value).toBe("app.example.bundle.install");

    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockImplementation(
      (input: { context: { requestId: number } }) =>
        apiSuccess(
          review({
            requestId: input.context.requestId,
            appArtifactId: "bundle",
            recipePath: "recipes/app.example.bundle.install.yaml",
          }),
        ),
    );
    await buildReviewedOutput(user);

    expect(screen.getByText(/Creates App example/)).toBeTruthy();
    expect(
      screen.getByText((_content, element) => element?.textContent === "apps/example.yaml (create)"),
    ).toBeTruthy();
    const reviewRequest = mocks.reviewAppGeneratorDraft.mock.calls[0][0];
    expect(reviewRequest).toMatchObject({
      sessionHandle: "session-1",
      apkHandle: "apk-1",
      rootHandle: "root-1",
      permissionSelection: null,
      regenerateIdentifiers: false,
    });
    expect(reviewRequest.context).toMatchObject({
      appArtifactId: "bundle",
      recipeId: "app.example.bundle.install",
      permissionAutomationEnabled: false,
    });

    mocks.saveAppGeneratorReview.mockResolvedValue(apiSuccess(savedOutcome()));
    await user.click(screen.getByRole("button", { name: "Save App and Recipe" }));
    await screen.findByRole("heading", { name: "Save completed" });

    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledWith(
      "session-1",
      "review-handle-1",
      reviewRequest.context.requestId,
    );
    expect(onSaved).toHaveBeenCalledWith(savedOutcome());
  });

  it("seeds an initial authored root and reviews a direct HTTPS source as the direct_url strategy", async () => {
    mocks.setAppGeneratorAuthoredRoot.mockResolvedValue(
      apiSuccess({ cancelled: false, rootHandle: "root-1", label: "catalog" }),
    );
    const user = userEvent.setup();
    renderGenerator({ initialAuthoredRoot: "/catalog" });
    await waitForStarted();
    await waitFor(() =>
      expect(mocks.setAppGeneratorAuthoredRoot).toHaveBeenCalledWith("session-1", "/catalog"),
    );

    fireEvent.change(screen.getByRole("combobox", { name: "App source" }), { target: { value: "direct_apk" } });
    fireEvent.change(screen.getByRole("textbox", { name: /APK download URL/ }), {
      target: { value: "https://cdn.example.com/app.apk" },
    });
    mocks.analyzeAppGeneratorSource.mockResolvedValue(apiSuccess(directAnalysis()));
    await user.click(screen.getByRole("button", { name: "Check source" }));

    expect(mocks.analyzeAppGeneratorSource).toHaveBeenCalledWith(
      "session-1",
      "direct_apk",
      "https://cdn.example.com/app.apk",
      false,
    );
    expect(await screen.findByRole("heading", { name: "Available APK files" })).toBeTruthy();
    await user.click(screen.getByRole("radio", { name: /app\.apk/ }));

    mocks.downloadAppGeneratorRemoteApk.mockResolvedValue(
      apiSuccess({
        apkHandle: "remote-apk-1",
        label: "app.apk",
        source: {
          mode: "direct_apk",
          strategy: "pinned_remote_asset",
          downloadUrl: "https://cdn.example.com/app.apk",
          provider: null,
          baseUrl: null,
          repository: null,
          releaseTag: null,
          assetName: "app.apk",
          assetPattern: null,
          includePrereleases: false,
        },
      }),
    );
    mocks.inspectAppGeneratorApk.mockResolvedValue(apiSuccess(inspection()));
    mocks.generateRemoteAppRecipeDraft.mockResolvedValue(apiSuccess(draft()));

    await user.click(screen.getByRole("button", { name: "Download and inspect APK" }));
    await screen.findByRole("textbox", { name: /App Artifact ID/ });
    expect(await screen.findByText("Selected and ready")).toBeTruthy();

    expect(mocks.downloadAppGeneratorRemoteApk).toHaveBeenCalledWith("session-1", "asset-1");
    expect(mocks.generateRemoteAppRecipeDraft).toHaveBeenCalledWith(
      "session-1",
      "remote-apk-1",
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
      "root-1",
      expect.objectContaining({ appArtifactId: "apk" }),
    );

    mocks.reviewRemoteAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);
    expect(mocks.reviewRemoteAppGeneratorDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionHandle: "session-1",
        apkHandle: "remote-apk-1",
        rootHandle: "root-1",
        assetHandle: "asset-1",
        strategy: "pinned_remote_asset",
        assetPattern: null,
        includePrereleases: false,
        trustedSha256: null,
      }),
    );
  });

  it("reviews a latest-compatible GitHub release with the authored asset pattern", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    fireEvent.change(screen.getByRole("combobox", { name: "App source" }), { target: { value: "github_release" } });
    fireEvent.change(screen.getByRole("textbox", { name: /Source URL/ }), {
      target: { value: "https://github.com/example/project/releases/tag/v3" },
    });
    mocks.analyzeAppGeneratorSource.mockResolvedValue(apiSuccess(releaseAnalysis()));
    await user.click(screen.getByRole("button", { name: "Check source" }));
    await user.click(screen.getByRole("radio", { name: /app-v3-arm64\.apk/ }));

    fireEvent.change(screen.getByRole("combobox", { name: /APK resolution/ }), {
      target: { value: "latest_compatible_release" },
    });
    const patternInput = screen.getByRole("textbox", { name: /APK filename pattern/ });
    fireEvent.change(patternInput, { target: { value: "^app-v3.*\\.apk$" } });

    mocks.downloadAppGeneratorRemoteApk.mockResolvedValue(
      apiSuccess({
        apkHandle: "remote-apk-1",
        label: "app-v3-arm64.apk",
        source: {
          mode: "github_release",
          strategy: "latest_compatible_release",
          downloadUrl: "https://github.com/example/project/releases/download/v3/app-v3-arm64.apk",
          provider: "github",
          baseUrl: null,
          repository: "example/project",
          releaseTag: "v3",
          assetName: "app-v3-arm64.apk",
          assetPattern: null,
          includePrereleases: false,
        },
      }),
    );
    mocks.inspectAppGeneratorApk.mockResolvedValue(apiSuccess(inspection()));
    mocks.generateRemoteAppRecipeDraft.mockResolvedValue(apiSuccess(draft()));

    await user.click(screen.getByRole("button", { name: "Download and inspect APK" }));
    await screen.findByRole("textbox", { name: /App Artifact ID/ });

    expect(mocks.generateRemoteAppRecipeDraft).toHaveBeenCalledWith(
      "session-1",
      "remote-apk-1",
      "asset-current",
      "latest_compatible_release",
      "^app-v3.*\\.apk$",
      false,
      null,
      null,
      null,
      null,
      null,
      false,
      null,
      expect.objectContaining({ appArtifactId: "apk" }),
    );

    await chooseAuthoredRoot(user);
    mocks.reviewRemoteAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    expect(mocks.reviewRemoteAppGeneratorDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        strategy: "latest_compatible_release",
        assetPattern: "^app-v3.*\\.apk$",
        includePrereleases: false,
      }),
    );
  });

  it("confirms prerelease assets and keeps a trusted publisher checksum for a pinned release", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    fireEvent.change(screen.getByRole("combobox", { name: "App source" }), { target: { value: "github_release" } });
    fireEvent.change(screen.getByRole("textbox", { name: /Source URL/ }), {
      target: { value: "https://github.com/example/project/releases/tag/v4-beta" },
    });
    mocks.analyzeAppGeneratorSource.mockResolvedValue(apiSuccess(releaseAnalysis()));
    await user.click(screen.getByRole("button", { name: "Check source" }));
    await user.click(screen.getByRole("radio", { name: /app-v4-arm64\.apk/ }));

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    await user.click(screen.getByRole("button", { name: "Download and inspect APK" }));
    expect(confirmSpy).toHaveBeenCalledWith("This release is marked as prerelease. Continue with this APK?");
    expect(mocks.downloadAppGeneratorRemoteApk).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    mocks.downloadAppGeneratorRemoteApk.mockResolvedValue(
      apiSuccess({
        apkHandle: "remote-apk-1",
        label: "app-v4-arm64.apk",
        source: {
          mode: "github_release",
          strategy: "pinned_remote_asset",
          downloadUrl: "https://github.com/example/project/releases/download/v4-beta/app-v4-arm64.apk",
          provider: "github",
          baseUrl: null,
          repository: "example/project",
          releaseTag: "v4-beta",
          assetName: "app-v4-arm64.apk",
          assetPattern: null,
          includePrereleases: false,
        },
      }),
    );
    mocks.inspectAppGeneratorApk.mockResolvedValue(apiSuccess(inspection()));
    mocks.generateRemoteAppRecipeDraft.mockResolvedValue(apiSuccess(draft()));
    await user.click(screen.getByRole("button", { name: "Download and inspect APK" }));
    await screen.findByRole("textbox", { name: /App Artifact ID/ });

    const checksumInput = screen.getByRole("textbox", { name: /Trusted publisher SHA-256/ });
    fireEvent.change(checksumInput, { target: { value: "A".repeat(64) } });

    await chooseAuthoredRoot(user);
    mocks.reviewRemoteAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    expect(mocks.reviewRemoteAppGeneratorDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        strategy: "pinned_remote_asset",
        trustedSha256: "A".repeat(64),
      }),
    );
  });

  it("partitions permission candidates and sends only the selected identities on opt-in", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);

    expect(screen.getByRole("heading", { name: "Permission review" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Runtime grants — baseline (no root)" })).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: "Runtime grants — elevated (root required, optional)" }),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "App-ops — baseline (no root)" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "App-ops — elevated (root required, optional)" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Other permissions" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Inspection context and warnings" })).toBeTruthy();
    expect(screen.getByText("Review this permission.")).toBeTruthy();
    expect(screen.getByText("This inspection is limited to one declared permission set.")).toBeTruthy();
    expect(screen.getByText(/These values were extracted natively from the selected APK manifest/)).toBeTruthy();
    expect(screen.getByText("ABCD")).toBeTruthy();

    await user.click(screen.getByRole("checkbox", { name: /android\.permission\.CAMERA/ }));
    await user.click(
      screen.getByRole("checkbox", { name: /android\.permission\.MANAGE_EXTERNAL_STORAGE/ }),
    );
    await user.click(
      screen.getByRole("checkbox", {
        name: "Generate permission automation for this App",
      }),
    );

    await chooseAuthoredRoot(user);
    const cameraAction = { kind: "runtime_permission" as const, permission: "android.permission.CAMERA" };
    const appOpAction = {
      kind: "app_op" as const,
      permission: "android.permission.MANAGE_EXTERNAL_STORAGE",
      op: "MANAGE_EXTERNAL_STORAGE",
      mode: "allow",
    };
    mocks.reviewAppGeneratorDraft.mockResolvedValue(
      apiSuccess(
        review({
          permissionAutomation: {
            enabled: true,
            baseline: { actions: [cameraAction] },
            elevated: { actions: [appOpAction], rootRequired: true, optional: true },
          },
          changeImpact: {
            addedArtifactIds: ["apk"],
            preservedArtifactIds: [],
            addedPermissionActions: { baseline: [cameraAction], elevated: [appOpAction] },
            affectedRecipes: [
              {
                recipeId: "app.retroarch.install",
                recipePath: "recipes/app.retroarch.install.yaml",
                status: "revalidated_ok",
                diagnostics: [],
              },
            ],
            warnings: [],
          },
        }),
      ),
    );
    await buildReviewedOutput(user);

    const reviewRequest = mocks.reviewAppGeneratorDraft.mock.calls[0][0];
    expect(reviewRequest.permissionSelection).toEqual({
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
    expect(reviewRequest.context.permissionAutomationEnabled).toBe(true);

    const automationPanel = screen.getByText("Permission automation").closest("section") as HTMLElement;
    expect(within(automationPanel).getByText("Baseline (no root)")).toBeTruthy();
    expect(within(automationPanel).getByText("Elevated (root-required, optional)")).toBeTruthy();
    expect(within(automationPanel).getByText("android.permission.CAMERA")).toBeTruthy();
    expect(
      within(automationPanel).getByText(
        "android.permission.MANAGE_EXTERNAL_STORAGE · op MANAGE_EXTERNAL_STORAGE · mode allow",
      ),
    ).toBeTruthy();
    expect(
      within(automationPanel).getByText(/Elevated actions require root, stay optional, and are never a launch dependency\./),
    ).toBeTruthy();
    expect(
      screen.queryByText("No permission actions are added to the App policy by this review."),
    ).toBeNull();
    expect(screen.getByText("app.retroarch.install (revalidated_ok)")).toBeTruthy();
    expect(screen.getByText("recipes/app.retroarch.install.yaml")).toBeTruthy();
  });

  it("holds one pending review and drops it when a relevant identity edit invalidates it", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    let resolveReview!: (value: EditorApiResult<AppGeneratorReviewResult>) => void;
    mocks.reviewAppGeneratorDraft.mockReturnValue(
      new Promise((resolve) => {
        resolveReview = resolve;
      }),
    );
    const build = screen.getByRole("button", { name: "Build reviewed output" }) as HTMLButtonElement;
    await user.click(build);
    await waitFor(() => expect(build.disabled).toBe(true));
    await user.click(build);
    expect(mocks.reviewAppGeneratorDraft).toHaveBeenCalledTimes(1);

    resolveReview(apiSuccess(review()));
    await screen.findByRole("heading", { name: "Reviewed output" });
    expect((screen.getByRole("button", { name: "Save App and Recipe" }) as HTMLButtonElement).disabled).toBe(false);

    fireEvent.change(screen.getByRole("textbox", { name: /App Artifact ID/ }), {
      target: { value: "bundle" },
    });
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Reviewed output" })).toBeNull(),
    );
    expect((screen.getByRole("button", { name: "Save App and Recipe" }) as HTMLButtonElement).disabled).toBe(true);
    expect(mocks.saveAppGeneratorReview).not.toHaveBeenCalled();
  });

  it("drops the reviewed output when the permission automation opt-in changes", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);
    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    await user.click(
      screen.getByRole("checkbox", { name: "Generate permission automation for this App" }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Reviewed output" })).toBeNull(),
    );
    expect(
      (screen.getByRole("button", { name: "Save App and Recipe" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(mocks.saveAppGeneratorReview).not.toHaveBeenCalled();
  });

  it("binds a save to the review request id and drops the review after an identity edit", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockImplementation(
      (input: { context: { requestId: number } }) =>
        apiSuccess(review({ requestId: input.context.requestId })),
    );
    await buildReviewedOutput(user);
    const firstReviewRequestId = mocks.reviewAppGeneratorDraft.mock.calls[0][0].context.requestId;

    fireEvent.change(screen.getByRole("textbox", { name: /App Artifact ID/ }), {
      target: { value: "bundle" },
    });
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Reviewed output" })).toBeNull(),
    );
    expect(
      (screen.getByRole("button", { name: "Save App and Recipe" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(mocks.saveAppGeneratorReview).not.toHaveBeenCalled();

    await buildReviewedOutput(user);
    const secondReviewRequestId = mocks.reviewAppGeneratorDraft.mock.calls[1][0].context.requestId;
    expect(secondReviewRequestId).toBeGreaterThan(firstReviewRequestId);

    mocks.saveAppGeneratorReview.mockResolvedValue(apiSuccess(savedOutcome()));
    await user.click(screen.getByRole("button", { name: "Save App and Recipe" }));
    await screen.findByRole("heading", { name: "Save completed" });

    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledWith(
      "session-1",
      "review-handle-1",
      secondReviewRequestId,
    );
  });

  it("renders a blocking review whose App Artifact ID resolved no artifact", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockResolvedValue(
      apiSuccess(
        review({
          artifact: null,
          blocking: true,
          diagnostics: [
            {
              severity: "error",
              code: "app_artifact_id_mismatch",
              message:
                "The App Definition declares the reviewed local APK artifact under a different App Artifact ID; set appArtifactId to the declared artifact ID.",
              field: "appArtifactId",
            },
          ],
        }),
      ),
    );
    await buildReviewedOutput(user);

    expect(
      screen.getByText(/Not resolved\. The reviewed App Artifact ID does not name an artifact/),
    ).toBeTruthy();
    expect(
      screen.getByText(/declares the reviewed local APK artifact under a different App Artifact ID/),
    ).toBeTruthy();
    expect(screen.getByText(/This review is blocking\./)).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Save App and Recipe" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(mocks.saveAppGeneratorReview).not.toHaveBeenCalled();
  });

  it("lists and locks an existing App and hard-blocks a mismatched inspected package", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    mocks.listAppGeneratorExistingApps.mockResolvedValue(
      apiSuccess({ apps: [existingSummary()], inventory: { state: "complete", diagnostics: [] } }),
    );
    await user.click(screen.getByRole("radio", { name: "Add a new artifact to an existing App" }));
    await waitFor(() =>
      expect(mocks.listAppGeneratorExistingApps).toHaveBeenCalledWith("session-1", null),
    );

    mocks.selectAppGeneratorExistingApp.mockResolvedValue(apiSuccess(existingSelection()));
    await user.click(await screen.findByRole("button", { name: /apps\/example\.yaml/ }));
    await waitFor(() =>
      expect(mocks.selectAppGeneratorExistingApp).toHaveBeenCalledWith("session-1", "apps/example.yaml"),
    );

    expect(await screen.findByText("Example (example)")).toBeTruthy();
    expect(screen.getByText("apps/example.yaml · package com.example.app")).toBeTruthy();

    await selectLocalApk(user, "com.other.app");

    expect(
      await screen.findByText(
        "The inspected APK package com.other.app does not match the selected App package com.example.app. Create a different App instead of updating this one.",
      ),
    ).toBeTruthy();
    await waitFor(() =>
      expect((screen.getByRole("button", { name: "Review" }) as HTMLButtonElement).disabled).toBe(true),
    );

    await user.click(screen.getByRole("button", { name: "Build reviewed output" }));
    expect(mocks.reviewAppGeneratorDraft).not.toHaveBeenCalled();
    expect(
      screen.getByText(
        "The inspected APK package com.other.app does not match the selected App package com.example.app. Create a different App instead of updating this one.",
      ),
    ).toBeTruthy();
  });

  it("reviews an existing App as one added artifact with the package id locked", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    mocks.listAppGeneratorExistingApps.mockResolvedValue(
      apiSuccess({ apps: [existingSummary()], inventory: { state: "complete", diagnostics: [] } }),
    );
    await user.click(screen.getByRole("radio", { name: "Add a new artifact to an existing App" }));
    mocks.selectAppGeneratorExistingApp.mockResolvedValue(apiSuccess(existingSelection()));
    await user.click(await screen.findByRole("button", { name: /apps\/example\.yaml/ }));
    await screen.findByText("Example (example)");

    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    await user.click(
      screen.getByRole("checkbox", { name: "Generate permission automation for this App" }),
    );

    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review({ targetMode: "existing_app" })));
    await buildReviewedOutput(user);

    expect(
      screen.getByText("Updates App example at apps/example.yaml. The package id is locked to com.example.app."),
    ).toBeTruthy();
    expect(
      screen.getByText(
        (_content, element) =>
          element?.tagName === "DD" && (element.textContent ?? "").includes("(apk, added)"),
      ),
    ).toBeTruthy();
    const reviewRequest = mocks.reviewAppGeneratorDraft.mock.calls[0][0];
    expect(reviewRequest.context).toMatchObject({
      existingAppHandle: "app-handle-1",
    });
    expect(reviewRequest.context.permissionAutomationEnabled).toBe(true);
    expect(reviewRequest.permissionSelection).toBeNull();
    // Existing-App generation preserves the selected App Definition, so the
    // review carries the Recipe edits but no App or mapping edits.
    expect(reviewRequest.app).toBeNull();
    expect(reviewRequest.mappings).toBeNull();
    expect(reviewRequest.recipe).not.toBeNull();
    expect((screen.getByRole("combobox", { name: /Category/ }) as HTMLInputElement).disabled).toBe(
      true,
    );
    expect((screen.getByRole("button", { name: "Add artifact" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect((screen.getByRole("textbox", { name: /Recipe name/ }) as HTMLInputElement).disabled).toBe(
      false,
    );
  });

  it("reviews an existing App from a remote source without App or mapping edits", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    mocks.listAppGeneratorExistingApps.mockResolvedValue(
      apiSuccess({ apps: [existingSummary()], inventory: { state: "complete", diagnostics: [] } }),
    );
    await user.click(screen.getByRole("radio", { name: "Add a new artifact to an existing App" }));
    mocks.selectAppGeneratorExistingApp.mockResolvedValue(apiSuccess(existingSelection()));
    await user.click(await screen.findByRole("button", { name: /apps\/example\.yaml/ }));
    await screen.findByText("Example (example)");

    fireEvent.change(screen.getByRole("combobox", { name: "App source" }), {
      target: { value: "direct_apk" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: /APK download URL/ }), {
      target: { value: "https://cdn.example.com/app.apk" },
    });
    mocks.analyzeAppGeneratorSource.mockResolvedValue(apiSuccess(directAnalysis()));
    await user.click(screen.getByRole("button", { name: "Check source" }));
    await screen.findByRole("heading", { name: "Available APK files" });
    await user.click(screen.getByRole("radio", { name: /app\.apk/ }));

    mocks.downloadAppGeneratorRemoteApk.mockResolvedValue(
      apiSuccess({
        apkHandle: "remote-apk-1",
        label: "app.apk",
        source: {
          mode: "direct_apk",
          strategy: "pinned_remote_asset",
          downloadUrl: "https://cdn.example.com/app.apk",
          provider: null,
          baseUrl: null,
          repository: null,
          releaseTag: null,
          assetName: "app.apk",
          assetPattern: null,
          includePrereleases: false,
        },
      }),
    );
    mocks.inspectAppGeneratorApk.mockResolvedValue(apiSuccess(inspection()));
    mocks.generateRemoteAppRecipeDraft.mockResolvedValue(apiSuccess(draft()));
    await user.click(screen.getByRole("button", { name: "Download and inspect APK" }));
    await screen.findByRole("textbox", { name: /App Artifact ID/ });
    await chooseAuthoredRoot(user);

    await user.click(
      screen.getByRole("checkbox", { name: "Generate permission automation for this App" }),
    );

    mocks.reviewRemoteAppGeneratorDraft.mockResolvedValue(
      apiSuccess(review({ targetMode: "existing_app" })),
    );
    await buildReviewedOutput(user);

    const payload = mocks.reviewRemoteAppGeneratorDraft.mock.calls[0][0];
    expect(payload.app).toBeNull();
    expect(payload.mappings).toBeNull();
    expect(payload.recipe).not.toBeNull();
    expect(payload.assetHandle).toBe("asset-1");
    expect(payload.context).toMatchObject({
      existingAppHandle: "app-handle-1",
      appArtifactId: "apk",
    });
    expect(payload.context.permissionAutomationEnabled).toBe(true);
    expect(payload.permissionSelection).toBeNull();
  });

  it("keeps existing App selection unavailable while the catalog inventory is incomplete", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();

    mocks.listAppGeneratorExistingApps.mockResolvedValue(
      apiSuccess({
        apps: [existingSummary({ selectable: false })],
        inventory: {
          state: "incomplete",
          diagnostics: [{ code: "unreadable_app", message: "apps/broken.yaml cannot be read.", file: "apps/broken.yaml" }],
        },
      }),
    );
    await user.click(screen.getByRole("radio", { name: "Add a new artifact to an existing App" }));

    expect(
      await screen.findByText(
        "The catalog could not be read completely, so existing App selection is unavailable until it is repaired.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("apps/broken.yaml: apps/broken.yaml cannot be read.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /apps\/example\.yaml/ })).toBeNull();
  });

  it("keeps an unknown publication uncertain until a status check resolves it and reuses the exact ids", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    mocks.saveAppGeneratorReview.mockResolvedValue(apiSuccess(unknownOutcome()));
    await user.click(screen.getByRole("button", { name: "Save App and Recipe" }));

    expect(await screen.findByRole("heading", { name: "Uncertain save" })).toBeTruthy();
    expect(screen.getByText(UNKNOWN_PUBLICATION_MESSAGE)).toBeTruthy();
    expect(screen.getByText("No automatic retry happens; review again keeps the same ids.")).toBeTruthy();
    expect(screen.getByText("Operation operation-unknown.")).toBeTruthy();
    expect(
      screen.getByText(
        "Review again keeps these exact ids so an earlier attempt blocks a duplicate publication even after a restart.",
      ),
    ).toBeTruthy();
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);

    mocks.appGeneratorPublicationStatus.mockResolvedValue(apiSuccess(publicationStatus("saved")));
    await user.click(screen.getByRole("button", { name: "Check status" }));
    expect(
      await screen.findByText(
        "A recorded save was confirmed for this operation; the catalog contains these files.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText("No automatic retry happens; review again keeps the same ids.")).toBeNull();

    const reviewAgainButtons = screen.getAllByRole("button", { name: "Review again" });
    expect(reviewAgainButtons.length).toBe(3);
    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await user.click(within(screen.getByRole("alert")).getByRole("button", { name: "Review again" }));
    await waitFor(() => expect(mocks.reviewAppGeneratorDraft).toHaveBeenCalledTimes(2));

    const first = mocks.reviewAppGeneratorDraft.mock.calls[0][0].context;
    const second = mocks.reviewAppGeneratorDraft.mock.calls[1][0].context;
    expect(second.appArtifactId).toBe(first.appArtifactId);
    expect(second.recipeId).toBe(first.recipeId);
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);
  });

  it("reports a recorded failed publication with its Recipe recovery requirement", async () => {
    const user = userEvent.setup();
    renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    mocks.saveAppGeneratorReview.mockResolvedValue(apiSuccess(unknownOutcome()));
    await user.click(screen.getByRole("button", { name: "Save App and Recipe" }));
    expect(await screen.findByRole("heading", { name: "Uncertain save" })).toBeTruthy();

    mocks.appGeneratorPublicationStatus.mockResolvedValue(
      apiSuccess(
        publicationStatus("failed", {
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
        }),
      ),
    );
    await user.click(screen.getByRole("button", { name: "Check status" }));

    expect(await screen.findByText("This operation is recorded as failed.")).toBeTruthy();
    expect(
      screen.getByText(
        "The Recipe file this attempt created needs manual recovery at recipes/app.example.install.yaml.",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText("Recorded failure: The Recipe file could not be committed completely."),
    ).toBeTruthy();
    expect(screen.queryByText(/did not commit the files/)).toBeNull();
    expect(screen.queryByText("No automatic retry happens; review again keeps the same ids.")).toBeNull();
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mocks.saveAppGeneratorReview).toHaveBeenCalledTimes(1);
  });

  it("reports a committed save whose Recipe document could not be opened automatically", async () => {
    const user = userEvent.setup();
    const outcome = savedOutcome({
      documentId: null,
      openError: { code: "document_open_failed", message: "Locked by another session" },
    });
    const { onSaved } = renderGenerator();
    await waitForStarted();
    await selectLocalApk(user);
    await chooseAuthoredRoot(user);

    mocks.reviewAppGeneratorDraft.mockResolvedValue(apiSuccess(review()));
    await buildReviewedOutput(user);

    mocks.saveAppGeneratorReview.mockResolvedValue(apiSuccess(outcome));
    await user.click(screen.getByRole("button", { name: "Save App and Recipe" }));

    expect(await screen.findByRole("heading", { name: "Save completed" })).toBeTruthy();
    expect(screen.getByText(/Wrote/)).toBeTruthy();
    expect(
      screen.getByText(
        (_content, element) =>
          element?.tagName === "P" &&
          element.textContent ===
            "The editor could not open the saved Recipe automatically: Locked by another session. Open recipes/app.example.install.yaml manually to review it.",
      ),
    ).toBeTruthy();
    expect(onSaved).toHaveBeenCalledWith(outcome);
  });
});

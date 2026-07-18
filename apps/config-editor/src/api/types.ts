export interface ApiError {
  code: string;
  message: string;
  details: Record<string, unknown>;
}

export type ApiEnvelope<T> =
  | {
      ok: true;
      result: T;
    }
  | {
      ok: false;
      error: ApiError;
      debug?: unknown;
    };

export interface DiagnosticDto {
  severity: string;
  code: string;
  message: string;
  file: string | null;
  objectKind: string | null;
  objectId: string | null;
  field: string | null;
}

export interface RefCandidateDto {
  ref: string;
  label: string;
  valueType: string | null;
  sourceKind: string;
  sourceId: string;
}

export interface RefIndexDto {
  inputRefs: string[];
  artifactRefs: string[];
  stepRefs: string[];
  stepOutputRefs: string[];
  allRefs: string[];
  candidates: RefCandidateDto[];
}

export interface CommandResultDto {
  changed: boolean;
}

export interface InputDto {
  id: string;
  recipeId: string;
  inputId: string;
  key: string;
  type: string;
  role: string;
  label: string;
  description: string;
  required: boolean;
  multiple: boolean;
  validation: {
    mustExist: boolean;
    allowedExtensions: string[];
    pathKind: string | null;
    allowedPrefixes: string[];
  };
  default: unknown;
  options: Array<{ value: unknown; label: string }>;
  sensitive: boolean;
  advanced: boolean;
  metadata: Record<string, unknown>;
}

export interface ArtifactDto {
  id: string;
  type: string;
  url: string;
  cache: string;
}

export interface StepConditionDto {
  type: string;
  params: Record<string, unknown>;
}

export interface StepDto {
  id: string;
  type: string;
  name: string;
  description: string;
  userToggleable: boolean;
  dependencies: string[];
  constraints: {
    capabilities: string[];
    conflictsWith: string[];
  };
  skipIf: StepConditionDto[];
  params: Record<string, unknown>;
  verify: StepConditionDto[];
}

export interface RecipeDto {
  schemaVersion: number;
  kind: string;
  id: string;
  name: string;
  description: string;
  recipeDependencies: string[];
  provides: {
    features: string[];
  };
  inputs: Record<string, InputDto>;
  artifacts: Record<string, ArtifactDto>;
  artifactGroups: Record<string, string[]>;
  steps: StepDto[];
}

export interface RecipeDocumentDto {
  documentId: string;
  path: string;
  authoredRoot: string | null;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  recipe: RecipeDto;
  yaml: string;
  diagnostics: DiagnosticDto[];
  refIndex: RefIndexDto;
}

export interface StepSpecDto {
  type: string;
  label: string;
  supported: boolean;
  primaryOutputName: string | null;
  outputs: Array<{
    name: string;
    valueType: string | null;
    primary: boolean;
  }>;
  paramOrder: string[];
  params: Record<
    string,
    {
      acceptedSources: string[];
      acceptedValueTypes: string[];
      required: boolean;
      enumValues: string[];
      shape?: StepParamShapeDto;
    }
  >;
  defaults: Record<string, unknown>;
}

export interface StepParamShapeFieldDto {
  kind: string;
  required: boolean;
  enumValues: string[];
  default?: unknown;
}

export interface StepParamShapeDto {
  kind: string;
  itemKind?: string;
  target?: string;
  ordered: boolean;
  unique: boolean;
  fields: Record<string, StepParamShapeFieldDto>;
}

export interface ListStepSpecsResult {
  stepSpecs: StepSpecDto[];
}

export interface SidecarStatusResult {
  running: boolean;
  pid: number | null;
  state?: string;
  compatible?: boolean | null;
  protocolVersion?: number | null;
  capabilities?: string[];
  lastError?: string | null;
  message?: string;
}

export interface SidecarPingResult {
  healthy: true;
}

export interface SidecarRestartResult {
  status: SidecarStatusResult;
  documentSessionsPreserved: false;
}

export interface OpenRecipeResult {
  document: RecipeDocumentDto;
}

export interface DocumentResult {
  document: RecipeDocumentDto;
}

export interface ApplyRecipeCommandResult {
  commandResult: CommandResultDto;
  document: RecipeDocumentDto;
}

export interface ValidateRecipePathResult {
  diagnostics: DiagnosticDto[];
}

export interface ValidateResult {
  diagnostics: DiagnosticDto[];
}

export interface EmitRecipeYamlFromPathResult {
  yaml: string;
}

export interface EmitYamlResult {
  yaml: string;
}

export interface GetRefIndexResult {
  refIndex: RefIndexDto;
}

export interface UserConfigurationDiagnosticDto {
  severity: string;
  code: string;
  message: string;
  key: string | null;
  provenance: string;
  details: Record<string, unknown>;
}

export interface UserConfigurationDto {
  schemaVersion: number;
  kind: "user_configuration";
  id: string;
  name: string;
  devicePlan: string;
  selectedRecipes: string[];
  bindings: Record<string, unknown>;
  extensions: Record<string, unknown>;
}

export interface UserConfigurationDocumentDto {
  documentId: string;
  path: string;
  authoredRoot: string | null;
  dirty: boolean;
  configuration: UserConfigurationDto;
  yaml: string;
  diagnostics: UserConfigurationDiagnosticDto[];
}

export interface UserConfigurationDocumentResult {
  document: UserConfigurationDocumentDto;
}

export interface UserConfigurationCommandResult extends UserConfigurationDocumentResult {
  commandResult: CommandResultDto;
}

export interface RuntimeConfigurationDiagnosticDto {
  severity: string;
  code: string;
  message: string;
  key: string | null;
  provenance: string | null;
  details: Record<string, unknown>;
}

export interface RuntimeConfigurationInputDto extends InputDto {
  value: unknown;
  valueSource: "explicit" | "user_configuration" | "device_plan" | "recipe_default" | null;
  diagnostics: RuntimeConfigurationDiagnosticDto[];
}

export interface ConfigurationDescriptionResult {
  devicePlan: string;
  selectedRecipes: string[];
  expandedRecipes: string[];
  inputs: RuntimeConfigurationInputDto[];
  diagnostics: RuntimeConfigurationDiagnosticDto[];
}

export interface PlanConfigurationResult {
  plan: Record<string, unknown> | null;
  resolvedInputs: Array<{
    key: string;
    recipeId: string;
    inputId: string;
    type: string;
    value: unknown;
    source: "explicit" | "user_configuration" | "device_plan" | "recipe_default" | null;
  }>;
  diagnostics: RuntimeConfigurationDiagnosticDto[];
}

export interface ConfigEditorAuthoredRootResult {
  authoredRoot: string | null;
}

export interface DeviceProfileGeneratorSessionResult {
  sessionHandle: string;
  rootHandle?: string | null;
  rootLabel?: string | null;
}

export interface DeviceProfileGeneratorDeviceDto {
  deviceHandle: string;
  state: string;
  model: string | null;
}

export interface DeviceProfileGeneratorDeviceListResult {
  state: string;
  devices: DeviceProfileGeneratorDeviceDto[];
}

export interface SafeDetectedDeviceFactsDto {
  manufacturer: string | null;
  brand: string | null;
  model: string | null;
  product: string | null;
  device: string | null;
  board: string | null;
  hardware: string | null;
  abis: string[];
  androidVersion: number | null;
  androidApiLevel: number | null;
}

export interface DeviceProfileProbeResult {
  facts: SafeDetectedDeviceFactsDto;
}

export interface DeviceProfileV1Dto {
  schema_version: 1;
  kind: "device_profile";
  id: string;
  name: string;
  description?: string;
  match: {
    manufacturer_contains: string[];
    brand_contains: string[];
    model_patterns: string[];
    android_version?: {
      min?: number;
      max?: number;
    };
  };
  capability_defaults: {
    adb_available: boolean;
    apk_install: boolean;
    shared_storage_write: boolean;
    app_launch: boolean;
    shell_command: boolean;
    package_remove_for_user: boolean;
    root_shell: boolean;
    app_data_write: boolean;
  };
  device_tags: string[];
  metadata: Record<string, unknown>;
}

export type DeviceProfileEvidenceState = "verified" | "derived" | "suggested" | "missing";

export interface DeviceProfileFieldEvidenceDto {
  field: string;
  state: DeviceProfileEvidenceState;
  source: string;
  editedFromProposal: boolean;
}

export interface DeviceProfileGenerationDiagnosticDto {
  severity: "error" | "warning";
  code: string;
  message: string;
  field: string;
}

export interface DeviceProfileDraftResult {
  profile: DeviceProfileV1Dto;
  canonicalYaml: string | null;
  evidence: DeviceProfileFieldEvidenceDto[];
  diagnostics: DeviceProfileGenerationDiagnosticDto[];
  destination: {
    fileName: string | null;
    relativePath: string | null;
  };
}

export interface DeviceProfileRootSelectionResult {
  cancelled: boolean;
  path?: string;
  rootHandle?: string;
  label?: string;
}

export interface DeviceProfileCollisionDto {
  severity: "blocking" | "warning";
  code: string;
  message: string;
  existingProfileId: string | null;
  fileName: string | null;
}

export interface DeviceProfileCollisionResult {
  collisions: DeviceProfileCollisionDto[];
  blocking: boolean;
}

export interface DeviceProfileSaveResult {
  fileName: string;
  displayPath: string;
}

export type AppGeneratorSourceMode =
  | "local_apk"
  | "github_repository"
  | "github_release"
  | "gitlab_repository"
  | "gitlab_release"
  | "forgejo_repository"
  | "forgejo_release"
  | "direct_apk";
export type AppGeneratorInstallStrategy =
  | "pinned_remote_asset"
  | "latest_compatible_release"
  | "user_provided_apk";

export interface RemoteAssetDto {
  assetHandle: string;
  fileName: string;
  size: number | null;
  contentType: string | null;
  releaseTag: string | null;
  releaseName: string | null;
  prerelease: boolean;
  publishedAt: string | null;
}

export interface RemoteReleaseDto {
  tag: string;
  name: string | null;
  prerelease: boolean;
  publishedAt: string | null;
  assets: RemoteAssetDto[];
}

export interface RemoteSourceAnalysisResult {
  sourceHandle: string;
  mode: AppGeneratorSourceMode;
  normalizedUrl: string;
  capabilities: {
    pinnedArtifact: boolean;
    latestRelease: boolean;
    prereleaseFiltering: boolean;
    deterministicAssetFiltering: boolean;
  };
  repository: {
    fullName: string;
    name: string | null;
    description: string | null;
    htmlUrl: string;
  } | null;
  releases: RemoteReleaseDto[];
  assets: RemoteAssetDto[];
  preselectedAssetHandle: string | null;
}

export interface RemoteSourceDescriptorDto {
  mode: AppGeneratorSourceMode;
  strategy: AppGeneratorInstallStrategy;
  downloadUrl: string;
  provider: "github" | "gitlab" | "forgejo" | null;
  baseUrl: string | null;
  repository: string | null;
  releaseTag: string | null;
  assetName: string | null;
  assetPattern: string | null;
  includePrereleases: boolean;
}

export interface RemoteApkDownloadResult {
  apkHandle: string;
  label: string;
  source: RemoteSourceDescriptorDto;
}

export interface AppGeneratorSessionResult {
  sessionHandle: string;
  rootHandle?: string | null;
  rootLabel?: string | null;
}

export interface AppGeneratorSelectionResult {
  cancelled: boolean;
  path?: string;
  apkHandle?: string;
  rootHandle?: string;
  label?: string;
}

export interface ApkManifestMetadataDto {
  packageName: string;
  versionCode: string | null;
  versionName: string | null;
  minSdkVersion: string | null;
  targetSdkVersion: string | null;
}

export type ApkPermissionDeclarationKind = "uses_permission" | "uses_permission_sdk_23";

export type ApkPermissionClassification =
  | "runtime_grantable"
  | "runtime_restricted"
  | "app_op_grantable"
  | "manual_special_access"
  | "install_time"
  | "signature_or_privileged"
  | "unknown";

export type ApkPermissionApplicabilityStatus =
  | "applicable"
  | "not_applicable"
  | "indeterminate";

export type ApkPermissionApplicabilityReason =
  | "declaration_requires_api_23"
  | "max_sdk_version_exceeded"
  | "permission_not_introduced"
  | "permission_replaced"
  | "target_sdk_below_minimum"
  | "invalid_max_sdk_version"
  | "target_sdk_unavailable"
  | "replacement_target_sdk_unavailable";

export interface ApkPermissionApplicabilityDto {
  status: ApkPermissionApplicabilityStatus;
  reason: ApkPermissionApplicabilityReason | null;
  maximumSdkVersion: number | null;
  introductionApi: number | null;
  minimumDeviceApi: number | null;
  minimumTargetSdk: number | null;
  targetSdkState: "missing" | "non_numeric" | null;
}

export interface ApkPermissionReviewDto {
  name: string;
  declarationKind: ApkPermissionDeclarationKind;
  maxSdkVersion: string | null;
  classification: ApkPermissionClassification | null;
  applicability: ApkPermissionApplicabilityDto | null;
}

export interface ApkRuntimeGrantCandidateDto {
  permissionName: string;
  requiresRoot: boolean;
  selected: boolean;
}

export interface ApkAppOpCandidateDto {
  permissionName: string;
  operationName: string;
  mode: "allow";
  requiresRoot: boolean;
  selected: boolean;
}

export interface ApkPermissionWarningDto {
  code:
    | "apk_permission_classification_context_unavailable"
    | "apk_permission_not_applicable"
    | "apk_permission_applicability_indeterminate"
    | "apk_permission_runtime_restricted"
    | "apk_permission_manual_special_access"
    | "apk_permission_signature_or_privileged"
    | "apk_permission_unknown";
  message: string;
  permissionName: string | null;
  applicabilityReason: ApkPermissionApplicabilityReason | null;
}

export interface AppGeneratorDiagnosticDto {
  severity: "error" | "warning";
  code: string;
  message: string;
  field: string;
}

export interface ApkInspectionResult {
  inspectionHandle: string;
  manifest: ApkManifestMetadataDto;
  permissions: ApkPermissionReviewDto[];
  runtimeGrantCandidates: ApkRuntimeGrantCandidateDto[];
  appOpCandidates: ApkAppOpCandidateDto[];
  warnings: ApkPermissionWarningDto[];
  calculatedSha256: string;
  checksumStatus: "not_compared";
  signatureVerification: "not_performed";
}

export interface AppDefinitionV1Dto {
  schema_version: 1;
  kind: "app_definition";
  id: string;
  name: string;
  description?: string;
  category: string;
  package: { primary: string; aliases: string[] };
  install_source: { type: string; resolver: string; options: Record<string, unknown> };
  tracking_source: { type: string; [key: string]: unknown };
  artifacts: {
    apk: { required: boolean };
    shared_storage_config: { supported: boolean };
    app_data_config: { supported: boolean };
    byo_apk: { required: boolean };
  };
  provisioning: {
    launch_once_recommended: boolean;
    shared_storage_paths: string[];
    app_data_paths: string[];
    config_targets: Array<Record<string, unknown>>;
  };
  inputs: Array<Record<string, unknown>>;
  metadata: Record<string, unknown>;
}

export interface GeneratedRecipeIdsDto {
  recipeId: string;
  inputId: string;
  featureId: string;
  installStepId: string;
  permissionStepId: string;
  launchStepId: string;
}

/** Identity-only permission choices submitted to the trusted native boundary. */
export interface PermissionSelectionRequestDto {
  inspectionHandle: string;
  runtimePermissions: Array<{ permissionName: string }>;
  appOps: Array<{
    permissionName: string;
    operationName: string;
    mode: "allow";
  }>;
}

export interface AppRecipeEditsDto {
  ids: GeneratedRecipeIdsDto | null;
  name: string;
  description: string;
  inputLabel: string;
  inputDescription: string;
  replaceExisting: boolean;
  launchEnabled: boolean;
  launcherActivity: string | null;
}

export interface AppMappingEditsDto {
  installSourceOptions: string;
  trackingSourceFields: string;
  metadata: string;
  inputs: string[];
  configTargets: string[];
}

export interface AppRecipeDraftResult {
  app: AppDefinitionV1Dto;
  recipe: RecipeDto;
  recipeEdits: AppRecipeEditsDto;
  appCanonicalYaml: string | null;
  recipeCanonicalYaml: string | null;
  appDestination: { fileName: string | null; relativePath: string | null };
  recipeDestination: { fileName: string | null; relativePath: string | null };
  evidence: DeviceProfileFieldEvidenceDto[];
  diagnostics: AppGeneratorDiagnosticDto[];
  blocking: boolean;
  collisions: AppRecipeCollisionResult | null;
}

export interface AppRecipeCollisionDto {
  severity: "blocking" | "warning";
  code: string;
  message: string;
  existingId: string | null;
  relativePath: string | null;
}

export interface AppRecipeCollisionResult {
  collisions: AppRecipeCollisionDto[];
  blocking: boolean;
}

export interface AppRecipeSaveResult {
  appFileName: string;
  recipeFileName: string;
  appRelativePath: string;
  recipeRelativePath: string;
  openedRecipe: OpenRecipeResult;
}

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

export interface DeviceProfileGeneratorSessionResult {
  sessionHandle: string;
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

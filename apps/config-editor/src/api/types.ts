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
  };
  default: unknown;
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

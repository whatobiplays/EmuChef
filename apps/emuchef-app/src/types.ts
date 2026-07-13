export type RuntimeStatus =
  | { status: "ready"; protocolVersion: number; catalogVersion: string | null }
  | { status: "starting" }
  | { status: "unsupported" | "failed"; error: ActionableError };

export interface ActionableError {
  code: string;
  message: string;
  actions?: Array<"import" | "replace" | "remove" | "retry">;
}

export interface AdbSetupStatus {
  status: "missing" | "ready" | "invalid" | "busy";
  version: string | null;
  warning: string | null;
  error: ActionableError | null;
  canImport: boolean;
  canReplace: boolean;
  canRemove: boolean;
}

export interface CatalogIdentity {
  sourceKind: "bundled";
  sourceId: string;
  version: string | null;
  contentDigest: { algorithm: "sha256"; value: string };
}

export interface CatalogRecipe {
  id: string;
  name: string;
  description: string | null;
}

export interface CatalogSummary {
  catalog: CatalogIdentity;
  recipes: CatalogRecipe[];
}

export interface DeviceSummary {
  deviceHandle: string;
  state: "available" | "unauthorized" | "offline";
  displayName: string;
  maskedSerial: string;
}

export interface DeviceFacts {
  deviceHandle: string;
  manufacturer: string | null;
  brand: string | null;
  model: string | null;
  androidVersion: number | null;
  androidApiLevel: number | null;
}

export interface PlanCandidate {
  planId: string;
  name: string;
  description: string | null;
  profileId: string;
  profileName: string;
  confidence?: "exact" | "high" | "low";
  reasons: string[];
  requiresExplicitChoice?: boolean;
}

export interface DeviceMatch {
  confidence: "exact" | "high" | "low" | "none";
  recommendedPlanId: string | null;
  requiresExplicitChoice: boolean;
  candidates: PlanCandidate[];
  safeGenericPlans: PlanCandidate[];
  blocked: boolean;
  blockReason: string | null;
}

export interface RecipeOption {
  id: string;
  name: string;
  description: string | null;
  selected: boolean;
  recommended: boolean;
  dependencyRequired: boolean;
  available: boolean;
  unavailableCapabilities: string[];
}

export interface InputDescriptor {
  key: string;
  recipeId: string;
  inputId: string;
  type: string;
  label: string;
  description: string | null;
  required: boolean;
  multiple?: boolean;
  options?: string[];
  pathKind?: "file" | "directory";
  acceptedExtensions?: string[];
  value: unknown;
  valueSource: "explicit" | "user_configuration" | "device_plan" | "recipe_default" | null;
  diagnostics: ValidationDiagnostic[];
}

export interface ValidationDiagnostic {
  key?: string | null;
  code: string;
  message: string;
  severity: string;
}

export interface ConfigurationDescription {
  devicePlan: string;
  selectedRecipes: string[];
  expandedRecipes: string[];
  recipeOptions: RecipeOption[];
  inputs: InputDescriptor[];
  diagnostics: ValidationDiagnostic[];
}

export interface ReviewGroup {
  recipeId: string;
  recipeName: string;
  recipeDescription: string | null;
  steps: Array<{
    name: string;
    note: string | null;
    elevated: boolean;
    kindLabel: string;
    requirements: string[];
    technicalId: string;
    technicalType: string;
  }>;
}

export interface ReviewInput {
  key: string;
  value: string;
  source: "explicit" | "user_configuration" | "device_plan" | "recipe_default" | null;
}

export interface ReviewSummary {
  reviewHandle: string;
  planDigest: string;
  target: {
    manufacturer: string | null;
    model: string | null;
    androidVersion: number | null;
    androidApiLevel: number | null;
  };
  groups: ReviewGroup[];
  selectedInputs: ReviewInput[];
  warnings: Array<{ code: string; message: string }>;
}

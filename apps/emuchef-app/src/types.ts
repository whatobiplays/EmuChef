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

export type PlatformToolsPickerResult =
  | { outcome: "cancelled" }
  | { outcome: "selected"; selectionHandle: string };

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
  selectionMode?: "blank";
}

export interface DeviceMatch {
  confidence: "exact" | "high" | "low" | "none";
  recommendedPlanId: string | null;
  requiresExplicitChoice: boolean;
  candidates: PlanCandidate[];
  safeGenericPlans: PlanCandidate[];
  blankSetupPlans?: PlanCandidate[];
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
  recipeDependencies?: string[];
  contentRequirements?: Array<"apk_file" | "bios_files" | "rom_content" | "network_download">;
  requiredCapabilities?: string[];
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
  sensitive: boolean;
  options?: string[];
  pathKind?: "file" | "directory";
  acceptedExtensions?: string[];
  presentationCategory?: string;
  presentationKind?: string;
  entries?: InputEntryDescriptor[];
  value: unknown;
  valueSource: "explicit" | "user_configuration" | "device_plan" | "recipe_default" | null;
  diagnostics: ValidationDiagnostic[];
}

export interface ValidationDiagnostic {
  key?: string | null;
  code: string;
  message: string;
  severity: string;
  entryIndex?: number | null;
}

export interface InputEntryDescriptor {
  index: number;
  displayName: string;
  displayPath: string;
  state: "valid" | "warning" | "error";
  diagnostics: ValidationDiagnostic[];
}

export interface ConfigurationDescription {
  devicePlan: string;
  selectedRecipes: string[];
  expandedRecipes: string[];
  recipeOptions: RecipeOption[];
  inputs: InputDescriptor[];
  diagnostics: ValidationDiagnostic[];
}

export interface ReviewSummary {
  reviewHandle: string;
  setup: {
    name: string;
    description?: string;
  };
  target: {
    label: string;
    manufacturer?: string;
    model?: string;
    androidVersion?: number;
    androidApiLevel?: number;
  };
  features: Array<{
    name: string;
    description?: string;
    automaticallyAdded: boolean;
    sections: Array<{
      kind:
        | "preparation"
        | "downloads"
        | "installs"
        | "copies"
        | "permissions"
        | "launches"
        | "device_changes";
      label: string;
      actions: Array<{
        title: string;
        description?: string;
        requirement: "required" | "conditional";
        deviceLocation?: string;
      }>;
    }>;
  }>;
  inputs: Array<{
    label: string;
    summary: string;
    required: boolean;
  }>;
  notices: Array<{
    severity: "warning" | "blocker";
    title: string;
    message: string;
  }>;
  work: {
    actionCount: number;
    knownWaitSeconds?: number;
  };
  canExecute: boolean;
}

export type ExecutionStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "succeeded_with_warnings"
  | "failed"
  | "cancelled";

export type RecipeExecutionStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "succeeded_with_warnings"
  | "failed"
  | "blocked"
  | "cancelled";

export type StepExecutionStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "skipped"
  | "failed"
  | "blocked"
  | "cancelled";

export interface ExecutionIssue {
  message: string;
  remediation: RemediationGuidance;
}

export type RemediationKind =
  | "reconnect_device"
  | "repair_platform_tools"
  | "review_inputs"
  | "generate_fresh_plan"
  | "view_report";

export interface RemediationGuidance {
  kind: RemediationKind;
  title: string;
  message: string;
}

export interface ExecutionCompletionSummary {
  classification: "in_progress" | "success" | "success_with_warnings" | "failed" | "cancelled";
  counts: {
    total: number;
    completed: number;
    skipped: number;
    blocked: number;
    failed: number;
    cancelled: number;
    pending: number;
  };
  warningCount: number;
  partialChangesPossible: boolean;
  features: Array<{
    name: string;
    status: RecipeExecutionStatus;
    counts: Partial<Record<"completed" | "skipped" | "blocked" | "failed" | "cancelled" | "pending", number>>;
  }>;
}

export interface LaunchAction {
  handle: string;
  label: string;
}

export interface ExecutionStep {
  name: string;
  note: string | null;
  status: StepExecutionStatus;
  message: string | null;
}

export interface ExecutionRecipe {
  name: string;
  description: string | null;
  status: RecipeExecutionStatus;
  steps: ExecutionStep[];
}

export interface ExecutionSnapshot {
  executionHandle: string;
  reviewHandle: string;
  simulated: true;
  verificationScope: "simulation_only";
  status: ExecutionStatus;
  startedAt: string | null;
  finishedAt: string | null;
  latestSequence: number;
  terminal: boolean;
  recipes: ExecutionRecipe[];
  warnings: ExecutionIssue[];
  errors: ExecutionIssue[];
  completion: ExecutionCompletionSummary;
  progress: {
    currentFeature: string | null;
    currentAction: string | null;
  };
}

export interface RealExecutionTarget {
  label: "Connected Android device";
  manufacturer?: string;
  model?: string;
  androidVersion?: string;
  androidApiLevel?: number;
}

export interface RealExecutionSnapshot {
  executionHandle: string;
  reviewHandle: string;
  simulated: false;
  verificationScope: "real_device";
  target: RealExecutionTarget;
  status: ExecutionStatus;
  startedAt: string | null;
  finishedAt: string | null;
  latestSequence: number;
  terminal: boolean;
  recipes: ExecutionRecipe[];
  warnings: ExecutionIssue[];
  errors: ExecutionIssue[];
  completion: ExecutionCompletionSummary;
  progress: {
    currentFeature: string | null;
    currentAction: string | null;
  };
  launchAction: LaunchAction | null;
}

export type AnyExecutionSnapshot = ExecutionSnapshot | RealExecutionSnapshot;

export interface RealExecutionAvailability {
  enabled: boolean;
}

export interface RealExecutionConfirmation {
  phrase: string;
  irreversibleChangesAcknowledged: boolean;
  noRollbackAcknowledged: boolean;
  keepDeviceConnectedAcknowledged: boolean;
}

export interface ExecutionEvent {
  sequence: number;
  timestamp: string;
  label: string;
  status: string | null;
  issue: ExecutionIssue | null;
}

export interface ExecutionEventBatch {
  executionHandle: string;
  events: ExecutionEvent[];
  latestSequence: number;
  terminal: boolean;
}

export interface ExecutionCancellation {
  executionHandle: string;
  accepted: boolean;
  status: ExecutionStatus;
}

export interface ReportExportResult {
  outcome: "saved" | "cancelled";
}

export interface LaunchResult {
  launched: true;
  message: string;
}

export type SavedConfigurationValidationState =
  | "valid"
  | "valid_with_warnings"
  | "requires_attention"
  | "cannot_use";

/** Safe React projection of one Tauri-owned sidecar document session. */
export interface SavedConfigurationDocument {
  outcome?: "opened" | "saved";
  configurationHandle: string;
  name: string;
  dirty: boolean;
  revision: number;
  devicePlan: string;
  selectedRecipes: string[];
  bindings: Record<string, unknown>;
  pendingSanitationCount?: number;
  schemaVersion?: number;
  compatibility?: {
    baselineState: "pending_first_v2_save" | "unchanged" | "materially_changed" | "repair_required" | "unavailable";
  };
  validation: {
    state: SavedConfigurationValidationState;
    diagnostics: ValidationDiagnostic[];
  };
}

export interface RecentConfiguration {
  recentHandle: string;
  name: string;
  fileLabel: string;
  lastOpenedEpochMs: number;
  availability: "available" | "missing";
  identityConflict: boolean;
}

export interface SavedConfigurationPreview {
  outcome: "previewed";
  previewHandle: string;
  name: string;
  fileLabel: string;
  schemaVersion: number;
  lastModifiedEpochMs: number | null;
  setupLabel: string;
  featureLabels: string[];
  savedInputCount: number;
  omittedInputCount: number;
  compatibility: {
    state:
      | "compatible"
      | "compatible_with_warnings"
      | "migrated_baseline_pending"
      | "materially_changed"
      | "repair_required";
    baselineState: string;
    requiresRepair: boolean;
    message: string;
  };
  comparison?: {
    state: "matches" | "differs" | "requires_repair" | "no_current_intent";
    message: string;
  };
  repairActions: Array<{
    repairHandle: string;
    kind: "remove_recipe" | "remove_binding" | "select_option" | "relink_input";
    label: string;
  }>;
}

export type SavedConfigurationPreviewResult =
  | { outcome: "cancelled" }
  | SavedConfigurationPreview;

export interface SavedConfigurationFileOperationResult {
  outcome: "saved" | "cancelled";
  name?: string;
  fileLabel?: string;
}

export type SavedConfigurationDialogResult =
  | { outcome: "cancelled" }
  | SavedConfigurationDocument;

export type SavedConfigurationMutation =
  | { kind: "device_plan"; value: string }
  | { kind: "selected_recipes"; value: string[] }
  | { kind: "binding"; key: string; value: unknown }
  | { kind: "remove_binding"; key: string };

export interface RecoveryDraftAvailable {
  state: "available";
  draftGeneration: number;
  displayName: string | null;
  savedAtEpochMs: number;
  sourceSavedConfiguration: boolean;
}

export type RecoveryDraftStatus =
  | { state: "none" }
  | { state: "invalid_removed"; reason: string }
  | RecoveryDraftAvailable;

export interface AppSessionStart {
  sessionGeneration: number;
  interruptedSession: boolean;
  recovery: RecoveryDraftStatus;
}

export interface RecoveryWriteAck {
  requestGeneration: number;
  draftGeneration: number;
  recordGeneration: number;
  omittedBindings: string[];
}

export interface RecoveryRestoreResult {
  requestGeneration: number;
  draftGeneration: number;
  displayName: string | null;
  sourceStatus: "available" | "missing" | "unsaved";
  document: SavedConfigurationDocument | null;
  intent: {
    dirty: boolean;
    devicePlan: string;
    selectedRecipes: string[];
    bindings: Record<string, unknown>;
    requiredReentryBindings: string[];
  };
}

export interface CacheEntry {
  cacheEntryHandle: string;
  category: "artifact" | "partial";
  artifactLabel: string;
  sourceKind: "file" | "http" | "https" | "unknown";
  integrityState: "complete" | "incomplete" | "unindexed" | "metadata_mismatch";
  sizeBytes: number;
  ageBucket: "under_1_day" | "1_to_7_days" | "8_to_30_days" | "over_30_days" | "unknown";
  inUse: boolean;
  removable: boolean;
}

export interface CacheInventory {
  generation: string;
  entries: CacheEntry[];
  summary: {
    entryCount: number;
    totalSizeBytes: number;
    inUseCount: number;
    unmanagedCount: number;
    unmanagedSizeBytes: number;
  };
}

export type CacheCleanupMode = "selected" | "unused" | "all_removable";

export interface CacheCleanupOutcome {
  entryHandle: string;
  outcome: "removed" | "skipped_in_use" | "already_missing" | "invalidated" | "failed";
  code: string;
  message: string;
}

export interface CacheCleanupResult {
  outcomes: CacheCleanupOutcome[];
  inventory: CacheInventory;
}

export interface SupportDiagnosticsExportResult {
  outcome: "saved" | "cancelled";
}

/** Display-only update state. URL, signature, key, path, and opener authority stay in Rust. */
export interface UpdateStatus {
  state: "unconfigured" | "idle" | "checking" | "up_to_date" | "update_available" | "failed";
  currentVersion: string;
  latestVersion: string | null;
  publishedAt: string | null;
  expiresAt: string | null;
  notes: string | null;
  dmgSizeBytes: number | null;
  dmgSha256: string | null;
  minimumMacosVersion: string | null;
  minimumMacosVersionIsInformational: true;
  canOpenDownload: boolean;
  message: string | null;
}

export interface UpdateInteractionSession {
  sessionId: string;
  generation: number;
}

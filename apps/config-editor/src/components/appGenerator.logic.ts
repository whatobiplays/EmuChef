import type {
  ApkInspectionResult,
  ApkPermissionApplicabilityDto,
  ApkPermissionReviewDto,
  ApkPermissionWarningDto,
  AppArtifactEditDto,
  AppArtifactKindDto,
  AppArtifactSourceDto,
  AppArtifactStrategyDto,
  AppArtifactV1Dto,
  AppDefinitionV1Dto,
  AppMappingEditsDto,
  AppRecipeCollisionResult,
  AppRecipeDraftResult,
  AppRecipeEditsDto,
  AppGeneratorDiagnosticDto,
  AppGeneratorExistingAppsResult,
  AppGeneratorInstallStrategy,
  AppGeneratorPublicationOutcomeDto,
  AppGeneratorPublicationStatusResult,
  AppGeneratorRequestContext,
  AppGeneratorReviewResult,
  AppGeneratorSourceMode,
  AppReleaseProviderDto,
  AppTargetEditDto,
  AppTargetKindDto,
  AppTargetLocationDto,
  AppTargetV1Dto,
  ExistingAppInventoryDto,
  ExistingAppSelectionResult,
  ExistingAppSummaryDto,
  PermissionSelectionRequestDto,
  ReleasePatternPreviewResult,
  RemoteAssetDto,
  RemoteSourceAnalysisResult,
  RemoteSourceDescriptorDto,
} from "../api/types.js";
import { parseMetadataObject } from "./deviceProfileGenerator.logic.js";

export type TrustedSha256Result =
  | { ok: true; value: string | null }
  | { ok: false; message: string };

function trimAsciiWhitespace(value: string): string {
  let start = 0;
  let end = value.length;
  while (start < end && isAsciiWhitespace(value.charCodeAt(start))) start += 1;
  while (end > start && isAsciiWhitespace(value.charCodeAt(end - 1))) end -= 1;
  return value.slice(start, end);
}

function isAsciiWhitespace(codePoint: number): boolean {
  return codePoint === 0x20 || (codePoint >= 0x09 && codePoint <= 0x0D);
}

/** Validate an optional publisher-provided checksum without accepting formatted digests. */
export function parseTrustedSha256(input: string): TrustedSha256Result {
  const trimmed = trimAsciiWhitespace(input);
  if (trimmed.length === 0) return { ok: true, value: null };
  if (!/^[0-9A-Fa-f]{64}$/u.test(trimmed)) {
    return {
      ok: false,
      message: "Trusted publisher SHA-256 must contain exactly 64 hexadecimal characters.",
    };
  }
  return { ok: true, value: trimmed.toUpperCase() };
}

/** Return declarations not represented by an applicable automation candidate section. */
export function otherRequestedPermissions(inspection: ApkInspectionResult): ApkPermissionReviewDto[] {
  return inspection.permissions.filter((permission) => {
    if (permission.applicability?.status !== "applicable") return true;
    return permission.classification !== "runtime_grantable"
      && permission.classification !== "app_op_grantable";
  });
}

/** Return only inspection-wide warnings; declaration-specific outcomes render with permissions. */
export function globalInspectionWarnings(
  inspection: ApkInspectionResult,
): ApkPermissionWarningDto[] {
  return inspection.warnings.filter((warning) => warning.permissionName === null);
}

/** Return declaration-specific warnings attached to one permission name. */
export function permissionWarnings(
  inspection: ApkInspectionResult,
  permissionName: string,
): ApkPermissionWarningDto[] {
  return inspection.warnings.filter((warning) => warning.permissionName === permissionName);
}

/** Format structured permission applicability without inferring from permission names. */
export function permissionApplicabilityLabel(
  applicability: ApkPermissionApplicabilityDto | null,
): string {
  if (applicability === null) return "applicability unavailable";
  if (
    applicability.status === "not_applicable"
    && applicability.reason === "target_sdk_above_maximum"
    && applicability.maximumTargetSdk !== null
    && applicability.actualTargetSdk !== null
  ) {
    return `not applicable — app target SDK ${applicability.actualTargetSdk} exceeds maximum supported target SDK ${applicability.maximumTargetSdk}`;
  }
  const details = permissionApplicabilityDetails(applicability);
  if (applicability.status === "applicable") {
    return details ? `applicable (${details})` : "applicable";
  }
  const reason = applicability.reason
    ? ` — ${permissionApplicabilityReasonLabel(applicability.reason)}`
    : "";
  const suffix = details ? `; ${details}` : "";
  return applicability.status === "not_applicable"
    ? `not applicable${reason}${suffix}`
    : `indeterminate${reason}${suffix}`;
}

/** Format one stable applicability reason for permission and global-warning text. */
export function permissionApplicabilityReasonLabel(
  reason: NonNullable<ApkPermissionApplicabilityDto["reason"]>,
): string {
  const labels: Record<NonNullable<ApkPermissionApplicabilityDto["reason"]>, string> = {
    max_sdk_version_exceeded: "the effective maximum is below the required minimum",
    target_sdk_above_maximum: "application target SDK exceeds the supported maximum",
    target_sdk_below_minimum: "application target SDK is below the required minimum",
    invalid_max_sdk_version: "maximum SDK could not be interpreted",
    target_sdk_unavailable: "target SDK context is unavailable",
    maximum_target_sdk_unavailable: "maximum target SDK context is unavailable",
  };
  return labels[reason];
}

function permissionApplicabilityDetails(
  applicability: ApkPermissionApplicabilityDto,
): string {
  const details: string[] = [];
  if (applicability.maximumSdkVersion !== null) {
    details.push(`maximum SDK ${applicability.maximumSdkVersion}`);
  }
  if (applicability.introductionApi !== null) {
    details.push(`introduced in API ${applicability.introductionApi}`);
  }
  if (applicability.minimumDeviceApi !== null) {
    details.push(`minimum device API ${applicability.minimumDeviceApi}`);
  }
  if (applicability.minimumTargetSdk !== null) {
    details.push(`minimum target SDK ${applicability.minimumTargetSdk}`);
  }
  if (applicability.maximumTargetSdk !== null) {
    details.push(`maximum target SDK ${applicability.maximumTargetSdk}`);
  }
  if (applicability.actualTargetSdk !== null) {
    details.push(`app target SDK ${applicability.actualTargetSdk}`);
  }
  if (applicability.targetSdkState !== null) {
    details.push(
      `target SDK ${applicability.targetSdkState === "missing" ? "missing" : "non-numeric"}`,
    );
  }
  return details.join(", ");
}

/** Return whether the inspected APK is the package-enforced artifact used at runtime. */
export interface InspectionPartition {
  runtime: { baseline: number[]; elevated: number[] };
  appOps: { baseline: number[]; elevated: number[] };
}

/**
 * Partition inspected permission candidates by whether granting them requires root.
 *
 * Baseline candidates can run on an unrooted device. Elevated candidates are
 * optional and never become a launch dependency. This grouping only drives
 * presentation; the trusted review reports the authoritative partition.
 */
export function inspectionPartition(inspection: ApkInspectionResult): InspectionPartition {
  const runtime = { baseline: [] as number[], elevated: [] as number[] };
  const appOps = { baseline: [] as number[], elevated: [] as number[] };
  inspection.runtimeGrantCandidates.forEach((candidate, index) => {
    (candidate.requiresRoot ? runtime.elevated : runtime.baseline).push(index);
  });
  inspection.appOpCandidates.forEach((candidate, index) => {
    (candidate.requiresRoot ? appOps.elevated : appOps.baseline).push(index);
  });
  return { runtime, appOps };
}

/** Serialize only selected candidate identities and the opaque inspection binding. */
export function permissionSelectionForInspection(
  inspection: ApkInspectionResult | null,
): PermissionSelectionRequestDto | null {
  if (!inspection) return null;
  const runtimePermissions = inspection.runtimeGrantCandidates
    .filter((candidate) => candidate.selected)
    .map((candidate) => ({ permissionName: candidate.permissionName }));
  const appOps = inspection.appOpCandidates
    .filter((candidate) => candidate.selected)
    .map((candidate) => ({
      permissionName: candidate.permissionName,
      operationName: candidate.operationName,
      mode: candidate.mode,
    }));
  if (runtimePermissions.length === 0 && appOps.length === 0) return null;
  return {
    inspectionHandle: inspection.inspectionHandle,
    runtimePermissions,
    appOps,
  };
}

function clearPermissionSelections(
  inspection: ApkInspectionResult | null,
): ApkInspectionResult | null {
  if (!inspection) return null;
  return {
    ...inspection,
    runtimeGrantCandidates: inspection.runtimeGrantCandidates.map((candidate) => ({
      ...candidate,
      selected: false,
    })),
    appOpCandidates: inspection.appOpCandidates.map((candidate) => ({
      ...candidate,
      selected: false,
    })),
  };
}

export type AppGeneratorPhase =
  | "starting"
  | "selecting"
  | "downloading"
  | "inspecting"
  | "editing"
  | "reviewing"
  | "saving";

/**
 * One editable App Artifact row.
 *
 * Every value is a plain string or boolean so the row can be rendered by
 * controlled inputs. Canonical omission of blank optional values happens when
 * the row is converted into the artifact payload submitted to Rust.
 */
export interface AppArtifactRowEdits {
  id: string;
  kind: AppArtifactKindDto;
  name: string;
  description: string;
  strategy: AppArtifactStrategyDto;
  url: string;
  sha256: string;
  provider: AppReleaseProviderDto;
  baseUrl: string;
  repository: string;
  assetPattern: string;
  invertAssetPattern: boolean;
  prerelease: boolean;
}

/** One editable App Target row. */
export interface AppTargetRowEdits {
  id: string;
  kind: AppTargetKindDto;
  location: AppTargetLocationDto;
  path: string;
}

/** Structured mapping rows edited in the form and submitted to the native layer. */
export interface AppMappingRowEdits {
  artifacts: AppArtifactRowEdits[];
  targets: AppTargetRowEdits[];
  metadata: string;
}

export interface AppGeneratorFormState {
  app: AppDefinitionV1Dto;
  recipe: AppRecipeEditsDto;
  mappings: AppMappingRowEdits;
}

/** Create one empty artifact row. */
export function emptyArtifactRow(id = ""): AppArtifactRowEdits {
  return {
    id,
    kind: "apk",
    name: "",
    description: "",
    strategy: "user_provided",
    url: "",
    sha256: "",
    provider: "github",
    baseUrl: "",
    repository: "",
    assetPattern: "",
    invertAssetPattern: false,
    prerelease: false,
  };
}

/** Create one empty App Target row. */
export function emptyTargetRow(id = ""): AppTargetRowEdits {
  return { id, kind: "file", location: "app_data", path: "" };
}

/**
 * Build the canonical artifact source for one row.
 *
 * Blank optional values are omitted, so a saved definition never stores an
 * empty pattern, an empty checksum, or a disabled inversion flag. The pattern
 * itself is optional: an absent pattern selects the only eligible APK asset of
 * the resolved release by artifact kind alone.
 */
export function artifactSourceForRow(row: AppArtifactRowEdits): AppArtifactSourceDto {
  switch (row.strategy) {
    case "direct_url": {
      const sha256 = row.sha256.trim().toLowerCase();
      return sha256
        ? { strategy: "direct_url", url: row.url.trim(), sha256 }
        : { strategy: "direct_url", url: row.url.trim() };
    }
    case "latest_release": {
      const pattern = row.assetPattern.trim();
      return {
        strategy: "latest_release",
        provider: row.provider,
        base_url: row.baseUrl.trim(),
        repository: row.repository.trim(),
        ...(pattern
          ? {
              asset_pattern: pattern,
              ...(row.invertAssetPattern ? { invert_asset_pattern: true } : {}),
            }
          : {}),
        prerelease: row.prerelease,
      };
    }
    default:
      return { strategy: "user_provided" };
  }
}

/** Convert one form artifact row into the artifact payload submitted to Rust. */
export function artifactEditForRow(row: AppArtifactRowEdits): AppArtifactEditDto {
  const edit: AppArtifactEditDto = {
    id: row.id.trim(),
    kind: row.kind,
    source: artifactSourceForRow(row),
  };
  const name = row.name.trim();
  const description = row.description.trim();
  if (name) edit.name = name;
  if (description) edit.description = description;
  return edit;
}

/** Convert one form target row into the App Target payload submitted to Rust. */
export function targetEditForRow(row: AppTargetRowEdits): AppTargetEditDto {
  return {
    id: row.id.trim(),
    kind: row.kind,
    location: row.location,
    path: row.path.trim(),
  };
}

/** Convert the structured mapping rows into the payload the native layer parses. */
export function mappingEditsFromRows(rows: AppMappingRowEdits): AppMappingEditsDto {
  return {
    artifacts: rows.artifacts.map(artifactEditForRow),
    targets: rows.targets.map(targetEditForRow),
    metadata: rows.metadata,
  };
}

/** Build the editable row for one canonical artifact definition. */
export function artifactRowForDefinition(
  id: string,
  artifact: AppArtifactV1Dto,
): AppArtifactRowEdits {
  const row = emptyArtifactRow(id);
  row.kind = artifact.kind;
  row.name = artifact.name ?? "";
  row.description = artifact.description ?? "";
  const source = artifact.source;
  if (source.strategy === "direct_url") {
    row.strategy = "direct_url";
    row.url = source.url;
    row.sha256 = source.sha256 ?? "";
  } else if (source.strategy === "latest_release") {
    row.strategy = "latest_release";
    row.provider = source.provider;
    row.baseUrl = source.base_url;
    row.repository = source.repository;
    row.assetPattern = source.asset_pattern ?? "";
    row.invertAssetPattern = source.invert_asset_pattern ?? false;
    row.prerelease = source.prerelease;
  }
  return row;
}

/** Build the editable row for one canonical App Target definition. */
export function targetRowForDefinition(id: string, target: AppTargetV1Dto): AppTargetRowEdits {
  return { id, kind: target.kind, location: target.location, path: target.path };
}

export type AppGeneratorTargetMode = "new_app" | "existing_app";

export interface AppGeneratorState {
  phase: AppGeneratorPhase;
  sessionHandle: string | null;
  apkHandle: string | null;
  apkLabel: string | null;
  targetMode: AppGeneratorTargetMode;
  sourceMode: AppGeneratorSourceMode;
  sourceUrl: string;
  includePrereleases: boolean;
  sourceAnalysis: RemoteSourceAnalysisResult | null;
  selectedAssetHandle: string | null;
  remoteSource: RemoteSourceDescriptorDto | null;
  installStrategy: AppGeneratorInstallStrategy;
  assetPattern: string;
  trustedSha256: string;
  rootHandle: string | null;
  rootLabel: string | null;
  inspection: ApkInspectionResult | null;
  draft: AppRecipeDraftResult | null;
  form: AppGeneratorFormState | null;
  collisions: AppRecipeCollisionResult | null;
  existingApps: ExistingAppSummaryDto[] | null;
  existingAppsInventory: ExistingAppInventoryDto | null;
  existingApp: ExistingAppSelectionResult | null;
  appArtifactId: string | null;
  recipeId: string | null;
  artifactName: string;
  proposedArtifactId: string;
  automationOptIn: boolean;
  review: AppGeneratorReviewResult | null;
  publication: AppGeneratorPublicationOutcomeDto | null;
  publicationStatus: AppGeneratorPublicationStatusResult | null;
  lastIssuedRequestId: number;
  pendingPreviewRequestId: number | null;
  pendingReviewRequestId: number | null;
  pendingExistingAppsRequestId: number | null;
  pendingExistingAppSelectRequestId: number | null;
  previewPending: boolean;
  reviewPending: boolean;
  savePending: boolean;
  existingAppsPending: boolean;
  existingAppSelectPending: boolean;
  publicationStatusPending: boolean;
  error: string | null;
}

export type AppGeneratorAction =
  | {
      type: "started";
      sessionHandle: string;
      rootHandle?: string | null;
      rootLabel?: string | null;
    }
  | { type: "target-mode"; mode: AppGeneratorTargetMode }
  | { type: "existing-apps-request"; requestId: number }
  | { type: "existing-apps-response"; requestId: number; result: AppGeneratorExistingAppsResult }
  | { type: "existing-app-select-request"; requestId: number }
  | { type: "existing-app-select-response"; requestId: number; result: ExistingAppSelectionResult }
  | { type: "apk-selected"; apkHandle: string; label: string }
  | { type: "source-mode"; mode: AppGeneratorSourceMode }
  | { type: "source-url"; value: string }
  | { type: "include-prereleases"; value: boolean }
  | { type: "source-analyzing" }
  | { type: "source-analyzed"; analysis: RemoteSourceAnalysisResult }
  | { type: "asset-selected"; assetHandle: string }
  | { type: "asset-pattern"; value: string }
  | { type: "trusted-sha256"; value: string }
  | { type: "install-strategy"; strategy: AppGeneratorInstallStrategy }
  | { type: "downloading" }
  | { type: "remote-downloaded"; apkHandle: string; label: string; source: RemoteSourceDescriptorDto }
  | { type: "inspecting" }
  | { type: "inspected"; inspection: ApkInspectionResult }
  | { type: "runtime-candidate-selected"; index: number; selected: boolean }
  | { type: "app-op-candidate-selected"; index: number; selected: boolean }
  | { type: "automation-opt-in"; value: boolean }
  | { type: "artifact-id"; value: string }
  | { type: "recipe-id"; value: string }
  | { type: "artifact-name"; value: string }
  | { type: "identity-reset" }
  | { type: "form"; form: AppGeneratorFormState }
  | { type: "root-selected"; rootHandle: string; label: string }
  | { type: "preview-request"; requestId: number }
  | { type: "preview-response"; requestId: number; draft: AppRecipeDraftResult }
  | { type: "review-request"; requestId: number }
  | { type: "review-response"; requestId: number; review: AppGeneratorReviewResult }
  | { type: "save-request"; requestId: number }
  | { type: "publication-response"; result: AppGeneratorPublicationOutcomeDto }
  | { type: "publication-status-request" }
  | { type: "publication-status-response"; result: AppGeneratorPublicationStatusResult }
  | { type: "failure"; message: string };

/** App Artifact id generation proposes while the author keeps the default. */
const DEFAULT_ARTIFACT_ID = "apk";
export const initialAppGeneratorState: AppGeneratorState = {
  phase: "starting",
  sessionHandle: null,
  apkHandle: null,
  apkLabel: null,
  targetMode: "new_app",
  sourceMode: "local_apk",
  sourceUrl: "",
  includePrereleases: false,
  sourceAnalysis: null,
  selectedAssetHandle: null,
  remoteSource: null,
  installStrategy: "pinned_remote_asset",
  assetPattern: "",
  trustedSha256: "",
  rootHandle: null,
  rootLabel: null,
  inspection: null,
  draft: null,
  form: null,
  collisions: null,
  existingApps: null,
  existingAppsInventory: null,
  existingApp: null,
  appArtifactId: null,
  recipeId: null,
  artifactName: "",
  proposedArtifactId: DEFAULT_ARTIFACT_ID,
  automationOptIn: false,
  review: null,
  publication: null,
  publicationStatus: null,
  lastIssuedRequestId: 0,
  pendingPreviewRequestId: null,
  pendingReviewRequestId: null,
  pendingExistingAppsRequestId: null,
  pendingExistingAppSelectRequestId: null,
  previewPending: false,
  reviewPending: false,
  savePending: false,
  existingAppsPending: false,
  existingAppSelectPending: false,
  publicationStatusPending: false,
  error: null,
};

/**
 * Return the App Artifact id generation sends for one state.
 *
 * The value is either the author's explicit id or the stable proposal, and it
 * is never re-derived from catalog contents, so preview, review, and save all
 * name the same artifact and a collision always asks the author to choose a
 * different id explicitly.
 */
export function effectiveArtifactId(state: AppGeneratorState): string {
  const explicit = state.appArtifactId?.trim() ?? "";
  if (explicit.length > 0) return explicit;
  const proposed = state.proposedArtifactId.trim();
  return proposed.length > 0 ? proposed : DEFAULT_ARTIFACT_ID;
}

/** Return the App id generation targets: the locked selection or the previewed form. */
export function effectiveAppId(state: AppGeneratorState): string {
  return state.existingApp?.appId ?? state.form?.app.id ?? state.draft?.app.id ?? "";
}

/** Return the default Recipe id one App and App Artifact identity compose. */
export function proposedRecipeId(appId: string, artifactId: string): string {
  return `app.${appId}.${artifactId}.install`;
}

/**
 * Return the Recipe id generation sends for one state.
 *
 * An explicit author id wins; otherwise the stable App-id-derived proposal is
 * used. An empty App id leaves the proposal unresolved until a preview or an
 * existing-app selection supplies one, and generation then omits the field
 * instead of inventing an identity.
 */
export function effectiveRecipeId(state: AppGeneratorState): string {
  const explicit = state.recipeId?.trim() ?? "";
  if (explicit.length > 0) return explicit;
  const appId = effectiveAppId(state).trim();
  if (appId.length === 0) return "";
  return proposedRecipeId(appId, effectiveArtifactId(state));
}

/** Return the identity and permission-intent fields one preview or review request carries. */
export function generatorRequestContext(
  state: AppGeneratorState,
  requestId: number,
): AppGeneratorRequestContext {
  const artifactName = state.artifactName.trim();
  const recipeId = effectiveRecipeId(state);
  return {
    requestId,
    existingAppHandle:
      state.targetMode === "existing_app" ? state.existingApp?.appHandle ?? null : null,
    appArtifactId: effectiveArtifactId(state),
    recipeId: recipeId.length > 0 ? recipeId : null,
    artifactName: artifactName.length > 0 ? artifactName : null,
    permissionAutomationEnabled: state.automationOptIn,
  };
}

/**
 * Return the preview and review bookkeeping one author edit invalidates.
 *
 * Any edit that changes what generation would produce discards the immutable
 * review and both pending response slots, so a late response for the replaced
 * inputs can never be applied over newer author intent.
 */
function invalidatedReview(): Pick<
  AppGeneratorState,
  "review" | "pendingPreviewRequestId" | "pendingReviewRequestId"
> {
  return { review: null, pendingPreviewRequestId: null, pendingReviewRequestId: null };
}

/**
 * Return whether an unknown publication still blocks a confident retry.
 *
 * A status lookup that resolves the recorded operation to saved or failed
 * ends the uncertainty; until then the author is told the catalog may already
 * contain the files, and no automatic resubmission happens.
 */
export function publicationUnresolved(state: AppGeneratorState): boolean {
  if (state.publication?.outcome !== "unknown") return false;
  const status = state.publicationStatus;
  return status === null || status.outcome === "unknown";
}

/** Recovery facts one failed publication status recorded for the author. */
export interface PublicationFailureRecovery {
  /** Error message the failed attempt recorded, when the backend reported one. */
  message: string | null;
  /** Recipe path the attempt left for manual recovery, when reported. */
  recipePath: string | null;
  /** Whether the Recipe file the attempt created needs manual recovery. */
  recipeRecoveryRequired: boolean;
  /** Whether the created Recipe file was removed again during rollback. */
  recipeRemoved: boolean;
}

/**
 * Read the recorded failure facts of one publication status result.
 *
 * A failed attempt can leave the Recipe file behind for manual recovery, so the
 * status panel reports the recorded rollback facts instead of claiming that no
 * files remain. Missing or malformed detail fields fall back to safe values.
 */
export function publicationFailureRecovery(
  status: AppGeneratorPublicationStatusResult,
): PublicationFailureRecovery {
  const detail = status.detail;
  const rollbackValue = detail === null ? null : detail.rollback;
  const errorValue = detail === null ? null : detail.error;
  const rollback = isRecord(rollbackValue) ? rollbackValue : null;
  const error = isRecord(errorValue) ? errorValue : null;
  const message = typeof error?.message === "string" ? error.message.trim() : "";
  const recipePath = typeof rollback?.recipePath === "string" ? rollback.recipePath.trim() : "";
  return {
    message: message.length > 0 ? message : null,
    recipePath: recipePath.length > 0 ? recipePath : null,
    recipeRecoveryRequired: rollback?.recipeRecoveryRequired === true,
    recipeRemoved: rollback?.recipeRemoved === true,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Return the inspected package that does not match the selected existing App.
 *
 * Generating into an existing App locks its package id, so a different APK
 * package is an actionable hard error that asks the author to create a
 * different App instead of replacing or rewriting the selected one.
 */
export function selectedPackageMismatch(
  state: AppGeneratorState,
): { inspected: string; selected: string } | null {
  if (state.targetMode !== "existing_app") return null;
  const inspected = state.inspection?.manifest.packageName ?? "";
  const selected = state.existingApp?.packageId ?? "";
  if (inspected.length === 0 || selected.length === 0) return null;
  return inspected === selected ? null : { inspected, selected };
}


/** Return the artifact source strategy one remote source strategy requires. */
function artifactStrategyForRemoteSource(
  remoteSource: RemoteSourceDescriptorDto,
): AppArtifactStrategyDto | null {
  switch (remoteSource.strategy) {
    case "pinned_remote_asset":
      return "direct_url";
    case "latest_compatible_release":
      return "latest_release";
    case "user_provided_apk":
      return "user_provided";
    default:
      return null;
  }
}

/**
 * Return the artifact row generation pairs with the reviewed App Artifact id.
 *
 * The pairing names the exact trimmed id the request carries and an APK kind,
 * so the editable form and the reviewed artifact cannot drift apart through an
 * implicit search for a matching source strategy. Zero or several matching
 * rows leave the pairing unresolved, and source-step mirrors then leave the
 * form untouched instead of rewriting a row the reviewed artifact would not
 * use.
 */
function pairedArtifactRow(
  form: AppGeneratorFormState | null,
  artifactId: string,
): AppArtifactRowEdits | null {
  if (!form) return null;
  const wanted = artifactId.trim();
  if (wanted.length === 0) return null;
  const matches = form.mappings.artifacts.filter(
    (row) => row.kind === "apk" && row.id.trim() === wanted,
  );
  return matches.length === 1 ? matches[0]! : null;
}

/** Compare publisher checksum text the way artifact rows store it. */
function normalizedSha256Text(value: string): string {
  return value.trim().toLowerCase();
}

/**
 * Apply one source-step edit to the artifact row of the reviewed App Artifact
 * id. Rows the author customized are left untouched, so the editable form and
 * the source step cannot drift apart without the author saying so.
 */
function mapSelectedRemoteArtifactRows(
  state: AppGeneratorState,
  update: (row: AppArtifactRowEdits) => AppArtifactRowEdits,
): AppGeneratorFormState | null {
  const form = state.form;
  if (!form) return null;
  // Generation pairs the reviewed source with the artifact named by the exact
  // App Artifact id, so a source-step edit may rewrite only that artifact.
  // Rewriting every row that merely shares the source identity would silently
  // overwrite independent authored artifact policies.
  const paired = pairedArtifactRow(form, effectiveArtifactId(state));
  if (!paired) return form;
  const next = update(paired);
  if (next === paired) return form;
  const artifacts = form.mappings.artifacts.map((row) => (row === paired ? next : row));
  return { ...form, mappings: { ...form.mappings, artifacts } };
}

/**
 * Mirror one source-step checksum edit into the artifact row of the reviewed
 * App Artifact id when the row still carries the previous checksum.
 */
function mapSelectedChecksumRows(
  state: AppGeneratorState,
  update: (row: AppArtifactRowEdits) => AppArtifactRowEdits,
): AppGeneratorFormState | null {
  const previous = normalizedSha256Text(state.trustedSha256);
  return mapSelectedRemoteArtifactRows(state, (row) =>
    normalizedSha256Text(row.sha256) === previous ? update(row) : row,
  );
}

/**
 * Return the artifact row the reviewed App Artifact id names, or null when no
 * single row carries it.
 */
function selectedRemoteArtifactRow(
  form: AppGeneratorFormState | null,
  artifactId: string,
): AppArtifactRowEdits | null {
  return pairedArtifactRow(form, artifactId);
}

/**
 * Return the checksum carried by the artifact row the reviewed App Artifact id
 * names, or null when no single row carries it.
 */
function selectedRemoteArtifactSha256(
  form: AppGeneratorFormState | null,
  artifactId: string,
): string | null {
  const row = selectedRemoteArtifactRow(form, artifactId);
  return row ? row.sha256 : null;
}

/**
 * Keep the source-step filename pattern and the reviewed artifact row showing
 * one value. Once a draft exists the reviewed artifact is authoritative for
 * the latest-release policy, so the source step mirrors the row instead of
 * drifting from it while the author edits either control.
 */
function mirroredReleaseAssetPattern(
  state: AppGeneratorState,
  form: AppGeneratorFormState | null,
): string {
  const row = selectedRemoteArtifactRow(form, effectiveArtifactId(state));
  return row && reviewedSourceCarriesReleasePolicy(state.remoteSource)
    ? row.assetPattern
    : state.assetPattern;
}

/**
 * Keep the source-step prerelease toggle and the reviewed artifact row showing
 * one value. The reviewed artifact row is authoritative the same way its
 * filename pattern is.
 */
function mirroredIncludePrereleases(
  state: AppGeneratorState,
  form: AppGeneratorFormState | null,
): boolean {
  const row = selectedRemoteArtifactRow(form, effectiveArtifactId(state));
  return row && reviewedSourceCarriesReleasePolicy(state.remoteSource)
    ? row.prerelease
    : state.includePrereleases;
}

/**
 * Return whether the reviewed source carries a release policy.
 *
 * Only a latest-release source selects assets by filename pattern and
 * prerelease policy, so a pinned artifact row neither supplies nor mirrors
 * those two source-step controls.
 */
function reviewedSourceCarriesReleasePolicy(
  remoteSource: RemoteSourceDescriptorDto | null,
): boolean {
  return remoteSource?.strategy === "latest_compatible_release";
}

/**
 * Return the publisher checksum shown beside the reviewed source.
 *
 * The reviewed artifact row is authoritative once it exists; before that the
 * source-step input holds the author's one-shot checksum and keeps showing the
 * same value.
 */
export function mirroredTrustedSha256(
  state: AppGeneratorState,
  form: AppGeneratorFormState | null,
): string {
  return (
    selectedRemoteArtifactSha256(form, effectiveArtifactId(state)) ??
    state.trustedSha256
  );
}

/** Return whether one authored row resolves from the analyzed GitHub repository. */
function rowDescribesAnalyzedGithub(row: AppArtifactRowEdits, fullName: string): boolean {
  return (
    row.provider === "github"
    && normalizeServiceOrigin(row.baseUrl) === "https://github.com"
    && row.repository.trim().toLowerCase() === fullName.trim().toLowerCase()
  );
}

/** Compare service origins without trailing slashes or letter case. */
function normalizeServiceOrigin(value: string): string {
  return value.trim().replace(/\/+$/u, "").toLowerCase();
}

/** Compare repository names the way the analyzed-repository check does. */
function normalizeRepositoryName(value: string): string {
  return value.trim().toLowerCase();
}

/**
 * Return whether the retained source analysis still describes every
 * latest-release artifact the generated documents resolve with.
 *
 * Editing an authored latest-release artifact's provider, service origin, or
 * repository leaves the analysis describing a different source, so its
 * releases cannot confirm or block the authored filename policy. Rust reports
 * the same retargeting as the nonblocking latest-release stale-analysis
 * warning, so the browser preview stops gating instead of blocking on stale
 * evidence.
 */
export function releaseAnalysisDescribesReviewedPolicy(state: AppGeneratorState): boolean {
  const analysis = state.sourceAnalysis;
  if (!analysis || !analysis.repository) return false;
  const fullName = analysis.repository.fullName;
  // Native generation validates the retained analysis against the artifact it
  // pairs with the reviewed source, so an unrelated latest-release artifact
  // cannot invalidate a policy the analysis still confirms.
  const row = pairedArtifactRow(state.form, effectiveArtifactId(state));
  if (row) {
    return rowDescribesAnalyzedGithub(row, fullName);
  }
  // Before a draft exists no artifact row can contradict the analysis, and
  // editing the source input discards it, so it still describes the selection
  // the author is choosing and the preview may follow it.
  const source = state.remoteSource;
  if (!source) return true;
  return (
    source.provider === "github"
    && normalizeServiceOrigin(source.baseUrl ?? "") === "https://github.com"
    && (source.repository ?? "").trim().toLowerCase() === fullName.trim().toLowerCase()
  );
}

export function reduceAppGenerator(
  state: AppGeneratorState,
  action: AppGeneratorAction,
): AppGeneratorState {
  switch (action.type) {
    case "started":
      return {
        ...state,
        phase: "selecting",
        sessionHandle: action.sessionHandle,
        rootHandle: action.rootHandle ?? null,
        rootLabel: action.rootLabel ?? null,
        error: null,
      };
    case "target-mode":
      return {
        ...state,
        ...invalidatedReview(),
        targetMode: action.mode,
        existingApp: null,
        existingApps: null,
        existingAppsInventory: null,
        draft: null,
        form: null,
        collisions: null,
        phase: state.phase === "reviewing" ? "editing" : state.phase,
        error: null,
      };
    case "existing-apps-request":
      return {
        ...state,
        existingAppsPending: true,
        pendingExistingAppsRequestId: action.requestId,
        lastIssuedRequestId: Math.max(state.lastIssuedRequestId, action.requestId),
        error: null,
      };
    case "existing-apps-response": {
      if (action.requestId !== state.pendingExistingAppsRequestId) return state;
      return {
        ...state,
        existingAppsPending: false,
        pendingExistingAppsRequestId: null,
        existingApps: action.result.apps,
        existingAppsInventory: action.result.inventory,
        error: null,
      };
    }
    case "existing-app-select-request":
      return {
        ...state,
        existingAppSelectPending: true,
        pendingExistingAppSelectRequestId: action.requestId,
        lastIssuedRequestId: Math.max(state.lastIssuedRequestId, action.requestId),
        error: null,
      };
    case "existing-app-select-response": {
      if (action.requestId !== state.pendingExistingAppSelectRequestId) return state;
      return {
        ...state,
        ...invalidatedReview(),
        existingAppSelectPending: false,
        pendingExistingAppSelectRequestId: null,
        existingApp: action.result,
        // A different target invalidates every preview built for the previous
        // target while the inspected APK and its permission identities remain.
        draft: null,
        form: null,
        collisions: null,
        phase: state.phase === "reviewing" ? "editing" : state.phase,
        error: null,
      };
    }
    case "source-mode":
      return {
        ...state,
        ...invalidatedReview(),
        phase: "selecting",
        sourceMode: action.mode,
        sourceUrl: "",
        sourceAnalysis: null,
        selectedAssetHandle: null,
        assetPattern: "",
        trustedSha256: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "source-url":
      return {
        ...state,
        ...invalidatedReview(),
        sourceUrl: action.value,
        sourceAnalysis: null,
        selectedAssetHandle: null,
        trustedSha256: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "include-prereleases": {
      if (
        state.sourceMode === "github_repository"
        && state.sourceAnalysis?.mode === "github_repository"
      ) {
        const selectedAsset = state.sourceAnalysis.assets.find(
          (asset) => asset.assetHandle === state.selectedAssetHandle,
        );
        const selectedAssetRemainsEligible =
          selectedAsset !== undefined
          && (action.value || !selectedAsset.prerelease);
        const form = selectedAssetRemainsEligible
          ? mapSelectedRemoteArtifactRows(state, (row) => {
              const cleared = normalizedSha256Text(row.sha256)
                === normalizedSha256Text(state.trustedSha256)
                ? { ...row, sha256: "" }
                : row;
              return cleared.prerelease === state.includePrereleases
                ? { ...cleared, prerelease: action.value }
                : cleared;
            })
          : null;
        return {
          ...state,
          ...invalidatedReview(),
          includePrereleases: action.value,
          selectedAssetHandle: selectedAssetRemainsEligible
            ? state.selectedAssetHandle
            : null,
          assetPattern: selectedAssetRemainsEligible ? state.assetPattern : "",
          trustedSha256: "",
          remoteSource: selectedAssetRemainsEligible ? state.remoteSource : null,
          apkHandle: selectedAssetRemainsEligible ? state.apkHandle : null,
          apkLabel: selectedAssetRemainsEligible ? state.apkLabel : null,
          inspection: selectedAssetRemainsEligible ? state.inspection : null,
          draft: null,
          form,
          collisions: null,
          error: null,
        };
      }
      return {
        ...state,
        ...invalidatedReview(),
        includePrereleases: action.value,
        sourceAnalysis: null,
        selectedAssetHandle: null,
        trustedSha256: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    }
    case "source-analyzing":
      return {
        ...state,
        ...invalidatedReview(),
        phase: "inspecting",
        trustedSha256: "",
        form: mapSelectedChecksumRows(state, (row) => ({ ...row, sha256: "" })),
        error: null,
      };
    case "source-analyzed":
      return {
        ...state,
        ...invalidatedReview(),
        phase: "selecting",
        sourceAnalysis: action.analysis,
        selectedAssetHandle: action.analysis.preselectedAssetHandle,
        assetPattern: suggestedPatternForHandle(
          action.analysis,
          action.analysis.preselectedAssetHandle,
        ),
        trustedSha256: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "asset-selected":
      return {
        ...state,
        ...invalidatedReview(),
        selectedAssetHandle: action.assetHandle,
        assetPattern: suggestedPatternForHandle(state.sourceAnalysis, action.assetHandle),
        trustedSha256: "",
        remoteSource: null,
        apkHandle: null,
        apkLabel: null,
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "asset-pattern":
      return {
        ...state,
        ...invalidatedReview(),
        assetPattern: action.value,
        form: mapSelectedRemoteArtifactRows(state, (row) =>
          // The row still mirrors the source step when it carries either the
          // previous source-step pattern or the pattern the selected source
          // resolved with; anything else is author customization.
          row.assetPattern === state.assetPattern
          || row.assetPattern === (state.remoteSource?.assetPattern ?? "")
            ? { ...row, assetPattern: action.value }
            : row,
        ),
        draft: null,
        collisions: null,
        error: null,
      };
    case "trusted-sha256":
      return {
        ...state,
        ...invalidatedReview(),
        trustedSha256: action.value,
        form: mapSelectedChecksumRows(state, (row) => ({ ...row, sha256: action.value })),
        draft: null,
        collisions: null,
        error: null,
      };
    case "install-strategy":
      return {
        ...state,
        ...invalidatedReview(),
        installStrategy: action.strategy,
        trustedSha256: "",
        inspection: clearPermissionSelections(state.inspection),
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "downloading":
      return { ...state, phase: "downloading", error: null };
    case "remote-downloaded":
      return {
        ...state,
        ...invalidatedReview(),
        apkHandle: action.apkHandle,
        apkLabel: action.label,
        remoteSource: {
          ...action.source,
          strategy: state.installStrategy,
          assetPattern:
            state.installStrategy === "latest_compatible_release" ? state.assetPattern : null,
          includePrereleases: state.includePrereleases,
        },
        trustedSha256: "",
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "apk-selected":
      return {
        ...state,
        ...invalidatedReview(),
        apkHandle: action.apkHandle,
        apkLabel: action.label,
        trustedSha256: "",
        inspection: null,
        draft: null,
        form: null,
        collisions: null,
        error: null,
      };
    case "inspecting":
      // Without a reviewed remote artifact the checksum is a one-shot source
      // input, so a new inspection discards it. Once the paired row exists it
      // stays authoritative, and the source step keeps showing the same value.
      return {
        ...state,
        ...invalidatedReview(),
        phase: "inspecting",
        trustedSha256:
          selectedRemoteArtifactSha256(state.form, effectiveArtifactId(state)) ?? "",
        error: null,
      };
    case "inspected":
      return {
        ...state,
        ...invalidatedReview(),
        phase: "editing",
        inspection: action.inspection,
        error: null,
      };
    case "runtime-candidate-selected":
      if (!state.inspection) return state;
      return {
        ...state,
        ...invalidatedReview(),
        phase: "editing",
        inspection: {
          ...state.inspection,
          runtimeGrantCandidates: state.inspection.runtimeGrantCandidates.map(
            (candidate, index) =>
              index === action.index ? { ...candidate, selected: action.selected } : candidate,
          ),
        },
        draft: null,
        collisions: null,
        error: null,
      };
    case "app-op-candidate-selected":
      if (!state.inspection) return state;
      return {
        ...state,
        ...invalidatedReview(),
        phase: "editing",
        inspection: {
          ...state.inspection,
          appOpCandidates: state.inspection.appOpCandidates.map((candidate, index) =>
            index === action.index ? { ...candidate, selected: action.selected } : candidate,
          ),
        },
        draft: null,
        collisions: null,
        error: null,
      };
    case "automation-opt-in":
      return {
        ...state,
        ...invalidatedReview(),
        automationOptIn: action.value,
        error: null,
      };
    case "artifact-id":
      return {
        ...state,
        ...invalidatedReview(),
        appArtifactId: action.value,
        error: null,
      };
    case "recipe-id":
      return {
        ...state,
        ...invalidatedReview(),
        recipeId: action.value,
        error: null,
      };
    case "artifact-name":
      return {
        ...state,
        ...invalidatedReview(),
        artifactName: action.value,
        error: null,
      };
    case "identity-reset":
      return {
        ...state,
        ...invalidatedReview(),
        appArtifactId: null,
        recipeId: null,
        artifactName: "",
        error: null,
      };
    case "form":
      return {
        ...state,
        ...invalidatedReview(),
        form: action.form,
        assetPattern: mirroredReleaseAssetPattern(state, action.form),
        includePrereleases: mirroredIncludePrereleases(state, action.form),
        trustedSha256: mirroredTrustedSha256(state, action.form),
        draft: null,
        collisions: null,
        error: null,
      };
    case "root-selected":
      return {
        ...state,
        ...invalidatedReview(),
        rootHandle: action.rootHandle,
        rootLabel: action.label,
        collisions: null,
        error: null,
      };
    case "preview-request":
      return {
        ...state,
        previewPending: true,
        pendingPreviewRequestId: action.requestId,
        pendingReviewRequestId: null,
        lastIssuedRequestId: Math.max(state.lastIssuedRequestId, action.requestId),
        error: null,
      };
    case "preview-response": {
      if (action.requestId !== state.pendingPreviewRequestId) return state;
      const form = draftToForm(action.draft);
      return {
        ...state,
        ...invalidatedReview(),
        phase: "editing",
        previewPending: false,
        draft: action.draft,
        form,
        assetPattern: mirroredReleaseAssetPattern(state, form),
        includePrereleases: mirroredIncludePrereleases(state, form),
        trustedSha256: mirroredTrustedSha256(state, form),
        collisions: action.draft.collisions,
        error: null,
      };
    }
    case "review-request":
      return {
        ...state,
        reviewPending: true,
        pendingReviewRequestId: action.requestId,
        pendingPreviewRequestId: null,
        lastIssuedRequestId: Math.max(state.lastIssuedRequestId, action.requestId),
        error: null,
      };
    case "review-response": {
      if (action.requestId !== state.pendingReviewRequestId) return state;
      return {
        ...state,
        reviewPending: false,
        pendingReviewRequestId: null,
        review: action.review,
        phase: "reviewing",
        error: null,
      };
    }
    case "save-request":
      return {
        ...state,
        savePending: true,
        pendingPreviewRequestId: null,
        lastIssuedRequestId: Math.max(state.lastIssuedRequestId, action.requestId),
        error: null,
      };
    case "publication-response":
      return {
        ...state,
        savePending: false,
        publication: action.result,
        publicationStatus: null,
        // The review handle is consumed by the attempt whether it resolved
        // saved, failed, or unknown, so a retry always builds a new review
        // with the same author-controlled identities.
        review: null,
        pendingReviewRequestId: null,
        pendingPreviewRequestId: null,
        phase: state.phase === "saving" ? "editing" : state.phase,
        error: null,
      };
    case "publication-status-request":
      return { ...state, publicationStatusPending: true, error: null };
    case "publication-status-response":
      return {
        ...state,
        publicationStatusPending: false,
        publicationStatus: action.result,
        error: null,
      };
    case "failure":
      return {
        ...state,
        phase: state.phase === "starting" ? "starting" : "editing",
        previewPending: false,
        reviewPending: false,
        savePending: false,
        existingAppsPending: false,
        existingAppSelectPending: false,
        publicationStatusPending: false,
        error: action.message,
      };
  }
}

export function draftToForm(draft: AppRecipeDraftResult): AppGeneratorFormState {
  const app = structuredClone(draft.app);
  const recipe = structuredClone(draft.recipeEdits);
  if (app.name === app.package_id) {
    const originalName = app.name;
    const readableName = readableNameFromPackage(app.package_id);
    app.name = readableName;
    recipe.name = recipe.name.replace(originalName, readableName);
    recipe.description = recipe.description.replace(originalName, readableName);
    recipe.inputLabel = recipe.inputLabel.replace(originalName, readableName);
    recipe.inputDescription = recipe.inputDescription.replace(originalName, readableName);
  }
  return {
    app,
    recipe,
    mappings: {
      artifacts: Object.entries(app.artifacts ?? {}).map(([id, artifact]) =>
        artifactRowForDefinition(id, artifact),
      ),
      targets: Object.entries(app.targets ?? {}).map(([id, target]) =>
        targetRowForDefinition(id, target),
      ),
      metadata: JSON.stringify(app.metadata ?? {}, null, 2),
    },
  };
}

export function readableNameFromPackage(packageName: string): string {
  const tail = packageName.split(".").filter(Boolean).at(-1) ?? packageName;
  return tail
    .split(/[_-]+/u)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function visibleDraftDiagnostics(
  diagnostics: AppGeneratorDiagnosticDto[],
  hasRootBackedReview: boolean,
): AppGeneratorDiagnosticDto[] {
  if (!hasRootBackedReview) return diagnostics;
  return diagnostics.filter((diagnostic) => diagnostic.code !== "validation_context_limited");
}

export function diagnosticDisplayTitle(code: string, severity: string): string {
  const titles: Record<string, string> = {
    validation_context_limited: "Catalog validation not yet available",
    apk_certificate_missing: "Signing certificate unavailable",
    apk_label_missing: "Application name derived",
  };
  return titles[code] ?? (severity === "error" || severity === "blocking" ? "Action required" : "Review recommended");
}

export type FormRequestResult =
  | {
      ok: true;
      app: AppDefinitionV1Dto;
      recipe: AppRecipeEditsDto;
      mappings: AppMappingEditsDto;
    }
  | { ok: false; message: string };

export function formToRequest(form: AppGeneratorFormState): FormRequestResult {
  const metadata = parseMetadataObject(form.mappings.metadata);
  if (!metadata.ok) {
    return { ok: false, message: `Metadata: ${metadata.message}` };
  }
  return {
    ok: true,
    app: structuredClone(form.app),
    recipe: structuredClone(form.recipe),
    mappings: mappingEditsFromRows(form.mappings),
  };
}

export function eligibleApkAssets(
  analysis: RemoteSourceAnalysisResult | null,
  releaseTag?: string | null,
) {
  if (!analysis) return [];
  return analysis.assets.filter(
    (asset) =>
      asset.fileName.toLowerCase().endsWith(".apk") &&
      (!releaseTag || asset.releaseTag === releaseTag),
  );
}

export function matchingAssetNames(pattern: string, fileNames: string[]): string[] {
  try {
    const expression = new RegExp(pattern, "u");
    return fileNames.filter((fileName) => expression.test(fileName));
  } catch {
    return [];
  }
}

function compareFileNames(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function isApkFileName(fileName: string): boolean {
  return fileName.toLowerCase().endsWith(".apk");
}

/**
 * Build an immediate browser preview from the retained GitHub analysis.
 *
 * This preview deliberately uses JavaScript regular expressions. Rust repeats
 * validation with its own regex engine and trusted session-owned release data
 * before generation or saving.
 */
export function buildReleasePatternPreview(
  analysis: RemoteSourceAnalysisResult,
  pattern: string,
  includePrereleases: boolean,
): ReleasePatternPreviewResult {
  const eligibleReleases = analysis.releases.filter(
    (release) => includePrereleases || !release.prerelease,
  );
  if (eligibleReleases.length === 0) {
    return blockingPatternPreview(
      0,
      "No analyzed releases are eligible for this prerelease policy.",
    );
  }
  const trimmed = pattern.trim();
  let expression: RegExp | null = null;
  if (trimmed) {
    try {
      expression = new RegExp(trimmed, "u");
    } catch {
      return blockingPatternPreview(
        eligibleReleases.length,
        "Enter a valid regular expression. Rust performs final validation before generation.",
      );
    }
  }
  const releases = eligibleReleases.map((release) => {
    const matchingNames = release.assets
      .map((asset) => asset.fileName)
      .filter((fileName) =>
        expression === null ? isApkFileName(fileName) : expression.test(fileName),
      )
      .sort(compareFileNames);
    return {
      releaseTag: release.tag,
      prerelease: release.prerelease,
      matchingAssetNames: matchingNames,
      outcome: matchingNames.length === 1
        ? "unique_match" as const
        : matchingNames.length === 0
          ? "no_match" as const
          : "multiple_matches" as const,
    };
  });
  const uniqueMatchCount = releases.filter((release) => release.outcome === "unique_match").length;
  const noMatchCount = releases.filter((release) => release.outcome === "no_match").length;
  const multipleMatchesCount = releases.length - uniqueMatchCount - noMatchCount;
  const newest = releases[0];
  const qualifier = expression === null ? "eligible APK" : "matching APK";
  const blockingMessage = newest?.outcome === "no_match"
    ? `The newest eligible analyzed release (${newest.releaseTag}) has no ${qualifier}.`
    : newest?.outcome === "multiple_matches"
      ? `The newest eligible analyzed release (${newest.releaseTag}) has multiple ${qualifier}s.`
      : null;
  return {
    releases,
    eligibleReleaseCount: releases.length,
    uniqueMatchCount,
    noMatchCount,
    multipleMatchesCount,
    patternError: null,
    blockingMessage,
    blocking: blockingMessage !== null,
  };
}

function blockingPatternPreview(
  eligibleReleaseCount: number,
  message: string,
): ReleasePatternPreviewResult {
  return {
    releases: [],
    eligibleReleaseCount,
    uniqueMatchCount: 0,
    noMatchCount: 0,
    multipleMatchesCount: 0,
    patternError: message,
    blockingMessage: message,
    blocking: true,
  };
}

/** Filter the asset picker without discarding the retained release snapshot. */
export function analysisForPrereleasePolicy(
  analysis: RemoteSourceAnalysisResult,
  includePrereleases: boolean,
): RemoteSourceAnalysisResult {
  if (analysis.mode !== "github_repository" || includePrereleases) return analysis;
  const releases = analysis.releases.filter((release) => !release.prerelease);
  const assets = analysis.assets.filter((asset) => !asset.prerelease);
  return {
    ...analysis,
    releases,
    assets,
    preselectedAssetHandle: eligiblePreselectedAssetHandle(assets),
  };
}

function eligiblePreselectedAssetHandle(assets: RemoteAssetDto[]): string | null {
  return assets.length === 1 ? assets[0]?.assetHandle ?? null : null;
}

export function assetPatternError(pattern: string, fileNames: string[]): string | null {
  const trimmed = pattern.trim();
  if (!trimmed) {
    const eligible = fileNames.filter(isApkFileName);
    if (eligible.length === 0) {
      return "The selected release has no eligible APK asset.";
    }
    if (eligible.length > 1) {
      return "The selected release has multiple eligible APK assets. Add an APK filename pattern to select one.";
    }
    return null;
  }
  try {
    const expression = new RegExp(pattern, "u");
    const matches = fileNames.filter((fileName) => expression.test(fileName));
    if (matches.length === 0) {
      return "The pattern does not match an APK in the selected release.";
    }
    if (matches.length > 1) {
      return "The pattern matches multiple APKs. Make it more specific.";
    }
    return null;
  } catch {
    return "Enter a valid regular expression.";
  }
}

export function suggestAssetPattern(
  selectedFileName: string,
  siblingFileNames: string[],
): string {
  if (!selectedFileName) return "";
  const escaped = escapeRegex(selectedFileName);
  const versionSegment = selectedFileName.match(/v?\d+(?:[._-]\d+){1,4}/u)?.[0];
  const generalized = versionSegment
    ? escaped.replace(
        escapeRegex(versionSegment),
        "v?\\d+(?:[._-]\\d+){1,4}",
      )
    : escaped;
  const suggested = `^${generalized}$`;
  const matches = matchingAssetNames(suggested, siblingFileNames);
  return matches.length === 1 && matches[0] === selectedFileName
    ? suggested
    : `^${escaped}$`;
}

function suggestedPatternForHandle(
  analysis: RemoteSourceAnalysisResult | null,
  assetHandle: string | null,
): string {
  if (!analysis || !assetHandle) return "";
  const selected = analysis.assets.find((asset) => asset.assetHandle === assetHandle);
  if (!selected) return "";
  const siblings = eligibleApkAssets(analysis, selected.releaseTag).map(
    (asset) => asset.fileName,
  );
  return suggestAssetPattern(selected.fileName, siblings);
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

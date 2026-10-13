import type {
  AppArtifactKindDto,
  AppArtifactSourceDto,
  AppArtifactStrategyDto,
  AppArtifactV1Dto,
  AppDefinitionV1Dto,
  AppOpActionDto,
  AppPermissionSetDto,
  AppReleaseProviderDto,
  AppRuntimePermissionActionDto,
  AppTargetKindDto,
  AppTargetLocationDto,
  AppTargetV1Dto,
} from "../api/types.js";
import type { AppDefinitionCommand } from "../api/commands.js";

export type { AppDefinitionCommand } from "../api/commands.js";

/** An artifact row can retain incomplete editor input until its typed command is applied. */
export interface AppArtifactEditorDraft {
  readonly localId: string;
  readonly originalId?: string;
  readonly id: string;
  readonly kind: AppArtifactKindDto;
  readonly name?: string;
  readonly description?: string;
  readonly sourceStrategy: AppArtifactStrategyDto;
  readonly url?: string;
  readonly sha256?: string;
  readonly provider: AppReleaseProviderDto;
  readonly baseUrl?: string;
  readonly repository?: string;
  readonly assetPattern?: string;
  readonly invertAssetPattern?: boolean;
  readonly prerelease: boolean;
}

/** A runtime-permission row retains numeric input as text until the whole set is applied. */
export interface AppRuntimePermissionEditorDraft {
  readonly localId: string;
  readonly originalPermission?: string;
  readonly permission: string;
  readonly androidApiMin?: string;
  readonly androidApiMax?: string;
}

/** An app-op row retains numeric input as text until the whole set is applied. */
export interface AppOpEditorDraft {
  readonly localId: string;
  readonly originalOp?: string;
  readonly op: string;
  readonly androidApiMin?: string;
  readonly androidApiMax?: string;
}

/** The local representation of one baseline or elevated permission set. */
export interface AppPermissionSetEditorDraft {
  readonly runtime?: readonly AppRuntimePermissionEditorDraft[];
  readonly appOps?: readonly AppOpEditorDraft[];
}

/** A target row keeps its stable authored key separate from the editable target value. */
export interface AppTargetEditorDraft {
  readonly localId: string;
  readonly originalId?: string;
  readonly id: string;
  readonly kind: AppTargetKindDto;
  readonly location: AppTargetLocationDto;
  readonly path: string;
}

/** A metadata row retains raw JSON text so invalid input is not lost while being corrected. */
export interface AppMetadataEditorDraft {
  readonly localId: string;
  readonly originalKey?: string;
  readonly key: string;
  readonly valueJson: string;
}

/** Serializable local form state, including values not yet accepted by the backend. */
export interface AppDefinitionEditorDraft {
  readonly identity: {
    readonly id: string;
    readonly packageId: string;
  };
  readonly presentation: {
    readonly name: string;
    readonly description?: string;
    readonly category?: string;
    readonly launcherActivity?: string;
  };
  readonly artifacts: readonly AppArtifactEditorDraft[];
  readonly permissionSets: {
    readonly baseline?: AppPermissionSetEditorDraft;
    readonly elevated?: AppPermissionSetEditorDraft;
  };
  readonly targets: readonly AppTargetEditorDraft[];
  readonly metadata: readonly AppMetadataEditorDraft[];
}

const idPattern = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const reservedMetadataKeys = new Set([
  "schema_version",
  "kind",
  "id",
  "name",
  "description",
  "category",
  "package_id",
  "artifacts",
  "permission_sets",
  "targets",
  "launcher_activity",
  "metadata",
  "apk_inspection",
]);

let localIdSequence = 0;

function nextLocalId(prefix: string): string {
  localIdSequence += 1;
  return `${prefix}-${localIdSequence}`;
}

function permissionSetDraft(
  value: AppPermissionSetDto | undefined,
  name: "baseline" | "elevated",
): AppPermissionSetEditorDraft | undefined {
  if (value === undefined) return undefined;
  return {
    runtime: value.runtime?.map((action, index) => ({
      localId: `${name}-runtime-${index}`,
      originalPermission: action.permission,
      permission: action.permission,
      androidApiMin: action.android_api_min === undefined ? undefined : String(action.android_api_min),
      androidApiMax: action.android_api_max === undefined ? undefined : String(action.android_api_max),
    })),
    appOps: value.app_ops?.map((action, index) => ({
      localId: `${name}-app-op-${index}`,
      originalOp: action.op,
      op: action.op,
      androidApiMin: action.android_api_min === undefined ? undefined : String(action.android_api_min),
      androidApiMax: action.android_api_max === undefined ? undefined : String(action.android_api_max),
    })),
  };
}

function artifactDraft(id: string, value: AppArtifactV1Dto, index: number): AppArtifactEditorDraft {
  const source = value.source;
  return {
    localId: `artifact-${index}-${id}`,
    originalId: id,
    id,
    kind: value.kind,
    name: value.name,
    description: value.description,
    sourceStrategy: source.strategy,
    url: source.strategy === "direct_url" ? source.url : undefined,
    sha256: source.strategy === "direct_url" ? source.sha256 : undefined,
    provider: source.strategy === "latest_release" ? source.provider : "github",
    baseUrl: source.strategy === "latest_release" ? source.base_url : undefined,
    repository: source.strategy === "latest_release" ? source.repository : undefined,
    assetPattern: source.strategy === "latest_release" ? source.asset_pattern : undefined,
    invertAssetPattern: source.strategy === "latest_release" ? source.invert_asset_pattern : undefined,
    prerelease: source.strategy === "latest_release" ? source.prerelease : false,
  };
}

function targetDraft(id: string, value: AppTargetV1Dto, index: number): AppTargetEditorDraft {
  return { localId: `target-${index}-${id}`, originalId: id, id, ...value };
}

function formatMetadataValue(value: unknown): string {
  const serialized = JSON.stringify(value, null, 2);
  return serialized === undefined ? "null" : serialized;
}

/** Creates local form values without validating or rewriting authored values. */
export function createAppDefinitionEditorDraft(value: AppDefinitionV1Dto): AppDefinitionEditorDraft {
  return {
    identity: { id: value.id, packageId: value.package_id },
    presentation: {
      name: value.name,
      description: value.description,
      category: value.category,
      launcherActivity: value.launcher_activity,
    },
    artifacts: Object.entries(value.artifacts ?? {}).map(([id, artifact], index) => artifactDraft(id, artifact, index)),
    permissionSets: {
      baseline: permissionSetDraft(value.permission_sets?.baseline, "baseline"),
      elevated: permissionSetDraft(value.permission_sets?.elevated, "elevated"),
    },
    targets: Object.entries(value.targets ?? {}).map(([id, target], index) => targetDraft(id, target, index)),
    metadata: Object.entries(value.metadata ?? {}).map(([key, metadataValue], index) => ({
      localId: `metadata-${index}-${key}`,
      originalKey: key,
      key,
      valueJson: formatMetadataValue(metadataValue),
    })),
  };
}

function sameDraftValue(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((item, index) => sameDraftValue(item, right[index]));
  }
  if (isDraftObject(left) && isDraftObject(right)) {
    const leftKeys = Object.keys(left).filter((key) => key !== "localId");
    const rightKeys = Object.keys(right).filter((key) => key !== "localId");
    return leftKeys.length === rightKeys.length &&
      leftKeys.every((key) => Object.hasOwn(right, key) && sameDraftValue(left[key], right[key]));
  }
  return false;
}

function isDraftObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

interface DraftRowKey {
  readonly key: string;
  readonly actionIdentity?: string;
}

function draftRowKey(value: unknown): DraftRowKey | null {
  if (!isDraftObject(value)) return null;
  for (const property of ["originalId", "originalKey"]) {
    if (Object.hasOwn(value, property) && value[property] !== undefined) {
      return { key: `${property}:${String(value[property])}` };
    }
  }
  if (typeof value.originalPermission === "string") {
    const identity = `runtime:${JSON.stringify(value.originalPermission)}`;
    return { key: `originalPermission:${JSON.stringify(value.originalPermission)}`, actionIdentity: identity };
  }
  if (typeof value.originalOp === "string") {
    const identity = `app-op:${JSON.stringify(value.originalOp)}`;
    return { key: `originalOp:${JSON.stringify(value.originalOp)}`, actionIdentity: identity };
  }
  return typeof value.localId === "string" ? { key: `localId:${value.localId}` } : null;
}

function rowAuthoredIdentity(value: unknown): string | undefined {
  if (!isDraftObject(value)) return undefined;
  if (typeof value.id === "string") return `id:${JSON.stringify(value.id)}`;
  if (typeof value.key === "string") return `key:${JSON.stringify(value.key)}`;
  if (typeof value.permission === "string") return `runtime:${JSON.stringify(value.permission)}`;
  return typeof value.op === "string" ? `app-op:${JSON.stringify(value.op)}` : undefined;
}

function rowAcceptedIdentity(value: unknown): string | undefined {
  if (!isDraftObject(value)) return undefined;
  if (typeof value.originalId === "string") return `id:${JSON.stringify(value.originalId)}`;
  if (typeof value.originalKey === "string") return `key:${JSON.stringify(value.originalKey)}`;
  if (typeof value.originalPermission === "string") {
    return `runtime:${JSON.stringify(value.originalPermission)}`;
  }
  return typeof value.originalOp === "string" ? `app-op:${JSON.stringify(value.originalOp)}` : undefined;
}

function reattachAcceptedRowIdentity(localRow: unknown, acceptedRow: unknown): unknown {
  if (!isDraftObject(localRow) || !isDraftObject(acceptedRow)) return localRow;
  const identityProperty = ["originalId", "originalKey", "originalPermission", "originalOp"]
    .find((property) => typeof acceptedRow[property] === "string");
  if (identityProperty === undefined) return localRow;
  const reattached = { ...localRow, [identityProperty]: acceptedRow[identityProperty] };
  return sameDraftValue(reattached, acceptedRow) ? acceptedRow : reattached;
}

function detachRemovedDraftIdentity(value: unknown): unknown {
  if (!isDraftObject(value)) return value;
  const detached = { ...value };
  delete detached.originalId;
  delete detached.originalKey;
  delete detached.originalPermission;
  delete detached.originalOp;
  return detached;
}

function mergePendingArray(local: readonly unknown[], previous: readonly unknown[], next: readonly unknown[]): readonly unknown[] {
  if (sameDraftValue(local, previous)) return next;

  const ambiguousActions = new Set<string>();
  for (const rows of [local, previous, next]) {
    const counts = new Map<string, number>();
    for (const row of rows) {
      const actionIdentity = draftRowKey(row)?.actionIdentity;
      if (actionIdentity !== undefined) counts.set(actionIdentity, (counts.get(actionIdentity) ?? 0) + 1);
    }
    for (const [identity, count] of counts) {
      if (count > 1) ambiguousActions.add(identity);
    }
  }
  const keyOf = (value: unknown) => {
    const rowKey = draftRowKey(value);
    if (rowKey?.actionIdentity !== undefined && ambiguousActions.has(rowKey.actionIdentity)) return null;
    return rowKey?.key ?? null;
  };
  // Duplicate action names are invalid and cannot identify a row safely across snapshots.
  // Keeping the local collection intact avoids moving pending input onto another action.
  const keyed = [...local, ...previous, ...next].every((value) => keyOf(value) !== null);
  if (!keyed) return local;

  const localByKey = new Map(local.map((value) => [keyOf(value) as string, value]));
  const previousByKey = new Map(previous.map((value) => [keyOf(value) as string, value]));
  const nextByKey = new Map(next.map((value) => [keyOf(value) as string, value]));
  const localAdditionKeysByIdentity = new Map<string, string[]>();
  for (const row of local) {
    const key = keyOf(row) as string;
    if (previousByKey.has(key) || !isDraftObject(row) || rowAcceptedIdentity(row) !== undefined) continue;
    const authoredIdentity = rowAuthoredIdentity(row);
    if (authoredIdentity === undefined) continue;
    localAdditionKeysByIdentity.set(
      authoredIdentity,
      [...(localAdditionKeysByIdentity.get(authoredIdentity) ?? []), key],
    );
  }
  const acceptedIdentityCounts = new Map<string, number>();
  for (const row of next) {
    const identity = rowAcceptedIdentity(row);
    if (identity !== undefined) acceptedIdentityCounts.set(identity, (acceptedIdentityCounts.get(identity) ?? 0) + 1);
  }
  const localKeyByAcceptedKey = new Map<string, string>();
  for (const row of next) {
    const acceptedKey = keyOf(row) as string;
    const acceptedIdentity = rowAcceptedIdentity(row);
    if (
      previousByKey.has(acceptedKey) ||
      acceptedIdentity === undefined ||
      acceptedIdentityCounts.get(acceptedIdentity) !== 1
    ) continue;
    const localKeys = localAdditionKeysByIdentity.get(acceptedIdentity) ?? [];
    if (localKeys.length === 1) localKeyByAcceptedKey.set(acceptedKey, localKeys[0] as string);
  }
  const acceptedKeyByLocalKey = new Map(
    [...localKeyByAcceptedKey].map(([acceptedKey, localKey]) => [localKey, acceptedKey]),
  );
  const previousOrder = previous.map((value) => keyOf(value) as string);
  const localExistingOrder = local
    .map((value) => keyOf(value) as string)
    .filter((key) => previousByKey.has(key));
  const hasPendingReorder = !sameDraftValue(localExistingOrder, previousOrder);

  const mergeExistingRow = (key: string): unknown | undefined => {
    const localRow = localByKey.get(key);
    const previousRow = previousByKey.get(key);
    const nextRow = nextByKey.get(key);
    if (localRow === undefined || previousRow === undefined) return nextRow;
    if (nextRow === undefined) {
      return sameDraftValue(localRow, previousRow) ? undefined : detachRemovedDraftIdentity(localRow);
    }
    return mergePendingDraftValue(localRow, previousRow, nextRow);
  };

  if (hasPendingReorder) {
    const merged: unknown[] = local.flatMap((row) => {
      const key = keyOf(row) as string;
      const acceptedKey = acceptedKeyByLocalKey.get(key);
      const acceptedRow = acceptedKey === undefined ? undefined : nextByKey.get(acceptedKey);
      const value = previousByKey.has(key)
        ? mergeExistingRow(key)
        : acceptedRow === undefined
          ? row
          : reattachAcceptedRowIdentity(row, acceptedRow);
      return value === undefined ? [] : [value];
    });
    for (const row of next) {
      const key = keyOf(row) as string;
      if (!previousByKey.has(key) && !localByKey.has(key) && !localKeyByAcceptedKey.has(key)) merged.push(row);
    }
    return merged;
  }

  const merged: unknown[] = next.flatMap((row) => {
    const key = keyOf(row) as string;
    if (previousByKey.has(key) && !localByKey.has(key)) return [];
    const localAdditionKey = localKeyByAcceptedKey.get(key);
    const localAddition = localAdditionKey === undefined ? undefined : localByKey.get(localAdditionKey);
    const value = previousByKey.has(key)
      ? mergeExistingRow(key)
      : localAddition === undefined
        ? row
        : reattachAcceptedRowIdentity(localAddition, row);
    return value === undefined ? [] : [value];
  });
  for (const row of local) {
    const key = keyOf(row) as string;
    if (!previousByKey.has(key)) {
      if (!acceptedKeyByLocalKey.has(key)) merged.push(row);
    } else if (!nextByKey.has(key) && !sameDraftValue(row, previousByKey.get(key))) {
      const value = mergeExistingRow(key);
      if (value !== undefined) merged.push(value);
    }
  }
  return merged;
}

function mergePendingDraftValue(local: unknown, previous: unknown, next: unknown): unknown {
  if (sameDraftValue(local, previous)) return next;
  if (Array.isArray(local) && Array.isArray(previous) && Array.isArray(next)) {
    return mergePendingArray(local, previous, next);
  }
  if (isDraftObject(local) && isDraftObject(previous) && isDraftObject(next)) {
    const merged: Record<string, unknown> = { ...next };
    const keys = new Set([...Object.keys(local), ...Object.keys(previous), ...Object.keys(next)]);
    for (const key of keys) {
      const hasLocal = Object.hasOwn(local, key);
      const hasPrevious = Object.hasOwn(previous, key);
      const hasNext = Object.hasOwn(next, key);
      if (!hasLocal) {
        if (hasPrevious && hasNext) delete merged[key];
        continue;
      }
      if (!hasPrevious) {
        merged[key] = local[key];
        continue;
      }
      if (!hasNext) {
        if (!sameDraftValue(local[key], previous[key])) merged[key] = local[key];
        continue;
      }
      merged[key] = mergePendingDraftValue(local[key], previous[key], next[key]);
    }
    return merged;
  }
  return local;
}

/** Keeps only unsubmitted local fields when an accepted App Definition changes. */
export function rebasePendingAppDefinitionEditorDraft(
  draft: AppDefinitionEditorDraft,
  previous: AppDefinitionV1Dto,
  next: AppDefinitionV1Dto,
): AppDefinitionEditorDraft {
  return mergePendingDraftValue(
    draft,
    createAppDefinitionEditorDraft(previous),
    createAppDefinitionEditorDraft(next),
  ) as AppDefinitionEditorDraft;
}

function projectPermissionSet(value: AppPermissionSetEditorDraft | undefined) {
  if (value === undefined) return undefined;
  return {
    runtime: value.runtime?.map(({ permission, androidApiMin, androidApiMax }) => ({
      permission,
      androidApiMin,
      androidApiMax,
    })),
    appOps: value.appOps?.map(({ op, androidApiMin, androidApiMax }) => ({
      op,
      androidApiMin,
      androidApiMax,
    })),
  };
}

function projectDraft(value: AppDefinitionEditorDraft) {
  return {
    identity: value.identity,
    presentation: value.presentation,
    artifacts: value.artifacts.map((artifact) => ({
      id: artifact.id,
      kind: artifact.kind,
      name: artifact.name,
      description: artifact.description,
      sourceStrategy: artifact.sourceStrategy,
      ...(artifact.sourceStrategy === "direct_url"
        ? { url: artifact.url, sha256: artifact.sha256 }
        : artifact.sourceStrategy === "latest_release"
          ? {
              provider: artifact.provider,
              baseUrl: artifact.baseUrl,
              repository: artifact.repository,
              assetPattern: artifact.assetPattern,
              invertAssetPattern: artifact.invertAssetPattern,
              prerelease: artifact.prerelease,
            }
          : {}),
    })),
    permissionSets: {
      baseline: projectPermissionSet(value.permissionSets.baseline),
      elevated: projectPermissionSet(value.permissionSets.elevated),
    },
    targets: value.targets.map(({ id, kind, location, path }) => ({ id, kind, location, path })),
    metadata: value.metadata.map(({ key, valueJson }) => ({ key, valueJson })),
  };
}

/** Compares local fields with the accepted document while ignoring UI-only row IDs. */
export function isAppDefinitionEditorDraftDirty(
  draft: AppDefinitionEditorDraft,
  accepted: AppDefinitionV1Dto,
): boolean {
  return JSON.stringify(projectDraft(draft)) !== JSON.stringify(projectDraft(createAppDefinitionEditorDraft(accepted)));
}

/** Starts an artifact row locally without adding an incomplete value to the document. */
export function createEmptyArtifactEditorDraft(): AppArtifactEditorDraft {
  return {
    localId: nextLocalId("artifact-new"),
    id: "",
    kind: "file",
    sourceStrategy: "user_provided",
    provider: "github",
    prerelease: false,
  };
}

/** Starts a target row locally without adding an incomplete value to the document. */
export function createEmptyTargetEditorDraft(): AppTargetEditorDraft {
  return {
    localId: nextLocalId("target-new"),
    id: "",
    kind: "file",
    location: "app_data",
    path: "",
  };
}

/** Starts a runtime-permission action with a blank required identifier. */
export function createEmptyPermissionActionEditorDraft(kind: "runtime"): AppRuntimePermissionEditorDraft;
/** Starts an app-op action with a blank required identifier. */
export function createEmptyPermissionActionEditorDraft(kind: "app_op"): AppOpEditorDraft;
export function createEmptyPermissionActionEditorDraft(
  kind: "runtime" | "app_op",
): AppRuntimePermissionEditorDraft | AppOpEditorDraft {
  return kind === "runtime"
    ? { localId: nextLocalId("runtime-action"), permission: "" }
    : { localId: nextLocalId("app-op-action"), op: "" };
}

/** Starts a metadata row with a valid JSON null value. */
export function createEmptyMetadataEditorDraft(): AppMetadataEditorDraft {
  return { localId: nextLocalId("metadata-new"), key: "", valueJson: "null" };
}

/** Identifies metadata keys reserved for App Definition schema fields. */
export function isReservedAppMetadataKey(key: string): boolean {
  return reservedMetadataKeys.has(key);
}

/** Validates and converts an artifact's active source-policy inputs to the authored DTO. */
export function appArtifactSourceFromDraft(draft: AppArtifactEditorDraft): AppArtifactSourceDto {
  if (draft.sourceStrategy === "user_provided") return { strategy: "user_provided" };
  if (draft.sourceStrategy === "direct_url") {
    if (!draft.url || !isPublicHttpsUrl(draft.url)) {
      throw new Error("Enter a public HTTPS URL without credentials or a fragment.");
    }
    if (draft.sha256 !== undefined && !/^[0-9a-f]{64}$/.test(draft.sha256)) {
      throw new Error("SHA-256 must contain exactly 64 lowercase hexadecimal characters, or be removed.");
    }
    return { strategy: "direct_url", url: draft.url, sha256: draft.sha256 };
  }

  if (!draft.baseUrl || !isServiceOrigin(draft.baseUrl)) {
    throw new Error("Enter an HTTPS service origin without credentials, path, query, or fragment.");
  }
  if (draft.repository === undefined || !draft.repository.trim()) throw new Error("Enter a release repository.");
  if (draft.assetPattern !== undefined) {
    if (!draft.assetPattern.trim()) {
      throw new Error("Asset pattern must be a non-empty regular expression, or be removed.");
    }
  }
  if (draft.assetPattern === undefined && draft.invertAssetPattern === true) {
    throw new Error("Add an asset pattern before inverting pattern matches.");
  }
  return {
    strategy: "latest_release",
    provider: draft.provider,
    base_url: draft.baseUrl,
    repository: draft.repository,
    asset_pattern: draft.assetPattern,
    invert_asset_pattern: draft.invertAssetPattern,
    prerelease: draft.prerelease,
  };
}

/** Validates one artifact value before placing it in an AddArtifact or ReplaceArtifact command. */
export function appArtifactFromDraft(draft: AppArtifactEditorDraft): AppArtifactV1Dto {
  requireAppDefinitionId(draft.id, "Artifact ID");
  return {
    kind: draft.kind,
    name: draft.name,
    description: draft.description,
    source: appArtifactSourceFromDraft(draft),
  };
}

/** Validates one target value before placing it in an AddTarget or ReplaceTarget command. */
export function appTargetFromDraft(draft: AppTargetEditorDraft): AppTargetV1Dto {
  requireAppDefinitionId(draft.id, "Target ID");
  if (!draft.path.trim()) throw new Error("Target path must not be empty.");
  return { kind: draft.kind, location: draft.location, path: draft.path };
}

/** Converts a local permission-set form to the authored snake_case DTO. */
export function appPermissionSetFromDraft(draft: AppPermissionSetEditorDraft): AppPermissionSetDto {
  const runtime: AppRuntimePermissionActionDto[] | undefined = draft.runtime?.map((action, index) => {
    if (!action.permission.trim()) throw new Error(`Runtime permission ${index + 1} must not be empty.`);
    const android_api_min = parseAndroidApiBound(action.androidApiMin, "Minimum Android API");
    const android_api_max = parseAndroidApiBound(action.androidApiMax, "Maximum Android API");
    validateApiRange(android_api_min, android_api_max, `Runtime permission ${index + 1}`);
    return { permission: action.permission, android_api_min, android_api_max };
  });
  const app_ops: AppOpActionDto[] | undefined = draft.appOps?.map((action, index) => {
    if (!action.op.trim()) throw new Error(`App-op action ${index + 1} must not be empty.`);
    const android_api_min = parseAndroidApiBound(action.androidApiMin, "Minimum Android API");
    const android_api_max = parseAndroidApiBound(action.androidApiMax, "Maximum Android API");
    validateApiRange(android_api_min, android_api_max, `App-op action ${index + 1}`);
    return { op: action.op, mode: "allow", android_api_min, android_api_max };
  });
  return { runtime, app_ops };
}

function validateApiRange(minimum: number | undefined, maximum: number | undefined, label: string): void {
  if (minimum !== undefined && maximum !== undefined && minimum > maximum) {
    throw new Error(`${label} minimum Android API must not exceed its maximum.`);
  }
}

/** Validates an App Definition catalog identifier using the schema-v1 identifier syntax. */
export function requireAppDefinitionId(value: string, label: string): void {
  if (!idPattern.test(value)) throw new Error(`${label} must use lowercase letters, numbers, dots, underscores, or hyphens.`);
}

function isPublicHttpsUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && url.username === "" && url.password === "" && url.hash === "";
  } catch {
    return false;
  }
}

function isServiceOrigin(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      url.protocol === "https:" &&
      url.username === "" &&
      url.password === "" &&
      url.search === "" &&
      url.hash === "" &&
      (url.pathname === "" || url.pathname === "/")
    );
  } catch {
    return false;
  }
}

/** Parses a positive optional API bound; an empty input means that no bound is authored. */
export function parseAndroidApiBound(value: string | undefined, label: string): number | undefined {
  if (value === undefined || value === "") return undefined;
  if (!/^\d+$/.test(value)) throw new Error(`${label} must be a positive whole number or blank.`);
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new Error(`${label} must be a positive whole number or blank.`);
  }
  return parsed;
}

/** Decodes a metadata editor value while leaving the original text available on failure. */
export function parseAppMetadataValue(valueJson: string): unknown {
  try {
    return JSON.parse(valueJson) as unknown;
  } catch {
    throw new Error("Metadata value must be valid JSON.");
  }
}

function acceptedArtifactRow(
  accepted: AppDefinitionV1Dto,
  id: string,
  current: AppArtifactEditorDraft | undefined,
): AppArtifactEditorDraft | undefined {
  const value = accepted.artifacts?.[id];
  if (value === undefined) return undefined;
  const index = Object.keys(accepted.artifacts ?? {}).indexOf(id);
  return { ...artifactDraft(id, value, index), localId: current?.localId ?? `artifact-${index}-${id}` };
}

function acceptedTargetRow(
  accepted: AppDefinitionV1Dto,
  id: string,
  current: AppTargetEditorDraft | undefined,
): AppTargetEditorDraft | undefined {
  const value = accepted.targets?.[id];
  if (value === undefined) return undefined;
  const index = Object.keys(accepted.targets ?? {}).indexOf(id);
  return { ...targetDraft(id, value, index), localId: current?.localId ?? `target-${index}-${id}` };
}

function acceptedMetadataRow(
  accepted: AppDefinitionV1Dto,
  key: string,
  current: AppMetadataEditorDraft | undefined,
): AppMetadataEditorDraft | undefined {
  if (!(key in (accepted.metadata ?? {}))) return undefined;
  const index = Object.keys(accepted.metadata ?? {}).indexOf(key);
  return {
    localId: current?.localId ?? `metadata-${index}-${key}`,
    originalKey: key,
    key,
    valueJson: formatMetadataValue(accepted.metadata?.[key]),
  };
}

function reorderByAcceptedKeys<T>(
  rows: readonly T[],
  acceptedKeys: readonly string[],
  keyOf: (row: T) => string | undefined,
): T[] {
  const byKey = new Map(rows.map((row) => [keyOf(row), row]));
  const ordered = acceptedKeys.flatMap((key) => {
    const row = byKey.get(key);
    return row === undefined ? [] : [row];
  });
  const included = new Set(acceptedKeys);
  return [...ordered, ...rows.filter((row) => !included.has(keyOf(row) ?? ""))];
}

/** Reconciles accepted command fields without discarding unrelated pending form input. */
export function reconcileAppDefinitionEditorDraft(
  draft: AppDefinitionEditorDraft,
  accepted: AppDefinitionV1Dto,
  command: AppDefinitionCommand,
  submittedRowLocalId?: string,
): AppDefinitionEditorDraft {
  const acceptedDraft = createAppDefinitionEditorDraft(accepted);
  switch (command.type) {
    case "SetId":
      return { ...draft, identity: { ...draft.identity, id: accepted.id } };
    case "SetPackageId":
      return { ...draft, identity: { ...draft.identity, packageId: accepted.package_id } };
    case "SetName":
      return { ...draft, presentation: { ...draft.presentation, name: accepted.name } };
    case "SetDescription":
      return { ...draft, presentation: { ...draft.presentation, description: accepted.description } };
    case "SetCategory":
      return { ...draft, presentation: { ...draft.presentation, category: accepted.category } };
    case "SetLauncherActivity":
      return { ...draft, presentation: { ...draft.presentation, launcherActivity: accepted.launcher_activity } };
    case "AddArtifact":
    case "ReplaceArtifact": {
      const currentIndex = draft.artifacts.findIndex(
        (item) => item.originalId === command.artifactId || item.id === command.artifactId,
      );
      const current = currentIndex < 0 ? undefined : draft.artifacts[currentIndex];
      const updated = acceptedArtifactRow(accepted, command.artifactId, current);
      if (updated === undefined) return draft;
      if (currentIndex < 0) return { ...draft, artifacts: [...draft.artifacts, updated] };
      return { ...draft, artifacts: draft.artifacts.map((item, index) => index === currentIndex ? updated : item) };
    }
    case "RemoveArtifact":
      return {
        ...draft,
        artifacts: draft.artifacts.filter((item) => item.originalId !== command.artifactId && item.id !== command.artifactId),
      };
    case "ReorderArtifact":
      return {
        ...draft,
        artifacts: reorderByAcceptedKeys(
          draft.artifacts,
          Object.keys(accepted.artifacts ?? {}),
          (item) => item.originalId ?? (item.id || undefined),
        ),
      };
    case "SetPermissionSet":
      return {
        ...draft,
        permissionSets: {
          ...draft.permissionSets,
          [command.permissionSet]: acceptedDraft.permissionSets[command.permissionSet],
        },
      };
    case "AddTarget":
    case "ReplaceTarget": {
      const currentIndex = draft.targets.findIndex(
        (item) => item.originalId === command.targetId || item.id === command.targetId,
      );
      const current = currentIndex < 0 ? undefined : draft.targets[currentIndex];
      const updated = acceptedTargetRow(accepted, command.targetId, current);
      if (updated === undefined) return draft;
      if (currentIndex < 0) return { ...draft, targets: [...draft.targets, updated] };
      return { ...draft, targets: draft.targets.map((item, index) => index === currentIndex ? updated : item) };
    }
    case "RemoveTarget":
      return {
        ...draft,
        targets: draft.targets.filter((item) => item.originalId !== command.targetId && item.id !== command.targetId),
      };
    case "ReorderTarget":
      return {
        ...draft,
        targets: reorderByAcceptedKeys(
          draft.targets,
          Object.keys(accepted.targets ?? {}),
          (item) => item.originalId ?? (item.id || undefined),
        ),
      };
    case "SetMetadataEntry": {
      const submittedIndex = submittedRowLocalId === undefined
        ? -1
        : draft.metadata.findIndex((item) => item.localId === submittedRowLocalId);
      const currentIndex = submittedIndex >= 0
        ? submittedIndex
        : draft.metadata.findIndex((item) => item.originalKey === command.key || item.key === command.key);
      const current = currentIndex < 0 ? undefined : draft.metadata[currentIndex];
      const updated = acceptedMetadataRow(accepted, command.key, current);
      if (updated === undefined) return draft;
      if (currentIndex < 0) return { ...draft, metadata: [...draft.metadata, updated] };
      return { ...draft, metadata: draft.metadata.map((item, index) => index === currentIndex ? updated : item) };
    }
    case "RemoveMetadataEntry":
      return {
        ...draft,
        metadata: draft.metadata.filter((item) => item.originalKey !== command.key && item.key !== command.key),
      };
  }
}

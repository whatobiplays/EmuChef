import type {
  AppCatalogAppDto,
  AppCatalogDto,
  AppCatalogTargetDto,
  ArtifactDto,
  InputDto,
  RefIndexDto,
  StepSpecDto,
} from "../api/types.js";

/**
 * Pure helpers for catalog-backed App Definition controls.
 *
 * The Rust document session projects the authored catalog into
 * `RecipeDocumentDto.appCatalog`. These helpers turn that projection into the
 * choices and messages the Config Editor shows, so React components stay thin
 * and every rule is unit-testable without a browser. Retained authored values
 * are always kept visible: a selection that the catalog does not define stays
 * in the option list marked as retained, matching the static validation
 * diagnostics Rust reports for the same document.
 */

/** How a step consumes App Definition context, taken from its step spec. */
export type AppContextControl = "hidden" | "required" | "optional";

function projectionIsReady(appCatalog: AppCatalogDto): boolean {
  return appCatalog.status === "ready";
}

/**
 * Classifies the App Definition control a step shows.
 *
 * `required` steps must select an App Definition, `optional` steps may also
 * select none, and every other step shows no control at all.
 */
export function appContextControl(stepSpec: StepSpecDto | null | undefined): AppContextControl {
  const requirement = stepSpec?.appContext ?? "none";
  if (requirement === "required") {
    return "required";
  }
  if (requirement === "optional") {
    return "optional";
  }
  return "hidden";
}

/**
 * App Definitions offered for selection, sorted by identifier.
 *
 * A document without a bound catalog root, or one whose catalog reported
 * diagnostics, offers no choices: structured controls fail closed and authors
 * rely on diagnostics for retained values.
 */
export function catalogApps(appCatalog: AppCatalogDto): AppCatalogAppDto[] {
  if (!projectionIsReady(appCatalog)) {
    return [];
  }
  return [...appCatalog.apps].sort((left, right) => left.id.localeCompare(right.id));
}

/** Resolves one App Definition by identifier, or null when it is unavailable. */
export function catalogApp(
  appCatalog: AppCatalogDto,
  appRef: string | null | undefined,
): AppCatalogAppDto | null {
  if (!appRef || !projectionIsReady(appCatalog)) {
    return null;
  }
  return appCatalog.apps.find((app) => app.id === appRef) ?? null;
}

/** One selectable App Definition identifier. */
export interface AppRefOption {
  value: string;
  label: string;
  /** True when the option preserves a value the catalog does not define. */
  retained: boolean;
}

/**
 * Options for a step or artifact App Definition select.
 *
 * A current value that the catalog does not define is listed first so
 * switching catalogs or authored roots never hides the authored selection.
 */
export function appRefOptions(
  appCatalog: AppCatalogDto,
  currentAppRef: string | null | undefined,
): AppRefOption[] {
  const apps = catalogApps(appCatalog);
  const options: AppRefOption[] = [];
  if (currentAppRef && !apps.some((app) => app.id === currentAppRef)) {
    options.push({
      value: currentAppRef,
      label: `${currentAppRef} (not in this catalog)`,
      retained: true,
    });
  }
  for (const app of apps) {
    options.push({ value: app.id, label: `${app.name} (${app.id})`, retained: false });
  }
  return options;
}

/**
 * Recipe refs of the APK artifacts an App Definition declares.
 *
 * `install_apk` consumes an APK through `artifacts.<id>.local_path`; only
 * artifacts whose kind is `apk` are valid, so other artifacts never appear.
 */
export function apkArtifactRefs(app: AppCatalogAppDto | null | undefined): string[] {
  return (app?.artifacts ?? [])
    .filter((artifact) => artifact.kind === "apk")
    .map((artifact) => `artifacts.${artifact.id}.local_path`)
    .sort();
}

/** One selectable literal value, for example a permission set name. */
export interface AppChoiceOption {
  value: string;
  label: string;
  /** True when the option preserves a value the App Definition does not define. */
  retained: boolean;
}

/**
 * Permission sets a `grant_permissions` step may select.
 *
 * Options come from the App Definition; a current value that is not defined
 * stays visible with an explanatory label instead of being dropped.
 */
export function permissionSetChoices(
  app: AppCatalogAppDto | null | undefined,
  currentValue: string | null | undefined,
): AppChoiceOption[] {
  const sets = app?.permissionSets ?? [];
  const options: AppChoiceOption[] = [];
  if (currentValue && !sets.includes(currentValue)) {
    options.push({
      value: currentValue,
      label: `${currentValue} (not defined by this App Definition)`,
      retained: true,
    });
  }
  for (const set of sets) {
    options.push({ value: set, label: set, retained: false });
  }
  return options;
}

/**
 * How well one App Target matches the current `copy_files` source and policy.
 *
 * `incompatible` means authored values already contradict a planner rule,
 * `unknown` means the copy source or policy cannot be resolved until
 * planning, and `compatible` means every planner rule that can be checked
 * from authored values holds.
 */
export type AppTargetCompatibility = "compatible" | "incompatible" | "unknown";

/** One selectable App Definition target with its copy compatibility. */
export interface AppTargetChoice {
  id: string;
  label: string;
  compatibility: AppTargetCompatibility;
  /** Why the target is incompatible or unknown, or null when it is compatible. */
  reason: string | null;
}

/**
 * App Targets a `copy_files` destination may use.
 *
 * Compatibility mirrors the execution planner: directory targets accept any
 * copy, while file targets need a single file source and cannot be used with a
 * `sync` policy. Targets the authored values already contradict stay listed
 * as incompatible with the planner's reason; targets whose compatibility
 * depends on a copy source or policy that cannot be resolved yet stay listed
 * as unknown and remain selectable, deferring to authoritative validation.
 */
export function appTargetChoices(
  app: AppCatalogAppDto | null | undefined,
  options: { sourceValueType: string | null; copyPolicy: string | null },
): AppTargetChoice[] {
  return (app?.targets ?? [])
    .map((target) => {
      const compatibility = targetCompatibility(target, options.sourceValueType, options.copyPolicy);
      return {
        id: target.id,
        label: `${target.id} (${target.kind})`,
        compatibility: compatibility.compatibility,
        reason: compatibility.reason,
      };
    })
    .sort((left, right) => left.id.localeCompare(right.id));
}

/**
 * Planner-mirrored compatibility of one App Target as a `copy_files` destination.
 *
 * Literal authored evidence wins: a known non-file copy source or a literal
 * `sync` policy marks a file target incompatible. When the source value type
 * or the copy policy cannot be resolved from the authored document, the
 * result is unknown instead of a guess so the UI defers to validation.
 */
function targetCompatibility(
  target: AppCatalogTargetDto,
  sourceValueType: string | null,
  copyPolicy: string | null,
): { compatibility: AppTargetCompatibility; reason: string | null } {
  if (target.kind === "directory") {
    return { compatibility: "compatible", reason: null };
  }
  if (sourceValueType !== null && sourceValueType !== "file_path") {
    return { compatibility: "incompatible", reason: "A file target needs a single file source." };
  }
  if (copyPolicy === "sync") {
    return { compatibility: "incompatible", reason: "A sync copy needs a directory target." };
  }
  if (sourceValueType === null) {
    return {
      compatibility: "unknown",
      reason: "The copy source is not resolved yet, so compatibility is unknown until validation.",
    };
  }
  if (copyPolicy === null) {
    return {
      compatibility: "unknown",
      reason: "The copy policy is not resolved yet, so compatibility is unknown until validation.",
    };
  }
  return { compatibility: "compatible", reason: null };
}

/** One selectable App Target, including a retained current value. */
export interface AppTargetOption extends AppTargetChoice {
  /** True when the option preserves a value the App Definition does not define. */
  retained: boolean;
}

/**
 * App Target select options for one `copy_files` destination.
 *
 * The list adds the retained current target in front of the App Definition's
 * targets when the catalog does not define it, so changing App Definitions or
 * authored roots never hides an authored selection. A retained target is
 * marked unknown because the App Definition that defines its kind is missing.
 * Compatibility otherwise follows the same planner-mirrored rules as
 * {@link appTargetChoices}.
 */
export function appTargetOptions(
  app: AppCatalogAppDto | null | undefined,
  options: {
    sourceValueType: string | null;
    copyPolicy: string | null;
    currentTargetId?: string | null;
  },
): AppTargetOption[] {
  const choices = appTargetChoices(app, options);
  const currentTargetId = options.currentTargetId ?? null;
  const entries: AppTargetOption[] = [];
  if (currentTargetId && !choices.some((choice) => choice.id === currentTargetId)) {
    entries.push({
      id: currentTargetId,
      label: currentTargetId + " (not defined by this App Definition)",
      compatibility: "unknown",
      reason: null,
      retained: true,
    });
  }
  for (const choice of choices) {
    entries.push({ ...choice, retained: false });
  }
  return entries;
}

/** Condition kinds whose `path` parameter may select an App Target. */
export type AppConditionType = "path_exists" | "file_exists";

/**
 * App Target select options for one `path_exists` or `file_exists` condition.
 *
 * The execution planner resolves condition targets through the enclosing
 * step's App Definition: `path_exists` accepts file and directory
 * targets, while `file_exists` can only inspect a file target. Targets
 * the condition kind rejects stay listed as incompatible with the planner's
 * reason. A current target the catalog does not define stays visible as a
 * retained unknown option so the authored value is never lost.
 */
export function conditionTargetOptions(
  app: AppCatalogAppDto | null | undefined,
  conditionType: AppConditionType,
  currentTargetId?: string | null,
): AppTargetOption[] {
  const choices: AppTargetChoice[] = (app?.targets ?? [])
    .map((target) => {
      const compatible = conditionType === "path_exists" || target.kind === "file";
      return {
        id: target.id,
        label: `${target.id} (${target.kind})`,
        compatibility: compatible ? ("compatible" as const) : ("incompatible" as const),
        reason: compatible ? null : "A file_exists check needs a file target.",
      };
    })
    .sort((left, right) => left.id.localeCompare(right.id));
  const entries: AppTargetOption[] = [];
  if (currentTargetId && !choices.some((choice) => choice.id === currentTargetId)) {
    entries.push({
      id: currentTargetId,
      label: currentTargetId + " (not defined by this App Definition)",
      compatibility: "unknown",
      reason: null,
      retained: true,
    });
  }
  for (const choice of choices) {
    entries.push({ ...choice, retained: false });
  }
  return entries;
}

/**
 * First App Target a newly added condition of `conditionType` may select.
 *
 * Returns null while the App Definition is unavailable or defines no target
 * of the required kind, so the Config Editor never authors an incompatible
 * target on behalf of the author.
 */
export function conditionTargetSeed(
  app: AppCatalogAppDto | null | undefined,
  conditionType: AppConditionType,
): string | null {
  return (
    conditionTargetOptions(app, conditionType).find(
      (option) => !option.retained && option.compatibility === "compatible",
    )?.id ?? null
  );
}

/**
 * Recipe artifact refs one app-aware `install_apk` step may select.
 *
 * The result joins the selected App Definition's APK artifacts with the
 * Recipe App Artifacts that reference them. Recipe artifacts owned by another
 * App Definition, artifacts whose App Definition kind is not `apk`, and
 * remote file artifacts never qualify. The projection is authoritative: an
 * unavailable catalog yields no choices.
 */
export function installApkArtifactRefs(
  appCatalog: AppCatalogDto,
  artifacts: Record<string, ArtifactDto>,
  appRef: string | null | undefined,
): string[] {
  const app = catalogApp(appCatalog, appRef);
  if (app === null) {
    return [];
  }
  const apkArtifactIds = new Set(
    app.artifacts.filter((artifact) => artifact.kind === "apk").map((artifact) => artifact.id),
  );
  return Object.values(artifacts)
    .filter(
      (artifact) =>
        artifact.appRef === app.id &&
        artifact.artifact !== undefined &&
        apkArtifactIds.has(artifact.artifact),
    )
    .map((artifact) => "artifacts." + artifact.id + ".local_path")
    .sort();
}

/**
 * The value type of one authored `{ ref }` param value.
 *
 * The ref index carries the value type a ref produces, which a `copy_files`
 * destination needs to mirror planner target compatibility. Values that are
 * not authored refs, and refs the index does not describe, report null.
 */
export function refSourceValueType(refIndex: RefIndexDto, value: unknown): string | null {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const ref = (value as { ref?: unknown }).ref;
  if (typeof ref !== "string") {
    return null;
  }
  return refIndex.candidates.find((candidate) => candidate.ref === ref)?.valueType ?? null;
}

/**
 * The literal `copy_files` copy policy, or null for any other value shape.
 *
 * A policy authored as an input ref only becomes known during planning, so
 * target compatibility treats it as unknown instead of guessing.
 */
export function copyPolicyValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

/**
 * App Definition artifact options for one Recipe App Artifact reference.
 *
 * Options list the selected App Definition's artifacts sorted by identifier.
 * A current artifact id the App Definition does not define stays visible as a
 * retained option in front of the defined choices.
 */
export function appArtifactOptions(
  app: AppCatalogAppDto | null | undefined,
  currentArtifactId: string | null | undefined,
): AppChoiceOption[] {
  const artifacts = [...(app?.artifacts ?? [])].sort((left, right) => left.id.localeCompare(right.id));
  const options: AppChoiceOption[] = [];
  if (currentArtifactId && !artifacts.some((artifact) => artifact.id === currentArtifactId)) {
    options.push({
      value: currentArtifactId,
      label: currentArtifactId + " (not defined by this App Definition)",
      retained: true,
    });
  }
  for (const artifact of artifacts) {
    options.push({ value: artifact.id, label: artifact.id + " (" + artifact.kind + ")", retained: false });
  }
  return options;
}

/**
 * Recipe input options for one user-provided App Artifact binding.
 *
 * Options list the compatible Recipe inputs: file inputs that hold exactly one
 * path. The Rust planner rejects every other binding shape as
 * `app_artifact_input_binding_invalid`, so incompatible inputs are never
 * offered for a new binding. A current binding the Recipe does not define, or
 * one that is not a single-file input, stays visible as a retained option with
 * an explanatory label instead of being dropped.
 */
export function inputBindingOptions(
  inputs: Record<string, InputDto>,
  currentInputId: string | null | undefined,
): AppChoiceOption[] {
  const entries = Object.entries(inputs)
    .filter(([, input]) => input.type === "file" && !input.multiple)
    .sort(([left], [right]) => left.localeCompare(right));
  const options: AppChoiceOption[] = [];
  if (currentInputId && !entries.some(([id]) => id === currentInputId)) {
    const declared = Object.prototype.hasOwnProperty.call(inputs, currentInputId)
      ? inputs[currentInputId]
      : undefined;
    options.push({
      value: currentInputId,
      label: declared
        ? declared.label + " (" + currentInputId + ") (not a single-file input)"
        : currentInputId + " (not defined in this Recipe)",
      retained: true,
    });
  }
  for (const [id, input] of entries) {
    options.push({ value: id, label: input.label + " (" + id + ")", retained: false });
  }
  return options;
}


/**
 * A free Recipe artifact id derived from an App Definition artifact id.
 *
 * The seed keeps the App Definition artifact name while it is unused and
 * appends `_2`, `_3`, ... until it is unique, so two Recipe App Artifacts can
 * reference the same App Definition artifact without colliding.
 */
export function uniqueArtifactId(base: string, existingIds: readonly string[]): string {
  const normalized = base.trim();
  if (!existingIds.includes(normalized)) {
    return normalized;
  }
  let suffix = 2;
  while (existingIds.includes(normalized + "_" + suffix)) {
    suffix += 1;
  }
  return normalized + "_" + suffix;
}

/**
 * Whether `launch_app` can resolve a canonical launcher activity.
 *
 * Returns false while the App Definition is unavailable, because a missing
 * projection is reported by app authority diagnostics instead.
 */
export function launcherUnavailable(app: AppCatalogAppDto | null | undefined): boolean {
  return app !== null && app !== undefined && app.launcherActivity === null;
}


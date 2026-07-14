/**
 * Frontend-only accessibility and resilient-interaction helpers.
 *
 * These helpers intentionally know nothing about Tauri handles, filesystem
 * paths, or backend payloads. They coordinate presentation state only.
 */

export const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "summary",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

export interface DialogSnapshot<Payload> {
  id: number;
  payload: Payload;
}

export interface DialogRequest<Result> {
  accepted: boolean;
  id: number | null;
  result: Promise<Result>;
}

interface PendingDialog<Payload, Result> extends DialogSnapshot<Payload> {
  safeResult: Result;
  settled: boolean;
  resolve: (result: Result) => void;
}

/**
 * Owns one promise-backed dialog request at a time.
 *
 * Every request has one resolver and one safe teardown result. A competing
 * request is rejected with its own safe result instead of replacing the live
 * resolver. Settlement, cancellation, and teardown are idempotent.
 */
export class DialogController<Payload, Result> {
  private nextId = 1;
  private pending: PendingDialog<Payload, Result> | null = null;
  private listeners = new Set<(snapshot: DialogSnapshot<Payload> | null) => void>();

  get activeId(): number | null {
    return this.pending?.id ?? null;
  }

  get snapshot(): DialogSnapshot<Payload> | null {
    return this.pending ? { id: this.pending.id, payload: this.pending.payload } : null;
  }

  subscribe(listener: (snapshot: DialogSnapshot<Payload> | null) => void): () => void {
    this.listeners.add(listener);
    listener(this.snapshot);
    return () => this.listeners.delete(listener);
  }

  request(payload: Payload, safeResult: Result): DialogRequest<Result> {
    if (this.pending) {
      return { accepted: false, id: null, result: Promise.resolve(safeResult) };
    }

    const id = this.nextId++;
    let resolver!: (result: Result) => void;
    const result = new Promise<Result>((resolve) => {
      resolver = resolve;
    });
    this.pending = {
      id,
      payload,
      safeResult,
      settled: false,
      resolve: resolver,
    };
    this.emit();
    return { accepted: true, id, result };
  }

  settle(id: number, result: Result): boolean {
    const pending = this.pending;
    if (!pending || pending.id !== id || pending.settled) return false;
    pending.settled = true;
    this.pending = null;
    pending.resolve(result);
    this.emit();
    return true;
  }

  cancelActive(): boolean {
    const pending = this.pending;
    return pending ? this.settle(pending.id, pending.safeResult) : false;
  }

  /** Settle safely without notifying a component that is being unmounted. */
  dispose(): boolean {
    const pending = this.pending;
    if (!pending || pending.settled) {
      this.listeners.clear();
      return false;
    }
    pending.settled = true;
    this.pending = null;
    this.listeners.clear();
    pending.resolve(pending.safeResult);
    return true;
  }

  private emit(): void {
    const snapshot = this.snapshot;
    for (const listener of this.listeners) listener(snapshot);
  }
}

/** Prevents an already-settled request from continuing after its owner tears down. */
export async function lifecycleBoundResult<Result>(
  result: Promise<Result>,
  safeResult: Result,
  generation: number,
  currentGeneration: () => number,
): Promise<Result> {
  const settled = await result;
  return generation === currentGeneration() ? settled : safeResult;
}

let focusTransitionGeneration = 0;

/** Claims ownership of the next asynchronous focus restoration. */
export function claimFocusTransition(): number {
  focusTransitionGeneration += 1;
  return focusTransitionGeneration;
}

export function isCurrentFocusTransition(generation: number): boolean {
  return generation === focusTransitionGeneration;
}

function hasBooleanProperty(element: HTMLElement, property: "disabled" | "hidden" | "inert"): boolean {
  return property in element && Boolean((element as unknown as Record<string, unknown>)[property]);
}

/** Returns whether an element can safely receive programmatic focus now. */
export function isUsableFocusTarget(element: HTMLElement | null | undefined): element is HTMLElement {
  if (!element || typeof element.focus !== "function" || !element.isConnected) return false;
  if (hasBooleanProperty(element, "disabled") || hasBooleanProperty(element, "hidden")) return false;
  if (element.getAttribute?.("aria-disabled") === "true") return false;
  if (element.closest?.("[hidden], [inert], [aria-hidden='true'], fieldset[disabled]")) return false;

  const view = element.ownerDocument?.defaultView;
  if (view?.getComputedStyle) {
    const style = view.getComputedStyle(element);
    if (
      style.display === "none"
      || style.visibility === "hidden"
      || style.visibility === "collapse"
      || style.opacity === "0"
      || style.contentVisibility === "hidden"
    ) {
      return false;
    }
  }
  if (typeof element.getClientRects === "function" && element.getClientRects().length === 0) return false;

  const hasExplicitTabIndex = element.hasAttribute?.("tabindex") ?? false;
  const naturallyFocusable = element.matches?.(FOCUSABLE_SELECTOR) ?? false;
  return naturallyFocusable || hasExplicitTabIndex;
}

export interface FocusRestorationOptions {
  invoker?: HTMLElement | null;
  preferred?: Array<HTMLElement | null | undefined>;
  document?: Document;
  generation?: number;
}

/**
 * Restores focus without ever using document.body as an implicit fallback.
 *
 * The order is the still-valid invoker, transition-specific destinations,
 * workflow fallback, main fallback, and finally the header primary action.
 */
export function restoreAccessibleFocus({
  invoker = null,
  preferred = [],
  document: targetDocument = globalThis.document,
  generation,
}: FocusRestorationOptions): HTMLElement | null {
  if (generation !== undefined && !isCurrentFocusTransition(generation)) return null;

  const selectors = [
    "[data-focus-fallback='workflow']",
    "[data-focus-fallback='main']",
    "#main-content",
    "[data-focus-fallback='header']",
  ];
  const candidates: Array<HTMLElement | null | undefined> = [invoker, ...preferred];
  for (const selector of selectors) {
    candidates.push(targetDocument?.querySelector<HTMLElement>(selector));
  }

  for (const candidate of candidates) {
    if (!isUsableFocusTarget(candidate)) continue;
    candidate.focus({ preventScroll: true });
    return candidate;
  }
  return null;
}

export function stableDomId(prefix: string, value: string, index = 0): string {
  const safe = value.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "item";
  let hash = 2166136261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619);
  }
  return `${prefix}-${index}-${safe}-${(hash >>> 0).toString(36)}`;
}

export function describedBy(...ids: Array<string | null | undefined | false>): string | undefined {
  const present = ids.filter((id): id is string => typeof id === "string" && id.length > 0);
  return present.length > 0 ? present.join(" ") : undefined;
}

export interface ExecutionProgressLike {
  status: string;
  terminal: boolean;
  completion: {
    counts: {
      total: number;
      completed: number;
      skipped: number;
      blocked: number;
      failed: number;
      cancelled: number;
    };
  };
  recipes: Array<{
    name: string;
    steps: Array<{ name: string; status: string }>;
  }>;
}

export interface ExecutionAnnouncement {
  key: string;
  message: string;
  assertive: boolean;
}

/** Coalesces high-frequency snapshots into phase, ten-percent, and terminal announcements. */
export function executionAnnouncement(
  snapshot: ExecutionProgressLike,
  previousKey: string | null,
): ExecutionAnnouncement | null {
  const counts = snapshot.completion.counts;
  const settled = counts.completed + counts.skipped + counts.blocked + counts.failed + counts.cancelled;
  const percentage = counts.total > 0 ? Math.min(100, Math.round((settled / counts.total) * 100)) : 0;
  const bucket = Math.floor(percentage / 10) * 10;
  const activeStep = snapshot.recipes
    .flatMap((recipe) => recipe.steps.map((step) => ({ recipe: recipe.name, ...step })))
    .find((step) => step.status === "running");
  const phase = activeStep ? `${activeStep.recipe}: ${activeStep.name}` : snapshot.status.replaceAll("_", " ");
  const key = snapshot.terminal
    ? `terminal:${snapshot.status}`
    : `progress:${bucket}:${phase}`;
  if (key === previousKey) return null;
  if (snapshot.terminal) {
    return {
      key,
      message: `Execution ${snapshot.status.replaceAll("_", " ")}. ${settled} of ${counts.total} steps settled.`,
      assertive: snapshot.status === "failed" || snapshot.status === "cancelled",
    };
  }
  return {
    key,
    message: counts.total > 0 ? `Execution ${bucket}% complete. ${phase}.` : `Execution in progress. ${phase}.`,
    assertive: false,
  };
}

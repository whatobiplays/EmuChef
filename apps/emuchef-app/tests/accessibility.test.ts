import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  DialogController,
  claimFocusTransition,
  describedBy,
  executionAnnouncement,
  isUsableFocusTarget,
  lifecycleBoundResult,
  restoreAccessibleFocus,
  stableDomId,
} from "../src/accessibility";
import { FrontendErrorFallback } from "../src/ErrorBoundary";

const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("dialog requests settle exactly once", async () => {
  const controller = new DialogController<{ kind: string }, string>();
  const request = controller.request({ kind: "unsaved" }, "cancel");
  assert.equal(request.accepted, true);
  assert.equal(controller.settle(request.id!, "save"), true);
  assert.equal(controller.settle(request.id!, "discard"), false);
  assert.equal(controller.cancelActive(), false);
  assert.equal(await request.result, "save");
});

test("overlapping prompts are rejected without replacing the first resolver", async () => {
  const controller = new DialogController<{ kind: string }, string>();
  const first = controller.request({ kind: "name" }, "first-cancelled");
  const second = controller.request({ kind: "cleanup" }, "second-cancelled");
  assert.equal(first.accepted, true);
  assert.equal(second.accepted, false);
  assert.equal(await second.result, "second-cancelled");
  assert.equal(controller.activeId, first.id);
  controller.settle(first.id!, "first-complete");
  assert.equal(await first.result, "first-complete");
});

test("runtime restart, configuration replacement, and app reset safely cancel pending prompts", async () => {
  for (const teardown of ["runtime restart", "configuration replacement", "app reset"]) {
    const controller = new DialogController<{ teardown: string }, boolean>();
    const request = controller.request({ teardown }, false);
    assert.equal(controller.cancelActive(), true, teardown);
    assert.equal(await request.result, false, teardown);
    assert.equal(controller.activeId, null, teardown);
  }
});

test("component unmount disposal settles pending work without notifying stale listeners", async () => {
  const controller = new DialogController<{ kind: string }, string | null>();
  let notifications = 0;
  controller.subscribe(() => notifications += 1);
  const request = controller.request({ kind: "name" }, null);
  const beforeDispose = notifications;
  assert.equal(controller.dispose(), true);
  assert.equal(await request.result, null);
  assert.equal(notifications, beforeDispose);
  assert.equal(controller.dispose(), false);
});

test("error-boundary activation can cancel an outstanding confirmation safely", async () => {
  const controller = new DialogController<{ kind: string }, "cancel" | "confirm">();
  const request = controller.request({ kind: "real-execution" }, "cancel");
  const onBoundaryActivate = () => controller.cancelActive();
  onBoundaryActivate();
  assert.equal(await request.result, "cancel");
  assert.equal(controller.settle(request.id!, "confirm"), false);
});

test("teardown after explicit settlement suppresses a stale destructive continuation", async () => {
  const controller = new DialogController<{ kind: string }, boolean>();
  let generation = 4;
  const request = controller.request({ kind: "real-execution" }, false);
  const guarded = lifecycleBoundResult(request.result, false, generation, () => generation);
  controller.settle(request.id!, true);
  generation += 1;
  assert.equal(await guarded, false);
});

interface FakeFocusOptions {
  connected?: boolean;
  disabled?: boolean;
  hidden?: boolean;
  inert?: boolean;
  display?: string;
  visibility?: string;
  opacity?: string;
  hasRect?: boolean;
  focusable?: boolean;
  tabIndex?: boolean;
}

function fakeElement(options: FakeFocusOptions = {}) {
  let focusCount = 0;
  const element = {
    isConnected: options.connected ?? true,
    disabled: options.disabled ?? false,
    hidden: options.hidden ?? false,
    focus: () => { focusCount += 1; },
    getAttribute: (name: string) => name === "aria-disabled" && options.disabled ? "true" : null,
    closest: () => options.inert ? element : null,
    hasAttribute: (name: string) => name === "tabindex" && (options.tabIndex ?? true),
    matches: () => options.focusable ?? true,
    ownerDocument: {
      defaultView: {
        getComputedStyle: () => ({
          display: options.display ?? "block",
          visibility: options.visibility ?? "visible",
          opacity: options.opacity ?? "1",
          contentVisibility: "visible",
        }),
      },
    },
    getClientRects: () => options.hasRect === false ? [] : [{}],
    get focusCount() { return focusCount; },
  };
  return element;
}

function fakeDocument(entries: Record<string, ReturnType<typeof fakeElement> | null>) {
  return {
    querySelector: (selector: string) => entries[selector] ?? null,
  } as unknown as Document;
}

test("focus restoration prefers a surviving valid invoker", () => {
  const invoker = fakeElement();
  const workflow = fakeElement();
  const focused = restoreAccessibleFocus({
    invoker: invoker as unknown as HTMLElement,
    document: fakeDocument({ "[data-focus-fallback='workflow']": workflow }),
  });
  assert.equal(focused, invoker);
  assert.equal(invoker.focusCount, 1);
  assert.equal(workflow.focusCount, 0);
});

test("disconnected, hidden, disabled, and inert invokers use the destination fallback", () => {
  for (const invalid of [
    fakeElement({ connected: false }),
    fakeElement({ hidden: true }),
    fakeElement({ disabled: true }),
    fakeElement({ inert: true }),
    fakeElement({ visibility: "hidden" }),
    fakeElement({ opacity: "0" }),
    fakeElement({ hasRect: false }),
  ]) {
    const destination = fakeElement();
    const focused = restoreAccessibleFocus({
      invoker: invalid as unknown as HTMLElement,
      preferred: [destination as unknown as HTMLElement],
      document: fakeDocument({}),
    });
    assert.equal(focused, destination);
    assert.equal(destination.focusCount, 1);
  }
});

test("focus fallback order is workflow, main, then header and never body", () => {
  const main = fakeElement();
  const header = fakeElement();
  const document = fakeDocument({
    "[data-focus-fallback='workflow']": null,
    "[data-focus-fallback='main']": main,
    "#main-content": null,
    "[data-focus-fallback='header']": header,
  });
  assert.equal(restoreAccessibleFocus({ document }), main);
  assert.equal(main.focusCount, 1);
  assert.equal(header.focusCount, 0);
});

test("stale modal and native-dialog restoration cannot steal focus from a newer transition", () => {
  const stale = claimFocusTransition();
  const newer = claimFocusTransition();
  const target = fakeElement();
  assert.notEqual(stale, newer);
  assert.equal(restoreAccessibleFocus({
    preferred: [target as unknown as HTMLElement],
    document: fakeDocument({}),
    generation: stale,
  }), null);
  assert.equal(target.focusCount, 0);
  assert.equal(restoreAccessibleFocus({
    preferred: [target as unknown as HTMLElement],
    document: fakeDocument({}),
    generation: newer,
  }), target);
});

test("programmatic focus targets must be visible, connected, enabled, and explicitly focusable", () => {
  assert.equal(isUsableFocusTarget(fakeElement() as unknown as HTMLElement), true);
  assert.equal(isUsableFocusTarget(fakeElement({ display: "none" }) as unknown as HTMLElement), false);
  assert.equal(isUsableFocusTarget(fakeElement({ focusable: false, tabIndex: false }) as unknown as HTMLElement), false);
});

test("execution announcements are coalesced by phase and ten-percent buckets", () => {
  const snapshot = {
    status: "running",
    terminal: false,
    completion: { counts: { total: 10, completed: 1, skipped: 0, blocked: 0, failed: 0, cancelled: 0 } },
    recipes: [{ name: "Recipe", steps: [{ name: "Install", status: "running" }] }],
  };
  const first = executionAnnouncement(snapshot, null);
  assert.match(first!.message, /10% complete/);
  assert.equal(executionAnnouncement(snapshot, first!.key), null);
  const terminal = executionAnnouncement({ ...snapshot, status: "failed", terminal: true }, first!.key);
  assert.equal(terminal!.assertive, true);
  assert.match(terminal!.message, /failed/);
});

test("stable IDs and composed descriptions remain deterministic", () => {
  assert.equal(stableDomId("input", "recipe/file"), stableDomId("input", "recipe/file"));
  assert.notEqual(stableDomId("input", "recipe/file"), stableDomId("input", "recipe_file"));
  assert.equal(describedBy("description", null, false, "error"), "description error");
  assert.equal(describedBy(null, undefined, false), undefined);
});

test("sanitized error fallback exposes a heading and safe reload without raw error data", () => {
  const markup = renderToStaticMarkup(createElement(FrontendErrorFallback));
  assert.match(markup, /EmuChef could not display this screen/);
  assert.match(markup, /Reload EmuChef safely/);
  assert.doesNotMatch(markup, /stack|serial|handle|\/Users\/|raw sidecar/i);
});

test("source contract includes modal containment and resilient visual settings", () => {
  const app = fs.readFileSync(path.join(appDir, "src/App.tsx"), "utf8");
  const dialog = fs.readFileSync(path.join(appDir, "src/AccessibleDialog.tsx"), "utf8");
  const styles = fs.readFileSync(path.join(appDir, "src/styles.css"), "utf8");
  assert.match(app, /className="skip-link"/);
  assert.match(app, /aria-live="polite"/);
  assert.match(app, /aria-live="assertive"/);
  assert.match(app, /<progress/);
  assert.match(dialog, /aria-modal="true"/);
  assert.match(dialog, /event\.key === "Escape"/);
  assert.match(dialog, /event\.key !== "Tab"/);
  assert.match(styles, /prefers-reduced-motion: reduce/);
  assert.match(styles, /forced-colors: active/);
  assert.match(styles, /max-width: 760px/);
});

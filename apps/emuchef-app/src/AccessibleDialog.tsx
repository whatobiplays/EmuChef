import { useEffect, useRef, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";

import {
  FOCUSABLE_SELECTOR,
  claimFocusTransition,
  isCurrentFocusTransition,
  isUsableFocusTarget,
  restoreAccessibleFocus,
} from "./accessibility";

interface AccessibleDialogProps {
  children: ReactNode;
  titleId: string;
  descriptionId?: string;
  role?: "dialog" | "alertdialog";
  className?: string;
  initialFocusRef?: RefObject<HTMLElement | null>;
  returnFocus?: HTMLElement | null;
  preferredReturnFocus?: Array<HTMLElement | null | undefined>;
  dismissible?: boolean;
  onDismiss?: () => void;
  onDismissBlocked?: () => void;
  currentDialogId?: () => number | null;
  dialogId?: number;
}

let openDialogCount = 0;

function updateBackgroundInert(): void {
  const root = document.getElementById("root");
  if (!root) return;
  if (openDialogCount > 0) {
    root.setAttribute("inert", "");
    root.setAttribute("aria-hidden", "true");
  } else {
    root.removeAttribute("inert");
    root.removeAttribute("aria-hidden");
  }
}

/** Accessible modal containment with deterministic, generation-safe focus restoration. */
export function AccessibleDialog({
  children,
  titleId,
  descriptionId,
  role = "dialog",
  className = "modal-panel",
  initialFocusRef,
  returnFocus,
  preferredReturnFocus = [],
  dismissible = true,
  onDismiss,
  onDismissBlocked,
  currentDialogId,
  dialogId,
}: AccessibleDialogProps) {
  const panelRef = useRef<HTMLElement>(null);
  const focusGenerationRef = useRef(0);

  useEffect(() => {
    focusGenerationRef.current = claimFocusTransition();
    openDialogCount += 1;
    updateBackgroundInert();
    const priorOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    const panel = panelRef.current;
    const initial = initialFocusRef?.current
      ?? panel?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR)
      ?? panel;
    queueMicrotask(() => {
      if (isCurrentFocusTransition(focusGenerationRef.current) && isUsableFocusTarget(initial)) {
        initial.focus({ preventScroll: true });
      }
    });

    return () => {
      openDialogCount = Math.max(0, openDialogCount - 1);
      updateBackgroundInert();
      if (openDialogCount === 0) document.body.style.overflow = priorOverflow;
      queueMicrotask(() => {
        const anotherDialogOwnsFocus = dialogId !== undefined
          && currentDialogId?.() !== null
          && currentDialogId?.() !== dialogId;
        if (anotherDialogOwnsFocus) return;
        restoreAccessibleFocus({
          invoker: returnFocus,
          preferred: preferredReturnFocus,
          generation: focusGenerationRef.current,
        });
      });
    };
  }, []);

  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (dismissible) onDismiss?.();
      else onDismissBlocked?.();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(panelRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR) ?? [])
      .filter(isUsableFocusTarget);
    if (focusable.length === 0) {
      event.preventDefault();
      panelRef.current?.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable.at(-1)!;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div className="modal-backdrop" role="presentation">
      <section
        aria-describedby={descriptionId}
        aria-labelledby={titleId}
        aria-modal="true"
        className={className}
        onKeyDown={onKeyDown}
        ref={panelRef}
        role={role}
        tabIndex={-1}
      >
        {children}
      </section>
    </div>,
    document.body,
  );
}

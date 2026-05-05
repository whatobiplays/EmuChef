import { PointerEvent, ReactNode, useCallback, useEffect, useRef, useState } from "react";

const HANDLE_WIDTH = 8;
const KEYBOARD_STEP = 16;

interface ResizableEditorLayoutProps {
  storageKey: string;
  resizeLabel: string;
  sidebarHeader: ReactNode;
  sidebarBody: ReactNode;
  children: ReactNode;
  defaultSidebarWidth?: number;
  minSidebarWidth?: number;
  maxSidebarWidth?: number;
  minDetailWidth?: number;
}

interface WidthClampOptions {
  minSidebarWidth: number;
  maxSidebarWidth: number;
  containerWidth: number;
  minDetailWidth: number;
  handleWidth: number;
}

interface DragState {
  pointerId: number;
  startX: number;
  startWidth: number;
}

export function ResizableEditorLayout({
  storageKey,
  resizeLabel,
  sidebarHeader,
  sidebarBody,
  children,
  defaultSidebarWidth = 288,
  minSidebarWidth = 224,
  maxSidebarWidth = 520,
  minDetailWidth = 360,
}: ResizableEditorLayoutProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const previousUserSelect = useRef<string | null>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const [dragState, setDragState] = useState<DragState | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() =>
    readStoredSidebarWidth(storageKey, defaultSidebarWidth, {
      minSidebarWidth,
      maxSidebarWidth,
      containerWidth: 0,
      minDetailWidth,
      handleWidth: HANDLE_WIDTH,
    }),
  );
  const maxSidebarWidthForCurrentContainer =
    containerWidth > 0
      ? Math.min(maxSidebarWidth, Math.max(0, containerWidth - minDetailWidth - HANDLE_WIDTH))
      : maxSidebarWidth;
  const minSidebarWidthForCurrentContainer = Math.min(minSidebarWidth, maxSidebarWidthForCurrentContainer);

  const clampWidth = useCallback(
    (width: number, measuredContainerWidth = containerWidth) =>
      clampSidebarWidth(width, {
        minSidebarWidth,
        maxSidebarWidth,
        containerWidth: measuredContainerWidth,
        minDetailWidth,
        handleWidth: HANDLE_WIDTH,
      }),
    [containerWidth, maxSidebarWidth, minDetailWidth, minSidebarWidth],
  );

  const applyWidth = useCallback(
    (width: number, measuredContainerWidth = containerWidth) => {
      const clampedWidth = clampWidth(width, measuredContainerWidth);
      setSidebarWidth(clampedWidth);
      writeStoredSidebarWidth(storageKey, clampedWidth);
    },
    [clampWidth, containerWidth, storageKey],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (container === null) {
      return undefined;
    }

    function updateWidth(width: number) {
      setContainerWidth(width);
      setSidebarWidth((currentWidth) => clampWidth(currentWidth, width));
    }

    updateWidth(container.clientWidth);
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) {
        updateWidth(entry.contentRect.width);
      }
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [clampWidth]);

  useEffect(() => {
    return () => {
      restoreTextSelection();
    };
  }, []);

  function preventTextSelection() {
    if (previousUserSelect.current !== null) {
      return;
    }
    previousUserSelect.current = document.body.style.userSelect;
    document.body.style.userSelect = "none";
  }

  function restoreTextSelection() {
    if (previousUserSelect.current === null) {
      return;
    }
    document.body.style.userSelect = previousUserSelect.current;
    previousUserSelect.current = null;
  }

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    preventTextSelection();
    setDragState({
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: sidebarWidth,
    });
  }

  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    if (dragState === null || dragState.pointerId !== event.pointerId) {
      return;
    }
    applyWidth(dragState.startWidth + event.clientX - dragState.startX);
  }

  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    if (dragState !== null && event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setDragState(null);
    restoreTextSelection();
  }

  function resizeWithKeyboard(delta: number) {
    applyWidth(sidebarWidth + delta);
  }

  return (
    <div
      className="grid h-full min-h-0 overflow-hidden"
      ref={containerRef}
      style={{
        gridTemplateColumns: `${sidebarWidth}px ${HANDLE_WIDTH}px minmax(0, 1fr)`,
      }}
    >
      <section className="flex min-h-0 min-w-0 flex-col bg-white">
        <div className="shrink-0 p-4 pb-3">{sidebarHeader}</div>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">{sidebarBody}</div>
      </section>
      <div
        aria-label={resizeLabel}
        aria-orientation="vertical"
        aria-valuemax={Math.round(maxSidebarWidthForCurrentContainer)}
        aria-valuemin={Math.round(minSidebarWidthForCurrentContainer)}
        aria-valuenow={Math.round(sidebarWidth)}
        className="relative cursor-col-resize border-x border-slate-200 bg-slate-100 outline-none hover:bg-slate-200 focus:bg-slate-200 focus:ring-2 focus:ring-slate-400"
        role="separator"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            resizeWithKeyboard(-KEYBOARD_STEP);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            resizeWithKeyboard(KEYBOARD_STEP);
          }
        }}
        onPointerCancel={onPointerUp}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      >
        <span className="absolute left-1/2 top-0 h-full border-l border-slate-400" />
      </div>
      <section className="min-h-0 min-w-0 overflow-y-auto p-6">{children}</section>
    </div>
  );
}

export function clampSidebarWidth(width: number, options: WidthClampOptions): number {
  const finiteWidth = Number.isFinite(width) ? width : options.minSidebarWidth;
  const sectionClamped = Math.min(Math.max(finiteWidth, options.minSidebarWidth), options.maxSidebarWidth);
  if (options.containerWidth <= 0) {
    return sectionClamped;
  }
  const maxWidthForDetail = Math.max(0, options.containerWidth - options.minDetailWidth - options.handleWidth);
  return Math.min(sectionClamped, maxWidthForDetail);
}

export function parseStoredSidebarWidth(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function readStoredSidebarWidth(
  storageKey: string,
  fallbackWidth: number,
  options: WidthClampOptions,
): number {
  return clampSidebarWidth(readLocalStorageNumber(storageKey) ?? fallbackWidth, options);
}

function readLocalStorageNumber(storageKey: string): number | null {
  try {
    return parseStoredSidebarWidth(window.localStorage.getItem(storageKey));
  } catch {
    return null;
  }
}

function writeStoredSidebarWidth(storageKey: string, width: number) {
  try {
    window.localStorage.setItem(storageKey, String(Math.round(width)));
  } catch {
    // Layout state is a convenience preference; failure should not block editing.
  }
}

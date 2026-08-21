export interface WidthClampOptions {
  minSidebarWidth: number;
  maxSidebarWidth: number;
  containerWidth: number;
  minDetailWidth: number;
  handleWidth: number;
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

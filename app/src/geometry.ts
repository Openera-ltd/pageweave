// Page geometry: display sizes, zoom levels, render-scale buckets and PDF → screen mapping.

import type { PageInfo, SearchRect } from "./types";

/** CSS pixels per PDF point at 100% zoom (96 dpi screen / 72 pt per inch). */
export const CSS_PX_PER_PT = 96 / 72;

export const ZOOM_PRESETS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4] as const;
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 6;

/** Rendering is bucketed so small zoom changes reuse cached images instead of re-rendering. */
export const RENDER_BUCKETS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 2.5, 3, 4, 5, 6, 8] as const;
/** Mirror of the engine's render pixel cap (pw-pdf MAX_RENDER_PIXELS). */
export const MAX_RENDER_PIXELS = 100_000_000;

export function clampZoom(z: number): number {
  if (!Number.isFinite(z)) return 1;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

export function nextZoom(current: number, direction: 1 | -1): number {
  if (direction > 0) {
    return clampZoom(ZOOM_PRESETS.find((p) => p > current + 1e-6) ?? MAX_ZOOM);
  }
  const lower = [...ZOOM_PRESETS].reverse().find((p) => p < current - 1e-6);
  return clampZoom(lower ?? MIN_ZOOM);
}

/** Size of the page as displayed (after its intrinsic /Rotate), in PDF points. */
export function displaySizePt(p: Pick<PageInfo, "width_pt" | "height_pt" | "rotation">): { w: number; h: number } {
  const swap = p.rotation === 90 || p.rotation === 270;
  return swap ? { w: p.height_pt, h: p.width_pt } : { w: p.width_pt, h: p.height_pt };
}

export function displaySizePx(p: Pick<PageInfo, "width_pt" | "height_pt" | "rotation">, zoom: number): { w: number; h: number } {
  const s = displaySizePt(p);
  return { w: Math.round(s.w * zoom * CSS_PX_PER_PT), h: Math.round(s.h * zoom * CSS_PX_PER_PT) };
}

/**
 * Engine render scale (1.0 = 72 dpi) needed for sharp output at `zoom` on a `dpr` display, rounded
 * up to a bucket and reduced if it would exceed the pixel cap. Returned in permille for URLs.
 */
export function renderScalePermille(page: Pick<PageInfo, "width_pt" | "height_pt">, zoom: number, dpr: number): number {
  const needed = zoom * CSS_PX_PER_PT * Math.max(1, dpr || 1);
  let scale: number = RENDER_BUCKETS.find((b) => b >= needed - 1e-6) ?? RENDER_BUCKETS[RENDER_BUCKETS.length - 1]!;
  const area = (s: number) => page.width_pt * s * page.height_pt * s;
  while (area(scale) > MAX_RENDER_PIXELS && scale > 0.05) {
    scale = scale / 2;
  }
  return Math.max(50, Math.round(scale * 1000));
}

/** Zoom that fits the widest page into `containerWidthPx` (minus padding). */
export function fitWidthZoom(pages: PageInfo[], containerWidthPx: number, paddingPx = 32): number {
  if (pages.length === 0 || containerWidthPx <= 0) return 1;
  const widest = Math.max(...pages.map((p) => displaySizePt(p).w));
  return clampZoom((containerWidthPx - paddingPx) / (widest * CSS_PX_PER_PT));
}

export interface ScreenRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Map a rectangle in unrotated PDF user space (origin bottom-left) to CSS pixels inside the
 * displayed page box (origin top-left), honouring the page's clockwise /Rotate.
 */
export function pdfRectToScreen(
  r: SearchRect,
  page: Pick<PageInfo, "width_pt" | "height_pt" | "rotation">,
  zoom: number,
): ScreenRect {
  const k = zoom * CSS_PX_PER_PT;
  const w = page.width_pt;
  const h = page.height_pt;
  const corners: Array<[number, number]> = [
    [r.left, r.bottom],
    [r.right, r.top],
  ];
  const mapped = corners.map(([x, y]) => {
    switch (page.rotation) {
      case 90:
        return [y, x];
      case 180:
        return [w - x, y];
      case 270:
        return [h - y, w - x];
      default:
        return [x, h - y];
    }
  });
  const xs = mapped.map((m) => m[0]!);
  const ys = mapped.map((m) => m[1]!);
  const left = Math.min(...xs);
  const top = Math.min(...ys);
  return { left: left * k, top: top * k, width: (Math.max(...xs) - left) * k, height: (Math.max(...ys) - top) * k };
}

/** Index of the page whose box contains the most visible area for the given scroll window. */
export function mostVisiblePage(tops: number[], heights: number[], viewTop: number, viewHeight: number): number {
  let best = 0;
  let bestArea = -1;
  const viewBottom = viewTop + viewHeight;
  for (let i = 0; i < tops.length; i++) {
    const t = tops[i]!;
    const b = t + heights[i]!;
    const visible = Math.max(0, Math.min(b, viewBottom) - Math.max(t, viewTop));
    if (visible > bestArea) {
      best = i;
      bestArea = visible;
    }
  }
  return best;
}

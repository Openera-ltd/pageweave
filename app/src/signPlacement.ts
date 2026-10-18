// Where a visible digital signature goes: a fixed-size box in a corner of the page as displayed,
// converted to the page's unrotated PDF space (what the engine expects). Pure; unit tested.

import { CSS_PX_PER_PT, displaySizePt, screenRectToPdf } from "./geometry";
import type { PageInfo } from "./types";

export const SIGN_POSITIONS = [
  { id: "bottom-right", label: "Bottom right" },
  { id: "bottom-left", label: "Bottom left" },
  { id: "top-right", label: "Top right" },
  { id: "top-left", label: "Top left" },
] as const;

export type SignPosition = (typeof SIGN_POSITIONS)[number]["id"];

/** Box size and margin as displayed (points). */
export const SIGN_BOX = { w: 200, h: 60, margin: 36 };

/** [x1, y1, x2, y2] in unrotated PDF space for a corner box on `page` as it is displayed. */
export function signatureRect(
  page: Pick<PageInfo, "width_pt" | "height_pt" | "rotation" | "origin_x" | "origin_y">,
  position: SignPosition,
  size: { w: number; h: number } = SIGN_BOX,
): [number, number, number, number] {
  const d = displaySizePt(page);
  // Shrink for small pages so the box always fits inside the margins.
  const w = Math.min(size.w, d.w - 2 * Math.min(SIGN_BOX.margin, d.w / 10));
  const h = Math.min(size.h, d.h - 2 * Math.min(SIGN_BOX.margin, d.h / 10));
  const mx = Math.min(SIGN_BOX.margin, (d.w - w) / 2);
  const my = Math.min(SIGN_BOX.margin, (d.h - h) / 2);
  const left = position.endsWith("left") ? mx : d.w - mx - w;
  const top = position.startsWith("top") ? my : d.h - my - h;
  // Displayed points → CSS px at zoom 1 → unrotated PDF space.
  const k = CSS_PX_PER_PT;
  const r = screenRectToPdf({ left: left * k, top: top * k, width: w * k, height: h * k }, page, 1);
  const round = (v: number) => Math.round(v * 100) / 100;
  return [round(r.left), round(r.bottom), round(r.right), round(r.top)];
}

// Compare view model: the engine's `compare` report flattened into a navigable difference list,
// plus labels and geometry helpers. Pure functions (unit tested); the view lives in CompareView.

import { displaySizePt, pdfRectToScreen } from "./geometry";
import type { PageInfo } from "./types";

export interface RectPt {
  left: number;
  bottom: number;
  right: number;
  top: number;
}

export type ChangeKind = "inserted" | "deleted" | "replaced" | "moved" | "format" | "visual" | "page_size";
export type PairStatus = "same" | "changed" | "inserted" | "deleted" | "moved";

export interface Change {
  kind: ChangeKind;
  text: string;
  new_text?: string;
  detail?: string;
  old_rects: RectPt[];
  new_rects: RectPt[];
}

export interface PagePair {
  /** 0-based page indexes; null where the page does not exist. */
  old: number | null;
  new: number | null;
  status: PairStatus;
  changes: Change[];
}

export interface CompareSummary {
  pages_changed: number;
  pages_inserted: number;
  pages_deleted: number;
  pages_moved: number;
  text_changes: number;
  format_changes: number;
  visual_changes: number;
}

export interface CompareReport {
  old_pages: number;
  new_pages: number;
  pairs: PagePair[];
  metadata: Array<{ field: string; old: string; new: string }>;
  summary: CompareSummary;
  notes: string[];
}

export type ItemKind = ChangeKind | "page_inserted" | "page_deleted" | "page_moved";

export interface DiffItem {
  key: string;
  /** Index into `report.pairs`. */
  pair: number;
  /** Index into the pair's changes; null for a whole-page item. */
  change: number | null;
  kind: ItemKind;
  text: string;
  where: string;
}

export const KIND_LABEL: Record<ItemKind, string> = {
  inserted: "Inserted",
  deleted: "Deleted",
  replaced: "Replaced",
  moved: "Moved",
  format: "Format",
  visual: "Visual",
  page_size: "Page size",
  page_inserted: "Page inserted",
  page_deleted: "Page deleted",
  page_moved: "Page moved",
};

const MAX_TEXT = 140;

function clip(s: string): string {
  return s.length > MAX_TEXT ? `${s.slice(0, MAX_TEXT - 1)}…` : s;
}

/** "Page 2 → 3", "Old page 3", "New page 4". */
export function pairLabel(p: Pick<PagePair, "old" | "new">): string {
  if (p.old != null && p.new != null) return p.old === p.new ? `Page ${p.new + 1}` : `Page ${p.old + 1} → ${p.new + 1}`;
  if (p.old != null) return `Old page ${p.old + 1}`;
  return `New page ${(p.new ?? 0) + 1}`;
}

function changeText(c: Change): string {
  if (c.kind === "replaced") return clip(`“${c.text}” → “${c.new_text ?? ""}”`);
  if (c.kind === "visual") return "Content differs (not text)";
  if (c.kind === "page_size") return c.detail ?? "Page size or orientation differs";
  const base = c.text ? `“${clip(c.text)}”` : "";
  return c.detail ? `${base} ${c.detail}`.trim() : base;
}

/** Every difference in page order: whole-page items first within a pair, then its changes. */
export function diffItems(r: CompareReport): DiffItem[] {
  const out: DiffItem[] = [];
  r.pairs.forEach((p, i) => {
    const where = pairLabel(p);
    if (p.status === "inserted" || p.status === "deleted" || p.status === "moved") {
      const kind: ItemKind = p.status === "inserted" ? "page_inserted" : p.status === "deleted" ? "page_deleted" : "page_moved";
      const text =
        p.status === "moved" && p.old != null && p.new != null
          ? `Old page ${p.old + 1} is now page ${p.new + 1}`
          : p.status === "inserted"
            ? "Only in the new document"
            : "Only in the old document";
      out.push({ key: `${i}`, pair: i, change: null, kind, text, where });
    }
    p.changes.forEach((c, j) => out.push({ key: `${i}.${j}`, pair: i, change: j, kind: c.kind, text: changeText(c), where }));
  });
  return out;
}

export function summaryText(s: CompareSummary): string {
  const parts: string[] = [];
  const n = (k: number, one: string, many: string) => k > 0 && parts.push(`${k} ${k === 1 ? one : many}`);
  n(s.pages_changed, "page changed", "pages changed");
  n(s.pages_inserted, "page inserted", "pages inserted");
  n(s.pages_deleted, "page deleted", "pages deleted");
  n(s.pages_moved, "page moved", "pages moved");
  return parts.length ? parts.join(" · ") : "No differences found";
}

/** A rectangle as percentages of the displayed page (so overlays follow any image size). */
export function rectPercent(r: RectPt, page: Pick<PageInfo, "width_pt" | "height_pt" | "rotation" | "origin_x" | "origin_y">) {
  const s = pdfRectToScreen(r, page, 1);
  const size = displaySizePt(page);
  const k = 96 / 72;
  const w = size.w * k;
  const h = size.h * k;
  // At least a thin visible box for zero-size rectangles.
  return {
    left: (100 * s.left) / w,
    top: (100 * s.top) / h,
    width: Math.max(0.4, (100 * s.width) / w),
    height: Math.max(0.4, (100 * s.height) / h),
  };
}

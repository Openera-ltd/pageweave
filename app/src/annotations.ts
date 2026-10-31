// Pending (unsaved) annotations: turning a text selection into PDF rectangles, merging them per line,
// and the undo/redo history. Saving goes through the engine's typed `annotate` operation.

import { displaySizePt, pageOrigin, screenRectToPdf, CSS_PX_PER_PT, type ScreenRect } from "./geometry";
import type { AnnotationKind, AnnotationSpec, PageInfo, RedactArea, SearchRect } from "./types";

export const ANNOTATION_COLORS: Record<AnnotationKind, string> = {
  highlight: "#ffd400",
  underline: "#1f6feb",
  strike_out: "#d1242f",
  note: "#ffd400",
};

/** The colour new markup of `kind` gets: the user's choice when it is a valid `#rrggbb`, otherwise the built-in one. */
export function markupColor(chosen: Partial<Record<AnnotationKind, string>> | undefined, kind: AnnotationKind): string {
  const c = chosen?.[kind];
  return c && /^#[0-9a-f]{6}$/i.test(c) ? c.toLowerCase() : ANNOTATION_COLORS[kind];
}

/**
 * Pending items share one undo history. `redact` is UI-only: an area marked for redaction, applied
 * with the engine's `redact` operation and never written as an annotation.
 */
export type PendingKind = AnnotationKind | "redact";

export const KIND_LABEL: Record<PendingKind, string> = {
  highlight: "Highlight",
  underline: "Underline",
  strike_out: "Strikethrough",
  note: "Note",
  redact: "Marked for redaction",
};

export const REDACT_MARK_COLOR = "#d1242f";

/** Side of the square a note marker occupies, in PDF points (matches the engine's icon box). */
export const NOTE_ICON_PT = 18;

export interface PendingAnnotation extends Omit<AnnotationSpec, "kind"> {
  id: number;
  kind: PendingKind;
}

export const isRedactMark = (a: Pick<PendingAnnotation, "kind">): boolean => a.kind === "redact";

/** The pending annotations to write into a saved copy (redaction marks excluded), as the engine contract. */
export function annotationSpecs(items: PendingAnnotation[]): AnnotationSpec[] {
  return items
    .filter((a) => !isRedactMark(a))
    .map(({ page, kind, rects, color, contents }) => ({ page, kind: kind as AnnotationKind, rects, color, contents: contents ?? null }));
}

/** Redaction areas of the pending marks, one per rectangle. */
export function redactAreas(items: PendingAnnotation[]): RedactArea[] {
  return items.filter(isRedactMark).flatMap((a) => a.rects.map((rect) => ({ page: a.page, rect })));
}

/** Remove one item (e.g. a single redaction mark); the redo list is kept. */
export function removeItem(h: AnnotationHistory, id: number): AnnotationHistory {
  return { ...h, items: h.items.filter((a) => a.id !== id) };
}

/** Pending items on `page` whose rectangles contain the PDF point (x, y). */
export function itemsAt(items: PendingAnnotation[], page: number, x: number, y: number): PendingAnnotation[] {
  return items.filter((a) => a.page === page && a.rects.some((r) => x >= r.left && x <= r.right && y >= r.bottom && y <= r.top));
}

export interface AnnotationHistory {
  items: PendingAnnotation[];
  /** Undone annotations, most recent last; cleared by any new annotation. */
  redo: PendingAnnotation[];
}

export const EMPTY_HISTORY: AnnotationHistory = { items: [], redo: [] };

export function addAnnotations(h: AnnotationHistory, added: PendingAnnotation[]): AnnotationHistory {
  return added.length ? { items: [...h.items, ...added], redo: [] } : h;
}

/** Undo the most recent action. Annotations created together (one selection spanning pages) undo together. */
export function undo(h: AnnotationHistory): AnnotationHistory {
  const last = h.items[h.items.length - 1];
  if (!last) return h;
  const group = groupOf(last.id);
  const keep = h.items.filter((a) => groupOf(a.id) !== group);
  const removed = h.items.filter((a) => groupOf(a.id) === group);
  return { items: keep, redo: [...h.redo, ...removed] };
}

export function redo(h: AnnotationHistory): AnnotationHistory {
  const last = h.redo[h.redo.length - 1];
  if (!last) return h;
  const group = groupOf(last.id);
  const back = h.redo.filter((a) => groupOf(a.id) === group);
  return { items: [...h.items, ...back], redo: h.redo.filter((a) => groupOf(a.id) !== group) };
}

/** Ids are `group * 1000 + n`, so one user action can add up to 1000 per-page annotations. */
export function makeId(group: number, n: number): number {
  return group * 1000 + n;
}
function groupOf(id: number): number {
  return Math.floor(id / 1000);
}

/** Merge rectangles that sit on the same text line and touch or overlap (PDF space, y up). */
export function mergeLineRects(rects: SearchRect[]): SearchRect[] {
  const out: SearchRect[] = [];
  const sorted = [...rects].sort((a, b) => b.top - a.top || a.left - b.left);
  for (const r of sorted) {
    const h = r.top - r.bottom;
    const prev = out.find((o) => {
      const overlap = Math.min(o.top, r.top) - Math.max(o.bottom, r.bottom);
      const minH = Math.min(o.top - o.bottom, h);
      const gap = Math.max(o.left, r.left) - Math.min(o.right, r.right);
      return overlap >= 0.5 * minH && gap <= 0.3 * Math.max(minH, 1);
    });
    if (prev) {
      prev.left = Math.min(prev.left, r.left);
      prev.right = Math.max(prev.right, r.right);
      prev.bottom = Math.min(prev.bottom, r.bottom);
      prev.top = Math.max(prev.top, r.top);
    } else {
      out.push({ ...r });
    }
  }
  return out;
}

export interface PageBox {
  page: number;
  /** Viewport rectangle of the displayed page element. */
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Assign viewport rectangles (from a selection) to the page they lie on, clip them to that page and
 * convert them to PDF space. The zoom is derived from the page element's measured width so UI
 * scaling and rounding cannot skew the result. Tiny fragments (line breaks, empty runs) are dropped.
 */
export function clientRectsToPageRects(
  rects: Array<{ left: number; top: number; width: number; height: number }>,
  boxes: PageBox[],
  pages: PageInfo[],
): Map<number, SearchRect[]> {
  const out = new Map<number, SearchRect[]>();
  for (const r of rects) {
    if (r.width < 1 || r.height < 1) continue;
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const box = boxes.find((b) => cx >= b.left && cx <= b.left + b.width && cy >= b.top && cy <= b.top + b.height);
    const page = box && pages[box.page];
    if (!box || !page) continue;
    const left = Math.max(r.left, box.left) - box.left;
    const top = Math.max(r.top, box.top) - box.top;
    const right = Math.min(r.left + r.width, box.left + box.width) - box.left;
    const bottom = Math.min(r.top + r.height, box.top + box.height) - box.top;
    if (right - left < 1 || bottom - top < 1) continue;
    const zoom = box.width / (displaySizePt(page).w * CSS_PX_PER_PT);
    const local: ScreenRect = { left, top, width: right - left, height: bottom - top };
    const list = out.get(box.page) ?? [];
    list.push(screenRectToPdf(local, page, zoom));
    out.set(box.page, list);
  }
  for (const [p, list] of out) out.set(p, mergeLineRects(list));
  return out;
}

/** Square note marker anchored at a PDF point (its top-left corner), kept inside the page. */
export function noteRect(x: number, y: number, page: Pick<PageInfo, "width_pt" | "height_pt" | "origin_x" | "origin_y">): SearchRect {
  const o = pageOrigin(page);
  const left = Math.min(Math.max(o.x, x), o.x + Math.max(0, page.width_pt - NOTE_ICON_PT));
  const top = Math.max(Math.min(o.y + page.height_pt, y), o.y + Math.min(page.height_pt, NOTE_ICON_PT));
  return { left, bottom: top - NOTE_ICON_PT, right: left + NOTE_ICON_PT, top };
}

/**
 * Viewport rectangles of the current selection's text, limited to page text layers. Walks the
 * selected text nodes and measures only the selected part of each, so element boxes (a whole page
 * layer when everything is selected) never become annotation areas.
 */
export function selectionClientRects(sel: Selection | null): DOMRect[] {
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return [];
  const out: DOMRect[] = [];
  for (let i = 0; i < sel.rangeCount; i++) {
    const range = sel.getRangeAt(i);
    const root = range.commonAncestorContainer;
    const walker = document.createTreeWalker(root.nodeType === Node.TEXT_NODE ? root.parentNode ?? root : root, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!range.intersectsNode(n) || !n.parentElement?.closest(".textlayer")) continue;
      const text = n as Text;
      const sub = document.createRange();
      sub.setStart(text, n === range.startContainer ? range.startOffset : 0);
      sub.setEnd(text, n === range.endContainer ? range.endOffset : text.length);
      if (sub.collapsed) continue;
      out.push(...Array.from(sub.getClientRects()));
    }
  }
  return out;
}

/** Viewport boxes of the rendered pages currently in the DOM. */
export function pageBoxes(root: ParentNode = document): PageBox[] {
  return Array.from(root.querySelectorAll<HTMLElement>(".viewer [data-page]")).map((el) => {
    const r = el.getBoundingClientRect();
    return { page: Number(el.dataset.page), left: r.left, top: r.top, width: r.width, height: r.height };
  });
}

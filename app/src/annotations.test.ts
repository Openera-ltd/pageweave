import { describe, expect, it } from "vitest";
import { addAnnotations, clientRectsToPageRects, EMPTY_HISTORY, makeId, mergeLineRects, noteRect, redo, undo, type PendingAnnotation } from "./annotations";
import { pdfRectToScreen, screenRectToPdf } from "./geometry";
import type { PageInfo } from "./types";

const page = (rotation: PageInfo["rotation"]): PageInfo => ({ index: 0, width_pt: 612, height_pt: 792, rotation, label: null });

describe("screen → pdf mapping is the inverse of pdf → screen", () => {
  for (const rot of [0, 90, 180, 270] as const) {
    it(`round-trips at ${rot}° and several zooms`, () => {
      const r = { left: 72, bottom: 600, right: 300, top: 614 };
      for (const zoom of [0.5, 1, 1.75]) {
        const back = screenRectToPdf(pdfRectToScreen(r, page(rot), zoom), page(rot), zoom);
        expect(back.left).toBeCloseTo(r.left, 6);
        expect(back.bottom).toBeCloseTo(r.bottom, 6);
        expect(back.right).toBeCloseTo(r.right, 6);
        expect(back.top).toBeCloseTo(r.top, 6);
      }
    });
  }

  it("top-left of an unrotated page is the PDF top-left corner", () => {
    const r = screenRectToPdf({ left: 0, top: 0, width: 10, height: 10 }, page(0), 1);
    expect(r.top).toBeCloseTo(792);
    expect(r.left).toBeCloseTo(0);
  });
});

describe("merging selection fragments", () => {
  it("joins touching fragments on one line and keeps separate lines apart", () => {
    const line1a = { left: 72, bottom: 700, right: 120, top: 712 };
    const line1b = { left: 121, bottom: 700.5, right: 200, top: 712 };
    const line2 = { left: 72, bottom: 680, right: 150, top: 692 };
    const merged = mergeLineRects([line2, line1b, line1a]);
    expect(merged).toHaveLength(2);
    expect(merged[0]).toEqual({ left: 72, bottom: 700, right: 200, top: 712 });
    expect(merged[1]).toEqual(line2);
  });

  it("does not join words far apart on the same line", () => {
    const merged = mergeLineRects([
      { left: 72, bottom: 700, right: 100, top: 712 },
      { left: 400, bottom: 700, right: 450, top: 712 },
    ]);
    expect(merged).toHaveLength(2);
  });

  it("collapses duplicates", () => {
    const r = { left: 72, bottom: 700, right: 100, top: 712 };
    expect(mergeLineRects([r, { ...r }])).toEqual([r]);
  });
});

describe("selection rectangles → per-page PDF rectangles", () => {
  const pages = [page(0), { ...page(90), index: 1 }];
  // Page 0 displayed at zoom 1 (816×1056 px) at (100, 50); page 1 rotated, 1056×816 px below it.
  const boxes = [
    { page: 0, left: 100, top: 50, width: 816, height: 1056 },
    { page: 1, left: 100, top: 1122, width: 1056, height: 816 },
  ];

  it("assigns rectangles to pages, clips them and drops slivers", () => {
    const out = clientRectsToPageRects(
      [
        { left: 196, top: 150, width: 100, height: 16 }, // page 0
        { left: 860, top: 150, width: 100, height: 16 }, // runs off the right edge of page 0
        { left: 200, top: 300, width: 0.4, height: 16 }, // line-break sliver
        { left: 300, top: 1200, width: 16, height: 80 }, // page 1 (rotated: vertical on screen)
        { left: 5, top: 5, width: 20, height: 20 }, // outside every page
      ],
      boxes,
      pages,
    );
    expect([...out.keys()].sort()).toEqual([0, 1]);
    const p0 = out.get(0)!;
    expect(p0).toHaveLength(2);
    const k = 96 / 72;
    expect(p0[0]!.left).toBeCloseTo(96 / k);
    expect(p0[0]!.top).toBeCloseTo(792 - 100 / k);
    expect(p0.every((r) => r.right <= 612 + 1e-6)).toBe(true);
    const p1 = out.get(1)![0]!;
    // On a 90° page, screen x maps to PDF y and screen y to PDF x.
    expect(p1.right - p1.left).toBeCloseTo(80 / k);
    expect(p1.top - p1.bottom).toBeCloseTo(16 / k);
  });

  it("derives zoom from the measured page width", () => {
    const half = [{ page: 0, left: 0, top: 0, width: 408, height: 528 }];
    const r = clientRectsToPageRects([{ left: 0, top: 0, width: 408, height: 528 }], half, pages).get(0)![0]!;
    expect(r.right).toBeCloseTo(612);
    expect(r.bottom).toBeCloseTo(0);
  });
});

describe("note marker", () => {
  it("stays inside the page", () => {
    const p = page(0);
    expect(noteRect(605, 5, p)).toEqual({ left: 594, bottom: 0, right: 612, top: 18 });
    expect(noteRect(10, 700, p)).toEqual({ left: 10, bottom: 682, right: 28, top: 700 });
  });
});

describe("undo / redo history", () => {
  const ann = (id: number, page = 0): PendingAnnotation => ({ id, page, kind: "highlight", rects: [{ left: 0, bottom: 0, right: 1, top: 1 }], color: "#ffd400" });

  it("undoes and redoes whole actions, and a new action clears redo", () => {
    let h = addAnnotations(EMPTY_HISTORY, [ann(makeId(1, 0))]);
    h = addAnnotations(h, [ann(makeId(2, 0), 0), ann(makeId(2, 1), 1)]); // one selection across two pages
    expect(h.items).toHaveLength(3);
    h = undo(h);
    expect(h.items.map((a) => a.id)).toEqual([1000]);
    expect(h.redo).toHaveLength(2);
    h = redo(h);
    expect(h.items).toHaveLength(3);
    h = undo(undo(h));
    expect(h.items).toHaveLength(0);
    expect(undo(h)).toBe(h);
    h = redo(h);
    expect(h.items.map((a) => a.id)).toEqual([1000]);
    h = addAnnotations(h, [ann(makeId(3, 0))]);
    expect(h.redo).toHaveLength(0);
    expect(redo(h)).toBe(h);
  });
});

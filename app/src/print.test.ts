import { describe, expect, it } from "vitest";
import { MAX_PRINT_MEGAPIXELS, printPlan } from "./print";
import type { PageInfo } from "./types";

const page = (index: number, w: number, h: number, rotation: 0 | 90 | 180 | 270 = 0): PageInfo => ({ index, width_pt: w, height_pt: h, rotation, label: null });

describe("print plan", () => {
  it("gives every sheet its displayed page size and shares @page rules between equal sizes", () => {
    const pages = [page(0, 612, 792), page(1, 595.276, 841.89), page(2, 612, 792, 90)];
    const plan = printPlan(pages, [0, 1, 2], 150);
    expect(plan.sheets.map((s) => [s.widthPt, s.heightPt])).toEqual([
      [612, 792],
      [595.28, 841.89],
      [792, 612],
    ]);
    expect(new Set(plan.sheets.map((s) => s.pageName)).size).toBe(3);
    expect(plan.css).toContain("size: 612pt 792pt; margin: 0;");
    expect(plan.css).toContain("size: 792pt 612pt");
    expect(printPlan([page(0, 612, 792), page(1, 612, 792)], [0, 1], 150).css.split("\n")).toHaveLength(1);
  });

  it("only includes chosen pages and sizes the render for the resolution", () => {
    const pages = [page(0, 612, 792), page(1, 612, 792), page(2, 612, 792)];
    const plan = printPlan(pages, [2, 0], 300);
    expect(plan.sheets.map((s) => s.index)).toEqual([2, 0]);
    expect(plan.scalePermille).toBe(4167);
    // 2550 x 3300 pixels per Letter page at 300 dpi.
    expect(plan.megapixels).toBeCloseTo((2 * 2550 * 3300) / 1e6, 3);
  });

  it("the memory guard allows a normal document and refuses an oversized job", () => {
    const pages = Array.from({ length: 500 }, (_, i) => page(i, 612, 792));
    const all = pages.map((p) => p.index);
    expect(printPlan(pages, all.slice(0, 40), 300).megapixels).toBeLessThan(MAX_PRINT_MEGAPIXELS);
    expect(printPlan(pages, all, 150).megapixels).toBeGreaterThan(MAX_PRINT_MEGAPIXELS);
  });
});

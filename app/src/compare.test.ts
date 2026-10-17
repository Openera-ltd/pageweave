import { describe, expect, it } from "vitest";
import { diffItems, pairLabel, rectPercent, summaryText, type CompareReport } from "./compare";

const report: CompareReport = {
  old_pages: 3,
  new_pages: 3,
  pairs: [
    { old: 0, new: 0, status: "same", changes: [] },
    {
      old: 1,
      new: 1,
      status: "changed",
      changes: [
        { kind: "replaced", text: "quarterly", new_text: "annual", old_rects: [], new_rects: [] },
        { kind: "format", text: "Revenue grew", detail: "Helvetica 12.0 pt → Helvetica-Bold 12.0 pt", old_rects: [], new_rects: [] },
        { kind: "visual", text: "", old_rects: [], new_rects: [] },
      ],
    },
    { old: 2, new: null, status: "deleted", changes: [] },
    { old: null, new: 2, status: "inserted", changes: [] },
  ],
  metadata: [],
  summary: { pages_changed: 1, pages_inserted: 1, pages_deleted: 1, pages_moved: 0, text_changes: 1, format_changes: 1, visual_changes: 1 },
  notes: [],
};

describe("compare model", () => {
  it("flattens pairs into a difference list in page order", () => {
    const items = diffItems(report);
    expect(items.map((i) => i.kind)).toEqual(["replaced", "format", "visual", "page_deleted", "page_inserted"]);
    expect(items[0]!.text).toBe("“quarterly” → “annual”");
    expect(items[1]!.text).toContain("Helvetica-Bold");
    expect(items.map((i) => i.where)).toEqual(["Page 2", "Page 2", "Page 2", "Old page 3", "New page 3"]);
    expect(new Set(items.map((i) => i.key)).size).toBe(items.length);
  });

  it("labels pages and summaries", () => {
    expect(pairLabel({ old: 3, new: 4 })).toBe("Page 4 → 5");
    expect(summaryText(report.summary)).toBe("1 page changed · 1 page inserted · 1 page deleted");
    expect(summaryText({ ...report.summary, pages_changed: 0, pages_inserted: 0, pages_deleted: 0 })).toBe("No differences found");
  });

  it("places rectangles as percentages of the displayed page, rotation included", () => {
    const page = { width_pt: 600, height_pt: 800, rotation: 0 as const };
    expect(rectPercent({ left: 60, bottom: 700, right: 300, top: 720 }, page)).toEqual({ left: 10, top: 10, width: 40, height: 2.5 });
    // On a page turned 90°, the page's bottom-left corner shows at the top-left.
    const r = rectPercent({ left: 0, bottom: 0, right: 80, top: 60 }, { ...page, rotation: 90 as const });
    expect(r.left).toBeCloseTo(0);
    expect(r.top).toBeCloseTo(0);
    expect(r.width).toBeCloseTo(7.5);
    expect(r.height).toBeCloseTo(13.333, 2);
  });
});

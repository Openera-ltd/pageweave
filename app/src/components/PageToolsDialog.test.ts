import { describe, expect, it } from "vitest";
import { UNIT_PT, displayedSize, expandStamp, stampCharsOk } from "./PageToolsDialog";

describe("stamp preview", () => {
  it("expands the template like the engine", () => {
    expect(expandStamp("Page {page} of {total}", 3, 0, 12)).toBe("Page 3 of 12");
    expect(expandStamp("ACME{page}", 41, 6, 5)).toBe("ACME000041");
    expect(expandStamp("{page}/{page}", 7, 2, 9)).toBe("07/07");
  });

  it("accepts Latin text and refuses characters the built-in font cannot draw", () => {
    expect(stampCharsOk("Seite {page} – ü")).toBe(false); // en dash is outside Latin-1
    expect(stampCharsOk("Seite {page} - über")).toBe(true);
    expect(stampCharsOk("页 {page}")).toBe(false);
  });
});


describe("crop helpers", () => {
  it("converts units to points", () => {
    expect(10 * UNIT_PT.mm).toBeCloseTo(28.3465, 3);
    expect(UNIT_PT.in).toBe(72);
    expect(UNIT_PT.pt).toBe(1);
  });
  it("uses the displayed size of rotated pages", () => {
    expect(displayedSize({ width_pt: 612, height_pt: 792, rotation: 0 })).toEqual([612, 792]);
    expect(displayedSize({ width_pt: 612, height_pt: 792, rotation: 90 })).toEqual([792, 612]);
    expect(displayedSize({ width_pt: 612, height_pt: 792, rotation: 270 })).toEqual([792, 612]);
  });
});

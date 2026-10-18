import { describe, expect, it } from "vitest";
import { pdfRectToScreen } from "./geometry";
import { signatureRect, SIGN_BOX } from "./signPlacement";
import type { PageInfo } from "./types";

const page = (w: number, h: number, rotation: 0 | 90 | 180 | 270 = 0): PageInfo => ({ index: 0, width_pt: w, height_pt: h, rotation, label: null });

describe("visible signature placement", () => {
  it("puts the box in the chosen corner of an upright Letter page", () => {
    expect(signatureRect(page(612, 792), "bottom-right")).toEqual([376, 36, 576, 96]);
    expect(signatureRect(page(612, 792), "top-left")).toEqual([36, 696, 236, 756]);
  });

  it("follows the page as displayed on rotated pages", () => {
    for (const rotation of [90, 180, 270] as const) {
      const p = page(612, 792, rotation);
      const [x1, y1, x2, y2] = signatureRect(p, "bottom-right");
      // Back on screen (zoom 1, CSS px), the box sits at the displayed bottom-right corner.
      const s = pdfRectToScreen({ left: x1, bottom: y1, right: x2, top: y2 }, p, 1);
      const k = 96 / 72;
      const dw = rotation === 180 ? 612 : 792;
      const dh = rotation === 180 ? 792 : 612;
      expect(s.left / k).toBeCloseTo(dw - SIGN_BOX.margin - SIGN_BOX.w, 1);
      expect(s.top / k).toBeCloseTo(dh - SIGN_BOX.margin - SIGN_BOX.h, 1);
      expect(s.width / k).toBeCloseTo(SIGN_BOX.w, 1);
    }
  });

  it("shrinks to fit small pages and stays inside them", () => {
    const [x1, y1, x2, y2] = signatureRect(page(150, 80), "bottom-left");
    expect(x1).toBeGreaterThanOrEqual(0);
    expect(y1).toBeGreaterThanOrEqual(0);
    expect(x2).toBeLessThanOrEqual(150);
    expect(y2).toBeLessThanOrEqual(80);
    expect(x2 - x1).toBeGreaterThanOrEqual(20);
    expect(y2 - y1).toBeGreaterThanOrEqual(10);
  });
});

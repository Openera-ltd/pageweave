import { describe, expect, it } from "vitest";
import {
  CSS_PX_PER_PT,
  clampZoom,
  displaySizePt,
  fitWidthZoom,
  mostVisiblePage,
  nextZoom,
  pdfRectToScreen,
  screenRectToPdf,
  renderScalePermille,
  unrotatedLayer,
  applyLayerTransform,
} from "./geometry";
import type { PageInfo } from "./types";

const letter = (rotation: PageInfo["rotation"] = 0): PageInfo => ({ index: 0, width_pt: 612, height_pt: 792, rotation, label: null });

describe("zoom", () => {
  it("clamps and steps through presets", () => {
    expect(clampZoom(NaN)).toBe(1);
    expect(clampZoom(100)).toBe(6);
    expect(nextZoom(1, 1)).toBe(1.25);
    expect(nextZoom(1, -1)).toBe(0.75);
    expect(nextZoom(1.1, 1)).toBe(1.25);
    expect(nextZoom(1.1, -1)).toBe(1);
    expect(nextZoom(4, 1)).toBe(6);
    expect(nextZoom(0.25, -1)).toBe(0.1);
  });

  it("fit width uses the widest displayed page", () => {
    const z = fitWidthZoom([letter(), letter(90)], 1000, 0);
    expect(z).toBeCloseTo(1000 / (792 * CSS_PX_PER_PT));
    expect(fitWidthZoom([], 1000)).toBe(1);
  });
});

describe("render scale buckets", () => {
  it("rounds up to a bucket and accounts for DPR", () => {
    expect(renderScalePermille(letter(), 1, 1)).toBe(1500); // 1.333 → 1.5
    expect(renderScalePermille(letter(), 1, 2)).toBe(3000); // 2.667 → 3
    expect(renderScalePermille(letter(), 0.25, 1)).toBe(500);
    expect(renderScalePermille(letter(), 1.05, 1)).toBe(renderScalePermille(letter(), 1.1, 1));
  });

  it("stays under the pixel cap for huge pages", () => {
    const poster = { width_pt: 14400, height_pt: 14400 };
    const permille = renderScalePermille(poster, 4, 2);
    const s = permille / 1000;
    expect(14400 * s * 14400 * s).toBeLessThanOrEqual(100_000_000);
    expect(permille).toBeGreaterThanOrEqual(50);
  });
});

describe("pdf → screen mapping", () => {
  const r = { left: 72, bottom: 700, right: 172, top: 712 };
  it("unrotated page flips the y axis", () => {
    const s = pdfRectToScreen(r, letter(0), 1 / CSS_PX_PER_PT);
    expect(s).toEqual({ left: 72, top: 80, width: 100, height: 12 });
  });
  it("90° maps x→y and y→x", () => {
    const s = pdfRectToScreen(r, letter(90), 1 / CSS_PX_PER_PT);
    expect(s).toEqual({ left: 700, top: 72, width: 12, height: 100 });
  });
  it("180° mirrors x", () => {
    const s = pdfRectToScreen(r, letter(180), 1 / CSS_PX_PER_PT);
    expect(s).toEqual({ left: 612 - 172, top: 700, width: 100, height: 12 });
  });
  it("270° keeps the box inside the rotated page", () => {
    const s = pdfRectToScreen(r, letter(270), 1 / CSS_PX_PER_PT);
    expect(s).toEqual({ left: 792 - 712, top: 612 - 172, width: 12, height: 100 });
    const d = displaySizePt(letter(270));
    expect(s.left + s.width).toBeLessThanOrEqual(d.w);
    expect(s.top + s.height).toBeLessThanOrEqual(d.h);
  });
});

describe("visible page", () => {
  it("picks the page with most visible area", () => {
    const tops = [0, 1000, 2000];
    const heights = [990, 990, 990];
    expect(mostVisiblePage(tops, heights, 0, 800)).toBe(0);
    expect(mostVisiblePage(tops, heights, 700, 800)).toBe(1);
    expect(mostVisiblePage(tops, heights, 2500, 800)).toBe(2);
  });
});

describe("text layer transform agrees with the search-rect mapping for every rotation", () => {
  const r = { left: 72, bottom: 700, right: 172, top: 712 };
  for (const rot of [0, 90, 180, 270] as const) {
    it(`rotation ${rot}`, () => {
      const page = letter(rot);
      const zoom = 1.25;
      const k = zoom * CSS_PX_PER_PT;
      const layer = unrotatedLayer(page, zoom);
      // Rect corners in the unrotated layer (origin top-left).
      const corners: Array<[number, number]> = [
        [r.left * k, (page.height_pt - r.top) * k],
        [r.right * k, (page.height_pt - r.bottom) * k],
      ];
      const mapped = corners.map(([x, y]) => applyLayerTransform(rot, layer.width, layer.height, x, y));
      const left = Math.min(mapped[0]![0], mapped[1]![0]);
      const top = Math.min(mapped[0]![1], mapped[1]![1]);
      const expected = pdfRectToScreen(r, page, zoom);
      expect(left).toBeCloseTo(expected.left, 6);
      expect(top).toBeCloseTo(expected.top, 6);
      if (rot === 0) expect(layer.transform).toBe("none");
      else expect(layer.transform).toContain(`rotate(${rot}deg)`);
    });
  }
});

describe("pages whose box does not start at 0,0", () => {
  for (const rot of [0, 90, 180, 270] as const) {
    it(`maps user space through the box origin at ${rot}°`, () => {
      const page = { index: 0, width_pt: 612, height_pt: 792, rotation: rot, label: null, origin_x: 100, origin_y: 100 };
      // The box's lower-left corner in user space is (100,100): a rect there touches the displayed corner.
      const corner = pdfRectToScreen({ left: 100, bottom: 100, right: 110, top: 120 }, page, 1);
      const unshifted = pdfRectToScreen({ left: 0, bottom: 0, right: 10, top: 20 }, { ...page, origin_x: 0, origin_y: 0 }, 1);
      expect(corner).toEqual(unshifted);
      const back = screenRectToPdf(corner, page, 1);
      expect(back.left).toBeCloseTo(100);
      expect(back.bottom).toBeCloseTo(100);
      expect(back.right).toBeCloseTo(110);
      expect(back.top).toBeCloseTo(120);
    });
  }
});

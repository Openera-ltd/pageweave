import { describe, expect, it } from "vitest";
import { duplicate, initialTiles, insertPages, isUnchanged, moveTo, remove, rotate, toPageRefs } from "./organize";

describe("page organizer model", () => {
  it("starts as the document, unchanged", () => {
    const t = initialTiles(3);
    expect(toPageRefs(t)).toEqual([0, 1, 2].map((page) => ({ source: 0, page, rotate: 0 })));
    expect(isUnchanged(t, 3)).toBe(true);
  });

  it("moves, duplicates, rotates and removes without mutating", () => {
    const t0 = initialTiles(3);
    const t1 = moveTo(t0, 2, 0);
    expect(t1.map((t) => t.page)).toEqual([2, 0, 1]);
    expect(t0.map((t) => t.page)).toEqual([0, 1, 2]);
    const t2 = duplicate(t1, 1);
    expect(t2.map((t) => t.page)).toEqual([2, 0, 0, 1]);
    expect(new Set(t2.map((t) => t.id)).size).toBe(4);
    const t3 = rotate(rotate(t2, 2, -90), 2, -90);
    expect(t3[2]!.rotate).toBe(180);
    expect(rotate(t3, 0, -90)[0]!.rotate).toBe(270);
    expect(remove(t3, 3).length).toBe(3);
    expect(isUnchanged(t3, 3)).toBe(false);
  });

  it("never removes the last page", () => {
    const one = initialTiles(1);
    expect(remove(one, 0)).toBe(one);
  });

  it("inserts all pages of another file after a position", () => {
    const t = insertPages(initialTiles(2), 0, 1, 2);
    expect(toPageRefs(t)).toEqual([
      { source: 0, page: 0, rotate: 0 },
      { source: 1, page: 0, rotate: 0 },
      { source: 1, page: 1, rotate: 0 },
      { source: 0, page: 1, rotate: 0 },
    ]);
    expect(insertPages(initialTiles(1), -1, 1, 1)[0]!.source).toBe(1);
  });

});

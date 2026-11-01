import { describe, expect, it } from "vitest";
import { addObject, align, cropBox, dragCorner, paragraphLayout, shapeGeometry, textPatch, bounds, commit, distribute, duplicate, group, hit, isStudioSource, moveMany, removeMany, rotate, snap, studioSourceFor, ungroup, withGroup, moveBy, redo, remove, reorder, resize, start, uncertain, undo, update } from "./studio";
import type { Design, DesignObject } from "./types";

const obj = (id: string, extra: Partial<DesignObject> = {}): DesignObject => ({
  id,
  name: id,
  kind: "shape",
  x: 10,
  y: 10,
  w: 20,
  h: 10,
  visible: true,
  locked: false,
  confidence: 1,
  bold: false,
  fill: "#ff0000",
  ...extra,
});

const design = (...objects: DesignObject[]): Design => ({
  version: 1,
  width: 100,
  height: 100,
  pt_per_px: 0.75,
  background: "#ffffff",
  source_png: "",
  source_name: "x.png",
  objects,
});

describe("design studio editing", () => {
  it("records each change once and walks back and forth", () => {
    let h = start(design(obj("a")));
    h = commit(h, moveBy(h.present, "a", 5, 0));
    h = commit(h, update(h.present, "a", { fill: "#00ff00" }));
    expect(h.present.objects[0]).toMatchObject({ x: 15, fill: "#00ff00" });
    h = undo(h);
    expect(h.present.objects[0]).toMatchObject({ x: 15, fill: "#ff0000" });
    h = undo(h);
    expect(h.present.objects[0]!.x).toBe(10);
    expect(undo(h)).toBe(h);
    h = redo(redo(h));
    expect(h.present.objects[0]).toMatchObject({ x: 15, fill: "#00ff00" });
    expect(redo(h)).toBe(h);
    // A no-op is not an undo step; a new change clears redo.
    expect(commit(h, moveBy(h.present, "a", 0, 0))).toBe(h);
    h = commit(undo(h), moveBy(undo(h).present, "a", 1, 1));
    expect(h.future).toHaveLength(0);
  });

  it("merges typing in one field into one undo step", () => {
    let h = start(design(obj("t", { kind: "text", text: "" })));
    for (const t of ["H", "He", "Hel"]) h = commit(h, update(h.present, "t", { text: t }), "t:text");
    h = commit(h, update(h.present, "t", { color: "#00ff00" }), "t:color");
    expect(h.past).toHaveLength(2);
    h = undo(h);
    expect(h.present.objects[0]!.text).toBe("Hel");
    h = undo(h);
    expect(h.present.objects[0]!.text).toBe("");
    // After undo, the same field starts a new step.
    h = commit(h, update(h.present, "t", { text: "X" }), "t:text");
    expect(h.past).toHaveLength(1);
  });

  it("never moves or resizes a locked object", () => {
    const d = design(obj("a", { locked: true }));
    expect(moveBy(d, "a", 3, 3)).toBe(d);
    expect(resize(d, "a", { x: 0, y: 0, w: 50, h: 50 })).toBe(d);
  });

  it("scales a text's size and baseline with its height", () => {
    const d = design(obj("t", { kind: "text", h: 20, font_size: 28, baseline: 20, text: "Hi", color: "#000000" }));
    const r = resize(d, "t", { x: 1, y: 2, w: 80, h: 40 }).objects[0]!;
    expect(r).toMatchObject({ x: 1, y: 2, w: 80, h: 40, font_size: 56, baseline: 40 });
    // Never smaller than one pixel.
    expect(resize(d, "t", { x: 0, y: 0, w: -5, h: 0 }).objects[0]).toMatchObject({ w: 1, h: 1 });
  });

  it("reorders within bounds, removes and duplicates with unique ids", () => {
    const d = design(obj("a"), obj("b"), obj("c"));
    expect(reorder(d, "a", 1).objects.map((o) => o.id)).toEqual(["b", "a", "c"]);
    expect(reorder(d, "a", -1)).toBe(d);
    expect(reorder(d, "c", 1)).toBe(d);
    expect(remove(d, "b").objects.map((o) => o.id)).toEqual(["a", "c"]);
    expect(remove(d, "zz")).toBe(d);
    const one = duplicate(d, "b");
    const two = duplicate(one.design, "b");
    expect(one.id).toBe("b-copy2");
    expect(two.id).toBe("b-copy3");
    expect(two.design.objects.map((o) => o.id)).toEqual(["a", "b", "b-copy3", "b-copy2", "c"]);
    expect(duplicate(d, "zz")).toEqual({ design: d, id: null });
  });

  it("hits the front-most visible object and flags uncertain ones", () => {
    const d = design(obj("back", { x: 0, y: 0, w: 100, h: 100 }), obj("front"), obj("hidden", { visible: false }));
    expect(hit(d, 15, 15)?.id).toBe("front");
    expect(hit(d, 90, 90)?.id).toBe("back");
    expect(hit(design(), 5, 5)).toBeNull();
    expect(uncertain(obj("p", { kind: "image", confidence: 0 }))).toBe(true);
    expect(uncertain(obj("t", { kind: "text", confidence: 0.7 }))).toBe(true);
    expect(uncertain(obj("t", { kind: "text", confidence: 0.95 }))).toBe(false);
    expect(isStudioSource("C:\\a\\Flyer.JPG")).toBe(true);
    expect(isStudioSource("a.docx")).toBe(false);
    expect(isStudioSource("scan.tiff") && isStudioSource("b.webp") && isStudioSource("c.BMP")).toBe(true);
  });

  it("studio.open: a project opens, a picture or PDF is rebuilt, other files are refused", () => {
    expect(studioSourceFor("/d/flyer.PWDESIGN")).toEqual({ kind: "open", path: "/d/flyer.PWDESIGN" });
    expect(studioSourceFor("C:\\x\\menu.jpeg")).toEqual({ kind: "rebuild", input: "C:\\x\\menu.jpeg", page: 0 });
    expect(studioSourceFor("/d/poster.pdf")).toEqual({ kind: "rebuild", input: "/d/poster.pdf", page: 0 });
    expect(studioSourceFor("/d/notes.txt")).toBeNull();
  });
});

describe("design studio: adding and snapping", () => {
  it("adds text or a rectangle centred in front, with unique ids", () => {
    const d = design(obj("a"));
    const t = addObject(d, "text");
    const o = t.design.objects[t.design.objects.length - 1]!;
    expect(o).toMatchObject({ id: t.id, kind: "text", text: "New text", color: "#000000", visible: true, locked: false });
    expect(o.x + o.w / 2).toBeCloseTo(50, 0);
    expect(o.y + o.h / 2).toBeCloseTo(50, 0);
    const r1 = addObject(t.design, "shape");
    const r2 = addObject(r1.design, "shape");
    expect(r1.id).not.toBe(r2.id);
    expect(r2.design.objects.at(-1)).toMatchObject({ kind: "shape", fill: "#2f6fde" });
  });

  it("snaps to the canvas centre and to other objects' edges, nearest per axis", () => {
    const d = design(obj("a", { x: 10, y: 10, w: 20, h: 10 }), obj("m", { x: 0, y: 0, w: 10, h: 10 }));
    // Left edge 2 px from "a"'s left edge (10) → snaps to 10; top far from everything → unchanged.
    expect(snap(d, "m", { x: 12, y: 63, w: 10, h: 10 }, 4)).toEqual({ x: 10, y: 63, guides: [{ axis: "x", at: 10 }] });
    // Centre 1 px from the canvas centre (50) on both axes.
    expect(snap(d, "m", { x: 46, y: 44, w: 10, h: 10 }, 4)).toEqual({ x: 45, y: 45, guides: [{ axis: "x", at: 50 }, { axis: "y", at: 50 }] });
    // Out of range: no snap, no guides; hidden objects are not targets.
    expect(snap(d, "m", { x: 70, y: 70, w: 5, h: 5 }, 2)).toEqual({ x: 70, y: 70, guides: [] });
    const hidden = design(obj("a", { x: 33, y: 33, w: 1, h: 1, visible: false }), obj("m"));
    expect(snap(hidden, "m", { x: 32, y: 80, w: 4, h: 4 }, 2).guides).toEqual([]);
  });
});

describe("design studio: several objects", () => {
  const three = () =>
    design(
      obj("a", { x: 10, y: 10, w: 10, h: 10 }),
      obj("b", { x: 40, y: 30, w: 20, h: 10 }),
      obj("c", { x: 80, y: 50, w: 10, h: 20 }),
    );

  it("aligns on the selection's box, or a single object on the canvas; locked ones stay", () => {
    const d = three();
    expect(align(d, ["a", "b", "c"], "left").objects.map((o) => o.x)).toEqual([10, 10, 10]);
    expect(align(d, ["a", "b", "c"], "right").objects.map((o) => o.x + o.w)).toEqual([90, 90, 90]);
    expect(align(d, ["a", "b", "c"], "center").objects.map((o) => o.x + o.w / 2)).toEqual([50, 50, 50]);
    expect(align(d, ["a", "b", "c"], "bottom").objects.map((o) => o.y + o.h)).toEqual([70, 70, 70]);
    expect(align(d, ["a"], "middle").objects[0]).toMatchObject({ y: 45 });
    const locked = design(obj("a", { x: 10, locked: true }), obj("b", { x: 40 }));
    expect(align(locked, ["a", "b"], "left").objects.map((o) => o.x)).toEqual([10, 10]);
    expect(align(align(d, ["a", "b", "c"], "left"), ["a", "b", "c"], "left")).toEqual(align(d, ["a", "b", "c"], "left"));
  });

  it("distributes equal gaps between three or more, the outermost staying", () => {
    const d = three();
    const r = distribute(d, ["c", "a", "b"], "x").objects;
    // Span 10..90 holds 40 px of objects: two gaps of 20.
    expect(r.map((o) => [o.x, o.x + o.w])).toEqual([[10, 20], [40, 60], [80, 90]]);
    expect(distribute(d, ["a", "b"], "x")).toBe(d);
  });

  it("groups select, move and delete together; ungroup separates", () => {
    let d = group(three(), ["a", "c"]);
    expect(withGroup(d, "a").sort()).toEqual(["a", "c"]);
    expect(withGroup(d, "b")).toEqual(["b"]);
    const g2 = group(d, ["b", "c"]);
    expect(g2.objects.find((o) => o.id === "b")!.group).not.toBe(d.objects.find((o) => o.id === "a")!.group);
    d = moveMany(d, withGroup(d, "c"), 5, -5);
    expect(d.objects.map((o) => [o.x, o.y])).toEqual([[15, 5], [40, 30], [85, 45]]);
    expect(bounds(d, ["a", "c"])).toEqual({ x: 15, y: 5, w: 80, h: 60 });
    expect(withGroup(ungroup(d, ["a"]), "c")).toEqual(["c"]);
    expect(removeMany(d, ["a", "c"]).objects.map((o) => o.id)).toEqual(["b"]);
    expect(group(d, ["a"])).toBe(d);
  });

  it("rotates within −180…180; a locked object does not turn", () => {
    const d = three();
    expect(rotate(d, "a", 90).objects[0]!.rotation).toBe(90);
    expect(rotate(d, "a", 270).objects[0]!.rotation).toBe(-90);
    expect(rotate(d, "a", -450).objects[0]!.rotation).toBe(-90);
    const l = design(obj("a", { locked: true }));
    expect(rotate(l, "a", 45)).toBe(l);
  });

  it("snaps a moving selection, ignoring its own members", () => {
    const d = three();
    // Moving a+b: their own edges are no targets; c's left (80) is.
    expect(snap(d, ["a", "b"], { x: 28, y: 90, w: 50, h: 10 }, 3).x).toBe(30);
  });

  it("drags a corner of an unturned box like before, never below 1 × 1", () => {
    const b = { x: 10, y: 20, w: 100, h: 50 };
    const near = (r: { x: number; y: number; w: number; h: number }, e: number[]) =>
      [r.x, r.y, r.w, r.h].forEach((v, i) => expect(v).toBeCloseTo(e[i]!, 6));
    near(dragCorner(b, 0, "se", 20, 10), [10, 20, 120, 60]);
    near(dragCorner(b, 0, "nw", 20, 10), [30, 30, 80, 40]);
    near(dragCorner(b, 0, "ne", -30, -5), [10, 15, 70, 55]);
    near(dragCorner(b, 0, "nw", 500, 500), [109, 69, 1, 1]);
  });

  it("drags a corner of a turned box along its own sides; the opposite corner stays put", () => {
    const b = { x: 0, y: 0, w: 100, h: 40 };
    const corner = (r: { x: number; y: number; w: number; h: number }, rot: number, sx: number, sy: number) => {
      const t = (rot * Math.PI) / 180;
      const [lx, ly] = [(sx * r.w) / 2, (sy * r.h) / 2];
      return [r.x + r.w / 2 + lx * Math.cos(t) - ly * Math.sin(t), r.y + r.h / 2 + lx * Math.sin(t) + ly * Math.cos(t)];
    };
    // Turned 90°: the box's own width runs down the canvas. Dragging the "se" corner 30 px down
    // makes it 30 wider; the "nw" corner does not move.
    const r = dragCorner(b, 90, "se", 0, 30);
    expect(r.w).toBeCloseTo(130, 6);
    expect(r.h).toBeCloseTo(40, 6);
    corner(r, 90, -1, -1).forEach((v, i) => expect(v).toBeCloseTo(corner(b, 90, -1, -1)[i]!, 6));
    // Any angle: the opposite corner is fixed.
    for (const rot of [30, -45, 135]) {
      const q = dragCorner(b, rot, "nw", 7, -11);
      corner(q, rot, 1, 1).forEach((v, i) => expect(v).toBeCloseTo(corner(b, rot, 1, 1)[i]!, 6));
    }
  });

  it("lays out a paragraph like the exports: anchors, scale from the left edge, spacing", () => {
    const p = obj("p", { kind: "text", x: 100, y: 0, w: 200, h: 40, text: "a\nbb", font_size: 20, baseline: 15, stretch: 0.8, align: "right", line_height: 25 });
    const l = paragraphLayout(p);
    // Right edge 300 on the canvas: unscaled 100 + 200 / 0.8 = 350, then scaled about x = 100.
    expect(l.anchor).toBe("end");
    expect(l.lines).toEqual([{ text: "a", x: 350, y: 15 }, { text: "bb", x: 350, y: 40 }]);
    expect(l.scale).toBe("matrix(0.8 0 0 1 20 0)");
    expect(100 + (350 - 100) * 0.8).toBe(300);
  });

  it("draws an outline inside the box, never wider than half the shorter side", () => {
    expect(shapeGeometry(obj("s", { x: 10, y: 10, w: 40, h: 20, radius: 6, stroke: "#000", stroke_width: 4 }))).toEqual({ x: 12, y: 12, w: 36, h: 16, r: 4, sw: 4 });
    expect(shapeGeometry(obj("s", { w: 40, h: 20, stroke: "#000", stroke_width: 50 })).sw).toBe(10);
    expect(shapeGeometry(obj("s", { w: 40, h: 20, stroke_width: 3 })).sw).toBe(0);
  });

  it("crops at least 1 × 1 and reports trims as fractions", () => {
    const p = obj("p", { kind: "image", x: 10, y: 20, w: 50, h: 40 });
    expect(cropBox(p, { l: 5, t: 0, r: 0, b: 10 })).toEqual({ box: { x: 15, y: 20, w: 45, h: 30 }, fractions: [0.1, 0, 0, 0.25], empty: false });
    const all = cropBox(p, { l: 100, t: 100, r: 100, b: 100 });
    expect(all.box).toEqual({ x: 59, y: 59, w: 1, h: 1 });
    expect(cropBox(p, { l: 0, t: 0, r: 0, b: 0 }).empty).toBe(true);
  });

  it("grows a text box to hold its lines; one line keeps its box", () => {
    const t = obj("t", { kind: "text", h: 20, font_size: 20, baseline: 15 });
    expect(textPatch(t, "one")).toEqual({ text: "one" });
    expect(textPatch(t, "one\ntwo\nthree")).toEqual({ text: "one\ntwo\nthree", h: 15 + 2 * 24 + 5 });
  });

  it("keeps a corner radius within half the shorter side when edited or resized", () => {
    const d = design(obj("a", { w: 40, h: 40, radius: 15 }));
    expect(update(d, "a", { radius: 99 }).objects[0]!.radius).toBe(20);
    expect(update(d, "a", { h: 10 }).objects[0]!.radius).toBe(5);
    expect(resize(d, "a", { x: 0, y: 0, w: 40, h: 21 }).objects[0]!.radius).toBe(10.5);
    expect(resize(d, "a", { x: 0, y: 0, w: 80, h: 80 }).objects[0]!.radius).toBe(15);
  });
});

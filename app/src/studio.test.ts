import { describe, expect, it } from "vitest";
import { commit, duplicate, hit, isStudioSource, studioSourceFor, moveBy, redo, remove, reorder, resize, start, uncertain, undo, update } from "./studio";
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
  });

  it("studio.open: a project opens, a picture or PDF is rebuilt, other files are refused", () => {
    expect(studioSourceFor("/d/flyer.PWDESIGN")).toEqual({ kind: "open", path: "/d/flyer.PWDESIGN" });
    expect(studioSourceFor("C:\\x\\menu.jpeg")).toEqual({ kind: "rebuild", input: "C:\\x\\menu.jpeg", page: 0 });
    expect(studioSourceFor("/d/poster.pdf")).toEqual({ kind: "rebuild", input: "/d/poster.pdf", page: 0 });
    expect(studioSourceFor("/d/notes.txt")).toBeNull();
  });
});

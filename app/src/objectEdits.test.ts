import { describe, expect, it } from "vitest";
import { EMPTY_HISTORY, netMove, normalize, push, redoLast, toPageEdits, undoLast, type ObjEntry } from "./objectEdits";

const text = (objects: number[], t: string, dx = 0, dy = 0): ObjEntry => ({ page: 0, edit: { kind: "text", objects, text: t, size: null, color: null, dx, dy } });

describe("pending object edits", () => {
  it("keeps the last wording per run and adds up its moves", () => {
    const items = [text([1, 2], "first"), text([1, 2], "second", 5, 0), text([1, 2], "second", 0, -3)];
    expect(normalize(items, 0)).toEqual([{ kind: "text", objects: [1, 2], text: "second", size: null, color: null, dx: 5, dy: -3 }]);
    expect(netMove(items, 0, 1)).toEqual({ dx: 5, dy: -3 });
  });

  it("adds up picture moves and turns; a replacement resets turns; a delete wins", () => {
    const items: ObjEntry[] = [
      { page: 0, edit: { kind: "move_objects", objects: [7], dx: 10, dy: 0 } },
      { page: 0, edit: { kind: "move_objects", objects: [7], dx: 5, dy: 2 } },
      { page: 0, edit: { kind: "rotate_image", object: 7, degrees: 90 } },
      { page: 0, edit: { kind: "rotate_image", object: 7, degrees: 270 } },
      { page: 0, edit: { kind: "rotate_image", object: 8, degrees: 90 } },
      { page: 0, edit: { kind: "replace_image", object: 8, path: "/a.png" } },
      { page: 1, edit: { kind: "delete_objects", objects: [3] } },
      { page: 1, edit: { kind: "rotate_image", object: 3, degrees: 90 } },
    ];
    expect(normalize(items, 0)).toEqual([
      { kind: "move_objects", objects: [7], dx: 15, dy: 2 },
      { kind: "replace_image", object: 8, path: "/a.png" },
    ]);
    expect(normalize(items, 1)).toEqual([{ kind: "delete_objects", objects: [3] }]);
    expect(toPageEdits(items).map((p) => p.page)).toEqual([0, 1]);
  });

  it("drops pages whose edits cancel out", () => {
    const items: ObjEntry[] = [
      { page: 2, edit: { kind: "rotate_image", object: 4, degrees: 180 } },
      { page: 2, edit: { kind: "rotate_image", object: 4, degrees: 180 } },
    ];
    expect(toPageEdits(items)).toEqual([]);
  });

  it("undo and redo walk the history; a new edit clears redo", () => {
    let h = push(push(EMPTY_HISTORY, text([1], "a")), text([1], "b"));
    h = undoLast(h);
    expect(h.items).toHaveLength(1);
    expect(h.redo).toHaveLength(1);
    h = redoLast(h);
    expect(normalize(h.items, 0)[0]).toMatchObject({ text: "b" });
    h = push(undoLast(h), text([1], "c"));
    expect(h.redo).toHaveLength(0);
    expect(undoLast(EMPTY_HISTORY)).toBe(EMPTY_HISTORY);
  });
});

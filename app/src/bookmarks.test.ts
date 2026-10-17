import { describe, expect, it } from "vitest";
import { bookmarksBlocker, canIndent, canMoveDown, canMoveUp, indent, insertAfter, moveDown, moveUp, outdent, remove, subtreeEnd, type EditableBookmark } from "./bookmarks";

let k = 0;
const b = (title: string, depth: number, page = 0): EditableBookmark => ({ key: k++, title, page, depth });
const shape = (l: EditableBookmark[]) => l.map((x) => `${"-".repeat(x.depth)}${x.title}`);

// A
// -A1
// --A1a
// -A2
// B
const tree = () => [b("A", 0), b("A1", 1), b("A1a", 2), b("A2", 1), b("B", 0)];

describe("outline editing", () => {
  it("finds subtrees", () => {
    expect(subtreeEnd(tree(), 0)).toBe(4);
    expect(subtreeEnd(tree(), 1)).toBe(3);
    expect(subtreeEnd(tree(), 4)).toBe(5);
  });

  it("indents and outdents whole subtrees and refuses invalid indents", () => {
    expect(canIndent(tree(), 0)).toBe(false);
    expect(canIndent(tree(), 3)).toBe(true);
    expect(shape(indent(tree(), 3))).toEqual(["A", "-A1", "--A1a", "--A2", "B"]);
    expect(shape(outdent(tree(), 1))).toEqual(["A", "A1", "-A1a", "-A2", "B"]);
    expect(canIndent(tree(), 2)).toBe(false);
  });

  it("moves a bookmark with its children among its siblings only", () => {
    expect(shape(moveUp(tree(), 4))).toEqual(["B", "A", "-A1", "--A1a", "-A2"]);
    expect(shape(moveDown(tree(), 1))).toEqual(["A", "-A2", "-A1", "--A1a", "B"]);
    expect(canMoveUp(tree(), 1)).toBe(false);
    expect(canMoveDown(tree(), 4)).toBe(false);
  });

  it("deleting keeps the children one level up", () => {
    expect(shape(remove(tree(), 1))).toEqual(["A", "-A1a", "-A2", "B"]);
    expect(bookmarksBlocker(remove(tree(), 0), 5)).toBeNull();
  });

  it("inserts after a whole block at the same level", () => {
    expect(shape(insertAfter(tree(), 1, b("New", 0)))).toEqual(["A", "-A1", "--A1a", "-New", "-A2", "B"]);
    expect(shape(insertAfter([], null, b("First", 3)))).toEqual(["First"]);
  });

  it("explains what blocks saving", () => {
    expect(bookmarksBlocker(tree(), 5)).toBeNull();
    expect(bookmarksBlocker([b(" ", 0)], 5)).toMatch(/needs a title/);
    expect(bookmarksBlocker([b("A", 0, 7)], 5)).toMatch(/from 1 to 5/);
    expect(bookmarksBlocker([{ ...b("A", 0), page: null }], 5)).toMatch(/choose a page/);
    expect(bookmarksBlocker([b("A", 1)], 5)).toMatch(/indented/);
    expect(bookmarksBlocker([b("a\nb", 0)], 5)).toMatch(/line breaks/);
  });
});

import { describe, expect, it } from "vitest";
import { kindName, markDeleted, offsetOf, revert, setColor, setOffset, setText, toChanges } from "./annotationChanges";
import type { ExistingAnnotation } from "./types";

const a = (page: number, index: number, subtype = "Text", text: string | null = "hello"): ExistingAnnotation => ({
  page,
  index,
  subtype,
  rect: { left: 0, bottom: 0, right: 1, top: 1 },
  text,
  author: null,
  modified: null,
  color: null,
  text_editable: subtype !== "FreeText",
});

describe("pending annotation changes", () => {
  it("records deletes and text changes, drops a change back to the original, and sorts for the engine", () => {
    let c = markDeleted({}, a(1, 3));
    c = setText(c, a(0, 2, "Highlight", "old"), "new");
    c = setText(c, a(0, 5, "Square", "same"), " same ");
    expect(toChanges(c)).toEqual([
      { page: 0, index: 2, subtype: "Highlight", delete: false, text: "new", color: null },
      { page: 1, index: 3, subtype: "Text", delete: true, text: null, color: null },
    ]);
    c = revert(c, a(1, 3));
    expect(Object.keys(c)).toEqual(["0:2"]);
    expect(setText(c, a(0, 2, "Highlight", "old"), "old")).toEqual({});
  });
  it("keeps a text and a colour change of one annotation together; each back to the original drops it", () => {
    const hl = { ...a(0, 2, "Highlight", "note"), color: "#ffff00" };
    let c = setColor({}, hl, "#00FF00");
    c = setText(c, hl, "changed");
    expect(toChanges(c)).toEqual([{ page: 0, index: 2, subtype: "Highlight", delete: false, text: "changed", color: "#00FF00" }]);
    c = setColor(c, hl, "#ffff00");
    expect(toChanges(c)).toEqual([{ page: 0, index: 2, subtype: "Highlight", delete: false, text: "changed", color: null }]);
    expect(setText(c, hl, "note")).toEqual({});
  });

  it("keeps a move with the other changes, rounds it to 0.01 pt, and a move back drops it", () => {
    const hl = { ...a(0, 2, "Highlight", "note"), color: "#ffff00" };
    expect(offsetOf({}, hl)).toEqual([0, 0]);
    let c = setOffset({}, hl, [12.3456, -40]);
    c = setColor(c, hl, "#00ff00");
    expect(toChanges(c)).toEqual([{ page: 0, index: 2, subtype: "Highlight", delete: false, text: null, color: "#00ff00", offset: [12.35, -40] }]);
    expect(offsetOf(c, hl)).toEqual([12.35, -40]);
    c = setOffset(c, hl, [0.001, 0]);
    expect(toChanges(c)).toEqual([{ page: 0, index: 2, subtype: "Highlight", delete: false, text: null, color: "#00ff00" }]);
    expect(setOffset(setColor(c, hl, "#ffff00"), hl, [0, 0])).toEqual({});
    // A deleted annotation is not moved.
    expect(offsetOf(markDeleted(setOffset({}, hl, [5, 5]), hl), hl)).toEqual([0, 0]);
  });

  it("names kinds the way people say them", () => {
    expect(kindName("Text")).toBe("Note");
    expect(kindName("StrikeOut")).toBe("Strikethrough");
    expect(kindName("Custom")).toBe("Custom");
  });
});

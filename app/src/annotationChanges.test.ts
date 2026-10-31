import { describe, expect, it } from "vitest";
import { boundsOf, kindName, moveBy, setBounds, markDeleted, offsetOf, revert, setColor, setOffset, setText, toChanges } from "./annotationChanges";
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

  it("a resize is a new box; later moves shift that box; back to the original box drops it", () => {
    const sq = { ...a(0, 7, "Square", null), rect: { left: 300, bottom: 500, right: 360, top: 540 } };
    let c = moveBy({}, sq, 10, 0);
    expect(boundsOf(c, sq)).toEqual({ left: 310, bottom: 500, right: 370, top: 540 });
    c = setBounds(c, sq, { left: 310, bottom: 500, right: 400.004, top: 560 });
    expect(toChanges(c)).toEqual([{ page: 0, index: 7, subtype: "Square", delete: false, text: null, color: null, rect: { left: 310, bottom: 500, right: 400, top: 560 } }]);
    c = moveBy(c, sq, -10, -5);
    expect(toChanges(c)[0]).toMatchObject({ rect: { left: 300, bottom: 495, right: 390, top: 555 } });
    expect(toChanges(c)[0]!.offset).toBeUndefined();
    expect(setBounds(c, sq, sq.rect)).toEqual({});
  });

  it("names kinds the way people say them", () => {
    expect(kindName("Text")).toBe("Note");
    expect(kindName("StrikeOut")).toBe("Strikethrough");
    expect(kindName("Custom")).toBe("Custom");
  });
});

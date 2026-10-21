import { describe, expect, it } from "vitest";
import { labelFor, previewLabels, rangesProblem, type LabelRange } from "./pageLabels";

const r = (start: number, style: LabelRange["style"], prefix = "", first = 1): LabelRange => ({ start, style, prefix, first });

describe("page labels", () => {
  it("computes labels like the engine (same cases as pw-core labels::tests)", () => {
    const ranges = [r(0, "roman_lower"), r(3, "decimal"), r(6, "decimal", "A-", 7), r(8, "none", "Back cover"), r(9, "letters_upper", "", 26)];
    expect(Array.from({ length: 11 }, (_, p) => labelFor(ranges, p))).toEqual(["i", "ii", "iii", "1", "2", "3", "A-7", "A-8", "Back cover", "Z", "AA"]);
    expect(labelFor([], 0)).toBeNull();
    expect(labelFor([r(0, "roman_upper", "", 1994)], 0)).toBe("MCMXCIV");
  });

  it("explains invalid ranges", () => {
    expect(rangesProblem([r(1, "decimal")], 5)).toBe("The first range must start on page 1.");
    expect(rangesProblem([r(0, "decimal"), r(0, "decimal")], 5)).toBe("Range 2 must start after range 1.");
    expect(rangesProblem([r(0, "decimal"), r(7, "decimal")], 5)).toBe("Range 2: choose a page from 1 to 5.");
    expect(rangesProblem([r(0, "none", " ")], 5)).toBe("Range 1: a range without numbers needs a prefix.");
    expect(rangesProblem([r(0, "decimal", "", 0)], 5)).toBe("Range 1: start numbering at 1 or more.");
    expect(rangesProblem([], 5)).toBeNull();
  });

  it("previews range starts and the last page", () => {
    expect(previewLabels([r(0, "roman_lower"), r(3, "decimal")], 10)).toEqual([
      { page: 0, label: "i" },
      { page: 1, label: "ii" },
      { page: 3, label: "1" },
      { page: 4, label: "2" },
      { page: 9, label: "7" },
    ]);
  });
});

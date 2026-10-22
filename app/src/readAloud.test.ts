import { describe, expect, it } from "vitest";
import { highlightAt, markAt, paragraphs, rectsFor, segmentFrom, sentenceStep, wordAt } from "./readAloud";
import type { ReadingPage } from "./types";

const r = (left: number, bottom: number) => ({ left, bottom, right: left + 20, top: bottom + 10 });
// "Hi there.\nNext line" — words: Hi(0-2) there.(3-9) Next(10-14) line(15-19)
const page: ReadingPage = {
  page: 2,
  text: "Hi there.\n—\nNext line",
  words: [
    { start: 0, end: 2, rects: [r(0, 100)] },
    { start: 3, end: 9, rects: [r(30, 100)] },
    { start: 10, end: 11, rects: [r(0, 90)] },
    { start: 12, end: 16, rects: [r(0, 80)] },
    { start: 17, end: 21, rects: [r(30, 80)] },
  ],
};

describe("read aloud", () => {
  it("splits paragraphs and skips ones with nothing to say", () => {
    expect(paragraphs(page).map((p) => [p.start, p.end, p.text])).toEqual([
      [0, 9, "Hi there."],
      [12, 21, "Next line"],
    ]);
    // Characters outside the BMP count once.
    const emoji: ReadingPage = { page: 0, text: "😀 ok\nb", words: [] };
    expect(paragraphs(emoji).map((p) => [p.start, p.end])).toEqual([
      [0, 4],
      [5, 6],
    ]);
  });

  it("reads from a word to the end of its paragraph", () => {
    expect(segmentFrom(page, 3)).toEqual({ index: 0, segment: { page: 2, start: 3, end: 9, text: "there." } });
    expect(segmentFrom(page, 10)?.segment.text).toBe("Next line");
    expect(segmentFrom(page, 99)).toBeNull();
  });

  it("finds the first selected word and the rectangles of a range", () => {
    expect(wordAt(page, { left: 32, bottom: 101, right: 60, top: 109 })).toBe(3);
    expect(wordAt(page, { left: 500, bottom: 0, right: 600, top: 5 })).toBeNull();
    // Words side by side on a line become one box; another line is another box.
    expect(rectsFor(page, 12, 21)).toEqual([{ left: 0, bottom: 80, right: 50, top: 90 }]);
    expect(rectsFor(page, 0, 21)).toEqual([{ left: 0, bottom: 100, right: 50, top: 110 }, { left: 0, bottom: 90, right: 20, top: 100 }, { left: 0, bottom: 80, right: 50, top: 90 }]);
  });

  it("follows the marks of the audio", () => {
    const marks = [
      { start: 0, end: 4, start_ms: 100, end_ms: 400 },
      { start: 5, end: 9, start_ms: 450, end_ms: 800 },
    ];
    expect(markAt(marks, 50)).toBe(-1);
    expect(markAt(marks, 100)).toBe(0);
    expect(markAt(marks, 449)).toBe(0);
    expect(markAt(marks, 2000)).toBe(1);
    const seg = paragraphs(page)[1]!;
    expect(highlightAt(page, seg, { sentences: [], words: marks }, 500)).toEqual({ start: 17, end: 21, rects: [r(30, 80)] });
    // Without word timings the sentence is highlighted.
    const sent = [{ start: 0, end: 9, start_ms: 0, end_ms: 900 }];
    expect(highlightAt(page, seg, { sentences: sent, words: [] }, 10)?.rects).toEqual([{ left: 0, bottom: 80, right: 50, top: 90 }]);
  });

  it("steps between sentences", () => {
    const s = [
      { start: 0, end: 5, start_ms: 0, end_ms: 2000 },
      { start: 6, end: 9, start_ms: 2000, end_ms: 5000 },
    ];
    expect(sentenceStep(s, 500, 1)).toBe(2000);
    expect(sentenceStep(s, 2500, 1)).toBeNull();
    expect(sentenceStep(s, 2500, -1)).toBe(0);
    expect(sentenceStep(s, 4000, -1)).toBe(2000);
    expect(sentenceStep(s, 500, -1)).toBeNull();
  });
});

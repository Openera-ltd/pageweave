// Read aloud (PRD 09): which text is read next and what to highlight while it is heard. Pure
// functions over the engine's reading-order page text; offsets count Unicode characters.

import type { ReadingPage, SearchRect, SpeechMark } from "./types";

/** A paragraph (or the rest of one) to speak: character range in its page's text. */
export interface Segment {
  page: number;
  start: number;
  end: number;
  text: string;
}

/** Paragraphs of a page (split at line ends), skipping ones with nothing to say. */
export function paragraphs(p: ReadingPage): Segment[] {
  const cs = Array.from(p.text);
  const out: Segment[] = [];
  let start = 0;
  for (let i = 0; i <= cs.length; i++) {
    if (i === cs.length || cs[i] === "\n") {
      const text = cs.slice(start, i).join("");
      if (/[\p{L}\p{N}]/u.test(text)) out.push({ page: p.page, start, end: i, text });
      start = i + 1;
    }
  }
  return out;
}

/** The segment to read from character `at` of a page: the rest of the paragraph holding it. */
export function segmentFrom(p: ReadingPage, at: number): { index: number; segment: Segment } | null {
  const ps = paragraphs(p);
  const index = ps.findIndex((s) => at < s.end);
  if (index < 0) return null;
  const s = ps[index]!;
  const from = Math.max(at, s.start);
  const cs = Array.from(p.text);
  return { index, segment: { page: p.page, start: from, end: s.end, text: cs.slice(from, s.end).join("") } };
}

function overlap(a: SearchRect, b: SearchRect): number {
  const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
  const h = Math.min(a.top, b.top) - Math.max(a.bottom, b.bottom);
  return w > 0 && h > 0 ? w * h : 0;
}

/** Start of the first word (in reading order) that a selection rectangle covers, if any. */
export function wordAt(p: ReadingPage, rect: SearchRect): number | null {
  for (const w of p.words) {
    if (w.rects.some((r) => overlap(r, rect) > 0.3 * Math.max(1e-6, (r.right - r.left) * (r.top - r.bottom)))) return w.start;
  }
  return null;
}

/** Index of the mark being heard at `ms` (the last one started), or -1 before the first. */
export function markAt(marks: SpeechMark[], ms: number): number {
  let lo = 0;
  let hi = marks.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (marks[mid]!.start_ms <= ms) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/** Page rectangles of the words overlapping characters [start, end): one per line of text. */
export function rectsFor(p: ReadingPage, start: number, end: number): SearchRect[] {
  return joinLines(p.words.filter((w) => w.start < end && w.end > start).flatMap((w) => w.rects));
}

/** Join rectangles that sit side by side on one line (in order), so a sentence is one box per line. */
export function joinLines(rects: SearchRect[]): SearchRect[] {
  const out: SearchRect[] = [];
  for (const r of rects) {
    const last = out[out.length - 1];
    const h = Math.max(r.top - r.bottom, 1);
    const sameLine = last && Math.min(last.top, r.top) - Math.max(last.bottom, r.bottom) > 0.5 * h;
    const near = last && r.left - last.right < 1.5 * h && r.right > last.left;
    if (last && sameLine && near) {
      out[out.length - 1] = { left: Math.min(last.left, r.left), bottom: Math.min(last.bottom, r.bottom), right: Math.max(last.right, r.right), top: Math.max(last.top, r.top) };
    } else out.push({ ...r });
  }
  return out;
}

/** The highlight for time `ms` in a spoken segment: words when the engine times words, else sentences. */
export function highlightAt(
  p: ReadingPage,
  seg: Segment,
  spoken: { sentences: SpeechMark[]; words: SpeechMark[] },
  ms: number,
): { start: number; end: number; rects: SearchRect[] } | null {
  const marks = spoken.words.length ? spoken.words : spoken.sentences;
  const i = markAt(marks, ms);
  if (i < 0) return null;
  const m = marks[i]!;
  const start = seg.start + m.start;
  const end = seg.start + m.end;
  return { start, end, rects: rectsFor(p, start, end) };
}

/** Where the sentence before/after the one heard at `ms` starts (ms), or null at the segment's edge. */
export function sentenceStep(sentences: SpeechMark[], ms: number, dir: 1 | -1): number | null {
  const i = markAt(sentences, ms);
  if (dir > 0) return i + 1 < sentences.length ? sentences[i + 1]!.start_ms : null;
  if (i < 0) return null;
  // Within the first second of a sentence "previous" goes to the one before; later, to its start.
  if (ms - sentences[i]!.start_ms > 1000) return sentences[i]!.start_ms;
  return i > 0 ? sentences[i - 1]!.start_ms : null;
}

export const READ_RATES = [0.75, 1, 1.25, 1.5, 2] as const;

// Page label ranges and the label text readers will show (mirrors pw-core `labels::label_for`).

export type LabelStyle = "decimal" | "roman_upper" | "roman_lower" | "letters_upper" | "letters_lower" | "none";

export interface LabelRange {
  /** 0-based first page. */
  start: number;
  style: LabelStyle;
  prefix: string;
  first: number;
}

export const STYLE_OPTIONS: Array<{ id: LabelStyle; label: string }> = [
  { id: "decimal", label: "1, 2, 3" },
  { id: "roman_lower", label: "i, ii, iii" },
  { id: "roman_upper", label: "I, II, III" },
  { id: "letters_lower", label: "a, b, c" },
  { id: "letters_upper", label: "A, B, C" },
  { id: "none", label: "Prefix only" },
];

function roman(n: number): string {
  const t: Array<[number, string]> = [
    [1000, "m"], [900, "cm"], [500, "d"], [400, "cd"], [100, "c"], [90, "xc"],
    [50, "l"], [40, "xl"], [10, "x"], [9, "ix"], [5, "v"], [4, "iv"], [1, "i"],
  ];
  let s = "";
  for (const [v, r] of t) {
    while (n >= v) {
      s += r;
      n -= v;
    }
  }
  return s;
}

function letters(n: number): string {
  const k = Math.max(1, n) - 1;
  return String.fromCharCode(97 + (k % 26)).repeat(Math.floor(k / 26) + 1);
}

export function labelFor(ranges: LabelRange[], page: number): string | null {
  const r = [...ranges].reverse().find((x) => x.start <= page);
  if (!r) return null;
  const n = r.first + (page - r.start);
  const num =
    r.style === "decimal" ? String(n)
    : r.style === "roman_lower" ? roman(n)
    : r.style === "roman_upper" ? roman(n).toUpperCase()
    : r.style === "letters_lower" ? letters(n)
    : r.style === "letters_upper" ? letters(n).toUpperCase()
    : "";
  return r.prefix + num;
}

/** Why the ranges can't be saved, or null (mirrors the engine's validation). */
export function rangesProblem(ranges: LabelRange[], pageCount: number): string | null {
  if (ranges.length && ranges[0]!.start !== 0) return "The first range must start on page 1.";
  for (let i = 0; i < ranges.length; i++) {
    const r = ranges[i]!;
    if (!Number.isInteger(r.start) || r.start < 0 || r.start >= pageCount) return `Range ${i + 1}: choose a page from 1 to ${pageCount}.`;
    if (i > 0 && r.start <= ranges[i - 1]!.start) return `Range ${i + 1} must start after range ${i}.`;
    if (!Number.isInteger(r.first) || r.first < 1 || r.first > 1_000_000) return `Range ${i + 1}: start numbering at 1 or more.`;
    if (r.prefix.length > 50) return `Range ${i + 1}: the prefix is too long.`;
    if (r.style === "none" && !r.prefix.trim()) return `Range ${i + 1}: a range without numbers needs a prefix.`;
  }
  return null;
}

/** A short preview: the first label of each range and the last page's label. */
export function previewLabels(ranges: LabelRange[], pageCount: number): Array<{ page: number; label: string }> {
  const pages = new Set<number>([...ranges.map((r) => r.start), Math.max(0, pageCount - 1)]);
  for (const r of ranges) if (r.start + 1 < pageCount) pages.add(r.start + 1);
  return [...pages].sort((a, b) => a - b).map((p) => ({ page: p, label: labelFor(ranges, p) ?? String(p + 1) }));
}

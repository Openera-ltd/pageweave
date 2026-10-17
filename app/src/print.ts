// Printing plan: which pages, at what size and resolution, and the CSS that gives every printed
// sheet its page's own size. Pure (unit tested); PrintDialog renders the pages and calls print().
//
// Pages are printed as images rendered by the engine (what you see on screen, annotations
// included) through the platform's print UI, which handles the printer, copies, paper, scaling,
// colour and two-sided settings.

import { displaySizePt } from "./geometry";
import type { PageInfo } from "./types";

export const PRINT_QUALITIES = [
  { id: "draft", label: "Draft (96 dpi)", dpi: 96 },
  { id: "standard", label: "Standard (150 dpi)", dpi: 150 },
  { id: "high", label: "High (300 dpi)", dpi: 300 },
] as const;

export type PrintQuality = (typeof PRINT_QUALITIES)[number]["id"];

/**
 * Largest print job prepared at once (decoded pixels). Every page image stays decoded until the
 * print UI has taken the job, so this bounds memory (~4 bytes per pixel): 400 MP ≈ 1.6 GB.
 */
export const MAX_PRINT_MEGAPIXELS = 400;

export interface PrintSheet {
  index: number;
  widthPt: number;
  heightPt: number;
  /** CSS named page for this sheet's size. */
  pageName: string;
}

export interface PrintPlan {
  sheets: PrintSheet[];
  /** `@page` rules, one per distinct page size. */
  css: string;
  scalePermille: number;
  megapixels: number;
}

const round2 = (v: number) => Math.round(v * 100) / 100;

export function printPlan(pages: PageInfo[], indexes: number[], dpi: number): PrintPlan {
  const names = new Map<string, string>();
  const sheets: PrintSheet[] = [];
  let pixels = 0;
  for (const i of indexes) {
    const p = pages[i];
    if (!p) continue;
    const s = displaySizePt(p);
    const w = round2(s.w);
    const h = round2(s.h);
    const key = `${w}x${h}`;
    if (!names.has(key)) names.set(key, `pw-sheet-${names.size}`);
    sheets.push({ index: i, widthPt: w, heightPt: h, pageName: names.get(key)! });
    pixels += ((w * dpi) / 72) * ((h * dpi) / 72);
  }
  const css = [...names.entries()]
    .map(([key, name]) => {
      const [w, h] = key.split("x");
      return `@page ${name} { size: ${w}pt ${h}pt; margin: 0; }`;
    })
    .join("\n");
  return { sheets, css, scalePermille: Math.round((dpi / 72) * 1000), megapixels: pixels / 1e6 };
}

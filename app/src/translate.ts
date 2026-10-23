// Translation choices (PRD 08): which languages can be chosen, which route a pair takes (directly,
// or through English), and which models still need their one-time download (D-023).

import type { TranslationDirection } from "./types";

export const LANGUAGE_NAMES: Record<string, string> = { en: "English", de: "German", fr: "French", es: "Spanish", it: "Italian" };

export const languageName = (code: string): string => LANGUAGE_NAMES[code] ?? code;

/** Every language PageWeave offers, English first, then by name. */
export function languages(dirs: TranslationDirection[]): string[] {
  const set = new Set<string>();
  for (const d of dirs) set.add(d.source).add(d.target);
  return [...set].sort((a, b) => (a === "en" ? -1 : b === "en" ? 1 : languageName(a).localeCompare(languageName(b))));
}

export interface Route {
  /** The directions used, in order (one, or two through English). */
  steps: TranslationDirection[];
  /** Those not downloaded yet. */
  needs: TranslationDirection[];
  viaEnglish: boolean;
}

/** How `source` → `target` is translated, or why it cannot be. */
export function route(dirs: TranslationDirection[], source: string, target: string): Route | string {
  if (source === target) return "Choose two different languages.";
  const find = (s: string, t: string) => dirs.find((d) => d.source === s && d.target === t);
  const direct = find(source, target);
  let steps: TranslationDirection[] | null = direct ? [direct] : null;
  if (!steps && source !== "en" && target !== "en") {
    const a = find(source, "en");
    const b = find("en", target);
    if (a && b) steps = [a, b];
  }
  if (!steps) return `Translating from ${languageName(source)} to ${languageName(target)} is not available.`;
  return { steps, needs: steps.filter((d) => !d.installed), viaEnglish: steps.length > 1 };
}

/** "111 MB" */
export const megabytes = (bytes: number): string => `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;

// Translation choices (PRD 08): which languages can be chosen, which route a pair takes (directly,
// or through English), and which models still need their one-time download (D-023).

import type { TranslationDirection } from "./types";

export const LANGUAGE_NAMES: Record<string, string> = { en: "English", de: "German", fr: "French", es: "Spanish", it: "Italian", pt: "Portuguese" };

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

export interface GlossaryEntry {
  source: string;
  target: string;
}

/**
 * A glossary typed as one "term = translation" per line (blank lines and # comments skipped), or
 * the reason it cannot be used (the same rules as the engine: both sides, ≤ 200 characters each,
 * at most 1000 lines).
 */
export function parseGlossary(text: string): GlossaryEntry[] | string {
  const out: GlossaryEntry[] = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!.trim();
    if (!line || line.startsWith("#")) continue;
    const at = line.indexOf("=");
    const source = at < 0 ? "" : line.slice(0, at).trim();
    const target = at < 0 ? "" : line.slice(at + 1).trim();
    if (!source || !target) return `Glossary line ${i + 1}: write it as "term = translation".`;
    if ([...source].length > 200 || [...target].length > 200) return `Glossary line ${i + 1} is too long (at most 200 characters a side).`;
    out.push({ source, target });
  }
  return out.length > 1000 ? "A glossary can have at most 1000 lines." : out;
}

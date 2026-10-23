// Translation choices (PRD 08): which languages can be chosen and whether a pair is available
// (directly, or through English when both halves are installed).

import type { TranslationDirection } from "./types";

export const LANGUAGE_NAMES: Record<string, string> = { en: "English", de: "German", fr: "French", es: "Spanish", it: "Italian" };

export const languageName = (code: string): string => LANGUAGE_NAMES[code] ?? code;

/** Every language that appears in an installed direction, English first, then by name. */
export function languages(dirs: TranslationDirection[]): string[] {
  const set = new Set<string>();
  for (const d of dirs) if (d.installed) set.add(d.source).add(d.target);
  return [...set].sort((a, b) => (a === "en" ? -1 : b === "en" ? 1 : languageName(a).localeCompare(languageName(b))));
}

/** Whether `source` → `target` can be translated: true, "via-english", or why not. */
export function availability(dirs: TranslationDirection[], source: string | null, target: string): true | "via-english" | string {
  const has = (s: string, t: string) => dirs.some((d) => d.source === s && d.target === t && d.installed);
  if (source === null) {
    // Detected later: the target must be reachable from at least one language.
    return languages(dirs).some((s) => s !== target && (has(s, target) || (s !== "en" && target !== "en" && has(s, "en") && has("en", target))))
      ? true
      : `Nothing can be translated into ${languageName(target)}.`;
  }
  if (source === target) return "Choose two different languages.";
  if (has(source, target)) return true;
  if (source !== "en" && target !== "en" && has(source, "en") && has("en", target)) return "via-english";
  return `Translating from ${languageName(source)} to ${languageName(target)} is not installed.`;
}

// Applies appearance settings to the document root as data attributes and CSS variables.

import type { Settings } from "./types";

/** Text color (#fff or #111) with the better WCAG contrast against `hex`. */
export function contrastText(hex: string): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return "#ffffff";
  const n = parseInt(m[1]!, 16);
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const L = 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
  const withWhite = 1.05 / (L + 0.05);
  const withBlack = (L + 0.05) / 0.05;
  return withWhite >= withBlack ? "#ffffff" : "#111111";
}

export function contrastRatio(a: string, b: string): number {
  const lum = (hex: string) => {
    const n = parseInt(hex.slice(1), 16);
    const ch = (c: number) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * ch((n >> 16) & 255) + 0.7152 * ch((n >> 8) & 255) + 0.0722 * ch(n & 255);
  };
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p) as [number, number];
  return (x + 0.05) / (y + 0.05);
}

/** Apply settings; returns a cleanup that stops following the OS theme. */
export function applyAppearance(s: Pick<Settings, "theme" | "accent" | "density" | "ui_scale" | "motion">, root: HTMLElement = document.documentElement): () => void {
  const media = typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  const setTheme = () => {
    const dark = s.theme === "dark" || (s.theme === "system" && !!media?.matches);
    root.dataset.theme = dark ? "dark" : "light";
  };
  setTheme();
  root.dataset.density = s.density;
  root.dataset.motion = s.motion;
  root.style.setProperty("--accent", s.accent);
  root.style.setProperty("--accent-contrast", contrastText(s.accent));
  root.style.setProperty("--ui-scale", String(s.ui_scale));
  if (s.theme === "system" && media) {
    media.addEventListener("change", setTheme);
    return () => media.removeEventListener("change", setTheme);
  }
  return () => undefined;
}

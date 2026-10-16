// Keyboard shortcut parsing, normalization and conflict detection (INTERACTION_CONTRACT "Keyboard").

const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Meta"] as const;
type Modifier = (typeof MODIFIER_ORDER)[number];

const KEY_ALIASES: Record<string, string> = {
  esc: "Escape",
  escape: "Escape",
  enter: "Enter",
  return: "Enter",
  tab: "Tab",
  space: "Space",
  " ": "Space",
  pageup: "PageUp",
  pagedown: "PageDown",
  home: "Home",
  end: "End",
  delete: "Delete",
  del: "Delete",
  backspace: "Backspace",
  arrowup: "ArrowUp",
  arrowdown: "ArrowDown",
  arrowleft: "ArrowLeft",
  arrowright: "ArrowRight",
  up: "ArrowUp",
  down: "ArrowDown",
  left: "ArrowLeft",
  right: "ArrowRight",
  plus: "Plus",
  "+": "Plus",
  "=": "=",
  "-": "-",
  minus: "-",
  contextmenu: "ContextMenu",
};

/** OS-reserved or accessibility-critical combinations that must never be bound (Windows first). */
export const RESERVED = new Set(["Alt+F4", "Ctrl+Alt+Delete", "Alt+Tab", "Meta+L", "Meta+D", "Ctrl+Escape", "Alt+Space", "Ctrl+Shift+Escape"]);

function normalizeKey(k: string): string | null {
  if (!k) return null;
  const lower = k.toLowerCase();
  if (KEY_ALIASES[lower]) return KEY_ALIASES[lower]!;
  if (/^f([1-9]|1[0-9]|2[0-4])$/.test(lower)) return lower.toUpperCase();
  if (k.length === 1) return k.toUpperCase();
  return null;
}

/** Parse "ctrl + shift + o" → "Ctrl+Shift+O". Returns null for invalid or modifier-only input. */
export function normalizeShortcut(input: string): string | null {
  const raw = input.trim();
  if (!raw) return null;
  // Allow a literal "+" key at the end: "Ctrl++".
  const parts = raw.endsWith("++") ? [...raw.slice(0, -2).split("+"), "+"] : raw.split("+");
  const mods = new Set<Modifier>();
  let key: string | null = null;
  for (const p of parts.map((s) => s.trim()).filter(Boolean)) {
    const lower = p.toLowerCase();
    if (lower === "ctrl" || lower === "control" || lower === "cmdorctrl") mods.add("Ctrl");
    else if (lower === "alt" || lower === "option") mods.add("Alt");
    else if (lower === "shift") mods.add("Shift");
    else if (lower === "meta" || lower === "win" || lower === "cmd" || lower === "super") mods.add("Meta");
    else {
      if (key !== null) return null;
      key = normalizeKey(p);
      if (key === null) return null;
    }
  }
  if (key === null) return null;
  return [...MODIFIER_ORDER.filter((m) => mods.has(m)), key].join("+");
}

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Shortcut string for a keyboard event, or null for bare modifier presses. */
export function eventToShortcut(e: KeyLike): string | null {
  if (["Control", "Alt", "Shift", "Meta", "AltGraph", "CapsLock", "Dead", "Process", "Unidentified"].includes(e.key)) return null;
  const key = normalizeKey(e.key);
  if (key === null) return null;
  const mods: Modifier[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  // Shift is implied by printable symbols such as "+"; keep it only for letters/named keys.
  if (e.shiftKey && !(key.length === 1 && !/[A-Z0-9]/.test(key)) && key !== "Plus") mods.push("Shift");
  if (e.metaKey) mods.push("Meta");
  return [...mods, key].join("+");
}

export type ValidationResult = { ok: true; value: string } | { ok: false; reason: string };

export function validateShortcut(input: string): ValidationResult {
  const n = normalizeShortcut(input);
  if (n === null) return { ok: false, reason: "Not a valid key combination." };
  if (RESERVED.has(n)) return { ok: false, reason: `${n} is reserved by Windows.` };
  const isFunctionKey = /^F\d+$/.test(n.split("+").pop() ?? "");
  const hasModifier = n.includes("+");
  if (!hasModifier && !isFunctionKey && !["Escape", "Delete", "PageUp", "PageDown", "Home", "End"].includes(n)) {
    return { ok: false, reason: "Add Ctrl or Alt so typing is not interrupted." };
  }
  return { ok: true, value: n };
}

/** Returns shortcut → list of command IDs that share it (only entries with 2+ commands). */
export function findConflicts(bindings: Record<string, string[]>): Map<string, string[]> {
  const byKey = new Map<string, string[]>();
  for (const [id, keys] of Object.entries(bindings)) {
    for (const k of keys) {
      const n = normalizeShortcut(k);
      if (!n) continue;
      byKey.set(n, [...(byKey.get(n) ?? []), id]);
    }
  }
  return new Map([...byKey].filter(([, ids]) => ids.length > 1));
}

/** Whether keyboard focus is in a text-entry control, where most app shortcuts must not fire. */
export function isTextEntry(el: Element | null): boolean {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    return !["button", "checkbox", "radio", "range", "color", "submit", "reset", "file"].includes(el.type);
  }
  return (el as HTMLElement).isContentEditable === true;
}

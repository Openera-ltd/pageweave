// Central command registry (PRD 02 "Keyboard and command registry"). Every toolbar button, menu item,
// context-menu entry and shortcut dispatches one of these IDs. Handlers and enablement live in App,
// which supplies them through `CommandHandlers`; a command without a handler is a test failure.

import { findConflicts, normalizeShortcut } from "./shortcuts";

export type CommandId =
  | "file.open"
  | "file.close"
  | "file.reveal"
  | "file.properties"
  | "tab.next"
  | "tab.previous"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.actualSize"
  | "view.fitWidth"
  | "view.toggleThumbnails"
  | "nav.nextPage"
  | "nav.previousPage"
  | "nav.firstPage"
  | "nav.lastPage"
  | "nav.goToPage"
  | "search.open"
  | "search.next"
  | "search.previous"
  | "search.close"
  | "pages.extract"
  | "pages.delete"
  | "pages.rotate"
  | "pages.split"
  | "pages.merge"
  | "app.settings"
  | "app.shortcuts"
  | "app.commandPalette"
  | "app.openLogs";

export interface CommandSpec {
  id: CommandId;
  label: string;
  category: "File" | "View" | "Navigate" | "Search" | "Pages" | "App";
  /** Default bindings; the first is shown in menus. */
  shortcuts: string[];
  /** Allowed while typing in a text field (e.g. Escape, F3). */
  allowInInput?: boolean;
}

export const COMMANDS: readonly CommandSpec[] = [
  { id: "file.open", label: "Open PDF…", category: "File", shortcuts: ["Ctrl+O"], allowInInput: true },
  { id: "file.close", label: "Close tab", category: "File", shortcuts: ["Ctrl+W", "Ctrl+F4"], allowInInput: true },
  { id: "file.reveal", label: "Show in folder", category: "File", shortcuts: [] },
  { id: "file.properties", label: "Document properties", category: "File", shortcuts: ["Ctrl+D"] },
  { id: "tab.next", label: "Next tab", category: "View", shortcuts: ["Ctrl+Tab"], allowInInput: true },
  { id: "tab.previous", label: "Previous tab", category: "View", shortcuts: ["Ctrl+Shift+Tab"], allowInInput: true },
  { id: "view.zoomIn", label: "Zoom in", category: "View", shortcuts: ["Ctrl+=", "Ctrl+Plus"] },
  { id: "view.zoomOut", label: "Zoom out", category: "View", shortcuts: ["Ctrl+-"] },
  { id: "view.actualSize", label: "Actual size (100%)", category: "View", shortcuts: ["Ctrl+0"] },
  { id: "view.fitWidth", label: "Fit width", category: "View", shortcuts: ["Ctrl+2"] },
  { id: "view.toggleThumbnails", label: "Show or hide page thumbnails", category: "View", shortcuts: ["Ctrl+B"] },
  { id: "nav.nextPage", label: "Next page", category: "Navigate", shortcuts: ["Ctrl+PageDown"] },
  { id: "nav.previousPage", label: "Previous page", category: "Navigate", shortcuts: ["Ctrl+PageUp"] },
  { id: "nav.firstPage", label: "First page", category: "Navigate", shortcuts: ["Ctrl+Home"] },
  { id: "nav.lastPage", label: "Last page", category: "Navigate", shortcuts: ["Ctrl+End"] },
  { id: "nav.goToPage", label: "Go to page…", category: "Navigate", shortcuts: ["Ctrl+G"] },
  { id: "search.open", label: "Find in document", category: "Search", shortcuts: ["Ctrl+F"], allowInInput: true },
  { id: "search.next", label: "Find next", category: "Search", shortcuts: ["F3"], allowInInput: true },
  { id: "search.previous", label: "Find previous", category: "Search", shortcuts: ["Shift+F3"], allowInInput: true },
  { id: "search.close", label: "Close find", category: "Search", shortcuts: [] },
  { id: "pages.extract", label: "Extract pages…", category: "Pages", shortcuts: ["Ctrl+Shift+E"] },
  { id: "pages.delete", label: "Delete pages…", category: "Pages", shortcuts: ["Ctrl+Shift+D"] },
  { id: "pages.rotate", label: "Rotate pages…", category: "Pages", shortcuts: ["Ctrl+Shift+R"] },
  { id: "pages.split", label: "Split document…", category: "Pages", shortcuts: [] },
  { id: "pages.merge", label: "Combine files…", category: "Pages", shortcuts: ["Ctrl+Shift+M"] },
  { id: "app.settings", label: "Settings", category: "App", shortcuts: ["Ctrl+,"] },
  { id: "app.shortcuts", label: "Keyboard shortcuts", category: "App", shortcuts: [] },
  { id: "app.commandPalette", label: "Command palette", category: "App", shortcuts: ["Ctrl+K", "Ctrl+Shift+P"], allowInInput: true },
  { id: "app.openLogs", label: "Show log folder", category: "App", shortcuts: [] },
];

export const COMMAND_BY_ID: ReadonlyMap<CommandId, CommandSpec> = new Map(COMMANDS.map((c) => [c.id, c]));

/** Enabled, or disabled with a user-facing reason. */
export type Availability = true | string;

export interface CommandHandler {
  run: () => void;
  available: () => Availability;
}

export type CommandHandlers = Record<CommandId, CommandHandler>;

/** Effective bindings after user overrides (override "" = unbound). */
export function effectiveBindings(overrides: Record<string, string>): Record<CommandId, string[]> {
  const out = {} as Record<CommandId, string[]>;
  for (const c of COMMANDS) {
    const o = overrides[c.id];
    if (o === undefined) out[c.id] = c.shortcuts.map((s) => normalizeShortcut(s)!).filter(Boolean);
    else out[c.id] = o === "" ? [] : [normalizeShortcut(o) ?? o];
  }
  return out;
}

export function bindingConflicts(overrides: Record<string, string>): Map<string, string[]> {
  return findConflicts(effectiveBindings(overrides));
}

export function commandForShortcut(shortcut: string, bindings: Record<CommandId, string[]>): CommandId | undefined {
  for (const [id, keys] of Object.entries(bindings) as Array<[CommandId, string[]]>) {
    if (keys.includes(shortcut)) return id;
  }
  return undefined;
}

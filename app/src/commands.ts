// Central command registry (PRD 02 "Keyboard and command registry"). Every toolbar button, menu item,
// context-menu entry and shortcut dispatches one of these IDs. Handlers and enablement live in App,
// which supplies them through `CommandHandlers`; a command without a handler is a test failure.

import { findConflicts, normalizeShortcut } from "./shortcuts";

export type CommandId =
  | "file.open"
  | "file.close"
  | "file.reveal"
  | "file.properties"
  | "file.protect"
  | "file.editProperties"
  | "file.attachFiles"
  | "file.editBookmarks"
  | "file.imagesToPdf"
  | "file.unprotect"
  | "file.sanitize"
  | "file.exportFormData"
  | "file.importFormData"
  | "edit.clearForm"
  | "file.reduceSize"
  | "file.compare"
  | "file.print"
  | "file.sign"
  | "file.signatureMark"
  | "file.trustedCertificates"
  | "edit.copy"
  | "edit.undo"
  | "edit.redo"
  | "edit.toggleEditMode"
  | "edit.addText"
  | "annotate.highlight"
  | "annotate.underline"
  | "annotate.strikeOut"
  | "annotate.note"
  | "redact.markSelection"
  | "redact.markArea"
  | "redact.markPage"
  | "redact.markPatterns"
  | "redact.markMatches"
  | "redact.apply"
  | "file.saveCopy"
  | "edit.discardChanges"
  | "tab.next"
  | "tab.previous"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.actualSize"
  | "view.fitWidth"
  | "view.toggleThumbnails"
  | "view.showBookmarks"
  | "view.showSignatures"
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
  | "pages.organize"
  | "pages.stamp"
  | "pages.exportImages"
  | "pages.exportText"
  | "pages.deskew"
  | "pages.orient"
  | "pages.removeBlank"
  | "pages.extractImages"
  | "pages.ocr"
  | "pages.watermark"
  | "pages.crop"
  | "app.settings"
  | "app.shortcuts"
  | "app.commandPalette"
  | "app.openLogs";

export interface CommandSpec {
  id: CommandId;
  label: string;
  category: "File" | "Edit" | "View" | "Navigate" | "Search" | "Pages" | "Annotate" | "Redact" | "App";
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
  { id: "file.protect", label: "Protect with password…", category: "File", shortcuts: [] },
  { id: "file.imagesToPdf", label: "Create PDF from images…", category: "File", shortcuts: [] },
  { id: "file.editBookmarks", label: "Edit bookmarks…", category: "File", shortcuts: [] },
  { id: "file.attachFiles", label: "Attach files…", category: "File", shortcuts: [] },
  { id: "file.editProperties", label: "Edit document properties…", category: "File", shortcuts: [] },
  { id: "file.unprotect", label: "Remove password…", category: "File", shortcuts: [] },
  { id: "file.sanitize", label: "Remove hidden information…", category: "File", shortcuts: [] },
  { id: "file.exportFormData", label: "Export form data…", category: "File", shortcuts: [] },
  { id: "file.importFormData", label: "Import form data…", category: "File", shortcuts: [] },
  { id: "edit.clearForm", label: "Clear form fields", category: "Edit", shortcuts: [] },
  { id: "file.reduceSize", label: "Reduce file size…", category: "File", shortcuts: [] },
  { id: "file.compare", label: "Compare with…", category: "File", shortcuts: [] },
  { id: "file.print", label: "Print…", category: "File", shortcuts: ["Ctrl+P"] },
  { id: "file.sign", label: "Sign with a certificate…", category: "File", shortcuts: [] },
  { id: "file.signatureMark", label: "Add signature or initials…", category: "File", shortcuts: [] },
  { id: "file.trustedCertificates", label: "Trusted certificates…", category: "File", shortcuts: [] },
  // Ctrl+C is handled natively by the text layer; this command backs the context-menu entry.
  { id: "edit.copy", label: "Copy selected text", category: "Edit", shortcuts: [] },
  { id: "edit.undo", label: "Undo", category: "Edit", shortcuts: ["Ctrl+Z"] },
  { id: "edit.redo", label: "Redo", category: "Edit", shortcuts: ["Ctrl+Y", "Ctrl+Shift+Z"] },
  { id: "edit.toggleEditMode", label: "Edit text and pictures", category: "Edit", shortcuts: ["Ctrl+E"] },
  { id: "edit.addText", label: "Add text", category: "Edit", shortcuts: [] },
  { id: "annotate.highlight", label: "Highlight selected text", category: "Annotate", shortcuts: ["Ctrl+Shift+H"] },
  { id: "annotate.underline", label: "Underline selected text", category: "Annotate", shortcuts: ["Ctrl+Shift+U"] },
  { id: "annotate.strikeOut", label: "Strike through selected text", category: "Annotate", shortcuts: [] },
  { id: "annotate.note", label: "Add note…", category: "Annotate", shortcuts: [] },
  { id: "redact.markSelection", label: "Mark selected text for redaction", category: "Redact", shortcuts: ["Ctrl+Shift+X"] },
  { id: "redact.markArea", label: "Mark areas for redaction", category: "Redact", shortcuts: [] },
  { id: "redact.markPage", label: "Mark current page for redaction", category: "Redact", shortcuts: [] },
  { id: "redact.markPatterns", label: "Mark patterns for redaction…", category: "Redact", shortcuts: [] },
  { id: "redact.markMatches", label: "Mark all search matches for redaction", category: "Redact", shortcuts: [], allowInInput: true },
  { id: "redact.apply", label: "Apply redactions…", category: "Redact", shortcuts: [] },
  { id: "file.saveCopy", label: "Save a copy with changes…", category: "File", shortcuts: ["Ctrl+S"], allowInInput: true },
  { id: "edit.discardChanges", label: "Discard unsaved changes", category: "Edit", shortcuts: [] },
  { id: "tab.next", label: "Next tab", category: "View", shortcuts: ["Ctrl+Tab"], allowInInput: true },
  { id: "tab.previous", label: "Previous tab", category: "View", shortcuts: ["Ctrl+Shift+Tab"], allowInInput: true },
  { id: "view.zoomIn", label: "Zoom in", category: "View", shortcuts: ["Ctrl+=", "Ctrl+Plus"] },
  { id: "view.zoomOut", label: "Zoom out", category: "View", shortcuts: ["Ctrl+-"] },
  { id: "view.actualSize", label: "Actual size (100%)", category: "View", shortcuts: ["Ctrl+0"] },
  { id: "view.fitWidth", label: "Fit width", category: "View", shortcuts: ["Ctrl+2"] },
  { id: "view.toggleThumbnails", label: "Show or hide page thumbnails", category: "View", shortcuts: ["Ctrl+B"] },
  { id: "view.showBookmarks", label: "Show bookmarks", category: "View", shortcuts: ["Ctrl+Shift+B"] },
  { id: "view.showSignatures", label: "Show signatures", category: "View", shortcuts: [] },
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
  { id: "pages.organize", label: "Organize pages…", category: "Pages", shortcuts: ["Ctrl+Shift+O"] },
  { id: "pages.stamp", label: "Page numbers and stamps…", category: "Pages", shortcuts: [] },
  { id: "pages.exportImages", label: "Export pages as images…", category: "Pages", shortcuts: [] },
  { id: "pages.exportText", label: "Export as text…", category: "Pages", shortcuts: [] },
  { id: "pages.deskew", label: "Straighten pages…", category: "Pages", shortcuts: [] },
  { id: "pages.orient", label: "Turn pages upright…", category: "Pages", shortcuts: [] },
  { id: "pages.removeBlank", label: "Remove blank pages…", category: "Pages", shortcuts: [] },
  { id: "pages.extractImages", label: "Extract images…", category: "Pages", shortcuts: [] },
  { id: "pages.ocr", label: "Recognize text (OCR)…", category: "Pages", shortcuts: [] },
  { id: "pages.watermark", label: "Watermark…", category: "Pages", shortcuts: [] },
  { id: "pages.crop", label: "Crop pages…", category: "Pages", shortcuts: [] },
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

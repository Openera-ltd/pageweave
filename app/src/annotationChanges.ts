// Pending changes to annotations already in the file: delete, new comment text, colour or
// position, keyed by page and position. Saved with the other edits into one copy (apply_edits.annotation_changes).

import type { AnnotationChange, ExistingAnnotation } from "./types";

export type AnnotChanges = Record<string, AnnotationChange>;

export const keyOf = (a: { page: number; index: number }) => `${a.page}:${a.index}`;

export function markDeleted(changes: AnnotChanges, a: ExistingAnnotation): AnnotChanges {
  return { ...changes, [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: true, text: null, color: null } };
}

/** Kinds whose colour can be changed (pw-core `RECOLOURABLE`): drawn from their colour. */
export const RECOLOURABLE = new Set(["Highlight", "Underline", "StrikeOut", "Squiggly", "Square", "Circle", "Ink", "Line", "Polygon", "PolyLine", "Text"]);

/** Text, colour and position changes of one annotation together; back to the original drops the change. */
function withChange(changes: AnnotChanges, a: ExistingAnnotation, patch: { text?: string; color?: string; offset?: [number, number] }): AnnotChanges {
  const { [keyOf(a)]: old, ...rest } = changes;
  const live = old && !old.delete ? old : null;
  const text = patch.text !== undefined ? patch.text : (live?.text ?? null);
  const color = patch.color !== undefined ? patch.color : (live?.color ?? null);
  const offset = patch.offset !== undefined ? patch.offset : (live?.offset ?? null);
  const textChanged = text !== null && text.trim() !== (a.text ?? "").trim();
  const colorChanged = color !== null && color.toLowerCase() !== (a.color ?? "").toLowerCase();
  // Moves are kept to 0.01 pt; a move back to where it was is no move.
  const moved = offset && (Math.abs(offset[0]) >= 0.01 || Math.abs(offset[1]) >= 0.01) ? ([round2(offset[0]), round2(offset[1])] as [number, number]) : null;
  if (!textChanged && !colorChanged && !moved) return rest;
  return { ...rest, [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: false, text: textChanged ? text : null, color: colorChanged ? color : null, ...(moved ? { offset: moved } : {}) } };
}

const round2 = (v: number) => Math.round(v * 100) / 100;

/** Pending move of an annotation, [dx, dy] PDF points ([0, 0] when it is not moved). */
export function offsetOf(changes: AnnotChanges, a: { page: number; index: number }): [number, number] {
  const c = changes[keyOf(a)];
  return c && !c.delete && c.offset ? c.offset : [0, 0];
}

/** Move to `offset` from where it is in the file; [0, 0] drops the move. */
export function setOffset(changes: AnnotChanges, a: ExistingAnnotation, offset: [number, number]): AnnotChanges {
  return withChange(changes, a, { offset });
}

/** New comment text; going back to the original text drops that part of the change. */
export function setText(changes: AnnotChanges, a: ExistingAnnotation, text: string): AnnotChanges {
  return withChange(changes, a, { text });
}

/** New colour "#rrggbb"; going back to the original colour drops that part of the change. */
export function setColor(changes: AnnotChanges, a: ExistingAnnotation, color: string): AnnotChanges {
  return withChange(changes, a, { color });
}

export function revert(changes: AnnotChanges, a: ExistingAnnotation): AnnotChanges {
  const { [keyOf(a)]: _old, ...rest } = changes;
  return rest;
}

/** Changes for the engine, in page and position order. */
export function toChanges(changes: AnnotChanges): AnnotationChange[] {
  return Object.values(changes).sort((x, y) => x.page - y.page || x.index - y.index);
}

/** What people call each kind. */
export const KIND_NAME: Record<string, string> = {
  Text: "Note",
  FreeText: "Text box",
  Highlight: "Highlight",
  Underline: "Underline",
  StrikeOut: "Strikethrough",
  Squiggly: "Squiggly underline",
  Ink: "Drawing",
  Square: "Rectangle",
  Circle: "Ellipse",
  Line: "Line",
  Polygon: "Polygon",
  PolyLine: "Polyline",
  Stamp: "Stamp",
  Caret: "Insert mark",
  FileAttachment: "Attached file",
  Sound: "Sound",
  Redact: "Redaction mark",
};
export const kindName = (s: string) => KIND_NAME[s] ?? s;

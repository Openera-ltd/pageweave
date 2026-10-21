// Pending changes to annotations already in the file: delete or new comment text, keyed by
// page and position. Saved with the other edits into one copy (apply_edits.annotation_changes).

import type { AnnotationChange, ExistingAnnotation } from "./types";

export type AnnotChanges = Record<string, AnnotationChange>;

export const keyOf = (a: { page: number; index: number }) => `${a.page}:${a.index}`;

export function markDeleted(changes: AnnotChanges, a: ExistingAnnotation): AnnotChanges {
  return { ...changes, [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: true, text: null } };
}

/** New comment text; going back to the original text drops the change. */
export function setText(changes: AnnotChanges, a: ExistingAnnotation, text: string): AnnotChanges {
  const { [keyOf(a)]: _old, ...rest } = changes;
  if (text.trim() === (a.text ?? "").trim()) return rest;
  return { ...rest, [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: false, text } };
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

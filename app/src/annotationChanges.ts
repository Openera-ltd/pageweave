// Pending changes to annotations already in the file: delete, new comment text, colour or
// position, keyed by page and position. Saved with the other edits into one copy (apply_edits.annotation_changes).

import type { AnnotationChange, ExistingAnnotation, SearchRect } from "./types";

export type AnnotChanges = Record<string, AnnotationChange>;

export const keyOf = (a: { page: number; index: number }) => `${a.page}:${a.index}`;

export function markDeleted(changes: AnnotChanges, a: ExistingAnnotation): AnnotChanges {
  return { ...changes, [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: true, text: null, color: null } };
}

/** Kinds whose colour can be changed (pw-core `RECOLOURABLE`): drawn from their colour. */
export const RECOLOURABLE = new Set(["Highlight", "Underline", "StrikeOut", "Squiggly", "Square", "Circle", "Ink", "Line", "Polygon", "PolyLine", "Text"]);

/** Kinds that can be resized (pw-core `RESIZABLE`): shapes and drawings are redrawn, stamps stretched. */
export const RESIZABLE = new Set(["Square", "Circle", "Ink", "Line", "Polygon", "PolyLine", "Stamp"]);

/**
 * Text, colour, position and size changes of one annotation together; back to the original drops
 * the change. A size change is the new box (which may also be moved); without one, a move is an offset.
 */
function withChange(changes: AnnotChanges, a: ExistingAnnotation, patch: { text?: string; color?: string; offset?: [number, number]; rect?: SearchRect }): AnnotChanges {
  const { [keyOf(a)]: old, ...rest } = changes;
  const live = old && !old.delete ? old : null;
  const text = patch.text !== undefined ? patch.text : (live?.text ?? null);
  const color = patch.color !== undefined ? patch.color : (live?.color ?? null);
  const rect = patch.rect !== undefined ? patch.rect : (live?.rect ?? null);
  const offset = rect ? null : patch.offset !== undefined ? patch.offset : (live?.offset ?? null);
  const textChanged = text !== null && text.trim() !== (a.text ?? "").trim();
  const colorChanged = color !== null && color.toLowerCase() !== (a.color ?? "").toLowerCase();
  // Places are kept to 0.01 pt; back to where it was is no change.
  const moved = offset && (Math.abs(offset[0]) >= 0.01 || Math.abs(offset[1]) >= 0.01) ? ([round2(offset[0]), round2(offset[1])] as [number, number]) : null;
  const box = rect ? { left: round2(rect.left), bottom: round2(rect.bottom), right: round2(rect.right), top: round2(rect.top) } : null;
  const resized = box && (["left", "bottom", "right", "top"] as const).some((k) => Math.abs(box[k] - a.rect[k]) >= 0.01) ? box : null;
  if (!textChanged && !colorChanged && !moved && !resized) return rest;
  return {
    ...rest,
    [keyOf(a)]: { page: a.page, index: a.index, subtype: a.subtype, delete: false, text: textChanged ? text : null, color: colorChanged ? color : null, ...(moved ? { offset: moved } : {}), ...(resized ? { rect: resized } : {}) },
  };
}

const round2 = (v: number) => Math.round(v * 100) / 100;

/** Where the annotation will be: its new box, or its place in the file moved by the pending offset. */
export function boundsOf(changes: AnnotChanges, a: ExistingAnnotation): SearchRect {
  const c = changes[keyOf(a)];
  if (c && !c.delete && c.rect) return c.rect;
  const [dx, dy] = offsetOf(changes, a);
  return { left: a.rect.left + dx, bottom: a.rect.bottom + dy, right: a.rect.right + dx, top: a.rect.top + dy };
}

/** Move by [dx, dy] more points (shifts a new box when it has one). */
export function moveBy(changes: AnnotChanges, a: ExistingAnnotation, dx: number, dy: number): AnnotChanges {
  const c = changes[keyOf(a)];
  if (c && !c.delete && c.rect) {
    const r = c.rect;
    return withChange(changes, a, { rect: { left: r.left + dx, bottom: r.bottom + dy, right: r.right + dx, top: r.top + dy } });
  }
  const [x, y] = offsetOf(changes, a);
  return withChange(changes, a, { offset: [x + dx, y + dy] });
}

/** New box (shapes, drawings, stamps); the box it has in the file drops the size change. */
export function setBounds(changes: AnnotChanges, a: ExistingAnnotation, rect: SearchRect): AnnotChanges {
  return withChange(changes, a, { rect });
}

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

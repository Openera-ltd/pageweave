// Design Studio editing (D-028): pure functions over a design, so every edit is one undoable step
// and can be tested without the canvas. Locked objects never move or resize.

import type { Design, DesignObject } from "./types";

export interface History {
  past: Design[];
  present: Design;
  future: Design[];
  /** Field of the last change: typing in the same field merges into one undo step. */
  key?: string;
}

const LIMIT = 200;

export const start = (d: Design): History => ({ past: [], present: d, future: [] });

/** Apply a change as one undo step; a change that returns the same design is not recorded, and
 * repeated changes with the same `key` (typing in one field) are one step. */
export function commit(h: History, next: Design, key?: string): History {
  if (next === h.present) return h;
  if (key && key === h.key) return { ...h, present: next, future: [] };
  return { past: [...h.past, h.present].slice(-LIMIT), present: next, future: [], key };
}

export function undo(h: History): History {
  const prev = h.past[h.past.length - 1];
  return prev ? { past: h.past.slice(0, -1), present: prev, future: [h.present, ...h.future] } : h;
}

export function redo(h: History): History {
  const next = h.future[0];
  return next ? { past: [...h.past, h.present], present: next, future: h.future.slice(1) } : h;
}

function mapObject(d: Design, id: string, f: (o: DesignObject) => DesignObject): Design {
  let changed = false;
  const objects = d.objects.map((o) => {
    if (o.id !== id) return o;
    const n = f(o);
    changed ||= n !== o;
    return n;
  });
  return changed ? { ...d, objects } : d;
}

export function update(d: Design, id: string, patch: Partial<DesignObject>): Design {
  return mapObject(d, id, (o) => ({ ...o, ...patch }));
}

export function moveBy(d: Design, id: string, dx: number, dy: number): Design {
  if (!dx && !dy) return d;
  return mapObject(d, id, (o) => (o.locked ? o : { ...o, x: round(o.x + dx), y: round(o.y + dy) }));
}

/** New box for an object; text keeps its proportions to the height (size and baseline scale). */
export function resize(d: Design, id: string, box: { x: number; y: number; w: number; h: number }): Design {
  const w = Math.max(1, round(box.w));
  const h = Math.max(1, round(box.h));
  return mapObject(d, id, (o) => {
    if (o.locked) return o;
    const k = h / (o.h || 1);
    const text = o.kind === "text" ? { font_size: round((o.font_size ?? o.h) * k), baseline: round((o.baseline ?? o.h * 0.8) * k) } : {};
    return { ...o, ...text, x: round(box.x), y: round(box.y), w, h };
  });
}

/** Move one step toward the front (+1) or back (−1) of the stack. */
export function reorder(d: Design, id: string, step: 1 | -1): Design {
  const i = d.objects.findIndex((o) => o.id === id);
  const j = i + step;
  if (i < 0 || j < 0 || j >= d.objects.length) return d;
  const objects = [...d.objects];
  [objects[i], objects[j]] = [objects[j]!, objects[i]!];
  return { ...d, objects };
}

export function remove(d: Design, id: string): Design {
  return d.objects.some((o) => o.id === id) ? { ...d, objects: d.objects.filter((o) => o.id !== id) } : d;
}

export function duplicate(d: Design, id: string): { design: Design; id: string | null } {
  const i = d.objects.findIndex((o) => o.id === id);
  const o = d.objects[i];
  if (!o) return { design: d, id: null };
  let n = 2;
  while (d.objects.some((x) => x.id === `${o.id}-copy${n}`)) n++;
  const copy = { ...o, id: `${o.id}-copy${n}`, name: `${o.name} (copy)`, x: o.x + 10, y: o.y + 10, locked: false };
  const objects = [...d.objects];
  objects.splice(i + 1, 0, copy);
  return { design: { ...d, objects }, id: copy.id };
}

/** Objects the reconstruction is not sure of: pictures, and text or shapes under 80 % confidence. */
export const uncertain = (o: DesignObject) => o.kind === "image" || o.confidence < 0.8;

/** The topmost visible object under a point (design pixels), front first. */
export function hit(d: Design, x: number, y: number): DesignObject | null {
  for (let i = d.objects.length - 1; i >= 0; i--) {
    const o = d.objects[i]!;
    if (o.visible && x >= o.x && x <= o.x + o.w && y >= o.y && y <= o.y + o.h) return o;
  }
  return null;
}

const round = (v: number) => Math.round(v * 10) / 10;

export const isDesignPath = (p: string) => /\.pwdesign$/i.test(p);
export const isStudioSource = (p: string) => /\.(png|jpe?g|pdf|pwdesign)$/i.test(p);

/** What "Open in Design Studio" does with a chosen file: open a project, rebuild a picture or a
 * PDF's first page, or nothing for other files. */
export function studioSourceFor(path: string): { kind: "open"; path: string } | { kind: "rebuild"; input: string; page: number } | null {
  if (isDesignPath(path)) return { kind: "open", path };
  return isStudioSource(path) ? { kind: "rebuild", input: path, page: 0 } : null;
}

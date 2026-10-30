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
  return mapObject(d, id, (o) => fitRadius({ ...o, ...patch }));
}

/** A corner radius never exceeds half the shorter side (the engine refuses more). */
function fitRadius(o: DesignObject): DesignObject {
  const max = Math.min(o.w, o.h) / 2;
  return o.radius && o.radius > max ? { ...o, radius: Math.floor(max * 10) / 10 } : o;
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
    return fitRadius({ ...o, ...text, x: round(box.x), y: round(box.y), w, h });
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

/** A new object in the middle of the canvas, placed in front: "New text" (black, 6 % of the
 * height) or a rectangle in the accent blue. Returns the design and the new object's id. */
export function addObject(d: Design, kind: "text" | "shape"): { design: Design; id: string } {
  let n = d.objects.length + 1;
  while (d.objects.some((o) => o.id === `${kind === "text" ? "text" : "shape"}-new${n}`)) n++;
  const id = `${kind === "text" ? "text" : "shape"}-new${n}`;
  const size = round(Math.max(8, d.height * 0.06));
  const base = { id, visible: true, locked: false, confidence: 1, bold: false, note: null };
  const o: DesignObject =
    kind === "text"
      ? { ...base, name: "New text", kind: "text", text: "New text", color: "#000000", font_size: size, baseline: round(size * 0.72), w: round(size * 4.2), h: round(size * 0.93), x: 0, y: 0 }
      : { ...base, name: "Rectangle", kind: "shape", fill: "#2f6fde", w: round(d.width * 0.25), h: round(d.height * 0.12), x: 0, y: 0 };
  o.x = round((d.width - o.w) / 2);
  o.y = round((d.height - o.h) / 2);
  return { design: { ...d, objects: [...d.objects, o] }, id };
}

export interface Guide {
  /** "x": a vertical line at `at`; "y": a horizontal line. */
  axis: "x" | "y";
  at: number;
}

/** Snap a moving box to the canvas edges and centre and to the other visible objects' edges and
 * centres, when within `threshold` design pixels; returns the snapped position and the guides to
 * draw. Each axis snaps to its nearest match only. */
export function snap(d: Design, moving: string | string[], box: { x: number; y: number; w: number; h: number }, threshold: number): { x: number; y: number; guides: Guide[] } {
  const skip = new Set(Array.isArray(moving) ? moving : [moving]);
  const others = d.objects.filter((o) => !skip.has(o.id) && o.visible);
  const targets = (axis: "x" | "y") => {
    const size = axis === "x" ? d.width : d.height;
    const t = [0, size / 2, size];
    for (const o of others) {
      const [p, s] = axis === "x" ? [o.x, o.w] : [o.y, o.h];
      t.push(p, p + s / 2, p + s);
    }
    return t;
  };
  const fit = (axis: "x" | "y", pos: number, len: number) => {
    let best: { delta: number; at: number } | null = null;
    for (const t of targets(axis)) {
      for (const edge of [pos, pos + len / 2, pos + len]) {
        const delta = t - edge;
        if (Math.abs(delta) <= threshold && (!best || Math.abs(delta) < Math.abs(best.delta))) best = { delta, at: t };
      }
    }
    return best;
  };
  const fx = fit("x", box.x, box.w);
  const fy = fit("y", box.y, box.h);
  const guides: Guide[] = [];
  if (fx) guides.push({ axis: "x", at: fx.at });
  if (fy) guides.push({ axis: "y", at: fy.at });
  return { x: round(box.x + (fx?.delta ?? 0)), y: round(box.y + (fy?.delta ?? 0)), guides };
}

/* ---------- Several objects ---------- */

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The objects a click on `id` selects: its whole group when it has one. */
export function withGroup(d: Design, id: string): string[] {
  const g = d.objects.find((o) => o.id === id)?.group;
  return g ? d.objects.filter((o) => o.group === g).map((o) => o.id) : [id];
}

/** The box around several objects (null when none). */
export function bounds(d: Design, ids: string[]): Box | null {
  const os = d.objects.filter((o) => ids.includes(o.id));
  if (!os.length) return null;
  const x = Math.min(...os.map((o) => o.x));
  const y = Math.min(...os.map((o) => o.y));
  return { x, y, w: Math.max(...os.map((o) => o.x + o.w)) - x, h: Math.max(...os.map((o) => o.y + o.h)) - y };
}

/** Move several objects together (locked ones stay). */
export function moveMany(d: Design, ids: string[], dx: number, dy: number): Design {
  if (!dx && !dy) return d;
  let changed = false;
  const objects = d.objects.map((o) => {
    if (!ids.includes(o.id) || o.locked) return o;
    changed = true;
    return { ...o, x: round(o.x + dx), y: round(o.y + dy) };
  });
  return changed ? { ...d, objects } : d;
}

export function removeMany(d: Design, ids: string[]): Design {
  const objects = d.objects.filter((o) => !ids.includes(o.id));
  return objects.length === d.objects.length ? d : { ...d, objects };
}

export type Align = "left" | "center" | "right" | "top" | "middle" | "bottom";

/** Line several objects up on the selection's box (one object: on the canvas). Locked ones stay. */
export function align(d: Design, ids: string[], how: Align): Design {
  const box = ids.length > 1 ? bounds(d, ids) : { x: 0, y: 0, w: d.width, h: d.height };
  if (!box) return d;
  let changed = false;
  const objects = d.objects.map((o) => {
    if (!ids.includes(o.id) || o.locked) return o;
    const x = how === "left" ? box.x : how === "center" ? box.x + (box.w - o.w) / 2 : how === "right" ? box.x + box.w - o.w : o.x;
    const y = how === "top" ? box.y : how === "middle" ? box.y + (box.h - o.h) / 2 : how === "bottom" ? box.y + box.h - o.h : o.y;
    if (round(x) === o.x && round(y) === o.y) return o;
    changed = true;
    return { ...o, x: round(x), y: round(y) };
  });
  return changed ? { ...d, objects } : d;
}

/** Equal gaps between three or more objects along an axis; the outermost ones stay put. */
export function distribute(d: Design, ids: string[], axis: "x" | "y"): Design {
  const os = d.objects.filter((o) => ids.includes(o.id)).sort((a, b) => (axis === "x" ? a.x - b.x : a.y - b.y));
  if (os.length < 3) return d;
  const pos = (o: DesignObject) => (axis === "x" ? o.x : o.y);
  const len = (o: DesignObject) => (axis === "x" ? o.w : o.h);
  const first = os[0]!;
  const last = os[os.length - 1]!;
  const total = pos(last) + len(last) - pos(first);
  const gap = (total - os.reduce((n, o) => n + len(o), 0)) / (os.length - 1);
  const place = new Map<string, number>();
  let at = pos(first);
  for (const o of os) {
    place.set(o.id, round(at));
    at += len(o) + gap;
  }
  let changed = false;
  const objects = d.objects.map((o) => {
    const p = place.get(o.id);
    if (p === undefined || o.locked || p === pos(o)) return o;
    changed = true;
    return axis === "x" ? { ...o, x: p } : { ...o, y: p };
  });
  return changed ? { ...d, objects } : d;
}

/** Group two or more objects (a new group id); `ungroup` removes their groups. */
export function group(d: Design, ids: string[]): Design {
  if (ids.length < 2) return d;
  let n = 1;
  while (d.objects.some((o) => o.group === `group-${n}`)) n++;
  return { ...d, objects: d.objects.map((o) => (ids.includes(o.id) ? { ...o, group: `group-${n}` } : o)) };
}

export function ungroup(d: Design, ids: string[]): Design {
  if (!d.objects.some((o) => ids.includes(o.id) && o.group)) return d;
  return { ...d, objects: d.objects.map((o) => (ids.includes(o.id) && o.group ? { ...o, group: null } : o)) };
}

/** Turn an object to `degrees` clockwise, kept within −180…180. Locked objects stay. */
export function rotate(d: Design, id: string, degrees: number): Design {
  const r = round(((((degrees + 180) % 360) + 360) % 360) - 180);
  return mapObject(d, id, (o) => (o.locked || (o.rotation ?? 0) === r ? o : { ...o, rotation: r }));
}

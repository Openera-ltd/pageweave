// Pending edits to existing page content (Edit mode). The UI records each user action as an entry;
// `normalize` turns a page's entries into the engine's request: the last wording per text run, moves
// added up, turns added up, the last replacement picture and size, and deletions winning. Pure (unit
// tested).

import type { PageEdit } from "./types";

export interface ObjEntry {
  page: number;
  edit: PageEdit;
}

export interface ObjHistory {
  items: ObjEntry[];
  redo: ObjEntry[];
}

export const EMPTY_HISTORY: ObjHistory = { items: [], redo: [] };

export function push(h: ObjHistory, e: ObjEntry): ObjHistory {
  return { items: [...h.items, e], redo: [] };
}

export function undoLast(h: ObjHistory): ObjHistory {
  const last = h.items[h.items.length - 1];
  return last ? { items: h.items.slice(0, -1), redo: [...h.redo, last] } : h;
}

export function redoLast(h: ObjHistory): ObjHistory {
  const last = h.redo[h.redo.length - 1];
  return last ? { items: [...h.items, last], redo: h.redo.slice(0, -1) } : h;
}

const key = (objects: number[]) => objects.join(",");

/** The edits for `page`, merged into what the engine should apply. */
export function normalize(items: ObjEntry[], page: number): PageEdit[] {
  const texts = new Map<string, Extract<PageEdit, { kind: "text" }>>();
  const moves = new Map<number, { dx: number; dy: number }>();
  const turns = new Map<number, number>();
  const replaced = new Map<number, string>();
  const sizes = new Map<number, Extract<PageEdit, { kind: "resize_image" }>["rect"]>();
  const deleted = new Set<number>();
  const added: PageEdit[] = [];
  for (const { page: p, edit: e } of items) {
    if (p !== page) continue;
    switch (e.kind) {
      case "text": {
        const prev = texts.get(key(e.objects));
        // Moves of a run accumulate; wording, size and colour take the latest values.
        texts.set(key(e.objects), { ...e, dx: (prev?.dx ?? 0) + e.dx, dy: (prev?.dy ?? 0) + e.dy, size: e.size ?? prev?.size ?? null, color: e.color ?? prev?.color ?? null });
        break;
      }
      case "move_objects":
        for (const o of e.objects) {
          const m = moves.get(o) ?? { dx: 0, dy: 0 };
          moves.set(o, { dx: m.dx + e.dx, dy: m.dy + e.dy });
        }
        break;
      case "rotate_image":
        turns.set(e.object, ((turns.get(e.object) ?? 0) + e.degrees) % 360);
        break;
      case "replace_image":
        replaced.set(e.object, e.path);
        // A new picture starts upright: earlier turns applied to the old one.
        turns.delete(e.object);
        break;
      case "resize_image":
        sizes.set(e.object, e.rect);
        break;
      case "delete_objects":
        e.objects.forEach((o) => deleted.add(o));
        break;
      case "add_text":
        added.push(e);
        break;
    }
  }
  const out: PageEdit[] = [];
  for (const t of texts.values()) {
    if (t.objects.some((o) => deleted.has(o))) continue;
    out.push(t);
  }
  for (const [o, m] of moves) if (!deleted.has(o) && (m.dx || m.dy)) out.push({ kind: "move_objects", objects: [o], dx: m.dx, dy: m.dy });
  for (const [o, path] of replaced) if (!deleted.has(o)) out.push({ kind: "replace_image", object: o, path });
  for (const [o, deg] of turns) if (!deleted.has(o) && deg) out.push({ kind: "rotate_image", object: o, degrees: deg as 90 | 180 | 270 });
  for (const [o, rect] of sizes) if (!deleted.has(o)) out.push({ kind: "resize_image", object: o, rect });
  if (deleted.size) out.push({ kind: "delete_objects", objects: [...deleted].sort((a, b) => a - b) });
  out.push(...added);
  return out;
}

/** Pages that have pending edits. */
export function pagesWithEdits(items: ObjEntry[]): number[] {
  return [...new Set(items.map((e) => e.page))].sort((a, b) => a - b);
}

/** Every page's merged edits, for saving. Pages whose edits cancel out are left out. */
export function toPageEdits(items: ObjEntry[]): Array<{ page: number; edits: PageEdit[] }> {
  return pagesWithEdits(items)
    .map((page) => ({ page, edits: normalize(items, page) }))
    .filter((p) => p.edits.length > 0);
}

/** Net move of an object so far (as displayed), for drawing its box where it now is. */
export function netMove(items: ObjEntry[], page: number, object: number): { dx: number; dy: number } {
  let dx = 0;
  let dy = 0;
  for (const { page: p, edit: e } of items) {
    if (p !== page) continue;
    if (e.kind === "move_objects" && e.objects.includes(object)) {
      dx += e.dx;
      dy += e.dy;
    } else if (e.kind === "text" && e.objects[0] === object) {
      dx += e.dx;
      dy += e.dy;
    }
  }
  return { dx, dy };
}

/** The latest pending box of a picture (before moves), or null when it keeps its size. */
export function netSize(items: ObjEntry[], page: number, object: number): Extract<PageEdit, { kind: "resize_image" }>["rect"] | null {
  let r = null;
  for (const { page: p, edit: e } of items) if (p === page && e.kind === "resize_image" && e.object === object) r = e.rect;
  return r;
}

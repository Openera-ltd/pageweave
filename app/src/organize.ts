// Page organizer model: an ordered list of tiles, each a page of the document or of an inserted
// file with an extra clockwise turn. Every operation returns a new list (history is a list of lists).

import type { PageInfo } from "./types";

export interface OrganizeSource {
  /** Host document id (for thumbnails). */
  docId: number;
  path: string;
  name: string;
  pages: PageInfo[];
}

export interface Tile {
  /** Stable identity for React keys and focus (duplicates get new ids). */
  id: number;
  /** 0 = the document being organized, 1.. = inserted files (operation `extra_inputs` order). */
  source: number;
  page: number;
  /** Extra clockwise turn: 0, 90, 180 or 270. */
  rotate: number;
}

export function initialTiles(pageCount: number): Tile[] {
  return Array.from({ length: pageCount }, (_, i) => ({ id: i + 1, source: 0, page: i, rotate: 0 }));
}

export function nextId(tiles: Tile[]): number {
  return tiles.reduce((m, t) => Math.max(m, t.id), 0) + 1;
}

/** Move tile `from` to position `to` (positions in the resulting list). */
export function moveTo(tiles: Tile[], from: number, to: number): Tile[] {
  if (from === to || from < 0 || from >= tiles.length) return tiles;
  const out = tiles.slice();
  const [t] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(out.length, to)), 0, t!);
  return out;
}

export function duplicate(tiles: Tile[], i: number): Tile[] {
  const t = tiles[i];
  if (!t) return tiles;
  const out = tiles.slice();
  out.splice(i + 1, 0, { ...t, id: nextId(tiles) });
  return out;
}

/** Remove a tile; the last remaining page cannot be removed (a PDF needs at least one page). */
export function remove(tiles: Tile[], i: number): Tile[] {
  if (tiles.length <= 1 || !tiles[i]) return tiles;
  return tiles.filter((_, j) => j !== i);
}

export function rotate(tiles: Tile[], i: number, degrees: 90 | -90): Tile[] {
  return tiles.map((t, j) => (j === i ? { ...t, rotate: (((t.rotate + degrees) % 360) + 360) % 360 } : t));
}

/** Insert every page of `source` after position `i` (or at the start when i < 0). */
export function insertPages(tiles: Tile[], i: number, source: number, pageCount: number): Tile[] {
  let id = nextId(tiles);
  const added = Array.from({ length: pageCount }, (_, p) => ({ id: id++, source, page: p, rotate: 0 }));
  const out = tiles.slice();
  out.splice(i + 1, 0, ...added);
  return out;
}

/** True when the tiles still describe the document unchanged. */
export function isUnchanged(tiles: Tile[], pageCount: number): boolean {
  return tiles.length === pageCount && tiles.every((t, i) => t.source === 0 && t.page === i && t.rotate === 0);
}

export function toPageRefs(tiles: Tile[]): Array<{ source: number; page: number; rotate: number }> {
  return tiles.map(({ source, page, rotate }) => ({ source, page, rotate }));
}

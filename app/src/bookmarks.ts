// Outline editing on a flat list in reading order (depth 0 = top level; a bookmark is a child of the
// closest earlier one with a smaller depth). Every operation keeps the list valid: indenting,
// outdenting and moving act on a bookmark together with its children.

export interface EditableBookmark {
  /** Stable key for React lists. */
  key: number;
  title: string;
  /** 0-based page, or null when the original bookmark pointed nowhere. */
  page: number | null;
  depth: number;
}

/** Mirrors pw-core limits. */
export const MAX_BOOKMARK_TITLE = 500;
export const MAX_BOOKMARK_DEPTH = 32;

/** Index one past the last descendant of item i. */
export function subtreeEnd(list: EditableBookmark[], i: number): number {
  let j = i + 1;
  while (j < list.length && list[j]!.depth > list[i]!.depth) j++;
  return j;
}

export function canIndent(list: EditableBookmark[], i: number): boolean {
  return i > 0 && list[i]!.depth <= list[i - 1]!.depth && list[i]!.depth < MAX_BOOKMARK_DEPTH;
}

export function indent(list: EditableBookmark[], i: number): EditableBookmark[] {
  if (!canIndent(list, i)) return list;
  const end = subtreeEnd(list, i);
  return list.map((b, k) => (k >= i && k < end ? { ...b, depth: b.depth + 1 } : b));
}

export function canOutdent(list: EditableBookmark[], i: number): boolean {
  return list[i]!.depth > 0;
}

export function outdent(list: EditableBookmark[], i: number): EditableBookmark[] {
  if (!canOutdent(list, i)) return list;
  const end = subtreeEnd(list, i);
  return list.map((b, k) => (k >= i && k < end ? { ...b, depth: b.depth - 1 } : b));
}

/** Start of the previous sibling's block, or -1. */
function previousSibling(list: EditableBookmark[], i: number): number {
  const d = list[i]!.depth;
  for (let k = i - 1; k >= 0; k--) {
    if (list[k]!.depth === d) return k;
    if (list[k]!.depth < d) return -1;
  }
  return -1;
}

export function canMoveUp(list: EditableBookmark[], i: number): boolean {
  return previousSibling(list, i) >= 0;
}

export function moveUp(list: EditableBookmark[], i: number): EditableBookmark[] {
  const p = previousSibling(list, i);
  if (p < 0) return list;
  const end = subtreeEnd(list, i);
  return [...list.slice(0, p), ...list.slice(i, end), ...list.slice(p, i), ...list.slice(end)];
}

export function canMoveDown(list: EditableBookmark[], i: number): boolean {
  const end = subtreeEnd(list, i);
  return end < list.length && list[end]!.depth === list[i]!.depth;
}

export function moveDown(list: EditableBookmark[], i: number): EditableBookmark[] {
  if (!canMoveDown(list, i)) return list;
  const end = subtreeEnd(list, i);
  return moveUp(list, end);
}

/** Remove one bookmark; its children move up one level and stay. */
export function remove(list: EditableBookmark[], i: number): EditableBookmark[] {
  const end = subtreeEnd(list, i);
  return [...list.slice(0, i), ...list.slice(i + 1, end).map((b) => ({ ...b, depth: b.depth - 1 })), ...list.slice(end)];
}

/** Insert a new top-level-or-sibling bookmark after item i's block (or at the end). */
export function insertAfter(list: EditableBookmark[], i: number | null, item: EditableBookmark): EditableBookmark[] {
  if (i === null || list.length === 0) return [...list, { ...item, depth: 0 }];
  const end = subtreeEnd(list, i);
  return [...list.slice(0, end), { ...item, depth: list[i]!.depth }, ...list.slice(end)];
}

/** Why the outline cannot be saved, or null. Mirrors the engine's checks. */
export function bookmarksBlocker(list: EditableBookmark[], pageCount: number): string | null {
  let prev = -1;
  for (const [n, b] of list.entries()) {
    const label = `Bookmark ${n + 1}`;
    if (!b.title.trim()) return `${label} needs a title.`;
    // eslint-disable-next-line no-control-regex
    if ([...b.title].length > MAX_BOOKMARK_TITLE || /[\u0000-\u001f\u007f-\u009f]/.test(b.title)) return `${label}: titles are at most ${MAX_BOOKMARK_TITLE} characters, without line breaks.`;
    if (b.page === null || !Number.isInteger(b.page) || b.page < 0 || b.page >= pageCount) return `${label}: choose a page from 1 to ${pageCount}.`;
    if (b.depth > prev + 1) return `${label} is indented more than one level below the one before it.`;
    prev = b.depth;
  }
  return null;
}

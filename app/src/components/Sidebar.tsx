import { useEffect, useRef } from "react";
import { Bookmark, LayoutGrid } from "lucide-react";
import type { OpenedDoc } from "../types";
import { Thumbnails } from "./Thumbnails";

export type SidebarView = "pages" | "bookmarks";

interface Props {
  doc: OpenedDoc;
  view: SidebarView;
  onView: (v: SidebarView) => void;
  currentPage: number;
  onSelectPage: (page: number) => void;
  onPageContextMenu: (page: number, x: number, y: number) => void;
}

/** Left panel: page thumbnails or the document outline (bookmarks). */
export function Sidebar({ doc, view, onView, currentPage, onSelectPage, onPageContextMenu }: Props) {
  const hasBookmarks = doc.info.bookmarks.length > 0;
  const shown: SidebarView = view === "bookmarks" && hasBookmarks ? "bookmarks" : "pages";
  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-tabs" role="tablist" aria-label="Navigation views">
        <button type="button" role="tab" id="sb-tab-pages" aria-controls="sb-panel" aria-selected={shown === "pages"} title="Pages" onClick={() => onView("pages")}>
          <LayoutGrid aria-hidden="true" /> Pages
        </button>
        <button
          type="button"
          role="tab"
          id="sb-tab-bookmarks"
          aria-controls="sb-panel"
          aria-selected={shown === "bookmarks"}
          aria-disabled={hasBookmarks ? undefined : true}
          title={hasBookmarks ? "Bookmarks" : "This document has no bookmarks"}
          onClick={() => hasBookmarks && onView("bookmarks")}
        >
          <Bookmark aria-hidden="true" /> Bookmarks
        </button>
      </div>
      <div id="sb-panel" role="tabpanel" aria-labelledby={shown === "pages" ? "sb-tab-pages" : "sb-tab-bookmarks"} className="sidebar-panel">
        {shown === "pages" ? (
          <Thumbnails doc={doc} currentPage={currentPage} onSelect={onSelectPage} onContextMenu={onPageContextMenu} />
        ) : (
          <Bookmarks doc={doc} currentPage={currentPage} onSelect={onSelectPage} />
        )}
      </div>
    </aside>
  );
}

function Bookmarks({ doc, currentPage, onSelect }: { doc: OpenedDoc; currentPage: number; onSelect: (p: number) => void }) {
  const ref = useRef<HTMLUListElement>(null);
  const items = doc.info.bookmarks;
  // The bookmark for the current page: last one starting at or before it.
  let active = -1;
  items.forEach((b, i) => {
    if (b.page !== null && b.page <= currentPage) active = i;
  });
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('[aria-current="location"]')?.scrollIntoView?.({ block: "nearest" });
  }, [active]);
  const onKeyDown = (e: React.KeyboardEvent) => {
    const all = Array.from(ref.current?.querySelectorAll<HTMLElement>("button") ?? []);
    const i = all.indexOf(document.activeElement as HTMLElement);
    const go = (n: number) => all[Math.max(0, Math.min(all.length - 1, n))]?.focus();
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else return;
    e.preventDefault();
  };
  return (
    <ul ref={ref} className="panel-scroll bookmarks" aria-label="Bookmarks" onKeyDown={onKeyDown}>
      {items.map((b, i) => (
        <li key={i}>
          <button
            type="button"
            className="bookmark"
            data-bookmark={i}
            style={{ paddingLeft: 8 + Math.min(b.depth, 8) * 14 }}
            aria-current={i === active ? "location" : undefined}
            aria-disabled={b.page === null ? true : undefined}
            title={b.page === null ? "This bookmark has no destination in the document" : `Go to page ${b.page + 1}`}
            tabIndex={i === Math.max(0, active) ? 0 : -1}
            onClick={() => b.page !== null && onSelect(b.page)}
          >
            <span className="title">{b.title || "Untitled"}</span>
            {b.page !== null && <span className="num">{b.page + 1}</span>}
          </button>
        </li>
      ))}
    </ul>
  );
}

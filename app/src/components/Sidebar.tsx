import { useEffect, useRef, useState } from "react";
import { Bookmark, Download, LayoutGrid, Paperclip, Plus } from "lucide-react";
import { useApi } from "../api";
import { toPwError, type AttachmentInfo, type OpenedDoc, type PwError } from "../types";
import { formatBytes } from "./Panels";
import { Thumbnails } from "./Thumbnails";

export type SidebarView = "pages" | "bookmarks" | "attachments";

interface Props {
  doc: OpenedDoc;
  view: SidebarView;
  onView: (v: SidebarView) => void;
  currentPage: number;
  onSelectPage: (page: number) => void;
  onPageContextMenu: (page: number, x: number, y: number) => void;
  onSaveAttachment: (a: AttachmentInfo) => void;
  onAttachFiles: () => void;
}

/** Left panel: page thumbnails or the document outline (bookmarks). */
export function Sidebar({ doc, view, onView, currentPage, onSelectPage, onPageContextMenu, onSaveAttachment, onAttachFiles }: Props) {
  const hasBookmarks = doc.info.bookmarks.length > 0;
  const shown: SidebarView = view === "bookmarks" ? (hasBookmarks ? "bookmarks" : "pages") : view;
  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-tabs" role="tablist" aria-label="Navigation views">
        <button type="button" role="tab" id="sb-tab-pages" aria-controls="sb-panel" aria-selected={shown === "pages"} aria-label="Pages" title="Pages" onClick={() => onView("pages")}>
          <LayoutGrid aria-hidden="true" />
        </button>
        <button
          type="button"
          role="tab"
          id="sb-tab-bookmarks"
          aria-controls="sb-panel"
          aria-selected={shown === "bookmarks"}
          aria-disabled={hasBookmarks ? undefined : true}
          aria-label="Bookmarks"
          title={hasBookmarks ? "Bookmarks" : "This document has no bookmarks"}
          onClick={() => hasBookmarks && onView("bookmarks")}
        >
          <Bookmark aria-hidden="true" />
        </button>
        <button type="button" role="tab" id="sb-tab-attachments" aria-controls="sb-panel" aria-selected={shown === "attachments"} title="Attachments" aria-label="Attachments" onClick={() => onView("attachments")}>
          <Paperclip aria-hidden="true" />
        </button>
      </div>
      <div id="sb-panel" role="tabpanel" aria-labelledby={`sb-tab-${shown}`} className="sidebar-panel">
        {shown === "pages" ? (
          <Thumbnails doc={doc} currentPage={currentPage} onSelect={onSelectPage} onContextMenu={onPageContextMenu} />
        ) : shown === "bookmarks" ? (
          <Bookmarks doc={doc} currentPage={currentPage} onSelect={onSelectPage} />
        ) : (
          <Attachments doc={doc} onSave={onSaveAttachment} onAttach={onAttachFiles} />
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

/** Files embedded in the document: save one out, or attach files into a new copy. */
function Attachments({ doc, onSave, onAttach }: { doc: OpenedDoc; onSave: (a: AttachmentInfo) => void; onAttach: () => void }) {
  const api = useApi();
  const [items, setItems] = useState<AttachmentInfo[] | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  useEffect(() => {
    let live = true;
    setItems(null);
    setError(null);
    api
      .listAttachments(doc.doc_id)
      .then((l) => live && setItems(l))
      .catch((e) => live && setError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.doc_id]);
  return (
    <div className="panel-scroll attachments" aria-busy={items === null && !error}>
      {error ? (
        <p className="error" role="alert">
          {error.message}
        </p>
      ) : items === null ? (
        <p className="hint">Loading…</p>
      ) : items.length === 0 ? (
        <p className="hint">No attached files.</p>
      ) : (
        <ul aria-label="Attached files">
          {items.map((a) => (
            <li key={a.index} className="attachment" data-attachment={a.index}>
              <span className="name" title={a.name}>
                {a.name}
              </span>
              <span className="hint">{formatBytes(a.bytes)}</span>
              <button type="button" className="btn icon" aria-label={`Save ${a.name}`} title="Save a copy of this file…" onClick={() => onSave(a)}>
                <Download />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="attach-actions">
        <button type="button" className="btn outline" data-command="file.attachFiles" onClick={onAttach} title="Attach files to a new copy of this document">
          <Plus /> Attach files…
        </button>
      </div>
    </div>
  );
}

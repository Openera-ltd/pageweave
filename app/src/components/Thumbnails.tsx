import { memo, useEffect, useRef, useState } from "react";
import { useApi } from "../api";
import { displaySizePt, renderScalePermille } from "../geometry";
import type { OpenedDoc, PageInfo } from "../types";

interface Props {
  doc: OpenedDoc;
  currentPage: number;
  onSelect: (page: number) => void;
  onContextMenu: (page: number, x: number, y: number) => void;
}

const THUMB_WIDTH = 120;

/** Page thumbnails. Images load lazily as they scroll into view; arrow keys move between pages. */
export function Thumbnails({ doc, currentPage, onSelect, onContextMenu }: Props) {
  const ref = useRef<HTMLElement>(null);

  useEffect(() => {
    const el = ref.current?.querySelector<HTMLElement>(`[data-thumb="${currentPage}"]`);
    if (el && "scrollIntoView" in el) el.scrollIntoView({ block: "nearest" });
  }, [currentPage]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const idx = Number((document.activeElement as HTMLElement | null)?.dataset.thumb ?? currentPage);
    let next: number | null = null;
    if (e.key === "ArrowDown") next = Math.min(doc.info.page_count - 1, idx + 1);
    else if (e.key === "ArrowUp") next = Math.max(0, idx - 1);
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = doc.info.page_count - 1;
    else if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
      e.preventDefault();
      const r = (document.activeElement as HTMLElement).getBoundingClientRect();
      onContextMenu(idx, r.right - 8, r.top + 8);
      return;
    }
    if (next === null) return;
    e.preventDefault();
    ref.current?.querySelector<HTMLElement>(`[data-thumb="${next}"]`)?.focus();
    onSelect(next);
  };

  return (
    <nav ref={ref} className="panel-scroll" aria-label="Page thumbnails" onKeyDown={onKeyDown}>
      {doc.info.pages.map((p) => (
        <Thumb
          key={p.index}
          docId={doc.doc_id}
          page={p}
          current={p.index === currentPage}
          root={ref}
          onSelect={onSelect}
          onContextMenu={onContextMenu}
        />
      ))}
    </nav>
  );
}

interface ThumbProps {
  docId: number;
  page: PageInfo;
  current: boolean;
  root: React.RefObject<HTMLElement | null>;
  onSelect: (page: number) => void;
  onContextMenu: (page: number, x: number, y: number) => void;
}

const Thumb = memo(function Thumb({ docId, page, current, root, onSelect, onContextMenu }: ThumbProps) {
  const api = useApi();
  const ref = useRef<HTMLButtonElement>(null);
  const [near, setNear] = useState(false);
  const [failed, setFailed] = useState(false);
  const d = displaySizePt(page);
  const w = THUMB_WIDTH;
  const h = Math.round((d.h / d.w) * w);
  const zoom = w / (d.w * (96 / 72));
  const scale = renderScalePermille(page, zoom, window.devicePixelRatio || 1);

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const io = new IntersectionObserver((en) => en.some((x) => x.isIntersecting) && setNear(true), {
      root: root.current,
      rootMargin: "200px 0px",
    });
    io.observe(el);
    return () => io.disconnect();
  }, [root]);

  return (
    <button
      ref={ref}
      type="button"
      className="thumb"
      data-thumb={page.index}
      aria-current={current ? "page" : undefined}
      aria-label={`Page ${page.index + 1}${page.label ? ` (${page.label})` : ""}`}
      tabIndex={current ? 0 : -1}
      onClick={() => onSelect(page.index)}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(page.index, e.clientX, e.clientY);
      }}
    >
      <span className="frame" style={{ width: w, height: h }}>
        {near && !failed && <img src={api.pageUrl(docId, page.index, scale)} alt="" draggable={false} onError={() => setFailed(true)} />}
      </span>
      <span className="num">{page.label ?? page.index + 1}</span>
    </button>
  );
});

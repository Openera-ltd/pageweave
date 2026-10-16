import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useApi } from "../api";
import { displaySizePx, mostVisiblePage, pdfRectToScreen, renderScalePermille } from "../geometry";
import type { OpenedDoc, PageInfo, SearchHit } from "../types";

export interface ScrollRequest {
  page: number;
  /** Changes on every request so repeated jumps to the same page still scroll. */
  nonce: number;
  /** Optional hit to centre in view. */
  hit?: SearchHit;
}

interface ViewerProps {
  doc: OpenedDoc;
  zoom: number;
  onZoomStep: (direction: 1 | -1, anchor?: { x: number; y: number }) => void;
  onCurrentPage: (page: number) => void;
  scrollRequest: ScrollRequest | null;
  hits: SearchHit[];
  currentHit: number;
  onPageContextMenu: (page: number, x: number, y: number) => void;
  /** Called once after mount with the real viewport width, so the first zoom can fit the page. */
  onMeasured?: (width: number) => void;
}

const PAGE_GAP = 16;

export function Viewer({ doc, zoom, onZoomStep, onCurrentPage, scrollRequest, hits, currentHit, onPageContextMenu, onMeasured }: ViewerProps) {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (ref.current && onMeasured) onMeasured(ref.current.clientWidth);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const lastReported = useRef(-1);
  const zoomRef = useRef(zoom);
  const pages = doc.info.pages;

  // Keep the reading position stable across zoom changes (proportional scroll).
  useLayoutEffect(() => {
    const el = ref.current;
    const prev = zoomRef.current;
    zoomRef.current = zoom;
    if (!el || prev === zoom) return;
    const ratio = zoom / prev;
    el.scrollTop = el.scrollTop * ratio;
    el.scrollLeft = el.scrollLeft * ratio;
  }, [zoom]);

  const reportVisible = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const nodes = Array.from(el.querySelectorAll<HTMLElement>("[data-page]"));
    const tops = nodes.map((n) => n.offsetTop);
    const heights = nodes.map((n) => n.offsetHeight);
    const p = mostVisiblePage(tops, heights, el.scrollTop, el.clientHeight);
    if (p !== lastReported.current) {
      lastReported.current = p;
      onCurrentPage(p);
    }
  }, [onCurrentPage]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(reportVisible);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    reportVisible();
    return () => {
      el.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(frame);
    };
  }, [reportVisible, doc.doc_id]);

  // Ctrl + wheel zooms; plain wheel scrolls natively.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      onZoomStep(e.deltaY < 0 ? 1 : -1, { x: e.clientX, y: e.clientY });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [onZoomStep]);

  useEffect(() => {
    const el = ref.current;
    if (!el || !scrollRequest) return;
    const node = el.querySelector<HTMLElement>(`[data-page="${scrollRequest.page}"]`);
    if (!node) return;
    let top = node.offsetTop - PAGE_GAP;
    const hit = scrollRequest.hit;
    const page = pages[scrollRequest.page];
    if (hit && page && hit.rects[0]) {
      const r = pdfRectToScreen(hit.rects[0], page, zoom);
      top = node.offsetTop + r.top - el.clientHeight / 2;
      const left = node.offsetLeft + r.left - el.clientWidth / 2;
      if (el.scrollWidth > el.clientWidth) el.scrollLeft = Math.max(0, left);
    }
    el.scrollTop = Math.max(0, top);
    reportVisible();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollRequest]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
      e.preventDefault();
      const el = ref.current;
      const node = el?.querySelector<HTMLElement>(`[data-page="${Math.max(0, lastReported.current)}"]`);
      const r = node?.getBoundingClientRect();
      onPageContextMenu(Math.max(0, lastReported.current), r ? r.left + 24 : 100, r ? Math.max(r.top, 60) + 24 : 100);
    }
  };

  return (
    <div
      ref={ref}
      className="viewer"
      tabIndex={0}
      role="document"
      aria-label={`${doc.name}, ${pages.length} pages`}
      onKeyDown={onKeyDown}
      data-testid="viewer"
    >
      <div className="pages">
        {pages.map((p) => (
          <Page
            key={p.index}
            docId={doc.doc_id}
            page={p}
            zoom={zoom}
            root={ref}
            hits={hits}
            currentHit={currentHit}
            onContextMenu={onPageContextMenu}
          />
        ))}
      </div>
    </div>
  );
}

interface PageProps {
  docId: number;
  page: PageInfo;
  zoom: number;
  root: React.RefObject<HTMLDivElement | null>;
  hits: SearchHit[];
  currentHit: number;
  onContextMenu: (page: number, x: number, y: number) => void;
}

const Page = memo(function Page({ docId, page, zoom, root, hits, currentHit, onContextMenu }: PageProps) {
  const api = useApi();
  const ref = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  const [status, setStatus] = useState<"loading" | "loaded" | "error">("loading");
  const [attempt, setAttempt] = useState(0);
  const size = displaySizePx(page, zoom);
  const scale = renderScalePermille(page, zoom, window.devicePixelRatio || 1);
  const src = `${api.pageUrl(docId, page.index, scale)}${attempt ? `?retry=${attempt}` : ""}`;

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    // Render pages within ~1.5 screens; drop images far away so memory stays bounded.
    const io = new IntersectionObserver((entries) => setNear(entries.some((en) => en.isIntersecting)), {
      root: root.current,
      rootMargin: "150% 0px",
    });
    io.observe(el);
    return () => io.disconnect();
  }, [root]);

  useEffect(() => setStatus("loading"), [src]);

  const pageHits = hits.map((h, i) => ({ h, i })).filter(({ h }) => h.page === page.index);

  return (
    <div
      ref={ref}
      className="page"
      data-page={page.index}
      style={{ width: size.w, height: size.h }}
      aria-label={`Page ${page.index + 1}`}
      role="img"
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(page.index, e.clientX, e.clientY);
      }}
    >
      {near && status !== "error" && (
        <img src={src} alt="" draggable={false} onLoad={() => setStatus("loaded")} onError={() => setStatus("error")} />
      )}
      {near && status === "loading" && <div className="loading" aria-hidden="true">Rendering page {page.index + 1}…</div>}
      {status === "error" && (
        <div className="page-error" role="alert">
          <div>
            Page {page.index + 1} could not be displayed.{" "}
            <button type="button" className="btn outline" onClick={() => setAttempt((a) => a + 1)}>
              Retry
            </button>
          </div>
        </div>
      )}
      {pageHits.flatMap(({ h, i }) =>
        h.rects.map((r, j) => {
          const s = pdfRectToScreen(r, page, zoom);
          return (
            <div
              key={`${i}-${j}`}
              className={`hit ${i === currentHit ? "current" : ""}`}
              data-hit={i}
              style={{ left: s.left, top: s.top, width: Math.max(2, s.width), height: Math.max(2, s.height) }}
            />
          );
        }),
      )}
    </div>
  );
});

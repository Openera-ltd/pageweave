import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { StickyNote } from "lucide-react";
import { useApi } from "../api";
import type { ObjEntry } from "../objectEdits";
import { EditLayer } from "./EditLayer";
import { KIND_LABEL, type PendingAnnotation } from "../annotations";
import { currentValue, fieldLabel, isFillable, type FormEdits } from "../forms";
import { CSS_PX_PER_PT, displaySizePt, displaySizePx, mostVisiblePage, pageOrigin, pdfRectToScreen, renderScalePermille, screenRectToPdf, unrotatedLayer } from "../geometry";
import type { FormField, FormInfo, FormValue, FormWidget, OpenedDoc, PageInfo, PageText, SearchHit, SearchRect } from "../types";

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
  /** Unsaved annotations to draw over the pages. */
  annotations: PendingAnnotation[];
  /** The document's form fields (null while loading or when it has none) and the user's edits. */
  form?: FormInfo | null;
  formEdits?: FormEdits;
  onFormEdit?: (field: FormField, value: FormValue) => void;
  /** Called once after mount with the real viewport width, so the first zoom can fit the page. */
  onMeasured?: (width: number) => void;
  /** Area-marking mode: dragging on a page marks a rectangle for redaction instead of selecting text. */
  markMode?: boolean;
  onMarkArea?: (page: number, rect: SearchRect) => void;
  /** Edit mode: boxes over editable text and pictures, with the pending edits previewed. */
  edit?: EditProps;
}

export interface EditProps {
  items: ObjEntry[];
  onEdit: (e: ObjEntry) => void;
  addText: boolean;
  onAddTextDone: () => void;
}

const PAGE_GAP = 16;
const NO_ANNOTATIONS: PendingAnnotation[] = [];
const NO_WIDGETS: PageWidget[] = [];
const NO_EDITS: FormEdits = {};

interface PageWidget {
  field: FormField;
  widget: FormWidget;
  key: string;
}
let firstPageReported = false;

export function Viewer({ doc, zoom, onZoomStep, onCurrentPage, scrollRequest, hits, currentHit, onPageContextMenu, annotations, form, formEdits, onFormEdit, onMeasured, markMode, onMarkArea, edit }: ViewerProps) {
  const ref = useRef<HTMLDivElement>(null);
  // Per-page lists with stable identity, so adding an annotation re-renders only its page.
  const byPage = useMemo(() => {
    const m = new Map<number, PendingAnnotation[]>();
    for (const a of annotations) m.set(a.page, [...(m.get(a.page) ?? []), a]);
    return m;
  }, [annotations]);
  const widgetsByPage = useMemo(() => {
    const m = new Map<number, PageWidget[]>();
    for (const field of form?.fields ?? []) {
      if (!isFillable(field)) continue;
      field.widgets.forEach((widget, i) => m.set(widget.page, [...(m.get(widget.page) ?? []), { field, widget, key: `${field.name}#${i}` }]));
    }
    return m;
  }, [form]);

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
            annotations={byPage.get(p.index) ?? NO_ANNOTATIONS}
            widgets={widgetsByPage.get(p.index) ?? NO_WIDGETS}
            formEdits={formEdits ?? NO_EDITS}
            onFormEdit={onFormEdit}
            onMarkArea={markMode ? onMarkArea : undefined}
            edit={edit}
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
  annotations: PendingAnnotation[];
  widgets: PageWidget[];
  formEdits: FormEdits;
  onFormEdit?: (field: FormField, value: FormValue) => void;
  /** Set while marking areas for redaction. */
  onMarkArea?: (page: number, rect: SearchRect) => void;
  edit?: EditProps;
}

const Page = memo(function Page({ docId, page, zoom, root, hits, currentHit, onContextMenu, annotations, widgets, formEdits, onFormEdit, onMarkArea, edit }: PageProps) {
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
      role="group"
      aria-roledescription="page"
      onMouseDown={(e) => {
        // Right-clicking inside the current text selection must not clear it (INTERACTION_CONTRACT).
        if (e.button === 2 && selectionContains(e.clientX, e.clientY)) e.preventDefault();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(page.index, e.clientX, e.clientY);
      }}
    >
      {near && status !== "error" && (
        <img
          src={src}
          alt=""
          draggable={false}
          onLoad={() => {
            setStatus("loaded");
            if (!firstPageReported) {
              // Startup diagnostics: time to the first visible page, logged by the host once per run.
              firstPageReported = true;
              api.markTiming("first_page").catch(() => undefined);
            }
          }}
          onError={() => setStatus("error")}
        />
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
      {annotations.length > 0 && <AnnotationLayer page={page} zoom={zoom} annotations={annotations} />}
      {near && !edit && <TextLayer docId={docId} page={page} zoom={zoom} />}
      {near && edit && <EditLayer docId={docId} page={page} zoom={zoom} items={edit.items} onEdit={edit.onEdit} addText={edit.addText} onAddTextDone={edit.onAddTextDone} />}
      {near && widgets.length > 0 && onFormEdit && <FormLayer docId={docId} page={page} zoom={zoom} widgets={widgets} edits={formEdits} onEdit={onFormEdit} />}
      {onMarkArea && <MarkCapture page={page} onMark={onMarkArea} />}
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

/**
 * Pointer layer over a page in area-marking mode: drag to draw a rectangle, released as a mark in
 * PDF space. The zoom comes from the page element's measured width (like text selections), so UI
 * scaling cannot skew the area. Keyboard users mark pages or search matches instead.
 */
function MarkCapture({ page, onMark }: { page: PageInfo; onMark: (page: number, rect: SearchRect) => void }) {
  const [draft, setDraft] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const local = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: Math.min(Math.max(e.clientX - r.left, 0), r.width), y: Math.min(Math.max(e.clientY - r.top, 0), r.height) };
  };
  const box = draft && { left: Math.min(draft.x0, draft.x1), top: Math.min(draft.y0, draft.y1), width: Math.abs(draft.x1 - draft.x0), height: Math.abs(draft.y1 - draft.y0) };
  return (
    <div
      className="markcapture"
      data-markcapture={page.index}
      aria-hidden="true"
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture?.(e.pointerId);
        const p = local(e);
        setDraft({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
      }}
      onPointerMove={(e) => {
        if (!draft) return;
        const p = local(e);
        setDraft((d) => d && { ...d, x1: p.x, y1: p.y });
      }}
      onPointerUp={(e) => {
        if (!draft || !box) return;
        setDraft(null);
        if (box.width < 4 || box.height < 4) return;
        const zoom = e.currentTarget.getBoundingClientRect().width / (displaySizePt(page).w * CSS_PX_PER_PT);
        onMark(page.index, screenRectToPdf(box, page, zoom));
      }}
      onPointerCancel={() => setDraft(null)}
    >
      {box && <div className="markdraft" style={{ left: box.left, top: box.top, width: box.width, height: box.height }} />}
    </div>
  );
}

/**
 * Unsaved annotations drawn over the page image, under the text layer so selection keeps working.
 * Decorative for pointer input; the status bar announces the count and the saved file is the record.
 */
const AnnotationLayer = memo(function AnnotationLayer({ page, zoom, annotations }: { page: PageInfo; zoom: number; annotations: PendingAnnotation[] }) {
  // The text baseline side of a rectangle on screen: PDF "bottom" after the page's /Rotate.
  const edge = ({ 0: "bottom", 90: "left", 180: "top", 270: "right" } as const)[page.rotation];
  return (
    <div className="annotlayer" aria-hidden="true">
      {annotations.flatMap((a) =>
        a.rects.slice(0, a.kind === "note" ? 1 : undefined).map((r, j) => {
          const s = pdfRectToScreen(r, page, zoom);
          return (
            <div
              key={`${a.id}-${j}`}
              className={`annot ${a.kind}`}
              data-annot={a.kind}
              data-edge={edge}
              title={a.kind === "note" ? (a.contents ?? "") : KIND_LABEL[a.kind]}
              style={{ left: s.left, top: s.top, width: Math.max(1, s.width), height: Math.max(1, s.height), ["--annot" as string]: a.color }}
            >
              {a.kind === "note" && <StickyNote />}
            </div>
          );
        }),
      )}
    </div>
  );
});

/**
 * Native controls over the page's form widgets, laid out in unrotated page space and rotated with
 * the page (like the text layer). Edits are kept by the app until saved into a copy.
 */
function FormLayer({ docId, page, zoom, widgets, edits, onEdit }: { docId: number; page: PageInfo; zoom: number; widgets: PageWidget[]; edits: FormEdits; onEdit: (field: FormField, value: FormValue) => void }) {
  const k = zoom * CSS_PX_PER_PT;
  const o = pageOrigin(page);
  const layer = unrotatedLayer(page, zoom);
  return (
    <div className="formlayer" data-formlayer={page.index} style={{ width: layer.width, height: layer.height, transform: layer.transform }}>
      {widgets.map(({ field, widget, key }) => {
        const r = widget.rect;
        const box = { left: (r.left - o.x) * k, top: (o.y + page.height_pt - r.top) * k, width: Math.max(4, (r.right - r.left) * k), height: Math.max(4, (r.top - r.bottom) * k) };
        const fontSize = Math.max(8, Math.min(box.height * 0.62, 13 * k));
        const label = fieldLabel(field);
        const common = {
          "data-field": field.name,
          "aria-label": label,
          "aria-required": field.required || undefined,
          disabled: field.read_only,
          title: field.read_only ? `${label} — read-only field` : label,
        };
        const value = currentValue(edits, field);
        const style = { ...box, fontSize };
        switch (field.kind) {
          case "checkbox":
            return <input key={key} type="checkbox" className="pw-field check" style={box} {...common} checked={value === true} onChange={(e) => onEdit(field, e.target.checked)} />;
          case "radio":
            return (
              <input
                key={key}
                type="radio"
                className="pw-field check"
                style={box}
                {...common}
                aria-label={`${label}: ${widget.export_value ?? ""}`}
                name={`pwf-${docId}-${field.name}`}
                value={widget.export_value ?? ""}
                checked={value === widget.export_value}
                onChange={() => widget.export_value && onEdit(field, widget.export_value)}
              />
            );
          case "combo":
            if (field.editable) {
              const listId = `pwf-${docId}-${field.name}-options`;
              return (
                <span key={key}>
                  <input className="pw-field" style={style} {...common} list={listId} value={String(value)} onChange={(e) => onEdit(field, e.target.value)} />
                  <datalist id={listId}>{field.options.map((o) => <option key={o} value={o} />)}</datalist>
                </span>
              );
            }
            return (
              <select key={key} className="pw-field" style={style} {...common} value={String(value)} onChange={(e) => onEdit(field, e.target.value)}>
                {!field.options.includes(String(value)) && <option value={String(value)}>{String(value)}</option>}
                {field.options.map((o) => <option key={o} value={o}>{o}</option>)}
              </select>
            );
          case "list":
            return (
              <select key={key} className="pw-field" style={style} {...common} size={Math.max(2, Math.min(field.options.length, Math.floor(box.height / (fontSize * 1.3)) || 2))} value={String(value)} onChange={(e) => onEdit(field, e.target.value)}>
                {field.options.map((o) => <option key={o} value={o}>{o}</option>)}
              </select>
            );
          default:
            return field.multiline ? (
              <textarea key={key} className="pw-field" style={style} {...common} value={String(value)} onChange={(e) => onEdit(field, e.target.value)} />
            ) : (
              <input key={key} className="pw-field" style={style} {...common} value={String(value)} onChange={(e) => onEdit(field, e.target.value)} />
            );
        }
      })}
    </div>
  );
}

/**
 * Invisible, selectable text positioned over the rendered page so native selection, double-click
 * word selection and Ctrl+C work. Laid out in unrotated page space and rotated with the page.
 */
const TextLayer = memo(function TextLayer({ docId, page, zoom }: { docId: number; page: PageInfo; zoom: number }) {
  const api = useApi();
  const [text, setText] = useState<PageText | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let live = true;
    api
      .pageText(docId, page.index)
      .then((t) => live && setText(t))
      // A page without a text layer stays viewable; selection is simply unavailable on it.
      .catch(() => live && setText({ page: page.index, segments: [], truncated: false }));
    return () => {
      live = false;
    };
  }, [api, docId, page.index]);

  // Stretch each run horizontally so the selectable text matches the rendered glyph extent.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    for (const span of Array.from(el.children) as HTMLElement[]) {
      span.style.transform = "";
      const target = Number(span.dataset.w);
      // offsetWidth is the untransformed layout width; getBoundingClientRect() would report the
      // rotated box on 90°/270° pages and over-stretch the run.
      const natural = span.offsetWidth;
      if (natural > 0 && target > 0) span.style.transform = `scaleX(${target / natural})`;
    }
  }, [text, zoom]);

  if (!text || text.segments.length === 0) return null;
  const k = zoom * CSS_PX_PER_PT;
  const o = pageOrigin(page);
  const layer = unrotatedLayer(page, zoom);
  return (
    <div ref={ref} className="textlayer" data-textlayer={page.index} style={{ width: layer.width, height: layer.height, transform: layer.transform }}>
      {text.segments.map((s, i) => {
        const h = Math.max(1, (s.top - s.bottom) * k);
        return (
          <span
            key={i}
            data-w={(s.right - s.left) * k}
            style={{ left: (s.left - o.x) * k, top: (o.y + page.height_pt - s.top) * k, fontSize: h, height: h }}
          >
            {s.text}
            {"\n"}
          </span>
        );
      })}
    </div>
  );
});

/** Whether a viewport point lies inside the current (non-empty) selection's rectangles. */
function selectionContains(x: number, y: number): boolean {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return false;
  for (let i = 0; i < sel.rangeCount; i++) {
    for (const r of Array.from(sel.getRangeAt(i).getClientRects())) {
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return true;
    }
  }
  return false;
}

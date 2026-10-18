import { useEffect, useRef, useState } from "react";
import { ImageUp, RotateCw, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { CSS_PX_PER_PT, pdfRectToScreen, renderScalePermille, screenRectToPdf } from "../geometry";
import { netMove, normalize, type ObjEntry } from "../objectEdits";
import { toPwError, type EditableObjects, type PageEdit, type PageInfo } from "../types";

interface Props {
  docId: number;
  page: PageInfo;
  zoom: number;
  /** All pending object edits of the document. */
  items: ObjEntry[];
  onEdit: (e: ObjEntry) => void;
  /** Set while "Add text" waits for a click on a page. */
  addText: boolean;
  onAddTextDone: () => void;
}

/** Pointer movement that turns a press into a drag (CSS px). */
const DRAG_PX = 4;

type Target = { kind: "text"; index: number } | { kind: "image"; index: number };

/** Edit mode for one page: boxes over editable text runs and pictures, and an exact preview. */
export function EditLayer({ docId, page, zoom, items, onEdit, addText, onAddTextDone }: Props) {
  const api = useApi();
  const [listing, setListing] = useState<EditableObjects | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ index: number; value: string } | null>(null);
  const [adding, setAdding] = useState<{ left: number; top: number; value: string } | null>(null);
  const [selected, setSelected] = useState<Target | null>(null);
  const [drag, setDrag] = useState<{ target: Target; x0: number; y0: number; dx: number; dy: number; moved: boolean } | null>(null);
  const k = zoom * CSS_PX_PER_PT;
  /** Set when a press ended a drag, so the click that follows does not also open the box. */
  const dragged = useRef(false);

  useEffect(() => {
    let live = true;
    api
      .pageObjects(docId, page.index)
      .then((l) => live && setListing(l))
      .catch((e) => live && setListError(toPwError(e).message));
    return () => {
      live = false;
    };
  }, [api, docId, page.index]);

  // Exact preview: the engine applies this page's pending edits in memory and renders the page.
  const edits = normalize(items, page.index);
  const editsKey = JSON.stringify(edits);
  useEffect(() => {
    if (edits.length === 0) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    let live = true;
    const t = setTimeout(() => {
      api
        .previewPageEdits(docId, page.index, edits, renderScalePermille(page, zoom, window.devicePixelRatio || 1))
        .then((url) => {
          if (!live) return;
          setPreview(url);
          setPreviewError(null);
        })
        .catch((e) => live && setPreviewError(toPwError(e).message));
    }, 120);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, docId, page.index, editsKey, zoom]);

  const deleted = new Set(edits.flatMap((e) => (e.kind === "delete_objects" ? e.objects : [])));
  const currentText = (objects: number[]): string => {
    let t: string | null = null;
    for (const { page: p, edit: e } of items) if (p === page.index && e.kind === "text" && e.objects.join() === objects.join()) t = e.text;
    return t ?? listing?.texts.find((r) => r.objects.join() === objects.join())?.text ?? "";
  };

  const record = (edit: PageEdit) => onEdit({ page: page.index, edit });
  const moveTarget = (t: Target, dxPt: number, dyPt: number) => {
    if (!listing || (!dxPt && !dyPt)) return;
    if (t.kind === "image") {
      record({ kind: "move_objects", objects: [listing.images[t.index]!.object], dx: dxPt, dy: dyPt });
    } else {
      const run = listing.texts[t.index]!;
      record({ kind: "text", objects: run.objects, text: currentText(run.objects), size: null, color: null, dx: dxPt, dy: dyPt });
    }
  };
  const deleteTarget = (t: Target) => {
    if (!listing) return;
    const objects = t.kind === "image" ? [listing.images[t.index]!.object] : listing.texts[t.index]!.objects;
    record({ kind: "delete_objects", objects });
    setSelected(null);
  };

  const boxOf = (rect: { left: number; bottom: number; right: number; top: number }, object: number) => {
    const s = pdfRectToScreen(rect, page, zoom);
    const m = netMove(items, page.index, object);
    return { left: s.left + m.dx * k, top: s.top - m.dy * k, width: Math.max(6, s.width), height: Math.max(6, s.height) };
  };

  /** Mouse click, Enter or Space; the click that ends a drag is not an activation. */
  const activate = (run: () => void) => () => {
    if (dragged.current) {
      dragged.current = false;
      return;
    }
    run();
  };

  const pointerHandlers = (t: Target) => ({
    onPointerDown: (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      e.stopPropagation();
      e.currentTarget.setPointerCapture(e.pointerId);
      dragged.current = false;
      setDrag({ target: t, x0: e.clientX, y0: e.clientY, dx: 0, dy: 0, moved: false });
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (!drag) return;
      const dx = e.clientX - drag.x0;
      const dy = e.clientY - drag.y0;
      setDrag({ ...drag, dx, dy, moved: drag.moved || Math.hypot(dx, dy) > DRAG_PX });
    },
    onPointerUp: () => {
      if (!drag) return;
      const d = drag;
      setDrag(null);
      dragged.current = d.moved;
      if (d.moved) moveTarget(d.target, Math.round((d.dx / k) * 100) / 100, Math.round((-d.dy / k) * 100) / 100);
    },

    onPointerCancel: () => setDrag(null),
    onKeyDown: (e: React.KeyboardEvent) => {
      const step = e.shiftKey ? 10 : 1;
      const arrows: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
      if (arrows[e.key]) {
        e.preventDefault();
        moveTarget(t, ...arrows[e.key]!);
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        deleteTarget(t);
      }
    },
  });

  const dragOffset = (t: Target) => (drag && drag.moved && drag.target.kind === t.kind && drag.target.index === t.index ? { transform: `translate(${drag.dx}px, ${drag.dy}px)` } : undefined);

  const commitText = (index: number, value: string) => {
    setEditing(null);
    const run = listing?.texts[index];
    if (!run || value === currentText(run.objects)) return;
    record({ kind: "text", objects: run.objects, text: value, size: null, color: null, dx: 0, dy: 0 });
  };

  const commitAdd = (value: string, left: number, top: number) => {
    setAdding(null);
    onAddTextDone();
    if (!value.trim()) return;
    const size = 12;
    // The new text's baseline starts one font size below the click, as displayed.
    const r = screenRectToPdf({ left, top: top + size * k, width: 0, height: 0 }, page, zoom);
    record({ kind: "add_text", x: r.left, y: r.bottom, text: value, size, color: "#000000" });
  };

  return (
    <div className="edit-layer" data-testid="edit-layer" data-edit-page={page.index}>
      {preview && <img className="edit-preview" src={preview} alt="" draggable={false} data-testid="edit-preview" />}
      {previewError && (
        <div className="edit-error" role="alert">
          {previewError}
        </div>
      )}
      {listError && (
        <div className="edit-error" role="alert">
          {listError}
        </div>
      )}
      {addText && !adding && (
        <div
          className="edit-add-capture"
          data-testid="add-text-capture"
          onClick={(e) => {
            const b = e.currentTarget.getBoundingClientRect();
            setAdding({ left: e.clientX - b.left, top: e.clientY - b.top, value: "" });
          }}
        />
      )}
      {adding && (
        <input
          className="edit-input"
          data-testid="add-text-input"
          aria-label="New text"
          autoFocus
          style={{ left: adding.left, top: adding.top, fontSize: 12 * k, minWidth: 160 }}
          value={adding.value}
          onChange={(e) => setAdding({ ...adding, value: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitAdd(adding.value, adding.left, adding.top);
            if (e.key === "Escape") {
              setAdding(null);
              onAddTextDone();
            }
          }}
          onBlur={() => commitAdd(adding.value, adding.left, adding.top)}
        />
      )}
      {listing?.texts.map((run, i) => {
        if (run.objects.some((o) => deleted.has(o))) return null;
        const b = boxOf(run.rect, run.objects[0]!);
        const t: Target = { kind: "text", index: i };
        if (editing?.index === i) {
          return (
            <input
              key={`t${i}`}
              className="edit-input"
              data-testid="edit-text-input"
              aria-label="Edit text"
              autoFocus
              style={{ left: b.left, top: b.top, width: Math.max(b.width + 40, 120), height: b.height + 6, fontSize: run.size * k * 0.9 }}
              value={editing.value}
              onChange={(e) => setEditing({ index: i, value: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitText(i, editing.value);
                if (e.key === "Escape") setEditing(null);
              }}
              onBlur={() => commitText(i, editing.value)}
            />
          );
        }
        if (run.locked) {
          return <div key={`t${i}`} className="edit-box locked" style={b} title={run.locked} aria-label={`Text that cannot be edited: ${run.locked}`} role="note" />;
        }
        return (
          <button
            key={`t${i}`}
            type="button"
            className={`edit-box text${selected?.kind === "text" && selected.index === i ? " selected" : ""}`}
            data-edit-text={i}
            style={{ ...b, ...dragOffset(t) }}
            title={`${run.font}${run.embedded ? "" : " (not embedded)"}, ${Math.round(run.size * 10) / 10} pt`}
            aria-label={`Edit text: ${currentText(run.objects)}`}
            onFocus={() => setSelected(t)}
            onClick={activate(() => setEditing({ index: i, value: currentText(run.objects) }))}
            {...pointerHandlers(t)}
          />
        );
      })}
      {listing?.images.map((img, i) => {
        if (deleted.has(img.object)) return null;
        const b = boxOf(img.rect, img.object);
        const t: Target = { kind: "image", index: i };
        const isSel = selected?.kind === "image" && selected.index === i;
        return (
          <div key={`i${i}`}>
            <button
              type="button"
              className={`edit-box image${isSel ? " selected" : ""}`}
              data-edit-image={i}
              style={{ ...b, ...dragOffset(t) }}
              aria-label={`Picture ${img.width_px} × ${img.height_px} pixels`}
              aria-pressed={isSel}
              onFocus={() => setSelected(t)}
              onClick={activate(() => setSelected(t))}
              {...pointerHandlers(t)}
            />
            {isSel && (
              <div className="edit-toolbar" role="toolbar" aria-label="Picture" style={{ left: b.left, top: Math.max(0, b.top - 36) }}>
                <button type="button" className="btn icon" title="Turn 90° clockwise" aria-label="Turn picture 90° clockwise" onClick={() => record({ kind: "rotate_image", object: img.object, degrees: 90 })}>
                  <RotateCw />
                </button>
                <button
                  type="button"
                  className="btn icon"
                  title="Replace with another picture…"
                  aria-label="Replace picture"
                  onClick={async () => {
                    const files = await api.pickImageFiles();
                    if (files[0]) record({ kind: "replace_image", object: img.object, path: files[0] });
                  }}
                >
                  <ImageUp />
                </button>
                <button type="button" className="btn icon" title="Delete picture" aria-label="Delete picture" onClick={() => deleteTarget(t)}>
                  <Trash2 />
                </button>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

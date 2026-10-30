// Design Studio (PRD 10, D-028): a flat picture or page rebuilt as editable objects. Layers on the
// left, the canvas in the middle (select, drag, resize from the corners, arrow keys nudge), the
// selected object's properties and the reconstruction's uncertainty on the right; the original can
// be shown, overlaid or differenced. Exports go through the engine (SVG/PDF, read back).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  Crop,
  AlignCenter,
  AlignLeft,
  AlignRight,
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignEndHorizontal,
  AlignEndVertical,
  AlignHorizontalDistributeCenter,
  AlignStartHorizontal,
  AlignStartVertical,
  AlignVerticalDistributeCenter,
  Group as GroupIcon,
  RotateCw,
  Ungroup as UngroupIcon,
  ArrowDown,
  ArrowUp,
  Copy,
  Eye,
  EyeOff,
  FileDown,
  Image as ImageIcon,
  Lock,
  LockOpen,
  Maximize,
  Redo2,
  Save,
  Square,
  Trash2,
  Type as TypeIcon,
  Undo2,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, join, stem } from "../paths";
import {
  addObject,
  align,
  bounds,
  commit,
  distribute,
  duplicate,
  group,
  hit,
  moveMany,
  redo,
  removeMany,
  reorder,
  resize,
  rotate,
  snap,
  start,
  uncertain,
  paragraphLayout,
  textPatch,
  shapeGeometry,
  cropBox,
  type Trim,
  dragCorner,
  type Corner,
  undo,
  ungroup,
  update,
  withGroup,
  type Align,
  type Guide,
  type History,
} from "../studio";
import { toPwError, type Design, type DesignFont, type DesignFormat, type DesignObject, type JobSnapshot, type PwError } from "../types";

/** The same families the exports use (SVG generic families; PDF Helvetica, Times, Courier). */
const FONT_CSS: Record<DesignFont, string> = {
  sans: "Helvetica, Arial, sans-serif",
  serif: "'Times New Roman', Times, serif",
  mono: "'Courier New', Courier, monospace",
};

export type StudioSource = { kind: "rebuild"; input: string; page: number; docId?: number } | { kind: "open"; path: string };

type Compare = "design" | "original" | "overlay" | "difference";
/** A pointer drag: the moved objects (one for a resize) and their box when it started. */
type Drag = { ids: string[]; corner: "move" | Corner; x0: number; y0: number; box: { x: number; y: number; w: number; h: number } };

interface Props {
  source: StudioSource;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
}

const KIND_ICON = { text: <TypeIcon />, shape: <Square />, image: <ImageIcon /> };
const KIND_LABEL = { text: "Text", shape: "Shape", image: "Picture" };

/** Where a rebuilt design is saved: next to its source. */
export function projectPath(src: { input: string; page: number }): string {
  const pdf = /\.pdf$/i.test(src.input);
  return join(dirname(src.input), `${stem(src.input)}${pdf ? `-page${src.page + 1}` : ""}.pwdesign`);
}

export function Studio({ source, jobs, onClose }: Props) {
  const api = useApi();
  const [path, setPath] = useState<string | null>(source.kind === "open" ? source.path : null);
  const [hist, setHist] = useState<History | null>(null);
  const [saved, setSaved] = useState<Design | null>(null);
  const [selected, setSelectedIds] = useState<string[]>([]);
  const setSelected = (id: string | null) => setSelectedIds(id ? [id] : []);
  const [preview, setPreview] = useState<Design | null>(null);
  // A picture being cropped: trims from each side (design pixels), previewed on the canvas.
  const [crop, setCrop] = useState<(Trim & { id: string }) | null>(null);
  const [cropping, setCropping] = useState(false);
  const selId = selected.length === 1 ? selected[0] : null;
  useEffect(() => setCrop(null), [selId]);
  const [guides, setGuides] = useState<Guide[]>([]);
  const [compare, setCompare] = useState<Compare>("design");
  const [opacity, setOpacity] = useState(50);
  const [zoom, setZoom] = useState(1);
  const [error, setError] = useState<PwError | null>(null);
  const [status, setStatus] = useState<{ text: string; reveal?: string } | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [exportJob, setExportJob] = useState<{ id: string; output: string } | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [fit, setFit] = useState(1);
  const canvas = useRef<HTMLDivElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const root = useRef<HTMLDivElement>(null);

  const design = preview ?? hist?.present ?? null;
  const dirty = !!hist && hist.present !== saved;
  const chosen = design ? design.objects.filter((o) => selected.includes(o.id)) : [];
  // One object: its properties; several: the arrange panel.
  const sel = chosen.length === 1 ? chosen[0]! : null;
  const job = jobId ? jobs[jobId] : undefined;
  const xjob = exportJob ? jobs[exportJob.id] : undefined;
  const building = source.kind === "rebuild" && !hist && !error;

  const load = useCallback(
    (p: string) => {
      api.loadDesign(p).then(
        (d) => {
          setPath(p);
          setHist(start(d));
          setSaved(d);
        },
        (e) => setError(toPwError(e)),
      );
    },
    [api],
  );

  // Open a project, or rebuild the source as one first.
  useEffect(() => {
    if (source.kind === "open") {
      load(source.path);
      return;
    }
    api
      .startOperation(
        { schema_version: 1, operation: { op: "reconstruct_design", input: source.input, page: source.page, output: projectPath(source) }, collision: "rename" },
        source.docId,
      )
      .then(setJobId, (e) => setError(toPwError(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      const out = job.result?.outputs[0]?.path;
      setJobId(null);
      if (out) load(out);
    } else if (job.state === "failed") {
      setJobId(null);
      setError(job.error ?? toPwError(new Error("The design could not be rebuilt.")));
    } else if (job.state === "cancelled") {
      onClose();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  useEffect(() => {
    if (!xjob || !exportJob) return;
    if (xjob.state === "succeeded") {
      setStatus({ text: `Exported ${basename(exportJob.output)}`, reveal: exportJob.output });
      setExportJob(null);
    } else if (xjob.state === "failed" || xjob.state === "cancelled") {
      if (xjob.error) setError(xjob.error);
      setExportJob(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [xjob?.state]);

  // Fit the page into the canvas; recomputed when the canvas resizes.
  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el || !design) return;
    const measure = () => setFit(Math.max(0.05, Math.min((el.clientWidth - 48) / design.width, (el.clientHeight - 48) / design.height)));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [design?.width, design?.height]);

  const scale = fit * zoom;

  /* ---------- Zoom and pan ---------- */
  // Ctrl+wheel zooms around the pointer: the design point under it stays under it.
  const anchor = useRef<{ x: number; y: number; cx: number; cy: number } | null>(null);
  const zoomBy = (factor: number, at?: { clientX: number; clientY: number }) => {
    const st = stage.current;
    if (at && st) {
      const r = st.getBoundingClientRect();
      anchor.current = { x: (at.clientX - r.left) / scale, y: (at.clientY - r.top) / scale, cx: at.clientX, cy: at.clientY };
    }
    setZoom((z) => Math.min(8, Math.max(0.25, z * factor)));
  };
  useLayoutEffect(() => {
    const a = anchor.current;
    const st = stage.current;
    const cv = canvas.current;
    anchor.current = null;
    if (!a || !st || !cv) return;
    const r = st.getBoundingClientRect();
    cv.scrollLeft += r.left + a.x * scale - a.cx;
    cv.scrollTop += r.top + a.y * scale - a.cy;
  }, [scale]);
  useEffect(() => {
    const cv = canvas.current;
    if (!cv) return;
    // Not passive: Ctrl+wheel must not also zoom the whole window.
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomBy(e.deltaY < 0 ? 1.25 : 1 / 1.25, e);
    };
    cv.addEventListener("wheel", onWheel, { passive: false });
    return () => cv.removeEventListener("wheel", onWheel);
  });
  // Pan: middle-button drag, or Space held with the left button.
  const [spaceDown, setSpaceDown] = useState(false);
  const pan = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  const [panning, setPanning] = useState(false);
  const onCanvasPointerDown = (e: React.PointerEvent) => {
    const cv = canvas.current;
    if (!cv || !(e.button === 1 || (e.button === 0 && spaceDown))) return;
    pan.current = { x: e.clientX, y: e.clientY, left: cv.scrollLeft, top: cv.scrollTop };
    setPanning(true);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
    e.stopPropagation();
  };
  const onCanvasPointerMove = (e: React.PointerEvent) => {
    const p = pan.current;
    const cv = canvas.current;
    if (!p || !cv) return;
    cv.scrollLeft = p.left - (e.clientX - p.x);
    cv.scrollTop = p.top - (e.clientY - p.y);
  };
  const endPan = () => {
    pan.current = null;
    setPanning(false);
  };

  const change = (next: Design, key?: string) => setHist((h) => (h ? commit(h, next, key) : h));
  const edit = (patch: Partial<DesignObject>, key?: string) => sel && hist && change(update(hist.present, sel.id, patch), key && `${sel.id}:${key}`);

  const save = async (): Promise<boolean> => {
    if (!hist || !path) return false;
    const d = hist.present;
    try {
      await api.saveDesign(path, d);
      setSaved(d);
      setStatus({ text: `Saved ${basename(path)}` });
      return true;
    } catch (e) {
      setError(toPwError(e));
      return false;
    }
  };

  const exportAs = async (format: DesignFormat) => {
    if (!hist || !path || exportJob) return;
    if (dirty && !(await save())) return;
    const output = await api.pickDesignExport(join(dirname(path), `${stem(path)}.${format}`), format);
    if (!output) return;
    try {
      // The save dialog has already asked about replacing an existing file.
      const id = await api.startOperation({ schema_version: 1, operation: { op: "export_design", input: path, format, output }, collision: "replace" });
      setStatus({ text: `Exporting ${basename(output)}…` });
      setExportJob({ id, output });
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const close = () => (dirty ? setConfirmClose(true) : onClose());

  /* ---------- Canvas pointer ---------- */
  const toDesign = (e: { clientX: number; clientY: number }) => {
    const r = stage.current!.getBoundingClientRect();
    return { x: (e.clientX - r.left) / scale, y: (e.clientY - r.top) / scale };
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (!hist || e.button !== 0) return;
    const p = toDesign(e);
    const corner = (e.target as HTMLElement).dataset.corner as Drag["corner"] | undefined;
    stage.current?.focus();
    if (corner && sel) {
      drag.current = { ids: [sel.id], corner, x0: p.x, y0: p.y, box: { x: sel.x, y: sel.y, w: sel.w, h: sel.h } };
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      e.preventDefault();
      return;
    }
    const o = hit(hist.present, p.x, p.y);
    // Shift adds or removes an object (with its group); a plain click selects it (and its group);
    // a click on an already selected object keeps the selection so all of it can be dragged.
    const ids = o ? withGroup(hist.present, o.id) : [];
    let next = ids;
    if (e.shiftKey) next = o && selected.includes(o.id) ? selected.filter((x) => !ids.includes(x)) : [...new Set([...selected, ...ids])];
    else if (o && selected.includes(o.id)) next = selected;
    setSelectedIds(next);
    if (!o || e.shiftKey) return;
    const moving = next.filter((id) => !hist.present.objects.find((x) => x.id === id)?.locked);
    const box = bounds(hist.present, moving);
    if (!box) return;
    drag.current = { ids: moving, corner: "move", x0: p.x, y0: p.y, box };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d || !hist) return;
    const p = toDesign(e);
    const dx = p.x - d.x0;
    const dy = p.y - d.y0;
    if (d.corner === "move") {
      // Snap to the canvas and other objects within about 6 screen pixels; Alt places freely.
      const box = { x: d.box.x + dx, y: d.box.y + dy, w: d.box.w, h: d.box.h };
      const s = e.altKey ? { x: box.x, y: box.y, guides: [] } : snap(hist.present, d.ids, box, 6 / scale);
      setGuides(s.guides);
      // Exact delta: rounding it to whole pixels pulled snapped objects that sit at fractional
      // positions up to half a pixel off the guide (moveBy keeps tenths).
      setPreview(moveMany(hist.present, d.ids, s.x - d.box.x, s.y - d.box.y));
      return;
    }
    const turned = hist.present.objects.find((o) => o.id === d.ids[0])?.rotation ?? 0;
    setPreview(resize(hist.present, d.ids[0]!, dragCorner(d.box, turned, d.corner, dx, dy)));
  };

  const onPointerUp = () => {
    if (drag.current && preview) change(preview);
    drag.current = null;
    setPreview(null);
    setGuides([]);
  };

  const add = (kind: "text" | "shape") => {
    if (!hist) return;
    const r = addObject(hist.present, kind);
    change(r.design);
    setSelected(r.id);
  };

  const applyCrop = async () => {
    if (!sel || sel.kind !== "image" || !crop || crop.id !== sel.id || cropping) return;
    const { box, fractions, empty } = cropBox(sel, crop);
    if (empty) return;
    const id = sel.id;
    setCropping(true);
    try {
      const png = await api.cropPicture(sel.png ?? "", fractions);
      setHist((h) => (h ? commit(h, update(h.present, id, { png, ...box })) : h));
      setCrop(null);
    } catch (e) {
      setError(toPwError(e));
    } finally {
      setCropping(false);
    }
  };

  const replacePicture = async () => {
    if (!sel || sel.kind !== "image") return;
    const id = sel.id;
    const [file] = await api.pickImageFiles();
    if (!file) return;
    try {
      const png = await api.loadPicture(file);
      setHist((h) => (h ? commit(h, update(h.present, id, { png, note: null, name: basename(file) })) : h));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  /* ---------- Keyboard ---------- */
  const onKeyDown = (e: React.KeyboardEvent) => {
    const typing = (e.target as HTMLElement).closest("input, textarea, select");
    const ctrl = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    let handled = true;
    if (ctrl && k === "s") void save();
    else if (ctrl && (k === "=" || k === "+")) zoomBy(1.25);
    else if (ctrl && k === "-") zoomBy(1 / 1.25);
    else if (ctrl && k === "0") setZoom(1);
    else if (e.key === " " && !typing && !(e.target as HTMLElement).closest("button, a, [role=radio]")) {
      if (!e.repeat) setSpaceDown(true);
    } else if (ctrl && (k === "y" || (k === "z" && e.shiftKey))) setHist((h) => (h ? redo(h) : h));
    else if (ctrl && k === "z") setHist((h) => (h ? undo(h) : h));
    else if (e.key === "Escape" && !typing) {
      if (confirmClose) setConfirmClose(false);
      else if (selected.length) setSelected(null);
      else close();
    } else if (typing || !hist) handled = false;
    else if (ctrl && k === "a") setSelectedIds(hist.present.objects.filter((o) => o.visible).map((o) => o.id));
    else if (!selected.length) handled = false;
    else if (ctrl && k === "g" && e.shiftKey) change(ungroup(hist.present, selected));
    else if (ctrl && k === "g") change(group(hist.present, selected));
    else if (ctrl && k === "d" && sel) {
      const r = duplicate(hist.present, sel.id);
      change(r.design);
      setSelected(r.id);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      change(removeMany(hist.present, selected));
      setSelected(null);
    } else if (e.key.startsWith("Arrow")) {
      const step = e.shiftKey ? 10 : 1;
      const [dx, dy] = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key] ?? [0, 0];
      change(moveMany(hist.present, selected, dx!, dy!), `${selected.join(",")}:nudge`);
    } else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  useEffect(() => root.current?.focus(), []);

  const layers = useMemo(() => (design ? [...design.objects].reverse() : []), [design]);
  const doubtful = useMemo(() => (design ? design.objects.filter(uncertain) : []), [design]);
  const sourceUrl = design ? `data:image/png;base64,${design.source_png}` : "";

  return (
    <div className="studio" role="dialog" aria-modal="true" aria-label="Design Studio" ref={root} tabIndex={-1} onKeyDown={onKeyDown} onKeyUp={(e) => e.key === " " && setSpaceDown(false)} onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setSpaceDown(false)} data-testid="studio">
      <header className="studio-bar" role="toolbar" aria-label="Design Studio">
        <h2 title={design?.source_name}>
          Design Studio{design && <span className="hint"> — {design.source_name}</span>}
          {dirty && <span className="dirty" title="Unsaved changes" />}
        </h2>
        <div className="group">
          <button type="button" className="btn icon" aria-label="Undo" title="Undo (Ctrl+Z)" aria-disabled={!hist?.past.length || undefined} onClick={() => setHist((h) => (h ? undo(h) : h))}>
            <Undo2 />
          </button>
          <button type="button" className="btn icon" aria-label="Redo" title="Redo (Ctrl+Y)" aria-disabled={!hist?.future.length || undefined} onClick={() => setHist((h) => (h ? redo(h) : h))}>
            <Redo2 />
          </button>
        </div>
        <div className="group">
          <button type="button" className="btn icon" aria-label="Add text" title="Add text" data-testid="studio-add-text" aria-disabled={!hist || undefined} onClick={() => add("text")}>
            <TypeIcon />
          </button>
          <button type="button" className="btn icon" aria-label="Add rectangle" title="Add rectangle" data-testid="studio-add-rect" aria-disabled={!hist || undefined} onClick={() => add("shape")}>
            <Square />
          </button>
        </div>
        <div className="segmented" role="radiogroup" aria-label="Compare with the original">
          {(["design", "original", "overlay", "difference"] as Compare[]).map((m) => (
            <button key={m} type="button" role="radio" aria-checked={compare === m} data-compare={m} onClick={() => setCompare(m)}>
              {{ design: "Design", original: "Original", overlay: "Overlay", difference: "Differences" }[m]}
            </button>
          ))}
        </div>
        {compare === "overlay" && (
          <input type="range" className="studio-opacity" min={0} max={100} value={opacity} aria-label="Original's opacity" title={`Original at ${opacity}%`} onChange={(e) => setOpacity(Number(e.target.value))} />
        )}
        <span className="spacer" />
        <div className="group">
          <button type="button" className="btn icon" aria-label="Zoom out" title="Zoom out (Ctrl+−)" onClick={() => zoomBy(1 / 1.25)}>
            <ZoomOut />
          </button>
          <button type="button" className="btn icon" aria-label="Fit" title={`Fit (Ctrl+0) — ${Math.round(scale * 100)}%`} data-testid="studio-fit" onClick={() => setZoom(1)}>
            <Maximize />
          </button>
          <button type="button" className="btn icon" aria-label="Zoom in" title="Zoom in (Ctrl+=)" onClick={() => zoomBy(1.25)}>
            <ZoomIn />
          </button>
        </div>
        <div className="group">
          <button type="button" className="btn outline" data-testid="studio-export-svg" aria-disabled={!hist || !!exportJob || undefined} title="Export as editable SVG" onClick={() => exportAs("svg")}>
            <FileDown /> SVG
          </button>
          <button type="button" className="btn outline" data-testid="studio-export-pdf" aria-disabled={!hist || !!exportJob || undefined} title="Export as PDF with real text" onClick={() => exportAs("pdf")}>
            <FileDown /> PDF
          </button>
          <button type="button" className="btn outline" data-testid="studio-export-png" aria-disabled={!hist || !!exportJob || undefined} title="Export as a PNG picture at the design's size" onClick={() => exportAs("png")}>
            <FileDown /> PNG
          </button>
          <button type="button" className="btn primary" data-testid="studio-save" aria-disabled={!dirty || undefined} title={dirty ? "Save the design (Ctrl+S)" : "No unsaved changes"} onClick={() => dirty && save()}>
            <Save /> Save
          </button>
          <button type="button" className="btn icon" aria-label="Close the Design Studio" title="Close (Esc)" data-testid="studio-close" onClick={close}>
            <X />
          </button>
        </div>
      </header>

      {confirmClose && (
        <div className="banner" role="alertdialog" aria-label="Unsaved changes">
          <span className="grow">Save the changes to this design before closing?</span>
          <button type="button" className="btn primary" data-testid="studio-close-save" onClick={async () => (await save()) && onClose()}>
            Save
          </button>
          <button type="button" className="btn outline" data-testid="studio-close-discard" onClick={onClose}>
            Don’t save
          </button>
          <button type="button" className="btn outline" onClick={() => setConfirmClose(false)}>
            Cancel
          </button>
        </div>
      )}
      {error && (
        <div className="banner error" role="alert" data-testid="studio-error">
          <span className="grow">
            {error.message}
            {error.action ? ` ${error.action}` : ""}
          </span>
          <button type="button" className="btn outline" onClick={() => (hist ? setError(null) : onClose())}>
            {hist ? "Dismiss" : "Close"}
          </button>
        </div>
      )}

      {building || (!design && !error) ? (
        <div className="studio-wait" aria-live="polite">
          <p>{building ? "Rebuilding the design: finding text, shapes and pictures…" : "Opening the design…"}</p>
          <div className="progress indeterminate" role="progressbar" aria-label="Rebuilding">
            <div />
          </div>
          {jobId && (
            <button type="button" className="btn outline" onClick={() => api.cancelJob(jobId)} disabled={job?.state === "cancelling"}>
              {job?.state === "cancelling" ? "Cancelling…" : "Cancel"}
            </button>
          )}
        </div>
      ) : design ? (
        <div className="studio-body">
          <aside className="studio-layers" aria-label="Layers">
            <div className="studio-pane-head">
              <h3>Layers</h3>
              <span className="hint">{design.objects.length}</span>
            </div>
            <ul aria-label="Layers, front first" data-testid="studio-layers">
              {layers.map((o) => (
                <li key={o.id} className={`layer ${selected.includes(o.id) ? "selected" : ""} ${o.visible ? "" : "hidden"}`}>
                  <button
                    type="button"
                    aria-current={selected.includes(o.id) || undefined}
                    className="layer-name"
                    data-layer={o.id}
                    title={`${o.note ?? KIND_LABEL[o.kind]}${o.group ? " · grouped" : ""} (Shift+click adds to the selection)`}
                    onClick={(e) => {
                      const ids = withGroup(design, o.id);
                      if (!e.shiftKey) setSelectedIds(ids);
                      else setSelectedIds((cur) => (cur.includes(o.id) ? cur.filter((x) => !ids.includes(x)) : [...new Set([...cur, ...ids])]));
                    }}
                  >
                    {KIND_ICON[o.kind]}
                    <span>{o.name || KIND_LABEL[o.kind]}</span>
                    {uncertain(o) && <AlertTriangle className="warn" aria-label="Uncertain" />}
                  </button>
                  <button type="button" className="btn icon" aria-label={o.visible ? `Hide ${o.name}` : `Show ${o.name}`} aria-pressed={!o.visible} title={o.visible ? "Hide" : "Show"} onClick={() => change(update(design, o.id, { visible: !o.visible }))}>
                    {o.visible ? <Eye /> : <EyeOff />}
                  </button>
                  <button type="button" className="btn icon" aria-label={o.locked ? `Unlock ${o.name}` : `Lock ${o.name}`} aria-pressed={o.locked} title={o.locked ? "Unlock" : "Lock"} onClick={() => change(update(design, o.id, { locked: !o.locked }))}>
                    {o.locked ? <Lock /> : <LockOpen />}
                  </button>
                </li>
              ))}
            </ul>
          </aside>

          <div
            className={`studio-canvas ${panning ? "panning" : spaceDown ? "pan-ready" : ""}`}
            ref={canvas}
            data-testid="studio-canvas"
            onPointerDownCapture={onCanvasPointerDown}
            onPointerMove={onCanvasPointerMove}
            onPointerUp={endPan}
            onPointerCancel={endPan}
            onAuxClick={(e) => e.preventDefault()}
          >
            <div
              className="studio-stage"
              ref={stage}
              tabIndex={0}
              role="application"
              aria-roledescription="design canvas"
              aria-label="Design canvas: Shift+click selects several objects; arrow keys move the selection (Shift for 10 pixels), Delete removes it, Ctrl+D duplicates, Ctrl+G groups, Ctrl+A selects all"
              data-testid="studio-stage"
              style={{ width: design.width * scale, height: design.height * scale }}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
            >
              {compare !== "original" && <DesignSvg design={design} />}
              {compare !== "design" && (
                <img
                  className="studio-source"
                  src={sourceUrl}
                  alt=""
                  draggable={false}
                  style={{ opacity: compare === "overlay" ? opacity / 100 : 1, mixBlendMode: compare === "difference" ? "difference" : "normal" }}
                />
              )}
              {guides.map((g, i) => (
                <span
                  key={i}
                  className={`studio-guide ${g.axis}`}
                  data-testid="studio-guide"
                  style={g.axis === "x" ? { left: g.at * scale } : { top: g.at * scale }}
                />
              ))}
              {chosen
                .filter((o) => o.visible)
                .map((o) => (
                  <div
                    key={o.id}
                    className={`studio-selection ${o.locked ? "locked" : ""}`}
                    data-selected={o.id}
                    style={{ left: o.x * scale, top: o.y * scale, width: o.w * scale, height: o.h * scale, transform: o.rotation ? `rotate(${o.rotation}deg)` : undefined }}
                  >
                    {/* Corner handles for one object; they turn with it. */}
                    {sel === o && !o.locked && (["nw", "ne", "sw", "se"] as const).map((c) => <span key={c} className={`handle ${c}`} data-corner={c} />)}
                  </div>
                ))}
              {sel && crop?.id === sel.id && !cropBox(sel, crop).empty && (() => {
                const k = cropBox(sel, crop).box;
                return <div className="studio-crop-box" data-testid="studio-crop-box" style={{ left: k.x * scale, top: k.y * scale, width: k.w * scale, height: k.h * scale }} />;
              })()}
            </div>
          </div>

          <aside className="studio-props" aria-label="Properties">
            {sel ? (
              <Properties o={sel} edit={edit} design={design} onReplace={replacePicture} crop={crop?.id === sel.id ? crop : null} cropping={cropping} onCrop={(t) => setCrop(t && { ...t, id: sel.id })} onApplyCrop={applyCrop} onOrder={(step) => change(reorder(design, sel.id, step))} onDuplicate={() => {
                const r = duplicate(design, sel.id);
                change(r.design);
                setSelected(r.id);
              }} onDelete={() => {
                change(removeMany(design, [sel.id]));
                setSelected(null);
              }} onRotate={(deg) => change(rotate(design, sel.id, deg), `${sel.id}:rotation`)} />
            ) : chosen.length > 1 ? (
              <Arrange
                count={chosen.length}
                grouped={chosen.some((o) => o.group)}
                onAlign={(how) => change(align(design, selected, how))}
                onDistribute={(axis) => change(distribute(design, selected, axis))}
                onGroup={() => change(group(design, selected))}
                onUngroup={() => change(ungroup(design, selected))}
                onDelete={() => {
                  change(removeMany(design, selected));
                  setSelected(null);
                }}
              />
            ) : (
              <div className="studio-summary" data-testid="studio-summary">
                <h3>Reconstruction</h3>
                <p className="hint">
                  {count(design, "text")} text · {count(design, "shape")} shapes · {count(design, "image")} pictures
                </p>
                <p className="hint">Rebuilt from a flat picture: these are likely objects, not the original layers.</p>
                <label className="studio-num studio-background">
                  <span>Background</span>
                  <input type="color" data-testid="studio-background" value={design.background} onChange={(e) => change({ ...design, background: e.target.value }, "background")} />
                </label>
                {doubtful.length > 0 && (
                  <>
                    <h3>Check these ({doubtful.length})</h3>
                    <ul className="studio-doubts">
                      {doubtful.map((o) => (
                        <li key={o.id}>
                          <button type="button" className="tool-item" onClick={() => setSelected(o.id)}>
                            {KIND_ICON[o.kind]}
                            <span>{o.name}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </div>
            )}
          </aside>
        </div>
      ) : null}

      {status && (
        <div className="studio-status" role="status" data-testid="studio-status">
          {status.text}
          {status.reveal && (
            <button type="button" className="btn link" onClick={() => api.revealInFolder(status.reveal!).catch((e) => setError(toPwError(e)))}>
              Show in folder
            </button>
          )}
        </div>
      )}
    </div>
  );
}

const count = (d: Design, kind: DesignObject["kind"]) => d.objects.filter((o) => o.kind === kind).length;

/** The design as SVG, drawn the way the engine's SVG export writes it. */
function Shape({ o, turn }: { o: DesignObject; turn?: string }) {
  const g = shapeGeometry(o);
  const paint = { fill: o.fill ?? "none", stroke: g.sw ? (o.stroke ?? undefined) : undefined, strokeWidth: g.sw || undefined, transform: turn };
  return o.ellipse ? (
    <ellipse data-object={o.id} cx={o.x + o.w / 2} cy={o.y + o.h / 2} rx={g.w / 2} ry={g.h / 2} {...paint} />
  ) : (
    <rect data-object={o.id} x={g.x} y={g.y} width={g.w} height={g.h} rx={g.r || undefined} ry={g.r || undefined} {...paint} />
  );
}

function Paragraph({ o, turn }: { o: DesignObject; turn?: string }) {
  const p = paragraphLayout(o);
  return (
    <text
      data-object={o.id}
      transform={[turn, p.scale].filter(Boolean).join(" ")}
      fontFamily={FONT_CSS[o.font ?? "sans"]}
      fontSize={o.font_size ?? o.h}
      fontWeight={o.bold ? "bold" : "normal"}
      fontStyle={o.italic ? "italic" : "normal"}
      fill={o.color}
      textAnchor={p.anchor}
      xmlSpace="preserve"
    >
      {p.lines.map((l, i) => (
        <tspan key={i} x={l.x} y={l.y}>
          {l.text}
        </tspan>
      ))}
    </text>
  );
}

export function DesignSvg({ design }: { design: Design }) {
  return (
    <svg className="studio-svg" viewBox={`0 0 ${design.width} ${design.height}`} aria-hidden="true">
      <rect width={design.width} height={design.height} fill={design.background} />
      {design.objects.map((o) => {
        const turn = o.rotation ? `rotate(${o.rotation} ${o.x + o.w / 2} ${o.y + o.h / 2})` : undefined;
        return !o.visible ? null : o.kind === "shape" ? (
          <Shape key={o.id} o={o} turn={turn} />
        ) : o.kind === "image" ? (
          <image key={o.id} data-object={o.id} x={o.x} y={o.y} width={o.w} height={o.h} preserveAspectRatio="none" href={`data:image/png;base64,${o.png}`} transform={turn} />
        ) : o.text?.includes("\n") ? (
          <Paragraph key={o.id} o={o} turn={turn} />
        ) : (
          <text
            key={o.id}
            data-object={o.id}
            transform={turn}
            x={o.x}
            y={o.y + (o.baseline ?? (o.font_size ?? o.h) * 0.72)}
            fontFamily={FONT_CSS[o.font ?? "sans"]}
            fontSize={o.font_size ?? o.h}
            fontWeight={o.bold ? "bold" : "normal"}
            fontStyle={o.italic ? "italic" : "normal"}
            fill={o.color}
            textLength={o.w}
            lengthAdjust="spacingAndGlyphs"
            xmlSpace="preserve"
          >
            {o.text}
          </text>
        );
      })}
    </svg>
  );
}

function Properties({
  o,
  design,
  edit,
  onOrder,
  onDuplicate,
  onDelete,
  onReplace,
  onRotate,
  crop,
  cropping,
  onCrop,
  onApplyCrop,
}: {
  o: DesignObject;
  design: Design;
  crop: Trim | null;
  cropping: boolean;
  onCrop: (t: Trim | null) => void;
  onApplyCrop: () => void;
  onReplace: () => void;
  onRotate: (degrees: number) => void;
  edit: (patch: Partial<DesignObject>, key?: string) => void;
  onOrder: (step: 1 | -1) => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const i = design.objects.findIndex((x) => x.id === o.id);
  const num = (key: "x" | "y" | "w" | "h", label: string) => (
    <label className="studio-num">
      <span>{label}</span>
      <input className="input" type="number" value={o[key]} disabled={o.locked} onChange={(e) => Number.isFinite(e.target.valueAsNumber) && edit({ [key]: Math.max(key === "w" || key === "h" ? 1 : -1e5, e.target.valueAsNumber) }, key)} />
    </label>
  );
  return (
    <div className="studio-form" data-testid="studio-props">
      <div className="studio-pane-head">
        <h3>
          {KIND_ICON[o.kind]} {KIND_LABEL[o.kind]}
        </h3>
        <span className={`studio-confidence ${uncertain(o) ? "low" : ""}`} title="How sure the reconstruction is">
          {o.kind === "image" ? "Kept as picture" : `${Math.round(o.confidence * 100)}% sure`}
        </span>
      </div>
      {o.note && (
        <p className="studio-note" role="note">
          <AlertTriangle /> {o.note}
        </p>
      )}
      <div className="field">
        <label htmlFor="studio-name">Name</label>
        <input id="studio-name" className="input" value={o.name} maxLength={200} onChange={(e) => edit({ name: e.target.value }, "name")} />
      </div>
      {o.kind === "text" && (
        <>
          <div className="field">
            <label htmlFor="studio-text">Text</label>
            <textarea id="studio-text" className="input" rows={3} value={o.text ?? ""} maxLength={10000} onChange={(e) => edit(textPatch(o, e.target.value), "text")} />
          </div>
          {o.text?.includes("\n") && (
            <div className="row">
              <div className="segmented" role="radiogroup" aria-label="Alignment">
                {(["left", "center", "right"] as const).map((a) => (
                  <button key={a} type="button" role="radio" data-align-text={a} aria-checked={(o.align ?? "left") === a} aria-label={`Align ${a === "center" ? "centre" : a}`} title={`Align ${a === "center" ? "centre" : a}`} onClick={() => edit({ align: a })}>
                    {a === "left" ? <AlignLeft /> : a === "center" ? <AlignCenter /> : <AlignRight />}
                  </button>
                ))}
              </div>
              <label className="studio-num">
                <span>Line spacing</span>
                <input className="input" type="number" min={1} data-testid="studio-line-height" value={Math.round(paragraphLayout(o).lineHeight * 10) / 10} onChange={(e) => e.target.valueAsNumber > 0 && edit({ line_height: e.target.valueAsNumber }, "line-height")} />
              </label>
            </div>
          )}
          <div className="row">
            <label className="studio-num">
              <span>Size</span>
              <input className="input" type="number" min={1} value={o.font_size ?? 0} onChange={(e) => e.target.valueAsNumber > 0 && edit({ font_size: e.target.valueAsNumber }, "size")} />
            </label>
            <label className="studio-num">
              <span>Colour</span>
              <input type="color" value={o.color ?? "#000000"} onChange={(e) => edit({ color: e.target.value }, "color")} />
            </label>
          </div>
          <div className="row">
            <label className="studio-num">
              <span>Font</span>
              <select className="select" data-testid="studio-font" value={o.font ?? "sans"} title="Family guessed from the letter shapes (serif or sans); the original font is not identified. These fonts are on every computer." onChange={(e) => edit({ font: e.target.value as DesignFont })}>
                <option value="sans">Sans serif</option>
                <option value="serif">Serif</option>
                <option value="mono">Monospace</option>
              </select>
            </label>
            <label className="row">
              <input type="checkbox" checked={o.bold} onChange={(e) => edit({ bold: e.target.checked })} /> Bold
            </label>
            <label className="row">
              <input type="checkbox" data-testid="studio-italic" checked={!!o.italic} onChange={(e) => edit({ italic: e.target.checked })} /> Italic
            </label>
          </div>
        </>
      )}
      {o.kind === "image" && (
        <button type="button" className="btn outline studio-replace" data-testid="studio-replace" disabled={o.locked} title={o.locked ? "Unlock the picture first" : "Replace with a PNG or JPEG picture; keeps its place and size"} onClick={onReplace}>
          <ImageIcon /> Replace picture…
        </button>
      )}
      {o.kind === "image" && (
        <fieldset className="studio-crop" disabled={o.locked || !!o.rotation || cropping} title={o.locked ? "Unlock the picture first" : o.rotation ? "Set the rotation to 0 to crop" : undefined}>
          <legend>Crop</legend>
          <div className="studio-grid">
            {(["l", "t", "r", "b"] as const).map((side) => (
              <label key={side} className="studio-num">
                <span>{{ l: "Left", t: "Top", r: "Right", b: "Bottom" }[side]}</span>
                <input
                  className="input"
                  type="number"
                  min={0}
                  data-crop={side}
                  value={crop?.[side] ?? 0}
                  onChange={(e) => {
                    const v = e.target.valueAsNumber;
                    if (Number.isFinite(v)) onCrop({ ...(crop ?? { l: 0, t: 0, r: 0, b: 0 }), [side]: Math.max(0, v) });
                  }}
                />
              </label>
            ))}
          </div>
          <div className="row">
            <button type="button" className="btn primary" data-testid="studio-crop-apply" disabled={!crop || cropBox(o, crop).empty} onClick={onApplyCrop}>
              <Crop /> {cropping ? "Cropping…" : "Crop"}
            </button>
            <button type="button" className="btn outline" disabled={!crop} onClick={() => onCrop(null)}>
              Reset
            </button>
          </div>
        </fieldset>
      )}
      {o.kind === "shape" && (
        <div className="row">
          <label className="studio-num">
            <span>Fill</span>
            <input type="color" data-testid="studio-fill" value={o.fill ?? "#ffffff"} disabled={!o.fill} onChange={(e) => edit({ fill: e.target.value }, "fill")} />
          </label>
          <label className="row" title={!o.stroke_width && o.fill ? "Add an outline first, or the shape would be invisible" : undefined}>
            <input type="checkbox" data-testid="studio-no-fill" checked={!o.fill} disabled={!o.stroke_width && !!o.fill} onChange={(e) => edit({ fill: e.target.checked ? null : "#ffffff" })} /> No fill
          </label>
        </div>
      )}
      {o.kind === "shape" && (
        <div className="row">
          <label className="studio-num">
            <span>Outline</span>
            <input type="color" data-testid="studio-stroke" value={o.stroke ?? "#000000"} onChange={(e) => edit({ stroke: e.target.value, stroke_width: o.stroke_width || 2 }, "stroke")} />
          </label>
          <label className="studio-num">
            <span>Outline width</span>
            <input
              className="input"
              type="number"
              min={0}
              data-testid="studio-stroke-width"
              value={o.stroke_width ?? 0}
              title={!o.fill ? "A shape without fill keeps an outline" : "0: no outline"}
              onChange={(e) => {
                const v = e.target.valueAsNumber;
                if (!Number.isFinite(v)) return;
                const width = Math.min(Math.max(o.fill ? 0 : 1, v), Math.min(o.w, o.h) / 2);
                edit({ stroke_width: width, stroke: o.stroke ?? "#000000" }, "stroke-width");
              }}
            />
          </label>
        </div>
      )}
      {o.kind === "shape" && (
        <div className="row">
          <label className="studio-num">
            <span>Corner radius</span>
            <input
              className="input"
              type="number"
              min={0}
              data-testid="studio-radius"
              value={o.radius ?? 0}
              disabled={!!o.ellipse}
              title={o.ellipse ? "An ellipse has no corners" : undefined}
              onChange={(e) => Number.isFinite(e.target.valueAsNumber) && edit({ radius: Math.min(Math.max(0, e.target.valueAsNumber), Math.min(o.w, o.h) / 2) }, "radius")}
            />
          </label>
          <label className="row">
            <input type="checkbox" data-testid="studio-ellipse" checked={!!o.ellipse} onChange={(e) => edit({ ellipse: e.target.checked, radius: 0 })} /> Ellipse
          </label>
        </div>
      )}
      <div className="studio-grid">
        {num("x", "X")}
        {num("y", "Y")}
        {num("w", "Width")}
        {num("h", "Height")}
      </div>
      <div className="row studio-rotate">
        <label className="studio-num">
          <span>Rotation (°)</span>
          <input
            className="input"
            type="number"
            data-testid="studio-rotation"
            value={o.rotation ?? 0}
            disabled={o.locked}
            onChange={(e) => Number.isFinite(e.target.valueAsNumber) && onRotate(e.target.valueAsNumber)}
          />
        </label>
        <button type="button" className="btn icon" aria-label="Rotate 90° clockwise" title="Rotate 90° clockwise" disabled={o.locked} onClick={() => onRotate((o.rotation ?? 0) + 90)}>
          <RotateCw />
        </button>
      </div>
      <div className="row studio-actions">
        <button type="button" className="btn icon" aria-label="Bring forward" title="Bring forward" aria-disabled={i >= design.objects.length - 1 || undefined} onClick={() => onOrder(1)}>
          <ArrowUp />
        </button>
        <button type="button" className="btn icon" aria-label="Send backward" title="Send backward" aria-disabled={i <= 0 || undefined} onClick={() => onOrder(-1)}>
          <ArrowDown />
        </button>
        <button type="button" className="btn icon" aria-label="Duplicate" title="Duplicate (Ctrl+D)" onClick={onDuplicate}>
          <Copy />
        </button>
        <button type="button" className="btn icon" aria-label="Delete" title="Delete (Delete)" onClick={onDelete}>
          <Trash2 />
        </button>
      </div>
    </div>
  );
}

/** Several objects selected: line them up, space them, group them. */
function Arrange({
  count,
  grouped,
  onAlign,
  onDistribute,
  onGroup,
  onUngroup,
  onDelete,
}: {
  count: number;
  grouped: boolean;
  onAlign: (how: Align) => void;
  onDistribute: (axis: "x" | "y") => void;
  onGroup: () => void;
  onUngroup: () => void;
  onDelete: () => void;
}) {
  const aligns: [Align, string, ReactNode][] = [
    ["left", "Align left edges", <AlignStartVertical key="l" />],
    ["center", "Align centres horizontally", <AlignCenterVertical key="c" />],
    ["right", "Align right edges", <AlignEndVertical key="r" />],
    ["top", "Align top edges", <AlignStartHorizontal key="t" />],
    ["middle", "Align centres vertically", <AlignCenterHorizontal key="m" />],
    ["bottom", "Align bottom edges", <AlignEndHorizontal key="b" />],
  ];
  const few = count < 3 ? "Select three or more objects to space them evenly" : undefined;
  return (
    <div className="studio-form" data-testid="studio-arrange">
      <div className="studio-pane-head">
        <h3>{count} objects</h3>
      </div>
      <span className="label">Align</span>
      <div className="row studio-actions">
        {aligns.map(([how, label, icon]) => (
          <button key={how} type="button" className="btn icon" aria-label={label} title={label} data-align={how} onClick={() => onAlign(how)}>
            {icon}
          </button>
        ))}
      </div>
      <span className="label">Distribute</span>
      <div className="row studio-actions">
        <button type="button" className="btn icon" aria-label="Space evenly across" title={few ?? "Space evenly across"} aria-disabled={!!few || undefined} data-distribute="x" onClick={() => !few && onDistribute("x")}>
          <AlignHorizontalDistributeCenter />
        </button>
        <button type="button" className="btn icon" aria-label="Space evenly down" title={few ?? "Space evenly down"} aria-disabled={!!few || undefined} data-distribute="y" onClick={() => !few && onDistribute("y")}>
          <AlignVerticalDistributeCenter />
        </button>
      </div>
      <div className="row studio-actions">
        <button type="button" className="btn outline" data-testid="studio-group" title="Group (Ctrl+G)" onClick={onGroup}>
          <GroupIcon /> Group
        </button>
        <button type="button" className="btn outline" data-testid="studio-ungroup" title={grouped ? "Ungroup (Ctrl+Shift+G)" : "Nothing selected is grouped"} aria-disabled={!grouped || undefined} onClick={() => grouped && onUngroup()}>
          <UngroupIcon /> Ungroup
        </button>
        <button type="button" className="btn icon" aria-label="Delete the selected objects" title="Delete (Delete)" onClick={onDelete}>
          <Trash2 />
        </button>
      </div>
      <p className="hint">Shift+click adds or removes objects; arrow keys move them all.</p>
    </div>
  );
}

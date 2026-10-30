// Design Studio (PRD 10, D-028): a flat picture or page rebuilt as editable objects. Layers on the
// left, the canvas in the middle (select, drag, resize from the corners, arrow keys nudge), the
// selected object's properties and the reconstruction's uncertainty on the right; the original can
// be shown, overlaid or differenced. Exports go through the engine (SVG/PDF, read back).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
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
import { addObject, commit, duplicate, hit, moveBy, redo, remove, reorder, resize, snap, start, uncertain, undo, update, type Guide, type History } from "../studio";
import { toPwError, type Design, type DesignObject, type JobSnapshot, type PwError } from "../types";

export type StudioSource = { kind: "rebuild"; input: string; page: number; docId?: number } | { kind: "open"; path: string };

type Compare = "design" | "original" | "overlay" | "difference";
type Drag = { id: string; corner: "move" | "nw" | "ne" | "sw" | "se"; x0: number; y0: number; box: { x: number; y: number; w: number; h: number } };

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
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<Design | null>(null);
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
  const sel = design?.objects.find((o) => o.id === selected) ?? null;
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

  const exportAs = async (format: "svg" | "pdf") => {
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
    const o = corner && sel ? sel : hit(hist.present, p.x, p.y);
    setSelected(o?.id ?? null);
    stage.current?.focus();
    if (!o || o.locked) return;
    drag.current = { id: o.id, corner: corner ?? "move", x0: p.x, y0: p.y, box: { x: o.x, y: o.y, w: o.w, h: o.h } };
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
      const s = e.altKey ? { x: box.x, y: box.y, guides: [] } : snap(hist.present, d.id, box, 6 / scale);
      setGuides(s.guides);
      // Exact delta: rounding it to whole pixels pulled snapped objects that sit at fractional
      // positions up to half a pixel off the guide (moveBy keeps tenths).
      setPreview(moveBy(hist.present, d.id, s.x - d.box.x, s.y - d.box.y));
      return;
    }
    const b = d.box;
    const left = d.corner === "nw" || d.corner === "sw";
    const top = d.corner === "nw" || d.corner === "ne";
    const x = left ? Math.min(b.x + dx, b.x + b.w - 1) : b.x;
    const y = top ? Math.min(b.y + dy, b.y + b.h - 1) : b.y;
    const w = left ? b.x + b.w - x : Math.max(1, b.w + dx);
    const h = top ? b.y + b.h - y : Math.max(1, b.h + dy);
    setPreview(resize(hist.present, d.id, { x, y, w, h }));
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
    else if (ctrl && (k === "y" || (k === "z" && e.shiftKey))) setHist((h) => (h ? redo(h) : h));
    else if (ctrl && k === "z") setHist((h) => (h ? undo(h) : h));
    else if (e.key === "Escape" && !typing) {
      if (confirmClose) setConfirmClose(false);
      else if (selected) setSelected(null);
      else close();
    } else if (typing || !sel || !hist) handled = false;
    else if (ctrl && k === "d") {
      const r = duplicate(hist.present, sel.id);
      change(r.design);
      setSelected(r.id);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      change(remove(hist.present, sel.id));
      setSelected(null);
    } else if (e.key.startsWith("Arrow")) {
      const step = e.shiftKey ? 10 : 1;
      const [dx, dy] = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key] ?? [0, 0];
      change(moveBy(hist.present, sel.id, dx!, dy!), `${sel.id}:nudge`);
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
    <div className="studio" role="dialog" aria-modal="true" aria-label="Design Studio" ref={root} tabIndex={-1} onKeyDown={onKeyDown} data-testid="studio">
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
          <button type="button" className="btn icon" aria-label="Zoom out" title="Zoom out" onClick={() => setZoom((z) => Math.max(0.25, z / 1.25))}>
            <ZoomOut />
          </button>
          <button type="button" className="btn icon" aria-label="Fit" title={`Fit (${Math.round(scale * 100)}%)`} onClick={() => setZoom(1)}>
            <Maximize />
          </button>
          <button type="button" className="btn icon" aria-label="Zoom in" title="Zoom in" onClick={() => setZoom((z) => Math.min(8, z * 1.25))}>
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
                <li key={o.id} className={`layer ${o.id === selected ? "selected" : ""} ${o.visible ? "" : "hidden"}`}>
                  <button type="button" aria-current={o.id === selected || undefined} className="layer-name" data-layer={o.id} title={o.note ?? KIND_LABEL[o.kind]} onClick={() => setSelected(o.id)}>
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

          <div className="studio-canvas" ref={canvas}>
            <div
              className="studio-stage"
              ref={stage}
              tabIndex={0}
              role="application"
              aria-roledescription="design canvas"
              aria-label="Design canvas: arrow keys move the selected object (Shift for 10 pixels), Delete removes it, Ctrl+D duplicates it"
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
              {sel && sel.visible && (
                <div className={`studio-selection ${sel.locked ? "locked" : ""}`} style={{ left: sel.x * scale, top: sel.y * scale, width: sel.w * scale, height: sel.h * scale }}>
                  {!sel.locked && (["nw", "ne", "sw", "se"] as const).map((c) => <span key={c} className={`handle ${c}`} data-corner={c} />)}
                </div>
              )}
            </div>
          </div>

          <aside className="studio-props" aria-label="Properties">
            {sel ? (
              <Properties o={sel} edit={edit} design={design} onReplace={replacePicture} onOrder={(step) => change(reorder(design, sel.id, step))} onDuplicate={() => {
                const r = duplicate(design, sel.id);
                change(r.design);
                setSelected(r.id);
              }} onDelete={() => {
                change(remove(design, sel.id));
                setSelected(null);
              }} />
            ) : (
              <div className="studio-summary" data-testid="studio-summary">
                <h3>Reconstruction</h3>
                <p className="hint">
                  {count(design, "text")} text · {count(design, "shape")} shapes · {count(design, "image")} pictures
                </p>
                <p className="hint">Rebuilt from a flat picture: these are likely objects, not the original layers.</p>
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
export function DesignSvg({ design }: { design: Design }) {
  return (
    <svg className="studio-svg" viewBox={`0 0 ${design.width} ${design.height}`} aria-hidden="true">
      <rect width={design.width} height={design.height} fill={design.background} />
      {design.objects.map((o) =>
        !o.visible ? null : o.kind === "shape" ? (
          <rect key={o.id} data-object={o.id} x={o.x} y={o.y} width={o.w} height={o.h} fill={o.fill} />
        ) : o.kind === "image" ? (
          <image key={o.id} data-object={o.id} x={o.x} y={o.y} width={o.w} height={o.h} preserveAspectRatio="none" href={`data:image/png;base64,${o.png}`} />
        ) : (
          <text
            key={o.id}
            data-object={o.id}
            x={o.x}
            y={o.y + (o.baseline ?? (o.font_size ?? o.h) * 0.72)}
            fontFamily="Helvetica, Arial, sans-serif"
            fontSize={o.font_size ?? o.h}
            fontWeight={o.bold ? "bold" : "normal"}
            fill={o.color}
            textLength={o.w}
            lengthAdjust="spacingAndGlyphs"
            xmlSpace="preserve"
          >
            {o.text}
          </text>
        ),
      )}
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
}: {
  o: DesignObject;
  design: Design;
  onReplace: () => void;
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
            <textarea id="studio-text" className="input" rows={3} value={o.text ?? ""} maxLength={10000} onChange={(e) => edit({ text: e.target.value }, "text")} />
          </div>
          <div className="row">
            <label className="studio-num">
              <span>Size</span>
              <input className="input" type="number" min={1} value={o.font_size ?? 0} onChange={(e) => e.target.valueAsNumber > 0 && edit({ font_size: e.target.valueAsNumber }, "size")} />
            </label>
            <label className="studio-num">
              <span>Colour</span>
              <input type="color" value={o.color ?? "#000000"} onChange={(e) => edit({ color: e.target.value }, "color")} />
            </label>
            <label className="row">
              <input type="checkbox" checked={o.bold} onChange={(e) => edit({ bold: e.target.checked })} /> Bold
            </label>
          </div>
          <p className="hint">Font: Helvetica-style sans serif (the original font is not identified).</p>
        </>
      )}
      {o.kind === "image" && (
        <button type="button" className="btn outline studio-replace" data-testid="studio-replace" disabled={o.locked} title={o.locked ? "Unlock the picture first" : "Replace with a PNG or JPEG picture; keeps its place and size"} onClick={onReplace}>
          <ImageIcon /> Replace picture…
        </button>
      )}
      {o.kind === "shape" && (
        <label className="studio-num">
          <span>Fill</span>
          <input type="color" value={o.fill ?? "#000000"} onChange={(e) => edit({ fill: e.target.value }, "fill")} />
        </label>
      )}
      <div className="studio-grid">
        {num("x", "X")}
        {num("y", "Y")}
        {num("w", "Width")}
        {num("h", "Height")}
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

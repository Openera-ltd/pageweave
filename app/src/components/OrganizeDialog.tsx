import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Copy, FilePlus, FolderOpen, Redo2, RotateCcw, RotateCw, Trash2, Undo2 } from "lucide-react";
import { useApi } from "../api";
import { CSS_PX_PER_PT, displaySizePt, renderScalePermille } from "../geometry";
import { duplicate, initialTiles, insertPages, isUnchanged, moveTo, remove, rotate, toPageRefs, type OrganizeSource, type Tile } from "../organize";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

const THUMB = 104;

function turnText(deg: number): string {
  return deg === 90 ? "90° right" : deg === 270 ? "90° left" : "180°";
}

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/**
 * Reorder, duplicate, rotate, delete and insert pages, then save the result as a new file in one
 * verified write (`organize_pages`). Mouse: click to select, drag to move. Keyboard: arrows select,
 * Ctrl+arrows move, R / Shift+R rotate, Ctrl+D duplicate, Delete removes, Ctrl+Z / Ctrl+Y undo/redo.
 */
export function OrganizeDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [sources, setSources] = useState<OrganizeSource[]>([{ docId: doc.doc_id, path: doc.path, name: doc.name, pages: doc.info.pages }]);
  const [history, setHistory] = useState<{ stack: Tile[][]; at: number }>({ stack: [initialTiles(doc.info.page_count)], at: 0 });
  const [selected, setSelected] = useState(0);
  const [output, setOutput] = useState(siblingOutput(doc.path, "organized"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  /** Preview documents opened for insertion (closed again when the dialog closes). */
  const previews = useRef<number[]>([]);
  const listRef = useRef<HTMLDivElement>(null);

  const tiles = history.stack[history.at]!;
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);
  const sel = Math.min(selected, tiles.length - 1);

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      onFinished(job);
      onClose();
    } else if (job.state === "failed" && job.error) {
      setJobId(null);
      setError(job.error);
      setConflict(job.error.category === "output_conflict");
    } else if (job.state === "cancelled") {
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  useEffect(
    () => () => {
      for (const id of previews.current) api.closeDocument(id).catch(() => undefined);
    },
    [api],
  );

  // Keep the selected tile in view.
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${sel}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [sel, tiles]);

  const apply = (next: Tile[], select?: number) => {
    if (next === tiles) return;
    setHistory((h) => ({ stack: [...h.stack.slice(0, h.at + 1), next], at: h.at + 1 }));
    if (select !== undefined) setSelected(Math.max(0, Math.min(next.length - 1, select)));
    setError(null);
  };
  const undo = () => history.at > 0 && setHistory((h) => ({ ...h, at: h.at - 1 }));
  const redo = () => history.at < history.stack.length - 1 && setHistory((h) => ({ ...h, at: h.at + 1 }));

  const insertFromFile = async () => {
    try {
      const [path] = await api.pickPdfFiles(false);
      if (!path) return;
      const opened = await api.openDocument(path, undefined, false);
      if (!opened.already_open) previews.current.push(opened.doc_id);
      const index = sources.length;
      setSources((s) => [...s, { docId: opened.doc_id, path: opened.path, name: opened.name, pages: opened.info.pages }]);
      apply(insertPages(tiles, sel, index, opened.info.page_count), sel + 1);
    } catch (e) {
      const err = toPwError(e);
      setError(err.category === "password_required" ? { ...err, message: "Password-protected files cannot be inserted yet.", action: "Remove the password in that file first, or choose another file." } : err);
    }
  };

  const unchanged = isUnchanged(tiles, doc.info.page_count) && sources.length === 1;
  const blocker = unchanged ? "No changes yet: move, rotate, duplicate, delete or insert pages first." : !output.trim() ? "Choose where to save the new file." : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const id = await api.startOperation(
        { schema_version: 1, operation: { op: "organize_pages", input: doc.path, extra_inputs: sources.slice(1).map((s) => s.path), pages: toPageRefs(tiles), output }, collision },
        doc.doc_id,
      );
      setJobId(id);
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const actions = useMemo(
    () => ({
      left: { label: "Move earlier", icon: <ArrowLeft />, avail: sel > 0 ? true : "Already first.", run: () => apply(moveTo(tiles, sel, sel - 1), sel - 1) },
      right: { label: "Move later", icon: <ArrowRight />, avail: sel < tiles.length - 1 ? true : "Already last.", run: () => apply(moveTo(tiles, sel, sel + 1), sel + 1) },
      rotl: { label: "Rotate left", icon: <RotateCcw />, avail: true as const, run: () => apply(rotate(tiles, sel, -90)) },
      rotr: { label: "Rotate right", icon: <RotateCw />, avail: true as const, run: () => apply(rotate(tiles, sel, 90)) },
      dup: { label: "Duplicate", icon: <Copy />, avail: true as const, run: () => apply(duplicate(tiles, sel), sel + 1) },
      del: { label: "Remove from the new file", icon: <Trash2 />, avail: tiles.length > 1 ? true : "A PDF must keep at least one page.", run: () => apply(remove(tiles, sel), sel) },
      undo: { label: "Undo", icon: <Undo2 />, avail: history.at > 0 ? true : "Nothing to undo.", run: undo },
      redo: { label: "Redo", icon: <Redo2 />, avail: history.at < history.stack.length - 1 ? true : "Nothing to redo.", run: redo },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tiles, sel, history],
  );
  type ActionKey = keyof typeof actions;
  const run = (k: ActionKey) => {
    if (busy) return;
    const a = actions[k];
    if (a.avail === true) a.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const ctrl = e.ctrlKey || e.metaKey;
    let handled = true;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") ctrl ? run("right") : setSelected(Math.min(tiles.length - 1, sel + 1));
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") ctrl ? run("left") : setSelected(Math.max(0, sel - 1));
    else if (e.key === "Home") setSelected(0);
    else if (e.key === "End") setSelected(tiles.length - 1);
    else if (e.key === "Delete" || e.key === "Backspace") run("del");
    else if (ctrl && (e.key === "d" || e.key === "D")) run("dup");
    else if (!ctrl && (e.key === "r" || e.key === "R")) run(e.shiftKey ? "rotl" : "rotr");
    else if (ctrl && (e.key === "z" || e.key === "Z") && !e.shiftKey) run("undo");
    else if (ctrl && (e.key === "y" || e.key === "Y" || ((e.key === "z" || e.key === "Z") && e.shiftKey))) run("redo");
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const footer = (
    <>
      {busy ? (
        <button type="button" className="btn outline" onClick={() => jobId && api.cancelJob(jobId)} disabled={job?.state === "cancelling"}>
          {job?.state === "cancelling" ? "Cancelling…" : "Cancel task"}
        </button>
      ) : (
        <button type="button" className="btn outline" onClick={onClose}>
          Cancel
        </button>
      )}
      <button type="button" className="btn primary" data-testid="organize-save" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Working…" : "Save"}
      </button>
    </>
  );

  return (
    <Dialog title="Organize pages" onClose={onClose} footer={footer} busy={busy} wide initialFocus=".organize-grid">
      <p className="hint" style={{ marginTop: 0 }}>
        Builds a new file from <strong>{doc.name}</strong>. The original file is not changed.
      </p>
      <div className="row organize-tools" role="toolbar" aria-label="Page actions">
        {(Object.keys(actions) as ActionKey[]).map((k) => {
          const a = actions[k];
          return (
            <button
              key={k}
              type="button"
              className="btn icon"
              data-action={k}
              aria-label={a.label}
              aria-disabled={a.avail === true && !busy ? undefined : true}
              title={a.avail === true ? a.label : `${a.label} — ${a.avail}`}
              onClick={() => run(k)}
            >
              {a.icon}
            </button>
          );
        })}
        <span className="spacer" />
        <button type="button" className="btn outline" data-action="insert" aria-disabled={busy || undefined} onClick={() => !busy && insertFromFile()}>
          <FilePlus /> Insert pages from file…
        </button>
      </div>
      <div
        ref={listRef}
        className="organize-grid"
        role="listbox"
        aria-label={`Pages of the new file, ${tiles.length} in total. Arrow keys select; Ctrl+arrows move; R rotates; Delete removes.`}
        aria-activedescendant={`org-tile-${tiles[sel]!.id}`}
        tabIndex={0}
        onKeyDown={onKeyDown}
      >
        {tiles.map((t, i) => {
          const src = sources[t.source]!;
          const p = src.pages[t.page]!;
          const d = displaySizePt(p);
          const turned = t.rotate === 90 || t.rotate === 270;
          const w = THUMB;
          const h = Math.round((d.h / d.w) * w);
          const scale = renderScalePermille(p, w / (d.w * CSS_PX_PER_PT), window.devicePixelRatio || 1);
          return (
            <div
              key={t.id}
              id={`org-tile-${t.id}`}
              role="option"
              aria-selected={i === sel}
              data-index={i}
              data-page-ref={`${t.source}:${t.page}:${t.rotate}`}
              className={`organize-tile ${i === sel ? "selected" : ""} ${dragFrom !== null && dragFrom !== i ? "drop-target" : ""}`}
              draggable={!busy}
              onClick={() => setSelected(i)}
              onDragStart={(e) => {
                setDragFrom(i);
                setSelected(i);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", String(i));
              }}
              onDragOver={(e) => {
                if (dragFrom !== null) e.preventDefault();
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (dragFrom !== null) apply(moveTo(tiles, dragFrom, i), i);
                setDragFrom(null);
              }}
              onDragEnd={() => setDragFrom(null)}
            >
              <span className="frame" style={{ width: turned ? h : w, height: turned ? w : h }}>
                <img src={api.pageUrl(src.docId, t.page, scale)} alt="" draggable={false} style={{ width: w, height: h, transform: t.rotate ? `rotate(${t.rotate}deg)` : undefined }} />
              </span>
              {/* The visible text is the accessible name; the turn is announced, not shown as text. */}
              <span className="num">
                {i + 1}
                <span className="from">{t.source === 0 ? ` · page ${t.page + 1}` : ` · ${src.name} page ${t.page + 1}`}</span>
                {t.rotate !== 0 && <span className="sr-only">{`, turned ${turnText(t.rotate)}`}</span>}
              </span>
            </div>
          );
        })}
      </div>
      <div className="field" style={{ marginTop: "var(--space-4)" }}>
        <label htmlFor="organize-output">Save as</label>
        <div className="row">
          <input id="organize-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickSavePath(output);
              if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
            }}
          >
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            {!conflict && <div className="hint">{error.action}</div>}
          </div>
          {conflict && (
            <div className="row">
              <button type="button" className="btn outline" onClick={() => start("rename")}>
                Keep both
              </button>
              <button type="button" className="btn danger" onClick={() => start("replace")}>
                Replace
              </button>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}

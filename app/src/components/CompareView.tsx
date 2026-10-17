import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftRight, ChevronDown, ChevronUp, FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { basename } from "../paths";
import { renderScalePermille } from "../geometry";
import { diffItems, KIND_LABEL, pairLabel, rectPercent, summaryText, type CompareReport, type DiffItem, type RectPt } from "../compare";
import { toPwError, type JobSnapshot, type OpenedDoc, type PageInfo, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  /** The active document (the older version unless swapped). */
  doc: OpenedDoc;
  /** Other open tabs that can be compared with it. */
  others: OpenedDoc[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
}

type Target = { kind: "tab"; docId: number } | { kind: "file" };

/** Width the page images are rendered for (CSS px); they scale down to fit their column. */
const PANE_PX = 520;

export function CompareView({ doc, others, jobs, onClose }: Props) {
  const api = useApi();
  const [target, setTarget] = useState<Target>(others[0] ? { kind: "tab", docId: others[0].doc_id } : { kind: "file" });
  const [path, setPath] = useState("");
  const [swapped, setSwapped] = useState(false);
  const [error, setError] = useState<PwError | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [sides, setSides] = useState<{ old: OpenedDoc; new: OpenedDoc } | null>(null);
  const [selected, setSelected] = useState(0);
  /** Documents this view opened itself (closed again when it closes). */
  const opened = useRef<number[]>([]);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(
    () => () => {
      for (const id of opened.current) void api.closeDocument(id).catch(() => undefined);
    },
    [api],
  );

  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);
  const report = job?.state === "succeeded" ? (job.result?.data as unknown as CompareReport | undefined) : undefined;
  const items = useMemo(() => (report ? diffItems(report) : []), [report]);

  useEffect(() => {
    if (job?.state === "failed" && job.error) setError(job.error);
  }, [job?.state, job?.error]);

  // When results arrive the chooser's controls are gone: put focus on the list (keyboard ready,
  // and Escape still reaches the dialog).
  const hasReport = !!report;
  useEffect(() => {
    if (hasReport) (listRef.current ?? document.querySelector<HTMLElement>(".compare .btn"))?.focus();
  }, [hasReport]);

  const other = target.kind === "tab" ? others.find((o) => o.doc_id === target.docId) : undefined;
  const blocker = target.kind === "file" && !path.trim() ? "Choose the file to compare with." : null;

  const start = async () => {
    if (blocker || busy) return;
    setError(null);
    try {
      let second = other;
      if (!second) {
        second = await api.openDocument(path.trim(), undefined, false);
        if (!second.already_open) opened.current.push(second.doc_id);
      }
      const [o, n] = swapped ? [second, doc] : [doc, second];
      setSides({ old: o, new: n });
      setSelected(0);
      const id = await api.startOperation({ schema_version: 1, operation: { op: "compare", old: o.path, new: n.path }, collision: "fail" }, o.doc_id);
      setJobId(id);
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const [p] = await api.pickPdfFiles(false);
    if (p) {
      setPath(p);
      setTarget({ kind: "file" });
    }
  };

  const move = (to: number) => {
    if (items.length === 0) return;
    const i = Math.max(0, Math.min(items.length - 1, to));
    setSelected(i);
    listRef.current?.querySelector<HTMLElement>(`[data-item="${i}"]`)?.scrollIntoView({ block: "nearest" });
  };

  const onListKey = (e: React.KeyboardEvent) => {
    const step: Record<string, number> = { ArrowDown: selected + 1, ArrowUp: selected - 1, Home: 0, End: items.length - 1 };
    if (e.key in step) {
      e.preventDefault();
      move(step[e.key]!);
    }
  };

  // ---- result ----
  if (report && sides) {
    const item: DiffItem | undefined = items[selected];
    const pair = item ? report.pairs[item.pair] : report.pairs.find((p) => p.status !== "same") ?? report.pairs[0];
    const footer = (
      <button type="button" className="btn outline" onClick={onClose}>
        Close
      </button>
    );
    return (
      <Dialog title="Compare documents" onClose={onClose} footer={footer} full>
        <div className="compare">
          <div className="compare-bar">
            <span className="compare-files" title={`${sides.old.path} → ${sides.new.path}`}>
              <strong>{sides.old.name}</strong> → <strong>{sides.new.name}</strong>
            </span>
            <span className="hint" data-testid="compare-summary">
              {summaryText(report.summary)}
            </span>
            <span className="grow" />
            <span className="hint" aria-live="polite">
              {items.length ? `${selected + 1} of ${items.length}` : ""}
            </span>
            <button type="button" className="btn icon" aria-label="Previous difference" title="Previous difference (↑ in the list)" aria-disabled={selected <= 0 || undefined} onClick={() => move(selected - 1)}>
              <ChevronUp />
            </button>
            <button type="button" className="btn icon" aria-label="Next difference" title="Next difference (↓ in the list)" aria-disabled={selected >= items.length - 1 || undefined} onClick={() => move(selected + 1)}>
              <ChevronDown />
            </button>
          </div>
          <div className="compare-main">
            <div className="compare-side">
              {report.metadata.length > 0 && (
                <dl className="compare-meta" aria-label="Document properties that differ">
                  {report.metadata.map((m) => (
                    <div key={m.field}>
                      <dt>{m.field}</dt>
                      <dd>
                        <del>{m.old || "(none)"}</del> → <ins>{m.new || "(none)"}</ins>
                      </dd>
                    </div>
                  ))}
                </dl>
              )}
              {items.length === 0 ? (
                <p className="hint">The pages look and read the same.</p>
              ) : (
                <div ref={listRef} className="compare-list" role="listbox" aria-label="Differences" tabIndex={0} aria-activedescendant={`diff-${selected}`} onKeyDown={onListKey}>
                  {items.map((it, i) => (
                    <div
                      key={it.key}
                      id={`diff-${i}`}
                      data-item={i}
                      role="option"
                      aria-selected={i === selected}
                      className={`compare-item kind-${it.kind}`}
                      onClick={() => setSelected(i)}
                    >
                      <span className="compare-kind">{KIND_LABEL[it.kind]}</span>
                      <span className="compare-where">{it.where}</span>
                      <span className="compare-text">{it.text}</span>
                    </div>
                  ))}
                </div>
              )}
              {report.notes.map((n) => (
                <p key={n} className="hint">
                  {n}
                </p>
              ))}
            </div>
            {pair && (
              <div className="compare-pages" aria-label={pairLabel(pair)}>
                <PagePane
                  label="Old"
                  doc={sides.old}
                  page={pair.old}
                  missing="Not in the old document"
                  boxes={pair.changes.map((c, j) => ({ kind: c.kind, rects: c.old_rects, current: item?.change === j }))}
                />
                <PagePane
                  label="New"
                  doc={sides.new}
                  page={pair.new}
                  missing="Not in the new document"
                  boxes={pair.changes.map((c, j) => ({ kind: c.kind, rects: c.new_rects, current: item?.change === j }))}
                />
              </div>
            )}
          </div>
        </div>
      </Dialog>
    );
  }

  // ---- choose / progress ----
  const progress = job?.progress;
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
      <button type="button" className="btn primary" data-testid="compare-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={start}>
        {busy ? "Comparing…" : "Compare"}
      </button>
    </>
  );
  const otherName = other?.name ?? (path.trim() ? basename(path.trim()) : "the chosen file");
  const [oldName, newName] = swapped ? [otherName, doc.name] : [doc.name, otherName];
  return (
    <Dialog title="Compare documents" onClose={onClose} footer={footer} busy={busy}>
      <fieldset className="field" style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>
        <legend className="label">Compare {doc.name} with</legend>
        {others.map((o) => (
          <label key={o.doc_id} className="row" title={o.path}>
            <input type="radio" name="compare-with" checked={target.kind === "tab" && target.docId === o.doc_id} onChange={() => setTarget({ kind: "tab", docId: o.doc_id })} />
            <span className="truncate">{o.name}</span>
          </label>
        ))}
        <label className="row">
          <input type="radio" name="compare-with" data-testid="compare-file" checked={target.kind === "file"} onChange={() => setTarget({ kind: "file" })} /> Another file
        </label>
        {target.kind === "file" && (
          <div className="row">
            <input id="compare-path" className="input grow" aria-label="File to compare with" value={path} placeholder="Path to a PDF" spellCheck={false} onChange={(e) => setPath(e.target.value)} onKeyDown={(e) => e.key === "Enter" && start()} />
            <button type="button" className="btn outline" onClick={browse}>
              <FolderOpen /> Browse…
            </button>
          </div>
        )}
      </fieldset>
      <div className="field">
        <span className="label">Order</span>
        <div className="row">
          <span className="truncate" data-testid="compare-order">
            Older: <strong>{oldName}</strong> · Newer: <strong>{newName}</strong>
          </span>
          <button type="button" className="btn outline" data-testid="compare-swap" onClick={() => setSwapped((s) => !s)} disabled={busy}>
            <ArrowLeftRight /> Swap
          </button>
        </div>
      </div>
      {busy && progress && (
        <div aria-live="polite">
          <div className="hint">{progress.kind === "determinate" ? `Comparing… ${progress.done} of ${progress.total} steps` : "Comparing…"}</div>
          <div className={`progress ${progress.kind === "indeterminate" ? "indeterminate" : ""}`} role="progressbar" aria-label="Comparison progress" aria-valuemin={0} aria-valuemax={progress.kind === "determinate" ? progress.total : undefined} aria-valuenow={progress.kind === "determinate" ? progress.done : undefined}>
            <div style={progress.kind === "determinate" ? { width: `${(100 * progress.done) / Math.max(1, progress.total)}%` } : undefined} />
          </div>
        </div>
      )}
      {job?.state === "cancelled" && <p className="hint">The comparison was cancelled.</p>}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            <div className="hint">{error.action}</div>
          </div>
        </div>
      )}
    </Dialog>
  );
}

function PagePane({
  label,
  doc,
  page,
  missing,
  boxes,
}: {
  label: string;
  doc: OpenedDoc;
  page: number | null;
  missing: string;
  boxes: Array<{ kind: string; rects: RectPt[]; current: boolean }>;
}) {
  const api = useApi();
  const info: PageInfo | undefined = page != null ? doc.info.pages[page] : undefined;
  const currentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    currentRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });
  return (
    <section className="compare-pane" aria-label={`${label}: ${info ? `page ${(page ?? 0) + 1} of ${doc.name}` : missing}`}>
      <div className="compare-pane-title">
        {label} · {info ? `page ${(page ?? 0) + 1}` : "—"}
      </div>
      {/* Focusable so keyboard users can scroll a page taller than its pane. */}
      <div className="compare-scroll" tabIndex={0} role="region" aria-label={`${label} page`}>
        {info ? (
          <div className="compare-page">
            <img src={api.pageUrl(doc.doc_id, info.index, renderScalePermille(info, PANE_PX / Math.max(info.width_pt, info.height_pt) / (96 / 72), window.devicePixelRatio || 1))} alt="" draggable={false} />
            {boxes.flatMap((b, i) =>
              b.rects.map((r, k) => {
                const p = rectPercent(r, info);
                return (
                  <div
                    key={`${i}-${k}`}
                    ref={b.current && k === 0 ? currentRef : undefined}
                    className={`compare-box kind-${b.kind}${b.current ? " current" : ""}`}
                    style={{ left: `${p.left}%`, top: `${p.top}%`, width: `${p.width}%`, height: `${p.height}%` }}
                    aria-hidden="true"
                  />
                );
              }),
            )}
          </div>
        ) : (
          <p className="hint compare-missing">{missing}</p>
        )}
      </div>
    </section>
  );
}

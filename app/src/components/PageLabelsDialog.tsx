import { useEffect, useMemo, useState } from "react";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { previewLabels, rangesProblem, STYLE_OPTIONS, type LabelRange, type LabelStyle } from "../pageLabels";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

interface Row {
  /** 1-based page as typed. */
  from: string;
  style: LabelStyle;
  prefix: string;
  first: string;
}

const toRanges = (rows: Row[]): LabelRange[] => rows.map((r) => ({ start: Number(r.from) - 1, style: r.style, prefix: r.prefix, first: Number(r.first) }));

/** Set the page numbers readers show (i–iv, then 1, 2, …) in a new file. */
export function PageLabelsDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const n = doc.info.page_count;
  const hasLabels = doc.info.pages.some((p) => p.label);
  const [rows, setRows] = useState<Row[]>([{ from: "1", style: "decimal", prefix: "", first: "1" }]);
  const [output, setOutput] = useState(siblingOutput(doc.path, "labelled"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

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

  const ranges = useMemo(() => toRanges(rows), [rows]);
  const problem = rangesProblem(ranges, n);
  const preview = useMemo(() => (problem ? [] : previewLabels(ranges, n)), [ranges, n, problem]);
  const blocker = problem ?? (output.trim() ? null : "Choose where to save the new file.");

  const update = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const addRow = () =>
    setRows((rs) => {
      const last = Number(rs[rs.length - 1]?.from ?? "1");
      return [...rs, { from: String(Math.min(n, last + 1)), style: "decimal", prefix: "", first: "1" }];
    });

  const start = async (collision: CollisionPolicy, remove = false) => {
    if ((!remove && blocker) || busy || !output.trim()) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "set_page_labels", input: doc.path, ranges: remove ? [] : ranges, output }, collision }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const p = await api.pickSavePath(output);
    if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
  };

  const footer = (
    <>
      {hasLabels && (
        <button type="button" className="btn outline" data-testid="labels-remove" onClick={() => start("fail", true)} aria-disabled={busy || !output.trim() ? true : undefined} title="Save a copy without page labels">
          Remove labels
        </button>
      )}
      <span className="grow" />
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="labels-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Saving…" : "Save"}
      </button>
    </>
  );

  return (
    <Dialog title="Page labels" onClose={onClose} footer={footer} busy={busy} wide>
      <p className="hint" style={{ marginTop: 0 }}>
        The page numbers readers show for {doc.name} ({n} pages). {hasLabels ? "Saving replaces the labels the file has now." : ""}
      </p>
      <table className="labels-table">
        <thead>
          <tr>
            <th scope="col">From page</th>
            <th scope="col">Numbers</th>
            <th scope="col">Prefix</th>
            <th scope="col">Start at</th>
            <th scope="col">
              <span className="sr-only">Remove</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} data-row={i}>
              <td>
                <input className="input" type="number" min={1} max={n} aria-label={`Range ${i + 1}: from page`} value={r.from} disabled={i === 0 || busy} onChange={(e) => update(i, { from: e.target.value })} />
              </td>
              <td>
                <select className="select" aria-label={`Range ${i + 1}: numbers`} value={r.style} disabled={busy} onChange={(e) => update(i, { style: e.target.value as LabelStyle })}>
                  {STYLE_OPTIONS.map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </td>
              <td>
                <input className="input" aria-label={`Range ${i + 1}: prefix`} value={r.prefix} placeholder="e.g. A-" disabled={busy} onChange={(e) => update(i, { prefix: e.target.value })} />
              </td>
              <td>
                <input className="input" type="number" min={1} aria-label={`Range ${i + 1}: start at`} value={r.first} disabled={busy || r.style === "none"} onChange={(e) => update(i, { first: e.target.value })} />
              </td>
              <td>
                {i > 0 && (
                  <button type="button" className="btn icon" aria-label={`Remove range ${i + 1}`} title="Remove range" disabled={busy} onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                    <Trash2 />
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div>
        <button type="button" className="btn outline" data-testid="labels-add" onClick={addRow} aria-disabled={busy || rows.length >= Math.min(n, 1000) ? true : undefined} title={rows.length >= n ? "Every page already starts a range." : undefined}>
          <Plus /> Add range
        </button>
      </div>
      <div className="hint" aria-live="polite" data-testid="labels-preview">
        {problem ? <span className="error">{problem}</span> : preview.map((p) => `Page ${p.page + 1} → ${p.label || "(no number)"}`).join(" · ")}
      </div>
      <div className="field">
        <label htmlFor="labels-output">Save as</label>
        <div className="row">
          <input id="labels-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} disabled={busy} />
          <button type="button" className="btn outline" onClick={browse} disabled={busy}>
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

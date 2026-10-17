import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

/** Sensitivity presets: the largest share of ink a page may have and still count as blank. */
export const BLANK_PRESETS = [
  { id: "empty", label: "Only empty pages", maxInk: 0.0002 },
  { id: "standard", label: "Empty or nearly empty (dust, specks)", maxInk: 0.002 },
  { id: "lenient", label: "Also pages with faint marks", maxInk: 0.005 },
] as const;

export type BlankPreset = (typeof BLANK_PRESETS)[number]["id"];

interface PageInk {
  page: number;
  ink: number;
  blank: boolean;
}

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** "Page 3" → "3", several → "2,5,9" (1-based, for delete_pages). */
export function pageSpec(pages: number[]): string {
  return [...pages].sort((a, b) => a - b).map((p) => p + 1).join(",");
}

export function RemoveBlankDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [preset, setPreset] = useState<BlankPreset>("standard");
  const [scanId, setScanId] = useState<string | null>(null);
  const [found, setFound] = useState<PageInk[] | null>(null);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [output, setOutput] = useState(siblingOutput(doc.path, "no-blank"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);

  // Analyse whenever the sensitivity changes (read-only job with progress).
  useEffect(() => {
    let live = true;
    setFound(null);
    setError(null);
    const maxInk = BLANK_PRESETS.find((p) => p.id === preset)!.maxInk;
    api
      .startOperation({ schema_version: 1, operation: { op: "find_blank_pages", input: doc.path, max_ink: maxInk }, collision: "fail" }, doc.doc_id)
      .then((id) => live && setScanId(id))
      .catch((e) => live && setError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.path, doc.doc_id, preset]);

  const scan = scanId ? jobs[scanId] : undefined;
  useEffect(() => {
    if (scan?.state === "succeeded") {
      const pages = ((scan.result?.data as { pages?: PageInk[] } | undefined)?.pages ?? []).filter((p) => p.blank);
      setFound(pages);
      setChosen(new Set(pages.map((p) => p.page)));
    } else if (scan?.state === "failed" && scan.error) {
      setError(scan.error);
    }
  }, [scan?.state, scan?.result, scan?.error]);

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

  const scanning = found === null && !error;
  const blocker = scanning
    ? "Looking for blank pages…"
    : !found || found.length === 0
      ? "No blank pages found."
      : chosen.size === 0
        ? "Choose at least one page to remove."
        : chosen.size >= doc.info.page_count
          ? "A PDF must keep at least one page."
          : !output.trim()
            ? "Choose where to save the new file."
            : null;

  const remove = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const id = await api.startOperation({ schema_version: 1, operation: { op: "delete_pages", input: doc.path, pages: pageSpec([...chosen]), output }, collision }, doc.doc_id);
      setJobId(id);
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const toggle = (p: number) =>
    setChosen((s) => {
      const n = new Set(s);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });

  const thumbScale = (page: number) => {
    const info = doc.info.pages[page];
    const longest = info ? Math.max(info.width_pt, info.height_pt) : 792;
    return Math.max(50, Math.round((96 / longest) * 1000 * (window.devicePixelRatio || 1)));
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="blank-remove" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => remove("fail")}>
        {busy ? "Removing…" : chosen.size > 0 ? `Remove ${chosen.size} page${chosen.size === 1 ? "" : "s"}` : "Remove"}
      </button>
    </>
  );

  return (
    <Dialog title="Remove blank pages" onClose={onClose} footer={footer} busy={busy} wide>
      <div className="field">
        <label htmlFor="blank-preset">Count as blank</label>
        <select id="blank-preset" className="select" value={preset} disabled={busy} onChange={(e) => setPreset(e.target.value as BlankPreset)}>
          {BLANK_PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        <span className="hint">Pages with any text or comments are never counted as blank.</span>
      </div>
      <div className="field" aria-live="polite" aria-busy={scanning}>
        {scanning ? (
          <span className="hint">
            Looking for blank pages
            {scan?.progress.kind === "determinate" ? ` (${scan.progress.done} of ${scan.progress.total})` : ""}…
          </span>
        ) : found && found.length === 0 ? (
          <span className="hint" data-testid="blank-none">
            No blank pages found.
          </span>
        ) : found ? (
          <ul className="blank-grid" aria-label="Blank pages found">
            {found.map((p) => (
              <li key={p.page}>
                <label>
                  <img src={api.pageUrl(doc.doc_id, p.page, thumbScale(p.page))} alt="" draggable={false} />
                  <span className="row">
                    <input type="checkbox" checked={chosen.has(p.page)} onChange={() => toggle(p.page)} data-blank-page={p.page} /> Page {p.page + 1}
                  </span>
                  <span className="hint">{p.ink === 0 ? "empty" : `${(p.ink * 100).toFixed(2)}% marks`}</span>
                </label>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      <div className="field">
        <label htmlFor="blank-output">Save as</label>
        <div className="row">
          <input id="blank-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
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
              <button type="button" className="btn outline" onClick={() => remove("rename")}>
                Keep both
              </button>
              <button type="button" className="btn danger" onClick={() => remove("replace")}>
                Replace
              </button>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}

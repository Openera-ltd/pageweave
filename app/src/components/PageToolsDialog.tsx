import { useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type Operation, type PwError } from "../types";
import { Dialog, useDebounced } from "./ui";

export type PageTool = "extract" | "delete" | "rotate" | "split" | "merge";

const TITLES: Record<PageTool, string> = {
  extract: "Extract pages",
  delete: "Delete pages",
  rotate: "Rotate pages",
  split: "Split document",
  merge: "Combine files",
};

const SUFFIX: Record<PageTool, string> = {
  extract: "extract",
  delete: "edited",
  rotate: "rotated",
  split: "",
  merge: "combined",
};

interface Props {
  tool: PageTool;
  doc: OpenedDoc;
  initialPages?: string;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

type RangeCheck = { state: "idle" | "checking" } | { state: "ok"; count: number } | { state: "error"; message: string };

function useRangeCheck(docId: number, spec: string, enabled: boolean): RangeCheck {
  const api = useApi();
  const debounced = useDebounced(spec, 200);
  const [check, setCheck] = useState<RangeCheck>({ state: "idle" });
  useEffect(() => {
    if (!enabled) return setCheck({ state: "idle" });
    if (!debounced.trim()) return setCheck({ state: "error", message: "Enter at least one page, for example 1-3, 5." });
    let live = true;
    setCheck({ state: "checking" });
    api
      .validateRange(docId, debounced)
      .then((pages) => live && setCheck({ state: "ok", count: pages.length }))
      .catch((e) => live && setCheck({ state: "error", message: toPwError(e).message }));
    return () => {
      live = false;
    };
  }, [api, docId, debounced, enabled]);
  // Treat the undebounced value as "checking" so Start can't run on a stale validation.
  if (enabled && spec !== debounced) return { state: "checking" };
  return check;
}

export function PageToolsDialog({ tool, doc, initialPages, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [pages, setPages] = useState(initialPages ?? (tool === "rotate" ? "" : ""));
  const [allPages, setAllPages] = useState(tool === "rotate" && !initialPages);
  const [degrees, setDegrees] = useState(90);
  const [output, setOutput] = useState(tool === "split" ? dirname(doc.path) : siblingOutput(doc.path, SUFFIX[tool]));
  const [splitMode, setSplitMode] = useState<"every_n" | "ranges">("every_n");
  const [everyN, setEveryN] = useState("1");
  const [ranges, setRanges] = useState("");
  const [files, setFiles] = useState<string[]>([doc.path]);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);

  const needsRange = tool === "extract" || tool === "delete" || (tool === "rotate" && !allPages);
  const range = useRangeCheck(doc.doc_id, pages, needsRange);
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
      setError(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const everyNValue = Number(everyN);
  const everyNValid = Number.isInteger(everyNValue) && everyNValue >= 1 && everyNValue <= doc.info.page_count;
  const rangeLines = useMemo(() => ranges.split(/\r?\n/).map((s) => s.trim()).filter(Boolean), [ranges]);

  const blocker: string | null = (() => {
    if (needsRange && range.state !== "ok") return range.state === "error" ? range.message : "Checking pages…";
    if (tool === "delete" && range.state === "ok" && range.count >= doc.info.page_count) return "A PDF must keep at least one page.";
    if (tool === "split" && splitMode === "every_n" && !everyNValid) return `Enter a number from 1 to ${doc.info.page_count}.`;
    if (tool === "split" && splitMode === "ranges" && rangeLines.length === 0) return "Enter one page range per line.";
    if (tool === "merge" && files.length < 2) return "Add at least one more file.";
    if (!output.trim()) return tool === "split" ? "Choose an output folder." : "Choose where to save the new file.";
    return null;
  })();

  const buildOperation = (): Operation => {
    switch (tool) {
      case "extract":
        return { op: "extract_pages", input: doc.path, pages, output };
      case "delete":
        return { op: "delete_pages", input: doc.path, pages, output };
      case "rotate":
        return { op: "rotate_pages", input: doc.path, pages: allPages ? null : pages, degrees, output };
      case "split":
        return {
          op: "split",
          input: doc.path,
          split: splitMode === "every_n" ? { mode: "every_n", n: everyNValue } : { mode: "ranges", ranges: rangeLines },
          output_dir: output,
        };
      case "merge":
        return { op: "merge", inputs: files, output };
    }
  };

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const id = await api.startOperation({ schema_version: 1, operation: buildOperation(), collision }, doc.doc_id);
      setJobId(id);
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    if (tool === "split") {
      const dir = await api.pickFolder(output || dirname(doc.path));
      if (dir) setOutput(dir);
    } else {
      const p = await api.pickSavePath(output);
      if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
    }
  };

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
      <button
        type="button"
        className="btn primary"
        data-testid="tool-start"
        aria-disabled={blocker || busy ? true : undefined}
        title={blocker ?? undefined}
        onClick={() => start("fail")}
      >
        {busy ? "Working…" : TITLES[tool].split(" ")[0]}
      </button>
    </>
  );

  return (
    <Dialog title={TITLES[tool]} onClose={onClose} footer={footer} busy={busy} wide={tool === "merge"}>
      {tool !== "merge" && (
        <p className="hint" style={{ marginTop: 0 }}>
          From <strong>{doc.name}</strong> ({doc.info.page_count} pages). The original file is not changed.
        </p>
      )}

      {tool === "rotate" && (
        <div className="field">
          <span className="label" id="rot-label">Direction</span>
          <div className="segmented" role="radiogroup" aria-labelledby="rot-label">
            {[
              [90, "90° right"],
              [180, "180°"],
              [-90, "90° left"],
            ].map(([d, l]) => (
              <button key={d} type="button" role="radio" aria-checked={degrees === d} onClick={() => setDegrees(d as number)}>
                {l}
              </button>
            ))}
          </div>
        </div>
      )}

      {tool === "rotate" && (
        <div className="field">
          <label className="row">
            <input type="checkbox" checked={allPages} onChange={(e) => setAllPages(e.target.checked)} /> All pages
          </label>
        </div>
      )}

      {needsRange && (
        <div className="field">
          <label htmlFor="pages">Pages</label>
          <input
            id="pages"
            className="input"
            value={pages}
            placeholder="e.g. 1-3, 5, 8-"
            autoComplete="off"
            aria-invalid={range.state === "error"}
            aria-describedby="pages-msg"
            onChange={(e) => setPages(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && start("fail")}
          />
          <span id="pages-msg" className={range.state === "error" ? "error" : "hint"} aria-live="polite">
            {range.state === "error" ? range.message : range.state === "ok" ? `${range.count} page${range.count === 1 ? "" : "s"} selected` : "Use commas and ranges; “8-” means 8 to the end."}
          </span>
        </div>
      )}

      {tool === "split" && (
        <>
          <div className="field">
            <span className="label" id="split-label">Split</span>
            <div className="segmented" role="radiogroup" aria-labelledby="split-label">
              <button type="button" role="radio" aria-checked={splitMode === "every_n"} onClick={() => setSplitMode("every_n")}>
                Every N pages
              </button>
              <button type="button" role="radio" aria-checked={splitMode === "ranges"} onClick={() => setSplitMode("ranges")}>
                By ranges
              </button>
            </div>
          </div>
          {splitMode === "every_n" ? (
            <div className="field">
              <label htmlFor="every">Pages per file</label>
              <input id="every" className="input" type="number" min={1} max={doc.info.page_count} value={everyN} aria-invalid={!everyNValid} onChange={(e) => setEveryN(e.target.value)} />
              <span className="hint">{everyNValid ? `Creates ${Math.ceil(doc.info.page_count / everyNValue)} files.` : ""}</span>
            </div>
          ) : (
            <div className="field">
              <label htmlFor="ranges">Ranges, one file per line</label>
              <textarea id="ranges" className="input" style={{ height: 96, paddingTop: 6 }} value={ranges} placeholder={"1-3\n4-"} onChange={(e) => setRanges(e.target.value)} />
            </div>
          )}
        </>
      )}

      {tool === "merge" && (
        <div className="field">
          <span className="label">Files, in order</span>
          <ul className="file-list" aria-label="Files to combine">
            {files.map((f, i) => (
              <li key={f + i}>
                <span className="name" title={f}>
                  {i + 1}. {basename(f)}
                </span>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} up`} title="Move up" aria-disabled={i === 0 || undefined} onClick={() => i > 0 && setFiles((fs) => swap(fs, i, i - 1))}>
                  <ArrowUp />
                </button>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} down`} title="Move down" aria-disabled={i === files.length - 1 || undefined} onClick={() => i < files.length - 1 && setFiles((fs) => swap(fs, i, i + 1))}>
                  <ArrowDown />
                </button>
                <button type="button" className="btn icon" aria-label={`Remove ${basename(f)} from the list`} title="Remove from list (the file is not deleted)" onClick={() => setFiles((fs) => fs.filter((_, j) => j !== i))}>
                  <Trash2 />
                </button>
              </li>
            ))}
          </ul>
          <div>
            <button
              type="button"
              className="btn outline"
              onClick={async () => {
                const picked = await api.pickPdfFiles(true);
                if (picked.length) setFiles((fs) => [...fs, ...picked]);
              }}
            >
              <Plus /> Add files…
            </button>
          </div>
        </div>
      )}

      <div className="field">
        <label htmlFor="output">{tool === "split" ? "Save parts in folder" : "Save as"}</label>
        <div className="row">
          <input id="output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
        {tool === "split" && <span className="hint">Files are named “{doc.name.replace(/\.pdf$/i, "")}_part01.pdf”, …</span>}
      </div>

      {busy && progress && (
        <div aria-live="polite">
          <div className="hint">{job?.state === "cancelling" ? "Cancelling…" : progress.kind === "determinate" ? `${progress.done} of ${progress.total} ${progress.unit}` : "Working…"}</div>
          <div className={`progress ${progress.kind === "indeterminate" ? "indeterminate" : ""}`} role="progressbar" aria-label="Task progress" aria-valuemin={0} aria-valuemax={progress.kind === "determinate" ? progress.total : undefined} aria-valuenow={progress.kind === "determinate" ? progress.done : undefined}>
            <div style={progress.kind === "determinate" ? { width: `${(100 * progress.done) / Math.max(1, progress.total)}%` } : undefined} />
          </div>
        </div>
      )}

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

function swap<T>(a: T[], i: number, j: number): T[] {
  const b = a.slice();
  [b[i], b[j]] = [b[j]!, b[i]!];
  return b;
}

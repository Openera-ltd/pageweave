import { useEffect, useMemo, useState } from "react";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { addFiles, BATCH_ACTIONS, batchOperation, batchOutputs, MAX_BATCH_FILES, summary, translateBlocker, type BatchAction, type ItemState } from "../batch";
import { languageName, languages } from "../translate";
import { basename, dirname } from "../paths";
import { toPwError, type JobSnapshot, type TranslationDirection } from "../types";
import { Dialog } from "./ui";

interface Props {
  /** Paths of the open documents, offered with "Add open files". */
  openFiles: string[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
}

interface Item {
  input: string;
  output: string;
  jobId: string | null;
  /** Set when the job could not even be started. */
  startError: string | null;
}

function stateOf(item: Item, jobs: Record<string, JobSnapshot>): ItemState {
  if (item.startError) return "failed";
  if (!item.jobId) return "waiting";
  const j = jobs[item.jobId];
  if (!j || j.state === "queued") return "waiting";
  if (j.state === "succeeded") return "done";
  if (j.state === "failed") return "failed";
  if (j.state === "cancelled") return "cancelled";
  return "running";
}

/** Run one action over many PDFs; every file goes to a new file in the chosen folder. */
export function BatchDialog({ openFiles, jobs, onClose }: Props) {
  const api = useApi();
  const [files, setFiles] = useState<string[]>([]);
  const [action, setAction] = useState<BatchAction>("optimize");
  const [folder, setFolder] = useState("");
  const [items, setItems] = useState<Item[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [target, setTarget] = useState("en");
  const [dirs, setDirs] = useState<TranslationDirection[] | null>(null);
  useEffect(() => {
    if (action !== "translate" || dirs) return;
    api.translationDirections().then(setDirs, (e) => setError(toPwError(e).message));
  }, [action, dirs, api]);

  const states = useMemo(() => (items ? items.map((i) => stateOf(i, jobs)) : []), [items, jobs]);
  const running = states.some((s) => s === "waiting" || s === "running");
  const outFolder = folder.trim() || (files[0] ? dirname(files[0]) : "");

  const blocker =
    files.length === 0 ? "Add PDF files first." : !outFolder ? "Choose an output folder." : action === "translate" ? translateBlocker(dirs, target) : null;

  const start = async () => {
    if (blocker || running) return;
    setError(null);
    const outputs = batchOutputs(files, action, outFolder, target);
    const list: Item[] = files.map((input, i) => ({ input, output: outputs[i]!, jobId: null, startError: null }));
    setItems(list);
    // Submitted in order; the host runs a few at a time and queues the rest.
    for (let i = 0; i < list.length; i++) {
      try {
        const id = await api.startOperation({ schema_version: 1, operation: batchOperation(action, list[i]!.input, list[i]!.output, target), collision: "rename" });
        list[i] = { ...list[i]!, jobId: id };
      } catch (e) {
        list[i] = { ...list[i]!, startError: toPwError(e).message };
      }
      setItems([...list]);
    }
  };

  const stop = () => {
    for (const [i, it] of (items ?? []).entries()) {
      const s = states[i];
      if (it.jobId && (s === "waiting" || s === "running")) api.cancelJob(it.jobId).catch(() => undefined);
    }
  };

  const pick = async () => {
    const picked = await api.pickPdfFiles(true);
    if (picked.length) setFiles((f) => addFiles(f, picked));
  };
  const pickFolder = async () => {
    const p = await api.pickFolder(outFolder || undefined);
    if (p) setFolder(p);
  };

  const resultText = (it: Item, s: ItemState): string => {
    if (it.startError) return it.startError;
    const j = it.jobId ? jobs[it.jobId] : undefined;
    if (s === "done") return `Saved ${basename(j?.result?.outputs[0]?.path ?? it.output)}`;
    if (s === "failed") return j?.error?.message ?? "Failed.";
    if (s === "cancelled") return "Not processed (stopped).";
    if (s === "running") return j?.progress.kind === "determinate" ? `Working… ${j.progress.done} of ${j.progress.total} ${j.progress.unit}` : "Working…";
    return "Waiting…";
  };

  const footer = items ? (
    <>
      {running ? (
        <button type="button" className="btn outline" data-testid="batch-stop" onClick={stop}>
          Stop the rest
        </button>
      ) : (
        <button type="button" className="btn outline" onClick={() => api.revealInFolder(items[0]!.output).catch((e) => setError(toPwError(e).message))}>
          Show folder
        </button>
      )}
      <button type="button" className="btn primary" data-testid="batch-close" onClick={onClose} aria-disabled={running ? true : undefined} title={running ? "Stop the rest first, or wait until the batch finishes." : undefined}>
        Close
      </button>
    </>
  ) : (
    <>
      <button type="button" className="btn outline" onClick={onClose}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="batch-start" aria-disabled={blocker ? true : undefined} title={blocker ?? undefined} onClick={start}>
        {files.length ? `Process ${files.length} file${files.length === 1 ? "" : "s"}` : "Process"}
      </button>
    </>
  );

  return (
    <Dialog title="Process several files" onClose={() => !running && onClose()} footer={footer} wide busy={running}>
      {!items ? (
        <>
          <div className="field">
            <label htmlFor="batch-action">Action</label>
            <select id="batch-action" className="select" value={action} onChange={(e) => setAction(e.target.value as BatchAction)}>
              {BATCH_ACTIONS.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label}
                </option>
              ))}
            </select>
          </div>
          {action === "translate" && (
            <div className="field">
              <label htmlFor="batch-target">Translate into</label>
              <select id="batch-target" className="select" value={target} onChange={(e) => setTarget(e.target.value)} disabled={!dirs}>
                {(dirs ? languages(dirs) : ["en"]).map((l) => (
                  <option key={l} value={l}>
                    {languageName(l)}
                  </option>
                ))}
              </select>
              <span className="hint">Each file's language is recognised. Machine translation: review the results.</span>
            </div>
          )}
          <div className="field">
            <span className="label">Files ({files.length})</span>
            {files.length > 0 && (
              <ul className="file-list" aria-label="Files to process">
                {files.map((f) => (
                  <li key={f}>
                    <span className="name" title={f}>
                      {basename(f)}
                    </span>
                    <button type="button" className="btn icon" aria-label={`Remove ${basename(f)} from the list`} title="Remove from list (the file is not deleted)" onClick={() => setFiles((fs) => fs.filter((x) => x !== f))}>
                      <Trash2 />
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <div className="row">
              <button type="button" className="btn outline" data-testid="batch-add" onClick={pick} aria-disabled={files.length >= MAX_BATCH_FILES ? true : undefined}>
                <Plus /> Add files…
              </button>
              <button
                type="button"
                className="btn outline"
                data-testid="batch-add-open"
                aria-disabled={openFiles.length === 0 ? true : undefined}
                title={openFiles.length === 0 ? "No documents are open." : undefined}
                onClick={() => openFiles.length && setFiles((f) => addFiles(f, openFiles))}
              >
                Add open files
              </button>
            </div>
          </div>
          <div className="field">
            <label htmlFor="batch-folder">Save results in folder</label>
            <div className="row">
              <input id="batch-folder" className="input grow" value={folder} placeholder={files[0] ? dirname(files[0]) : "Folder of the first file"} onChange={(e) => setFolder(e.target.value)} spellCheck={false} />
              <button type="button" className="btn outline" onClick={pickFolder}>
                <FolderOpen /> Browse…
              </button>
            </div>
            <span className="hint">Originals are not changed. When a result file already exists, both are kept.</span>
          </div>
        </>
      ) : (
        <>
          <div className="hint" aria-live="polite" data-testid="batch-summary">
            {summary(states)}
          </div>
          <ul className="formdata-list" aria-label="Results" data-testid="batch-results">
            {items.map((it, i) => (
              <li key={it.input} className="row" data-state={states[i]}>
                <span className="formdata-name" title={it.input}>
                  {basename(it.input)}
                </span>
                <span className={`formdata-value grow ${states[i] === "failed" ? "error" : ""}`} title={resultText(it, states[i]!)}>
                  {resultText(it, states[i]!)}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">{error}</div>
        </div>
      )}
    </Dialog>
  );
}

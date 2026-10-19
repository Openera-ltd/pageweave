import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { toPwError, type AreaPreview, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError, type RedactArea } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  areas: RedactArea[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** One line describing what an area removes, e.g. “SECRET” · 1 image · 2 drawings. */
export function describeArea(p: AreaPreview): string {
  const parts: string[] = [];
  if (p.text) parts.push(`“${p.text.length > 80 ? `${p.text.slice(0, 80)}…` : p.text}”`);
  const n = (k: number, one: string, many: string) => k > 0 && parts.push(`${k} ${k === 1 ? one : many}`);
  n(p.images, "image", "images");
  n(p.graphics - p.graphics_kept, "drawing", "drawings");
  n(p.annotations, "comment or link", "comments or links");
  return parts.length ? parts.join(" · ") : "Nothing found under this area";
}

/**
 * Preview and apply redaction marks: shows what each marked area removes (from the engine), then
 * writes a new, verified copy. Applying is explicit and never changes the original file.
 */
export function RedactDialog({ doc, areas, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [preview, setPreview] = useState<AreaPreview[] | null>(null);
  const [previewError, setPreviewError] = useState<PwError | null>(null);
  const [output, setOutput] = useState(siblingOutput(doc.path, "redacted"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  useEffect(() => {
    let live = true;
    api
      .redactionPreview(doc.doc_id, areas)
      .then((p) => live && setPreview(p))
      .catch((e) => live && setPreviewError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.doc_id, areas]);

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

  // Per area, so one background crossing several areas is not counted as several drawings.
  const kept = preview?.filter((p) => p.graphics_kept > 0).length ?? 0;
  const blocker = previewError
    ? "The marked areas could not be checked."
    : !preview
      ? "Checking what will be removed…"
      : !output.trim()
        ? "Choose where to save the redacted copy."
        : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "redact", input: doc.path, areas, output }, collision }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const p = await api.pickSavePath(output);
    if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
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
        className="btn danger"
        data-testid="redact-apply"
        aria-disabled={blocker || busy ? true : undefined}
        title={blocker ?? undefined}
        onClick={() => start("fail")}
      >
        {busy ? "Redacting…" : "Apply redactions"}
      </button>
    </>
  );

  return (
    <Dialog title="Apply redactions" onClose={onClose} footer={footer} busy={busy} initialFocus='[data-testid="redact-output"]'>
      <p className="hint" style={{ marginTop: 0 }}>
        Permanently removes the content under {areas.length} marked area{areas.length === 1 ? "" : "s"} from a new copy of <strong>{doc.name}</strong>, then
        checks the copy. The original file is not changed.
      </p>

      <div className="field">
        <span className="label" id="redact-list-label">
          What will be removed
        </span>
        {previewError ? (
          <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
            <div className="grow">
              <div>{previewError.message}</div>
              <div className="hint">{previewError.action}</div>
            </div>
          </div>
        ) : !preview ? (
          <span className="hint" aria-live="polite">
            Checking what will be removed…
          </span>
        ) : (
          <ul className="file-list redact-list" aria-labelledby="redact-list-label" data-testid="redact-preview">
            {preview.map((p, i) => (
              <li key={i}>
                <span className="name">
                  Page {p.page + 1}: {describeArea(p)}
                </span>
              </li>
            ))}
          </ul>
        )}
        {kept > 0 && (
          <span className="hint">
            {kept === 1 ? "1 marked area only partly covers" : `${kept} marked areas only partly cover`} a drawing (such as a background): it is covered in black there, but kept in the file.
          </span>
        )}
      </div>

      <p className="hint">
        Bookmarks, document properties, attachments and fillable form fields are not carried over. A page whose content cannot be removed precisely is
        replaced by an image of the page.
      </p>

      <div className="field">
        <label htmlFor="redact-output">Save redacted copy as</label>
        <div className="row">
          <input id="redact-output" data-testid="redact-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>

      {busy && progress && (
        <div aria-live="polite">
          <div className="hint">{job?.state === "cancelling" ? "Cancelling…" : "Removing content and checking the copy…"}</div>
          <div className="progress indeterminate" role="progressbar" aria-label="Redaction progress">
            <div />
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

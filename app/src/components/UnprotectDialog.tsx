import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Save a copy without password protection; the host supplies the document's password. */
export function UnprotectDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [output, setOutput] = useState(siblingOutput(doc.path, "unprotected"));
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

  const blocker = output.trim() ? null : "Choose where to save the copy.";

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "unprotect", input: doc.path, output }, collision }, doc.doc_id));
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
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="unprotect-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Removing…" : "Remove password"}
      </button>
    </>
  );

  return (
    <Dialog title="Remove password" onClose={onClose} footer={footer} busy={busy} initialFocus="#unprotect-output">
      <p className="hint" style={{ marginTop: 0 }}>
        Saves a copy of <strong>{doc.name}</strong> that opens without a password. The original file stays protected.
      </p>
      <div className="field">
        <label htmlFor="unprotect-output">Save copy as</label>
        <div className="row">
          <input id="unprotect-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} onKeyDown={(e) => e.key === "Enter" && start("fail")} />
          <button type="button" className="btn outline" onClick={browse}>
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

import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { dirname, join } from "../paths";
import { toPwError, type AttachmentInfo, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { formatBytes } from "../format";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  attachment: AttachmentInfo;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Save one embedded file as it is stored; nothing is opened or run. */
export function SaveAttachmentDialog({ doc, attachment, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [output, setOutput] = useState(join(dirname(doc.path), attachment.name));
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

  const blocker = output.trim() ? null : "Choose where to save the file.";
  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "extract_attachment", input: doc.path, index: attachment.index, output }, collision }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="attachment-save" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Saving…" : "Save"}
      </button>
    </>
  );

  return (
    <Dialog title="Save attached file" onClose={onClose} footer={footer} busy={busy} initialFocus="#attachment-output">
      <p className="hint" style={{ marginTop: 0 }}>
        <strong>{attachment.name}</strong> ({formatBytes(attachment.bytes)}) from {doc.name}. It is saved exactly as stored; only open attached files you trust.
      </p>
      <div className="field">
        <label htmlFor="attachment-output">Save as</label>
        <div className="row">
          <input id="attachment-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} onKeyDown={(e) => e.key === "Enter" && start("fail")} />
          <button
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickSaveAnyPath(output);
              if (p) setOutput(p);
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

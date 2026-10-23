import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, join, stem } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type PwError } from "../types";
import { Dialog } from "./ui";

/** Office documents PageWeave opens by converting them to PDF (with the installed LibreOffice). */
const OFFICE_EXT = /\.(docx|doc|odt|rtf|txt|xlsx|xls|ods|csv|pptx|ppt|odp|html|htm)$/i;

export function isOfficePath(p: string): boolean {
  return OFFICE_EXT.test(p);
}

interface Props {
  /** The Office file, or null to pick one first. */
  initialFile: string | null;
  /** The LibreOffice version, or null when it is not installed (the dialog says so). */
  engine: string | null;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  /** Called with the finished job; the caller opens the PDF. */
  onFinished: (job: JobSnapshot) => void;
}

/** Open a Word, Excel or PowerPoint file: convert it to a PDF next to it, then open that. */
export function OfficeDialog({ initialFile, engine, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [file, setFile] = useState<string | null>(initialFile);
  const [output, setOutput] = useState(initialFile ? join(dirname(initialFile), `${stem(initialFile)}.pdf`) : "");
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

  const choose = async () => {
    const p = await api.pickOfficeFile();
    if (p) {
      setFile(p);
      setOutput(join(dirname(p), `${stem(p)}.pdf`));
    }
  };

  const blocker = !engine
    ? "LibreOffice is not installed on this computer."
    : !file
      ? "Choose a file first."
      : !isOfficePath(file)
        ? "Choose a Word, Excel, PowerPoint, OpenDocument, RTF, text, CSV or HTML file."
        : !output.trim()
          ? "Choose where to save the PDF."
          : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy || !file) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "convert_office_to_pdf", input: file, output }, collision }));
    } catch (e) {
      setError(toPwError(e));
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
      <button type="button" className="btn primary" data-testid="office-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Converting…" : "Convert and open"}
      </button>
    </>
  );

  return (
    <Dialog title="Open an Office file" onClose={onClose} footer={footer} busy={busy}>
      {engine ? (
        <p className="hint" style={{ marginTop: 0 }}>
          The file is converted to a PDF with {engine} on this computer, then opened. Macros in the file never run, and the file itself is not changed.
        </p>
      ) : (
        <p className="error" role="alert" data-testid="office-missing">
          Opening Word, Excel and PowerPoint files needs LibreOffice (free, from libreoffice.org). Install it, then try again. Everything else in PageWeave works without it.
        </p>
      )}
      <div className="field">
        <span className="label">File</span>
        <div className="row">
          <span className="grow" data-testid="office-file" title={file ?? undefined}>
            {file ? basename(file) : <span className="hint">No file chosen</span>}
          </span>
          <button type="button" className="btn outline" onClick={choose} disabled={busy}>
            <FolderOpen /> Choose…
          </button>
        </div>
      </div>
      <div className="field">
        <label htmlFor="office-output">Save the PDF as</label>
        <input id="office-output" className="input" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} disabled={busy} />
      </div>
      {busy && (
        <div aria-live="polite">
          <div className="hint">{job?.state === "cancelling" ? "Cancelling…" : "Converting…"}</div>
          <div className="progress indeterminate" role="progressbar" aria-label="Task progress">
            <div />
          </div>
        </div>
      )}
      {error && (
        <div className="error" role="alert">
          {error.message}
          {error.action ? ` ${error.action}` : ""}
          {conflict && (
            <div className="row" style={{ marginTop: "var(--space-2)" }}>
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

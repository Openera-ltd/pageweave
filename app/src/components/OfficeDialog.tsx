import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, join, stem } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OfficePackage, type PwError } from "../types";
import { Dialog } from "./ui";

import { isOfficePath } from "../office";
import { megabytes } from "../translate";
export { isOfficePath };

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
export function OfficeDialog({ initialFile, engine: initialEngine, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  // Re-read after LibreOffice is downloaded from this dialog.
  const [engine, setEngine] = useState<string | null>(initialEngine);
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
        <>
          <p className="error" role="alert" data-testid="office-missing">
            Opening Word, Excel and PowerPoint files needs LibreOffice (free). Everything else in PageWeave works without it.
          </p>
          <OfficeDownload jobs={jobs} onDone={() => api.officeEngine().then(setEngine, () => undefined)} />
        </>
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

/** One-time download of LibreOffice into PageWeave's data folder (Windows): size, progress in
 * megabytes, cancel, and a clear error with Try again. Elsewhere it says how to install it. */
export function OfficeDownload({ jobs, onDone }: { jobs?: Record<string, JobSnapshot>; onDone: () => void }) {
  const api = useApi();
  const [pkg, setPkg] = useState<OfficePackage | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Without the app's job list (Settings), follow the job here.
  const [own, setOwn] = useState<JobSnapshot | null>(null);
  useEffect(() => {
    if (jobs || !jobId) return;
    let off: (() => void) | undefined;
    let gone = false;
    void api
      .onJobUpdate((j) => {
        if (j.id === jobId) setOwn(j);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [api, jobs, jobId]);
  const job = jobId ? (jobs ? jobs[jobId] : own ?? undefined) : undefined;
  useEffect(() => {
    api.officePackage().then(setPkg, () => setPkg(null));
  }, [api]);
  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      setJobId(null);
      onDone();
    } else if (job.state === "failed") {
      setError(job.error ? `${job.error.message} ${job.error.action}`.trim() : "The download failed.");
      setJobId(null);
    } else if (job.state === "cancelled") {
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);
  if (!pkg) return null;
  if (!pkg.can_install) {
    return <p className="hint">Install LibreOffice with your system's package manager, then open the file again.</p>;
  }
  const p = job?.progress;
  const busy = jobId !== null;
  return (
    <div className="download-panel" data-testid="office-download" role="group" aria-label="LibreOffice download">
      <span className="hint">
        {busy
          ? p && p.kind === "determinate" && p.done < p.total
            ? `Downloading LibreOffice ${pkg.version}: ${megabytes(p.done)} of ${megabytes(p.total)}`
            : "Unpacking LibreOffice…"
          : `PageWeave can download LibreOffice ${pkg.version} once (${megabytes(pkg.bytes)}, checked against its published checksum) and keep it in its own folder.`}
      </span>
      {busy && p && p.kind === "determinate" && <progress max={p.total} value={p.done} aria-label="Download progress" />}
      {error && (
        <span className="hint error" role="alert">
          {error}
        </span>
      )}
      {busy ? (
        <button type="button" className="btn outline" data-testid="office-download-cancel" disabled={job?.state === "cancelling"} onClick={() => jobId && void api.cancelJob(jobId)}>
          {job?.state === "cancelling" ? "Cancelling…" : "Cancel download"}
        </button>
      ) : (
        <button
          type="button"
          className="btn"
          data-testid="office-download-start"
          onClick={async () => {
            setError(null);
            try {
              setJobId(await api.startOperation({ schema_version: 1, operation: { op: "install_office" }, collision: "fail" }));
            } catch (e) {
              setError(toPwError(e).message);
            }
          }}
        >
          {error ? "Try again" : `Download LibreOffice (${megabytes(pkg.bytes)})`}
        </button>
      )}
    </div>
  );
}

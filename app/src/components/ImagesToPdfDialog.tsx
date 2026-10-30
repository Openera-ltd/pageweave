import { useEffect, useState } from "react";
import { ArrowDown, ArrowUp, FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, join } from "../paths";
import { toPwError, type CollisionPolicy, type ImagePageSize, type JobSnapshot, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  initialFiles: string[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

const IMAGE_EXT = /\.(jpe?g|png)$/i;

/** True for files this dialog takes (JPEG and PNG, by name). */
export function isImagePath(p: string): boolean {
  return IMAGE_EXT.test(p);
}

/** Default output: "images.pdf" next to the first image. */
export function defaultImagesOutput(files: string[]): string {
  return files.length ? join(dirname(files[0]!), "images.pdf") : "";
}

const MM = 72 / 25.4;

/** Make a PDF from images, one per page; JPEGs are embedded unchanged. */
export function ImagesToPdfDialog({ initialFiles, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [files, setFiles] = useState<string[]>(initialFiles);
  // More pictures can arrive while the dialog is open (several given at launch): add the new ones.
  useEffect(() => setFiles((f) => [...f, ...initialFiles.filter((x) => !f.includes(x))]), [initialFiles]);
  const [size, setSize] = useState<ImagePageSize>("a4");
  const [marginMm, setMarginMm] = useState("0");
  const [output, setOutput] = useState(defaultImagesOutput(initialFiles));
  const [outputTouched, setOutputTouched] = useState(false);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  useEffect(() => {
    if (!outputTouched) setOutput(defaultImagesOutput(files));
  }, [files, outputTouched]);

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

  const margin = Number(marginMm) * MM;
  const blocker = files.length === 0
    ? "Add images first."
    : !Number.isFinite(margin) || margin < 0 || margin > 144
      ? "Use a margin of 0 to 50 mm."
      : !output.trim()
        ? "Choose where to save the PDF."
        : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "images_to_pdf", inputs: files, page_size: size, margin, output }, collision }));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const move = (i: number, d: -1 | 1) =>
    setFiles((f) => {
      const g = f.slice();
      [g[i], g[i + d]] = [g[i + d]!, g[i]!];
      return g;
    });

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
      <button type="button" className="btn primary" data-testid="images-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Creating…" : "Create PDF"}
      </button>
    </>
  );

  return (
    <Dialog title="Create PDF from images" onClose={onClose} footer={footer} busy={busy} wide>
      <p className="hint" style={{ marginTop: 0 }}>
        One image per page, in this order. JPEG photos keep their exact quality; phone photos are turned upright.
      </p>
      <div className="field">
        <span className="label">Images</span>
        {files.length === 0 ? (
          <p className="hint">No images yet. Add JPEG or PNG files, or drop them on the window.</p>
        ) : (
          <ul className="file-list" aria-label="Images, in page order">
            {files.map((f, i) => (
              <li key={f + i}>
                <span className="name" title={f}>
                  {i + 1}. {basename(f)}
                </span>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} up`} title="Move up" aria-disabled={i === 0 || undefined} onClick={() => i > 0 && move(i, -1)}>
                  <ArrowUp />
                </button>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} down`} title="Move down" aria-disabled={i === files.length - 1 || undefined} onClick={() => i < files.length - 1 && move(i, 1)}>
                  <ArrowDown />
                </button>
                <button type="button" className="btn icon" aria-label={`Remove ${basename(f)} from the list`} title="Remove from list (the file is not deleted)" onClick={() => setFiles((fs) => fs.filter((_, j) => j !== i))}>
                  <Trash2 />
                </button>
              </li>
            ))}
          </ul>
        )}
        <div>
          <button
            type="button"
            className="btn outline"
            data-testid="images-add"
            onClick={async () => {
              const picked = await api.pickImageFiles();
              if (picked.length) setFiles((fs) => [...fs, ...picked]);
            }}
          >
            <Plus /> Add images…
          </button>
        </div>
      </div>
      <div className="row" style={{ gap: "var(--space-4)" }}>
        <div className="field">
          <label htmlFor="images-size">Page size</label>
          <select id="images-size" className="select" value={size} onChange={(e) => setSize(e.target.value as ImagePageSize)}>
            <option value="a4">A4</option>
            <option value="letter">Letter</option>
            <option value="image">Same as the image</option>
          </select>
        </div>
        <div className="field">
          <label htmlFor="images-margin">Margin (mm)</label>
          <input id="images-margin" className="input" type="number" min={0} max={50} step="any" style={{ width: "8ch" }} value={marginMm} onChange={(e) => setMarginMm(e.target.value)} />
        </div>
      </div>
      <div className="field">
        <label htmlFor="images-output">Save as</label>
        <div className="row">
          <input
            id="images-output"
            className="input grow"
            value={output}
            onChange={(e) => {
              setOutputTouched(true);
              setOutput(e.target.value);
            }}
            spellCheck={false}
          />
          <button
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickSavePath(output);
              if (p) {
                setOutputTouched(true);
                setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
              }
            }}
          >
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      {busy && progress && progress.kind === "determinate" && (
        <div aria-live="polite" className="hint">
          {progress.done} of {progress.total} images
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

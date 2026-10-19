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

/** Mirrors pw-core MAX_PROPERTY_CHARS. */
export const MAX_PROPERTY_CHARS = 2000;

type Fields = { title: string; author: string; subject: string; keywords: string };

const LABELS: Array<[keyof Fields, string]> = [
  ["title", "Title"],
  ["author", "Author"],
  ["subject", "Subject"],
  ["keywords", "Keywords"],
];

/** Why the values cannot be saved yet, or null. Mirrors the engine's checks (pw-core). */
export function propertiesBlocker(values: Fields, original: Fields): string | null {
  for (const [k, label] of LABELS) {
    const v = values[k];
    if ([...v].length > MAX_PROPERTY_CHARS) return `${label} can be at most ${MAX_PROPERTY_CHARS} characters long.`;
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u001f\u007f-\u009f]/.test(v)) return `${label} cannot contain line breaks or control characters.`;
  }
  if (LABELS.every(([k]) => values[k].trim() === original[k].trim())) return "Change a property first.";
  return null;
}

/** Only changed properties are sent: unchanged ones are kept by the engine as they are. */
export function propertyChanges(values: Fields, original: Fields): Partial<Fields> {
  const out: Partial<Fields> = {};
  for (const [k] of LABELS) if (values[k].trim() !== original[k].trim()) out[k] = values[k].trim();
  return out;
}

/** Edit title, author, subject and keywords; saves a verified copy. */
export function EditPropertiesDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const original: Fields = {
    title: doc.info.title ?? "",
    author: doc.info.author ?? "",
    subject: doc.info.subject ?? "",
    keywords: doc.info.keywords ?? "",
  };
  const [values, setValues] = useState<Fields>(original);
  const [output, setOutput] = useState(siblingOutput(doc.path, "edited"));
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

  const blocker = doc.info.encrypted
    ? "Remove the password first, then change the properties of that copy."
    : (propertiesBlocker(values, original) ?? (output.trim() ? null : "Choose where to save the copy."));

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(
        await api.startOperation({ schema_version: 1, operation: { op: "set_properties", input: doc.path, ...propertyChanges(values, original), output }, collision }, doc.doc_id),
      );
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
      <button type="button" className="btn primary" data-testid="props-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Saving…" : "Save copy"}
      </button>
    </>
  );

  return (
    <Dialog title="Edit document properties" onClose={onClose} footer={footer} busy={busy} initialFocus="#prop-title">
      <p className="hint" style={{ marginTop: 0 }}>
        Saves a copy of <strong>{doc.name}</strong> with these properties. The original file is not changed. Leave a field empty to remove it.
      </p>
      {LABELS.map(([k, label]) => (
        <div className="field" key={k}>
          <label htmlFor={`prop-${k}`}>{label}</label>
          <input
            id={`prop-${k}`}
            className="input"
            value={values[k]}
            disabled={busy}
            onChange={(e) => setValues((v) => ({ ...v, [k]: e.target.value }))}
            onKeyDown={(e) => e.key === "Enter" && start("fail")}
          />
        </div>
      ))}
      <div className="field">
        <label htmlFor="props-output">Save copy as</label>
        <div className="row">
          <input id="props-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      {blocker && blocker !== "Change a property first." && (
        <span className="error" role="status">
          {blocker}
        </span>
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

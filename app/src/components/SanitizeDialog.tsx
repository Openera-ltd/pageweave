import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError, type SanitizeFindings, type SanitizeOptions } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Same defaults as the engine (pw-core `SanitizeOptions::default`). */
export const DEFAULT_SANITIZE: SanitizeOptions = {
  metadata: true,
  scripts: true,
  attachments: true,
  hidden_layers: true,
  comments: true,
  form_fields: false,
  web_links: false,
};

const KINDS: Array<[keyof SanitizeOptions, string]> = [
  ["metadata", "Document properties and metadata"],
  ["scripts", "Scripts and actions"],
  ["attachments", "Attachments"],
  ["hidden_layers", "Hidden layers"],
  ["comments", "Comments and markups"],
  ["form_fields", "Form fields and their data"],
  ["web_links", "Links to websites"],
];

const count = (n: number) => (n > 0 ? `${n} found` : "None found");

/** What the file holds for one kind, in a few words. */
export function findingSummary(kind: keyof SanitizeOptions, f: SanitizeFindings): string {
  switch (kind) {
    case "metadata":
      return f.metadata.length ? f.metadata.join(", ") : "None found";
    case "attachments":
      return f.attachments === 0 ? "None found" : f.attachment_names.length ? `${f.attachments} found: ${f.attachment_names.join(", ")}` : count(f.attachments);
    case "hidden_layers":
      if (f.hidden_layers.length) return f.hidden_layers.join(", ");
      return f.layers > 0 ? `None hidden (${f.layers} layer${f.layers === 1 ? "" : "s"})` : "None found";
    default:
      return count(f[kind]);
  }
}

/** Save a copy without the selected kinds of hidden information (verified by the engine). */
export function SanitizeDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [options, setOptions] = useState<SanitizeOptions>(DEFAULT_SANITIZE);
  const [findings, setFindings] = useState<SanitizeFindings | null>(null);
  const [inspectError, setInspectError] = useState<PwError | null>(null);
  const [output, setOutput] = useState(siblingOutput(doc.path, "cleaned"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  useEffect(() => {
    let live = true;
    api
      .sanitizeInspect(doc.doc_id)
      .then((f) => live && setFindings(f))
      .catch((e) => live && setInspectError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.doc_id]);

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

  const blocker = inspectError
    ? inspectError.message
    : !findings
      ? "Checking the file…"
      : !output.trim()
        ? "Choose where to save the cleaned copy."
        : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "sanitize", input: doc.path, options, output }, collision }, doc.doc_id));
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
      <button type="button" className="btn primary" data-testid="sanitize-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Removing…" : "Remove"}
      </button>
    </>
  );

  return (
    <Dialog title="Remove hidden information" onClose={onClose} footer={footer} busy={busy}>
      <p className="hint" style={{ marginTop: 0 }}>
        Saves a copy of <strong>{doc.name}</strong> without the selected items. The original file is not changed.
      </p>
      {inspectError && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{inspectError.message}</div>
            <div className="hint">{inspectError.action}</div>
          </div>
        </div>
      )}
      <fieldset className="field sanitize-kinds" style={{ border: 0, padding: 0, margin: 0 }} aria-busy={!findings && !inspectError}>
        <legend className="label">Remove</legend>
        {KINDS.map(([k, label]) => (
          <label key={k} className="row sanitize-row">
            <input type="checkbox" checked={options[k]} data-kind={k} disabled={busy} onChange={(e) => setOptions((o) => ({ ...o, [k]: e.target.checked }))} />
            <span className="grow">{label}</span>
            <span className="hint sanitize-finding" data-finding={k} title={findings ? findingSummary(k, findings) : undefined}>
              {findings ? findingSummary(k, findings) : inspectError ? "" : "Checking…"}
            </span>
          </label>
        ))}
      </fieldset>
      <p className="hint" data-testid="sanitize-always">
        {findings?.earlier_versions ? "Earlier saved versions of the file were found and are always removed." : "Earlier saved versions of the file are always removed."} Text drawn in white or covered by other content is not detected.
      </p>
      <div className="field">
        <label htmlFor="sanitize-output">Save cleaned copy as</label>
        <div className="row">
          <input id="sanitize-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} onKeyDown={(e) => e.key === "Enter" && start("fail")} />
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

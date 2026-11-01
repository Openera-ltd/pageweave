import { useEffect, useMemo, useState } from "react";
import { FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { toFieldValues, type FormEdits } from "../forms";
import { defaultDataPath, fieldLabel, planImport, shownValue, switchDataExtension, withDataExtension, type FormDataFormat, type FormDataMatch, type ImportPlan } from "../formData";
import { basename } from "../paths";
import { toPwError, type CollisionPolicy, type FormInfo, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

const DONE = ["succeeded", "failed", "cancelled"];

interface ExportProps {
  doc: OpenedDoc;
  form: FormInfo;
  edits: FormEdits;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Save the form's values (unsaved changes included) as an XFDF or JSON file. */
export function ExportFormDataDialog({ doc, form, edits, jobs, onClose, onFinished }: ExportProps) {
  const api = useApi();
  const [format, setFormat] = useState<FormDataFormat>("xfdf");
  const [output, setOutput] = useState(defaultDataPath(doc.path, "xfdf"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !DONE.includes(job.state);
  const values = useMemo(() => toFieldValues(edits, form.fields), [edits, form.fields]);

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

  const choose = (f: FormDataFormat) => {
    setFormat(f);
    setOutput((o) => switchDataExtension(o, f));
  };

  const blocker = output.trim() ? null : "Choose where to save the form data.";

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const out = withDataExtension(output.trim(), format);
      setOutput(out);
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "export_form_data", input: doc.path, format, values, output: out }, collision }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const p = await api.pickFormDataSavePath(output, format);
    if (p) setOutput(withDataExtension(p, format));
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="formdata-export-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Saving…" : "Export"}
      </button>
    </>
  );

  return (
    <Dialog title="Export form data" onClose={onClose} footer={footer} busy={busy} initialFocus="#formdata-export-output">
      <fieldset className="field" style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }} disabled={busy}>
        <legend className="label">Format</legend>
        <label className="row">
          <input type="radio" name="formdata-format" checked={format === "xfdf"} onChange={() => choose("xfdf")} /> XFDF — opens in most PDF apps
        </label>
        <label className="row">
          <input type="radio" name="formdata-format" data-testid="formdata-format-json" checked={format === "json"} onChange={() => choose("json")} /> JSON — for scripts and spreadsheets
        </label>
      </fieldset>
      <div className="field">
        <label htmlFor="formdata-export-output">Save as</label>
        <div className="row">
          <input id="formdata-export-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} disabled={busy} onKeyDown={(e) => e.key === "Enter" && start("fail")} />
          <button type="button" className="btn outline" onClick={browse} disabled={busy}>
            <FolderOpen /> Browse…
          </button>
        </div>
        <span className="hint" data-testid="formdata-export-note">
          {values.length > 0 ? `Includes ${values.length} unsaved change${values.length === 1 ? "" : "s"}.` : "Saves the values the form holds now."}
        </span>
      </div>
      {error && <ErrorBanner error={error} conflict={conflict} onKeep={() => start("rename")} onReplace={() => start("replace")} />}
    </Dialog>
  );
}

function ErrorBanner({ error, conflict, onKeep, onReplace }: { error: PwError; conflict: boolean; onKeep: () => void; onReplace: () => void }) {
  return (
    <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
      <div className="grow">
        <div>{error.message}</div>
        {!conflict && <div className="hint">{error.action}</div>}
      </div>
      {conflict && (
        <div className="row">
          <button type="button" className="btn outline" onClick={onKeep}>
            Keep both
          </button>
          <button type="button" className="btn danger" onClick={onReplace}>
            Replace
          </button>
        </div>
      )}
    </div>
  );
}

interface ImportProps {
  doc: OpenedDoc;
  form: FormInfo;
  edits: FormEdits;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  /** Apply the planned edits (they stay unsaved until the user saves a copy). */
  onApply: (plan: ImportPlan, source: string) => void;
}

/** Read a form data file, show which fields it fills, then put its values into the form. */
export function ImportFormDataDialog({ doc, form, edits, jobs, onClose, onApply }: ImportProps) {
  const api = useApi();
  const [path, setPath] = useState("");
  const [readId, setReadId] = useState<string | null>(null);
  const [match, setMatch] = useState<FormDataMatch | null>(null);
  const [readPath, setReadPath] = useState("");
  const [error, setError] = useState<PwError | null>(null);
  const job = readId ? jobs[readId] : undefined;
  const reading = !!job && !DONE.includes(job.state);

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      setMatch((job.result?.data as FormDataMatch | undefined) ?? null);
    } else if (job.state === "failed" && job.error) {
      setError(job.error);
    }
    if (DONE.includes(job.state)) setReadId(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const read = async (p: string) => {
    const file = p.trim();
    if (!file || reading) return;
    setError(null);
    setMatch(null);
    setReadPath(file);
    try {
      setReadId(await api.startOperation({ schema_version: 1, operation: { op: "read_form_data", input: doc.path, data: file }, collision: "fail" }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const p = await api.pickFormDataFile();
    if (p) {
      setPath(p);
      void read(p);
    }
  };

  // Only a preview of the file that is still in the path field may be applied.
  const current = match && readPath === path.trim() ? match : null;
  const plan = useMemo(() => (current ? planImport(current, form.fields, edits) : null), [current, form.fields, edits]);
  const notUsed = current ? current.skipped.length + current.unknown.length : 0;

  const blocker = !path.trim()
    ? "Choose a form data file."
    : reading
      ? "Reading the file…"
      : !current
        ? "Read the file first."
        : plan && plan.changes.length === 0
          ? current.matched.length > 0
            ? "The form already shows every value in this file."
            : "None of the values in this file fit this form."
          : null;

  const apply = () => {
    if (blocker || !plan) return;
    onApply(plan, basename(readPath));
    onClose();
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="formdata-import-apply" aria-disabled={blocker ? true : undefined} title={blocker ?? undefined} onClick={apply}>
        {plan && plan.changes.length > 0 ? `Fill ${plan.changes.length} field${plan.changes.length === 1 ? "" : "s"}` : "Fill fields"}
      </button>
    </>
  );

  return (
    <Dialog title="Import form data" onClose={onClose} footer={footer} wide initialFocus="#formdata-import-path">
      <div className="field">
        <label htmlFor="formdata-import-path">Form data file (XFDF or JSON)</label>
        <div className="row">
          <input
            id="formdata-import-path"
            className="input grow"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            spellCheck={false}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !(current && plan?.changes.length)) {
                e.preventDefault();
                void read(path);
              }
            }}
          />
          <button type="button" className="btn outline" data-testid="formdata-import-read" onClick={() => read(path)} aria-disabled={!path.trim() || reading ? true : undefined} title={!path.trim() ? "Choose a form data file." : undefined}>
            {reading ? "Reading…" : "Read"}
          </button>
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      <div aria-live="polite" data-testid="formdata-import-summary" className="hint">
        {current && plan && (
          <>
            {plan.changes.length} field{plan.changes.length === 1 ? "" : "s"} to fill
            {plan.unchanged > 0 && ` · ${plan.unchanged} already the same`}
            {notUsed > 0 && ` · ${notUsed} not used`}
          </>
        )}
      </div>
      {plan && plan.changes.length > 0 && (
        <ul className="formdata-list" aria-label="Fields to fill" data-testid="formdata-import-changes">
          {plan.changes.map((c) => (
            <li key={c.field.name} className="row">
              <span className="formdata-name" title={c.field.name}>
                {fieldLabel(c.field)}
              </span>
              <span className="formdata-value grow" title={typeof c.to === "string" ? c.to : undefined}>
                {shownValue(c.to, c.field.kind)}
              </span>
            </li>
          ))}
        </ul>
      )}
      {current && notUsed > 0 && (
        <details className="formdata-unused" data-testid="formdata-import-unused">
          <summary>Not used ({notUsed})</summary>
          <ul className="formdata-list">
            {current.skipped.map((s, i) => (
              <li key={`s${i}`} className="row">
                <span className="formdata-name" title={s.name}>
                  {s.name}
                </span>
                <span className="formdata-value grow">{s.reason}</span>
              </li>
            ))}
            {current.unknown.map((n, i) => (
              <li key={`u${i}`} className="row">
                <span className="formdata-name" title={n}>
                  {n}
                </span>
                <span className="formdata-value grow">Not a field of this form.</span>
              </li>
            ))}
          </ul>
        </details>
      )}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            <div className="hint">{error.action}</div>
          </div>
        </div>
      )}
      <p className="hint" style={{ marginBottom: 0 }}>
        Values go into the form as unsaved changes; save a copy to keep them.
      </p>
    </Dialog>
  );
}

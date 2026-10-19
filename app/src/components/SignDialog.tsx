import { useEffect, useState } from "react";
import { Eye, EyeOff, FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { SIGN_POSITIONS, signatureRect, type SignPosition } from "../signPlacement";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { PlacementPreview } from "./PlacementPreview";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  currentPage: number;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Sign a copy with a certificate file (.pfx/.p12): invisible, or in a box on a page. */
export function SignDialog({ doc, currentPage, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [certificate, setCertificate] = useState("");
  const [password, setPassword] = useState("");
  const [show, setShow] = useState(false);
  const [reason, setReason] = useState("");
  const [location, setLocation] = useState("");
  const [visible, setVisible] = useState(true);
  const [page, setPage] = useState(currentPage + 1);
  const [position, setPosition] = useState<SignPosition>("bottom-right");
  const [output, setOutput] = useState(siblingOutput(doc.path, "signed"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  // The password lives only as long as the dialog.
  useEffect(() => () => setPassword(""), []);

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

  const pageInfo = doc.info.pages[page - 1];
  const pageOk = Number.isInteger(page) && !!pageInfo;
  const rect = visible && pageInfo ? signatureRect(pageInfo, position) : null;
  const blocker = !certificate.trim()
    ? "Choose your certificate file (.pfx or .p12)."
    : !password
      ? "Enter the certificate's password."
      : visible && !pageOk
        ? `Choose a page from 1 to ${doc.info.page_count}.`
        : !output.trim()
          ? "Choose where to save the signed copy."
          : null;

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(
        await api.startOperation(
          {
            schema_version: 1,
            operation: {
              op: "sign",
              input: doc.path,
              certificate,
              reason: reason.trim() || null,
              location: location.trim() || null,
              placement: rect ? { page: page - 1, rect } : null,
              output,
            },
            collision,
            certificate_password: password,
          },
          doc.doc_id,
        ),
      );
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const preview = visible && pageInfo && rect ? <PlacementPreview docId={doc.doc_id} page={pageInfo} rect={rect} /> : null;

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="sign-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Signing…" : "Sign"}
      </button>
    </>
  );

  return (
    <Dialog title="Sign with a certificate" onClose={onClose} footer={footer} busy={busy} wide initialFocus="#sign-cert-browse">
      <p className="hint" style={{ marginTop: 0 }}>
        Adds a digital signature to a copy of <strong>{doc.name}</strong>. Readers trust it only if they trust your certificate; changes made later show as changes after signing.
      </p>
      <div className="field">
        <label htmlFor="sign-cert">Certificate file</label>
        <div className="row">
          <input id="sign-cert" className="input grow" value={certificate} placeholder=".pfx or .p12 with your private key" spellCheck={false} onChange={(e) => setCertificate(e.target.value)} />
          <button
            id="sign-cert-browse"
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickCertificateFile();
              if (p) setCertificate(p);
            }}
          >
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      <div className="field">
        <label htmlFor="sign-pw">Certificate password</label>
        <div className="row">
          <input
            id="sign-pw"
            className="input grow"
            type={show ? "text" : "password"}
            autoComplete="off"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && start("fail")}
          />
          <button type="button" className="btn icon" aria-label={show ? "Hide password" : "Show password"} title={show ? "Hide password" : "Show password"} aria-pressed={show} onClick={() => setShow((s) => !s)}>
            {show ? <EyeOff /> : <Eye />}
          </button>
        </div>
      </div>
      <div className="row" style={{ gap: "var(--space-4)", alignItems: "flex-start" }}>
        <div className="grow">
          <div className="field">
            <label htmlFor="sign-reason">Reason (optional)</label>
            <input id="sign-reason" className="input" value={reason} maxLength={200} onChange={(e) => setReason(e.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="sign-location">Location (optional)</label>
            <input id="sign-location" className="input" value={location} maxLength={200} onChange={(e) => setLocation(e.target.value)} />
          </div>
          <div className="field">
            <label className="row">
              <input type="checkbox" checked={visible} data-testid="sign-visible" onChange={(e) => setVisible(e.target.checked)} /> Show the signature on a page
            </label>
          </div>
          {visible && (
            <div className="row" style={{ gap: "var(--space-3)" }}>
              <div className="field">
                <label htmlFor="sign-page">Page</label>
                <input
                  id="sign-page"
                  className="input"
                  type="number"
                  min={1}
                  max={doc.info.page_count}
                  value={Number.isNaN(page) ? "" : page}
                  aria-invalid={!pageOk}
                  style={{ width: 80 }}
                  onChange={(e) => setPage(e.target.valueAsNumber)}
                />
              </div>
              <div className="field grow">
                <label htmlFor="sign-position">Position</label>
                <select id="sign-position" className="select" value={position} onChange={(e) => setPosition(e.target.value as SignPosition)}>
                  {SIGN_POSITIONS.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}
        </div>
        {preview}
      </div>
      <div className="field">
        <label htmlFor="sign-output">Save as</label>
        <div className="row">
          <input id="sign-output" className="input grow" value={output} spellCheck={false} onChange={(e) => setOutput(e.target.value)} />
          <button
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickSavePath(output);
              if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
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
            {!conflict && error.action && <div className="hint">{error.action}</div>}
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

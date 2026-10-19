import { useEffect, useState } from "react";
import { Eye, EyeOff, FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type ProtectPermissions, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

const PERMISSIONS: Array<[keyof ProtectPermissions, string]> = [
  ["print", "Printing"],
  ["copy", "Copying text and images"],
  ["modify", "Editing"],
  ["annotate", "Comments"],
  ["fill_forms", "Filling in forms"],
  ["assemble", "Inserting, deleting and turning pages"],
];

const ALL: ProtectPermissions = { print: true, copy: true, modify: true, annotate: true, fill_forms: true, assemble: true };

/** Why the passwords cannot be used yet, or null. Mirrors the engine's checks (pw-core). */
export function protectBlocker(pw: string, confirm: string, owner: string): string | null {
  if (!pw) return "Enter a password.";
  if (pw !== confirm) return "The passwords do not match.";
  const bytes = (s: string) => new TextEncoder().encode(s).length;
  if (bytes(pw) > 127 || bytes(owner) > 127) return "Passwords can be at most 127 bytes long.";
  if (owner && owner === pw) return "Use a different permissions password, or leave it empty.";
  return null;
}

/** Encrypt a copy with AES-256 and an open password; optional permissions for readers that honour them. */
export function ProtectDialog({ doc, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [owner, setOwner] = useState("");
  const [show, setShow] = useState(false);
  const [perms, setPerms] = useState<ProtectPermissions>(ALL);
  const [output, setOutput] = useState(siblingOutput(doc.path, "protected"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);
  const restricted = Object.values(perms).some((v) => !v);

  // Passwords live only as long as the dialog.
  useEffect(
    () => () => {
      setPw("");
      setConfirm("");
      setOwner("");
    },
    [],
  );

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

  const blocker = protectBlocker(pw, confirm, owner) ?? (output.trim() ? null : "Choose where to save the protected copy.");

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      setJobId(
        await api.startOperation(
          { schema_version: 1, operation: { op: "protect", input: doc.path, permissions: perms, output }, collision, new_password: pw, new_owner_password: owner || null },
          doc.doc_id,
        ),
      );
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    const p = await api.pickSavePath(output);
    if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
  };

  const pwType = show ? "text" : "password";
  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="protect-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Protecting…" : "Protect"}
      </button>
    </>
  );

  return (
    <Dialog title="Protect with password" onClose={onClose} footer={footer} busy={busy} initialFocus="#protect-pw">
      <p className="hint" style={{ marginTop: 0 }}>
        Saves an encrypted copy of <strong>{doc.name}</strong> that opens only with this password. The original file is not changed.
      </p>
      <div className="field">
        <label htmlFor="protect-pw">Password</label>
        <div className="row">
          <input id="protect-pw" className="input grow" type={pwType} autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} />
          <button type="button" className="btn icon" aria-label={show ? "Hide passwords" : "Show passwords"} title={show ? "Hide passwords" : "Show passwords"} aria-pressed={show} onClick={() => setShow((s) => !s)}>
            {show ? <EyeOff /> : <Eye />}
          </button>
        </div>
      </div>
      <div className="field">
        <label htmlFor="protect-confirm">Confirm password</label>
        <input
          id="protect-confirm"
          className="input"
          type={pwType}
          autoComplete="new-password"
          value={confirm}
          aria-invalid={!!confirm && confirm !== pw}
          aria-describedby="protect-msg"
          onChange={(e) => setConfirm(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && start("fail")}
        />
        <span id="protect-msg" className={confirm && confirm !== pw ? "error" : "hint"} aria-live="polite">
          {confirm && confirm !== pw ? "The passwords do not match." : "The password cannot be recovered if it is lost."}
        </span>
      </div>
      <fieldset className="field" style={{ border: 0, padding: 0, margin: 0 }}>
        <legend className="label">Allow others to</legend>
        {PERMISSIONS.map(([k, label]) => (
          <label key={k} className="row">
            <input type="checkbox" checked={perms[k]} data-perm={k} onChange={(e) => setPerms((p) => ({ ...p, [k]: e.target.checked }))} /> {label}
          </label>
        ))}
      </fieldset>
      {restricted && (
        <div className="field">
          <label htmlFor="protect-owner">Permissions password (optional)</label>
          <input id="protect-owner" className="input" type={pwType} autoComplete="new-password" value={owner} onChange={(e) => setOwner(e.target.value)} aria-describedby="protect-owner-help" />
          <span id="protect-owner-help" className="hint">
            Needed to lift the restrictions later. Leave empty to make them permanent. Restrictions rely on the reader honouring them.
          </span>
        </div>
      )}
      <div className="field">
        <label htmlFor="protect-output">Save protected copy as</label>
        <div className="row">
          <input id="protect-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
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

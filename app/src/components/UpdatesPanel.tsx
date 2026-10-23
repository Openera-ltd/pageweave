// Settings ▸ Updates (D-024): this version, check now, download with progress and cancel, then
// install and restart. Checking reads only this project's release page.

import { useEffect, useState } from "react";
import { useApi } from "../api";
import { megabytes } from "../translate";
import { toPwError, type AvailableUpdate, type JobSnapshot, type UpdateInfo } from "../types";

interface Props {
  checkOnStart: boolean;
  onCheckOnStart: (v: boolean) => void;
  /** An update found earlier (at startup), so the panel opens with it. */
  known?: AvailableUpdate | null;
}

type Status = { kind: "idle" } | { kind: "checking" } | { kind: "latest" } | { kind: "available"; update: AvailableUpdate } | { kind: "error"; message: string };

export function UpdatesPanel({ checkOnStart, onCheckOnStart, known }: Props) {
  const api = useApi();
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [status, setStatus] = useState<Status>(known ? { kind: "available", update: known } : { kind: "idle" });
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<JobSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    api.updateInfo().then(setInfo, () => setInfo(null));
  }, [api]);

  useEffect(() => {
    if (!jobId) return;
    let off: (() => void) | undefined;
    let gone = false;
    void api
      .onJobUpdate((j) => {
        if (j.id === jobId) setJob(j);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [api, jobId]);

  useEffect(() => {
    if (!job || status.kind !== "available") return;
    if (job.state === "succeeded") {
      setStatus({ kind: "available", update: { ...status.update, downloaded: true } });
      setJobId(null);
      setJob(null);
    } else if (job.state === "failed") {
      setError(job.error?.message ?? "The download failed.");
      setJobId(null);
      setJob(null);
    } else if (job.state === "cancelled") {
      setJobId(null);
      setJob(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const check = async () => {
    setError(null);
    setStatus({ kind: "checking" });
    try {
      const u = await api.checkForUpdate();
      setStatus(u ? { kind: "available", update: u } : { kind: "latest" });
    } catch (e) {
      setStatus({ kind: "error", message: toPwError(e).message });
    }
  };

  const download = async (u: AvailableUpdate) => {
    setError(null);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "download_update", version: u.version }, collision: "fail" }));
    } catch (e) {
      setError(toPwError(e).message);
    }
  };

  const install = async (u: AvailableUpdate) => {
    setError(null);
    setInstalling(true);
    try {
      await api.installUpdate(u.version);
    } catch (e) {
      setError(toPwError(e).message);
      setInstalling(false);
    }
  };

  const canInstall = !!info && info.can_install && info.platform_supported;
  const busy = jobId !== null;
  const p = job?.progress;
  return (
    <div className="field" data-testid="updates-panel">
      <div className="row">
        <strong className="grow">PageWeave {info?.current ?? ""}</strong>
        <button type="button" className="btn outline" data-testid="updates-check" aria-disabled={status.kind === "checking" || busy || undefined} onClick={() => status.kind !== "checking" && !busy && void check()}>
          {status.kind === "checking" ? "Checking…" : "Check now"}
        </button>
      </div>
      <div aria-live="polite" data-testid="updates-status">
        {status.kind === "latest" && <span className="hint">PageWeave is up to date.</span>}
        {status.kind === "error" && (
          <span className="error" role="alert">
            {status.message}
          </span>
        )}
        {status.kind === "available" && (
          <div className="download-panel" role="group" aria-label="Available update">
            <span>
              Version {status.update.version} is available ({megabytes(status.update.bytes)}).
            </span>
            {status.update.notes && <p className="hint update-notes">{status.update.notes}</p>}
            {busy && p && p.kind === "determinate" && (
              <>
                <span className="hint">
                  Downloading: {megabytes(p.done)} of {megabytes(p.total)}
                </span>
                <progress max={p.total} value={p.done} aria-label="Update download progress" />
              </>
            )}
            {busy && (!p || p.kind !== "determinate") && <span className="hint">Downloading…</span>}
            {error && (
              <span className="hint error" role="alert">
                {error}
              </span>
            )}
            {!canInstall ? (
              <div className="row">
                <span className="hint grow">{info && !info.platform_supported ? "Updates install on Windows." : "This copy can't install updates itself."}</span>
                <button type="button" className="btn" data-testid="updates-releases" onClick={() => void api.openReleasesPage().catch((e) => setError(toPwError(e).message))}>
                  Open releases page
                </button>
              </div>
            ) : busy ? (
              <button type="button" className="btn outline" data-testid="updates-cancel" disabled={job?.state === "cancelling"} onClick={() => jobId && void api.cancelJob(jobId)}>
                {job?.state === "cancelling" ? "Cancelling…" : "Cancel download"}
              </button>
            ) : status.update.downloaded ? (
              <button type="button" className="btn primary" data-testid="updates-install" aria-disabled={installing || undefined} onClick={() => !installing && void install(status.update)}>
                {installing ? "Starting the installer…" : "Install and restart"}
              </button>
            ) : (
              <button type="button" className="btn primary" data-testid="updates-download" onClick={() => void download(status.update)}>
                {error ? "Try again" : `Download (${megabytes(status.update.bytes)})`}
              </button>
            )}
          </div>
        )}
      </div>
      <label className="row">
        <input type="checkbox" checked={checkOnStart} data-testid="updates-auto" onChange={(e) => onCheckOnStart(e.target.checked)} />
        Check for updates when PageWeave starts
      </label>
      <span className="hint">Checking reads this project's release page on GitHub. Nothing about you or your documents is sent. Updates replace only the program; downloaded languages and settings are kept.</span>
    </div>
  );
}

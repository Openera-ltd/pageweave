// One-time download of translation models (D-023): what is needed, how big it is, progress in
// megabytes, cancel, and a clear error with Try again. Each direction is one job.

import { useEffect, useState } from "react";
import { useApi } from "../api";
import { languageName, megabytes } from "../translate";
import { toPwError, type JobSnapshot, type TranslationDirection } from "../types";

interface Props {
  needs: TranslationDirection[];
  jobs: Record<string, JobSnapshot>;
  /** All needed directions are now downloaded. */
  onDone: () => void;
}

export function TranslationDownload({ needs, jobs, onDone }: Props) {
  const api = useApi();
  const [queue, setQueue] = useState<TranslationDirection[] | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const total = needs.reduce((n, d) => n + d.bytes, 0);
  const job = jobId ? jobs[jobId] : undefined;

  const startNext = async (rest: TranslationDirection[]) => {
    const next = rest[0];
    if (!next) {
      setQueue(null);
      setJobId(null);
      onDone();
      return;
    }
    setQueue(rest);
    try {
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "install_translation", source: next.source, target: next.target }, collision: "fail" }));
    } catch (e) {
      setQueue(null);
      setError(toPwError(e).message);
    }
  };

  useEffect(() => {
    if (!job || !queue) return;
    if (job.state === "succeeded") void startNext(queue.slice(1));
    else if (job.state === "failed") {
      setError(job.error?.message ?? "The download failed.");
      setQueue(null);
      setJobId(null);
    } else if (job.state === "cancelled") {
      setQueue(null);
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const busy = queue !== null;
  const p = job?.progress;
  const current = queue?.[0];
  return (
    <div className="download-panel" data-testid="translate-download" role="group" aria-label="Translation download">
      <span className="hint">
        {busy && current
          ? `Downloading ${languageName(current.source)} → ${languageName(current.target)}${p && p.kind === "determinate" ? `: ${megabytes(p.done)} of ${megabytes(p.total)}` : "…"}`
          : `Needs a one-time download: ${needs.map((d) => `${languageName(d.source)} → ${languageName(d.target)}`).join(", ")} (${megabytes(total)}). It is kept for later and is not included in app updates.`}
      </span>
      {busy && p && p.kind === "determinate" && <progress max={p.total} value={p.done} aria-label="Download progress" />}
      {error && (
        <span className="hint error" role="alert">
          {error}
        </span>
      )}
      {busy ? (
        <button type="button" className="btn outline" data-testid="translate-download-cancel" onClick={() => jobId && api.cancelJob(jobId)} disabled={job?.state === "cancelling"}>
          {job?.state === "cancelling" ? "Cancelling…" : "Cancel download"}
        </button>
      ) : (
        <button
          type="button"
          className="btn"
          data-testid="translate-download-start"
          onClick={() => {
            setError(null);
            void startNext(needs);
          }}
        >
          {error ? "Try again" : `Download (${megabytes(total)})`}
        </button>
      )}
    </div>
  );
}

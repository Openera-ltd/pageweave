import { useEffect, useMemo, useState } from "react";
import { useApi } from "../api";
import { toPwError, type JobSnapshot, type OpenedDoc, type PwError, type SearchRect } from "../types";
import { Dialog } from "./ui";

export type PatternKind = "email" | "phone" | "card_number" | "iban" | "us_ssn";

export const PATTERN_KINDS: Array<{ id: PatternKind; label: string }> = [
  { id: "email", label: "E-mail addresses" },
  { id: "phone", label: "Phone numbers" },
  { id: "card_number", label: "Payment card numbers" },
  { id: "iban", label: "Bank account numbers (IBAN)" },
  { id: "us_ssn", label: "US Social Security numbers" },
];

export const KIND_LABEL: Record<string, string> = { email: "E-mail", phone: "Phone", card_number: "Card", iban: "IBAN", us_ssn: "SSN", custom: "Pattern" };

export interface PatternHit {
  page: number;
  kind: string;
  text: string;
  rects: SearchRect[];
}

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  /** Mark the chosen matches for redaction (one mark per match). */
  onMark: (hits: PatternHit[]) => void;
}

/** Find sensitive text, review every match, then mark the chosen ones for redaction. */
export function RedactPatternsDialog({ doc, jobs, onClose, onMark }: Props) {
  const api = useApi();
  const [kinds, setKinds] = useState<Set<PatternKind>>(new Set(PATTERN_KINDS.map((k) => k.id)));
  const [custom, setCustom] = useState("");
  const [jobId, setJobId] = useState<string | null>(null);
  const [hits, setHits] = useState<PatternHit[] | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [error, setError] = useState<PwError | null>(null);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      const data = job.result?.data as { hits?: PatternHit[]; truncated?: boolean } | undefined;
      const found = data?.hits ?? [];
      setHits(found);
      setTruncated(!!data?.truncated);
      setChosen(new Set(found.map((_, i) => i)));
      setJobId(null);
    } else if (job.state === "failed" && job.error) {
      setError(job.error);
      setJobId(null);
    } else if (job.state === "cancelled") {
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const findBlocker = kinds.size === 0 && !custom.trim() ? "Choose what to look for." : null;

  const find = async () => {
    if (findBlocker || busy) return;
    setError(null);
    setHits(null);
    try {
      setJobId(
        await api.startOperation(
          { schema_version: 1, operation: { op: "find_patterns", input: doc.path, patterns: [...kinds], custom: custom.trim() || null, pages: null }, collision: "fail" },
          doc.doc_id,
        ),
      );
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const toggleKind = (k: PatternKind) =>
    setKinds((s) => {
      const n = new Set(s);
      if (n.has(k)) n.delete(k);
      else n.add(k);
      return n;
    });
  const toggleHit = (i: number) =>
    setChosen((s) => {
      const n = new Set(s);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });

  const markBlocker = !hits ? "Find matches first." : chosen.size === 0 ? "Choose at least one match." : null;
  const mark = () => {
    if (markBlocker || !hits) return;
    onMark(hits.filter((_, i) => chosen.has(i)));
    onClose();
  };

  const summary = useMemo(() => {
    if (!hits) return "";
    if (hits.length === 0) return "No matches found.";
    const pages = new Set(hits.map((h) => h.page)).size;
    return `${hits.length} match${hits.length === 1 ? "" : "es"} on ${pages} page${pages === 1 ? "" : "s"}${truncated ? " (first matches only)" : ""}`;
  }, [hits, truncated]);

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="patterns-mark" aria-disabled={markBlocker ? true : undefined} title={markBlocker ?? undefined} onClick={mark}>
        {chosen.size > 0 && hits ? `Mark ${chosen.size} for redaction` : "Mark for redaction"}
      </button>
    </>
  );

  return (
    <Dialog title="Mark patterns for redaction" onClose={onClose} footer={footer} wide busy={busy}>
      <fieldset className="field" style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }} disabled={busy}>
        <legend className="label">Look for</legend>
        {PATTERN_KINDS.map((k) => (
          <label key={k.id} className="row">
            <input type="checkbox" data-kind={k.id} checked={kinds.has(k.id)} onChange={() => toggleKind(k.id)} /> {k.label}
          </label>
        ))}
      </fieldset>
      <div className="field">
        <label htmlFor="patterns-custom">Also this pattern (regular expression)</label>
        <div className="row">
          <input id="patterns-custom" className="input grow" value={custom} placeholder="e.g. INV-\d{6}" spellCheck={false} disabled={busy} onChange={(e) => setCustom(e.target.value)} onKeyDown={(e) => e.key === "Enter" && find()} />
          <button type="button" className="btn outline" data-testid="patterns-find" aria-disabled={findBlocker || busy ? true : undefined} title={findBlocker ?? undefined} onClick={find}>
            {busy ? "Finding…" : "Find"}
          </button>
        </div>
      </div>
      <div className="hint" aria-live="polite" data-testid="patterns-summary">
        {busy ? "Looking through the pages…" : summary}
      </div>
      {hits && hits.length > 0 && (
        <ul className="formdata-list" aria-label="Matches" data-testid="patterns-hits">
          {hits.map((h, i) => (
            <li key={i}>
              <label className="row" style={{ minWidth: 0 }}>
                <input type="checkbox" checked={chosen.has(i)} onChange={() => toggleHit(i)} />
                <span className="pattern-kind">{KIND_LABEL[h.kind] ?? h.kind}</span>
                <span className="formdata-value grow" title={h.text}>
                  {h.text}
                </span>
                <span className="hint">Page {h.page + 1}</span>
              </label>
            </li>
          ))}
        </ul>
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
        Review each match: automatic search can miss text (for example in scanned pages without recognized text) or find look-alikes.
      </p>
    </Dialog>
  );
}

import { useEffect, useState } from "react";
import { useApi } from "../api";
import { toPwError, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

export type CheckStatus = "pass" | "fail" | "warning" | "manual" | "not_applicable";

export interface AccessibilityCheck {
  id: string;
  title: string;
  status: CheckStatus;
  detail: string;
  pages: number[];
  names: string[];
}

export interface AccessibilityReport {
  tagged: boolean;
  checks: AccessibilityCheck[];
  passed: number;
  failed: number;
  warnings: number;
  manual: number;
}

/** Status words are always shown in text, never by colour alone. */
export const STATUS_LABEL: Record<CheckStatus, string> = { fail: "Fails", warning: "Warning", manual: "Review", pass: "Passes", not_applicable: "Not applicable" };

/** Groups in reading order: what needs fixing first, what passed last. */
export const GROUPS: Array<{ status: CheckStatus; heading: string }> = [
  { status: "fail", heading: "Needs fixing" },
  { status: "warning", heading: "Worth fixing" },
  { status: "manual", heading: "Review by hand" },
  { status: "pass", heading: "Passed" },
  { status: "not_applicable", heading: "Not applicable" },
];

export function summaryLine(r: AccessibilityReport): string {
  const parts = [`${r.failed} to fix`, `${r.warnings} warning${r.warnings === 1 ? "" : "s"}`, `${r.passed} passed`, `${r.manual} to review by hand`];
  return parts.join(" · ");
}

interface Props {
  doc: OpenedDoc;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onGoToPage: (page: number) => void;
}

/** Automated accessibility checks of the open document, with the items a person must review. */
export function AccessibilityDialog({ doc, jobs, onClose, onGoToPage }: Props) {
  const api = useApi();
  const [jobId, setJobId] = useState<string | null>(null);
  const [report, setReport] = useState<AccessibilityReport | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const job = jobId ? jobs[jobId] : undefined;

  useEffect(() => {
    let live = true;
    api
      .startOperation({ schema_version: 1, operation: { op: "check_accessibility", input: doc.path }, collision: "fail" }, doc.doc_id)
      .then((id) => live && setJobId(id))
      .catch((e) => live && setError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.path, doc.doc_id]);

  useEffect(() => {
    if (job?.state === "succeeded") setReport((job.result?.data as AccessibilityReport | undefined) ?? null);
    else if (job?.state === "failed" && job.error) setError(job.error);
  }, [job?.state, job?.result, job?.error]);

  const footer = (
    <button type="button" className="btn primary" onClick={onClose}>
      Close
    </button>
  );

  return (
    <Dialog title="Accessibility check" onClose={onClose} footer={footer} wide>
      <div className="hint" aria-live="polite" data-testid="a11y-summary">
        {report ? summaryLine(report) : error ? "" : "Checking…"}
      </div>
      {report &&
        GROUPS.map((g) => {
          const items = report.checks.filter((c) => c.status === g.status);
          if (items.length === 0) return null;
          const list = (
            <ul className="a11y-list" aria-label={g.heading}>
              {items.map((c) => (
                <li key={c.id} data-check={c.id} data-status={c.status}>
                  <div className="row" style={{ minWidth: 0 }}>
                    <span className={`a11y-status ${c.status}`}>{STATUS_LABEL[c.status]}</span>
                    <strong className="grow">{c.title}</strong>
                  </div>
                  <div className="hint">{c.detail}</div>
                  {c.names.length > 0 && <div className="hint a11y-names">{c.names.join(", ")}</div>}
                  {c.pages.length > 0 && (
                    <div className="row a11y-pages">
                      {c.pages.slice(0, 12).map((p) => (
                        <button
                          type="button"
                          key={p}
                          className="btn outline"
                          aria-label={`Go to page ${p + 1} (${c.title})`}
                          onClick={() => {
                            onGoToPage(p);
                            onClose();
                          }}
                        >
                          Page {p + 1}
                        </button>
                      ))}
                      {c.pages.length > 12 && <span className="hint">and {c.pages.length - 12} more</span>}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          );
          return g.status === "pass" || g.status === "not_applicable" ? (
            <details key={g.status} className="formdata-unused" data-testid={`a11y-group-${g.status}`}>
              <summary>
                {g.heading} ({items.length})
              </summary>
              {list}
            </details>
          ) : (
            <section key={g.status} data-testid={`a11y-group-${g.status}`}>
              <h3 className="a11y-heading">
                {g.heading} ({items.length})
              </h3>
              {list}
            </section>
          );
        })}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            <div className="hint">{error.action}</div>
          </div>
        </div>
      )}
      <p className="hint" style={{ marginBottom: 0 }}>
        Automated checks find common problems; they don’t show that the document conforms to PDF/UA or WCAG.
      </p>
    </Dialog>
  );
}

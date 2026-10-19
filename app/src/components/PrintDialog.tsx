import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useApi } from "../api";
import { MAX_PRINT_MEGAPIXELS, PRINT_QUALITIES, printPlan, type PrintPlan, type PrintQuality } from "../print";
import { toPwError, type OpenedDoc } from "../types";
import { Dialog, useDebounced } from "./ui";

interface Props {
  doc: OpenedDoc;
  currentPage: number;
  onClose: () => void;
}

type Range = { state: "idle" | "checking" } | { state: "ok"; pages: number[] } | { state: "error"; message: string };

/** Choose pages and resolution, prepare the pages, then hand them to the platform's print UI. */
export function PrintDialog({ doc, currentPage, onClose }: Props) {
  const api = useApi();
  const [which, setWhich] = useState<"all" | "current" | "range">("all");
  const [spec, setSpec] = useState("");
  const [quality, setQuality] = useState<PrintQuality>("standard");
  const [plan, setPlan] = useState<PrintPlan | null>(null);
  const [loaded, setLoaded] = useState(0);
  const [failed, setFailed] = useState<number | null>(null);
  const printed = useRef(false);

  // Validate a typed range with the engine's parser (same rules as every page tool).
  const debounced = useDebounced(spec, 200);
  const [range, setRange] = useState<Range>({ state: "idle" });
  useEffect(() => {
    if (which !== "range") return setRange({ state: "idle" });
    if (!debounced.trim()) return setRange({ state: "error", message: "Enter at least one page, for example 1-3, 5." });
    let live = true;
    setRange({ state: "checking" });
    api
      .validateRange(doc.doc_id, debounced)
      .then((pages) => live && setRange({ state: "ok", pages }))
      .catch((e) => live && setRange({ state: "error", message: toPwError(e).message }));
    return () => {
      live = false;
    };
  }, [api, doc.doc_id, debounced, which]);

  const indexes = useMemo(() => {
    if (which === "all") return doc.info.pages.map((p) => p.index);
    if (which === "current") return [currentPage];
    return range.state === "ok" && spec === debounced ? range.pages : null;
  }, [which, doc.info.pages, currentPage, range, spec, debounced]);
  const dpi = PRINT_QUALITIES.find((q) => q.id === quality)!.dpi;
  const preview = indexes ? printPlan(doc.info.pages, indexes, dpi) : null;
  const tooLarge = preview !== null && preview.megapixels > MAX_PRINT_MEGAPIXELS;
  const blocker =
    which === "range" && range.state === "error"
      ? range.message
      : indexes === null
        ? "Checking the pages…"
        : tooLarge
          ? "Too much to prepare at once: choose fewer pages or a lower resolution."
          : null;
  const preparing = plan !== null;

  // Once every page image has loaded, open the print UI; clean up when it closes.
  useEffect(() => {
    if (!plan || printed.current || loaded < plan.sheets.length) return;
    printed.current = true;
    const done = () => onClose();
    window.addEventListener("afterprint", done, { once: true });
    requestAnimationFrame(() => window.print());
    return () => window.removeEventListener("afterprint", done);
  }, [plan, loaded, onClose]);

  const start = () => {
    if (blocker || preparing || !preview) return;
    printed.current = false;
    setLoaded(0);
    setFailed(null);
    setPlan(preview);
  };

  const cancel = () => {
    setPlan(null);
    setLoaded(0);
    printed.current = false;
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={preparing ? cancel : onClose}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="print-start" aria-disabled={blocker || preparing ? true : undefined} title={blocker ?? undefined} onClick={start}>
        {preparing ? "Preparing…" : "Print…"}
      </button>
    </>
  );

  return (
    <Dialog title="Print" onClose={onClose} footer={footer}>
      <fieldset className="field" style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }} disabled={preparing}>
        <legend className="label">Pages</legend>
        <label className="row">
          <input type="radio" name="print-pages" checked={which === "all"} onChange={() => setWhich("all")} /> All ({doc.info.page_count})
        </label>
        <label className="row">
          <input type="radio" name="print-pages" checked={which === "current"} onChange={() => setWhich("current")} /> Current page ({currentPage + 1})
        </label>
        <label className="row">
          <input type="radio" name="print-pages" data-testid="print-range" checked={which === "range"} onChange={() => setWhich("range")} /> Pages
        </label>
        {which === "range" && (
          <>
            <input id="print-pages" className="input" aria-label="Pages to print" value={spec} placeholder="e.g. 1-3, 5, 8-" autoComplete="off" aria-invalid={range.state === "error"} aria-describedby="print-pages-msg" onChange={(e) => setSpec(e.target.value)} />
            <span id="print-pages-msg" className={range.state === "error" ? "error" : "hint"} aria-live="polite">
              {range.state === "error" ? range.message : range.state === "ok" ? `${range.pages.length} page${range.pages.length === 1 ? "" : "s"} selected` : "Use commas and ranges; “8-” means 8 to the end."}
            </span>
          </>
        )}
      </fieldset>
      <div className="field">
        <label htmlFor="print-quality">Resolution</label>
        <select id="print-quality" className="select" value={quality} disabled={preparing} onChange={(e) => setQuality(e.target.value as PrintQuality)}>
          {PRINT_QUALITIES.map((q) => (
            <option key={q.id} value={q.id}>
              {q.label}
            </option>
          ))}
        </select>
        <span className="hint">Pages print as they look on screen. Printer, copies, paper and two-sided options are in the next window.</span>
      </div>
      {preparing && (
        <div aria-live="polite">
          <div className="hint">
            Preparing page {Math.min(loaded + 1, plan.sheets.length)} of {plan.sheets.length}…
          </div>
          <div className="progress" role="progressbar" aria-label="Preparing pages" aria-valuemin={0} aria-valuemax={plan.sheets.length} aria-valuenow={loaded}>
            <div style={{ width: `${(100 * loaded) / Math.max(1, plan.sheets.length)}%` }} />
          </div>
        </div>
      )}
      {failed !== null && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          Page {failed + 1} could not be prepared for printing. Nothing was sent to the printer.
        </div>
      )}
      {plan &&
        createPortal(
          <div id="print-root" aria-hidden="true">
            <style id="print-css">{plan.css}</style>
            {plan.sheets.map((s) => (
              <div key={s.index} className="print-sheet" style={{ width: `${s.widthPt}pt`, height: `${s.heightPt}pt`, ["page" as string]: s.pageName }}>
                <img
                  src={api.pageUrl(doc.doc_id, s.index, plan.scalePermille)}
                  alt=""
                  onLoad={() => setLoaded((n) => n + 1)}
                  onError={() => {
                    setFailed(s.index);
                    setPlan(null);
                  }}
                />
              </div>
            ))}
          </div>,
          document.body,
        )}
    </Dialog>
  );
}

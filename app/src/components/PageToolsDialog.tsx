import { useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, siblingOutput } from "../paths";
import { toPwError, type AnnotationSpec, type CollisionPolicy, type FieldValue, type StampPosition, type JobSnapshot, type WatermarkLayer, type OpenedDoc, type Operation, type PwError } from "../types";
import { Dialog, useDebounced } from "./ui";

export type PageTool = "extract" | "delete" | "rotate" | "split" | "merge" | "save" | "stamp" | "images" | "ocr" | "watermark" | "crop";

const TITLES: Record<PageTool, string> = {
  extract: "Extract pages",
  delete: "Delete pages",
  rotate: "Rotate pages",
  split: "Split document",
  merge: "Combine files",
  save: "Save a copy with your changes",
  stamp: "Page numbers and stamps",
  images: "Export pages as images",
  ocr: "Recognize text (OCR)",
  watermark: "Watermark",
  crop: "Crop pages",
};

/** Start-button label (the title's first word does not fit every tool). */
const START: Partial<Record<PageTool, string>> = { ocr: "Recognize", watermark: "Add watermark", crop: "Crop" };

const SUFFIX: Record<PageTool, string> = {
  extract: "extract",
  delete: "edited",
  rotate: "rotated",
  split: "",
  merge: "combined",
  save: "edited",
  stamp: "numbered",
  images: "",
  ocr: "searchable",
  watermark: "watermarked",
  crop: "cropped",
};

export type CropUnit = "mm" | "in" | "pt";
/** Points per unit. */
export const UNIT_PT: Record<CropUnit, number> = { mm: 72 / 25.4, in: 72, pt: 1 };

/** Displayed size of a page in points (rotation swaps the sides). */
export function displayedSize(p: { width_pt: number; height_pt: number; rotation: number }): [number, number] {
  return p.rotation === 90 || p.rotation === 270 ? [p.height_pt, p.width_pt] : [p.width_pt, p.height_pt];
}

/** Watermark angles as displayed (degrees counter-clockwise). */
const ANGLES: Array<[number, string]> = [
  [45, "Diagonal up"],
  [-45, "Diagonal down"],
  [0, "Horizontal"],
  [90, "Vertical"],
];

const DPI_CHOICES = [72, 96, 150, 300, 600];

const POSITIONS: Array<[StampPosition, string]> = [
  ["bottom_center", "Bottom center"],
  ["bottom_right", "Bottom right"],
  ["bottom_left", "Bottom left"],
  ["top_center", "Top center"],
  ["top_right", "Top right"],
  ["top_left", "Top left"],
];

/** Mirrors pw-core `stamp_text_for` (preview only; the engine expands the real text). */
export function expandStamp(template: string, n: number, digits: number, total: number): string {
  return template.replaceAll("{page}", String(n).padStart(digits, "0")).replaceAll("{total}", String(total));
}

/** Mirrors the engine's character check: printable ASCII and Latin-1. */
export function stampCharsOk(text: string): boolean {
  return [...text].every((c) => (c >= " " && c <= "~") || (c >= "\u00a0" && c <= "\u00ff"));
}

interface Props {
  tool: PageTool;
  doc: OpenedDoc;
  initialPages?: string;
  /** For the save tool: pending annotations and changed form values to write into the copy. */
  annotations?: AnnotationSpec[];
  formValues?: FieldValue[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

type RangeCheck = { state: "idle" | "checking" } | { state: "ok"; count: number } | { state: "error"; message: string };

function useRangeCheck(docId: number, spec: string, enabled: boolean): RangeCheck {
  const api = useApi();
  const debounced = useDebounced(spec, 200);
  const [check, setCheck] = useState<RangeCheck>({ state: "idle" });
  useEffect(() => {
    if (!enabled) return setCheck({ state: "idle" });
    if (!debounced.trim()) return setCheck({ state: "error", message: "Enter at least one page, for example 1-3, 5." });
    let live = true;
    setCheck({ state: "checking" });
    api
      .validateRange(docId, debounced)
      .then((pages) => live && setCheck({ state: "ok", count: pages.length }))
      .catch((e) => live && setCheck({ state: "error", message: toPwError(e).message }));
    return () => {
      live = false;
    };
  }, [api, docId, debounced, enabled]);
  // Treat the undebounced value as "checking" so Start can't run on a stale validation.
  if (enabled && spec !== debounced) return { state: "checking" };
  return check;
}

export function PageToolsDialog({ tool, doc, initialPages, annotations, formValues, jobs, onClose, onFinished }: Props) {
  const nAnnots = annotations?.length ?? 0;
  const nFields = formValues?.length ?? 0;
  // "-annotated" / "-filled" when only one kind of change is saved, "-edited" for both.
  const suffix = tool === "save" ? (nFields === 0 ? "annotated" : nAnnots === 0 ? "filled" : "edited") : SUFFIX[tool];
  const api = useApi();
  const [pages, setPages] = useState(initialPages ?? (tool === "rotate" ? "" : ""));
  const [allPages, setAllPages] = useState((tool === "rotate" || tool === "stamp" || tool === "images" || tool === "ocr" || tool === "watermark" || tool === "crop") && !initialPages);
  const [includeTextPages, setIncludeTextPages] = useState(false);
  const [dpi, setDpi] = useState(150);
  /** Tools that write several files into a folder rather than one new PDF. */
  const toFolder = tool === "split" || tool === "images";
  const [stampText, setStampText] = useState("Page {page} of {total}");
  const [position, setPosition] = useState<StampPosition>("bottom_center");
  const [fontSize, setFontSize] = useState("10");
  const [startAt, setStartAt] = useState("1");
  const [digits, setDigits] = useState("0");
  const [degrees, setDegrees] = useState(90);
  const [wmText, setWmText] = useState("CONFIDENTIAL");
  const [wmSize, setWmSize] = useState("72");
  const [wmColor, setWmColor] = useState("#808080");
  const [wmOpacity, setWmOpacity] = useState(25);
  const [wmAngle, setWmAngle] = useState(45);
  const [wmLayer, setWmLayer] = useState<WatermarkLayer>("over");
  const [cropUnit, setCropUnit] = useState<CropUnit>("mm");
  const [crop, setCrop] = useState({ top: "0", right: "0", bottom: "0", left: "0" });
  const [output, setOutput] = useState(tool === "split" || tool === "images" ? dirname(doc.path) : siblingOutput(doc.path, suffix));
  const [splitMode, setSplitMode] = useState<"every_n" | "ranges">("every_n");
  const [everyN, setEveryN] = useState("1");
  const [ranges, setRanges] = useState("");
  const [files, setFiles] = useState<string[]>([doc.path]);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);

  const needsRange = tool === "extract" || tool === "delete" || ((tool === "rotate" || tool === "stamp" || tool === "images" || tool === "ocr" || tool === "watermark" || tool === "crop") && !allPages);
  const wmSizeN = Number(wmSize);
  const cropPt = {
    top: Number(crop.top) * UNIT_PT[cropUnit],
    right: Number(crop.right) * UNIT_PT[cropUnit],
    bottom: Number(crop.bottom) * UNIT_PT[cropUnit],
    left: Number(crop.left) * UNIT_PT[cropUnit],
  };
  const firstPage = doc.info.pages[0];
  const [pw, ph] = firstPage ? displayedSize(firstPage) : [612, 792];
  const sizeN = Number(fontSize);
  const startN = Number(startAt);
  const digitsN = Number(digits);
  const range = useRangeCheck(doc.doc_id, pages, needsRange);
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
      setError(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const everyNValue = Number(everyN);
  const everyNValid = Number.isInteger(everyNValue) && everyNValue >= 1 && everyNValue <= doc.info.page_count;
  const rangeLines = useMemo(() => ranges.split(/\r?\n/).map((s) => s.trim()).filter(Boolean), [ranges]);

  const blocker: string | null = (() => {
    if (needsRange && range.state !== "ok") return range.state === "error" ? range.message : "Checking pages…";
    if (tool === "delete" && range.state === "ok" && range.count >= doc.info.page_count) return "A PDF must keep at least one page.";
    if (tool === "split" && splitMode === "every_n" && !everyNValid) return `Enter a number from 1 to ${doc.info.page_count}.`;
    if (tool === "split" && splitMode === "ranges" && rangeLines.length === 0) return "Enter one page range per line.";
    if (tool === "merge" && files.length < 2) return "Add at least one more file.";
    if (tool === "save" && nAnnots + nFields === 0) return "There are no changes to save.";
    if (tool === "stamp") {
      if (!stampText.trim()) return "Enter the text to stamp.";
      if (!stampCharsOk(stampText)) return "Stamps support Latin characters only for now.";
      if (!(sizeN >= 4 && sizeN <= 72)) return "Use a text size from 4 to 72.";
      if (!Number.isInteger(startN) || startN < 0) return "Start at must be a whole number.";
      if (!Number.isInteger(digitsN) || digitsN < 0 || digitsN > 12) return "Digits must be 0 to 12.";
    }
    if (tool === "crop") {
      const v = Object.values(cropPt);
      if (v.some((x) => !Number.isFinite(x) || x < 0)) return "Margins must be zero or more.";
      if (v.every((x) => x === 0)) return "Enter at least one margin to crop.";
      if (cropPt.left + cropPt.right > pw - 18 || cropPt.top + cropPt.bottom > ph - 18) return "The margins leave almost nothing of the page.";
    }
    if (tool === "watermark") {
      if (!wmText.trim()) return "Enter the watermark text.";
      if (!stampCharsOk(wmText)) return "Watermarks support Latin characters only for now.";
      if (!(wmSizeN >= 8 && wmSizeN <= 400)) return "Use a text size from 8 to 400.";
    }
    if (!output.trim()) return toFolder ? "Choose an output folder." : "Choose where to save the new file.";
    return null;
  })();

  const buildOperation = (): Operation => {
    switch (tool) {
      case "extract":
        return { op: "extract_pages", input: doc.path, pages, output };
      case "delete":
        return { op: "delete_pages", input: doc.path, pages, output };
      case "rotate":
        return { op: "rotate_pages", input: doc.path, pages: allPages ? null : pages, degrees, output };
      case "split":
        return {
          op: "split",
          input: doc.path,
          split: splitMode === "every_n" ? { mode: "every_n", n: everyNValue } : { mode: "ranges", ranges: rangeLines },
          output_dir: output,
        };
      case "merge":
        return { op: "merge", inputs: files, output };
      case "images":
        return { op: "export_images", input: doc.path, pages: allPages ? null : pages, dpi, output_dir: output };
      case "ocr":
        return { op: "ocr_pages", input: doc.path, pages: allPages ? null : pages, skip_text_pages: !includeTextPages, dpi: 200, output };
      case "stamp":
        return {
          op: "stamp_text",
          input: doc.path,
          pages: allPages ? null : pages,
          stamps: [{ text: stampText, position, font_size: sizeN, color: "#000000", margin: 28 }],
          start: startN,
          digits: digitsN,
          output,
        };
      case "crop":
        return { op: "crop_pages", input: doc.path, pages: allPages ? null : pages, margins: cropPt, output };
      case "watermark":
        return {
          op: "watermark",
          input: doc.path,
          pages: allPages ? null : pages,
          watermark: { text: wmText, font_size: wmSizeN, color: wmColor, opacity: wmOpacity / 100, angle: wmAngle, layer: wmLayer },
          output,
        };
      case "save":
        // Only the contract fields: UI bookkeeping (ids) stays in the UI.
        return {
          op: "apply_edits",
          input: doc.path,
          form_values: formValues ?? [],
          annotations: (annotations ?? []).map(({ page, kind, rects, color, contents }) => ({ page, kind, rects, color, contents: contents ?? null })),
          output,
        };
    }
  };

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const id = await api.startOperation({ schema_version: 1, operation: buildOperation(), collision }, doc.doc_id);
      setJobId(id);
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const browse = async () => {
    if (toFolder) {
      const dir = await api.pickFolder(output || dirname(doc.path));
      if (dir) setOutput(dir);
    } else {
      const p = await api.pickSavePath(output);
      if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
    }
  };

  const progress = job?.progress;
  const footer = (
    <>
      {busy ? (
        <button type="button" className="btn outline" onClick={() => jobId && api.cancelJob(jobId)} disabled={job?.state === "cancelling"}>
          {job?.state === "cancelling" ? "Cancelling…" : "Cancel task"}
        </button>
      ) : (
        <button type="button" className="btn outline" onClick={onClose}>
          Cancel
        </button>
      )}
      <button
        type="button"
        className="btn primary"
        data-testid="tool-start"
        aria-disabled={blocker || busy ? true : undefined}
        title={blocker ?? undefined}
        onClick={() => start("fail")}
      >
        {busy ? "Working…" : START[tool] ?? TITLES[tool].split(" ")[0]}
      </button>
    </>
  );

  return (
    <Dialog title={TITLES[tool]} onClose={onClose} footer={footer} busy={busy} wide={tool === "merge"}>
      {tool === "save" ? (
        <p className="hint" style={{ marginTop: 0 }}>
          Saves {[nFields ? `${nFields} filled field${nFields === 1 ? "" : "s"}` : "", nAnnots ? `${nAnnots} annotation${nAnnots === 1 ? "" : "s"}` : ""].filter(Boolean).join(" and ")} into a copy of{" "}
          <strong>{doc.name}</strong>. The original file is not changed.
        </p>
      ) : (
        tool !== "merge" && (
          <p className="hint" style={{ marginTop: 0 }}>
            From <strong>{doc.name}</strong> ({doc.info.page_count} pages). The original file is not changed.
          </p>
        )
      )}

      {tool === "rotate" && (
        <div className="field">
          <span className="label" id="rot-label">Direction</span>
          <div className="segmented" role="radiogroup" aria-labelledby="rot-label">
            {[
              [90, "90° right"],
              [180, "180°"],
              [-90, "90° left"],
            ].map(([d, l]) => (
              <button key={d} type="button" role="radio" aria-checked={degrees === d} onClick={() => setDegrees(d as number)}>
                {l}
              </button>
            ))}
          </div>
        </div>
      )}

      {tool === "stamp" && (
        <>
          <div className="field">
            <label htmlFor="stamp-text">Text</label>
            <input id="stamp-text" className="input" value={stampText} onChange={(e) => setStampText(e.target.value)} aria-describedby="stamp-help" spellCheck={false} />
            <span id="stamp-help" className="hint">
              {"{page}"} is the running number, {"{total}"} the number of pages. Example: “{expandStamp(stampText, Number.isInteger(startN) ? startN : 1, Number.isInteger(digitsN) ? Math.min(12, Math.max(0, digitsN)) : 0, doc.info.page_count)}”
            </span>
          </div>
          <div className="row" style={{ gap: "var(--space-4)", alignItems: "flex-start" }}>
            <div className="field">
              <label htmlFor="stamp-position">Position</label>
              <select id="stamp-position" className="select" value={position} onChange={(e) => setPosition(e.target.value as StampPosition)}>
                {POSITIONS.map(([v, l]) => (
                  <option key={v} value={v}>
                    {l}
                  </option>
                ))}
              </select>
            </div>
            <div className="field">
              <label htmlFor="stamp-size">Size (pt)</label>
              <input id="stamp-size" className="input" type="number" min={4} max={72} style={{ width: "7ch" }} value={fontSize} onChange={(e) => setFontSize(e.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="stamp-start">Start at</label>
              <input id="stamp-start" className="input" type="number" min={0} style={{ width: "9ch" }} value={startAt} onChange={(e) => setStartAt(e.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="stamp-digits">Digits</label>
              <input id="stamp-digits" className="input" type="number" min={0} max={12} style={{ width: "6ch" }} value={digits} onChange={(e) => setDigits(e.target.value)} title="Zero-pad numbers to this many digits (for Bates numbering)" />
            </div>
          </div>
        </>
      )}

      {tool === "watermark" && (
        <>
          <div className="row" style={{ gap: "var(--space-4)", alignItems: "flex-start" }}>
            <div className="grow" style={{ display: "grid", gap: "var(--space-3)" }}>
              <div className="field">
                <label htmlFor="wm-text">Text</label>
                <input id="wm-text" className="input" value={wmText} onChange={(e) => setWmText(e.target.value)} spellCheck={false} />
              </div>
              <div className="row" style={{ gap: "var(--space-4)" }}>
                <div className="field">
                  <label htmlFor="wm-size">Size (pt)</label>
                  <input id="wm-size" className="input" type="number" min={8} max={400} style={{ width: "8ch" }} value={wmSize} onChange={(e) => setWmSize(e.target.value)} />
                </div>
                <div className="field">
                  <label htmlFor="wm-color">Color</label>
                  <input id="wm-color" className="input" type="color" style={{ width: 48, padding: 2 }} value={wmColor} onChange={(e) => setWmColor(e.target.value)} />
                </div>
                <div className="field grow">
                  <label htmlFor="wm-opacity">Opacity {wmOpacity}%</label>
                  <input id="wm-opacity" type="range" min={5} max={100} step={5} value={wmOpacity} onChange={(e) => setWmOpacity(Number(e.target.value))} />
                </div>
              </div>
            </div>
            <svg className="wm-preview" viewBox="0 0 96 124" role="img" aria-label={`Preview: “${wmText}” at ${wmAngle}°, ${wmOpacity}% opacity`} data-testid="wm-preview">
              <rect x="0" y="0" width="96" height="124" fill="#fff" />
              <text
                x="48"
                y="62"
                textAnchor="middle"
                dominantBaseline="middle"
                transform={`rotate(${-wmAngle} 48 62)`}
                fill={wmColor}
                fillOpacity={wmOpacity / 100}
                fontSize={Math.max(4, Math.min(40, (wmSizeN || 72) / 6))}
                fontFamily="Helvetica, Arial, sans-serif"
              >
                {wmText}
              </text>
            </svg>
          </div>
          <div className="field">
            <span className="label" id="wm-angle-label">Angle</span>
            <div className="segmented" role="radiogroup" aria-labelledby="wm-angle-label">
              {ANGLES.map(([a, l]) => (
                <button key={a} type="button" role="radio" aria-checked={wmAngle === a} data-angle={a} onClick={() => setWmAngle(a)}>
                  {l}
                </button>
              ))}
            </div>
          </div>
          <div className="field">
            <span className="label" id="wm-layer-label">Place</span>
            <div className="segmented" role="radiogroup" aria-labelledby="wm-layer-label">
              <button type="button" role="radio" aria-checked={wmLayer === "over"} data-layer="over" onClick={() => setWmLayer("over")}>
                Over the content
              </button>
              <button type="button" role="radio" aria-checked={wmLayer === "behind"} data-layer="behind" onClick={() => setWmLayer("behind")} title="Hidden where the page is covered, for example by a scanned image">
                Behind the content
              </button>
            </div>
          </div>
        </>
      )}

      {tool === "crop" && (
        <div className="row" style={{ gap: "var(--space-4)", alignItems: "flex-start" }}>
          <div className="grow" style={{ display: "grid", gap: "var(--space-3)" }}>
            <div className="crop-grid">
              {(["top", "left", "right", "bottom"] as const).map((side) => (
                <div className="field" key={side} style={{ gridArea: side }}>
                  <label htmlFor={`crop-${side}`}>{side[0]!.toUpperCase() + side.slice(1)}</label>
                  <input id={`crop-${side}`} className="input" type="number" min={0} step="any" style={{ width: "9ch" }} value={crop[side]} onChange={(e) => setCrop((c) => ({ ...c, [side]: e.target.value }))} />
                </div>
              ))}
            </div>
            <div className="field">
              <label htmlFor="crop-unit">Unit</label>
              <select id="crop-unit" className="select" style={{ width: "16ch" }} value={cropUnit} onChange={(e) => setCropUnit(e.target.value as CropUnit)}>
                <option value="mm">Millimetres</option>
                <option value="in">Inches</option>
                <option value="pt">Points</option>
              </select>
            </div>
            <span className="hint">Hidden content stays in the file; use Redact to remove it.</span>
          </div>
          {firstPage && (
            <div className="crop-preview" role="img" aria-label="Preview of page 1 with the cropped margins shaded" data-testid="crop-preview" style={{ width: 120, height: (120 * ph) / pw }}>
              <img src={api.pageUrl(doc.doc_id, 0, Math.max(50, Math.round((120 / pw) * 1000 * (window.devicePixelRatio || 1))))} alt="" draggable={false} />
              {(() => {
                const k = 120 / pw;
                const c = (v: number) => Math.max(0, Number.isFinite(v) ? v * k : 0);
                return (
                  <>
                    <div className="cut" style={{ left: 0, top: 0, right: 0, height: c(cropPt.top) }} />
                    <div className="cut" style={{ left: 0, bottom: 0, right: 0, height: c(cropPt.bottom) }} />
                    <div className="cut" style={{ left: 0, top: c(cropPt.top), bottom: c(cropPt.bottom), width: c(cropPt.left) }} />
                    <div className="cut" style={{ right: 0, top: c(cropPt.top), bottom: c(cropPt.bottom), width: c(cropPt.right) }} />
                  </>
                );
              })()}
            </div>
          )}
        </div>
      )}

      {tool === "images" && (
        <div className="field">
          <label htmlFor="export-dpi">Resolution</label>
          <select id="export-dpi" className="select" value={dpi} onChange={(e) => setDpi(Number(e.target.value))}>
            {DPI_CHOICES.map((d) => (
              <option key={d} value={d}>
                {d} dpi{d === 72 ? " (100%)" : d === 150 ? " (screen)" : d === 300 ? " (print)" : ""}
              </option>
            ))}
          </select>
        </div>
      )}

      {tool === "ocr" && (
        <p className="hint" style={{ marginTop: 0 }}>
          Adds selectable, searchable text to scanned pages in a new copy. The pages look exactly the same.
        </p>
      )}

      {(tool === "rotate" || tool === "stamp" || tool === "images" || tool === "ocr" || tool === "watermark" || tool === "crop") && (
        <div className="field">
          <label className="row">
            <input type="checkbox" checked={allPages} onChange={(e) => setAllPages(e.target.checked)} /> All pages
          </label>
        </div>
      )}

      {tool === "ocr" && (
        <div className="field">
          <label className="row">
            <input type="checkbox" checked={includeTextPages} onChange={(e) => setIncludeTextPages(e.target.checked)} data-testid="ocr-include-text-pages" /> Also process pages that already have text
          </label>
        </div>
      )}

      {needsRange && (
        <div className="field">
          <label htmlFor="pages">Pages</label>
          <input
            id="pages"
            className="input"
            value={pages}
            placeholder="e.g. 1-3, 5, 8-"
            autoComplete="off"
            aria-invalid={range.state === "error"}
            aria-describedby="pages-msg"
            onChange={(e) => setPages(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && start("fail")}
          />
          <span id="pages-msg" className={range.state === "error" ? "error" : "hint"} aria-live="polite">
            {range.state === "error" ? range.message : range.state === "ok" ? `${range.count} page${range.count === 1 ? "" : "s"} selected` : "Use commas and ranges; “8-” means 8 to the end."}
          </span>
        </div>
      )}

      {tool === "split" && (
        <>
          <div className="field">
            <span className="label" id="split-label">Split</span>
            <div className="segmented" role="radiogroup" aria-labelledby="split-label">
              <button type="button" role="radio" aria-checked={splitMode === "every_n"} onClick={() => setSplitMode("every_n")}>
                Every N pages
              </button>
              <button type="button" role="radio" aria-checked={splitMode === "ranges"} onClick={() => setSplitMode("ranges")}>
                By ranges
              </button>
            </div>
          </div>
          {splitMode === "every_n" ? (
            <div className="field">
              <label htmlFor="every">Pages per file</label>
              <input id="every" className="input" type="number" min={1} max={doc.info.page_count} value={everyN} aria-invalid={!everyNValid} onChange={(e) => setEveryN(e.target.value)} />
              <span className="hint">{everyNValid ? `Creates ${Math.ceil(doc.info.page_count / everyNValue)} files.` : ""}</span>
            </div>
          ) : (
            <div className="field">
              <label htmlFor="ranges">Ranges, one file per line</label>
              <textarea id="ranges" className="input" style={{ height: 96, paddingTop: 6 }} value={ranges} placeholder={"1-3\n4-"} onChange={(e) => setRanges(e.target.value)} />
            </div>
          )}
        </>
      )}

      {tool === "merge" && (
        <div className="field">
          <span className="label">Files, in order</span>
          <ul className="file-list" aria-label="Files to combine">
            {files.map((f, i) => (
              <li key={f + i}>
                <span className="name" title={f}>
                  {i + 1}. {basename(f)}
                </span>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} up`} title="Move up" aria-disabled={i === 0 || undefined} onClick={() => i > 0 && setFiles((fs) => swap(fs, i, i - 1))}>
                  <ArrowUp />
                </button>
                <button type="button" className="btn icon" aria-label={`Move ${basename(f)} down`} title="Move down" aria-disabled={i === files.length - 1 || undefined} onClick={() => i < files.length - 1 && setFiles((fs) => swap(fs, i, i + 1))}>
                  <ArrowDown />
                </button>
                <button type="button" className="btn icon" aria-label={`Remove ${basename(f)} from the list`} title="Remove from list (the file is not deleted)" onClick={() => setFiles((fs) => fs.filter((_, j) => j !== i))}>
                  <Trash2 />
                </button>
              </li>
            ))}
          </ul>
          <div>
            <button
              type="button"
              className="btn outline"
              onClick={async () => {
                const picked = await api.pickPdfFiles(true);
                if (picked.length) setFiles((fs) => [...fs, ...picked]);
              }}
            >
              <Plus /> Add files…
            </button>
          </div>
        </div>
      )}

      <div className="field">
        <label htmlFor="output">{tool === "split" ? "Save parts in folder" : tool === "images" ? "Save images in folder" : "Save as"}</label>
        <div className="row">
          <input id="output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
        {tool === "split" && <span className="hint">Files are named “{doc.name.replace(/\.pdf$/i, "")}_part01.pdf”, …</span>}
        {tool === "images" && <span className="hint">PNG files named “{doc.name.replace(/\.pdf$/i, "")}_p001.png”, …</span>}
      </div>

      {busy && progress && (
        <div aria-live="polite">
          <div className="hint">{job?.state === "cancelling" ? "Cancelling…" : progress.kind === "determinate" ? `${progress.done} of ${progress.total} ${progress.unit}` : "Working…"}</div>
          <div className={`progress ${progress.kind === "indeterminate" ? "indeterminate" : ""}`} role="progressbar" aria-label="Task progress" aria-valuemin={0} aria-valuemax={progress.kind === "determinate" ? progress.total : undefined} aria-valuenow={progress.kind === "determinate" ? progress.done : undefined}>
            <div style={progress.kind === "determinate" ? { width: `${(100 * progress.done) / Math.max(1, progress.total)}%` } : undefined} />
          </div>
        </div>
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

function swap<T>(a: T[], i: number, j: number): T[] {
  const b = a.slice();
  [b[i], b[j]] = [b[j]!, b[i]!];
  return b;
}

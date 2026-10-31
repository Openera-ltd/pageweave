import { useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, FolderOpen, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { basename, dirname, join, siblingOutput, stem } from "../paths";
import { toPwError, type AnnotationChange, type AnnotationSpec, type BookmarkInfo, type CollisionPolicy, type FieldValue, type PageEdit, type StampPosition, type JobSnapshot, type WatermarkLayer, type OpenedDoc, type Operation, type PwError, type TranslationDirection } from "../types";
import { Dialog, useDebounced } from "./ui";
import { languageName, languages, route } from "../translate";
import { TranslationDownload } from "./TranslationDownload";

export type PageTool = "extract" | "delete" | "rotate" | "split" | "merge" | "save" | "stamp" | "images" | "ocr" | "watermark" | "crop" | "text" | "word" | "slides" | "audio" | "translate" | "tables" | "optimize" | "deskew" | "extractImages" | "orient";

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
  text: "Export as text",
  word: "Convert to Word",
  slides: "Convert to PowerPoint",
  audio: "Save as audio",
  translate: "Translate",
  tables: "Export tables",
  optimize: "Reduce file size",
  deskew: "Straighten pages",
  extractImages: "Extract images",
  orient: "Turn pages upright",
};

/** Start-button label (the title's first word does not fit every tool). */
const START: Partial<Record<PageTool, string>> = { translate: "Translate", audio: "Save audio", slides: "Convert", tables: "Export", word: "Convert", ocr: "Recognize", watermark: "Add watermark", crop: "Crop", text: "Export", optimize: "Reduce size", deskew: "Straighten", extractImages: "Extract", orient: "Turn upright" };

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
  text: "",
  word: "",
  slides: "",
  audio: "",
  translate: "",
  tables: "",
  optimize: "smaller",
  deskew: "straightened",
  extractImages: "",
  orient: "upright",
};

/** Tools that offer "All pages" (on by default unless pages were preselected). */
const ALL_PAGES_TOOLS: PageTool[] = ["rotate", "stamp", "images", "ocr", "watermark", "crop", "text", "word", "slides", "audio", "translate", "tables", "deskew", "extractImages", "orient"];

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

/** Image reduction presets for "Reduce file size": resolution and JPEG quality. */
export const IMAGE_PRESETS: Array<{ id: string; label: string; max_dpi: number; quality: number }> = [
  { id: "screen", label: "Screen (96 dpi)", max_dpi: 96, quality: 60 },
  { id: "standard", label: "Standard (150 dpi)", max_dpi: 150, quality: 75 },
  { id: "print", label: "Print (300 dpi)", max_dpi: 300, quality: 85 },
];

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
  /** For the save tool: edits to existing text and pictures (Edit mode). */
  objectEdits?: Array<{ page: number; edits: PageEdit[] }>;
  /** Deletes / text changes of annotations already in the file. */
  annotationChanges?: AnnotationChange[];
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
  /** For the audio tool: the read-aloud voice (null = default) and speed. */
  readVoice?: string | null;
  readRate?: number;
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

export function PageToolsDialog({ tool, doc, initialPages, annotations, formValues, objectEdits, annotationChanges, jobs, onClose, onFinished, readVoice = null, readRate = 1 }: Props) {
  const nAnnots = annotations?.length ?? 0;
  const nFields = formValues?.length ?? 0;
  const nObjects = (objectEdits ?? []).reduce((n, p) => n + p.edits.length, 0);
  const nComments = annotationChanges?.length ?? 0;
  // "-annotated" / "-filled" when only one kind of change is saved, "-edited" otherwise.
  const suffix = tool === "save" ? (nObjects === 0 && nFields === 0 ? "annotated" : nObjects === 0 && nAnnots === 0 ? "filled" : "edited") : SUFFIX[tool];
  const api = useApi();
  const [pages, setPages] = useState(initialPages ?? (tool === "rotate" ? "" : ""));
  const [allPages, setAllPages] = useState(ALL_PAGES_TOOLS.includes(tool) && !initialPages);
  const [includeTextPages, setIncludeTextPages] = useState(false);
  const [dpi, setDpi] = useState(150);
  const [reduceImages, setReduceImages] = useState(false);
  const [sheetFormat, setSheetFormat] = useState<"xlsx" | "csv">("xlsx");
  // Translate: languages (source null = detect), output format, and the installed directions.
  const [trFrom, setTrFrom] = useState<string | null>(null);
  const [trTo, setTrTo] = useState("en");
  const [trFormat, setTrFormat] = useState<"pdf" | "docx" | "txt">("pdf");
  /** PDF output: each original page next to its translation. */
  const [trSideBySide, setTrSideBySide] = useState(false);
  const [trDirs, setTrDirs] = useState<TranslationDirection[] | null>(null);
  const [trDirsError, setTrDirsError] = useState<string | null>(null);
  const [trDetected, setTrDetected] = useState<string | null | undefined>(undefined);
  const [trDirsNonce, setTrDirsNonce] = useState(0);
  useEffect(() => {
    if (tool !== "translate") return;
    let live = true;
    api.translationDirections().then(
      (d) => live && setTrDirs(d),
      (e) => live && setTrDirsError(toPwError(e).message),
    );
    return () => {
      live = false;
    };
  }, [api, tool, trDirsNonce]);
  // The document's language, for "Detect automatically" (null: could not tell).
  useEffect(() => {
    if (tool !== "translate") return;
    let live = true;
    api.detectLanguage(doc.doc_id).then(
      (l) => live && setTrDetected(l),
      () => live && setTrDetected(null),
    );
    return () => {
      live = false;
    };
  }, [api, tool, doc.doc_id]);
  const trSource = trFrom ?? trDetected ?? null;
  const trRoute = trDirs && trSource ? route(trDirs, trSource, trTo) : null;
  const [imagePreset, setImagePreset] = useState("standard");
  /** Tools that write several files into a folder rather than one new PDF. */
  const toFolder = tool === "split" || tool === "images" || tool === "extractImages" || (tool === "tables" && sheetFormat === "csv");
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
  const [output, setOutput] = useState(
    toFolder
      ? dirname(doc.path)
      : tool === "text"
        ? join(dirname(doc.path), `${stem(doc.path)}.txt`)
        : tool === "word"
          ? join(dirname(doc.path), `${stem(doc.path)}.docx`)
          : tool === "slides"
            ? join(dirname(doc.path), `${stem(doc.path)}.pptx`)
          : tool === "audio"
            ? join(dirname(doc.path), `${stem(doc.path)}.wav`)
          : tool === "translate"
            ? join(dirname(doc.path), `${stem(doc.path)}-en.pdf`)
          : tool === "tables"
            ? join(dirname(doc.path), `${stem(doc.path)}.xlsx`)
            : siblingOutput(doc.path, suffix),
  );
  const [splitMode, setSplitMode] = useState<"every_n" | "ranges" | "bookmarks" | "max_size">("every_n");
  const [bmLevel, setBmLevel] = useState(1);
  const [maxSize, setMaxSize] = useState("10");
  const [sizeUnit, setSizeUnit] = useState<"KB" | "MB">("MB");
  const [everyN, setEveryN] = useState("1");
  const [ranges, setRanges] = useState("");
  const [files, setFiles] = useState<string[]>([doc.path]);
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);

  const needsRange = tool === "extract" || tool === "delete" || (ALL_PAGES_TOOLS.includes(tool) && !allPages);
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
  const bmParts = useMemo(() => bookmarkParts(doc.info.bookmarks ?? [], doc.info.page_count, bmLevel), [doc.info.bookmarks, doc.info.page_count, bmLevel]);
  const bmLevels = useMemo(() => bookmarkLevels(doc.info.bookmarks ?? [], doc.info.page_count), [doc.info.bookmarks, doc.info.page_count]);
  const maxBytes = Math.round(Number(maxSize) * (sizeUnit === "MB" ? 1024 * 1024 : 1024));
  const rangeLines = useMemo(() => ranges.split(/\r?\n/).map((s) => s.trim()).filter(Boolean), [ranges]);

  const blocker: string | null = (() => {
    if (needsRange && range.state !== "ok") return range.state === "error" ? range.message : "Checking pages…";
    if (tool === "delete" && range.state === "ok" && range.count >= doc.info.page_count) return "A PDF must keep at least one page.";
    if (tool === "split" && splitMode === "every_n" && !everyNValid) return `Enter a number from 1 to ${doc.info.page_count}.`;
    if (tool === "split" && splitMode === "ranges" && rangeLines.length === 0) return "Enter one page range per line.";
    if (tool === "split" && splitMode === "bookmarks" && bmParts.length === 0) return bmLevels.length ? `There are no level ${bmLevel} bookmarks.` : "This document has no bookmarks that point to a page.";
    if (tool === "split" && splitMode === "max_size" && !(Number.isFinite(maxBytes) && maxBytes >= MIN_SPLIT_BYTES)) return "Choose a size of at least 10 KB.";
    if (tool === "merge" && files.length < 2) return "Add at least one more file.";
    if (tool === "save" && nAnnots + nFields + nObjects + nComments === 0) return "There are no changes to save.";
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
    if (tool === "translate") {
      if (trDirsError) return trDirsError;
      if (!trDirs) return "Checking the installed languages…";
      if (!trFrom && trDetected === undefined) return "Recognizing the document's language…";
      if (!trSource) return "The document's language could not be recognized: choose it under From.";
      if (typeof trRoute === "string") return trRoute;
      if (trRoute && trRoute.needs.length) return "Download the language first.";
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
          split:
            splitMode === "every_n"
              ? { mode: "every_n", n: everyNValue }
              : splitMode === "ranges"
                ? { mode: "ranges", ranges: rangeLines }
                : splitMode === "bookmarks"
                  ? { mode: "bookmarks", level: bmLevel }
                  : { mode: "max_size", max_bytes: maxBytes },
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
      case "orient":
        return { op: "fix_orientation", input: doc.path, pages: allPages ? null : pages, output };
      case "extractImages":
        return { op: "extract_images", input: doc.path, pages: allPages ? null : pages, output_dir: output };
      case "deskew":
        return { op: "deskew", input: doc.path, pages: allPages ? null : pages, output };
      case "optimize": {
        const p = IMAGE_PRESETS.find((x) => x.id === imagePreset) ?? IMAGE_PRESETS[1]!;
        return { op: "optimize", input: doc.path, images: reduceImages ? { max_dpi: p.max_dpi, quality: p.quality } : null, output };
      }
      case "text":
        return { op: "export_text", input: doc.path, pages: allPages ? null : pages, output };
      case "word":
        return { op: "convert_to_docx", input: doc.path, pages: allPages ? null : pages, output };
      case "slides":
        return { op: "convert_to_pptx", input: doc.path, pages: allPages ? null : pages, output };
      case "translate":
        return { op: "translate_document", input: doc.path, pages: allPages ? null : pages, source: trFrom, target: trTo, output, bilingual: trFormat === "pdf" && trSideBySide };
      case "audio":
        return { op: "read_aloud_to_wav", input: doc.path, pages: allPages ? null : pages, voice: readVoice, rate: readRate, output };
      case "tables":
        return { op: "export_tables", input: doc.path, pages: allPages ? null : pages, format: sheetFormat, output };
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
          object_edits: objectEdits ?? [],
          annotation_changes: annotationChanges ?? [],
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
    } else if (tool === "text") {
      const p = await api.pickSaveAnyPath(output);
      if (p) setOutput(/\.txt$/i.test(p) ? p : `${p}.txt`);
    } else if (tool === "slides") {
      const p = await api.pickSaveAnyPath(output);
      if (p) setOutput(/\.pptx$/i.test(p) ? p : `${p}.pptx`);
    } else if (tool === "translate") {
      const p = await api.pickSaveAnyPath(output);
      const ext = `.${trFormat}`;
      if (p) setOutput(p.toLowerCase().endsWith(ext) ? p : `${p}${ext}`);
    } else if (tool === "audio") {
      const p = await api.pickSaveAnyPath(output);
      if (p) setOutput(/\.wav$/i.test(p) ? p : `${p}.wav`);
    } else if (tool === "tables") {
      const p = await api.pickSaveAnyPath(output);
      if (p) setOutput(/\.xlsx$/i.test(p) ? p : `${p}.xlsx`);
    } else if (tool === "word") {
      const p = await api.pickSaveAnyPath(output);
      if (p) setOutput(/\.docx$/i.test(p) ? p : `${p}.docx`);
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
          Saves {[nObjects ? `${nObjects} content edit${nObjects === 1 ? "" : "s"}` : "", nFields ? `${nFields} filled field${nFields === 1 ? "" : "s"}` : "", nAnnots ? `${nAnnots} annotation${nAnnots === 1 ? "" : "s"}` : "", nComments ? `${nComments} change${nComments === 1 ? "" : "s"} to existing comments` : ""].filter(Boolean).join(", ").replace(/, ([^,]*)$/, " and $1")} into a copy of{" "}
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

      {tool === "orient" && (
        <p className="hint" style={{ marginTop: 0 }}>
          Finds scanned pages that face sideways or upside down and turns them upright in a new copy. Nothing is redrawn; pages with their own text are left as they are.
        </p>
      )}

      {tool === "deskew" && (
        <p className="hint" style={{ marginTop: 0 }}>
          Turns skewed scanned pages straight in a new copy. Images are not resampled; pages with their own text are left as they are.
        </p>
      )}

      {tool === "translate" && (
        <>
          <div className="field-row">
            <label className="field">
              <span className="label">From</span>
              <select
                id="translate-from"
                className="select"
                value={trFrom ?? ""}
                disabled={!trDirs}
                onChange={(e) => setTrFrom(e.target.value || null)}
              >
                <option value="">{trDetected ? `Detect automatically (${languageName(trDetected)})` : "Detect automatically"}</option>
                {languages(trDirs ?? []).map((l) => (
                  <option key={l} value={l}>
                    {languageName(l)}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="label">To</span>
              <select
                id="translate-to"
                className="select"
                value={trTo}
                disabled={!trDirs}
                onChange={(e) => {
                  const to = e.target.value;
                  setTrTo(to);
                  setOutput(join(dirname(doc.path), `${stem(doc.path)}-${to}.${trFormat}`));
                }}
              >
                {languages(trDirs ?? []).map((l) => (
                  <option key={l} value={l}>
                    {languageName(l)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          {trRoute && typeof trRoute !== "string" && trRoute.viaEnglish && (
            <span className="hint" data-testid="translate-via">Translated through English (no direct model for this pair).</span>
          )}
          {trRoute && typeof trRoute !== "string" && trRoute.needs.length > 0 && (
            <TranslationDownload key={trRoute.needs.map((d) => `${d.source}-${d.target}`).join(",")} needs={trRoute.needs} jobs={jobs} onDone={() => setTrDirsNonce((n) => n + 1)} />
          )}
          <div className="field">
            <span className="label" id="translate-format-label">Save as</span>
            <div className="segmented" role="radiogroup" aria-labelledby="translate-format-label">
              {(
                [
                  ["pdf", "PDF (same layout)"],
                  ["docx", "Word document"],
                  ["txt", "Text"],
                ] as const
              ).map(([f, l]) => (
                <button
                  key={f}
                  type="button"
                  role="radio"
                  aria-checked={trFormat === f}
                  data-testid={`translate-format-${f}`}
                  onClick={() => {
                    if (f === trFormat) return;
                    setTrFormat(f);
                    setOutput(join(dirname(doc.path), `${stem(doc.path)}-${trTo}.${f}`));
                  }}
                >
                  {l}
                </button>
              ))}
            </div>
          </div>
          {trFormat === "pdf" && (
            <div className="field">
              <label className="row">
                <input type="checkbox" checked={trSideBySide} onChange={(e) => setTrSideBySide(e.target.checked)} data-testid="translate-side-by-side" /> Original and translation side by side
              </label>
            </div>
          )}
        </>
      )}
      {tool === "tables" && (
        <div className="field">
          <span className="label" id="sheet-format-label">Format</span>
          <div className="segmented" role="radiogroup" aria-labelledby="sheet-format-label">
            {(
              [
                ["xlsx", "Excel workbook"],
                ["csv", "CSV files"],
              ] as const
            ).map(([f, l]) => (
              <button
                key={f}
                type="button"
                role="radio"
                aria-checked={sheetFormat === f}
                data-testid={`sheet-format-${f}`}
                onClick={() => {
                  if (f === sheetFormat) return;
                  setSheetFormat(f);
                  setOutput(f === "csv" ? dirname(doc.path) : join(dirname(doc.path), `${stem(doc.path)}.xlsx`));
                }}
              >
                {l}
              </button>
            ))}
          </div>
          <span className="hint">
            {sheetFormat === "xlsx" ? "One sheet per table; merged cells kept." : "One file per table, text only."} Ruled tables and columns aligned over three or more rows are found; values are kept exactly.
          </span>
        </div>
      )}

      {tool === "optimize" && (
        <p className="hint" style={{ marginTop: 0 }}>
          Saves a smaller copy: data is packed more tightly and repeated parts are stored once. Text and drawings are never changed.
        </p>
      )}

      {tool === "optimize" && (
        <div className="field">
          <label className="row">
            <input type="checkbox" checked={reduceImages} onChange={(e) => setReduceImages(e.target.checked)} data-testid="optimize-images" /> Reduce image resolution
          </label>
          {reduceImages ? (
            <>
              <select id="optimize-preset" className="select" aria-label="Image resolution" value={imagePreset} onChange={(e) => setImagePreset(e.target.value)}>
                {IMAGE_PRESETS.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
              <span className="hint">Images shown above this resolution are resampled and stored as JPEG; some detail is lost.</span>
            </>
          ) : (
            <span className="hint">Off: every image keeps its exact data.</span>
          )}
        </div>
      )}

      {tool === "ocr" && (
        <p className="hint" style={{ marginTop: 0 }}>
          Adds selectable, searchable text to scanned pages in a new copy. The pages look exactly the same.
        </p>
      )}

      {ALL_PAGES_TOOLS.includes(tool) && (
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
              <button type="button" role="radio" aria-checked={splitMode === "bookmarks"} onClick={() => setSplitMode("bookmarks")}>
                By bookmarks
              </button>
              <button type="button" role="radio" aria-checked={splitMode === "max_size"} onClick={() => setSplitMode("max_size")}>
                By size
              </button>
            </div>
          </div>
          {splitMode === "bookmarks" ? (
            <div className="field">
              <label htmlFor="split-level">Bookmark level</label>
              <select id="split-level" className="select" value={bmLevel} disabled={bmLevels.length === 0} onChange={(e) => setBmLevel(Number(e.target.value))}>
                {(bmLevels.length ? bmLevels : [1]).map((l) => (
                  <option key={l} value={l}>
                    {l === 1 ? "Top level" : `Level ${l}`}
                  </option>
                ))}
              </select>
              <span className="hint" data-testid="split-bookmarks-note">
                {bmParts.length ? `Creates ${bmParts.length} file${bmParts.length === 1 ? "" : "s"}, named after the bookmarks.` : "No bookmarks at this level point to a page."}
              </span>
            </div>
          ) : splitMode === "max_size" ? (
            <div className="field">
              <label htmlFor="split-size">Largest file size</label>
              <div className="row">
                <input id="split-size" className="input" type="number" min={1} step="any" value={maxSize} aria-invalid={!(maxBytes >= MIN_SPLIT_BYTES)} onChange={(e) => setMaxSize(e.target.value)} style={{ width: 110 }} />
                <select className="select" aria-label="Size unit" value={sizeUnit} onChange={(e) => setSizeUnit(e.target.value as "KB" | "MB")}>
                  <option value="MB">MB</option>
                  <option value="KB">KB</option>
                </select>
              </div>
              <span className="hint">A page larger than this on its own becomes a file by itself.</span>
            </div>
          ) : splitMode === "every_n" ? (
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
        <label htmlFor="output">{tool === "split" ? "Save parts in folder" : tool === "images" || tool === "extractImages" ? "Save images in folder" : toFolder ? "Save files in folder" : "Save as"}</label>
        <div className="row">
          <input id="output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button type="button" className="btn outline" onClick={browse}>
            <FolderOpen /> Browse…
          </button>
        </div>
        {tool === "split" && splitMode !== "bookmarks" && <span className="hint">Files are named “{doc.name.replace(/\.pdf$/i, "")}_part01.pdf”, …</span>}
        {tool === "extractImages" && <span className="hint">JPEG images are saved exactly as stored; others as PNG. Repeated images are saved once.</span>}
        {tool === "text" && <span className="hint">UTF-8 text, one section per page. Scanned pages need text recognition first.</span>}
        {tool === "translate" && <span className="hint">Translated on this computer; the result is a new file and the PDF is not changed. Review machine translation before relying on it.</span>}
        {tool === "audio" && <span className="hint">The pages read aloud with the read-aloud voice and speed ({readRate}×), as a WAV file.</span>}
        {tool === "slides" && <span className="hint">One slide per page with editable text boxes, tables and pictures. Slides take the first page's size.</span>}
        {tool === "word" && <span className="hint">Editable text, headings, columns, pictures and links. Scanned pages need text recognition first.</span>}
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

/** Smallest size limit (pw-core MIN_SPLIT_BYTES). */
export const MIN_SPLIT_BYTES = 10 * 1024;

/** Outline levels (1-based) that have at least one bookmark pointing to a page. */
export function bookmarkLevels(bookmarks: BookmarkInfo[], pageCount: number): number[] {
  const levels = new Set(bookmarks.filter((b) => b.page !== null && b.page < pageCount).map((b) => b.depth + 1));
  return [...levels].sort((a, b) => a - b);
}

/** The parts splitting by bookmarks at `level` makes, as the engine does: one per distinct start page. */
export function bookmarkParts(bookmarks: BookmarkInfo[], pageCount: number, level: number): Array<{ title: string; first: number }> {
  const starts = bookmarks
    .filter((b) => b.depth + 1 === level && b.page !== null && b.page < pageCount)
    .map((b) => ({ title: b.title, first: b.page! }))
    .sort((a, b) => a.first - b.first);
  return starts.filter((b, i) => i === 0 || b.first !== starts[i - 1]!.first);
}

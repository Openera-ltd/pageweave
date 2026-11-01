// Batch processing (PRD 06): one action over many files, each file one existing typed operation.
// The host's job manager runs them with bounded concurrency; nothing here touches files.

import { DEFAULT_SANITIZE } from "./components/SanitizeDialog";
import { basename, join, stem } from "./paths";
import { languageName } from "./translate";
import type { Operation, TranslationDirection } from "./types";

export type BatchAction = "optimize" | "optimize_images" | "ocr" | "sanitize" | "deskew" | "orient" | "text" | "word" | "slides" | "translate";

export const BATCH_ACTIONS: Array<{ id: BatchAction; label: string; suffix: string; ext: "pdf" | "txt" | "docx" | "pptx" }> = [
  { id: "optimize", label: "Reduce file size (lossless)", suffix: "smaller", ext: "pdf" },
  { id: "optimize_images", label: "Reduce file size, images at 150 dpi", suffix: "smaller", ext: "pdf" },
  { id: "ocr", label: "Recognize text (OCR)", suffix: "searchable", ext: "pdf" },
  { id: "sanitize", label: "Remove hidden information", suffix: "clean", ext: "pdf" },
  { id: "deskew", label: "Straighten scanned pages", suffix: "straight", ext: "pdf" },
  { id: "orient", label: "Turn pages upright", suffix: "upright", ext: "pdf" },
  { id: "text", label: "Export as text", suffix: "", ext: "txt" },
  { id: "word", label: "Convert to Word", suffix: "", ext: "docx" },
  { id: "slides", label: "Convert to PowerPoint", suffix: "", ext: "pptx" },
  // The suffix is the target language ("report-de.pdf"), as in the Translate dialog.
  { id: "translate", label: "Translate (into a PDF)", suffix: "", ext: "pdf" },
];

/** Most files in one batch (beyond this, use the CLI in a script). */
export const MAX_BATCH_FILES = 500;

/** Output names in `folder`, unique within the batch ("a.pdf" twice from different folders → "a-smaller (2).pdf"). */
export function batchOutputs(inputs: string[], action: BatchAction, folder: string, target = "en"): string[] {
  const a = BATCH_ACTIONS.find((x) => x.id === action)!;
  const suffix = action === "translate" ? target : a.suffix;
  const used = new Map<string, number>();
  return inputs.map((p) => {
    const base = suffix ? `${stem(p)}-${suffix}` : stem(p);
    const key = base.toLowerCase();
    const n = (used.get(key) ?? 0) + 1;
    used.set(key, n);
    return join(folder, `${base}${n > 1 ? ` (${n})` : ""}.${a.ext}`);
  });
}

/** `target`: the language to translate into (translate only; each file's language is detected). */
export function batchOperation(action: BatchAction, input: string, output: string, target = "en"): Operation {
  switch (action) {
    case "optimize":
      return { op: "optimize", input, images: null, output };
    case "optimize_images":
      return { op: "optimize", input, images: { max_dpi: 150, quality: 75 }, output };
    case "ocr":
      return { op: "ocr_pages", input, pages: null, skip_text_pages: true, dpi: 200, output };
    case "sanitize":
      return { op: "sanitize", input, options: DEFAULT_SANITIZE, output };
    case "deskew":
      return { op: "deskew", input, pages: null, output };
    case "orient":
      return { op: "fix_orientation", input, pages: null, output };
    case "text":
      return { op: "export_text", input, pages: null, output };
    case "word":
      return { op: "convert_to_docx", input, pages: null, output };
    case "slides":
      return { op: "convert_to_pptx", input, pages: null, output };
    case "translate":
      return { op: "translate_document", input, pages: null, source: null, target, output };
  }
}

/** Only PDFs, each once (same path twice is one file). */
export function addFiles(current: string[], picked: string[]): string[] {
  const seen = new Set(current.map((p) => p.toLowerCase()));
  const out = [...current];
  for (const p of picked) {
    if (!p.toLowerCase().endsWith(".pdf") || seen.has(p.toLowerCase())) continue;
    seen.add(p.toLowerCase());
    out.push(p);
  }
  return out.slice(0, MAX_BATCH_FILES);
}

export type ItemState = "waiting" | "running" | "done" | "failed" | "cancelled";

export function summary(states: ItemState[]): string {
  const c = (s: ItemState) => states.filter((x) => x === s).length;
  const parts = [`${c("done")} done`];
  if (c("failed")) parts.push(`${c("failed")} failed`);
  if (c("cancelled")) parts.push(`${c("cancelled")} cancelled`);
  const left = c("waiting") + c("running");
  if (left) parts.push(`${left} to go`);
  return `${parts.join(" · ")} (of ${states.length})`;
}

export { basename };

/** Why a batch translation into `target` cannot start: no downloaded model leads into it, directly
 *  or through English. Files whose own language has no downloaded route fail one by one with the
 *  engine's message. */
export function translateBlocker(dirs: TranslationDirection[] | null, target: string): string | null {
  if (!dirs) return "Reading the translation languages…";
  const installed = dirs.filter((d) => d.installed);
  const into = installed.some((d) => d.target === target);
  if (into) return null;
  return `No translation into ${languageName(target)} is downloaded yet. Download one in Translate… first.`;
}

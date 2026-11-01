// Form data files (XFDF / JSON): default names, and how an imported file's values become the
// app's pending form edits. The engine decides which values fit which field (read_form_data); this
// module only merges its answer with what the form currently shows, including unsaved edits.

import { currentValue, fieldLabel, setEdit, type FormEdits } from "./forms";
import { dirname, join, stem } from "./paths";
import type { FieldKind, FormField, FormValue } from "./types";

export type FormDataFormat = "xfdf" | "json";

/** One value from the engine's read_form_data answer. */
export interface MatchedValue {
  name: string;
  label: string | null;
  kind: FieldKind;
  value: FormValue;
  current: FormValue;
  changed: boolean;
}

export interface FormDataMatch {
  format: FormDataFormat;
  entries: number;
  matched: MatchedValue[];
  skipped: Array<{ name: string; reason: string }>;
  unknown: string[];
}

/** "C:\\forms\\tax.pdf" → "C:\\forms\\tax-data.xfdf". */
export function defaultDataPath(pdf: string, format: FormDataFormat): string {
  return join(dirname(pdf), `${stem(pdf)}-data.${format}`);
}

/** Give a chosen path the format's extension unless it already ends in .xfdf, .xml or .json. */
export function withDataExtension(path: string, format: FormDataFormat): string {
  const lower = path.toLowerCase();
  if (format === "json") return lower.endsWith(".json") ? path : `${path}.json`;
  return lower.endsWith(".xfdf") || lower.endsWith(".xml") ? path : `${path}.xfdf`;
}

/** Switch a default-looking path between formats ("x.xfdf" ↔ "x.json"); other paths are kept. */
export function switchDataExtension(path: string, format: FormDataFormat): string {
  return path.replace(/\.(xfdf|json)$/i, `.${format}`);
}

export interface ImportPlan {
  /** Pending edits once the file is applied. */
  edits: FormEdits;
  /** Fields whose shown value changes, in form order. */
  changes: Array<{ field: FormField; from: FormValue; to: FormValue }>;
  /** Matched values the form already shows. */
  unchanged: number;
}

/** Apply matched values to the current edits. A value equal to the document's own drops the edit. */
export function planImport(match: FormDataMatch, fields: FormField[], edits: FormEdits): ImportPlan {
  const order = new Map(fields.map((f, i) => [f.name, i]));
  // Form order, whatever order the file lists its fields in.
  const matched = match.matched.filter((m) => order.has(m.name)).sort((a, b) => order.get(a.name)! - order.get(b.name)!);
  let next = edits;
  const changes: ImportPlan["changes"] = [];
  let unchanged = 0;
  for (const m of matched) {
    const f = fields[order.get(m.name)!];
    // The form was re-read since the preview, or the field cannot take edits here.
    if (!f || f.read_only) continue;
    const from = currentValue(next, f);
    if (from === m.value) {
      unchanged++;
      continue;
    }
    changes.push({ field: f, from, to: m.value });
    next = setEdit(next, f, m.value);
  }
  return { edits: next, changes, unchanged };
}

/** A value as shown in the preview list (an empty radio group or choice reads "nothing chosen"). */
export function shownValue(v: FormValue, kind?: FormField["kind"]): string {
  if (v === true) return "Checked";
  if (v === false) return "Not checked";
  if (v === "") return kind === "radio" || kind === "combo" || kind === "list" ? "(nothing chosen)" : "(empty)";
  return v.replace(/\s*\n\s*/g, " ⏎ ");
}

export { fieldLabel };

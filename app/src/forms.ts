// Form filling state: what each field currently shows, which values the user changed, and the
// payload for the engine's apply_edits operation.

import type { FieldValue, FormField, FormValue } from "./types";

/** Kinds PageWeave can fill. Push buttons, signatures and unknown fields are never shown as inputs. */
export function isFillable(f: FormField): boolean {
  return f.kind === "text" || f.kind === "checkbox" || f.kind === "radio" || f.kind === "combo" || f.kind === "list";
}

/** The value a field has in the document (before any edit). */
export function originalValue(f: FormField): FormValue {
  switch (f.kind) {
    case "checkbox":
      return f.widgets.some((w) => w.checked);
    case "radio":
      return f.widgets.find((w) => w.checked)?.export_value ?? "";
    default:
      return f.value ?? "";
  }
}

export type FormEdits = Record<string, FormValue>;

/** Record a value; setting a field back to its original value removes the edit. */
export function setEdit(edits: FormEdits, f: FormField, value: FormValue): FormEdits {
  const { [f.name]: _old, ...rest } = edits;
  return value === originalValue(f) ? rest : { ...rest, [f.name]: value };
}

export function currentValue(edits: FormEdits, f: FormField): FormValue {
  return f.name in edits ? edits[f.name]! : originalValue(f);
}

/** Edits as engine field values, in document field order. Unknown names (stale edits) are dropped. */
export function toFieldValues(edits: FormEdits, fields: FormField[]): FieldValue[] {
  return fields.filter((f) => f.name in edits && isFillable(f) && !f.read_only).map((f) => ({ name: f.name, value: edits[f.name]! }));
}

/** Required fields that are still empty after edits (reported before saving, not blocking). */
export function missingRequired(edits: FormEdits, fields: FormField[]): FormField[] {
  return fields.filter((f) => f.required && isFillable(f) && !f.read_only).filter((f) => {
    const v = currentValue(edits, f);
    return v === "" || v === false;
  });
}

/** Accessible name for a field's control. */
export function fieldLabel(f: FormField): string {
  return f.label ?? f.name.replace(/[._]+/g, " ").trim();
}

/**
 * Edits that empty the form: text fields to "", checkboxes unticked. Choice lists and radio groups
 * are kept — the engine can only set them to one of their options — and counted in `kept`.
 */
export function clearedEdits(edits: FormEdits, fields: FormField[]): { edits: FormEdits; changed: number; kept: number } {
  let next = edits;
  let changed = 0;
  let kept = 0;
  for (const f of fields) {
    if (!isFillable(f) || f.read_only) continue;
    if (f.kind === "text" || f.kind === "checkbox") {
      const empty: FormValue = f.kind === "text" ? "" : false;
      if (currentValue(next, f) !== empty) {
        next = setEdit(next, f, empty);
        changed++;
      }
    } else if (f.kind === "combo" && f.editable) {
      if (currentValue(next, f) !== "") {
        next = setEdit(next, f, "");
        changed++;
      }
    } else if (currentValue(next, f) !== "") {
      kept++;
    }
  }
  return { edits: next, changed, kept };
}

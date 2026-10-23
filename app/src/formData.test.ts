import { describe, expect, it } from "vitest";
import { defaultDataPath, planImport, shownValue, switchDataExtension, withDataExtension, type FormDataMatch } from "./formData";
import type { FormField } from "./types";

const field = (name: string, kind: FormField["kind"], extra: Partial<FormField> = {}): FormField => ({
  name,
  label: null,
  kind,
  value: null,
  options: [],
  read_only: false,
  required: false,
  multiline: false,
  editable: false,
  widgets: [{ page: 0, rect: { left: 0, bottom: 0, right: 1, top: 1 }, export_value: kind === "checkbox" ? "On" : null, checked: false }],
  ...extra,
});

const fields = [field("name", "text", { value: "Ada" }), field("agree", "checkbox"), field("city", "text"), field("ref", "text", { read_only: true })];

const match = (matched: FormDataMatch["matched"]): FormDataMatch => ({ format: "xfdf", entries: matched.length, matched, skipped: [], unknown: [] });

describe("form data files", () => {
  it("names files next to the PDF and keeps extensions in step with the format", () => {
    expect(defaultDataPath("C:\\forms\\tax.pdf", "xfdf")).toBe("C:\\forms\\tax-data.xfdf");
    expect(defaultDataPath("/f/tax.pdf", "json")).toBe("/f/tax-data.json");
    expect(switchDataExtension("/f/tax-data.xfdf", "json")).toBe("/f/tax-data.json");
    expect(switchDataExtension("/f/custom.txt", "json")).toBe("/f/custom.txt");
    expect(withDataExtension("/f/out", "xfdf")).toBe("/f/out.xfdf");
    expect(withDataExtension("/f/out.XML", "xfdf")).toBe("/f/out.XML");
    expect(withDataExtension("/f/out.xfdf", "json")).toBe("/f/out.xfdf.json");
  });

  it("plans changes against what the form shows, including unsaved edits", () => {
    const plan = planImport(
      match([
        { name: "name", label: null, kind: "text", value: "Ada", current: "Ada", changed: false },
        { name: "agree", label: null, kind: "checkbox", value: true, current: false, changed: true },
        { name: "city", label: null, kind: "text", value: "Oslo", current: "", changed: true },
        { name: "ref", label: null, kind: "text", value: "X", current: "", changed: true },
        { name: "gone", label: null, kind: "text", value: "X", current: "", changed: true },
      ]),
      fields,
      // Unsaved: name was changed to Grace; the file puts the document's own value back.
      { name: "Grace", city: "Oslo" },
    );
    expect(plan.changes.map((c) => [c.field.name, c.from, c.to])).toEqual([
      ["name", "Grace", "Ada"],
      ["agree", false, true],
    ]);
    expect(plan.unchanged).toBe(1);
    // Back to the document's value → the edit is dropped rather than stored.
    expect(plan.edits).toEqual({ city: "Oslo", agree: true });
  });

  it("lists changes in form order, not file order", () => {
    const plan = planImport(
      match([
        { name: "city", label: null, kind: "text", value: "Oslo", current: "", changed: true },
        { name: "agree", label: null, kind: "checkbox", value: true, current: false, changed: true },
      ]),
      fields,
      {},
    );
    expect(plan.changes.map((c) => c.field.name)).toEqual(["agree", "city"]);
  });

  it("shows values plainly", () => {
    expect(shownValue(true)).toBe("Checked");
    expect(shownValue(false)).toBe("Not checked");
    expect(shownValue("")).toBe("(empty)");
    expect(shownValue("a\n b")).toBe("a ⏎ b");
  });
});

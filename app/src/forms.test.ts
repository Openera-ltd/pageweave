import { describe, expect, it } from "vitest";
import { clearedEdits, currentValue, fieldLabel, isFillable, missingRequired, originalValue, resetEdits, setEdit, toFieldValues } from "./forms";
import type { FormField } from "./types";

const base = { label: null, value: null, options: [], read_only: false, required: false, multiline: false, editable: false };
const w = (checked = false, export_value: string | null = null) => ({ page: 0, rect: { left: 0, bottom: 0, right: 10, top: 10 }, export_value, checked });
const text: FormField = { ...base, name: "full_name", kind: "text", value: "Ada", widgets: [w()] };
const box: FormField = { ...base, name: "subscribe", kind: "checkbox", widgets: [w(false, "On")] };
const radio: FormField = { ...base, name: "plan", kind: "radio", widgets: [w(true, "basic"), w(false, "pro")] };
const sig: FormField = { ...base, name: "sig", kind: "signature", widgets: [w()] };

describe("form edits", () => {
  it("derives original values per kind", () => {
    expect(originalValue(text)).toBe("Ada");
    expect(originalValue(box)).toBe(false);
    expect(originalValue(radio)).toBe("basic");
  });

  it("tracks only real changes and drops edits set back to the original", () => {
    let e = setEdit({}, text, "Grace");
    e = setEdit(e, box, true);
    expect(e).toEqual({ full_name: "Grace", subscribe: true });
    e = setEdit(e, text, "Ada");
    expect(e).toEqual({ subscribe: true });
    expect(currentValue(e, text)).toBe("Ada");
    expect(currentValue(e, box)).toBe(true);
  });

  it("builds engine values in field order and never sends unfillable or read-only fields", () => {
    const ro: FormField = { ...text, name: "locked", read_only: true };
    const e = { subscribe: true, full_name: "Grace", sig: "x", locked: "y", gone: "z" };
    expect(toFieldValues(e, [text, box, sig, ro])).toEqual([
      { name: "full_name", value: "Grace" },
      { name: "subscribe", value: true },
    ]);
    expect(isFillable(sig)).toBe(false);
  });

  it("reports required fields left empty", () => {
    const req: FormField = { ...text, name: "email", value: "", required: true };
    expect(missingRequired({}, [req, text]).map((f) => f.name)).toEqual(["email"]);
    expect(missingRequired({ email: "a@b.c" }, [req])).toEqual([]);
  });

  it("labels fields by /TU, else a readable name", () => {
    expect(fieldLabel({ ...text, label: "Full name" })).toBe("Full name");
    expect(fieldLabel({ ...text, name: "address.post_code" })).toBe("address post code");
  });
});

describe("clear form", () => {
  const w = { page: 0, rect: { left: 0, bottom: 0, right: 1, top: 1 }, export_value: "On", checked: true };
  const f = (name: string, kind: FormField["kind"], extra: Partial<FormField> = {}): FormField => ({
    name, label: null, kind, value: null, options: [], read_only: false, required: false, multiline: false, editable: false, widgets: [w], ...extra,
  });
  it("empties text, checkboxes, choices and radio groups, leaves read-only fields", () => {
    const fields = [
      f("name", "text", { value: "Ada" }),
      f("notes", "text", { value: "" }),
      f("agree", "checkbox"),
      f("plan", "radio"),
      f("size", "radio", { widgets: [{ page: 0, rect: { left: 0, bottom: 0, right: 10, top: 10 }, export_value: "L", checked: true }] }),
      f("country", "combo", { value: "Japan", options: ["Japan"] }),
      f("city", "combo", { value: "Oslo", editable: true }),
      f("ref", "text", { value: "R-1", read_only: true }),
    ];
    const r = clearedEdits({ notes: "typed" }, fields);
    // "plan" has a button on (the shared widget is checked): emptied too.
    expect(r.edits).toEqual({ name: "", agree: false, plan: "", size: "", country: "", city: "" });
    expect(r.changed).toBe(7);
  });

  it("resets to the document's defaults and empties fields without a usable one", () => {
    const wd = (checked = false, export_value: string | null = null) => ({ page: 0, rect: { left: 0, bottom: 0, right: 10, top: 10 }, export_value, checked });
    const fields: FormField[] = [
      { ...text, default_value: "Jane" },
      { ...text, name: "notes", value: "x" },
      { ...box, default_value: "On" },
      { ...box, name: "news", widgets: [wd(true, "Yes")], default_value: "Off" },
      { ...radio, default_value: "pro" },
      { ...radio, name: "size" },
      { ...base, name: "country", kind: "combo", value: "Canada", options: ["Canada", "Japan"], default_value: "Japan", widgets: [wd()] },
      { ...base, name: "colour", kind: "list", value: "Red", options: ["Red"], default_value: "Gone", widgets: [wd()] },
      { ...base, name: "city", kind: "combo", value: "Oslo", editable: true, widgets: [wd()] },
      { ...text, name: "ref", value: "R-1", read_only: true, default_value: "R-0" },
    ];
    const r = resetEdits({ notes: "typed", plan: "basic" }, fields);
    // "size" (basic on, no default) and "colour" (its default is not an option) are emptied.
    expect(r.edits).toEqual({ full_name: "Jane", notes: "", subscribe: true, news: false, plan: "pro", size: "", country: "Japan", colour: "", city: "" });
    expect(r.changed).toBe(9);
    // A second reset changes nothing.
    expect(resetEdits(r.edits, fields).changed).toBe(0);
  });
});

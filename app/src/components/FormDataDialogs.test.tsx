import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { FormInfo, JobSnapshot, OpenedDoc, OperationRequest } from "../types";
import { ExportFormDataDialog, ImportFormDataDialog } from "./FormDataDialogs";
import type { FormDataMatch, ImportPlan } from "../formData";

const doc = { doc_id: 1, path: "/f/tax.pdf", name: "tax.pdf", info: { page_count: 1, pages: [], form_type: "acro_form" } } as unknown as OpenedDoc;
const form: FormInfo = {
  has_xfa: false,
  truncated: false,
  fields: [
    { name: "full_name", label: "Full name", kind: "text", value: "", options: [], read_only: false, required: false, multiline: false, editable: false, widgets: [] },
    { name: "agree", label: null, kind: "checkbox", value: null, options: [], read_only: false, required: false, multiline: false, editable: false, widgets: [] },
  ],
};

function job(id: string, state: JobSnapshot["state"], data?: unknown, error?: JobSnapshot["error"]): JobSnapshot {
  return { id, label: "x", state, progress: null, result: data === undefined ? null : { operation: "read_form_data", outputs: [], warnings: [], data }, error: error ?? null } as unknown as JobSnapshot;
}

afterEach(cleanup);

describe("Export form data", () => {
  it("sends unsaved values and switches the extension with the format", async () => {
    const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
    const api = { startOperation, pickFormDataSavePath: vi.fn() } as unknown as Api;
    render(
      <ApiContext.Provider value={api}>
        <ExportFormDataDialog doc={doc} form={form} edits={{ full_name: "Ada" }} jobs={{}} onClose={() => undefined} onFinished={() => undefined} />
      </ApiContext.Provider>,
    );
    expect(screen.getByTestId("formdata-export-note").textContent).toBe("Includes 1 unsaved change.");
    fireEvent.click(screen.getByTestId("formdata-format-json"));
    expect((screen.getByLabelText("Save as") as HTMLInputElement).value).toBe("/f/tax-data.json");
    await act(async () => {
      fireEvent.click(screen.getByTestId("formdata-export-start"));
    });
    const op = startOperation.mock.calls[0]![0].operation;
    expect(op).toEqual({ op: "export_form_data", input: "/f/tax.pdf", format: "json", values: [{ name: "full_name", value: "Ada" }], output: "/f/tax-data.json" });
  });

  it("needs a destination", () => {
    const api = { startOperation: vi.fn() } as unknown as Api;
    render(
      <ApiContext.Provider value={api}>
        <ExportFormDataDialog doc={doc} form={form} edits={{}} jobs={{}} onClose={() => undefined} onFinished={() => undefined} />
      </ApiContext.Provider>,
    );
    fireEvent.change(screen.getByLabelText("Save as"), { target: { value: " " } });
    expect(screen.getByTestId("formdata-export-start").getAttribute("title")).toBe("Choose where to save the form data.");
    fireEvent.click(screen.getByTestId("formdata-export-start"));
    expect(api.startOperation).not.toHaveBeenCalled();
  });
});

describe("Import form data", () => {
  const preview: FormDataMatch = {
    format: "xfdf",
    entries: 4,
    matched: [
      { name: "full_name", label: "Full name", kind: "text", value: "Ada", current: "", changed: true },
      { name: "agree", label: null, kind: "checkbox", value: true, current: false, changed: true },
    ],
    skipped: [{ name: "ref", reason: "The field is read-only." }],
    unknown: ["zip"],
  };

  function setup(jobs: Record<string, JobSnapshot>) {
    let n = 0;
    const startOperation = vi.fn(async (_r: OperationRequest) => `read-${++n}`);
    const onApply = vi.fn((_p: ImportPlan, _s: string) => undefined);
    const api = { startOperation, pickFormDataFile: vi.fn(async () => "/f/in.xfdf") } as unknown as Api;
    const ui = (j: Record<string, JobSnapshot>) => (
      <ApiContext.Provider value={api}>
        <ImportFormDataDialog doc={doc} form={form} edits={{}} jobs={j} onClose={() => undefined} onApply={onApply} />
      </ApiContext.Provider>
    );
    const r = render(ui(jobs));
    return { startOperation, onApply, rerender: (j: Record<string, JobSnapshot>) => r.rerender(ui(j)) };
  }

  it("reads the chosen file, previews the fields, then fills them", async () => {
    const { startOperation, onApply, rerender } = setup({});
    const apply = screen.getByTestId("formdata-import-apply");
    expect(apply.getAttribute("title")).toBe("Choose a form data file.");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Browse/ }));
    });
    expect(startOperation.mock.calls[0]![0].operation).toEqual({ op: "read_form_data", input: "/f/tax.pdf", data: "/f/in.xfdf" });
    rerender({ "read-1": job("read-1", "running") });
    expect(screen.getByTestId("formdata-import-apply").getAttribute("title")).toBe("Reading the file…");
    rerender({ "read-1": job("read-1", "succeeded", preview) });
    expect(screen.getByTestId("formdata-import-summary").textContent).toBe("2 fields to fill · 2 not used");
    expect(screen.getByTestId("formdata-import-changes").textContent).toContain("Full name");
    expect(screen.getByTestId("formdata-import-unused").textContent).toContain("Not a field of this form.");
    fireEvent.click(screen.getByTestId("formdata-import-apply"));
    expect(onApply).toHaveBeenCalledTimes(1);
    expect(onApply.mock.calls[0]![0].edits).toEqual({ full_name: "Ada", agree: true });
    expect(onApply.mock.calls[0]![1]).toBe("in.xfdf");
  });

  it("does not apply a preview once the path changes, and shows read errors", async () => {
    const { onApply, rerender } = setup({});
    fireEvent.change(screen.getByLabelText("Form data file (XFDF or JSON)"), { target: { value: "/f/in.xfdf" } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("formdata-import-read"));
    });
    rerender({ "read-1": job("read-1", "succeeded", preview) });
    fireEvent.change(screen.getByLabelText("Form data file (XFDF or JSON)"), { target: { value: "/f/other.xfdf" } });
    expect(screen.getByTestId("formdata-import-apply").getAttribute("title")).toBe("Read the file first.");
    fireEvent.click(screen.getByTestId("formdata-import-apply"));
    expect(onApply).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(screen.getByTestId("formdata-import-read"));
    });
    rerender({ "read-2": job("read-2", "failed", undefined, { category: "invalid_input", code: "form_data.xfdf", message: "The XFDF file could not be read.", action: "Check the file." } as never) });
    expect(screen.getByRole("alert").textContent).toContain("The XFDF file could not be read.");
  });
});

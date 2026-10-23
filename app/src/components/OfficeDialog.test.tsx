import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot, OperationRequest } from "../types";
import { OfficeDialog, isOfficePath } from "./OfficeDialog";

function setup(initial: string | null, engine: string | null, jobs: Record<string, JobSnapshot> = {}) {
  const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
  const pickOfficeFile = vi.fn(async () => "/docs/Budget 2026.xlsx");
  const api = { startOperation, cancelJob: vi.fn(), pickOfficeFile } as unknown as Api;
  const onFinished = vi.fn();
  const view = render(
    <ApiContext.Provider value={api}>
      <OfficeDialog initialFile={initial} engine={engine} jobs={jobs} onClose={() => undefined} onFinished={onFinished} />
    </ApiContext.Provider>,
  );
  return { startOperation, pickOfficeFile, onFinished, view, api };
}

afterEach(cleanup);

describe("Open an Office file", () => {
  it("recognises Office files by name", () => {
    for (const p of ["/a/Report.DOCX", "/a/b.odt", "/a/c.pptx", "/a/d.csv", "/a/e.htm"]) expect(isOfficePath(p)).toBe(true);
    for (const p of ["/a/b.pdf", "/a/b.exe", "/a/docx"]) expect(isOfficePath(p)).toBe(false);
  });

  it("says LibreOffice is needed when it is not installed, and does not start", async () => {
    const { startOperation } = setup("/docs/a.docx", null);
    expect(screen.getByTestId("office-missing").textContent).toContain("needs LibreOffice");
    const start = screen.getByTestId("office-start");
    expect(start.getAttribute("title")).toBe("LibreOffice is not installed on this computer.");
    await act(async () => {
      fireEvent.click(start);
    });
    expect(startOperation).not.toHaveBeenCalled();
  });

  it("picks a file, names the PDF next to it and converts", async () => {
    const { startOperation } = setup(null, "LibreOffice 24.2");
    expect(screen.getByTestId("office-start").getAttribute("title")).toBe("Choose a file first.");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Choose/ }));
    });
    expect(screen.getByTestId("office-file").textContent).toBe("Budget 2026.xlsx");
    expect((screen.getByLabelText("Save the PDF as") as HTMLInputElement).value).toBe("/docs/Budget 2026.pdf");
    await act(async () => {
      fireEvent.click(screen.getByTestId("office-start"));
    });
    expect(startOperation.mock.calls[0]![0].operation).toEqual({ op: "convert_office_to_pdf", input: "/docs/Budget 2026.xlsx", output: "/docs/Budget 2026.pdf" });
    expect(startOperation.mock.calls[0]![0].collision).toBe("fail");
  });

  it("offers Keep both or Replace when the PDF exists", async () => {
    const failed: JobSnapshot = {
      id: "job-1",
      operation: "convert_office_to_pdf",
      state: "failed",
      progress: { kind: "indeterminate" },
      phase: "",
      error: { category: "output_conflict", code: "output.exists", message: "A file with that name already exists.", action: null, retryable: false },
      result: null,
    } as unknown as JobSnapshot;
    const { startOperation, view, api } = setup("/docs/a.docx", "LibreOffice 24.2");
    await act(async () => {
      fireEvent.click(screen.getByTestId("office-start"));
    });
    view.rerender(
      <ApiContext.Provider value={api}>
        <OfficeDialog initialFile="/docs/a.docx" engine="LibreOffice 24.2" jobs={{ "job-1": failed }} onClose={() => undefined} onFinished={() => undefined} />
      </ApiContext.Provider>,
    );
    expect(screen.getByRole("alert").textContent).toContain("already exists");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Keep both" }));
    });
    expect(startOperation.mock.calls[1]![0].collision).toBe("rename");
  });
});

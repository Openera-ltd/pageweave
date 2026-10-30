import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot, OperationRequest } from "../types";
import { OfficeDialog, isOfficePath } from "./OfficeDialog";

function setup(initial: string | null, engine: string | null | undefined, jobs: Record<string, JobSnapshot> = {}, canInstall = true) {
  const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
  const pickOfficeFile = vi.fn(async () => "/docs/Budget 2026.xlsx");
  const officePackage = vi.fn(async () => ({ version: "26.2.6", bytes: 373_252_096, can_install: canInstall }));
  const officeEngine = vi.fn(async () => "LibreOffice 26.2.6.2");
  const api = { startOperation, cancelJob: vi.fn(), pickOfficeFile, officePackage, officeEngine } as unknown as Api;
  const onFinished = vi.fn();
  const view = render(
    <ApiContext.Provider value={api}>
      <OfficeDialog initialFile={initial} engine={engine} jobs={jobs} onClose={() => undefined} onFinished={onFinished} />
    </ApiContext.Provider>,
  );
  return { startOperation, pickOfficeFile, onFinished, view, api, officeEngine };
}

afterEach(cleanup);

describe("Open an Office file", () => {
  it("recognises Office files by name", () => {
    for (const p of ["/a/Report.DOCX", "/a/b.odt", "/a/c.pptx", "/a/d.csv", "/a/e.htm", "/a/m.docm", "/a/s.xlsb", "/a/t.potx", "/a/v.vsdx", "/a/p.pub"]) expect(isOfficePath(p)).toBe(true);
    for (const p of ["/a/b.pdf", "/a/b.exe", "/a/docx"]) expect(isOfficePath(p)).toBe(false);
  });

  it("waits for the app's LibreOffice check when opened first (a file given at launch)", async () => {
    const { startOperation, view, api } = setup("/docs/a.docx", undefined);
    expect(screen.getByTestId("office-checking")).toBeTruthy();
    expect(screen.queryByTestId("office-missing")).toBeNull();
    expect(screen.getByTestId("office-start").getAttribute("title")).toBe("Looking for LibreOffice…");
    await act(async () => {
      fireEvent.click(screen.getByTestId("office-start"));
    });
    expect(startOperation).not.toHaveBeenCalled();
    // The check finishes after the dialog opened: the dialog follows it.
    view.rerender(
      <ApiContext.Provider value={api}>
        <OfficeDialog initialFile="/docs/a.docx" engine="LibreOffice 24.2" jobs={{}} onClose={() => undefined} onFinished={() => undefined} />
      </ApiContext.Provider>,
    );
    expect(screen.queryByTestId("office-checking")).toBeNull();
    expect(screen.getByTestId("office-start").getAttribute("title")).toBeNull();
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

  it("offers a one-time LibreOffice download when it is missing, then converts", async () => {
    const snap = (state: JobSnapshot["state"], extra: Partial<JobSnapshot> = {}): JobSnapshot => ({ id: "job-1", label: "install_office", state, phase: "", progress: { kind: "indeterminate" }, result: null, error: null, warnings: [], elapsed_ms: 0, ...extra });
    const { startOperation, view, api, officeEngine } = setup("/docs/a.docx", null);
    await act(async () => undefined);
    expect(screen.getByTestId("office-download").textContent).toContain("LibreOffice 26.2.6 once (373 MB");
    await act(async () => {
      fireEvent.click(screen.getByTestId("office-download-start"));
    });
    expect(startOperation.mock.calls[0]![0].operation).toEqual({ op: "install_office" });
    const rerender = (jobs: Record<string, JobSnapshot>) =>
      view.rerender(
        <ApiContext.Provider value={api}>
          <OfficeDialog initialFile="/docs/a.docx" engine={null} jobs={jobs} onClose={() => undefined} onFinished={() => undefined} />
        </ApiContext.Provider>,
      );
    await act(async () => rerender({ "job-1": snap("running", { progress: { kind: "determinate", done: 100_000_000, total: 373_252_096, unit: "bytes" } }) }));
    expect(screen.getByTestId("office-download").textContent).toContain("Downloading LibreOffice 26.2.6: 100 MB of 373 MB");
    await act(async () => rerender({ "job-1": snap("succeeded") }));
    expect(officeEngine).toHaveBeenCalled();
    await act(async () => undefined);
    expect(screen.queryByTestId("office-missing")).toBeNull();
    expect(screen.getByTestId("office-start").getAttribute("aria-disabled")).toBeNull();
  });

  it("elsewhere it says to use the package manager", async () => {
    setup("/docs/a.docx", null, {}, false);
    await act(async () => undefined);
    expect(screen.queryByTestId("office-download-start")).toBeNull();
    expect(screen.getByText(/package manager/)).toBeTruthy();
  });
});

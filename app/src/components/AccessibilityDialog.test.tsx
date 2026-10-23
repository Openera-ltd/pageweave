import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot, OpenedDoc, OperationRequest } from "../types";
import { AccessibilityDialog, summaryLine, type AccessibilityReport } from "./AccessibilityDialog";

const doc = { doc_id: 1, path: "/f/a.pdf", name: "a.pdf", info: { page_count: 3, pages: [] } } as unknown as OpenedDoc;
const report: AccessibilityReport = {
  tagged: true,
  passed: 1,
  failed: 1,
  warnings: 0,
  manual: 1,
  checks: [
    { id: "figures_alt", title: "Pictures have alternate text", status: "fail", detail: "1 of 2 …", pages: [2], names: [] },
    { id: "title", title: "Document title", status: "pass", detail: "ok", pages: [], names: [] },
    { id: "reading_order", title: "Reading order", status: "manual", detail: "Check…", pages: [], names: [] },
  ],
};

afterEach(cleanup);

describe("Accessibility check", () => {
  it("runs the check, groups results with text labels, and jumps to a page", async () => {
    const startOperation = vi.fn(async (_r: OperationRequest) => "a1");
    const onGoToPage = vi.fn();
    const onClose = vi.fn();
    const api = { startOperation } as unknown as Api;
    const ui = (jobs: Record<string, JobSnapshot>) => (
      <ApiContext.Provider value={api}>
        <AccessibilityDialog doc={doc} jobs={jobs} onClose={onClose} onGoToPage={onGoToPage} />
      </ApiContext.Provider>
    );
    const r = render(ui({}));
    expect(screen.getByTestId("a11y-summary").textContent).toBe("Checking…");
    await vi.waitFor(() => expect(startOperation).toHaveBeenCalled());
    expect(startOperation.mock.calls[0]![0].operation).toEqual({ op: "check_accessibility", input: "/f/a.pdf" });
    r.rerender(ui({ a1: { id: "a1", state: "succeeded", result: { data: report } } as unknown as JobSnapshot }));
    expect(screen.getByTestId("a11y-summary").textContent).toBe(summaryLine(report));
    expect(screen.getByTestId("a11y-group-fail").textContent).toContain("Fails");
    expect(screen.getByTestId("a11y-group-manual").textContent).toContain("Reading order");
    expect(screen.queryByTestId("a11y-group-warning")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Go to page 3 (Pictures have alternate text)" }));
    expect(onGoToPage).toHaveBeenCalledWith(2);
    expect(onClose).toHaveBeenCalled();
  });
});

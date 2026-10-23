import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot, OperationRequest } from "../types";
import { BatchDialog } from "./BatchDialog";

afterEach(cleanup);

describe("Process several files", () => {
  it("submits one job per file and reports each result, failures with their reason", async () => {
    let n = 0;
    const startOperation = vi.fn(async (_r: OperationRequest) => `j${++n}`);
    const api = { startOperation, cancelJob: vi.fn(async () => true), pickPdfFiles: vi.fn(), pickFolder: vi.fn(), revealInFolder: vi.fn() } as unknown as Api;
    const ui = (jobs: Record<string, JobSnapshot>) => (
      <ApiContext.Provider value={api}>
        <BatchDialog openFiles={["/d/a.pdf", "/d/b.pdf"]} jobs={jobs} onClose={() => undefined} />
      </ApiContext.Provider>
    );
    const r = render(ui({}));
    fireEvent.click(screen.getByTestId("batch-add-open"));
    await act(async () => {
      fireEvent.click(screen.getByTestId("batch-start"));
    });
    expect(startOperation).toHaveBeenCalledTimes(2);
    expect(startOperation.mock.calls[0]![0]).toMatchObject({ operation: { op: "optimize", input: "/d/a.pdf", output: "/d/a-smaller.pdf" }, collision: "rename" });
    const job = (state: string, extra: object = {}) => ({ state, progress: { kind: "indeterminate" }, ...extra }) as unknown as JobSnapshot;
    r.rerender(ui({ j1: job("running"), j2: job("queued") }));
    expect(screen.getByTestId("batch-summary").textContent).toBe("0 done · 2 to go (of 2)");
    fireEvent.click(screen.getByTestId("batch-stop"));
    expect(api.cancelJob).toHaveBeenCalledTimes(2);
    r.rerender(
      ui({
        j1: job("succeeded", { result: { outputs: [{ path: "/d/a-smaller.pdf" }] } }),
        j2: job("failed", { error: { message: "There is nothing to gain: the copy would not be smaller." } }),
      }),
    );
    expect(screen.getByTestId("batch-summary").textContent).toBe("1 done · 1 failed (of 2)");
    expect(screen.getByTestId("batch-results").textContent).toContain("nothing to gain");
    expect(screen.queryByTestId("batch-stop")).toBeNull();
  });
});

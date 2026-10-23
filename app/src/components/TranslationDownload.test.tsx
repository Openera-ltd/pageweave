import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot } from "../types";
import { TranslationDownload } from "./TranslationDownload";

const needs = [
  { source: "de", target: "en", installed: false, bytes: 111_000_000 },
  { source: "en", target: "fr", installed: false, bytes: 113_000_000 },
];

const snap = (id: string, state: JobSnapshot["state"], extra: Partial<JobSnapshot> = {}): JobSnapshot => ({
  id,
  label: "install_translation",
  state,
  phase: "",
  progress: { kind: "indeterminate" },
  result: null,
  error: null,
  warnings: [],
  elapsed_ms: 0,
  ...extra,
});

function setup() {
  let n = 0;
  const startOperation = vi.fn(async (_request: { operation: unknown }) => `job-${++n}`);
  const cancelJob = vi.fn(async () => undefined);
  const api = { startOperation, cancelJob } as unknown as Api;
  const onDone = vi.fn();
  const view = render(
    <ApiContext.Provider value={api}>
      <TranslationDownload needs={needs} jobs={{}} onDone={onDone} />
    </ApiContext.Provider>,
  );
  const rerender = (jobs: Record<string, JobSnapshot>) =>
    view.rerender(
      <ApiContext.Provider value={api}>
        <TranslationDownload needs={needs} jobs={jobs} onDone={onDone} />
      </ApiContext.Provider>,
    );
  return { startOperation, cancelJob, onDone, rerender };
}

afterEach(cleanup);

describe("translation download", () => {
  it("says what is needed and how big, downloads each direction in turn with progress, then reports done", async () => {
    const { startOperation, onDone, rerender } = setup();
    expect(screen.getByTestId("translate-download").textContent).toContain("German → English, English → French (224 MB)");
    await act(async () => fireEvent.click(screen.getByTestId("translate-download-start")));
    expect(startOperation.mock.calls[0]![0]).toEqual({ schema_version: 1, operation: { op: "install_translation", source: "de", target: "en" }, collision: "fail" });
    await act(async () => rerender({ "job-1": snap("job-1", "running", { progress: { kind: "determinate", done: 55_500_000, total: 111_000_000, unit: "bytes" } }) }));
    expect(screen.getByTestId("translate-download").textContent).toContain("Downloading German → English: 56 MB of 111 MB");
    await act(async () => rerender({ "job-1": snap("job-1", "succeeded") }));
    expect(startOperation.mock.calls[1]![0].operation).toEqual({ op: "install_translation", source: "en", target: "fr" });
    await act(async () => rerender({ "job-1": snap("job-1", "succeeded"), "job-2": snap("job-2", "succeeded") }));
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("can be cancelled, and a failure says why and offers Try again", async () => {
    const { cancelJob, onDone, rerender } = setup();
    await act(async () => fireEvent.click(screen.getByTestId("translate-download-start")));
    await act(async () => rerender({ "job-1": snap("job-1", "running") }));
    await act(async () => fireEvent.click(screen.getByTestId("translate-download-cancel")));
    expect(cancelJob).toHaveBeenCalledWith("job-1");
    await act(async () => rerender({ "job-1": snap("job-1", "cancelled") }));
    expect(screen.getByTestId("translate-download-start").textContent).toBe("Download (224 MB)");
    await act(async () => fireEvent.click(screen.getByTestId("translate-download-start")));
    const err = { category: "network", code: "translate.download", message: "The translation model could not be downloaded (timed out).", action: "", retryable: true };
    await act(async () => rerender({ "job-2": snap("job-2", "failed", { error: err as never }) }));
    expect(screen.getByRole("alert").textContent).toContain("could not be downloaded");
    expect(screen.getByTestId("translate-download-start").textContent).toBe("Try again");
    expect(onDone).not.toHaveBeenCalled();
  });
});

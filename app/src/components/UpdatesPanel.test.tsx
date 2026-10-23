import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { AvailableUpdate, JobSnapshot, UpdateInfo } from "../types";
import { UpdatesPanel } from "./UpdatesPanel";

const update: AvailableUpdate = { version: "0.2.0", current: "0.1.0", notes: "Faster search.", pub_date: null, bytes: 52_000_000, downloaded: false };

const snap = (state: JobSnapshot["state"], extra: Partial<JobSnapshot> = {}): JobSnapshot => ({
  id: "job-1",
  label: "download_update",
  state,
  phase: "",
  progress: { kind: "indeterminate" },
  result: null,
  error: null,
  warnings: [],
  elapsed_ms: 0,
  ...extra,
});

function setup(info: UpdateInfo, found: AvailableUpdate | null | Error = update) {
  let emit: (j: JobSnapshot) => void = () => undefined;
  const api = {
    updateInfo: vi.fn(async () => info),
    checkForUpdate: vi.fn(async () => {
      if (found instanceof Error) throw { category: "network", code: "update.download", message: "The update could not be downloaded (offline).", action: "", retryable: true };
      return found;
    }),
    startOperation: vi.fn(async (_r: { operation: unknown }) => "job-1"),
    cancelJob: vi.fn(async () => true),
    installUpdate: vi.fn(async () => undefined),
    openReleasesPage: vi.fn(async () => undefined),
    onJobUpdate: vi.fn(async (cb: (j: JobSnapshot) => void) => {
      emit = cb;
      return () => undefined;
    }),
  };
  const onAuto = vi.fn();
  render(
    <ApiContext.Provider value={api as unknown as Api}>
      <UpdatesPanel checkOnStart onCheckOnStart={onAuto} />
    </ApiContext.Provider>,
  );
  return { api, onAuto, emit: (j: JobSnapshot) => act(async () => emit(j)) };
}

const windows: UpdateInfo = { current: "0.1.0", can_install: true, platform_supported: true };

describe("UpdatesPanel", () => {
  afterEach(cleanup);

  it("checks, downloads with progress, then installs", async () => {
    const { api, emit } = setup(windows);
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    expect(screen.getByTestId("updates-status").textContent).toContain("Version 0.2.0 is available (52 MB)");
    expect(screen.getByTestId("updates-status").textContent).toContain("Faster search.");
    await act(async () => fireEvent.click(screen.getByTestId("updates-download")));
    expect(api.startOperation.mock.calls[0]![0].operation).toEqual({ op: "download_update", version: "0.2.0" });
    await emit(snap("running", { progress: { kind: "determinate", done: 26_000_000, total: 52_000_000, unit: "bytes" } }));
    expect(screen.getByTestId("updates-status").textContent).toContain("Downloading: 26 MB of 52 MB");
    await emit(snap("succeeded"));
    await act(async () => fireEvent.click(screen.getByTestId("updates-install")));
    expect(api.installUpdate).toHaveBeenCalledWith("0.2.0");
  });

  it("says when it is up to date, shows errors, and cancels downloads", async () => {
    const { onAuto } = setup(windows, null);
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    expect(screen.getByTestId("updates-status").textContent).toBe("PageWeave is up to date.");
    fireEvent.click(screen.getByTestId("updates-auto"));
    expect(onAuto).toHaveBeenCalledWith(false);
    cleanup();

    setup(windows, new Error("offline"));
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    expect(screen.getByRole("alert").textContent).toContain("offline");
    cleanup();

    const c = setup(windows);
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    await act(async () => fireEvent.click(screen.getByTestId("updates-download")));
    await act(async () => fireEvent.click(screen.getByTestId("updates-cancel")));
    expect(c.api.cancelJob).toHaveBeenCalledWith("job-1");
    await c.emit(snap("cancelled"));
    expect(screen.getByTestId("updates-download")).toBeInTheDocument();
  });

  it("a failed download offers Try again; an install that is refused says why", async () => {
    const { api, emit } = setup(windows);
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    await act(async () => fireEvent.click(screen.getByTestId("updates-download")));
    await emit(snap("failed", { error: { category: "security_policy", code: "update.signature", message: "The update was not installed because its signature is not valid.", action: "", retryable: false } }));
    expect(screen.getByRole("alert").textContent).toContain("signature is not valid");
    expect(screen.getByTestId("updates-download").textContent).toBe("Try again");
    await act(async () => fireEvent.click(screen.getByTestId("updates-download")));
    await emit(snap("succeeded"));
    api.installUpdate.mockRejectedValueOnce({ category: "output_conflict", code: "update.unsaved", message: "Save or discard your unsaved changes before updating.", action: "", retryable: false });
    await act(async () => fireEvent.click(screen.getByTestId("updates-install")));
    expect(screen.getByRole("alert").textContent).toContain("unsaved changes");
    expect(screen.getByTestId("updates-install").textContent).toBe("Install and restart");
  });

  it("copies that cannot install themselves point to the releases page", async () => {
    const { api } = setup({ current: "0.1.0", can_install: false, platform_supported: true });
    await act(async () => fireEvent.click(screen.getByTestId("updates-check")));
    expect(screen.queryByTestId("updates-download")).toBeNull();
    fireEvent.click(screen.getByTestId("updates-releases"));
    expect(api.openReleasesPage).toHaveBeenCalled();
  });
});

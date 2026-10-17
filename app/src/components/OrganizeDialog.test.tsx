import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { OpenedDoc, OperationRequest } from "../types";
import { OrganizeDialog } from "./OrganizeDialog";

const page = (index: number) => ({ index, width_pt: 612, height_pt: 792, rotation: 0 as const, label: null });
const doc: OpenedDoc = {
  doc_id: 7,
  path: "/docs/report.pdf",
  name: "report.pdf",
  already_open: false,
  info: { page_count: 3, file_bytes: 1, sha256: "", pdf_version: "1.7", title: null, author: null, encrypted: false, has_xfa: false, pages: [page(0), page(1), page(2)], bookmarks: [], warnings: [] },
};

function setup(overrides: Partial<Api> = {}) {
  const startOperation = vi.fn(async (_req: OperationRequest) => "job-1");
  const api = {
    pageUrl: (d: number, p: number) => `pwpage://${d}/${p}`,
    startOperation,
    cancelJob: vi.fn(),
    closeDocument: vi.fn(async () => true),
    pickPdfFiles: vi.fn(async () => ["/docs/extra.pdf"]),
    openDocument: vi.fn(async () => ({ ...doc, doc_id: 9, path: "/docs/extra.pdf", name: "extra.pdf", info: { ...doc.info, page_count: 1, pages: [page(0)] } })),
    pickSavePath: vi.fn(),
    ...overrides,
  } as unknown as Api;
  const utils = render(
    <ApiContext.Provider value={api}>
      <OrganizeDialog doc={doc} jobs={{}} onClose={() => undefined} onFinished={() => undefined} />
    </ApiContext.Provider>,
  );
  const refs = () => Array.from(utils.container.ownerDocument.querySelectorAll<HTMLElement>("[data-page-ref]")).map((t) => t.dataset.pageRef);
  return { api, startOperation, refs, utils };
}

afterEach(cleanup);

describe("OrganizeDialog", () => {
  it("starts unchanged with Save disabled and a reason", () => {
    const { refs } = setup();
    expect(refs()).toEqual(["0:0:0", "0:1:0", "0:2:0"]);
    const save = screen.getByTestId("organize-save");
    expect(save.getAttribute("aria-disabled")).toBe("true");
    expect(save.getAttribute("title")).toMatch(/No changes yet/);
  });

  it("keyboard: select, move, rotate, duplicate, delete, undo/redo", () => {
    const { refs } = setup();
    const grid = screen.getByRole("listbox");
    grid.focus();
    fireEvent.keyDown(grid, { key: "ArrowRight" }); // select page 2
    fireEvent.keyDown(grid, { key: "ArrowRight", ctrlKey: true }); // move it after page 3
    expect(refs()).toEqual(["0:0:0", "0:2:0", "0:1:0"]);
    fireEvent.keyDown(grid, { key: "r" });
    expect(refs()[2]).toBe("0:1:90");
    expect(screen.getAllByRole("option")[2]!.textContent).toBe("3 · page 2, turned 90° right");
    fireEvent.keyDown(grid, { key: "R", shiftKey: true });
    fireEvent.keyDown(grid, { key: "R", shiftKey: true });
    expect(refs()[2]).toBe("0:1:270");
    fireEvent.keyDown(grid, { key: "d", ctrlKey: true });
    expect(refs()).toHaveLength(4);
    fireEvent.keyDown(grid, { key: "Delete" });
    expect(refs()).toHaveLength(3);
    fireEvent.keyDown(grid, { key: "z", ctrlKey: true });
    expect(refs()).toHaveLength(4);
    fireEvent.keyDown(grid, { key: "y", ctrlKey: true });
    expect(refs()).toHaveLength(3);
    expect(screen.getAllByRole("option").filter((o) => o.getAttribute("aria-selected") === "true")).toHaveLength(1);
  });

  it("mouse: dragging a tile onto another moves it there", () => {
    const { refs } = setup();
    const tiles = screen.getAllByRole("option");
    const dataTransfer = { setData: vi.fn(), effectAllowed: "" };
    fireEvent.dragStart(tiles[2]!, { dataTransfer });
    fireEvent.dragOver(tiles[0]!, { dataTransfer });
    fireEvent.drop(tiles[0]!, { dataTransfer });
    expect(refs()).toEqual(["0:2:0", "0:0:0", "0:1:0"]);
  });

  it("toolbar buttons explain why they are unavailable and never act then", () => {
    const { refs } = setup();
    const earlier = screen.getByRole("button", { name: "Move earlier" });
    expect(earlier.getAttribute("aria-disabled")).toBe("true");
    expect(earlier.getAttribute("title")).toMatch(/Already first/);
    fireEvent.click(earlier);
    expect(refs()).toEqual(["0:0:0", "0:1:0", "0:2:0"]);
  });

  it("inserts pages from another file without adding it to recents, then saves the page list", async () => {
    const { api, startOperation, refs } = setup();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Insert pages from file/ }));
    });
    expect(api.openDocument).toHaveBeenCalledWith("/docs/extra.pdf", undefined, false);
    expect(refs()).toEqual(["0:0:0", "1:0:0", "0:1:0", "0:2:0"]);
    expect(screen.getAllByRole("option")[1]!.textContent).toBe("2 · extra.pdf page 1");
    await act(async () => {
      fireEvent.click(screen.getByTestId("organize-save"));
    });
    const req = startOperation.mock.calls[0]![0];
    expect(req.operation).toEqual({
      op: "organize_pages",
      input: "/docs/report.pdf",
      extra_inputs: ["/docs/extra.pdf"],
      pages: [
        { source: 0, page: 0, rotate: 0 },
        { source: 1, page: 0, rotate: 0 },
        { source: 0, page: 1, rotate: 0 },
        { source: 0, page: 2, rotate: 0 },
      ],
      output: "/docs/report-organized.pdf",
    });
  });
  it("inserts a blank page after the selected one and saves it as blank", async () => {
    const { refs, startOperation } = setup();
    fireEvent.click(document.querySelector<HTMLElement>('[data-index="1"]')!);
    fireEvent.click(document.querySelector<HTMLButtonElement>('[data-action="blank"]')!);
    expect(refs()).toEqual(["0:0:0", "0:1:0", "0:1:0:blank", "0:2:0"]);
    expect(screen.getByText(/blank page/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByTestId("organize-save"));
    });
    const req = startOperation.mock.calls[0]![0];
    expect(req.operation.op === "organize_pages" && req.operation.pages[2]).toEqual({ source: 0, page: 1, rotate: 0, blank: true });
  });
});

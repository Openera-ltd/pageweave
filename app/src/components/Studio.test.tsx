import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { Design, JobSnapshot, OperationRequest } from "../types";
import { Home } from "./Panels";
import { Studio, projectPath, type StudioSource } from "./Studio";

const design = (): Design => ({
  version: 1,
  width: 200,
  height: 100,
  pt_per_px: 0.75,
  background: "#ffffff",
  source_png: "",
  source_name: "flyer.png",
  objects: [
    { id: "shape-1", name: "Shape 1", kind: "shape", x: 0, y: 0, w: 200, h: 40, visible: true, locked: false, confidence: 0.99, bold: false, fill: "#123456" },
    { id: "picture-1", name: "Picture 1", kind: "image", x: 120, y: 50, w: 40, h: 40, visible: true, locked: false, confidence: 0, bold: false, png: "", note: "Kept as a picture." },
    { id: "text-1", name: "HELLO", kind: "text", x: 10, y: 10, w: 80, h: 20, visible: true, locked: false, confidence: 0.97, bold: true, text: "HELLO", color: "#ffffff", font_size: 28, baseline: 20 },
  ],
});

function setup(source: StudioSource, jobs: Record<string, JobSnapshot> = {}) {
  const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
  const loadDesign = vi.fn(async () => design());
  const saveDesign = vi.fn(async () => 1234);
  const pickDesignExport = vi.fn(async () => "/d/flyer.svg");
  const pickImageFiles = vi.fn(async () => ["/pics/logo.png"]);
  const loadPicture = vi.fn(async () => "iVBORw0KGgoNEW");
  const api = { startOperation, cancelJob: vi.fn(), loadDesign, saveDesign, pickDesignExport, pickImageFiles, loadPicture, revealInFolder: vi.fn(async () => undefined) } as unknown as Api;
  const onClose = vi.fn();
  const view = render(
    <ApiContext.Provider value={api}>
      <Studio source={source} jobs={jobs} onClose={onClose} />
    </ApiContext.Provider>,
  );
  const rerender = (j: Record<string, JobSnapshot>) =>
    view.rerender(
      <ApiContext.Provider value={api}>
        <Studio source={source} jobs={j} onClose={onClose} />
      </ApiContext.Provider>,
    );
  return { startOperation, loadDesign, saveDesign, pickDesignExport, pickImageFiles, loadPicture, onClose, rerender };
}

const job = (state: JobSnapshot["state"], out?: string): JobSnapshot => ({
  id: "job-1",
  label: "x",
  state,
  phase: "",
  progress: { done: 0, total: 0, unit: "" } as unknown as JobSnapshot["progress"],
  result: out ? ({ operation: "x", outputs: [{ path: out, bytes: 1, pages: null }], data: {}, warnings: [] } as unknown as JobSnapshot["result"]) : null,
  error: null,
  warnings: [],
  elapsed_ms: 1,
});

const flush = () => act(async () => undefined);

afterEach(cleanup);

describe("Design Studio", () => {
  it("names the project after the source and its page", () => {
    expect(projectPath({ input: "/d/Poster.pdf", page: 2 })).toBe("/d/Poster-page3.pwdesign");
    expect(projectPath({ input: "C:\\d\\menu.jpg", page: 0 })).toBe("C:\\d\\menu.pwdesign");
  });

  it("rebuilds a page through the engine, then opens the project it wrote", async () => {
    const s = setup({ kind: "rebuild", input: "/d/a.pdf", page: 1, docId: 7 });
    await flush();
    expect(s.startOperation).toHaveBeenCalledWith(
      { schema_version: 1, operation: { op: "reconstruct_design", input: "/d/a.pdf", page: 1, output: "/d/a-page2.pwdesign" }, collision: "rename" },
      7,
    );
    expect(screen.getByRole("progressbar")).toBeTruthy();
    s.rerender({ "job-1": job("succeeded", "/d/a-page2 (2).pwdesign") });
    await flush();
    expect(s.loadDesign).toHaveBeenCalledWith("/d/a-page2 (2).pwdesign");
    expect(document.querySelectorAll("[data-layer]")).toHaveLength(3);
  });

  it("lists layers front first, flags uncertain ones and edits the selected text with undo", async () => {
    const s = setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    expect([...document.querySelectorAll<HTMLElement>("[data-layer]")].map((b) => b.dataset.layer)).toEqual(["text-1", "picture-1", "shape-1"]);
    expect(screen.getByTestId("studio-summary").textContent).toContain("Check these (1)");
    fireEvent.click(document.querySelector('[data-layer="text-1"]')!);
    const text = screen.getByLabelText("Text") as HTMLTextAreaElement;
    fireEvent.change(text, { target: { value: "HELLO THERE" } });
    expect(document.querySelector('[data-object="text-1"]')!.textContent).toBe("HELLO THERE");
    expect(screen.getByTestId("studio-save").getAttribute("aria-disabled")).toBeNull();
    fireEvent.keyDown(text, { key: "z", ctrlKey: true });
    expect(document.querySelector('[data-object="text-1"]')!.textContent).toBe("HELLO");
    fireEvent.keyDown(text, { key: "y", ctrlKey: true });
    expect(document.querySelector('[data-object="text-1"]')!.textContent).toBe("HELLO THERE");
    // Save writes the edited design over the project.
    await act(async () => {
      fireEvent.keyDown(text, { key: "s", ctrlKey: true });
    });
    expect(s.saveDesign).toHaveBeenCalledWith("/d/f.pwdesign", expect.objectContaining({ objects: expect.arrayContaining([expect.objectContaining({ text: "HELLO THERE" })]) }));
    expect(screen.getByTestId("studio-status").textContent).toContain("Saved f.pwdesign");
    expect(screen.getByTestId("studio-save").getAttribute("aria-disabled")).toBe("true");
  });

  it("nudges with the arrow keys, deletes, and asks before closing with unsaved changes", async () => {
    const s = setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(document.querySelector('[data-layer="shape-1"]')!);
    const stage = screen.getByTestId("studio-stage");
    fireEvent.keyDown(stage, { key: "ArrowDown", shiftKey: true });
    fireEvent.keyDown(stage, { key: "ArrowRight" });
    expect((document.querySelector('[data-object="shape-1"]') as SVGRectElement).getAttribute("y")).toBe("10");
    expect((document.querySelector('[data-object="shape-1"]') as SVGRectElement).getAttribute("x")).toBe("1");
    fireEvent.keyDown(stage, { key: "Delete" });
    expect(document.querySelector('[data-object="shape-1"]')).toBeNull();
    fireEvent.click(screen.getByTestId("studio-close"));
    expect(s.onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    fireEvent.click(screen.getByTestId("studio-close-discard"));
    expect(s.onClose).toHaveBeenCalled();
    expect(s.saveDesign).not.toHaveBeenCalled();
  });

  it("hides and locks from the layer list; a locked object does not move", async () => {
    setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(screen.getByLabelText("Lock Picture 1"));
    fireEvent.click(document.querySelector('[data-layer="picture-1"]')!);
    fireEvent.keyDown(screen.getByTestId("studio-stage"), { key: "ArrowLeft" });
    expect(document.querySelector('[data-object="picture-1"]')!.getAttribute("x")).toBe("120");
    fireEvent.click(screen.getByLabelText("Hide Picture 1"));
    expect(document.querySelector('[data-object="picture-1"]')).toBeNull();
    expect(screen.getByLabelText("Show Picture 1")).toBeTruthy();
  });

  it("exports through the engine after saving, and reports the file", async () => {
    const s = setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(document.querySelector('[data-layer="text-1"]')!);
    fireEvent.change(screen.getByLabelText("Text"), { target: { value: "NEW" } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("studio-export-svg"));
    });
    expect(s.saveDesign).toHaveBeenCalled();
    expect(s.pickDesignExport).toHaveBeenCalledWith("/d/f.svg", "svg");
    expect(s.startOperation).toHaveBeenCalledWith({ schema_version: 1, operation: { op: "export_design", input: "/d/f.pwdesign", format: "svg", output: "/d/flyer.svg" }, collision: "replace" });
    s.rerender({ "job-1": job("succeeded", "/d/flyer.svg") });
    await flush();
    expect(screen.getByTestId("studio-status").textContent).toContain("Exported flyer.svg");
  });

  it("adds text and rectangles in front, selected, as undoable steps", async () => {
    setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(screen.getByTestId("studio-add-text"));
    expect(document.querySelectorAll("[data-layer]")[0]!.textContent).toContain("New text");
    expect((screen.getByLabelText("Text") as HTMLTextAreaElement).value).toBe("New text");
    fireEvent.click(screen.getByTestId("studio-add-rect"));
    expect(document.querySelectorAll("[data-layer]")).toHaveLength(5);
    fireEvent.keyDown(screen.getByTestId("studio-stage"), { key: "z", ctrlKey: true });
    fireEvent.keyDown(screen.getByTestId("studio-stage"), { key: "z", ctrlKey: true });
    expect(document.querySelectorAll("[data-layer]")).toHaveLength(3);
  });

  it("replaces a picture with a chosen file, keeping its place; undo brings the old one back", async () => {
    const s = setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(document.querySelector('[data-layer="picture-1"]')!);
    await act(async () => {
      fireEvent.click(screen.getByTestId("studio-replace"));
    });
    expect(s.loadPicture).toHaveBeenCalledWith("/pics/logo.png");
    const img = document.querySelector('[data-object="picture-1"]')!;
    expect(img.getAttribute("href")).toBe("data:image/png;base64,iVBORw0KGgoNEW");
    expect(img.getAttribute("x")).toBe("120");
    expect(document.querySelector('[data-layer="picture-1"]')!.textContent).toContain("logo.png");
    fireEvent.keyDown(screen.getByTestId("studio-stage"), { key: "z", ctrlKey: true });
    expect(document.querySelector('[data-object="picture-1"]')!.getAttribute("href")).toBe("data:image/png;base64,");
  });

  it("Shift+click selects several; align, group (selects together), select all and delete many", async () => {
    setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(document.querySelector('[data-layer="shape-1"]')!);
    fireEvent.click(document.querySelector('[data-layer="picture-1"]')!, { shiftKey: true });
    expect(screen.getByTestId("studio-arrange").textContent).toContain("2 objects");
    // Align left: both on the left edge of their box (shape-1 at x 0).
    fireEvent.click(document.querySelector('[data-align="left"]')!);
    expect(document.querySelector('[data-object="picture-1"]')!.getAttribute("x")).toBe("0");
    // Distribute needs three.
    expect(document.querySelector('[data-distribute="x"]')!.getAttribute("aria-disabled")).toBe("true");
    // Group; a plain click on one member selects the whole group.
    fireEvent.click(screen.getByTestId("studio-group"));
    fireEvent.click(document.querySelector('[data-layer="text-1"]')!);
    fireEvent.click(document.querySelector('[data-layer="shape-1"]')!);
    expect(document.querySelectorAll("[data-layer][aria-current]")).toHaveLength(2);
    // Arrow keys move the whole selection; Ctrl+A selects everything; Delete removes the selection.
    const stage = screen.getByTestId("studio-stage");
    fireEvent.keyDown(stage, { key: "ArrowDown" });
    expect(document.querySelector('[data-object="shape-1"]')!.getAttribute("y")).toBe("1");
    expect(document.querySelector('[data-object="picture-1"]')!.getAttribute("y")).toBe("51");
    fireEvent.keyDown(stage, { key: "a", ctrlKey: true });
    expect(screen.getByTestId("studio-arrange").textContent).toContain("3 objects");
    fireEvent.keyDown(stage, { key: "Delete" });
    expect(document.querySelectorAll("[data-object]")).toHaveLength(0);
    fireEvent.keyDown(stage, { key: "z", ctrlKey: true });
    expect(document.querySelectorAll("[data-object]")).toHaveLength(3);
  });

  it("rotates from the field and by 90°, drawn turned around the centre", async () => {
    setup({ kind: "open", path: "/d/f.pwdesign" });
    await flush();
    fireEvent.click(document.querySelector('[data-layer="shape-1"]')!);
    fireEvent.click(screen.getByLabelText("Rotate 90° clockwise"));
    expect(document.querySelector('[data-object="shape-1"]')!.getAttribute("transform")).toBe("rotate(90 100 20)");
    fireEvent.change(screen.getByTestId("studio-rotation"), { target: { value: "-30" } });
    expect(document.querySelector('[data-object="shape-1"]')!.getAttribute("transform")).toBe("rotate(-30 100 20)");
    // A turned object has no corner handles (it is sized in its fields).
    expect(document.querySelector("[data-corner]")).toBeNull();
  });

  it("the start screen offers Open in Design Studio (studio.open)", () => {
    const onStudio = vi.fn();
    render(
      <Home recent={[]} dropActive={false} onOpen={vi.fn()} onImagesToPdf={vi.fn()} onBatch={vi.fn()} onOpenOffice={vi.fn()} onStudio={onStudio} onOpenRecent={vi.fn()} onRemoveRecent={vi.fn()} />,
    );
    fireEvent.click(document.querySelector('[data-command="studio.open"]')!);
    expect(onStudio).toHaveBeenCalled();
  });
});

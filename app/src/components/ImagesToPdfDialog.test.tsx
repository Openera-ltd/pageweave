import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { OperationRequest } from "../types";
import { ImagesToPdfDialog, defaultImagesOutput, isImagePath } from "./ImagesToPdfDialog";

function setup(initial: string[] = []) {
  const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
  const api = { startOperation, cancelJob: vi.fn(), pickImageFiles: vi.fn(async () => ["/pics/b.png", "/pics/c.jpg"]), pickSavePath: vi.fn() } as unknown as Api;
  render(
    <ApiContext.Provider value={api}>
      <ImagesToPdfDialog initialFiles={initial} jobs={{}} onClose={() => undefined} onFinished={() => undefined} />
    </ApiContext.Provider>,
  );
  return { startOperation };
}

afterEach(cleanup);

describe("Create PDF from images", () => {
  it("recognises image files and names the output next to the first one", () => {
    expect(isImagePath("/a/photo.JPEG")).toBe(true);
    expect(isImagePath("/a/scan.png")).toBe(true);
    expect(isImagePath("/a/doc.pdf")).toBe(false);
    expect(defaultImagesOutput(["/pics/a.jpg"])).toBe("/pics/images.pdf");
  });

  it("needs images, then creates the PDF in the listed order with the margin in points", async () => {
    const { startOperation } = setup();
    const start = screen.getByTestId("images-start");
    expect(start.getAttribute("title")).toBe("Add images first.");
    await act(async () => {
      fireEvent.click(screen.getByTestId("images-add"));
    });
    expect((screen.getByLabelText(/Save as/) as HTMLInputElement).value).toBe("/pics/images.pdf");
    fireEvent.click(screen.getByRole("button", { name: "Move c.jpg up" }));
    fireEvent.change(screen.getByLabelText("Margin (mm)"), { target: { value: "10" } });
    fireEvent.change(screen.getByLabelText("Page size"), { target: { value: "letter" } });
    await act(async () => {
      fireEvent.click(start);
    });
    const op = startOperation.mock.calls[0]![0].operation;
    expect(op.op).toBe("images_to_pdf");
    if (op.op === "images_to_pdf") {
      expect(op.inputs).toEqual(["/pics/c.jpg", "/pics/b.png"]);
      expect(op.page_size).toBe("letter");
      expect(op.margin).toBeCloseTo(28.35, 2);
    }
  });

  it("explains an impossible margin", () => {
    setup(["/pics/a.jpg"]);
    fireEvent.change(screen.getByLabelText("Margin (mm)"), { target: { value: "80" } });
    expect(screen.getByTestId("images-start").getAttribute("title")).toBe("Use a margin of 0 to 50 mm.");
  });
});

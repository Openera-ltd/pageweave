import { describe, expect, it } from "vitest";
import { addFiles, batchOperation, batchOutputs, summary } from "./batch";

describe("batch processing", () => {
  it("names outputs in the folder, unique within the batch", () => {
    expect(batchOutputs(["/a/report.pdf", "/b/Report.pdf", "/c/scan.pdf"], "optimize", "/out")).toEqual([
      "/out/report-smaller.pdf",
      "/out/Report-smaller (2).pdf",
      "/out/scan-smaller.pdf",
    ]);
    expect(batchOutputs(["C:\\docs\\x.pdf"], "text", "C:\\out")).toEqual(["C:\\out\\x.txt"]);
    expect(batchOutputs(["C:\\docs\\x.pdf"], "word", "C:\\out")).toEqual(["C:\\out\\x.docx"]);
    expect(batchOperation("word", "/a.pdf", "/a.docx")).toEqual({ op: "convert_to_docx", input: "/a.pdf", pages: null, output: "/a.docx" });
  });
  it("builds the typed operation for each action", () => {
    expect(batchOperation("ocr", "/a.pdf", "/o.pdf")).toEqual({ op: "ocr_pages", input: "/a.pdf", pages: null, skip_text_pages: true, dpi: 200, output: "/o.pdf" });
    expect(batchOperation("optimize_images", "/a.pdf", "/o.pdf")).toMatchObject({ op: "optimize", images: { max_dpi: 150, quality: 75 } });
    expect(batchOperation("orient", "/a.pdf", "/o.pdf").op).toBe("fix_orientation");
  });
  it("adds PDFs once and summarises progress", () => {
    expect(addFiles(["/a.pdf"], ["/A.PDF", "/b.pdf", "/c.png"])).toEqual(["/a.pdf", "/b.pdf"]);
    expect(summary(["done", "failed", "waiting", "running"])).toBe("1 done · 1 failed · 2 to go (of 4)");
    expect(summary(["done", "cancelled"])).toBe("1 done · 1 cancelled (of 2)");
  });
});

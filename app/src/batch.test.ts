import { describe, expect, it } from "vitest";
import { addFiles, batchOperation, batchOutputs, summary, translateBlocker } from "./batch";
import type { TranslationDirection } from "./types";

describe("batch processing", () => {
  it("names outputs in the folder, unique within the batch", () => {
    expect(batchOutputs(["/a/report.pdf", "/b/Report.pdf", "/c/scan.pdf"], "optimize", "/out")).toEqual([
      "/out/report-smaller.pdf",
      "/out/Report-smaller (2).pdf",
      "/out/scan-smaller.pdf",
    ]);
    expect(batchOutputs(["C:\\docs\\x.pdf"], "text", "C:\\out")).toEqual(["C:\\out\\x.txt"]);
    expect(batchOutputs(["C:\\docs\\x.pdf"], "word", "C:\\out")).toEqual(["C:\\out\\x.docx"]);
    expect(batchOperation("slides", "/a.pdf", "/a.pptx")).toEqual({ op: "convert_to_pptx", input: "/a.pdf", pages: null, output: "/a.pptx" });
    expect(batchOperation("word", "/a.pdf", "/a.docx")).toEqual({ op: "convert_to_docx", input: "/a.pdf", pages: null, output: "/a.docx" });
  });
  it("builds the typed operation for each action", () => {
    expect(batchOperation("ocr", "/a.pdf", "/o.pdf")).toEqual({ op: "ocr_pages", input: "/a.pdf", pages: null, skip_text_pages: true, dpi: 200, output: "/o.pdf" });
    expect(batchOperation("optimize_images", "/a.pdf", "/o.pdf")).toMatchObject({ op: "optimize", images: { max_dpi: 150, quality: 75 } });
    expect(batchOperation("orient", "/a.pdf", "/o.pdf").op).toBe("fix_orientation");
  });
  it("translates each file into the chosen language, detected source, named after the target", () => {
    expect(batchOutputs(["/a/report.pdf"], "translate", "/out", "de")).toEqual(["/out/report-de.pdf"]);
    expect(batchOperation("translate", "/a/report.pdf", "/out/report-de.pdf", "de")).toEqual({
      op: "translate_document",
      input: "/a/report.pdf",
      pages: null,
      source: null,
      target: "de",
      output: "/out/report-de.pdf",
    });
    const dir = (source: string, target: string, installed: boolean) => ({ source, target, installed }) as TranslationDirection;
    const dirs = [dir("en", "de", true), dir("de", "en", false), dir("en", "fr", false)];
    expect(translateBlocker(null, "de")).toContain("Reading");
    expect(translateBlocker(dirs, "de")).toBeNull();
    expect(translateBlocker(dirs, "fr")).toContain("No translation into French is downloaded");
    expect(translateBlocker(dirs, "en")).toContain("into English");
  });

  it("adds PDFs once and summarises progress", () => {
    expect(addFiles(["/a.pdf"], ["/A.PDF", "/b.pdf", "/c.png"])).toEqual(["/a.pdf", "/b.pdf"]);
    expect(summary(["done", "failed", "waiting", "running"])).toBe("1 done · 1 failed · 2 to go (of 4)");
    expect(summary(["done", "cancelled"])).toBe("1 done · 1 cancelled (of 2)");
  });
});

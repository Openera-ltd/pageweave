import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { JobSnapshot, OpenedDoc, OperationRequest } from "../types";
import { RedactPatternsDialog, type PatternHit } from "./RedactPatternsDialog";

const doc = { doc_id: 1, path: "/f/a.pdf", name: "a.pdf", info: { page_count: 2, pages: [] } } as unknown as OpenedDoc;
const hit = (page: number, kind: string, text: string): PatternHit => ({ page, kind, text, rects: [{ left: 1, bottom: 1, right: 2, top: 2 }] });

afterEach(cleanup);

describe("Mark patterns for redaction", () => {
  function setup() {
    const startOperation = vi.fn(async (_r: OperationRequest) => "job-1");
    const onMark = vi.fn((_h: PatternHit[]) => undefined);
    const api = { startOperation } as unknown as Api;
    const ui = (jobs: Record<string, JobSnapshot>) => (
      <ApiContext.Provider value={api}>
        <RedactPatternsDialog doc={doc} jobs={jobs} onClose={() => undefined} onMark={onMark} />
      </ApiContext.Provider>
    );
    const r = render(ui({}));
    return { startOperation, onMark, rerender: (j: Record<string, JobSnapshot>) => r.rerender(ui(j)) };
  }

  it("asks for something to look for, sends the chosen kinds and custom pattern, marks only ticked matches", async () => {
    const { startOperation, onMark, rerender } = setup();
    for (const k of ["email", "phone", "card_number", "iban", "us_ssn"]) fireEvent.click(document.querySelector(`input[data-kind="${k}"]`)!);
    expect(screen.getByTestId("patterns-find").getAttribute("title")).toBe("Choose what to look for.");
    fireEvent.click(document.querySelector('input[data-kind="email"]')!);
    fireEvent.change(screen.getByLabelText(/Also this pattern/), { target: { value: " INV-\\d+ " } });
    await act(async () => {
      fireEvent.click(screen.getByTestId("patterns-find"));
    });
    expect(startOperation.mock.calls[0]![0].operation).toEqual({ op: "find_patterns", input: "/f/a.pdf", patterns: ["email"], custom: "INV-\\d+", pages: null });
    const hits = [hit(0, "email", "a@b.org"), hit(1, "custom", "INV-12")];
    rerender({ "job-1": { id: "job-1", state: "succeeded", result: { data: { hits, truncated: false } } } as unknown as JobSnapshot });
    expect(screen.getByTestId("patterns-summary").textContent).toBe("2 matches on 2 pages");
    const boxes = screen.getByTestId("patterns-hits").querySelectorAll("input");
    fireEvent.click(boxes[1]!);
    expect(screen.getByTestId("patterns-mark").textContent).toBe("Mark 1 for redaction");
    fireEvent.click(screen.getByTestId("patterns-mark"));
    expect(onMark).toHaveBeenCalledWith([hits[0]]);
  });

  it("says when nothing was found and cannot mark", async () => {
    const { rerender, onMark } = setup();
    await act(async () => {
      fireEvent.click(screen.getByTestId("patterns-find"));
    });
    rerender({ "job-1": { id: "job-1", state: "succeeded", result: { data: { hits: [], truncated: false } } } as unknown as JobSnapshot });
    expect(screen.getByTestId("patterns-summary").textContent).toBe("No matches found.");
    expect(screen.getByTestId("patterns-mark").getAttribute("title")).toBe("Choose at least one match.");
    fireEvent.click(screen.getByTestId("patterns-mark"));
    expect(onMark).not.toHaveBeenCalled();
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { COMMANDS, type CommandHandlers } from "../commands";
import { TOOL_GROUPS, ToolsPanel, matches } from "./ToolsPanel";

afterEach(cleanup);

function handlers(disabled: string[] = []) {
  const run = vi.fn();
  const h = Object.fromEntries(
    COMMANDS.map((c) => [c.id, { run: () => run(c.id), available: () => (disabled.includes(c.id) ? "Open a PDF first." : true) }]),
  ) as unknown as CommandHandlers;
  return { h, run };
}

describe("All tools pane", () => {
  it("matches words from their start, not inside other words", () => {
    expect(matches("Convert to Word", "word")).toBe(true);
    expect(matches("Protect with a password", "word")).toBe(false);
    expect(matches("Protect with a password", "pass")).toBe(true);
    expect(matches("Read aloud", "read al")).toBe(true);
    expect(matches("Anything", "")).toBe(true);
  });

  it("lists every group, filters to one tool, runs it, and explains disabled tools", () => {
    const { h, run } = handlers(["file.compare"]);
    render(<ToolsPanel handlers={h} bindings={{}} onClose={() => undefined} />);
    expect(document.querySelectorAll(".tools-group")).toHaveLength(TOOL_GROUPS.length);
    const compare = document.querySelector('[data-tool="file.compare"]') as HTMLElement;
    expect(compare.getAttribute("aria-disabled")).toBe("true");
    expect(compare.title).toContain("Open a PDF first.");
    fireEvent.click(compare);
    expect(run).not.toHaveBeenCalled();
    fireEvent.change(screen.getByTestId("tools-filter"), { target: { value: "to word" } });
    const items = document.querySelectorAll(".tool-item");
    expect(items).toHaveLength(1);
    fireEvent.click(items[0]);
    expect(run).toHaveBeenCalledWith("file.toWord");
    fireEvent.change(screen.getByTestId("tools-filter"), { target: { value: "zzzz" } });
    expect(screen.getByText(/No tool matches/)).toBeTruthy();
  });

  it("closes", () => {
    const onClose = vi.fn();
    render(<ToolsPanel handlers={handlers().h} bindings={{}} onClose={onClose} />);
    fireEvent.click(screen.getByTestId("tools-close"));
    expect(onClose).toHaveBeenCalled();
  });
});

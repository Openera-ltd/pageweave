import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { COMMAND_BY_ID, COMMANDS, type CommandHandlers } from "../commands";
import { CONVERT_MENU, MENU_ICONS, PAGES_MENU, TopBar, barGroups, fitGroups, overflowCommands } from "./TopBar";

afterEach(cleanup);

function handlers(disabled: string[] = []) {
  const run = vi.fn();
  const h = Object.fromEntries(
    COMMANDS.map((c) => [c.id, { run: () => run(c.id), available: () => (disabled.includes(c.id) ? "Open a PDF first." : true) }]),
  ) as unknown as CommandHandlers;
  return { h, run };
}

function bar(overrides: Partial<Parameters<typeof TopBar>[0]> = {}) {
  const { h, run } = handlers();
  const onMenu = vi.fn();
  const onAppMenu = vi.fn();
  const props = {
    handlers: h,
    bindings: { "file.saveCopy": ["Ctrl+S"] },
    groups: barGroups({ reading: false, editMode: false }),
    openMenu: null,
    onAppMenu,
    onMenu,
    pressed: {},
    scaleKey: "compact-1",
    hasDoc: true,
    noDocReason: "Open a PDF first.",
    ...overrides,
  };
  render(<TopBar {...props} />);
  return { run, onMenu, onAppMenu };
}

describe("toolbar fitting", () => {
  const groups = [
    { id: "a", priority: 1 },
    { id: "b", priority: 3 },
    { id: "c", priority: 2 },
    { id: "d", priority: 3 },
  ];
  const widths = { a: 100, b: 100, c: 100, d: 100 };

  it("shows everything when it fits", () => {
    expect(fitGroups(groups, widths, 400, 30).size).toBe(0);
  });

  it("hides the highest priority numbers first (rightmost on ties) and leaves room for More", () => {
    expect([...fitGroups(groups, widths, 399, 30)].sort()).toEqual(["d"]);
    expect([...fitGroups(groups, widths, 250, 30)].sort()).toEqual(["b", "d"]);
    expect([...fitGroups(groups, widths, 50, 30)].sort()).toEqual(["a", "b", "c", "d"]);
  });

  it("never shows a group after one with a lower number was hidden", () => {
    // c (priority 2) does not fit after a; b and d must not take the space left over.
    const hidden = fitGroups(groups, { a: 100, b: 10, c: 200, d: 10 }, 200, 30);
    expect([...hidden].sort()).toEqual(["b", "c", "d"]);
  });

  it("puts the hidden groups' commands in the overflow menu once, dropdown entries included", () => {
    const all = barGroups({ reading: false, editMode: false });
    const more = overflowCommands(all, new Set(["comment", "pages"]));
    expect(more.slice(0, 4)).toEqual(["annotate.highlight", "annotate.underline", "annotate.strikeOut", "annotate.note"]);
    expect(more[4]).toBe("separator");
    const pages = more.slice(5);
    expect(pages.filter((c) => c === "pages.organize")).toHaveLength(1);
    expect(pages).toContain("pages.merge");
    expect(new Set(pages).size).toBe(pages.length);
  });

  it("lists a command once even when it is both a button and a dropdown entry", () => {
    const all = barGroups({ reading: false, editMode: true });
    const more = overflowCommands(all, new Set(all.map((g) => g.id))).filter((c) => c !== "separator");
    expect(more.filter((c) => c === "file.translate")).toHaveLength(1);
    expect(new Set(more).size).toBe(more.length);
  });

  it("offers Add text only in Edit mode and the stop command while reading", () => {
    const ids = (s: { reading: boolean; editMode: boolean }) => overflowCommands(barGroups(s), new Set(barGroups(s).map((g) => g.id)));
    expect(ids({ reading: false, editMode: false })).not.toContain("edit.addText");
    expect(ids({ reading: false, editMode: true })).toContain("edit.addText");
    expect(ids({ reading: true, editMode: false })).toContain("read.stop");
    expect(ids({ reading: true, editMode: false })).not.toContain("read.aloud");
  });

  it("every dropdown and toolbar command is registered and has a menu icon", () => {
    const groups = barGroups({ reading: false, editMode: true });
    const ids = [...overflowCommands(groups, new Set(groups.map((g) => g.id))), ...PAGES_MENU, ...CONVERT_MENU].filter((c) => c !== "separator");
    for (const id of ids) {
      expect(COMMAND_BY_ID.has(id), id).toBe(true);
      expect(MENU_ICONS[id], id).toBeTruthy();
    }
  });
});

describe("TopBar", () => {
  it("renders named icon buttons in labelled groups and runs their commands", () => {
    const { run } = bar();
    for (const name of ["File", "Undo and redo", "View", "Comment", "Edit", "Sign and protect", "Pages", "Convert and read"]) {
      expect(screen.getByRole("group", { name })).toBeTruthy();
    }
    const save = document.querySelector('[data-command="file.saveCopy"]') as HTMLElement;
    expect(save.getAttribute("aria-label")).toBe("Save a copy with changes");
    expect(save.title).toBe("Save a copy with changes (Ctrl+S)");
    fireEvent.click(save);
    expect(run).toHaveBeenCalledWith("file.saveCopy");
    fireEvent.click(document.querySelector('[data-command="annotate.highlight"]')!);
    expect(run).toHaveBeenCalledWith("annotate.highlight");
    expect(document.querySelector('[data-testid="toolbar-more"]')).toBeNull();
  });

  it("opens the dropdowns and the app menu at their buttons", () => {
    const { onMenu, onAppMenu } = bar();
    fireEvent.click(screen.getByTestId("menu-convert"));
    expect(onMenu).toHaveBeenCalledWith("Convert", expect.anything(), CONVERT_MENU);
    fireEvent.click(screen.getByTestId("menu-pages"));
    expect(onMenu).toHaveBeenCalledWith("Pages", expect.anything(), PAGES_MENU);
    fireEvent.click(screen.getByTestId("app-menu"));
    expect(onAppMenu).toHaveBeenCalled();
  });

  it("marks the open dropdown and pressed tools", () => {
    bar({ openMenu: "Convert", pressed: { "edit.toggleEditMode": true } });
    expect(screen.getByTestId("menu-convert").getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("menu-pages").getAttribute("aria-expanded")).toBe("false");
    expect(document.querySelector('[data-command="edit.toggleEditMode"]')!.getAttribute("aria-pressed")).toBe("true");
  });

  it("disables the dropdowns without a document, with the reason", () => {
    const { onMenu } = bar({ hasDoc: false });
    const convert = screen.getByTestId("menu-convert");
    expect(convert.getAttribute("aria-disabled")).toBe("true");
    expect(convert.title).toBe("Convert — Open a PDF first.");
    fireEvent.click(convert);
    expect(onMenu).not.toHaveBeenCalled();
  });

  describe("in a narrow window", () => {
    let restore: (() => void)[] = [];
    beforeEach(() => {
      const def = (prop: "offsetWidth" | "offsetHeight" | "clientWidth", get: (el: HTMLElement) => number) => {
        const orig = Object.getOwnPropertyDescriptor(HTMLElement.prototype, prop);
        Object.defineProperty(HTMLElement.prototype, prop, { configurable: true, get() { return get(this as HTMLElement); } });
        restore.push(() => orig && Object.defineProperty(HTMLElement.prototype, prop, orig));
      };
      def("offsetWidth", (el) => (el.dataset.group ? 100 : 0));
      def("offsetHeight", () => 28);
      def("clientWidth", (el) => (el.classList.contains("toolbar-groups") ? 450 : 0));
    });
    afterEach(() => {
      restore.forEach((r) => r());
      restore = [];
    });

    it("moves the groups that do not fit into More tools", () => {
      const { onMenu } = bar();
      // 8 groups of 100 px in 450 px with a 36 px More button: four stay (file, view, history, edit).
      const shown = [...document.querySelectorAll<HTMLElement>("[data-group]")].map((g) => g.dataset.group);
      expect(shown).toEqual(["file", "history", "view", "edit"]);
      fireEvent.click(screen.getByTestId("toolbar-more"));
      const [label, , ids] = onMenu.mock.calls[0]!;
      expect(label).toBe("More tools");
      expect(ids).toContain("annotate.highlight");
      expect(ids).toContain("file.toWord");
      expect(ids).toContain("file.protect");
      expect(ids).not.toContain("file.saveCopy");
    });
  });
});

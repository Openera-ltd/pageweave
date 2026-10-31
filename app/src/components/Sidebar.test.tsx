import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { AttachmentInfo, OpenedDoc } from "../types";
import { Sidebar } from "./Sidebar";

const doc: OpenedDoc = {
  doc_id: 3,
  path: "/docs/report.pdf",
  name: "report.pdf",
  already_open: false,
  info: { page_count: 1, file_bytes: 1, sha256: "", pdf_version: "1.7", title: null, author: null, encrypted: false, has_xfa: false, pages: [{ index: 0, width_pt: 612, height_pt: 792, rotation: 0, label: null }], bookmarks: [], warnings: [] },
};

function setup(list: () => Promise<AttachmentInfo[]>) {
  const onSave = vi.fn();
  const onAttach = vi.fn();
  const api = { listAttachments: vi.fn(list), pageUrl: () => "" } as unknown as Api;
  render(
    <ApiContext.Provider value={api}>
      <Sidebar doc={doc} view="attachments" onView={() => undefined} currentPage={0} onSelectPage={() => undefined} onPageContextMenu={() => undefined} onSaveAttachment={onSave} onAttachFiles={onAttach} onEditBookmarks={() => undefined} />
    </ApiContext.Provider>,
  );
  return { api, onSave, onAttach };
}

afterEach(cleanup);

describe("Attachments panel", () => {
  it("lists attached files and saves one through its button", async () => {
    const { onSave } = setup(async () => [{ index: 0, name: "terms.txt", bytes: 2048 }]);
    await screen.findByText("terms.txt");
    expect(screen.getByText("2.0 KB")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save terms.txt" }));
    expect(onSave).toHaveBeenCalledWith({ index: 0, name: "terms.txt", bytes: 2048 });
  });

  it("says when there is nothing attached, and attaching stays available", async () => {
    const { onAttach } = setup(async () => []);
    await screen.findByText("No attached files.");
    const attach = document.querySelector<HTMLButtonElement>('[data-command="file.attachFiles"]')!;
    fireEvent.click(attach);
    expect(onAttach).toHaveBeenCalledOnce();
  });

  it("shows why the list could not be read", async () => {
    setup(async () => Promise.reject({ category: "corrupt", code: "pdf.format", message: "The file is damaged.", retryable: false, action: "" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("The file is damaged."));
  });
});

describe("Comments panel", () => {
  const note = { page: 0, index: 0, subtype: "Text", rect: { left: 0, bottom: 0, right: 1, top: 1 }, text: "Fix this", author: "Ada", modified: null, color: "#ffd400", text_editable: true };
  const box = { ...note, index: 3, subtype: "FreeText", text: "Box", author: null, text_editable: false };
  function setupComments(changes = {}) {
    const onChanges = vi.fn();
    const onSelect = vi.fn();
    const api = { listAnnotations: vi.fn(async () => ({ annotations: [note, box], truncated: false })), pageUrl: () => "" } as unknown as Api;
    const r = render(
      <ApiContext.Provider value={api}>
        <Sidebar doc={doc} view="comments" onView={() => undefined} currentPage={0} onSelectPage={onSelect} onPageContextMenu={() => undefined} onSaveAttachment={() => undefined} onAttachFiles={() => undefined} onEditBookmarks={() => undefined} annotChanges={changes} onAnnotChanges={onChanges} />
      </ApiContext.Provider>,
    );
    return { onChanges, onSelect, r };
  }

  it("lists comments; delete and edit record pending changes; text boxes can't be edited", async () => {
    const { onChanges, onSelect } = setupComments();
    await screen.findByText("Fix this");
    expect(screen.getByText("Note")).toBeTruthy();
    expect(screen.getByText("Text box")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Delete Note on page 1" }));
    expect(onChanges).toHaveBeenLastCalledWith({ "0:0": { page: 0, index: 0, subtype: "Text", delete: true, text: null, color: null } });
    fireEvent.click(screen.getByRole("button", { name: "Edit comment of Note on page 1" }));
    fireEvent.change(screen.getByLabelText("Comment for Note on page 1"), { target: { value: "Fixed" } });
    fireEvent.click(screen.getByTestId("comment-apply"));
    expect(onChanges).toHaveBeenLastCalledWith({ "0:0": { page: 0, index: 0, subtype: "Text", delete: false, text: "Fixed", color: null } });
    // A note's colour can change; a text box (drawn text) offers no colour.
    fireEvent.change(screen.getByLabelText("Colour of Note on page 1"), { target: { value: "#00aa00" } });
    expect(onChanges).toHaveBeenLastCalledWith({ "0:0": { page: 0, index: 0, subtype: "Text", delete: false, text: null, color: "#00aa00" } });
    expect(screen.queryByLabelText("Colour of Text box on page 1")).toBeNull();
    const editBox = screen.getByRole("button", { name: "Edit comment of Text box on page 1" });
    expect(editBox.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(screen.getByText("Fix this"));
    expect(onSelect).toHaveBeenCalledWith(0);
  });

  it("Move hands the annotation to the page; while it moves, Done and Cancel finish it", async () => {
    const onMove = vi.fn();
    const onMoveDone = vi.fn();
    const onMoveCancel = vi.fn();
    const api = { listAnnotations: vi.fn(async () => ({ annotations: [note, box], truncated: false })), pageUrl: () => "" } as unknown as Api;
    const props = { doc, view: "comments" as const, onView: () => undefined, currentPage: 0, onSelectPage: () => undefined, onPageContextMenu: () => undefined, onSaveAttachment: () => undefined, onAttachFiles: () => undefined, onEditBookmarks: () => undefined, annotChanges: {}, onAnnotChanges: () => undefined, onMove, onMoveDone, onMoveCancel };
    const r = render(
      <ApiContext.Provider value={api}>
        <Sidebar {...props} moving={null} />
      </ApiContext.Provider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Move Note on page 1" }));
    expect(onMove).toHaveBeenCalledWith(note);
    r.rerender(
      <ApiContext.Provider value={api}>
        <Sidebar {...props} moving="0:0" />
      </ApiContext.Provider>,
    );
    expect(screen.queryByRole("button", { name: "Move Note on page 1" })).toBeNull();
    fireEvent.click(screen.getByTestId("move-done"));
    expect(onMoveDone).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onMoveCancel).toHaveBeenCalled();
    // The other annotation can still be moved.
    expect(screen.getByRole("button", { name: "Move Text box on page 1" })).toBeTruthy();
  });

  it("shows pending deletes and lets them be undone", async () => {
    const { onChanges } = setupComments({ "0:0": { page: 0, index: 0, subtype: "Text", delete: true, text: null } });
    await screen.findByText("Removed when you save a copy");
    fireEvent.click(screen.getByRole("button", { name: "Undo change to Note on page 1" }));
    expect(onChanges).toHaveBeenLastCalledWith({});
  });
});

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
      <Sidebar doc={doc} view="attachments" onView={() => undefined} currentPage={0} onSelectPage={() => undefined} onPageContextMenu={() => undefined} onSaveAttachment={onSave} onAttachFiles={onAttach} />
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

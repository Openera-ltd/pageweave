import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, FolderOpen, IndentDecrease, IndentIncrease, Plus, Trash2 } from "lucide-react";
import { useApi } from "../api";
import { bookmarksBlocker, canIndent, canMoveDown, canMoveUp, indent, insertAfter, moveDown, moveUp, outdent, remove, type EditableBookmark } from "../bookmarks";
import { siblingOutput } from "../paths";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  /** 0-based page for a new bookmark (the page in view). */
  currentPage: number;
  /** Start with a new bookmark for `currentPage` already added. */
  addOnOpen?: boolean;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

/** Add, rename, reorder, indent and delete bookmarks; saves a verified copy. */
export function EditBookmarksDialog({ doc, currentPage, addOnOpen, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const nextKey = useRef(0);
  const fresh = (page: number): EditableBookmark => ({ key: nextKey.current++, title: `Page ${page + 1}`, page, depth: 0 });
  const [list, setList] = useState<EditableBookmark[]>(() => {
    const start = doc.info.bookmarks.map((b) => ({ key: nextKey.current++, title: b.title, page: b.page, depth: b.depth }));
    return addOnOpen ? [...start, fresh(currentPage)] : start;
  });
  const [selected, setSelected] = useState<number | null>(addOnOpen ? list.length - 1 : list.length ? 0 : null);
  const [output, setOutput] = useState(siblingOutput(doc.path, "bookmarked"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);
  const pages = doc.info.page_count;

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      onFinished(job);
      onClose();
    } else if (job.state === "failed" && job.error) {
      setJobId(null);
      setError(job.error);
      setConflict(job.error.category === "output_conflict");
    } else if (job.state === "cancelled") {
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const truncated = (doc.info as { bookmarks_truncated?: boolean }).bookmarks_truncated;
  const blocker = doc.info.encrypted
    ? "Remove the password first, then edit the bookmarks of that copy."
    : truncated
      ? "This document has more bookmarks than can be edited here."
      : (bookmarksBlocker(list, pages) ?? (output.trim() ? null : "Choose where to save the copy."));

  const apply = (next: EditableBookmark[], select: number | null) => {
    setList(next);
    setSelected(select);
  };
  const keyOf = (i: number) => list[i]!.key;
  const indexOfKey = (l: EditableBookmark[], key: number) => l.findIndex((b) => b.key === key);

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy) return;
    setError(null);
    setConflict(false);
    try {
      const bookmarks = list.map((b) => ({ title: b.title.trim(), page: b.page as number, depth: b.depth }));
      setJobId(await api.startOperation({ schema_version: 1, operation: { op: "set_bookmarks", input: doc.path, bookmarks, output }, collision }, doc.doc_id));
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const sel = selected !== null && selected < list.length ? selected : null;
  const act = (label: string, icon: React.ReactNode, enabled: boolean, reason: string, run: () => void, testid: string) => (
    <button type="button" className="btn icon" aria-label={label} title={enabled ? label : reason} aria-disabled={enabled ? undefined : true} data-testid={testid} onClick={() => enabled && run()}>
      {icon}
    </button>
  );
  const none = "Select a bookmark first.";

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="bookmarks-save" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Saving…" : "Save copy"}
      </button>
    </>
  );

  return (
    <Dialog title="Edit bookmarks" onClose={onClose} footer={footer} busy={busy} wide>
      <p className="hint" style={{ marginTop: 0 }}>
        Saves a copy of <strong>{doc.name}</strong> with these bookmarks; each one opens its page. The original file is not changed.
      </p>
      <div className="row bookmark-tools" role="toolbar" aria-label="Bookmark actions">
        <button
          type="button"
          className="btn outline"
          data-testid="bookmark-add"
          onClick={() => {
            const next = insertAfter(list, sel, fresh(currentPage));
            apply(next, indexOfKey(next, nextKey.current - 1));
          }}
        >
          <Plus /> Add for page {currentPage + 1}
        </button>
        {act("Move up", <ArrowUp />, sel !== null && canMoveUp(list, sel), sel === null ? none : "Already first at its level.", () => {
          const k = keyOf(sel!);
          const next = moveUp(list, sel!);
          apply(next, indexOfKey(next, k));
        }, "bookmark-up")}
        {act("Move down", <ArrowDown />, sel !== null && canMoveDown(list, sel), sel === null ? none : "Already last at its level.", () => {
          const k = keyOf(sel!);
          const next = moveDown(list, sel!);
          apply(next, indexOfKey(next, k));
        }, "bookmark-down")}
        {act("Make it a sub-bookmark", <IndentIncrease />, sel !== null && canIndent(list, sel), sel === null ? none : "Needs a bookmark above it at the same level.", () => apply(indent(list, sel!), sel), "bookmark-indent")}
        {act("Move it up a level", <IndentDecrease />, sel !== null && list[sel]!.depth > 0, sel === null ? none : "Already at the top level.", () => apply(outdent(list, sel!), sel), "bookmark-outdent")}
        {act("Delete bookmark", <Trash2 />, sel !== null, none, () => {
          const next = remove(list, sel!);
          apply(next, next.length ? Math.min(sel!, next.length - 1) : null);
        }, "bookmark-delete")}
      </div>
      {list.length === 0 ? (
        <p className="hint">No bookmarks. Saving removes the document’s bookmarks.</p>
      ) : (
        <ol className="bookmark-edit" aria-label="Bookmarks">
          {list.map((b, i) => (
            <li key={b.key} aria-current={i === sel ? "true" : undefined} style={{ paddingLeft: `calc(${b.depth} * var(--space-5))` }} onFocusCapture={() => setSelected(i)} onClick={() => setSelected(i)}>
              <input
                className="input grow"
                aria-label={`Bookmark ${i + 1} title`}
                data-bookmark-title={i}
                value={b.title}
                onChange={(e) => setList((l) => l.map((x, k) => (k === i ? { ...x, title: e.target.value } : x)))}
              />
              <label className="row" style={{ gap: "var(--space-2)" }}>
                <span className="hint">Page</span>
                <input
                  className="input"
                  type="number"
                  min={1}
                  max={pages}
                  style={{ width: "7ch" }}
                  aria-label={`Bookmark ${i + 1} page`}
                  data-bookmark-page={i}
                  value={b.page === null ? "" : b.page + 1}
                  onChange={(e) => {
                    const v = e.target.value === "" ? null : Number(e.target.value) - 1;
                    setList((l) => l.map((x, k) => (k === i ? { ...x, page: v } : x)));
                  }}
                />
              </label>
            </li>
          ))}
        </ol>
      )}
      <div className="field">
        <label htmlFor="bookmarks-output">Save copy as</label>
        <div className="row">
          <input id="bookmarks-output" className="input grow" value={output} onChange={(e) => setOutput(e.target.value)} spellCheck={false} />
          <button
            type="button"
            className="btn outline"
            onClick={async () => {
              const p = await api.pickSavePath(output);
              if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
            }}
          >
            <FolderOpen /> Browse…
          </button>
        </div>
      </div>
      {blocker && list.length > 0 && (
        <span className="error" role="status">
          {blocker}
        </span>
      )}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            {!conflict && <div className="hint">{error.action}</div>}
          </div>
          {conflict && (
            <div className="row">
              <button type="button" className="btn outline" onClick={() => start("rename")}>
                Keep both
              </button>
              <button type="button" className="btn danger" onClick={() => start("replace")}>
                Replace
              </button>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}

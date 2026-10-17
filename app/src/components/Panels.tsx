import { useEffect, useMemo, useRef, useState } from "react";
import { CaseSensitive, ChevronDown, ChevronUp, FileText, FolderOpen, Trash2, WholeWord, X } from "lucide-react";
import { COMMANDS, type CommandHandlers, type CommandId } from "../commands";
import { basename, dirname } from "../paths";
import type { JobSnapshot, OpenedDoc, PwError } from "../types";
import { Dialog } from "./ui";

/* ---------------- Home ---------------- */

export function Home({
  recent,
  dropActive,
  onOpen,
  onOpenRecent,
  onRemoveRecent,
  openShortcut,
}: {
  recent: string[];
  dropActive: boolean;
  onOpen: () => void;
  onOpenRecent: (path: string) => void;
  onRemoveRecent: (path: string) => void;
  openShortcut?: string;
}) {
  return (
    <main className="home" aria-label="Start">
      <div className="panel">
        <h1>PageWeave</h1>
        <p>Open a PDF to view, search and reorganize its pages. Your files stay on this computer.</p>
        <div className={`drop ${dropActive ? "active" : ""}`} aria-hidden="true">
          {dropActive ? "Release to open" : "Drop PDF files here"}
        </div>
        <button type="button" className="btn primary" data-command="file.open" onClick={onOpen} title={openShortcut ? `Open PDF (${openShortcut})` : "Open PDF"}>
          <FolderOpen /> Open PDF…
        </button>
        {recent.length > 0 && (
          <>
            <h2 style={{ fontSize: "var(--font-size-2)", margin: "24px 0 8px" }}>Recent</h2>
            <ul className="recent" aria-label="Recent files">
              {recent.map((p) => (
                <li key={p}>
                  <button type="button" className="btn open" title={p} onClick={() => onOpenRecent(p)}>
                    <FileText />
                    <span>
                      {basename(p)} <span className="path">— {dirname(p)}</span>
                    </span>
                  </button>
                  <button type="button" className="btn icon" aria-label={`Remove ${basename(p)} from recent files`} title="Remove from recent files (the file is not deleted)" onClick={() => onRemoveRecent(p)}>
                    <Trash2 />
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </main>
  );
}

/* ---------------- Search bar ---------------- */

export interface SearchState {
  query: string;
  matchCase: boolean;
  wholeWord: boolean;
  status: "idle" | "searching" | "done" | "error";
  count: number;
  current: number;
  truncated: boolean;
  error?: string;
}

export function SearchBar({
  state,
  onChange,
  onSubmit,
  onNext,
  onPrevious,
  onClose,
  focusNonce,
}: {
  state: SearchState;
  onChange: (patch: Partial<SearchState>) => void;
  onSubmit: () => void;
  onNext: () => void;
  onPrevious: () => void;
  onClose: () => void;
  focusNonce: number;
}) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [focusNonce]);

  const label =
    state.status === "searching"
      ? "Searching…"
      : state.status === "error"
        ? state.error ?? "Search failed"
        : state.status === "done"
          ? state.count === 0
            ? "No matches"
            : `${state.current + 1} of ${state.count}${state.truncated ? "+" : ""}`
          : "";
  const noHits = state.status !== "done" || state.count === 0;

  return (
    <div
      className="searchbar"
      role="search"
      onKeyDown={(e) => {
        // Escape closes Find from any control in the bar, not only the text field.
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <input
        ref={input}
        className="input"
        type="search"
        aria-label="Find in document"
        placeholder="Find in document"
        value={state.query}
        onChange={(e) => onChange({ query: e.target.value, status: "idle" })}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            if (state.status === "done") (e.shiftKey ? onPrevious : onNext)();
            else onSubmit();
          }
        }}
      />
      <button type="button" className="btn icon" aria-label="Match case" title="Match case" aria-pressed={state.matchCase} onClick={() => onChange({ matchCase: !state.matchCase, status: "idle" })}>
        <CaseSensitive />
      </button>
      <button type="button" className="btn icon" aria-label="Whole words only" title="Whole words only" aria-pressed={state.wholeWord} onClick={() => onChange({ wholeWord: !state.wholeWord, status: "idle" })}>
        <WholeWord />
      </button>
      <button type="button" className="btn icon" data-command="search.previous" aria-label="Previous match" title={noHits ? "Previous match — no matches yet" : "Previous match (Shift+F3)"} aria-disabled={noHits || undefined} onClick={() => !noHits && onPrevious()}>
        <ChevronUp />
      </button>
      <button type="button" className="btn icon" data-command="search.next" aria-label="Next match" title={noHits ? "Next match — no matches yet" : "Next match (F3)"} aria-disabled={noHits || undefined} onClick={() => !noHits && onNext()}>
        <ChevronDown />
      </button>
      <span className="count" role="status" aria-live="polite">
        {label}
      </span>
      <button type="button" className="btn icon" data-command="search.close" aria-label="Close find" title="Close find (Esc)" onClick={onClose}>
        <X />
      </button>
    </div>
  );
}

/* ---------------- Status bar ---------------- */

export function StatusBar({
  doc,
  currentPage,
  zoom,
  jobs,
  onGoToPage,
  goToNonce,
  onCancelJob,
  unsaved = 0,
  onSaveAnnotations,
}: {
  doc: OpenedDoc | null;
  currentPage: number;
  zoom: number;
  jobs: JobSnapshot[];
  onGoToPage: (page: number) => void;
  goToNonce: number;
  onCancelJob: (id: string) => void;
  /** Unsaved annotations in the active document. */
  unsaved?: number;
  onSaveAnnotations?: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (goToNonce) {
      input.current?.focus();
      input.current?.select();
    }
  }, [goToNonce]);
  const running = jobs.filter((j) => j.state === "running" || j.state === "queued" || j.state === "cancelling");
  const commit = () => {
    if (!doc || draft === null) return;
    const n = Number(draft);
    if (Number.isInteger(n) && n >= 1 && n <= doc.info.page_count) onGoToPage(n - 1);
    setDraft(null);
  };
  return (
    <footer className="statusbar" aria-label="Status">
      {doc ? (
        <>
          <span>
            Page{" "}
            <input
              ref={input}
              className="input page-input"
              aria-label={`Current page, 1 to ${doc.info.page_count}. Type a number and press Enter to go there.`}
              value={draft ?? String(currentPage + 1)}
              onChange={(e) => setDraft(e.target.value.replace(/[^0-9]/g, ""))}
              onFocus={(e) => e.target.select()}
              onBlur={() => setDraft(null)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commit();
                if (e.key === "Escape") {
                  setDraft(null);
                  (e.target as HTMLInputElement).blur();
                }
              }}
            />{" "}
            of {doc.info.page_count}
          </span>
          <span>{Math.round(zoom * 100)}%</span>
          {doc.info.encrypted && <span title="This document is password protected">Protected</span>}
          <span aria-live="polite">
            {unsaved > 0 && (
              <button type="button" className="btn link" data-testid="unsaved" title="Save annotated copy (Ctrl+S)" onClick={() => onSaveAnnotations?.()}>
                {unsaved} unsaved annotation{unsaved === 1 ? "" : "s"}
              </button>
            )}
          </span>
        </>
      ) : (
        <span>No document open</span>
      )}
      <span className="spacer" />
      {running.length > 0 && (
        <span role="status" aria-live="polite" className="row">
          {running[0]!.state === "cancelling" ? "Cancelling" : "Working on"} {running[0]!.label.replace(/_/g, " ")}
          {running.length > 1 ? ` (+${running.length - 1} more)` : ""}
          <button type="button" className="btn" style={{ height: 20 }} onClick={() => onCancelJob(running[0]!.id)} aria-disabled={running[0]!.state === "cancelling" || undefined}>
            Cancel
          </button>
        </span>
      )}
    </footer>
  );
}

/* ---------------- Notices ---------------- */

export interface NoticeAction {
  label: string;
  run: () => void;
}

export interface Notice {
  id: number;
  kind: "info" | "success" | "warning" | "error";
  title: string;
  detail?: string;
  actions?: NoticeAction[];
  /** Errors and warnings persist until dismissed; routine success auto-dismisses. */
  persistent: boolean;
}

export function Notices({ notices, onDismiss }: { notices: Notice[]; onDismiss: (id: number) => void }) {
  return (
    <div className="notices" role="region" aria-label="Notifications" aria-live="polite">
      {notices.map((n) => (
        <NoticeView key={n.id} notice={n} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function NoticeView({ notice, onDismiss }: { notice: Notice; onDismiss: (id: number) => void }) {
  const [hover, setHover] = useState(false);
  useEffect(() => {
    if (notice.persistent || hover) return;
    const t = setTimeout(() => onDismiss(notice.id), 6000);
    return () => clearTimeout(t);
  }, [notice, hover, onDismiss]);
  return (
    <div
      className={`notice ${notice.kind}`}
      role={notice.kind === "error" ? "alert" : "status"}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setHover(true)}
      onBlur={() => setHover(false)}
    >
      <span aria-hidden="true" />
      <div>
        <div className="title">{notice.title}</div>
        {notice.detail && <div className="detail">{notice.detail}</div>}
        {notice.actions && notice.actions.length > 0 && (
          <div className="actions">
            {notice.actions.map((a) => (
              <button key={a.label} type="button" className="btn outline" data-action={a.label} onClick={() => { a.run(); onDismiss(notice.id); }}>
                {a.label}
              </button>
            ))}
          </div>
        )}
      </div>
      <button type="button" className="btn icon" aria-label="Dismiss notification" title="Dismiss" onClick={() => onDismiss(notice.id)}>
        <X />
      </button>
    </div>
  );
}

/* ---------------- Command palette ---------------- */

export function CommandPalette({ handlers, bindings, onClose }: { handlers: CommandHandlers; bindings: Record<string, string[]>; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const items = useMemo(
    () =>
      COMMANDS.filter((c) => c.id !== "app.commandPalette")
        .filter((c) => `${c.category} ${c.label}`.toLowerCase().includes(q.toLowerCase()))
        .map((c) => ({ c, avail: handlers[c.id].available() })),
    [q, handlers],
  );
  const run = (id: CommandId) => {
    const h = handlers[id];
    if (h.available() !== true) return;
    onClose();
    // Let the palette unmount (and restore focus) before the command opens its own UI.
    setTimeout(() => h.run(), 0);
  };
  return (
    <Dialog title="Commands" onClose={onClose}>
      <input
        className="input"
        style={{ width: "100%" }}
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-activedescendant={items[sel] ? `cmd-${items[sel]!.c.id}` : undefined}
        aria-label="Type a command"
        placeholder="Type a command"
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          setSel(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") setSel((s) => Math.min(items.length - 1, s + 1));
          else if (e.key === "ArrowUp") setSel((s) => Math.max(0, s - 1));
          else if (e.key === "Enter" && items[sel]) run(items[sel]!.c.id);
          else return;
          e.preventDefault();
        }}
      />
      <ul id="palette-list" className="palette-list" role="listbox" aria-label="Commands">
        {items.map(({ c, avail }, i) => (
          <li
            key={c.id}
            id={`cmd-${c.id}`}
            data-command={c.id}
            role="option"
            aria-selected={i === sel}
            aria-disabled={avail !== true || undefined}
            onMouseEnter={() => setSel(i)}
            onClick={() => run(c.id)}
          >
            <span className="grow">
              {c.category}: {c.label}
              {avail !== true && <div className="reason">{avail}</div>}
            </span>
            {bindings[c.id]?.[0] && <kbd>{bindings[c.id]![0]}</kbd>}
          </li>
        ))}
        {items.length === 0 && <li className="hint">No matching commands</li>}
      </ul>
    </Dialog>
  );
}

/* ---------------- Password ---------------- */

export function PasswordDialog({ name, error, onSubmit, onCancel }: { name: string; error?: PwError | null; onSubmit: (pw: string) => void; onCancel: () => void }) {
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => setBusy(false), [error]);
  return (
    <Dialog
      title="Password required"
      onClose={onCancel}
      footer={
        <>
          <button type="button" className="btn outline" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn primary" aria-disabled={!pw || busy || undefined} onClick={() => { if (pw && !busy) { setBusy(true); onSubmit(pw); } }}>
            {busy ? "Opening…" : "Open"}
          </button>
        </>
      }
    >
      <form onSubmit={(e) => { e.preventDefault(); if (pw && !busy) { setBusy(true); onSubmit(pw); } }}>
        <div className="field">
          <label htmlFor="pdf-password">Password for “{name}”</label>
          <input id="pdf-password" className="input" type="password" autoComplete="off" value={pw} aria-invalid={!!error} aria-describedby="pw-err" onChange={(e) => setPw(e.target.value)} />
          <span id="pw-err" className="error" role={error ? "alert" : undefined}>
            {error ? "That password did not work. Check it and try again." : ""}
          </span>
        </div>
        <p className="hint" style={{ margin: 0 }}>The password is kept only while the document is open and is never saved.</p>
      </form>
    </Dialog>
  );
}

/* ---------------- Properties ---------------- */

export function PropertiesDialog({ doc, onClose }: { doc: OpenedDoc; onClose: () => void }) {
  const i = doc.info;
  const first = i.pages[0];
  const rows: Array<[string, string]> = [
    ["File", doc.path],
    ["Size", formatBytes(i.file_bytes)],
    ["Pages", String(i.page_count)],
    ["PDF version", i.pdf_version],
    ["Title", i.title ?? "—"],
    ["Author", i.author ?? "—"],
    ["First page size", first ? `${(first.width_pt / 72).toFixed(2)} × ${(first.height_pt / 72).toFixed(2)} in` : "—"],
    ["Password protected", i.encrypted ? "Yes" : "No"],
    ["Bookmarks", String(i.bookmarks.length)],
    ["SHA-256", i.sha256],
  ];
  return (
    <Dialog title="Document properties" onClose={onClose} wide footer={<button type="button" className="btn primary" onClick={onClose}>Close</button>}>
      <table className="shortcut-table">
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}>
              <th scope="row">{k}</th>
              <td style={{ wordBreak: "break-all" }}>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {i.warnings.length > 0 && (
        <div className="banner" role="note" style={{ marginTop: 12, borderRadius: 6 }}>
          {i.warnings.join(" ")}
        </div>
      )}
    </Dialog>
  );
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

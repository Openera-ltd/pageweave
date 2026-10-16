import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Command as CommandIcon,
  FolderOpen,
  Layers,
  PanelLeft,
  Search,
  Settings as SettingsIcon,
  X,
  ZoomIn,
  ZoomOut,
  MoveHorizontal,
} from "lucide-react";
import { useApi } from "./api";
import { COMMAND_BY_ID, COMMANDS, commandForShortcut, effectiveBindings, type CommandHandlers, type CommandId } from "./commands";
import { clampZoom, fitWidthZoom, nextZoom, ZOOM_PRESETS } from "./geometry";
import { basename } from "./paths";
import { eventToShortcut, isTextEntry } from "./shortcuts";
import { applyAppearance } from "./theme";
import { toPwError, type AppInfo, type JobSnapshot, type OpenedDoc, type PwError, type SearchHit, type Settings } from "./types";
import { CommandButton, Menu, type MenuItemOrSeparator } from "./components/ui";
import { Viewer, type ScrollRequest } from "./components/Viewer";
import { Thumbnails } from "./components/Thumbnails";
import { PageToolsDialog, type PageTool } from "./components/PageToolsDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { CommandPalette, Home, Notices, PasswordDialog, PropertiesDialog, SearchBar, StatusBar, type Notice, type NoticeAction, type SearchState } from "./components/Panels";

interface ViewState {
  zoom: number;
  page: number;
  /** True until the viewer has measured itself and fitted the first zoom. */
  fitPending?: boolean;
}

type DialogState =
  | { kind: "tool"; tool: PageTool; initialPages?: string }
  | { kind: "settings"; tab: "appearance" | "shortcuts"; original: Settings }
  | { kind: "palette" }
  | { kind: "properties" }
  | { kind: "password"; path: string; error: PwError | null }
  | null;

interface MenuState {
  items: MenuItemOrSeparator[];
  x: number;
  y: number;
  label: string;
}

const EMPTY_SEARCH: SearchState = { query: "", matchCase: false, wholeWord: false, status: "idle", count: 0, current: 0, truncated: false };

export function App() {
  const api = useApi();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [preview, setPreview] = useState<Settings | null>(null);
  const [tabs, setTabs] = useState<OpenedDoc[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [views, setViews] = useState<Record<number, ViewState>>({});
  const [showThumbs, setShowThumbs] = useState(true);
  const [dialog, setDialog] = useState<DialogState>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [notices, setNotices] = useState<Notice[]>([]);
  const [jobs, setJobs] = useState<Record<string, JobSnapshot>>({});
  const [dropActive, setDropActive] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [search, setSearch] = useState<SearchState>(EMPTY_SEARCH);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searchFocus, setSearchFocus] = useState(0);
  const [scrollRequest, setScrollRequest] = useState<ScrollRequest | null>(null);
  const [goToNonce, setGoToNonce] = useState(0);
  const noticeId = useRef(1);
  const searchToken = useRef(0);
  const viewerHost = useRef<HTMLDivElement>(null);

  const active = tabs.find((t) => t.doc_id === activeId) ?? null;
  const view: ViewState = (active && views[active.doc_id]) || { zoom: 1, page: 0 };
  const effective = preview ?? settings;
  const bindings = useMemo(() => effectiveBindings(effective?.shortcuts ?? {}), [effective?.shortcuts]);

  const notify = useCallback((n: Omit<Notice, "id">) => {
    const id = noticeId.current++;
    setNotices((ns) => [...ns.filter((x) => !(x.title === n.title && x.detail === n.detail)), { ...n, id }].slice(-5));
    return id;
  }, []);
  const dismiss = useCallback((id: number) => setNotices((ns) => ns.filter((n) => n.id !== id)), []);

  const notifyError = useCallback(
    (title: string, e: unknown, actions?: NoticeAction[]) => {
      const err = toPwError(e);
      notify({ kind: "error", title, detail: `${err.message} ${err.action}`.trim(), actions, persistent: true });
    },
    [notify],
  );

  /* ---------- Startup ---------- */
  useEffect(() => {
    api.appInfo().then(setInfo).catch((e) => notifyError("PageWeave could not start correctly", e));
    api.getSettings().then(setSettings).catch((e) => notifyError("Settings could not be loaded", e));
  }, [api, notifyError]);

  useEffect(() => {
    if (!info) return;
    const st = info.settings_status.status;
    if (st === "recovered_defaults") notify({ kind: "warning", title: "Settings were reset", detail: "The settings file was damaged, so defaults are in use. A copy of the damaged file was kept.", persistent: true });
    if (st === "repaired") notify({ kind: "warning", title: "Some settings were reset", detail: "Invalid values were replaced with defaults.", persistent: true });
    if (!info.engine_ready && info.engine_error) notifyError("The PDF engine is not available", info.engine_error);
  }, [info, notify, notifyError]);

  useEffect(() => {
    if (!effective) return;
    return applyAppearance(effective);
  }, [effective]);

  useEffect(() => {
    let off: (() => void) | undefined;
    api.onJobUpdate((j) => setJobs((js) => ({ ...js, [j.id]: j }))).then((u) => (off = u));
    return () => off?.();
  }, [api]);

  /* ---------- Documents ---------- */
  const openPath = useCallback(
    async (path: string, password?: string) => {
      try {
        const doc = await api.openDocument(path, password);
        setDialog((d) => (d?.kind === "password" ? null : d));
        if (!doc.already_open) {
          setTabs((ts) => [...ts, doc]);
          setViews((v) => ({ ...v, [doc.doc_id]: { zoom: 1, page: 0, fitPending: true } }));
          for (const w of doc.info.warnings) notify({ kind: "warning", title: doc.name, detail: w, persistent: true });
        }
        setActiveId(doc.doc_id);
        api.getSettings().then(setSettings).catch(() => undefined);
      } catch (e) {
        const err = toPwError(e);
        if (err.category === "password_required") {
          setDialog({ kind: "password", path, error: password ? err : null });
          return;
        }
        const actions: NoticeAction[] = [];
        if (err.category === "not_found" && settings?.recent_files.includes(path)) {
          actions.push({ label: "Remove from recent", run: () => removeRecent(path) });
        }
        notifyError(`Couldn’t open ${basename(path)}`, err, actions);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [api, notify, notifyError, settings],
  );

  const removeRecent = async (path: string) => {
    if (!settings) return;
    try {
      setSettings(await api.saveSettings({ ...settings, recent_files: settings.recent_files.filter((p) => p !== path) }));
    } catch (e) {
      notifyError("Couldn’t update recent files", e);
    }
  };

  const openDialog = useCallback(async () => {
    try {
      const files = await api.pickPdfFiles(true);
      for (const f of files) await openPath(f);
    } catch (e) {
      notifyError("The file picker could not be shown", e);
    }
  }, [api, openPath, notifyError]);

  const closeTab = useCallback(
    (id: number) => {
      api.closeDocument(id).catch(() => undefined);
      setTabs((ts) => {
        const i = ts.findIndex((t) => t.doc_id === id);
        const next = ts.filter((t) => t.doc_id !== id);
        if (id === activeId) setActiveId(next[Math.min(i, next.length - 1)]?.doc_id ?? null);
        return next;
      });
      setViews(({ [id]: _gone, ...rest }) => rest);
      if (id === activeId) {
        setSearchOpen(false);
        setHits([]);
        setSearch(EMPTY_SEARCH);
      }
    },
    [api, activeId],
  );

  useEffect(() => {
    api.launchFiles().then((files) => files.forEach((f) => openPath(f))).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    api
      .onFileDrop(async (e) => {
        if (e.type === "enter" || e.type === "over") setDropActive(true);
        else if (e.type === "leave") setDropActive(false);
        else if (e.type === "drop") {
          setDropActive(false);
          for (const p of e.paths) await openPath(p);
        }
      })
      .then((u) => (off = u))
      .catch(() => undefined);
    return () => off?.();
  }, [api, openPath]);

  /* ---------- View ---------- */
  const setView = useCallback(
    (patch: Partial<ViewState>) => {
      if (!active) return;
      setViews((v) => ({ ...v, [active.doc_id]: { ...(v[active.doc_id] ?? { zoom: 1, page: 0 }), ...patch } }));
    },
    [active],
  );
  const zoomStep = useCallback((d: 1 | -1) => setView({ zoom: nextZoom(view.zoom, d) }), [setView, view.zoom]);
  const goToPage = useCallback(
    (page: number) => {
      if (!active) return;
      const p = Math.max(0, Math.min(active.info.page_count - 1, page));
      setScrollRequest({ page: p, nonce: Date.now() });
    },
    [active],
  );

  /* ---------- Search ---------- */
  const runSearch = useCallback(async () => {
    if (!active || !search.query.trim()) return;
    const token = ++searchToken.current;
    setSearch((s) => ({ ...s, status: "searching" }));
    try {
      const r = await api.search(active.doc_id, search.query, search.matchCase, search.wholeWord);
      if (token !== searchToken.current) return; // stale result
      setHits(r.hits);
      const start = Math.max(0, r.hits.findIndex((h) => h.page >= view.page));
      setSearch((s) => ({ ...s, status: "done", count: r.hits.length, current: r.hits.length ? start : 0, truncated: r.truncated }));
      if (r.hits.length) setScrollRequest({ page: r.hits[start]!.page, nonce: Date.now(), hit: r.hits[start] });
    } catch (e) {
      if (token !== searchToken.current) return;
      setHits([]);
      setSearch((s) => ({ ...s, status: "error", error: toPwError(e).message }));
    }
  }, [api, active, search.query, search.matchCase, search.wholeWord, view.page]);

  const stepHit = useCallback(
    (d: 1 | -1) => {
      if (search.status !== "done" || hits.length === 0) {
        runSearch();
        return;
      }
      const next = (search.current + d + hits.length) % hits.length;
      setSearch((s) => ({ ...s, current: next }));
      setScrollRequest({ page: hits[next]!.page, nonce: Date.now(), hit: hits[next] });
    },
    [hits, search.current, search.status, runSearch],
  );

  const closeSearch = useCallback(() => {
    searchToken.current++;
    setSearchOpen(false);
    setHits([]);
    setSearch((s) => ({ ...s, status: "idle", count: 0, current: 0 }));
    document.querySelector<HTMLElement>('[data-testid="viewer"]')?.focus();
  }, []);

  /* ---------- Commands ---------- */
  const NO_DOC = "Open a PDF first.";
  const needDoc = (): true | string => (active ? true : NO_DOC);
  // The command palette itself does not block commands launched from it.
  const noDialog = (): true | string => (dialog && dialog.kind !== "palette" ? "Close the open dialog first." : true);
  const all = (...checks: Array<() => true | string>) => () => {
    for (const c of checks) {
      const r = c();
      if (r !== true) return r;
    }
    return true as const;
  };

  const openTool = (tool: PageTool, initialPages?: string) => setDialog({ kind: "tool", tool, initialPages });

  const handlers: CommandHandlers = {
    "file.open": { run: openDialog, available: () => true },
    "file.close": { run: () => active && closeTab(active.doc_id), available: needDoc },
    "file.reveal": { run: () => active && api.revealInFolder(active.path).catch((e) => notifyError("Couldn’t show the file", e)), available: needDoc },
    "file.properties": { run: () => setDialog({ kind: "properties" }), available: all(needDoc, noDialog) },
    "tab.next": {
      run: () => {
        const i = tabs.findIndex((t) => t.doc_id === activeId);
        setActiveId(tabs[(i + 1) % tabs.length]!.doc_id);
      },
      available: () => (tabs.length > 1 ? true : "Only one document is open."),
    },
    "tab.previous": {
      run: () => {
        const i = tabs.findIndex((t) => t.doc_id === activeId);
        setActiveId(tabs[(i - 1 + tabs.length) % tabs.length]!.doc_id);
      },
      available: () => (tabs.length > 1 ? true : "Only one document is open."),
    },
    "view.zoomIn": { run: () => zoomStep(1), available: all(needDoc, () => (view.zoom < 6 ? true : "Already at maximum zoom.")) },
    "view.zoomOut": { run: () => zoomStep(-1), available: all(needDoc, () => (view.zoom > 0.1 ? true : "Already at minimum zoom.")) },
    "view.actualSize": { run: () => setView({ zoom: 1 }), available: needDoc },
    "view.fitWidth": {
      run: () => {
        const w = document.querySelector<HTMLElement>('[data-testid="viewer"]')?.clientWidth ?? 800;
        if (active) setView({ zoom: fitWidthZoom(active.info.pages, w) });
      },
      available: needDoc,
    },
    "view.toggleThumbnails": { run: () => setShowThumbs((s) => !s), available: needDoc },
    "nav.nextPage": { run: () => goToPage(view.page + 1), available: all(needDoc, () => (active && view.page < active.info.page_count - 1 ? true : "Already on the last page.")) },
    "nav.previousPage": { run: () => goToPage(view.page - 1), available: all(needDoc, () => (view.page > 0 ? true : "Already on the first page.")) },
    "nav.firstPage": { run: () => goToPage(0), available: needDoc },
    "nav.lastPage": { run: () => active && goToPage(active.info.page_count - 1), available: needDoc },
    "nav.goToPage": { run: () => setGoToNonce((n) => n + 1), available: needDoc },
    "search.open": {
      run: () => {
        setSearchOpen(true);
        setSearchFocus((n) => n + 1);
      },
      available: needDoc,
    },
    "search.next": { run: () => (searchOpen ? stepHit(1) : (setSearchOpen(true), setSearchFocus((n) => n + 1))), available: needDoc },
    "search.previous": { run: () => (searchOpen ? stepHit(-1) : (setSearchOpen(true), setSearchFocus((n) => n + 1))), available: needDoc },
    "search.close": { run: closeSearch, available: () => (searchOpen ? true : "Find is not open.") },
    "pages.extract": { run: () => openTool("extract"), available: all(needDoc, noDialog) },
    "pages.delete": { run: () => openTool("delete"), available: all(needDoc, noDialog, () => (active && active.info.page_count > 1 ? true : "A one-page document has no pages to delete.")) },
    "pages.rotate": { run: () => openTool("rotate"), available: all(needDoc, noDialog) },
    "pages.split": { run: () => openTool("split"), available: all(needDoc, noDialog, () => (active && active.info.page_count > 1 ? true : "A one-page document cannot be split.")) },
    "pages.merge": { run: () => openTool("merge"), available: all(needDoc, noDialog) },
    "app.settings": { run: () => settings && setDialog({ kind: "settings", tab: "appearance", original: settings }), available: () => (settings ? noDialog() : "Settings are still loading.") },
    "app.shortcuts": { run: () => settings && setDialog({ kind: "settings", tab: "shortcuts", original: settings }), available: () => (settings ? noDialog() : "Settings are still loading.") },
    "app.commandPalette": { run: () => setDialog({ kind: "palette" }), available: noDialog },
    "app.openLogs": {
      run: async () => {
        try {
          await api.revealInFolder(await api.logDir());
        } catch (e) {
          notifyError("Couldn’t open the log folder", e);
        }
      },
      available: () => true,
    },
  };

  // Every registry command must have a handler (also enforced by tests).
  for (const c of COMMANDS) if (!handlers[c.id]) throw new Error(`missing handler for ${c.id}`);

  const runCommand = (id: CommandId) => {
    const h = handlers[id];
    if (h.available() === true) h.run();
  };

  /* ---------- Keyboard ---------- */
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || menu) return;
      const sc = eventToShortcut(e);
      if (!sc) return;
      const id = commandForShortcut(sc, bindings);
      if (!id) return;
      if (dialog) return; // modal dialogs own the keyboard
      if (isTextEntry(document.activeElement) && !COMMAND_BY_ID.get(id)?.allowInInput) return;
      e.preventDefault();
      const h = handlersRef.current[id];
      if (h.available() === true) h.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [bindings, dialog, menu]);

  /* ---------- Menus ---------- */
  const item = (id: CommandId, label?: string, run?: () => void, available?: true | string): MenuItemOrSeparator => ({
    id,
    label: label ?? COMMAND_BY_ID.get(id)!.label,
    shortcut: label ? undefined : bindings[id]?.[0],
    available: available ?? handlers[id].available(),
    run: run ?? (() => runCommand(id)),
  });

  const pageMenu = (page: number, x: number, y: number) => {
    const n = page + 1;
    const multi = (active?.info.page_count ?? 0) > 1;
    setMenu({
      label: `Page ${n}`,
      x,
      y,
      items: [
        item("pages.extract", `Extract page ${n}…`, () => openTool("extract", String(n))),
        item("pages.rotate", `Rotate page ${n}…`, () => openTool("rotate", String(n))),
        item("pages.delete", `Delete page ${n}…`, () => openTool("delete", String(n)), multi ? true : "A one-page document has no pages to delete."),
        "separator",
        item("view.zoomIn"),
        item("view.zoomOut"),
        item("view.fitWidth"),
        "separator",
        item("file.properties"),
      ],
    });
  };

  const tabMenu = (doc: OpenedDoc, x: number, y: number) =>
    setMenu({
      label: doc.name,
      x,
      y,
      items: [
        item("file.close", "Close", () => closeTab(doc.doc_id), true),
        item("file.close", "Close other tabs", () => tabs.filter((t) => t.doc_id !== doc.doc_id).forEach((t) => closeTab(t.doc_id)), tabs.length > 1 ? true : "No other tabs are open."),
        "separator",
        item("file.reveal", "Show in folder", () => api.revealInFolder(doc.path).catch((e) => notifyError("Couldn’t show the file", e)), true),
      ],
    });

  const pagesMenu = (e: React.MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ label: "Page tools", x: r.left, y: r.bottom + 2, items: [item("pages.extract"), item("pages.delete"), item("pages.rotate"), item("pages.split"), "separator", item("pages.merge")] });
  };

  /* ---------- Job results ---------- */
  const onJobFinished = (job: JobSnapshot) => {
    const r = job.result;
    if (!r) return;
    const out = r.outputs[0];
    const title = r.outputs.length > 1 ? `Saved ${r.outputs.length} files` : out ? `Saved ${basename(out.path)}` : "Done";
    const actions: NoticeAction[] = [];
    if (out && r.outputs.length === 1) actions.push({ label: "Open", run: () => openPath(out.path) });
    if (out) actions.push({ label: "Show in folder", run: () => api.revealInFolder(out.path).catch((e) => notifyError("Couldn’t show the file", e)) });
    notify({ kind: "success", title, detail: out?.pages != null && r.outputs.length === 1 ? `${out.pages} pages` : undefined, actions, persistent: false });
    for (const w of r.warnings) notify({ kind: "warning", title: "Please note", detail: w, persistent: true });
  };

  /* ---------- Render ---------- */
  const zoomPct = Math.round(view.zoom * 100);
  const zoomOptions = Array.from(new Set([...ZOOM_PRESETS.map((z) => Math.round(z * 100)), zoomPct])).sort((a, b) => a - b);

  return (
    <div className="app">
      <header className="toolbar" role="toolbar" aria-label="Main toolbar">
        <div className="group">
          <CommandButton command="file.open" handlers={handlers} bindings={bindings} icon={<FolderOpen />} showLabel label="Open" />
        </div>
        <div className="divider" aria-hidden="true" />
        <div className="group">
          <CommandButton command="view.toggleThumbnails" handlers={handlers} bindings={bindings} icon={<PanelLeft />} pressed={!!active && showThumbs} />
          <CommandButton command="view.zoomOut" handlers={handlers} bindings={bindings} icon={<ZoomOut />} />
          <select
            className="select"
            aria-label="Zoom level"
            title={active ? "Zoom level" : NO_DOC}
            disabled={!active}
            value={zoomPct}
            onChange={(e) => setView({ zoom: clampZoom(Number(e.target.value) / 100) })}
          >
            {zoomOptions.map((z) => (
              <option key={z} value={z}>
                {z}%
              </option>
            ))}
          </select>
          <CommandButton command="view.zoomIn" handlers={handlers} bindings={bindings} icon={<ZoomIn />} />
          <CommandButton command="view.fitWidth" handlers={handlers} bindings={bindings} icon={<MoveHorizontal />} />
        </div>
        <div className="divider" aria-hidden="true" />
        <div className="group">
          <button
            type="button"
            className="btn"
            aria-haspopup="menu"
            aria-expanded={menu?.label === "Page tools"}
            aria-disabled={!active || undefined}
            title={active ? "Page tools" : `Page tools — ${NO_DOC}`}
            onClick={(e) => active && pagesMenu(e)}
          >
            <Layers /> Pages
          </button>
          <CommandButton command="search.open" handlers={handlers} bindings={bindings} icon={<Search />} pressed={searchOpen} />
        </div>
        <span className="spacer" />
        <div className="group">
          <CommandButton command="app.commandPalette" handlers={handlers} bindings={bindings} icon={<CommandIcon />} />
          <CommandButton command="app.settings" handlers={handlers} bindings={bindings} icon={<SettingsIcon />} />
        </div>
      </header>

      {tabs.length > 0 ? (
        <div className="tabbar" role="tablist" aria-label="Open documents">
          {tabs.map((t) => (
            <div
              key={t.doc_id}
              className="tab"
              role="tab"
              tabIndex={t.doc_id === activeId ? 0 : -1}
              aria-selected={t.doc_id === activeId}
              title={t.path}
              onClick={() => setActiveId(t.doc_id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") setActiveId(t.doc_id);
                if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
                  e.preventDefault();
                  const r = e.currentTarget.getBoundingClientRect();
                  tabMenu(t, r.left, r.bottom);
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                tabMenu(t, e.clientX, e.clientY);
              }}
              onAuxClick={(e) => e.preventDefault()}
            >
              <span className="name">{t.name}</span>
              <button
                type="button"
                className="btn icon close"
                aria-label={`Close ${t.name}`}
                title={`Close (${bindings["file.close"]?.[0] ?? ""})`}
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.doc_id);
                }}
              >
                <X />
              </button>
            </div>
          ))}
        </div>
      ) : (
        <div />
      )}

      {active ? (
        <div className="workspace" style={showThumbs ? undefined : { gridTemplateColumns: "1fr" }}>
          {showThumbs && <Thumbnails doc={active} currentPage={view.page} onSelect={goToPage} onContextMenu={pageMenu} />}
          <div ref={viewerHost} style={{ display: "grid", gridTemplateRows: "auto 1fr", minHeight: 0, minWidth: 0 }}>
            {searchOpen ? (
              <SearchBar
                state={search}
                focusNonce={searchFocus}
                onChange={(p) => {
                  if (p.query !== undefined || p.matchCase !== undefined || p.wholeWord !== undefined) {
                    searchToken.current++;
                    setHits([]);
                  }
                  setSearch((s) => ({ ...s, ...p }));
                }}
                onSubmit={runSearch}
                onNext={() => stepHit(1)}
                onPrevious={() => stepHit(-1)}
                onClose={closeSearch}
              />
            ) : (
              <div />
            )}
            <Viewer
              key={active.doc_id}
              doc={active}
              zoom={view.zoom}
              onZoomStep={zoomStep}
              onCurrentPage={(page) => setView({ page })}
              scrollRequest={scrollRequest}
              hits={hits}
              currentHit={search.current}
              onPageContextMenu={pageMenu}
              onMeasured={(width) => {
                if (view.fitPending) setView({ zoom: Math.min(1.5, fitWidthZoom(active.info.pages, width)), fitPending: false });
              }}
            />
          </div>
        </div>
      ) : (
        <div ref={viewerHost} style={{ display: "grid", minHeight: 0 }}>
          <Home
            recent={settings?.recent_files ?? []}
            dropActive={dropActive}
            onOpen={openDialog}
            onOpenRecent={(p) => openPath(p)}
            onRemoveRecent={removeRecent}
            openShortcut={bindings["file.open"]?.[0]}
          />
        </div>
      )}

      <StatusBar doc={active} currentPage={view.page} zoom={view.zoom} jobs={Object.values(jobs)} onGoToPage={goToPage} goToNonce={goToNonce} onCancelJob={(id) => api.cancelJob(id)} />

      {menu && <Menu {...menu} onClose={() => setMenu(null)} />}

      {dialog?.kind === "tool" && active && (
        <PageToolsDialog tool={dialog.tool} doc={active} initialPages={dialog.initialPages} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />
      )}
      {dialog?.kind === "settings" && (
        <SettingsDialog
          initial={dialog.original}
          initialTab={dialog.tab}
          onPreview={setPreview}
          onCancel={() => {
            setPreview(null);
            setDialog(null);
          }}
          onSave={async (s) => {
            const saved = await api.saveSettings(s);
            setSettings(saved);
            setPreview(null);
            setDialog(null);
            notify({ kind: "success", title: "Settings saved", persistent: false });
          }}
        />
      )}
      {dialog?.kind === "palette" && <CommandPalette handlers={handlers} bindings={bindings} onClose={() => setDialog(null)} />}
      {dialog?.kind === "properties" && active && <PropertiesDialog doc={active} onClose={() => setDialog(null)} />}
      {dialog?.kind === "password" && (
        <PasswordDialog name={basename(dialog.path)} error={dialog.error} onSubmit={(pw) => openPath(dialog.path, pw)} onCancel={() => setDialog(null)} />
      )}

      <Notices notices={notices} onDismiss={dismiss} />
    </div>
  );
}

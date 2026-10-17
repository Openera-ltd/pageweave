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
import {
  addAnnotations,
  ANNOTATION_COLORS,
  annotationSpecs,
  clientRectsToPageRects,
  EMPTY_HISTORY,
  isRedactMark,
  itemsAt,
  KIND_LABEL,
  makeId,
  noteRect,
  pageBoxes,
  REDACT_MARK_COLOR,
  redactAreas,
  redo,
  removeItem,
  selectionClientRects,
  undo,
  type AnnotationHistory,
  type PendingAnnotation,
  type PendingKind,
} from "./annotations";
import { missingRequired, setEdit, toFieldValues, fieldLabel, type FormEdits } from "./forms";
import { COMMAND_BY_ID, COMMANDS, commandForShortcut, effectiveBindings, type CommandHandlers, type CommandId } from "./commands";
import { clampZoom, fitWidthZoom, nextZoom, pageOrigin, screenRectToPdf, ZOOM_PRESETS, CSS_PX_PER_PT, displaySizePt } from "./geometry";
import { basename, siblingOutput } from "./paths";
import { sizeChange } from "./format";
import { signatureSummary } from "./signatures";
import { eventToShortcut, isTextEntry } from "./shortcuts";
import { applyAppearance } from "./theme";
import { toPwError, type AppInfo, type AttachmentInfo, type CollisionPolicy, type FormField, type FormInfo, type FormValue, type JobSnapshot, type OpenedDoc, type Operation, type PwError, type RecoveredJob, type RedactArea, type SearchHit, type SearchRect, type Settings, type SignatureReport } from "./types";
import { CommandButton, Menu, type MenuItemOrSeparator } from "./components/ui";
import { Viewer, type ScrollRequest } from "./components/Viewer";
import { Sidebar, type SidebarView } from "./components/Sidebar";
import { PageToolsDialog, type PageTool } from "./components/PageToolsDialog";
import { OrganizeDialog } from "./components/OrganizeDialog";
import { RedactDialog } from "./components/RedactDialog";
import { ProtectDialog } from "./components/ProtectDialog";
import { SanitizeDialog } from "./components/SanitizeDialog";
import { CompareView } from "./components/CompareView";
import { PrintDialog } from "./components/PrintDialog";
import { UnprotectDialog } from "./components/UnprotectDialog";
import { EditPropertiesDialog } from "./components/EditPropertiesDialog";
import { SaveAttachmentDialog } from "./components/SaveAttachmentDialog";
import { EditBookmarksDialog } from "./components/EditBookmarksDialog";
import { ImagesToPdfDialog, isImagePath } from "./components/ImagesToPdfDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { NoteDialog, UnsavedDialog } from "./components/AnnotationDialogs";
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
  | { kind: "note"; page: number; rect: SearchRect }
  | { kind: "organize" }
  | { kind: "redact"; areas: RedactArea[] }
  | { kind: "protect" }
  | { kind: "sanitize" }
  | { kind: "compare" }
  | { kind: "print" }
  | { kind: "unprotect" }
  | { kind: "editProperties" }
  | { kind: "saveAttachment"; attachment: AttachmentInfo }
  | { kind: "bookmarks"; addOnOpen: boolean; page: number }
  | { kind: "images"; files: string[] }
  | { kind: "unsaved"; reason: "tab"; docId: number }
  | { kind: "unsaved"; reason: "quit" }
  | null;

/** What to do once an annotated copy has been saved from the unsaved-changes prompt. */
type AfterSave = { kind: "close"; docId: number } | { kind: "quit" } | null;

const NO_PENDING: PendingAnnotation[] = [];
const NO_EDITS: FormEdits = {};

const OPERATION_TITLES: Partial<Record<Operation["op"], string>> = {
  extract_pages: "Extract pages",
  delete_pages: "Delete pages",
  rotate_pages: "Rotate pages",
  split: "Split document",
  merge: "Combine files",
  annotate: "Save annotated copy",
  redact: "Apply redactions",
  ocr_pages: "Recognize text",
  protect: "Protect with password",
  sanitize: "Remove hidden information",
  unprotect: "Remove password",
  set_properties: "Edit document properties",
  extract_attachment: "Save attached file",
  set_bookmarks: "Edit bookmarks",
  images_to_pdf: "Create PDF from images",
  add_attachments: "Attach files",
  watermark: "Add watermark",
  crop_pages: "Crop pages",
};

function operationTarget(op: Operation): string {
  if ("output" in op) return basename(op.output);
  if ("output_dir" in op) return basename(op.output_dir);
  return "";
}
/** Mirrors pw-core MAX_ANNOTATIONS_PER_REQUEST. */
const MAX_PENDING = 10_000;

interface MenuState {
  items: MenuItemOrSeparator[];
  x: number;
  y: number;
  label: string;
  /** Called after the menu closes (e.g. to restore a text selection the menu's focus cleared). */
  afterClose?: () => void;
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
  const [sidebarView, setSidebarView] = useState<SidebarView>("pages");
  /** Signature check of the active document (undefined while it runs). */
  const [sigs, setSigs] = useState<{ docId: number; report?: SignatureReport; error?: PwError } | null>(null);
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
  const [annots, setAnnots] = useState<Record<number, AnnotationHistory>>({});
  /** Area-marking mode: dragging on a page marks a rectangle for redaction. */
  const [markMode, setMarkMode] = useState(false);
  const annGroup = useRef(1);
  /** Form fields per document (null: none, or still loading) and the user's unsaved field edits. */
  const [forms, setForms] = useState<Record<number, FormInfo | null>>({});
  const [formEdits, setFormEdits] = useState<Record<number, FormEdits>>({});
  /** Jobs started outside a dialog (e.g. "Run again"): their outcome is reported by a notice. */
  /** Jobs started outside a dialog: their outcome is shown as a notice (failure title per job). */
  const [watched, setWatched] = useState<Array<{ id: string; failTitle: string }>>([]);
  const afterSave = useRef<AfterSave>(null);
  const noticeId = useRef(1);
  const searchToken = useRef(0);
  const viewerHost = useRef<HTMLDivElement>(null);

  const active = tabs.find((t) => t.doc_id === activeId) ?? null;

  // Check signatures once per opened document (read-only, on a worker thread in the host).
  const sigDocId = active?.doc_id;
  useEffect(() => {
    if (sigDocId === undefined) return;
    let live = true;
    setSigs({ docId: sigDocId });
    api
      .verifySignatures(sigDocId)
      .then((report) => live && setSigs({ docId: sigDocId, report }))
      .catch((e) => live && setSigs({ docId: sigDocId, error: toPwError(e) }));
    return () => {
      live = false;
    };
  }, [api, sigDocId]);
  const view: ViewState = (active && views[active.doc_id]) || { zoom: 1, page: 0 };
  const effective = preview ?? settings;
  const pendingOf = (id: number) => annots[id]?.items ?? NO_PENDING;
  const activePending = active ? pendingOf(active.doc_id) : NO_PENDING;
  const editsOf = (id: number) => formEdits[id] ?? NO_EDITS;
  /** Unsaved changes of a document: pending annotations and redaction marks plus changed form fields. */
  const changesOf = (id: number) => pendingOf(id).length + Object.keys(editsOf(id)).length;
  const marksOf = (id: number) => pendingOf(id).filter(isRedactMark).length;
  /** Changes a saved copy can hold (redaction marks are applied, not saved). */
  const savableOf = (id: number) => changesOf(id) - marksOf(id);
  const activeChanges = active ? changesOf(active.doc_id) : 0;
  const activeMarks = active ? marksOf(active.doc_id) : 0;
  const activeSavable = active ? savableOf(active.doc_id) : 0;
  const dirtyTabs = tabs.filter((t) => changesOf(t.doc_id) > 0);
  const totalUnsaved = dirtyTabs.reduce((n, t) => n + changesOf(t.doc_id), 0);
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
    // Diagnostics only; a failure here must not disturb the user.
    api.markTiming("ui_ready").catch(() => undefined);
  }, [api]);

  useEffect(() => {
    api.appInfo().then(setInfo).catch((e) => notifyError("PageWeave could not start correctly", e));
    api.getSettings().then(setSettings).catch((e) => notifyError("Settings could not be loaded", e));
  }, [api, notifyError]);

  // Tasks cut off when PageWeave last closed: say so, say what was cleaned up, offer to run again.
  useEffect(() => {
    api
      .recoveredJobs()
      .then((list: RecoveredJob[]) => {
        for (const r of list) {
          const what = `${OPERATION_TITLES[r.operation.op] ?? r.label}${operationTarget(r.operation) ? ` → ${operationTarget(r.operation)}` : ""}`;
          const cleanup =
            r.leftover_partial_files.length > 0
              ? ` A partial file could not be removed: ${r.leftover_partial_files.map(basename).join(", ")}.`
              : r.removed_partial_files > 0
                ? " Its unfinished file was removed."
                : "";
          notify({
            kind: "warning",
            title: "A task was interrupted",
            detail: `${what} did not finish when PageWeave last closed. No output was saved.${cleanup}`,
            actions: [{ label: "Run again", run: () => runAgain(r) }],
            persistent: true,
          });
        }
      })
      .catch((e) => notifyError("Couldn’t check for interrupted tasks", e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api]);

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
        } else {
          // Open in the host but not shown (e.g. after the page was reloaded): show it again.
          setTabs((ts) => (ts.some((t) => t.doc_id === doc.doc_id) ? ts : [...ts, doc]));
          setViews((v) => (v[doc.doc_id] ? v : { ...v, [doc.doc_id]: { zoom: 1, page: 0, fitPending: true } }));
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

  const closeTabNow = useCallback(
    (id: number) => {
      api.closeDocument(id).catch(() => undefined);
      setTabs((ts) => {
        const i = ts.findIndex((t) => t.doc_id === id);
        const next = ts.filter((t) => t.doc_id !== id);
        if (id === activeId) setActiveId(next[Math.min(i, next.length - 1)]?.doc_id ?? null);
        return next;
      });
      setViews(({ [id]: _gone, ...rest }) => rest);
      setAnnots(({ [id]: _dropped, ...rest }) => rest);
      setForms(({ [id]: _f, ...rest }) => rest);
      setFormEdits(({ [id]: _e, ...rest }) => rest);
      if (id === activeId) {
        setSearchOpen(false);
        setHits([]);
        setSearch(EMPTY_SEARCH);
      }
    },
    [api, activeId],
  );

  /** Close a tab, asking first when it has unsaved annotations. */
  const closeTab = useCallback(
    (id: number) => {
      if ((annots[id]?.items.length ?? 0) + Object.keys(formEdits[id] ?? {}).length > 0) setDialog({ kind: "unsaved", reason: "tab", docId: id });
      else closeTabNow(id);
    },
    [annots, formEdits, closeTabNow],
  );

  // Read a document's form fields the first time it is shown (only documents that have a form).
  useEffect(() => {
    if (!active || active.doc_id in forms) return;
    const id = active.doc_id;
    setForms((f) => ({ ...f, [id]: null }));
    if ((active.info.form_type ?? "none") === "none") return;
    api
      .formFields(id)
      .then((info) => {
        setForms((f) => (id in f ? { ...f, [id]: info } : f));
        if (info.has_xfa) notify({ kind: "warning", title: active.name, detail: "This form uses XFA; its fields cannot be filled in PageWeave.", persistent: true });
      })
      .catch((e) => notifyError("Couldn’t read the form fields", e));
  }, [api, active, forms, notify, notifyError]);

  const onFormEdit = useCallback(
    (field: FormField, value: FormValue) => {
      if (activeId === null) return;
      setFormEdits((all) => ({ ...all, [activeId]: setEdit(all[activeId] ?? {}, field, value) }));
    },
    [activeId],
  );

  // Keep the host's window-close guard in step with the number of unsaved annotations.
  useEffect(() => {
    api.setUnsavedChanges(totalUnsaved).catch((e) => notifyError("Unsaved-changes protection is not active", e));
  }, [api, totalUnsaved, notifyError]);

  useEffect(() => {
    let off: (() => void) | undefined;
    api
      .onCloseRequested(() => {
        setMenu(null);
        setDialog({ kind: "unsaved", reason: "quit" });
      })
      .then((u) => (off = u))
      .catch(() => undefined);
    return () => off?.();
  }, [api]);

  useEffect(() => {
    // Restore tabs the host still has open (the page may have been reloaded), then open files from
    // the command line in order so the last one listed ends up active.
    api
      .openDocuments()
      .then((docs) => {
        if (!docs.length) return;
        setTabs((ts) => [...ts, ...docs.filter((d) => !ts.some((t) => t.doc_id === d.doc_id))]);
        setViews((v) => Object.fromEntries([...Object.entries(v), ...docs.filter((d) => !v[d.doc_id]).map((d) => [d.doc_id, { zoom: 1, page: 0, fitPending: true }])]));
        setActiveId((id) => id ?? docs[docs.length - 1]!.doc_id);
      })
      .catch((e) => notifyError("Couldn’t restore open tabs", e))
      .then(() => api.launchFiles())
      .then(async (files) => {
        // openPath reports its own failures; each file is marked handled only afterwards, so a
        // reload part-way through offers the rest again.
        for (const f of files) {
          await openPath(f);
          await api.launchFileDone(f);
        }
      })
      .catch((e) => notifyError("Couldn’t open the files given at startup", e));
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
          // Dropped images go to "Create PDF from images"; everything else is opened.
          const images = e.paths.filter(isImagePath);
          for (const p of e.paths.filter((p) => !isImagePath(p))) await openPath(p);
          if (images.length) setDialog((d) => (d ? d : { kind: "images", files: images }));
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
  const runSearch = useCallback(async (override?: Partial<Pick<SearchState, "matchCase" | "wholeWord">>) => {
    const opts = { matchCase: search.matchCase, wholeWord: search.wholeWord, ...override };
    if (!active || !search.query.trim()) return;
    const token = ++searchToken.current;
    setSearch((s) => ({ ...s, status: "searching" }));
    try {
      const r = await api.search(active.doc_id, search.query, opts.matchCase, opts.wholeWord);
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

  /** Text currently selected inside a page's text layer (not elsewhere in the UI). */
  const pageSelection = (): string => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed) return "";
    const node = sel.anchorNode instanceof Element ? sel.anchorNode : sel.anchorNode?.parentElement;
    return node?.closest(".textlayer") ? sel.toString().replace(/\n+$/, "") : "";
  };

  const copySelection = async (snapshot?: string) => {
    const text = snapshot ?? pageSelection();
    if (!text) return;
    try {
      await api.copyText(text);
      notify({ kind: "success", title: "Copied", detail: `${text.length} characters`, persistent: false });
    } catch (e) {
      notifyError("Couldn’t copy the text", e);
    }
  };

  /** PDF rectangles of the current page-text selection, grouped by page. */
  const selectionRects = (): Map<number, SearchRect[]> => {
    if (!active || !pageSelection()) return new Map();
    return clientRectsToPageRects(selectionClientRects(window.getSelection()), pageBoxes(), active.info.pages);
  };

  const updateAnnots = (docId: number, f: (h: AnnotationHistory) => AnnotationHistory) =>
    setAnnots((a) => ({ ...a, [docId]: f(a[docId] ?? EMPTY_HISTORY) }));

  const annotateSelection = (kind: Exclude<PendingKind, "note">, snapshot?: Map<number, SearchRect[]>) => {
    if (!active) return;
    const rects = snapshot ?? selectionRects();
    if (rects.size === 0) {
      const what = kind === "redact" ? "mark" : KIND_LABEL[kind].toLowerCase();
      notify({ kind: "warning", title: `Nothing to ${what}`, detail: "The selection has no text on a page.", persistent: false });
      return;
    }
    const group = annGroup.current++;
    const color = kind === "redact" ? REDACT_MARK_COLOR : ANNOTATION_COLORS[kind];
    const added: PendingAnnotation[] = [...rects.entries()]
      .sort(([a], [b]) => a - b)
      .map(([page, r], n) => ({ id: makeId(group, n), page, kind, rects: r.slice(0, 1000), color }));
    updateAnnots(active.doc_id, (h) => addAnnotations(h, added));
    window.getSelection()?.removeAllRanges();
  };

  /** Open the note editor anchored at the selection, a clicked point, or the current page's corner. */
  const startNote = (at?: { page: number; x: number; y: number }, snapshot?: Map<number, SearchRect[]>) => {
    if (!active) return;
    const rects = snapshot ?? selectionRects();
    const first = [...rects.entries()].sort(([a], [b]) => a - b)[0];
    if (first && first[1][0]) {
      const p = active.info.pages[first[0]]!;
      setDialog({ kind: "note", page: first[0], rect: noteRect(first[1][0].left, first[1][0].top, p) });
      return;
    }
    const pageIndex = at?.page ?? view.page;
    const p = active.info.pages[pageIndex];
    if (!p) return;
    const box = at && pageBoxes().find((b) => b.page === pageIndex);
    if (at && box) {
      const zoom = box.width / (displaySizePt(p).w * CSS_PX_PER_PT);
      const pt = screenRectToPdf({ left: at.x - box.left, top: at.y - box.top, width: 0, height: 0 }, p, zoom);
      setDialog({ kind: "note", page: pageIndex, rect: noteRect(pt.left, pt.top, p) });
    } else {
      const o = pageOrigin(p);
      setDialog({ kind: "note", page: pageIndex, rect: noteRect(o.x + 36, o.y + p.height_pt - 36, p) });
    }
  };

  /** Add redaction marks (one undo step): each entry is a page and its rectangles. */
  const addMarks = (entries: Array<[number, SearchRect[]]>) => {
    if (!active || entries.length === 0) return;
    const group = annGroup.current++;
    const room = Math.max(0, MAX_PENDING - activePending.length);
    const added: PendingAnnotation[] = entries
      .slice(0, Math.min(room, 1000))
      .map(([page, r], n) => ({ id: makeId(group, n), page, kind: "redact", rects: r.slice(0, 1000), color: REDACT_MARK_COLOR }));
    updateAnnots(active.doc_id, (h) => addAnnotations(h, added));
    if (added.length < entries.length) {
      notify({ kind: "warning", title: `Marked ${added.length} of ${entries.length}`, detail: "Apply these redactions first, then mark the rest.", persistent: true });
    }
  };

  const markPage = (pageIndex: number) => {
    const p = active?.info.pages[pageIndex];
    if (!p) return;
    const o = pageOrigin(p);
    addMarks([[pageIndex, [{ left: o.x, bottom: o.y, right: o.x + p.width_pt, top: o.y + p.height_pt }]]]);
  };

  const markMatches = () => {
    // One mark per match, so each can be reviewed and undone with the others in one step.
    addMarks(hits.map((h) => [h.page, h.rects]));
  };

  const removeMark = (docId: number, mark: PendingAnnotation) => {
    updateAnnots(docId, (h) => removeItem(h, mark.id));
    notify({
      kind: "info",
      title: "Redaction mark removed",
      actions: [{ label: "Undo", run: () => updateAnnots(docId, (h) => addAnnotations({ ...h, redo: h.redo }, [mark])) }],
      persistent: false,
    });
  };

  const openRedactDialog = () => {
    if (!active) return;
    setMarkMode(false);
    setDialog({ kind: "redact", areas: redactAreas(activePending) });
  };

  const discardChanges = (docId: number) => {
    const n = changesOf(docId);
    if (!n) return;
    const before = annots[docId];
    const beforeEdits = formEdits[docId];
    setAnnots(({ [docId]: _gone, ...rest }) => rest);
    setFormEdits(({ [docId]: _gone, ...rest }) => rest);
    notify({
      kind: "info",
      title: `Discarded ${n} change${n === 1 ? "" : "s"}`,
      actions: [
        {
          label: "Undo",
          run: () => {
            if (before) setAnnots((a) => ({ ...a, [docId]: before }));
            if (beforeEdits) setFormEdits((e) => ({ ...e, [docId]: beforeEdits }));
          },
        },
      ],
      persistent: false,
    });
  };

  const openSaveDialog = () => {
    if (!active) return;
    const missing = missingRequired(editsOf(active.doc_id), forms[active.doc_id]?.fields ?? []);
    if (missing.length) {
      notify({ kind: "warning", title: "Required fields are empty", detail: missing.slice(0, 5).map(fieldLabel).join(", ") + (missing.length > 5 ? ", …" : ""), persistent: false });
    }
    openTool("save");
  };

  const selectionAvail = (): true | string => (pageSelection() ? true : "Select text on a page first.");
  const roomAvail = (): true | string => (activePending.length < MAX_PENDING ? true : "Save a copy first; too many unsaved annotations.");

  const handlers: CommandHandlers = {
    "edit.copy": { run: () => copySelection(), available: () => (pageSelection() ? true : "Select text on a page first.") },
    "edit.undo": {
      run: () => active && updateAnnots(active.doc_id, undo),
      available: all(needDoc, () => (activePending.length ? true : "There is nothing to undo.")),
    },
    "edit.redo": {
      run: () => active && updateAnnots(active.doc_id, redo),
      available: all(needDoc, () => (active && annots[active.doc_id]?.redo.length ? true : "There is nothing to redo.")),
    },
    "annotate.highlight": { run: () => annotateSelection("highlight"), available: all(needDoc, noDialog, roomAvail, selectionAvail) },
    "annotate.underline": { run: () => annotateSelection("underline"), available: all(needDoc, noDialog, roomAvail, selectionAvail) },
    "annotate.strikeOut": { run: () => annotateSelection("strike_out"), available: all(needDoc, noDialog, roomAvail, selectionAvail) },
    "annotate.note": { run: () => startNote(), available: all(needDoc, noDialog, roomAvail) },
    "redact.markSelection": { run: () => annotateSelection("redact"), available: all(needDoc, noDialog, roomAvail, selectionAvail) },
    "redact.markArea": {
      run: () => setMarkMode((m) => !m),
      available: all(needDoc, noDialog, () => (markMode ? true : roomAvail())),
    },
    "redact.markPage": { run: () => markPage(view.page), available: all(needDoc, noDialog, roomAvail) },
    "redact.markMatches": {
      run: markMatches,
      available: all(needDoc, noDialog, roomAvail, () => (searchOpen && hits.length ? true : "Find text first; every match is then marked.")),
    },
    "redact.apply": {
      run: openRedactDialog,
      available: all(
        needDoc,
        noDialog,
        () => (activeMarks ? true : "Mark text or areas for redaction first."),
        () => (activeSavable ? "Save or discard your other changes first; redaction starts from the file as saved." : true),
      ),
    },
    "file.saveCopy": {
      run: openSaveDialog,
      available: all(needDoc, noDialog, () => (activeSavable ? true : activeMarks ? "Redaction marks are not saved; use Apply redactions." : "There are no unsaved changes.")),
    },
    "edit.discardChanges": {
      run: () => active && discardChanges(active.doc_id),
      available: all(needDoc, () => (activeChanges ? true : "There are no unsaved changes.")),
    },
    "file.open": { run: openDialog, available: () => true },
    "file.close": { run: () => active && closeTab(active.doc_id), available: needDoc },
    "file.reveal": { run: () => active && api.revealInFolder(active.path).catch((e) => notifyError("Couldn’t show the file", e)), available: needDoc },
    "file.properties": { run: () => setDialog({ kind: "properties" }), available: all(needDoc, noDialog) },
    "file.protect": { run: () => setDialog({ kind: "protect" }), available: all(needDoc, noDialog) },
    "file.sanitize": { run: () => setDialog({ kind: "sanitize" }), available: all(needDoc, noDialog) },
    "file.reduceSize": { run: () => openTool("optimize"), available: all(needDoc, noDialog) },
    "file.compare": { run: () => setDialog({ kind: "compare" }), available: all(needDoc, noDialog) },
    "file.print": { run: () => setDialog({ kind: "print" }), available: all(needDoc, noDialog) },
    "file.imagesToPdf": { run: () => setDialog({ kind: "images", files: [] }), available: noDialog },
    "file.editBookmarks": { run: () => setDialog({ kind: "bookmarks", addOnOpen: false, page: view.page }), available: all(needDoc, noDialog) },
    "file.attachFiles": { run: () => active && attachFiles(active), available: all(needDoc, noDialog) },
    "file.editProperties": { run: () => setDialog({ kind: "editProperties" }), available: all(needDoc, () => (dialog && dialog.kind !== "properties" ? "Close the open dialog first." : true)) },
    "file.unprotect": {
      run: () => setDialog({ kind: "unprotect" }),
      available: all(needDoc, noDialog, () => (active?.info.encrypted ? true : "This file is not password protected.")),
    },
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
    "view.showSignatures": {
      run: () => {
        setShowThumbs(true);
        setSidebarView("signatures");
        requestAnimationFrame(() => document.querySelector<HTMLElement>("#sb-tab-signatures")?.focus());
      },
      available: needDoc,
    },
    "view.showBookmarks": {
      run: () => {
        setShowThumbs(true);
        setSidebarView("bookmarks");
        requestAnimationFrame(() => document.querySelector<HTMLElement>('.bookmarks [tabindex="0"]')?.focus());
      },
      available: all(needDoc, () => (active && active.info.bookmarks.length > 0 ? true : "This document has no bookmarks.")),
    },
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
    "pages.organize": { run: () => setDialog({ kind: "organize" }), available: all(needDoc, noDialog) },
    "pages.stamp": { run: () => openTool("stamp"), available: all(needDoc, noDialog) },
    "pages.exportImages": { run: () => openTool("images"), available: all(needDoc, noDialog) },
    "pages.exportText": { run: () => openTool("text"), available: all(needDoc, noDialog) },
    "pages.deskew": { run: () => openTool("deskew"), available: all(needDoc, noDialog) },
    "pages.ocr": { run: () => openTool("ocr"), available: all(needDoc, noDialog) },
    "pages.watermark": { run: () => openTool("watermark"), available: all(needDoc, noDialog) },
    "pages.crop": { run: () => openTool("crop"), available: all(needDoc, noDialog) },
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

  useEffect(() => setMarkMode(false), [activeId]);
  useEffect(() => {
    if (!markMode) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !dialog && !menu) {
        e.preventDefault();
        setMarkMode(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [markMode, dialog, menu]);

  /* ---------- Keyboard ---------- */
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const sc = eventToShortcut(e);
      // The browser's own print would print the app's screens: printing always goes through the
      // Print dialog, never the default action (also while a menu, dialog or text field has focus).
      if (sc === "Ctrl+P") e.preventDefault();
      if (e.defaultPrevented && sc !== "Ctrl+P") return;
      if (menu || !sc) return;
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
    // Opening the menu moves focus, which can clear the page selection: snapshot it and restore on close.
    const selected = pageSelection();
    const rects = selected ? selectionRects() : null;
    const room = roomAvail();
    // Redaction marks under the pointer (a point in this page's PDF space).
    const p = active?.info.pages[page];
    const box = pageBoxes().find((b) => b.page === page);
    const pt = p && box ? screenRectToPdf({ left: x - box.left, top: y - box.top, width: 0, height: 0 }, p, box.width / (displaySizePt(p).w * CSS_PX_PER_PT)) : null;
    const marksHere = pt ? itemsAt(activePending.filter(isRedactMark), page, pt.left, pt.top) : [];
    const sel = window.getSelection();
    const range = selected && sel && sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
    const restore = () => {
      if (!range) return;
      const s2 = window.getSelection();
      if (s2 && (s2.isCollapsed || s2.rangeCount === 0)) {
        s2.removeAllRanges();
        s2.addRange(range);
      }
    };
    setMenu({
      label: `Page ${n}`,
      x,
      y,
      afterClose: restore,
      items: [
        // Actions on the mark under the pointer come first.
        ...marksHere.flatMap((m, i) => [item("redact.apply", "Remove this redaction mark", () => active && removeMark(active.doc_id, m), true), ...(i === marksHere.length - 1 ? ["separator" as const] : [])]),
        ...(selected
          ? [
              { ...(item("edit.copy", "Copy", () => copySelection(selected), true) as Exclude<MenuItemOrSeparator, "separator">), shortcut: "Ctrl+C" },
              "separator" as const,
              { ...(item("annotate.highlight", "Highlight", () => annotateSelection("highlight", rects!), room) as Exclude<MenuItemOrSeparator, "separator">), shortcut: bindings["annotate.highlight"]?.[0] },
              { ...(item("annotate.underline", "Underline", () => annotateSelection("underline", rects!), room) as Exclude<MenuItemOrSeparator, "separator">), shortcut: bindings["annotate.underline"]?.[0] },
              item("annotate.strikeOut", "Strikethrough", () => annotateSelection("strike_out", rects!), room),
              item("annotate.note", "Add note…", () => startNote(undefined, rects!), room),
              { ...(item("redact.markSelection", "Mark for redaction", () => annotateSelection("redact", rects!), room) as Exclude<MenuItemOrSeparator, "separator">), shortcut: bindings["redact.markSelection"]?.[0] },
              "separator" as const,
            ]
          : [item("annotate.note", "Add note here…", () => startNote({ page, x, y }, new Map()), room), "separator" as const]),
        item("pages.extract", `Extract page ${n}…`, () => openTool("extract", String(n))),
        item("pages.rotate", `Rotate page ${n}…`, () => openTool("rotate", String(n))),
        item("pages.delete", `Delete page ${n}…`, () => openTool("delete", String(n)), multi ? true : "A one-page document has no pages to delete."),
        item("pages.organize"),
        item("file.editBookmarks", `Add bookmark for page ${n}…`, () => setDialog({ kind: "bookmarks", addOnOpen: true, page }), dialog ? "Close the open dialog first." : true),
        "separator",
        item("redact.markPage", `Mark page ${n} for redaction`, () => markPage(page), room),
        item("redact.markArea", markMode ? "Stop marking areas" : "Mark areas for redaction"),
        item("redact.apply"),
        "separator",
        item("view.zoomIn"),
        item("view.zoomOut"),
        item("view.fitWidth"),
        "separator",
        item("file.properties"),
      ],
    });
  };

  /** Close every other tab; tabs with unsaved annotations stay open (one prompt per tab would pile up). */
  const closeOthers = (keepId: number) => {
    const others = tabs.filter((t) => t.doc_id !== keepId);
    const kept = others.filter((t) => changesOf(t.doc_id) > 0);
    others.filter((t) => !kept.includes(t)).forEach((t) => closeTabNow(t.doc_id));
    if (kept.length) {
      notify({ kind: "warning", title: `${kept.length} tab${kept.length === 1 ? " was" : "s were"} kept open`, detail: `Unsaved changes: ${kept.map((t) => t.name).join(", ")}.`, persistent: true });
    }
  };

  const tabMenu = (doc: OpenedDoc, x: number, y: number) =>
    setMenu({
      label: doc.name,
      x,
      y,
      items: [
        item("file.close", "Close", () => closeTab(doc.doc_id), true),
        item("file.close", "Close other tabs", () => closeOthers(doc.doc_id), tabs.length > 1 ? true : "No other tabs are open."),
        "separator",
        item("file.reveal", "Show in folder", () => api.revealInFolder(doc.path).catch((e) => notifyError("Couldn’t show the file", e)), true),
        item("file.protect", "Protect with password…", () => {
          setActiveId(doc.doc_id);
          setDialog({ kind: "protect" });
        }, dialog ? "Close the open dialog first." : true),
        item("file.unprotect", "Remove password…", () => {
          setActiveId(doc.doc_id);
          setDialog({ kind: "unprotect" });
        }, dialog ? "Close the open dialog first." : doc.info.encrypted ? true : "This file is not password protected."),
        item("file.sanitize", "Remove hidden information…", () => {
          setActiveId(doc.doc_id);
          setDialog({ kind: "sanitize" });
        }, dialog ? "Close the open dialog first." : true),
        item("file.compare", "Compare with…", () => {
          setActiveId(doc.doc_id);
          setDialog({ kind: "compare" });
        }, dialog ? "Close the open dialog first." : true),
        item("file.reduceSize", "Reduce file size…", () => {
          setActiveId(doc.doc_id);
          setDialog({ kind: "tool", tool: "optimize" });
        }, dialog ? "Close the open dialog first." : true),
      ],
    });

  const pagesMenu = (e: React.MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ label: "Page tools", x: r.left, y: r.bottom + 2, items: [item("pages.organize"), "separator", item("pages.extract"), item("pages.delete"), item("pages.rotate"), item("pages.split"), item("pages.stamp"), item("pages.watermark"), item("pages.crop"), item("pages.exportImages"), item("pages.exportText"), item("pages.ocr"), item("pages.deskew"), "separator", item("pages.merge")] });
  };

  /* ---------- Job results ---------- */
  const onJobFinished = (job: JobSnapshot) => {
    const r = job.result;
    if (!r) return;
    const out = r.outputs[0];
    const title = r.outputs.length > 1 ? `Saved ${r.outputs.length} files` : out ? `Saved ${basename(out.path)}` : "Done";
    const actions: NoticeAction[] = [];
    // Only PDFs open in PageWeave (an exported image or a saved attachment does not).
    if (out && r.outputs.length === 1 && out.path.toLowerCase().endsWith(".pdf")) actions.push({ label: "Open", run: () => openPath(out.path) });
    if (out) actions.push({ label: "Show in folder", run: () => api.revealInFolder(out.path).catch((e) => notifyError("Couldn’t show the file", e)) });
    const before = r.data?.bytes_before;
    const after = r.data?.bytes_after;
    const straightened = r.data?.straightened;
    const detail =
      typeof before === "number" && typeof after === "number"
        ? sizeChange(before, after)
        : typeof straightened === "number"
          ? `${straightened} page${straightened === 1 ? "" : "s"} straightened`
          : out?.pages != null && r.outputs.length === 1
            ? `${out.pages} pages`
            : undefined;
    notify({ kind: "success", title, detail, actions, persistent: false });
    for (const w of r.warnings) notify({ kind: "warning", title: "Please note", detail: w, persistent: true });
  };

  /** Start a job for the active document outside any dialog; its outcome arrives as a notice. */
  const runWatched = async (operation: Operation, docId: number, failTitle: string, collision: CollisionPolicy = "rename") => {
    try {
      const id = await api.startOperation({ schema_version: 1, operation, collision }, docId);
      setWatched((w) => [...w, { id, failTitle }]);
    } catch (e) {
      notifyError(failTitle, e);
    }
  };


  const attachFiles = async (doc: OpenedDoc) => {
    const files = await api.pickAnyFiles();
    if (files.length) await runWatched({ op: "add_attachments", input: doc.path, files, output: siblingOutput(doc.path, "attached") }, doc.doc_id, "Couldn’t attach the files");
  };

  const runAgain = async (r: RecoveredJob) => {
    try {
      const id = await api.startOperation({ schema_version: 1, operation: r.operation, collision: r.collision });
      setWatched((w) => [...w, { id, failTitle: "The task failed again" }]);
    } catch (e) {
      notifyError(`Couldn’t run ${OPERATION_TITLES[r.operation.op]?.toLowerCase() ?? "the task"} again`, e);
    }
  };

  useEffect(() => {
    for (const { id, failTitle } of watched) {
      const j = jobs[id];
      if (!j || !["succeeded", "failed", "cancelled"].includes(j.state)) continue;
      setWatched((w) => w.filter((x) => x.id !== id));
      if (j.state === "succeeded") onJobFinished(j);
      else if (j.state === "failed") notifyError(failTitle, j.error);
      else notify({ kind: "info", title: "Task cancelled", persistent: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobs, watched]);

  /**
   * The copy with the user's changes is committed: drop the saved changes, then either continue a
   * pending close/quit or switch the tab to the saved copy (Save As semantics), keeping the view.
   */
  const afterChangesSaved = (docId: number, job: JobSnapshot) => {
    // Redaction marks are not part of the saved copy: they move to its tab (same pages, same coordinates).
    const marks = pendingOf(docId).filter(isRedactMark);
    setAnnots(({ [docId]: _saved, ...rest }) => rest);
    setFormEdits(({ [docId]: _saved, ...rest }) => rest);
    const next = afterSave.current;
    afterSave.current = null;
    if (next?.kind === "close") return closeTabNow(next.docId);
    if (next?.kind === "quit") {
      if (dirtyTabs.every((t) => t.doc_id === docId)) api.quitApp().catch((e) => notifyError("PageWeave could not close", e));
      return;
    }
    const out = job.result?.outputs[0]?.path;
    if (!out) return;
    api
      .openDocument(out)
      .then((doc) => {
        if (doc.already_open) {
          closeTabNow(docId);
          setActiveId(doc.doc_id);
          return;
        }
        setTabs((ts) => ts.map((t) => (t.doc_id === docId ? doc : t)));
        if (marks.length) setAnnots((a) => ({ ...a, [doc.doc_id]: { items: marks, redo: [] } }));
        setViews((v) => {
          const { [docId]: old, ...rest } = v;
          return { ...rest, [doc.doc_id]: { ...(old ?? { zoom: 1, page: 0 }), fitPending: false } };
        });
        setForms(({ [docId]: _f, ...rest }) => rest);
        setActiveId(doc.doc_id);
        api.closeDocument(docId).catch(() => undefined);
      })
      .catch((e) => notifyError(`Saved, but ${basename(out)} could not be opened`, e, [{ label: "Open", run: () => openPath(out) }]));
  };

  /** The redacted copy is verified and written: the marks are used up; open the copy to inspect it. */
  const afterRedacted = (docId: number, job: JobSnapshot) => {
    updateAnnots(docId, (h) => ({ items: h.items.filter((a) => !isRedactMark(a)), redo: h.redo.filter((a) => !isRedactMark(a)) }));
    const out = job.result?.outputs[0]?.path;
    if (out) openPath(out);
  };

  /* ---------- Render ---------- */
  const zoomPct = Math.round(view.zoom * 100);
  const zoomOptions = Array.from(new Set([...ZOOM_PRESETS.map((z) => Math.round(z * 100)), zoomPct])).sort((a, b) => a - b);

  return (
    <div className="app">
      <header className="toolbar-host">
      <div className="toolbar" role="toolbar" aria-label="Main toolbar">
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
      </div>
      </header>

      {tabs.length > 0 ? (
        <nav className="tabbar" aria-label="Open documents">
          <ul className="tablist">
          {tabs.map((t) => (
            // The close button is a sibling of the tab, not inside it (nested interactive controls are
            // not reliably announced by screen readers).
            <li key={t.doc_id} className={`tab ${t.doc_id === activeId ? "selected" : ""}`} title={t.path} onAuxClick={(e) => e.preventDefault()}>
              <button
                type="button"
                className="tab-label"
                data-doc={t.doc_id}
                tabIndex={t.doc_id === activeId ? 0 : -1}
                aria-current={t.doc_id === activeId ? "page" : undefined}
                onClick={() => setActiveId(t.doc_id)}
                onKeyDown={(e) => {
                  const i = tabs.findIndex((x) => x.doc_id === t.doc_id);
                  const move = (j: number) => {
                    const next = tabs[(j + tabs.length) % tabs.length]!;
                    setActiveId(next.doc_id);
                    requestAnimationFrame(() => document.querySelector<HTMLElement>(`.tab-label[data-doc="${next.doc_id}"]`)?.focus());
                  };
                  if (e.key === "ArrowRight") move(i + 1);
                  else if (e.key === "ArrowLeft") move(i - 1);
                  else if (e.key === "Home") move(0);
                  else if (e.key === "End") move(tabs.length - 1);
                  else if (e.key === "Delete") closeTab(t.doc_id);
                  else if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
                    const r = e.currentTarget.getBoundingClientRect();
                    tabMenu(t, r.left, r.bottom);
                  } else return;
                  e.preventDefault();
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  tabMenu(t, e.clientX, e.clientY);
                }}
              >
                <span className="name">{t.name}</span>
              </button>
              <button
                type="button"
                className="btn icon close"
                tabIndex={-1}
                aria-label={`Close ${t.name}`}
                title={`Close (${bindings["file.close"]?.[0] ?? ""}, or Delete on the tab)`}
                onClick={() => closeTab(t.doc_id)}
              >
                <X />
              </button>
            </li>
          ))}
          </ul>
        </nav>
      ) : (
        <div />
      )}

      {active ? (
        <main className="workspace" aria-label={active.name} style={showThumbs ? undefined : { gridTemplateColumns: "1fr" }}>
          <h1 className="sr-only">{active.name}</h1>
          {showThumbs && (
            <Sidebar
              doc={active}
              view={sidebarView}
              onView={setSidebarView}
              currentPage={view.page}
              onSelectPage={goToPage}
              onPageContextMenu={pageMenu}
              onSaveAttachment={(a) => setDialog({ kind: "saveAttachment", attachment: a })}
              onAttachFiles={() => attachFiles(active)}
              onEditBookmarks={() => setDialog({ kind: "bookmarks", addOnOpen: false, page: view.page })}
              signatures={sigs?.docId === active.doc_id ? (sigs.report ?? (sigs.error ? null : undefined)) : undefined}
              signaturesError={sigs?.docId === active.doc_id ? (sigs.error ?? null) : null}
            />
          )}
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
                  // Toggling an option re-runs an existing query immediately.
                  if ((p.matchCase !== undefined || p.wholeWord !== undefined) && search.query.trim()) {
                    runSearch({ matchCase: p.matchCase ?? search.matchCase, wholeWord: p.wholeWord ?? search.wholeWord });
                  }
                }}
                onSubmit={() => runSearch()}
                onNext={() => stepHit(1)}
                onPrevious={() => stepHit(-1)}
                onClose={closeSearch}
                onMarkAll={markMatches}
                markAllAvailable={handlers["redact.markMatches"].available()}
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
              annotations={activePending}
              form={forms[active.doc_id] ?? null}
              formEdits={editsOf(active.doc_id)}
              onFormEdit={onFormEdit}
              markMode={markMode}
              onMarkArea={(page, rect) => addMarks([[page, [rect]]])}
              onMeasured={(width) => {
                if (view.fitPending) setView({ zoom: Math.min(1.5, fitWidthZoom(active.info.pages, width)), fitPending: false });
              }}
            />
          </div>
        </main>
      ) : (
        <div ref={viewerHost} style={{ display: "grid", minHeight: 0 }}>
          <Home
            recent={settings?.recent_files ?? []}
            dropActive={dropActive}
            onOpen={openDialog}
            onImagesToPdf={() => setDialog({ kind: "images", files: [] })}
            onOpenRecent={(p) => openPath(p)}
            onRemoveRecent={removeRecent}
            openShortcut={bindings["file.open"]?.[0]}
          />
        </div>
      )}

      <StatusBar doc={active} currentPage={view.page} zoom={view.zoom} jobs={Object.values(jobs)} onGoToPage={goToPage} goToNonce={goToNonce} onCancelJob={(id) => api.cancelJob(id)} unsaved={activeSavable} onSaveAnnotations={() => runCommand("file.saveCopy")} marks={activeMarks} onApplyRedactions={() => runCommand("redact.apply")} markMode={markMode} onEndMarkMode={() => setMarkMode(false)} signature={sigs?.docId === active?.doc_id ? signatureSummary(sigs?.report ?? null) : null} onShowSignatures={() => runCommand("view.showSignatures")} />

      {menu && (
        <Menu
          {...menu}
          onClose={() => {
            setMenu(null);
            menu.afterClose?.();
          }}
        />
      )}

      {dialog?.kind === "tool" && active && (
        <PageToolsDialog
          tool={dialog.tool}
          doc={active}
          initialPages={dialog.initialPages}
          annotations={dialog.tool === "save" ? annotationSpecs(activePending) : undefined}
          formValues={dialog.tool === "save" ? toFieldValues(editsOf(active.doc_id), forms[active.doc_id]?.fields ?? []) : undefined}
          jobs={jobs}
          onClose={() => {
            afterSave.current = null;
            setDialog(null);
          }}
          onFinished={(job) => {
            if (dialog.tool === "save") afterChangesSaved(active.doc_id, job);
            onJobFinished(job);
          }}
        />
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
      {dialog?.kind === "properties" && active && <PropertiesDialog doc={active} onClose={() => setDialog(null)} onEdit={() => setDialog({ kind: "editProperties" })} />}
      {dialog?.kind === "password" && (
        <PasswordDialog name={basename(dialog.path)} error={dialog.error} onSubmit={(pw) => openPath(dialog.path, pw)} onCancel={() => setDialog(null)} />
      )}

      {dialog?.kind === "redact" && active && (
        <RedactDialog
          doc={active}
          areas={dialog.areas}
          jobs={jobs}
          onClose={() => setDialog(null)}
          onFinished={(job) => {
            afterRedacted(active.doc_id, job);
            onJobFinished(job);
          }}
        />
      )}
      {dialog?.kind === "protect" && active && <ProtectDialog doc={active} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "images" && <ImagesToPdfDialog initialFiles={dialog.files} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "bookmarks" && active && <EditBookmarksDialog doc={active} currentPage={dialog.page} addOnOpen={dialog.addOnOpen} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "saveAttachment" && active && <SaveAttachmentDialog doc={active} attachment={dialog.attachment} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "editProperties" && active && <EditPropertiesDialog doc={active} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "unprotect" && active && <UnprotectDialog doc={active} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "sanitize" && active && <SanitizeDialog doc={active} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "print" && active && <PrintDialog doc={active} currentPage={view.page} onClose={() => setDialog(null)} />}
      {dialog?.kind === "compare" && active && <CompareView doc={active} others={tabs.filter((t) => t.doc_id !== active.doc_id)} jobs={jobs} onClose={() => setDialog(null)} />}
      {dialog?.kind === "organize" && active && <OrganizeDialog doc={active} jobs={jobs} onClose={() => setDialog(null)} onFinished={onJobFinished} />}
      {dialog?.kind === "note" && active && (
        <NoteDialog
          page={dialog.page}
          onCancel={() => setDialog(null)}
          onAdd={(text) => {
            const group = annGroup.current++;
            updateAnnots(active.doc_id, (h) => addAnnotations(h, [{ id: makeId(group, 0), page: dialog.page, kind: "note", rects: [dialog.rect], color: ANNOTATION_COLORS.note, contents: text }]));
            setDialog(null);
          }}
        />
      )}
      {dialog?.kind === "unsaved" && (() => {
        const docs = dialog.reason === "tab" ? tabs.filter((t) => t.doc_id === dialog.docId) : dirtyTabs;
        const target = docs.length === 1 ? docs[0]! : null;
        return (
          <UnsavedDialog
            names={docs.map((d) => d.name)}
            count={docs.reduce((n, d) => n + changesOf(d.doc_id), 0)}
            marks={docs.reduce((n, d) => n + marksOf(d.doc_id), 0)}
            canSave={!!target && savableOf(target.doc_id) > 0}
            onCancel={() => setDialog(null)}
            onDiscard={() => {
              setDialog(null);
              if (dialog.reason === "tab") closeTabNow(dialog.docId);
              else api.quitApp().catch((e) => notifyError("PageWeave could not close", e));
            }}
            onSave={() => {
              if (!target) return;
              afterSave.current = dialog.reason === "tab" ? { kind: "close", docId: target.doc_id } : { kind: "quit" };
              setActiveId(target.doc_id);
              setDialog({ kind: "tool", tool: "save" });
            }}
          />
        );
      })()}

      <Notices notices={notices} onDismiss={dismiss} />
    </div>
  );
}

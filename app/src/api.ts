// The UI's only gateway to the host. Components receive an `Api` object (see ApiContext) so tests
// can supply a fake without touching Tauri internals.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { createContext, useContext } from "react";
import type {
  AppInfo,
  JobSnapshot,
  OpenedDoc,
  OperationRequest,
  PageText,
  SearchHit,
  Settings,
} from "./types";

export interface DropEvent {
  type: "enter" | "over" | "drop" | "leave";
  paths: string[];
}

export interface Api {
  appInfo(): Promise<AppInfo>;
  getSettings(): Promise<Settings>;
  saveSettings(s: Settings): Promise<Settings>;
  openDocument(path: string, password?: string): Promise<OpenedDoc>;
  closeDocument(docId: number): Promise<boolean>;
  validateRange(docId: number, spec: string): Promise<number[]>;
  search(docId: number, query: string, matchCase: boolean, wholeWord: boolean): Promise<{ hits: SearchHit[]; truncated: boolean }>;
  startOperation(request: OperationRequest, docId?: number): Promise<string>;
  cancelJob(jobId: string): Promise<boolean>;
  onJobUpdate(cb: (job: JobSnapshot) => void): Promise<UnlistenFn>;
  onFileDrop(cb: (e: DropEvent) => void): Promise<UnlistenFn>;
  pageUrl(docId: number, page: number, scalePermille: number): string;
  pickPdfFiles(multiple: boolean): Promise<string[]>;
  pickSavePath(defaultPath: string): Promise<string | null>;
  pickFolder(defaultPath?: string): Promise<string | null>;
  revealInFolder(path: string): Promise<void>;
  launchFiles(): Promise<string[]>;
  logDir(): Promise<string>;
  pageText(docId: number, page: number): Promise<PageText>;
  copyText(text: string): Promise<void>;
  /** Tell the host how many unsaved edits exist, so closing the window asks first. */
  setUnsavedChanges(count: number): Promise<void>;
  /** The window close button was pressed while unsaved edits exist (the host held the close back). */
  onCloseRequested(cb: () => void): Promise<UnlistenFn>;
  /** Close the window, discarding unsaved edits. */
  quitApp(): Promise<void>;
  /** Report a startup milestone; the host logs time since start once per run. */
  markTiming(name: "ui_ready" | "first_page"): Promise<number | null>;
}

export const tauriApi: Api = {
  appInfo: () => invoke("app_info"),
  getSettings: () => invoke("get_settings"),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  openDocument: (path, password) => invoke("open_document", { path, password: password ?? null }),
  closeDocument: (docId) => invoke("close_document", { docId }),
  validateRange: (docId, spec) => invoke("validate_range", { docId, spec }),
  search: (docId, query, matchCase, wholeWord) => invoke("search", { docId, query, matchCase, wholeWord }),
  startOperation: (request, docId) => invoke("start_operation", { request, docId: docId ?? null }),
  cancelJob: (jobId) => invoke("cancel_job", { jobId }),
  onJobUpdate: (cb) => listen<JobSnapshot>("job-update", (e) => cb(e.payload)),
  onFileDrop: (cb) =>
    getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      cb({ type: p.type, paths: "paths" in p ? p.paths : [] });
    }),
  pageUrl: (docId, page, scale) => convertFileSrc(`${docId}/${page}/${scale}`, "pwpage"),
  pickPdfFiles: async (multiple) => {
    const r = await openDialog({ multiple, directory: false, filters: [{ name: "PDF documents", extensions: ["pdf"] }] });
    if (r === null) return [];
    return Array.isArray(r) ? r : [r];
  },
  pickSavePath: (defaultPath) => saveDialog({ defaultPath, filters: [{ name: "PDF document", extensions: ["pdf"] }] }),
  pickFolder: async (defaultPath) => {
    const r = await openDialog({ directory: true, multiple: false, defaultPath });
    return typeof r === "string" ? r : null;
  },
  revealInFolder: (path) => revealItemInDir(path),
  launchFiles: () => invoke("launch_files"),
  logDir: () => invoke("log_dir"),
  pageText: (docId, page) => invoke("page_text", { docId, page }),
  setUnsavedChanges: (count) => invoke("set_unsaved_changes", { count }),
  onCloseRequested: (cb) => listen("close-requested", () => cb()),
  quitApp: () => invoke("quit_app"),
  markTiming: (name) => invoke("mark_timing", { name }),
  copyText: async (text) => {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
    if (!document.execCommand("copy")) throw new Error("copy failed");
  },
};

export const ApiContext = createContext<Api>(tauriApi);
export const useApi = () => useContext(ApiContext);

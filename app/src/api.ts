// The UI's only gateway to the host. Components receive an `Api` object (see ApiContext) so tests
// can supply a fake without touching Tauri internals.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { createContext, useContext } from "react";
import { OFFICE_EXTENSIONS } from "./office";
import type {
  AppInfo,
  Design,
  DesignFormat,
  AttachmentInfo,
  ExistingAnnotation,
  ResourceReport,
  AreaPreview,
  JobSnapshot,
  RedactArea,
  OpenedDoc,
  OperationRequest,
  FormInfo,
  PageText,
  ReadingPage,
  SpeechInfo,
  TranslationDirection,
  UpdateInfo,
  AvailableUpdate,
  OfficePackage,
  Spoken,
  RecoveredJob,
  SanitizeFindings,
  SignatureReport,
  ChainCertificate,
  TrustedCert,
  EditableObjects,
  PageEdit,
  SearchHit,
  Settings,
} from "./types";

const EXPORT_FILTERS: Record<DesignFormat, { name: string; extensions: string[] }> = {
  svg: { name: "SVG drawing", extensions: ["svg"] },
  pdf: { name: "PDF document", extensions: ["pdf"] },
  png: { name: "PNG picture", extensions: ["png"] },
};

export interface DropEvent {
  type: "enter" | "over" | "drop" | "leave";
  paths: string[];
}

export interface Api {
  appInfo(): Promise<AppInfo>;
  getSettings(): Promise<Settings>;
  saveSettings(s: Settings): Promise<Settings>;
  /** `remember: false` opens without adding to recent files (page previews for insertion). */
  openDocument(path: string, password?: string, remember?: boolean): Promise<OpenedDoc>;
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
  /** Save dialog for a design export (SVG or PDF). */
  pickDesignExport(defaultPath: string, format: DesignFormat): Promise<string | null>;
  /** A picture, PDF or design project to open in the Studio. */
  pickDesignSource(): Promise<string | null>;
  loadDesign(path: string): Promise<Design>;
  /** A PNG/JPEG file decoded and re-encoded as PNG (base64), for replacing a picture. */
  loadPicture(path: string): Promise<string>;
  /** A picture (PNG, base64) with fractions of its width/height trimmed from each side, re-encoded. */
  cropPicture(png: string, trim: [number, number, number, number]): Promise<string>;
  /** Save edits over the design project at `path` (an existing .pwdesign only); returns its size. */
  saveDesign(path: string, design: Design): Promise<number>;
  pickFolder(defaultPath?: string): Promise<string | null>;
  revealInFolder(path: string): Promise<void>;
  /** Command-line files the UI has not handled yet; `launchFileDone` marks one handled. */
  /** What redacting `areas` would remove (read-only, runs in the engine). */
  redactionPreview(docId: number, areas: RedactArea[]): Promise<AreaPreview[]>;
  /** What hidden information the document holds (read-only, runs in the engine). */
  sanitizeInspect(docId: number): Promise<SanitizeFindings>;
  /** The document's digital signatures, checked (read-only). */
  verifySignatures(docId: number): Promise<SignatureReport>;
  /** Editable text runs and pictures of a page. */
  pageObjects(docId: number, page: number): Promise<EditableObjects>;
  /** A page with pending object edits applied, as a PNG data URL (nothing is written). */
  previewPageEdits(docId: number, page: number, edits: PageEdit[], scalePermille: number): Promise<string>;
  /** Certificates the user trusts for signatures. */
  trustedCertificates(): Promise<TrustedCert[]>;
  /** The certificate chain of signature `index` (signer first). */
  signatureChain(docId: number, index: number): Promise<ChainCertificate[]>;
  /** Trust certificate `position` of signature `index`'s chain. */
  trustCertificate(docId: number, index: number, position: number): Promise<TrustedCert>;
  removeTrustedCertificate(fingerprint: string): Promise<boolean>;
  /** Files embedded in the document (read-only). */
  listAttachments(docId: number): Promise<AttachmentInfo[]>;
  /** Installed resources checked against their pinned checksums. */
  resourceStatus(): Promise<ResourceReport>;
  /** Comments and markup already in the document (read-only). */
  listAnnotations(docId: number): Promise<{ annotations: ExistingAnnotation[]; truncated: boolean }>;
  /** JPEG and PNG images. */
  pickImageFiles(): Promise<string[]>;
  pickOfficeFile(): Promise<string | null>;
  /** The LibreOffice package PageWeave can download (Windows). */
  officePackage(): Promise<OfficePackage>;
  /** The installed LibreOffice ("LibreOffice 24.2…"), or null. */
  officeEngine(): Promise<string | null>;
  /** This version and whether it can install updates itself. */
  updateInfo(): Promise<UpdateInfo>;
  /** A newer published version, or null when this is the newest. */
  checkForUpdate(): Promise<AvailableUpdate | null>;
  /** Run the downloaded update's installer and close PageWeave (it starts again afterwards). */
  installUpdate(version: string): Promise<void>;
  /** Open the project's releases page in the browser. */
  openReleasesPage(): Promise<void>;
  /** Translation directions and whether each model is installed. */
  translationDirections(): Promise<TranslationDirection[]>;
  /** The language of a document's first pages (null when unclear). */
  detectLanguage(docId: number): Promise<string | null>;
  /** The speech engine and its voices (fails with speech.unavailable when there is none). */
  speechInfo(): Promise<SpeechInfo>;
  /** One page's text in reading order with word positions. */
  readingText(docId: number, page: number): Promise<ReadingPage>;
  /** Speak a paragraph: audio plus timings. */
  speakText(text: string, voice: string | null, rate: number): Promise<Spoken>;
  /** A signing certificate file (.pfx/.p12). */
  pickCertificateFile(): Promise<string | null>;
  /** Any files (no type filter), e.g. to attach. */
  pickAnyFiles(): Promise<string[]>;
  /** A form data file to import (.xfdf, .xml, .json). */
  pickFormDataFile(): Promise<string | null>;
  /** A save location for a form data file. */
  pickFormDataSavePath(defaultPath: string, format: "xfdf" | "json"): Promise<string | null>;
  /** A save location for a file of any type. */
  pickSaveAnyPath(defaultPath: string): Promise<string | null>;
  launchFiles(): Promise<string[]>;
  launchFileDone(path: string): Promise<void>;
  /** Documents the host has open as tabs (restored after a page reload). */
  openDocuments(): Promise<OpenedDoc[]>;
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
  /** Jobs interrupted when the app last closed (returned once). */
  recoveredJobs(): Promise<RecoveredJob[]>;
  formFields(docId: number): Promise<FormInfo>;
}

export const tauriApi: Api = {
  appInfo: () => invoke("app_info"),
  getSettings: () => invoke("get_settings"),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  openDocument: (path, password, remember) => invoke("open_document", { path, password: password ?? null, remember: remember ?? null }),
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
  pickDesignExport: (defaultPath, format) =>
    saveDialog({ defaultPath, filters: [EXPORT_FILTERS[format]] }),
  pickDesignSource: async () => {
    const r = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "Pictures, PDFs and designs", extensions: ["png", "jpg", "jpeg", "webp", "tif", "tiff", "gif", "bmp", "pdf", "pwdesign"] }],
    });
    return typeof r === "string" ? r : null;
  },
  loadDesign: (path) => invoke("load_design", { path }),
  loadPicture: (path) => invoke("load_picture", { path }),
  cropPicture: (png, trim) => invoke("crop_picture", { png, left: trim[0], top: trim[1], right: trim[2], bottom: trim[3] }),
  saveDesign: (path, design) => invoke("save_design", { path, design }),
  pickFolder: async (defaultPath) => {
    const r = await openDialog({ directory: true, multiple: false, defaultPath });
    return typeof r === "string" ? r : null;
  },
  revealInFolder: (path) => revealItemInDir(path),
  redactionPreview: (docId, areas) => invoke("redaction_preview", { docId, areas }),
  sanitizeInspect: (docId) => invoke("sanitize_inspect", { docId }),
  listAttachments: (docId) => invoke("list_attachments", { docId }),
  listAnnotations: (docId) => invoke("list_annotations", { docId }),
  resourceStatus: () => invoke("resource_status"),
  trustedCertificates: () => invoke("trusted_certificates"),
  pageObjects: (docId, page) => invoke("page_objects", { docId, page }),
  previewPageEdits: (docId, page, edits, scalePermille) => invoke("preview_page_edits", { docId, page, edits, scalePermille }),
  signatureChain: (docId, index) => invoke("signature_chain", { docId, index }),
  trustCertificate: (docId, index, position) => invoke("trust_certificate", { docId, index, position }),
  removeTrustedCertificate: (fingerprint) => invoke("remove_trusted_certificate", { fingerprint }),
  pickImageFiles: async () => {
    const r = await openDialog({ multiple: true, directory: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "bmp"] }] });
    if (r === null) return [];
    return Array.isArray(r) ? r : [r];
  },
  pickOfficeFile: async () => {
    const r = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "Office documents", extensions: OFFICE_EXTENSIONS }],
    });
    return typeof r === "string" ? r : null;
  },
  officeEngine: () => invoke("office_engine"),
  officePackage: () => invoke("office_package"),
  speechInfo: () => invoke("speech_info"),
  updateInfo: () => invoke("update_info"),
  checkForUpdate: () => invoke("check_for_update"),
  installUpdate: (version) => invoke("install_update", { version }),
  openReleasesPage: () => invoke("open_releases_page"),
  translationDirections: () => invoke("translation_directions"),
  detectLanguage: (docId) => invoke("detect_language", { docId }),
  readingText: (docId, page) => invoke("reading_text", { docId, page }),
  speakText: (text, voice, rate) => invoke("speak_text", { text, voice, rate }),
  pickCertificateFile: async () => {
    const r = await openDialog({ multiple: false, directory: false, filters: [{ name: "Certificate files", extensions: ["pfx", "p12"] }] });
    return typeof r === "string" ? r : null;
  },
  pickAnyFiles: async () => {
    const r = await openDialog({ multiple: true, directory: false });
    if (r === null) return [];
    return Array.isArray(r) ? r : [r];
  },
  pickSaveAnyPath: (defaultPath) => saveDialog({ defaultPath }),
  pickFormDataFile: async () => {
    const r = await openDialog({ multiple: false, directory: false, filters: [{ name: "Form data", extensions: ["xfdf", "xml", "json"] }] });
    return typeof r === "string" ? r : null;
  },
  pickFormDataSavePath: (defaultPath, format) =>
    saveDialog({ defaultPath, filters: [format === "json" ? { name: "JSON form data", extensions: ["json"] } : { name: "XFDF form data", extensions: ["xfdf"] }] }),
  verifySignatures: (docId) => invoke("verify_signatures", { docId }),
  launchFiles: () => invoke("launch_files"),
  launchFileDone: (path) => invoke("launch_file_done", { path }),
  openDocuments: () => invoke("open_documents"),
  logDir: () => invoke("log_dir"),
  pageText: (docId, page) => invoke("page_text", { docId, page }),
  setUnsavedChanges: (count) => invoke("set_unsaved_changes", { count }),
  onCloseRequested: (cb) => listen("close-requested", () => cb()),
  quitApp: () => invoke("quit_app"),
  markTiming: (name) => invoke("mark_timing", { name }),
  recoveredJobs: () => invoke("recovered_jobs"),
  formFields: (docId) => invoke("form_fields", { docId }),
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

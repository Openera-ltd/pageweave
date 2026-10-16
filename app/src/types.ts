// Types mirroring the Rust contracts in crates/pw-core and crates/pw-pdf (serde JSON shapes).

export type ErrorCategory =
  | "invalid_input"
  | "unsupported_operation"
  | "unsupported_format"
  | "corrupt"
  | "password_required"
  | "permission"
  | "not_found"
  | "output_conflict"
  | "disk_full"
  | "missing_resource"
  | "corrupt_resource"
  | "network"
  | "cancelled"
  | "timeout"
  | "validation_failed"
  | "security_policy"
  | "engine_failure"
  | "internal";

export interface PwError {
  category: ErrorCategory;
  code: string;
  message: string;
  retryable: boolean;
  action: string;
}

export function isPwError(e: unknown): e is PwError {
  return typeof e === "object" && e !== null && "category" in e && "message" in e && "code" in e;
}

/** Normalize anything thrown by invoke() into a PwError so the UI never shows raw exceptions. */
export function toPwError(e: unknown): PwError {
  if (isPwError(e)) return e;
  return {
    category: "internal",
    code: "ui.unexpected",
    message: "Something went wrong.",
    retryable: false,
    action: "Try again. If it keeps happening, export diagnostics.",
  };
}

export interface PageInfo {
  index: number;
  width_pt: number;
  height_pt: number;
  rotation: 0 | 90 | 180 | 270;
  label: string | null;
}

export interface BookmarkInfo {
  title: string;
  page: number | null;
  depth: number;
}

export interface DocumentInfo {
  page_count: number;
  file_bytes: number;
  sha256: string;
  pdf_version: string;
  title: string | null;
  author: string | null;
  encrypted: boolean;
  has_xfa: boolean;
  pages: PageInfo[];
  bookmarks: BookmarkInfo[];
  warnings: string[];
}

export interface OpenedDoc {
  doc_id: number;
  path: string;
  name: string;
  info: DocumentInfo;
  already_open: boolean;
}

export interface SearchRect {
  left: number;
  bottom: number;
  right: number;
  top: number;
}

export interface SearchHit {
  page: number;
  text: string;
  rects: SearchRect[];
}

export type Theme = "system" | "light" | "dark";
export type Density = "compact" | "comfortable";
export type Motion = "system" | "reduced" | "full";

export interface Settings {
  version: number;
  theme: Theme;
  accent: string;
  density: Density;
  ui_scale: number;
  motion: Motion;
  recent_files: string[];
  shortcuts: Record<string, string>;
  restore_session: boolean;
}

export type CollisionPolicy = "fail" | "rename" | "replace";

export type SplitMode = { mode: "every_n"; n: number } | { mode: "ranges"; ranges: string[] };

export type AnnotationKind = "highlight" | "underline" | "strike_out" | "note";

export interface AnnotationSpec {
  /** 0-based page index. */
  page: number;
  kind: AnnotationKind;
  /** PDF points, unrotated page space. */
  rects: SearchRect[];
  /** `#rrggbb`. */
  color: string;
  contents?: string | null;
}

export type Operation =
  | { op: "extract_pages"; input: string; pages: string; output: string }
  | { op: "delete_pages"; input: string; pages: string; output: string }
  | { op: "rotate_pages"; input: string; pages: string | null; degrees: number; output: string }
  | { op: "merge"; inputs: string[]; output: string }
  | { op: "split"; input: string; split: SplitMode; output_dir: string }
  | { op: "annotate"; input: string; annotations: AnnotationSpec[]; output: string };

export interface OperationRequest {
  schema_version: number;
  operation: Operation;
  collision: CollisionPolicy;
}

export interface OutputInfo {
  path: string;
  bytes: number;
  pages: number | null;
}

export interface OperationResult {
  operation: string;
  outputs: OutputInfo[];
  warnings: string[];
  duration_ms: number;
}

export type JobState = "queued" | "running" | "cancelling" | "cancelled" | "succeeded" | "failed";

export type Progress = { kind: "indeterminate" } | { kind: "determinate"; done: number; total: number; unit: string };

export interface JobSnapshot {
  id: string;
  label: string;
  state: JobState;
  phase: string;
  progress: Progress;
  result: OperationResult | null;
  error: PwError | null;
  warnings: string[];
  elapsed_ms: number;
}

export interface AppInfo {
  version: string;
  engine: string;
  schema_version: number;
  settings_status: { status: string; [k: string]: unknown };
  engine_ready: boolean;
  engine_error: PwError | null;
  platform: string;
}

export interface TextSegment {
  text: string;
  left: number;
  bottom: number;
  right: number;
  top: number;
}

export interface PageText {
  page: number;
  segments: TextSegment[];
  truncated: boolean;
}

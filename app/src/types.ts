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
  /** Lower-left corner of the displayed page box in PDF user space (0,0 for most files). */
  origin_x?: number;
  origin_y?: number;
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
  subject?: string | null;
  keywords?: string | null;
  creator?: string | null;
  producer?: string | null;
  signature_count?: number;
  encrypted: boolean;
  has_xfa: boolean;
  /** "none", "acrobat", "xfa_full", "xfa_foreground". */
  form_type?: string;
  pages: PageInfo[];
  bookmarks: BookmarkInfo[];
  warnings: string[];
}

export type SignatureStatus = "valid" | "invalid" | "unsupported" | "unreadable";

export interface SignatureInfo {
  field: string | null;
  page: number | null;
  status: SignatureStatus;
  detail: string;
  signer: string | null;
  signer_subject: string | null;
  issuer: string | null;
  certificate_valid_from: string | null;
  certificate_valid_to: string | null;
  self_signed: boolean;
  signing_time: string | null;
  reason: string | null;
  location: string | null;
  sub_filter: string | null;
  digest_algorithm: string | null;
  signature_algorithm: string | null;
  covers_whole_file: boolean;
  changed_after: boolean;
  has_timestamp: boolean;
  trust: "not_checked";
}

export interface SignatureReport {
  signatures: SignatureInfo[];
  trust_note: string;
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

export type StampPosition = "top_left" | "top_center" | "top_right" | "bottom_left" | "bottom_center" | "bottom_right";

export type FieldKind = "text" | "checkbox" | "radio" | "combo" | "list" | "push_button" | "signature" | "unknown";

export interface FormWidget {
  page: number;
  rect: SearchRect;
  export_value: string | null;
  checked: boolean;
}

export interface FormField {
  name: string;
  label: string | null;
  kind: FieldKind;
  value: string | null;
  options: string[];
  read_only: boolean;
  required: boolean;
  multiline: boolean;
  editable: boolean;
  widgets: FormWidget[];
}

export interface FormInfo {
  fields: FormField[];
  has_xfa: boolean;
  truncated: boolean;
}

/** true/false for checkboxes; text for text, choice and radio (export value) fields. */
export type FormValue = boolean | string;

export interface FieldValue {
  name: string;
  value: FormValue;
}

export type Operation =
  | { op: "extract_pages"; input: string; pages: string; output: string }
  | { op: "delete_pages"; input: string; pages: string; output: string }
  | { op: "rotate_pages"; input: string; pages: string | null; degrees: number; output: string }
  | { op: "merge"; inputs: string[]; output: string }
  | { op: "split"; input: string; split: SplitMode; output_dir: string }
  | { op: "annotate"; input: string; annotations: AnnotationSpec[]; output: string }
  | { op: "apply_edits"; input: string; form_values: FieldValue[]; annotations: AnnotationSpec[]; output: string }
  | {
      op: "stamp_text";
      input: string;
      pages: string | null;
      stamps: Array<{ text: string; position: StampPosition; font_size: number; color: string; margin: number }>;
      start: number;
      digits: number;
      output: string;
    }
  | { op: "export_images"; input: string; pages: string | null; dpi: number; output_dir: string }
  | { op: "organize_pages"; input: string; extra_inputs: string[]; pages: Array<{ source: number; page: number; rotate: number; blank?: boolean }>; output: string }
  | { op: "redact"; input: string; areas: RedactArea[]; output: string }
  | { op: "ocr_pages"; input: string; pages: string | null; skip_text_pages: boolean; dpi: number; output: string }
  | { op: "protect"; input: string; permissions: ProtectPermissions; output: string }
  | { op: "unprotect"; input: string; output: string }
  | { op: "images_to_pdf"; inputs: string[]; page_size: ImagePageSize; margin: number; output: string }
  | { op: "set_bookmarks"; input: string; bookmarks: Array<{ title: string; page: number; depth: number }>; output: string }
  | { op: "extract_attachment"; input: string; index: number; output: string }
  | { op: "add_attachments"; input: string; files: string[]; output: string }
  | { op: "compare"; old: string; new: string }
  | { op: "verify_signatures"; input: string }
  | { op: "deskew"; input: string; pages: string | null; output: string }
  | { op: "fix_orientation"; input: string; pages: string | null; output: string }
  | {
      op: "sign";
      input: string;
      certificate: string;
      reason: string | null;
      location: string | null;
      placement: { page: number; rect: [number, number, number, number] } | null;
      output: string;
    }
  | { op: "find_blank_pages"; input: string; max_ink: number }
  | { op: "extract_images"; input: string; pages: string | null; output_dir: string }
  | { op: "optimize"; input: string; images: { max_dpi: number; quality: number } | null; output: string }
  | { op: "export_text"; input: string; pages: string | null; output: string }
  | { op: "crop_pages"; input: string; pages: string | null; margins: { top: number; right: number; bottom: number; left: number }; output: string }
  | { op: "watermark"; input: string; pages: string | null; watermark: WatermarkSpec; output: string }
  | { op: "set_properties"; input: string; title?: string; author?: string; subject?: string; keywords?: string; output: string }
  | { op: "sanitize"; input: string; options: SanitizeOptions; output: string };

export type ImagePageSize = "a4" | "letter" | "image";

/** A file embedded in the document (engine `list_attachments`). */
export interface AttachmentInfo {
  index: number;
  name: string;
  bytes: number;
}

export type WatermarkLayer = "over" | "behind";

/** A text watermark centred on each page (pw-core `WatermarkSpec`). */
export interface WatermarkSpec {
  text: string;
  font_size: number;
  color: string;
  /** 0.05–1. */
  opacity: number;
  /** Degrees counter-clockwise as displayed, −90 to 90. */
  angle: number;
  layer: WatermarkLayer;
}

/** What a sanitized copy leaves out (pw-core `SanitizeOptions`). */
export interface SanitizeOptions {
  metadata: boolean;
  scripts: boolean;
  attachments: boolean;
  hidden_layers: boolean;
  comments: boolean;
  form_fields: boolean;
  web_links: boolean;
}

/** Hidden information found in a file (engine `sanitize_inspect`). */
export interface SanitizeFindings {
  metadata: string[];
  scripts: number;
  attachments: number;
  attachment_names: string[];
  layers: number;
  hidden_layers: string[];
  comments: number;
  form_fields: number;
  web_links: number;
  earlier_versions: boolean;
}

/** An area to redact, in PDF user space of a 0-based page. */
export interface RedactArea {
  page: number;
  rect: SearchRect;
}

/** What redacting one area would remove (engine `redaction_preview`). */
export interface AreaPreview {
  page: number;
  text: string;
  images: number;
  graphics: number;
  /** Drawings crossing the area's edge: covered by the black box but kept in the file. */
  graphics_kept: number;
  annotations: number;
}

export interface RecoveredJob {
  label: string;
  operation: Operation;
  collision: CollisionPolicy;
  removed_partial_files: number;
  leftover_partial_files: string[];
}

export interface OperationRequest {
  schema_version: number;
  operation: Operation;
  collision: CollisionPolicy;
  /** `protect` only. Sent to the engine but never stored or logged by it. */
  new_password?: string;
  /** For `sign`; never serialized by the host. */
  certificate_password?: string;
  new_owner_password?: string | null;
}

export interface ProtectPermissions {
  print: boolean;
  copy: boolean;
  modify: boolean;
  annotate: boolean;
  fill_forms: boolean;
  assemble: boolean;
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
  /** Operation-specific details (for example sizes before and after `optimize`). */
  data?: Record<string, unknown>;
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

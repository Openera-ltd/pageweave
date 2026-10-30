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
  trust: TrustStatus;
  trust_detail: string;
  trusted_by: string | null;
  chain: string[];
  revocation: "not_checked";
}

export type TrustStatus = "not_checked" | "trusted" | "unknown_signer" | "expired" | "chain_invalid";

/** One certificate of a signature's chain, offered for trusting. */
export interface ChainCertificate {
  name: string;
  subject: string;
  fingerprint: string;
  valid_to: string;
  is_ca: boolean;
  self_signed: boolean;
}

/** A certificate the user trusts for signatures. */
export interface TrustedCert {
  fingerprint: string;
  name: string;
  subject: string;
  valid_to: string;
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
  /** Read aloud voice id (system default when null) and speed (0.5–2). */
  read_voice: string | null;
  read_rate: number;
  check_updates: boolean;
}

export type CollisionPolicy = "fail" | "rename" | "replace";

export type SplitMode =
  | { mode: "every_n"; n: number }
  | { mode: "ranges"; ranges: string[] }
  | { mode: "bookmarks"; level: number }
  | { mode: "max_size"; max_bytes: number };

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
  /** Value "Reset form" restores, when the document sets one. */
  default_value?: string | null;
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

/** Installed resources (engine `resources::resource_status`). File names are diagnostics only. */
export interface ResourceReport {
  all_ready: boolean;
  capabilities: Array<{
    id: string;
    title: string;
    state: "ready" | "missing" | "damaged";
    detail: string;
    bytes: number;
    location: string | null;
    files: Array<{ name: string; state: "ready" | "missing" | "damaged"; bytes: number }>;
    required: boolean;
  }>;
}

/** A comment or markup already in the file (engine `list_annotations`). */
export interface ExistingAnnotation {
  page: number;
  index: number;
  subtype: string;
  rect: { left: number; bottom: number; right: number; top: number };
  text: string | null;
  author: string | null;
  modified: string | null;
  color: string | null;
  text_editable: boolean;
}

/** A pending change to an annotation already in the file (pw-core `AnnotationChange`). */
export interface AnnotationChange {
  page: number;
  index: number;
  subtype: string;
  delete: boolean;
  text: string | null;
}

export type Operation =
  | { op: "extract_pages"; input: string; pages: string; output: string }
  | { op: "delete_pages"; input: string; pages: string; output: string }
  | { op: "rotate_pages"; input: string; pages: string | null; degrees: number; output: string }
  | { op: "merge"; inputs: string[]; output: string }
  | { op: "split"; input: string; split: SplitMode; output_dir: string }
  | { op: "annotate"; input: string; annotations: AnnotationSpec[]; output: string }
  | { op: "export_form_data"; input: string; format: "xfdf" | "json"; values: FieldValue[]; output: string }
  | { op: "read_form_data"; input: string; data: string }
  | { op: "check_accessibility"; input: string }
  | { op: "set_page_labels"; input: string; ranges: Array<{ start: number; style: string; prefix: string; first: number }>; output: string }
  | { op: "find_patterns"; input: string; patterns: Array<"email" | "phone" | "card_number" | "iban" | "us_ssn">; custom: string | null; pages: string | null }
  | { op: "import_form_data"; input: string; data: string; output: string }
  | { op: "apply_edits"; input: string; form_values: FieldValue[]; annotations: AnnotationSpec[]; object_edits?: Array<{ page: number; edits: PageEdit[] }>; annotation_changes?: AnnotationChange[]; output: string }
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
  | { op: "verify_signatures"; input: string; trusted?: string[] }
  | { op: "deskew"; input: string; pages: string | null; output: string }
  | { op: "fix_orientation"; input: string; pages: string | null; output: string }
  | {
      op: "add_signature_mark";
      input: string;
      page: number;
      rect: { left: number; bottom: number; right: number; top: number };
      mark:
        | { kind: "typed"; text: string; style: "formal" | "plain" | "typewriter"; color: string }
        | { kind: "drawn"; strokes: Array<Array<[number, number]>>; color: string; width: number }
        | { kind: "image"; path: string };
      output: string;
    }
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
  | { op: "reconstruct_design"; input: string; page: number; output: string }
  | { op: "export_design"; input: string; format: DesignFormat; output: string }
  | { op: "convert_to_docx"; input: string; pages: string | null; output: string }
  | { op: "install_translation"; source: string; target: string }
  | { op: "download_update"; version: string }
  | { op: "install_office" }
  | { op: "translate_document"; input: string; pages: string | null; source: string | null; target: string; output: string }
  | { op: "read_aloud_to_wav"; input: string; pages: string | null; voice: string | null; rate: number; output: string }
  | { op: "convert_to_pptx"; input: string; pages: string | null; output: string }
  | { op: "convert_office_to_pdf"; input: string; output: string }
  | { op: "export_tables"; input: string; pages: string | null; format: "xlsx" | "csv"; output: string }
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

export interface OfficePackage {
  version: string;
  bytes: number;
  /** PageWeave can download and unpack it (Windows). */
  can_install: boolean;
}

export interface UpdateInfo {
  current: string;
  /** This build carries the release key. */
  can_install: boolean;
  /** Updates install on Windows only. */
  platform_supported: boolean;
}

export interface AvailableUpdate {
  version: string;
  current: string;
  notes: string;
  pub_date: string | null;
  bytes: number;
  /** Already downloaded and verified. */
  downloaded: boolean;
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

/** A change to existing page content (engine `PageEdit`). Moves are in points as displayed. */
export type PageEdit =
  | { kind: "text"; objects: number[]; text: string; size: number | null; color: string | null; dx: number; dy: number }
  | { kind: "delete_objects"; objects: number[] }
  | { kind: "move_objects"; objects: number[]; dx: number; dy: number }
  | { kind: "replace_image"; object: number; path: string }
  | { kind: "rotate_image"; object: number; degrees: 90 | 180 | 270 }
  | { kind: "add_text"; x: number; y: number; text: string; size: number; color: string };

export interface PdfRect {
  left: number;
  bottom: number;
  right: number;
  top: number;
}

export interface EditableText {
  objects: number[];
  text: string;
  rect: PdfRect;
  font: string;
  size: number;
  color: string;
  embedded: boolean;
  bold: boolean;
  italic: boolean;
  locked: string | null;
}

export interface EditableImage {
  object: number;
  rect: PdfRect;
  width_px: number;
  height_px: number;
}

export interface EditableObjects {
  page: number;
  rotation: number;
  texts: EditableText[];
  images: EditableImage[];
}

/** The computer's speech engine (read aloud). */
export interface SpeechVoice {
  id: string;
  name: string;
  language: string;
  default: boolean;
}

export interface SpeechInfo {
  name: string;
  /** Whether highlighting follows words or whole sentences. */
  precision: "word" | "sentence";
  voices: SpeechVoice[];
}

/** One page's text in reading order; offsets count Unicode characters (use Array.from). */
export interface ReadingWord {
  start: number;
  end: number;
  rects: SearchRect[];
}

export interface ReadingPage {
  page: number;
  text: string;
  words: ReadingWord[];
}

/** A stretch of spoken text (character offsets) and when it is heard (ms from the start). */
export interface SpeechMark {
  start: number;
  end: number;
  start_ms: number;
  end_ms: number;
}

export interface Spoken {
  /** WAV data URL. */
  audio: string;
  duration_ms: number;
  sentences: SpeechMark[];
  words: SpeechMark[];
  precision: "word" | "sentence";
}

/** A translation direction PageWeave offers (BCP 47 codes). */
export interface TranslationDirection {
  source: string;
  target: string;
  installed: boolean;
  /** Download size (bytes). */
  bytes: number;
}

/** Design Reconstruction Studio (D-028): one editable object, in design pixels from the top left. */
export interface DesignObject {
  id: string;
  name: string;
  kind: "text" | "shape" | "image";
  x: number;
  y: number;
  w: number;
  h: number;
  visible: boolean;
  locked: boolean;
  /** 0–1: recognition confidence (text), colour uniformity (shape); pictures are 0. */
  confidence: number;
  note?: string | null;
  text?: string;
  color?: string;
  font_size?: number;
  bold: boolean;
  /** Text baseline below the top of the box. */
  baseline?: number;
  fill?: string | null;
  /** PNG, base64. */
  png?: string;
  /** Objects with the same group id are selected and moved together. */
  group?: string | null;
  /** Clockwise turn in degrees around the box's centre. */
  rotation?: number;
  /** Shape: an ellipse filling the box. */
  ellipse?: boolean;
  /** Shape: corner radius in design pixels. */
  radius?: number;
  /** Shape: outline colour and width (drawn inside the box); `fill` absent = no fill. */
  stroke?: string | null;
  stroke_width?: number;
  /** Text: typeface family (absent: sans) and slant. */
  font?: DesignFont;
  italic?: boolean;
  /** Text with several lines ("\n"): baseline distance (absent: 1.2 × size), alignment and the
   *  glyphs' horizontal scale measured from the original (absent: 1). */
  line_height?: number;
  align?: TextAlign;
  stretch?: number;
}

export type TextAlign = "left" | "center" | "right";

export type DesignFont = "sans" | "serif" | "mono";
export type DesignFormat = "svg" | "pdf" | "png";

export interface Design {
  version: number;
  width: number;
  height: number;
  pt_per_px: number;
  background: string;
  source_png: string;
  source_name: string;
  /** Back to front. */
  objects: DesignObject[];
}

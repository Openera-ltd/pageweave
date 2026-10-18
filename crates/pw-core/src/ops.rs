//! Versioned, typed operation contract (PRD 04 "Boundaries and contracts", PRD 17).
//!
//! Every interface — desktop UI, CLI, local API, MCP — builds an [`OperationRequest`] and hands it to
//! the same engine. Validation here is structural (shape, bounds); the engine validates against the
//! actual document (page counts, encryption, …).

use crate::error::{PwError, PwResult};
use crate::output::CollisionPolicy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_MERGE_INPUTS: usize = 500;
pub const MAX_QUERY_CHARS: usize = 1000;
pub const MIN_RENDER_SCALE: f32 = 0.05;
pub const MAX_RENDER_SCALE: f32 = 8.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SplitMode {
    /// One output per `n` pages.
    EveryN { n: u32 },
    /// One output per range expression, e.g. `["1-3", "4-", "last"]`.
    Ranges { ranges: Vec<String> },
}

pub const MAX_ANNOTATIONS_PER_REQUEST: usize = 10_000;
pub const MAX_NOTE_CHARS: usize = 10_000;
pub const MAX_FORM_VALUES: usize = 10_000;
pub const MAX_ORGANIZE_PAGES: usize = 100_000;
pub const MAX_STAMPS: usize = 6;
pub const MAX_STAMP_CHARS: usize = 200;
pub const MAX_REDACT_AREAS: usize = 10_000;
/// Page size for images turned into PDF pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImagePageSize {
    #[default]
    A4,
    Letter,
    /// The image's own size at its recorded resolution (96 dpi when none is recorded).
    Image,
}

/// Most images per new PDF.
pub const MAX_IMAGES_TO_PDF: usize = 2_000;

/// Most bookmarks in one outline, longest title, deepest nesting.
pub const MAX_BOOKMARKS: usize = 10_000;
pub const MAX_BOOKMARK_TITLE: usize = 500;
pub const MAX_BOOKMARK_DEPTH: u32 = 32;

/// One bookmark in an outline given as a flat list in reading order; `depth` 0 is top level and a
/// bookmark is a child of the closest earlier one with a smaller depth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkSpec {
    pub title: String,
    /// 0-based page index.
    pub page: u32,
    #[serde(default)]
    pub depth: u32,
}

/// Most files attached in one request, and the largest single attachment.
pub const MAX_ATTACH_FILES: usize = 100;
pub const MAX_ATTACH_BYTES: u64 = 256 * 1024 * 1024;
/// Longest document property value (title, author, subject, keywords), in characters.
pub const MAX_PROPERTY_CHARS: usize = 2_000;

/// Where a stamp sits on the page as displayed (after the page's own rotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StampPosition {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

fn default_font_size() -> f32 {
    10.0
}
fn default_margin() -> f32 {
    28.0
}
fn default_color() -> String {
    "#000000".into()
}
fn default_start() -> u64 {
    1
}

fn default_true() -> bool {
    true
}

pub const DEFAULT_OCR_DPI: u32 = 200;

fn default_ocr_dpi() -> u32 {
    DEFAULT_OCR_DPI
}

/// Text placed on pages. `text` may contain `{page}` (running number over the stamped pages,
/// starting at the operation's `start`, zero-padded to `digits`) and `{total}` (pages in the document).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StampSpec {
    pub text: String,
    pub position: StampPosition,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_color")]
    pub color: String,
    /// Distance from the page edges, in points.
    #[serde(default = "default_margin")]
    pub margin: f32,
}

/// Expand a stamp template for one page.
pub fn stamp_text_for(template: &str, number: u64, digits: u8, total: u32) -> String {
    let n = format!("{number:0width$}", width = digits as usize);
    template.replace("{page}", &n).replace("{total}", &total.to_string())
}

/// One page of an organized output: page `page` (0-based) of source `source` (0 = the input,
/// 1.. = `extra_inputs`), turned clockwise by `rotate` degrees relative to its current rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRef {
    #[serde(default)]
    pub source: u32,
    pub page: u32,
    #[serde(default)]
    pub rotate: i32,
    /// A new empty page with the size and orientation of `source`/`page`, instead of a copy of it.
    #[serde(default)]
    pub blank: bool,
}
pub const MAX_FORM_TEXT_CHARS: usize = 100_000;

/// A value to put into a form field. Text fields, combo and list boxes take text (a choice must be
/// one of the field's options unless the combo box is editable); a radio group takes the export
/// value of the button to select; a checkbox takes true/false.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormValue {
    Checked(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldValue {
    /// Fully qualified field name (e.g. `address.city`).
    pub name: String,
    pub value: FormValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Highlight,
    Underline,
    StrikeOut,
    /// A sticky note; positioned at the first rectangle's top-left corner.
    Note,
}

/// Rectangle in PDF points in unrotated page space (origin bottom-left).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RectPt {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

/// What readers that honour PDF permissions allow once the file is opened with the open password.
/// Permissions are not security: they rely on the reader's cooperation (PRD 05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectPermissions {
    #[serde(default = "default_true")]
    pub print: bool,
    #[serde(default = "default_true")]
    pub copy: bool,
    #[serde(default = "default_true")]
    pub modify: bool,
    #[serde(default = "default_true")]
    pub annotate: bool,
    #[serde(default = "default_true")]
    pub fill_forms: bool,
    #[serde(default = "default_true")]
    pub assemble: bool,
}

impl Default for ProtectPermissions {
    fn default() -> Self {
        ProtectPermissions { print: true, copy: true, modify: true, annotate: true, fill_forms: true, assemble: true }
    }
}

/// What a sanitized copy leaves out (PRD 05 "sanitization"). Earlier saved versions of the file
/// (incremental updates) are always removed, because the copy is written from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizeOptions {
    /// Document properties, XMP metadata, private application data and page thumbnails.
    #[serde(default = "default_true")]
    pub metadata: bool,
    /// JavaScript, automatic actions, and actions that launch programs, open other files, submit or
    /// import data; also XFA form scripts. Links to pages and websites stay.
    #[serde(default = "default_true")]
    pub scripts: bool,
    /// Embedded files, file attachments and embedded media.
    #[serde(default = "default_true")]
    pub attachments: bool,
    /// Content in layers that are hidden; the remaining layers become ordinary page content.
    #[serde(default = "default_true")]
    pub hidden_layers: bool,
    /// Comments and markups (notes, highlights, drawings, stamps), including pending redaction marks.
    #[serde(default = "default_true")]
    pub comments: bool,
    /// Form fields and the data typed into them.
    #[serde(default)]
    pub form_fields: bool,
    /// Links to websites.
    #[serde(default)]
    pub web_links: bool,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        SanitizeOptions {
            metadata: true,
            scripts: true,
            attachments: true,
            hidden_layers: true,
            comments: true,
            form_fields: false,
            web_links: false,
        }
    }
}

/// Lossy image reduction for `optimize`: images shown above `max_dpi` are resampled to it and
/// stored as JPEG at `quality` (30–95, higher is better).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageReduce {
    pub max_dpi: u32,
    pub quality: u8,
}

/// Default ink share under which a page counts as blank (0.2%: dust and scanner noise).
pub const DEFAULT_MAX_INK: f32 = 0.002;

fn default_max_ink() -> f32 {
    DEFAULT_MAX_INK
}

pub const MIN_IMAGE_DPI: u32 = 36;
pub const MAX_IMAGE_DPI: u32 = 1200;
pub const MIN_JPEG_QUALITY: u8 = 30;
pub const MAX_JPEG_QUALITY: u8 = 95;

/// Margins to cut from each side of a page, in points, as the page is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Margins {
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
    #[serde(default)]
    pub left: f32,
}

/// Where a watermark goes relative to the page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkLayer {
    /// Drawn over the content (always visible).
    #[default]
    Over,
    /// Drawn behind the content (hidden where the page is covered, e.g. by a scanned image).
    Behind,
}

fn default_watermark_size() -> f32 {
    72.0
}
fn default_watermark_color() -> String {
    "#808080".into()
}
fn default_watermark_opacity() -> f32 {
    0.25
}
fn default_watermark_angle() -> f32 {
    45.0
}

/// A text watermark, centred on each page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatermarkSpec {
    pub text: String,
    #[serde(default = "default_watermark_size")]
    pub font_size: f32,
    /// `#rrggbb`.
    #[serde(default = "default_watermark_color")]
    pub color: String,
    /// 0.05–1.
    #[serde(default = "default_watermark_opacity")]
    pub opacity: f32,
    /// Degrees counter-clockwise, −90 to 90, as the page is displayed.
    #[serde(default = "default_watermark_angle")]
    pub angle: f32,
    #[serde(default)]
    pub layer: WatermarkLayer,
}

/// Lettering of a typed signature (standard PDF fonts, Latin-1 text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypedStyle {
    /// Times Italic.
    #[default]
    Formal,
    /// Helvetica Oblique.
    Plain,
    /// Courier Oblique.
    Typewriter,
}

fn default_ink() -> String {
    "#1a2b5c".into()
}

fn default_pen() -> f32 {
    1.6
}

/// A visual signature or initials (a picture of a signature, not a digital signature).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignatureMark {
    Typed {
        text: String,
        #[serde(default)]
        style: TypedStyle,
        #[serde(default = "default_ink")]
        color: String,
    },
    /// Strokes as drawn: points 0..1 across the box as displayed, y downwards.
    Drawn {
        strokes: Vec<Vec<[f32; 2]>>,
        #[serde(default = "default_ink")]
        color: String,
        /// Pen width in points.
        #[serde(default = "default_pen")]
        width: f32,
    },
    /// A PNG or JPEG picture of a signature.
    Image { path: PathBuf },
}

/// Where a visible signature is drawn: 0-based page and a box [x1, y1, x2, y2] in that page's
/// unrotated PDF space (points).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SignPlacement {
    pub page: u32,
    pub rect: [f32; 4],
}

/// An area to redact: everything under it is removed and it is painted black.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RedactArea {
    /// 0-based page index.
    pub page: u32,
    pub rect: RectPt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationSpec {
    /// 0-based page index.
    pub page: u32,
    pub kind: AnnotationKind,
    pub rects: Vec<RectPt>,
    /// `#rrggbb`.
    pub color: String,
    #[serde(default)]
    pub contents: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Inspect {
        input: PathBuf,
    },
    /// List the document's AcroForm fields (read-only).
    FormFields {
        input: PathBuf,
    },
    ExtractText {
        input: PathBuf,
        pages: Option<String>,
    },
    Search {
        input: PathBuf,
        query: String,
        #[serde(default)]
        match_case: bool,
        #[serde(default)]
        whole_word: bool,
    },
    Render {
        input: PathBuf,
        page: u32,
        scale: f32,
        output: PathBuf,
    },
    Merge {
        inputs: Vec<PathBuf>,
        output: PathBuf,
    },
    ExtractPages {
        input: PathBuf,
        pages: String,
        output: PathBuf,
    },
    DeletePages {
        input: PathBuf,
        pages: String,
        output: PathBuf,
    },
    RotatePages {
        input: PathBuf,
        pages: Option<String>,
        degrees: i32,
        output: PathBuf,
    },
    Split {
        input: PathBuf,
        split: SplitMode,
        output_dir: PathBuf,
    },
    /// Add annotations and write the result to a new file.
    Annotate {
        input: PathBuf,
        annotations: Vec<AnnotationSpec>,
        output: PathBuf,
    },
    /// Fill AcroForm fields and write the result to a new file.
    FillForm {
        input: PathBuf,
        values: Vec<FieldValue>,
        output: PathBuf,
    },
    /// Build a new file from pages in a chosen order: reorder, duplicate, drop, rotate, and insert
    /// pages from other files in one step.
    OrganizePages {
        input: PathBuf,
        #[serde(default)]
        extra_inputs: Vec<PathBuf>,
        pages: Vec<PageRef>,
        output: PathBuf,
    },
    /// Save pages as PNG images (`<name>_p001.png`, …) in a folder.
    ExportImages {
        input: PathBuf,
        /// Pages to export (range syntax); all pages when absent.
        #[serde(default)]
        pages: Option<String>,
        /// Resolution in dots per inch (72 = 100%).
        dpi: u32,
        output_dir: PathBuf,
    },
    /// Add page numbers, headers/footers or Bates-style numbers to a copy.
    StampText {
        input: PathBuf,
        /// Pages to stamp (range syntax); all pages when absent.
        #[serde(default)]
        pages: Option<String>,
        stamps: Vec<StampSpec>,
        #[serde(default = "default_start")]
        start: u64,
        #[serde(default)]
        digits: u8,
        output: PathBuf,
    },
    /// Encrypt a copy with AES-256: an open password (from the request, never serialized) and
    /// permissions for readers that honour them.
    Protect {
        input: PathBuf,
        #[serde(default)]
        permissions: ProtectPermissions,
        output: PathBuf,
    },
    /// Save the text of pages (all when `pages` is absent) as a UTF-8 text file, one block per page.
    ExportText {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        output: PathBuf,
    },
    /// Make a new PDF from JPEG and PNG images, one image per page in the given order.
    ImagesToPdf {
        inputs: Vec<PathBuf>,
        #[serde(default)]
        page_size: ImagePageSize,
        /// Space around each image, in points (0–144).
        #[serde(default)]
        margin: f32,
        output: PathBuf,
    },
    /// Replace the document's bookmarks (outline) in a new file. An empty list removes them.
    SetBookmarks {
        input: PathBuf,
        bookmarks: Vec<BookmarkSpec>,
        output: PathBuf,
    },
    /// List the files embedded in the document (read-only).
    ListAttachments {
        input: PathBuf,
    },
    /// Save one embedded file (by its index in `ListAttachments`) to `output`. The bytes are written
    /// as they are; nothing is opened or run.
    ExtractAttachment {
        input: PathBuf,
        index: u32,
        output: PathBuf,
    },
    /// Embed files in a new copy of the document.
    AddAttachments {
        input: PathBuf,
        files: Vec<PathBuf>,
        output: PathBuf,
    },
    /// Crop pages (all when `pages` is absent) by margins as displayed, in a new file. Content outside
    /// the new page area is hidden, not removed.
    CropPages {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        margins: Margins,
        output: PathBuf,
    },
    /// Add a text watermark to pages (all when `pages` is absent) in a new file.
    Watermark {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        watermark: WatermarkSpec,
        output: PathBuf,
    },
    /// Change document properties in a new file. `None` keeps a property, an empty string removes it.
    SetProperties {
        input: PathBuf,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        subject: Option<String>,
        #[serde(default)]
        keywords: Option<String>,
        output: PathBuf,
    },
    /// Save a copy without password protection. The file must be opened with a password that
    /// grants full rights: lifting an owner's restrictions needs the permissions password.
    Unprotect {
        input: PathBuf,
        output: PathBuf,
    },
    /// Recognise text on page images and add it as an invisible, searchable text layer in a new
    /// file; the pages look exactly as before.
    OcrPages {
        input: PathBuf,
        /// Pages to process (range syntax); all pages when absent.
        #[serde(default)]
        pages: Option<String>,
        /// Leave pages that already have text untouched.
        #[serde(default = "default_true")]
        skip_text_pages: bool,
        /// Rendering resolution for recognition (dpi).
        #[serde(default = "default_ocr_dpi")]
        dpi: u32,
        output: PathBuf,
    },
    /// What hidden information the file holds, per sanitize category (read-only).
    SanitizeInspect {
        input: PathBuf,
    },
    /// Write a copy without the chosen kinds of hidden information, verified independently.
    Sanitize {
        input: PathBuf,
        #[serde(default)]
        options: SanitizeOptions,
        output: PathBuf,
    },
    /// A smaller copy with the same content: lossless recompression, shared duplicates, unused
    /// objects and earlier versions dropped; verified to look and read the same.
    Optimize {
        input: PathBuf,
        /// Also resample high-resolution images (lossy; off unless given).
        #[serde(default)]
        images: Option<ImageReduce>,
        output: PathBuf,
    },
    /// Save the images embedded in the chosen (or all) pages into a folder: JPEG as stored, others
    /// as PNG; repeated images once.
    ExtractImages {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        output_dir: PathBuf,
    },
    /// Which pages are blank (read-only): no visible text or annotations and an ink share of at
    /// most `max_ink` (0–0.05).
    FindBlankPages {
        input: PathBuf,
        #[serde(default = "default_max_ink")]
        max_ink: f32,
    },
    /// Place a visual signature or initials (typed, drawn or a picture) in a box on a page of a copy.
    AddSignatureMark {
        input: PathBuf,
        /// 0-based page.
        page: u32,
        /// Box in the page's unrotated PDF space; the mark is upright as the page is displayed.
        rect: RectPt,
        mark: SignatureMark,
        output: PathBuf,
    },
    /// Sign a copy with a certificate (PKCS#12 identity; its password travels in
    /// `OperationRequest::certificate_password`, never serialized). Invisible when `placement` is
    /// absent; otherwise drawn in a box on that page (unrotated PDF points).
    Sign {
        input: PathBuf,
        certificate: PathBuf,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        location: Option<String>,
        #[serde(default)]
        placement: Option<SignPlacement>,
        output: PathBuf,
    },
    /// Turn scanned pages that face sideways or upside down upright (by /Rotate) in a new file.
    FixOrientation {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        output: PathBuf,
    },
    /// Straighten skewed scanned pages (all, or the chosen ones) in a new file.
    Deskew {
        input: PathBuf,
        #[serde(default)]
        pages: Option<String>,
        output: PathBuf,
    },
    /// Check every digital signature in the document (read-only).
    VerifySignatures {
        input: PathBuf,
    },
    /// Differences between two documents (read-only): aligned pages, text, format and visual changes.
    Compare {
        old: PathBuf,
        new: PathBuf,
    },
    /// What redacting `areas` would remove (read-only; shown before applying).
    RedactionPreview {
        input: PathBuf,
        areas: Vec<RedactArea>,
    },
    /// Permanently remove the content under `areas` and write a verified new file.
    Redact {
        input: PathBuf,
        areas: Vec<RedactArea>,
        output: PathBuf,
    },
    /// Apply pending edits (form values and/or annotations) to one new file.
    ApplyEdits {
        input: PathBuf,
        #[serde(default)]
        form_values: Vec<FieldValue>,
        #[serde(default)]
        annotations: Vec<AnnotationSpec>,
        output: PathBuf,
    },
}

/// Every operation id (`Operation::id`), for capability listings; a test keeps it in step with the enum.
pub const OPERATION_IDS: &[&str] = &[
    "inspect",
    "form_fields",
    "extract_text",
    "search",
    "render",
    "merge",
    "extract_pages",
    "delete_pages",
    "rotate_pages",
    "split",
    "annotate",
    "fill_form",
    "apply_edits",
    "organize_pages",
    "stamp_text",
    "export_images",
    "redaction_preview",
    "ocr_pages",
    "protect",
    "unprotect",
    "set_properties",
    "watermark",
    "crop_pages",
    "set_bookmarks",
    "images_to_pdf",
    "export_text",
    "optimize",
    "compare",
    "verify_signatures",
    "deskew",
    "fix_orientation",
    "sign",
    "add_signature_mark",
    "find_blank_pages",
    "extract_images",
    "list_attachments",
    "extract_attachment",
    "add_attachments",
    "sanitize_inspect",
    "sanitize",
    "redact",
];

impl Operation {
    pub fn id(&self) -> &'static str {
        match self {
            Operation::Inspect { .. } => "inspect",
            Operation::FormFields { .. } => "form_fields",
            Operation::ExtractText { .. } => "extract_text",
            Operation::Search { .. } => "search",
            Operation::Render { .. } => "render",
            Operation::Merge { .. } => "merge",
            Operation::ExtractPages { .. } => "extract_pages",
            Operation::DeletePages { .. } => "delete_pages",
            Operation::RotatePages { .. } => "rotate_pages",
            Operation::Split { .. } => "split",
            Operation::Annotate { .. } => "annotate",
            Operation::FillForm { .. } => "fill_form",
            Operation::ApplyEdits { .. } => "apply_edits",
            Operation::OrganizePages { .. } => "organize_pages",
            Operation::StampText { .. } => "stamp_text",
            Operation::ExportImages { .. } => "export_images",
            Operation::RedactionPreview { .. } => "redaction_preview",
            Operation::OcrPages { .. } => "ocr_pages",
            Operation::Protect { .. } => "protect",
            Operation::Unprotect { .. } => "unprotect",
            Operation::SetProperties { .. } => "set_properties",
            Operation::Watermark { .. } => "watermark",
            Operation::CropPages { .. } => "crop_pages",
            Operation::SetBookmarks { .. } => "set_bookmarks",
            Operation::ImagesToPdf { .. } => "images_to_pdf",
            Operation::ExportText { .. } => "export_text",
            Operation::Optimize { .. } => "optimize",
            Operation::Compare { .. } => "compare",
            Operation::VerifySignatures { .. } => "verify_signatures",
            Operation::Deskew { .. } => "deskew",
            Operation::FixOrientation { .. } => "fix_orientation",
            Operation::Sign { .. } => "sign",
            Operation::AddSignatureMark { .. } => "add_signature_mark",
            Operation::FindBlankPages { .. } => "find_blank_pages",
            Operation::ExtractImages { .. } => "extract_images",
            Operation::ListAttachments { .. } => "list_attachments",
            Operation::ExtractAttachment { .. } => "extract_attachment",
            Operation::AddAttachments { .. } => "add_attachments",
            Operation::SanitizeInspect { .. } => "sanitize_inspect",
            Operation::Sanitize { .. } => "sanitize",
            Operation::Redact { .. } => "redact",
        }
    }

    /// Whether the operation writes files.
    pub fn writes_output(&self) -> bool {
        !matches!(
            self,
            Operation::Inspect { .. }
                | Operation::FormFields { .. }
                | Operation::ExtractText { .. }
                | Operation::Search { .. }
                | Operation::RedactionPreview { .. }
                | Operation::SanitizeInspect { .. }
                | Operation::ListAttachments { .. }
                | Operation::Compare { .. }
                | Operation::VerifySignatures { .. }
                | Operation::FindBlankPages { .. }
        )
    }

    /// All input paths, used to protect sources from being overwritten.
    pub fn inputs(&self) -> Vec<PathBuf> {
        match self {
            Operation::Merge { inputs, .. } | Operation::ImagesToPdf { inputs, .. } => inputs.clone(),
            Operation::Compare { old, new } => vec![old.clone(), new.clone()],
            // Files being attached are inputs too: never overwritten by the output.
            Operation::AddAttachments { input, files, .. } => {
                std::iter::once(input.clone()).chain(files.iter().cloned()).collect()
            }
            Operation::OrganizePages { input, extra_inputs, .. } => {
                std::iter::once(input.clone()).chain(extra_inputs.iter().cloned()).collect()
            }
            Operation::Inspect { input }
            | Operation::FormFields { input }
            | Operation::ExtractText { input, .. }
            | Operation::Search { input, .. }
            | Operation::Render { input, .. }
            | Operation::ExtractPages { input, .. }
            | Operation::DeletePages { input, .. }
            | Operation::RotatePages { input, .. }
            | Operation::Split { input, .. }
            | Operation::Annotate { input, .. }
            | Operation::FillForm { input, .. }
            | Operation::StampText { input, .. }
            | Operation::ExportImages { input, .. }
            | Operation::RedactionPreview { input, .. }
            | Operation::OcrPages { input, .. }
            | Operation::Protect { input, .. }
            | Operation::Unprotect { input, .. }
            | Operation::SetProperties { input, .. }
            | Operation::Watermark { input, .. }
            | Operation::CropPages { input, .. }
            | Operation::ListAttachments { input }
            | Operation::VerifySignatures { input }
            | Operation::Deskew { input, .. }
            | Operation::FixOrientation { input, .. }
            | Operation::Sign { input, .. }
            | Operation::AddSignatureMark { input, .. }
            | Operation::FindBlankPages { input, .. }
            | Operation::ExtractImages { input, .. }
            | Operation::SetBookmarks { input, .. }
            | Operation::ExportText { input, .. }
            | Operation::Optimize { input, .. }
            | Operation::ExtractAttachment { input, .. }
            | Operation::SanitizeInspect { input }
            | Operation::Sanitize { input, .. }
            | Operation::Redact { input, .. }
            | Operation::ApplyEdits { input, .. } => vec![input.clone()],
        }
    }

    /// Folder the operation writes into (where its staged files live), if it writes files.
    pub fn output_dir(&self) -> Option<PathBuf> {
        let parent = |p: &PathBuf| {
            Some(
                p.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from(".")),
            )
        };
        match self {
            Operation::Inspect { .. }
            | Operation::FormFields { .. }
            | Operation::ExtractText { .. }
            | Operation::Search { .. }
            | Operation::RedactionPreview { .. }
            | Operation::SanitizeInspect { .. }
            | Operation::ListAttachments { .. }
            | Operation::Compare { .. }
            | Operation::VerifySignatures { .. }
            | Operation::FindBlankPages { .. } => None,
            Operation::Split { output_dir, .. }
            | Operation::ExportImages { output_dir, .. }
            | Operation::ExtractImages { output_dir, .. } => Some(output_dir.clone()),
            Operation::Render { output, .. }
            | Operation::Merge { output, .. }
            | Operation::ExtractPages { output, .. }
            | Operation::DeletePages { output, .. }
            | Operation::RotatePages { output, .. }
            | Operation::Annotate { output, .. }
            | Operation::FillForm { output, .. }
            | Operation::ApplyEdits { output, .. }
            | Operation::OrganizePages { output, .. }
            | Operation::Redact { output, .. }
            | Operation::OcrPages { output, .. }
            | Operation::Protect { output, .. }
            | Operation::Unprotect { output, .. }
            | Operation::SetProperties { output, .. }
            | Operation::Watermark { output, .. }
            | Operation::CropPages { output, .. }
            | Operation::ExtractAttachment { output, .. }
            | Operation::SetBookmarks { output, .. }
            | Operation::ImagesToPdf { output, .. }
            | Operation::ExportText { output, .. }
            | Operation::Optimize { output, .. }
            | Operation::Deskew { output, .. }
            | Operation::FixOrientation { output, .. }
            | Operation::Sign { output, .. }
            | Operation::AddSignatureMark { output, .. }
            | Operation::AddAttachments { output, .. }
            | Operation::Sanitize { output, .. }
            | Operation::StampText { output, .. } => parent(output),
        }
    }

    pub fn validate(&self) -> PwResult<()> {
        for p in self.inputs() {
            if p.as_os_str().is_empty() {
                return Err(PwError::invalid("op.empty_path", "An input file is required."));
            }
        }
        match self {
            Operation::Merge { inputs, .. } => {
                if inputs.len() < 2 {
                    return Err(PwError::invalid("merge.too_few", "Choose at least two files to combine."));
                }
                if inputs.len() > MAX_MERGE_INPUTS {
                    return Err(PwError::invalid(
                        "merge.too_many",
                        format!("Combine at most {MAX_MERGE_INPUTS} files at once."),
                    ));
                }
            }
            Operation::Search { query, .. } => {
                if query.trim().is_empty() {
                    return Err(PwError::invalid("search.empty", "Enter text to search for."));
                }
                if query.chars().count() > MAX_QUERY_CHARS {
                    return Err(PwError::invalid("search.too_long", "The search text is too long."));
                }
            }
            Operation::Render { page, scale, .. } => {
                if *page == 0 {
                    return Err(PwError::invalid("render.page_zero", "Pages start at 1."));
                }
                if !scale.is_finite() || !(MIN_RENDER_SCALE..=MAX_RENDER_SCALE).contains(scale) {
                    return Err(PwError::invalid("render.scale", "Zoom must be between 5% and 800%."));
                }
            }
            Operation::RotatePages { degrees, .. } => {
                if degrees % 90 != 0 || *degrees == 0 || degrees.abs() > 270 {
                    return Err(PwError::invalid("rotate.degrees", "Rotate by 90, 180 or 270 degrees."));
                }
            }
            Operation::Split { split: SplitMode::EveryN { n }, .. } if *n == 0 => {
                return Err(PwError::invalid("split.n_zero", "Split size must be at least 1 page."));
            }
            Operation::Split { split: SplitMode::Ranges { ranges }, .. } if ranges.is_empty() => {
                return Err(PwError::invalid("split.no_ranges", "Enter at least one page range."));
            }
            Operation::Annotate { annotations, .. } => validate_annotations(annotations)?,
            Operation::FillForm { values, .. } => validate_form_values(values)?,
            Operation::ExportImages { dpi, .. } => {
                if !(36..=600).contains(dpi) {
                    return Err(PwError::invalid("export.dpi", "Choose a resolution from 36 to 600 dpi."));
                }
            }
            Operation::StampText { stamps, start, digits, .. } => {
                if stamps.is_empty() || stamps.len() > MAX_STAMPS {
                    return Err(PwError::invalid("stamp.count", format!("Add between 1 and {MAX_STAMPS} stamps.")));
                }
                if *digits > 12 || *start > 999_999_999_999 {
                    return Err(PwError::invalid("stamp.number", "The start number or digit count is too large."));
                }
                for s in stamps {
                    if s.text.trim().is_empty() {
                        return Err(PwError::invalid("stamp.empty", "A stamp needs some text."));
                    }
                    if s.text.chars().count() > MAX_STAMP_CHARS {
                        return Err(PwError::invalid("stamp.too_long", "Stamp text is too long."));
                    }
                    // The built-in Helvetica font covers Latin-1 only; refuse rather than drop glyphs.
                    if !s.text.chars().all(|c| (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c)) {
                        return Err(PwError::invalid(
                            "stamp.charset",
                            "Stamps support Latin characters only for now (letters, digits and common symbols).",
                        ));
                    }
                    if !(4.0..=72.0).contains(&s.font_size) || !(0.0..=144.0).contains(&s.margin) {
                        return Err(PwError::invalid(
                            "stamp.size",
                            "Use a text size of 4–72 pt and a margin of 0–144 pt.",
                        ));
                    }
                    if !crate::settings::is_valid_accent(&s.color) {
                        return Err(PwError::invalid("stamp.color", "Stamp colors must look like #rrggbb."));
                    }
                }
            }
            Operation::OrganizePages { extra_inputs, pages, .. } => {
                if pages.is_empty() {
                    return Err(PwError::invalid("organize.empty", "A PDF must keep at least one page."));
                }
                if pages.len() > MAX_ORGANIZE_PAGES {
                    return Err(PwError::invalid("organize.too_many", "Too many pages in one document."));
                }
                if extra_inputs.len() > MAX_MERGE_INPUTS {
                    return Err(PwError::invalid("organize.too_many_files", "Too many files to insert from."));
                }
                for p in pages {
                    if p.source as usize > extra_inputs.len() {
                        return Err(PwError::invalid(
                            "organize.source",
                            "A page refers to a file that is not in the list.",
                        ));
                    }
                    if p.rotate % 90 != 0 || p.rotate.abs() >= 360 {
                        return Err(PwError::invalid(
                            "organize.rotate",
                            "Pages can only be turned by 90, 180 or 270 degrees.",
                        ));
                    }
                }
            }
            Operation::Redact { areas, .. } | Operation::RedactionPreview { areas, .. } => {
                validate_redact_areas(areas)?
            }
            Operation::ImagesToPdf { inputs, margin, .. } => {
                if inputs.is_empty() {
                    return Err(PwError::invalid("images.none", "Add at least one image."));
                }
                if inputs.len() > MAX_IMAGES_TO_PDF {
                    return Err(PwError::invalid(
                        "images.too_many",
                        format!("Use at most {MAX_IMAGES_TO_PDF} images at once."),
                    ));
                }
                if !margin.is_finite() || !(0.0..=144.0).contains(margin) {
                    return Err(PwError::invalid("images.margin", "Use a margin of 0–144 pt."));
                }
            }
            Operation::SetBookmarks { bookmarks, .. } => {
                if bookmarks.len() > MAX_BOOKMARKS {
                    return Err(PwError::invalid(
                        "bookmarks.too_many",
                        format!("Use at most {MAX_BOOKMARKS} bookmarks."),
                    ));
                }
                let mut prev: Option<u32> = None;
                for (i, b) in bookmarks.iter().enumerate() {
                    let n = i + 1;
                    if b.title.trim().is_empty() {
                        return Err(PwError::invalid("bookmarks.title_empty", format!("Bookmark {n} needs a title.")));
                    }
                    if b.title.chars().count() > MAX_BOOKMARK_TITLE || b.title.chars().any(|c| c.is_control()) {
                        return Err(PwError::invalid(
                            "bookmarks.title",
                            format!("Bookmark {n}: titles are at most {MAX_BOOKMARK_TITLE} characters, without line breaks."),
                        ));
                    }
                    let max_depth = prev.map_or(0, |p| p + 1).min(MAX_BOOKMARK_DEPTH);
                    if b.depth > max_depth {
                        return Err(PwError::invalid(
                            "bookmarks.depth",
                            format!("Bookmark {n} is indented more than one level below the one before it."),
                        ));
                    }
                    prev = Some(b.depth);
                }
            }
            Operation::AddAttachments { files, .. } => {
                if files.is_empty() {
                    return Err(PwError::invalid("attach.none", "Choose at least one file to attach."));
                }
                if files.len() > MAX_ATTACH_FILES {
                    return Err(PwError::invalid(
                        "attach.too_many",
                        format!("Attach at most {MAX_ATTACH_FILES} files at once."),
                    ));
                }
            }
            Operation::Optimize { images: Some(r), .. } => {
                if !(MIN_IMAGE_DPI..=MAX_IMAGE_DPI).contains(&r.max_dpi) {
                    return Err(PwError::invalid(
                        "optimize.dpi",
                        format!("Image resolution must be between {MIN_IMAGE_DPI} and {MAX_IMAGE_DPI} dpi."),
                    ));
                }
                if !(MIN_JPEG_QUALITY..=MAX_JPEG_QUALITY).contains(&r.quality) {
                    return Err(PwError::invalid(
                        "optimize.quality",
                        format!("Image quality must be between {MIN_JPEG_QUALITY} and {MAX_JPEG_QUALITY}."),
                    ));
                }
            }
            Operation::FindBlankPages { max_ink, .. } => {
                if !(max_ink.is_finite() && *max_ink >= 0.0 && *max_ink <= 0.05) {
                    return Err(PwError::invalid(
                        "blank.threshold",
                        "The blank-page threshold must be between 0 and 5% ink.",
                    ));
                }
            }
            Operation::AddSignatureMark { rect, mark, .. } => {
                let (w, h) = ((rect.right - rect.left).abs(), (rect.top - rect.bottom).abs());
                if ![rect.left, rect.bottom, rect.right, rect.top].iter().all(|v| v.is_finite()) || w < 10.0 || h < 5.0
                {
                    return Err(PwError::invalid("mark.box", "Make the signature box at least 10 × 5 points."));
                }
                match mark {
                    SignatureMark::Typed { text, .. } => {
                        let t = text.trim();
                        if t.is_empty() || t.chars().count() > 100 {
                            return Err(PwError::invalid("mark.text", "Type 1 to 100 characters."));
                        }
                        if t.chars().any(|c| (c as u32) > 255 || c.is_control()) {
                            return Err(PwError::invalid(
                                "mark.text_charset",
                                "Typed signatures can use Latin letters only for now (Western European characters). Draw it or use a picture instead.",
                            ));
                        }
                    }
                    SignatureMark::Drawn { strokes, width, .. } => {
                        let points: usize = strokes.iter().map(Vec::len).sum();
                        if points == 0 {
                            return Err(PwError::invalid("mark.empty", "Draw your signature first."));
                        }
                        if strokes.len() > 1000 || points > 50_000 {
                            return Err(PwError::invalid("mark.too_complex", "The drawing has too many points."));
                        }
                        let ok = |v: f32| v.is_finite() && (-0.01..=1.01).contains(&v);
                        if !strokes.iter().flatten().all(|p| ok(p[0]) && ok(p[1])) || !(0.2..=20.0).contains(width) {
                            return Err(PwError::invalid("mark.points", "The drawing has points outside its box."));
                        }
                    }
                    SignatureMark::Image { .. } => {}
                }
            }
            Operation::CropPages { margins: m, .. } => {
                let all = [m.top, m.right, m.bottom, m.left];
                if all.iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 14_400.0) {
                    return Err(PwError::invalid("crop.margin", "Margins must be between 0 and 14,400 points."));
                }
                if all.iter().all(|v| *v == 0.0) {
                    return Err(PwError::invalid("crop.empty", "Enter at least one margin to crop."));
                }
            }
            Operation::Watermark { watermark: w, .. } => {
                if w.text.trim().is_empty() {
                    return Err(PwError::invalid("watermark.empty", "Enter the watermark text."));
                }
                if w.text.chars().count() > MAX_STAMP_CHARS {
                    return Err(PwError::invalid("watermark.too_long", "The watermark text is too long."));
                }
                if !w.text.chars().all(|c| (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c)) {
                    return Err(PwError::invalid(
                        "watermark.charset",
                        "Watermarks support Latin characters only for now (letters, digits and common symbols).",
                    ));
                }
                if !(8.0..=400.0).contains(&w.font_size) {
                    return Err(PwError::invalid("watermark.size", "Use a text size of 8–400 pt."));
                }
                if !(0.05..=1.0).contains(&w.opacity) {
                    return Err(PwError::invalid("watermark.opacity", "Use an opacity of 5–100%."));
                }
                if !(-90.0..=90.0).contains(&w.angle) || !w.angle.is_finite() {
                    return Err(PwError::invalid("watermark.angle", "Use an angle from −90° to 90°."));
                }
                let hex = w.color.strip_prefix('#').unwrap_or("");
                if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(PwError::invalid("watermark.color", "Watermark colors must look like #rrggbb."));
                }
            }
            Operation::SetProperties { title, author, subject, keywords, .. } => {
                let fields = [title, author, subject, keywords];
                if fields.iter().all(|f| f.is_none()) {
                    return Err(PwError::invalid("properties.empty", "There are no property changes to save."));
                }
                for v in fields.into_iter().flatten() {
                    if v.chars().count() > MAX_PROPERTY_CHARS {
                        return Err(PwError::invalid(
                            "properties.too_long",
                            format!("Properties can be at most {MAX_PROPERTY_CHARS} characters long."),
                        ));
                    }
                    if v.chars().any(|c| c.is_control() && c != '\t') {
                        return Err(PwError::invalid(
                            "properties.control_chars",
                            "Properties cannot contain line breaks or control characters.",
                        ));
                    }
                }
            }
            Operation::OcrPages { dpi, .. } => {
                if !(100..=400).contains(dpi) {
                    return Err(PwError::invalid("ocr.dpi", "Choose a recognition resolution from 100 to 400 dpi."));
                }
            }
            Operation::ApplyEdits { form_values, annotations, .. } => {
                if form_values.is_empty() && annotations.is_empty() {
                    return Err(PwError::invalid("edits.empty", "There are no changes to save."));
                }
                if !form_values.is_empty() {
                    validate_form_values(form_values)?;
                }
                if !annotations.is_empty() {
                    validate_annotations(annotations)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn validate_form_values(list: &[FieldValue]) -> PwResult<()> {
    if list.is_empty() {
        return Err(PwError::invalid("form.empty", "There are no field values to save."));
    }
    if list.len() > MAX_FORM_VALUES {
        return Err(PwError::invalid("form.too_many", "Too many field values in one request."));
    }
    let mut seen = std::collections::HashSet::new();
    for v in list {
        if v.name.trim().is_empty() {
            return Err(PwError::invalid("form.no_name", "A field value has no field name."));
        }
        if !seen.insert(v.name.as_str()) {
            return Err(PwError::invalid("form.duplicate", format!("The field “{}” is listed twice.", v.name)));
        }
        if let FormValue::Text(t) = &v.value {
            if t.chars().count() > MAX_FORM_TEXT_CHARS {
                return Err(PwError::invalid("form.too_long", format!("The value for “{}” is too long.", v.name)));
            }
        }
    }
    Ok(())
}

fn validate_redact_areas(areas: &[RedactArea]) -> PwResult<()> {
    if areas.is_empty() {
        return Err(PwError::invalid("redact.empty", "Mark at least one area to redact."));
    }
    if areas.len() > MAX_REDACT_AREAS {
        return Err(PwError::invalid("redact.too_many", "Too many redaction areas in one request."));
    }
    for a in areas {
        let r = a.rect;
        let finite = [r.left, r.bottom, r.right, r.top].iter().all(|v| v.is_finite() && v.abs() < 1.0e6);
        if !finite || r.right <= r.left || r.top <= r.bottom {
            return Err(PwError::invalid("redact.rect", "A redaction area has an invalid location."));
        }
    }
    Ok(())
}

fn validate_annotations(list: &[AnnotationSpec]) -> PwResult<()> {
    if list.is_empty() {
        return Err(PwError::invalid("annotate.empty", "There are no annotations to save."));
    }
    if list.len() > MAX_ANNOTATIONS_PER_REQUEST {
        return Err(PwError::invalid("annotate.too_many", "Too many annotations in one request."));
    }
    for a in list {
        if a.rects.is_empty() || a.rects.len() > 1000 {
            return Err(PwError::invalid("annotate.rects", "Each annotation needs a location."));
        }
        for r in &a.rects {
            let finite = [r.left, r.bottom, r.right, r.top].iter().all(|v| v.is_finite() && v.abs() < 1.0e6);
            if !finite || r.right <= r.left || r.top <= r.bottom {
                return Err(PwError::invalid("annotate.rect", "An annotation has an invalid location."));
            }
        }
        if !crate::settings::is_valid_accent(&a.color) {
            return Err(PwError::invalid("annotate.color", "Annotation colors must look like #rrggbb."));
        }
        if a.contents.as_ref().is_some_and(|c| c.chars().count() > MAX_NOTE_CHARS) {
            return Err(PwError::invalid("annotate.note_too_long", "The note is too long."));
        }
        if a.kind == AnnotationKind::Note && a.contents.as_deref().unwrap_or("").trim().is_empty() {
            return Err(PwError::invalid("annotate.note_empty", "A note needs some text."));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub schema_version: u32,
    #[serde(default)]
    pub request_id: Option<String>,
    pub operation: Operation,
    #[serde(default)]
    pub collision: CollisionPolicy,
    /// Document open password. Accepted on input but never serialized, so it cannot leak into
    /// results, logs or diagnostics.
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// For `protect`: the new open password. Never serialized (not in the job journal or logs),
    /// so an interrupted protect job must be given the password again to re-run.
    #[serde(default, skip_serializing)]
    pub new_password: Option<String>,
    /// For `protect`: the permissions password; a random one is generated when absent.
    #[serde(default, skip_serializing)]
    pub new_owner_password: Option<String>,
    /// For `sign`: the certificate file's password. Never serialized.
    #[serde(default, skip_serializing)]
    pub certificate_password: Option<String>,
}

impl OperationRequest {
    pub fn new(operation: Operation) -> Self {
        OperationRequest {
            schema_version: SCHEMA_VERSION,
            request_id: None,
            operation,
            collision: CollisionPolicy::Fail,
            password: None,
            new_password: None,
            new_owner_password: None,
            certificate_password: None,
        }
    }

    pub fn validate(&self) -> PwResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(PwError::new(
                crate::error::ErrorCategory::UnsupportedOperation,
                "op.schema_version",
                format!("Unsupported request schema version {} (expected {SCHEMA_VERSION}).", self.schema_version),
            ));
        }
        self.operation.validate()?;
        if let Operation::Sign { reason, location, .. } = &self.operation {
            if self.certificate_password.is_none() {
                return Err(PwError::invalid("sign.password_missing", "Enter the certificate's password."));
            }
            for t in [reason, location].into_iter().flatten() {
                if t.chars().count() > 200 {
                    return Err(PwError::invalid(
                        "sign.text_long",
                        "Reason and location can be at most 200 characters.",
                    ));
                }
            }
        }
        if let Operation::Protect { .. } = self.operation {
            let pw = self.new_password.as_deref().unwrap_or("");
            if pw.is_empty() {
                return Err(PwError::invalid("protect.password_empty", "Enter a password to protect the file with."));
            }
            for p in [Some(pw), self.new_owner_password.as_deref()].into_iter().flatten() {
                // AES-256 (revision 6) uses at most 127 bytes of a password.
                if p.len() > 127 {
                    return Err(PwError::invalid("protect.password_long", "Passwords can be at most 127 bytes long."));
                }
            }
            if self.new_owner_password.as_deref() == Some(pw) {
                return Err(PwError::invalid(
                    "protect.same_passwords",
                    "Use a different permissions password from the open password, or leave it empty.",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub path: PathBuf,
    pub bytes: u64,
    pub pages: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationResult {
    pub schema_version: u32,
    pub operation: String,
    pub outputs: Vec<OutputInfo>,
    /// Operation-specific structured data (inspection report, search hits, …).
    pub data: serde_json::Value,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn operation_ids_list_every_variant_once() {
        // Windows checkouts may have CRLF line endings.
        let src = include_str!("ops.rs").replace("\r\n", "\n");
        let start = src.find("pub fn id(&self)").expect("id fn");
        let end = start + src[start..].find("\n    }\n").expect("end of id fn");
        let mut ids: Vec<&str> =
            src[start..end].split("=> \"").skip(1).map(|t| &t[..t.find('"').expect("quote")]).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "operation ids are unique");
        let mut listed: Vec<&str> = OPERATION_IDS.to_vec();
        listed.sort_unstable();
        assert_eq!(listed, ids, "OPERATION_IDS must match Operation::id");
    }

    use super::*;

    #[test]
    fn json_shape_is_stable() {
        let req = OperationRequest::new(Operation::RotatePages {
            input: "a.pdf".into(),
            pages: Some("1-2".into()),
            degrees: 90,
            output: "b.pdf".into(),
        });
        let v = serde_json::to_value(&req).expect("ser");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["operation"]["op"], "rotate_pages");
        assert_eq!(v["collision"], "fail");
        let back: OperationRequest = serde_json::from_value(v).expect("de");
        assert_eq!(back, req);
    }

    #[test]
    fn protect_passwords_are_validated_and_never_serialized() {
        let mut req = OperationRequest::new(Operation::Protect {
            input: "a.pdf".into(),
            permissions: ProtectPermissions::default(),
            output: "o.pdf".into(),
        });
        assert_eq!(req.validate().err().map(|e| e.code).as_deref(), Some("protect.password_empty"));
        req.new_password = Some("open-sesame".into());
        req.new_owner_password = Some("open-sesame".into());
        assert_eq!(req.validate().err().map(|e| e.code).as_deref(), Some("protect.same_passwords"));
        req.new_owner_password = Some("x".repeat(128));
        assert_eq!(req.validate().err().map(|e| e.code).as_deref(), Some("protect.password_long"));
        req.new_owner_password = Some("owner-secret".into());
        assert!(req.validate().is_ok());
        let out = serde_json::to_string(&req).expect("json");
        assert!(!out.contains("open-sesame") && !out.contains("owner-secret"), "{out}");
        let back: OperationRequest = serde_json::from_str(
            r#"{"schema_version":1,"operation":{"op":"protect","input":"a.pdf","output":"o.pdf"},"new_password":"pw"}"#,
        )
        .expect("parse");
        assert_eq!(back.new_password.as_deref(), Some("pw"));
        assert!(
            matches!(back.operation, Operation::Protect { permissions, .. } if permissions == ProtectPermissions::default())
        );
    }

    #[test]
    fn password_is_accepted_but_never_serialized() {
        let input =
            serde_json::json!({"schema_version":1,"operation":{"op":"inspect","input":"a.pdf"},"password":"hunter2"});
        let req: OperationRequest = serde_json::from_value(input).expect("de");
        assert_eq!(req.password.as_deref(), Some("hunter2"));
        let out = serde_json::to_string(&req).expect("ser");
        assert!(!out.contains("hunter2") && !out.contains("password"), "{out}");
    }

    #[test]
    fn form_values_parse_and_validate() {
        let req: OperationRequest = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "operation": {"op": "fill_form", "input": "f.pdf", "output": "o.pdf", "values": [
                {"name": "full_name", "value": "Ada"},
                {"name": "subscribe", "value": true}
            ]}
        }))
        .expect("parse");
        let Operation::FillForm { values, .. } = &req.operation else { panic!("variant") };
        assert_eq!(values[0].value, FormValue::Text("Ada".into()));
        assert_eq!(values[1].value, FormValue::Checked(true));
        assert!(req.validate().is_ok());
        let v = |values: Vec<FieldValue>| {
            OperationRequest::new(Operation::FillForm { input: "f".into(), values, output: "o".into() })
                .validate()
                .err()
                .map(|e| e.code)
        };
        let fv = |n: &str, t: &str| FieldValue { name: n.into(), value: FormValue::Text(t.into()) };
        assert_eq!(v(vec![]).as_deref(), Some("form.empty"));
        assert_eq!(v(vec![fv(" ", "x")]).as_deref(), Some("form.no_name"));
        assert_eq!(v(vec![fv("a", "x"), fv("a", "y")]).as_deref(), Some("form.duplicate"));
        assert_eq!(v(vec![fv("a", &"x".repeat(MAX_FORM_TEXT_CHARS + 1))]).as_deref(), Some("form.too_long"));

        // apply_edits: either list may be empty, not both; each non-empty list is validated.
        let edits = |form_values: Vec<FieldValue>, annotations: Vec<AnnotationSpec>| {
            OperationRequest::new(Operation::ApplyEdits {
                input: "f".into(),
                form_values,
                annotations,
                output: "o".into(),
            })
            .validate()
            .err()
            .map(|e| e.code)
        };
        assert_eq!(edits(vec![], vec![]).as_deref(), Some("edits.empty"));
        assert_eq!(edits(vec![fv("a", "x")], vec![]), None);
        assert_eq!(edits(vec![fv("a", "x"), fv("a", "y")], vec![]).as_deref(), Some("form.duplicate"));
        let bad = AnnotationSpec {
            page: 0,
            kind: AnnotationKind::Highlight,
            rects: vec![],
            color: "#ffcc00".into(),
            contents: None,
        };
        assert_eq!(edits(vec![], vec![bad]).as_deref(), Some("annotate.rects"));
    }

    #[test]
    fn rejects_unknown_operation_and_fields_types() {
        let bad = serde_json::json!({"schema_version":1,"operation":{"op":"format_disk","input":"x"}});
        assert!(serde_json::from_value::<OperationRequest>(bad).is_err());
    }

    #[test]
    fn structural_validation() {
        let v = |op: Operation| OperationRequest::new(op).validate().err().map(|e| e.code);
        assert_eq!(
            v(Operation::Merge { inputs: vec!["a".into()], output: "o".into() }).as_deref(),
            Some("merge.too_few")
        );
        assert_eq!(
            v(Operation::Search { input: "a".into(), query: "  ".into(), match_case: false, whole_word: false })
                .as_deref(),
            Some("search.empty")
        );
        assert_eq!(
            v(Operation::Render { input: "a".into(), page: 0, scale: 1.0, output: "o".into() }).as_deref(),
            Some("render.page_zero")
        );
        assert_eq!(
            v(Operation::Render { input: "a".into(), page: 1, scale: f32::NAN, output: "o".into() }).as_deref(),
            Some("render.scale")
        );
        assert_eq!(
            v(Operation::RotatePages { input: "a".into(), pages: None, degrees: 45, output: "o".into() }).as_deref(),
            Some("rotate.degrees")
        );
        assert_eq!(
            v(Operation::RotatePages { input: "a".into(), pages: None, degrees: 360, output: "o".into() }).as_deref(),
            Some("rotate.degrees")
        );
        assert_eq!(
            v(Operation::Split { input: "a".into(), split: SplitMode::EveryN { n: 0 }, output_dir: "d".into() })
                .as_deref(),
            Some("split.n_zero")
        );
        assert_eq!(v(Operation::Inspect { input: "".into() }).as_deref(), Some("op.empty_path"));
        assert!(
            v(Operation::RotatePages { input: "a".into(), pages: None, degrees: -90, output: "o".into() }).is_none()
        );
        let rect = RectPt { left: 1.0, bottom: 1.0, right: 2.0, top: 2.0 };
        let ann = |kind, color: &str, contents: Option<&str>, rects: Vec<RectPt>| Operation::Annotate {
            input: "a".into(),
            annotations: vec![AnnotationSpec {
                page: 0,
                kind,
                rects,
                color: color.into(),
                contents: contents.map(Into::into),
            }],
            output: "o".into(),
        };
        assert!(v(ann(AnnotationKind::Highlight, "#ffcc00", None, vec![rect])).is_none());
        assert_eq!(v(ann(AnnotationKind::Highlight, "yellow", None, vec![rect])).as_deref(), Some("annotate.color"));
        assert_eq!(v(ann(AnnotationKind::Highlight, "#ffcc00", None, vec![])).as_deref(), Some("annotate.rects"));
        let flipped = RectPt { left: 5.0, ..rect };
        let redact = |areas: Vec<RedactArea>| Operation::Redact { input: "a".into(), areas, output: "o".into() };
        assert!(v(redact(vec![RedactArea { page: 0, rect }])).is_none());
        assert_eq!(v(redact(vec![])).as_deref(), Some("redact.empty"));
        assert_eq!(v(redact(vec![RedactArea { page: 0, rect: flipped }])).as_deref(), Some("redact.rect"));
        let inf = RectPt { top: f32::INFINITY, ..rect };
        assert_eq!(
            v(Operation::RedactionPreview { input: "a".into(), areas: vec![RedactArea { page: 0, rect: inf }] })
                .as_deref(),
            Some("redact.rect")
        );
        assert!(!Operation::RedactionPreview { input: "a".into(), areas: vec![] }.writes_output());
        assert_eq!(v(ann(AnnotationKind::Underline, "#ffcc00", None, vec![flipped])).as_deref(), Some("annotate.rect"));
        let nan = RectPt { left: f32::NAN, ..rect };
        assert_eq!(v(ann(AnnotationKind::Underline, "#ffcc00", None, vec![nan])).as_deref(), Some("annotate.rect"));
        assert_eq!(
            v(ann(AnnotationKind::Note, "#ffcc00", Some("  "), vec![rect])).as_deref(),
            Some("annotate.note_empty")
        );
        let empty = Operation::Annotate { input: "a".into(), annotations: vec![], output: "o".into() };
        assert_eq!(v(empty).as_deref(), Some("annotate.empty"));
        let mut r = OperationRequest::new(Operation::Inspect { input: "a".into() });
        r.schema_version = 2;
        assert_eq!(r.validate().expect_err("version").code, "op.schema_version");
    }
}

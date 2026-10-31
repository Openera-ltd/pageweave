//! `pageweave` — scriptable CLI over the shared engine (PRD 17 "CLI and local API").
//!
//! Every command builds a typed `OperationRequest` and calls the same `pw_pdf::execute` the desktop
//! app uses. Results are printed as human text or, with `--json`, as a stable JSON envelope.
//! Exit codes come from the error category (see `ErrorCategory::exit_code`); 0 is success and 1 is
//! a usage error; 70 is also used if the worker thread panics and 71 if it cannot be started.
//! Passwords are only accepted on stdin (`--password-stdin`), never as arguments.

use clap::{Parser, Subcommand, ValueEnum};
use pw_core::ops::{Operation, OperationRequest, OperationResult, SplitMode};
use pw_core::output::CollisionPolicy;
use pw_core::PwError;
use serde_json::json;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "pageweave", version, about = "PageWeave document tools", long_about = None)]
struct Cli {
    /// Print a JSON envelope instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    /// What to do when an output file already exists.
    #[arg(long, global = true, value_enum, default_value_t = Conflict::Fail)]
    on_conflict: Conflict,
    /// Read the document password from the first line of standard input.
    #[arg(long, global = true)]
    password_stdin: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Conflict {
    Fail,
    Rename,
    Replace,
}

#[derive(Clone, Copy, ValueEnum)]
enum PatternKind {
    Email,
    Phone,
    Card,
    Iban,
    Ssn,
}

#[derive(Clone, Copy, ValueEnum)]
enum SheetFormat {
    Xlsx,
    Csv,
}

#[derive(Clone, Copy, ValueEnum)]
enum DataFormat {
    Xfdf,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Show page count, sizes, metadata, bookmarks and security status.
    Inspect { file: PathBuf },
    /// List form fields (names, kinds, values, options) as JSON.
    Fields { file: PathBuf },
    /// List the comments and markup already in a file, or change them in a new file (-o):
    /// --delete PAGE:INDEX:TYPE removes one (with its pop-up); --set-text PAGE:INDEX:TYPE=TEXT
    /// changes its comment; --set-color PAGE:INDEX:TYPE=#rrggbb its colour (markup, shapes, ink,
    /// notes). PAGE is 1-based; INDEX and TYPE are as the listing prints them.
    Annotations {
        file: PathBuf,
        #[arg(long, value_name = "PAGE:INDEX:TYPE")]
        delete: Vec<String>,
        #[arg(long, value_name = "PAGE:INDEX:TYPE=TEXT")]
        set_text: Vec<String>,
        #[arg(long, value_name = "PAGE:INDEX:TYPE=#RRGGBB")]
        set_color: Vec<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Set page labels (the numbers readers show) in a new file: --range PAGE:STYLE[:FIRST[:PREFIX]]
    /// per range, e.g. --range 1:roman-lower --range 5:decimal --range 17:decimal:1:A-; styles:
    /// decimal, roman-upper, roman-lower, letters-upper, letters-lower, none. --remove clears them.
    Labels {
        file: PathBuf,
        #[arg(long = "range", value_name = "PAGE:STYLE[:FIRST[:PREFIX]]", required_unless_present = "remove")]
        ranges: Vec<String>,
        #[arg(long, conflicts_with = "ranges")]
        remove: bool,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Check what can be checked automatically about accessibility (tags, language, title,
    /// alternate text, table headers, form labels, …) and list what a person must review.
    Accessibility { file: PathBuf },
    /// Find sensitive text — e-mail addresses, phone numbers, card numbers, IBANs, US SSNs and/or a
    /// custom regular expression — with the position of each match (to redact with `redact`).
    Patterns {
        file: PathBuf,
        /// Kinds to look for, comma-separated.
        #[arg(long, value_enum, value_delimiter = ',')]
        kinds: Vec<PatternKind>,
        /// A regular expression to look for as well.
        #[arg(long)]
        custom: Option<String>,
        #[arg(long)]
        pages: Option<String>,
    },
    /// Save the form's field values as a form data file: XFDF (default), or JSON with --format json
    /// or an output name ending in .json.
    FormExport {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, value_enum)]
        format: Option<DataFormat>,
    },
    /// Fill the form from an XFDF or JSON form data file into a new file (-o), or show how the file
    /// matches the form without filling anything (--preview).
    FormImport {
        file: PathBuf,
        data: PathBuf,
        #[arg(short, long, required_unless_present = "preview")]
        output: Option<PathBuf>,
        #[arg(long, conflicts_with = "output")]
        preview: bool,
    },
    /// Print the text of all or selected pages, or save it as a UTF-8 text file with -o.
    Text {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Rebuild a flat picture (PNG, JPEG, WebP, TIFF, GIF or BMP, or one page of a PDF) as editable text, shapes and
    /// pictures, saved as a design project (.pwdesign).
    Design {
        file: PathBuf,
        /// Page of a PDF (1-based).
        #[arg(long, default_value_t = 1)]
        page: u32,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export a design project as editable SVG, as PDF or as a PNG picture (chosen by the output's extension).
    DesignExport {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Convert to an editable Word document (.docx): paragraphs, headings, columns, pictures and
    /// links; a report says what could not be carried over.
    ToDocx {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Convert an Office document (Word, Excel, PowerPoint, OpenDocument, RTF, text, CSV, HTML) to
    /// PDF with the LibreOffice installed on this computer. Macros never run.
    FromOffice {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Translate pages with the local translation models into a PDF with the same layout
    /// (-o file.pdf), a Word document (-o file.docx) or a text file (-o file.txt). Without a
    /// file, list the language directions.
    Translate {
        file: Option<PathBuf>,
        #[arg(long)]
        pages: Option<String>,
        /// Language of the document (detected when absent), e.g. en.
        #[arg(long)]
        from: Option<String>,
        /// Language to translate into, e.g. de.
        #[arg(long)]
        to: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Download a language direction's model (e.g. en-de) into PageWeave's data folder.
        #[arg(long, value_name = "SOURCE-TARGET")]
        install: Option<String>,
        /// Delete a downloaded language direction's model.
        #[arg(long, value_name = "SOURCE-TARGET")]
        remove: Option<String>,
    },
    /// Show which LibreOffice opens Office files; with --install (Windows), download the pinned
    /// LibreOffice package and unpack it into PageWeave's data folder.
    Office {
        #[arg(long)]
        install: bool,
    },
    /// Check whether a newer PageWeave version is published (reads this project's release page on
    /// GitHub; nothing else is sent). The desktop app downloads and installs it.
    Update {
        /// Release check: verify an installer against its signature file and version with the key
        /// built into this program (exit 0 only when valid).
        #[arg(long, hide = true, value_name = "INSTALLER", requires_all = ["signature", "version"])]
        verify: Option<PathBuf>,
        #[arg(long, hide = true)]
        signature: Option<PathBuf>,
        #[arg(long, hide = true)]
        version: Option<String>,
    },
    /// Read pages aloud with the computer's speech voice into a WAV file (-o file.wav). Without
    /// -o, list the voices.
    Speak {
        file: Option<PathBuf>,
        #[arg(long)]
        pages: Option<String>,
        /// A voice id from `speak` without a file (the default voice when absent).
        #[arg(long)]
        voice: Option<String>,
        /// 0.5 to 2 (1 is normal).
        #[arg(long, default_value_t = 1.0)]
        rate: f32,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert to a PowerPoint presentation (.pptx), one slide per page: text boxes, tables,
    /// pictures and links; a report says what could not be carried over.
    ToPptx {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Save the tables (ruled or aligned columns) as an Excel workbook (-o file.xlsx, one sheet per table) or as CSV
    /// files (--format csv, -o an existing folder). Values are kept exactly.
    Tables {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(long, value_enum, default_value = "xlsx")]
        format: SheetFormat,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Find text and report pages and positions.
    Search {
        file: PathBuf,
        query: String,
        #[arg(long)]
        match_case: bool,
        #[arg(long)]
        whole_word: bool,
    },
    /// Render one page to a PNG image.
    Render {
        file: PathBuf,
        #[arg(long, default_value_t = 1)]
        page: u32,
        /// 1.0 = 72 dpi; 2.0 = 144 dpi.
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Combine PDFs in the given order.
    Merge {
        #[arg(required = true, num_args = 2..)]
        files: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Copy selected pages (e.g. "1-3, 5") into a new PDF.
    Extract {
        file: PathBuf,
        #[arg(long)]
        pages: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Write a copy without the selected pages.
    Delete {
        file: PathBuf,
        #[arg(long)]
        pages: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Write a copy with pages rotated (90, 180, 270 or negative).
    Rotate {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        degrees: i32,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Split into several PDFs, every N pages or by ranges.
    Split {
        file: PathBuf,
        #[arg(long, conflicts_with_all = ["ranges", "by_bookmarks", "max_size"])]
        every: Option<u32>,
        /// Repeat for each part, e.g. --ranges 1-3 --ranges 4-
        #[arg(long, conflicts_with_all = ["by_bookmarks", "max_size"])]
        ranges: Vec<String>,
        /// A part per bookmark of this outline level (1 = top level), named after the bookmark.
        #[arg(long, value_name = "LEVEL", conflicts_with = "max_size")]
        by_bookmarks: Option<u32>,
        /// Parts of at most this size, e.g. 500KB, 10MB (1 MB = 1024 KB).
        #[arg(long, value_name = "SIZE")]
        max_size: Option<String>,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Save pages as PNG images in a folder.
    Images {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        /// Resolution in dots per inch (36–600).
        #[arg(long, default_value_t = 150)]
        dpi: u32,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Encrypt a copy with AES-256. The new open password is read from standard input (after the
    /// current password, if --password-stdin is also given); never pass passwords as arguments.
    Protect {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Also read a permissions password from the next line of standard input.
        #[arg(long)]
        owner_password_stdin: bool,
        #[arg(long)]
        no_print: bool,
        #[arg(long)]
        no_copy: bool,
        #[arg(long)]
        no_modify: bool,
        #[arg(long)]
        no_annotate: bool,
        #[arg(long)]
        no_fill_forms: bool,
        #[arg(long)]
        no_assemble: bool,
    },
    /// Make a PDF from images (JPEG, PNG, WebP, TIFF, GIF or BMP), one per page, in the order given. JPEG data is kept exactly; TIFF and GIF give their first page or frame.
    ImagesToPdf {
        images: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        /// a4, letter, or image (the image's own size at its recorded resolution).
        #[arg(long, default_value = "a4")]
        page_size: String,
        /// Space around each image, in points.
        #[arg(long, default_value_t = 0.0)]
        margin: f32,
    },
    /// List embedded files, save one (--save INDEX -o FILE), or attach files (--add FILE … -o OUT).
    Attachments {
        file: PathBuf,
        /// Index (from the list) of the attachment to save.
        #[arg(long, conflicts_with = "add")]
        save: Option<u32>,
        /// Files to attach (repeat).
        #[arg(long)]
        add: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Crop pages by margins as displayed, in a new file (content outside is hidden, not removed).
    Crop {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Pages (range syntax); all pages when absent.
        #[arg(long)]
        pages: Option<String>,
        #[arg(long, default_value_t = 0.0)]
        top: f32,
        #[arg(long, default_value_t = 0.0)]
        right: f32,
        #[arg(long, default_value_t = 0.0)]
        bottom: f32,
        #[arg(long, default_value_t = 0.0)]
        left: f32,
        /// Unit of the margins: pt, mm or in.
        #[arg(long, default_value = "pt")]
        unit: String,
    },
    /// Save the images embedded in a document into a folder (JPEG as stored, others as PNG).
    ExtractImages {
        file: PathBuf,
        #[arg(short, long)]
        output_dir: PathBuf,
        /// Pages (range syntax); all pages when absent.
        #[arg(long)]
        pages: Option<String>,
    },
    /// List blank pages (no text, comments or ink beyond a threshold); remove them with `delete`.
    BlankPages {
        file: PathBuf,
        /// Largest ink share of a blank page, in percent (0–5).
        #[arg(long, default_value_t = 0.2)]
        max_ink: f32,
    },
    /// Place a typed name or a picture of a signature in a box on a page (not a digital signature;
    /// drawn strokes are available through `request`).
    SignatureMark {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Page, from 1.
        #[arg(long, default_value_t = 1)]
        page: u32,
        /// Box X1,Y1,X2,Y2 in the page's unrotated PDF points.
        #[arg(long = "box")]
        rect: String,
        /// Typed name or initials.
        #[arg(long, conflicts_with = "image")]
        text: Option<String>,
        /// Picture of the signature (PNG, JPEG, WebP, TIFF, GIF or BMP).
        #[arg(long)]
        image: Option<PathBuf>,
    },
    /// Sign a copy with a certificate (.pfx/.p12). The certificate password is read from standard
    /// input (after the document password when --password-stdin is given).
    Sign {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// PKCS#12 identity file (.pfx or .p12).
        #[arg(long)]
        certificate: PathBuf,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        location: Option<String>,
        /// Show the signature in a box: PAGE,X1,Y1,X2,Y2 (1-based page, PDF points).
        #[arg(long)]
        visible: Option<String>,
    },
    /// Turn scanned pages that face sideways or upside down upright in a new file (lossless).
    Orient {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Pages (range syntax); all pages when absent.
        #[arg(long)]
        pages: Option<String>,
    },
    /// Straighten skewed scanned pages in a new file (lossless: the page content is turned).
    Deskew {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Pages (range syntax); all pages when absent.
        #[arg(long)]
        pages: Option<String>,
    },
    /// List the editable text runs and pictures of a page (the object indices `edit` refers to).
    Objects {
        file: PathBuf,
        /// 1-based page.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Edit existing text and pictures, or add text, in a copy. The edits file is a JSON array of
    /// `{"page": <0-based>, "edits": [...]}` as in the `edit_objects` operation contract.
    Edit {
        file: PathBuf,
        #[arg(long)]
        edits: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Check the document's digital signatures (integrity; signer trust is not checked).
    Signatures {
        file: PathBuf,
        /// A certificate (DER or PEM) whose holder you trust; repeat for several.
        #[arg(long = "trust")]
        trust: Vec<PathBuf>,
    },
    /// List the differences between two documents (pages, text, format, visual).
    Compare { old: PathBuf, new: PathBuf },
    /// Save a smaller copy with the same content (lossless; images unchanged), verified page by page.
    Optimize {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Also resample images shown above this resolution (lossy), e.g. 150.
        #[arg(long)]
        image_dpi: Option<u32>,
        /// JPEG quality for resampled images (30–95).
        #[arg(long, default_value_t = 75)]
        image_quality: u8,
    },
    /// Add a text watermark, centred on each page, in a new file.
    Watermark {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        text: String,
        /// Pages (range syntax); all pages when absent.
        #[arg(long)]
        pages: Option<String>,
        /// Text size in points (8–400).
        #[arg(long, default_value_t = 72.0)]
        size: f32,
        /// #rrggbb
        #[arg(long, default_value = "#808080")]
        color: String,
        /// Opacity in percent (5–100).
        #[arg(long, default_value_t = 25)]
        opacity: u32,
        /// Degrees counter-clockwise as displayed (−90 to 90).
        #[arg(long, default_value_t = 45.0, allow_hyphen_values = true)]
        angle: f32,
        /// Place it behind the page content instead of over it.
        #[arg(long)]
        behind: bool,
    },
    /// Change document properties in a new file. Options not given keep their value; an empty
    /// value (--author "") removes the property.
    Properties {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        keywords: Option<String>,
    },
    /// Save a copy without password protection (give the current password with --password-stdin).
    /// A file whose owner restricted it needs its permissions password.
    Unprotect {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Recognise text on scanned pages and add it as an invisible, searchable layer in a new file.
    Ocr {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        /// Also process pages that already contain text (by default they are left unchanged).
        #[arg(long)]
        all_pages: bool,
        /// Rendering resolution for recognition (100–400).
        #[arg(long, default_value_t = pw_core::ops::DEFAULT_OCR_DPI)]
        dpi: u32,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Permanently remove everything under the given areas and paint them black, in a new file.
    /// The result is verified before it is written. Use --preview to list what would be removed.
    Redact {
        file: PathBuf,
        /// PAGE:LEFT,BOTTOM,RIGHT,TOP in PDF points (pages from 1), e.g. --area 1:72,700,200,712.
        /// Repeat for several areas.
        #[arg(long = "area", required = true)]
        areas: Vec<String>,
        /// Only list the text and objects under each area; write nothing.
        #[arg(long)]
        preview: bool,
        #[arg(short, long, required_unless_present = "preview")]
        output: Option<PathBuf>,
    },
    /// Remove hidden information in a new, verified file: metadata, scripts and actions, attachments,
    /// hidden layers and comments by default; earlier saved versions always. Use --inspect to list
    /// what the file holds.
    Sanitize {
        file: PathBuf,
        /// Only list the hidden information found; write nothing.
        #[arg(long)]
        inspect: bool,
        #[arg(short, long, required_unless_present = "inspect")]
        output: Option<PathBuf>,
        #[arg(long)]
        keep_metadata: bool,
        #[arg(long)]
        keep_scripts: bool,
        #[arg(long)]
        keep_attachments: bool,
        #[arg(long)]
        keep_hidden_layers: bool,
        #[arg(long)]
        keep_comments: bool,
        /// Also remove form fields and their data.
        #[arg(long)]
        remove_form_fields: bool,
        /// Also remove links to websites.
        #[arg(long)]
        remove_web_links: bool,
    },
    /// Run a typed JSON request (the same schema the local API and MCP use) from a file or "-" for stdin.
    Run { request: PathBuf },
    /// List the operations this build supports.
    Capabilities,
    /// Check the installed resources (PDF engine, text recognition) against their pinned checksums.
    /// Exits 0 when everything is ready and 10 when something is missing or damaged.
    Resources,
    /// Serve PageWeave's tools to an AI agent over MCP (stdio). Files only inside the --root
    /// folders; tools that create files only with --allow-write; existing files are never replaced.
    Mcp {
        /// A folder the agent may use (repeat for several).
        #[arg(long = "root", required = true, value_name = "FOLDER")]
        roots: Vec<PathBuf>,
        /// Also offer tools that create new files inside those folders.
        #[arg(long)]
        allow_write: bool,
        /// Operations run at the same time.
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
        workers: u8,
    },
}

/// "1:0:Text" → (0-based page, index, subtype).
fn parse_annot_ref(s: &str) -> Result<(u32, u32, String), PwError> {
    let bad = || PwError::invalid("annotations.syntax", format!("“{s}”: use PAGE:INDEX:TYPE, e.g. 1:0:Text."));
    let mut p = s.splitn(3, ':');
    let page: u32 = p.next().and_then(|x| x.trim().parse().ok()).filter(|x| *x >= 1).ok_or_else(bad)?;
    let index: u32 = p.next().and_then(|x| x.trim().parse().ok()).ok_or_else(bad)?;
    let subtype = p.next().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).ok_or_else(bad)?;
    Ok((page - 1, index, subtype))
}

/// "5:roman-lower", "17:decimal:1:A-" → a label range (pages 1-based on the command line).
fn parse_label_range(s: &str) -> Result<pw_core::ops::PageLabelRange, PwError> {
    use pw_core::ops::LabelStyle as L;
    let bad = || {
        PwError::invalid(
            "labels.syntax",
            format!("“{s}” is not a label range; use PAGE:STYLE[:FIRST[:PREFIX]], e.g. 5:decimal."),
        )
    };
    let mut parts = s.splitn(4, ':');
    let page: u32 = parts.next().and_then(|p| p.trim().parse().ok()).filter(|p| *p >= 1).ok_or_else(bad)?;
    let style = match parts.next().map(|x| x.trim().to_ascii_lowercase()).as_deref() {
        Some("decimal") => L::Decimal,
        Some("roman-upper") => L::RomanUpper,
        Some("roman-lower") => L::RomanLower,
        Some("letters-upper") => L::LettersUpper,
        Some("letters-lower") => L::LettersLower,
        Some("none") => L::None,
        _ => return Err(bad()),
    };
    let first = match parts.next() {
        Some(f) if !f.trim().is_empty() => f.trim().parse().map_err(|_| bad())?,
        _ => 1,
    };
    let prefix = parts.next().unwrap_or("").to_string();
    Ok(pw_core::ops::PageLabelRange { start: page - 1, style, prefix, first })
}

/// "10MB", "500 KB", "2.5mb", "123456" (bytes) → bytes; 1 KB = 1024 bytes.
fn parse_size(s: &str) -> Result<u64, PwError> {
    let bad = || PwError::invalid("split.size_syntax", format!("“{s}” is not a size; use e.g. 500KB or 10MB."));
    let t = s.trim().to_ascii_lowercase();
    let (num, mult) = [("gb", 1u64 << 30), ("mb", 1 << 20), ("kb", 1 << 10), ("b", 1)]
        .iter()
        .find_map(|(suffix, m)| t.strip_suffix(suffix).map(|n| (n.trim().to_string(), *m)))
        .unwrap_or((t.clone(), 1));
    let v: f64 = num.parse().map_err(|_| bad())?;
    if !v.is_finite() || v <= 0.0 || v * mult as f64 > u64::MAX as f64 / 2.0 {
        return Err(bad());
    }
    Ok((v * mult as f64).round() as u64)
}

fn capabilities() -> serde_json::Value {
    json!({
        "schema_version": pw_core::ops::SCHEMA_VERSION,
        "engine": pw_pdf::engine_version(),
        "operations": pw_core::ops::OPERATION_IDS,
        "collision_policies": ["fail", "rename", "replace"],
        "network": "none",
    })
}

fn build_request(cli: &Cli) -> Result<Option<OperationRequest>, PwError> {
    let op = match &cli.command {
        Command::Capabilities | Command::Mcp { .. } | Command::Resources | Command::Update { .. } => return Ok(None),
        Command::Office { install: true } => Operation::InstallOffice,
        Command::Office { install: false } => return Ok(None),
        Command::Run { request } => {
            let text = if request.as_os_str() == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s).map_err(|e| PwError::from_io(&e, "request"))?;
                s
            } else {
                std::fs::read_to_string(request).map_err(|e| PwError::from_io(&e, "request file"))?
            };
            let req: OperationRequest = serde_json::from_str(&text)
                .map_err(|e| PwError::invalid("request.json", format!("The request is not valid: {e}")))?;
            return Ok(Some(req));
        }
        Command::Inspect { file } => Operation::Inspect { input: file.clone() },
        Command::Fields { file } => Operation::FormFields { input: file.clone() },
        Command::Annotations { file, delete, set_text, set_color, output } => {
            if delete.is_empty() && set_text.is_empty() && set_color.is_empty() {
                Operation::ListAnnotations { input: file.clone() }
            } else {
                let out = output
                    .clone()
                    .ok_or_else(|| PwError::invalid("annotations.output", "Give -o for the changed copy."))?;
                let mut changes = Vec::new();
                for d in delete {
                    let (page, index, subtype) = parse_annot_ref(d)?;
                    changes.push(pw_core::ops::AnnotationChange {
                        page,
                        index,
                        subtype,
                        delete: true,
                        text: None,
                        color: None,
                    });
                }
                for t in set_text {
                    let (r, text) = t.split_once('=').ok_or_else(|| {
                        PwError::invalid("annotations.syntax", format!("“{t}”: use PAGE:INDEX:TYPE=TEXT."))
                    })?;
                    let (page, index, subtype) = parse_annot_ref(r)?;
                    changes.push(pw_core::ops::AnnotationChange {
                        page,
                        index,
                        subtype,
                        delete: false,
                        text: Some(text.to_string()),
                        color: None,
                    });
                }
                for t in set_color {
                    let (r, color) = t.split_once('=').ok_or_else(|| {
                        PwError::invalid("annotations.syntax", format!("“{t}”: use PAGE:INDEX:TYPE=#RRGGBB."))
                    })?;
                    let (page, index, subtype) = parse_annot_ref(r)?;
                    // Several changes to one annotation are one change (text and colour together).
                    match changes.iter_mut().find(|c| c.page == page && c.index == index) {
                        Some(c) => c.color = Some(color.to_string()),
                        None => changes.push(pw_core::ops::AnnotationChange {
                            page,
                            index,
                            subtype,
                            delete: false,
                            text: None,
                            color: Some(color.to_string()),
                        }),
                    }
                }
                Operation::ApplyEdits {
                    input: file.clone(),
                    form_values: vec![],
                    annotations: vec![],
                    object_edits: vec![],
                    annotation_changes: changes,
                    output: out,
                }
            }
        }
        Command::Labels { file, ranges, remove: _, output } => Operation::SetPageLabels {
            input: file.clone(),
            ranges: ranges.iter().map(|r| parse_label_range(r)).collect::<Result<_, _>>()?,
            output: output.clone(),
        },
        Command::Accessibility { file } => Operation::CheckAccessibility { input: file.clone() },
        Command::Patterns { file, kinds, custom, pages } => {
            use pw_core::ops::TextPattern as T;
            let patterns = kinds
                .iter()
                .map(|k| match k {
                    PatternKind::Email => T::Email,
                    PatternKind::Phone => T::Phone,
                    PatternKind::Card => T::CardNumber,
                    PatternKind::Iban => T::Iban,
                    PatternKind::Ssn => T::UsSsn,
                })
                .collect();
            Operation::FindPatterns { input: file.clone(), patterns, custom: custom.clone(), pages: pages.clone() }
        }
        Command::FormExport { file, output, format } => {
            use pw_core::formdata::FormDataFormat as F;
            let by_name = output.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
            let format = match format {
                Some(DataFormat::Json) => F::Json,
                Some(DataFormat::Xfdf) => F::Xfdf,
                None if by_name => F::Json,
                None => F::Xfdf,
            };
            Operation::ExportFormData { input: file.clone(), format, values: vec![], output: output.clone() }
        }
        Command::FormImport { file, data, output: Some(out), preview: false } => {
            Operation::ImportFormData { input: file.clone(), data: data.clone(), output: out.clone() }
        }
        Command::FormImport { file, data, .. } => Operation::ReadFormData { input: file.clone(), data: data.clone() },
        Command::Text { file, pages, output: None } => {
            Operation::ExtractText { input: file.clone(), pages: pages.clone() }
        }
        Command::Text { file, pages, output: Some(out) } => {
            Operation::ExportText { input: file.clone(), pages: pages.clone(), output: out.clone() }
        }
        Command::Tables { file, pages, format, output } => Operation::ExportTables {
            input: file.clone(),
            pages: pages.clone(),
            format: match format {
                SheetFormat::Xlsx => pw_core::ops::TableFormat::Xlsx,
                SheetFormat::Csv => pw_core::ops::TableFormat::Csv,
            },
            output: output.clone(),
        },
        Command::FromOffice { file, output } => {
            Operation::ConvertOfficeToPdf { input: file.clone(), output: output.clone() }
        }
        Command::Speak { file: Some(file), pages, voice, rate, output: Some(output) } => Operation::ReadAloudToWav {
            input: file.clone(),
            pages: pages.clone(),
            voice: voice.clone(),
            rate: *rate,
            output: output.clone(),
        },
        Command::Speak { .. } => {
            return Err(PwError::invalid("usage", "Give a PDF and -o file.wav, or nothing to list the voices."))
        }
        Command::Translate { install: Some(pair), file: None, .. } => {
            let (source, target) = pair
                .split_once('-')
                .ok_or_else(|| PwError::invalid("usage", "Give the direction as SOURCE-TARGET, for example en-de."))?;
            Operation::InstallTranslation { source: source.to_string(), target: target.to_string() }
        }
        Command::Translate {
            file: Some(file),
            pages,
            from,
            to: Some(to),
            output: Some(output),
            install: None,
            remove: None,
        } => Operation::TranslateDocument {
            input: file.clone(),
            pages: pages.clone(),
            source: from.clone(),
            target: to.clone(),
            output: output.clone(),
        },
        Command::Translate { .. } => {
            return Err(PwError::invalid(
                "usage",
                "Give a PDF, --to LANGUAGE and -o file.docx (or .txt), or nothing to list the languages.",
            ))
        }
        Command::ToPptx { file, pages, output } => {
            Operation::ConvertToPptx { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Design { file, page, output } => {
            if *page == 0 {
                return Err(PwError::invalid("usage", "Pages are numbered from 1."));
            }
            Operation::ReconstructDesign { input: file.clone(), page: page - 1, output: output.clone() }
        }
        Command::DesignExport { file, output } => {
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let format = match ext.as_str() {
                "svg" => pw_core::ops::DesignFormat::Svg,
                "pdf" => pw_core::ops::DesignFormat::Pdf,
                "png" => pw_core::ops::DesignFormat::Png,
                _ => return Err(PwError::invalid("usage", "Export to a .svg, .pdf or .png file.")),
            };
            Operation::ExportDesign { input: file.clone(), format, output: output.clone() }
        }
        Command::ToDocx { file, pages, output } => {
            Operation::ConvertToDocx { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Search { file, query, match_case, whole_word } => Operation::Search {
            input: file.clone(),
            query: query.clone(),
            match_case: *match_case,
            whole_word: *whole_word,
        },
        Command::Render { file, page, scale, output } => {
            Operation::Render { input: file.clone(), page: *page, scale: *scale, output: output.clone() }
        }
        Command::Merge { files, output } => Operation::Merge { inputs: files.clone(), output: output.clone() },
        Command::Extract { file, pages, output } => {
            Operation::ExtractPages { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Delete { file, pages, output } => {
            Operation::DeletePages { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Rotate { file, pages, degrees, output } => Operation::RotatePages {
            input: file.clone(),
            pages: pages.clone(),
            degrees: *degrees,
            output: output.clone(),
        },
        Command::Split { file, every, ranges, by_bookmarks, max_size, out_dir } => {
            let split = match (every, ranges.is_empty(), by_bookmarks, max_size) {
                (Some(n), ..) => SplitMode::EveryN { n: *n },
                (None, false, ..) => SplitMode::Ranges { ranges: ranges.clone() },
                (None, true, Some(level), _) => SplitMode::Bookmarks { level: *level },
                (None, true, None, Some(size)) => SplitMode::MaxSize { max_bytes: parse_size(size)? },
                (None, true, None, None) => {
                    return Err(PwError::invalid(
                        "split.mode",
                        "Use --every N, --ranges, --by-bookmarks LEVEL or --max-size SIZE.",
                    ))
                }
            };
            Operation::Split { input: file.clone(), split, output_dir: out_dir.clone() }
        }
        Command::Protect {
            file,
            output,
            no_print,
            no_copy,
            no_modify,
            no_annotate,
            no_fill_forms,
            no_assemble,
            ..
        } => Operation::Protect {
            input: file.clone(),
            permissions: pw_core::ops::ProtectPermissions {
                print: !no_print,
                copy: !no_copy,
                modify: !no_modify,
                annotate: !no_annotate,
                fill_forms: !no_fill_forms,
                assemble: !no_assemble,
            },
            output: output.clone(),
        },
        Command::ImagesToPdf { images, output, page_size, margin } => Operation::ImagesToPdf {
            inputs: images.clone(),
            page_size: match page_size.as_str() {
                "a4" => pw_core::ops::ImagePageSize::A4,
                "letter" => pw_core::ops::ImagePageSize::Letter,
                "image" => pw_core::ops::ImagePageSize::Image,
                _ => return Err(PwError::invalid("images.page_size", "Use --page-size a4, letter or image.")),
            },
            margin: *margin,
            output: output.clone(),
        },
        Command::Attachments { file, save, add, output } => match (save, add.is_empty(), output) {
            (None, true, _) => Operation::ListAttachments { input: file.clone() },
            (Some(i), _, Some(out)) => {
                Operation::ExtractAttachment { input: file.clone(), index: *i, output: out.clone() }
            }
            (None, false, Some(out)) => {
                Operation::AddAttachments { input: file.clone(), files: add.clone(), output: out.clone() }
            }
            _ => return Err(PwError::invalid("attach.output", "Give -o with --save or --add.")),
        },
        Command::ExtractImages { file, output_dir, pages } => {
            Operation::ExtractImages { input: file.clone(), pages: pages.clone(), output_dir: output_dir.clone() }
        }
        Command::BlankPages { file, max_ink } => {
            Operation::FindBlankPages { input: file.clone(), max_ink: max_ink / 100.0 }
        }
        Command::SignatureMark { file, output, page, rect, text, image } => {
            let n: Vec<f32> = rect
                .split(',')
                .map(|x| x.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| PwError::invalid("mark.box", "--box takes X1,Y1,X2,Y2 (numbers)."))?;
            if n.len() != 4 || *page == 0 {
                return Err(PwError::invalid("mark.box", "--box takes X1,Y1,X2,Y2 and --page starts at 1."));
            }
            let mark = match (text, image) {
                (Some(t), None) => pw_core::ops::SignatureMark::Typed {
                    text: t.clone(),
                    style: Default::default(),
                    color: "#1a2b5c".into(),
                },
                (None, Some(p)) => pw_core::ops::SignatureMark::Image { path: p.clone() },
                _ => return Err(PwError::invalid("mark.kind", "Give --text or --image.")),
            };
            Operation::AddSignatureMark {
                input: file.clone(),
                page: page - 1,
                rect: pw_core::ops::RectPt { left: n[0], bottom: n[1], right: n[2], top: n[3] },
                mark,
                output: output.clone(),
            }
        }
        Command::Sign { file, output, certificate, reason, location, visible } => {
            let placement = match visible {
                None => None,
                Some(v) => {
                    let n: Vec<f32> = v
                        .split(',')
                        .map(|x| x.trim().parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| PwError::invalid("sign.box", "--visible takes PAGE,X1,Y1,X2,Y2 (numbers)."))?;
                    if n.len() != 5 || n[0] < 1.0 || n[0].fract() != 0.0 {
                        return Err(PwError::invalid("sign.box", "--visible takes PAGE,X1,Y1,X2,Y2 (page from 1)."));
                    }
                    Some(pw_core::ops::SignPlacement { page: n[0] as u32 - 1, rect: [n[1], n[2], n[3], n[4]] })
                }
            };
            Operation::Sign {
                input: file.clone(),
                certificate: certificate.clone(),
                reason: reason.clone(),
                location: location.clone(),
                placement,
                output: output.clone(),
            }
        }
        Command::Orient { file, output, pages } => {
            Operation::FixOrientation { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Deskew { file, output, pages } => {
            Operation::Deskew { input: file.clone(), pages: pages.clone(), output: output.clone() }
        }
        Command::Objects { file, page } => {
            if *page == 0 {
                return Err(PwError::invalid("page.range", "Pages are numbered from 1."));
            }
            Operation::ListPageObjects { input: file.clone(), page: page - 1 }
        }
        Command::Edit { file, edits, output } => {
            let text = std::fs::read_to_string(edits).map_err(|e| PwError::from_io(&e, "edits file"))?;
            let pages = serde_json::from_str(&text)
                .map_err(|e| PwError::invalid("edit.invalid", format!("The edits file is not valid: {e}")))?;
            Operation::EditObjects { input: file.clone(), pages, output: output.clone() }
        }
        Command::Signatures { file, trust } => {
            Operation::VerifySignatures { input: file.clone(), trusted: trust.clone() }
        }
        Command::Compare { old, new } => Operation::Compare { old: old.clone(), new: new.clone() },
        Command::Optimize { file, output, image_dpi, image_quality } => Operation::Optimize {
            input: file.clone(),
            images: image_dpi.map(|max_dpi| pw_core::ops::ImageReduce { max_dpi, quality: *image_quality }),
            output: output.clone(),
        },
        Command::Crop { file, output, pages, top, right, bottom, left, unit } => {
            let k = match unit.as_str() {
                "pt" => 1.0,
                "mm" => 72.0 / 25.4,
                "in" => 72.0,
                _ => return Err(PwError::invalid("crop.unit", "Use --unit pt, mm or in.")),
            };
            Operation::CropPages {
                input: file.clone(),
                pages: pages.clone(),
                margins: pw_core::ops::Margins { top: top * k, right: right * k, bottom: bottom * k, left: left * k },
                output: output.clone(),
            }
        }
        Command::Watermark { file, output, text, pages, size, color, opacity, angle, behind } => Operation::Watermark {
            input: file.clone(),
            pages: pages.clone(),
            watermark: pw_core::ops::WatermarkSpec {
                text: text.clone(),
                font_size: *size,
                color: color.clone(),
                opacity: *opacity as f32 / 100.0,
                angle: *angle,
                layer: if *behind { pw_core::ops::WatermarkLayer::Behind } else { pw_core::ops::WatermarkLayer::Over },
            },
            output: output.clone(),
        },
        Command::Properties { file, output, title, author, subject, keywords } => Operation::SetProperties {
            input: file.clone(),
            title: title.clone(),
            author: author.clone(),
            subject: subject.clone(),
            keywords: keywords.clone(),
            output: output.clone(),
        },
        Command::Unprotect { file, output } => Operation::Unprotect { input: file.clone(), output: output.clone() },
        Command::Ocr { file, pages, all_pages, dpi, output } => Operation::OcrPages {
            input: file.clone(),
            pages: pages.clone(),
            skip_text_pages: !all_pages,
            dpi: *dpi,
            output: output.clone(),
        },
        Command::Redact { file, areas, preview, output } => {
            let areas = areas.iter().map(|a| parse_area(a)).collect::<Result<Vec<_>, _>>()?;
            match (preview, output) {
                (false, Some(out)) => Operation::Redact { input: file.clone(), areas, output: out.clone() },
                _ => Operation::RedactionPreview { input: file.clone(), areas },
            }
        }
        Command::Sanitize {
            file,
            inspect,
            output,
            keep_metadata,
            keep_scripts,
            keep_attachments,
            keep_hidden_layers,
            keep_comments,
            remove_form_fields,
            remove_web_links,
        } => match (inspect, output) {
            (false, Some(out)) => Operation::Sanitize {
                input: file.clone(),
                options: pw_core::ops::SanitizeOptions {
                    metadata: !keep_metadata,
                    scripts: !keep_scripts,
                    attachments: !keep_attachments,
                    hidden_layers: !keep_hidden_layers,
                    comments: !keep_comments,
                    form_fields: *remove_form_fields,
                    web_links: *remove_web_links,
                },
                output: out.clone(),
            },
            _ => Operation::SanitizeInspect { input: file.clone() },
        },
        Command::Images { file, pages, dpi, out_dir } => Operation::ExportImages {
            input: file.clone(),
            pages: pages.clone(),
            dpi: *dpi,
            output_dir: out_dir.clone(),
        },
    };
    let mut req = OperationRequest::new(op);
    req.collision = match cli.on_conflict {
        Conflict::Fail => CollisionPolicy::Fail,
        Conflict::Rename => CollisionPolicy::Rename,
        Conflict::Replace => CollisionPolicy::Replace,
    };
    Ok(Some(req))
}

/// `PAGE:LEFT,BOTTOM,RIGHT,TOP` (page from 1) → area on the 0-based page.
fn parse_area(s: &str) -> Result<pw_core::ops::RedactArea, PwError> {
    let bad =
        || PwError::invalid("redact.area_syntax", format!("Use PAGE:LEFT,BOTTOM,RIGHT,TOP for an area, not {s:?}."));
    let (page, rect) = s.split_once(':').ok_or_else(bad)?;
    let page: u32 = page.trim().parse().map_err(|_| bad())?;
    let v: Vec<f32> = rect.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<_, _>>().map_err(|_| bad())?;
    if page == 0 || v.len() != 4 {
        return Err(bad());
    }
    Ok(pw_core::ops::RedactArea {
        page: page - 1,
        rect: pw_core::ops::RectPt { left: v[0], bottom: v[1], right: v[2], top: v[3] },
    })
}

fn print_human(r: &OperationResult) {
    match r.operation.as_str() {
        "inspect" => {
            let d = &r.data;
            println!("Pages:      {}", d["page_count"]);
            println!("PDF:        {}", d["pdf_version"].as_str().unwrap_or("?"));
            if let Some(t) = d["title"].as_str() {
                println!("Title:      {t}");
            }
            if let Some(a) = d["author"].as_str() {
                println!("Author:     {a}");
            }
            println!("Encrypted:  {}", if d["encrypted"].as_bool() == Some(true) { "yes" } else { "no" });
            println!("Bookmarks:  {}", d["bookmarks"].as_array().map(|b| b.len()).unwrap_or(0));
            println!("Size:       {} bytes", d["file_bytes"]);
        }
        "form_fields" => {
            let fields = r.data["fields"].as_array().cloned().unwrap_or_default();
            if fields.is_empty() {
                println!("No form fields.");
            }
            for f in fields {
                let mut line =
                    format!("{}  ({})", f["name"].as_str().unwrap_or("?"), f["kind"].as_str().unwrap_or("?"));
                match f["kind"].as_str() {
                    Some("checkbox") => {
                        let on = f["widgets"].as_array().is_some_and(|w| w.iter().any(|w| w["checked"] == true));
                        line += if on { " = checked" } else { " = unchecked" };
                    }
                    Some("radio") => {
                        let opts: Vec<String> = f["widgets"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|w| {
                                let v = w["export_value"].as_str().unwrap_or("?");
                                if w["checked"] == true {
                                    format!("[{v}]")
                                } else {
                                    v.to_string()
                                }
                            })
                            .collect();
                        line += &format!(" options: {}", opts.join(" "));
                    }
                    _ => {
                        line += &format!(" = {:?}", f["value"].as_str().unwrap_or(""));
                        if let Some(o) = f["options"].as_array().filter(|o| !o.is_empty()) {
                            let o: Vec<&str> = o.iter().filter_map(|x| x.as_str()).collect();
                            line += &format!(" options: {}", o.join(" | "));
                        }
                    }
                }
                if f["read_only"] == true {
                    line += " (read-only)";
                }
                println!("{line}");
            }
        }
        "find_blank_pages" => {
            let blank: Vec<u64> = r.data["blank"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()).collect();
            if blank.is_empty() {
                println!("No blank pages.");
            } else {
                let list: Vec<String> = blank.iter().map(|p| (p + 1).to_string()).collect();
                println!("Blank pages: {}", list.join(", "));
                println!("Remove them with: pageweave delete --pages \"{}\" -o OUTPUT FILE", list.join(","));
            }
        }
        "sign" => {
            let d = &r.data;
            println!(
                "Signed as {} ({}, issued by {}).",
                d["signer"].as_str().unwrap_or("?"),
                d["algorithm"].as_str().unwrap_or("?"),
                d["issuer"].as_str().unwrap_or("?")
            );
        }
        "verify_signatures" => {
            let list = r.data["signatures"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("No digital signatures.");
            }
            for (i, s) in list.iter().enumerate() {
                let get = |k: &str| s[k].as_str().unwrap_or("").to_string();
                println!(
                    "Signature {} ({}): {}",
                    i + 1,
                    if get("field").is_empty() { "unnamed".into() } else { get("field") },
                    get("status").to_uppercase()
                );
                println!("  {}", get("detail"));
                if s["changed_after"] == true {
                    println!("  The document was changed after this signature (later revision).");
                }
                if get("status") == "valid" {
                    println!("  Trust: {} — {}", get("trust").replace('_', " "), get("trust_detail"));
                }
                for (label, key) in [
                    ("Signer", "signer"),
                    ("Issuer", "issuer"),
                    ("Signed", "signing_time"),
                    ("Reason", "reason"),
                    ("Algorithm", "signature_algorithm"),
                ] {
                    let v = get(key);
                    if !v.is_empty() {
                        println!("  {label}: {v}");
                    }
                }
            }
            if !list.is_empty() {
                println!("Note: {}", r.data["trust_note"].as_str().unwrap_or(""));
            }
        }
        "compare" => {
            let d = &r.data;
            let s = &d["summary"];
            println!(
                "Old: {} pages, new: {} pages. Pages changed: {}, inserted: {}, deleted: {}, moved: {}.",
                d["old_pages"],
                d["new_pages"],
                s["pages_changed"],
                s["pages_inserted"],
                s["pages_deleted"],
                s["pages_moved"]
            );
            let page = |v: &serde_json::Value| v.as_u64().map(|n| (n + 1).to_string()).unwrap_or_else(|| "-".into());
            for p in d["pairs"].as_array().into_iter().flatten() {
                let status = p["status"].as_str().unwrap_or("?");
                if status == "same" {
                    continue;
                }
                println!("page {} -> {}: {status}", page(&p["old"]), page(&p["new"]));
                for c in p["changes"].as_array().into_iter().flatten() {
                    let kind = c["kind"].as_str().unwrap_or("?");
                    let text = c["text"].as_str().unwrap_or("");
                    let mut line = format!("  {kind}");
                    if let Some(n) = c["new_text"].as_str() {
                        line += &format!(": \"{text}\" -> \"{n}\"");
                    } else if !text.is_empty() {
                        line += &format!(": \"{text}\"");
                    }
                    if let Some(x) = c["detail"].as_str() {
                        line += &format!(" ({x})");
                    }
                    println!("{line}");
                }
            }
            for m in d["metadata"].as_array().into_iter().flatten() {
                println!(
                    "{}: {:?} -> {:?}",
                    m["field"].as_str().unwrap_or("?"),
                    m["old"].as_str().unwrap_or(""),
                    m["new"].as_str().unwrap_or("")
                );
            }
        }
        "extract_text" => {
            for p in r.data["pages"].as_array().into_iter().flatten() {
                println!("--- page {} ---", p["page"]);
                println!("{}", p["text"].as_str().unwrap_or(""));
            }
        }
        "export_tables" => {
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            if let Some(found) = r.data["found"].as_array() {
                for t in found {
                    println!("Page {} table {}: {} rows × {} columns", t[0], t[1], t[2], t[3]);
                }
            }
        }
        "convert_office_to_pdf" => {
            for o in &r.outputs {
                println!("Wrote {} ({} bytes, {} pages)", o.path.display(), o.bytes, o.pages.unwrap_or(0));
            }
        }
        "install_translation" => {
            let d = &r.data;
            println!(
                "{}-{} ready in {} ({} MB downloaded or repaired)",
                d["source"].as_str().unwrap_or(""),
                d["target"].as_str().unwrap_or(""),
                d["folder"].as_str().unwrap_or(""),
                d["downloaded_bytes"].as_u64().unwrap_or(0) / 1_000_000
            );
        }
        "translate_document" => {
            let d = &r.data;
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            println!(
                "{} → {}{}{}: {} paragraphs in {:.1} s; numbers/codes kept {}/{}",
                d["source"].as_str().unwrap_or(""),
                d["target"].as_str().unwrap_or(""),
                if d["detected"] == true { " (detected)" } else { "" },
                if d["via_english"] == true { " through English" } else { "" },
                d["paragraphs"],
                d["millis"].as_f64().unwrap_or(0.0) / 1000.0,
                d["protected_kept"],
                d["protected_total"]
            );
        }
        "read_aloud_to_wav" => {
            let d = &r.data;
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            println!(
                "Pages: {}, sentences: {}, length: {:.1} s, voice: {} ({})",
                d["pages"],
                d["sentences"],
                d["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
                d["voice"].as_str().unwrap_or(""),
                d["engine"].as_str().unwrap_or("")
            );
        }
        "convert_to_pptx" => {
            let d = &r.data;
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            println!(
                "Slides: {}, tables: {}, pictures: {}, links: {}",
                d["pages"], d["tables"], d["pictures"], d["links"]
            );
        }
        "convert_to_docx" => {
            let d = &r.data;
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            println!(
                "Pages: {}, paragraphs: {}, headings: {}, tables: {}, pictures: {}, links: {}",
                d["pages"], d["paragraphs"], d["headings"], d["tables"], d["pictures"], d["links"]
            );
        }
        "ocr_pages" => {
            let d = &r.data;
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            println!(
                "Pages recognised: {}, skipped (already had text): {}, lines: {}, mean confidence: {:.1}%",
                d["pages_processed"],
                d["pages_skipped_with_text"],
                d["lines"],
                d["mean_confidence"].as_f64().unwrap_or(0.0) * 100.0
            );
        }
        "list_attachments" => {
            let list = r.data["attachments"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("No attachments.");
            }
            for a in list {
                println!("{:>4}  {:>12} bytes  {}", a["index"], a["bytes"], a["name"].as_str().unwrap_or(""));
            }
        }
        "sanitize_inspect" | "sanitize" => {
            for o in &r.outputs {
                println!("Wrote {} ({} bytes)", o.path.display(), o.bytes);
            }
            let d = if r.operation == "sanitize" { &r.data["removed"] } else { &r.data };
            let list = |v: &serde_json::Value| -> String {
                let items: Vec<&str> = v.as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
                if items.is_empty() {
                    "none".into()
                } else {
                    items.join(", ")
                }
            };
            let count = |v: &serde_json::Value| v.as_u64().unwrap_or(0);
            if r.operation == "sanitize" {
                println!("Removed:");
            }
            println!("  Metadata:            {}", list(&d["metadata"]));
            println!("  Scripts and actions: {}", count(&d["scripts"]));
            let names = list(&d["attachment_names"]);
            let names = if names == "none" { String::new() } else { format!(" ({names})") };
            println!("  Attachments:         {}{names}", count(&d["attachments"]));
            println!("  Hidden layers:       {} (of {} layers)", list(&d["hidden_layers"]), count(&d["layers"]));
            println!("  Comments:            {}", count(&d["comments"]));
            println!("  Form fields:         {}", count(&d["form_fields"]));
            println!("  Web links:           {}", count(&d["web_links"]));
            let earlier = if d["earlier_versions"] == true { "yes" } else { "none" };
            println!("  Earlier versions:    {earlier}");
        }
        "redaction_preview" => {
            for a in r.data["areas"].as_array().into_iter().flatten() {
                println!(
                    "page {}: {:?} (images {}, graphics {}, annotations {})",
                    a["page"].as_u64().unwrap_or(0) + 1,
                    a["text"].as_str().unwrap_or(""),
                    a["images"],
                    a["graphics"],
                    a["annotations"]
                );
            }
        }
        "list_annotations" => {
            let list = r.data["annotations"].as_array().cloned().unwrap_or_default();
            for a in &list {
                let who = a["author"].as_str().map(|w| format!(" — {w}")).unwrap_or_default();
                println!(
                    "page {}  #{:<3} {:<15} {}{who}",
                    a["page"].as_u64().unwrap_or(0) + 1,
                    a["index"].as_u64().unwrap_or(0),
                    a["subtype"].as_str().unwrap_or(""),
                    a["text"].as_str().unwrap_or("")
                );
            }
            println!("{} annotation(s)", list.len());
        }
        "check_accessibility" => {
            for c in r.data["checks"].as_array().into_iter().flatten() {
                let status = match c["status"].as_str().unwrap_or("") {
                    "pass" => "PASS",
                    "fail" => "FAIL",
                    "warning" => "WARN",
                    "manual" => "REVIEW",
                    _ => "N/A",
                };
                println!("{status:<7} {}: {}", c["title"].as_str().unwrap_or(""), c["detail"].as_str().unwrap_or(""));
                let pages: Vec<String> = c["pages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_u64())
                    .map(|p| (p + 1).to_string())
                    .collect();
                if !pages.is_empty() {
                    println!("        pages: {}", pages.join(", "));
                }
                let names: Vec<&str> = c["names"].as_array().into_iter().flatten().filter_map(|n| n.as_str()).collect();
                if !names.is_empty() {
                    println!("        {}", names.join(", "));
                }
            }
            let d = &r.data;
            println!(
                "{} passed, {} failed, {} warnings, {} to review by hand (automated checks only; not a PDF/UA or WCAG verdict)",
                d["passed"], d["failed"], d["warnings"], d["manual"]
            );
        }
        "find_patterns" => {
            let hits = r.data["hits"].as_array().cloned().unwrap_or_default();
            for h in &hits {
                println!(
                    "page {}  {:<11}  {}",
                    h["page"].as_u64().unwrap_or(0) + 1,
                    h["kind"].as_str().unwrap_or(""),
                    h["text"].as_str().unwrap_or("")
                );
            }
            println!("{} match(es)", hits.len());
        }
        "search" => {
            for h in r.data["hits"].as_array().into_iter().flatten() {
                println!("page {}: {}", h["page"].as_u64().unwrap_or(0) + 1, h["text"].as_str().unwrap_or(""));
            }
            println!("{} match(es)", r.data["count"]);
        }
        _ => {
            for o in &r.outputs {
                match o.pages {
                    Some(n) => println!("Wrote {} ({n} pages, {} bytes)", o.path.display(), o.bytes),
                    None => println!("Wrote {} ({} bytes)", o.path.display(), o.bytes),
                }
            }
        }
    }
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
}

/// Stack for the CLI's work. Windows gives the main thread 1 MB, which a debug build's argument
/// parsing outgrew (CI: "thread 'main' has overflowed its stack"); every platform gets the same.
const CLI_STACK_BYTES: usize = 16 * 1024 * 1024;

fn main() -> ExitCode {
    // Worker mode (D-015): one operation over stdin/stdout for a parent process.
    if std::env::args_os().nth(1).is_some_and(|a| a == pw_pdf::worker::WORKER_FLAG) {
        return match std::thread::Builder::new().name("pageweave-worker".into()).stack_size(CLI_STACK_BYTES).spawn(
            || {
                pw_pdf::worker::run_worker();
                ExitCode::SUCCESS
            },
        ) {
            Ok(h) => h.join().unwrap_or(ExitCode::from(70)),
            Err(_) => ExitCode::from(71),
        };
    }
    // Reader mode (D-018): read-only calls for a parent process, until stdin closes.
    if std::env::args_os().nth(1).is_some_and(|a| a == pw_pdf::service::READER_FLAG) {
        return match std::thread::Builder::new().name("pageweave-reader".into()).stack_size(CLI_STACK_BYTES).spawn(
            || {
                pw_pdf::service::run_reader();
                ExitCode::SUCCESS
            },
        ) {
            Ok(h) => h.join().unwrap_or(ExitCode::from(70)),
            Err(_) => ExitCode::from(71),
        };
    }
    match std::thread::Builder::new().name("pageweave-cli".into()).stack_size(CLI_STACK_BYTES).spawn(run) {
        Ok(h) => h.join().unwrap_or(ExitCode::from(70)),
        Err(_) => ExitCode::from(71),
    }
}

fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 1 } else { 0 });
        }
    };
    if let Command::Mcp { roots, allow_write, workers } = &cli.command {
        // stdout carries the protocol only; problems starting up go to stderr.
        let roots = match pw_mcp::Roots::new(roots) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {}", e.message);
                return ExitCode::from(e.category.exit_code().clamp(2, 255) as u8);
            }
        };
        let cfg = pw_mcp::Config { roots, allow_write: *allow_write, workers: *workers as usize };
        pw_mcp::serve(cfg, std::io::stdin().lock(), std::io::stdout());
        return ExitCode::SUCCESS;
    }
    if let Command::Resources = &cli.command {
        return match pw_pdf::resources::resource_status() {
            Ok(r) => {
                if cli.json {
                    println!("{}", json!({"ok": true, "result": r}));
                } else {
                    for c in &r.capabilities {
                        let state = match c.state {
                            pw_pdf::resources::ResourceState::Ready => "READY",
                            pw_pdf::resources::ResourceState::Missing => "MISSING",
                            pw_pdf::resources::ResourceState::Damaged => "DAMAGED",
                        };
                        println!("{state:<8} {}: {} ({} bytes)", c.title, c.detail, c.bytes);
                    }
                }
                use pw_pdf::resources::ResourceState as S;
                // Optional capabilities (e.g. Office documents) never make the exit code fail.
                let worst =
                    r.capabilities.iter().filter(|c| c.required).map(|c| c.state).fold(S::Ready, |a, b| match (a, b) {
                        (S::Damaged, _) | (_, S::Damaged) => S::Damaged,
                        (S::Missing, _) | (_, S::Missing) => S::Missing,
                        _ => S::Ready,
                    });
                let cat = match worst {
                    S::Ready => return ExitCode::SUCCESS,
                    S::Missing => pw_core::ErrorCategory::MissingResource,
                    S::Damaged => pw_core::ErrorCategory::CorruptResource,
                };
                ExitCode::from(cat.exit_code().clamp(2, 255) as u8)
            }
            Err(e) => {
                eprintln!("error: {}", e.message);
                ExitCode::from(e.category.exit_code().clamp(2, 255) as u8)
            }
        };
    }
    let outcome = (|| -> Result<serde_json::Value, PwError> {
        if let Command::Translate { remove: Some(pair), file: None, install: None, .. } = &cli.command {
            let (source, target) = pair
                .split_once('-')
                .ok_or_else(|| PwError::invalid("usage", "Give the direction as SOURCE-TARGET, for example en-de."))?;
            let removed = pw_translate::install::remove(source, target)?;
            let shared: Vec<String> =
                pw_translate::sharing(source, target)?.into_iter().map(|(s, t)| format!("{s}-{t}")).collect();
            if !cli.json {
                if removed && shared.len() > 1 {
                    println!("Removed. The same model also served: {}.", shared.join(", "));
                } else if removed {
                    println!("Removed.");
                } else {
                    println!("It was not downloaded (shipped or development models are never removed).");
                }
            }
            return Ok(json!({ "removed": removed, "directions": if removed { shared } else { Vec::new() } }));
        }
        if let Command::Translate { file: None, output: None, to: None, install: None, .. } = &cli.command {
            let dirs = pw_translate::directions()?;
            if !cli.json {
                for d in &dirs {
                    println!(
                        "{} {}-{}  {} to {} ({} MB)",
                        if d.installed { "READY     " } else { "DOWNLOAD  " },
                        d.source,
                        d.target,
                        pw_translate::language_name(&d.source),
                        pw_translate::language_name(&d.target),
                        d.bytes / 1_000_000
                    );
                }
                let langs: std::collections::BTreeSet<&str> =
                    dirs.iter().flat_map(|d| [d.source.as_str(), d.target.as_str()]).collect();
                if dirs.len() < langs.len() * langs.len().saturating_sub(1) {
                    println!("Other pairs of these languages are translated through English.");
                }
                println!("Download one with: pageweave translate --install en-de");
            }
            return Ok(json!({ "directions": dirs }));
        }
        if let Command::Office { install: false } = &cli.command {
            let found = pw_pdf::office::find_office();
            let pkg = pw_pdf::office::office_package()?;
            if !cli.json {
                match &found {
                    Some(e) => println!("{} at {}", e.version, e.program.display()),
                    None if cfg!(windows) => println!(
                        "LibreOffice is not installed. Download it with: pageweave office --install ({} MB)",
                        pkg.bytes / 1_000_000
                    ),
                    None => println!("LibreOffice is not installed. Install it with your package manager."),
                }
            }
            return Ok(json!({ "engine": found, "package": pkg }));
        }
        if let Command::Update { verify: Some(file), signature: Some(sig), version: Some(v) } = &cli.command {
            let key = pw_update::public_key().ok_or_else(|| {
                PwError::new(
                    pw_core::ErrorCategory::SecurityPolicy,
                    "update.unsigned_build",
                    "This build has no release key.",
                )
            })?;
            let sig = std::fs::read_to_string(sig).map_err(|e| PwError::from_io(&e, "signature file"))?;
            pw_update::verify_file(file, &sig, key, v)?;
            if !cli.json {
                println!("Signature valid for version {v}.");
            }
            return Ok(json!({ "valid": true, "version": v }));
        }
        if let Command::Update { .. } = &cli.command {
            let found = pw_update::check(env!("CARGO_PKG_VERSION"))?;
            if !cli.json {
                match &found {
                    Some(u) => println!(
                        "PageWeave {} is available (you have {}; {} MB). Install it from the app: Settings ▸ Updates.",
                        u.version,
                        u.current,
                        u.bytes.div_ceil(1_000_000)
                    ),
                    None => println!("PageWeave {} is up to date.", env!("CARGO_PKG_VERSION")),
                }
            }
            return Ok(json!({ "current": env!("CARGO_PKG_VERSION"), "update": found }));
        }
        if let Command::Speak { file: None, output: None, .. } = &cli.command {
            let info = pw_speech::engine_info()?;
            if !cli.json {
                for v in &info.voices {
                    println!("{} {}  {}  {}", if v.default { "*" } else { " " }, v.id, v.language, v.name);
                }
                let by = match info.precision {
                    pw_speech::Precision::Word => "word",
                    pw_speech::Precision::Sentence => "sentence",
                };
                println!("Engine: {} (timings by {by})", info.name);
            }
            return Ok(serde_json::to_value(info).unwrap_or_default());
        }
        let Some(mut req) = build_request(&cli)? else {
            return Ok(capabilities());
        };
        // Passwords come only from standard input, one per line, in this order: current document
        // password (--password-stdin), new open password (protect), permissions password.
        let read_line = || -> Result<String, PwError> {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map_err(|e| PwError::from_io(&e, "password"))?;
            Ok(line.trim_end_matches(['\r', '\n']).to_string())
        };
        if cli.password_stdin {
            req.password = Some(read_line()?);
        }
        if let Command::Sign { .. } = &cli.command {
            req.certificate_password = Some(read_line()?);
            req.validate()?;
        }
        if let Command::Protect { owner_password_stdin, .. } = &cli.command {
            req.new_password = Some(read_line()?);
            if *owner_password_stdin {
                req.new_owner_password = Some(read_line()?);
            }
            req.validate()?;
        }
        let result = pw_pdf::execute(&req, None)?;
        if !cli.json {
            print_human(&result);
        }
        Ok(serde_json::to_value(result).unwrap_or_default())
    })();
    match outcome {
        Ok(v) => {
            if cli.json {
                println!("{}", json!({"ok": true, "result": v}));
            } else if matches!(cli.command, Command::Capabilities) {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if cli.json {
                println!("{}", json!({"ok": false, "error": e}));
            } else {
                eprintln!("error: {}", e.message);
                eprintln!("{}", e.action);
            }
            ExitCode::from(e.category.exit_code().clamp(2, 255) as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn sizes_parse_with_binary_units() {
        assert_eq!(parse_size("10MB").ok(), Some(10 << 20));
        assert_eq!(parse_size(" 500 kb ").ok(), Some(500 << 10));
        assert_eq!(parse_size("2.5mb").ok(), Some((2.5 * (1u64 << 20) as f64) as u64));
        assert_eq!(parse_size("123456").ok(), Some(123_456));
        assert_eq!(parse_size("1GB").ok(), Some(1 << 30));
        for bad in ["", "MB", "-1MB", "0", "ten", "1e400"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }
}

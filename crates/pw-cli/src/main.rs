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
    /// Make a PDF from JPEG and PNG images, one per page, in the order given.
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
        /// PNG or JPEG picture of the signature.
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
        Command::Capabilities => return Ok(None),
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
    let outcome = (|| -> Result<serde_json::Value, PwError> {
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

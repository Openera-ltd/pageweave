//! `pageweave` — scriptable CLI over the shared engine (PRD 17 "CLI and local API").
//!
//! Every command builds a typed `OperationRequest` and calls the same `pw_pdf::execute` the desktop
//! app uses. Results are printed as human text or, with `--json`, as a stable JSON envelope.
//! Exit codes come from the error category (see `ErrorCategory::exit_code`); 0 is success and 1 is
//! a usage error. Passwords are only accepted on stdin (`--password-stdin`), never as arguments.

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

#[derive(Subcommand)]
enum Command {
    /// Show page count, sizes, metadata, bookmarks and security status.
    Inspect { file: PathBuf },
    /// List form fields (names, kinds, values, options) as JSON.
    Fields { file: PathBuf },
    /// Print the text of all or selected pages.
    Text {
        file: PathBuf,
        #[arg(long)]
        pages: Option<String>,
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
        #[arg(long, conflicts_with = "ranges")]
        every: Option<u32>,
        /// Repeat for each part, e.g. --ranges 1-3 --ranges 4-
        #[arg(long)]
        ranges: Vec<String>,
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

fn capabilities() -> serde_json::Value {
    json!({
        "schema_version": pw_core::ops::SCHEMA_VERSION,
        "engine": pw_pdf::engine_version(),
        "operations": ["inspect", "extract_text", "search", "render", "merge", "extract_pages", "delete_pages", "rotate_pages", "split", "annotate", "form_fields", "fill_form", "apply_edits", "organize_pages", "stamp_text", "export_images", "redaction_preview", "redact", "ocr_pages", "protect", "unprotect", "set_properties", "sanitize_inspect", "sanitize"],
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
        Command::Text { file, pages } => Operation::ExtractText { input: file.clone(), pages: pages.clone() },
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
        Command::Split { file, every, ranges, out_dir } => {
            let split = match (every, ranges.is_empty()) {
                (Some(n), _) => SplitMode::EveryN { n: *n },
                (None, false) => SplitMode::Ranges { ranges: ranges.clone() },
                (None, true) => return Err(PwError::invalid("split.mode", "Use --every N or --ranges.")),
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

fn main() -> ExitCode {
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

//! The tools an agent may call. Each one is a thin, typed mapping onto an existing
//! [`pw_core::ops::Operation`]; the engine does the work exactly as for the app and the CLI.

use crate::paths::Roots;
use pw_core::ops::{FieldValue, FormValue, ImageReduce, Operation, SplitMode};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// What a tool does to files, as declared in its listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads approved files only.
    ReadOnly,
    /// Writes new files inside approved folders; never replaces or changes existing files.
    WritesNewFiles,
}

pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// May run long enough to report progress (and be cancelled).
    pub long: bool,
    pub schema: fn() -> Value,
}

fn path_prop(what: &str) -> Value {
    json!({ "type": "string", "description": format!("{what} (absolute, or relative to the first approved folder)") })
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

const PAGES: &str = "Pages, e.g. \"1-3, 5, 8-\" (1-based). All pages when left out.";

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "list_capabilities",
        title: "List capabilities",
        description: "What this PageWeave server can do: its tools, approved folders and whether writing is allowed.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({}), &[]),
    },
    Tool {
        name: "inspect_document",
        title: "Inspect a PDF",
        description: "Page count, page sizes and rotation, metadata, bookmarks, form type, encryption and attachments of a PDF.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({ "path": path_prop("PDF file") }), &["path"]),
    },
    Tool {
        name: "extract_text",
        title: "Extract text",
        description: "Plain text of all or chosen pages, per page. Scanned pages have no text until they are recognized (recognize_text).",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({ "path": path_prop("PDF file"), "pages": { "type": "string", "description": PAGES } }), &["path"]),
    },
    Tool {
        name: "search_text",
        title: "Find text",
        description: "Find text in a PDF; returns each match's page and position.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || {
            obj(
                json!({
                    "path": path_prop("PDF file"),
                    "query": { "type": "string", "minLength": 1, "maxLength": 1000 },
                    "match_case": { "type": "boolean", "default": false },
                    "whole_word": { "type": "boolean", "default": false }
                }),
                &["path", "query"],
            )
        },
    },
    Tool {
        name: "list_form_fields",
        title: "List form fields",
        description: "The PDF's fillable form fields: names, kinds, current values, options and whether each is read-only or required.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({ "path": path_prop("PDF file") }), &["path"]),
    },
    Tool {
        name: "compare_documents",
        title: "Compare two PDFs",
        description: "Differences between an older and a newer PDF: aligned pages, inserted/deleted/changed words, moved passages, formatting and visual changes.",
        effect: Effect::ReadOnly,
        long: true,
        schema: || obj(json!({ "old": path_prop("Older PDF"), "new": path_prop("Newer PDF") }), &["old", "new"]),
    },
    Tool {
        name: "check_signatures",
        title: "Check digital signatures",
        description: "Whether each digital signature is intact and its signer's signature is valid. Signer trust is not judged.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({ "path": path_prop("PDF file") }), &["path"]),
    },
    Tool {
        name: "check_accessibility",
        title: "Check accessibility",
        description: "Automated accessibility checks of a PDF (tags, language, title, alternate text for pictures, table headers, headings, form field labels, link descriptions, tab order, embedded fonts, text on every page) plus the items a person must review. Not a PDF/UA or WCAG conformance verdict.",
        effect: Effect::ReadOnly,
        long: false,
        schema: || obj(json!({ "path": path_prop("PDF file") }), &["path"]),
    },
    Tool {
        name: "find_sensitive_text",
        title: "Find sensitive text",
        description: "Find e-mail addresses, phone numbers, payment card numbers (Luhn-checked), IBANs (checksum-checked), US Social Security numbers and/or a custom regular expression, with the page and position of each match. Matches are candidates for review, not a guarantee that nothing else is sensitive.",
        effect: Effect::ReadOnly,
        long: true,
        schema: || {
            obj(
                json!({
                    "path": path_prop("PDF file"),
                    "kinds": { "type": "array", "items": { "type": "string", "enum": ["email", "phone", "card_number", "iban", "us_ssn"] }, "uniqueItems": true },
                    "custom": { "type": "string", "maxLength": 500, "description": "Regular expression (Rust regex syntax)" },
                    "pages": { "type": "string", "description": PAGES }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "extract_pages",
        title: "Extract pages",
        description: "Save chosen pages, in the given order, as a new PDF. The source is not changed.",
        effect: Effect::WritesNewFiles,
        long: false,
        schema: || {
            obj(
                json!({ "path": path_prop("Source PDF"), "pages": { "type": "string", "description": "Pages, e.g. \"1-3, 5\"" }, "output": path_prop("New PDF file (must not exist)") }),
                &["path", "pages", "output"],
            )
        },
    },
    Tool {
        name: "delete_pages",
        title: "Delete pages",
        description: "Save a copy without the chosen pages as a new PDF. The source is not changed.",
        effect: Effect::WritesNewFiles,
        long: false,
        schema: || {
            obj(
                json!({ "path": path_prop("Source PDF"), "pages": { "type": "string", "description": "Pages to leave out" }, "output": path_prop("New PDF file (must not exist)") }),
                &["path", "pages", "output"],
            )
        },
    },
    Tool {
        name: "rotate_pages",
        title: "Rotate pages",
        description: "Save a copy with pages turned clockwise by 90, 180 or 270 degrees (negative turns counter-clockwise).",
        effect: Effect::WritesNewFiles,
        long: false,
        schema: || {
            obj(
                json!({
                    "path": path_prop("Source PDF"),
                    "pages": { "type": "string", "description": PAGES },
                    "degrees": { "type": "integer", "enum": [90, 180, 270, -90, -180, -270] },
                    "output": path_prop("New PDF file (must not exist)")
                }),
                &["path", "degrees", "output"],
            )
        },
    },
    Tool {
        name: "merge_documents",
        title: "Combine PDFs",
        description: "Combine 2–500 PDFs, in the given order, into one new PDF.",
        effect: Effect::WritesNewFiles,
        long: true,
        schema: || {
            obj(
                json!({ "paths": { "type": "array", "items": path_prop("PDF file"), "minItems": 2, "maxItems": 500 }, "output": path_prop("New PDF file (must not exist)") }),
                &["paths", "output"],
            )
        },
    },
    Tool {
        name: "split_document",
        title: "Split a PDF",
        description: "Split a PDF into several new files in a folder: every N pages, by page ranges, at each bookmark of an outline level, or into parts no larger than a size.",
        effect: Effect::WritesNewFiles,
        long: true,
        schema: || {
            obj(
                json!({
                    "path": path_prop("Source PDF"),
                    "every_n": { "type": "integer", "minimum": 1 },
                    "ranges": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                    "bookmark_level": { "type": "integer", "minimum": 1, "maximum": 64 },
                    "max_bytes": { "type": "integer", "minimum": 10240 },
                    "output_dir": path_prop("Existing folder for the parts")
                }),
                &["path", "output_dir"],
            )
        },
    },
    Tool {
        name: "fill_form",
        title: "Fill a form",
        description: "Save a copy of a PDF form with field values filled in. Values: text for text fields, choices and radio groups (a radio group takes its option's export value); true/false for checkboxes. Field names come from list_form_fields.",
        effect: Effect::WritesNewFiles,
        long: false,
        schema: || {
            obj(
                json!({
                    "path": path_prop("PDF form"),
                    "values": { "type": "object", "additionalProperties": { "type": ["string", "boolean"] }, "minProperties": 1 },
                    "output": path_prop("New PDF file (must not exist)")
                }),
                &["path", "values", "output"],
            )
        },
    },
    Tool {
        name: "recognize_text",
        title: "Recognize text (OCR)",
        description: "Make scanned pages searchable: recognizes text on the device and saves a copy with an invisible text layer; pages look the same. Pages that already have text are left alone.",
        effect: Effect::WritesNewFiles,
        long: true,
        schema: || {
            obj(
                json!({ "path": path_prop("Source PDF"), "pages": { "type": "string", "description": PAGES }, "output": path_prop("New PDF file (must not exist)") }),
                &["path", "output"],
            )
        },
    },
    Tool {
        name: "reduce_file_size",
        title: "Reduce file size",
        description: "Save a smaller copy that looks and reads the same (lossless). With image_dpi, images shown above that resolution are also resampled (lossy).",
        effect: Effect::WritesNewFiles,
        long: true,
        schema: || {
            obj(
                json!({
                    "path": path_prop("Source PDF"),
                    "image_dpi": { "type": "integer", "minimum": 36, "maximum": 1200 },
                    "image_quality": { "type": "integer", "minimum": 30, "maximum": 95, "default": 75 },
                    "output": path_prop("New PDF file (must not exist)")
                }),
                &["path", "output"],
            )
        },
    },
    Tool {
        name: "export_text",
        title: "Export as text",
        description: "Save the text of all or chosen pages as a UTF-8 .txt file with a heading per page.",
        effect: Effect::WritesNewFiles,
        long: false,
        schema: || {
            obj(
                json!({ "path": path_prop("Source PDF"), "pages": { "type": "string", "description": PAGES }, "output": path_prop("New .txt file (must not exist)") }),
                &["path", "output"],
            )
        },
    },
    Tool {
        name: "convert_to_docx",
        title: "Convert to Word",
        description: "Convert all or chosen pages to an editable Word document (.docx): paragraphs, headings, columns, pictures and links. The result reports what could not be carried over (drawn lines and table borders, comments, form fields, pages without a text layer).",
        effect: Effect::WritesNewFiles,
        long: true,
        schema: || {
            obj(
                json!({ "path": path_prop("Source PDF"), "pages": { "type": "string", "description": PAGES }, "output": path_prop("New .docx file (must not exist)") }),
                &["path", "output"],
            )
        },
    },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

fn args<T: for<'de> Deserialize<'de>>(v: &Value) -> PwResult<T> {
    serde_json::from_value(v.clone()).map_err(|e| {
        // serde's message names the field; it never contains file content.
        PwError::invalid("mcp.arguments", format!("The tool arguments are not valid: {e}"))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArg {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PagesArg {
    path: String,
    #[serde(default)]
    pages: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArg {
    path: String,
    query: String,
    #[serde(default)]
    match_case: bool,
    #[serde(default)]
    whole_word: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternsArg {
    path: String,
    #[serde(default)]
    kinds: Vec<pw_core::ops::TextPattern>,
    #[serde(default)]
    custom: Option<String>,
    #[serde(default)]
    pages: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareArg {
    old: String,
    new: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PagesOutArg {
    path: String,
    pages: String,
    output: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptPagesOutArg {
    path: String,
    #[serde(default)]
    pages: Option<String>,
    output: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateArg {
    path: String,
    #[serde(default)]
    pages: Option<String>,
    degrees: i32,
    output: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeArg {
    paths: Vec<String>,
    output: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SplitArg {
    path: String,
    #[serde(default)]
    every_n: Option<u32>,
    #[serde(default)]
    ranges: Option<Vec<String>>,
    #[serde(default)]
    bookmark_level: Option<u32>,
    #[serde(default)]
    max_bytes: Option<u64>,
    output_dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillArg {
    path: String,
    values: BTreeMap<String, Value>,
    output: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptimizeArg {
    path: String,
    #[serde(default)]
    image_dpi: Option<u32>,
    #[serde(default)]
    image_quality: Option<u8>,
    output: String,
}

/// The typed operation for a tool call, with every path checked against the approved folders.
pub fn operation(tool: &Tool, a: &Value, roots: &Roots) -> PwResult<Operation> {
    let pdf = |p: &str| roots.output(p, &["pdf"]);
    Ok(match tool.name {
        "inspect_document" => Operation::Inspect { input: roots.input(&args::<PathArg>(a)?.path)? },
        "list_form_fields" => Operation::FormFields { input: roots.input(&args::<PathArg>(a)?.path)? },
        "check_signatures" => {
            Operation::VerifySignatures { input: roots.input(&args::<PathArg>(a)?.path)?, trusted: vec![] }
        }
        "extract_text" => {
            let x: PagesArg = args(a)?;
            Operation::ExtractText { input: roots.input(&x.path)?, pages: x.pages }
        }
        "search_text" => {
            let x: SearchArg = args(a)?;
            Operation::Search {
                input: roots.input(&x.path)?,
                query: x.query,
                match_case: x.match_case,
                whole_word: x.whole_word,
            }
        }
        "check_accessibility" => Operation::CheckAccessibility { input: roots.input(&args::<PathArg>(a)?.path)? },
        "find_sensitive_text" => {
            let x: PatternsArg = args(a)?;
            Operation::FindPatterns {
                input: roots.input(&x.path)?,
                patterns: x.kinds,
                custom: x.custom,
                pages: x.pages,
            }
        }
        "compare_documents" => {
            let x: CompareArg = args(a)?;
            Operation::Compare { old: roots.input(&x.old)?, new: roots.input(&x.new)? }
        }
        "extract_pages" => {
            let x: PagesOutArg = args(a)?;
            Operation::ExtractPages { input: roots.input(&x.path)?, pages: x.pages, output: pdf(&x.output)? }
        }
        "delete_pages" => {
            let x: PagesOutArg = args(a)?;
            Operation::DeletePages { input: roots.input(&x.path)?, pages: x.pages, output: pdf(&x.output)? }
        }
        "rotate_pages" => {
            let x: RotateArg = args(a)?;
            Operation::RotatePages {
                input: roots.input(&x.path)?,
                pages: x.pages,
                degrees: x.degrees,
                output: pdf(&x.output)?,
            }
        }
        "merge_documents" => {
            let x: MergeArg = args(a)?;
            if x.paths.len() > 500 {
                return Err(PwError::invalid("merge.too_many", "Combine at most 500 files at once."));
            }
            let inputs = x.paths.iter().map(|p| roots.input(p)).collect::<PwResult<Vec<_>>>()?;
            Operation::Merge { inputs, output: pdf(&x.output)? }
        }
        "split_document" => {
            let x: SplitArg = args(a)?;
            let modes = [x.every_n.is_some(), x.ranges.is_some(), x.bookmark_level.is_some(), x.max_bytes.is_some()];
            if modes.iter().filter(|m| **m).count() != 1 {
                return Err(PwError::invalid(
                    "mcp.split_mode",
                    "Give exactly one of every_n, ranges, bookmark_level or max_bytes.",
                ));
            }
            let split = if let Some(n) = x.every_n {
                SplitMode::EveryN { n }
            } else if let Some(ranges) = x.ranges {
                SplitMode::Ranges { ranges }
            } else if let Some(level) = x.bookmark_level {
                SplitMode::Bookmarks { level }
            } else {
                SplitMode::MaxSize { max_bytes: x.max_bytes.unwrap_or_default() }
            };
            Operation::Split { input: roots.input(&x.path)?, split, output_dir: roots.output_dir(&x.output_dir)? }
        }
        "fill_form" => {
            let x: FillArg = args(a)?;
            let values = x
                .values
                .into_iter()
                .map(|(name, v)| {
                    let value = match v {
                        Value::Bool(b) => FormValue::Checked(b),
                        Value::String(s) => FormValue::Text(s),
                        _ => {
                            return Err(PwError::invalid(
                                "mcp.form_value",
                                format!("The value for “{name}” must be text or true/false."),
                            ))
                        }
                    };
                    Ok(FieldValue { name, value })
                })
                .collect::<PwResult<Vec<_>>>()?;
            Operation::FillForm { input: roots.input(&x.path)?, values, output: pdf(&x.output)? }
        }
        "recognize_text" => {
            let x: OptPagesOutArg = args(a)?;
            Operation::OcrPages {
                input: roots.input(&x.path)?,
                pages: x.pages,
                skip_text_pages: true,
                dpi: pw_core::ops::DEFAULT_OCR_DPI,
                output: pdf(&x.output)?,
            }
        }
        "reduce_file_size" => {
            let x: OptimizeArg = args(a)?;
            let images = x.image_dpi.map(|max_dpi| ImageReduce { max_dpi, quality: x.image_quality.unwrap_or(75) });
            if images.is_none() && x.image_quality.is_some() {
                return Err(PwError::invalid("mcp.image_quality", "image_quality needs image_dpi."));
            }
            Operation::Optimize { input: roots.input(&x.path)?, images, output: pdf(&x.output)? }
        }
        "export_text" => {
            let x: OptPagesOutArg = args(a)?;
            Operation::ExportText {
                input: roots.input(&x.path)?,
                pages: x.pages,
                output: roots.output(&x.output, &["txt"])?,
            }
        }
        "convert_to_docx" => {
            let x: OptPagesOutArg = args(a)?;
            Operation::ConvertToDocx {
                input: roots.input(&x.path)?,
                pages: x.pages,
                output: roots.output(&x.output, &["docx"])?,
            }
        }
        other => {
            return Err(PwError::new(
                ErrorCategory::UnsupportedOperation,
                "mcp.unknown_tool",
                format!("PageWeave has no tool named “{other}”."),
            ))
        }
    })
}

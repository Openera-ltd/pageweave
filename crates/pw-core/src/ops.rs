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
        )
    }

    /// All input paths, used to protect sources from being overwritten.
    pub fn inputs(&self) -> Vec<PathBuf> {
        match self {
            Operation::Merge { inputs, .. } => inputs.clone(),
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
            | Operation::Search { .. } => None,
            Operation::Split { output_dir, .. } => Some(output_dir.clone()),
            Operation::Render { output, .. }
            | Operation::Merge { output, .. }
            | Operation::ExtractPages { output, .. }
            | Operation::DeletePages { output, .. }
            | Operation::RotatePages { output, .. }
            | Operation::Annotate { output, .. }
            | Operation::FillForm { output, .. }
            | Operation::ApplyEdits { output, .. }
            | Operation::OrganizePages { output, .. }
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
}

impl OperationRequest {
    pub fn new(operation: Operation) -> Self {
        OperationRequest {
            schema_version: SCHEMA_VERSION,
            request_id: None,
            operation,
            collision: CollisionPolicy::Fail,
            password: None,
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
        self.operation.validate()
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

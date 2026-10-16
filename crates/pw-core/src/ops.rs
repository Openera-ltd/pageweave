//! Versioned, typed operation contract (PRD 04 "Boundaries and contracts", PRD 17).
//!
//! Every interface — desktop UI, CLI, local API, MCP — builds an [`OperationRequest`] and hands it to
//! the same engine. Validation here is structural (shape, bounds); the engine validates against the
//! actual document (page counts, encryption, …).

use crate::error::{PwError, PwResult};
use crate::output::CollisionPolicy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Inspect {
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
}

impl Operation {
    pub fn id(&self) -> &'static str {
        match self {
            Operation::Inspect { .. } => "inspect",
            Operation::ExtractText { .. } => "extract_text",
            Operation::Search { .. } => "search",
            Operation::Render { .. } => "render",
            Operation::Merge { .. } => "merge",
            Operation::ExtractPages { .. } => "extract_pages",
            Operation::DeletePages { .. } => "delete_pages",
            Operation::RotatePages { .. } => "rotate_pages",
            Operation::Split { .. } => "split",
        }
    }

    /// Whether the operation writes files.
    pub fn writes_output(&self) -> bool {
        !matches!(self, Operation::Inspect { .. } | Operation::ExtractText { .. } | Operation::Search { .. })
    }

    /// All input paths, used to protect sources from being overwritten.
    pub fn inputs(&self) -> Vec<PathBuf> {
        match self {
            Operation::Merge { inputs, .. } => inputs.clone(),
            Operation::Inspect { input }
            | Operation::ExtractText { input, .. }
            | Operation::Search { input, .. }
            | Operation::Render { input, .. }
            | Operation::ExtractPages { input, .. }
            | Operation::DeletePages { input, .. }
            | Operation::RotatePages { input, .. }
            | Operation::Split { input, .. } => vec![input.clone()],
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
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub schema_version: u32,
    #[serde(default)]
    pub request_id: Option<String>,
    pub operation: Operation,
    #[serde(default)]
    pub collision: CollisionPolicy,
}

impl OperationRequest {
    pub fn new(operation: Operation) -> Self {
        OperationRequest {
            schema_version: SCHEMA_VERSION,
            request_id: None,
            operation,
            collision: CollisionPolicy::Fail,
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
        let mut r = OperationRequest::new(Operation::Inspect { input: "a".into() });
        r.schema_version = 2;
        assert_eq!(r.validate().expect_err("version").code, "op.schema_version");
    }
}

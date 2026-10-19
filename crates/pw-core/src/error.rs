//! Typed error taxonomy shared by every interface (UI, CLI, API, MCP).
//!
//! Messages are safe to show users and to log: they must never contain document
//! text, credentials or full paths. Technical detail goes in `code`.

use serde::{Deserialize, Serialize};
use std::io;

/// Error categories from PRD 04 "Errors and results".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    InvalidInput,
    UnsupportedOperation,
    UnsupportedFormat,
    Corrupt,
    PasswordRequired,
    Permission,
    NotFound,
    OutputConflict,
    DiskFull,
    MissingResource,
    CorruptResource,
    Network,
    Cancelled,
    Timeout,
    ValidationFailed,
    SecurityPolicy,
    EngineFailure,
    Internal,
}

impl ErrorCategory {
    /// Whether retrying the same request unchanged could plausibly succeed.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCategory::Network | ErrorCategory::Timeout | ErrorCategory::DiskFull | ErrorCategory::EngineFailure
        )
    }

    /// Stable process exit code for the CLI (0 is success, 1 is reserved for usage errors).
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCategory::InvalidInput => 2,
            ErrorCategory::UnsupportedOperation | ErrorCategory::UnsupportedFormat => 3,
            ErrorCategory::Corrupt => 4,
            ErrorCategory::PasswordRequired => 5,
            ErrorCategory::Permission => 6,
            ErrorCategory::NotFound => 7,
            ErrorCategory::OutputConflict => 8,
            ErrorCategory::DiskFull => 9,
            ErrorCategory::MissingResource | ErrorCategory::CorruptResource => 10,
            ErrorCategory::Network => 11,
            ErrorCategory::Cancelled => 12,
            ErrorCategory::Timeout => 13,
            ErrorCategory::ValidationFailed => 14,
            ErrorCategory::SecurityPolicy => 15,
            ErrorCategory::EngineFailure => 16,
            ErrorCategory::Internal => 70,
        }
    }

    fn default_action(self) -> &'static str {
        match self {
            ErrorCategory::InvalidInput => "Correct the highlighted value and try again.",
            ErrorCategory::UnsupportedOperation | ErrorCategory::UnsupportedFormat => {
                "Choose a supported file or operation."
            }
            ErrorCategory::Corrupt => "The file appears damaged. Try another copy of the file.",
            ErrorCategory::PasswordRequired => "Enter the document password.",
            ErrorCategory::Permission => "Choose a location you have permission to use.",
            ErrorCategory::NotFound => "Check that the file still exists and try again.",
            ErrorCategory::OutputConflict => "Choose a different output name or collision policy.",
            ErrorCategory::DiskFull => "Free some disk space and retry.",
            ErrorCategory::MissingResource | ErrorCategory::CorruptResource => {
                "Open Setup to install or repair resources."
            }
            ErrorCategory::Network => "Check your connection and retry.",
            ErrorCategory::Cancelled => "No changes were committed.",
            ErrorCategory::Timeout => "Retry, or try a smaller input.",
            ErrorCategory::ValidationFailed => "The output failed verification and was not saved.",
            ErrorCategory::SecurityPolicy => "This action is blocked by a safety policy.",
            ErrorCategory::EngineFailure => "Retry. If it keeps failing, export diagnostics.",
            ErrorCategory::Internal => "This is a bug. Export diagnostics and report it.",
        }
    }
}

/// Structured, user-safe error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct PwError {
    pub category: ErrorCategory,
    /// Stable machine-readable code, e.g. `range.out_of_bounds`.
    pub code: String,
    /// Safe, human-readable message. Never contains document content or secrets.
    pub message: String,
    pub retryable: bool,
    pub action: String,
}

impl PwError {
    pub fn new(category: ErrorCategory, code: impl Into<String>, message: impl Into<String>) -> Self {
        PwError {
            category,
            code: code.into(),
            message: message.into(),
            retryable: category.retryable(),
            action: category.default_action().to_string(),
        }
    }

    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::InvalidInput, code, message)
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCategory::Cancelled, "job.cancelled", "The operation was cancelled.")
    }

    pub fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Internal, code, message)
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = action.into();
        self
    }

    /// Map an I/O error. `what` describes the object in safe terms ("output file"), never a path.
    pub fn from_io(err: &io::Error, what: &str) -> Self {
        use io::ErrorKind as K;
        let (cat, code, msg) = match err.kind() {
            K::NotFound => (ErrorCategory::NotFound, "io.not_found", format!("The {what} was not found.")),
            K::PermissionDenied => {
                (ErrorCategory::Permission, "io.permission_denied", format!("Permission denied for the {what}."))
            }
            K::AlreadyExists => {
                (ErrorCategory::OutputConflict, "io.already_exists", format!("The {what} already exists."))
            }
            K::StorageFull | K::QuotaExceeded | K::FileTooLarge => {
                (ErrorCategory::DiskFull, "io.storage_full", "Not enough disk space.".to_string())
            }
            K::ReadOnlyFilesystem => {
                (ErrorCategory::Permission, "io.read_only", format!("The location of the {what} is read-only."))
            }
            K::InvalidFilename => {
                (ErrorCategory::InvalidInput, "io.invalid_filename", format!("The {what} name is not valid."))
            }
            K::TimedOut => (ErrorCategory::Timeout, "io.timed_out", format!("Timed out accessing the {what}.")),
            _ => (ErrorCategory::Internal, "io.other", format!("Could not access the {what}.")),
        };
        PwError::new(cat, code, msg)
    }
}

pub type PwResult<T> = Result<T, PwError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_mapping_is_specific_and_safe() {
        let e =
            PwError::from_io(&io::Error::new(io::ErrorKind::PermissionDenied, "C:\\secret\\path.pdf"), "output file");
        assert_eq!(e.category, ErrorCategory::Permission);
        assert!(!e.message.contains("secret"), "message must not leak the raw OS error/path");
        let full = PwError::from_io(&io::Error::from(io::ErrorKind::StorageFull), "output file");
        assert_eq!(full.category, ErrorCategory::DiskFull);
        assert!(full.retryable);
    }

    #[test]
    fn exit_codes_are_unique_per_family() {
        use ErrorCategory::*;
        let cats = [
            InvalidInput,
            UnsupportedOperation,
            Corrupt,
            PasswordRequired,
            Permission,
            NotFound,
            OutputConflict,
            DiskFull,
            MissingResource,
            Network,
            Cancelled,
            Timeout,
            ValidationFailed,
            SecurityPolicy,
            EngineFailure,
            Internal,
        ];
        let mut codes: Vec<i32> = cats.iter().map(|c| c.exit_code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), cats.len());
        assert!(codes.iter().all(|c| *c >= 2));
    }

    #[test]
    fn serializes_with_snake_case_category() {
        let e = PwError::invalid("range.empty", "Enter at least one page.");
        let json = serde_json::to_value(&e).expect("serialize");
        assert_eq!(json["category"], "invalid_input");
        assert_eq!(json["retryable"], false);
    }
}

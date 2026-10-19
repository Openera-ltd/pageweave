//! PageWeave PDF engine adapter over PDFium (decision D-002).
//!
//! PDFium is loaded once per process and shared. All calls are serialized by pdfium-render's
//! `thread_safe` feature, so the adapter may be used from worker threads; it must never be called
//! from a UI thread. PDF JavaScript and XFA are disabled in the pinned build.
//!
//! Engine access goes through [`engine`], which holds one reentrant lock for the whole operation,
//! so multi-call sequences over PDFium's global state are atomic. The form module calls a few
//! PDFium functions directly (pdfium-render does not expose its form-fill handle) under the same lock.

mod attachments;
mod blank;
mod compare;
mod content_ops;
pub mod deskew;
mod document;
mod downsample;
mod edit;
mod exec;
mod extract;
mod forms;
mod images;
mod marks;
pub mod objects;
mod ocr;
mod optimize;
pub mod orient;
mod outline;
mod properties;
mod protect;
mod redact;
mod sanitize;
mod sign;
mod signatures;
mod unprotect;

pub use attachments::{add_attachments, clean_name, extract_attachment, list_attachments, AttachmentInfo};
pub use blank::{find_blank_pages, PageInk};
pub use compare::{compare, ChangeKind, CompareReport, PairStatus, MAX_COMPARE_PAGES};
pub use document::{
    open_document, page_text_segments, render_page_png, search, BookmarkInfo, DocumentInfo, OpenOptions, PageInfo,
    PageText, SearchHit, SearchRect, TextSegment,
};
pub use edit::{
    annotate, crop_pages, delete_pages, extract_pages, merge, organize_pages, rotate_pages, split, watermark,
};
pub use exec::execute;
pub use extract::{extract_images, ExtractedImage};
pub use forms::{apply_edits, fill_form, form_fields, FieldKind, FormField, FormInfo, FormWidget};
pub use images::images_to_pdf;
pub use ocr::{ocr_pages, OcrSummary};
pub use optimize::{optimize, OptimizeSummary};
pub use outline::set_bookmarks;
pub use properties::{set_properties, PropertyChanges};
pub use protect::protect;
pub use redact::{redact, redaction_preview, AreaPreview};
pub use sanitize::{sanitize, sanitize_inspect, SanitizeFindings};
pub use signatures::{
    fingerprint, load_trusted_certificates, parse_certificates, signature_chain, verify_signatures, ChainCertificate,
    SignatureInfo, SignatureReport, SignatureStatus, TrustStatus, TRUST_NOTE,
};
pub use unprotect::unprotect;

use pdfium_render::prelude::*;
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static PDFIUM: OnceLock<Pdfium> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
/// Path of the PDFium library that was bound (used to resolve the form-fill functions).
static LIB_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Shared for pdfium-render use, exclusive for direct PDFium calls (see module docs).
/// Serializes engine use per *operation* (reentrant, so nested use on one thread is fine).
///
/// pdfium-render serializes individual PDFium calls, but several operations are multi-call
/// sequences over process-global PDFium state — e.g. a failed load followed by FPDF_GetLastError.
/// With calls from different threads interleaving, a corrupt file was reported as "password
/// protected" when another thread's load failed in between (reproduced in tests). Holding this lock
/// for a whole operation makes every sequence atomic. PDFium does not run calls in parallel anyway.
static ENGINE_LOCK: parking_lot::ReentrantMutex<()> = parking_lot::ReentrantMutex::new(());

/// Engine access for one operation: holds the engine lock while in scope.
pub(crate) struct Engine {
    _guard: parking_lot::ReentrantMutexGuard<'static, ()>,
    pdfium: &'static Pdfium,
}

impl std::ops::Deref for Engine {
    type Target = Pdfium;
    fn deref(&self) -> &Pdfium {
        self.pdfium
    }
}

/// Initialize if needed and take the engine lock for the caller's operation.
pub(crate) fn engine() -> PwResult<Engine> {
    let pdfium = init(None)?;
    Ok(Engine { _guard: ENGINE_LOCK.lock(), pdfium })
}

/// Engine lock plus the library path, for the direct PDFium calls of the forms module.
pub(crate) fn engine_exclusive() -> PwResult<(parking_lot::ReentrantMutexGuard<'static, ()>, &'static Path)> {
    init(None)?;
    let path = LIB_PATH.get().ok_or_else(|| {
        PwError::new(
            ErrorCategory::EngineFailure,
            "pdfium.forms_unavailable",
            "Form filling is not available in this session.",
        )
    })?;
    Ok((ENGINE_LOCK.lock(), path.as_path()))
}

/// Environment variable that overrides where the PDFium library is looked up (debug builds only).
pub const PDFIUM_DIR_ENV: &str = "PAGEWEAVE_PDFIUM_DIR";

/// Candidate directories for the PDFium shared library, in priority order.
///
/// Release builds only look next to the executable (and its `resources` folder). The environment
/// override and the development checkout path exist only in debug builds, so a packaged app never
/// probes a build-machine path or an environment-controlled folder for a DLL (PRD 11: DLL hijacking).
pub fn library_search_dirs(explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = explicit {
        dirs.push(d.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("resources"));
        }
    }
    if cfg!(debug_assertions) {
        if let Some(d) = std::env::var_os(PDFIUM_DIR_ENV) {
            dirs.insert(usize::from(explicit.is_some()), PathBuf::from(d));
        }
        // Development checkout: third_party/pdfium/<platform>/{lib,bin} fetched by tools/fetch_pdfium.py.
        let platform = if cfg!(windows) { "win-x64" } else { "linux-x64" };
        let sub = if cfg!(windows) { "bin" } else { "lib" };
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        dirs.push(workspace.join("third_party").join("pdfium").join(platform).join(sub));
    }
    dirs
}

/// Initialize PDFium (idempotent). Returns a `MissingResource` error if the library cannot be found.
/// A failure is not cached, so a later call can succeed after resources are repaired.
pub fn init(explicit_dir: Option<&Path>) -> PwResult<&'static Pdfium> {
    if let Some(p) = PDFIUM.get() {
        return Ok(p);
    }
    let _guard = INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(p) = PDFIUM.get() {
        return Ok(p);
    }
    for dir in library_search_dirs(explicit_dir) {
        let candidate = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if !candidate.is_file() {
            continue;
        }
        let pdfium = match Pdfium::bind_to_library(&candidate) {
            Ok(bindings) => {
                let _ = LIB_PATH.set(candidate.clone());
                Pdfium::new(bindings)
            }
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
            Err(_) => continue,
        };
        return Ok(PDFIUM.get_or_init(|| pdfium));
    }
    Err(PwError::new(ErrorCategory::MissingResource, "pdfium.not_found", "The PDF engine component is missing.")
        .with_action("Reinstall PageWeave or run Setup to repair resources."))
}

/// Engine version string for diagnostics and result reports.
pub fn engine_version() -> &'static str {
    "pdfium chromium/7881 (pdfium-render 0.9.4)"
}

pub(crate) fn map_pdfium_error(e: &PdfiumError, during: &str) -> PwError {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            PwError::new(ErrorCategory::PasswordRequired, "pdf.password", "This document is password protected.")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            PwError::new(ErrorCategory::Corrupt, "pdf.format", "The file is damaged or is not a valid PDF.")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => PwError::new(
            ErrorCategory::UnsupportedFormat,
            "pdf.security_handler",
            "This document uses an unsupported security handler.",
        ),
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => {
            PwError::new(ErrorCategory::NotFound, "pdf.file", "The file could not be read.")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PageError) => {
            PwError::new(ErrorCategory::Corrupt, "pdf.page", "A page in this document could not be loaded.")
        }
        PdfiumError::IoError(io) => PwError::from_io(io, "file"),
        PdfiumError::PageIndexOutOfBounds => {
            PwError::invalid("pdf.page_out_of_bounds", "That page does not exist in this document.")
        }
        _ => PwError::new(
            ErrorCategory::EngineFailure,
            format!("pdf.engine.{during}"),
            "The PDF engine reported an error.",
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_engine_use_on_one_thread_does_not_deadlock() {
        let Ok(outer) = super::engine() else { return }; // engine not installed: nothing to check
        let inner = super::engine_exclusive().expect("nested access on the same thread");
        drop(inner);
        drop(outer);
    }

    /// Regression: concurrent failed loads must each report their own error (PDFium's last error
    /// is process-global and was read in a separate call).
    #[test]
    fn concurrent_failed_opens_report_their_own_errors() {
        let fixture = |n: &str| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(n);
        if super::init(None).is_err() {
            return;
        }
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let (name, want) = if t % 2 == 0 {
                    ("encrypted-rc4-user-password.pdf", "pdf.password")
                } else {
                    ("corrupt-truncated.pdf", "")
                };
                let path = fixture(name);
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        match crate::open_document(&path, &crate::OpenOptions { password: None }) {
                            Err(e) if want.is_empty() => assert_ne!(e.code, "pdf.password", "{name} misreported"),
                            Err(e) => assert_eq!(e.code, want, "{name}"),
                            Ok(_) => assert!(want.is_empty(), "{name} must not open without a password"),
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("thread");
        }
    }
}

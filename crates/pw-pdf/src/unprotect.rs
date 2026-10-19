//! Remove password protection (PRD 05): PDFium saves the document without its encryption, keeping
//! everything else. An owner's restrictions are honoured: a restricted file opened with only its
//! open password is refused. Verified before commit: the copy opens without a password, with the
//! same page count and page text, and an independent parser (lopdf) finds no encryption.

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::Path;

/// Pages whose text is compared between source and copy.
const TEXT_CHECK_PAGES: u32 = 3;

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The unprotected copy failed verification ({what}) and was not saved."),
    )
}

/// Page count and the first pages' text of an open document.
pub(crate) fn fingerprint(doc: &PdfDocument) -> PwResult<(u32, Vec<String>)> {
    let n = doc.pages().len().max(0) as u32;
    let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
    let mut texts = Vec::new();
    for i in 0..n.min(TEXT_CHECK_PAGES) {
        let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        texts.push(page.text().map(|t| t.inside_rect(everything)).unwrap_or_default());
    }
    Ok((n, texts))
}

pub(crate) fn is_protected(doc: &PdfDocument) -> bool {
    !matches!(doc.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected))
}

pub fn unprotect(
    input: &Path,
    opts: &OpenOptions,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    let pdfium = engine()?;
    let (count, texts) = {
        let src = load(&pdfium, input, opts)?;
        if !is_protected(&src) {
            return Err(PwError::invalid("unprotect.not_protected", "This file is not password protected."));
        }
        fingerprint(&src)?
    };
    let bytes = crate::forms::save_without_security(input, opts.password)?;
    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        let d = pdfium.load_pdf_from_file(staged, None).map_err(|_| fail("it does not open without a password"))?;
        if is_protected(&d) {
            return Err(fail("it is still protected"));
        }
        if fingerprint(&d).map_err(|_| fail("page unreadable"))? != (count, texts.clone()) {
            return Err(fail("pages or text changed"));
        }
        let raw = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        let check = lopdf::Document::load_mem(&raw).map_err(|_| fail("independent parser could not read it"))?;
        if check.trailer.has(b"Encrypt") {
            return Err(fail("an encryption dictionary remains"));
        }
        Ok(())
    })?;
    Ok((c, count, vec!["Anyone with the copy can now open it.".into()]))
}

//! Password protection (PRD 05 "Passwords, redaction and sanitization"): a new copy encrypted with
//! AES-256 (PDF 2.0, security handler revision 6) by lopdf, with an open password and permissions.
//! Verified before commit: PDFium must refuse the copy without a password and open it with either
//! password, with the same page count and page text as the source; lopdf must find a revision-6
//! encryption dictionary with exactly the requested permission bits and decrypt it with the open
//! password. Passwords are never logged or written anywhere but into the encrypted file.

use crate::document::{load, OpenOptions};
use crate::edit::save_bytes;
use crate::{engine, map_pdfium_error};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
use pdfium_render::prelude::*;
use pw_core::ops::ProtectPermissions;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Pages whose text is compared between source and protected copy.
const TEXT_CHECK_PAGES: u32 = 3;

pub fn permission_bits(p: &ProtectPermissions) -> Permissions {
    let mut bits = Permissions::empty();
    if p.print {
        bits |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY;
    }
    if p.copy {
        bits |= Permissions::COPYABLE;
    }
    // Text extraction for accessibility is always allowed (screen readers).
    bits |= Permissions::COPYABLE_FOR_ACCESSIBILITY;
    if p.modify {
        bits |= Permissions::MODIFIABLE;
    }
    if p.annotate {
        bits |= Permissions::ANNOTABLE;
    }
    if p.fill_forms {
        bits |= Permissions::FILLABLE;
    }
    if p.assemble {
        bits |= Permissions::ASSEMBLABLE;
    }
    bits
}

fn random_bytes<const N: usize>() -> PwResult<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b)
        .map_err(|_| PwError::internal("protect.random", "The system random number source is unavailable."))?;
    Ok(b)
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The protected copy failed verification ({what}) and was not saved."),
    )
}

/// Unprotected lopdf document of the source, and whether the source was already protected.
/// Protected sources are decrypted by PDFium (its pages copied into a fresh document; lopdf's own
/// decryption produced a malformed file from an RC4 source in testing). Unprotected sources that
/// lopdf cannot parse are normalised by PDFium first.
fn source_document(input: &Path, opts: &OpenOptions) -> PwResult<(lopdf::Document, bool)> {
    let bytes = std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?;
    let parsed = lopdf::Document::load_mem(&bytes);
    if let Ok(doc) = &parsed {
        if !doc.is_encrypted() {
            return Ok((parsed.expect("checked"), false));
        }
    }
    let pdfium = engine()?;
    let src = load(&pdfium, input, opts)?;
    let protected =
        !matches!(src.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected));
    let plain = if protected {
        let mut fresh = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
        let n = src.pages().len();
        if n > 0 {
            fresh
                .pages_mut()
                .copy_page_range_from_document(&src, 0..=(n - 1), 0)
                .map_err(|e| map_pdfium_error(&e, "copy"))?;
        }
        save_bytes(&fresh)?
    } else {
        save_bytes(&src)?
    };
    let doc = lopdf::Document::load_mem(&plain).map_err(|_| {
        PwError::new(ErrorCategory::Corrupt, "protect.parse", "This file could not be prepared for protection.")
    })?;
    Ok((doc, protected))
}

#[allow(clippy::too_many_arguments)]
pub fn protect(
    input: &Path,
    opts: &OpenOptions,
    user_password: &str,
    owner_password: Option<&str>,
    permissions: &ProtectPermissions,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    // Source facts for verification: page count and the first pages' text.
    let (count, texts) = {
        let pdfium = engine()?;
        let doc = load(&pdfium, input, opts)?;
        let n = doc.pages().len().max(0) as u32;
        let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
        let mut texts = Vec::new();
        for i in 0..n.min(TEXT_CHECK_PAGES) {
            let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            texts.push(page.text().map(|t| t.inside_rect(everything)).unwrap_or_default());
        }
        (n, texts)
    };
    let (mut doc, was_protected) = source_document(input, opts)?;
    let generated_owner: String;
    let owner = match owner_password {
        Some(o) => o,
        None => {
            generated_owner = random_bytes::<24>()?.iter().map(|b| format!("{b:02x}")).collect();
            &generated_owner
        }
    };
    let key = random_bytes::<32>()?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let bits = permission_bits(permissions);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password,
        permissions: bits,
    })
    .map_err(|_| PwError::invalid("protect.password", "That password cannot be used for PDF encryption."))?;
    doc.encrypt(&state).map_err(|_| PwError::internal("protect.encrypt", "The file could not be encrypted."))?;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|_| PwError::internal("protect.save", "The protected copy could not be written."))?;

    let pdfium = engine()?;
    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        // PDFium: no password → refused; each password → same pages and text.
        match pdfium.load_pdf_from_file(staged, None) {
            Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => {}
            Ok(_) => return Err(fail("it opens without a password")),
            Err(_) => return Err(fail("it could not be read")),
        }
        for pw in [user_password, owner] {
            let d = pdfium.load_pdf_from_file(staged, Some(pw)).map_err(|_| fail("a password does not open it"))?;
            if d.pages().len().max(0) as u32 != count {
                return Err(fail("page count changed"));
            }
            let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
            for (i, want) in texts.iter().enumerate() {
                let page = d.pages().get(i as PdfPageIndex).map_err(|_| fail("page unreadable"))?;
                if &page.text().map(|t| t.inside_rect(everything)).unwrap_or_default() != want {
                    return Err(fail("page text changed"));
                }
            }
        }
        // lopdf (independent): revision 6, the requested permissions, decrypts with the open password.
        let raw = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        let mut check = lopdf::Document::load_mem(&raw).map_err(|_| fail("independent parser could not read it"))?;
        let enc = check
            .trailer
            .get(b"Encrypt")
            .and_then(|o| o.as_reference())
            .and_then(|id| check.get_dictionary(id))
            .map_err(|_| fail("no encryption dictionary"))?;
        let int = |k: &[u8]| enc.get(k).and_then(|v| v.as_i64()).ok();
        if int(b"V") != Some(5) || int(b"R") != Some(6) {
            return Err(fail("not AES-256"));
        }
        let p = int(b"P").ok_or_else(|| fail("no permissions"))?;
        // Compare the meaningful permission bits (reserved bits are fixed by the specification).
        let mask = Permissions::all().bits();
        if (p as u64 & mask) != (bits.bits() & mask) {
            return Err(fail("permissions differ from those requested"));
        }
        check.decrypt(user_password).map_err(|_| fail("independent parser could not decrypt it"))?;
        Ok(())
    })?;
    let mut warnings =
        vec!["Keep the password safe: it cannot be recovered, and the copy cannot be opened without it.".to_string()];
    if was_protected {
        warnings.push(
            "The original password no longer opens the copy. Bookmarks and document properties were not carried over from the protected original."
                .into(),
        );
    }
    if *permissions != ProtectPermissions::default() {
        warnings.push(
            "Permissions are honoured by most PDF readers but are not absolute security; anyone with the open password can read the content."
                .into(),
        );
    }
    Ok((c, count, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_bits_follow_the_switches() {
        let all = permission_bits(&ProtectPermissions::default());
        assert!(all.contains(Permissions::PRINTABLE | Permissions::COPYABLE | Permissions::MODIFIABLE));
        let none = permission_bits(&ProtectPermissions {
            print: false,
            copy: false,
            modify: false,
            annotate: false,
            fill_forms: false,
            assemble: false,
        });
        assert_eq!(none, Permissions::COPYABLE_FOR_ACCESSIBILITY, "accessibility extraction stays allowed");
    }
}

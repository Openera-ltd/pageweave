//! Embedded files (PRD 05 "attachments"): list them, save one out, embed new ones in a copy.
//! Attachment bytes are untrusted: they are written exactly as stored and never opened or run;
//! names shown to the user are cleaned of path separators and control characters.

use crate::document::{load, OpenOptions};
use crate::edit::save_bytes;
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::{MAX_ATTACH_BYTES, MAX_ATTACH_FILES};
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Listed attachments per document (hostile files can hold very many).
const MAX_LISTED: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachmentInfo {
    pub index: u32,
    /// Display name, cleaned (no path separators or control characters).
    pub name: String,
    pub bytes: u64,
}

/// A file name safe to show and to offer as a save name: last path component, no control
/// characters, no reserved Windows characters, not empty.
pub fn clean_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut s: String = last
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    s = s.trim().trim_matches('.').to_string();
    if s.is_empty() {
        "attachment".into()
    } else {
        s.chars().take(200).collect()
    }
}

pub fn list_attachments(input: &Path, opts: &OpenOptions) -> PwResult<(Vec<AttachmentInfo>, bool)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let all = doc.attachments();
    let mut out = Vec::new();
    for (i, a) in all.iter().enumerate().take(MAX_LISTED) {
        out.push(AttachmentInfo { index: i as u32, name: clean_name(&a.name()), bytes: a.len() as u64 });
    }
    Ok((out, all.len() as usize > MAX_LISTED))
}

pub fn extract_attachment(
    input: &Path,
    opts: &OpenOptions,
    index: u32,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, AttachmentInfo)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let a = doc.attachments().get(index as PdfAttachmentIndex).map_err(|_| {
        PwError::invalid("attach.index", "That attachment is not in this document (the list may be out of date).")
    })?;
    let info = AttachmentInfo { index, name: clean_name(&a.name()), bytes: a.len() as u64 };
    let bytes = a.save_to_bytes().map_err(|e| map_pdfium_error(&e, "attachment"))?;
    let digest = Sha256::digest(&bytes);
    let protected = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if Sha256::digest(&back) == digest {
            Ok(())
        } else {
            Err(PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The saved attachment does not match."))
        }
    })?;
    Ok((c, info))
}

/// `name` made unique among `taken` ("report.txt" → "report (2).txt").
fn unique(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|c| !taken.iter().any(|t| t == c)).expect("unbounded")
}

pub fn add_attachments(
    input: &Path,
    opts: &OpenOptions,
    files: &[std::path::PathBuf],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    if files.is_empty() || files.len() > MAX_ATTACH_FILES {
        return Err(PwError::invalid("attach.none", "Choose between 1 and 100 files to attach."));
    }
    let mut payload = Vec::with_capacity(files.len());
    for f in files {
        let meta = std::fs::metadata(f).map_err(|e| PwError::from_io(&e, "file to attach"))?;
        if !meta.is_file() {
            return Err(PwError::invalid(
                "attach.not_file",
                format!("{} is not a file.", clean_name(&f.to_string_lossy())),
            ));
        }
        if meta.len() > MAX_ATTACH_BYTES {
            return Err(PwError::invalid(
                "attach.too_large",
                format!("{} is larger than 256 MB.", clean_name(&f.to_string_lossy())),
            ));
        }
        payload.push((
            clean_name(&f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            std::fs::read(f).map_err(|e| PwError::from_io(&e, "file to attach"))?,
        ));
    }
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    let total = doc.pages().len().max(0) as u32;
    let mut taken: Vec<String> = doc.attachments().iter().map(|a| a.name()).collect();
    let before = taken.len();
    let mut expected = Vec::new();
    for (name, bytes) in &payload {
        let name = unique(name, &taken);
        doc.attachments_mut().create_attachment_from_bytes(&name, bytes).map_err(|e| map_pdfium_error(&e, "attach"))?;
        expected.push((name.clone(), Sha256::digest(bytes)));
        taken.push(name);
    }
    let out = save_bytes(&doc)?;
    let protected: Vec<_> = std::iter::once(input.to_path_buf()).chain(files.iter().cloned()).collect();
    let c = write_atomic_bytes(output, policy, &protected, &out, |staged| {
        let fail = |what: &str| {
            PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                format!("The copy failed verification ({what})."),
            )
        };
        let back = pdfium.load_pdf_from_file(staged, opts.password).map_err(|_| fail("it could not be reopened"))?;
        if back.pages().len().max(0) as u32 != total {
            return Err(fail("page count changed"));
        }
        let got: Vec<(String, Vec<u8>)> =
            back.attachments().iter().map(|a| (a.name(), a.save_to_bytes().unwrap_or_default())).collect();
        if got.len() != before + expected.len() {
            return Err(fail("attachment count differs"));
        }
        for (name, digest) in &expected {
            if !got.iter().any(|(n, b)| n == name && Sha256::digest(b) == *digest) {
                return Err(fail("an attached file differs"));
            }
        }
        Ok(())
    })?;
    let mut warnings = Vec::new();
    let renamed: Vec<&String> =
        expected.iter().map(|(n, _)| n).filter(|n| !payload.iter().any(|(p, _)| p == *n)).collect();
    if !renamed.is_empty() {
        warnings.push(format!(
            "Renamed to avoid duplicate names: {}.",
            renamed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok((c, total, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_made_unique() {
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("C:\\\\Users\\\\x\\\\evil.exe"), "evil.exe");
        assert_eq!(clean_name("a\u{0}b:c?.txt"), "a_b_c_.txt");
        assert_eq!(clean_name(" .. "), "attachment");
        let taken = vec!["notes.txt".to_string(), "notes (2).txt".to_string()];
        assert_eq!(unique("notes.txt", &taken), "notes (3).txt");
        assert_eq!(unique("README", &["README".to_string()]), "README (2)");
        assert_eq!(unique("new.pdf", &taken), "new.pdf");
    }
}

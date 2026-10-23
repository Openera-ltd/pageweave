//! Page labels (PRD 05 page operations "label"): the numbers readers show for pages, such as i–iv
//! for front matter and 1–20 after it. Written as the catalog's `/PageLabels` number tree with
//! lopdf into a rewritten copy; before commit PDFium must read back, for every page, exactly the
//! label computed by [`pw_core::labels::label_for`], and every page must render as before.

use crate::document::OpenOptions;
use crate::engine;
use crate::properties::signature_warning;
use crate::sanitize::{check_pages, has_earlier_versions, load_plain};
use lopdf::{Dictionary, Object};
use pw_core::labels::label_for;
use pw_core::ops::{LabelStyle, PageLabelRange};
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::Path;

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The copy with page labels failed verification ({what}) and was not saved."),
    )
}

fn style_name(s: LabelStyle) -> Option<&'static [u8]> {
    match s {
        LabelStyle::Decimal => Some(b"D"),
        LabelStyle::RomanUpper => Some(b"R"),
        LabelStyle::RomanLower => Some(b"r"),
        LabelStyle::LettersUpper => Some(b"A"),
        LabelStyle::LettersLower => Some(b"a"),
        LabelStyle::None => None,
    }
}

pub fn set_page_labels(
    input: &Path,
    opts: &OpenOptions,
    ranges: &[PageLabelRange],
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    let pdfium = engine()?;
    {
        let src = crate::document::load(&pdfium, input, opts)?;
        if crate::unprotect::is_protected(&src) {
            return Err(PwError::new(
                ErrorCategory::UnsupportedOperation,
                "labels.protected",
                "Page labels of a password-protected file cannot be changed here.",
            )
            .with_action("Remove the password first (Remove password…), then label that copy."));
        }
    }
    let (source, mut doc, raw) = load_plain(&pdfium, input, opts)?;
    let count = source.pages().len().max(0) as u32;
    if let Some(r) = ranges.iter().find(|r| r.start >= count) {
        return Err(PwError::invalid(
            "labels.page_out_of_range",
            format!("A range starts on page {}, but the document has {count} pages.", r.start + 1),
        ));
    }
    let signatures = source.signatures().len();
    let earlier = has_earlier_versions(&raw);

    let root_id = doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(|_| fail("no catalog"))?;
    let mut nums = Vec::with_capacity(ranges.len() * 2);
    for r in ranges {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"PageLabel".to_vec()));
        if let Some(s) = style_name(r.style) {
            d.set("S", Object::Name(s.to_vec()));
        }
        if !r.prefix.is_empty() {
            d.set("P", lopdf::text_string(&r.prefix));
        }
        if r.first != 1 {
            d.set("St", Object::Integer(r.first as i64));
        }
        nums.push(Object::Integer(r.start as i64));
        nums.push(Object::Dictionary(d));
    }
    {
        let catalog = doc.get_dictionary_mut(root_id).map_err(|_| fail("no catalog"))?;
        if ranges.is_empty() {
            catalog.remove(b"PageLabels");
        } else {
            let mut tree = Dictionary::new();
            tree.set("Nums", Object::Array(nums));
            catalog.set("PageLabels", Object::Dictionary(tree));
        }
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    doc.prune_objects();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|_| PwError::internal("labels.save", "The copy could not be written."))?;

    let want: Vec<Option<String>> = (0..count).map(|p| label_for(ranges, p)).collect();
    let check = |bytes: &[u8]| -> PwResult<()> {
        let out = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|_| fail("it could not be reopened"))?;
        for (i, page) in out.pages().iter().enumerate() {
            let got = page.label().map(str::to_string);
            // PDFium reports no label when there are none, and "" for a style-less, prefix-less range.
            let expected = want.get(i).cloned().flatten();
            if got.as_deref().unwrap_or("") != expected.as_deref().unwrap_or("") {
                return Err(fail(&format!("page {} reads back a different label", i + 1)));
            }
        }
        Ok(())
    };
    check(&bytes)?;
    check_pages(&pdfium, &source, &bytes, cancelled, progress)?;
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(fail("the written file differs"));
        }
        check(&back)
    })?;
    let mut warnings = Vec::new();
    if signatures > 0 {
        warnings.push(signature_warning(signatures));
    }
    if earlier {
        warnings.push("Earlier saved versions of the file were not carried over.".into());
    }
    Ok((c, count, warnings))
}
